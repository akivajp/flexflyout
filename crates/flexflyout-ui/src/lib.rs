//! Drawing of the FlexFlyout panel.
//!
//! The panel is drawn from a snapshot of what to show ([`FlyoutView`]) and
//! reports which button was clicked as a [`Command`]. It knows nothing about
//! windows or the OS: the host sizes the window to [`BASE_SIZE`] times its
//! scale, sets egui's zoom factor to that scale and calls [`draw`].

pub mod fonts;

use egui::{
    Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Role, Sense, Shape, Stroke, Ui,
    WidgetInfo, pos2, vec2,
};
use flexflyout_core::{Command, NowPlaying, PlaybackStatus, Volume};

/// Size of the panel at scale 1.0, in egui points.
pub const BASE_SIZE: egui::Vec2 = vec2(380.0, 128.0);

/// Texts shown on the panel, in the user's language.
///
/// The host translates them; this crate does not depend on the i18n system.
#[derive(Clone, Debug)]
pub struct Labels {
    /// Title shown when no player reports a track.
    pub nothing_playing: String,
    /// Shown next to the volume bar while muted.
    pub muted: String,
    /// Accessible name of the "previous track" button.
    pub previous: String,
    /// Accessible name of the "play / pause" button.
    pub play_pause: String,
    /// Accessible name of the "next track" button.
    pub next: String,
}

/// Everything the panel shows in one frame.
#[derive(Clone, Copy, Debug)]
pub struct FlyoutView<'a> {
    /// The current track, or `None` when nothing is playing.
    pub now_playing: Option<&'a NowPlaying>,
    /// The volume, or `None` to leave out the volume bar (e.g. when the
    /// system's own volume indicator is kept and already shows it).
    pub volume: Option<Volume>,
    /// Opacity of the panel's background, from the settings (0.0 to 1.0).
    /// The text and buttons stay opaque so that they remain readable.
    pub background_opacity: f32,
    /// Translated texts.
    pub labels: &'a Labels,
}

/// Background color of the panel (before the opacity is applied).
const BACKGROUND: Color32 = Color32::from_rgb(28, 28, 34);
/// Placeholder color for the cover art.
const COVER_PLACEHOLDER: Color32 = Color32::from_rgb(60, 60, 72);
/// Color of the filled part of the volume bar.
const VOLUME_FILL: Color32 = Color32::from_rgb(90, 160, 255);
/// Color of the volume bar while muted.
const VOLUME_MUTED: Color32 = Color32::from_gray(120);
/// Color of the empty part of the volume bar.
const VOLUME_TRACK: Color32 = Color32::from_gray(70);
/// Color of the secondary text (artist, percentage).
const SECONDARY_TEXT: Color32 = Color32::from_gray(190);

/// Background of the playback buttons: at rest, hovered and pressed
/// (translucent white, premultiplied).
const BUTTON_IDLE: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);
const BUTTON_HOVERED: Color32 = Color32::from_rgba_premultiplied(40, 40, 40, 40);
const BUTTON_PRESSED: Color32 = Color32::from_rgba_premultiplied(64, 64, 64, 64);

/// Margin around the content, in points.
const MARGIN: f32 = 16.0;
/// Gap between the cover art and the text, in points.
const GAP: f32 = 14.0;

/// Draws the panel over the whole of `ui`'s area and returns the command of
/// the button that was clicked in this frame, if any.
///
/// The caller fades the whole panel with [`Ui::multiply_opacity`].
pub fn draw(ui: &mut Ui, view: &FlyoutView<'_>) -> Option<Command> {
    let rect = ui.max_rect();
    let painter = ui.painter().clone();

    // Rounded background. Its alpha comes from the settings only.
    let alpha = (view.background_opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let background =
        Color32::from_rgba_unmultiplied(BACKGROUND.r(), BACKGROUND.g(), BACKGROUND.b(), alpha);
    painter.rect_filled(rect, CornerRadius::same(14), background);

    // Square cover art on the left, as tall as the content area.
    // Real cover art comes with the SMTC backend; until then a placeholder.
    let side = rect.height() - 2.0 * MARGIN;
    let cover = Rect::from_min_size(rect.min + vec2(MARGIN, MARGIN), vec2(side, side));
    painter.rect_filled(cover, CornerRadius::same(8), COVER_PLACEHOLDER);
    paint_note(&painter, cover.center(), side * 0.4, SECONDARY_TEXT);

    // Text and controls to the right of the cover art.
    let left = cover.right() + GAP;
    let right = rect.right() - MARGIN;
    let width = right - left;

    let (title, artist) = match view.now_playing {
        Some(track) => (track.title.as_str(), track.artist.as_str()),
        None => (view.labels.nothing_playing.as_str(), ""),
    };
    // Long titles are cut with "…" instead of wrapping out of the panel.
    let title_rect = Rect::from_min_size(pos2(left, rect.top() + MARGIN - 2.0), vec2(width, 24.0));
    put_left(
        ui,
        title_rect,
        egui::Label::new(RichText::new(title).size(18.0).color(Color32::WHITE)).truncate(),
    );
    if !artist.is_empty() {
        let artist_rect = Rect::from_min_size(pos2(left, title_rect.bottom()), vec2(width, 20.0));
        put_left(
            ui,
            artist_rect,
            egui::Label::new(RichText::new(artist).size(14.0).color(SECONDARY_TEXT)).truncate(),
        );
    }

    // Playback buttons. The icons are drawn as shapes rather than text:
    // egui's bundled fonts have no media symbols, and shapes stay sharp at
    // every scale.
    let mut command = None;
    let playing = view
        .now_playing
        .is_some_and(|t| t.status == PlaybackStatus::Playing);
    let buttons = [
        (
            Icon::Previous,
            view.labels.previous.as_str(),
            Command::Previous,
        ),
        (
            if playing { Icon::Pause } else { Icon::Play },
            view.labels.play_pause.as_str(),
            Command::TogglePlayPause,
        ),
        (Icon::Next, view.labels.next.as_str(), Command::Next),
    ];
    let button_size = vec2(36.0, 28.0);
    let buttons_top = rect.top() + MARGIN + 46.0;
    for (i, (icon, name, cmd)) in buttons.into_iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(left + i as f32 * (button_size.x + 6.0), buttons_top),
            button_size,
        );
        let response = ui.allocate_rect(r, Sense::click());
        // Screen readers (and tests) see the translated name.
        response.widget_info(|| WidgetInfo::labeled(Role::Button, true, name));
        // A faint rounded background, brighter on hover and while pressed.
        // Own colors rather than egui's theme, which follows the OS (light
        // or dark) while the panel is always dark.
        let fill = if response.is_pointer_button_down_on() {
            BUTTON_PRESSED
        } else if response.hovered() {
            BUTTON_HOVERED
        } else {
            BUTTON_IDLE
        };
        painter.rect_filled(r, CornerRadius::same(6), fill);
        paint_icon(&painter, icon, r.center(), 7.0, Color32::WHITE);
        // No tooltip: it would not fit in the small window.
        if response.clicked() {
            command = Some(cmd);
        }
    }

    // Volume bar along the bottom of the text area, with the level on the right.
    if let Some(volume) = view.volume {
        let label = if volume.muted {
            view.labels.muted.clone()
        } else {
            format!("{}%", volume.percent())
        };
        let label_width = 52.0;
        let bar_y = cover.bottom() - 6.0;
        let bar = Rect::from_min_max(pos2(left, bar_y), pos2(right - label_width, bar_y + 6.0));
        painter.rect_filled(bar, CornerRadius::same(3), VOLUME_TRACK);
        let filled = Rect::from_min_size(
            bar.min,
            vec2(bar.width() * volume.level.clamp(0.0, 1.0), bar.height()),
        );
        painter.rect_filled(
            filled,
            CornerRadius::same(3),
            if volume.muted {
                VOLUME_MUTED
            } else {
                VOLUME_FILL
            },
        );
        painter.text(
            pos2(right, bar.center().y),
            Align2::RIGHT_CENTER,
            label,
            FontId::proportional(13.0),
            SECONDARY_TEXT,
        );
    }

    command
}

/// Adds `widget` at the left edge of `rect`, vertically centered
/// (`Ui::put` would center it horizontally as well).
fn put_left(ui: &mut Ui, rect: Rect, widget: impl egui::Widget) {
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(layout), |ui| {
        ui.add(widget)
    });
}

/// Icons of the playback buttons.
#[derive(Clone, Copy, Debug)]
enum Icon {
    Previous,
    Play,
    Pause,
    Next,
}

/// Draws `icon` centered on `center`, `r` points from the center to the edge.
fn paint_icon(painter: &egui::Painter, icon: Icon, center: Pos2, r: f32, color: Color32) {
    // A triangle pointing right (or left), `r` wide on each side of `x`.
    let triangle = |x: f32, right: bool| {
        let (tip, base) = if right {
            (x + r, x - r)
        } else {
            (x - r, x + r)
        };
        vec![
            pos2(base, center.y - r),
            pos2(tip, center.y),
            pos2(base, center.y + r),
        ]
    };
    let bar = |x: f32| Rect::from_center_size(pos2(x, center.y), vec2(r * 0.4, r * 2.0));
    match icon {
        Icon::Play => {
            let shape = triangle(center.x + r * 0.15, true);
            painter.add(Shape::convex_polygon(shape, color, Stroke::NONE));
        }
        Icon::Pause => {
            painter.rect_filled(bar(center.x - r * 0.45), 1.0, color);
            painter.rect_filled(bar(center.x + r * 0.45), 1.0, color);
        }
        Icon::Previous => {
            painter.rect_filled(bar(center.x - r * 0.8), 1.0, color);
            painter.add(Shape::convex_polygon(
                triangle(center.x + r * 0.2, false),
                color,
                Stroke::NONE,
            ));
        }
        Icon::Next => {
            painter.rect_filled(bar(center.x + r * 0.8), 1.0, color);
            painter.add(Shape::convex_polygon(
                triangle(center.x - r * 0.2, true),
                color,
                Stroke::NONE,
            ));
        }
    }
}

/// Draws a musical note (the cover art placeholder), `size` points tall.
fn paint_note(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let head_r = size * 0.18;
    let head = pos2(center.x - size * 0.12, center.y + size * 0.5 - head_r);
    let stem_x = head.x + head_r * 0.9;
    let top = center.y - size * 0.5;
    painter.circle_filled(head, head_r, color);
    painter.line_segment(
        [pos2(stem_x, head.y), pos2(stem_x, top)],
        Stroke::new(size * 0.06, color),
    );
    // The flag.
    painter.line_segment(
        [
            pos2(stem_x, top),
            pos2(stem_x + size * 0.28, top + size * 0.22),
        ],
        Stroke::new(size * 0.06, color),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable as _;

    fn labels() -> Labels {
        Labels {
            nothing_playing: "Nothing playing".into(),
            muted: "Muted".into(),
            previous: "Previous".into(),
            play_pause: "Play / pause".into(),
            next: "Next".into(),
        }
    }

    /// Draws the panel in a test harness and collects the clicked commands.
    fn harness<'a>(track: &'a NowPlaying, labels: &'a Labels) -> Harness<'a, Vec<Command>> {
        Harness::builder().with_size(BASE_SIZE).build_ui_state(
            move |ui, clicked: &mut Vec<Command>| {
                let view = FlyoutView {
                    now_playing: Some(track),
                    volume: Some(Volume {
                        level: 0.42,
                        muted: false,
                    }),
                    background_opacity: 0.9,
                    labels,
                };
                clicked.extend(draw(ui, &view));
            },
            Vec::new(),
        )
    }

    #[test]
    fn buttons_send_their_commands() {
        let track = NowPlaying {
            title: "Title".into(),
            artist: "Artist".into(),
            status: PlaybackStatus::Playing,
        };
        let labels = labels();
        let mut harness = harness(&track, &labels);
        for (name, expected) in [
            ("Previous", Command::Previous),
            ("Play / pause", Command::TogglePlayPause),
            ("Next", Command::Next),
        ] {
            harness.get_by_label(name).click();
            harness.run();
            assert_eq!(harness.state().last(), Some(&expected), "button {name}");
        }
        assert_eq!(harness.state().len(), 3);
    }

    #[test]
    fn shows_the_track_and_the_volume() {
        let track = NowPlaying {
            title: "Title".into(),
            artist: "Artist".into(),
            status: PlaybackStatus::Paused,
        };
        let labels = labels();
        let harness = harness(&track, &labels);
        harness.get_by_label("Title");
        harness.get_by_label("Artist");
    }
}
