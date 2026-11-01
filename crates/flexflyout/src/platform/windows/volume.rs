//! Core Audio: the volume of the default output device, and its changes.
//!
//! A thread of its own binds to the default output device, reports its
//! volume once, then forwards every change Windows notifies (from any app,
//! a key or the taskbar). When the default device changes (headphones
//! plugged in), it binds to the new one. It also carries out the flyout's
//! volume bar and mute button on the device it is bound to.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use anyhow::Context as _;
use flexflyout_core::Volume;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{
    AUDIO_VOLUME_NOTIFICATION_DATA, DEVICE_STATE, EDataFlow, ERole, IMMDeviceEnumerator,
    IMMNotificationClient, IMMNotificationClient_Impl, MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::core::{PCWSTR, implement};

/// Receives the volume: once when bound to a device at the start
/// (`changed` false), then on every change and when bound to another
/// device (`changed` true). Called from COM's threads.
type Sink = Arc<dyn Fn(Volume, bool) + Send + Sync>;

/// What the volume thread is asked to do.
#[derive(Debug)]
enum Request {
    /// The default output device changed.
    DeviceChanged,
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
pub fn spawn(sink: impl Fn(Volume, bool) + Send + Sync + 'static) -> VolumeControl {
    let sink: Sink = Arc::new(sink);
    let (requests, received) = channel();
    let control = VolumeControl {
        requests: requests.clone(),
    };
    let spawned = std::thread::Builder::new()
        .name("volume".into())
        .spawn(move || {
            if let Err(err) = run(sink, requests, received) {
                log::error!("the volume cannot be followed: {err:#}");
            }
        });
    if let Err(err) = spawned {
        log::error!("cannot start the volume thread: {err}");
    }
    control
}

/// Binds to the default device, again after every change of it, and
/// carries out the requests, forever.
fn run(sink: Sink, requests: Sender<Request>, received: Receiver<Request>) -> anyhow::Result<()> {
    // SAFETY: initializes COM for this thread, which lives until the end.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .context("cannot initialize COM")?;
    // SAFETY: plain COM object creation.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .context("cannot list the audio devices")?;
    let watcher: IMMNotificationClient = DeviceWatcher {
        device_changed: requests,
    }
    .into();
    // SAFETY: `watcher` stays alive (and registered) as long as this thread.
    unsafe { enumerator.RegisterEndpointNotificationCallback(&watcher) }
        .context("cannot watch the default audio device")?;

    // Kept alive until the next device change; dropping them stops the
    // notifications for the old device. `None` while there is no output
    // device at all, until one appears.
    let mut bound = bind_or_log(&enumerator, &sink, false);
    loop {
        let Ok(request) = received.recv() else {
            return Ok(());
        };
        match request {
            Request::DeviceChanged => {
                if let Some((endpoint, callback)) = bound.take() {
                    // SAFETY: unregisters the callback registered by `bind`.
                    let _ = unsafe { endpoint.UnregisterControlChangeNotify(&callback) };
                }
                log::info!("the default output device changed");
                // Show the new device's volume (another pair of headphones
                // has a volume of its own).
                bound = bind_or_log(&enumerator, &sink, true);
            }
            Request::SetLevel(level) => {
                if let Some((endpoint, _)) = &bound {
                    // SAFETY: a plain COM call on the bound device. Windows
                    // reports the change through the callback.
                    if let Err(err) =
                        unsafe { endpoint.SetMasterVolumeLevelScalar(level, std::ptr::null()) }
                    {
                        log::warn!("cannot set the volume: {err}");
                    }
                }
            }
            Request::ToggleMute => {
                if let Some((endpoint, _)) = &bound {
                    // SAFETY: plain COM calls on the bound device.
                    let result = unsafe {
                        endpoint
                            .GetMute()
                            .and_then(|muted| endpoint.SetMute(!muted.as_bool(), std::ptr::null()))
                    };
                    if let Err(err) = result {
                        log::warn!("cannot mute or unmute: {err}");
                    }
                }
            }
        }
    }
}

/// [`bind`], with a failure logged.
fn bind_or_log(
    enumerator: &IMMDeviceEnumerator,
    sink: &Sink,
    changed: bool,
) -> Option<(IAudioEndpointVolume, IAudioEndpointVolumeCallback)> {
    bind(enumerator, sink, changed)
        .inspect_err(|err| log::warn!("no output device to follow: {err:#}"))
        .ok()
}

/// Starts following the default output device and reports its volume.
fn bind(
    enumerator: &IMMDeviceEnumerator,
    sink: &Sink,
    changed: bool,
) -> anyhow::Result<(IAudioEndpointVolume, IAudioEndpointVolumeCallback)> {
    // SAFETY: plain COM calls with valid arguments.
    unsafe {
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .context("no default output device")?;
        let endpoint: IAudioEndpointVolume = device
            .Activate(CLSCTX_ALL, None)
            .context("cannot open the device's volume")?;
        let callback: IAudioEndpointVolumeCallback = VolumeCallback {
            sink: Arc::clone(sink),
        }
        .into();
        endpoint
            .RegisterControlChangeNotify(&callback)
            .context("cannot watch the volume")?;
        let volume = Volume {
            level: endpoint.GetMasterVolumeLevelScalar()?,
            muted: endpoint.GetMute()?.as_bool(),
        };
        log::info!("output device volume: {volume:?}");
        sink(volume, changed);
        Ok((endpoint, callback))
    }
}

/// Forwards the volume changes of one device.
#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCallback {
    sink: Sink,
}

impl IAudioEndpointVolumeCallback_Impl for VolumeCallback_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        // SAFETY: Windows passes a valid notification for the call's duration.
        if let Some(data) = unsafe { data.as_ref() } {
            (self.sink)(
                Volume {
                    level: data.fMasterVolume,
                    muted: data.bMuted.as_bool(),
                },
                true,
            );
        }
        Ok(())
    }
}

/// Tells the volume thread when the default output device changes.
#[implement(IMMNotificationClient)]
struct DeviceWatcher {
    device_changed: Sender<Request>,
}

impl IMMNotificationClient_Impl for DeviceWatcher_Impl {
    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        _device_id: &PCWSTR,
    ) -> windows::core::Result<()> {
        if flow == eRender && role == eConsole {
            let _ = self.device_changed.send(Request::DeviceChanged);
        }
        Ok(())
    }

    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }

    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> windows::core::Result<()> {
        Ok(())
    }
}
