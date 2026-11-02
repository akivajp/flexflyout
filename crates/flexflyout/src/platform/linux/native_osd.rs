//! KDE Plasma's own volume popup (plasma-pa's OSD): hiding it, or knowing
//! where it appears so that the flyout can stay clear of it.
//!
//! plasma-pa shows the popup only while its `VolumeOsd` and `MuteOsd`
//! settings (in `plasmaparc`) are on, and reloads them as soon as they are
//! written with a change notification, which `kwriteconfig6 --notify` sends.
//! Unlike on Windows, this hides the volume popup alone (brightness and the
//! others stay). The settings are put back when FlexFlyout quits; a note in
//! the state folder remembers to put them back at the next start if it was
//! killed.
//!
//! KWin places the popup (an on-screen display window) centered
//! horizontally, with its middle two thirds of the way down the screen's
//! work area; it is about 15 grid units wide.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

use flexflyout_core::config::NativeOsd;
use flexflyout_core::layout::PixelRect;

/// The settings that hide the volume popup.
const KEYS: [&str; 2] = ["VolumeOsd", "MuteOsd"];

/// Size of Plasma's volume popup at the default font, in logical pixels
/// (15 grid units of 18 pixels and the padding; one medium icon high).
const SIZE: (i32, i32) = (310, 64);

/// The mode in use.
static MODE: Mutex<NativeOsd> = Mutex::new(NativeOsd::Avoid);

/// Whether this looks like a KDE Plasma session.
pub fn is_kde() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| {
        desktop
            .split(':')
            .any(|name| name.eq_ignore_ascii_case("KDE"))
    })
}

/// Where Plasma's volume popup appears on an output of `area` (logical
/// size), before it has been seen. The panel's room is not known, so the
/// output's whole height counts.
pub fn estimate(area: (i32, i32)) -> PixelRect {
    let (width, height) = area;
    PixelRect::from_origin_size(
        (width - SIZE.0) / 2,
        2 * height / 3 - SIZE.1 / 2,
        SIZE.0,
        SIZE.1,
    )
}

/// Applies the mode at the start: puts back settings that an earlier run
/// left off (it was killed), then hides the popup if asked to.
pub fn start(mode: NativeOsd) {
    if note().is_some_and(|note| note.exists()) {
        log::info!("turning Plasma's volume popup back on (left off by an earlier run)");
        show_again();
    }
    set_mode(mode);
}

/// Changes the mode: hides the popup, or brings it back.
pub fn set_mode(mode: NativeOsd) {
    let mut current = MODE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let was = *current;
    *current = mode;
    if !is_kde() {
        return;
    }
    match (was == NativeOsd::Hide, mode == NativeOsd::Hide) {
        (false, true) => hide(),
        (true, false) => show_again(),
        _ => {}
    }
}

/// Brings the popup back if this process hid it (at exit).
pub fn restore() {
    let hidden = *MODE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        == NativeOsd::Hide;
    if hidden && is_kde() {
        show_again();
    }
}

/// Turns plasma-pa's volume and mute popups off, unless they are off
/// already (then they are the user's choice and stay so).
fn hide() {
    if !KEYS.iter().any(|key| read(key)) {
        log::info!("Plasma's volume popup is off already");
        return;
    }
    // The settings as they were, written before the change, so that a kill
    // in between still restores them.
    let before: String = KEYS
        .iter()
        .map(|key| format!("{key}={}\n", read(key)))
        .collect();
    if let Some(note) = note() {
        if let Some(dir) = note.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(err) = std::fs::write(&note, before) {
            log::warn!("cannot write {}: {err}", note.display());
        }
    }
    if KEYS.iter().all(|key| write(key, false)) {
        log::info!("hid Plasma's volume popup");
    }
}

/// Puts the settings back as the note says (on, without a note) and
/// forgets the note.
fn show_again() {
    let note_path = note();
    let before = note_path
        .as_ref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    let was_on = |key: &str| {
        !before
            .lines()
            .any(|line| line.trim() == format!("{key}=false"))
    };
    if KEYS.iter().all(|key| write(key, was_on(key))) {
        log::info!("Plasma's volume popup is back");
        if let Some(path) = note_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The note that this program turned the popups off.
fn note() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
        })?;
    Some(state.join("flexflyout").join("plasma-volume-popup-hidden"))
}

/// Reads a plasma-pa setting (they default to on).
fn read(key: &str) -> bool {
    Command::new("kreadconfig6")
        .args(["--file", "plasmaparc", "--group", "General", "--key", key])
        // Without --type: with `--type bool`, kreadconfig6 6.x prints nothing.
        .args(["--default", "true"])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim() != "false")
        .unwrap_or(true)
}

/// Writes a plasma-pa setting and tells plasma-pa about it.
fn write(key: &str, value: bool) -> bool {
    let value = if value { "true" } else { "false" };
    match Command::new("kwriteconfig6")
        .args(["--file", "plasmaparc", "--group", "General", "--key", key])
        .args(["--type", "bool", "--notify", value])
        .status()
    {
        Ok(status) if status.success() => true,
        Ok(status) => {
            log::warn!("kwriteconfig6 failed for {key}: {status}");
            false
        }
        Err(err) => {
            log::warn!("cannot run kwriteconfig6 for {key}: {err}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_popup_is_centered_two_thirds_down() {
        let rect = estimate((1920, 1080));
        assert_eq!(rect.left + rect.width() / 2, 960);
        assert_eq!(rect.top + rect.height() / 2, 720);
    }
}
