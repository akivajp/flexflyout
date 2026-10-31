# FlexFlyout

[日本語](README.ja.md)

> **Early development.** Nothing is usable yet. This repository holds the plan for now, and the Windows version comes first.

A flexible, cross-platform media flyout. When you change the volume or a new track starts, it pops up with what is playing — cover art, title and artist — along with playback controls and the volume level, then fades away.

Screen sizes and monitor setups differ from desk to desk, so every part of the flyout is meant to be adjustable:

- **Size** — scale the whole flyout
- **Position** — nine anchor points plus pixel offsets
- **Monitor** — the primary one, the one under the cursor, or a specific one
- **Display time** and **opacity**
- **The system's own volume indicator** — keep it and place the flyout beside it, or hide it

## Platforms

| Platform | Status |
|---|---|
| Windows 11 | Planned first |
| Linux — KDE Plasma (Wayland) | Planned |
| Linux — GNOME (Wayland) | Planned, as a small GNOME Shell extension that draws what FlexFlyout sends |
| macOS | Future |

## How it works (plan)

- **Now playing and playback controls**: the System Media Transport Controls (SMTC) on Windows, MPRIS on Linux
- **Volume**: Core Audio on Windows, PipeWire / PulseAudio on Linux
- **UI**: Rust and [egui](https://github.com/emilk/egui)
- **Languages**: English and Japanese first, on top of a translation mechanism that makes adding more languages easy

## Background

On Windows 10, the volume flyout also showed the current track with playback controls. Windows 11 dropped that part. The third-party replacements either stopped working on recent Windows 11 builds or give little control over size and placement, and none of them run on Linux desktops. FlexFlyout aims to fill both gaps.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
