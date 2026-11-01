//! The Windows host: an eframe window that draws the flyout and shows and
//! hides itself following [`FlyoutTimer`].
//!
//! Sources (for now the demo and the settings file) run on their own
//! threads. They send a [`Message`], show the window through Win32 at once
//! when there is something to show, and ask egui for a repaint; the app then
//! picks the message up in [`eframe::App::logic`].

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Instant;

use eframe::egui;
use flexflyout_core::{Config, FlyoutTimer, NowPlaying, Volume};
use flexflyout_ui::{BASE_SIZE, FlyoutView, Labels};

use crate::platform::windows::window::{self, FlyoutWindow};

/// Distance of the window from the corner of the screen, in pixels.
const MARGIN: i32 = 24;

/// What the sources send to the window.
#[derive(Debug)]
pub enum Message {
    /// Show the flyout with this content.
    Show {
        /// The current track, or `None` when nothing is playing.
        now_playing: Option<NowPlaying>,
        /// The current volume.
        volume: Volume,
        /// When the message was sent, to log how long showing took.
        sent: Instant,
    },
    /// Use new settings (the file was saved).
    Settings {
        /// The settings read from the file.
        config: Box<Config>,
        /// The texts again, when the language changed.
        labels: Option<Labels>,
    },
    /// Close the app (the end of the demo).
    Quit,
}

/// Options of the Windows host.
#[derive(Clone, Debug)]
pub struct Options {
    /// The settings file to watch for changes.
    pub config_file: PathBuf,
    /// The language given on the command line, which overrides the one in
    /// the settings.
    pub lang: Option<String>,
    /// Number of demo appearances (0 = endless).
    pub demo_cycles: u32,
    /// Show the window with egui's viewport command instead of Win32.
    pub show_via_egui: bool,
}

/// Opens the flyout window and runs until it is closed.
pub fn run(labels: Labels, config: Config, options: Options) -> anyhow::Result<()> {
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
    eframe::run_native(
        "FlexFlyout",
        native_options,
        Box::new(move |cc| {
            let app = FlyoutApp::new(cc, labels, config, options)?;
            Ok(Box::new(app))
        }),
    )
    .map_err(|err| anyhow::anyhow!("the window failed: {err}"))
}

/// State of the flyout window.
struct FlyoutApp {
    labels: Labels,
    config: Config,
    now_playing: Option<NowPlaying>,
    volume: Option<Volume>,
    timer: FlyoutTimer,
    messages: Receiver<Message>,
    window: FlyoutWindow,
    show_via_egui: bool,
    /// When the message that is being shown was sent, until the first frame
    /// with it has been drawn (then the delay is logged).
    pending_since: Option<Instant>,
    /// Whether a fully transparent frame has been drawn since the timer
    /// ended. The window is hidden only then, so that showing it again does
    /// not briefly bring back the last picture.
    cleared: bool,
}

impl FlyoutApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        labels: Labels,
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
        window.move_to_top_right(MARGIN)?;
        let fonts = setup_fonts(&cc.egui_ctx);
        log::info!("fonts: {fonts:?}");

        let (sender, messages) = channel();
        let ctx = cc.egui_ctx.clone();
        let raw = window.raw();
        let show_via_egui = options.show_via_egui;
        let send = move |message: Message| send(&sender, &ctx, raw, message, show_via_egui);
        watch_settings(&options, &config, send.clone());
        crate::demo::spawn(options.demo_cycles, send);

        Ok(Self {
            labels,
            timer: FlyoutTimer::new(config.timing.timing()),
            config,
            now_playing: None,
            volume: None,
            messages,
            window,
            show_via_egui: options.show_via_egui,
            pending_since: None,
            cleared: false,
        })
    }
}

/// Sends the settings to the window every time the file is saved, with the
/// texts in the new language when that changed.
fn watch_settings(options: &Options, config: &Config, send: impl Fn(Message) + Send + 'static) {
    let lang = options.lang.clone();
    let mut language = crate::settings::language(lang.as_deref(), config);
    crate::settings::watch(options.config_file.clone(), move |config| {
        let new_language = crate::settings::language(lang.as_deref(), &config);
        let labels = if new_language != language {
            // A bad language code keeps the texts in use.
            match crate::i18n::load(new_language.as_deref()) {
                Ok(loader) => Some(crate::i18n::labels(&loader)),
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
    });
}

/// Hands a message to the window from a source's thread.
///
/// The message goes first, so that the window has it by the time it is
/// shown and painted; then the window is shown and egui is woken up.
fn send(
    sender: &Sender<Message>,
    ctx: &egui::Context,
    raw: isize,
    message: Message,
    show_via_egui: bool,
) {
    let show = matches!(message, Message::Show { .. });
    if sender.send(message).is_err() {
        return; // The window is closing.
    }
    if show && !show_via_egui {
        window::show_from_any_thread(raw);
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
                Message::Show {
                    now_playing,
                    volume,
                    sent,
                } => {
                    self.now_playing = now_playing;
                    self.volume = Some(volume);
                    self.timer.trigger(now);
                    self.pending_since = Some(sent);
                    self.cleared = false;
                    if self.show_via_egui {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    }
                }
                Message::Settings { config, labels } => {
                    self.timer.set_timing(config.timing.timing());
                    self.config = *config;
                    if let Some(labels) = labels {
                        self.labels = labels;
                    }
                }
                Message::Quit => {
                    log::info!("closing");
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
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
            // Draw nothing, then let `logic` hide the window.
            self.cleared = true;
            ui.ctx().request_repaint();
            return;
        }
        if let Some(sent) = self.pending_since.take() {
            log::info!(
                "drawing {:.1} ms after the message was sent; focus: \"{}\"",
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
            // Sent to the player once the SMTC backend exists.
            log::info!("button: {command:?}");
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
