//! Drawing of the FlexFlyout panel.
//!
//! The panel is drawn from a snapshot of what to show ([`FlyoutView`]) and
//! reports what the user did with its controls as a [`Command`]. It knows
//! nothing about windows or the OS: the host sizes the window to
//! [`panel_size`] times its scale, sets egui's zoom factor to that scale and
//! calls [`draw`].

pub mod fonts;

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{
    Align2, Color32, ColorImage, CornerRadius, FontId, Pos2, Rect, Response, RichText, Role, Sense,
    Shape, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, WidgetInfo, pos2, vec2,
};
use flexflyout_core::{
    Appearance, Command, Cover, NowPlaying, PlaybackStatus, RepeatMode, Theme, Volume,
};

/// Size of the panel at scale 1.0, in egui points, with the parts the
/// settings show: it gets smaller without the cover art, the position or
/// the volume.
pub fn panel_size(appearance: &Appearance) -> egui::Vec2 {
    let height = content_height(appearance);
    let cover = if appearance.show_cover_art {
        height + GAP
    } else {
        0.0
    };
    vec2(
        2.0 * MARGIN + cover + column_width(appearance),
        2.0 * MARGIN + height,
    )
}

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
    /// The settings button (opens the settings window).
    pub settings: String,
}

/// Everything the panel shows in one frame.
#[derive(Clone, Copy, Debug)]
pub struct FlyoutView<'a> {
    /// The current track, or `None` when nothing is playing.
    pub now_playing: Option<&'a NowPlaying>,
    /// The volume, or `None` while it is not known yet.
    pub volume: Option<Volume>,
    /// Whether the flyout is pinned (stays until unpinned).
    pub pinned: bool,
    /// The time to show the track's position at.
    pub now: Instant,
    /// Opacity of the panel's background, from the settings (0.0 to 1.0).
    /// The text and buttons stay opaque so that they remain readable.
    pub background_opacity: f32,
    /// The colors, the corners and the parts to show, from the settings.
    pub appearance: &'a Appearance,
    /// Translated texts.
    pub labels: &'a Labels,
}

/// The colors of the panel, for its theme and accent color.
///
/// The panel uses its own colors rather than egui's theme, which follows
/// the OS (light or dark) rather than the settings.
#[derive(Clone, Copy, Debug)]
struct Palette {
    /// Background (before the opacity is applied).
    background: Color32,
    /// The cover art's placeholder.
    cover_placeholder: Color32,
    /// The title, the icons of the playback buttons and the bars' knob.
    text: Color32,
    /// The secondary text (artist, times, percentage) and the small icons.
    secondary: Color32,
    /// The icons of controls the player does not offer.
    disabled: Color32,
    /// The empty part of the bars.
    bar_track: Color32,
    /// The volume bar while muted.
    muted: Color32,
    /// The filled part of the bars, and the buttons that are on.
    accent: Color32,
    /// Background of the buttons: at rest, hovered and pressed
    /// (translucent, premultiplied).
    button_idle: Color32,
    button_hovered: Color32,
    button_pressed: Color32,
}

impl Palette {
    fn new(appearance: &Appearance) -> Self {
        let (r, g, b) = appearance.accent_rgb();
        let accent = Color32::from_rgb(r, g, b);
        match appearance.theme {
            Theme::Dark => Self {
                background: Color32::from_rgb(28, 28, 34),
                cover_placeholder: Color32::from_rgb(60, 60, 72),
                text: Color32::WHITE,
                secondary: Color32::from_gray(190),
                disabled: Color32::from_gray(95),
                bar_track: Color32::from_gray(70),
                muted: Color32::from_gray(120),
                accent,
                // Translucent white.
                button_idle: Color32::from_rgba_premultiplied(20, 20, 20, 20),
                button_hovered: Color32::from_rgba_premultiplied(40, 40, 40, 40),
                button_pressed: Color32::from_rgba_premultiplied(64, 64, 64, 64),
            },
            Theme::Light => Self {
                background: Color32::from_rgb(243, 243, 246),
                cover_placeholder: Color32::from_rgb(212, 212, 220),
                text: Color32::from_rgb(20, 20, 24),
                secondary: Color32::from_gray(85),
                disabled: Color32::from_gray(178),
                bar_track: Color32::from_gray(208),
                muted: Color32::from_gray(150),
                accent,
                // Translucent black.
                button_idle: Color32::from_rgba_premultiplied(0, 0, 0, 14),
                button_hovered: Color32::from_rgba_premultiplied(0, 0, 0, 28),
                button_pressed: Color32::from_rgba_premultiplied(0, 0, 0, 46),
            },
        }
    }
}

/// How the buttons look.
#[derive(Clone, Copy, Debug)]
struct ButtonStyle {
    colors: Palette,
    /// Radius of their corners.
    corner: u8,
}

/// Margin around the content, in points.
const MARGIN: f32 = 16.0;
/// Gap between the cover art and the text, in points.
const GAP: f32 = 14.0;
/// Width of the column right of the cover art (text and controls).
const COLUMN_WIDTH: f32 = 222.0;
/// Room for the settings button after the five playback buttons, when
/// there is no volume row to put it in.
const SETTINGS_ROOM: f32 = 36.0;
/// Height of the title and the artist.
const HEADER_HEIGHT: f32 = 44.0;
/// Room between the header and the playback buttons with the position
/// (in the middle) and without it.
const POSITION_ROW: f32 = 22.0;
const NO_POSITION_ROW: f32 = 6.0;
/// Room between the playback buttons and the volume row.
const VOLUME_GAP: f32 = 14.0;
/// Size of the playback buttons, and the gap between them.
const BUTTON_SIZE: egui::Vec2 = vec2(36.0, 28.0);
const BUTTON_GAP: f32 = 6.0;
/// Size of the small buttons (close, pin, settings, mute).
const SMALL_BUTTON_SIZE: egui::Vec2 = vec2(28.0, 24.0);
/// Width kept for a time or the percentage next to a bar.
const BAR_LABEL_WIDTH: f32 = 40.0;
/// Volume change for one step of the mouse wheel over the volume bar (a
/// notch of most wheels; Windows' volume keys change it by as much).
const WHEEL_STEP: f32 = 0.02;
/// Scrolling, in points, that makes one step when a device scrolls by
/// points (touchpads): what egui makes of one notch of a wheel.
const POINTS_PER_STEP: f32 = 40.0;

/// Top of the playback buttons, from the top of the content.
fn buttons_top(appearance: &Appearance) -> f32 {
    HEADER_HEIGHT
        + if appearance.show_position {
            POSITION_ROW
        } else {
            NO_POSITION_ROW
        }
}

/// Width of the column right of the cover art.
fn column_width(appearance: &Appearance) -> f32 {
    if !appearance.show_volume && appearance.show_shuffle_repeat {
        COLUMN_WIDTH + SETTINGS_ROOM
    } else {
        COLUMN_WIDTH
    }
}

/// Height of the content (the panel without its margins), which is also
/// the side of the cover art.
fn content_height(appearance: &Appearance) -> f32 {
    let volume = if appearance.show_volume {
        VOLUME_GAP + SMALL_BUTTON_SIZE.y
    } else {
        0.0
    };
    buttons_top(appearance) + BUTTON_SIZE.y + volume
}

/// Draws the panel over the whole of `ui`'s area and returns what the user
/// did with its controls in this frame, if anything.
///
/// The caller fades the whole panel with [`Ui::multiply_opacity`].
pub fn draw(ui: &mut Ui, view: &FlyoutView<'_>) -> Option<Command> {
    let rect = ui.max_rect();
    let painter = ui.painter().clone();
    let labels = view.labels;
    let appearance = view.appearance;
    let colors = Palette::new(appearance);
    let mut command = None;

    // Rounded background. Its alpha comes from the settings only.
    let corner = f32::from(appearance.corner_radius);
    let alpha = (view.background_opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let background = Color32::from_rgba_unmultiplied(
        colors.background.r(),
        colors.background.g(),
        colors.background.b(),
        alpha,
    );
    painter.rect_filled(
        rect,
        CornerRadius::same(appearance.corner_radius),
        background,
    );
    // The cover art and the buttons are rounded less than the panel, and
    // square with it.
    let cover_corner = (corner * 0.6).min(8.0).round() as u8;
    let style = ButtonStyle {
        colors,
        corner: (corner * 0.45).min(6.0).round() as u8,
    };

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
    // The rows of the content, from the top.
    let top = rect.top() + MARGIN;
    let side = content_height(appearance);
    let buttons_top = top + buttons_top(appearance);
    let volume_y = buttons_top + BUTTON_SIZE.y + VOLUME_GAP + SMALL_BUTTON_SIZE.y / 2.0;

    // Square cover art on the left, as tall as the content, or a
    // placeholder when the player gives none.
    let mut left = rect.left() + MARGIN;
    if appearance.show_cover_art {
        let cover = Rect::from_min_size(pos2(left, top), vec2(side, side));
        let art = view.now_playing.and_then(|track| track.cover.as_ref());
        if !art.is_some_and(|art| paint_cover(ui, art, cover, cover_corner)) {
            painter.rect_filled(
                cover,
                CornerRadius::same(cover_corner),
                colors.cover_placeholder,
            );
            paint_note(&painter, cover.center(), side * 0.4, colors.secondary);
        }
        // Text and controls to the right of the cover art.
        left = cover.right() + GAP;
    }
    let right = rect.right() - MARGIN;

    // The close and pin buttons in the top-right corner.
    let close = Rect::from_min_size(
        pos2(right - SMALL_BUTTON_SIZE.x, top - 4.0),
        SMALL_BUTTON_SIZE,
    );
    if button(
        ui,
        close,
        Icon::Close,
        &labels.close,
        Some(colors.secondary),
        style,
        &mut on_control,
    ) {
        command = Some(Command::Close);
    }
    let pin = close.translate(vec2(-(SMALL_BUTTON_SIZE.x + 4.0), 0.0));
    let pin_color = if view.pinned {
        colors.accent
    } else {
        colors.secondary
    };
    if button(
        ui,
        pin,
        Icon::Pin,
        &labels.pin,
        Some(pin_color),
        style,
        &mut on_control,
    ) {
        command = Some(Command::Pin(!view.pinned));
    }

    // The settings button at the right end of the last row: the volume, or
    // the playback buttons without it.
    let settings_y = if appearance.show_volume {
        volume_y
    } else {
        buttons_top + BUTTON_SIZE.y / 2.0
    };
    let settings = Rect::from_center_size(
        pos2(right - SMALL_BUTTON_SIZE.x / 2.0, settings_y),
        SMALL_BUTTON_SIZE,
    );
    if button(
        ui,
        settings,
        Icon::Settings,
        &labels.settings,
        Some(colors.secondary),
        style,
        &mut on_control,
    ) {
        command = Some(Command::OpenSettings);
    }

    let (title, artist) = match view.now_playing {
        Some(track) => (track.title.as_str(), track.artist.as_str()),
        None => (labels.nothing_playing.as_str(), ""),
    };
    // Long titles are cut with "…" instead of wrapping out of the panel.
    let text_width = pin.left() - 4.0 - left;
    let title_rect = Rect::from_min_size(pos2(left, top - 2.0), vec2(text_width, 24.0));
    put_left(
        ui,
        title_rect,
        // Not selectable: a selectable label takes the drags meant for
        // moving the pinned flyout.
        egui::Label::new(RichText::new(title).size(18.0).color(colors.text))
            .truncate()
            .selectable(false),
    );
    if !artist.is_empty() {
        let artist_rect =
            Rect::from_min_size(pos2(left, title_rect.bottom()), vec2(text_width, 20.0));
        put_left(
            ui,
            artist_rect,
            egui::Label::new(RichText::new(artist).size(14.0).color(colors.secondary))
                .truncate()
                .selectable(false),
        );
    }

    // The position in the track, with the time played on the left and the
    // length on the right. Left out when the player does not tell them.
    let track = view.now_playing;
    if let Some((timeline, position)) = track
        .filter(|_| appearance.show_position)
        .and_then(|track| track.timeline.zip(track.position_at(view.now)))
    {
        let y = top + HEADER_HEIGHT + 10.0;
        let bar = Rect::from_min_max(
            pos2(left + BAR_LABEL_WIDTH, y - 2.0),
            pos2(right - BAR_LABEL_WIDTH, y + 2.0),
        );
        let length = timeline.length.as_secs_f32();
        let value = position.as_secs_f32() / length;
        let (response, shown) = slider(ui, bar, value, &labels.position, timeline.seekable);
        on_control |= response.hovered();
        paint_bar(&painter, bar, shown, colors.accent, &colors, &response);
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
            colors.secondary,
        );
        painter.text(
            pos2(right, y),
            Align2::RIGHT_CENTER,
            format_time(timeline.length),
            font,
            colors.secondary,
        );
    }

    // Playback buttons. The icons are drawn as shapes rather than text:
    // egui's bundled fonts have no media symbols, and shapes stay sharp at
    // every scale. Shuffle and repeat show whether they are on, and are
    // grayed out when the player cannot change them; all of them are
    // grayed out when no player is there.
    let playing = track.is_some_and(|t| t.status == PlaybackStatus::Playing);
    let player = track.is_some().then_some(colors.text);
    let shuffle = track.and_then(|t| t.shuffle);
    let repeat = track.and_then(|t| t.repeat);
    let toggle_color = |on: Option<bool>| match on {
        Some(true) => Some(colors.accent),
        Some(false) => Some(colors.text),
        None => None,
    };
    // Whether each one is a toggle (shuffle, repeat), which the settings
    // may leave out.
    let buttons = [
        (
            true,
            Icon::Shuffle,
            &labels.shuffle,
            toggle_color(shuffle),
            shuffle.map(|on| Command::SetShuffle(!on)),
        ),
        (
            false,
            Icon::Previous,
            &labels.previous,
            player,
            Some(Command::Previous),
        ),
        (
            false,
            if playing { Icon::Pause } else { Icon::Play },
            &labels.play_pause,
            player,
            Some(Command::TogglePlayPause),
        ),
        (false, Icon::Next, &labels.next, player, Some(Command::Next)),
        (
            true,
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
    let shown = buttons
        .into_iter()
        .filter(|(toggle, ..)| appearance.show_shuffle_repeat || !toggle);
    for (i, (_, icon, name, color, cmd)) in shown.enumerate() {
        let r = Rect::from_min_size(
            pos2(left + i as f32 * (BUTTON_SIZE.x + BUTTON_GAP), buttons_top),
            BUTTON_SIZE,
        );
        if button(ui, r, icon, name, color, style, &mut on_control) {
            command = cmd;
        }
    }

    // Mute button and volume bar along the bottom, with the level on the
    // right. The volume follows the pointer while the bar is held.
    if let Some(volume) = view.volume.filter(|_| appearance.show_volume) {
        let y = volume_y;
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
            Some(colors.secondary),
            style,
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
                    .layout_no_wrap(text.to_owned(), font.clone(), colors.secondary)
                    .size()
                    .x
            })
            .fold(BAR_LABEL_WIDTH - 8.0, f32::max);
        let volume_right = settings.left() - 8.0;
        let bar = Rect::from_min_max(
            pos2(mute.right() + 8.0, y - 3.0),
            pos2(volume_right - label_width - 12.0, y + 3.0),
        );
        let level = volume.level.clamp(0.0, 1.0);
        let (response, shown) = slider(ui, bar, level, &labels.volume, true);
        on_control |= response.hovered();
        let fill = if volume.muted {
            colors.muted
        } else {
            colors.accent
        };
        paint_bar(&painter, bar, shown, fill, &colors, &response);
        let shown_volume = Volume {
            level: shown,
            ..volume
        };
        if (response.is_pointer_button_down_on() || response.clicked())
            && shown_volume.percent() != volume.percent()
        {
            command = Some(Command::SetVolume(shown));
        }
        // The wheel over the bar changes the volume step by step, up for
        // louder, to whole percents.
        let steps = wheel_steps(ui, &response);
        if steps != 0 {
            let stepped = (level + steps as f32 * WHEEL_STEP).clamp(0.0, 1.0);
            command = Some(Command::SetVolume((stepped * 100.0).round() / 100.0));
        }
        let label = if volume.muted {
            labels.muted.clone()
        } else {
            format!("{}%", shown_volume.percent())
        };
        painter.text(
            pos2(volume_right, y),
            Align2::RIGHT_CENTER,
            label,
            font,
            colors.secondary,
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
    style: ButtonStyle,
    hovered: &mut bool,
) -> bool {
    let colors = &style.colors;
    let sense = if color.is_some() {
        Sense::click()
    } else {
        Sense::hover()
    };
    let response = ui.allocate_rect(rect, sense);
    *hovered |= response.hovered();
    // Screen readers (and tests) see the translated name.
    response.widget_info(|| WidgetInfo::labeled(Role::Button, color.is_some(), name));
    let fill = if color.is_none() {
        colors.button_idle
    } else if response.is_pointer_button_down_on() {
        colors.button_pressed
    } else if response.hovered() {
        colors.button_hovered
    } else {
        colors.button_idle
    };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(style.corner), fill);
    let r = 7.0_f32.min(rect.height() * 0.28);
    paint_icon(
        painter,
        icon,
        rect.center(),
        r,
        color.unwrap_or(colors.disabled),
    );
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

/// Whole steps the mouse wheel turned over `response` in this frame:
/// positive when turned up (away from the user). Fractions of a step (from
/// high-resolution wheels and touchpads) add up while the pointer stays.
fn wheel_steps(ui: &Ui, response: &Response) -> i32 {
    let id = response.id.with("wheel");
    if !response.contains_pointer() {
        ui.data_mut(|data| data.remove::<f32>(id));
        return 0;
    }
    let turned: f32 = ui.input(|input| {
        input
            .events
            .iter()
            .map(|event| match event {
                egui::Event::MouseWheel { unit, delta, .. } => match unit {
                    egui::MouseWheelUnit::Point => delta.y / POINTS_PER_STEP,
                    egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => delta.y,
                },
                _ => 0.0,
            })
            .sum()
    });
    if turned == 0.0 {
        return 0;
    }
    let total = ui.data(|data| data.get_temp::<f32>(id)).unwrap_or(0.0) + turned;
    let steps = total.trunc();
    ui.data_mut(|data| data.insert_temp(id, total - steps));
    steps as i32
}

/// Paints a bar filled up to `value`, with a knob while the pointer is on it.
fn paint_bar(
    painter: &egui::Painter,
    bar: Rect,
    value: f32,
    fill: Color32,
    colors: &Palette,
    response: &Response,
) {
    let radius = CornerRadius::same((bar.height() / 2.0).round() as u8);
    painter.rect_filled(bar, radius, colors.bar_track);
    let filled = Rect::from_min_size(bar.min, vec2(bar.width() * value, bar.height()));
    painter.rect_filled(filled, radius, fill);
    if response.hovered() || response.is_pointer_button_down_on() {
        painter.circle_filled(pos2(filled.right(), bar.center().y), 6.0, colors.text);
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
/// (video thumbnails are wide) and with corners rounded by `corner`.
/// Returns `false` for pixels that do not match the stated size, which are
/// not drawn.
///
/// The texture is uploaded once per cover and kept in egui's memory with
/// the cover it was made from.
fn paint_cover(ui: &Ui, art: &Arc<Cover>, rect: Rect, corner: u8) -> bool {
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
        .corner_radius(corner)
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
    /// A gear: the settings.
    Settings,
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
        Icon::Settings => {
            // Eight teeth around a ring, with a hole in the middle.
            let teeth = r * 0.95;
            for i in 0..8 {
                let angle = i as f32 * std::f32::consts::FRAC_PI_4;
                let (s, c) = angle.sin_cos();
                painter.line_segment(
                    [
                        pos2(x + c * r * 0.5, y + s * r * 0.5),
                        pos2(x + c * teeth, y + s * teeth),
                    ],
                    Stroke::new(r * 0.32, color),
                );
            }
            painter.circle_stroke(center, r * 0.55, Stroke::new(r * 0.3, color));
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
            settings: "Settings".into(),
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

    /// The default appearance, which shows everything.
    static DEFAULT: std::sync::LazyLock<Appearance> = std::sync::LazyLock::new(Appearance::default);

    /// Draws the panel in a test harness and collects the commands.
    fn harness<'a>(
        track: &'a NowPlaying,
        labels: &'a Labels,
        now: Instant,
    ) -> Harness<'a, Vec<Command>> {
        harness_with(track, labels, now, false, &DEFAULT)
    }

    /// [`harness`], pinned or not, with the given appearance.
    fn harness_with<'a>(
        track: &'a NowPlaying,
        labels: &'a Labels,
        now: Instant,
        pinned: bool,
        appearance: &'a Appearance,
    ) -> Harness<'a, Vec<Command>> {
        Harness::builder()
            .with_size(panel_size(appearance))
            .build_ui_state(
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
                        appearance,
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
            ("Settings", Command::OpenSettings),
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
        assert_eq!(harness.state().len(), 9);
    }

    #[test]
    fn only_a_pinned_flyout_can_be_dragged_by_its_background() {
        let now = Instant::now();
        let track = track(now);
        let labels = labels();
        for pinned in [false, true] {
            let mut harness = harness_with(&track, &labels, now, pinned, &DEFAULT);
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
        let mut harness = Harness::builder()
            .with_size(panel_size(&DEFAULT))
            .build_ui_state(
                |ui, commands: &mut Vec<Command>| {
                    let view = FlyoutView {
                        now_playing: None,
                        volume: None,
                        pinned: false,
                        now: Instant::now(),
                        background_opacity: 0.9,
                        appearance: &DEFAULT,
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

    /// The mouse wheel turned by `notches`, up when positive.
    fn wheel(notches: f32) -> egui::Event {
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, notches),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn the_wheel_over_the_volume_bar_steps_the_volume() {
        let now = Instant::now();
        let track = track(now);
        let labels = labels();
        // The volume stays at 42% here: every step starts from there.
        let mut harness = harness(&track, &labels, now);
        let bar = harness.get_by_label("Volume").rect().center();
        harness.hover_at(bar);
        harness.run();
        harness.event(wheel(1.0));
        harness.run();
        assert_eq!(harness.state().last(), Some(&Command::SetVolume(0.44)));
        harness.event(wheel(-2.0));
        harness.run();
        assert_eq!(harness.state().last(), Some(&Command::SetVolume(0.38)));
        // Half notches (a high-resolution wheel) add up to a step.
        let count = harness.state().len();
        harness.event(wheel(0.5));
        harness.run();
        assert_eq!(harness.state().len(), count);
        harness.event(wheel(0.5));
        harness.run();
        assert_eq!(harness.state().last(), Some(&Command::SetVolume(0.44)));
        // Elsewhere on the panel the wheel does nothing.
        let count = harness.state().len();
        harness.hover_at(harness.get_by_label("Title").rect().center());
        harness.run();
        harness.event(wheel(1.0));
        harness.run();
        assert_eq!(harness.state().len(), count);
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
    fn the_panel_gets_smaller_without_its_parts() {
        let all = Appearance::default();
        assert_eq!(panel_size(&all), vec2(400.0, 164.0));
        let without = |change: fn(&mut Appearance)| {
            let mut appearance = all.clone();
            change(&mut appearance);
            panel_size(&appearance)
        };
        // The cover art is as tall as the content, so it narrows the panel
        // when a row goes.
        assert_eq!(without(|a| a.show_cover_art = false), vec2(254.0, 164.0));
        assert_eq!(without(|a| a.show_position = false), vec2(384.0, 148.0));
        // Without the volume row, the settings button goes after the
        // playback buttons, which takes room unless shuffle and repeat go.
        assert_eq!(without(|a| a.show_volume = false), vec2(398.0, 126.0));
        let compact = |a: &mut Appearance| {
            a.show_volume = false;
            a.show_shuffle_repeat = false;
        };
        assert_eq!(without(compact), vec2(362.0, 126.0));
        assert_eq!(without(|a| a.show_shuffle_repeat = false), panel_size(&all));
    }

    #[test]
    fn hidden_parts_are_left_out_and_settings_stays_reachable() {
        let now = Instant::now();
        let track = track(now);
        let labels = labels();
        // Every combination of the four parts, in both themes and with
        // square corners too: nothing may panic, the parts that are off are
        // missing, and the settings button can still be clicked.
        for bits in 0..32_u8 {
            let appearance = Appearance {
                theme: if bits & 16 == 0 {
                    Theme::Dark
                } else {
                    Theme::Light
                },
                corner_radius: if bits % 3 == 0 { 0 } else { 32 },
                show_cover_art: bits & 1 != 0,
                show_position: bits & 2 != 0,
                show_shuffle_repeat: bits & 4 != 0,
                show_volume: bits & 8 != 0,
                ..Appearance::default()
            };
            let mut harness = harness_with(&track, &labels, now, false, &appearance);
            for (name, shown) in [
                ("Position", appearance.show_position),
                ("Shuffle", appearance.show_shuffle_repeat),
                ("Repeat", appearance.show_shuffle_repeat),
                ("Volume", appearance.show_volume),
                ("Mute", appearance.show_volume),
            ] {
                let found = harness.query_by_label(name).is_some();
                assert_eq!(found, shown, "{name} with {appearance:?}");
            }
            harness.get_by_label("Settings").click();
            harness.run();
            assert_eq!(
                harness.state().last(),
                Some(&Command::OpenSettings),
                "{appearance:?}"
            );
            // The settings button does not cover another control.
            let settings = harness.get_by_label("Settings").rect();
            for name in ["Close", "Pin", "Next", "Repeat", "Volume"] {
                if let Some(other) = harness.query_by_label(name) {
                    assert!(
                        !settings.intersects(other.rect()),
                        "Settings over {name} with {appearance:?}"
                    );
                }
            }
            // Everything that is drawn lies inside the panel.
            let size = panel_size(&appearance);
            for name in ["Settings", "Close", "Pin", "Next"] {
                let rect = harness.get_by_label(name).rect();
                assert!(
                    rect.min.x >= 0.0 && rect.max.x <= size.x && rect.max.y <= size.y,
                    "{name} at {rect:?} with {appearance:?}"
                );
            }
        }
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
