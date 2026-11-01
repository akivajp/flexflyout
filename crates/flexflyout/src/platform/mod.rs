//! OS-specific parts: reading the media and the volume, and the window.

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;
