//! The Linux host (Wayland, layer-shell): one event loop that takes the
//! compositor's events and the sources' messages, and draws the flyout
//! while it is shown.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use flexflyout_core::config::{Animation, Monitor, NativeOsd};
use flexflyout_core::layout::{self, PixelRect};
use flexflyout_core::{Change, Command, Config};
use flexflyout_ui::{Labels, panel_size};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop::channel::{self, Sender};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use wayland_client::Connection;
use wayland_client::globals::registry_queue_init;

use crate::flyout::{FlyoutState, Received};
use crate::i18n::TrayLabels;
use crate::message::{Message, TrayCommand, Trigger};
use crate::platform::linux::layer::{self, Wayland};
use crate::platform::linux::media::{self, Media};
use crate::platform::linux::volume::{self, VolumeControl};
use crate::platform::linux::{desktop, native_osd, tray};

/// Distance kept from Plasma's volume popup, in logical pixels.
const NATIVE_OSD_GAP: i32 = 12;

/// Time between frames while the flyout fades or slides.
const FRAME: Duration = Duration::from_millis(16);

/// Options of the Linux host.
#[derive(Clone, Debug)]
pub struct Options {
    /// The settings file to watch for changes.
    pub config_file: PathBuf,
    /// The language given on the command line, which overrides the one in
    /// the settings.
    pub lang: Option<String>,
    /// Show the demo, this many times (0 = endless), instead of following
    /// the system.
    pub demo: Option<u32>,
}

/// What the flyout's controls act on (not in the demo).
struct Controls {
    /// The current player.
    media: Media,
    /// The volume of the default output device.
    volume: VolumeControl,
}

/// The state of the event loop.
struct Host {
    wayland: Wayland,
    state: FlyoutState,
    ctx: egui::Context,
    /// `None` in the demo, where the controls change the flyout only.
    controls: Option<Controls>,
    exit: bool,
    /// Why and when the flyout was last asked to show, until it is drawn.
    pending: Option<(crate::message::Trigger, Instant)>,
    /// The settings file, where a dragged place is remembered.
    config_file: PathBuf,
    /// The language given on the command line, for the settings window.
    lang: Option<String>,
}

/// Runs the flyout until it is asked to quit.
pub fn run(
    labels: Labels,
    tray_labels: TrayLabels,
    config: Config,
    options: Options,
) -> anyhow::Result<()> {
    let connection =
        Connection::connect_to_env().context("no Wayland session to show the flyout in")?;
    let (globals, queue) =
        registry_queue_init::<Wayland>(&connection).context("cannot talk to the compositor")?;
    let wayland = Wayland::new(connection.clone(), &globals, queue.handle())?;

    let mut event_loop: EventLoop<Host> = EventLoop::try_new().context("no event loop")?;
    let handle = event_loop.handle();
    handle
        .insert_source(WaylandSource::new(connection, queue), |_, queue, host| {
            queue.dispatch_pending(&mut host.wayland)
        })
        .map_err(|err| anyhow::anyhow!("cannot watch the compositor: {err}"))?;
    let (sender, messages) = channel::channel();
    handle
        .insert_source(messages, |event, _, host: &mut Host| {
            if let channel::Event::Msg(message) = event {
                host.receive(message);
            }
        })
        .map_err(|err| anyhow::anyhow!("cannot receive messages: {err}"))?;

    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
    let fonts = flexflyout_ui::fonts::install(&ctx, crate::platform::linux::fonts::find());
    log::info!("fonts: {fonts:?}");

    // Read by the player's thread, which decides whether to show.
    let show_on_play_pause = Arc::new(AtomicBool::new(config.general.show_on_play_pause));
    let reload = watch_settings(&options, &config, &show_on_play_pause, sender.clone());
    let send = move |message: Message| {
        let _ = sender.send(message);
    };
    let config_file = options.config_file.clone();
    let lang = options.lang.clone();
    let to_flyout = send.clone();
    tray::spawn(tray_labels, move |command| match command {
        TrayCommand::Show => to_flyout(Message::show(Trigger::Tray)),
        TrayCommand::Settings => crate::settings_window::open(&config_file, lang.as_deref()),
        TrayCommand::OpenSettings => desktop::open_in_editor(&config_file),
        TrayCommand::ReloadSettings => reload.now(),
        TrayCommand::ToggleAutostart => {
            if let Err(err) = desktop::set_autostart(!desktop::autostart_enabled(), &config_file) {
                log::error!("{err:#}");
            }
        }
        TrayCommand::Quit => to_flyout(Message::Quit),
    });
    let controls = match options.demo {
        Some(cycles) => {
            crate::demo::spawn(cycles, send);
            None
        }
        None => Some(start_sources(send, show_on_play_pause)),
    };

    let mut host = Host {
        wayland,
        state: FlyoutState::new(labels, config),
        ctx,
        controls,
        exit: false,
        pending: None,
        config_file: options.config_file.clone(),
        lang: options.lang.clone(),
    };
    log::info!("outputs: {:?}", host.wayland.output_names());
    native_osd::start(host.state.config.general.native_osd);
    while !host.exit {
        let timeout = host.tick();
        event_loop
            .dispatch(timeout, &mut host)
            .context("the event loop failed")?;
        host.wayland.flush();
    }
    host.wayland.hide();
    native_osd::restore();
    Ok(())
}

impl Host {
    /// Takes in a message from a source.
    fn receive(&mut self, message: Message) {
        match self.state.receive(message, Instant::now()) {
            Received::Content => {}
            Received::Settings(old) => {
                log::debug!("settings changed from {old:?}");
                let mode = self.state.config.general.native_osd;
                if mode != old.general.native_osd {
                    native_osd::set_mode(mode);
                }
                // A remembered drag comes back as new settings, which
                // place the flyout there now.
                self.wayland.forget_drag();
            }
            Received::Shown { trigger, sent } => self.pending = Some((trigger, sent)),
            Received::Quit => {
                log::info!("closing");
                self.exit = true;
            }
        }
    }

    /// Shows, moves, draws or hides the flyout as the timer says. Returns
    /// how long the event loop may wait for an event before calling again.
    fn tick(&mut self) -> Option<Duration> {
        let now = Instant::now();
        let mut wake = self.state.keep_alive(now, self.wayland.pointer_inside());
        match self.state.timer.next_change(now) {
            Some(after) => {
                self.wayland
                    .show(self.placement(now), self.state.config.display.scale);
                if let Some(output) = self.draw(now) {
                    let delay = output
                        .viewport_output
                        .get(&egui::ViewportId::ROOT)
                        .map(|viewport| viewport.repaint_delay);
                    wake = earliest(wake, delay);
                }
                // While fading, every frame; else when the fade-out starts.
                wake = earliest(wake, Some(after.max(FRAME)));
            }
            None => self.wayland.hide(),
        }
        wake
    }

    /// Where the flyout goes now (it slides while fading).
    fn placement(&self, now: Instant) -> layer::Placement {
        let display = &self.state.config.display;
        let panel = panel_size(&self.state.config.appearance);
        let size = (
            (panel.x * display.scale).round() as u32,
            (panel.y * display.scale).round() as u32,
        );
        let slide = match self.state.config.timing.animation {
            Animation::Slide => {
                let rest = PixelRect::from_origin_size(0, 0, size.0 as i32, size.1 as i32);
                let fraction = layout::slide_fraction(self.state.timer.opacity(now));
                let moved = layout::slide(rest, display.anchor, fraction);
                (moved.left, moved.top)
            }
            Animation::Fade => (0, 0),
        };
        // The compositor picks the monitor for "primary" and "cursor" (KWin
        // shows it on the active one); a number names one.
        let output = match display.monitor {
            Monitor::Number(n) => {
                Some(n as usize - 1).filter(|&i| i < self.wayland.output_names().len())
            }
            Monitor::Primary | Monitor::Cursor => None,
        };
        let area = self.wayland.output_size(output);
        let mut placement = layer::placement(display, size, slide, area);
        placement.output = output;
        // Clear of Plasma's volume popup: moved as the resting place would
        // have to move.
        if self.state.config.general.native_osd == NativeOsd::Avoid
            && native_osd::is_kde()
            && let Some(area) = area
        {
            let rest = layer::rect_on(&layer::placement(display, size, (0, 0), Some(area)), area);
            let clear = layout::avoid(
                rest,
                native_osd::estimate(area),
                PixelRect::from_origin_size(0, 0, area.0, area.1),
                NATIVE_OSD_GAP,
            );
            placement = layer::moved(placement, (clear.left - rest.left, clear.top - rest.top));
        }
        placement
    }

    /// Draws a frame and carries out what the user did with the controls.
    fn draw(&mut self, now: Instant) -> Option<egui::FullOutput> {
        let opacity = self.state.timer.opacity(now);
        let mut clicked = None;
        let view = self.state.view(now);
        let output = self.wayland.draw(&self.ctx, |ui| {
            if opacity <= 0.0 {
                return;
            }
            ui.multiply_opacity(opacity);
            clicked = flexflyout_ui::draw(ui, &view);
        })?;
        if let Some((trigger, sent)) = self.pending.take() {
            log::info!(
                "{trigger:?}: drawing {:.1} ms after the message was sent",
                sent.elapsed().as_secs_f64() * 1000.0
            );
        }
        if let Some(command) = clicked {
            self.carry_out(command, now);
        }
        if let Some(dragged) = self.wayland.take_move_ended() {
            self.finish_move(dragged);
        }
        Some(output)
    }

    /// Carries out a control: the flyout's own part, then the platform's.
    fn carry_out(&mut self, command: Command, now: Instant) {
        let demo = self.controls.is_none();
        let Some(command) = self.state.carry_out(command, now, demo) else {
            return;
        };
        match (command, &self.controls) {
            (Command::StartMove, _) => self.wayland.start_move(),
            (Command::OpenSettings, _) => {
                crate::settings_window::open(&self.config_file, self.lang.as_deref());
            }
            (Command::SetVolume(level), Some(controls)) => controls.volume.set_level(level),
            (Command::ToggleMute, Some(controls)) => controls.volume.toggle_mute(),
            (_, Some(controls)) => controls.media.command(command),
            (_, None) => {}
        }
    }
}

impl Host {
    /// The user dropped the flyout `dragged` logical pixels away: it stays
    /// there for this appearance, and from then on if the settings say so.
    fn finish_move(&mut self, dragged: (i32, i32)) {
        log::info!("moved by {dragged:?}");
        let config = &self.state.config;
        if !config.display.remember_drag || dragged == (0, 0) {
            return;
        }
        let new = Config {
            display: layer::display_moved(&config.display, dragged),
            ..config.clone()
        };
        // The settings file is watched: the new place comes back as new
        // settings, like a change made in the settings window.
        match new.save(&self.config_file, config) {
            Ok(()) => log::info!("remembered the place: {:?}", new.display),
            Err(err) => log::error!("cannot remember the place: {err}"),
        }
    }
}

/// Starts following the players and the volume. Returns what the flyout's
/// controls act on.
fn start_sources(
    send: impl Fn(Message) + Clone + Send + 'static,
    show_on_play_pause: Arc<AtomicBool>,
) -> Controls {
    let to_flyout = send.clone();
    let volume = volume::spawn(move |volume, changed| {
        to_flyout(Message::Volume(volume));
        if changed {
            to_flyout(Message::show(Trigger::VolumeChange));
        }
    });
    let media = media::spawn(move |now_playing, change| {
        send(Message::NowPlaying(now_playing));
        match change {
            Change::NewTrack => send(Message::show(Trigger::TrackChange)),
            Change::PlayPause if show_on_play_pause.load(Ordering::Relaxed) => {
                send(Message::show(Trigger::PlayPause));
            }
            Change::PlayPause | Change::Other => {}
        }
    });
    Controls { media, volume }
}

/// The earlier of two optional waits.
fn earliest(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Sends the settings to the flyout every time the file is saved, with the
/// texts in the new language when that changed. Returns the handle to
/// reload them on demand.
fn watch_settings(
    options: &Options,
    config: &Config,
    show_on_play_pause: &Arc<AtomicBool>,
    sender: Sender<Message>,
) -> crate::settings::Reload {
    let lang = options.lang.clone();
    let mut language = crate::settings::language(lang.as_deref(), config);
    let show_on_play_pause = Arc::clone(show_on_play_pause);
    crate::settings::watch(options.config_file.clone(), move |config| {
        show_on_play_pause.store(config.general.show_on_play_pause, Ordering::Relaxed);
        let new_language = crate::settings::language(lang.as_deref(), &config);
        let labels = (new_language != language)
            .then(|| match crate::i18n::load(new_language.as_deref()) {
                Ok(loader) => {
                    tray::set_labels(crate::i18n::tray_labels(&loader));
                    Some(crate::i18n::labels(&loader))
                }
                Err(err) => {
                    log::error!("{err:#}");
                    None
                }
            })
            .flatten();
        if labels.is_some() {
            language = new_language;
        }
        let _ = sender.send(Message::Settings {
            config: Box::new(config),
            labels,
        });
    })
}
