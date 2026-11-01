//! Demo mode: shows sample tracks every few seconds from a background
//! thread, the same way the real sources (SMTC, Core Audio, the keys) will.

use std::time::{Duration, Instant};

use flexflyout_core::{NowPlaying, PlaybackStatus, Volume};

use crate::app::Message;

/// Time between two appearances. Longer than one appearance (about 3.6 s
/// with the default timing), so that every one starts from a hidden window.
const INTERVAL: Duration = Duration::from_secs(6);

/// Sample tracks: a Japanese title (fonts), a long one (truncation) and a
/// video without an artist.
const TRACKS: [(&str, &str); 3] = [
    ("サンプルの曲（Sample Track）", "FlexFlyout デモ"),
    (
        "A very long title that does not fit on the panel at the default size",
        "Some Artist",
    ),
    ("Video without an artist", ""),
];

/// Starts the demo thread. It sends `cycles` appearances (0 = endless),
/// then [`Message::Quit`].
pub fn spawn(cycles: u32, send: impl Fn(Message) + Send + 'static) {
    std::thread::Builder::new()
        .name("demo".into())
        .spawn(move || {
            // Let the window finish starting up first.
            std::thread::sleep(Duration::from_millis(1500));
            let mut i = 0;
            while cycles == 0 || i < cycles {
                let (title, artist) = TRACKS[i as usize % TRACKS.len()];
                let now_playing = NowPlaying {
                    title: title.into(),
                    artist: artist.into(),
                    status: if i % 2 == 0 {
                        PlaybackStatus::Playing
                    } else {
                        PlaybackStatus::Paused
                    },
                };
                // 20%, 40%, … with every fourth one muted.
                let volume = Volume {
                    level: ((i % 5) + 1) as f32 * 0.2,
                    muted: i % 4 == 3,
                };
                log::info!("demo: appearance {} of {cycles}", i + 1);
                send(Message::Show {
                    now_playing: Some(now_playing),
                    volume,
                    sent: Instant::now(),
                });
                i += 1;
                std::thread::sleep(INTERVAL);
            }
            send(Message::Quit);
        })
        .expect("cannot start the demo thread");
}
