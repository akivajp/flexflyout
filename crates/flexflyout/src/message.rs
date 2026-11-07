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
pub enum Trigger {
    /// A volume key was pressed (on Windows, and on KDE Plasma).
    VolumeKey,
    /// A media key was pressed (on Windows, and on KDE Plasma).
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

impl Message {
    /// A [`Message::Show`] sent now.
    pub fn show(trigger: Trigger) -> Self {
        Self::Show {
            trigger,
            sent: Instant::now(),
        }
    }
}

/// What the user picked from the notification area icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    /// Show the flyout (also a left click on the icon).
    Show,
    /// Open the settings window.
    Settings,
    /// Open the settings file in an editor.
    OpenSettings,
    /// Read the settings file again now.
    ReloadSettings,
    /// Start with Windows (or with the desktop session on Linux), or stop
    /// doing so (a check mark in the menu).
    ToggleAutostart,
    /// End the app.
    Quit,
}
