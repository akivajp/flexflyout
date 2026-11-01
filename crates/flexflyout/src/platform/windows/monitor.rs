//! The monitors and the mouse pointer, for choosing where the flyout goes.
//!
//! Coordinates are physical pixels: winit makes the process per-monitor DPI
//! aware before any window exists, so Windows reports unscaled values.

use flexflyout_core::layout::{MonitorArea, PixelRect};
use windows::Win32::Foundation::{LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, MONITORINFOF_PRIMARY};
use windows::core::BOOL;

/// The DPI of a monitor at 100% display scaling.
const BASE_DPI: f32 = 96.0;

/// All monitors, in the order Windows lists them (the order behind the
/// monitor numbers of the settings).
pub fn list() -> Vec<MonitorArea> {
    let mut monitors: Vec<MonitorArea> = Vec::new();
    // SAFETY: the callback runs synchronously, before EnumDisplayMonitors
    // returns, so the pointer to `monitors` stays valid throughout.
    let ok = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(add_monitor),
            LPARAM(&raw mut monitors as isize),
        )
    };
    if !ok.as_bool() {
        log::warn!("cannot list the monitors");
    }
    monitors
}

/// Called by EnumDisplayMonitors for each monitor; `data` points to the
/// list being filled.
unsafe extern "system" fn add_monitor(
    monitor: HMONITOR,
    _dc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> BOOL {
    // SAFETY: `data` is the `&mut Vec` passed by `list`.
    let monitors = unsafe { &mut *(data.0 as *mut Vec<MonitorArea>) };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is a MONITORINFO with its size set, as required.
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        let (mut dpi_x, mut dpi_y) = (0, 0);
        // SAFETY: both pointers are to live u32s.
        let scale =
            match unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } {
                Ok(()) if dpi_x > 0 => dpi_x as f32 / BASE_DPI,
                _ => 1.0,
            };
        monitors.push(MonitorArea {
            bounds: pixel_rect(info.rcMonitor),
            work: pixel_rect(info.rcWork),
            scale,
            primary: info.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }
    true.into() // Continue with the next monitor.
}

/// Where the mouse pointer is, or `None` if Windows does not say (e.g. on
/// the secure desktop).
pub fn cursor() -> Option<(i32, i32)> {
    let mut point = POINT::default();
    // SAFETY: `point` is a live POINT.
    unsafe { GetCursorPos(&mut point) }
        .ok()
        .map(|()| (point.x, point.y))
}

/// Converts a Win32 rectangle.
pub fn pixel_rect(rect: RECT) -> PixelRect {
    PixelRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}
