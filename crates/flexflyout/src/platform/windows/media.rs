//! SMTC (System Media Transport Controls): what the current player plays,
//! where it is in the track, and the playback controls.
//!
//! WinRT calls the event handlers on its own threads. They only post a
//! request to the media thread, which does all the reading (title, artist,
//! cover art) and the button commands in order. Players often send several
//! events for one change (the title, then the cover art), so requests that
//! arrive close together are handled once.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use flexflyout_core::{Command, Cover, NowPlaying, PlaybackStatus, RepeatMode, Timeline};
use windows::Foundation::{DateTime, TypedEventHandler};
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform,
    ColorManagementMode, ExifOrientationMode,
};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackInfo as PlaybackInfo,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as SmtcStatus,
};
use windows::Media::MediaPlaybackAutoRepeatMode as SmtcRepeat;
use windows::Storage::Streams::IRandomAccessStreamReference;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};

/// How long to wait for more events after the first one.
const SETTLE: Duration = Duration::from_millis(120);

/// Longest side of the cover art kept in memory, in pixels: the panel shows
/// it at 96 points, so this is enough up to about 3× (scale and display
/// scaling together).
const MAX_COVER_SIDE: u32 = 320;

/// WinRT times count 100 ns ticks; dates count them from 1601.
const TICKS_PER_SECOND: i64 = 10_000_000;
/// Seconds from 1601-01-01 to 1970-01-01 (the Unix epoch).
const SECONDS_1601_TO_1970: i64 = 11_644_473_600;

/// Receives the current track (`None` when no player reports one) and
/// what changed.
type Sink = Box<dyn Fn(Option<NowPlaying>, Change) + Send>;

/// What changed in an update of the current track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// A different, named track (a reason to show the flyout).
    NewTrack,
    /// The same track started or paused (a reason to show, if the settings
    /// say so).
    PlayPause,
    /// Anything else: the position, a late cover art, the repeat mode, the
    /// player closing.
    Other,
}

/// What the media thread is asked to do.
#[derive(Debug)]
enum Request {
    /// Another player became the current one (or the last one closed).
    SessionChanged,
    /// The track's title, artist or cover art changed.
    PropertiesChanged,
    /// The player started, paused or stopped, or changed its repeat or
    /// shuffle mode.
    PlaybackChanged,
    /// The position in the track changed other than by playing on (a seek),
    /// or the player reported it again.
    TimelineChanged,
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
pub fn spawn(sink: impl Fn(Option<NowPlaying>, Change) + Send + 'static) -> Media {
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
    timeline_token: i64,
}

impl Drop for Current {
    fn drop(&mut self) {
        // Stop the events of a player that is no longer the current one.
        let _ = self
            .session
            .RemoveMediaPropertiesChanged(self.properties_token);
        let _ = self.session.RemovePlaybackInfoChanged(self.playback_token);
        let _ = self
            .session
            .RemoveTimelinePropertiesChanged(self.timeline_token);
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
    sink(track.clone(), Change::Other);

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
                Request::PlaybackChanged | Request::TimelineChanged => playback = true,
                Request::Command(command) => {
                    send_command(current.as_ref(), command);
                    // Read the state back once the player has carried it
                    // out: some players report neither a seek nor a new
                    // repeat mode on their own.
                    playback = true;
                }
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
            // Only the state or the position changed: keep the title and
            // the cover art.
            read_track(current.as_ref(), track.as_ref())
        } else {
            continue; // Only commands.
        };
        if new == track {
            continue;
        }
        let change = match (&track, &new) {
            (_, Some(new)) if new.title.is_empty() => Change::Other,
            (Some(old), Some(new)) if old.same_track(new) => {
                if old.status != new.status {
                    Change::PlayPause
                } else {
                    Change::Other
                }
            }
            (_, Some(_)) => Change::NewTrack,
            (_, None) => Change::Other,
        };
        // The position changes on every seek and with some players every
        // second: not worth a line of its own.
        if change != Change::Other || !same_but_timeline(track.as_ref(), new.as_ref()) {
            log::info!("now playing ({change:?}): {new:?}");
        }
        track = new;
        sink(track.clone(), change);
    }
}

/// Whether the two differ in their timeline at most.
fn same_but_timeline(old: Option<&NowPlaying>, new: Option<&NowPlaying>) -> bool {
    match (old, new) {
        (Some(old), Some(new)) => {
            let old = NowPlaying {
                timeline: None,
                ..old.clone()
            };
            old == NowPlaying {
                timeline: None,
                ..new.clone()
            }
        }
        (old, new) => old.is_none() && new.is_none(),
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
    let sender = requests.clone();
    let timeline = session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
        let _ = sender.send(Request::TimelineChanged);
        Ok(())
    }));
    match (properties, playback, timeline) {
        (Ok(properties_token), Ok(playback_token), Ok(timeline_token)) => Some(Current {
            session,
            properties_token,
            playback_token,
            timeline_token,
        }),
        (properties, playback, timeline) => {
            log::warn!("cannot follow {app}: {properties:?}, {playback:?}, {timeline:?}");
            None
        }
    }
}

/// The playback state, repeat mode and shuffle of the player.
struct Playback {
    status: PlaybackStatus,
    repeat: Option<RepeatMode>,
    shuffle: Option<bool>,
    seekable: bool,
}

/// Reads the playback state; the repeat mode and shuffle only where the
/// player can change them.
fn read_playback(session: &Session) -> Playback {
    let info: Option<PlaybackInfo> = session.GetPlaybackInfo().ok();
    let status = info
        .as_ref()
        .and_then(|info| info.PlaybackStatus().ok())
        .map(|status| match status {
            SmtcStatus::Playing => PlaybackStatus::Playing,
            SmtcStatus::Paused => PlaybackStatus::Paused,
            _ => PlaybackStatus::Stopped,
        })
        .unwrap_or_default();
    let controls = info.as_ref().and_then(|info| info.Controls().ok());
    let enabled = |is_enabled: fn(&_) -> windows::core::Result<bool>| {
        controls
            .as_ref()
            .is_some_and(|controls| is_enabled(controls).unwrap_or(false))
    };
    let repeat = enabled(|c| c.IsRepeatEnabled()).then(|| {
        info.as_ref()
            .and_then(|info| info.AutoRepeatMode().and_then(|mode| mode.Value()).ok())
            .map_or(RepeatMode::Off, |mode| match mode {
                SmtcRepeat::Track => RepeatMode::Track,
                SmtcRepeat::List => RepeatMode::List,
                _ => RepeatMode::Off,
            })
    });
    let shuffle = enabled(|c| c.IsShuffleEnabled()).then(|| {
        info.as_ref()
            .and_then(|info| info.IsShuffleActive().and_then(|on| on.Value()).ok())
            .unwrap_or(false)
    });
    Playback {
        status,
        repeat,
        shuffle,
        seekable: enabled(|c| c.IsPlaybackPositionEnabled()),
    }
}

/// Reads the position and the length of the track, if the player tells
/// them (a live stream has no length).
fn read_timeline(session: &Session, seekable: bool) -> Option<Timeline> {
    let timeline = session.GetTimelineProperties().ok()?;
    let start = timeline.StartTime().ok()?.Duration;
    let end = timeline.EndTime().ok()?.Duration;
    let position = timeline.Position().ok()?.Duration;
    if end <= start {
        return None;
    }
    let length = duration_from_ticks(end - start);
    // The player reports the position as of some moment, which can be a
    // while ago; a date that makes no sense counts as now.
    let now = Instant::now();
    let updated = timeline
        .LastUpdatedTime()
        .ok()
        .and_then(age)
        .filter(|age| *age <= length)
        .and_then(|age| now.checked_sub(age))
        .unwrap_or(now);
    Some(Timeline {
        position: duration_from_ticks(position - start).min(length),
        updated,
        length,
        seekable,
    })
}

/// How long ago `date` was, or `None` if it is in the future.
fn age(date: DateTime) -> Option<Duration> {
    let since_1970 = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    let now = (since_1970.as_secs() as i64 + SECONDS_1601_TO_1970) * TICKS_PER_SECOND
        + i64::from(since_1970.subsec_nanos() / 100);
    let ticks = now - date.UniversalTime;
    (ticks >= 0).then(|| duration_from_ticks(ticks))
}

/// A WinRT duration (100 ns ticks; negative counts as zero).
fn duration_from_ticks(ticks: i64) -> Duration {
    Duration::from_nanos(u64::try_from(ticks).unwrap_or(0).saturating_mul(100))
}

/// Reads the current track. With `known`, only the playback state and the
/// position are read, and the title, artist and cover art are copied from it.
fn read_track(current: Option<&Current>, known: Option<&NowPlaying>) -> Option<NowPlaying> {
    let session = &current?.session;
    let Playback {
        status,
        repeat,
        shuffle,
        seekable,
    } = read_playback(session);
    let timeline = read_timeline(session, seekable);
    if let Some(known) = known {
        return Some(NowPlaying {
            status,
            timeline,
            repeat,
            shuffle,
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
        timeline,
        repeat,
        shuffle,
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
        Command::Seek(position) => {
            // Positions count from the timeline's start, which is usually 0.
            let start = session
                .GetTimelineProperties()
                .and_then(|timeline| timeline.StartTime())
                .map_or(0, |start| start.Duration);
            let ticks = i64::try_from(position.as_nanos() / 100).unwrap_or(i64::MAX);
            session.TryChangePlaybackPositionAsync(start.saturating_add(ticks))
        }
        Command::SetRepeat(mode) => session.TryChangeAutoRepeatModeAsync(match mode {
            RepeatMode::Off => SmtcRepeat::None,
            RepeatMode::Track => SmtcRepeat::Track,
            RepeatMode::List => SmtcRepeat::List,
        }),
        Command::SetShuffle(on) => session.TryChangeShuffleActiveAsync(on),
        Command::SetVolume(_) | Command::ToggleMute | Command::Pin(_) => {
            log::warn!("{command:?} is not for the player");
            return;
        }
    };
    match operation.and_then(|operation| operation.join()) {
        Ok(true) => log::info!("sent {command:?} to the player"),
        Ok(false) => log::info!("the player refused {command:?}"),
        Err(err) => log::warn!("cannot send {command:?}: {err}"),
    }
}
