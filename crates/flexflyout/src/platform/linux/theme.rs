//! The desktop's light or dark color scheme, for the theme that follows the
//! system: the "color-scheme" setting of xdg-desktop-portal, which KDE
//! Plasma, GNOME and others provide. Read at the start, then followed
//! through the portal's change signal. Without a portal, nothing is known
//! and the flyout keeps its default colors.

use anyhow::Context as _;
use flexflyout_core::Theme;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

/// The portal's settings, and the one wanted.
const DESTINATION: &str = "org.freedesktop.portal.Desktop";
const PATH: &str = "/org/freedesktop/portal/desktop";
const INTERFACE: &str = "org.freedesktop.portal.Settings";
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// Starts following the color scheme; `sink` gets it at once and after
/// every change.
pub fn spawn(sink: impl Fn(Theme) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("theme".into())
        .spawn(move || {
            if let Err(err) = run(&sink) {
                log::info!("the desktop's color scheme is not known: {err:#}");
            }
        });
    if let Err(err) = spawned {
        log::warn!("cannot follow the desktop's color scheme: {err}");
    }
}

/// Reports the color scheme, then every change of it.
fn run(sink: &impl Fn(Theme)) -> anyhow::Result<()> {
    let connection = Connection::session().context("no D-Bus session bus")?;
    let proxy = Proxy::new(&connection, DESTINATION, PATH, INTERFACE)?;
    // Listening before reading, so that a change in between is not lost.
    let changes = proxy.receive_signal("SettingChanged")?;
    let theme = read(&proxy)?;
    log::info!("the desktop's color scheme is {theme:?}");
    sink(theme);
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
            sink(theme);
        }
    }
    Ok(())
}

/// The color scheme now. `ReadOne` is the portal's newer call; older ones
/// have only `Read`, which wraps the value in one more variant.
fn read(proxy: &Proxy<'_>) -> anyhow::Result<Theme> {
    let value: OwnedValue = match proxy.call("ReadOne", &(NAMESPACE, KEY)) {
        Ok(value) => value,
        Err(_) => proxy
            .call("Read", &(NAMESPACE, KEY))
            .context("no desktop portal")?,
    };
    scheme(&value).context("the color scheme is not a number")
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
