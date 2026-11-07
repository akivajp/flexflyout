//! One process of each kind per user session: the flyout (the app itself)
//! and the settings window. A process finds out here whether another one of
//! its kind runs already, and the caller hands over to that one.
//!
//! Windows: a named mutex in the session's namespace. Linux: a lock on a
//! file in the user's runtime folder. Either is released when the process
//! ends, also when it is killed, so a crash never blocks the next start.

/// What a process is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The flyout.
    Flyout,
    /// The settings window (`--settings`).
    Settings,
}

/// Whether no other process of `kind` runs in this session. If none does,
/// this process is the one from now on, until it ends. When that cannot be
/// told (no mutex, no lock file), it counts as the first: running twice is
/// the lesser evil than not running.
pub fn claim(kind: Kind) -> bool {
    os::claim(kind)
}

#[cfg(windows)]
mod os {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::w;

    use super::Kind;

    pub fn claim(kind: Kind) -> bool {
        // The settings window's name is the one it has always had, so that
        // versions find each other.
        let name = match kind {
            Kind::Flyout => w!("Local\\FlexFlyout"),
            Kind::Settings => w!("Local\\FlexFlyoutSettings"),
        };
        // SAFETY: creates or opens a named mutex; the handle is deliberately
        // kept open until the process ends, when Windows closes it (also if
        // the process is killed).
        let created = unsafe { CreateMutexW(None, false, name) };
        // SAFETY: reads this thread's last error, set by the call above.
        let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        created.is_err() || !existed
    }
}

#[cfg(target_os = "linux")]
mod os {
    use std::fs::{File, TryLockError};
    use std::path::PathBuf;
    use std::sync::{Mutex, PoisonError};

    use super::Kind;

    /// The lock files, held open (and so locked) until the process ends.
    static HELD: Mutex<Vec<File>> = Mutex::new(Vec::new());

    pub fn claim(kind: Kind) -> bool {
        // The settings window's file is the one it has always had.
        let name = match kind {
            Kind::Flyout => "flexflyout.lock",
            Kind::Settings => "flexflyout-settings.lock",
        };
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let Ok(file) = File::create(dir.join(name)) else {
            return true;
        };
        match file.try_lock() {
            Ok(()) => {
                HELD.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(file);
                true
            }
            Err(TryLockError::WouldBlock) => false,
            // A file system without locks: as if there were no file.
            Err(TryLockError::Error(_)) => true,
        }
    }
}
