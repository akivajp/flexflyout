//! What the flyout shows and how it reacts, shared by the hosts: the
//! current track and volume, the timer of an appearance, pinning, and the
//! part of the controls that the flyout handles itself. The hosts own the
//! window, the platform's player and volume, and drawing.

use std::time::{Duration, Instant};

use flexflyout_core::{Command, Config, FlyoutTimer, NowPlaying, PlaybackStatus, Volume};
use flexflyout_ui::{FlyoutView, Labels};

use crate::message::{Message, Trigger};

/// How often to check whether the mouse pointer still rests on the shown
/// flyout (which keeps it from fading out).
pub const HOVER_CHECK: Duration = Duration::from_millis(200);

/// How often the shown flyout is drawn again while a track plays, to move
/// its position bar on.
pub const POSITION_REFRESH: Duration = Duration::from_millis(250);

/// The flyout's state.
pub struct FlyoutState {
    pub labels: Labels,
    pub config: Config,
    pub now_playing: Option<NowPlaying>,
    pub volume: Option<Volume>,
    pub timer: FlyoutTimer,
    /// Whether the user pinned the flyout: it then stays until unpinned.
    pub pinned: bool,
    /// When the user last moved the position with the flyout's bar.
    seeked: Option<Instant>,
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
            seeked: None,
        }
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
            Message::Show { trigger, sent } => {
                self.timer.trigger(now);
                Received::Shown { trigger, sent }
            }
            Message::Settings { config, labels } => {
                self.timer.set_timing(config.timing.timing());
                let old = std::mem::replace(&mut self.config, *config);
                if let Some(labels) = labels {
                    self.labels = labels;
                }
                // Show the flyout with the new settings.
                self.timer.trigger(now);
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
            labels: &self.labels,
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
            Command::StartMove | Command::OpenSettings => Some(command),
            Command::SetVolume(level) if demo => {
                if let Some(volume) = &mut self.volume {
                    volume.level = level;
                }
                None
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
