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
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject};
use windows::Win32::UI::WindowsAndMessaging::{
    GCLP_HBRBACKGROUND, GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextW, HWND_TOPMOST, IsWindowVisible, SET_WINDOW_POS_FLAGS, SW_HIDE,
    SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SWP_SHOWWINDOW, SetClassLongPtrW, SetWindowLongPtrW, SetWindowPos, ShowWindow, WS_EX_APPWINDOW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

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
        // SAFETY: reads and writes attributes of our own, live window.
        unsafe {
            // No focus when shown or clicked; a tool window has no taskbar
            // button. eframe adds WS_EX_APPWINDOW, which forces a button.
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let ex = (ex | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize)
                & !(WS_EX_APPWINDOW.0 as isize);
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
