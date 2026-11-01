//! Starting with Windows: the "Run" key of the current user.
//!
//! The Task Manager's "Startup apps" page can disable an entry without
//! removing it; it then writes a value under `StartupApproved\Run` whose
//! first byte is odd. Such an entry counts as off, and turning it on again
//! here removes that mark (the user asked for it explicitly).

use std::ffi::OsString;
use std::path::Path;

use anyhow::Context as _;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW,
    RegSetKeyValueW,
};
use windows::core::{HSTRING, w};

/// The key Windows reads at sign-in.
const RUN: windows::core::PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
/// Where the Task Manager marks entries as disabled.
const APPROVED: windows::core::PCWSTR =
    w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");
/// The name of our entry in both keys.
const NAME: windows::core::PCWSTR = w!("FlexFlyout");

/// Whether FlexFlyout starts with Windows.
pub fn is_enabled() -> bool {
    // SAFETY: a size query of a value; nothing is written.
    let exists = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN,
            NAME,
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
    }
    .is_ok();
    exists && !disabled_in_task_manager()
}

/// Makes FlexFlyout start with Windows, reading `config_file` (passed with
/// `--config` when it is not the default one), or stops it from doing so.
pub fn set(enabled: bool, config_file: &Path) -> anyhow::Result<()> {
    let config = (config_file != crate::settings::default_file()).then_some(config_file);
    if !enabled {
        // SAFETY: deletes our own value.
        let result = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, NAME) };
        return ok_or_missing(result).context("cannot remove the startup entry");
    }
    let exe = std::env::current_exe().context("cannot tell where this program is")?;
    let mut command = OsString::from("\"");
    command.push(exe.as_os_str());
    command.push("\"");
    if let Some(config) = config {
        command.push(" --config \"");
        command.push(config.as_os_str());
        command.push("\"");
    }
    let command = HSTRING::from(command);
    // REG_SZ data: the UTF-16 text with its terminating zero.
    let bytes = (command.len() + 1) * 2;
    // SAFETY: the data pointer and its length describe `command`, which
    // HSTRING keeps zero-terminated.
    let result = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN,
            NAME,
            REG_SZ.0,
            Some(command.as_ptr().cast()),
            bytes as u32,
        )
    };
    result.ok().context("cannot write the startup entry")?;
    // SAFETY: deletes our own value.
    let result = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, APPROVED, NAME) };
    ok_or_missing(result).context("cannot re-enable the startup entry")?;
    log::info!("starts with Windows: {command}");
    Ok(())
}

/// Whether the Task Manager disabled our entry.
fn disabled_in_task_manager() -> bool {
    let mut data = [0u8; 12];
    let mut size = data.len() as u32;
    // SAFETY: reads at most `size` bytes into `data`.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            APPROVED,
            NAME,
            RRF_RT_REG_BINARY,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    result.is_ok() && size > 0 && data[0] % 2 == 1
}

/// Success, or a value that did not exist (nothing to delete).
fn ok_or_missing(result: WIN32_ERROR) -> windows::core::Result<()> {
    if result == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        result.ok()
    }
}
