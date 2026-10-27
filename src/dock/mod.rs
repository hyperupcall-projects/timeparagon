//! The dock: a thin always-visible strip of today, pinned to a screen edge.
//!
//! The strip is a `zwlr_layer_shell_v1` surface, which is what lets it hug the
//! edge with no left margin, stay above other windows, and be exactly as wide
//! as asked. It shows only the first column of the day view (the highest
//! priority thing at each moment) as coloured spans on the day view's own
//! background, with a red line for "right now". Hovering a span opens a card
//! beside the strip, level with the span.
//!
//! A small button at the very top of the strip, no wider than the strip, brings
//! back the window the dock replaced (see [`launch`]).
//!
//! The surface is wider than the strip, to leave room for that card. Only the
//! strip is opaque and only the strip takes pointer input, so the rest of the
//! surface neither hides nor blocks what is underneath.
//!
//! Layer shell is supported by KDE Plasma, Sway, Hyprland, niri and COSMIC. It
//! is not available on GNOME.

pub mod config;
pub mod launch;
pub mod model;
pub mod render;

use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use chrono::{Local, NaiveDate};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{EventLoop, channel};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{
    wl_output, wl_pointer, wl_seat, wl_shm, wl_surface,
};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm, registry_handlers,
};

use crate::calendar::{self, Snapshot, Store};
use crate::config::Config;
use crate::panel::Side;
use crate::theme::Catalog;

use render::{Layout, Palette, Renderer, Scene};

/// Linux evdev code of the left mouse button, which is what Wayland reports.
const BTN_LEFT: u32 = 0x110;

/// How often the "now" line moves and the day rolls over.
const TICK: Duration = Duration::from_secs(30);

/// Runs the dock until its surface is closed.
pub fn run() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("timetracker=info"))
        .init();

    let origin = launch::Origin::from_args(std::env::args().skip(1));
    let dock_config = config::DockConfig::load();
    let app_config = Config::load();
    log::info!(
        "timetracker-dock starting (side={:?}, width={}px)",
        dock_config.side,
        dock_config.width
    );

    let connection = Connection::connect_to_env().context("no Wayland compositor to connect to")?;
    let (globals, event_queue) = registry_queue_init(&connection)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor is missing")?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .context("this compositor has no wlr-layer-shell, so it cannot host the dock (GNOME?)")?;
    let shm = Shm::bind(&globals, &qh).context("wl_shm is missing")?;

    let surface = compositor.create_surface(&qh);
    let layer =
        layer_shell.create_layer_surface(&qh, surface, Layer::Top, Some("timetracker-dock"), None);
    let (anchor, margin) = match dock_config.side {
        Side::Left => (
            Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM,
            (dock_config.margin_top, dock_config.margin_bottom),
        ),
        Side::Right => (
            Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM,
            (dock_config.margin_top, dock_config.margin_bottom),
        ),
    };
    layer.set_anchor(anchor);
    layer.set_margin(margin.0 as i32, 0, margin.1 as i32, 0);
    layer.set_exclusive_zone(0);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    // A height of zero asks the compositor to stretch between the anchors.
    layer.set_size(dock_config.surface_width(), 0);
    layer.commit();

    let theme = Catalog::load()
        .resolve(&app_config.appearance.theme)
        .theme
        .clone();

    let day_start_hour = app_config.calendar.day_start_hour();
    let mut store = Store::new(day_start_hour);
    store.cache_days_in(crate::config::cache_days_dir());
    store.apply_task_colors(&app_config.calendar);
    let today = calendar::day_at(Local::now(), day_start_hour);
    store.set_day(today, today);

    let mut event_loop: EventLoop<Dock> = EventLoop::try_new()?;
    let handle = event_loop.handle();

    let (snapshots_tx, snapshots_rx) = channel::channel::<Result<Snapshot, String>>();
    let wake = store
        .is_signed_in()
        .then(|| spawn_sync(&app_config, snapshots_tx));

    let pool = SlotPool::new(4096, &shm).context("could not allocate shared memory")?;
    let mut dock = Dock {
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        compositor,
        layer,
        pool,
        pointer: None,
        config: dock_config,
        app_config,
        palette: Palette::from_theme(&theme),
        store,
        segments: Vec::new(),
        day: today,
        origin,
        hovered: None,
        button_hovered: false,
        pointer_position: None,
        size: None,
        scale: 1,
        renderer: Renderer::default(),
        wake,
        exit: false,
    };
    dock.refresh_segments();

    WaylandSource::new(connection, event_queue)
        .insert(handle.clone())
        .map_err(|error| anyhow!("could not watch the Wayland socket: {error}"))?;
    handle
        .insert_source(Timer::from_duration(TICK), |_, _, dock| {
            dock.tick();
            TimeoutAction::ToDuration(TICK)
        })
        .map_err(|error| anyhow!("could not start the clock: {error}"))?;
    handle
        .insert_source(snapshots_rx, |event, _, dock| {
            if let channel::Event::Msg(result) = event {
                dock.synced(result);
            }
        })
        .map_err(|error| anyhow!("could not listen for syncs: {error}"))?;

    while !dock.exit {
        event_loop.dispatch(None, &mut dock)?;
    }

    Ok(())
}

/// Syncs today in the background, on the app's own cadence, and again whenever
/// woken (the day rolled over). The returned sender is that wake-up.
fn spawn_sync(
    config: &Config,
    results: channel::Sender<Result<Snapshot, String>>,
) -> mpsc::Sender<()> {
    let (wake, woken) = mpsc::channel::<()>();
    let calendar = config.calendar.clone();

    std::thread::Builder::new()
        .name("dock-sync".into())
        .spawn(move || {
            let hour = calendar.day_start_hour();
            let every = Duration::from_secs(u64::from(calendar.sync_minutes.max(1)) * 60);

            loop {
                let day = calendar::day_at(Local::now(), hour);
                let result = calendar::sync_day(
                    &calendar.client_id,
                    &calendar.client_secret,
                    &calendar.disabled,
                    day,
                    hour,
                    calendar.tasks_calendar.as_deref(),
                );
                if results.send(result).is_err() {
                    break;
                }
                if let Err(mpsc::RecvTimeoutError::Disconnected) = woken.recv_timeout(every) {
                    break;
                }
            }
        })
        .expect("failed to spawn the dock sync thread");

    wake
}

struct Dock {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    compositor: CompositorState,
    layer: LayerSurface,
    pool: SlotPool,
    pointer: Option<wl_pointer::WlPointer>,

    config: config::DockConfig,
    app_config: Config,
    palette: Palette,
    store: Store,
    segments: Vec<model::Segment>,
    day: NaiveDate,

    /// What the dock replaced, which its back button returns to.
    origin: launch::Origin,
    hovered: Option<usize>,
    button_hovered: bool,
    pointer_position: Option<(f64, f64)>,
    /// The configured logical size, once the compositor has sent one.
    size: Option<(u32, u32)>,
    scale: i32,
    renderer: Renderer,
    wake: Option<mpsc::Sender<()>>,
    exit: bool,
}

impl Dock {
    fn layout(&self) -> Option<Layout> {
        let (width, height) = self.size?;
        Some(Layout::new(&self.config, width, height, self.scale as u32))
    }

    fn refresh_segments(&mut self) {
        self.segments = model::segments(&self.store, &self.app_config.calendar, self.day);
        self.hovered = match (self.layout(), self.pointer_position) {
            (Some(layout), Some((x, y))) => layout.hit(&self.segments, x, y),
            _ => None,
        };
    }

    /// Hands over to what the dock replaced, then quits.
    fn go_back(&mut self) {
        match launch::close_dock(self.origin) {
            Ok(()) => self.exit = true,
            Err(error) => log::error!("could not leave the dock: {error:#}"),
        }
    }

    /// Moves the "now" line, and follows the clock into a new day.
    fn tick(&mut self) {
        let today = calendar::day_at(Local::now(), self.app_config.calendar.day_start_hour());
        if today != self.day {
            self.store.set_day(today, today);
            self.day = today;
            if let Some(wake) = &self.wake {
                let _ = wake.send(());
            }
            self.refresh_segments();
        }
        self.draw();
    }

    fn synced(&mut self, result: Result<Snapshot, String>) {
        match result {
            Ok(snapshot) if snapshot.day == self.day => {
                self.store.apply_snapshot(snapshot);
                self.store.apply_task_colors(&self.app_config.calendar);
                self.refresh_segments();
                self.draw();
            }
            Ok(_) => {}
            Err(error) => log::warn!("dock sync failed: {error}"),
        }
    }

    fn draw(&mut self) {
        let Some(layout) = self.layout() else {
            return;
        };

        let (width, height) = layout.pixel_size();
        let Some(mut pixmap) = tiny_skia::Pixmap::new(width, height) else {
            return;
        };

        let now = model::now_minute(Local::now(), self.day, &self.app_config.calendar);
        let scene = Scene {
            segments: &self.segments,
            now_minute: now,
            hovered: self.hovered,
            button_hovered: self.button_hovered,
            back_label: self.origin.back_label(),
            side: self.config.side,
            palette: self.palette,
        };
        self.renderer.paint(&layout, &scene, &mut pixmap);

        let stride = width as i32 * 4;
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) else {
            log::warn!("could not allocate a frame");
            return;
        };

        // tiny-skia is premultiplied RGBA; wl_shm's ARGB8888 is premultiplied
        // BGRA when read a byte at a time on little-endian machines.
        for (target, source) in canvas
            .chunks_exact_mut(4)
            .zip(pixmap.data().chunks_exact(4))
        {
            target.copy_from_slice(&[source[2], source[1], source[0], source[3]]);
        }

        let surface = self.layer.wl_surface();
        surface.set_buffer_scale(self.scale);
        surface.damage_buffer(0, 0, width as i32, height as i32);

        // The strip alone is opaque and clickable.
        let (x, y, w, h) = layout.strip_rect();
        let region = Region::new(&self.compositor).ok();
        if let Some(region) = &region {
            region.add(x as i32, y as i32, w as i32, h as i32);
            self.layer.set_input_region(Some(region.wl_region()));
            self.layer.set_opaque_region(Some(region.wl_region()));
        }

        if buffer.attach_to(surface).is_err() {
            return;
        }
        self.layer.commit();
    }

    fn pointer_moved(&mut self, position: Option<(f64, f64)>) {
        self.pointer_position = position;
        let (hovered, button_hovered) = match (self.layout(), position) {
            (Some(layout), Some((x, y))) => {
                (layout.hit(&self.segments, x, y), layout.hit_button(x, y))
            }
            _ => (None, false),
        };
        if hovered != self.hovered || button_hovered != self.button_hovered {
            self.hovered = hovered;
            self.button_hovered = button_hovered;
            self.draw();
        }
    }
}

impl CompositorHandler for Dock {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if new_factor != self.scale {
            self.scale = new_factor.max(1);
            self.draw();
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
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
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

impl OutputHandler for Dock {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Dock {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (width, height) = configure.new_size;
        let width = if width == 0 {
            self.config.surface_width()
        } else {
            width
        };
        if height == 0 {
            return;
        }

        self.size = Some((width, height));
        self.refresh_segments();
        self.draw();
    }
}

impl SeatHandler for Dock {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => self.pointer = Some(pointer),
                Err(error) => log::warn!("could not use the pointer: {error}"),
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

impl PointerHandler for Dock {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    self.pointer_moved(Some(event.position));
                }
                PointerEventKind::Leave { .. } => self.pointer_moved(None),
                PointerEventKind::Press { button, .. }
                    if button == BTN_LEFT && self.button_hovered =>
                {
                    self.go_back();
                }
                _ => {}
            }
        }
    }
}

impl ShmHandler for Dock {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(Dock);
delegate_output!(Dock);
delegate_shm!(Dock);
delegate_seat!(Dock);
delegate_pointer!(Dock);
delegate_layer!(Dock);
delegate_registry!(Dock);

impl ProvidesRegistryState for Dock {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}
