//! Where the flyout goes: choosing the monitor and placing the window in its
//! work area (the screen without the taskbar), in physical pixels.
//!
//! The host lists the monitors and reads the pointer position; everything
//! else is decided here, without side effects, so it can be tested.

use crate::config::{Anchor, Display, Monitor};

/// A rectangle in physical pixels of the virtual screen (all monitors
/// together; the primary monitor's top-left corner is the origin, so other
/// monitors can have negative coordinates).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelRect {
    /// Left edge (inclusive).
    pub left: i32,
    /// Top edge (inclusive).
    pub top: i32,
    /// Right edge (exclusive).
    pub right: i32,
    /// Bottom edge (exclusive).
    pub bottom: i32,
}

impl PixelRect {
    /// A rectangle from its top-left corner and its size.
    pub fn from_origin_size(left: i32, top: i32, width: i32, height: i32) -> Self {
        Self {
            left,
            top,
            right: left + width,
            bottom: top + height,
        }
    }

    /// Width in pixels.
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    /// Height in pixels.
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    /// Whether the point is inside.
    pub fn contains(&self, (x, y): (i32, i32)) -> bool {
        (self.left..self.right).contains(&x) && (self.top..self.bottom).contains(&y)
    }
}

/// One monitor, as the host reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorArea {
    /// The whole monitor.
    pub bounds: PixelRect,
    /// The part not covered by the taskbar and docked bars.
    pub work: PixelRect,
    /// The monitor's display scaling (1.0 = 100%, 1.5 = 150%).
    pub scale: f32,
    /// Whether this is the primary monitor.
    pub primary: bool,
}

/// Picks the monitor to show the flyout on: its index in `monitors` (in the
/// order the host listed them), or `None` if there are none.
///
/// A monitor number that does not exist and a pointer outside every monitor
/// fall back to the primary monitor (or the first one, should none be
/// marked primary).
pub fn choose_monitor(
    monitors: &[MonitorArea],
    choice: Monitor,
    cursor: Option<(i32, i32)>,
) -> Option<usize> {
    let primary = || {
        monitors
            .iter()
            .position(|m| m.primary)
            .or((!monitors.is_empty()).then_some(0))
    };
    match choice {
        Monitor::Primary => primary(),
        Monitor::Cursor => cursor
            .and_then(|point| monitors.iter().position(|m| m.bounds.contains(point)))
            .or_else(primary),
        Monitor::Number(n) => {
            let index = n as usize - 1;
            (index < monitors.len()).then_some(index).or_else(primary)
        }
    }
}

/// The window's rectangle on `monitor` for a panel of `base_size` (egui
/// points at scale 1.0), following the placement settings.
///
/// The size and the offsets grow with both the setting's scale and the
/// monitor's display scaling, so the flyout looks the same on every
/// monitor. The result always lies inside the work area when it fits.
pub fn place(display: &Display, monitor: &MonitorArea, base_size: (f32, f32)) -> PixelRect {
    let work = monitor.work;
    let to_pixels = |points: f32| (points * monitor.scale).round() as i32;
    let width = to_pixels(base_size.0 * display.scale);
    let height = to_pixels(base_size.1 * display.scale);
    let dx = to_pixels(display.offset_x as f32);
    let dy = to_pixels(display.offset_y as f32);

    // Horizontal and vertical position for each of the three columns and
    // rows: from the start edge, centered, from the end edge (inwards).
    let (column, row) = anchor_grid(display.anchor);
    let x = match column {
        Edge::Start => work.left + dx,
        Edge::Center => work.left + (work.width() - width) / 2 + dx,
        Edge::End => work.right - width - dx,
    };
    let y = match row {
        Edge::Start => work.top + dy,
        Edge::Center => work.top + (work.height() - height) / 2 + dy,
        Edge::End => work.bottom - height - dy,
    };

    // Keep the whole flyout on the monitor, so that a large offset never
    // hides it. When it is larger than the work area, its top-left corner
    // stays visible (that is where the cover art and the title are).
    let x = x.min(work.right - width).max(work.left);
    let y = y.min(work.bottom - height).max(work.top);
    PixelRect::from_origin_size(x, y, width, height)
}

/// Where along one axis an anchor lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    /// Left or top.
    Start,
    /// The middle.
    Center,
    /// Right or bottom.
    End,
}

/// The column and the row of an anchor in the 3 × 3 grid.
fn anchor_grid(anchor: Anchor) -> (Edge, Edge) {
    use Edge::*;
    match anchor {
        Anchor::TopLeft => (Start, Start),
        Anchor::Top => (Center, Start),
        Anchor::TopRight => (End, Start),
        Anchor::Left => (Start, Center),
        Anchor::Center => (Center, Center),
        Anchor::Right => (End, Center),
        Anchor::BottomLeft => (Start, End),
        Anchor::Bottom => (Center, End),
        Anchor::BottomRight => (End, End),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1920 × 1080 primary monitor with a 48-pixel taskbar at the bottom,
    /// and a 2560 × 1440 monitor at 150% on its left.
    fn monitors() -> Vec<MonitorArea> {
        vec![
            MonitorArea {
                bounds: PixelRect::from_origin_size(0, 0, 1920, 1080),
                work: PixelRect::from_origin_size(0, 0, 1920, 1032),
                scale: 1.0,
                primary: true,
            },
            MonitorArea {
                bounds: PixelRect::from_origin_size(-2560, -200, 2560, 1440),
                work: PixelRect::from_origin_size(-2560, -200, 2560, 1440),
                scale: 1.5,
                primary: false,
            },
        ]
    }

    fn display(anchor: Anchor, offset_x: i32, offset_y: i32) -> Display {
        Display {
            anchor,
            offset_x,
            offset_y,
            ..Display::default()
        }
    }

    const BASE: (f32, f32) = (380.0, 128.0);

    #[test]
    fn the_monitor_follows_the_choice() {
        let monitors = monitors();
        assert_eq!(choose_monitor(&monitors, Monitor::Primary, None), Some(0));
        assert_eq!(choose_monitor(&monitors, Monitor::Number(2), None), Some(1));
        assert_eq!(
            choose_monitor(&monitors, Monitor::Cursor, Some((-100, 500))),
            Some(1)
        );
        assert_eq!(
            choose_monitor(&monitors, Monitor::Cursor, Some((100, 500))),
            Some(0)
        );
    }

    #[test]
    fn missing_monitors_fall_back_to_the_primary() {
        let mut monitors = monitors();
        monitors.swap(0, 1); // The primary one is not the first.
        assert_eq!(choose_monitor(&monitors, Monitor::Number(3), None), Some(1));
        assert_eq!(
            choose_monitor(&monitors, Monitor::Cursor, Some((99_999, 0))),
            Some(1)
        );
        assert_eq!(choose_monitor(&monitors, Monitor::Cursor, None), Some(1));
        assert_eq!(choose_monitor(&[], Monitor::Primary, None), None);
    }

    #[test]
    fn the_nine_anchors_with_offsets_measured_inwards() {
        let monitor = monitors()[0];
        let at = |anchor| {
            let r = place(&display(anchor, 24, 10), &monitor, BASE);
            (r.left, r.top)
        };
        // Work area: 1920 × 1032; panel: 380 × 128.
        assert_eq!(at(Anchor::TopLeft), (24, 10));
        assert_eq!(at(Anchor::Top), ((1920 - 380) / 2 + 24, 10));
        assert_eq!(at(Anchor::TopRight), (1920 - 380 - 24, 10));
        assert_eq!(at(Anchor::Left), (24, (1032 - 128) / 2 + 10));
        assert_eq!(at(Anchor::Center), (794, 462));
        assert_eq!(at(Anchor::Right), (1516, 462));
        assert_eq!(at(Anchor::BottomLeft), (24, 1032 - 128 - 10));
        assert_eq!(at(Anchor::Bottom), (794, 894));
        assert_eq!(at(Anchor::BottomRight), (1516, 894));
    }

    #[test]
    fn size_and_offsets_follow_both_scales() {
        let monitor = monitors()[1]; // 150%
        let settings = Display {
            scale: 2.0,
            ..display(Anchor::TopLeft, 20, 20)
        };
        let r = place(&settings, &monitor, BASE);
        assert_eq!((r.width(), r.height()), (1140, 384));
        assert_eq!((r.left, r.top), (-2560 + 30, -200 + 30));
    }

    #[test]
    fn the_flyout_stays_inside_the_work_area() {
        let monitor = monitors()[0];
        // Offsets that would push it off the screen on either side.
        let r = place(&display(Anchor::TopRight, -500, -500), &monitor, BASE);
        assert_eq!((r.left, r.top), (1920 - 380, 0));
        let r = place(&display(Anchor::TopLeft, 5000, 5000), &monitor, BASE);
        assert_eq!((r.right, r.bottom), (1920, 1032));
        // Larger than the monitor: the top-left corner stays visible.
        let huge = Display {
            scale: 4.0,
            ..display(Anchor::BottomRight, 0, 0)
        };
        let small = MonitorArea {
            work: PixelRect::from_origin_size(0, 0, 800, 400),
            ..monitor
        };
        let r = place(&huge, &small, BASE);
        assert_eq!((r.left, r.top), (0, 0));
        assert_eq!((r.width(), r.height()), (1520, 512));
    }
}
