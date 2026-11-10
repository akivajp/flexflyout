//! FlexFlyout: a flexible media flyout.
//!
//! Shows what is playing (cover art, title, artist) with playback controls
//! and the volume when the volume changes or a new track starts.
//!
//! There are two hosts: Windows (eframe) and Linux on Wayland (layer-shell).
//! `--demo` shows sample tracks every few seconds instead of following the
//! system.

// Release builds on Windows use the GUI subsystem (no console window).
// `console::attach_parent` brings `--help` and errors back to a terminal.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod console;
mod flyout;
mod i18n;
mod message;
mod settings;
#[cfg(any(windows, target_os = "linux"))]
mod update;

#[cfg(windows)]
mod app;
#[cfg(any(windows, target_os = "linux"))]
mod demo;
#[cfg(any(windows, target_os = "linux"))]
mod instance;
#[cfg(target_os = "linux")]
mod linux_app;
#[cfg(any(windows, target_os = "linux"))]
mod platform;
#[cfg(any(windows, target_os = "linux"))]
mod settings_window;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context as _;
use clap::Parser;

/// Command-line options.
#[derive(Debug, Parser)]
#[command(
    version,
    about = "A flexible media flyout: cover art, title, playback controls and volume"
)]
struct Cli {
    /// Show a sample track every few seconds instead of following the
    /// system (to try the look and the placement, and for testing).
    #[arg(long)]
    demo: bool,

    /// With --demo: how many times to show the flyout before exiting
    /// (0 = until the process is ended).
    #[arg(long, value_name = "N", default_value_t = 5, requires = "demo")]
    cycles: u32,

    /// Language of the texts, such as "en" or "ja" (default: the one in
    /// the settings, else the system's).
    #[arg(long, value_name = "LANG")]
    lang: Option<String>,

    /// Settings file, written with the defaults if it does not exist
    /// (default: %APPDATA%\flexflyout\config.toml on Windows, or the
    /// package's LocalState folder when installed from the Microsoft Store).
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Open the settings window instead of the flyout. A running flyout
    /// applies the changes as they are made.
    #[arg(long, conflicts_with = "demo")]
    settings: bool,

    /// Where to write the log (default: flexflyout.log, or settings.log
    /// with --settings, in %LOCALAPPDATA%\flexflyout on Windows and in
    /// ~/.local/state/flexflyout on Linux; with the settings in the package's
    /// LocalState folder when installed from the Microsoft Store).
    #[arg(long, value_name = "PATH")]
    log_file: Option<PathBuf>,

    /// With --demo: show the window with egui's viewport command instead of
    /// Win32 (to compare how long showing a hidden window takes).
    #[arg(long, hide = true, requires = "demo")]
    show_via_egui: bool,

    /// Check for a newer version a few seconds after the start, as if this
    /// version were running (to try the update notice).
    #[arg(long, hide = true, value_name = "VERSION")]
    check_update_as: Option<String>,
}

fn main() -> ExitCode {
    console::attach_parent();
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("{err:#}");
            console::report_error(&format!("{err:#}"));
            ExitCode::FAILURE
        }
    }
}

/// Sets up logging and the translations, then hands over to the host.
fn run(cli: Cli) -> anyhow::Result<()> {
    // Started again while it runs (from the Start menu or the app menu,
    // say): hand over to the running flyout before touching its log. The
    // demo is for trying things out, and may run next to it.
    #[cfg(any(windows, target_os = "linux"))]
    if !cli.settings && !cli.demo && !instance::claim(instance::Kind::Flyout) {
        return hand_over(&cli);
    }
    let log_name = if cli.settings {
        "settings.log"
    } else {
        "flexflyout.log"
    };
    let log_file = cli
        .log_file
        .clone()
        .unwrap_or_else(|| default_log_dir().join(log_name));
    // A log that cannot be written is no reason not to run.
    if let Err(err) = init_logging(&log_file) {
        eprintln!("cannot write the log to {}: {err:#}", log_file.display());
    }
    install_panic_hook();
    log::info!(
        "FlexFlyout {} starting; options: {cli:?}",
        env!("CARGO_PKG_VERSION")
    );
    #[cfg(windows)]
    if let Some(package) = platform::windows::package::family_name() {
        log::info!("running from the package {package} (the Microsoft Store)");
    }

    let config_file = cli.config.clone().unwrap_or_else(settings::default_file);
    let (config, first_run) = settings::load_at_start(&config_file);
    let loader = i18n::load(settings::language(cli.lang.as_deref(), &config).as_deref())?;
    let labels = i18n::labels(&loader);

    #[cfg(any(windows, target_os = "linux"))]
    if cli.settings {
        return settings_window::run(config_file, cli.lang);
    }
    #[cfg(any(windows, target_os = "linux"))]
    start_update_check(&cli, &config);
    #[cfg(windows)]
    {
        app::run(
            labels,
            i18n::tray_labels(&loader),
            config,
            app::Options {
                config_file,
                lang: cli.lang,
                demo: cli.demo.then_some(cli.cycles),
                show_via_egui: cli.show_via_egui,
                first_run,
            },
        )
    }
    #[cfg(target_os = "linux")]
    {
        linux_app::run(
            labels,
            i18n::tray_labels(&loader),
            config,
            linux_app::Options {
                config_file,
                lang: cli.lang,
                demo: cli.demo.then_some(cli.cycles),
                first_run,
            },
        )
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (labels, config, config_file, first_run);
        anyhow::bail!("FlexFlyout runs only on Windows and Linux for now")
    }
}

/// How long after the start the flyout shows once (`greet`): the sources
/// (the track, the volume) have their first values by then.
#[cfg(any(windows, target_os = "linux"))]
const GREETING_DELAY: std::time::Duration = std::time::Duration::from_millis(1500);

/// FlexFlyout lives in the tray, so that someone who starts it might think
/// nothing happened: shows the flyout once shortly after the start (also
/// at sign-in, which says that it runs), and on the very first start (no
/// settings file yet) opens the settings window too. Started again while
/// it runs, it opens the settings window only (`hand_over`).
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn greet(
    first_run: bool,
    config_file: PathBuf,
    lang: Option<String>,
    send: impl Fn(message::Message) + Send + 'static,
) {
    let spawned = std::thread::Builder::new()
        .name("greeting".into())
        .spawn(move || {
            std::thread::sleep(GREETING_DELAY);
            send(message::Message::show(message::Trigger::Start));
            if first_run {
                log::info!("the first start: opening the settings window");
                settings_window::open(&config_file, lang.as_deref());
            }
        });
    if let Err(err) = spawned {
        log::error!("cannot show the flyout at the start: {err}");
    }
}

/// Another flyout runs in this session: opens the settings window, the
/// nearest thing FlexFlyout has to a main window (and what someone who
/// starts it again most likely looks for), and leaves the running flyout
/// alone. Its changes reach the running flyout through the settings file.
#[cfg(any(windows, target_os = "linux"))]
fn hand_over(cli: &Cli) -> anyhow::Result<()> {
    let config_file = cli.config.clone().unwrap_or_else(settings::default_file);
    settings_window::spawn(&config_file, cli.lang.as_deref())
        .context("FlexFlyout runs already, and its settings window cannot be opened")?;
    Ok(())
}

/// Starts looking for a newer release (not in the demo, which is for
/// trying the flyout, unless asked to with `--check-update-as`).
#[cfg(any(windows, target_os = "linux"))]
fn start_update_check(cli: &Cli, config: &flexflyout_core::Config) {
    use i18n_embed_fl::fl;

    let (current, first_check) = match &cli.check_update_as {
        Some(version) => (version.clone(), std::time::Duration::from_secs(3)),
        None if cli.demo => return,
        None => (env!("CARGO_PKG_VERSION").to_owned(), update::FIRST_CHECK),
    };
    // From the Microsoft Store, the Store brings the new versions.
    #[cfg(windows)]
    if cli.check_update_as.is_none() && platform::windows::package::is_packaged() {
        log::info!("not looking for new versions: the Microsoft Store updates this app");
        return;
    }
    // The notice is in the language at the start.
    let loader = match i18n::load(settings::language(cli.lang.as_deref(), config).as_deref()) {
        Ok(loader) => loader,
        Err(err) => {
            log::error!("{err:#}");
            return;
        }
    };
    #[cfg(windows)]
    let fetch = platform::windows::web::get;
    #[cfg(target_os = "linux")]
    let fetch = platform::linux::web::get;
    update::spawn(
        config.general.check_updates,
        current,
        first_check,
        fetch,
        move |release, news| {
            let notice = news.then(|| {
                let title = fl!(
                    loader,
                    "update-notice-title",
                    version = release.version.as_str()
                );
                #[cfg(windows)]
                let body = fl!(loader, "update-notice-body-windows");
                #[cfg(not(windows))]
                let body = fl!(loader, "update-notice-body");
                (title, body)
            });
            #[cfg(windows)]
            platform::windows::tray::offer_update(release, notice);
            #[cfg(target_os = "linux")]
            {
                platform::linux::tray::offer_update(release);
                if let Some((title, body)) = notice {
                    platform::linux::desktop::notify(&title, &body);
                }
            }
        },
    );
}

/// Default folder of the logs: next to the other per-user data of the app.
/// `%LOCALAPPDATA%` on Windows, `$XDG_STATE_HOME` (or `~/.local/state`)
/// elsewhere. From the Microsoft Store (an MSIX package), the package's
/// own folder, next to the settings (see `settings::default_file`).
pub(crate) fn default_log_dir() -> PathBuf {
    #[cfg(windows)]
    if let Some(folder) = platform::windows::package::local_state() {
        return folder;
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_STATE_HOME").filter(|dir| !dir.is_empty()))
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state"))
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("flexflyout")
}

/// The log without the chatter of the D-Bus library (zbus, also behind the
/// tray icon's ksni), which writes every step of a connection at the info
/// level, some of it through the spans of `tracing` (logged under
/// `tracing::span`, which nothing else here uses): only its warnings and
/// errors are kept.
struct Quieter(Box<dyn log::Log>);

impl Quieter {
    /// Whether a record from `target` at `level` is worth a line.
    fn wanted(level: log::Level, target: &str) -> bool {
        level <= log::Level::Warn
            || !["zbus", "ksni", "tracing::span"]
                .iter()
                .any(|chatty| target.starts_with(chatty))
    }
}

impl log::Log for Quieter {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        Self::wanted(metadata.level(), metadata.target()) && self.0.enabled(metadata)
    }

    fn log(&self, record: &log::Record<'_>) {
        if Self::wanted(record.level(), record.target()) {
            self.0.log(record);
        }
    }

    fn flush(&self) {
        self.0.flush();
    }
}

/// Where the log of the run before is kept: `flexflyout-previous.log` next
/// to `flexflyout.log`.
fn previous_log(path: &std::path::Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map_or_else(|| "flexflyout".into(), |stem| stem.to_string_lossy());
    let name = match path.extension() {
        Some(extension) => format!("{stem}-previous.{}", extension.to_string_lossy()),
        None => format!("{stem}-previous"),
    };
    path.with_file_name(name)
}

/// Writes the log to `path`. The log of the run before is kept next to it
/// (see [`previous_log`]): after a crash, FlexFlyout is usually started
/// again, which would otherwise overwrite what tells why.
fn init_logging(path: &std::path::Path) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Nothing to keep on the first start.
    let _ = std::fs::rename(path, previous_log(path));
    let file = std::fs::File::create(path)?;
    let config = simplelog::ConfigBuilder::new()
        // The renderer logs a lot at the info level; its warnings still matter
        // but simplelog cannot set levels per module, so it is left out.
        .add_filter_ignore_str("wgpu")
        .add_filter_ignore_str("naga")
        .set_time_offset_to_local()
        .unwrap_or_else(|builder| builder)
        .build();
    let logger = simplelog::WriteLogger::new(log::LevelFilter::Info, config, file);
    log::set_boxed_logger(Box::new(Quieter(logger)))?;
    log::set_max_level(log::LevelFilter::Info);
    Ok(())
}

/// Logs panics, which would otherwise vanish in a GUI-subsystem process,
/// and leaves the system as it was before the process ends.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        // Leave the system as it was: a hidden volume indicator would stay
        // hidden after the process ends.
        #[cfg(windows)]
        {
            platform::windows::native_osd::restore();
            platform::windows::tray::remove();
        }
        #[cfg(target_os = "linux")]
        platform::linux::native_osd::restore();
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_previous_log_is_kept_next_to_it() {
        let dir = std::path::Path::new("logs");
        assert_eq!(
            previous_log(&dir.join("flexflyout.log")),
            dir.join("flexflyout-previous.log")
        );
        assert_eq!(
            previous_log(&dir.join("settings.log")),
            dir.join("settings-previous.log")
        );
        assert_eq!(previous_log(&dir.join("trace")), dir.join("trace-previous"));
    }

    #[test]
    fn the_d_bus_chatter_stays_out_of_the_log() {
        use log::Level;
        assert!(!Quieter::wanted(Level::Info, "zbus::connection::handshake"));
        assert!(!Quieter::wanted(Level::Info, "ksni::service"));
        assert!(!Quieter::wanted(Level::Info, "tracing::span"));
        // Its problems still count, and so does everything else.
        assert!(Quieter::wanted(Level::Warn, "zbus::connection"));
        assert!(Quieter::wanted(Level::Info, "flexflyout::settings"));
    }
}
