//! The Linux host's platform parts: the layer-shell window (Wayland), the
//! players (MPRIS), the volume (PulseAudio or PipeWire), the keys (KDE
//! Plasma's shortcuts), the color scheme, the tray icon, the desktop's
//! conventions and the fonts.

pub mod desktop;
pub mod fonts;
pub mod keys;
pub mod layer;
pub mod media;
pub mod native_osd;
pub mod theme;
pub mod tray;
pub mod volume;
pub mod web;
