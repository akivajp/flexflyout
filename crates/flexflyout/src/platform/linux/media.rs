//! MPRIS (the D-Bus interface Linux media players offer): what the current
//! player plays, where it is in the track, and the playback controls.
//!
//! Like the Windows version, the D-Bus signals only post requests to the
//! media thread, which reads the players and carries out the controls in
//! order; requests that arrive close together are handled once.
//!
//! The current player is the one that last started playing; while none
//! plays, the one shown before stays, else any. When the settings choose
//! one (`general.player`, its MPRIS name without the instance, such as
//! "spotify"), only that one is followed. The list of players is reported
//! for the flyout's player button.
//!
//! Cover art given as a web address (Spotify) is downloaded on a thread of
//! its own, and joins the track when it arrives: a slow or unreachable
//! server never holds up a new track's flyout or the controls.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use flexflyout_core::{
    Change, Command, Cover, NowPlaying, PlaybackStatus, Player, RepeatMode, Timeline,
};
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

/// Receives the players there are and the ID of the one followed.
type PlayersSink = Box<dyn Fn(Vec<Player>, Option<String>) + Send>;

/// What the media thread is asked to do.
#[derive(Debug)]
enum Request {
    /// A player changed something (its unique bus name).
    Changed(String),
    /// A player appeared or went away.
    Players,
    /// A control of the flyout was used.
    Command(Command),
    /// Follow this player (its ID), or whichever plays (`None`).
    Choose(Option<String>),
    /// The cover art at this web address was downloaded (`None`: it could
    /// not be).
    Cover(String, Option<Arc<Cover>>),
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

    /// Follows the player `choice` (its ID), or whichever plays.
    pub fn choose(&self, choice: Option<String>) {
        let _ = self.requests.send(Request::Choose(choice));
    }
}

/// Starts following the player `choice` (its ID), or whichever plays.
pub fn spawn(
    choice: Option<String>,
    sink: impl Fn(Option<NowPlaying>, Change) + Send + 'static,
    players: impl Fn(Vec<Player>, Option<String>) + Send + 'static,
) -> Media {
    let (requests, received) = channel();
    let media = Media {
        requests: requests.clone(),
    };
    let sink: Sink = Box::new(sink);
    let players: PlayersSink = Box::new(players);
    let spawned = std::thread::Builder::new()
        .name("media".into())
        .spawn(move || {
            if let Err(err) = run(choice, &sink, &players, &requests, &received) {
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
    mut choice: Option<String>,
    sink: &Sink,
    players_sink: &PlayersSink,
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

    let mut covers = CoverCache::new(requests.clone());
    let mut active: Option<String> = None;
    let mut current = pick(&connection, None, None, choice.as_deref());
    let mut track = current.as_ref().and_then(|c| read_track(c, &mut covers));
    log::info!("now playing: {track:?}");
    sink(track.clone(), Change::Other);
    let mut listed = (
        players(&connection),
        current.as_ref().map(|c| player_id(&c.name).to_owned()),
    );
    players_sink(listed.0.clone(), listed.1.clone());

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
                Request::Choose(chosen) => {
                    log::info!("player chosen: {chosen:?}");
                    choice = chosen;
                    changed = true;
                }
                Request::Cover(url, cover) => {
                    // Read the track again, now with its cover art.
                    if covers.finish(url, cover) {
                        changed = true;
                    }
                }
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
        let chosen = pick(
            &connection,
            active.as_deref(),
            current.as_ref().map(|c| c.name.as_str()),
            choice.as_deref(),
        );
        if chosen.as_ref().map(|c| &c.name) != current.as_ref().map(|c| &c.name) {
            log::info!(
                "current player: {}",
                chosen.as_ref().map_or("none", |c| c.name.as_str())
            );
        }
        current = chosen;
        let now = (
            players(&connection),
            current.as_ref().map(|c| player_id(&c.name).to_owned()),
        );
        if now != listed {
            players_sink(now.0.clone(), now.1.clone());
            listed = now;
        }
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

/// A player's ID from its bus name: the part after the prefix, without an
/// instance ("org.mpris.MediaPlayer2.chromium.instance123" → "chromium"),
/// which stays the same from run to run.
fn player_id(name: &str) -> &str {
    let name = name.strip_prefix(PREFIX).unwrap_or(name);
    name.split('.').next().unwrap_or(name)
}

/// The players' bus names.
fn player_names(connection: &Connection) -> Vec<String> {
    let Ok(bus) = zbus::blocking::fdo::DBusProxy::new(connection) else {
        return Vec::new();
    };
    bus.list_names()
        .map(|names| {
            names
                .into_iter()
                .map(|name| name.to_string())
                .filter(|name| name.starts_with(PREFIX))
                .collect()
        })
        .unwrap_or_default()
}

/// The players there are (one per ID), with the names they give
/// themselves (their `Identity`).
fn players(connection: &Connection) -> Vec<Player> {
    let mut players: Vec<Player> = Vec::new();
    for name in player_names(connection) {
        let id = player_id(&name).to_owned();
        if players.iter().any(|player| player.id == id) {
            continue;
        }
        let identity = zbus::blocking::proxy::Builder::<Proxy<'_>>::new(connection)
            .destination(name.clone())
            .and_then(|builder| builder.path(PATH))
            .and_then(|builder| builder.interface("org.mpris.MediaPlayer2"))
            .and_then(|builder| builder.cache_properties(CacheProperties::No).build())
            .and_then(|proxy| proxy.get_property::<String>("Identity"))
            .ok()
            .filter(|identity| !identity.trim().is_empty());
        let name = identity.unwrap_or_else(|| id.clone());
        players.push(Player { id, name });
    }
    // In a steady order: the bus lists names in no particular one.
    players.sort_by_key(|player| player.name.to_lowercase());
    players
}

/// The players there are now, for a process that does not follow them
/// (the settings window).
pub fn list() -> Vec<Player> {
    match Connection::session() {
        Ok(connection) => players(&connection),
        Err(err) => {
            log::warn!("cannot list the players: {err}");
            Vec::new()
        }
    }
}

/// Picks the player to show: among those of the ID `choice` if it is set,
/// the one that last started playing (`active`, a unique name) if it
/// still plays, else any that plays, else the one shown before
/// (`previous`), else any.
fn pick(
    connection: &Connection,
    active: Option<&str>,
    previous: Option<&str>,
    choice: Option<&str>,
) -> Option<Current> {
    let Ok(bus) = zbus::blocking::fdo::DBusProxy::new(connection) else {
        return None;
    };
    let names: Vec<String> = player_names(connection)
        .into_iter()
        .filter(|name| choice.is_none_or(|choice| player_id(name) == choice))
        .collect();
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
        | Command::OpenSettings
        | Command::ChoosePlayer(_)
        | Command::Expand => {
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
struct CoverCache {
    last: Option<(String, Option<Arc<Cover>>)>,
    /// The web address being downloaded, if any.
    loading: Option<String>,
    /// Where a download reports back: the media thread.
    requests: Sender<Request>,
}

impl CoverCache {
    /// An empty cache whose downloads report to `requests`.
    fn new(requests: Sender<Request>) -> Self {
        Self {
            last: None,
            loading: None,
            requests,
        }
    }

    /// The cover art at `url` (a file, a `data:` URL as mpv gives it, or a
    /// web address as Spotify gives it), if it is there. A web address
    /// starts a download and gives `None` until [`Self::finish`] has its
    /// result.
    fn get(&mut self, url: &str) -> Option<Arc<Cover>> {
        if let Some((known, cover)) = &self.last
            && known == url
        {
            return cover.clone();
        }
        if !is_web(url) {
            let cover = load_logged(url);
            self.last = Some((url.to_owned(), cover.clone()));
            return cover;
        }
        if self.loading.as_deref() != Some(url) {
            let (address, requests) = (url.to_owned(), self.requests.clone());
            let spawned = std::thread::Builder::new()
                .name("cover".into())
                .spawn(move || {
                    let cover = load_logged(&address);
                    let _ = requests.send(Request::Cover(address, cover));
                });
            match spawned {
                Ok(_) => self.loading = Some(url.to_owned()),
                Err(err) => log::warn!("cannot download the cover art: {err}"),
            }
        }
        None
    }

    /// Takes in a downloaded cover art. Returns whether it is the one
    /// wanted (not one of a track that has passed meanwhile).
    fn finish(&mut self, url: String, cover: Option<Arc<Cover>>) -> bool {
        if self.loading.as_deref() != Some(url.as_str()) {
            return false;
        }
        self.loading = None;
        self.last = Some((url, cover));
        true
    }
}

/// Whether the cover art is a web address, to be downloaded on the side.
fn is_web(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

/// [`load_cover`], with a failure logged.
fn load_logged(url: &str) -> Option<Arc<Cover>> {
    load_cover(url)
        .inspect_err(|err| log::warn!("cannot read the cover art at {url}: {err:#}"))
        .ok()
        .map(Arc::new)
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
    } else if is_web(url) {
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

    /// Serves `body` as an image on a local port to every request, and
    /// counts them. Returns its address and the count.
    fn serve(body: Vec<u8>) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read as _, Write as _};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/cover.png", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&hits);
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                counted.fetch_add(1, Ordering::SeqCst);
                let mut request = [0u8; 2048];
                let _ = stream.read(&mut request);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(&body);
            }
        });
        (address, hits)
    }

    /// A PNG image of `width` × `height` pixels.
    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn web_cover_art_arrives_later_and_is_downloaded_once() {
        let (address, hits) = serve(png(4, 3));
        let (requests, received) = channel();
        let mut covers = CoverCache::new(requests);
        // Not there yet: the track shows without it, and looking again
        // does not download it again.
        assert!(covers.get(&address).is_none());
        assert!(covers.get(&address).is_none());
        let Ok(Request::Cover(url, cover)) = received.recv_timeout(Duration::from_secs(10)) else {
            panic!("no cover art came");
        };
        assert_eq!(url, address);
        assert!(covers.finish(url, cover));
        let cover = covers.get(&address).expect("the cover art");
        assert_eq!((cover.width, cover.height), (4, 3));
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        // The art of a track that has passed meanwhile is not taken.
        assert!(!covers.finish("http://127.0.0.1:9/passed.png".into(), None));
        assert!(covers.get(&address).is_some());
    }

    #[test]
    fn player_ids_leave_out_the_instance() {
        assert_eq!(player_id("org.mpris.MediaPlayer2.spotify"), "spotify");
        assert_eq!(
            player_id("org.mpris.MediaPlayer2.chromium.instance12345"),
            "chromium"
        );
        assert_eq!(
            player_id("org.mpris.MediaPlayer2.firefox.instance_1_88"),
            "firefox"
        );
        assert_eq!(player_id("org.mpris.MediaPlayer2.mpv"), "mpv");
    }

    #[test]
    fn file_urls_are_decoded() {
        assert_eq!(percent_decode("/tmp/My%20Song.jpg"), "/tmp/My Song.jpg");
        assert_eq!(percent_decode("/tmp/%E6%9B%B2.png"), "/tmp/曲.png");
        // A stray percent sign stays.
        assert_eq!(percent_decode("/tmp/100%.png"), "/tmp/100%.png");
    }
}
