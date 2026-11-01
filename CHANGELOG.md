# Changelog

All notable changes to FlexFlyout are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/) (before 1.0, minor versions may
include breaking changes).

## [Unreleased]

### Added
- The flyout slides in from the nearest edge of the screen while it fades
  in, and back out while it fades out. `timing.animation = "fade"` (or
  "Fade only" in the settings window) keeps it in place.

## [0.1.0] - 2026-11-02

The first release, for Windows 11.

### Added
- A flyout with the current track (cover art, title, artist), its position,
  the playback controls (previous, play / pause, next, shuffle, repeat) and
  the volume, shown when the volume changes, a volume or media key is
  pressed, a new track starts, or playback starts or pauses. Works with any
  player that shows up in the Windows media controls (SMTC), such as
  Spotify, browsers and Media Player.
- Click or drag the position bar to seek, and the volume bar to set the
  volume; a mute button.
- Pin the flyout to keep it on screen, and drag it anywhere while pinned
  (optionally as its new place); close it at once with the close button.
  It also stays while the mouse pointer is on it.
- Placement: the primary monitor, the one with the mouse pointer, or a
  numbered one; nine anchor points with offsets; size, display time, fades
  and background opacity.
- The Windows volume indicator: stay clear of it (the default), hide it, or
  ignore it.
- A settings window (`flexflyout --settings`, or Settings… in the tray
  menu). The settings are kept in `%APPDATA%\flexflyout\config.toml`, which
  can also be edited by hand and is applied within a second of saving.
- A tray icon: show the flyout, open the settings, start with Windows, quit.
- English and Japanese.
- `flexflyout --demo` shows sample tracks.

[Unreleased]: https://github.com/akivajp/flexflyout/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/akivajp/flexflyout/releases/tag/v0.1.0
