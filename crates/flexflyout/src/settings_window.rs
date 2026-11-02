//! The settings window (`flexflyout --settings`), a process of its own.
//!
//! It edits the settings file: every change is written to it shortly after
//! the user stops changing things, and the running flyout picks the file up
//! within a second (and shows itself as a preview). Only the values changed
//! here are written, so the file keeps its comments, and edits made to it
//! elsewhere meanwhile are read back into the window.
//!
//! A process of its own rather than a second window of the flyout: the
//! flyout's window is hidden most of the time, eframe slows hidden windows
//! down, and the settings window costs no memory once closed. Only one is
//! open at a time: a second one brings the first to the front and ends.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;
use flexflyout_core::Config;
use flexflyout_core::config::{Anchor, Animation, Monitor, NativeOsd, SCALE_RANGE};
use i18n_embed::fluent::FluentLanguageLoader;
use i18n_embed_fl::fl;

/// How long the settings must stay unchanged before they are written (a
/// drag of a slider would otherwise write the file on every frame).
const SAVE_DELAY: Duration = Duration::from_millis(300);

/// How often the file is checked for changes made elsewhere.
const FILE_CHECK: Duration = Duration::from_secs(1);

/// Languages offered, besides the system's: code and name (in itself).
const LANGUAGES: [(&str, &str); 2] = [("en", "English"), ("ja", "日本語")];

/// Opens the settings window and runs until it is closed, or brings the
/// one already open to the front.
pub fn run(config_file: PathBuf, lang: Option<String>) -> anyhow::Result<()> {
    if !first_instance() {
        return Ok(());
    }
    let state = SettingsWindow::new(config_file, lang)?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(fl!(state.loader, "settings-title"))
            .with_inner_size([480.0, 620.0])
            .with_min_inner_size([400.0, 320.0])
            .with_icon(icon()),
        ..Default::default()
    };
    eframe::run_native(
        "FlexFlyout settings",
        options,
        Box::new(move |cc| {
            os::install_fonts(&cc.egui_ctx);
            Ok(Box::new(state))
        }),
    )
    .map_err(|err| anyhow::anyhow!("the settings window failed: {err}"))
}

/// State of the settings window.
struct SettingsWindow {
    config_file: PathBuf,
    /// The language given on the command line, which overrides the one in
    /// the settings (as in the flyout).
    lang: Option<String>,
    loader: FluentLanguageLoader,
    /// The settings as edited in the window.
    config: Config,
    /// The settings as last read from or written to the file.
    saved: Config,
    /// Why the file cannot be edited (it does not parse), if so.
    error: Option<String>,
    /// The settings as of the previous frame, and when they last changed.
    previous: Config,
    changed_at: Instant,
    /// The file's modification time after the last read or write, to notice
    /// edits made elsewhere.
    stamp: Option<SystemTime>,
    checked_at: Instant,
    /// Whether FlexFlyout starts with Windows.
    autostart: bool,
    /// The monitors, listed when the window opened.
    monitors: Vec<flexflyout_core::layout::MonitorArea>,
}

impl SettingsWindow {
    fn new(config_file: PathBuf, lang: Option<String>) -> anyhow::Result<Self> {
        let mut window = Self {
            loader: crate::i18n::load(lang.as_deref())?,
            config_file,
            lang,
            config: Config::default(),
            saved: Config::default(),
            error: None,
            previous: Config::default(),
            changed_at: Instant::now(),
            stamp: None,
            checked_at: Instant::now(),
            autostart: os::autostart_enabled(),
            monitors: os::monitors(),
        };
        window.read_file();
        Ok(window)
    }

    /// Reads the settings from the file, replacing the ones in the window.
    fn read_file(&mut self) {
        self.stamp = modified(&self.config_file);
        match Config::load(&self.config_file) {
            Ok(loaded) => {
                self.config = loaded.config.clone();
                self.saved = loaded.config.clone();
                self.previous = loaded.config;
                self.error = None;
            }
            Err(err) => {
                log::error!("{}: {err}", self.config_file.display());
                self.error = Some(err.to_string());
            }
        }
        self.reload_language();
    }

    /// Loads the texts in the language of the settings (unless the command
    /// line chose one).
    fn reload_language(&mut self) {
        let lang = crate::settings::language(self.lang.as_deref(), &self.config);
        match crate::i18n::load(lang.as_deref()) {
            Ok(loader) => self.loader = loader,
            Err(err) => log::error!("{err:#}"),
        }
    }

    /// Writes the changes once the settings have rested for a moment, and
    /// reads the file again when it was changed elsewhere.
    fn sync_with_file(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.config != self.previous {
            if self.config.general.language != self.previous.general.language {
                self.reload_language();
                let title = fl!(self.loader, "settings-title");
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
            }
            self.previous = self.config.clone();
            self.changed_at = now;
        }
        let pending = self.config != self.saved;
        let held = ctx.input(|input| input.pointer.any_down());
        if pending {
            if !held && now.duration_since(self.changed_at) >= SAVE_DELAY {
                self.write_file();
            } else {
                ctx.request_repaint_after(SAVE_DELAY);
            }
        } else if now.duration_since(self.checked_at) >= FILE_CHECK {
            self.checked_at = now;
            if modified(&self.config_file) != self.stamp {
                log::info!("the settings file changed elsewhere; reading it again");
                self.read_file();
            }
        }
        ctx.request_repaint_after(FILE_CHECK);
    }

    /// Writes the values changed in the window to the file.
    fn write_file(&mut self) {
        match self.config.save(&self.config_file, &self.saved) {
            Ok(()) => {
                log::info!("settings written: {:?}", self.config);
                self.saved = self.config.clone();
                self.stamp = modified(&self.config_file);
            }
            Err(err) => {
                log::error!("{}: {err}", self.config_file.display());
                self.error = Some(err.to_string());
            }
        }
    }
}

/// The app's icon for the title bar, the taskbar and Alt+Tab (the
/// executable's own icon is not used for a window unless it is set).
fn icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/flexflyout.png"))
        .unwrap_or_default()
}

/// The label of the autostart check box on this platform.
fn autostart_label(loader: &FluentLanguageLoader) -> String {
    if cfg!(windows) {
        fl!(loader, "settings-start-with-windows")
    } else {
        fl!(loader, "settings-start-at-login")
    }
}

/// The texts of the system's volume indicator option (its name, and what
/// hiding does), or `None` where it cannot be avoided or hidden.
fn native_osd_labels() -> Option<(&'static str, &'static str)> {
    #[cfg(windows)]
    return Some(("settings-native-osd", "settings-native-osd-hide"));
    #[cfg(target_os = "linux")]
    return crate::platform::linux::native_osd::is_kde()
        .then_some(("settings-native-osd-kde", "settings-native-osd-hide-kde"));
    #[allow(unreachable_code)]
    None
}

/// Opens the settings window (`--settings`, a process of its own), or
/// brings it to the front if this process opened it already.
pub fn open(config_file: &std::path::Path, lang: Option<&str>) {
    static OPEN: std::sync::Mutex<Option<std::process::Child>> = std::sync::Mutex::new(None);
    let mut open = OPEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(child) = open.as_mut()
        && matches!(child.try_wait(), Ok(None))
    {
        os::bring_to_front(child.id());
        return;
    }
    let spawned = std::env::current_exe().and_then(|exe| {
        let mut command = std::process::Command::new(exe);
        command.arg("--settings").arg("--config").arg(config_file);
        if let Some(lang) = lang {
            command.arg("--lang").arg(lang);
        }
        command.spawn()
    });
    match spawned {
        Ok(child) => *open = Some(child),
        Err(err) => log::error!("cannot open the settings window: {err}"),
    }
}

/// Whether no other settings window is open (if one is, it is brought to
/// the front where the platform allows that).
fn first_instance() -> bool {
    os::first_instance()
}

/// What the settings window needs from Windows.
#[cfg(windows)]
mod os {
    use std::path::{Path, PathBuf};

    use flexflyout_core::layout::MonitorArea;
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::w;

    use crate::platform::windows::{autostart, monitor, tray, window};

    pub fn install_fonts(ctx: &eframe::egui::Context) {
        crate::app::setup_fonts(ctx);
    }

    pub fn autostart_enabled() -> bool {
        autostart::is_enabled()
    }

    pub fn set_autostart(enabled: bool, config_file: &Path) -> anyhow::Result<()> {
        autostart::set(enabled, config_file)
    }

    pub fn monitors() -> Vec<MonitorArea> {
        monitor::list()
    }

    pub fn open_in_editor(path: &Path) {
        tray::open_in_editor(path);
    }

    pub fn open_url(url: &str) {
        tray::open_url(url);
    }

    pub fn bring_to_front(pid: u32) {
        window::bring_process_to_front(pid);
    }

    /// Where the open settings window's process writes its ID, for a second
    /// one to find it.
    fn pid_file() -> PathBuf {
        std::env::temp_dir().join("flexflyout-settings.pid")
    }

    /// Whether no other settings window is open. If one is, it is brought to
    /// the front. The check holds a named mutex for the life of the process
    /// (Windows releases it when the process ends, even when killed), so a
    /// left-over ID file of an ended process is never trusted.
    pub fn first_instance() -> bool {
        // SAFETY: creates or opens a named mutex; the handle is deliberately
        // kept open until the process ends.
        let created = unsafe { CreateMutexW(None, false, w!("Local\\FlexFlyoutSettings")) };
        // SAFETY: reads this thread's last error, set by the call above.
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if created.is_err() || !existed {
            let _ = std::fs::write(pid_file(), std::process::id().to_string());
            return true;
        }
        match std::fs::read_to_string(pid_file()).map(|text| text.trim().parse::<u32>()) {
            Ok(Ok(pid)) => {
                log::info!("the settings window is open already (process {pid})");
                window::bring_process_to_front(pid);
            }
            _ => log::warn!("the settings window is open already, but cannot be found"),
        }
        false
    }
}

/// What the settings window needs from the Linux desktop.
#[cfg(target_os = "linux")]
mod os {
    use std::path::Path;
    use std::sync::OnceLock;

    use flexflyout_core::layout::MonitorArea;

    use crate::platform::linux::{desktop, fonts, layer};

    pub fn install_fonts(ctx: &eframe::egui::Context) {
        flexflyout_ui::fonts::install(ctx, fonts::find());
    }

    pub fn autostart_enabled() -> bool {
        desktop::autostart_enabled()
    }

    pub fn set_autostart(enabled: bool, config_file: &Path) -> anyhow::Result<()> {
        desktop::set_autostart(enabled, config_file)
    }

    pub fn monitors() -> Vec<MonitorArea> {
        layer::outputs()
    }

    pub fn open_in_editor(path: &Path) {
        desktop::open_in_editor(path);
    }

    pub fn open_url(url: &str) {
        desktop::open_url(url);
    }

    /// Wayland lets no other program raise a window: nothing to do.
    pub fn bring_to_front(_pid: u32) {}

    /// Whether no other settings window is open: it holds a lock on a file
    /// for the life of its process (released when the process ends, even
    /// when killed).
    pub fn first_instance() -> bool {
        static LOCK: OnceLock<std::fs::File> = OnceLock::new();
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let file = match std::fs::File::create(dir.join("flexflyout-settings.lock")) {
            Ok(file) => file,
            // Without the file, a second window is the lesser evil.
            Err(_) => return true,
        };
        match file.try_lock() {
            Ok(()) => {
                let _ = LOCK.set(file);
                true
            }
            Err(_) => {
                log::info!("the settings window is open already");
                false
            }
        }
    }
}

/// When the file was last changed.
fn modified(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

impl eframe::App for SettingsWindow {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.error.is_none() {
            self.sync_with_file(ctx);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // The panel paints the theme's background and keeps a margin.
        egui::CentralPanel::default().show(ui, |ui| self.contents(ui));
    }
}

impl SettingsWindow {
    /// Everything in the window.
    fn contents(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(4.0);
            if let Some(error) = self.error.clone() {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    fl!(self.loader, "settings-file-error"),
                );
                // Monospace, so that the parser's caret points at the place.
                ui.label(egui::RichText::new(error).monospace());
                ui.horizontal(|ui| {
                    if ui.button(fl!(self.loader, "settings-open-file")).clicked() {
                        os::open_in_editor(&self.config_file);
                    }
                    if ui.button(fl!(self.loader, "settings-defaults")).clicked() {
                        self.restore_broken_file();
                    }
                });
                // Read it again once it has been fixed.
                ui.ctx().request_repaint_after(FILE_CHECK);
                if modified(&self.config_file) != self.stamp {
                    self.read_file();
                }
                return;
            }
            ui.label(fl!(self.loader, "settings-note"));
            ui.add_space(8.0);
            self.general(ui);
            ui.add_space(12.0);
            self.display(ui);
            ui.add_space(12.0);
            self.timing(ui);
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if ui.button(fl!(self.loader, "settings-open-file")).clicked() {
                    // Write pending changes first, so that the file shows them.
                    if self.config != self.saved {
                        self.write_file();
                    }
                    os::open_in_editor(&self.config_file);
                }
                if ui.button(fl!(self.loader, "settings-defaults")).clicked() {
                    self.config = Config::default();
                }
            });
            ui.add_space(16.0);
            ui.separator();
            self.about(ui);
        });
    }

    /// The version, and links to the project and to supporting it.
    fn about(&self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("FlexFlyout {}", env!("CARGO_PKG_VERSION")));
            for (id, url) in [
                ("settings-website", env!("CARGO_PKG_REPOSITORY")),
                ("settings-sponsor", "https://buymeacoffee.com/akivajp"),
            ] {
                ui.label("·");
                // Opened by the shell: eframe is built without its own
                // link opener.
                if ui.link(self.loader.get(id)).on_hover_text(url).clicked() {
                    os::open_url(url);
                }
            }
        });
    }
}

impl SettingsWindow {
    /// Language, the Windows volume indicator, play / pause, autostart.
    fn general(&mut self, ui: &mut egui::Ui) {
        let loader = &self.loader;
        ui.heading(fl!(loader, "settings-general"));
        egui::Grid::new("general")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label(fl!(loader, "settings-language"));
                let language = &mut self.config.general.language;
                let auto = fl!(loader, "settings-language-auto");
                let current = LANGUAGES
                    .iter()
                    .find(|(code, _)| language.eq_ignore_ascii_case(code))
                    .map_or(auto.clone(), |(_, name)| (*name).to_owned());
                egui::ComboBox::from_id_salt("language")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(language, "auto".to_owned(), auto);
                        for (code, name) in LANGUAGES {
                            ui.selectable_value(language, code.to_owned(), name);
                        }
                    });
                ui.end_row();

                // The system's volume indicator, where it can be avoided or hidden.
                if let Some((label, hide)) = native_osd_labels() {
                    ui.label(loader.get(label));
                    ui.vertical(|ui| {
                        let mode = &mut self.config.general.native_osd;
                        for (value, id) in [
                            (NativeOsd::Avoid, "settings-native-osd-avoid"),
                            (NativeOsd::Hide, hide),
                            (NativeOsd::Keep, "settings-native-osd-keep"),
                        ] {
                            ui.radio_value(mode, value, loader.get(id));
                        }
                    });
                    ui.end_row();
                }
            });
        ui.checkbox(
            &mut self.config.general.show_on_play_pause,
            fl!(loader, "settings-show-on-play-pause"),
        );
        let mut autostart = self.autostart;
        if ui
            .checkbox(&mut autostart, autostart_label(loader))
            .changed()
        {
            match os::set_autostart(autostart, &self.config_file) {
                Ok(()) => self.autostart = autostart,
                Err(err) => log::error!("{err:#}"),
            }
        }
    }

    /// Monitor, position, offsets, size and opacity.
    fn display(&mut self, ui: &mut egui::Ui) {
        let loader = &self.loader;
        ui.heading(fl!(loader, "settings-display"));
        let display = &mut self.config.display;
        egui::Grid::new("display")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label(fl!(loader, "settings-monitor"));
                let monitor_name = |monitor: Monitor| match monitor {
                    Monitor::Primary => fl!(loader, "settings-monitor-primary"),
                    Monitor::Cursor => fl!(loader, "settings-monitor-cursor"),
                    Monitor::Number(number) => {
                        let name = fl!(loader, "settings-monitor-number", number = number);
                        match self.monitors.get(number as usize - 1) {
                            Some(area) => {
                                let size =
                                    format!("{}×{}", area.bounds.width(), area.bounds.height());
                                if area.primary {
                                    let mark = fl!(loader, "settings-monitor-primary-mark");
                                    format!("{name} ({size}, {mark})")
                                } else {
                                    format!("{name} ({size})")
                                }
                            }
                            None => format!("{name} ({})", fl!(loader, "settings-monitor-missing")),
                        }
                    }
                };
                let mut choices = vec![Monitor::Primary, Monitor::Cursor];
                choices.extend((1..=self.monitors.len() as u32).map(Monitor::Number));
                if !choices.contains(&display.monitor) {
                    choices.push(display.monitor);
                }
                egui::ComboBox::from_id_salt("monitor")
                    .selected_text(monitor_name(display.monitor))
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        for choice in choices {
                            ui.selectable_value(&mut display.monitor, choice, monitor_name(choice));
                        }
                    });
                ui.end_row();

                ui.label(fl!(loader, "settings-position"));
                // The nine anchors, laid out as they are on the screen.
                egui::Grid::new("anchor")
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        let rows = [
                            [Anchor::TopLeft, Anchor::Top, Anchor::TopRight],
                            [Anchor::Left, Anchor::Center, Anchor::Right],
                            [Anchor::BottomLeft, Anchor::Bottom, Anchor::BottomRight],
                        ];
                        for row in rows {
                            for anchor in row {
                                ui.radio_value(&mut display.anchor, anchor, "");
                            }
                            ui.end_row();
                        }
                    });
                ui.end_row();

                ui.label(fl!(loader, "settings-offset-x"));
                ui.add(
                    egui::DragValue::new(&mut display.offset_x)
                        .range(-4000..=4000)
                        .suffix(" px"),
                );
                ui.end_row();
                ui.label(fl!(loader, "settings-offset-y"));
                ui.add(
                    egui::DragValue::new(&mut display.offset_y)
                        .range(-4000..=4000)
                        .suffix(" px"),
                );
                ui.end_row();

                ui.label(fl!(loader, "settings-scale"));
                ui.add(
                    egui::Slider::new(&mut display.scale, SCALE_RANGE.0..=SCALE_RANGE.1)
                        .logarithmic(true)
                        .step_by(0.05)
                        .custom_formatter(|value, _| format!("{:.0}%", value * 100.0))
                        .custom_parser(parse_percent),
                );
                ui.end_row();

                ui.label(fl!(loader, "settings-opacity"));
                ui.add(
                    egui::Slider::new(&mut display.opacity, 0.0..=1.0)
                        .step_by(0.01)
                        .custom_formatter(|value, _| format!("{:.0}%", value * 100.0))
                        .custom_parser(parse_percent),
                );
                ui.end_row();
            });
        ui.checkbox(
            &mut display.remember_drag,
            fl!(loader, "settings-remember-drag"),
        );
    }

    /// Fade in, display time, fade out.
    fn timing(&mut self, ui: &mut egui::Ui) {
        let loader = &self.loader;
        ui.heading(fl!(loader, "settings-timing"));
        let timing = &mut self.config.timing;
        let max_fade = flexflyout_core::config::MAX_FADE_MS;
        egui::Grid::new("timing")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                for (id, value, max, speed) in [
                    ("settings-fade-in", &mut timing.fade_in_ms, max_fade, 5.0),
                    ("settings-hold", &mut timing.hold_ms, 600_000, 50.0),
                    ("settings-fade-out", &mut timing.fade_out_ms, max_fade, 5.0),
                ] {
                    ui.label(loader.get(id));
                    ui.add(
                        egui::DragValue::new(value)
                            .range(0..=max)
                            .speed(speed)
                            .suffix(" ms"),
                    );
                    ui.end_row();
                }
                ui.label(loader.get("settings-animation"));
                ui.horizontal(|ui| {
                    ui.radio_value(
                        &mut timing.animation,
                        Animation::Slide,
                        loader.get("settings-animation-slide"),
                    );
                    ui.radio_value(
                        &mut timing.animation,
                        Animation::Fade,
                        loader.get("settings-animation-fade"),
                    );
                });
                ui.end_row();
            });
    }

    /// Replaces a file that cannot be read with the commented defaults,
    /// keeping it next to the new one as `config.toml.broken`.
    fn restore_broken_file(&mut self) {
        let mut kept = self.config_file.as_os_str().to_owned();
        kept.push(".broken");
        if let Err(err) = std::fs::rename(&self.config_file, &kept) {
            log::warn!("cannot keep the broken settings: {err}");
        }
        match Config::write_default_if_missing(&self.config_file) {
            Ok(_) => self.read_file(),
            Err(err) => self.error = Some(err.to_string()),
        }
    }
}

/// Reads "125%" or "125" as 1.25.
fn parse_percent(text: &str) -> Option<f64> {
    let number: f64 = text.trim().trim_end_matches('%').trim().parse().ok()?;
    Some(number / 100.0)
}
