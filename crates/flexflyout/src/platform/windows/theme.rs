//! Windows' light or dark mode ("Choose your Windows mode", which the
//! taskbar and Windows' own flyouts follow), for the theme that follows the
//! system: read at the start, and again whenever it changes. A thread of
//! its own waits for Windows to tell of a change to the key, so nothing is
//! polled.

use flexflyout_core::Theme;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET, RRF_RT_REG_DWORD,
    RegCloseKey, RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW,
};
use windows::core::{PCWSTR, w};

/// The key that holds the modes.
const KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
/// The Windows mode: 1 for light, 0 for dark.
const VALUE: PCWSTR = w!("SystemUsesLightTheme");

/// Starts following the mode; `sink` gets it at once and after every
/// change.
pub fn spawn(sink: impl Fn(Theme) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("theme".into())
        .spawn(move || run(&sink));
    if let Err(err) = spawned {
        log::warn!("cannot follow the Windows mode: {err}");
    }
}

/// Reports the mode, then waits for a change and reports it again, until
/// the key cannot be watched.
fn run(sink: &impl Fn(Theme)) {
    let mut key = HKEY::default();
    // SAFETY: opens a key of the current user into a live HKEY.
    let opened = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            KEY,
            None,
            KEY_READ | KEY_NOTIFY,
            &mut key,
        )
    };
    if opened != ERROR_SUCCESS {
        // Windows before light and dark modes: the default colors.
        log::info!("the Windows mode is not known: {opened:?}");
        return;
    }
    let mut last = None;
    loop {
        let theme = read(key);
        if last != Some(theme) {
            log::info!("the Windows mode is {theme:?}");
            sink(theme);
            last = Some(theme);
        }
        // SAFETY: waits on our own open key until one of its values is
        // set (synchronous: no event handle).
        let waited =
            unsafe { RegNotifyChangeKeyValue(key, false, REG_NOTIFY_CHANGE_LAST_SET, None, false) };
        if waited != ERROR_SUCCESS {
            log::warn!("cannot watch the Windows mode: {waited:?}");
            break;
        }
    }
    // SAFETY: closes the key opened above.
    let _ = unsafe { RegCloseKey(key) };
}

/// The mode now: light only when Windows says so.
fn read(key: HKEY) -> Theme {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: reads a DWORD into `data`, whose size `size` gives.
    let result = unsafe {
        RegGetValueW(
            key,
            None,
            VALUE,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut data).cast()),
            Some(&mut size),
        )
    };
    if result == ERROR_SUCCESS && data == 1 {
        Theme::Light
    } else {
        Theme::Dark
    }
}
