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

    /// Whether the two rectangles share any pixel.
    pub fn intersects(&self, other: &PixelRect) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    /// The rectangle grown by `margin` pixels on every side.
    pub fn expanded(&self, margin: i32) -> Self {
        Self {
            left: self.left - margin,
            top: self.top - margin,
            right: self.right + margin,
            bottom: self.bottom + margin,
        }
    }

    /// The same rectangle moved by `(dx, dy)`.
    pub fn translated(&self, dx: i32, dy: i32) -> Self {
        Self::from_origin_size(self.left + dx, self.top + dy, self.width(), self.height())
    }

    /// Whether `other` lies entirely inside.
    pub fn encloses(&self, other: &PixelRect) -> bool {
        self.left <= other.left
            && self.top <= other.top
            && other.right <= self.right
            && other.bottom <= self.bottom
    }
}

/// Size of the Windows 11 volume indicator at 100% display scaling, in
/// pixels, including its transparent margin (measured on 26H2).
pub const NATIVE_OSD_SIZE: (i32, i32) = (234, 62);

/// Where the Windows 11 volume indicator appears on a monitor, before it
/// has been seen: centered at the bottom of the work area.
pub fn native_osd_estimate(monitor: &MonitorArea) -> PixelRect {
    let work = monitor.work;
    let width = (NATIVE_OSD_SIZE.0 as f32 * monitor.scale).round() as i32;
    let height = (NATIVE_OSD_SIZE.1 as f32 * monitor.scale).round() as i32;
    PixelRect::from_origin_size(
        work.left + (work.width() - width) / 2,
        work.bottom - height,
        width,
        height,
    )
}

/// Moves `rect` the shortest distance that keeps it `gap` pixels clear of
/// `obstacle` and inside `work`: up or down first (the flyout keeps its
/// column), then left or right. Returns `rect` unchanged if it does not
/// overlap, or if no such position exists.
pub fn avoid(rect: PixelRect, obstacle: PixelRect, work: PixelRect, gap: i32) -> PixelRect {
    let blocked = obstacle.expanded(gap);
    if !rect.intersects(&blocked) {
        return rect;
    }
    let vertical = [
        rect.translated(0, blocked.top - rect.bottom),
        rect.translated(0, blocked.bottom - rect.top),
    ];
    let horizontal = [
        rect.translated(blocked.left - rect.right, 0),
        rect.translated(blocked.right - rect.left, 0),
    ];
    let distance =
        |moved: &PixelRect| (moved.left - rect.left).abs() + (moved.top - rect.top).abs();
    let best = |candidates: [PixelRect; 2]| {
        candidates
            .into_iter()
            .filter(|moved| work.encloses(moved))
            .min_by_key(distance)
    };
    best(vertical).or_else(|| best(horizontal)).unwrap_or(rect)
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

/// The placement settings that put a flyout of `rect` where it is now: the
/// rest of `display` with the monitor, anchor and offsets changed (for a
/// flyout the user dragged there).
///
/// The monitor is the one under the middle of `rect` (or the nearest one);
/// the monitor setting stays if it already leads there (with the pointer
/// at `cursor`). The anchor is the one of the nine regions of the work
/// area the middle lies in, so that the flyout keeps its place relative to
/// the nearest edges. A rectangle partly outside the work area is taken
/// as moved back inside, as [`place`] would put it.
pub fn display_for(
    rect: PixelRect,
    display: &Display,
    monitors: &[MonitorArea],
    cursor: Option<(i32, i32)>,
) -> Display {
    let mut result = display.clone();
    let middle = (rect.left + rect.width() / 2, rect.top + rect.height() / 2);
    // Squared distance from the middle to a monitor (0 inside it).
    let distance = |monitor: &MonitorArea| {
        let b = monitor.bounds;
        let dx = (b.left - middle.0).max(middle.0 - (b.right - 1)).max(0) as i64;
        let dy = (b.top - middle.1).max(middle.1 - (b.bottom - 1)).max(0) as i64;
        dx * dx + dy * dy
    };
    let Some(index) = (0..monitors.len()).min_by_key(|&i| distance(&monitors[i])) else {
        return result;
    };
    let monitor = &monitors[index];
    if choose_monitor(monitors, display.monitor, cursor) != Some(index) {
        result.monitor = Monitor::Number(index as u32 + 1);
    }

    let work = monitor.work;
    let (width, height) = (rect.width(), rect.height());
    let left = rect.left.min(work.right - width).max(work.left);
    let top = rect.top.min(work.bottom - height).max(work.top);
    // Which third of the work area the middle is in, along each axis.
    let third = |position: i32, start: i32, length: i32| {
        let offset = (position - start) * 3;
        if offset < length {
            Edge::Start
        } else if offset < 2 * length {
            Edge::Center
        } else {
            Edge::End
        }
    };
    let column = third(left + width / 2, work.left, work.width());
    let row = third(top + height / 2, work.top, work.height());
    result.anchor = match (column, row) {
        (Edge::Start, Edge::Start) => Anchor::TopLeft,
        (Edge::Center, Edge::Start) => Anchor::Top,
        (Edge::End, Edge::Start) => Anchor::TopRight,
        (Edge::Start, Edge::Center) => Anchor::Left,
        (Edge::Center, Edge::Center) => Anchor::Center,
        (Edge::End, Edge::Center) => Anchor::Right,
        (Edge::Start, Edge::End) => Anchor::BottomLeft,
        (Edge::Center, Edge::End) => Anchor::Bottom,
        (Edge::End, Edge::End) => Anchor::BottomRight,
    };
    // The offsets as `place` measures them, in pixels at 100% scaling.
    let to_points = |pixels: i32| (pixels as f32 / monitor.scale).round() as i32;
    result.offset_x = to_points(match column {
        Edge::Start => left - work.left,
        Edge::Center => left - (work.left + (work.width() - width) / 2),
        Edge::End => work.right - width - left,
    });
    result.offset_y = to_points(match row {
        Edge::Start => top - work.top,
        Edge::Center => top - (work.top + (work.height() - height) / 2),
        Edge::End => work.bottom - height - top,
    });
    result
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
    fn the_native_indicator_is_estimated_at_the_bottom_center() {
        // The indicator measured on a 1280 × 800 screen with a 48-pixel
        // taskbar: (523, 690) to (757, 752).
        let monitor = MonitorArea {
            bounds: PixelRect::from_origin_size(0, 0, 1280, 800),
            work: PixelRect::from_origin_size(0, 0, 1280, 752),
            scale: 1.0,
            primary: true,
        };
        assert_eq!(
            native_osd_estimate(&monitor),
            PixelRect::from_origin_size(523, 690, 234, 62)
        );
        let scaled = native_osd_estimate(&MonitorArea {
            scale: 1.5,
            ..monitor
        });
        assert_eq!((scaled.width(), scaled.height()), (351, 93));
        assert_eq!(scaled.bottom, 752);
    }

    #[test]
    fn the_flyout_moves_out_of_the_way_of_an_obstacle() {
        let work = PixelRect::from_origin_size(0, 0, 1280, 752);
        let obstacle = PixelRect::from_origin_size(523, 690, 234, 62);
        // Bottom center, overlapping: moved up above it, keeping its column.
        let flyout = PixelRect::from_origin_size(450, 600, 380, 128);
        let moved = avoid(flyout, obstacle, work, 10);
        assert_eq!(moved, flyout.translated(0, 680 - 728));
        assert!(!moved.intersects(&obstacle.expanded(10)));
        // Far away: unchanged.
        let corner = PixelRect::from_origin_size(876, 24, 380, 128);
        assert_eq!(avoid(corner, obstacle, work, 10), corner);
        // Just inside the gap: still moved.
        let close = PixelRect::from_origin_size(450, 560, 380, 128);
        assert_eq!(avoid(close, obstacle, work, 10).bottom, 680);
    }

    #[test]
    fn avoiding_goes_sideways_when_up_and_down_do_not_fit() {
        // A work area too short to put the flyout above or below.
        let work = PixelRect::from_origin_size(0, 0, 1280, 200);
        let obstacle = PixelRect::from_origin_size(500, 100, 200, 100);
        let flyout = PixelRect::from_origin_size(450, 40, 200, 128);
        let moved = avoid(flyout, obstacle, work, 10);
        assert_eq!(moved.top, flyout.top);
        assert_eq!(moved.right, 490);
        // Nowhere to go: unchanged.
        let tiny = PixelRect::from_origin_size(0, 0, 300, 200);
        let flyout = PixelRect::from_origin_size(0, 40, 300, 128);
        let obstacle = PixelRect::from_origin_size(0, 100, 300, 100);
        assert_eq!(avoid(flyout, obstacle, tiny, 10), flyout);
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

    #[test]
    fn a_dragged_flyout_is_placed_there_again() {
        let monitors = monitors();
        let base = (400.0, 164.0);
        // Spots on both monitors, near every kind of anchor, and one
        // partly off the screen.
        let spots = [
            (30, 40),
            (760, 20),
            (1500, 860),
            (1700, 400),
            (900, 500),
            (1850, 1000),
            (-2400, -100),
            (-1300, 500),
            (-700, 1000),
        ];
        for (x, y) in spots {
            let start = Display::default();
            let monitor =
                choose_monitor(&monitors, Monitor::Number(if x < 0 { 2 } else { 1 }), None)
                    .unwrap();
            let size = place(&start, &monitors[monitor], base);
            let rect = PixelRect::from_origin_size(x, y, size.width(), size.height());
            let display = display_for(rect, &start, &monitors, Some((x + 10, y + 10)));
            let chosen =
                choose_monitor(&monitors, display.monitor, Some((x + 10, y + 10))).unwrap();
            assert_eq!(chosen, monitor, "monitor for {rect:?}");
            let again = place(&display, &monitors[chosen], base);
            // Inside the work area, within a pixel of rounding.
            let work = monitors[chosen].work;
            let expected_left = x.min(work.right - rect.width()).max(work.left);
            let expected_top = y.min(work.bottom - rect.height()).max(work.top);
            assert!(
                (again.left - expected_left).abs() <= 1 && (again.top - expected_top).abs() <= 1,
                "{rect:?} came back as {again:?} with {display:?}"
            );
        }
    }

    #[test]
    fn dragging_keeps_a_monitor_setting_that_still_fits() {
        let monitors = monitors();
        let rect = PixelRect::from_origin_size(100, 100, 400, 164);
        let primary = Display::default();
        assert_eq!(
            display_for(rect, &primary, &monitors, None).monitor,
            Monitor::Primary
        );
        let cursor = Display {
            monitor: Monitor::Cursor,
            ..Display::default()
        };
        assert_eq!(
            display_for(rect, &cursor, &monitors, Some((150, 150))).monitor,
            Monitor::Cursor
        );
        // Onto the other monitor: by number.
        let other = PixelRect::from_origin_size(-1000, 100, 600, 246);
        assert_eq!(
            display_for(other, &primary, &monitors, None).monitor,
            Monitor::Number(2)
        );
        assert_eq!(
            display_for(rect, &primary, &monitors, None).anchor,
            Anchor::TopLeft
        );
    }
}
