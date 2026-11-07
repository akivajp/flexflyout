//! The volume of the default output device, through the PulseAudio
//! protocol (PipeWire answers it too, with pipewire-pulse), and its changes.
//!
//! One connection subscribes to the changes of the sinks and the server
//! (the default sink); the volume thread reads the default sink's volume
//! on another one after each, and carries out the flyout's volume bar and
//! mute button. Volume key presses reach the desktop, which changes the
//! volume; the change is what shows the flyout.
//!
//! The sound server can go away while FlexFlyout runs (PipeWire restarts
//! after an update, or by hand to fix a Bluetooth device) or not be there
//! yet at login: the volume thread then connects again, waiting a little
//! longer after every failed attempt.

use std::ffi::CString;
use std::io::BufReader;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use anyhow::Context as _;
use flexflyout_core::Volume;
use pulseaudio::protocol;

/// How long to wait before connecting again, after the first failure and
/// at most: the wait doubles after every failed attempt.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);

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
    /// The connection that watches the changes broke (the sound server
    /// quit or restarted): why.
    Lost(String),
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
        .spawn(move || run(&sink, &requests, &received));
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
    /// The raw volume of each channel (`protocol::Volume::NORM` is 100%).
    channels: Vec<u32>,
}

/// Follows the volume until the app ends, connecting again whenever the
/// sound server goes away or cannot be reached.
fn run(sink: &Sink, requests: &Sender<Request>, received: &Receiver<Request>) {
    // The device reported last, kept across connections.
    let mut known: Option<Device> = None;
    let mut wait = RETRY_FIRST;
    // Only the first failure in a row is logged: a machine without a sound
    // server would otherwise fill the log.
    let mut logged = false;
    loop {
        let mut connected = false;
        let Err(err) = follow(sink, requests, received, &mut known, &mut connected) else {
            return; // The app is closing.
        };
        if connected {
            // It worked for a while: start again with short waits.
            wait = RETRY_FIRST;
            logged = false;
        }
        if !logged {
            log::warn!("the volume cannot be followed: {err:#}; connecting again");
            logged = true;
        }
        std::thread::sleep(wait);
        wait = (wait * 2).min(RETRY_MAX);
        // What was asked meanwhile is about a state that is gone.
        while received.try_recv().is_ok() {}
    }
}

/// Connects to the sound server, reports the volume, then reads it again
/// after every change and carries out the requests. Returns `Ok` when the
/// app ends, and an error when the server cannot be reached or goes away
/// (`connected` tells whether it was reached). `known` is the device
/// reported last.
fn follow(
    sink: &Sink,
    requests: &Sender<Request>,
    received: &Receiver<Request>,
    known: &mut Option<Device>,
    connected: &mut bool,
) -> anyhow::Result<()> {
    let mut pulse = Pulse::connect("FlexFlyout")?;
    let mut events = Pulse::connect("FlexFlyout events")?;
    events.ack(&protocol::Command::Subscribe(
        protocol::SubscriptionMask::SINK | protocol::SubscriptionMask::SERVER,
    ))?;
    let mut device = pulse.default_device()?;
    *connected = true;
    match known.as_ref() {
        None => {
            log::info!("output device {:?}: {:?}", device.name, device.volume);
            sink(device.volume, false);
        }
        // Another device or volume after the server came back: shown like
        // any other change.
        Some(old) if old.name != device.name || old.volume != device.volume => {
            log::info!(
                "connected to the sound server again; output device {:?}: {:?}",
                device.name,
                device.volume
            );
            sink(device.volume, true);
        }
        Some(_) => log::info!("connected to the sound server again"),
    }
    *known = Some(device.clone());

    // The events are read on a thread of their own (reading blocks). It
    // ends, telling why, when the connection breaks; events that came
    // since the subscription wait in its socket.
    let to_volume = requests.clone();
    std::thread::Builder::new()
        .name("volume-events".into())
        .spawn(move || {
            loop {
                if let Err(err) = events.event() {
                    let _ = to_volume.send(Request::Lost(format!("{err:#}")));
                    return;
                }
                if to_volume.send(Request::Refresh).is_err() {
                    return;
                }
            }
        })
        .context("cannot watch the volume")?;

    while let Ok(request) = received.recv() {
        // Bursts of events come for one change (and for unrelated ones,
        // like a stream starting): read once for all that are waiting.
        let mut refresh = false;
        let mut next = Some(request);
        while let Some(request) = next.take() {
            match request {
                Request::Refresh => refresh = true,
                Request::Lost(why) => anyhow::bail!("the sound server went away: {why}"),
                Request::SetLevel(level) => {
                    // The loudest channel at the level, the others as far
                    // below it as they were (the balance stays).
                    let target = (level * protocol::Volume::NORM.as_u32() as f32).round() as u32;
                    let mut volume = protocol::ChannelVolume::empty();
                    for raw in scaled(&device.channels, target) {
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
        if new == device {
            continue;
        }
        // A change of the balance alone is kept (for the next change from
        // the volume bar) but not shown.
        let shown = new.name != device.name || new.volume != device.volume;
        if new.name != device.name {
            log::info!("the default output device is now {:?}", new.name);
        }
        device = new;
        *known = Some(device.clone());
        if shown {
            sink(device.volume, true);
        }
    }
    Ok(())
}

/// The channel volumes that keep the balance of `channels` with the
/// loudest one at `target` (how PulseAudio itself scales a volume). Every
/// channel gets `target` when all are silent, as there is no balance left.
fn scaled(channels: &[u32], target: u32) -> Vec<u32> {
    let loudest = channels.iter().copied().max().unwrap_or(0);
    if loudest == 0 {
        return vec![target; channels.len().max(1)];
    }
    channels
        .iter()
        .map(|&raw| {
            let raw = u64::from(raw) * u64::from(target);
            // Rounded to the nearest; at most `target`, so it fits.
            ((raw + u64::from(loudest) / 2) / u64::from(loudest)) as u32
        })
        .collect()
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
        let channels: Vec<u32> = info
            .cvolume
            .channels()
            .iter()
            .map(protocol::Volume::as_u32)
            .collect();
        // The loudest channel, as the desktop shows it; 100% is "normal".
        let raw = channels.iter().copied().max().unwrap_or(0);
        Ok(Device {
            name,
            volume: Volume {
                level: raw as f32 / protocol::Volume::NORM.as_u32() as f32,
                muted: info.muted,
            },
            channels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_the_volume_keeps_the_balance() {
        let norm = protocol::Volume::NORM.as_u32();
        // Left at half of right: still half after the change.
        assert_eq!(scaled(&[norm / 2, norm], norm / 4), [norm / 8, norm / 4]);
        // Louder than before, and to silence.
        assert_eq!(scaled(&[norm / 4, norm / 2], norm), [norm / 2, norm]);
        assert_eq!(scaled(&[norm, norm / 2], 0), [0, 0]);
        // Balanced channels stay balanced.
        assert_eq!(scaled(&[300, 300, 300], 1000), [1000, 1000, 1000]);
        // All silent (or unknown): every channel at the target.
        assert_eq!(scaled(&[0, 0], 500), [500, 500]);
        assert_eq!(scaled(&[], 500), [500]);
        // The largest raw volumes do not overflow.
        assert_eq!(scaled(&[u32::MAX, u32::MAX / 2], u32::MAX)[0], u32::MAX);
    }
}
