//! Translations of the GUI texts (Fluent, through i18n-embed).
//!
//! The `.ftl` files under `i18n/<language>/` are embedded in the executable.
//! English is the fallback: a text missing in another language shows in
//! English, and `fl!` refuses to build when an ID is missing in English.

use anyhow::Context as _;
use flexflyout_ui::Labels;
use i18n_embed::DesktopLanguageRequester;
use i18n_embed::fluent::{FluentLanguageLoader, fluent_language_loader};
use i18n_embed::unic_langid::LanguageIdentifier;
use i18n_embed_fl::fl;
use rust_embed::RustEmbed;

/// The translation files, embedded at build time.
#[derive(RustEmbed)]
#[folder = "i18n"]
struct Localizations;

/// Loads the translations for `lang` (such as "ja"), or for the system's
/// languages when `lang` is `None`.
pub fn load(lang: Option<&str>) -> anyhow::Result<FluentLanguageLoader> {
    let loader: FluentLanguageLoader = fluent_language_loader!();
    let requested: Vec<LanguageIdentifier> = match lang {
        Some(lang) => vec![
            lang.parse()
                .with_context(|| format!("not a language code: {lang}"))?,
        ],
        None => DesktopLanguageRequester::requested_languages(),
    };
    let selected = i18n_embed::select(&loader, &Localizations, &requested)
        .context("cannot load the translations")?;
    // Fluent wraps every inserted value in Unicode isolation marks by default.
    // egui has no glyphs for them and would draw boxes.
    loader.set_use_isolating(false);
    log::info!("languages requested: {requested:?}; selected: {selected:?}");
    Ok(loader)
}

/// Texts of the notification area icon's menu.
#[derive(Clone, Debug)]
// Unused where FlexFlyout has no host yet (only Windows and Linux do).
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub struct TrayLabels {
    /// Show the flyout.
    pub show: String,
    /// Open the settings window.
    pub settings: String,
    /// Open the settings file.
    pub open_settings: String,
    /// Read the settings again.
    pub reload_settings: String,
    /// Start with Windows, or at login on Linux (a check box).
    pub autostart: String,
    /// End the app.
    pub quit: String,
    /// Open the page of a newer release (followed by its version).
    pub update: String,
}

/// Texts of the tray menu in the loaded language.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
pub fn tray_labels(loader: &FluentLanguageLoader) -> TrayLabels {
    TrayLabels {
        show: fl!(loader, "tray-show"),
        settings: fl!(loader, "tray-settings"),
        open_settings: fl!(loader, "tray-open-settings"),
        reload_settings: fl!(loader, "tray-reload-settings"),
        #[cfg(windows)]
        autostart: fl!(loader, "tray-start-with-windows"),
        #[cfg(not(windows))]
        autostart: fl!(loader, "tray-start-at-login"),
        quit: fl!(loader, "tray-quit"),
        update: fl!(loader, "tray-update"),
    }
}

/// Texts of the flyout panel in the loaded language.
pub fn labels(loader: &FluentLanguageLoader) -> Labels {
    Labels {
        nothing_playing: fl!(loader, "nothing-playing"),
        muted: fl!(loader, "muted"),
        previous: fl!(loader, "previous"),
        play_pause: fl!(loader, "play-pause"),
        next: fl!(loader, "next"),
        position: fl!(loader, "position"),
        shuffle: fl!(loader, "shuffle"),
        repeat: fl!(loader, "repeat"),
        volume: fl!(loader, "volume"),
        mute: fl!(loader, "mute"),
        pin: fl!(loader, "pin"),
        close: fl!(loader, "close"),
        settings: fl!(loader, "settings"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Message IDs defined in one embedded `.ftl` file.
    fn ids(path: &str) -> BTreeSet<String> {
        let file = Localizations::get(path).unwrap_or_else(|| panic!("{path} is not embedded"));
        let text = std::str::from_utf8(&file.data).expect("UTF-8");
        text.lines()
            .filter(|line| !line.starts_with([' ', '#', '.']) && line.contains('='))
            .map(|line| line.split('=').next().unwrap_or_default().trim().to_owned())
            .collect()
    }

    /// Every language translates exactly the English messages: none missing
    /// (it would silently show in English) and none left over (a typo).
    #[test]
    fn every_language_has_the_english_ids() {
        let english = ids("en/flexflyout.ftl");
        assert!(!english.is_empty());
        let others: Vec<String> = Localizations::iter()
            .map(|path| path.into_owned())
            .filter(|path| path != "en/flexflyout.ftl")
            .collect();
        assert!(!others.is_empty(), "no translations besides English");
        for path in others {
            assert_eq!(ids(&path), english, "{path}");
        }
    }

    #[test]
    fn loads_japanese() {
        let loader = load(Some("ja")).expect("load");
        assert_eq!(labels(&loader).next, "次へ");
        assert_eq!(tray_labels(&loader).quit, "終了");
    }

    #[test]
    fn unknown_languages_fall_back_to_english() {
        let loader = load(Some("xx")).expect("load");
        assert_eq!(labels(&loader).next, "Next");
    }
}
