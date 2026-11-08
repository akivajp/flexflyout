//! What the sources (the player, the volume, the keys, the settings file,
//! the tray, the demo) send to the flyout, on every platform.

use std::time::Instant;

use flexflyout_core::{Config, NowPlaying, Theme, Volume};
use flexflyout_ui::Labels;

/// What the sources send to the window.
#[derive(Debug)]
pub enum Message {
    /// The current track changed or was updated (`None`: nothing playing).
    NowPlaying(Option<NowPlaying>),
    /// The volume changed or was read for the first time.
    Volume(Volume),
    /// The system's colors changed (dark or light), or were read for the
    /// first time.
    SystemTheme(Theme),
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

impl Trigger {
    /// Whether the flyout comes up by itself for this (the volume, a key,
    /// the track), rather than because the user asked for it (the tray
    /// icon, the demo). Only these keep away from full-screen apps.
    // The Linux host keeps away from them through the layer it shows in.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn is_automatic(self) -> bool {
        match self {
            Self::VolumeKey
            | Self::MediaKey
            | Self::VolumeChange
            | Self::TrackChange
            | Self::PlayPause => true,
            Self::Tray | Self::Demo => false,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_what_the_user_asked_for_is_not_automatic() {
        for trigger in [
            Trigger::VolumeKey,
            Trigger::MediaKey,
            Trigger::VolumeChange,
            Trigger::TrackChange,
            Trigger::PlayPause,
        ] {
            assert!(trigger.is_automatic(), "{trigger:?}");
        }
        assert!(!Trigger::Tray.is_automatic());
        assert!(!Trigger::Demo.is_automatic());
    }
}
