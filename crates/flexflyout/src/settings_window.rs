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

use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use eframe::egui;
use flexflyout_core::Config;
use flexflyout_core::NewTrackShow;
use flexflyout_core::config::{Anchor, Animation, Monitor, NativeOsd, SCALE_RANGE};
use i18n_embed::fluent::FluentLanguageLoader;
use i18n_embed_fl::fl;

/// How long the settings must stay unchanged before they are written (a
/// drag of a slider would otherwise write the file on every frame).
const SAVE_DELAY: Duration = Duration::from_millis(300);

/// How often the file is checked for changes made elsewhere.
const FILE_CHECK: Duration = Duration::from_secs(1);

/// How long a "−" or "+" button must be held before the value keeps
/// changing, and how often it changes then (in seconds, egui's clock).
const REPEAT_DELAY: f64 = 0.4;
const REPEAT_INTERVAL: f64 = 0.05;

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
    /// The players, listed when the window opened.
    players: Vec<flexflyout_core::Player>,
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
            players: os::players(),
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
    match spawn(config_file, lang) {
        Ok(child) => *open = Some(child),
        Err(err) => log::error!("cannot open the settings window: {err}"),
    }
}

/// Starts the settings window's process (`--settings`) for `config_file`,
/// in `lang` if given. A settings window that is open already comes to
/// the front instead, and the new process ends.
pub fn spawn(
    config_file: &std::path::Path,
    lang: Option<&str>,
) -> std::io::Result<std::process::Child> {
    let mut command = std::process::Command::new(std::env::current_exe()?);
    command.arg("--settings").arg("--config").arg(config_file);
    if let Some(lang) = lang {
        command.arg("--lang").arg(lang);
    }
    command.spawn()
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

    use crate::instance::{self, Kind};
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

    pub fn players() -> Vec<flexflyout_core::Player> {
        crate::platform::windows::media::list()
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
    /// the front. Which one is open is told by `instance` (released when the
    /// process ends, even when killed), so a left-over ID file of an ended
    /// process is never trusted.
    pub fn first_instance() -> bool {
        if instance::claim(Kind::Settings) {
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

    use flexflyout_core::layout::MonitorArea;

    use crate::instance::{self, Kind};
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

    pub fn players() -> Vec<flexflyout_core::Player> {
        crate::platform::linux::media::list()
    }

    pub fn open_in_editor(path: &Path) {
        desktop::open_in_editor(path);
    }

    pub fn open_url(url: &str) {
        desktop::open_url(url);
    }

    /// Wayland lets no other program raise a window: nothing to do.
    pub fn bring_to_front(_pid: u32) {}

    /// Whether no other settings window is open (told by `instance`).
    pub fn first_instance() -> bool {
        let first = instance::claim(Kind::Settings);
        if !first {
            log::info!("the settings window is open already");
        }
        first
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
            self.appearance(ui);
            ui.add_space(12.0);
            self.new_track(ui);
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
        // The player to follow: "Auto", the players there are, and the one
        // chosen if it is not running now.
        ui.horizontal(|ui| {
            ui.label(fl!(loader, "settings-player"));
            let player = &mut self.config.general.player;
            let auto = fl!(loader, "settings-player-auto");
            let name_of = |id: &str| {
                if id.eq_ignore_ascii_case("auto") || id.trim().is_empty() {
                    return auto.clone();
                }
                match self.players.iter().find(|p| p.id == id) {
                    Some(found) => found.name.clone(),
                    None => format!("{id} ({})", fl!(loader, "settings-player-missing")),
                }
            };
            let mut choices: Vec<String> = vec!["auto".to_owned()];
            choices.extend(self.players.iter().map(|p| p.id.clone()));
            if !choices.iter().any(|choice| choice == player) && player.trim() != "" {
                choices.push(player.clone());
            }
            egui::ComboBox::from_id_salt("player")
                .selected_text(name_of(player))
                .width(220.0)
                .show_ui(ui, |ui| {
                    for choice in choices {
                        let name = name_of(&choice);
                        ui.selectable_value(player, choice, name);
                    }
                });
        });
        ui.checkbox(
            &mut self.config.general.show_on_play_pause,
            fl!(loader, "settings-show-on-play-pause"),
        );
        ui.checkbox(
            &mut self.config.general.show_over_fullscreen,
            fl!(loader, "settings-show-over-fullscreen"),
        );
        ui.checkbox(
            &mut self.config.general.check_updates,
            fl!(loader, "settings-check-updates"),
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
                placement_rows(
                    ui,
                    "display",
                    loader,
                    &self.monitors,
                    PlacementFields {
                        monitor: &mut display.monitor,
                        anchor: &mut display.anchor,
                        offset_x: &mut display.offset_x,
                        offset_y: &mut display.offset_y,
                        scale: &mut display.scale,
                    },
                );
                ui.label(fl!(loader, "settings-opacity"));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Slider::new(&mut display.opacity, 0.0..=1.0)
                            .step_by(0.01)
                            .custom_formatter(|value, _| format!("{:.0}%", value * 100.0))
                            .custom_parser(parse_percent),
                    );
                    steppers(ui, loader, &mut display.opacity, 0.01, 0.0..=1.0);
                });
                ui.end_row();
            });
        ui.checkbox(
            &mut display.remember_drag,
            fl!(loader, "settings-remember-drag"),
        );
    }

    /// What a new track shows, for how long, and where.
    fn new_track(&mut self, ui: &mut egui::Ui) {
        let loader = &self.loader;
        ui.heading(fl!(loader, "settings-new-track"));
        let new_track = &mut self.config.new_track;
        egui::Grid::new("new-track")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label(fl!(loader, "settings-new-track-show"));
                ui.vertical(|ui| {
                    for (value, id) in [
                        (NewTrackShow::Compact, "settings-new-track-compact"),
                        (NewTrackShow::Flyout, "settings-new-track-flyout"),
                        (NewTrackShow::Nothing, "settings-new-track-nothing"),
                    ] {
                        ui.radio_value(&mut new_track.show, value, loader.get(id));
                    }
                });
                ui.end_row();
                if new_track.show == NewTrackShow::Compact {
                    ui.label(fl!(loader, "settings-new-track-hold"));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut new_track.hold_ms)
                                .range(0..=600_000)
                                .speed(50.0)
                                .suffix(" ms"),
                        );
                        steppers(ui, loader, &mut new_track.hold_ms, 100.0, 0.0..=600_000.0);
                    });
                    ui.end_row();
                }
            });
        if new_track.show != NewTrackShow::Compact {
            return;
        }
        ui.checkbox(
            &mut new_track.same_place,
            fl!(loader, "settings-new-track-same-place"),
        );
        if !new_track.same_place {
            egui::Grid::new("new-track-place")
                .num_columns(2)
                .spacing([16.0, 8.0])
                .show(ui, |ui| {
                    placement_rows(
                        ui,
                        "new-track",
                        loader,
                        &self.monitors,
                        PlacementFields {
                            monitor: &mut new_track.monitor,
                            anchor: &mut new_track.anchor,
                            offset_x: &mut new_track.offset_x,
                            offset_y: &mut new_track.offset_y,
                            scale: &mut new_track.scale,
                        },
                    );
                });
        }
    }

    /// Colors, corners and the parts of the flyout to show.
    fn appearance(&mut self, ui: &mut egui::Ui) {
        use flexflyout_core::config::{DEFAULT_ACCENT, MAX_CORNER_RADIUS};
        use flexflyout_core::{Appearance, Theme};

        let loader = &self.loader;
        ui.heading(fl!(loader, "settings-appearance"));
        let appearance = &mut self.config.appearance;
        egui::Grid::new("appearance")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label(fl!(loader, "settings-theme"));
                ui.horizontal(|ui| {
                    ui.radio_value(
                        &mut appearance.theme,
                        Theme::Dark,
                        fl!(loader, "settings-theme-dark"),
                    );
                    ui.radio_value(
                        &mut appearance.theme,
                        Theme::Light,
                        fl!(loader, "settings-theme-light"),
                    );
                    ui.radio_value(
                        &mut appearance.theme,
                        Theme::System,
                        fl!(loader, "settings-theme-system"),
                    );
                });
                ui.end_row();

                ui.label(fl!(loader, "settings-accent"));
                ui.horizontal(|ui| {
                    let (r, g, b) = appearance.accent_rgb();
                    let mut rgb = [r, g, b];
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        appearance.accent = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
                    }
                    let is_default = appearance.accent_rgb() == Appearance::default().accent_rgb();
                    if ui
                        .add_enabled(
                            !is_default,
                            egui::Button::new(fl!(loader, "settings-accent-default")),
                        )
                        .clicked()
                    {
                        appearance.accent = DEFAULT_ACCENT.to_owned();
                    }
                });
                ui.end_row();

                ui.label(fl!(loader, "settings-corner-radius"));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Slider::new(&mut appearance.corner_radius, 0..=MAX_CORNER_RADIUS)
                            .suffix(" px"),
                    );
                    steppers(
                        ui,
                        loader,
                        &mut appearance.corner_radius,
                        1.0,
                        0.0..=f64::from(MAX_CORNER_RADIUS),
                    );
                });
                ui.end_row();

                ui.label(fl!(loader, "settings-parts"));
                ui.vertical(|ui| {
                    for (value, id) in [
                        (&mut appearance.show_cover_art, "settings-show-cover-art"),
                        (&mut appearance.show_position, "settings-show-position"),
                        (
                            &mut appearance.show_shuffle_repeat,
                            "settings-show-shuffle-repeat",
                        ),
                        (&mut appearance.show_volume, "settings-show-volume"),
                    ] {
                        ui.checkbox(value, loader.get(id));
                    }
                });
                ui.end_row();
            });
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
                // The speed of a drag, and the step of the "−" and "+" buttons.
                for (id, value, max, speed, step) in [
                    (
                        "settings-fade-in",
                        &mut timing.fade_in_ms,
                        max_fade,
                        5.0,
                        10.0,
                    ),
                    ("settings-hold", &mut timing.hold_ms, 600_000, 50.0, 100.0),
                    (
                        "settings-fade-out",
                        &mut timing.fade_out_ms,
                        max_fade,
                        5.0,
                        10.0,
                    ),
                ] {
                    ui.label(loader.get(id));
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut *value)
                                .range(0..=max)
                                .speed(speed)
                                .suffix(" ms"),
                        );
                        steppers(ui, loader, value, step, 0.0..=max as f64);
                    });
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

/// The placement settings a grid row each edits: the monitor, the anchor,
/// the offsets and the size.
struct PlacementFields<'a> {
    monitor: &'a mut Monitor,
    anchor: &'a mut Anchor,
    offset_x: &'a mut i32,
    offset_y: &'a mut i32,
    scale: &'a mut f32,
}

/// The rows of a grid that edit a placement (of the flyout, or of the small
/// panel of a new track); `id` keeps the two sets of widgets apart.
fn placement_rows(
    ui: &mut egui::Ui,
    id: &str,
    loader: &FluentLanguageLoader,
    monitors: &[flexflyout_core::layout::MonitorArea],
    fields: PlacementFields<'_>,
) {
    ui.label(fl!(loader, "settings-monitor"));
    let monitor_name = |monitor: Monitor| match monitor {
        Monitor::Primary => fl!(loader, "settings-monitor-primary"),
        Monitor::Cursor => fl!(loader, "settings-monitor-cursor"),
        Monitor::Number(number) => {
            let name = fl!(loader, "settings-monitor-number", number = number);
            match monitors.get(number as usize - 1) {
                Some(area) => {
                    let size = format!("{}×{}", area.bounds.width(), area.bounds.height());
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
    choices.extend((1..=monitors.len() as u32).map(Monitor::Number));
    if !choices.contains(fields.monitor) {
        choices.push(*fields.monitor);
    }
    egui::ComboBox::from_id_salt((id, "monitor"))
        .selected_text(monitor_name(*fields.monitor))
        .width(260.0)
        .show_ui(ui, |ui| {
            for choice in choices {
                ui.selectable_value(fields.monitor, choice, monitor_name(choice));
            }
        });
    ui.end_row();

    ui.label(fl!(loader, "settings-position"));
    // The nine anchors, laid out as they are on the screen.
    egui::Grid::new((id, "anchor"))
        .spacing([10.0, 6.0])
        .show(ui, |ui| {
            let rows = [
                [Anchor::TopLeft, Anchor::Top, Anchor::TopRight],
                [Anchor::Left, Anchor::Center, Anchor::Right],
                [Anchor::BottomLeft, Anchor::Bottom, Anchor::BottomRight],
            ];
            for row in rows {
                for anchor in row {
                    ui.radio_value(fields.anchor, anchor, "");
                }
                ui.end_row();
            }
        });
    ui.end_row();

    for (label, offset) in [
        (fl!(loader, "settings-offset-x"), fields.offset_x),
        (fl!(loader, "settings-offset-y"), fields.offset_y),
    ] {
        ui.label(label);
        ui.horizontal(|ui| {
            ui.add(
                egui::DragValue::new(&mut *offset)
                    .range(-4000..=4000)
                    .suffix(" px"),
            );
            steppers(ui, loader, offset, 1.0, -4000.0..=4000.0);
        });
        ui.end_row();
    }

    ui.label(fl!(loader, "settings-scale"));
    ui.horizontal(|ui| {
        ui.add(
            egui::Slider::new(&mut *fields.scale, SCALE_RANGE.0..=SCALE_RANGE.1)
                .logarithmic(true)
                .step_by(0.05)
                .custom_formatter(|value, _| format!("{:.0}%", value * 100.0))
                .custom_parser(parse_percent),
        );
        steppers(
            ui,
            loader,
            fields.scale,
            0.05,
            f64::from(SCALE_RANGE.0)..=f64::from(SCALE_RANGE.1),
        );
    });
    ui.end_row();
}

/// "−" and "+" buttons after a number, for fine adjustments without typing:
/// each changes `value` to the next multiple of `step` down or up, within
/// `range`, once per click and repeatedly while held.
fn steppers<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    loader: &FluentLanguageLoader,
    value: &mut T,
    step: f64,
    range: RangeInclusive<f64>,
) {
    let current = value.to_f64();
    let buttons = [
        (false, "\u{2212}", fl!(loader, "settings-decrease")),
        (true, "+", fl!(loader, "settings-increase")),
    ];
    for (up, text, hint) in buttons {
        let room = if up {
            current < *range.end()
        } else {
            current > *range.start()
        };
        for _ in 0..step_button(ui, text, room, &hint) {
            *value = T::from_f64(stepped(value.to_f64(), step, up, &range));
        }
    }
}

/// A small button of [`steppers`]: by how many steps its value should change
/// in this frame (one at once when pressed, then more after a moment while
/// held, or one when pressed from the keyboard).
fn step_button(ui: &mut egui::Ui, text: &str, enabled: bool, hint: &str) -> u32 {
    let response = ui
        .add_enabled(
            enabled,
            egui::Button::new(text).min_size(egui::vec2(22.0, 0.0)),
        )
        .on_hover_text(hint);
    // A button's text is a sign; tell screen readers what it does.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, hint));
    let id = response.id;
    let (now, button_down) = ui.input(|input| (input.time, input.pointer.primary_down()));
    // When it was pressed, and how many steps it has made since.
    let held = ui.data(|data| data.get_temp::<(f64, u32)>(id));
    // egui tells when the press starts on the button, but stops telling
    // after the longest a click may take (0.8 s): from then on it is held
    // while the button stays down over it.
    let down = match held {
        None => response.is_pointer_button_down_on(),
        Some(_) => button_down && response.contains_pointer(),
    };
    if enabled && down {
        // While held, egui must keep drawing for the repeats to come.
        ui.ctx().request_repaint();
        let (pressed, done) = held.unwrap_or((now, 0));
        let due = steps_held(now - pressed);
        ui.data_mut(|data| data.insert_temp(id, (pressed, due)));
        return due.saturating_sub(done);
    }
    ui.data_mut(|data| data.remove::<(f64, u32)>(id));
    // Space or Enter on the focused button (a mouse click has been counted
    // already, when it was pressed).
    u32::from(response.clicked() && !response.clicked_by(egui::PointerButton::Primary))
}

/// How many steps a button held for `seconds` has made in all: one when
/// pressed, then one every [`REPEAT_INTERVAL`] after [`REPEAT_DELAY`]. By
/// the time rather than by the frames, so that it keeps the same pace
/// however often the window is drawn.
fn steps_held(seconds: f64) -> u32 {
    if seconds < REPEAT_DELAY {
        1
    } else {
        2 + ((seconds - REPEAT_DELAY) / REPEAT_INTERVAL) as u32
    }
}

/// The next multiple of `step` above (`up`) or below `value`, within
/// `range`. A value between two multiples (typed in by hand) goes to the
/// nearer one in that direction rather than by a whole step.
fn stepped(value: f64, step: f64, up: bool, range: &RangeInclusive<f64>) -> f64 {
    // Tolerates the rounding of values such as 0.92 (f32) / 0.01.
    const EPSILON: f64 = 1e-6;
    let steps = value / step;
    let next = if up {
        (steps + EPSILON).floor() + 1.0
    } else {
        (steps - EPSILON).ceil() - 1.0
    };
    (next * step).clamp(*range.start(), *range.end())
}

/// Reads "125%" or "125" as 1.25.
fn parse_percent(text: &str) -> Option<f64> {
    let number: f64 = text.trim().trim_end_matches('%').trim().parse().ok()?;
    Some(number / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_button_keeps_its_pace() {
        assert_eq!(steps_held(0.0), 1);
        assert_eq!(steps_held(REPEAT_DELAY - 0.01), 1);
        assert_eq!(steps_held(REPEAT_DELAY), 2);
        // Held 1.5 s: 1 + 1 + 1.1 s / 0.05 s.
        assert_eq!(steps_held(1.5), 24);
    }

    #[test]
    fn steps_go_to_the_next_multiple_within_the_range() {
        let px = -4000.0..=4000.0;
        assert_eq!(stepped(24.0, 1.0, true, &px), 25.0);
        assert_eq!(stepped(0.0, 1.0, false, &px), -1.0);
        assert_eq!(stepped(4000.0, 1.0, true, &px), 4000.0);
        // A value typed in between two multiples goes to the nearer one in
        // that direction.
        let ms = 0.0..=600_000.0;
        assert_eq!(stepped(3050.0, 100.0, true, &ms), 3100.0);
        assert_eq!(stepped(3050.0, 100.0, false, &ms), 3000.0);
        assert_eq!(stepped(50.0, 100.0, false, &ms), 0.0);
        // Fractions as the settings hold them (f32), without drifting.
        let up = stepped(f64::from(0.92_f32), 0.01, true, &(0.0..=1.0));
        assert!((up - 0.93).abs() < 1e-9, "{up}");
        let down = stepped(f64::from(1.35_f32), 0.05, false, &(0.5..=4.0));
        assert!((down - 1.30).abs() < 1e-9, "{down}");
        let up = stepped(f64::from(1.35_f32), 0.05, true, &(0.5..=4.0));
        assert!((up - 1.40).abs() < 1e-9, "{up}");
    }
}
