//! The volume and media keys, through a low-level keyboard hook.
//!
//! The keys are only watched, never swallowed: Windows still changes the
//! volume and the player still gets the media keys. A key shows the flyout
//! even when the volume does not change (already at 0% or 100%, where Core
//! Audio sends no notification).
//!
//! Limits (see the project notes): the hook is not called while an elevated
//! window has the focus, and some keyboards send media keys as HID consumer
//! reports that never reach it. The volume notifications cover both cases
//! for the volume; the track change covers the media keys.

use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};

use anyhow::Context as _;
use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_MEDIA_NEXT_TRACK, VK_MEDIA_PLAY_PAUSE, VK_MEDIA_PREV_TRACK, VK_MEDIA_STOP,
    VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, MSG,
    SetWindowsHookExW, TranslateMessage, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

/// Which kind of key was pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Volume up, down or mute.
    Volume,
    /// Play/pause, next, previous or stop.
    Media,
}

/// Where the hook sends the keys: to the thread that tells the app. The
/// hook procedure only hands a key on, because Windows silently removes a
/// low-level hook that takes longer than `LowLevelHooksTimeout` to return
/// (telling the app places the window, which can take a while when the
/// system is busy). It is a plain function, so it finds the channel here;
/// there is one hook per process.
static KEYS: OnceLock<Sender<Key>> = OnceLock::new();

/// Starts watching the keys.
pub fn spawn(sink: impl Fn(Key) + Send + 'static) {
    let (keys, received) = channel();
    if KEYS.set(keys).is_err() {
        log::warn!("the keys are already watched");
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("keys-sink".into())
        .spawn(move || {
            for key in received {
                sink(key);
            }
        });
    if let Err(err) = spawned {
        log::error!("cannot start the key thread: {err}");
        return;
    }
    let spawned = std::thread::Builder::new().name("keys".into()).spawn(|| {
        if let Err(err) = run() {
            log::error!("the volume keys cannot be watched: {err:#}");
        }
    });
    if let Err(err) = spawned {
        log::error!("cannot start the key thread: {err}");
    }
}

/// Installs the hook and runs the message loop that Windows calls it from.
fn run() -> anyhow::Result<()> {
    // SAFETY: the hook procedure is a valid function for the whole process
    // lifetime, and the module handle is our own executable's.
    let _hook = unsafe {
        let module = GetModuleHandleW(None).context("no module handle")?;
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), Some(module.into()), 0)
            .context("cannot install the keyboard hook")?
    };
    log::info!("watching the volume and media keys");
    let mut message = MSG::default();
    // SAFETY: a standard message loop on this thread's own queue.
    unsafe {
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

/// The hook procedure: reports volume and media keys, passes everything on.
unsafe extern "system" fn hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let pressed = wparam.0 as u32 == WM_KEYDOWN || wparam.0 as u32 == WM_SYSKEYDOWN;
    if code == HC_ACTION as i32 && pressed {
        // SAFETY: for HC_ACTION, lparam points to a KBDLLHOOKSTRUCT.
        let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        // Keys sent by other programs (SendInput) count too: some vendors'
        // drivers send the volume keys that way.
        let key = match VIRTUAL_KEY(info.vkCode as u16) {
            VK_VOLUME_UP | VK_VOLUME_DOWN | VK_VOLUME_MUTE => Some(Key::Volume),
            VK_MEDIA_PLAY_PAUSE | VK_MEDIA_NEXT_TRACK | VK_MEDIA_PREV_TRACK | VK_MEDIA_STOP => {
                Some(Key::Media)
            }
            _ => None,
        };
        if let (Some(key), Some(keys)) = (key, KEYS.get()) {
            let _ = keys.send(key);
        }
    }
    // SAFETY: passes the event on unchanged, as every hook must.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}
