//! The desktop's colors, for the theme and the accent that follow the
//! system:
//!
//! - light or dark: the "color-scheme" setting of xdg-desktop-portal, which
//!   KDE Plasma, GNOME and others provide;
//! - the windows' background, text and accent: KDE's color scheme, from
//!   `kdeglobals` (the portal does not tell the first two, and its accent
//!   can lag behind), so that the panel looks like Plasma's own popups;
//! - elsewhere, the accent: the portal's "accent-color" setting.
//!
//! Read at the start, then again after each of the portal's change signals
//! (KDE's portal also signals changes of `kdeglobals`). Without a portal,
//! only `kdeglobals` is read once, and without that either, the flyout
//! keeps its own colors.

use std::path::PathBuf;

use anyhow::Context as _;
use flexflyout_core::{DesktopColors, Theme};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

/// The portal's settings, and the ones wanted.
const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";
const ACCENT_KEY: &str = "accent-color";
/// The namespaces KDE's portal uses for `kdeglobals` (one per group).
const KDE_NAMESPACE: &str = "org.kde.kdeglobals";
/// How long the change signals must have stopped before the colors are
/// read again.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(500);

/// Starts following the desktop's colors: `on_theme` gets dark or light,
/// `on_colors` the desktop's own colors, each at once and after every
/// change.
pub fn spawn(
    on_theme: impl Fn(Theme) + Send + 'static,
    on_colors: impl Fn(DesktopColors) + Send + 'static,
) {
    let spawned = std::thread::Builder::new()
        .name("theme".into())
        .spawn(move || {
            if let Err(err) = run(&on_theme, on_colors) {
                log::info!("the desktop's color scheme is not known: {err:#}");
            }
        });
    if let Err(err) = spawned {
        log::warn!("cannot follow the desktop's color scheme: {err}");
    }
}

/// Reports the colors, then every change of them.
fn run(
    on_theme: &impl Fn(Theme),
    on_colors: impl Fn(DesktopColors) + Send + 'static,
) -> anyhow::Result<()> {
    let portal = Connection::session()
        .context("no D-Bus session bus")
        .and_then(|connection| Ok(Proxy::new(&connection, DESTINATION, PATH, INTERFACE)?));
    let proxy = match portal {
        Ok(proxy) => proxy,
        Err(err) => {
            // Still KDE's colors, once, if there are any.
            report_colors(&on_colors, None);
            return Err(err);
        }
    };
    // Listening before reading, so that a change in between is not lost.
    let changes = proxy.receive_signal("SettingChanged")?;
    match read(&proxy, KEY)
        .and_then(|value| scheme(&value).context("the color scheme is not a number"))
    {
        Ok(theme) => {
            log::info!("the desktop's color scheme is {theme:?}");
            on_theme(theme);
        }
        Err(err) => log::info!("the desktop's color scheme is not known: {err:#}"),
    }
    let colors = report_colors(&on_colors, accent(&proxy));
    // The colors are read again on their own thread once the signals have
    // settled: the portal signals before Plasma has written `kdeglobals`,
    // and a new color scheme comes as a burst of signals.
    let (changed, settle) = std::sync::mpsc::channel::<()>();
    let reader = proxy.clone();
    std::thread::Builder::new()
        .name("desktop-colors".into())
        .spawn(move || follow_colors(&reader, &settle, colors, &on_colors))?;
    for message in changes {
        let Ok((namespace, key, value)) =
            message.body().deserialize::<(String, String, OwnedValue)>()
        else {
            continue;
        };
        if namespace == NAMESPACE
            && key == KEY
            && let Some(theme) = scheme(&value)
        {
            log::info!("the desktop's color scheme is now {theme:?}");
            on_theme(theme);
        }
        if (namespace == NAMESPACE || namespace.starts_with(KDE_NAMESPACE))
            && changed.send(()).is_err()
        {
            break;
        }
    }
    Ok(())
}

/// After each burst of change signals (on `settle`), reads the colors again
/// and passes them on when they differ from `colors`.
fn follow_colors(
    proxy: &Proxy<'_>,
    settle: &std::sync::mpsc::Receiver<()>,
    mut colors: DesktopColors,
    on_colors: &impl Fn(DesktopColors),
) {
    while settle.recv().is_ok() {
        // Wait until no signal has come for a while.
        while settle.recv_timeout(SETTLE).is_ok() {}
        let now = with_accent(kde_colors(), accent(proxy));
        if now != colors {
            log::info!("the desktop's colors are now {now:?}");
            on_colors(now);
            colors = now;
        }
    }
}

/// Reads KDE's colors, with the portal's `accent` where KDE has none, logs
/// and passes them on.
fn report_colors(
    on_colors: &impl Fn(DesktopColors),
    accent: Option<(u8, u8, u8)>,
) -> DesktopColors {
    let colors = with_accent(kde_colors(), accent);
    log::info!("the desktop's colors: {colors:?}");
    on_colors(colors);
    colors
}

/// KDE's colors with the portal's accent where KDE's own is not known
/// (KDE's portal can lag behind a change of the accent).
fn with_accent(kde: DesktopColors, portal: Option<(u8, u8, u8)>) -> DesktopColors {
    DesktopColors {
        accent: kde.accent.or(portal),
        ..kde
    }
}

/// A setting of the portal. `ReadOne` is the portal's newer call; older
/// ones have only `Read`, which wraps the value in one more variant.
fn read(proxy: &Proxy<'_>, key: &str) -> anyhow::Result<OwnedValue> {
    match proxy.call("ReadOne", &(NAMESPACE, key)) {
        Ok(value) => Ok(value),
        Err(_) => proxy
            .call("Read", &(NAMESPACE, key))
            .context("no desktop portal"),
    }
}

/// The accent color, if the portal tells one.
fn accent(proxy: &Proxy<'_>) -> Option<(u8, u8, u8)> {
    read(proxy, ACCENT_KEY)
        .ok()
        .and_then(|value| rgb_of(&value))
}

/// The colors a color-scheme value asks for: 1 prefers dark, 2 prefers
/// light, 0 has no preference (the default colors, dark).
fn scheme(value: &Value<'_>) -> Option<Theme> {
    match value {
        Value::U32(2) => Some(Theme::Light),
        Value::U32(_) => Some(Theme::Dark),
        Value::Value(inner) => scheme(inner),
        _ => None,
    }
}

/// An accent-color value: red, green and blue from 0 to 1. Out of that
/// range means "none set".
fn rgb_of(value: &Value<'_>) -> Option<(u8, u8, u8)> {
    match value {
        Value::Value(inner) => rgb_of(inner),
        Value::Structure(structure) => {
            let channel = |v: &Value<'_>| match v {
                Value::F64(x) if (0.0..=1.0).contains(x) => Some((x * 255.0).round() as u8),
                _ => None,
            };
            match structure.fields() {
                [r, g, b] => Some((channel(r)?, channel(g)?, channel(b)?)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// KDE's `kdeglobals`: the user's, where Plasma writes the color scheme.
fn kdeglobals() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("kdeglobals"))
}

/// The windows' background, text and accent of KDE's color scheme (all
/// `None` outside KDE, or when the file does not have them).
fn kde_colors() -> DesktopColors {
    if !super::native_osd::is_kde() {
        return DesktopColors::default();
    }
    kdeglobals()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| window_colors(&text))
        .unwrap_or_default()
}

/// From the text of a `kdeglobals` (colors as `r,g,b`): the
/// `[Colors:Window]` group's `BackgroundNormal` and `ForegroundNormal`, and
/// the accent: `[General]`'s `AccentColor` when the user chose one, else
/// the scheme's own, `[Colors:Window]`'s `DecorationFocus`.
fn window_colors(text: &str) -> DesktopColors {
    let mut colors = DesktopColors::default();
    let (mut chosen, mut scheme) = (None, None);
    let mut group = "";
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            group = name;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match (group, key.trim()) {
            ("Colors:Window", "BackgroundNormal") => colors.background = kde_rgb(value),
            ("Colors:Window", "ForegroundNormal") => colors.text = kde_rgb(value),
            ("Colors:Window", "DecorationFocus") => scheme = kde_rgb(value),
            ("General", "AccentColor") => chosen = kde_rgb(value),
            _ => {}
        }
    }
    colors.accent = chosen.or(scheme);
    colors
}

/// A KDE color, `r,g,b` (a fourth number, the alpha, is left out).
fn kde_rgb(value: &str) -> Option<(u8, u8, u8)> {
    let mut parts = value.split(',').map(|part| part.trim().parse::<u8>().ok());
    let rgb = (parts.next()??, parts.next()??, parts.next()??);
    Some(rgb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::StructureBuilder;

    #[test]
    fn color_schemes_are_read_as_colors() {
        assert_eq!(scheme(&Value::U32(1)), Some(Theme::Dark));
        assert_eq!(scheme(&Value::U32(2)), Some(Theme::Light));
        assert_eq!(scheme(&Value::U32(0)), Some(Theme::Dark));
        // The older call wraps the value once more.
        let wrapped = Value::Value(Box::new(Value::U32(2)));
        assert_eq!(scheme(&wrapped), Some(Theme::Light));
        assert_eq!(scheme(&Value::from("light")), None);
    }

    #[test]
    fn accent_colors_are_read_from_the_portal() {
        let accent = |r: f64, g: f64, b: f64| {
            Value::Structure(
                StructureBuilder::new()
                    .add_field(r)
                    .add_field(g)
                    .add_field(b)
                    .build()
                    .unwrap(),
            )
        };
        // Breeze's blue, as KDE's portal tells it.
        let blue = accent(0.239216, 0.682353, 0.913725);
        assert_eq!(rgb_of(&blue), Some((61, 174, 233)));
        assert_eq!(rgb_of(&Value::Value(Box::new(blue))), Some((61, 174, 233)));
        // Out of range: no accent set.
        assert_eq!(rgb_of(&accent(-1.0, -1.0, -1.0)), None);
        assert_eq!(rgb_of(&Value::U32(3)), None);
    }

    #[test]
    fn kde_window_colors_are_read() {
        let text = "[Colors:Button]\nBackgroundNormal=1,2,3\n\n\
                    [Colors:Window]\nBackgroundAlternate=41,44,48\n\
                    BackgroundNormal=32,35,38\nDecorationFocus=61,174,233\n\
                    ForegroundNormal = 252,252,252,255\n\
                    [General]\nForegroundNormal=9,9,9\n";
        let colors = window_colors(text);
        assert_eq!(colors.background, Some((32, 35, 38)));
        assert_eq!(colors.text, Some((252, 252, 252)));
        // The scheme's own accent, unless the user chose one.
        assert_eq!(colors.accent, Some((61, 174, 233)));
        let chosen = format!("{text}AccentColor=233,61,88\n");
        assert_eq!(window_colors(&chosen).accent, Some((233, 61, 88)));
        // KDE's accent comes before the portal's.
        assert_eq!(
            with_accent(colors, Some((1, 2, 3))).accent,
            Some((61, 174, 233))
        );
        assert_eq!(
            with_accent(DesktopColors::default(), Some((1, 2, 3))).accent,
            Some((1, 2, 3))
        );
        assert_eq!(
            window_colors("[General]\nName=x\n"),
            DesktopColors::default()
        );
        assert_eq!(kde_rgb("1,2"), None);
        assert_eq!(kde_rgb("300,2,3"), None);
    }
}
