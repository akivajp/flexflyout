//! The settings file (TOML): what it contains, its defaults, and reading it.
//!
//! Every key is optional; a missing key takes its default, so an old file
//! keeps working when new keys arrive. Keys the program does not know (a typo
//! or a key from a newer version) are not an error but are reported, so that
//! the host can log them. Values out of their range are clamped, also with a
//! warning, so a bad number never stops the flyout.
//!
//! Settings changed in a window are written back with [`update_text`] and
//! [`Config::save`], which change only those values and keep the user's
//! comments and layout.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize};

use crate::Timing;

/// The settings file written on the first start: the defaults, with a
/// comment for every key.
pub const DEFAULT_FILE: &str = include_str!("../default-config.toml");

/// Allowed range of [`Display::scale`].
pub const SCALE_RANGE: (f32, f32) = (0.25, 4.0);

/// Longest allowed duration of a fade (the hold has no upper limit).
pub const MAX_FADE_MS: u64 = 10_000;

/// All settings.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// Settings that are not about the look.
    pub general: General,
    /// Where the flyout appears and how it looks.
    pub display: Display,
    /// How long the flyout stays and how it fades.
    pub timing: TimingConfig,
}

/// Settings that are not about the look.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct General {
    /// Language of the texts: `"auto"` (the system's) or a code such as
    /// `"en"` or `"ja"`.
    pub language: String,
    /// What to do with the operating system's own volume indicator.
    pub native_osd: NativeOsd,
    /// Whether the flyout also shows when the player starts or pauses
    /// (besides a new track, a volume change and the keys).
    pub show_on_play_pause: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            language: "auto".to_owned(),
            native_osd: NativeOsd::default(),
            show_on_play_pause: true,
        }
    }
}

impl General {
    /// The language code to use, or `None` to follow the system.
    pub fn language(&self) -> Option<&str> {
        let lang = self.language.trim();
        (!lang.is_empty() && !lang.eq_ignore_ascii_case("auto")).then_some(lang)
    }
}

/// What to do with the operating system's own volume indicator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativeOsd {
    /// Leave it, and place the flyout so that the two do not overlap.
    #[default]
    Avoid,
    /// Hide it (only the volume one where the system allows that).
    Hide,
    /// Leave it and ignore it (the two may overlap).
    Keep,
}

/// Where the flyout appears and how it looks.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Display {
    /// The monitor to show the flyout on.
    pub monitor: Monitor,
    /// The point of the monitor's work area the flyout is placed at.
    pub anchor: Anchor,
    /// Horizontal distance from the anchor in pixels (at 100% scaling of
    /// the monitor). At a left or right anchor it is measured inwards from
    /// that edge; at a centered one, positive moves right.
    pub offset_x: i32,
    /// Vertical distance from the anchor, like `offset_x`: inwards from the
    /// top or bottom edge, or downwards at a centered anchor.
    pub offset_y: i32,
    /// Size of the flyout relative to its normal size (on top of the
    /// monitor's own scaling).
    pub scale: f32,
    /// Opacity of the panel's background, from 0.0 (invisible) to 1.0.
    /// The text and the buttons stay opaque.
    pub opacity: f32,
    /// Whether dragging a pinned flyout sets where it appears from then on
    /// (the monitor, anchor and offsets above are rewritten), rather than
    /// moving it until it hides.
    pub remember_drag: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            monitor: Monitor::default(),
            anchor: Anchor::default(),
            offset_x: 24,
            offset_y: 24,
            scale: 1.0,
            opacity: 0.92,
            remember_drag: false,
        }
    }
}

/// The monitor to show the flyout on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Monitor {
    /// The primary monitor (the one with the taskbar's clock on Windows).
    #[default]
    Primary,
    /// The monitor the mouse pointer is on.
    Cursor,
    /// A monitor by number, from 1, in the order the system lists them.
    /// If it does not exist, the primary monitor is used.
    Number(u32),
}

impl Serialize for Monitor {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Primary => serializer.serialize_str("primary"),
            Self::Cursor => serializer.serialize_str("cursor"),
            Self::Number(n) => serializer.serialize_u32(*n),
        }
    }
}

impl<'de> Deserialize<'de> for Monitor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// Accepts `"primary"`, `"cursor"` or a number from 1.
        struct MonitorVisitor;

        impl Visitor<'_> for MonitorVisitor {
            type Value = Monitor;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(r#""primary", "cursor" or a monitor number from 1"#)
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Monitor, E> {
                match value {
                    "primary" => Ok(Monitor::Primary),
                    "cursor" => Ok(Monitor::Cursor),
                    _ => Err(E::invalid_value(de::Unexpected::Str(value), &self)),
                }
            }

            fn visit_i64<E: de::Error>(self, value: i64) -> Result<Monitor, E> {
                // TOML integers arrive as i64.
                u32::try_from(value)
                    .ok()
                    .filter(|&n| n >= 1)
                    .map(Monitor::Number)
                    .ok_or_else(|| E::invalid_value(de::Unexpected::Signed(value), &self))
            }

            fn visit_u64<E: de::Error>(self, value: u64) -> Result<Monitor, E> {
                self.visit_i64(i64::try_from(value).unwrap_or(-1))
            }
        }

        deserializer.deserialize_any(MonitorVisitor)
    }
}

/// One of nine points of a monitor's work area: the corners, the middles of
/// the edges and the center.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Anchor {
    /// The top-left corner.
    TopLeft,
    /// The middle of the top edge.
    Top,
    /// The top-right corner.
    #[default]
    TopRight,
    /// The middle of the left edge.
    Left,
    /// The center.
    Center,
    /// The middle of the right edge.
    Right,
    /// The bottom-left corner.
    BottomLeft,
    /// The middle of the bottom edge.
    Bottom,
    /// The bottom-right corner.
    BottomRight,
}

/// How long the flyout stays and how it fades, in milliseconds.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct TimingConfig {
    /// From invisible to fully opaque (0 = appear at once).
    pub fade_in_ms: u64,
    /// How long the flyout stays fully opaque after the last trigger.
    pub hold_ms: u64,
    /// From fully opaque to invisible (0 = vanish at once).
    pub fade_out_ms: u64,
    /// How the flyout comes and goes, besides fading.
    pub animation: Animation,
}

/// How the flyout comes and goes, besides fading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Animation {
    /// It slides in from the nearest edge of the screen while fading in,
    /// and back out while fading out.
    #[default]
    Slide,
    /// It only fades, in place.
    Fade,
}

impl Default for TimingConfig {
    fn default() -> Self {
        let timing = Timing::default();
        Self {
            fade_in_ms: timing.fade_in.as_millis() as u64,
            hold_ms: timing.hold.as_millis() as u64,
            fade_out_ms: timing.fade_out.as_millis() as u64,
            animation: Animation::default(),
        }
    }
}

impl TimingConfig {
    /// The durations for [`crate::FlyoutTimer`].
    pub fn timing(&self) -> Timing {
        Timing {
            fade_in: Duration::from_millis(self.fade_in_ms),
            hold: Duration::from_millis(self.hold_ms),
            fade_out: Duration::from_millis(self.fade_out_ms),
        }
    }
}

/// Settings read from a file, and what was wrong with them but did not stop
/// them from being used.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    /// The settings, with every problem below already corrected.
    pub config: Config,
    /// Unknown keys and clamped values, one sentence each, for the log.
    pub warnings: Vec<String>,
}

/// Why the settings could not be read. The host keeps the settings it has.
#[derive(Debug)]
pub enum Error {
    /// The file could not be read.
    Io(std::io::Error),
    /// The file is not valid TOML, or a value has the wrong type or an
    /// unknown name (the message says where).
    Parse(toml::de::Error),
    /// The file could not be written.
    Write(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "cannot read the settings: {err}"),
            Self::Parse(err) => write!(f, "invalid settings: {err}"),
            Self::Write(err) => write!(f, "cannot write the settings: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) | Self::Write(err) => Some(err),
            Self::Parse(err) => Some(err),
        }
    }
}

impl Config {
    /// Reads settings from the text of a TOML file.
    pub fn parse(text: &str) -> Result<Loaded, Error> {
        let mut warnings = Vec::new();
        let deserializer = toml::Deserializer::parse(text).map_err(Error::Parse)?;
        // serde_ignored reports every key that the types above skip.
        let mut config: Config = serde_ignored::deserialize(deserializer, |path| {
            warnings.push(format!("unknown setting `{path}` (ignored)"));
        })
        .map_err(Error::Parse)?;
        config.clamp(&mut warnings);
        Ok(Loaded { config, warnings })
    }

    /// Reads settings from a file.
    pub fn load(path: &Path) -> Result<Loaded, Error> {
        let text = std::fs::read_to_string(path).map_err(Error::Io)?;
        Self::parse(&text)
    }

    /// Writes [`DEFAULT_FILE`] to `path`, creating its folder, unless the
    /// file exists. Returns whether it was written.
    pub fn write_default_if_missing(path: &Path) -> std::io::Result<bool> {
        if path.exists() {
            return Ok(false);
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // `create_new` so that a file made in the meantime is never replaced.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => {
                use std::io::Write as _;
                file.write_all(DEFAULT_FILE.as_bytes())?;
                Ok(true)
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// Writes the values that differ between `before` and `self` into the
    /// file at `path` (see [`update_text`]), or into a new file with the
    /// commented defaults if there is none. The file is replaced in one
    /// step, so that a reader never sees half of it.
    pub fn save(&self, path: &Path, before: &Config) -> Result<(), Error> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => DEFAULT_FILE.to_owned(),
            Err(err) => return Err(Error::Io(err)),
        };
        let text = update_text(&text, before, self)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(Error::Write)?;
        }
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        std::fs::write(&temporary, text).map_err(Error::Write)?;
        std::fs::rename(&temporary, path).map_err(Error::Write)
    }

    /// Brings every value into its range, with a warning for each change.
    fn clamp(&mut self, warnings: &mut Vec<String>) {
        let display = &mut self.display;
        clamp_f32(
            "display.scale",
            &mut display.scale,
            SCALE_RANGE,
            Display::default().scale,
            warnings,
        );
        clamp_f32(
            "display.opacity",
            &mut display.opacity,
            (0.0, 1.0),
            Display::default().opacity,
            warnings,
        );
        let timing = &mut self.timing;
        for (name, value) in [
            ("timing.fade_in_ms", &mut timing.fade_in_ms),
            ("timing.fade_out_ms", &mut timing.fade_out_ms),
        ] {
            if *value > MAX_FADE_MS {
                warnings.push(format!(
                    "`{name}` = {value} is longer than {MAX_FADE_MS}; using {MAX_FADE_MS}"
                ));
                *value = MAX_FADE_MS;
            }
        }
    }
}

/// Writes the values that differ between `before` and `after` into `text`
/// (the contents of a settings file) and returns the new text.
///
/// Only those values change: comments, the order of the keys and every
/// other value (also ones edited elsewhere since `before` was read) stay as
/// they are. A key missing from the text is added to its section.
pub fn update_text(text: &str, before: &Config, after: &Config) -> Result<String, Error> {
    let mut document: toml_edit::DocumentMut =
        text.parse().map_err(|err: toml_edit::TomlError| {
            // Report it the way reading the file would.
            match toml::from_str::<toml::Table>(text) {
                Err(parse) => Error::Parse(parse),
                Ok(_) => Error::Write(std::io::Error::other(err.to_string())),
            }
        })?;
    let tables = |config: &Config| -> toml::Table {
        // Plain data: serializing it cannot fail.
        toml::Table::try_from(config).unwrap_or_default()
    };
    let (before, after) = (tables(before), tables(after));
    for (section, keys) in &after {
        let Some(keys) = keys.as_table() else {
            continue;
        };
        for (key, value) in keys {
            let old = before.get(section).and_then(|keys| keys.get(key));
            if old == Some(value) {
                continue;
            }
            if !document.contains_table(section) {
                document[section] = toml_edit::table();
            }
            let new = edit_value(value);
            match document[section]
                .get_mut(key)
                .and_then(|item| item.as_value_mut())
            {
                // Keep the spaces and the comment around the old value.
                Some(old) => {
                    let decor = old.decor().clone();
                    *old = new;
                    *old.decor_mut() = decor;
                }
                None => document[section][key] = toml_edit::value(new),
            }
        }
    }
    Ok(document.to_string())
}

/// A settings value as `toml_edit` writes it. Every number with a fraction
/// in the settings is an `f32`: written as the shortest decimal that reads
/// back as the same `f32` (0.92 rather than 0.9200000166893005).
fn edit_value(value: &toml::Value) -> toml_edit::Value {
    match value {
        toml::Value::String(text) => text.as_str().into(),
        toml::Value::Integer(number) => (*number).into(),
        toml::Value::Float(number) => {
            let shortest = (*number as f32).to_string();
            shortest.parse::<f64>().unwrap_or(*number).into()
        }
        toml::Value::Boolean(flag) => (*flag).into(),
        // Not used by the settings; written as TOML would.
        other => other
            .to_string()
            .parse()
            .unwrap_or_else(|_| other.to_string().into()),
    }
}

/// Clamps `value` into `(min, max)`; a value that is not a number at all
/// (NaN, which TOML allows as `nan`) takes `default`.
fn clamp_f32(
    name: &str,
    value: &mut f32,
    (min, max): (f32, f32),
    default: f32,
    warnings: &mut Vec<String>,
) {
    let fixed = if value.is_nan() {
        default
    } else {
        value.clamp(min, max)
    };
    // Compare the bits so that a NaN, which equals nothing, also counts.
    if fixed.to_bits() != value.to_bits() {
        warnings.push(format!(
            "`{name}` = {value} is outside {min}..={max}; using {fixed}"
        ));
        *value = fixed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_file_has_the_default_values() {
        let loaded = Config::parse(DEFAULT_FILE).unwrap();
        assert_eq!(loaded.config, Config::default());
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn every_key_of_the_default_config_is_in_the_default_file() {
        // The default file is written by hand; serializing the defaults
        // and parsing the file must give the same set of keys, so that no
        // setting goes undocumented.
        let keys = |value: toml::Value| -> Vec<String> {
            let mut keys = Vec::new();
            for (section, table) in value.as_table().unwrap() {
                for key in table.as_table().unwrap().keys() {
                    keys.push(format!("{section}.{key}"));
                }
            }
            keys.sort();
            keys
        };
        let documented: toml::Value = toml::from_str(DEFAULT_FILE).unwrap();
        let defaults = toml::Value::try_from(Config::default()).unwrap();
        assert_eq!(keys(documented), keys(defaults));
    }

    #[test]
    fn missing_keys_take_their_defaults() {
        let loaded = Config::parse("[display]\nanchor = \"bottom-left\"\n").unwrap();
        let expected = Config {
            display: Display {
                anchor: Anchor::BottomLeft,
                ..Display::default()
            },
            ..Config::default()
        };
        assert_eq!(loaded.config, expected);
        assert!(loaded.warnings.is_empty());
        assert_eq!(Config::parse("").unwrap().config, Config::default());
    }

    #[test]
    fn unknown_keys_are_reported_but_not_fatal() {
        let loaded = Config::parse(
            "[display]\nopactiy = 0.5\n[sound]\nbeep = true\n[timing]\nhold_ms = 5000\n",
        )
        .unwrap();
        assert_eq!(loaded.config.timing.hold_ms, 5000);
        assert_eq!(loaded.config.display.opacity, Display::default().opacity);
        assert_eq!(
            loaded.warnings,
            [
                "unknown setting `display.opactiy` (ignored)",
                "unknown setting `sound` (ignored)",
            ]
        );
    }

    #[test]
    fn monitors_by_name_and_by_number() {
        let monitor = |value: &str| {
            Config::parse(&format!("[display]\nmonitor = {value}\n"))
                .map(|loaded| loaded.config.display.monitor)
        };
        assert_eq!(monitor(r#""primary""#).unwrap(), Monitor::Primary);
        assert_eq!(monitor(r#""cursor""#).unwrap(), Monitor::Cursor);
        assert_eq!(monitor("2").unwrap(), Monitor::Number(2));
        // Numbers start at 1, as people count monitors.
        assert!(monitor("0").is_err());
        assert!(monitor("-1").is_err());
        assert!(monitor(r#""second""#).is_err());
        assert!(monitor("1.5").is_err());
    }

    #[test]
    fn wrong_values_are_errors_that_say_where() {
        let err = Config::parse("[display]\nanchor = \"top-middle\"\n").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("top-middle"), "{message}");
        assert!(message.contains("line 2"), "{message}");
        assert!(Config::parse("[timing]\nhold_ms = \"long\"\n").is_err());
        assert!(Config::parse("[display\n").is_err());
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let loaded =
            Config::parse("[display]\nscale = 10.0\nopacity = nan\n[timing]\nfade_in_ms = 60000\n")
                .unwrap();
        let config = loaded.config;
        assert_eq!(config.display.scale, SCALE_RANGE.1);
        assert_eq!(config.display.opacity, Display::default().opacity);
        assert_eq!(config.timing.fade_in_ms, MAX_FADE_MS);
        assert_eq!(loaded.warnings.len(), 3, "{:?}", loaded.warnings);
        // A value inside its range is left alone.
        let loaded = Config::parse("[display]\nopacity = 0.0\n").unwrap();
        assert_eq!(loaded.config.display.opacity, 0.0);
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn language_auto_means_the_system() {
        let lang = |language: &str| {
            General {
                language: language.to_owned(),
                ..General::default()
            }
            .language()
            .map(str::to_owned)
        };
        assert_eq!(lang("auto"), None);
        assert_eq!(lang("Auto"), None);
        assert_eq!(lang(""), None);
        assert_eq!(lang(" ja "), Some("ja".to_owned()));
    }

    #[test]
    fn updating_keeps_the_comments_and_the_other_values() {
        let text = "# My settings.\n[display]\nscale = 1.0 # bigger later\nanchor = \"top\"\n\n[timing]\nhold_ms = 5000\n";
        let before = Config::parse(text).unwrap().config;
        let mut after = before.clone();
        after.display.scale = 1.25;
        after.display.opacity = 0.8;
        after.general.native_osd = NativeOsd::Hide;
        // Someone changed the hold time in the file meanwhile; the window
        // did not touch it, so it stays.
        let edited = text.replace("hold_ms = 5000", "hold_ms = 7000");
        let updated = update_text(&edited, &before, &after).unwrap();
        assert!(updated.starts_with("# My settings.\n"), "{updated}");
        assert!(
            updated.contains("scale = 1.25 # bigger later\n"),
            "{updated}"
        );
        assert!(updated.contains("opacity = 0.8\n"), "{updated}");
        assert!(updated.contains("anchor = \"top\"\n"), "{updated}");
        assert!(updated.contains("hold_ms = 7000"), "{updated}");
        let config = Config::parse(&updated).unwrap().config;
        assert_eq!(config.display.scale, 1.25);
        assert_eq!(config.display.opacity, 0.8);
        assert_eq!(config.general.native_osd, NativeOsd::Hide);
        assert_eq!(config.timing.hold_ms, 7000);
        // Nothing changed, nothing written.
        assert_eq!(update_text(text, &before, &before).unwrap(), text);
    }

    #[test]
    fn saving_writes_the_default_file_with_the_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flexflyout").join("config.toml");
        let mut config = Config::default();
        config.display.monitor = Monitor::Number(2);
        config.save(&path, &Config::default()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        // The commented file, with the one change.
        assert!(text.starts_with("# FlexFlyout settings."), "{text}");
        assert!(text.contains("monitor = 2\n"), "{text}");
        assert_eq!(Config::load(&path).unwrap().config, config);
        // A broken file is left alone.
        std::fs::write(&path, "[display\n").unwrap();
        assert!(matches!(
            Config::default().save(&path, &config),
            Err(Error::Parse(_))
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[display\n");
    }

    #[test]
    fn the_default_file_is_written_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new folder").join("config.toml");
        assert!(Config::write_default_if_missing(&path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_FILE);
        // The user's own file is never replaced.
        std::fs::write(&path, "[timing]\nhold_ms = 1000\n").unwrap();
        assert!(!Config::write_default_if_missing(&path).unwrap());
        assert_eq!(Config::load(&path).unwrap().config.timing.hold_ms, 1000);
    }
}
