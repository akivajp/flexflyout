//! The Windows host: an eframe window that draws the flyout and shows and
//! hides itself following [`FlyoutTimer`].
//!
//! Sources (the player through SMTC, the volume, the keys, the settings
//! file, or the demo instead of the first three) run on their own threads.
//! They send a [`Message`], place and show the window through Win32 at once
//! when there is something to show, and ask egui for a repaint; the app then
//! picks the message up in [`eframe::App::logic`].

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use eframe::egui;
use flexflyout_core::config::Display;
use flexflyout_core::layout::{self, PixelRect};
use flexflyout_core::{Config, FlyoutTimer, NowPlaying, Volume};
use flexflyout_ui::{BASE_SIZE, FlyoutView, Labels};

use crate::i18n::TrayLabels;
use crate::platform::windows::keys::{self, Key};
use crate::platform::windows::media::{self, Media};
use crate::platform::windows::tray::{self, TrayCommand};
use crate::platform::windows::window::{self, FlyoutWindow};
use crate::platform::windows::{monitor, volume};

/// How often to check whether the mouse pointer still rests on the shown
/// flyout (which keeps it from fading out).
const HOVER_CHECK: Duration = Duration::from_millis(200);

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
    /// A volume key was pressed.
    VolumeKey,
    /// A media key was pressed.
    MediaKey,
    /// The volume changed.
    VolumeChange,
    /// A different track started.
    TrackChange,
    /// The tray icon was clicked.
    Tray,
    /// The demo.
    Demo,
}

impl Message {
    /// A [`Message::Show`] sent now.
    fn show(trigger: Trigger) -> Self {
        Self::Show {
            trigger,
            sent: Instant::now(),
        }
    }
}

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
            .with_inner_size(BASE_SIZE)
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_taskbar(false)
            .with_active(false)
            .with_resizable(false),
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
    /// Where the window was placed last.
    rect: Option<PixelRect>,
}

/// [`Placement`] behind a lock.
#[derive(Clone, Debug)]
struct SharedPlacement(Arc<Mutex<Placement>>);

impl SharedPlacement {
    fn new(display: Display) -> Self {
        Self(Arc::new(Mutex::new(Placement {
            display,
            rect: None,
        })))
    }

    fn lock(&self) -> MutexGuard<'_, Placement> {
        // The data stays consistent even if a thread panicked holding it.
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Chooses the monitor and the rectangle for the next appearance (the
    /// monitor under the pointer can change every time), and remembers it.
    fn next_rect(&self) -> Option<PixelRect> {
        let mut placement = self.lock();
        let monitors = monitor::list();
        let rect = layout::choose_monitor(&monitors, placement.display.monitor, monitor::cursor())
            .map(|i| layout::place(&placement.display, &monitors[i], (BASE_SIZE.x, BASE_SIZE.y)));
        if rect.is_none() {
            log::warn!("no monitor to show the flyout on");
        }
        placement.rect = rect;
        rect
    }
}

/// State of the flyout window.
struct FlyoutApp {
    labels: Labels,
    config: Config,
    now_playing: Option<NowPlaying>,
    volume: Option<Volume>,
    timer: FlyoutTimer,
    messages: Receiver<Message>,
    /// The current player's buttons (`None` in the demo).
    media: Option<Media>,
    window: FlyoutWindow,
    placement: SharedPlacement,
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
        let fonts = setup_fonts(&cc.egui_ctx);
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
        let placement = SharedPlacement::new(config.display.clone());
        let show_via_egui = options.show_via_egui;
        let send = {
            let placement = placement.clone();
            move |message: Message| send(&sender, &ctx, raw, &placement, message, show_via_egui)
        };
        let reload = watch_settings(&options, &config, send.clone());
        let config_file = options.config_file.clone();
        let to_window = send.clone();
        tray::spawn(tray_labels, move |command| match command {
            TrayCommand::Show => to_window(Message::show(Trigger::Tray)),
            TrayCommand::OpenSettings => tray::open_in_editor(&config_file),
            TrayCommand::ReloadSettings => reload.now(),
            TrayCommand::Quit => to_window(Message::Quit),
        });
        let media = match options.demo {
            Some(cycles) => {
                crate::demo::spawn(cycles, send);
                None
            }
            None => Some(start_sources(send)),
        };

        Ok(Self {
            labels,
            timer: FlyoutTimer::new(config.timing.timing()),
            config,
            now_playing: None,
            volume: None,
            messages,
            media,
            window,
            placement,
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
    fn keep_in_place(&self) {
        if !self.window.is_visible() {
            return;
        }
        let Some(wanted) = self.placement.lock().rect else {
            return;
        };
        let Some(actual) = self.window.rect() else {
            return;
        };
        if actual != wanted {
            // A position that differs only because the asynchronous move is
            // still on its way is not worth a line; a different size is.
            if (actual.width(), actual.height()) != (wanted.width(), wanted.height()) {
                log::info!("the window was resized to {actual:?}; putting it back to {wanted:?}");
            }
            self.window.set_rect(wanted);
        }
    }
}

/// Starts following the player, the volume and the keys. Returns the
/// player's buttons.
fn start_sources(send: impl Fn(Message) + Clone + Send + Sync + 'static) -> Media {
    let to_window = send.clone();
    volume::spawn(move |volume, changed| {
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
    media::spawn(move |now_playing, new_track| {
        send(Message::NowPlaying(now_playing));
        if new_track {
            send(Message::show(Trigger::TrackChange));
        }
    })
}

/// Sends the settings to the window every time the file is saved, with the
/// texts in the new language when that changed. Returns the handle to
/// reload them on demand.
fn watch_settings(
    options: &Options,
    config: &Config,
    send: impl Fn(Message) + Send + 'static,
) -> crate::settings::Reload {
    let lang = options.lang.clone();
    let mut language = crate::settings::language(lang.as_deref(), config);
    crate::settings::watch(options.config_file.clone(), move |config| {
        let new_language = crate::settings::language(lang.as_deref(), &config);
        let labels = if new_language != language {
            // A bad language code keeps the texts in use.
            match crate::i18n::load(new_language.as_deref()) {
                Ok(loader) => {
                    tray::set_labels(crate::i18n::tray_labels(&loader));
                    Some(crate::i18n::labels(&loader))
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
    let show = match &message {
        Message::Show { .. } => true,
        Message::Settings { config, .. } => {
            placement.lock().display = config.display.clone();
            true
        }
        Message::NowPlaying(_) | Message::Volume(_) | Message::Quit => false,
    };
    if sender.send(message).is_err() {
        return; // The window is closing.
    }
    if show && let Some(rect) = placement.next_rect() {
        // With --show-via-egui, egui shows the window once it has the
        // message; it is only moved here.
        window::place_from_any_thread(raw, rect, !show_via_egui);
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
            match message {
                Message::NowPlaying(now_playing) => self.now_playing = now_playing,
                Message::Volume(volume) => self.volume = Some(volume),
                Message::Show { trigger, sent } => {
                    self.timer.trigger(now);
                    self.pending = Some((trigger, sent));
                    self.cleared = false;
                    if self.show_via_egui {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    }
                }
                Message::Settings { config, labels } => {
                    self.timer.set_timing(config.timing.timing());
                    ctx.set_zoom_factor(config.display.scale);
                    self.config = *config;
                    if let Some(labels) = labels {
                        self.labels = labels;
                    }
                    // Show the flyout with the new settings.
                    self.timer.trigger(now);
                    self.cleared = false;
                    if self.show_via_egui {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    }
                }
                Message::Quit => {
                    log::info!("closing");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }

        self.keep_in_place();
        // A flyout under the mouse pointer stays (to reach its buttons).
        if self.timer.is_active(now) && self.pointer_over() {
            self.timer.trigger(now);
            ctx.request_repaint_after(HOVER_CHECK);
        }

        match self.timer.next_change(now) {
            // Fading: every frame. Fully shown: when the fade-out starts.
            Some(after) => ctx.request_repaint_after(after),
            // Finished: hide once a blank frame is on the screen.
            None if self.window.is_visible() => {
                if self.cleared {
                    if self.show_via_egui {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    } else {
                        self.window.hide();
                    }
                } else {
                    ctx.request_repaint();
                }
            }
            None => {}
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        let opacity = self.timer.opacity(now);
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
        let view = FlyoutView {
            now_playing: self.now_playing.as_ref(),
            volume: self.volume,
            background_opacity: self.config.display.opacity,
            labels: &self.labels,
        };
        if let Some(command) = flexflyout_ui::draw(ui, &view) {
            log::info!("button: {command:?}");
            if let Some(media) = &self.media {
                media.command(command);
            }
        }
    }
}

/// Uses the Windows UI fonts so that the flyout looks native: Segoe UI for
/// Latin text, then a Japanese font (titles can be Japanese whatever the GUI
/// language is), then egui's own fonts for anything else. Returns the fonts
/// used and their baseline offsets, for the log.
///
/// Whole font files stay in memory, so only one per script is loaded for
/// now. Chinese and Korean fonts can be added on demand later.
fn setup_fonts(ctx: &egui::Context) -> Vec<(String, f32)> {
    // In order of preference: the first file found in each group is used.
    let groups: [&[&str]; 2] = [
        &[r"C:\Windows\Fonts\segoeui.ttf"],
        &[
            r"C:\Windows\Fonts\YuGothM.ttc",
            r"C:\Windows\Fonts\meiryo.ttc",
            r"C:\Windows\Fonts\msgothic.ttc",
        ],
    ];
    let fonts = groups
        .iter()
        .filter_map(|group| {
            group.iter().find_map(|path| {
                std::fs::read(path)
                    .ok()
                    .map(|bytes| (path.to_string(), bytes))
            })
        })
        .collect();
    flexflyout_ui::fonts::install(ctx, fonts)
}
