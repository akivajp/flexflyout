//! Drawing of the FlexFlyout panel.
//!
//! The panel is drawn from a snapshot of what to show ([`FlyoutView`]) and
//! reports what the user did with its controls as a [`Command`]. It knows
//! nothing about windows or the OS: the host sizes the window to
//! [`BASE_SIZE`] times its scale, sets egui's zoom factor to that scale and
//! calls [`draw`].

pub mod fonts;

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{
    Align2, Color32, ColorImage, CornerRadius, FontId, Pos2, Rect, Response, RichText, Role, Sense,
    Shape, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, WidgetInfo, pos2, vec2,
};
use flexflyout_core::{Command, Cover, NowPlaying, PlaybackStatus, RepeatMode, Volume};

/// Size of the panel at scale 1.0, in egui points.
pub const BASE_SIZE: egui::Vec2 = vec2(400.0, 164.0);

/// Texts shown on the panel, in the user's language.
///
/// The host translates them; this crate does not depend on the i18n system.
/// Most are the accessible names of the controls.
#[derive(Clone, Debug)]
pub struct Labels {
    /// Title shown when no player reports a track.
    pub nothing_playing: String,
    /// Shown next to the volume bar while muted.
    pub muted: String,
    /// The "previous track" button.
    pub previous: String,
    /// The "play / pause" button.
    pub play_pause: String,
    /// The "next track" button.
    pub next: String,
    /// The position bar (seek bar).
    pub position: String,
    /// The shuffle button.
    pub shuffle: String,
    /// The repeat button.
    pub repeat: String,
    /// The volume bar.
    pub volume: String,
    /// The mute button.
    pub mute: String,
    /// The pin button (keeps the flyout shown).
    pub pin: String,
    /// The close button (hides the flyout now).
    pub close: String,
}

/// Everything the panel shows in one frame.
#[derive(Clone, Copy, Debug)]
pub struct FlyoutView<'a> {
    /// The current track, or `None` when nothing is playing.
    pub now_playing: Option<&'a NowPlaying>,
    /// The volume, or `None` to leave out the volume bar (e.g. when the
    /// system's own volume indicator is kept and already shows it).
    pub volume: Option<Volume>,
    /// Whether the flyout is pinned (stays until unpinned).
    pub pinned: bool,
    /// The time to show the track's position at.
    pub now: Instant,
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
/// Color of the filled part of the bars, and of the buttons that are on.
const ACCENT: Color32 = Color32::from_rgb(90, 160, 255);
/// Color of the volume bar while muted.
const VOLUME_MUTED: Color32 = Color32::from_gray(120);
/// Color of the empty part of the bars.
const BAR_TRACK: Color32 = Color32::from_gray(70);
/// Color of the secondary text (artist, times, percentage).
const SECONDARY_TEXT: Color32 = Color32::from_gray(190);
/// Color of the icons of controls the player does not offer.
const DISABLED: Color32 = Color32::from_gray(95);

/// Background of the buttons: at rest, hovered and pressed (translucent
/// white, premultiplied).
const BUTTON_IDLE: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);
const BUTTON_HOVERED: Color32 = Color32::from_rgba_premultiplied(40, 40, 40, 40);
const BUTTON_PRESSED: Color32 = Color32::from_rgba_premultiplied(64, 64, 64, 64);

/// Margin around the content, in points.
const MARGIN: f32 = 16.0;
/// Gap between the cover art and the text, in points.
const GAP: f32 = 14.0;
/// Size of the playback buttons, and the gap between them.
const BUTTON_SIZE: egui::Vec2 = vec2(36.0, 28.0);
const BUTTON_GAP: f32 = 6.0;
/// Size of the small buttons (pin, mute).
const SMALL_BUTTON_SIZE: egui::Vec2 = vec2(28.0, 24.0);
/// Width kept for a time or the percentage next to a bar.
const BAR_LABEL_WIDTH: f32 = 40.0;

/// Draws the panel over the whole of `ui`'s area and returns what the user
/// did with its controls in this frame, if anything.
///
/// The caller fades the whole panel with [`Ui::multiply_opacity`].
pub fn draw(ui: &mut Ui, view: &FlyoutView<'_>) -> Option<Command> {
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    let labels = view.labels;
    let mut command = None;

    // Rounded background. Its alpha comes from the settings only.
    let alpha = (view.background_opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let background =
        Color32::from_rgba_unmultiplied(BACKGROUND.r(), BACKGROUND.g(), BACKGROUND.b(), alpha);
    painter.rect_filled(rect, CornerRadius::same(14), background);

    // A pinned flyout can be dragged by its background. The controls are
    // added later, on top, so they keep their clicks and drags.
    // It senses clicks too, so that a drag starts only once the pointer
    // moved a little: a press on a control must stay a click.
    let grab = view
        .pinned
        .then(|| ui.interact(rect, ui.scope_id().with("move"), Sense::click_and_drag()));
    // Whether the pointer is on a control (where the "move" cursor would
    // be wrong, although a drag there moves the flyout too).
    let mut on_control = false;

    // Square cover art on the left, as tall as the content area, or a
    // placeholder when the player gives none.
    let side = rect.height() - 2.0 * MARGIN;
    let cover = Rect::from_min_size(rect.min + vec2(MARGIN, MARGIN), vec2(side, side));
    let art = view.now_playing.and_then(|track| track.cover.as_ref());
    if !art.is_some_and(|art| paint_cover(ui, art, cover)) {
        painter.rect_filled(cover, CornerRadius::same(8), COVER_PLACEHOLDER);
        paint_note(&painter, cover.center(), side * 0.4, SECONDARY_TEXT);
    }

    // Text and controls to the right of the cover art.
    let left = cover.right() + GAP;
    let right = rect.right() - MARGIN;

    // The close and pin buttons in the top-right corner.
    let close = Rect::from_min_size(
        pos2(right - SMALL_BUTTON_SIZE.x, rect.top() + MARGIN - 4.0),
        SMALL_BUTTON_SIZE,
    );
    if button(
        ui,
        close,
        Icon::Close,
        &labels.close,
        Some(SECONDARY_TEXT),
        &mut on_control,
    ) {
        command = Some(Command::Close);
    }
    let pin = close.translate(vec2(-(SMALL_BUTTON_SIZE.x + 4.0), 0.0));
    let pin_color = if view.pinned { ACCENT } else { SECONDARY_TEXT };
    if button(
        ui,
        pin,
        Icon::Pin,
        &labels.pin,
        Some(pin_color),
        &mut on_control,
    ) {
        command = Some(Command::Pin(!view.pinned));
    }

    let (title, artist) = match view.now_playing {
        Some(track) => (track.title.as_str(), track.artist.as_str()),
        None => (labels.nothing_playing.as_str(), ""),
    };
    // Long titles are cut with "…" instead of wrapping out of the panel.
    let text_width = pin.left() - 4.0 - left;
    let title_rect = Rect::from_min_size(
        pos2(left, rect.top() + MARGIN - 2.0),
        vec2(text_width, 24.0),
    );
    put_left(
        ui,
        title_rect,
        // Not selectable: a selectable label takes the drags meant for
        // moving the pinned flyout.
        egui::Label::new(RichText::new(title).size(18.0).color(Color32::WHITE))
            .truncate()
            .selectable(false),
    );
    if !artist.is_empty() {
        let artist_rect =
            Rect::from_min_size(pos2(left, title_rect.bottom()), vec2(text_width, 20.0));
        put_left(
            ui,
            artist_rect,
            egui::Label::new(RichText::new(artist).size(14.0).color(SECONDARY_TEXT))
                .truncate()
                .selectable(false),
        );
    }

    // The position in the track, with the time played on the left and the
    // length on the right. Left out when the player does not tell them.
    let track = view.now_playing;
    if let Some((timeline, position)) =
        track.and_then(|track| track.timeline.zip(track.position_at(view.now)))
    {
        let y = rect.top() + MARGIN + 54.0;
        let bar = Rect::from_min_max(
            pos2(left + BAR_LABEL_WIDTH, y - 2.0),
            pos2(right - BAR_LABEL_WIDTH, y + 2.0),
        );
        let length = timeline.length.as_secs_f32();
        let value = position.as_secs_f32() / length;
        let (response, shown) = slider(ui, bar, value, &labels.position, timeline.seekable);
        on_control |= response.hovered();
        paint_bar(&painter, bar, shown, ACCENT, &response);
        // Seek once the button is released: players stutter when asked to
        // seek on every frame of a drag.
        if response.clicked() || response.drag_stopped() {
            command = Some(Command::Seek(Duration::from_secs_f32(shown * length)));
        }
        let font = FontId::proportional(12.0);
        let played = Duration::from_secs_f32(shown * length);
        painter.text(
            pos2(left, y),
            Align2::LEFT_CENTER,
            format_time(played),
            font.clone(),
            SECONDARY_TEXT,
        );
        painter.text(
            pos2(right, y),
            Align2::RIGHT_CENTER,
            format_time(timeline.length),
            font,
            SECONDARY_TEXT,
        );
    }

    // Playback buttons. The icons are drawn as shapes rather than text:
    // egui's bundled fonts have no media symbols, and shapes stay sharp at
    // every scale. Shuffle and repeat show whether they are on, and are
    // grayed out when the player cannot change them; all of them are
    // grayed out when no player is there.
    let playing = track.is_some_and(|t| t.status == PlaybackStatus::Playing);
    let player = track.is_some().then_some(Color32::WHITE);
    let shuffle = track.and_then(|t| t.shuffle);
    let repeat = track.and_then(|t| t.repeat);
    let toggle_color = |on: Option<bool>| match on {
        Some(true) => Some(ACCENT),
        Some(false) => Some(Color32::WHITE),
        None => None,
    };
    let buttons = [
        (
            Icon::Shuffle,
            &labels.shuffle,
            toggle_color(shuffle),
            shuffle.map(|on| Command::SetShuffle(!on)),
        ),
        (
            Icon::Previous,
            &labels.previous,
            player,
            Some(Command::Previous),
        ),
        (
            if playing { Icon::Pause } else { Icon::Play },
            &labels.play_pause,
            player,
            Some(Command::TogglePlayPause),
        ),
        (Icon::Next, &labels.next, player, Some(Command::Next)),
        (
            if repeat == Some(RepeatMode::Track) {
                Icon::RepeatTrack
            } else {
                Icon::Repeat
            },
            &labels.repeat,
            toggle_color(repeat.map(|mode| mode != RepeatMode::Off)),
            repeat.map(|mode| Command::SetRepeat(mode.next())),
        ),
    ];
    let buttons_top = rect.top() + MARGIN + 66.0;
    for (i, (icon, name, color, cmd)) in buttons.into_iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(left + i as f32 * (BUTTON_SIZE.x + BUTTON_GAP), buttons_top),
            BUTTON_SIZE,
        );
        if button(ui, r, icon, name, color, &mut on_control) {
            command = cmd;
        }
    }

    // Mute button and volume bar along the bottom, with the level on the
    // right. The volume follows the pointer while the bar is held.
    if let Some(volume) = view.volume {
        let y = cover.bottom() - SMALL_BUTTON_SIZE.y / 2.0;
        let mute = Rect::from_center_size(
            pos2(left + SMALL_BUTTON_SIZE.x / 2.0 - 4.0, y),
            SMALL_BUTTON_SIZE,
        );
        let icon = if volume.muted {
            Icon::SpeakerMuted
        } else {
            Icon::Speaker
        };
        if button(
            ui,
            mute,
            icon,
            &labels.mute,
            Some(SECONDARY_TEXT),
            &mut on_control,
        ) {
            command = Some(Command::ToggleMute);
        }
        // Room for the wider of "100%" and the "muted" text (long in some
        // languages), so that the bar keeps its length when muting.
        let font = FontId::proportional(13.0);
        let label_width = [labels.muted.as_str(), "100%"]
            .into_iter()
            .map(|text| {
                painter
                    .layout_no_wrap(text.to_owned(), font.clone(), SECONDARY_TEXT)
                    .size()
                    .x
            })
            .fold(BAR_LABEL_WIDTH - 8.0, f32::max);
        let bar = Rect::from_min_max(
            pos2(mute.right() + 8.0, y - 3.0),
            pos2(right - label_width - 12.0, y + 3.0),
        );
        let level = volume.level.clamp(0.0, 1.0);
        let (response, shown) = slider(ui, bar, level, &labels.volume, true);
        on_control |= response.hovered();
        let fill = if volume.muted { VOLUME_MUTED } else { ACCENT };
        paint_bar(&painter, bar, shown, fill, &response);
        let shown_volume = Volume {
            level: shown,
            ..volume
        };
        if (response.is_pointer_button_down_on() || response.clicked())
            && shown_volume.percent() != volume.percent()
        {
            command = Some(Command::SetVolume(shown));
        }
        let label = if volume.muted {
            labels.muted.clone()
        } else {
            format!("{}%", shown_volume.percent())
        };
        painter.text(
            pos2(right, y),
            Align2::RIGHT_CENTER,
            label,
            font,
            SECONDARY_TEXT,
        );
    }

    if let Some(grab) = grab {
        // A drag that starts on a control (off by a few pixels from a
        // click) does not move the flyout.
        if grab.drag_started() && !on_control {
            command = Some(Command::StartMove);
        }
        // The "move" cursor on the background, and while dragging.
        if (grab.hovered() && !on_control) || grab.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        }
    }

    command
}

/// A button with an icon on a faint rounded background, brighter on hover
/// and while pressed. `color` is the icon's color, or `None` for a button
/// that cannot be used now (drawn grayed out). Returns whether it was
/// clicked, and sets `hovered` when the pointer is on it.
fn button(
    ui: &mut Ui,
    rect: Rect,
    icon: Icon,
    name: &str,
    color: Option<Color32>,
    hovered: &mut bool,
) -> bool {
    let sense = if color.is_some() {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.allocate_rect(rect, sense);
    *hovered |= response.hovered();
    // Screen readers (and tests) see the translated name.
    response.widget_info(|| WidgetInfo::labeled(Role::Button, color.is_some(), name));
    // Own colors rather than egui's theme, which follows the OS (light or
    // dark) while the panel is always dark.
    let fill = if color.is_none() {
        BUTTON_IDLE
    } else if response.is_pointer_button_down_on() {
        BUTTON_PRESSED
    } else if response.hovered() {
        BUTTON_HOVERED
    } else {
        BUTTON_IDLE
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), fill);
    let r = 7.0_f32.min(rect.height() * 0.28);
    paint_icon(painter, icon, rect.center(), r, color.unwrap_or(DISABLED));
    // No tooltip: it would not fit in the small window.
    response.clicked()
}

/// A bar the user can click or drag along (`enabled`), at `bar`, showing
/// `value` (0.0 to 1.0). Returns its response and the value to show: under
/// the pointer while the bar is held, else `value`.
fn slider(ui: &mut Ui, bar: Rect, value: f32, name: &str, enabled: bool) -> (Response, f32) {
    // Easier to hit than the thin bar it shows.
    let hit = bar.expand2(vec2(6.0, 8.0));
    let sense = if enabled {
        Sense::click_and_drag()
    } else {
        Sense::hover()
    };
    let response = ui.allocate_rect(hit, sense);
    response.widget_info(|| WidgetInfo::slider(enabled, f64::from(value), name));
    let held =
        response.is_pointer_button_down_on() || response.clicked() || response.drag_stopped();
    let under_pointer = response
        .interact_pointer_pos()
        .map(|pointer| ((pointer.x - bar.left()) / bar.width()).clamp(0.0, 1.0));
    let shown = match under_pointer {
        Some(pointer) if enabled && held => pointer,
        _ => value.clamp(0.0, 1.0),
    };
    (response, shown)
}

/// Paints a bar filled up to `value`, with a knob while the pointer is on it.
fn paint_bar(painter: &egui::Painter, bar: Rect, value: f32, fill: Color32, response: &Response) {
    let radius = CornerRadius::same((bar.height() / 2.0).round() as u8);
    painter.rect_filled(bar, radius, BAR_TRACK);
    let filled = Rect::from_min_size(bar.min, vec2(bar.width() * value, bar.height()));
    painter.rect_filled(filled, radius, fill);
    if response.hovered() || response.is_pointer_button_down_on() {
        painter.circle_filled(pos2(filled.right(), bar.center().y), 6.0, Color32::WHITE);
    }
}

/// A time as the players show it: "4:05", or "1:02:03" past an hour.
pub fn format_time(time: Duration) -> String {
    let seconds = time.as_secs();
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Draws the cover art into `rect`, cropped to a square around its center
/// (video thumbnails are wide) and with rounded corners. Returns `false`
/// for pixels that do not match the stated size, which are not drawn.
///
/// The texture is uploaded once per cover and kept in egui's memory with
/// the cover it was made from.
fn paint_cover(ui: &Ui, art: &Arc<Cover>, rect: Rect) -> bool {
    let (width, height) = (art.width as usize, art.height as usize);
    if width == 0 || height == 0 || art.rgba.len() != width * height * 4 {
        return false;
    }
    let id = egui::Id::unique("flexflyout-cover");
    let cached: Option<(Arc<Cover>, TextureHandle)> = ui.data(|data| data.get_temp(id));
    let texture = match cached {
        // Holding the Arc keeps its address from being reused by a new
        // cover, so comparing pointers is enough.
        Some((cover, texture)) if Arc::ptr_eq(&cover, art) => texture,
        _ => {
            let image = ColorImage::from_rgba_premultiplied([width, height], &art.rgba);
            let texture = ui
                .ctx()
                .load_texture("cover", image, TextureOptions::LINEAR);
            ui.data_mut(|data| data.insert_temp(id, (Arc::clone(art), texture.clone())));
            texture
        }
    };
    let (w, h) = (width as f32, height as f32);
    let uv = if w > h {
        let margin = (1.0 - h / w) / 2.0;
        Rect::from_min_max(pos2(margin, 0.0), pos2(1.0 - margin, 1.0))
    } else {
        let margin = (1.0 - w / h) / 2.0;
        Rect::from_min_max(pos2(0.0, margin), pos2(1.0, 1.0 - margin))
    };
    egui::Image::from_texture(&texture)
        .uv(uv)
        .corner_radius(8)
        .paint_at(ui, rect);
    true
}

/// Adds `widget` at the left edge of `rect`, vertically centered
/// (`Ui::put` would center it horizontally as well).
fn put_left(ui: &mut Ui, rect: Rect, widget: impl egui::Widget) {
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(layout), |ui| {
        ui.add(widget)
    });
}

/// Icons of the buttons.
#[derive(Clone, Copy, Debug)]
enum Icon {
    Previous,
    Play,
    Pause,
    Next,
    Shuffle,
    Repeat,
    /// Repeat the current track.
    RepeatTrack,
    Speaker,
    SpeakerMuted,
    Pin,
    Close,
}

/// Draws `icon` centered on `center`, `r` points from the center to the edge.
fn paint_icon(painter: &egui::Painter, icon: Icon, center: Pos2, r: f32, color: Color32) {
    let (x, y) = (center.x, center.y);
    let line = Stroke::new(r * 0.22, color);
    // A triangle pointing right (or left), `r` wide on each side of `x`.
    let triangle = |x: f32, right: bool| {
        let (tip, base) = if right {
            (x + r, x - r)
        } else {
            (x - r, x + r)
        };
        vec![pos2(base, y - r), pos2(tip, y), pos2(base, y + r)]
    };
    // A small arrowhead with its tip at `tip`, pointing along (dx, dy).
    let arrowhead = |tip: Pos2, dx: f32, dy: f32| {
        let size = r * 0.45;
        let back = pos2(tip.x - dx * size, tip.y - dy * size);
        // Perpendicular to the direction.
        let (px, py) = (-dy * size * 0.8, dx * size * 0.8);
        Shape::convex_polygon(
            vec![
                tip,
                pos2(back.x + px, back.y + py),
                pos2(back.x - px, back.y - py),
            ],
            color,
            Stroke::NONE,
        )
    };
    let bar = |x: f32| Rect::from_center_size(pos2(x, y), vec2(r * 0.4, r * 2.0));
    match icon {
        Icon::Play => {
            let shape = triangle(x + r * 0.15, true);
            painter.add(Shape::convex_polygon(shape, color, Stroke::NONE));
        }
        Icon::Pause => {
            painter.rect_filled(bar(x - r * 0.45), 1.0, color);
            painter.rect_filled(bar(x + r * 0.45), 1.0, color);
        }
        Icon::Previous => {
            painter.rect_filled(bar(x - r * 0.8), 1.0, color);
            painter.add(Shape::convex_polygon(
                triangle(x + r * 0.2, false),
                color,
                Stroke::NONE,
            ));
        }
        Icon::Next => {
            painter.rect_filled(bar(x + r * 0.8), 1.0, color);
            painter.add(Shape::convex_polygon(
                triangle(x - r * 0.2, true),
                color,
                Stroke::NONE,
            ));
        }
        Icon::Shuffle => {
            // Two crossing paths, each ending in an arrowhead on the right.
            let (from_top, to_bottom) = (pos2(x - r, y - r * 0.6), pos2(x + r * 0.7, y + r * 0.6));
            let (from_bottom, to_top) = (pos2(x - r, y + r * 0.6), pos2(x + r * 0.7, y - r * 0.6));
            painter.line_segment([from_top, to_bottom], line);
            painter.line_segment([from_bottom, to_top], line);
            painter.add(arrowhead(pos2(x + r * 1.1, y + r * 0.6), 1.0, 0.0));
            painter.add(arrowhead(pos2(x + r * 1.1, y - r * 0.6), 1.0, 0.0));
        }
        Icon::Repeat | Icon::RepeatTrack => {
            // A loop with an arrowhead on its top edge.
            let rect = Rect::from_center_size(center, vec2(r * 2.0, r * 1.3));
            painter.rect_stroke(
                rect,
                CornerRadius::same((r * 0.5) as u8),
                line,
                StrokeKind::Middle,
            );
            painter.add(arrowhead(pos2(x + r * 0.45, rect.top()), 1.0, 0.0));
            if matches!(icon, Icon::RepeatTrack) {
                painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    "1",
                    FontId::proportional(r * 1.2),
                    color,
                );
            }
        }
        Icon::Speaker | Icon::SpeakerMuted => {
            // The box and the cone.
            let body =
                Rect::from_min_max(pos2(x - r, y - r * 0.35), pos2(x - r * 0.5, y + r * 0.35));
            painter.rect_filled(body, 0.0, color);
            painter.add(Shape::convex_polygon(
                vec![
                    pos2(x - r * 0.5, y - r * 0.35),
                    pos2(x, y - r * 0.85),
                    pos2(x, y + r * 0.85),
                    pos2(x - r * 0.5, y + r * 0.35),
                ],
                color,
                Stroke::NONE,
            ));
            if matches!(icon, Icon::SpeakerMuted) {
                // A cross where the sound waves would be.
                let (a, b) = (x + r * 0.3, x + r);
                painter.line_segment([pos2(a, y - r * 0.35), pos2(b, y + r * 0.35)], line);
                painter.line_segment([pos2(a, y + r * 0.35), pos2(b, y - r * 0.35)], line);
            } else {
                // Two sound waves.
                for radius in [r * 0.5, r * 0.95] {
                    painter.add(Shape::line(arc(pos2(x, y), radius, 0.8), line));
                }
            }
        }
        Icon::Close => {
            let d = r * 0.75;
            painter.line_segment([pos2(x - d, y - d), pos2(x + d, y + d)], line);
            painter.line_segment([pos2(x - d, y + d), pos2(x + d, y - d)], line);
        }
        Icon::Pin => {
            // An upright push pin: the cap, the body, the collar and the
            // needle.
            let part = |top: f32, bottom: f32, half_width: f32| {
                Rect::from_min_max(
                    pos2(x - r * half_width, y + r * top),
                    pos2(x + r * half_width, y + r * bottom),
                )
            };
            painter.rect_filled(part(-1.0, -0.7, 0.5), 1.0, color);
            painter.rect_filled(part(-0.7, 0.05, 0.3), 0.0, color);
            painter.rect_filled(part(0.05, 0.35, 0.75), 1.0, color);
            painter.line_segment(
                [pos2(x, y + r * 0.35), pos2(x, y + r * 1.15)],
                Stroke::new(r * 0.16, color),
            );
        }
    }
}

/// Points of an arc around `center`, opening to the right, `half_angle`
/// radians up and down from the horizontal.
fn arc(center: Pos2, radius: f32, half_angle: f32) -> Vec<Pos2> {
    const STEPS: usize = 8;
    (0..=STEPS)
        .map(|i| {
            let angle = -half_angle + 2.0 * half_angle * i as f32 / STEPS as f32;
            pos2(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            )
        })
        .collect()
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
    use flexflyout_core::Timeline;

    fn labels() -> Labels {
        Labels {
            nothing_playing: "Nothing playing".into(),
            muted: "Muted".into(),
            previous: "Previous".into(),
            play_pause: "Play / pause".into(),
            next: "Next".into(),
            position: "Position".into(),
            shuffle: "Shuffle".into(),
            repeat: "Repeat".into(),
            volume: "Volume".into(),
            mute: "Mute".into(),
            pin: "Pin".into(),
            close: "Close".into(),
        }
    }

    /// A playing track one minute long, 15 seconds in, with shuffle off and
    /// repeat off.
    fn track(now: Instant) -> NowPlaying {
        NowPlaying {
            title: "Title".into(),
            artist: "Artist".into(),
            status: PlaybackStatus::Playing,
            timeline: Some(Timeline {
                position: Duration::from_secs(15),
                updated: now,
                length: Duration::from_secs(60),
                seekable: true,
            }),
            repeat: Some(RepeatMode::Off),
            shuffle: Some(false),
            ..NowPlaying::default()
        }
    }

    /// Draws the panel in a test harness and collects the commands.
    fn harness<'a>(
        track: &'a NowPlaying,
        labels: &'a Labels,
        now: Instant,
    ) -> Harness<'a, Vec<Command>> {
        harness_pinned(track, labels, now, false)
    }

    /// [`harness`], pinned or not.
    fn harness_pinned<'a>(
        track: &'a NowPlaying,
        labels: &'a Labels,
        now: Instant,
        pinned: bool,
    ) -> Harness<'a, Vec<Command>> {
        Harness::builder().with_size(BASE_SIZE).build_ui_state(
            move |ui, commands: &mut Vec<Command>| {
                let view = FlyoutView {
                    now_playing: Some(track),
                    volume: Some(Volume {
                        level: 0.42,
                        muted: false,
                    }),
                    pinned,
                    now,
                    background_opacity: 0.9,
                    labels,
                };
                commands.extend(draw(ui, &view));
            },
            Vec::new(),
        )
    }

    #[test]
    fn buttons_send_their_commands() {
        let now = Instant::now();
        let track = track(now);
        let labels = labels();
        let mut harness = harness(&track, &labels, now);
        for (name, expected) in [
            ("Close", Command::Close),
            ("Previous", Command::Previous),
            ("Play / pause", Command::TogglePlayPause),
            ("Next", Command::Next),
            ("Shuffle", Command::SetShuffle(true)),
            ("Repeat", Command::SetRepeat(RepeatMode::List)),
            ("Mute", Command::ToggleMute),
            ("Pin", Command::Pin(true)),
        ] {
            harness.get_by_label(name).click();
            harness.run();
            assert_eq!(harness.state().last(), Some(&expected), "button {name}");
        }
        assert_eq!(harness.state().len(), 8);
    }

    #[test]
    fn only_a_pinned_flyout_can_be_dragged_by_its_background() {
        let now = Instant::now();
        let track = track(now);
        let labels = labels();
        for pinned in [false, true] {
            let mut harness = harness_pinned(&track, &labels, now, pinned);
            // Right of the playback buttons: nothing but the background.
            let (from, to) = (pos2(370.0, 96.0), pos2(300.0, 130.0));
            harness.hover_at(from);
            harness.drag_at(from);
            harness.run();
            harness.hover_at(to);
            harness.run();
            harness.drop_at(to);
            harness.run();
            let moved = harness.state().contains(&Command::StartMove);
            assert_eq!(moved, pinned, "pinned: {pinned}");
        }
    }

    #[test]
    fn without_a_player_the_playback_buttons_do_nothing() {
        let labels = labels();
        let mut harness = Harness::builder().with_size(BASE_SIZE).build_ui_state(
            |ui, commands: &mut Vec<Command>| {
                let view = FlyoutView {
                    now_playing: None,
                    volume: None,
                    pinned: false,
                    now: Instant::now(),
                    background_opacity: 0.9,
                    labels: &labels,
                };
                commands.extend(draw(ui, &view));
            },
            Vec::new(),
        );
        harness.get_by_label("Nothing playing");
        for name in ["Previous", "Play / pause", "Next"] {
            harness.get_by_label(name).click();
            harness.run();
        }
        assert_eq!(harness.state(), &[]);
    }

    #[test]
    fn controls_the_player_lacks_do_nothing() {
        let now = Instant::now();
        let track = NowPlaying {
            repeat: None,
            shuffle: None,
            ..track(now)
        };
        let labels = labels();
        let mut harness = harness(&track, &labels, now);
        for name in ["Shuffle", "Repeat"] {
            harness.get_by_label(name).click();
            harness.run();
        }
        assert_eq!(harness.state(), &[]);
    }

    #[test]
    fn clicking_the_bars_seeks_and_sets_the_volume() {
        let now = Instant::now();
        let track = track(now);
        let labels = labels();
        let mut harness = harness(&track, &labels, now);
        // Click three quarters along each bar.
        for name in ["Position", "Volume"] {
            let rect = harness.get_by_label(name).rect();
            let pos = pos2(
                rect.left() + 6.0 + (rect.width() - 12.0) * 0.75,
                rect.center().y,
            );
            harness.hover_at(pos);
            harness.drag_at(pos);
            harness.run();
            harness.drop_at(pos);
            harness.run();
        }
        let commands = harness.state();
        let seek = commands.iter().find_map(|c| match c {
            Command::Seek(position) => Some(position.as_secs_f32()),
            _ => None,
        });
        let volume = commands.iter().find_map(|c| match c {
            Command::SetVolume(level) => Some(*level),
            _ => None,
        });
        let seek = seek.expect("a seek");
        assert!((seek - 45.0).abs() < 1.0, "seek to {seek}");
        let volume = volume.expect("a volume change");
        assert!((volume - 0.75).abs() < 0.02, "volume {volume}");
    }

    #[test]
    fn shows_the_track_and_the_times() {
        let now = Instant::now();
        let track = NowPlaying {
            status: PlaybackStatus::Paused,
            ..track(now)
        };
        let labels = labels();
        let harness = harness(&track, &labels, now);
        harness.get_by_label("Title");
        harness.get_by_label("Artist");
        harness.get_by_label("Position");
    }

    #[test]
    fn times_are_formatted_like_players_do() {
        assert_eq!(format_time(Duration::from_secs(0)), "0:00");
        assert_eq!(format_time(Duration::from_secs(245)), "4:05");
        assert_eq!(format_time(Duration::from_secs(3723)), "1:02:03");
    }

    #[test]
    fn cover_art_is_drawn_or_replaced_when_broken() {
        let now = Instant::now();
        let labels = labels();
        // A wide 4 × 2 image (cropped to a square) and one whose pixels do
        // not match its size: neither may panic.
        for rgba_len in [4 * 2 * 4, 5] {
            let track = NowPlaying {
                title: "Title".into(),
                cover: Some(Arc::new(Cover {
                    width: 4,
                    height: 2,
                    rgba: vec![255; rgba_len],
                })),
                ..NowPlaying::default()
            };
            let mut harness = harness(&track, &labels, now);
            harness.run();
            let textures = harness.ctx.tex_manager().read().num_allocated();
            // The font atlas, plus the cover when it is valid.
            let expected = if rgba_len == 32 { 2 } else { 1 };
            assert_eq!(textures, expected, "pixels: {rgba_len}");
        }
    }
}
