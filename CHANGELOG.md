# Changelog

All notable changes to FlexFlyout are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/) (before 1.0, minor versions may
include breaking changes).

## [Unreleased]

### Added
- KDE Plasma: the volume and media keys show the flyout themselves, as
  on Windows, so a volume key shows it also at 0% and 100%, where the
  volume does not change. FlexFlyout follows Plasma's announcements of
  these two kinds of shortcuts only.

### Fixed
- Linux: after the sound server restarted (PipeWire does after an update,
  or when restarted to fix a Bluetooth device), or when it was not up yet
  at login, the flyout no longer followed the volume. FlexFlyout now
  connects again.
- Linux: the volume bar set every channel to the same level, losing the
  balance between left and right. It now keeps it.
- Linux: ending FlexFlyout with a signal (at logout, or with `kill` or
  Ctrl+C) left Plasma's volume popup hidden, with `native_osd = "hide"`,
  until FlexFlyout started again. It now closes as Quit in the tray menu
  does, and brings the popup back.

## [0.6.0] - 2026-11-05

### Added
- Appearance settings (`[appearance]`, and a new section in the settings
  window): a dark or light theme, the accent color of the bars and of the
  buttons that are on, the roundness of the corners (0 for square), and
  whether to show the cover art, the position bar, the shuffle and repeat
  buttons and the volume. The flyout gets smaller without them.
- Update check: once a day FlexFlyout asks GitHub for the latest release,
  and when a newer one is out, offers its page at the top of the tray menu
  and says so once with a notification (on Windows, clicking it opens the
  page). Nothing is downloaded or installed. `general.check_updates =
  false` (or the setting in the window) turns it off.

## [0.5.0] - 2026-11-05

### Added
- A settings button (a gear) at the right end of the flyout's volume row
  opens the settings window, as Settings… in the tray menu does.

## [0.4.0] - 2026-11-05

### Added
- KDE Plasma: the `native_osd` setting now applies to Plasma's own volume
  popup. "avoid" (the default) keeps the flyout clear of where KWin puts
  it (centered, two thirds down the screen); "hide" turns plasma-pa's
  volume and mute popups off while FlexFlyout runs (brightness and the
  others stay) and puts them back as they were when it quits, or at its
  next start if it was killed. The settings window shows the option on
  KDE Plasma.

## [0.3.2] - 2026-11-02

### Fixed
- Linux: the centered anchors (top, left, center, right, bottom) put the
  flyout in the wrong place on KDE Plasma, with its controls on top of
  each other. The flyout was anchored to both edges of a centered axis,
  which KWin sized differently; it is now placed from the start edge,
  centered on the output by its size.

## [0.3.1] - 2026-11-02

### Fixed
- Linux: changing the size, or an anchor other than the top right (on KDE
  Plasma), ended FlexFlyout as soon as the flyout showed. A frame was
  drawn at the size asked for while the surface still had the size the
  compositor had given it; the frames now follow the compositor's size,
  and a graphics error loses a frame (logged) instead of ending the app.

## [0.3.0] - 2026-11-02

### Added
- Linux: FlexFlyout runs on Wayland desktops with layer-shell — KDE
  Plasma, sway, Hyprland and other wlroots ones (not GNOME, not X11). It
  follows MPRIS players and the PipeWire or PulseAudio volume, with the
  same flyout, controls, pinning and dragging, tray menu (StatusNotifier-
  Item), settings window and "Start at login" (an XDG autostart entry).
  Cover art comes from files, data: URLs, or the web address a player
  gives (Spotify) — the only network access.
- Linux release archive (`x86_64-unknown-linux-gnu`, glibc 2.35 or newer)
  with a desktop entry and the icon, and a PKGBUILD for Arch Linux and
  derivatives.
- The settings window shows the version, with links to the website and to
  supporting the development.

### Changed
- The playback buttons are grayed out while no player is there.
- egui's bundled fonts are no longer built in (their licence is not an OSI
  one): on Windows the flyout uses the Windows fonts (Segoe UI and Yu
  Gothic, or Arial and Meiryo where they are missing), as before, and on
  Linux the desktop's (through fontconfig). The executable is about
  0.8 MB smaller.

## [0.2.0] - 2026-11-02

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

[Unreleased]: https://github.com/akivajp/flexflyout/compare/v0.6.0...HEAD
[0.6.0]: https://github.com/akivajp/flexflyout/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/akivajp/flexflyout/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/akivajp/flexflyout/compare/v0.3.2...v0.4.0
[0.3.2]: https://github.com/akivajp/flexflyout/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/akivajp/flexflyout/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/akivajp/flexflyout/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/akivajp/flexflyout/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/akivajp/flexflyout/releases/tag/v0.1.0
