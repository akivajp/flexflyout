//! What the flyout shows and how it reacts, shared by the hosts: the
//! current track and volume, the timer of an appearance, pinning, and the
//! part of the controls that the flyout handles itself. The hosts own the
//! window, the platform's player and volume, and drawing.

use std::time::{Duration, Instant};

use flexflyout_core::config::{Display, NewTrack};
use flexflyout_core::{
    Command, Config, FlyoutTimer, NewTrackShow, NowPlaying, PlaybackStatus, Player, Theme, Volume,
};
use flexflyout_ui::{FlyoutView, Labels};

use crate::message::{Message, Trigger};

/// How often to check whether the mouse pointer still rests on the shown
/// flyout (which keeps it from fading out).
pub const HOVER_CHECK: Duration = Duration::from_millis(200);

/// How often the shown flyout is drawn again while a track plays, to move
/// its position bar on.
pub const POSITION_REFRESH: Duration = Duration::from_millis(250);

/// Which panel an appearance shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// The whole flyout.
    #[default]
    Full,
    /// The small panel of a new track.
    Compact,
}

/// The panel to show for `trigger`, when `shown` is the one showing now
/// (`None`: hidden), or `None` to show nothing. A new track shows what the
/// settings say, unless the whole flyout is up already (it then shows the
/// new track itself); everything else shows the whole flyout.
pub fn mode_for(trigger: Trigger, new_track: NewTrackShow, shown: Option<Mode>) -> Option<Mode> {
    if trigger != Trigger::TrackChange || shown == Some(Mode::Full) {
        return Some(Mode::Full);
    }
    match new_track {
        NewTrackShow::Compact => Some(Mode::Compact),
        NewTrackShow::Flyout => Some(Mode::Full),
        // A small panel already up for an earlier track shows this one.
        NewTrackShow::Nothing => shown,
    }
}

/// The panel that previews new settings: the small one when its settings
/// changed (and it is in use), else the whole flyout.
pub fn preview_mode(old: &NewTrack, new: &NewTrack) -> Mode {
    if old != new && new.show == NewTrackShow::Compact {
        Mode::Compact
    } else {
        Mode::Full
    }
}

/// The placement settings of a panel: the flyout's, or the small panel's.
pub fn display_for(config: &Config, mode: Mode) -> Display {
    match mode {
        Mode::Full => config.display.clone(),
        Mode::Compact => config.new_track.display(&config.display),
    }
}

/// The flyout's state.
pub struct FlyoutState {
    pub labels: Labels,
    pub config: Config,
    pub now_playing: Option<NowPlaying>,
    pub volume: Option<Volume>,
    pub timer: FlyoutTimer,
    /// Whether the user pinned the flyout: it then stays until unpinned.
    pub pinned: bool,
    /// The system's colors, dark or light, for the theme that follows them
    /// (dark until the system says otherwise).
    system_theme: Theme,
    /// When the user last moved the position with the flyout's bar.
    seeked: Option<Instant>,
    /// The panel of the current (or last) appearance.
    pub mode: Mode,
    /// The players there are, and the one whose track is shown.
    pub players: Vec<Player>,
    pub current_player: Option<String>,
}

/// What a message changed, for the host to act on.
#[derive(Debug)]
pub enum Received {
    /// Only what is shown.
    Content,
    /// The flyout is to be shown.
    Shown {
        /// Why.
        trigger: Trigger,
        /// When the message was sent.
        sent: Instant,
    },
    /// New settings are in use (and the flyout is shown with them); these
    /// were the settings before.
    Settings(Box<Config>),
    /// The app is to close.
    Quit,
}

impl FlyoutState {
    /// A hidden flyout with nothing to show yet.
    pub fn new(labels: Labels, config: Config) -> Self {
        Self {
            labels,
            timer: FlyoutTimer::new(config.timing.timing()),
            config,
            now_playing: None,
            volume: None,
            pinned: false,
            system_theme: Theme::Dark,
            seeked: None,
            mode: Mode::Full,
            players: Vec::new(),
            current_player: None,
        }
    }

    /// The panel showing at `now`, if any.
    pub fn shown_mode(&self, now: Instant) -> Option<Mode> {
        self.timer.is_active(now).then_some(self.mode)
    }

    /// The size setting of the current panel (egui's zoom factor).
    pub fn scale(&self) -> f32 {
        display_for(&self.config, self.mode).scale
    }

    /// Shows `mode`, with its timing (or keeps it shown longer).
    fn show(&mut self, mode: Mode, now: Instant) {
        self.mode = mode;
        let timing = match mode {
            Mode::Full => self.config.timing.timing(),
            Mode::Compact => self.config.new_track.timing(&self.config.timing),
        };
        self.timer.set_timing(timing);
        self.timer.trigger(now);
    }

    /// Follows the `choice`-th player of the list shown (`None`: whichever
    /// plays). Returns the settings before, for the host to save the
    /// change against.
    pub fn choose_player(&mut self, choice: Option<usize>) -> Config {
        let before = self.config.clone();
        self.config.general.player = choice
            .and_then(|i| self.players.get(i))
            .map_or_else(|| "auto".to_owned(), |player| player.id.clone());
        before
    }

    /// Takes in a message from a source.
    pub fn receive(&mut self, message: Message, now: Instant) -> Received {
        match message {
            Message::NowPlaying(now_playing) => {
                self.set_now_playing(now_playing);
                Received::Content
            }
            Message::Volume(volume) => {
                self.volume = Some(volume);
                Received::Content
            }
            Message::SystemTheme(theme) => {
                self.system_theme = theme;
                Received::Content
            }
            Message::Players { players, current } => {
                self.players = players;
                self.current_player = current;
                Received::Content
            }
            Message::Show {
                trigger,
                sent,
                mode,
            } => {
                let shown = self.shown_mode(now);
                match mode.or_else(|| mode_for(trigger, self.config.new_track.show, shown)) {
                    Some(mode) => {
                        self.show(mode, now);
                        Received::Shown { trigger, sent }
                    }
                    None => Received::Content,
                }
            }
            Message::Settings { config, labels } => {
                let old = std::mem::replace(&mut self.config, *config);
                if let Some(labels) = labels {
                    self.labels = *labels;
                }
                // Show the panel the change is about with the new settings.
                let mode = preview_mode(&old.new_track, &self.config.new_track);
                self.show(mode, now);
                Received::Settings(Box::new(old))
            }
            Message::Quit => Received::Quit,
        }
    }

    /// Keeps a pinned flyout, or one under the mouse pointer (to reach its
    /// buttons), from fading out. Returns how soon to look again, if the
    /// flyout is shown: to check the pointer, or to move the position on.
    pub fn keep_alive(&mut self, now: Instant, pointer_over: bool) -> Option<Duration> {
        if !self.timer.is_active(now) {
            return None;
        }
        let mut after: Option<Duration> = None;
        if self.pinned {
            self.timer.trigger(now);
        } else if pointer_over {
            self.timer.trigger(now);
            after = Some(HOVER_CHECK);
        }
        if self.now_playing.as_ref().is_some_and(|track| {
            track.status == PlaybackStatus::Playing && track.timeline.is_some()
        }) {
            after = Some(after.map_or(POSITION_REFRESH, |a| a.min(POSITION_REFRESH)));
        }
        after
    }

    /// What to draw at `now`.
    pub fn view(&self, now: Instant) -> FlyoutView<'_> {
        FlyoutView {
            now_playing: self.now_playing.as_ref(),
            volume: self.volume,
            pinned: self.pinned,
            now,
            background_opacity: self.config.display.opacity,
            appearance: &self.config.appearance,
            theme: self.config.appearance.theme.resolved(self.system_theme),
            labels: &self.labels,
            players: &self.players,
            player_choice: self.config.general.player(),
            current_player: self.current_player.as_deref(),
        }
    }

    /// Handles the part of a control that belongs to the flyout itself
    /// (pinning, closing, showing a seek at once). Returns the command for
    /// the host to carry out (the player, the volume, moving the window),
    /// if any. In the demo (`demo`), which has neither a player nor a
    /// device, the volume controls change what is shown instead.
    pub fn carry_out(&mut self, command: Command, now: Instant, demo: bool) -> Option<Command> {
        log::info!("control: {command:?}");
        match command {
            Command::Seek(position) => {
                if let Some(timeline) = self.now_playing.as_mut().and_then(|t| t.timeline.as_mut())
                {
                    // Show the new position at once rather than when the
                    // player reports it.
                    timeline.position = position;
                    timeline.updated = now;
                    self.seeked = Some(now);
                }
                (!demo).then_some(command)
            }
            Command::Close => {
                self.pinned = false;
                self.timer.cancel();
                None
            }
            Command::Pin(pinned) => {
                self.pinned = pinned;
                // Unpinned: the usual display time from now on.
                self.timer.trigger(now);
                None
            }
            Command::StartMove | Command::OpenSettings | Command::ChoosePlayer(_) => Some(command),
            Command::Expand => {
                self.show(Mode::Full, now);
                Some(command)
            }
            Command::SetVolume(level) => {
                // Show the new level at once rather than when the system
                // reports it, so that the next step of a quickly turned
                // wheel starts from there.
                if let Some(volume) = &mut self.volume {
                    volume.level = level;
                }
                (!demo).then_some(command)
            }
            Command::ToggleMute if demo => {
                if let Some(volume) = &mut self.volume {
                    volume.muted = !volume.muted;
                }
                None
            }
            _ => (!demo).then_some(command),
        }
    }

    /// Uses an update of the current track from the player.
    fn set_now_playing(&mut self, mut new: Option<NowPlaying>) {
        // A player that has not caught up with a seek yet (Media Player
        // reports the position only every few seconds) still tells where
        // it was before: keep the position the flyout moved to.
        if let (Some(seeked), Some(old), Some(new)) = (self.seeked, &self.now_playing, &mut new)
            && old.same_track(new)
            && new
                .timeline
                .is_some_and(|timeline| timeline.updated < seeked)
        {
            new.timeline = old.timeline;
        }
        self.now_playing = new;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_track_shows_what_the_settings_say() {
        use NewTrackShow::{Compact, Flyout, Nothing};
        let track = Trigger::TrackChange;
        // Hidden: by the setting.
        assert_eq!(mode_for(track, Compact, None), Some(Mode::Compact));
        assert_eq!(mode_for(track, Flyout, None), Some(Mode::Full));
        assert_eq!(mode_for(track, Nothing, None), None);
        // The whole flyout up already: it shows the new track.
        for show in [Compact, Flyout, Nothing] {
            assert_eq!(mode_for(track, show, Some(Mode::Full)), Some(Mode::Full));
        }
        // The small panel up: it shows the next one too.
        assert_eq!(
            mode_for(track, Nothing, Some(Mode::Compact)),
            Some(Mode::Compact)
        );
        // Everything else shows the whole flyout, also over the small panel.
        for trigger in [Trigger::VolumeKey, Trigger::VolumeChange, Trigger::Tray] {
            assert_eq!(
                mode_for(trigger, Compact, Some(Mode::Compact)),
                Some(Mode::Full)
            );
        }
    }

    #[test]
    fn settings_preview_the_panel_they_change() {
        let old = NewTrack::default();
        assert_eq!(preview_mode(&old, &old), Mode::Full);
        let longer = NewTrack {
            hold_ms: 5000,
            ..NewTrack::default()
        };
        assert_eq!(preview_mode(&old, &longer), Mode::Compact);
        let off = NewTrack {
            show: NewTrackShow::Nothing,
            ..NewTrack::default()
        };
        assert_eq!(preview_mode(&old, &off), Mode::Full);
    }
}
