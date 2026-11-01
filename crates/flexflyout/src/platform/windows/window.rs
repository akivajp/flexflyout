//! Win32 settings of the flyout window that eframe does not offer, and
//! showing and hiding it without taking the focus.
//!
//! Showing goes through Win32 directly, not through egui's viewport command:
//! eframe repaints hidden windows at most every 100 ms, so a `Visible(true)`
//! sent from another thread could wait that long. [`show_from_any_thread`]
//! makes the window visible at once, and eframe then paints it as usual.

use std::ffi::c_void;

use anyhow::Context as _;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{BLACK_BRUSH, GetStockObject};
use windows::Win32::UI::WindowsAndMessaging::{
    GCLP_HBRBACKGROUND, GWL_EXSTYLE, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextW, HWND_TOPMOST, IsWindowVisible, SPI_GETWORKAREA, SW_HIDE, SWP_ASYNCWINDOWPOS,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetClassLongPtrW, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, SystemParametersInfoW, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
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

    /// Moves the window to the top-right corner of the primary monitor's
    /// work area (the screen without the taskbar), `margin` pixels inside.
    ///
    /// A stand-in until the placement settings (monitor, nine anchors,
    /// offsets) arrive.
    pub fn move_to_top_right(&self, margin: i32) -> anyhow::Result<()> {
        let mut work = RECT::default();
        let mut window = RECT::default();
        // SAFETY: both pointers are to live RECTs of the right size.
        unsafe {
            SystemParametersInfoW(
                SPI_GETWORKAREA,
                0,
                Some((&raw mut work).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .context("cannot read the work area")?;
            GetWindowRect(self.0, &mut window).context("cannot read the window size")?;
            let width = window.right - window.left;
            SetWindowPos(
                self.0,
                None,
                work.right - margin - width,
                work.top + margin,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .context("cannot move the window")?;
        }
        Ok(())
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

/// Shows the window without activating it and puts it above the other
/// always-on-top windows. Safe to call from any thread: the request is
/// posted to the window's thread instead of waiting for it.
pub fn show_from_any_thread(raw: isize) {
    if raw == 0 {
        return; // The window does not exist yet.
    }
    // SAFETY: the window belongs to this process and lives until it exits;
    // SWP_ASYNCWINDOWPOS keeps a busy UI thread from blocking the caller.
    let result = unsafe {
        SetWindowPos(
            HWND(raw as *mut c_void),
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_ASYNCWINDOWPOS,
        )
    };
    if let Err(err) = result {
        log::warn!("cannot show the window: {err}");
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
