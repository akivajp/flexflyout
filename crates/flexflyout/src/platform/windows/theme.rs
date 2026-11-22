//! Windows' colors, for the theme and the accent that follow the system:
//!
//! - light or dark: "Choose your Windows mode", which the taskbar and
//!   Windows' own flyouts follow;
//! - the accent color: the shades Windows makes from it, a light one for
//!   dark panels and a dark one for light panels, as its own flyouts use.
//!
//! Each is read at the start, and again whenever it changes. A thread for
//! each waits for Windows to tell of a change to its key, so nothing is
//! polled.

use flexflyout_core::{DesktopColors, Theme};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET, REG_VALUE_TYPE,
    RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RegCloseKey, RegGetValueW, RegNotifyChangeKeyValue,
    RegOpenKeyExW,
};
use windows::core::{PCWSTR, w};

/// The key that holds the modes.
const MODE_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
/// The Windows mode: 1 for light, 0 for dark.
const MODE_VALUE: PCWSTR = w!("SystemUsesLightTheme");
/// The key that holds the accent color's shades.
const ACCENT_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent");
/// The shades: eight colors of four bytes (red, green, blue, unused), from
/// the lightest to the darkest, with the accent itself fourth.
const PALETTE_VALUE: PCWSTR = w!("AccentPalette");
/// The shade for dark panels ("light 2", as on Windows' toggles in dark
/// mode) and the one for light panels ("dark 1").
const ON_DARK: usize = 1;
const ON_LIGHT: usize = 4;

/// Starts following the mode and the accent: `on_theme` gets dark or light,
/// `on_colors` the accent's shades, each at once and after every change.
pub fn spawn(
    on_theme: impl Fn(Theme) + Send + 'static,
    on_colors: impl Fn(DesktopColors) + Send + 'static,
) {
    watch("theme", MODE_KEY, read_mode, move |theme| {
        log::info!("the Windows mode is {theme:?}");
        on_theme(theme);
    });
    watch("accent", ACCENT_KEY, read_accent, move |colors| {
        log::info!("the Windows accent color: {colors:?}");
        on_colors(colors);
    });
}

/// On a thread of its own (`name`), reads `path` of the current user with
/// `read`, passes the value to `sink`, then waits for a change of the key
/// and passes the value again when it differs, until the key cannot be
/// watched.
fn watch<T: Copy + PartialEq + Send + 'static>(
    name: &str,
    path: PCWSTR,
    read: fn(HKEY) -> T,
    sink: impl Fn(T) + Send + 'static,
) {
    // `PCWSTR` is a raw pointer; the `w!` strings are static, so its
    // address goes to the thread as a number.
    let address = path.as_ptr() as usize;
    let spawned = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let path = PCWSTR(address as *const u16);
            let mut key = HKEY::default();
            // SAFETY: opens a key of the current user into a live HKEY.
            let opened = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    path,
                    None,
                    KEY_READ | KEY_NOTIFY,
                    &mut key,
                )
            };
            if opened != ERROR_SUCCESS {
                // Windows before light and dark modes, or no accent set
                // yet: the default colors.
                log::info!("cannot read the Windows colors' key: {opened:?}");
                return;
            }
            let mut last = None;
            loop {
                let value = read(key);
                if last != Some(value) {
                    sink(value);
                    last = Some(value);
                }
                // SAFETY: waits on our own open key until one of its
                // values is set (synchronous: no event handle).
                let waited = unsafe {
                    RegNotifyChangeKeyValue(key, false, REG_NOTIFY_CHANGE_LAST_SET, None, false)
                };
                if waited != ERROR_SUCCESS {
                    log::warn!("cannot watch the Windows colors' key: {waited:?}");
                    break;
                }
            }
            // SAFETY: closes the key opened above.
            let _ = unsafe { RegCloseKey(key) };
        });
    if let Err(err) = spawned {
        log::warn!("cannot follow the Windows colors: {err}");
    }
}

/// The mode now: light only when Windows says so.
fn read_mode(key: HKEY) -> Theme {
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: reads a DWORD into `data`, whose size `size` gives.
    let result = unsafe {
        RegGetValueW(
            key,
            None,
            MODE_VALUE,
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

/// The accent's shades now (none if Windows has not made them).
fn read_accent(key: HKEY) -> DesktopColors {
    let mut data = [0u8; 32];
    let mut size = data.len() as u32;
    let mut kind = REG_VALUE_TYPE::default();
    // SAFETY: reads at most `size` bytes into `data`.
    let result = unsafe {
        RegGetValueW(
            key,
            None,
            PALETTE_VALUE,
            RRF_RT_REG_BINARY,
            Some(&mut kind),
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if result != ERROR_SUCCESS {
        return DesktopColors::default();
    }
    shades(&data[..size as usize])
}

/// The shades for dark and light panels from the bytes of `AccentPalette`.
fn shades(palette: &[u8]) -> DesktopColors {
    let shade = |index: usize| {
        palette
            .get(index * 4..index * 4 + 3)
            .map(|rgb| (rgb[0], rgb[1], rgb[2]))
    };
    DesktopColors {
        accent: shade(ON_DARK),
        accent_on_light: shade(ON_LIGHT),
        ..DesktopColors::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_accent_shades_are_read_from_the_palette() {
        // Windows' default blue.
        let palette = [
            0x99, 0xeb, 0xff, 0, 0x4c, 0xc2, 0xff, 0, 0x00, 0x91, 0xf8, 0, 0x00, 0x78, 0xd4, 0,
            0x00, 0x67, 0xc0, 0, 0x00, 0x3e, 0x92, 0, 0x00, 0x1a, 0x68, 0, 0xf7, 0x63, 0x0c, 0,
        ];
        let colors = shades(&palette);
        assert_eq!(colors.accent, Some((0x4c, 0xc2, 0xff)));
        assert_eq!(colors.accent_on_light, Some((0x00, 0x67, 0xc0)));
        assert_eq!(colors.background, None);
        // Too short: only what is there.
        assert_eq!(shades(&palette[..8]).accent_on_light, None);
        assert_eq!(shades(&[]), DesktopColors::default());
    }
}
