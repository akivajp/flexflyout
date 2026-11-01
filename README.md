# FlexFlyout

<img src="crates/flexflyout/assets/flexflyout.svg" alt="FlexFlyout icon" width="96" align="right">

[![CI](https://github.com/akivajp/flexflyout/actions/workflows/ci.yml/badge.svg)](https://github.com/akivajp/flexflyout/actions/workflows/ci.yml) [![Release](https://img.shields.io/github/v/release/akivajp/flexflyout)](https://github.com/akivajp/flexflyout/releases/latest)

[日本語](README.ja.md)

> **Early development.** Windows 11 comes first; the Linux desktops follow.

A flexible, cross-platform media flyout. When you change the volume or a new track starts, it pops up with what is playing — cover art, title and artist — along with the position in the track, playback controls (including shuffle and repeat) and the volume, then fades away. Click or drag the bars to seek or set the volume. Pin the flyout to keep it on screen — and drag it wherever you like, optionally as its new usual place — or close it at once.

Screen sizes and monitor setups differ from desk to desk, so every part of the flyout is meant to be adjustable:

- **Size** — scale the whole flyout
- **Position** — nine anchor points plus pixel offsets
- **Monitor** — the primary one, the one under the cursor, or a specific one
- **Display time** and **opacity**
- **The system's own volume indicator** — keep it and place the flyout beside it, or hide it

## Platforms

| Platform | Status |
|---|---|
| Windows 11 | Available (x86_64) |
| Linux — KDE Plasma (Wayland) | Planned |
| Linux — GNOME (Wayland) | Planned, as a small GNOME Shell extension that draws what FlexFlyout sends |
| macOS | Future |

## Installation (Windows 11)

With [Scoop](https://scoop.sh), from the [akivajp bucket](https://github.com/akivajp/scoop-bucket), which also adds FlexFlyout to the Start menu (`scoop update flexflyout` updates it; quit FlexFlyout from its tray icon first):

```powershell
scoop bucket add akivajp https://github.com/akivajp/scoop-bucket
scoop install akivajp/flexflyout
```

If you installed it from the manifest URL before, switch to the bucket as its README explains, so that updates keep coming.

Or download `flexflyout-<version>-x86_64-pc-windows-msvc.zip` from the [Releases](https://github.com/akivajp/flexflyout/releases) page, extract it anywhere and run `flexflyout.exe` (`SHA256SUMS` lists the checksum). The release is not code-signed yet, so Windows SmartScreen may warn the first time: choose **More info** → **Run anyway**.

FlexFlyout runs in the notification area (the tray). Its menu shows the flyout, opens the settings window, turns **Start with Windows** on or off, and quits. The settings are saved in `%APPDATA%\flexflyout\config.toml`, which can also be edited by hand (it is applied within a second of saving). `flexflyout --help` lists the command-line options; `flexflyout --demo` shows sample tracks.

### Uninstall

1. Turn **Start with Windows** off in the tray menu and quit FlexFlyout (Scoop turns it off for you).
2. Remove it: `scoop uninstall flexflyout`, or delete the folder you extracted the zip to.
3. Delete the settings and the log, if you like: `%APPDATA%\flexflyout` and `%LOCALAPPDATA%\flexflyout`.

If you hid the Windows volume indicator, it comes back when FlexFlyout quits.

## How it works

- **Now playing and playback controls**: the System Media Transport Controls (SMTC) on Windows, MPRIS on Linux
- **Volume**: Core Audio on Windows, PipeWire / PulseAudio on Linux
- **UI**: Rust and [egui](https://github.com/emilk/egui)
- **Languages**: English and Japanese first, on top of a translation mechanism that makes adding more languages easy

## Background

On Windows 10, the volume flyout also showed the current track with playback controls. Windows 11 dropped that part. The third-party replacements either stopped working on recent Windows 11 builds or give little control over size and placement, and none of them run on Linux desktops. FlexFlyout aims to fill both gaps.

## Code signing policy

Code signing of the Windows releases through the [SignPath Foundation](https://signpath.org) is planned; until it is in place, releases are unsigned. Once approved: free code signing provided by [SignPath.io](https://about.signpath.io), certificate by [SignPath Foundation](https://signpath.org).

- **What is signed**: `flexflyout.exe` in the Windows release zip, built by GitHub Actions ([`release.yml`](.github/workflows/release.yml)) from the tagged source of this repository. Nothing built elsewhere is signed, and every signing request is approved by hand.
- **Team**: [Akiva Miura (@akivajp)](https://github.com/akivajp) is the author, reviewer and approver. Changes from others come as pull requests and are reviewed before they are merged.

### Privacy

FlexFlyout collects no telemetry and makes no network connections. It reads what the media players report to Windows (the track, its cover art and position) and the volume, and watches the volume and media keys only to know when to show the flyout (no other key is recorded or kept). Its settings and its log stay on your machine.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
