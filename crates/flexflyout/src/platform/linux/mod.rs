//! The Linux host's platform parts: the layer-shell window (Wayland), the
//! players (MPRIS), the volume (PulseAudio or PipeWire), the tray icon,
//! the desktop's conventions and the fonts.

pub mod desktop;
pub mod fonts;
pub mod layer;
pub mod media;
pub mod native_osd;
pub mod tray;
pub mod volume;
