//! Demo mode: shows sample tracks every few seconds from a background
//! thread, through the same messages as the real sources (SMTC, Core Audio,
//! the keys).

use std::sync::Arc;
use std::time::{Duration, Instant};

use flexflyout_core::{Cover, NowPlaying, PlaybackStatus, RepeatMode, Timeline, Volume};

use crate::message::{Message, Trigger};

/// Time between two appearances. Longer than one appearance (about 3.6 s
/// with the default timing), so that every one starts from a hidden window.
const INTERVAL: Duration = Duration::from_secs(6);

/// Sample tracks: a Japanese title (fonts) with square cover art, a long
/// one (truncation) with wide art (cropping), a video without an artist
/// or art (the placeholder), and Korean, Chinese and Thai (fonts found on
/// demand). The first two have a position and shuffle and repeat; the
/// others have none of them (a live stream).
const TRACKS: [SampleTrack; 4] = [
    (
        "サンプルの曲（Sample Track）",
        "FlexFlyout デモ",
        Some((160, 160)),
    ),
    (
        "A very long title that does not fit on the panel at the default size",
        "Some Artist",
        Some((320, 180)),
    ),
    ("Video without an artist", "", None),
    // "Song" in Korean and "our song" in Simplified Chinese (short enough
    // to show whole); "sample artist" in Thai.
    ("노래 · 我们的歌", "ศิลปินตัวอย่าง", None),
];

/// A sample track: title, artist and the size of its cover art, if any.
type SampleTrack = (&'static str, &'static str, Option<(u32, u32)>);

/// Starts the demo thread. It sends `cycles` appearances (0 = endless),
/// then [`Message::Quit`].
pub fn spawn(cycles: u32, send: impl Fn(Message) + Send + 'static) {
    std::thread::Builder::new()
        .name("demo".into())
        .spawn(move || {
            // Let the window finish starting up first.
            std::thread::sleep(Duration::from_millis(1500));
            let covers: Vec<_> = TRACKS
                .iter()
                .map(|(_, _, size)| size.map(|(w, h)| Arc::new(gradient(w, h))))
                .collect();
            let mut i = 0;
            while cycles == 0 || i < cycles {
                let n = i as usize % TRACKS.len();
                let (title, artist, _) = TRACKS[n];
                send(Message::NowPlaying(Some(NowPlaying {
                    title: title.into(),
                    artist: artist.into(),
                    status: if i % 2 == 0 {
                        PlaybackStatus::Playing
                    } else {
                        PlaybackStatus::Paused
                    },
                    cover: covers[n].clone(),
                    // 1:05 into a 3:30 track, then 2:10 into the long one.
                    timeline: (n < 2).then(|| Timeline {
                        position: Duration::from_secs(65 * (n as u64 + 1)),
                        updated: Instant::now(),
                        length: Duration::from_secs(210 + 200 * n as u64),
                        seekable: true,
                    }),
                    repeat: (n < 2).then_some(if n == 0 {
                        RepeatMode::List
                    } else {
                        RepeatMode::Track
                    }),
                    shuffle: (n < 2).then_some(n == 0),
                })));
                // 20%, 40%, … with every fourth one muted.
                send(Message::Volume(Volume {
                    level: ((i % 5) + 1) as f32 * 0.2,
                    muted: i % 4 == 3,
                }));
                log::info!("demo: appearance {} of {cycles}", i + 1);
                send(Message::Show {
                    trigger: Trigger::Demo,
                    sent: Instant::now(),
                });
                i += 1;
                std::thread::sleep(INTERVAL);
            }
            send(Message::Quit);
        })
        .expect("cannot start the demo thread");
}

/// Sample cover art: a diagonal gradient with a light frame, so that the
/// cropping and the rounded corners can be seen.
fn gradient(width: u32, height: u32) -> Cover {
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let frame = x < 4 || y < 4 || x >= width - 4 || y >= height - 4;
            let pixel = if frame {
                [230, 230, 230, 255]
            } else {
                let t = (x + y) as f32 / (width + height) as f32;
                [
                    (40.0 + 200.0 * t) as u8,
                    (80.0 + 60.0 * (1.0 - t)) as u8,
                    (200.0 - 120.0 * t) as u8,
                    255,
                ]
            };
            rgba.extend_from_slice(&pixel);
        }
    }
    Cover {
        width,
        height,
        rgba,
    }
}
