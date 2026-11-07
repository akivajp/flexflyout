//! The notification area ("tray") icon and its menu.
//!
//! A thread of its own owns a hidden top-level window that receives the
//! icon's clicks. It is top-level rather than message-only so that it gets
//! the "TaskbarCreated" broadcast, after which the icon must be added again
//! (Explorer restarted and forgot it).

use std::path::Path;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use anyhow::Context as _;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_EXECUTABLE, AssocQueryStringW, NIF_ICON, NIF_INFO,
    NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIN_BALLOONUSERCLICK,
    NOTIFYICONDATAW, Shell_NotifyIconW, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DispatchMessageW, GetCursorPos, GetMessageW, GetSystemMetrics, HICON, IMAGE_ICON,
    LR_DEFAULTCOLOR, LoadImageW, MF_CHECKED, MF_SEPARATOR, MF_STRING, MSG, PostMessageW,
    RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SM_CYSMICON, SW_SHOWNORMAL,
    SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    TranslateMessage, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL,
    WM_RBUTTONUP, WNDCLASSW,
};
use windows::core::{HSTRING, PCWSTR, w};

use flexflyout_core::NowPlaying;
use flexflyout_core::update::Release;

use crate::i18n::TrayLabels;

pub use crate::message::TrayCommand;

/// The menu, in order: command ID (0 = a separator) and command.
const MENU: [(usize, Option<TrayCommand>); 9] = [
    (1, Some(TrayCommand::Show)),
    (0, None),
    (6, Some(TrayCommand::Settings)),
    (2, Some(TrayCommand::OpenSettings)),
    (3, Some(TrayCommand::ReloadSettings)),
    (0, None),
    (5, Some(TrayCommand::ToggleAutostart)),
    (0, None),
    (4, Some(TrayCommand::Quit)),
];

/// The command ID of the menu item that opens a newer release's page.
const UPDATE_ID: usize = 7;

/// A newer release, offered at the top of the menu.
static UPDATE: Mutex<Option<Release>> = Mutex::new(None);

/// What plays, in one line, shown under the app's name in the icon's tip.
static PLAYING: Mutex<Option<String>> = Mutex::new(None);

/// The message the icon sends to the window.
const WM_TRAY: u32 = WM_APP + 1;

/// The icon's ID within our window (there is one).
const ICON_ID: u32 = 1;

/// State of the tray thread, reachable from the window procedure.
struct Tray {
    labels: Mutex<TrayLabels>,
    on_command: Box<dyn Fn(TrayCommand) + Send + Sync>,
}

static TRAY: OnceLock<Tray> = OnceLock::new();
/// The hidden window, so that [`remove`] can delete the icon from any thread.
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// The ID of the "TaskbarCreated" message.
static TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

/// Adds the icon. `on_command` is called on the tray thread.
pub fn spawn(labels: TrayLabels, on_command: impl Fn(TrayCommand) + Send + Sync + 'static) {
    let tray = Tray {
        labels: Mutex::new(labels),
        on_command: Box::new(on_command),
    };
    if TRAY.set(tray).is_err() {
        log::warn!("the tray icon exists already");
        return;
    }
    let spawned = std::thread::Builder::new().name("tray".into()).spawn(|| {
        if let Err(err) = run() {
            log::error!("no tray icon: {err:#}");
        }
    });
    if let Err(err) = spawned {
        log::error!("cannot start the tray thread: {err}");
    }
}

/// Changes the menu's texts (after the language changed).
pub fn set_labels(labels: TrayLabels) {
    if let Some(tray) = TRAY.get() {
        *tray.labels.lock().unwrap_or_else(PoisonError::into_inner) = labels;
    }
}

/// Offers `release` at the top of the menu, and shows `notice` (a title
/// and a text) as a notification; a click on it opens the release's page.
pub fn offer_update(release: &Release, notice: Option<(String, String)>) {
    *UPDATE.lock().unwrap_or_else(PoisonError::into_inner) = Some(release.clone());
    let hwnd = WINDOW.load(Ordering::Acquire);
    let Some((title, text)) = notice.filter(|_| hwnd != 0) else {
        return;
    };
    let mut data = icon_data(HWND(hwnd as *mut _));
    data.uFlags = NIF_INFO;
    data.dwInfoFlags = NIIF_INFO;
    copy_text(&mut data.szInfoTitle, &title);
    copy_text(&mut data.szInfo, &text);
    // SAFETY: modifies our own icon; callable from any thread.
    if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
        log::warn!("cannot show the notification");
    }
}

/// Shows what plays in the icon's tip (`None`: nothing). Called for every
/// update of the track; the icon changes only when the title or the artist
/// does.
pub fn set_now_playing(now_playing: Option<&NowPlaying>) {
    let summary = now_playing.and_then(NowPlaying::summary);
    {
        let mut playing = PLAYING.lock().unwrap_or_else(PoisonError::into_inner);
        if *playing == summary {
            return;
        }
        *playing = summary;
    }
    let hwnd = WINDOW.load(Ordering::Acquire);
    if hwnd == 0 {
        return; // The icon gets the tip when it is added.
    }
    let mut data = icon_data(HWND(hwnd as *mut _));
    data.uFlags = NIF_TIP;
    // SAFETY: modifies our own icon; callable from any thread.
    if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
        log::warn!("cannot change the tray icon's tip");
    }
}

/// The icon's tip: the app's name, and what plays below it.
fn tip() -> String {
    match PLAYING
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_deref()
    {
        Some(track) => format!("FlexFlyout\n{track}"),
        None => "FlexFlyout".to_owned(),
    }
}

/// Copies `text` into a fixed buffer of UTF-16, keeping the terminating
/// zero. Text that does not fit is cut, never inside a character, and ends
/// with "…".
fn copy_text(slot: &mut [u16], text: &str) {
    let Some(room) = slot.len().checked_sub(1) else {
        return;
    };
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut used = if units.len() <= room {
        units.len()
    } else {
        // Room for the ellipsis, and not between the halves of a pair.
        let mut cut = room.saturating_sub(1);
        if cut > 0 && (0xD800..0xDC00).contains(&units[cut - 1]) {
            cut -= 1;
        }
        cut
    };
    slot[..used].copy_from_slice(&units[..used]);
    if used < units.len() && used < room {
        slot[used] = ELLIPSIS;
        used += 1;
    }
    slot[used] = 0;
}

/// "…" in UTF-16.
const ELLIPSIS: u16 = 0x2026;

/// Opens the page of the release on offer, if any.
fn open_update() {
    let url = UPDATE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|release| release.url.clone());
    if let Some(url) = url {
        log::info!("tray: opening {url}");
        open_url(&url);
    }
}

/// Deletes the icon (before the process ends; otherwise it stays in the
/// notification area until the mouse passes over it).
pub fn remove() {
    let hwnd = WINDOW.load(Ordering::Acquire);
    if hwnd != 0 {
        let data = icon_data(HWND(hwnd as *mut _));
        // SAFETY: deletes our own icon; callable from any thread.
        let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
    }
}

/// Opens `url` in the default browser.
pub fn open_url(url: &str) {
    let url = HSTRING::from(url);
    // SAFETY: a plain shell call with valid strings.
    let result = unsafe { ShellExecuteW(None, w!("open"), &url, None, None, SW_SHOWNORMAL) };
    // Values up to 32 are errors.
    if result.0 as usize <= 32 {
        log::warn!("cannot open {url}: {}", result.0 as usize);
    }
}

/// Opens `path` in the editor Windows associates with its extension, or in
/// Notepad when there is none (`.toml` has no association by default, and
/// opening it anyway would ask the user to pick an app).
pub fn open_in_editor(path: &Path) {
    let file = HSTRING::from(path.as_os_str());
    let extension = path
        .extension()
        .map(|ext| HSTRING::from(format!(".{}", ext.to_string_lossy())))
        .unwrap_or_default();
    // SAFETY: plain shell calls with valid strings; the size query writes
    // only to `size`.
    unsafe {
        let mut size = 0u32;
        // Fails for an unknown extension instead of naming the "Open with"
        // dialog, thanks to ASSOCF_INIT_IGNOREUNKNOWN.
        let associated = AssocQueryStringW(
            ASSOCF_INIT_IGNOREUNKNOWN,
            ASSOCSTR_EXECUTABLE,
            &extension,
            w!("open"),
            None,
            &mut size,
        )
        .is_ok();
        let result = if associated {
            ShellExecuteW(None, w!("open"), &file, None, None, SW_SHOWNORMAL)
        } else {
            ShellExecuteW(
                None,
                w!("open"),
                w!("notepad.exe"),
                &file,
                None,
                SW_SHOWNORMAL,
            )
        };
        // Values above 32 mean success.
        if result.0 as isize <= 32 {
            log::warn!("cannot open {}", path.display());
        }
    }
}

/// Creates the window and the icon, then runs the window's message loop.
fn run() -> anyhow::Result<()> {
    // SAFETY: registers a class with a valid window procedure and creates
    // a hidden window of it on this thread, which runs its message loop.
    unsafe {
        let instance = GetModuleHandleW(None).context("no module handle")?;
        let class = w!("FlexFlyoutTray");
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        });
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("FlexFlyout"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .context("cannot create the tray window")?;
        WINDOW.store(hwnd.0 as isize, Ordering::Release);
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(w!("TaskbarCreated")),
            Ordering::Release,
        );
        add_icon(hwnd);

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

/// Adds the icon to the notification area.
fn add_icon(hwnd: HWND) {
    let data = icon_data(hwnd);
    // SAFETY: `data` describes our window and icon.
    if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
        log::warn!("cannot add the tray icon");
    }
}

/// The description of our icon.
fn icon_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon(),
        ..Default::default()
    };
    copy_text(&mut data.szTip, &tip());
    data
}

/// The window procedure of the hidden window.
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_TRAY {
        // Without NOTIFYICON_VERSION_4, lparam is the mouse message.
        match lparam.0 as u32 {
            WM_LBUTTONUP => carry_out(TrayCommand::Show),
            WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
            NIN_BALLOONUSERCLICK => open_update(),
            _ => {}
        }
        return LRESULT(0);
    }
    if message != 0 && message == TASKBAR_CREATED.load(Ordering::Acquire) {
        add_icon(hwnd);
        return LRESULT(0);
    }
    // SAFETY: everything else goes to the default procedure.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// Shows the menu at the pointer and carries out the chosen command.
fn show_menu(hwnd: HWND) {
    let Some(tray) = TRAY.get() else {
        return;
    };
    let labels = tray
        .labels
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let text = |command: TrayCommand| match command {
        TrayCommand::Show => &labels.show,
        TrayCommand::Settings => &labels.settings,
        TrayCommand::OpenSettings => &labels.open_settings,
        TrayCommand::ReloadSettings => &labels.reload_settings,
        TrayCommand::ToggleAutostart => &labels.autostart,
        TrayCommand::Quit => &labels.quit,
    };
    // Read every time: the Task Manager can change it meanwhile.
    let autostart = super::autostart::is_enabled();
    // SAFETY: a menu built, shown and destroyed here, owned by our window.
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let update = UPDATE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|release| release.version.clone());
        if let Some(version) = update {
            let label = HSTRING::from(format!("{} ({version})…", labels.update));
            let _ = AppendMenuW(menu, MF_STRING, UPDATE_ID, &label);
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        }
        for (id, command) in MENU {
            let _ = match command {
                Some(command) => {
                    let label = HSTRING::from(text(command).as_str());
                    let checked = command == TrayCommand::ToggleAutostart && autostart;
                    let flags = if checked {
                        MF_STRING | MF_CHECKED
                    } else {
                        MF_STRING
                    };
                    AppendMenuW(menu, flags, id, &label)
                }
                None => AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()),
            };
        }
        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        // Needed for the menu to close when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            point.x,
            point.y,
            None,
            hwnd,
            None,
        );
        // Also part of the documented recipe for notification icon menus.
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        let chosen = chosen.0 as usize;
        if chosen == UPDATE_ID {
            open_update();
        } else if let Some((_, Some(command))) =
            MENU.iter().find(|(id, _)| *id == chosen && chosen != 0)
        {
            carry_out(*command);
        }
    }
}

/// Hands a command to the app.
fn carry_out(command: TrayCommand) {
    log::info!("tray: {command:?}");
    if let Some(tray) = TRAY.get() {
        (tray.on_command)(command);
    }
}

/// The app's icon at the size of the notification area, from the
/// executable's resources (ID 1, see build.rs), or the drawn one in a
/// build without resources.
fn icon() -> HICON {
    static ICON: OnceLock<isize> = OnceLock::new();
    let raw = *ICON.get_or_init(|| {
        // SAFETY: loads an icon resource of this executable; the size is the
        // small icon size for the current DPI, so Windows picks the closest
        // image of the ICO instead of scaling the large one.
        let loaded = unsafe {
            let module = GetModuleHandleW(None).ok();
            LoadImageW(
                module.map(|module| HINSTANCE(module.0)),
                // MAKEINTRESOURCE(1): the ID in place of a name.
                PCWSTR(std::ptr::without_provenance(1)),
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            )
        };
        match loaded {
            Ok(handle) => handle.0 as isize,
            Err(_) => drawn_icon(),
        }
    });
    HICON(raw as *mut _)
}

/// A stand-in for builds without the icon resource (cross builds from
/// Linux): three rising bars (a volume meter) on a blue rounded square,
/// 32 × 32 with smooth edges.
fn drawn_icon() -> isize {
    const SIZE: usize = 32;
    let mut bgra = vec![0u8; SIZE * SIZE * 4];
    for y in 0..SIZE {
        for x in 0..SIZE {
            // Coverage of each shape from 4 × 4 samples per pixel.
            let (mut back, mut bar) = (0u32, 0u32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = x as f32 + (sx as f32 + 0.5) / 4.0;
                    let py = y as f32 + (sy as f32 + 0.5) / 4.0;
                    back += u32::from(in_rounded_square(px, py, SIZE as f32, 7.0));
                    bar += u32::from(in_bars(px, py));
                }
            }
            let (back, bar) = (back as f32 / 16.0, bar as f32 / 16.0);
            // White bars over blue (90, 160, 255), premultiplied.
            let color = |blue_part: f32| (blue_part * (1.0 - bar) + 255.0 * bar) * back;
            let i = (y * SIZE + x) * 4;
            bgra[i] = color(255.0) as u8;
            bgra[i + 1] = color(160.0) as u8;
            bgra[i + 2] = color(90.0) as u8;
            bgra[i + 3] = (back * 255.0) as u8;
        }
    }
    // A 32-bit icon takes its transparency from the alpha channel; the
    // AND mask must exist but is ignored when it is all zeros.
    let mask = [0u8; SIZE * SIZE / 8];
    // SAFETY: both buffers have the sizes CreateIcon expects.
    unsafe {
        CreateIcon(
            None,
            SIZE as i32,
            SIZE as i32,
            1,
            32,
            mask.as_ptr(),
            bgra.as_ptr(),
        )
    }
    .map(|icon| icon.0 as isize)
    .unwrap_or_else(|err| {
        log::warn!("cannot create the tray icon image: {err}");
        0
    })
}

/// Whether a point lies in a square of side `size` with rounded corners.
fn in_rounded_square(x: f32, y: f32, size: f32, radius: f32) -> bool {
    let cx = x.clamp(radius, size - radius);
    let cy = y.clamp(radius, size - radius);
    (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
}

/// Whether a point lies in one of the three bars.
fn in_bars(x: f32, y: f32) -> bool {
    // Left edge and top of each bar; all end at y = 25 and are 5 wide.
    [(7.0, 17.0), (13.5, 12.0), (20.0, 7.0)]
        .iter()
        .any(|&(left, top)| (left..left + 5.0).contains(&x) && (top..25.0).contains(&y))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The text in a buffer, up to its terminating zero.
    fn read(slot: &[u16]) -> String {
        let end = slot.iter().position(|&unit| unit == 0).expect("terminated");
        String::from_utf16(&slot[..end]).expect("whole characters")
    }

    #[test]
    fn texts_are_cut_to_their_buffers() {
        let mut slot = [0xFFFF_u16; 8];
        copy_text(&mut slot, "Song");
        assert_eq!(read(&slot), "Song");
        // Seven units fit; a longer text keeps six and an ellipsis.
        copy_text(&mut slot, "A longer title");
        assert_eq!(read(&slot), "A long…");
        // Never between the halves of a surrogate pair (𝄞 takes two).
        copy_text(&mut slot, "abcde𝄞fg");
        assert_eq!(read(&slot), "abcde…");
        copy_text(&mut slot, "abcdefg");
        assert_eq!(read(&slot), "abcdefg");
        let mut tiny = [0xFFFF_u16; 1];
        copy_text(&mut tiny, "x");
        assert_eq!(read(&tiny), "");
    }
}
