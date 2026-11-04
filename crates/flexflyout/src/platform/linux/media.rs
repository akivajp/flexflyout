//! MPRIS (the D-Bus interface Linux media players offer): what the current
//! player plays, where it is in the track, and the playback controls.
//!
//! Like the Windows version, the D-Bus signals only post requests to the
//! media thread, which reads the players and carries out the controls in
//! order; requests that arrive close together are handled once.
//!
//! The current player is the one that last started playing; while none
//! plays, the one shown before stays, else any.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use flexflyout_core::{Change, Command, Cover, NowPlaying, PlaybackStatus, RepeatMode, Timeline};
use zbus::MatchRule;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::message::Type;
use zbus::proxy::CacheProperties;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

/// How long to wait for more events after the first one.
const SETTLE: Duration = Duration::from_millis(120);

/// Longest side of the cover art kept in memory, in pixels (as on Windows).
const MAX_COVER_SIDE: u32 = 320;

/// Largest cover art file read or downloaded.
const MAX_COVER_BYTES: u64 = 16 * 1024 * 1024;

/// The prefix of the players' bus names.
const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

/// Receives the current track (`None` when no player reports one) and
/// what changed.
type Sink = Box<dyn Fn(Option<NowPlaying>, Change) + Send>;

/// What the media thread is asked to do.
#[derive(Debug)]
enum Request {
    /// A player changed something (its unique bus name).
    Changed(String),
    /// A player appeared or went away.
    Players,
    /// A control of the flyout was used.
    Command(Command),
}

/// Sends the flyout's controls to the current player.
#[derive(Clone, Debug)]
pub struct Media {
    requests: Sender<Request>,
}

impl Media {
    /// Asks the current player to do `command`.
    pub fn command(&self, command: Command) {
        let _ = self.requests.send(Request::Command(command));
    }
}

/// Starts following the players.
pub fn spawn(sink: impl Fn(Option<NowPlaying>, Change) + Send + 'static) -> Media {
    let (requests, received) = channel();
    let media = Media {
        requests: requests.clone(),
    };
    let sink: Sink = Box::new(sink);
    let spawned = std::thread::Builder::new()
        .name("media".into())
        .spawn(move || {
            if let Err(err) = run(&sink, &requests, &received) {
                log::error!("the media players cannot be followed: {err:#}");
            }
        });
    if let Err(err) = spawned {
        log::error!("cannot start the media thread: {err}");
    }
    media
}

/// The player shown, and what is known about it.
struct Current {
    /// Its well-known bus name (org.mpris.MediaPlayer2.…).
    name: String,
    proxy: Proxy<'static>,
}

/// Follows the players until the app ends.
fn run(
    sink: &Sink,
    requests: &Sender<Request>,
    received: &Receiver<Request>,
) -> anyhow::Result<()> {
    let connection = Connection::session().context("no D-Bus session bus")?;
    watch(
        &connection,
        MatchRule::builder()
            .msg_type(Type::Signal)
            .interface("org.freedesktop.DBus.Properties")?
            .member("PropertiesChanged")?
            .path(PATH)?
            .build(),
        requests,
        |message| Request::Changed(sender_of(message)),
    );
    watch(
        &connection,
        MatchRule::builder()
            .msg_type(Type::Signal)
            .interface(PLAYER)?
            .member("Seeked")?
            .path(PATH)?
            .build(),
        requests,
        |message| Request::Changed(sender_of(message)),
    );
    watch(
        &connection,
        MatchRule::builder()
            .msg_type(Type::Signal)
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .arg0ns("org.mpris.MediaPlayer2")?
            .build(),
        requests,
        |_| Request::Players,
    );

    let mut covers = CoverCache::default();
    let mut active: Option<String> = None;
    let mut current = choose(&connection, None, None);
    let mut track = current.as_ref().and_then(|c| read_track(c, &mut covers));
    log::info!("now playing: {track:?}");
    sink(track.clone(), Change::Other);

    loop {
        let Ok(first) = received.recv() else {
            return Ok(());
        };
        let mut next = Some(first);
        let mut changed = false;
        while let Some(request) = next.take() {
            match request {
                Request::Changed(sender) => {
                    // A player that starts playing becomes the current one.
                    if playing(&connection, &sender) {
                        active = Some(sender);
                    }
                    changed = true;
                }
                Request::Players => changed = true,
                Request::Command(command) => {
                    if let Some(current) = &current {
                        send_command(current, command, track.as_ref());
                    }
                    // Read the state back once the player has carried it out.
                    changed = true;
                }
            }
            next = match received.recv_timeout(SETTLE) {
                Ok(request) => Some(request),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            };
        }
        if !changed {
            continue;
        }
        let chosen = choose(
            &connection,
            active.as_deref(),
            current.as_ref().map(|c| c.name.as_str()),
        );
        if chosen.as_ref().map(|c| &c.name) != current.as_ref().map(|c| &c.name) {
            log::info!(
                "current player: {}",
                chosen.as_ref().map_or("none", |c| c.name.as_str())
            );
        }
        current = chosen;
        let new = current.as_ref().and_then(|c| read_track(c, &mut covers));
        if new == track {
            continue;
        }
        let change = Change::between(track.as_ref(), new.as_ref());
        if change != Change::Other {
            log::info!("now playing ({change:?}): {new:?}");
        }
        track = new;
        sink(track.clone(), change);
    }
}

/// Forwards every message matching `rule` as a request, from a thread of
/// its own (the iterator blocks).
fn watch(
    connection: &Connection,
    rule: MatchRule<'static>,
    requests: &Sender<Request>,
    request: impl Fn(&zbus::Message) -> Request + Send + 'static,
) {
    let connection = connection.clone();
    let requests = requests.clone();
    let spawned = std::thread::Builder::new()
        .name("media-signals".into())
        .spawn(move || {
            let messages = match MessageIterator::for_match_rule(rule, &connection, None) {
                Ok(messages) => messages,
                Err(err) => {
                    log::error!("cannot watch the media players: {err}");
                    return;
                }
            };
            for message in messages.flatten() {
                if requests.send(request(&message)).is_err() {
                    return;
                }
            }
        });
    if let Err(err) = spawned {
        log::error!("cannot start a media signal thread: {err}");
    }
}

/// The unique bus name a message came from.
fn sender_of(message: &zbus::Message) -> String {
    message
        .header()
        .sender()
        .map(|sender| sender.to_string())
        .unwrap_or_default()
}

/// A proxy of the player interface of the player `name`, which reads the
/// properties every time (players do not always signal every change).
fn player(connection: &Connection, name: &str) -> Option<Proxy<'static>> {
    zbus::blocking::proxy::Builder::new(connection)
        .destination(name.to_owned())
        .ok()?
        .path(PATH)
        .ok()?
        .interface(PLAYER)
        .ok()?
        .cache_properties(CacheProperties::No)
        .build()
        .ok()
}

/// Whether the player at `name` (any bus name) is playing.
fn playing(connection: &Connection, name: &str) -> bool {
    player(connection, name)
        .and_then(|proxy| proxy.get_property::<String>("PlaybackStatus").ok())
        .is_some_and(|status| status == "Playing")
}

/// Picks the player to show: the one that last started playing (`active`,
/// a unique name) if it still plays, else any that plays, else the one
/// shown before (`previous`), else any.
fn choose(
    connection: &Connection,
    active: Option<&str>,
    previous: Option<&str>,
) -> Option<Current> {
    let Ok(bus) = zbus::blocking::fdo::DBusProxy::new(connection) else {
        return None;
    };
    let names: Vec<String> = bus
        .list_names()
        .map(|names| {
            names
                .into_iter()
                .map(|name| name.to_string())
                .filter(|name| name.starts_with(PREFIX))
                .collect()
        })
        .unwrap_or_default();
    let owner = |name: &str| -> Option<String> {
        let name = zbus::names::BusName::try_from(name).ok()?;
        bus.get_name_owner(name).ok().map(|owner| owner.to_string())
    };
    let active_name: Option<&String> = active.and_then(|active| {
        names
            .iter()
            .find(|name| owner(name).as_deref() == Some(active))
            .filter(|name| playing(connection, name))
    });
    let name: String = active_name
        .or_else(|| names.iter().find(|name| playing(connection, name)))
        .or_else(|| previous.and_then(|previous| names.iter().find(|name| *name == previous)))
        .or_else(|| names.first())?
        .clone();
    let proxy = player(connection, &name)?;
    Some(Current { name, proxy })
}

/// Reads the current track of the player.
fn read_track(current: &Current, covers: &mut CoverCache) -> Option<NowPlaying> {
    let proxy = &current.proxy;
    let status = match proxy
        .get_property::<String>("PlaybackStatus")
        .ok()?
        .as_str()
    {
        "Playing" => PlaybackStatus::Playing,
        "Paused" => PlaybackStatus::Paused,
        _ => PlaybackStatus::Stopped,
    };
    let metadata: HashMap<String, OwnedValue> = proxy.get_property("Metadata").unwrap_or_default();
    let text = |key: &str| match metadata.get(key).map(|value| &**value) {
        Some(Value::Str(text)) => text.to_string(),
        // xesam:artist is a list of names.
        Some(Value::Array(names)) => names
            .iter()
            .filter_map(|name| match name {
                Value::Str(name) => Some(name.to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    };
    let length = metadata.get("mpris:length").and_then(|value| number(value));
    let track_id = track_id(&metadata);
    let seekable = proxy.get_property::<bool>("CanSeek").unwrap_or(false) && track_id.is_some();
    let timeline = length.filter(|length| *length > 0).map(|length| {
        let length = Duration::from_micros(length as u64);
        let position = proxy
            .get_property::<i64>("Position")
            .map_or(Duration::ZERO, |micros| {
                Duration::from_micros(micros.max(0) as u64)
            });
        Timeline {
            position: position.min(length),
            updated: Instant::now(),
            length,
            seekable,
        }
    });
    // Players that do not offer these leave the buttons grayed out.
    let repeat = proxy
        .get_property::<String>("LoopStatus")
        .ok()
        .map(|status| match status.as_str() {
            "Track" => RepeatMode::Track,
            "Playlist" => RepeatMode::List,
            _ => RepeatMode::Off,
        });
    let shuffle = proxy.get_property::<bool>("Shuffle").ok();
    let cover = metadata
        .get("mpris:artUrl")
        .and_then(|value| match &**value {
            Value::Str(url) => Some(url.to_string()),
            _ => None,
        })
        .and_then(|url| covers.get(&url));
    Some(NowPlaying {
        title: text("xesam:title"),
        artist: text("xesam:artist"),
        status,
        cover,
        timeline,
        repeat,
        shuffle,
    })
}

/// A number of any integer type (players differ in mpris:length's type).
fn number(value: &Value<'_>) -> Option<i64> {
    match value {
        Value::I64(n) => Some(*n),
        Value::U64(n) => i64::try_from(*n).ok(),
        Value::I32(n) => Some(i64::from(*n)),
        Value::U32(n) => Some(i64::from(*n)),
        Value::F64(n) => Some(*n as i64),
        _ => None,
    }
}

/// The track's object path, needed to seek.
fn track_id(metadata: &HashMap<String, OwnedValue>) -> Option<ObjectPath<'static>> {
    match metadata.get("mpris:trackid").map(|value| &**value) {
        Some(Value::ObjectPath(path)) => Some(path.to_owned()),
        // Some players send it as a string.
        Some(Value::Str(path)) => ObjectPath::try_from(path.to_string()).ok(),
        _ => None,
    }
}

/// Asks the player to carry out a control.
fn send_command(current: &Current, command: Command, track: Option<&NowPlaying>) {
    let proxy = &current.proxy;
    let result = match command {
        Command::Previous => proxy.call_method("Previous", &()).map(drop),
        Command::TogglePlayPause => proxy.call_method("PlayPause", &()).map(drop),
        Command::Next => proxy.call_method("Next", &()).map(drop),
        Command::Seek(position) => {
            let metadata: HashMap<String, OwnedValue> =
                proxy.get_property("Metadata").unwrap_or_default();
            match track_id(&metadata) {
                Some(id) => {
                    let micros = i64::try_from(position.as_micros()).unwrap_or(i64::MAX);
                    proxy.call_method("SetPosition", &(id, micros)).map(drop)
                }
                None => {
                    log::info!("the player gives no track id to seek in");
                    Ok(())
                }
            }
        }
        Command::SetRepeat(mode) => {
            let status = match mode {
                RepeatMode::Off => "None",
                RepeatMode::Track => "Track",
                RepeatMode::List => "Playlist",
            };
            proxy
                .set_property("LoopStatus", status)
                .map_err(zbus::Error::from)
        }
        Command::SetShuffle(on) => proxy.set_property("Shuffle", on).map_err(zbus::Error::from),
        Command::SetVolume(_)
        | Command::ToggleMute
        | Command::Pin(_)
        | Command::Close
        | Command::StartMove
        | Command::OpenSettings => {
            log::warn!("{command:?} is not for the player");
            return;
        }
    };
    match result {
        Ok(()) => log::info!("sent {command:?} to {}", current.name),
        Err(err) => log::warn!(
            "cannot send {command:?} to {} (playing {:?}): {err}",
            current.name,
            track.map(|t| &t.title)
        ),
    }
}

/// The cover art of the last track, by its URL, so that a player that
/// repeats its properties does not make it load again.
#[derive(Default)]
struct CoverCache {
    last: Option<(String, Option<Arc<Cover>>)>,
}

impl CoverCache {
    /// The cover art at `url` (a file, a `data:` URL as mpv gives it, or
    /// https as Spotify gives it).
    fn get(&mut self, url: &str) -> Option<Arc<Cover>> {
        if let Some((known, cover)) = &self.last
            && known == url
        {
            return cover.clone();
        }
        let cover = load_cover(url)
            .inspect_err(|err| log::warn!("cannot read the cover art at {url}: {err:#}"))
            .ok()
            .map(Arc::new);
        self.last = Some((url.to_owned(), cover.clone()));
        cover
    }
}

/// Reads and decodes cover art, and scales it down.
fn load_cover(url: &str) -> anyhow::Result<Cover> {
    let bytes = if let Some(path) = url.strip_prefix("file://") {
        let path = percent_decode(path);
        let size = std::fs::metadata(&path)?.len();
        anyhow::ensure!(size <= MAX_COVER_BYTES, "too large ({size} bytes)");
        std::fs::read(&path)?
    } else if let Some(data) = url.strip_prefix("data:") {
        // data:image/jpeg;base64,....
        let (kind, payload) = data.split_once(',').context("not a data URL")?;
        anyhow::ensure!(kind.ends_with(";base64"), "not base64 data");
        base64_decode(payload).context("broken base64 data")?
    } else if url.starts_with("https://") || url.starts_with("http://") {
        // The cover art the player itself names (besides the update check,
        // the only network access).
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .new_agent();
        agent
            .get(url)
            .call()?
            .body_mut()
            .with_config()
            .limit(MAX_COVER_BYTES)
            .read_to_vec()?
    } else {
        anyhow::bail!("not a file or web address");
    };
    let image = image::load_from_memory(&bytes)?;
    let image = if image.width().max(image.height()) > MAX_COVER_SIDE {
        image.resize(
            MAX_COVER_SIDE,
            MAX_COVER_SIDE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };
    let mut rgba = image.to_rgba8();
    // egui wants premultiplied alpha.
    for pixel in rgba.pixels_mut() {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    Ok(Cover {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// Decodes standard base64 (padding optional, whitespace ignored).
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    };
    let mut decoded = Vec::with_capacity(text.len() * 3 / 4);
    let (mut bits, mut count) = (0u32, 0);
    for c in text.bytes() {
        if c == b'=' {
            break;
        }
        if c.is_ascii_whitespace() {
            continue;
        }
        bits = (bits << 6) | u32::from(value(c)?);
        count += 6;
        if count >= 8 {
            count -= 8;
            decoded.push((bits >> count) as u8);
        }
    }
    Some(decoded)
}

/// Decodes %XX escapes in a file URL's path.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            decoded.push((high * 16 + low) as u8);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_is_decoded() {
        assert_eq!(base64_decode("aGVsbG8=").as_deref(), Some(&b"hello"[..]));
        assert_eq!(base64_decode("aGVs\nbG8").as_deref(), Some(&b"hello"[..]));
        assert_eq!(
            base64_decode("aGVsbG8hIQ==").as_deref(),
            Some(&b"hello!!"[..])
        );
        assert_eq!(base64_decode("a*b"), None);
    }

    #[test]
    fn file_urls_are_decoded() {
        assert_eq!(percent_decode("/tmp/My%20Song.jpg"), "/tmp/My Song.jpg");
        assert_eq!(percent_decode("/tmp/%E6%9B%B2.png"), "/tmp/曲.png");
        // A stray percent sign stays.
        assert_eq!(percent_decode("/tmp/100%.png"), "/tmp/100%.png");
    }
}
