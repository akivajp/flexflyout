//! The Windows host: an eframe window that draws the flyout and shows and
//! hides itself following [`FlyoutTimer`].
//!
//! Sources (the player through SMTC, the volume, the keys, the settings
//! file, or the demo instead of the first three) run on their own threads.
//! They send a [`Message`], place and show the window through Win32 at once
//! when there is something to show, and ask egui for a repaint; the app then
//! picks the message up in [`eframe::App::logic`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use eframe::egui;
use flexflyout_core::config::{Animation, Display, Monitor, NativeOsd, NewTrack};
use flexflyout_core::layout::{self, MonitorArea, PixelRect};
use flexflyout_core::{Change, Command, Config};
use flexflyout_ui::{Labels, compact_size, panel_size};

use crate::flyout::{self, FlyoutState, Mode, Received};
use crate::i18n::TrayLabels;
use crate::platform::windows::keys::{self, Key};
use crate::platform::windows::media::{self, Media};
use crate::platform::windows::tray::{self, TrayCommand};
use crate::platform::windows::volume::{self, VolumeControl};
use crate::platform::windows::window::{self, FlyoutWindow};
use crate::platform::windows::{autostart, monitor, native_osd, theme};

/// Distance kept from the Windows volume indicator, in pixels at 100%.
const NATIVE_OSD_GAP: f32 = 12.0;

pub use crate::message::{Message, Trigger};

/// Options of the Windows host.
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
    /// Show the window with egui's viewport command instead of Win32.
    pub show_via_egui: bool,
}

/// Opens the flyout window and runs until it is closed.
pub fn run(
    labels: Labels,
    tray_labels: TrayLabels,
    config: Config,
    options: Options,
) -> anyhow::Result<()> {
    // Present through DirectComposition: the default presentation of DirectX
    // 12 cannot show a transparent window. wgpu reads this when it starts.
    // SAFETY: no other thread exists yet (logging and i18n start none).
    unsafe { std::env::set_var("WGPU_DX12_PRESENTATION_SYSTEM", "Visual") };

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("FlexFlyout")
            .with_inner_size(panel_size(&config.appearance))
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_active(false)
            .with_resizable(false),
        // The flyout installs its own fallback (see `FlyoutApp::new`).
        system_font_fallback: false,
        ..Default::default()
    };
    let result = eframe::run_native(
        "FlexFlyout",
        native_options,
        Box::new(move |cc| {
            let app = FlyoutApp::new(cc, labels, tray_labels, config, options)?;
            Ok(Box::new(app))
        }),
    )
    .map_err(|err| anyhow::anyhow!("the window failed: {err}"));
    native_osd::restore();
    tray::remove();
    result
}

/// Where the flyout goes, shared by the sources' threads (which place the
/// window when they show it) and the UI thread (which puts it back when
/// something else moved it).
#[derive(Debug)]
struct Placement {
    /// The placement settings.
    display: Display,
    /// What to do with the Windows volume indicator.
    native_osd: NativeOsd,
    /// How the flyout comes and goes.
    animation: Animation,
    /// Whether the flyout comes up by itself over a full-screen app.
    show_over_fullscreen: bool,
    /// Size of the panel at scale 1.0 (it depends on the parts shown), and
    /// of the small panel of a new track.
    size: egui::Vec2,
    compact_size: egui::Vec2,
    /// What a new track shows, and where the small panel goes.
    new_track: NewTrack,
    /// The panel of the current (or last) appearance.
    mode: Mode,
    /// Where the window was placed last (where it rests, when sliding).
    rect: Option<PixelRect>,
    /// Where the user moved the shown window to: it stays there until it
    /// hides or the settings change.
    moved_to: Option<PixelRect>,
}

/// [`Placement`] behind a lock.
#[derive(Clone, Debug)]
struct SharedPlacement(Arc<Mutex<Placement>>);

impl SharedPlacement {
    fn new(config: &Config) -> Self {
        Self(Arc::new(Mutex::new(Placement {
            display: config.display.clone(),
            native_osd: config.general.native_osd,
            animation: config.timing.animation,
            show_over_fullscreen: config.general.show_over_fullscreen,
            size: panel_size(&config.appearance),
            compact_size: compact_size(&config.appearance),
            new_track: config.new_track.clone(),
            mode: Mode::Full,
            rect: None,
            moved_to: None,
        })))
    }

    fn lock(&self) -> MutexGuard<'_, Placement> {
        // The data stays consistent even if a thread panicked holding it.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a full-screen app on the monitor the panel `mode` would show
    /// on keeps it from coming up by itself (the setting says so).
    fn kept_away_by_fullscreen(&self, mode: Mode) -> bool {
        let placement = self.lock();
        if placement.show_over_fullscreen {
            return false;
        }
        let monitors = monitor::list();
        layout::choose_monitor(
            &monitors,
            placement.display(mode).monitor,
            monitor::cursor(),
        )
        .is_some_and(|i| monitor::fullscreen_app_on(monitors[i].bounds))
    }

    /// Chooses the monitor and the rectangle for the next appearance of the
    /// panel `mode` (the monitor under the pointer can change every time),
    /// and remembers both.
    fn next_rect(&self, mode: Mode) -> Option<PixelRect> {
        let mut placement = self.lock();
        placement.mode = mode;
        // A place the user dragged the whole flyout to.
        if let (Mode::Full, Some(rect)) = (mode, placement.moved_to) {
            placement.rect = Some(rect);
            return Some(rect);
        }
        let display = placement.display(mode);
        let base = placement.size(mode);
        let monitors = monitor::list();
        let rect = layout::choose_monitor(&monitors, display.monitor, monitor::cursor()).map(|i| {
            let rect = layout::place(&display, &monitors[i], (base.x, base.y));
            match placement.native_osd {
                NativeOsd::Avoid => avoid_native_osd(rect, &monitors[i], &monitors),
                NativeOsd::Hide | NativeOsd::Keep => rect,
            }
        });
        if rect.is_none() {
            log::warn!("no monitor to show the flyout on");
        }
        placement.rect = rect;
        rect
    }
}

impl Placement {
    /// The placement settings of the panel `mode`.
    fn display(&self, mode: Mode) -> Display {
        match mode {
            Mode::Full => self.display.clone(),
            Mode::Compact => self.new_track.display(&self.display),
        }
    }

    /// The size of the panel `mode` at scale 1.0.
    fn size(&self, mode: Mode) -> egui::Vec2 {
        match mode {
            Mode::Full => self.size,
            Mode::Compact => self.compact_size,
        }
    }
}

/// Moves `rect` on `monitor` out of the way of the Windows volume indicator:
/// where it was last seen, or else where it appears on the primary monitor.
fn avoid_native_osd(rect: PixelRect, monitor: &MonitorArea, monitors: &[MonitorArea]) -> PixelRect {
    let indicator = native_osd::last_seen().or_else(|| {
        layout::choose_monitor(monitors, Monitor::Primary, None)
            .map(|primary| layout::native_osd_estimate(&monitors[primary]))
    });
    match indicator {
        Some(indicator) => {
            let gap = (NATIVE_OSD_GAP * monitor.scale).round() as i32;
            layout::avoid(rect, indicator, monitor.work, gap)
        }
        None => rect,
    }
}

/// What the flyout's controls act on (not in the demo).
struct Controls {
    /// The current player.
    media: Media,
    /// The volume of the default output device.
    volume: VolumeControl,
}

/// State of the flyout window.
struct FlyoutApp {
    /// What is shown, and the appearance's timer.
    state: FlyoutState,
    messages: Receiver<Message>,
    /// `None` in the demo, where the controls change the flyout only.
    controls: Option<Controls>,
    window: FlyoutWindow,
    placement: SharedPlacement,
    /// The settings file, where a dragged place is remembered.
    config_file: PathBuf,
    /// The language given on the command line, for the settings window.
    lang: Option<String>,
    show_via_egui: bool,
    /// Why and when the flyout was last asked to show, until the first
    /// frame after that has been drawn (then the delay is logged).
    pending: Option<(Trigger, Instant)>,
    /// Whether a fully transparent frame has been drawn since the timer
    /// ended. The window is hidden only then, so that showing it again does
    /// not briefly bring back the last picture.
    cleared: bool,
}

impl FlyoutApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        labels: Labels,
        tray_labels: TrayLabels,
        config: Config,
        options: Options,
    ) -> anyhow::Result<Self> {
        // egui asks winit for a drop shadow on undecorated windows, which
        // winit makes by keeping a 1-pixel non-client strip at the top: a
        // light line above the panel. A transparent flyout has no use for a
        // rectangular shadow anyway.
        if let Some(winit_window) = cc.winit_window() {
            use winit::platform::windows::WindowExtWindows as _;
            winit_window.set_undecorated_shadow(false);
        }
        let window = FlyoutWindow::new(cc)?;
        // Also applies the shadow change above (SWP_FRAMECHANGED).
        window.apply_flyout_style()?;
        // The Windows UI fonts, then any other the system's fallback finds
        // for a title in a script they lack (Korean, Chinese, Thai…).
        let fonts = flexflyout_ui::fonts::install_with_fallback(
            &cc.egui_ctx,
            windows_fonts(),
            egui_system_font_provider::SystemFontProvider::new(),
        );
        log::info!("fonts: {fonts:?}");
        // The panel is drawn in points at scale 1.0; the zoom factor makes
        // it larger or smaller, and the window is sized to match on every
        // appearance. The flyout never has the focus, but a zoom shortcut
        // must not change the size behind the settings' back either.
        cc.egui_ctx.set_zoom_factor(config.display.scale);
        cc.egui_ctx
            .options_mut(|options| options.zoom_with_keyboard = false);
        for (i, monitor) in monitor::list().iter().enumerate() {
            log::info!("monitor {}: {monitor:?}", i + 1);
        }

        let (sender, messages) = channel();
        let ctx = cc.egui_ctx.clone();
        let raw = window.raw();
        let placement = SharedPlacement::new(&config);
        native_osd::start(config.general.native_osd);
        let show_via_egui = options.show_via_egui;
        let send = {
            let placement = placement.clone();
            move |message: Message| send(&sender, &ctx, raw, &placement, message, show_via_egui)
        };
        // The light or dark mode, for the theme that follows it.
        let to_window = send.clone();
        theme::spawn(move |theme| to_window(Message::SystemTheme(theme)));
        // Read by the player's thread, which decides whether to show.
        let show_on_play_pause = Arc::new(AtomicBool::new(config.general.show_on_play_pause));
        let reload = watch_settings(&options, &config, &show_on_play_pause, send.clone());
        let config_file = options.config_file.clone();
        let lang = options.lang.clone();
        let to_window = send.clone();
        tray::spawn(tray_labels, move |command| match command {
            TrayCommand::Show => to_window(Message::show(Trigger::Tray)),
            TrayCommand::OpenSettings => tray::open_in_editor(&config_file),
            TrayCommand::ReloadSettings => reload.now(),
            TrayCommand::Settings => crate::settings_window::open(&config_file, lang.as_deref()),
            TrayCommand::ToggleAutostart => {
                if let Err(err) = autostart::set(!autostart::is_enabled(), &config_file) {
                    log::error!("{err:#}");
                }
            }
            TrayCommand::Quit => to_window(Message::Quit),
        });
        let controls = match options.demo {
            Some(cycles) => {
                crate::demo::spawn(cycles, send);
                None
            }
            None => Some(start_sources(
                send,
                show_on_play_pause,
                config.general.player().map(str::to_owned),
            )),
        };

        Ok(Self {
            state: FlyoutState::new(labels, config),
            messages,
            controls,
            window,
            placement,
            config_file: options.config_file.clone(),
            lang: options.lang.clone(),
            show_via_egui: options.show_via_egui,
            pending: None,
            cleared: false,
        })
    }
}

impl FlyoutApp {
    /// Whether the mouse pointer is on the shown flyout. Read from Windows
    /// rather than from egui, which keeps the last position it saw when
    /// the window was hidden under the pointer.
    fn pointer_over(&self) -> bool {
        self.window.is_visible()
            && monitor::cursor()
                .zip(self.window.rect())
                .is_some_and(|(point, rect)| rect.contains(point))
    }

    /// Puts a shown window back where it was placed. Moving the window to a
    /// monitor with another display scaling makes Windows and winit resize
    /// it for the new scaling, and they may move it as well; the rectangle
    /// was already computed for that monitor, so placing it again settles.
    /// Also moves a sliding flyout along as it fades in or out.
    fn keep_in_place(&self, now: Instant) {
        // Not while the user moves it, of course.
        if !self.window.is_visible() || window::is_moving() {
            return;
        }
        let wanted = {
            let placement = self.placement.lock();
            let Some(rect) = placement.rect else {
                return;
            };
            match placement.animation {
                Animation::Slide => {
                    let fraction = layout::slide_fraction(self.state.timer.opacity(now));
                    let anchor = placement.display(placement.mode).anchor;
                    layout::slide(rect, anchor, fraction)
                }
                Animation::Fade => rect,
            }
        };
        let Some(actual) = self.window.rect() else {
            return;
        };
        if actual != wanted {
            // A position that differs only because the asynchronous move is
            // still on its way, or because the flyout slides, is not worth
            // a line; a different size is.
            if (actual.width(), actual.height()) != (wanted.width(), wanted.height()) {
                log::info!("the window was resized to {actual:?}; putting it back to {wanted:?}");
            }
            self.window.set_rect(wanted);
        }
    }
}

/// Starts following the player, the volume and the keys. Returns what the
/// flyout's controls act on.
fn start_sources(
    send: impl Fn(Message) + Clone + Send + Sync + 'static,
    show_on_play_pause: Arc<AtomicBool>,
    player: Option<String>,
) -> Controls {
    let to_window = send.clone();
    let volume = volume::spawn(move |volume, changed| {
        to_window(Message::Volume(volume));
        if changed {
            to_window(Message::show(Trigger::VolumeChange));
        }
    });
    let to_window = send.clone();
    keys::spawn(move |key| {
        to_window(Message::show(match key {
            Key::Volume => Trigger::VolumeKey,
            Key::Media => Trigger::MediaKey,
        }));
    });
    let to_window = send.clone();
    let players = move |players, current| to_window(Message::Players { players, current });
    let media = media::spawn(
        player,
        move |now_playing, change| {
            tray::set_now_playing(now_playing.as_ref());
            send(Message::NowPlaying(now_playing));
            match change {
                Change::NewTrack => send(Message::show(Trigger::TrackChange)),
                Change::PlayPause if show_on_play_pause.load(Ordering::Relaxed) => {
                    send(Message::show(Trigger::PlayPause));
                }
                Change::PlayPause | Change::Other => {}
            }
        },
        players,
    );
    Controls { media, volume }
}

/// Sends the settings to the window every time the file is saved, with the
/// texts in the new language when that changed. Returns the handle to
/// reload them on demand.
fn watch_settings(
    options: &Options,
    config: &Config,
    show_on_play_pause: &Arc<AtomicBool>,
    send: impl Fn(Message) + Send + 'static,
) -> crate::settings::Reload {
    let lang = options.lang.clone();
    let mut language = crate::settings::language(lang.as_deref(), config);
    let show_on_play_pause = Arc::clone(show_on_play_pause);
    crate::settings::watch(options.config_file.clone(), move |config| {
        show_on_play_pause.store(config.general.show_on_play_pause, Ordering::Relaxed);
        crate::update::set_enabled(config.general.check_updates);
        let new_language = crate::settings::language(lang.as_deref(), &config);
        let labels = if new_language != language {
            // A bad language code keeps the texts in use.
            match crate::i18n::load(new_language.as_deref()) {
                Ok(loader) => {
                    tray::set_labels(crate::i18n::tray_labels(&loader));
                    Some(Box::new(crate::i18n::labels(&loader)))
                }
                Err(err) => {
                    log::error!("{err:#}");
                    None
                }
            }
        } else {
            None
        };
        if labels.is_some() {
            language = new_language;
        }
        send(Message::Settings {
            config: Box::new(config),
            labels,
        });
    })
}

/// Hands a message to the window from a source's thread.
///
/// The message goes first, so that the window has it by the time it is
/// shown and painted; then the window is placed and shown, and egui is
/// woken up.
fn send(
    sender: &Sender<Message>,
    ctx: &egui::Context,
    raw: isize,
    placement: &SharedPlacement,
    message: Message,
    show_via_egui: bool,
) {
    let mut message = message;
    let show = match &mut message {
        Message::Show { trigger, mode, .. } => {
            // The panel for it, decided here (where the window is placed)
            // and handed on with the message, so that both agree.
            let visible = window::is_visible(raw);
            let (shown, new_track) = {
                let placement = placement.lock();
                (visible.then_some(placement.mode), placement.new_track.show)
            };
            let Some(chosen) = mode.or_else(|| flyout::mode_for(*trigger, new_track, shown)) else {
                return;
            };
            // Not over a full-screen game or video, unless the user asked
            // for it, or it shows already (pinned, say).
            if trigger.is_automatic() && !visible && placement.kept_away_by_fullscreen(chosen) {
                log::info!("{trigger:?}: not shown over the full-screen app");
                return;
            }
            *mode = Some(chosen);
            placement.lock().mode = chosen;
            true
        }
        Message::Settings { config, .. } => {
            let mut placement = placement.lock();
            let preview = flyout::preview_mode(&placement.new_track, &config.new_track);
            placement.new_track = config.new_track.clone();
            placement.compact_size = compact_size(&config.appearance);
            placement.mode = preview;
            placement.display = config.display.clone();
            placement.native_osd = config.general.native_osd;
            placement.animation = config.timing.animation;
            placement.show_over_fullscreen = config.general.show_over_fullscreen;
            placement.size = panel_size(&config.appearance);
            // New settings place the flyout anew (a remembered drag comes
            // back here as new settings too).
            placement.moved_to = None;
            true
        }
        Message::NowPlaying(_)
        | Message::Volume(_)
        | Message::SystemTheme(_)
        | Message::Players { .. }
        | Message::Quit => false,
    };
    if sender.send(message).is_err() {
        return; // The window is closing.
    }
    let mode = placement.lock().mode;
    if show && let Some(rect) = placement.next_rect(mode) {
        if window::is_visible(raw) {
            // Shown already: the UI thread puts it where it belongs now
            // (`keep_in_place`), also in the middle of a slide.
        } else {
            // A sliding flyout starts out of place.
            let (anchor, animation) = {
                let placement = placement.lock();
                (placement.display(mode).anchor, placement.animation)
            };
            let start = match animation {
                Animation::Slide => layout::slide(rect, anchor, 1.0),
                Animation::Fade => rect,
            };
            // With --show-via-egui, egui shows the window once it has the
            // message; it is only moved here.
            window::place_from_any_thread(raw, start, !show_via_egui);
        }
    }
    ctx.request_repaint();
}

impl eframe::App for FlyoutApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Fully transparent: only the rounded panel is visible.
        [0.0; 4]
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        while let Ok(message) = self.messages.try_recv() {
            match self.state.receive(message, now) {
                Received::Content => {}
                Received::Shown { trigger, sent } => {
                    self.pending = Some((trigger, sent));
                    self.cleared = false;
                    if self.show_via_egui {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    }
                }
                Received::Settings(old) => {
                    let config = &self.state.config;
                    if config.general.native_osd != old.general.native_osd {
                        native_osd::set_mode(config.general.native_osd);
                    }
                    if config.general.player != old.general.player
                        && let Some(controls) = &self.controls
                    {
                        controls
                            .media
                            .choose(config.general.player().map(str::to_owned));
                    }
                    self.cleared = false;
                    if self.show_via_egui {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    }
                }
                Received::Quit => {
                    log::info!("closing");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        // The panel is drawn in points at scale 1.0, and each panel has its
        // own size setting.
        let scale = self.state.scale();
        if ctx.zoom_factor() != scale {
            ctx.set_zoom_factor(scale);
        }
        if window::take_move_ended() {
            self.finish_move();
        }
        self.keep_in_place(now);
        let pointer_over = self.pointer_over();
        if let Some(after) = self.state.keep_alive(now, pointer_over) {
            ctx.request_repaint_after(after);
        }

        match self.state.timer.next_change(now) {
            // Fading: every frame. Fully shown: when the fade-out starts.
            Some(after) => ctx.request_repaint_after(after),
            // Finished: hide once a blank frame is on the screen.
            None if self.window.is_visible() => {
                if self.cleared {
                    // A place the user dragged it to lasts one appearance.
                    self.placement.lock().moved_to = None;
                    if self.show_via_egui {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    } else {
                        self.window.hide();
                    }
                    flexflyout_ui::reset(ctx);
                } else {
                    ctx.request_repaint();
                }
            }
            None => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let opacity = self.state.timer.opacity(now);
        if opacity <= 0.0 {
            // Draw nothing, then let `logic` hide the window. Ask for that
            // one more frame only once: eframe keeps calling this while the
            // window is hidden, and asking every time would keep it drawing
            // ten times a second.
            if !self.cleared {
                self.cleared = true;
                ui.ctx().request_repaint();
            }
            return;
        }
        if let Some((trigger, sent)) = self.pending.take() {
            log::info!(
                "{trigger:?}: drawing {:.1} ms after the message was sent; focus: \"{}\"",
                sent.elapsed().as_secs_f64() * 1000.0,
                window::foreground_window_title()
            );
        }
        ui.multiply_opacity(opacity);
        let view = self.state.view(now);
        let command = match self.state.mode {
            Mode::Full => flexflyout_ui::draw(ui, &view),
            Mode::Compact => flexflyout_ui::draw_compact(ui, &view),
        };
        if let Some(command) = command {
            self.carry_out(command, now, ui.ctx());
        }
    }
}

impl FlyoutApp {
    /// The user dropped the window somewhere else: it stays there for this
    /// appearance, and from then on if the settings say so.
    fn finish_move(&mut self) {
        let Some(rect) = self.window.rect() else {
            return;
        };
        {
            let mut placement = self.placement.lock();
            placement.rect = Some(rect);
            placement.moved_to = Some(rect);
        }
        log::info!(
            "moved to {rect:?}; focus: \"{}\"",
            window::foreground_window_title()
        );
        if !self.state.config.display.remember_drag {
            return;
        }
        let display = layout::display_for(
            rect,
            &self.state.config.display,
            &monitor::list(),
            monitor::cursor(),
        );
        let config = Config {
            display,
            ..self.state.config.clone()
        };
        // The settings file is watched: the new place comes back as new
        // settings, like a change made in the settings window.
        match config.save(&self.config_file, &self.state.config) {
            Ok(()) => log::info!("remembered the place: {:?}", config.display),
            Err(err) => log::error!("cannot remember the place: {err}"),
        }
    }

    /// Carries out what the user did with the flyout's controls.
    fn carry_out(&mut self, command: Command, now: Instant, ctx: &egui::Context) {
        let demo = self.controls.is_none();
        let Some(command) = self.state.carry_out(command, now, demo) else {
            // Closing needs a frame to fade out on.
            ctx.request_repaint();
            return;
        };
        match (command, &self.controls) {
            // Windows moves the window from here, until the button is
            // released (see `finish_move`).
            (Command::StartMove, _) => self.window.start_move(),
            (Command::OpenSettings, _) => {
                crate::settings_window::open(&self.config_file, self.lang.as_deref());
            }
            (Command::ChoosePlayer(choice), _) => {
                let before = self.state.choose_player(choice);
                let chosen = self.state.config.general.player().map(str::to_owned);
                if let Some(controls) = &self.controls {
                    controls.media.choose(chosen);
                }
                // Kept for the next start; the reloaded settings change
                // nothing more.
                if let Err(err) = self.state.config.save(&self.config_file, &before) {
                    log::error!("cannot save the player chosen: {err}");
                }
            }
            // The whole flyout, where it goes, in place of the small panel.
            (Command::Expand, _) => {
                if let Some(rect) = self.placement.next_rect(Mode::Full) {
                    self.window.set_rect(rect);
                }
            }
            (Command::SetVolume(level), Some(controls)) => controls.volume.set_level(level),
            (Command::ToggleMute, Some(controls)) => controls.volume.toggle_mute(),
            (_, Some(controls)) => controls.media.command(command),
            (_, None) => {}
        }
    }
}

/// Uses the Windows UI fonts so that the flyout looks native: Segoe UI for
/// Latin text, then a Japanese font (titles can be Japanese whatever the GUI
/// language is). egui's own fonts are not built in (their licence is not an
/// OSI one, which code signing for open source asks about). Returns the
/// fonts used and their baseline offsets, for the log.
pub(crate) fn setup_fonts(ctx: &egui::Context) -> Vec<(String, f32)> {
    flexflyout_ui::fonts::install(ctx, windows_fonts())
}

/// The Windows UI fonts (see [`setup_fonts`]), read whole. Only one per
/// script is read up front; fonts for other scripts are found when needed.
fn windows_fonts() -> Vec<(String, Vec<u8>)> {
    // In order of preference: the first file found in each group is used.
    let groups: [&[&str]; 2] = [
        &[
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
            r"C:\Windows\Fonts\tahoma.ttf",
        ],
        &[
            r"C:\Windows\Fonts\YuGothM.ttc",
            r"C:\Windows\Fonts\meiryo.ttc",
            r"C:\Windows\Fonts\msgothic.ttc",
        ],
    ];
    groups
        .iter()
        .filter_map(|group| {
            group.iter().find_map(|path| {
                std::fs::read(path)
                    .ok()
                    .map(|bytes| (path.to_string(), bytes))
            })
        })
        .collect()
}
