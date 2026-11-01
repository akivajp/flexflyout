//! FlexFlyout: a flexible media flyout.
//!
//! Shows what is playing (cover art, title, artist) with playback controls
//! and the volume when the volume changes or a new track starts.
//!
//! Only the Windows host exists so far. `--demo` shows sample tracks every
//! few seconds instead of following the system.

// Release builds on Windows use the GUI subsystem (no console window).
// `console::attach_parent` brings `--help` and errors back to a terminal.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod console;
mod flyout;
mod i18n;
mod message;
mod settings;

#[cfg(windows)]
mod app;
#[cfg(any(windows, target_os = "linux"))]
mod demo;
#[cfg(target_os = "linux")]
mod linux_app;
#[cfg(any(windows, target_os = "linux"))]
mod platform;
#[cfg(windows)]
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
    /// (default: %APPDATA%\flexflyout\config.toml on Windows).
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Open the settings window instead of the flyout. A running flyout
    /// applies the changes as they are made.
    #[arg(long, conflicts_with = "demo")]
    settings: bool,

    /// Where to write the log (default: flexflyout.log, or settings.log
    /// with --settings, in %LOCALAPPDATA%\flexflyout on Windows).
    #[arg(long, value_name = "PATH")]
    log_file: Option<PathBuf>,

    /// With --demo: show the window with egui's viewport command instead of
    /// Win32 (to compare how long showing a hidden window takes).
    #[arg(long, hide = true, requires = "demo")]
    show_via_egui: bool,
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
    let log_name = if cli.settings {
        "settings.log"
    } else {
        "flexflyout.log"
    };
    let log_file = cli
        .log_file
        .clone()
        .unwrap_or_else(|| default_log_dir().join(log_name));
    init_logging(&log_file)
        .with_context(|| format!("cannot write the log to {}", log_file.display()))?;
    log::info!(
        "FlexFlyout {} starting; options: {cli:?}",
        env!("CARGO_PKG_VERSION")
    );

    let config_file = cli.config.clone().unwrap_or_else(settings::default_file);
    let config = settings::load_at_start(&config_file);
    let loader = i18n::load(settings::language(cli.lang.as_deref(), &config).as_deref())?;
    let labels = i18n::labels(&loader);

    #[cfg(windows)]
    if cli.settings {
        return settings_window::run(config_file, cli.lang);
    }
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
            },
        )
    }
    #[cfg(target_os = "linux")]
    {
        linux_app::run(
            labels,
            config,
            linux_app::Options {
                config_file,
                lang: cli.lang,
                demo: cli.demo.then_some(cli.cycles),
            },
        )
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = (labels, config, config_file);
        anyhow::bail!("FlexFlyout runs only on Windows and Linux for now")
    }
}

/// Default folder of the logs: next to the other per-user data of the app.
fn default_log_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("flexflyout")
}

/// Writes the log to `path` (overwritten on every start) and logs panics,
/// which would otherwise vanish in a GUI-subsystem process.
fn init_logging(path: &std::path::Path) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::File::create(path)?;
    let config = simplelog::ConfigBuilder::new()
        // The renderer logs a lot at the info level; its warnings still matter
        // but simplelog cannot set levels per module, so it is left out.
        .add_filter_ignore_str("wgpu")
        .add_filter_ignore_str("naga")
        .set_time_offset_to_local()
        .unwrap_or_else(|builder| builder)
        .build();
    simplelog::WriteLogger::init(log::LevelFilter::Info, config, file)?;

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
        default_hook(info);
    }));
    Ok(())
}
