//! The notification area icon (a StatusNotifierItem, which KDE Plasma and
//! most other desktops show; GNOME needs an extension) and its menu, the
//! same as on Windows.

use std::sync::{Mutex, OnceLock, PoisonError};

use flexflyout_core::NowPlaying;
use flexflyout_core::update::Release;
use ksni::blocking::TrayMethods as _;

use crate::i18n::TrayLabels;
use crate::message::TrayCommand;

/// The running icon, to change its texts.
static HANDLE: OnceLock<ksni::blocking::Handle<Tray>> = OnceLock::new();

/// A newer release offered before the icon was there.
static UPDATE: Mutex<Option<Release>> = Mutex::new(None);

/// What plays, in one line (kept also before the icon is there).
static PLAYING: Mutex<Option<String>> = Mutex::new(None);

/// The icon's state, read by the D-Bus service.
struct Tray {
    labels: TrayLabels,
    /// A newer release, offered at the top of the menu.
    update: Option<Release>,
    /// What plays, shown in the tooltip.
    playing: Option<String>,
    on_command: Box<dyn Fn(TrayCommand) + Send + Sync>,
    icon: Vec<ksni::Icon>,
}

/// Adds the icon. `on_command` is called on the tray's thread.
pub fn spawn(labels: TrayLabels, on_command: impl Fn(TrayCommand) + Send + Sync + 'static) {
    let tray = Tray {
        labels,
        update: UPDATE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        playing: PLAYING
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone(),
        on_command: Box::new(on_command),
        icon: icon(),
    };
    match tray.spawn() {
        Ok(handle) => {
            let _ = HANDLE.set(handle);
        }
        // No StatusNotifierWatcher (GNOME without an extension): the
        // flyout still works, only without the icon.
        Err(err) => log::warn!("no tray icon: {err}"),
    }
}

/// Changes the menu's texts (after the language changed).
pub fn set_labels(labels: TrayLabels) {
    if let Some(handle) = HANDLE.get() {
        handle.update(|tray| tray.labels = labels);
    }
}

/// Shows what plays in the tooltip (`None`: nothing). Called for every
/// update of the track; the icon changes only when the title or the artist
/// does.
pub fn set_now_playing(now_playing: Option<&NowPlaying>) {
    let summary = now_playing.and_then(NowPlaying::summary);
    {
        let mut playing = PLAYING.lock().unwrap_or_else(PoisonError::into_inner);
        if *playing == summary {
            return;
        }
        playing.clone_from(&summary);
    }
    if let Some(handle) = HANDLE.get() {
        handle.update(move |tray| tray.playing = summary);
    }
}

/// Offers `release` at the top of the menu.
pub fn offer_update(release: &Release) {
    *UPDATE.lock().unwrap_or_else(PoisonError::into_inner) = Some(release.clone());
    if let Some(handle) = HANDLE.get() {
        let release = release.clone();
        handle.update(move |tray| tray.update = Some(release));
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "flexflyout".into()
    }

    fn title(&self) -> String {
        "FlexFlyout".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "FlexFlyout".into(),
            description: self.playing.clone().unwrap_or_default(),
            ..Default::default()
        }
    }

    /// A left click shows the flyout, as on Windows.
    fn activate(&mut self, _x: i32, _y: i32) {
        (self.on_command)(TrayCommand::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{CheckmarkItem, MenuItem, StandardItem};
        let item = |label: &str, command: TrayCommand| -> MenuItem<Self> {
            StandardItem {
                label: label.to_owned(),
                activate: Box::new(move |tray: &mut Self| (tray.on_command)(command)),
                ..Default::default()
            }
            .into()
        };
        let labels = &self.labels;
        let mut menu = Vec::new();
        if let Some(release) = &self.update {
            let url = release.url.clone();
            menu.push(
                StandardItem {
                    label: format!("{} ({})…", labels.update, release.version),
                    activate: Box::new(move |_: &mut Self| super::desktop::open_url(&url)),
                    ..Default::default()
                }
                .into(),
            );
            menu.push(MenuItem::Separator);
        }
        menu.extend([
            item(&labels.show, TrayCommand::Show),
            MenuItem::Separator,
            item(&labels.settings, TrayCommand::Settings),
            item(&labels.open_settings, TrayCommand::OpenSettings),
            item(&labels.reload_settings, TrayCommand::ReloadSettings),
            MenuItem::Separator,
            CheckmarkItem {
                label: labels.autostart.clone(),
                // Read every time the menu is built.
                checked: super::desktop::autostart_enabled(),
                activate: Box::new(|tray: &mut Self| {
                    (tray.on_command)(TrayCommand::ToggleAutostart);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            item(&labels.quit, TrayCommand::Quit),
        ]);
        menu
    }
}

/// The app's icon in the sizes panels ask for, from the embedded PNG.
fn icon() -> Vec<ksni::Icon> {
    let Ok(image) = image::load_from_memory(include_bytes!("../../../assets/flexflyout.png"))
    else {
        return Vec::new();
    };
    [22, 32, 48, 64]
        .into_iter()
        .map(|size| {
            let rgba = image
                .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
                .to_rgba8();
            // ARGB in network byte order.
            let data = rgba
                .pixels()
                .flat_map(|p| [p[3], p[0], p[1], p[2]])
                .collect();
            ksni::Icon {
                width: size as i32,
                height: size as i32,
                data,
            }
        })
        .collect()
}
