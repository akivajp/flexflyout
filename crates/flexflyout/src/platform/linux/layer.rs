//! The flyout's window on Wayland: a layer-shell surface (above the other
//! windows, never taking the keyboard focus), drawn by egui through wgpu.
//!
//! winit, and so eframe, cannot make layer-shell surfaces, so this module
//! talks to the compositor with smithay-client-toolkit and feeds egui its
//! input itself. KDE Plasma (KWin), sway, Hyprland and other wlroots
//! compositors offer layer-shell; GNOME does not.
//!
//! The surface exists only while the flyout is shown: showing creates it,
//! hiding destroys it (and its wgpu surface). The wgpu device and egui's
//! renderer are kept.

use std::ptr::NonNull;
use std::time::Instant;

use anyhow::Context as _;
use flexflyout_core::config::{Anchor, Display};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, RawDisplayHandle, RawWindowHandle,
    WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{
    AxisScroll, PointerEvent, PointerEventKind, PointerHandler,
};
use smithay_client_toolkit::seat::relative_pointer::{
    RelativeMotionEvent, RelativePointerHandler, RelativePointerState,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    self, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use wayland_client::globals::GlobalList;
use wayland_client::protocol::{wl_output, wl_pointer, wl_seat, wl_surface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_plasma::output_order::v1::client::kde_output_order_v1::{
    self, KdeOutputOrderV1,
};

/// Mouse buttons in Linux input event codes.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// Where and how large the flyout is, in the compositor's logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Size of the surface.
    pub width: u32,
    pub height: u32,
    /// The edges the surface is anchored to.
    pub anchor: wlr_layer::Anchor,
    /// Distances from the edges: top, right, bottom, left.
    pub margins: (i32, i32, i32, i32),
    /// The output (monitor) by its index in the compositor's list, or
    /// `None` to let the compositor choose (KWin: the active one).
    pub output: Option<usize>,
    /// Whether the flyout goes above full-screen windows (the overlay
    /// layer), or below them (the top layer, which is still above the
    /// other windows).
    pub above_fullscreen: bool,
}

/// Where the flyout goes for the placement settings, `size` (logical
/// pixels), `slide` (how far out of place a sliding flyout is, as a
/// distance in logical pixels towards its edge; see `layout::slide`) and
/// the size of the output it shows on (logical pixels), if known.
///
/// Next to an edge, the surface is anchored to it and the offset is the
/// margin; the compositor keeps it clear of panels (their exclusive zones).
/// Along a centered axis it is anchored to the start edge with the margin
/// that centers it on the output, plus the offset. (Anchoring to both
/// edges leaves the size to the compositor's reading of the protocol, and
/// KWin sized it differently.) Without the output's size, the axis is left
/// unanchored and the compositor centers it, without the offset.
pub fn placement(
    display: &Display,
    size: (u32, u32),
    slide: (i32, i32),
    area: Option<(i32, i32)>,
) -> Placement {
    use wlr_layer::Anchor as A;
    let (column, row) = edges(display.anchor);
    // One axis: the anchor flags and the margins at its start and end.
    let axis =
        |edge: Edge, offset: i32, slide: i32, length: u32, area: Option<i32>, start: A, end: A| {
            match (edge, area) {
                (Edge::Start, _) => (start, offset + slide, 0),
                (Edge::End, _) => (end, 0, offset - slide),
                (Edge::Center, Some(area)) => {
                    (start, (area - length as i32) / 2 + offset + slide, 0)
                }
                (Edge::Center, None) => (A::empty(), 0, 0),
            }
        };
    let (horizontal, left, right) = axis(
        column,
        display.offset_x,
        slide.0,
        size.0,
        area.map(|a| a.0),
        A::LEFT,
        A::RIGHT,
    );
    let (vertical, top, bottom) = axis(
        row,
        display.offset_y,
        slide.1,
        size.1,
        area.map(|a| a.1),
        A::TOP,
        A::BOTTOM,
    );
    Placement {
        width: size.0,
        height: size.1,
        anchor: horizontal | vertical,
        margins: (top, right, bottom, left),
        output: None,
        above_fullscreen: true,
    }
}

/// Where `placement` puts the flyout on an output of `area` (logical
/// size), measured from the output's top-left corner. Panels are not
/// known here, so this is where it would be without them.
pub fn rect_on(placement: &Placement, area: (i32, i32)) -> flexflyout_core::layout::PixelRect {
    use wlr_layer::Anchor as A;
    let (top, right, bottom, left) = placement.margins;
    let (width, height) = (placement.width as i32, placement.height as i32);
    let along =
        |start: bool, end: bool, start_margin: i32, end_margin: i32, length: i32, area: i32| {
            if start {
                start_margin
            } else if end {
                area - end_margin - length
            } else {
                (area - length) / 2
            }
        };
    let anchor = placement.anchor;
    let x = along(
        anchor.contains(A::LEFT),
        anchor.contains(A::RIGHT),
        left,
        right,
        width,
        area.0,
    );
    let y = along(
        anchor.contains(A::TOP),
        anchor.contains(A::BOTTOM),
        top,
        bottom,
        height,
        area.1,
    );
    flexflyout_core::layout::PixelRect::from_origin_size(x, y, width, height)
}

/// `placement` moved by `(dx, dy)` logical pixels (where the user dragged
/// it): the margin on an anchored edge grows or shrinks, and a centered
/// axis shifts as [`placement`] shifts it.
pub fn moved(placement: Placement, (dx, dy): (i32, i32)) -> Placement {
    use wlr_layer::Anchor as A;
    let (mut top, mut right, mut bottom, mut left) = placement.margins;
    // One axis: its margins at the start and the end, moved by `d`.
    let axis = |start: &mut i32, end: &mut i32, d: i32, start_edge: A, end_edge: A| {
        let anchor = placement.anchor;
        match (anchor.contains(start_edge), anchor.contains(end_edge)) {
            (true, true) => {
                let shift = (*start - *end) / 2 + d;
                *start = (2 * shift).max(0);
                *end = (-2 * shift).max(0);
            }
            (true, false) => *start += d,
            (false, true) => *end -= d,
            (false, false) => {}
        }
    };
    axis(&mut left, &mut right, dx, A::LEFT, A::RIGHT);
    axis(&mut top, &mut bottom, dy, A::TOP, A::BOTTOM);
    Placement {
        margins: (top, right, bottom, left),
        ..placement
    }
}

/// The placement settings with the offsets moved by `(dx, dy)` logical
/// pixels: what puts the flyout where the user dragged it from now on.
pub fn display_moved(display: &Display, (dx, dy): (i32, i32)) -> Display {
    let (column, row) = edges(display.anchor);
    let along = |edge: Edge, offset: i32, d: i32| match edge {
        Edge::End => offset - d,
        Edge::Start | Edge::Center => offset + d,
    };
    Display {
        offset_x: along(column, display.offset_x, dx),
        offset_y: along(row, display.offset_y, dy),
        ..display.clone()
    }
}

/// Where along one axis an anchor lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Start,
    Center,
    End,
}

/// The column and the row of an anchor.
fn edges(anchor: Anchor) -> (Edge, Edge) {
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

/// The compositor connection, as wgpu wants it for OpenGL.
#[derive(Debug)]
struct DisplayPointer(NonNull<std::ffi::c_void>);

// SAFETY: the pointer is only handed to the graphics driver, which may use
// it from any thread; the connection outlives the wgpu instance.
unsafe impl Send for DisplayPointer {}
// SAFETY: as above.
unsafe impl Sync for DisplayPointer {}

impl HasDisplayHandle for DisplayPointer {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let raw = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(self.0));
        // SAFETY: the display lives as long as the connection.
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}

/// The GPU objects, made when the flyout is first shown.
struct Gpu {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: egui_wgpu::Renderer,
    format: wgpu::TextureFormat,
}

/// The surface while the flyout is shown.
struct Shown {
    /// Dropped before `layer`: the wgpu surface draws into it.
    target: Option<wgpu::Surface<'static>>,
    layer: LayerSurface,
    /// Where the host wants it.
    placement: Placement,
    /// Where it is: the host's placement, moved by where it was dragged.
    applied: Placement,
    /// Whether the compositor has sized it (only then can it be drawn).
    configured: bool,
    /// The size the compositor gave it (logical pixels). It can differ from
    /// the one asked for, and a new size asked for applies only once the
    /// compositor answers: the frames are drawn at this one.
    size: (u32, u32),
    /// The size the wgpu surface is configured for (buffer pixels).
    target_size: (u32, u32),
    /// The output's integer scale (buffer pixels per logical pixel).
    scale: i32,
}

/// The outputs in the order the desktop places its things on them, the
/// primary one first. KDE Plasma tells it (kde_output_order_v1, a
/// protocol of its own); other compositors have no primary output.
#[derive(Debug, Default)]
struct OutputOrder {
    /// The list being received.
    pending: Vec<String>,
    /// The last complete list.
    names: Vec<String>,
}

impl OutputOrder {
    /// Takes in an event of the protocol. Returns whether a list is now
    /// complete.
    fn take(&mut self, event: kde_output_order_v1::Event) -> bool {
        match event {
            kde_output_order_v1::Event::Output { output_name } => {
                self.pending.push(output_name);
                false
            }
            kde_output_order_v1::Event::Done => {
                self.names = std::mem::take(&mut self.pending);
                true
            }
            _ => false,
        }
    }

    /// The primary output's name, if it is known.
    fn primary(&self) -> Option<&str> {
        self.names.first().map(String::as_str)
    }
}

/// The Wayland side of the flyout.
pub struct Wayland {
    connection: Connection,
    queue: QueueHandle<Wayland>,
    registry: RegistryState,
    outputs: OutputState,
    seats: SeatState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    pointer: Option<wl_pointer::WlPointer>,
    /// Relative pointer motion, which moves a dragged flyout (positions on
    /// the surface change as it moves). `None` where the compositor lacks
    /// it: the flyout cannot be dragged then.
    relative: Option<(
        RelativePointerState,
        Option<wayland_protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_v1::ZwpRelativePointerV1>,
    )>,
    /// The part of a relative motion not moved yet (below a pixel).
    remainder: (f64, f64),
    instance: wgpu::Instance,
    gpu: Option<Gpu>,
    shown: Option<Shown>,
    /// Input for egui since the last frame.
    events: Vec<egui::Event>,
    /// Whether the mouse pointer is on the surface.
    pointer_inside: bool,
    /// The zoom (the scale setting), to turn logical pixels into points.
    zoom: f32,
    /// The output the flyout was last shown on.
    entered: Option<wl_output::WlOutput>,
    /// Where the left button went down on the surface (logical pixels),
    /// while it is down.
    pressed_at: Option<(f64, f64)>,
    /// Where the pointer was last on the surface (logical pixels).
    pointer_at: (f64, f64),
    /// Whether the user drags the flyout to move it.
    dragging: bool,
    /// How far the user dragged it, until it hides (logical pixels).
    dragged: (i32, i32),
    /// A drag that ended since the host last asked.
    move_ended: Option<(i32, i32)>,
    started: Instant,
    /// The order of the outputs, where the compositor tells it, and the
    /// object that receives it (kept alive for its changes).
    order: OutputOrder,
    _order_source: Option<KdeOutputOrderV1>,
}

impl Wayland {
    /// Binds the compositor's globals. Fails without layer-shell (GNOME).
    pub fn new(
        connection: Connection,
        globals: &GlobalList,
        queue: QueueHandle<Wayland>,
    ) -> anyhow::Result<Self> {
        let compositor = CompositorState::bind(globals, &queue)
            .context("the compositor has no wl_compositor")?;
        let layer_shell = LayerShell::bind(globals, &queue).context(
            "the compositor does not offer layer-shell (KDE Plasma, sway and Hyprland do; GNOME does not)",
        )?;
        let display = DisplayPointer(
            NonNull::new(connection.backend().display_ptr().cast())
                .context("no libwayland display")?,
        );
        let instance = wgpu::Instance::new(
            wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(display)),
        );
        let relative = RelativePointerState::bind(globals, &queue);
        // Only KDE Plasma has it; the primary output is unknown elsewhere.
        let order_source = globals
            .bind::<KdeOutputOrderV1, _, _>(&queue, 1..=1, ())
            .ok();
        Ok(Self {
            registry: RegistryState::new(globals),
            outputs: OutputState::new(globals, &queue),
            seats: SeatState::new(globals, &queue),
            connection,
            queue,
            compositor,
            layer_shell,
            pointer: None,
            relative: Some((relative, None)),
            remainder: (0.0, 0.0),
            instance,
            gpu: None,
            shown: None,
            events: Vec::new(),
            pointer_inside: false,
            zoom: 1.0,
            entered: None,
            pressed_at: None,
            pointer_at: (0.0, 0.0),
            dragging: false,
            dragged: (0, 0),
            move_ended: None,
            started: Instant::now(),
            order: OutputOrder::default(),
            _order_source: order_source,
        })
    }

    /// Whether the mouse pointer rests on the flyout.
    pub fn pointer_inside(&self) -> bool {
        self.shown.is_some() && self.pointer_inside
    }

    /// The size (logical pixels) of the output `index`, or else of the one
    /// the flyout is on (or the first one, before it is shown).
    pub fn output_size(&self, index: Option<usize>) -> Option<(i32, i32)> {
        let output = index
            .and_then(|i| self.outputs.outputs().nth(i))
            .or_else(|| self.entered.clone())
            .or_else(|| self.outputs.outputs().next())?;
        self.outputs.info(&output)?.logical_size
    }

    /// The primary output by its index (as [`Placement::output`] counts
    /// them), where the compositor tells which one it is (KDE Plasma).
    pub fn primary_output(&self) -> Option<usize> {
        let primary = self.order.primary()?;
        self.output_names().iter().position(|name| name == primary)
    }

    /// The outputs' names, in the order [`Placement::output`] counts them.
    pub fn output_names(&self) -> Vec<String> {
        self.outputs
            .outputs()
            .map(|output| {
                self.outputs
                    .info(&output)
                    .and_then(|info| info.name.clone())
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Shows the flyout at `placement`, or moves it there if it is shown.
    pub fn show(&mut self, placement: Placement, zoom: f32) {
        self.zoom = zoom;
        let applied = moved(placement, self.dragged);
        if let Some(shown) = &mut self.shown {
            // An output change needs a new surface (a layer surface's
            // output is fixed), and so does a layer change, which is rare
            // enough (the settings changed) not to change it in place.
            if shown.placement.output == placement.output
                && shown.placement.above_fullscreen == placement.above_fullscreen
            {
                shown.placement = placement;
                if shown.applied != applied {
                    apply(&shown.layer, &applied);
                    shown.layer.commit();
                    shown.applied = applied;
                }
                return;
            }
            self.hide();
        }
        let output = placement.output.and_then(|i| self.outputs.outputs().nth(i));
        let surface = self.compositor.create_surface(&self.queue);
        let layer = self.layer_shell.create_layer_surface(
            &self.queue,
            surface,
            if placement.above_fullscreen {
                Layer::Overlay
            } else {
                Layer::Top
            },
            Some("flexflyout"),
            output.as_ref(),
        );
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        apply(&layer, &applied);
        // The first commit carries no buffer; the compositor answers with
        // a configure, after which the surface can be drawn.
        layer.commit();
        self.shown = Some(Shown {
            target: None,
            layer,
            placement,
            applied,
            configured: false,
            size: (placement.width, placement.height),
            target_size: (0, 0),
            scale: 1,
        });
    }

    /// Hides the flyout (destroys its surface). A place it was dragged to
    /// lasts one appearance.
    pub fn hide(&mut self) {
        if let Some(mut shown) = self.shown.take() {
            shown.target = None;
            drop(shown);
            self.pointer_inside = false;
            self.pressed_at = None;
            self.dragging = false;
            self.dragged = (0, 0);
            let _ = self.connection.flush();
        }
    }

    /// Lets the user move the flyout with the left button that is down on
    /// it (the panel's background was grabbed).
    pub fn start_move(&mut self) {
        let relative = self.relative.as_ref().is_some_and(|(_, r)| r.is_some());
        let Some(pressed) = self.pressed_at.filter(|_| relative) else {
            return;
        };
        self.dragging = true;
        // egui calls it a drag only after the pointer moved a little: move
        // by that much at once, so that the grabbed spot stays under it.
        self.remainder = (self.pointer_at.0 - pressed.0, self.pointer_at.1 - pressed.1);
        self.move_by(0.0, 0.0);
    }

    /// Moves a dragged flyout by a relative motion (logical pixels); whole
    /// pixels at a time, with the rest kept for the next motion.
    fn move_by(&mut self, dx: f64, dy: f64) {
        let x = self.remainder.0 + dx;
        let y = self.remainder.1 + dy;
        let (dx, dy) = (x.trunc() as i32, y.trunc() as i32);
        self.remainder = (x.fract(), y.fract());
        if (dx, dy) == (0, 0) {
            return;
        }
        self.dragged = (self.dragged.0 + dx, self.dragged.1 + dy);
        if let Some(shown) = &mut self.shown {
            shown.applied = moved(shown.placement, self.dragged);
            apply(&shown.layer, &shown.applied);
            shown.layer.commit();
        }
    }

    /// Sends the requests made so far. Called after each dispatch, so that
    /// a move answered from an event callback (which must not flush itself:
    /// the dispatch holds the connection) reaches the compositor before
    /// the event loop sleeps.
    pub fn flush(&self) {
        let _ = self.connection.flush();
    }

    /// Forgets where the flyout was dragged (the placement settings now
    /// put it there, or elsewhere).
    pub fn forget_drag(&mut self) {
        self.dragged = (0, 0);
    }

    /// How far the user dragged the flyout, once per drag that ended.
    pub fn take_move_ended(&mut self) -> Option<(i32, i32)> {
        self.move_ended.take()
    }

    /// Draws a frame with `ui` (on top of a transparent background) if the
    /// surface is ready. Returns egui's output, for the repaint request.
    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        ui: impl FnMut(&mut egui::Ui),
    ) -> Option<egui::FullOutput> {
        let shown = self.shown.as_mut()?;
        if !shown.configured {
            return None;
        }
        if shown.target.is_none() {
            match make_target(
                &self.instance,
                &self.connection,
                &shown.layer,
                &mut self.gpu,
            ) {
                Ok(target) => shown.target = Some(target),
                Err(err) => {
                    log::error!("cannot draw the flyout: {err:#}");
                    return None;
                }
            }
            configure_target(shown, self.gpu.as_ref()?);
        }
        // The scale can change without a configure: keep the buffer in step.
        let pixels = (
            shown.size.0 * shown.scale as u32,
            shown.size.1 * shown.scale as u32,
        );
        if shown.target_size != pixels {
            configure_target(shown, self.gpu.as_ref()?);
        }
        let gpu = self.gpu.as_mut()?;
        let target = shown.target.as_ref()?;
        let scale = shown.scale as f32;
        let size_points = egui::vec2(
            shown.size.0 as f32 / self.zoom,
            shown.size.1 as f32 / self.zoom,
        );
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size_points)),
            time: Some(self.started.elapsed().as_secs_f64()),
            focused: true,
            events: std::mem::take(&mut self.events),
            ..Default::default()
        };
        if let Some(viewport) = input.viewports.get_mut(&egui::ViewportId::ROOT) {
            viewport.native_pixels_per_point = Some(scale);
        }
        ctx.set_zoom_factor(self.zoom);
        let output = ctx.run_ui(input, ui);

        let frame = match target.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                configure_target(shown, gpu);
                return Some(output);
            }
            other => {
                log::debug!("no frame to draw on: {other:?}");
                return Some(output);
            }
        };
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [pixels.0, pixels.1],
            pixels_per_point: output.pixels_per_point,
        };
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                gpu.renderer
                    .update_texture(&gpu.device, &gpu.queue, *id, delta);
            }
        }
        let jobs = ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let commands =
            gpu.renderer
                .update_buffers(&gpu.device, &gpu.queue, &mut encoder, &jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("flyout"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Transparent: only the rounded panel shows.
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            gpu.renderer
                .render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        gpu.queue
            .submit(commands.into_iter().chain([encoder.finish()]));
        gpu.queue.present(frame);
        for id in &output.textures_delta.free {
            gpu.renderer.free_texture(id);
        }
        Some(output)
    }
}

/// Sets the size, anchors and margins of a layer surface.
fn apply(layer: &LayerSurface, placement: &Placement) {
    let (top, right, bottom, left) = placement.margins;
    layer.set_size(placement.width, placement.height);
    layer.set_anchor(placement.anchor);
    layer.set_margin(top, right, bottom, left);
}

/// Makes the wgpu surface for a layer surface, and the GPU objects if they
/// do not exist yet.
fn make_target(
    instance: &wgpu::Instance,
    connection: &Connection,
    layer: &LayerSurface,
    gpu: &mut Option<Gpu>,
) -> anyhow::Result<wgpu::Surface<'static>> {
    let display = NonNull::new(connection.backend().display_ptr().cast()).context("no display")?;
    let surface = NonNull::new(layer.wl_surface().id().as_ptr().cast()).context("no surface")?;
    // SAFETY: the display and the surface outlive the wgpu surface, which
    // is dropped before the layer surface (see `Shown`).
    let target = unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
                display,
            ))),
            raw_window_handle: RawWindowHandle::Wayland(WaylandWindowHandle::new(surface)),
        })
    }
    .context("cannot make a wgpu surface")?;
    if gpu.is_none() {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&target),
            power_preference: wgpu::PowerPreference::LowPower,
            ..Default::default()
        }))
        .context("no graphics adapter")?;
        log::info!("graphics adapter: {:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .context("cannot open the graphics device")?;
        // wgpu panics on a validation error by default; a flyout that lives
        // in the tray must rather lose a frame (and say so in the log).
        device.on_uncaptured_error(std::sync::Arc::new(|err| {
            log::error!("graphics error: {err}");
        }));
        let capabilities = target.get_capabilities(&adapter);
        // egui draws in sRGB-encoded colors: a non-sRGB format, as eframe
        // picks.
        let format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .or(capabilities.formats.first().copied())
            .context("the surface has no format")?;
        let renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());
        *gpu = Some(Gpu {
            adapter,
            device,
            queue,
            renderer,
            format,
        });
    }
    Ok(target)
}

/// Configures the wgpu surface for the surface's size and scale.
fn configure_target(shown: &mut Shown, gpu: &Gpu) {
    let Some(target) = &shown.target else {
        return;
    };
    let capabilities = target.get_capabilities(&gpu.adapter);
    // Premultiplied alpha, as egui draws, where the compositor takes it.
    let alpha_mode = [
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied,
    ]
    .into_iter()
    .find(|mode| capabilities.alpha_modes.contains(mode))
    .unwrap_or(wgpu::CompositeAlphaMode::Auto);
    // Never wait for the compositor's frame callback (FIFO does): it may
    // not come for a while (the surface moving, an occluded output), and
    // waiting would stall the whole event loop. The flyout draws only
    // when something changed anyway.
    let present_mode = [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
        .into_iter()
        .find(|mode| capabilities.present_modes.contains(mode))
        .unwrap_or(wgpu::PresentMode::Fifo);
    shown.layer.wl_surface().set_buffer_scale(shown.scale);
    target.configure(
        &gpu.device,
        &wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: gpu.format,
            width: shown.size.0 * shown.scale as u32,
            height: shown.size.1 * shown.scale as u32,
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        },
    );
    shown.target_size = (
        shown.size.0 * shown.scale as u32,
        shown.size.1 * shown.scale as u32,
    );
}

impl LayerShellHandler for Wayland {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        // The compositor dropped it (its output went away): show it anew
        // next time.
        self.shown = None;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let Some(shown) = self.shown.as_mut().filter(|shown| &shown.layer == layer) else {
            return;
        };
        let (width, height) = configure.new_size;
        if width > 0 && height > 0 {
            if (width, height) != (shown.applied.width, shown.applied.height) {
                log::info!(
                    "the compositor sized the flyout {width}x{height} instead of {}x{}",
                    shown.applied.width,
                    shown.applied.height
                );
            }
            shown.size = (width, height);
        }
        // A change of the margins (a slide, a drag) is applied by the
        // compositor once the configure it answers with is acknowledged
        // (sctk does that) and the surface is committed again: commit now
        // rather than at the next frame, which may be a while away.
        // (Sent by `flush` right after this dispatch: flushing from inside
        // it would deadlock.)
        if shown.configured {
            shown.layer.commit();
        }
        shown.configured = true;
        if let Some(gpu) = &self.gpu {
            configure_target(shown, gpu);
        }
    }
}

impl CompositorHandler for Wayland {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        factor: i32,
    ) {
        if let Some(shown) = self
            .shown
            .as_mut()
            .filter(|shown| shown.layer.wl_surface() == surface)
            && shown.scale != factor
        {
            shown.scale = factor.max(1);
            if let Some(gpu) = &self.gpu {
                configure_target(shown, gpu);
            }
        }
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        output: &wl_output::WlOutput,
    ) {
        if self
            .shown
            .as_ref()
            .is_some_and(|s| s.layer.wl_surface() == surface)
        {
            self.entered = Some(output.clone());
        }
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Wayland {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl SeatHandler for Wayland {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        queue: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            match self.seats.get_pointer(queue, &seat) {
                Ok(pointer) => {
                    if let Some((state, relative)) = &mut self.relative {
                        match state.get_relative_pointer(&pointer, queue) {
                            Ok(new) => *relative = Some(new),
                            Err(err) => {
                                log::info!("no relative pointer motion (no dragging): {err}")
                            }
                        }
                    }
                    self.pointer = Some(pointer);
                }
                Err(err) => log::warn!("no mouse pointer: {err}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && let Some(pointer) = self.pointer.take()
        {
            pointer.release();
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for Wayland {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let Some(surface) = self.shown.as_ref().map(|s| s.layer.wl_surface().clone()) else {
            return;
        };
        for event in events {
            if event.surface != surface {
                continue;
            }
            // Logical pixels to egui points.
            let pos = egui::pos2(
                event.position.0 as f32 / self.zoom,
                event.position.1 as f32 / self.zoom,
            );
            match event.kind {
                PointerEventKind::Enter { .. } => {
                    self.pointer_inside = true;
                    self.events.push(egui::Event::PointerMoved(pos));
                }
                PointerEventKind::Leave { .. } => {
                    self.pointer_inside = false;
                    self.events.push(egui::Event::PointerGone);
                }
                PointerEventKind::Motion { .. } => {
                    self.pointer_at = event.position;
                    // While dragging, the relative motion moves the surface,
                    // and egui must not see the pointer move on it.
                    if !self.dragging {
                        self.events.push(egui::Event::PointerMoved(pos));
                    }
                }
                PointerEventKind::Press { button, .. }
                | PointerEventKind::Release { button, .. } => {
                    let pressed = matches!(event.kind, PointerEventKind::Press { .. });
                    if button == BTN_LEFT {
                        self.pressed_at = pressed.then_some(event.position);
                        if !pressed && self.dragging {
                            self.dragging = false;
                            self.move_ended = Some(self.dragged);
                        }
                    }
                    let button = match button {
                        BTN_LEFT => egui::PointerButton::Primary,
                        BTN_RIGHT => egui::PointerButton::Secondary,
                        BTN_MIDDLE => egui::PointerButton::Middle,
                        _ => continue,
                    };
                    self.events.push(egui::Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                }
                PointerEventKind::Axis {
                    horizontal,
                    vertical,
                    source,
                    ..
                } => {
                    if let Some(event) = wheel_event(&horizontal, &vertical, source, self.zoom) {
                        self.events.push(event);
                    }
                }
            }
        }
    }
}

/// egui's wheel event for a scroll of the pointer's axes: in notches for a
/// wheel (high-resolution steps count in 120ths), else in points (`zoom`
/// logical pixels each). Wayland's axes grow down and to the right, egui's
/// delta the other way (it says where the content moves). `None` for no
/// movement (the end of a touchpad scroll).
fn wheel_event(
    horizontal: &AxisScroll,
    vertical: &AxisScroll,
    source: Option<wl_pointer::AxisSource>,
    zoom: f32,
) -> Option<egui::Event> {
    let (unit, delta) = if horizontal.value120 != 0 || vertical.value120 != 0 {
        let notches = |axis: &AxisScroll| -(axis.value120 as f32) / 120.0;
        (
            egui::MouseWheelUnit::Line,
            egui::vec2(notches(horizontal), notches(vertical)),
        )
    } else if horizontal.discrete != 0 || vertical.discrete != 0 {
        // Compositors older than wl_pointer 8.
        (
            egui::MouseWheelUnit::Line,
            egui::vec2(-horizontal.discrete as f32, -vertical.discrete as f32),
        )
    } else {
        let points = |axis: &AxisScroll| -(axis.absolute as f32) / zoom;
        (
            egui::MouseWheelUnit::Point,
            egui::vec2(points(horizontal), points(vertical)),
        )
    };
    if delta == egui::Vec2::ZERO {
        return None;
    }
    let source = match source {
        Some(wl_pointer::AxisSource::Wheel | wl_pointer::AxisSource::WheelTilt) => {
            egui::MouseWheelSource::Wheel
        }
        Some(wl_pointer::AxisSource::Finger | wl_pointer::AxisSource::Continuous) => {
            egui::MouseWheelSource::Trackpad
        }
        _ => egui::MouseWheelSource::Unknown,
    };
    Some(egui::Event::MouseWheel {
        unit,
        delta,
        phase: egui::TouchPhase::Move,
        source,
        modifiers: egui::Modifiers::NONE,
    })
}

impl Dispatch<KdeOutputOrderV1, ()> for Wayland {
    fn event(
        state: &mut Self,
        _: &KdeOutputOrderV1,
        event: kde_output_order_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if state.order.take(event) {
            log::info!(
                "the outputs in order, primary first: {:?}",
                state.order.names
            );
        }
    }
}

impl RelativePointerHandler for Wayland {
    fn relative_pointer_motion(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wayland_protocols::wp::relative_pointer::zv1::client::zwp_relative_pointer_v1::ZwpRelativePointerV1,
        _: &wl_pointer::WlPointer,
        event: RelativeMotionEvent,
    ) {
        if self.dragging {
            self.move_by(event.delta.0, event.delta.1);
        }
    }
}

impl ProvidesRegistryState for Wayland {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(Wayland);
smithay_client_toolkit::delegate_dispatch2!(Wayland);

/// The monitors, as the settings window lists them: their positions and
/// sizes in logical pixels, in the order [`Placement::output`] counts them,
/// with the primary one marked where the compositor tells it (KDE Plasma).
/// Empty without a Wayland session.
pub fn outputs() -> Vec<flexflyout_core::layout::MonitorArea> {
    use flexflyout_core::layout::{MonitorArea, PixelRect};

    /// Only the outputs, for a short connection.
    struct Outputs {
        registry: RegistryState,
        outputs: OutputState,
        order: OutputOrder,
    }
    impl Dispatch<KdeOutputOrderV1, ()> for Outputs {
        fn event(
            state: &mut Self,
            _: &KdeOutputOrderV1,
            event: kde_output_order_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            state.order.take(event);
        }
    }
    impl OutputHandler for Outputs {
        fn output_state(&mut self) -> &mut OutputState {
            &mut self.outputs
        }
        fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
        fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        }
        fn output_destroyed(
            &mut self,
            _: &Connection,
            _: &QueueHandle<Self>,
            _: wl_output::WlOutput,
        ) {
        }
    }
    impl ProvidesRegistryState for Outputs {
        fn registry(&mut self) -> &mut RegistryState {
            &mut self.registry
        }
        registry_handlers![OutputState];
    }
    delegate_registry!(Outputs);
    smithay_client_toolkit::delegate_dispatch2!(Outputs);

    let list = || -> Option<Vec<MonitorArea>> {
        let connection = Connection::connect_to_env().ok()?;
        let (globals, mut queue) =
            wayland_client::globals::registry_queue_init::<Outputs>(&connection).ok()?;
        let mut state = Outputs {
            registry: RegistryState::new(&globals),
            outputs: OutputState::new(&globals, &queue.handle()),
            order: OutputOrder::default(),
        };
        let _order = globals
            .bind::<KdeOutputOrderV1, _, _>(&queue.handle(), 1..=1, ())
            .ok();
        // The outputs' details and their order arrive after their
        // announcement.
        queue.roundtrip(&mut state).ok()?;
        queue.roundtrip(&mut state).ok()?;
        let primary = state.order.primary().map(str::to_owned);
        Some(
            state
                .outputs
                .outputs()
                .filter_map(|output| state.outputs.info(&output))
                .map(|info| {
                    let (x, y) = info.logical_position.unwrap_or_default();
                    let (width, height) = info.logical_size.unwrap_or_default();
                    let bounds = PixelRect::from_origin_size(x, y, width, height);
                    MonitorArea {
                        bounds,
                        work: bounds,
                        scale: info.scale_factor as f32,
                        primary: primary.is_some() && info.name == primary,
                    }
                })
                .collect(),
        )
    };
    list().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wlr_layer::Anchor as A;

    fn display(anchor: Anchor) -> Display {
        Display {
            anchor,
            offset_x: 24,
            offset_y: 16,
            ..Display::default()
        }
    }

    #[test]
    fn corners_are_anchored_to_their_edges() {
        let p = placement(&display(Anchor::TopRight), (400, 164), (0, 0), None);
        assert_eq!(p.anchor, A::TOP | A::RIGHT);
        assert_eq!(p.margins, (16, 24, 0, 0));
        let p = placement(&display(Anchor::BottomLeft), (400, 164), (0, 0), None);
        assert_eq!(p.anchor, A::BOTTOM | A::LEFT);
        assert_eq!(p.margins, (0, 0, 16, 24));
    }

    #[test]
    fn centered_axes_are_centered_on_the_output() {
        // Bottom center of a 1920-wide output, 24 to the right: anchored to
        // the left edge, (1920 - 400) / 2 + 24 from it.
        let area = Some((1920, 1080));
        let p = placement(&display(Anchor::Bottom), (400, 164), (0, 0), area);
        assert_eq!(p.anchor, A::LEFT | A::BOTTOM);
        assert_eq!(p.margins, (0, 0, 16, 784));
        // The center, 16 down: both axes from their start edges.
        let p = placement(&display(Anchor::Center), (400, 164), (0, 0), area);
        assert_eq!(p.anchor, A::LEFT | A::TOP);
        assert_eq!(p.margins, (474, 0, 0, 784));
        // Without the output's size, the compositor centers it.
        let p = placement(&display(Anchor::Left), (400, 164), (0, 0), None);
        assert_eq!(p.anchor, A::LEFT);
        assert_eq!(p.margins, (0, 0, 0, 24));
    }

    #[test]
    fn dragging_moves_the_margins_and_the_offsets() {
        // Top right, dragged 30 left and 10 down.
        let top_right = display(Anchor::TopRight);
        let p = moved(placement(&top_right, (400, 164), (0, 0), None), (-30, 10));
        assert_eq!(p.margins, (26, 54, 0, 0));
        let again = display_moved(&top_right, (-30, 10));
        assert_eq!((again.offset_x, again.offset_y), (54, 26));
        assert_eq!(
            placement(&again, (400, 164), (0, 0), None).margins,
            p.margins
        );
        // Bottom center, dragged 40 left.
        let area = Some((1920, 1080));
        let bottom = display(Anchor::Bottom);
        let p = moved(placement(&bottom, (400, 164), (0, 0), area), (-40, 0));
        assert_eq!(p.margins, (0, 0, 16, 744));
        let again = display_moved(&bottom, (-40, 0));
        assert_eq!(
            placement(&again, (400, 164), (0, 0), area).margins,
            p.margins
        );
    }

    #[test]
    fn sliding_moves_towards_the_edge() {
        // Top right, sliding 28 up: a smaller top margin.
        let p = placement(&display(Anchor::TopRight), (400, 164), (0, -28), None);
        assert_eq!(p.margins, (-12, 24, 0, 0));
        // Bottom center, sliding 28 down: a smaller bottom margin.
        let p = placement(
            &display(Anchor::Bottom),
            (400, 164),
            (0, 28),
            Some((1920, 1080)),
        );
        assert_eq!(p.margins, (0, 0, -12, 784));
    }

    #[test]
    fn the_output_order_names_the_primary_output_first() {
        let output = |name: &str| kde_output_order_v1::Event::Output {
            output_name: name.into(),
        };
        let mut order = OutputOrder::default();
        assert_eq!(order.primary(), None);
        assert!(!order.take(output("DP-2")));
        assert!(!order.take(output("HDMI-A-1")));
        // Only a complete list counts.
        assert_eq!(order.primary(), None);
        assert!(order.take(kde_output_order_v1::Event::Done));
        assert_eq!(order.primary(), Some("DP-2"));
        // A new list replaces it when it is complete.
        order.take(output("HDMI-A-1"));
        assert_eq!(order.primary(), Some("DP-2"));
        order.take(output("DP-2"));
        order.take(kde_output_order_v1::Event::Done);
        assert_eq!(order.primary(), Some("HDMI-A-1"));
    }

    #[test]
    fn the_wheel_turns_into_egui_steps() {
        let delta = |horizontal: AxisScroll, vertical: AxisScroll| match wheel_event(
            &horizontal,
            &vertical,
            None,
            2.0,
        ) {
            Some(egui::Event::MouseWheel { unit, delta, .. }) => Some((unit, delta)),
            _ => None,
        };
        let none = AxisScroll::default();
        // A notch down on Wayland (120 high-resolution units, 15 pixels)
        // is a step down for egui, whose delta grows upwards.
        let notch_down = AxisScroll {
            absolute: 15.0,
            discrete: 1,
            value120: 120,
            ..AxisScroll::default()
        };
        assert_eq!(
            delta(none, notch_down),
            Some((egui::MouseWheelUnit::Line, egui::vec2(0.0, -1.0)))
        );
        // Half a notch of a high-resolution wheel.
        let half_up = AxisScroll {
            absolute: -7.5,
            value120: -60,
            ..AxisScroll::default()
        };
        assert_eq!(
            delta(none, half_up),
            Some((egui::MouseWheelUnit::Line, egui::vec2(0.0, 0.5)))
        );
        // A touchpad: pixels, in points at the zoom of 2.
        let finger = AxisScroll {
            absolute: -10.0,
            ..AxisScroll::default()
        };
        assert_eq!(
            delta(finger, none),
            Some((egui::MouseWheelUnit::Point, egui::vec2(5.0, 0.0)))
        );
        // The end of a touchpad scroll moves nothing.
        let stop = AxisScroll {
            stop: true,
            ..AxisScroll::default()
        };
        assert_eq!(delta(none, stop), None);
    }
}
