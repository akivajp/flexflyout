//! The volume of the default output device, through the PulseAudio
//! protocol (PipeWire answers it too, with pipewire-pulse), and its changes.
//!
//! One connection subscribes to the changes of the sinks and the server
//! (the default sink); the volume thread reads the default sink's volume
//! on another one after each, and carries out the flyout's volume bar and
//! mute button. Volume key presses reach the desktop, which changes the
//! volume; the change is what shows the flyout.

use std::ffi::CString;
use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};

use anyhow::Context as _;
use flexflyout_core::Volume;
use pulseaudio::protocol;

/// Receives the volume: once at the start (`changed` false), then on every
/// change of the level, of muting or of the default device (`changed`
/// true).
type Sink = Box<dyn Fn(Volume, bool) + Send>;

/// What the volume thread is asked to do.
#[derive(Debug)]
enum Request {
    /// A sink or the server changed: read the volume again.
    Refresh,
    /// Set the volume (0.0 to 1.0).
    SetLevel(f32),
    /// Mute, or unmute.
    ToggleMute,
}

/// Changes the volume of the default output device.
#[derive(Clone, Debug)]
pub struct VolumeControl {
    requests: Sender<Request>,
}

impl VolumeControl {
    /// Sets the volume (0.0 to 1.0).
    pub fn set_level(&self, level: f32) {
        let _ = self.requests.send(Request::SetLevel(level.clamp(0.0, 1.0)));
    }

    /// Mutes, or unmutes.
    pub fn toggle_mute(&self) {
        let _ = self.requests.send(Request::ToggleMute);
    }
}

/// Starts watching the volume.
pub fn spawn(sink: impl Fn(Volume, bool) + Send + 'static) -> VolumeControl {
    let (requests, received) = channel();
    let control = VolumeControl {
        requests: requests.clone(),
    };
    let sink: Sink = Box::new(sink);
    let spawned = std::thread::Builder::new()
        .name("volume".into())
        .spawn(move || {
            if let Err(err) = run(&sink, requests, &received) {
                log::error!("the volume cannot be followed: {err:#}");
            }
        });
    if let Err(err) = spawned {
        log::error!("cannot start the volume thread: {err}");
    }
    control
}

/// The default sink as last read.
#[derive(Clone, Debug, PartialEq)]
struct Device {
    name: CString,
    volume: Volume,
    channels: u8,
}

/// Reads the volume after every change, and carries out the requests.
fn run(sink: &Sink, requests: Sender<Request>, received: &Receiver<Request>) -> anyhow::Result<()> {
    let mut pulse = Pulse::connect("FlexFlyout")?;
    let mut events = Pulse::connect("FlexFlyout events")?;
    events.ack(&protocol::Command::Subscribe(
        protocol::SubscriptionMask::SINK | protocol::SubscriptionMask::SERVER,
    ))?;
    let spawned = std::thread::Builder::new()
        .name("volume-events".into())
        .spawn(move || {
            loop {
                match events.event() {
                    Ok(()) => {
                        if requests.send(Request::Refresh).is_err() {
                            return;
                        }
                    }
                    Err(err) => {
                        log::error!("the volume events stopped: {err:#}");
                        return;
                    }
                }
            }
        });
    if let Err(err) = spawned {
        log::error!("cannot watch the volume: {err}");
    }

    let mut device = pulse.default_device()?;
    log::info!("output device {:?}: {:?}", device.name, device.volume);
    sink(device.volume, false);
    while let Ok(request) = received.recv() {
        // Bursts of events come for one change (and for unrelated ones,
        // like a stream starting): read once for all that are waiting.
        let mut refresh = false;
        let mut next = Some(request);
        while let Some(request) = next.take() {
            match request {
                Request::Refresh => refresh = true,
                Request::SetLevel(level) => {
                    let raw = (level * protocol::Volume::NORM.as_u32() as f32).round() as u32;
                    let mut volume = protocol::ChannelVolume::empty();
                    for _ in 0..device.channels.max(1) {
                        volume.push(protocol::Volume::from_u32_clamped(raw));
                    }
                    let set = pulse.ack(&protocol::Command::SetSinkVolume(
                        protocol::SetDeviceVolumeParams {
                            device_index: None,
                            device_name: Some(device.name.clone()),
                            volume,
                        },
                    ));
                    if let Err(err) = set {
                        log::warn!("cannot set the volume: {err:#}");
                    }
                }
                Request::ToggleMute => {
                    let set = pulse.ack(&protocol::Command::SetSinkMute(
                        protocol::SetDeviceMuteParams {
                            device_index: None,
                            device_name: Some(device.name.clone()),
                            mute: !device.volume.muted,
                        },
                    ));
                    if let Err(err) = set {
                        log::warn!("cannot mute or unmute: {err:#}");
                    }
                }
            }
            next = received.try_recv().ok();
        }
        if !refresh {
            continue;
        }
        let new = match pulse.default_device() {
            Ok(new) => new,
            Err(err) => {
                log::warn!("cannot read the volume: {err:#}");
                continue;
            }
        };
        if new.name != device.name {
            log::info!("the default output device is now {:?}", new.name);
        } else if new.volume == device.volume {
            continue;
        }
        device = new;
        sink(device.volume, true);
    }
    Ok(())
}

/// A connection to the sound server.
struct Pulse {
    socket: BufReader<UnixStream>,
    version: u16,
    sequence: u32,
}

impl Pulse {
    /// Connects and introduces itself as `name`.
    fn connect(name: &str) -> anyhow::Result<Self> {
        let path = pulseaudio::socket_path_from_env()
            .context("no PulseAudio or PipeWire sound server found")?;
        let socket = UnixStream::connect(&path)
            .with_context(|| format!("cannot connect to {}", path.display()))?;
        let mut pulse = Self {
            socket: BufReader::new(socket),
            version: protocol::MAX_VERSION,
            sequence: 0,
        };
        let cookie = pulseaudio::cookie_path_from_env()
            .and_then(|path| std::fs::read(path).ok())
            .unwrap_or_default();
        let auth: protocol::AuthReply =
            pulse.request(&protocol::Command::Auth(protocol::AuthParams {
                version: protocol::MAX_VERSION,
                supports_shm: false,
                supports_memfd: false,
                cookie,
            }))?;
        pulse.version = protocol::MAX_VERSION.min(auth.version);
        let mut props = protocol::Props::new();
        props.set(protocol::Prop::ApplicationName, CString::new(name)?);
        let _: protocol::SetClientNameReply =
            pulse.request(&protocol::Command::SetClientName(props))?;
        Ok(pulse)
    }

    /// Sends a command.
    fn send(&mut self, command: &protocol::Command) -> anyhow::Result<()> {
        self.sequence += 1;
        protocol::write_command_message(self.socket.get_mut(), self.sequence, command, self.version)
            .context("cannot talk to the sound server")
    }

    /// Sends a command and reads its reply.
    fn request<T: protocol::CommandReply>(
        &mut self,
        command: &protocol::Command,
    ) -> anyhow::Result<T> {
        self.send(command)?;
        let (_, reply) = protocol::read_reply_message::<T>(&mut self.socket, self.version)
            .context("the sound server refused")?;
        Ok(reply)
    }

    /// Sends a command that is answered with a plain acknowledgement.
    fn ack(&mut self, command: &protocol::Command) -> anyhow::Result<()> {
        self.send(command)?;
        protocol::read_ack_message(&mut self.socket).context("the sound server refused")?;
        Ok(())
    }

    /// Waits for the next subscription event.
    fn event(&mut self) -> anyhow::Result<()> {
        loop {
            let (_, message) = protocol::read_command_message(&mut self.socket, self.version)?;
            if matches!(message, protocol::Command::SubscribeEvent(_)) {
                return Ok(());
            }
        }
    }

    /// The default sink and its volume.
    fn default_device(&mut self) -> anyhow::Result<Device> {
        let server: protocol::ServerInfo = self.request(&protocol::Command::GetServerInfo)?;
        let name = server
            .default_sink_name
            .context("the sound server has no default output")?;
        let info: protocol::SinkInfo =
            self.request(&protocol::Command::GetSinkInfo(protocol::GetSinkInfo {
                index: None,
                name: Some(name.clone()),
            }))?;
        // The loudest channel, as the desktop shows it; 100% is "normal".
        let raw = info
            .cvolume
            .channels()
            .iter()
            .map(protocol::Volume::as_u32)
            .max()
            .unwrap_or(0);
        Ok(Device {
            name,
            volume: Volume {
                level: raw as f32 / protocol::Volume::NORM.as_u32() as f32,
                muted: info.muted,
            },
            channels: info.cvolume.channels().len() as u8,
        })
    }
}
