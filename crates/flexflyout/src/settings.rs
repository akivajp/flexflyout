//! The settings file on disk: where it is, reading it with the problems
//! logged, and noticing when it changes.
//!
//! A file that cannot be read never stops the app: at the start the defaults
//! are used, and later the settings in use are kept, with the reason in the
//! log either way.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, SystemTime};

use flexflyout_core::Config;

/// How often the file is checked for changes.
const WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// Default place of the settings: `%APPDATA%\flexflyout\config.toml` on
/// Windows, `$XDG_CONFIG_HOME/flexflyout/config.toml` (or `~/.config/...`)
/// elsewhere.
pub fn default_file() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("flexflyout").join("config.toml")
}

/// Reads the settings at the start, after writing the commented default
/// file if there is none yet. Falls back to the defaults.
pub fn load_at_start(path: &Path) -> Config {
    match Config::write_default_if_missing(path) {
        Ok(true) => log::info!("wrote the default settings to {}", path.display()),
        Ok(false) => {}
        Err(err) => log::warn!(
            "cannot write the default settings to {}: {err}",
            path.display()
        ),
    }
    read(path).unwrap_or_else(|| {
        log::warn!("using the default settings");
        Config::default()
    })
}

/// The language to show the texts in: the command line's, else the
/// settings', else `None` for the system's.
pub fn language(command_line: Option<&str>, config: &Config) -> Option<String> {
    command_line
        .or_else(|| config.general.language())
        .map(str::to_owned)
}

/// Asks the watching thread to read the settings at once.
#[derive(Clone, Debug)]
// Unused where FlexFlyout has no host yet (only Windows and Linux do).
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub struct Reload(Sender<()>);

#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
impl Reload {
    /// Reads the settings now, even if the file looks unchanged.
    pub fn now(&self) {
        let _ = self.0.send(());
    }
}

/// Calls `on_change` on a thread of its own with the new settings every
/// time the file is saved and can be read, or when asked through the
/// returned [`Reload`].
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub fn watch(path: PathBuf, mut on_change: impl FnMut(Config) + Send + 'static) -> Reload {
    let (reload, requests) = channel();
    let spawned = thread::Builder::new()
        .name("settings".to_owned())
        .spawn(move || {
            let mut seen = stamp(&path);
            loop {
                let asked = match requests.recv_timeout(WATCH_INTERVAL) {
                    Ok(()) => true,
                    Err(RecvTimeoutError::Timeout) => false,
                    // No one can ask any more: keep watching the file at the
                    // same pace (the receive no longer waits).
                    Err(RecvTimeoutError::Disconnected) => {
                        thread::sleep(WATCH_INTERVAL);
                        false
                    }
                };
                let now = stamp(&path);
                // A missing file keeps the settings in use: it may be in the
                // middle of being replaced by an editor.
                if !asked && (now.is_none() || now == seen) {
                    continue;
                }
                seen = now;
                if let Some(config) = read(&path) {
                    log::info!("settings reloaded from {}", path.display());
                    on_change(config);
                }
            }
        });
    if let Err(err) = spawned {
        log::warn!("cannot watch the settings for changes: {err}");
    }
    Reload(reload)
}

/// What tells that the file changed: its modification time and size (the
/// size catches two saves within the timer resolution of the file system).
fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

/// Reads the settings and logs every problem; `None` if they are unusable.
fn read(path: &Path) -> Option<Config> {
    match Config::load(path) {
        Ok(loaded) => {
            for warning in &loaded.warnings {
                log::warn!("{}: {warning}", path.display());
            }
            log::info!("settings: {:?}", loaded.config);
            Some(loaded.config)
        }
        Err(err) => {
            log::error!("{}: {err}", path.display());
            None
        }
    }
}
