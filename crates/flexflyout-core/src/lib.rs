//! Platform-independent parts of FlexFlyout.
//!
//! This crate depends neither on an OS API nor on egui, so that every host
//! (Windows with eframe now, KDE and GNOME later) shares the same model and
//! the same timing rules, and so that they can be tested anywhere.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub mod config;
pub mod layout;
mod timer;

pub use config::Config;
pub use timer::{FlyoutTimer, Timing};

/// Playback state reported by the media player.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlaybackStatus {
    /// Nothing is playing (or the player does not say).
    #[default]
    Stopped,
    /// A track is playing.
    Playing,
    /// A track is paused.
    Paused,
}

/// What is playing right now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NowPlaying {
    /// Track title.
    pub title: String,
    /// Artist (may be empty, e.g. for videos).
    pub artist: String,
    /// Whether the track is playing or paused.
    pub status: PlaybackStatus,
    /// Cover art, if the player provides one. Shared, because the pixels
    /// travel from the media thread to the window with every update.
    pub cover: Option<Arc<Cover>>,
    /// Position and length of the track, if the player tells them.
    pub timeline: Option<Timeline>,
    /// The repeat mode, or `None` if the player cannot change it.
    pub repeat: Option<RepeatMode>,
    /// Whether shuffle is on, or `None` if the player cannot change it.
    pub shuffle: Option<bool>,
}

impl NowPlaying {
    /// Whether `other` is the same track (title and artist), whatever its
    /// playback state and cover art: a different track is a reason to show
    /// the flyout, a paused one or a late cover art is not.
    pub fn same_track(&self, other: &NowPlaying) -> bool {
        self.title == other.title && self.artist == other.artist
    }

    /// Where playback is at `now`: the position the player last reported,
    /// moved on by the time since then while playing (players report it only
    /// now and then). `None` without a timeline.
    pub fn position_at(&self, now: Instant) -> Option<Duration> {
        let timeline = self.timeline?;
        let position = if self.status == PlaybackStatus::Playing {
            timeline.position + now.saturating_duration_since(timeline.updated)
        } else {
            timeline.position
        };
        Some(position.min(timeline.length))
    }
}

/// What changed between two reports of the current track.
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

impl Change {
    /// What changed from `old` to `new` (`None`: no player).
    pub fn between(old: Option<&NowPlaying>, new: Option<&NowPlaying>) -> Self {
        match (old, new) {
            (_, Some(new)) if new.title.is_empty() => Self::Other,
            (Some(old), Some(new)) if old.same_track(new) => {
                if old.status != new.status {
                    Self::PlayPause
                } else {
                    Self::Other
                }
            }
            (_, Some(_)) => Self::NewTrack,
            (_, None) => Self::Other,
        }
    }
}

/// Position and length of the current track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeline {
    /// The position the player reported.
    pub position: Duration,
    /// When the position was right (the player's report can be older than
    /// the moment it arrives).
    pub updated: Instant,
    /// Length of the track (more than zero).
    pub length: Duration,
    /// Whether the player accepts [`Command::Seek`].
    pub seekable: bool,
}

/// What the player repeats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatMode {
    /// Nothing: playback stops at the end of the list.
    #[default]
    Off,
    /// The current track.
    Track,
    /// The whole list.
    List,
}

impl RepeatMode {
    /// The mode after this one when the repeat button is clicked: off, the
    /// list, the track, off again (the order most players use).
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::List,
            Self::List => Self::Track,
            Self::Track => Self::Off,
        }
    }
}

/// Cover art: RGBA pixels with premultiplied alpha, row by row, already
/// scaled down to about the size it is shown at.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Cover {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

impl fmt::Debug for Cover {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The pixels would flood the log.
        write!(f, "Cover({}x{})", self.width, self.height)
    }
}

/// Volume of the default output device.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Volume {
    /// Level from 0.0 to 1.0 (the scalar the OS reports).
    pub level: f32,
    /// Whether the output is muted (the level is kept while muted).
    pub muted: bool,
}

impl Volume {
    /// Level as a whole percentage (0 to 100), rounded like the OS shows it.
    pub fn percent(&self) -> u8 {
        // Clamp first: drivers occasionally report values slightly out of range.
        (self.level.clamp(0.0, 1.0) * 100.0).round() as u8
    }
}

/// An action the user asks for from the flyout's controls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Go to the previous track.
    Previous,
    /// Play when paused, pause when playing.
    TogglePlayPause,
    /// Go to the next track.
    Next,
    /// Go to this position of the track.
    Seek(Duration),
    /// Use this repeat mode.
    SetRepeat(RepeatMode),
    /// Turn shuffle on or off.
    SetShuffle(bool),
    /// Set the volume (0.0 to 1.0).
    SetVolume(f32),
    /// Mute when not muted, and the other way round.
    ToggleMute,
    /// Keep the flyout shown until it is unpinned (`true`), or let it fade
    /// out again (`false`).
    Pin(bool),
    /// Hide the flyout now (and unpin it).
    Close,
    /// Start moving the (pinned) flyout with the mouse: the pointer was
    /// pressed on its background and is being dragged.
    StartMove,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_rounds_and_clamps() {
        let percent = |level| {
            Volume {
                level,
                muted: false,
            }
            .percent()
        };
        assert_eq!(percent(0.0), 0);
        assert_eq!(percent(0.444), 44);
        assert_eq!(percent(0.445), 45);
        assert_eq!(percent(1.0), 100);
        // Out-of-range values from a driver must not wrap around.
        assert_eq!(percent(-0.1), 0);
        assert_eq!(percent(1.2), 100);
    }

    #[test]
    fn position_moves_on_only_while_playing() {
        let updated = Instant::now();
        let mut track = NowPlaying {
            status: PlaybackStatus::Playing,
            timeline: Some(Timeline {
                position: Duration::from_secs(10),
                updated,
                length: Duration::from_secs(60),
                seekable: true,
            }),
            ..NowPlaying::default()
        };
        let later = updated + Duration::from_secs(5);
        assert_eq!(track.position_at(later), Some(Duration::from_secs(15)));
        // Never past the end, even if the player reports late.
        let much_later = updated + Duration::from_secs(600);
        assert_eq!(track.position_at(much_later), Some(Duration::from_secs(60)));
        track.status = PlaybackStatus::Paused;
        assert_eq!(track.position_at(later), Some(Duration::from_secs(10)));
        track.timeline = None;
        assert_eq!(track.position_at(later), None);
    }

    #[test]
    fn changes_are_told_apart() {
        let track = |title: &str, status| NowPlaying {
            title: title.into(),
            status,
            ..NowPlaying::default()
        };
        let playing = track("A", PlaybackStatus::Playing);
        let paused = track("A", PlaybackStatus::Paused);
        let other = track("B", PlaybackStatus::Playing);
        let untitled = track("", PlaybackStatus::Playing);
        assert_eq!(Change::between(None, Some(&playing)), Change::NewTrack);
        assert_eq!(
            Change::between(Some(&playing), Some(&other)),
            Change::NewTrack
        );
        assert_eq!(
            Change::between(Some(&playing), Some(&paused)),
            Change::PlayPause
        );
        assert_eq!(
            Change::between(Some(&playing), Some(&playing)),
            Change::Other
        );
        assert_eq!(Change::between(Some(&playing), None), Change::Other);
        // A player that has not told the title yet.
        assert_eq!(Change::between(None, Some(&untitled)), Change::Other);
    }

    #[test]
    fn repeat_cycles_through_every_mode() {
        let mut mode = RepeatMode::Off;
        let mut seen = Vec::new();
        for _ in 0..3 {
            mode = mode.next();
            seen.push(mode);
        }
        assert_eq!(seen, [RepeatMode::List, RepeatMode::Track, RepeatMode::Off]);
    }
}
