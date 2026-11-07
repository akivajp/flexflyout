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
/// whether checking was turned on). A check that got no answer (offline)
/// is tried again then, rather than a day later.
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
            // When GitHub last answered.
            let mut last: Option<SystemTime> = None;
            let mut offered: Option<String> = None;
            loop {
                // The wall clock, which also runs while the PC sleeps.
                let now = SystemTime::now();
                if ENABLED.load(Ordering::Relaxed) && is_due(last, now) {
                    match check(&current, fetch) {
                        Checked::Newer(release) => {
                            last = Some(now);
                            if offered.as_deref() != Some(release.version.as_str()) {
                                found(&release, remember(&release.version));
                                offered = Some(release.version);
                            }
                        }
                        Checked::UpToDate => last = Some(now),
                        // Offline, most likely: again at the next wake.
                        Checked::NoAnswer => {}
                    }
                }
                std::thread::sleep(WAKE);
            }
        });
    if let Err(err) = spawned {
        log::error!("cannot start the update check: {err}");
    }
}

/// Whether a check is due at `now`, when GitHub last answered at `last`:
/// never answered yet, a day later, or the clock was set back past it.
fn is_due(last: Option<SystemTime>, now: SystemTime) -> bool {
    last.is_none_or(|last| !now.duration_since(last).is_ok_and(|since| since < INTERVAL))
}

/// What a check found out.
#[derive(Debug)]
enum Checked {
    /// A newer version is out.
    Newer(Release),
    /// The latest release is this one (or older).
    UpToDate,
    /// No usable answer (offline, or an error from GitHub).
    NoAnswer,
}

/// Asks for the latest release, and compares it with `current`.
fn check(current: &str, fetch: fn(&str) -> anyhow::Result<String>) -> Checked {
    let answer = match fetch(LATEST_RELEASE_URL) {
        Ok(answer) => answer,
        Err(err) => {
            log::info!("cannot check for a newer version: {err:#}");
            return Checked::NoAnswer;
        }
    };
    let Some(release) = update::parse_latest(&answer) else {
        log::warn!("unexpected answer about the latest release");
        return Checked::NoAnswer;
    };
    if update::is_newer(current, &release.version) {
        log::info!(
            "a newer version is out: {} ({})",
            release.version,
            release.url
        );
        Checked::Newer(release)
    } else {
        log::info!("up to date (the latest release is {})", release.version);
        Checked::UpToDate
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_are_due_daily_after_an_answer() {
        let answered = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        // Never answered (also after a check that got no answer).
        assert!(is_due(None, answered));
        // Answered: not again within a day.
        assert!(!is_due(Some(answered), answered));
        assert!(!is_due(Some(answered), answered + INTERVAL - WAKE));
        assert!(is_due(Some(answered), answered + INTERVAL));
        // The clock was set back past the answer: check rather than wait.
        assert!(is_due(Some(answered), answered - WAKE));
    }

    #[test]
    fn a_check_without_an_answer_is_not_an_answer() {
        let offline: fn(&str) -> anyhow::Result<String> = |_| anyhow::bail!("offline");
        assert!(matches!(check("0.6.0", offline), Checked::NoAnswer));
        let limited: fn(&str) -> anyhow::Result<String> =
            |_| Ok(r#"{"message":"API rate limit exceeded"}"#.into());
        assert!(matches!(check("0.6.0", limited), Checked::NoAnswer));
        let latest: fn(&str) -> anyhow::Result<String> = |_| {
            Ok(r#"{"tag_name":"v0.7.0","html_url":"https://github.com/akivajp/flexflyout/releases/tag/v0.7.0"}"#.into())
        };
        assert!(matches!(check("0.7.0", latest), Checked::UpToDate));
        assert!(matches!(check("0.6.0", latest), Checked::Newer(_)));
    }
}
