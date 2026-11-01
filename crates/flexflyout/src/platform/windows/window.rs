//! Win32 settings of the flyout window that eframe does not offer, and
//! showing and hiding it without taking the focus.
//!
//! Showing goes through Win32 directly, not through egui's viewport command:
//! eframe repaints hidden windows at most every 100 ms, so a `Visible(true)`
//! sent from another thread could wait that long. [`place_from_any_thread`]
//! moves the window and makes it visible at once, and eframe then paints it
//! as usual.

use std::ffi::c_void;

use anyhow::Context as _;
use flexflyout_core::layout::PixelRect;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GCLP_HBRBACKGROUND, GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, HWND_TOPMOST, IsIconic,
    IsWindowVisible, MA_NOACTIVATE, SET_WINDOW_POS_FLAGS, STYLESTRUCT, SW_HIDE, SW_RESTORE,
    SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SWP_SHOWWINDOW, SetClassLongPtrW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, WINDOW_EX_STYLE, WM_MOUSEACTIVATE, WM_MOUSEMOVE, WM_STYLECHANGING, WS_EX_APPWINDOW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

/// Sent when the pointer leaves a window that asked for it (the windows
/// crate has it only with its large `Win32_UI_Controls` feature).
const WM_MOUSELEAVE: u32 = 0x02A3;

/// Extended styles the flyout always has: it never takes the focus, and a
/// tool window has no taskbar button.
const FLYOUT_EX_STYLE: WINDOW_EX_STYLE = WINDOW_EX_STYLE(WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0);

/// The flyout's window, used from the UI thread.
#[derive(Clone, Copy, Debug)]
pub struct FlyoutWindow(HWND);

impl FlyoutWindow {
    /// The Win32 window behind eframe's window.
    pub fn new(window: &impl HasWindowHandle) -> anyhow::Result<Self> {
        let handle = window.window_handle().context("no window handle")?;
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            anyhow::bail!("not a Win32 window");
        };
        Ok(Self(HWND(handle.hwnd.get() as *mut c_void)))
    }

    /// The handle as a plain number, to pass to other threads
    /// (`HWND` is a pointer and therefore not `Send`).
    pub fn raw(&self) -> isize {
        self.0.0 as isize
    }

    /// Makes the window a flyout: it never takes the focus, has no taskbar
    /// button and no Windows 11 border, and lets its background show through.
    pub fn apply_flyout_style(&self) -> anyhow::Result<()> {
        let hwnd = self.0;
        // SAFETY: reads and writes attributes of our own, live window; the
        // subclass procedure is a valid function for the process lifetime.
        unsafe {
            // winit rewrites the whole extended style whenever its own state
            // of the window changes (e.g. when eframe first shows it), which
            // would drop the styles below. The subclass puts them back on
            // every change, and refuses activation by a click as well.
            if !SetWindowSubclass(hwnd, Some(keep_flyout_styles), 1, 0).as_bool() {
                log::warn!("cannot guard the window styles");
            }
            // eframe adds WS_EX_APPWINDOW, which forces a taskbar button.
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let ex = (ex | FLYOUT_EX_STYLE.0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
            // Let GDI paint the window's surface black, which a transparent
            // window treats as see-through. Without it the parts egui leaves
            // transparent show white (found in the prototype).
            SetClassLongPtrW(
                hwnd,
                GCLP_HBRBACKGROUND,
                GetStockObject(BLACK_BRUSH).0 as isize,
            );
            // Windows 11 draws a thin border around every window; remove it.
            let none = DWMWA_COLOR_NONE;
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                (&raw const none).cast(),
                size_of_val(&none) as u32,
            )
            .context("cannot remove the window border")?;
            // Changed frame styles take effect only after SWP_FRAMECHANGED,
            // which also makes Windows recompute the non-client area.
            SetWindowPos(
                hwnd,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            )
            .context("cannot apply the window styles")?;
        }
        Ok(())
    }

    /// The window's rectangle on the screen.
    pub fn rect(&self) -> Option<PixelRect> {
        let mut rect = RECT::default();
        // SAFETY: a plain query into a live RECT.
        unsafe { GetWindowRect(self.0, &mut rect) }
            .ok()
            .map(|()| super::monitor::pixel_rect(rect))
    }

    /// Moves and resizes the window from its own thread, waiting until done.
    pub fn set_rect(&self, rect: PixelRect) {
        set_window_pos(self.0, rect, SWP_NOZORDER | SWP_NOACTIVATE);
    }

    /// Whether the window is shown.
    pub fn is_visible(&self) -> bool {
        // SAFETY: a plain query on our own window.
        unsafe { IsWindowVisible(self.0).as_bool() }
    }

    /// Hides the window.
    pub fn hide(&self) {
        // SAFETY: a plain call on our own window. The return value is the
        // previous visibility, not an error.
        let _ = unsafe { ShowWindow(self.0, SW_HIDE) };
    }
}

/// Subclass procedure of the flyout window, in front of winit's.
unsafe extern "system" fn keep_flyout_styles(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    match message {
        // A click must not activate the window (WS_EX_NOACTIVATE already
        // says so; this holds even while the style is being changed).
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSELEAVE => {
            // winit (0.30.13) keeps the last pointer position when the
            // pointer leaves, and reports a move only to a different
            // position. egui forgets the position on leaving and ignores
            // clicks while it has none. So a click on the very pixel where
            // the pointer left (the same button again, or a flyout shown
            // under a resting pointer) was lost. A move far outside makes
            // winit forget the position; the leave that follows tells egui
            // that the pointer is gone.
            let outside = (i16::MIN as u16 as u32) | ((i16::MIN as u16 as u32) << 16);
            // SAFETY: a mouse move with no buttons down, to winit's procedure.
            unsafe { DefSubclassProc(hwnd, WM_MOUSEMOVE, WPARAM(0), LPARAM(outside as isize)) };
        }
        WM_STYLECHANGING if wparam.0 as i32 == GWL_EXSTYLE.0 => {
            // SAFETY: for WM_STYLECHANGING, lparam points to the new styles,
            // which may be changed before they are applied.
            if let Some(styles) = unsafe { (lparam.0 as *mut STYLESTRUCT).as_mut() } {
                styles.styleNew = (styles.styleNew | FLYOUT_EX_STYLE.0) & !WS_EX_APPWINDOW.0;
            }
        }
        _ => {}
    }
    // SAFETY: passes the message on to winit's window procedure.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

/// Moves and resizes the window, puts it above the other always-on-top
/// windows and, if `show`, shows it, all without activating it. Safe to call
/// from any thread: the request is posted to the window's thread instead of
/// waiting for it.
pub fn place_from_any_thread(raw: isize, rect: PixelRect, show: bool) {
    if raw == 0 {
        return; // The window does not exist yet.
    }
    let show = if show {
        SWP_SHOWWINDOW
    } else {
        SET_WINDOW_POS_FLAGS(0)
    };
    // SWP_ASYNCWINDOWPOS keeps a busy UI thread from blocking the caller.
    set_window_pos(
        HWND(raw as *mut c_void),
        rect,
        SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS | show,
    );
}

/// `SetWindowPos` to `rect`, topmost unless `flags` has SWP_NOZORDER;
/// failures are logged.
fn set_window_pos(hwnd: HWND, rect: PixelRect, flags: SET_WINDOW_POS_FLAGS) {
    // SAFETY: the window belongs to this process and lives until it exits.
    let result = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            rect.left,
            rect.top,
            rect.width(),
            rect.height(),
            flags,
        )
    };
    if let Err(err) = result {
        log::warn!("cannot place the window at {rect:?}: {err}");
    }
}

/// Title of the window that has the focus (to check that the flyout does
/// not take it).
pub fn foreground_window_title() -> String {
    // SAFETY: reads the title into a local buffer of the given length.
    unsafe {
        let mut buf = [0u16; 256];
        let len = GetWindowTextW(GetForegroundWindow(), &mut buf);
        String::from_utf16_lossy(&buf[..len.max(0) as usize])
    }
}

/// Brings the visible top-level window of process `pid` to the front (the
/// settings window, when it is asked for again while open). Restores it if
/// it is minimized.
pub fn bring_process_to_front(pid: u32) {
    /// Finds the window: `lparam` points to (pid, found window).
    unsafe extern "system" fn find(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
        // SAFETY: `lparam` is the pointer passed below, alive during the
        // enumeration.
        let search = unsafe { &mut *(lparam.0 as *mut (u32, Option<HWND>)) };
        let mut owner = 0;
        // SAFETY: plain queries on a window being enumerated.
        let visible = unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut owner));
            IsWindowVisible(hwnd).as_bool()
        };
        if owner == search.0 && visible {
            search.1 = Some(hwnd);
            return false.into(); // Stop.
        }
        true.into()
    }
    let mut search: (u32, Option<HWND>) = (pid, None);
    // SAFETY: the callback only touches `search`, which outlives the call.
    // EnumWindows reports an error when the callback stops it early.
    let _ = unsafe { EnumWindows(Some(find), LPARAM(&mut search as *mut _ as isize)) };
    if let Some(hwnd) = search.1 {
        // SAFETY: plain window calls; the tray thread just had the
        // foreground (its menu), which allows handing it on.
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }
}
