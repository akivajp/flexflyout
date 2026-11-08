//! The Windows 11 volume indicator (the "native OSD"): hiding it, or
//! remembering where it appears so that the flyout can stay clear of it.
//!
//! Explorer shows it in an `XamlExplorerHostIslandWindow` that it shares
//! with the brightness, airplane mode and similar indicators, so hiding it
//! hides those too. On 26H2 the window has no child windows; it is told
//! apart by its band (18, `ZBID_ABOVELOCK_UX`, read with the undocumented
//! `GetWindowBand`) and its small size. See the knowledge note "Windows 11
//! の音量 OSD を外から隠す方法と落とし穴".
//!
//! Hiding empties the window's visible region (`SetWindowRgn`), as Bolume
//! does. A WinEvent hook reports every time a window is shown, so the region
//! is set again whenever Explorer shows the indicator, with no polling. The
//! region stays after this process ends, so it is removed on every exit
//! path this process controls, and at the next start if it was killed.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use flexflyout_core::config::NativeOsd;
use flexflyout_core::layout::PixelRect;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, GetWindowRgnBox, NULLREGION, SetWindowRgn};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, DispatchMessageW, EVENT_OBJECT_SHOW, FindWindowExW, GetClassNameW, GetMessageW,
    GetShellWindow, GetWindowRect, GetWindowThreadProcessId, IsWindow, IsWindowVisible, MSG,
    OBJID_WINDOW, SetTimer, TranslateMessage, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};
use windows::core::{BOOL, s, w};

use super::monitor::pixel_rect;

/// Window class of the indicator.
const CLASS: &str = "XamlExplorerHostIslandWindow";

/// The band of the indicator's window (`ZBID_ABOVELOCK_UX`).
const OSD_BAND: u32 = 18;

/// Largest size (pixels) of a window that can be the indicator; larger
/// windows of the same class are something else (the same limits as the
/// Windhawk mods use).
const MAX_SIZE: (i32, i32) = (700, 250);

/// How long after the indicator is shown its final position is read
/// (it grows from its hidden size when shown).
const SETTLE_MS: u32 = 200;

/// The mode chosen in the settings, as a number for an atomic.
static MODE: AtomicU8 = AtomicU8::new(0);
/// Where the indicator was last seen while shown.
static LAST_SEEN: Mutex<Option<PixelRect>> = Mutex::new(None);
/// The windows whose region this process emptied (raw handles).
static HIDDEN: Mutex<Vec<isize>> = Mutex::new(Vec::new());

/// Starts watching the indicator in `mode`.
pub fn start(mode: NativeOsd) {
    set_mode(mode);
    let spawned = std::thread::Builder::new()
        .name("native-osd".into())
        .spawn(run);
    if let Err(err) = spawned {
        log::error!("cannot watch the Windows volume indicator: {err}");
    }
}

/// Changes the mode: hides the indicator at once, or brings it back.
pub fn set_mode(mode: NativeOsd) {
    MODE.store(encode(mode), Ordering::Release);
    if mode == NativeOsd::Hide {
        for hwnd in find_all() {
            hide(hwnd);
        }
    } else {
        restore();
        // A region left by an earlier run that was killed: nothing else
        // empties this window's region.
        for hwnd in find_all() {
            if has_empty_region(hwnd) {
                log::info!("bringing back a hidden volume indicator");
                show_again(hwnd);
            }
        }
    }
}

/// Where the indicator was last seen, if it has been seen.
pub fn last_seen() -> Option<PixelRect> {
    *LAST_SEEN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Brings back every indicator this process hid (at exit). Windows that
/// are gone (Explorer restarted meanwhile) are skipped.
pub fn restore() {
    let hidden = std::mem::take(&mut *HIDDEN.lock().unwrap_or_else(PoisonError::into_inner));
    for raw in hidden {
        let hwnd = HWND(raw as *mut _);
        // SAFETY: a plain query; a stale handle only answers false.
        if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            show_again(hwnd);
        }
    }
}

fn encode(mode: NativeOsd) -> u8 {
    match mode {
        NativeOsd::Avoid => 0,
        NativeOsd::Hide => 1,
        NativeOsd::Keep => 2,
    }
}

fn mode() -> NativeOsd {
    match MODE.load(Ordering::Acquire) {
        1 => NativeOsd::Hide,
        2 => NativeOsd::Keep,
        _ => NativeOsd::Avoid,
    }
}

/// Installs the WinEvent hook and runs the message loop it is called from.
fn run() {
    // SAFETY: the callback is a valid function for the process lifetime;
    // out-of-context hooks are called through this thread's message loop.
    unsafe {
        let hook = SetWinEventHook(
            EVENT_OBJECT_SHOW,
            EVENT_OBJECT_SHOW,
            None,
            Some(on_show),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        if hook.is_invalid() {
            log::error!("cannot watch the Windows volume indicator: no WinEvent hook");
            return;
        }
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

/// Called for every window shown anywhere: acts on the indicator only.
unsafe extern "system" fn on_show(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    if object != OBJID_WINDOW.0 || child != CHILDID_SELF as i32 || !is_indicator(hwnd) {
        return;
    }
    match mode() {
        NativeOsd::Hide => hide(hwnd),
        NativeOsd::Avoid => {
            // Read its place once it has grown to its shown size.
            // SAFETY: a thread timer with a valid callback on this thread.
            unsafe { SetTimer(None, 0, SETTLE_MS, Some(on_settled)) };
        }
        NativeOsd::Keep => {}
    }
}

/// Timer callback after the indicator was shown: remembers where it is.
unsafe extern "system" fn on_settled(_hwnd: HWND, _message: u32, timer: usize, _time: u32) {
    // SAFETY: stops the one-shot thread timer that called us.
    let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::KillTimer(None, timer) };
    for hwnd in find_all() {
        // SAFETY: plain queries on another process's window.
        if unsafe { IsWindowVisible(hwnd) }.as_bool()
            && let Some(rect) = window_rect(hwnd)
        {
            let mut last = LAST_SEEN.lock().unwrap_or_else(PoisonError::into_inner);
            if *last != Some(rect) {
                log::info!("the Windows volume indicator appears at {rect:?}");
                *last = Some(rect);
            }
        }
    }
}

/// Every window that looks like the indicator.
fn find_all() -> Vec<HWND> {
    let mut found = Vec::new();
    let mut after = None;
    // SAFETY: plain window enumeration. The previous result is passed on,
    // or the same window would come back forever.
    while let Ok(hwnd) =
        unsafe { FindWindowExW(None, after, w!("XamlExplorerHostIslandWindow"), None) }
    {
        if is_indicator(hwnd) {
            found.push(hwnd);
        }
        after = Some(hwnd);
    }
    found
}

/// Whether `hwnd` is Explorer's indicator window.
fn is_indicator(hwnd: HWND) -> bool {
    let mut class = [0u16; 64];
    // SAFETY: reads the class name into a local buffer.
    let len = unsafe { GetClassNameW(hwnd, &mut class) };
    if String::from_utf16_lossy(&class[..len.max(0) as usize]) != CLASS {
        return false;
    }
    // Explorer's: the same process as the desktop's shell window.
    let (mut owner, mut shell) = (0u32, 0u32);
    // SAFETY: plain queries.
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut owner));
        GetWindowThreadProcessId(GetShellWindow(), Some(&mut shell));
    }
    if owner == 0 || owner != shell {
        return false;
    }
    if let Some(band) = window_band(hwnd)
        && band != OSD_BAND
    {
        return false;
    }
    window_rect(hwnd).is_some_and(|r| {
        (1..MAX_SIZE.0).contains(&r.width()) && (1..MAX_SIZE.1).contains(&r.height())
    })
}

/// The window's band, through the undocumented `GetWindowBand`, or `None`
/// where Windows does not have it.
fn window_band(hwnd: HWND) -> Option<u32> {
    type GetWindowBand = unsafe extern "system" fn(HWND, *mut u32) -> BOOL;
    static FUNCTION: OnceLock<Option<GetWindowBand>> = OnceLock::new();
    let function = FUNCTION.get_or_init(|| {
        // SAFETY: looks up an export of user32, which every GUI process
        // has loaded; its signature is the one used by Modern Flyouts.
        unsafe {
            let user32 = GetModuleHandleW(w!("user32.dll")).ok()?;
            GetProcAddress(user32, s!("GetWindowBand"))
                .map(|f| std::mem::transmute::<_, GetWindowBand>(f))
        }
    });
    let mut band = 0;
    // SAFETY: calls the function found above with a live output pointer.
    (*function).and_then(|f| unsafe { f(hwnd, &mut band) }.as_bool().then_some(band))
}

/// The window's rectangle on the screen.
fn window_rect(hwnd: HWND) -> Option<PixelRect> {
    let mut rect = RECT::default();
    // SAFETY: a plain query into a live RECT.
    unsafe { GetWindowRect(hwnd, &mut rect) }
        .ok()
        .map(|()| pixel_rect(rect))
}

/// Whether the window's region is set and empty.
fn has_empty_region(hwnd: HWND) -> bool {
    let mut rect = RECT::default();
    // SAFETY: a plain query into a live RECT.
    let kind = unsafe { GetWindowRgnBox(hwnd, &mut rect) };
    kind == NULLREGION
}

/// Empties the window's visible region (unless it is empty already).
fn hide(hwnd: HWND) {
    if has_empty_region(hwnd) {
        return;
    }
    // SAFETY: the window takes over the region; an empty rectangle region.
    let result = unsafe { SetWindowRgn(hwnd, Some(CreateRectRgn(0, 0, 0, 0)), true) };
    if result == 0 {
        log::warn!("cannot hide the Windows volume indicator");
        return;
    }
    let mut hidden = HIDDEN.lock().unwrap_or_else(PoisonError::into_inner);
    if !hidden.contains(&(hwnd.0 as isize)) {
        log::info!("hid the Windows volume indicator");
        hidden.push(hwnd.0 as isize);
    }
}

/// Removes the window's region, so that all of it shows again.
fn show_again(hwnd: HWND) {
    // SAFETY: removing the region of a window is always allowed.
    if unsafe { SetWindowRgn(hwnd, None, true) } == 0 {
        log::warn!("cannot bring back the Windows volume indicator");
    }
}
