//! The Linux host (Wayland, layer-shell): one event loop that takes the
//! compositor's events and the sources' messages, and draws the flyout
//! while it is shown.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use flexflyout_core::config::{Animation, Monitor};
use flexflyout_core::layout::{self, PixelRect};
use flexflyout_core::{Command, Config};
use flexflyout_ui::{BASE_SIZE, Labels};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop::channel::{self, Sender};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use wayland_client::Connection;
use wayland_client::globals::registry_queue_init;

use crate::flyout::{FlyoutState, Received};
use crate::message::Message;
use crate::platform::linux::layer::{self, Wayland};

/// Time between frames while the flyout fades or slides.
const FRAME: Duration = Duration::from_millis(16);

/// Options of the Linux host.
#[derive(Clone, Debug)]
pub struct Options {
    /// The settings file to watch for changes.
    pub config_file: PathBuf,
    /// The language given on the command line, which overrides the one in
    /// the settings.
    pub lang: Option<String>,
    /// Show the demo, this many times (0 = endless), instead of following
    /// the system.
    pub demo: Option<u32>,
}

/// The state of the event loop.
struct Host {
    wayland: Wayland,
    state: FlyoutState,
    ctx: egui::Context,
    /// Whether this is the demo (no player, no device).
    demo: bool,
    exit: bool,
    /// Why and when the flyout was last asked to show, until it is drawn.
    pending: Option<(crate::message::Trigger, Instant)>,
}

/// Runs the flyout until it is asked to quit.
pub fn run(labels: Labels, config: Config, options: Options) -> anyhow::Result<()> {
    let connection =
        Connection::connect_to_env().context("no Wayland session to show the flyout in")?;
    let (globals, queue) =
        registry_queue_init::<Wayland>(&connection).context("cannot talk to the compositor")?;
    let wayland = Wayland::new(connection.clone(), &globals, queue.handle())?;

    let mut event_loop: EventLoop<Host> = EventLoop::try_new().context("no event loop")?;
    let handle = event_loop.handle();
    handle
        .insert_source(WaylandSource::new(connection, queue), |_, queue, host| {
            queue.dispatch_pending(&mut host.wayland)
        })
        .map_err(|err| anyhow::anyhow!("cannot watch the compositor: {err}"))?;
    let (sender, messages) = channel::channel();
    handle
        .insert_source(messages, |event, _, host: &mut Host| {
            if let channel::Event::Msg(message) = event {
                host.receive(message);
            }
        })
        .map_err(|err| anyhow::anyhow!("cannot receive messages: {err}"))?;

    let ctx = egui::Context::default();
    ctx.options_mut(|options| options.zoom_with_keyboard = false);
    let fonts = flexflyout_ui::fonts::install(&ctx, crate::platform::linux::fonts::find());
    log::info!("fonts: {fonts:?}");

    watch_settings(&options, &config, sender.clone());
    match options.demo {
        Some(cycles) => crate::demo::spawn(cycles, move |message| {
            let _ = sender.send(message);
        }),
        None => log::warn!("following the player and the volume is not there yet on Linux"),
    }

    let mut host = Host {
        wayland,
        state: FlyoutState::new(labels, config),
        ctx,
        demo: options.demo.is_some(),
        exit: false,
        pending: None,
    };
    log::info!("outputs: {:?}", host.wayland.output_names());
    while !host.exit {
        let timeout = host.tick();
        event_loop
            .dispatch(timeout, &mut host)
            .context("the event loop failed")?;
    }
    host.wayland.hide();
    Ok(())
}

impl Host {
    /// Takes in a message from a source.
    fn receive(&mut self, message: Message) {
        match self.state.receive(message, Instant::now()) {
            Received::Content => {}
            Received::Settings(old) => {
                log::debug!("settings changed from {old:?}");
            }
            Received::Shown { trigger, sent } => self.pending = Some((trigger, sent)),
            Received::Quit => {
                log::info!("closing");
                self.exit = true;
            }
        }
    }

    /// Shows, moves, draws or hides the flyout as the timer says. Returns
    /// how long the event loop may wait for an event before calling again.
    fn tick(&mut self) -> Option<Duration> {
        let now = Instant::now();
        let mut wake = self.state.keep_alive(now, self.wayland.pointer_inside());
        match self.state.timer.next_change(now) {
            Some(after) => {
                self.wayland
                    .show(self.placement(now), self.state.config.display.scale);
                if let Some(output) = self.draw(now) {
                    let delay = output
                        .viewport_output
                        .get(&egui::ViewportId::ROOT)
                        .map(|viewport| viewport.repaint_delay);
                    wake = earliest(wake, delay);
                }
                // While fading, every frame; else when the fade-out starts.
                wake = earliest(wake, Some(after.max(FRAME)));
            }
            None => self.wayland.hide(),
        }
        wake
    }

    /// Where the flyout goes now (it slides while fading).
    fn placement(&self, now: Instant) -> layer::Placement {
        let display = &self.state.config.display;
        let size = (
            (BASE_SIZE.x * display.scale).round() as u32,
            (BASE_SIZE.y * display.scale).round() as u32,
        );
        let slide = match self.state.config.timing.animation {
            Animation::Slide => {
                let rest = PixelRect::from_origin_size(0, 0, size.0 as i32, size.1 as i32);
                let fraction = layout::slide_fraction(self.state.timer.opacity(now));
                let moved = layout::slide(rest, display.anchor, fraction);
                (moved.left, moved.top)
            }
            Animation::Fade => (0, 0),
        };
        let mut placement = layer::placement(display, size, slide);
        // The compositor picks the monitor for "primary" and "cursor" (KWin
        // shows it on the active one); a number names one.
        if let Monitor::Number(n) = display.monitor {
            let index = n as usize - 1;
            if index < self.wayland.output_names().len() {
                placement.output = Some(index);
            }
        }
        placement
    }

    /// Draws a frame and carries out what the user did with the controls.
    fn draw(&mut self, now: Instant) -> Option<egui::FullOutput> {
        let opacity = self.state.timer.opacity(now);
        let mut clicked = None;
        let view = self.state.view(now);
        let output = self.wayland.draw(&self.ctx, |ui| {
            if opacity <= 0.0 {
                return;
            }
            ui.multiply_opacity(opacity);
            clicked = flexflyout_ui::draw(ui, &view);
        })?;
        if let Some((trigger, sent)) = self.pending.take() {
            log::info!(
                "{trigger:?}: drawing {:.1} ms after the message was sent",
                sent.elapsed().as_secs_f64() * 1000.0
            );
        }
        if let Some(command) = clicked {
            self.carry_out(command, now);
        }
        Some(output)
    }

    /// Carries out a control: the flyout's own part, then the platform's.
    fn carry_out(&mut self, command: Command, now: Instant) {
        if let Some(command) = self.state.carry_out(command, now, self.demo) {
            log::info!("not carried out on Linux yet: {command:?}");
        }
    }
}

/// The earlier of two optional waits.
fn earliest(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Sends the settings to the flyout every time the file is saved, with the
/// texts in the new language when that changed.
fn watch_settings(options: &Options, config: &Config, sender: Sender<Message>) {
    let lang = options.lang.clone();
    let mut language = crate::settings::language(lang.as_deref(), config);
    // The handle to reload on demand is for the tray, which comes later.
    let _ = crate::settings::watch(options.config_file.clone(), move |config| {
        let new_language = crate::settings::language(lang.as_deref(), &config);
        let labels = (new_language != language)
            .then(|| match crate::i18n::load(new_language.as_deref()) {
                Ok(loader) => Some(crate::i18n::labels(&loader)),
                Err(err) => {
                    log::error!("{err:#}");
                    None
                }
            })
            .flatten();
        if labels.is_some() {
            language = new_language;
        }
        let _ = sender.send(Message::Settings {
            config: Box::new(config),
            labels,
        });
    });
}
