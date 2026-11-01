//! Platform-independent parts of FlexFlyout.
//!
//! This crate depends neither on an OS API nor on egui, so that every host
//! (Windows with eframe now, KDE and GNOME later) shares the same model and
//! the same timing rules, and so that they can be tested anywhere.

use std::fmt;
use std::sync::Arc;

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
}

impl NowPlaying {
    /// Whether `other` is the same track (title and artist), whatever its
    /// playback state and cover art: a different track is a reason to show
    /// the flyout, a paused one or a late cover art is not.
    pub fn same_track(&self, other: &NowPlaying) -> bool {
        self.title == other.title && self.artist == other.artist
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

/// An action the user asks for from the flyout's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Go to the previous track.
    Previous,
    /// Play when paused, pause when playing.
    TogglePlayPause,
    /// Go to the next track.
    Next,
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
}
