//! SMTC (System Media Transport Controls): what the current player plays,
//! and the playback buttons.
//!
//! WinRT calls the event handlers on its own threads. They only post a
//! request to the media thread, which does all the reading (title, artist,
//! cover art) and the button commands in order. Players often send several
//! events for one change (the title, then the cover art), so requests that
//! arrive close together are handled once.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::Duration;

use anyhow::Context as _;
use flexflyout_core::{Command, Cover, NowPlaying, PlaybackStatus};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
    ColorManagementMode, ExifOrientationMode,
};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as SmtcStatus,
};
use windows::Storage::Streams::IRandomAccessStreamReference;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

/// How long to wait for more events after the first one.
const SETTLE: Duration = Duration::from_millis(120);

/// Longest side of the cover art kept in memory, in pixels: the panel shows
/// it at 96 points, so this is enough up to about 3× (scale and display
/// scaling together).
const MAX_COVER_SIDE: u32 = 320;

/// Receives the current track (`None` when no player reports one) and
/// whether it is a different track than before (a reason to show).
type Sink = Box<dyn Fn(Option<NowPlaying>, bool) + Send>;

/// What the media thread is asked to do.
#[derive(Debug)]
enum Request {
    /// Another player became the current one (or the last one closed).
    SessionChanged,
    /// The track's title, artist or cover art changed.
    PropertiesChanged,
    /// The player started, paused or stopped.
    PlaybackChanged,
    /// A button of the flyout was clicked.
    Command(Command),
}

/// Sends the flyout's buttons to the current player.
#[derive(Clone, Debug)]
pub struct Media {
    requests: Sender<Request>,
}

impl Media {
    /// Asks the current player to do `command`.
    pub fn command(&self, command: Command) {
        let _ = self.requests.send(Request::Command(command));
    }
}

/// Starts following the current player.
pub fn spawn(sink: impl Fn(Option<NowPlaying>, bool) + Send + 'static) -> Media {
    let (requests, received) = channel();
    let media = Media {
        requests: requests.clone(),
    };
    let sink: Sink = Box::new(sink);
    let spawned = std::thread::Builder::new()
        .name("media".into())
        .spawn(move || {
            if let Err(err) = run(&sink, &requests, &received) {
                log::error!("the media player cannot be followed: {err:#}");
            }
        });
    if let Err(err) = spawned {
        log::error!("cannot start the media thread: {err}");
    }
    media
}

/// The current player and its event subscriptions.
struct Current {
    session: Session,
    properties_token: i64,
    playback_token: i64,
}

impl Drop for Current {
    fn drop(&mut self) {
        // Stop the events of a player that is no longer the current one.
        let _ = self
            .session
            .RemoveMediaPropertiesChanged(self.properties_token);
        let _ = self.session.RemovePlaybackInfoChanged(self.playback_token);
    }
}

/// Follows the current player until the app ends.
fn run(
    sink: &Sink,
    requests: &Sender<Request>,
    received: &Receiver<Request>,
) -> anyhow::Result<()> {
    // SAFETY: initializes COM for this thread, which lives until the end.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
        .ok()
        .context("cannot initialize COM")?;
    let manager = SessionManager::RequestAsync()
        .and_then(|operation| operation.join())
        .context("SMTC is not available")?;
    let sender = requests.clone();
    manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
        let _ = sender.send(Request::SessionChanged);
        Ok(())
    }))?;

    let mut current = subscribe(&manager, requests);
    let mut track = read_track(current.as_ref(), None);
    log::info!("now playing: {track:?}");
    sink(track.clone(), false);

    loop {
        let Ok(first) = received.recv() else {
            return Ok(());
        };
        // Gather what arrives meanwhile; carry out commands at once.
        let (mut session, mut properties, mut playback) = (false, false, false);
        let mut next = Some(first);
        while let Some(request) = next.take() {
            match request {
                Request::SessionChanged => session = true,
                Request::PropertiesChanged => properties = true,
                Request::PlaybackChanged => playback = true,
                Request::Command(command) => send_command(current.as_ref(), command),
            }
            next = match received.recv_timeout(SETTLE) {
                Ok(request) => Some(request),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            };
        }

        if session {
            current = subscribe(&manager, requests);
        }
        let new = if session || properties {
            read_track(current.as_ref(), None)
        } else if playback {
            // Only the state changed: keep the title and the cover art.
            read_track(current.as_ref(), track.as_ref())
        } else {
            continue; // Only commands.
        };
        if new == track {
            continue;
        }
        // A different, named track is a reason to show the flyout; a
        // pause, a late cover art or a player closing is not.
        let new_track = new.as_ref().is_some_and(|new| {
            !new.title.is_empty() && track.as_ref().is_none_or(|old| !old.same_track(new))
        });
        log::info!(
            "now playing: {new:?}{}",
            if new_track { " (new track)" } else { "" }
        );
        track = new;
        sink(track.clone(), new_track);
    }
}

/// Subscribes to the events of the current player, if there is one.
fn subscribe(manager: &SessionManager, requests: &Sender<Request>) -> Option<Current> {
    let session = manager.GetCurrentSession().ok()?;
    let app = session.SourceAppUserModelId().unwrap_or_default();
    log::info!("current player: {app}");
    let sender = requests.clone();
    let properties = session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
        let _ = sender.send(Request::PropertiesChanged);
        Ok(())
    }));
    let sender = requests.clone();
    let playback = session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
        let _ = sender.send(Request::PlaybackChanged);
        Ok(())
    }));
    match (properties, playback) {
        (Ok(properties_token), Ok(playback_token)) => Some(Current {
            session,
            properties_token,
            playback_token,
        }),
        (properties, playback) => {
            log::warn!("cannot follow {app}: {properties:?}, {playback:?}");
            None
        }
    }
}

/// Reads the current track. With `known`, only the playback state is read
/// and the rest is copied from it.
fn read_track(current: Option<&Current>, known: Option<&NowPlaying>) -> Option<NowPlaying> {
    let session = &current?.session;
    let status = session
        .GetPlaybackInfo()
        .and_then(|info| info.PlaybackStatus())
        .map(|status| match status {
            SmtcStatus::Playing => PlaybackStatus::Playing,
            SmtcStatus::Paused => PlaybackStatus::Paused,
            _ => PlaybackStatus::Stopped,
        })
        .unwrap_or_default();
    if let Some(known) = known {
        return Some(NowPlaying {
            status,
            ..known.clone()
        });
    }
    let properties = match session
        .TryGetMediaPropertiesAsync()
        .and_then(|operation| operation.join())
    {
        Ok(properties) => properties,
        Err(err) => {
            log::warn!("cannot read the track: {err}");
            return None;
        }
    };
    let text = |value: windows::core::Result<windows::core::HSTRING>| {
        value.map(|s| s.to_string_lossy()).unwrap_or_default()
    };
    let cover = properties.Thumbnail().ok().and_then(|reference| {
        read_cover(&reference)
            .inspect_err(|err| log::warn!("cannot read the cover art: {err:#}"))
            .ok()
    });
    Some(NowPlaying {
        title: text(properties.Title()),
        artist: text(properties.Artist()),
        status,
        cover: cover.map(Arc::new),
    })
}

/// Decodes cover art (any format Windows reads) and scales it down.
fn read_cover(reference: &IRandomAccessStreamReference) -> anyhow::Result<Cover> {
    let stream = reference.OpenReadAsync()?.join()?;
    let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
    let (width, height) = (decoder.PixelWidth()?, decoder.PixelHeight()?);
    anyhow::ensure!(width > 0 && height > 0, "empty image");
    let shrink = (MAX_COVER_SIDE as f32 / width.max(height) as f32).min(1.0);
    let scaled = |side: u32| ((side as f32 * shrink).round() as u32).max(1);
    let (width, height) = (scaled(width), scaled(height));
    let transform = BitmapTransform::new()?;
    transform.SetScaledWidth(width)?;
    transform.SetScaledHeight(height)?;
    transform.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
    let pixels = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Rgba8,
            BitmapAlphaMode::Premultiplied,
            &transform,
            ExifOrientationMode::IgnoreExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )?
        .join()?;
    Ok(Cover {
        width,
        height,
        rgba: pixels.DetachPixelData()?.to_vec(),
    })
}

/// Asks the current player to carry out a button's command.
fn send_command(current: Option<&Current>, command: Command) {
    let Some(current) = current else {
        log::info!("no player for {command:?}");
        return;
    };
    let session = &current.session;
    let operation = match command {
        Command::Previous => session.TrySkipPreviousAsync(),
        Command::TogglePlayPause => session.TryTogglePlayPauseAsync(),
        Command::Next => session.TrySkipNextAsync(),
    };
    match operation.and_then(|operation| operation.join()) {
        Ok(true) => log::info!("sent {command:?} to the player"),
        Ok(false) => log::info!("the player refused {command:?}"),
        Err(err) => log::warn!("cannot send {command:?}: {err}"),
    }
}
