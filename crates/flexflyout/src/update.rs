//! Looks for a newer release once a day (`general.check_updates`), for the
//! host to offer in the tray menu and with a notification. Nothing is
//! downloaded besides GitHub's short answer about the latest release.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use flexflyout_core::update::{self, LATEST_RELEASE_URL, Release};

/// Whether to check (the setting), changed when the settings are saved.
static ENABLED: AtomicBool = AtomicBool::new(true);

/// How long after the start the first check waits (the start of a session
/// is busy enough, and the network may not be up yet).
pub const FIRST_CHECK: Duration = Duration::from_secs(60);
/// How often to check.
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// How often the thread wakes up to see whether a check is due (and
/// whether checking was turned on).
const WAKE: Duration = Duration::from_secs(60 * 60);

/// Follows the setting.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// Starts checking on a thread of its own, first after `first_check`.
/// `current` is the version running ("0.5.0"). `fetch` reads a web address
/// as text. `found` is called once for each newer release, with whether
/// the user has not been told about this version yet (to notify only
/// once, even across restarts).
pub fn spawn(
    enabled: bool,
    current: String,
    first_check: Duration,
    fetch: fn(&str) -> anyhow::Result<String>,
    found: impl Fn(&Release, bool) + Send + 'static,
) {
    set_enabled(enabled);
    let spawned = std::thread::Builder::new()
        .name("update".into())
        .spawn(move || {
            std::thread::sleep(first_check);
            let mut last: Option<SystemTime> = None;
            let mut offered: Option<String> = None;
            loop {
                // The wall clock, which also runs while the PC sleeps.
                let due = last.is_none_or(|time| time.elapsed().is_ok_and(|e| e >= INTERVAL));
                if ENABLED.load(Ordering::Relaxed) && due {
                    last = Some(SystemTime::now());
                    if let Some(release) = check(&current, fetch)
                        && offered.as_deref() != Some(release.version.as_str())
                    {
                        found(&release, remember(&release.version));
                        offered = Some(release.version);
                    }
                }
                std::thread::sleep(WAKE);
            }
        });
    if let Err(err) = spawned {
        log::error!("cannot start the update check: {err}");
    }
}

/// Asks for the latest release; returns it when it is newer than
/// `current`.
fn check(current: &str, fetch: fn(&str) -> anyhow::Result<String>) -> Option<Release> {
    let answer = match fetch(LATEST_RELEASE_URL) {
        Ok(answer) => answer,
        // Offline, most likely: try again at the next check.
        Err(err) => {
            log::info!("cannot check for a newer version: {err:#}");
            return None;
        }
    };
    let Some(release) = update::parse_latest(&answer) else {
        log::warn!("unexpected answer about the latest release");
        return None;
    };
    if update::is_newer(current, &release.version) {
        log::info!(
            "a newer version is out: {} ({})",
            release.version,
            release.url
        );
        Some(release)
    } else {
        log::info!("up to date (the latest release is {})", release.version);
        None
    }
}

/// Notes that the user was told about `version`. Returns whether this is
/// news (a different version than the one noted before).
fn remember(version: &str) -> bool {
    let path = notified_file();
    if std::fs::read_to_string(&path).is_ok_and(|noted| noted.trim() == version) {
        return false;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(err) = std::fs::write(&path, version) {
        log::warn!("cannot write {}: {err}", path.display());
    }
    true
}

/// The file that holds the version the user was last told about.
fn notified_file() -> PathBuf {
    crate::default_log_dir().join("update-notified")
}

/// The `User-Agent` of the requests (GitHub's API requires one).
pub fn user_agent() -> String {
    format!("FlexFlyout/{}", env!("CARGO_PKG_VERSION"))
}
