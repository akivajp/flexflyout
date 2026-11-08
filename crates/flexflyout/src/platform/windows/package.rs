//! Running from an MSIX package (the Microsoft Store), and what changes
//! then.
//!
//! The same executable runs from the zip, Scoop and winget too. Packaged,
//! Windows redirects the app's writes to `%APPDATA%`, `%LOCALAPPDATA%` and
//! `HKCU` into a private copy that only the package sees: the settings file
//! would then be invisible to Notepad (opened from the tray) and a "Run"
//! entry would not start anything. So, packaged, the files go to the
//! package's own folder (`LocalState`, a real folder every program can
//! open), starting with Windows is a startup task of the package, and the
//! Store, not this app, looks for new versions.

use std::path::PathBuf;
use std::sync::OnceLock;

use windows::Win32::Foundation::{APPMODEL_ERROR_NO_PACKAGE, ERROR_INSUFFICIENT_BUFFER};
use windows::Win32::Storage::Packaging::Appx::GetCurrentPackageFamilyName;
use windows::core::PWSTR;

/// The package family name ("akivajp.FlexFlyout_1a2b3c4d5e6f7"), if running
/// from a package. Looked up once: it cannot change while running.
pub fn family_name() -> Option<&'static str> {
    static NAME: OnceLock<Option<String>> = OnceLock::new();
    NAME.get_or_init(query_family_name).as_deref()
}

/// Whether running from an MSIX package.
pub fn is_packaged() -> bool {
    family_name().is_some()
}

/// The package's folder for its own files,
/// `%LOCALAPPDATA%\Packages\<family name>\LocalState`, if packaged.
pub fn local_state() -> Option<PathBuf> {
    let family = family_name()?;
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(
        PathBuf::from(local)
            .join("Packages")
            .join(family)
            .join("LocalState"),
    )
}

/// Asks Windows for the family name of the package this process runs in.
fn query_family_name() -> Option<String> {
    let mut length = 0u32;
    // SAFETY: a size query (no buffer); Windows writes the length needed.
    let result = unsafe { GetCurrentPackageFamilyName(&mut length, None) };
    if result == APPMODEL_ERROR_NO_PACKAGE {
        return None;
    }
    if result != ERROR_INSUFFICIENT_BUFFER || length == 0 {
        log::warn!("cannot tell whether running from a package: {result:?}");
        return None;
    }
    let mut buffer = vec![0u16; length as usize];
    // SAFETY: the buffer holds `length` UTF-16 units, as Windows asked for.
    let result =
        unsafe { GetCurrentPackageFamilyName(&mut length, Some(PWSTR(buffer.as_mut_ptr()))) };
    if result.is_err() {
        log::warn!("cannot read the package family name: {result:?}");
        return None;
    }
    // `length` counts the terminating zero.
    let name = String::from_utf16_lossy(&buffer[..(length as usize).saturating_sub(1)]);
    log::info!("running from the package {name}");
    Some(name)
}
