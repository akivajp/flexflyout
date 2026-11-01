//! Core Audio: the volume of the default output device, and its changes.
//!
//! A thread of its own binds to the default output device, reports its
//! volume once, then forwards every change Windows notifies (from any app,
//! a key or the taskbar). When the default device changes (headphones
//! plugged in), it binds to the new one.

use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

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

/// Receives the volume: once when bound to a device (`changed` false),
/// then on every change (`changed` true). Called from COM's threads.
type Sink = Arc<dyn Fn(Volume, bool) + Send + Sync>;

/// Starts watching the volume.
pub fn spawn(sink: impl Fn(Volume, bool) + Send + Sync + 'static) {
    let sink: Sink = Arc::new(sink);
    let spawned = std::thread::Builder::new()
        .name("volume".into())
        .spawn(move || {
            if let Err(err) = run(sink) {
                log::error!("the volume cannot be followed: {err:#}");
            }
        });
    if let Err(err) = spawned {
        log::error!("cannot start the volume thread: {err}");
    }
}

/// Binds to the default device, again after every change of it, forever.
fn run(sink: Sink) -> anyhow::Result<()> {
    // SAFETY: initializes COM for this thread, which lives until the end.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .context("cannot initialize COM")?;
    // SAFETY: plain COM object creation.
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .context("cannot list the audio devices")?;
    let (device_changed, changes) = channel();
    let watcher: IMMNotificationClient = DeviceWatcher { device_changed }.into();
    // SAFETY: `watcher` stays alive (and registered) as long as this thread.
    unsafe { enumerator.RegisterEndpointNotificationCallback(&watcher) }
        .context("cannot watch the default audio device")?;

    loop {
        // Kept alive until the next device change; dropping them stops the
        // notifications for the old device.
        let bound = match bind(&enumerator, &sink) {
            Ok(bound) => Some(bound),
            Err(err) => {
                // No output device at all: wait until one appears.
                log::warn!("no output device to follow: {err:#}");
                None
            }
        };
        if changes.recv().is_err() {
            return Ok(());
        }
        // Several notifications come for one change (one per role); the
        // first is enough.
        while changes.try_recv().is_ok() {}
        if let Some((endpoint, callback)) = bound {
            // SAFETY: unregisters the callback registered by `bind`.
            let _ = unsafe { endpoint.UnregisterControlChangeNotify(&callback) };
        }
        log::info!("the default output device changed");
    }
}

/// Starts following the default output device and reports its volume.
fn bind(
    enumerator: &IMMDeviceEnumerator,
    sink: &Sink,
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
        sink(volume, false);
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
    device_changed: Sender<()>,
}

impl IMMNotificationClient_Impl for DeviceWatcher_Impl {
    fn OnDefaultDeviceChanged(
        &self,
        flow: EDataFlow,
        role: ERole,
        _device_id: &PCWSTR,
    ) -> windows::core::Result<()> {
        if flow == eRender && role == eConsole {
            let _ = self.device_changed.send(());
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
