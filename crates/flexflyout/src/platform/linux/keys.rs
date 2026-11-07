//! The volume and media keys on KDE Plasma, through its global shortcuts
//! (kglobalaccel).
//!
//! Wayland gives no program the keys meant for others, but the volume and
//! media keys are global shortcuts on KDE Plasma, and its shortcut service
//! announces on the session bus every shortcut it carries out: plasma-pa's
//! (the component "kmix", a name kept from KMix so that the users' own
//! shortcuts stay) and the media controls' ("mediacontrol"). Watching them
//! shows the flyout for a key even when the volume does not change (already
//! at 0% or 100%), as the keyboard hook does on Windows. Only these two
//! components are listened to: no other shortcut reaches FlexFlyout.

use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator};
use zbus::message::Type;

/// Which kind of key was pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Volume up, down or mute.
    Volume,
    /// Play/pause, next, previous or stop.
    Media,
}

/// The interface of a component of the shortcut service.
const COMPONENT: &str = "org.kde.kglobalaccel.Component";

/// The object paths of the components watched.
const PATHS: [&str; 2] = ["/component/kmix", "/component/mediacontrol"];

/// What a shortcut of those components is, if it shows the flyout (the
/// microphone's and seeking do not).
fn key_of(component: &str, action: &str) -> Option<Key> {
    match (component, action) {
        (
            "kmix",
            "increase_volume"
            | "decrease_volume"
            | "increase_volume_small"
            | "decrease_volume_small"
            | "mute",
        ) => Some(Key::Volume),
        (
            "mediacontrol",
            "playpausemedia" | "playmedia" | "pausemedia" | "stopmedia" | "nextmedia"
            | "previousmedia",
        ) => Some(Key::Media),
        _ => None,
    }
}

/// Starts watching the keys, calling `sink` for each press (and for each
/// repeat while a key is held).
pub fn spawn(sink: impl Fn(Key) + Clone + Send + 'static) {
    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(err) => {
            log::warn!("the volume and media keys cannot be watched: {err}");
            return;
        }
    };
    for path in PATHS {
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(COMPONENT)
            .and_then(|rule| rule.path(path))
            .map(|rule| rule.build());
        let rule = match rule {
            Ok(rule) => rule,
            Err(err) => {
                log::error!("cannot watch the shortcuts of {path}: {err}");
                continue;
            }
        };
        let connection = connection.clone();
        let sink = sink.clone();
        let spawned = std::thread::Builder::new()
            .name("keys".into())
            .spawn(move || watch(&connection, rule, &sink));
        if let Err(err) = spawned {
            log::error!("cannot start a key thread: {err}");
        }
    }
    log::info!("watching the volume and media keys (KDE Plasma's shortcuts)");
}

/// Reports the keys among the signals matching `rule`, until the bus goes.
fn watch(connection: &Connection, rule: MatchRule<'static>, sink: &impl Fn(Key)) {
    let messages = match MessageIterator::for_match_rule(rule, connection, None) {
        Ok(messages) => messages,
        Err(err) => {
            log::error!("cannot watch the volume and media keys: {err}");
            return;
        }
    };
    for message in messages.flatten() {
        let header = message.header();
        let pressed = header.member().is_some_and(|member| {
            matches!(
                member.as_str(),
                "globalShortcutPressed" | "globalShortcutRepeated"
            )
        });
        if !pressed {
            continue;
        }
        // The component, the action and a timestamp.
        if let Ok((component, action, _)) = message.body().deserialize::<(String, String, i64)>()
            && let Some(key) = key_of(&component, &action)
        {
            sink(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_volume_and_playback_shortcuts_count() {
        assert_eq!(key_of("kmix", "increase_volume"), Some(Key::Volume));
        assert_eq!(key_of("kmix", "decrease_volume_small"), Some(Key::Volume));
        assert_eq!(key_of("kmix", "mute"), Some(Key::Volume));
        assert_eq!(key_of("mediacontrol", "playpausemedia"), Some(Key::Media));
        assert_eq!(key_of("mediacontrol", "nextmedia"), Some(Key::Media));
        // The microphone's, seeking, and other components' shortcuts.
        assert_eq!(key_of("kmix", "mic_mute"), None);
        assert_eq!(key_of("kmix", "increase_microphone_volume"), None);
        assert_eq!(key_of("mediacontrol", "seekforwardmedia"), None);
        assert_eq!(key_of("kwin", "increase_volume"), None);
    }
}
