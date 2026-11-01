//! What the sources (the player, the volume, the keys, the settings file,
//! the tray, the demo) send to the flyout, on every platform.

use std::time::Instant;

use flexflyout_core::{Config, NowPlaying, Volume};
use flexflyout_ui::Labels;

/// What the sources send to the window.
#[derive(Debug)]
pub enum Message {
    /// The current track changed or was updated (`None`: nothing playing).
    NowPlaying(Option<NowPlaying>),
    /// The volume changed or was read for the first time.
    Volume(Volume),
    /// Show the flyout with what it has.
    Show {
        /// Why.
        trigger: Trigger,
        /// When the message was sent, to log how long showing took.
        sent: Instant,
    },
    /// Use new settings (the file was saved). The flyout is shown with
    /// them, so that a change of the placement or the look can be seen.
    Settings {
        /// The settings read from the file.
        config: Box<Config>,
        /// The texts again, when the language changed.
        labels: Option<Labels>,
    },
    /// Close the app (Quit in the tray menu, or the end of the demo).
    Quit,
}

/// Why the flyout is shown (for the log).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// The Linux host follows only the demo so far.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub enum Trigger {
    /// A volume key was pressed.
    // Only the Windows host watches the keys.
    #[cfg_attr(not(windows), allow(dead_code))]
    VolumeKey,
    /// A media key was pressed.
    #[cfg_attr(not(windows), allow(dead_code))]
    MediaKey,
    /// The volume changed, or another output device became the default.
    VolumeChange,
    /// A different track started.
    TrackChange,
    /// The player started or paused.
    PlayPause,
    /// The tray icon was clicked.
    Tray,
    /// The demo.
    Demo,
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
impl Message {
    /// A [`Message::Show`] sent now.
    pub fn show(trigger: Trigger) -> Self {
        Self::Show {
            trigger,
            sent: Instant::now(),
        }
    }
}
