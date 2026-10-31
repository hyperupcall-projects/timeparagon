//! The dock: a thin always-visible strip of today, pinned to a screen edge.
//!
//! The strip is a `zwlr_layer_shell_v1` surface, which is what lets it hug the
//! edge with no left margin, stay above other windows, and be exactly as wide
//! as asked. It shows only the first column of the day view (the highest
//! priority thing at each moment) as coloured spans on the day view's own
//! background, with a red line for "right now". Hovering a span opens a card
//! beside the strip, level with the span.
//!
//! A small button at the start of the strip brings back the window the dock
//! replaced (see [`launch`]). The plus at the opposite end opens the shared
//! task editor beside the strip, saving to Inbox.
//!
//! The surface is wider than the strip, to leave room for that card. Only the
//! strip accepts input; hover cards leave the rest of the surface click-through.
//!
//! Layer shell is supported by KDE Plasma, Sway, Hyprland, niri and COSMIC. It
//! is not available on GNOME.

pub mod animation;
pub mod config;
pub mod launch;
pub mod model;
pub mod render;

use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use chrono::{Local, NaiveDate};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopHandle, channel};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{
    wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface,
};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::keyboard::{
    KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers,
};
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_keyboard, delegate_layer, delegate_output, delegate_pointer,
    delegate_registry, delegate_seat, delegate_shm, registry_handlers,
};

use crate::calendar::{self, Snapshot, Store};
use crate::config::{Config, DockSide};
use crate::ipc;
use crate::theme::Catalog;

use render::{Layout, Palette, PixelRect, Renderer, Scene};
use tiny_skia::Pixmap;

/// Linux evdev code of the left mouse button, which is what Wayland reports.
const BTN_LEFT: u32 = 0x110;

/// How often the fade-in is redrawn, about 60 frames a second.
const FADE_STEP: Duration = Duration::from_millis(16);

/// How often the "now" line moves and the day rolls over.
const TICK: Duration = Duration::from_secs(30);

/// Runs the dock until its surface is closed or it is told to hide.
pub fn run() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("timeslay=info"))
        .init();

    anyhow::ensure!(!ipc::is_visible(ipc::DOCK), "the dock is already running");

    let app_config = Config::load();
    let dock_config = config::DockConfig::load(&app_config.views);
    log::info!(
        "dock starting (side={:?}, width={}px)",
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
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Top,
        Some(dock_config.animation.namespace("timeslay-dock")),
        None,
    );
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    configure_layer(&layer, &dock_config);

    let theme = Catalog::load()
        .resolve(&app_config.appearance.theme)
        .theme
        .clone();

    let day_start_hour = app_config.calendar.day_start_hour();
    let mut store = Store::new(day_start_hour);
    store.apply_task_colors(&app_config.calendar);
    store.cache_days_in(crate::config::cache_days_dir());
    let today = calendar::day_at(Local::now(), day_start_hour);
    store.set_day(today, today);

    let mut event_loop: EventLoop<Dock> = EventLoop::try_new()?;
    let handle = event_loop.handle();

    // Settings can be edited in the regular app while the dock is visible.
    // Watch the directory because saves atomically replace config.toml.
    let (config_tx, config_rx) = channel::channel::<Config>();
    let _config_watcher = watch_config(config_tx);
    handle
        .insert_source(config_rx, |event, _, dock| {
            if let channel::Event::Msg(config) = event {
                let updated = dock.config.clone().with_views(&config.views);
                if updated != dock.config {
                    dock.config = updated;
                    configure_layer(&dock.layer, &dock.config);
                    dock.regions_for = None;
                    dock.draw();
                }
                dock.app_config.views = config.views;
                if dock.app_config.appearance.time_format != config.appearance.time_format {
                    dock.app_config.appearance.time_format = config.appearance.time_format;
                    dock.refresh_segments();
                    dock.draw();
                }
            }
        })
        .map_err(|error| anyhow!("could not watch dock reservation settings: {error}"))?;

    // Tasks added over IPC appear without waiting for the next sync.
    let (created_tx, created_rx) = channel::channel::<Result<calendar::Event, String>>();

    // Listening only now means the shortcut daemon can take an answer to mean
    // the strip is really going to appear.
    let (hide_tx, hide_rx) = channel::channel::<()>();
    spawn_ipc(hide_tx, created_tx.clone())?;
    handle
        .insert_source(hide_rx, |event, _, dock| {
            if let channel::Event::Msg(()) = event {
                dock.request_exit();
            }
        })
        .map_err(|error| anyhow!("could not listen for the hide command: {error}"))?;

    let (snapshots_tx, snapshots_rx) = channel::channel::<Result<Snapshot, String>>();
    let wake = store
        .is_signed_in()
        .then(|| spawn_sync(&app_config, snapshots_tx));

    handle
        .insert_source(created_rx, |event, _, dock| {
            if let channel::Event::Msg(result) = event {
                match result {
                    Ok(event) => {
                        dock.store.add_event(event);
                        if let Some(wake) = &dock.wake {
                            let _ = wake.send(());
                        }
                    }
                    Err(error) => log::warn!("could not create task: {error}"),
                }
                dock.draw();
            }
        })
        .map_err(|error| anyhow!("could not listen for new tasks: {error}"))?;

    let pool = SlotPool::new(4096, &shm).context("could not allocate shared memory")?;
    let opening = animation::Opening::new(dock_config.animation, dock_config.fade_duration_ms);
    let mut dock = Dock {
        loop_handle: handle.clone(),
        registry_state: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        output_state: OutputState::new(&globals, &qh),
        shm,
        compositor,
        layer,
        pool,
        pointer: None,
        keyboard: None,
        add_hovered: false,
        task_editor: crate::panel::layer::spawn_task_popover(created_tx.clone())?,
        config: dock_config,
        app_config,
        palette: Palette::from_theme(&theme),
        store,
        segments: Vec::new(),
        day: today,
        hovered: None,
        pinned: None,
        opening,
        fading: false,
        button_hovered: false,
        pointer_position: None,
        size: None,
        scale: 1,
        renderer: Renderer::with_warmup(render::warm_up_text()),
        pixmap: None,
        frames: Vec::new(),
        regions_for: None,
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

    // The socket file is left behind on purpose: a dock started straight
    // after this one was hidden has already replaced it, and an unanswered
    // socket reads as "not running".
    while !dock.exit {
        // The fade has no frame callback to pace it, so the loop wakes itself.
        let timeout = dock.fading.then_some(FADE_STEP);
        event_loop.dispatch(timeout, &mut dock)?;
        if dock.fading {
            dock.draw();
        }
        if dock.opening.is_closed() {
            dock.exit = true;
        }
    }

    Ok(())
}

fn configure_layer(layer: &LayerSurface, config: &config::DockConfig) {
    let (anchor, margins) = match config.side {
        DockSide::Left => (
            Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM,
            (
                config.screen_margin as i32,
                0,
                config.screen_margin as i32,
                config.left_margin as i32,
            ),
        ),
        DockSide::Right => (
            Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM,
            (
                config.screen_margin as i32,
                config.right_margin as i32,
                config.screen_margin as i32,
                0,
            ),
        ),
        DockSide::Top => (
            Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            (0, config.right_margin as i32, 0, config.left_margin as i32),
        ),
        DockSide::Bottom => (
            Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            (0, config.right_margin as i32, 0, config.left_margin as i32),
        ),
    };
    layer.set_anchor(anchor);
    layer.set_margin(margins.0, margins.1, margins.2, margins.3);
    layer.set_exclusive_zone(config.exclusive_zone());
    // A zero asks the compositor to stretch between the anchors along that
    // axis, so only the strip's fixed size is set.
    if config.side.is_vertical() {
        layer.set_size(config.surface_width(), 0);
    } else {
        layer.set_size(0, config.surface_height());
    }
    layer.commit();
}

fn watch_config(sender: channel::Sender<Config>) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher;

    let path = crate::config::config_path()?;
    let parent = path.parent()?.to_path_buf();
    let watched_path = path.clone();
    let result = (|| {
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event
                    && event.paths.contains(&watched_path)
                    && matches!(
                        event.kind,
                        notify::EventKind::Create(_) | notify::EventKind::Modify(_)
                    )
                {
                    if let Ok(contents) = std::fs::read_to_string(&watched_path)
                        && let Ok(config) = toml::from_str::<Config>(&contents)
                    {
                        let _ = sender.send(config);
                    }
                }
            })?;
        watcher.watch(&parent, notify::RecursiveMode::NonRecursive)?;
        Ok::<_, notify::Error>(watcher)
    })();
    match result {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            log::warn!("live dock reservation settings unavailable: {error}");
            None
        }
    }
}

/// Answers the shortcut daemon (`status`, `hide`) and the browser extension or
/// Slack app (a JSON inbox request).
fn spawn_ipc(
    hide: channel::Sender<()>,
    created: channel::Sender<Result<calendar::Event, String>>,
) -> Result<()> {
    let listener = ipc::bind(ipc::DOCK)?;
    let closing = std::sync::atomic::AtomicBool::new(false);

    std::thread::Builder::new()
        .name("dock-ipc".into())
        .spawn(move || {
            let result = ipc::serve(listener, |command| {
                if command.starts_with('{') {
                    let handled = crate::webextension::handle_ipc(command);
                    if let Some(event) = handled.event {
                        let _ = created.send(Ok(event));
                    }
                    return handled.reply;
                }
                match command {
                    "toggle-reservation" => {
                        let mut config = Config::load();
                        config.views.dock_reserve_space = !config.views.dock_reserve_space;
                        match config.save() {
                            Ok(()) => ipc::OK.to_owned(),
                            Err(error) => {
                                log::error!("could not save dock reservation: {error}");
                                "error".to_owned()
                            }
                        }
                    }
                    "hide" => {
                        closing.store(true, std::sync::atomic::Ordering::Relaxed);
                        let _ = hide.send(());
                        ipc::OK.to_owned()
                    }
                    "status" if closing.load(std::sync::atomic::Ordering::Relaxed) => {
                        ipc::HIDDEN.to_owned()
                    }
                    "status" => ipc::VISIBLE.to_owned(),
                    other => {
                        log::warn!("unknown dock command: {other:?}");
                        String::new()
                    }
                }
            });
            if let Err(error) = result {
                log::error!("dock IPC listener stopped: {error:#}");
            }
        })
        .context("failed to spawn the dock IPC listener")?;
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
                    calendar.app_calendar.as_deref(),
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

/// A shared-memory buffer, and the parts of the pixmap it has yet to be given.
struct Frame {
    buffer: Buffer,
    stale: Vec<PixelRect>,
}

/// How many pending patches a frame keeps before it just takes everything.
const MAX_STALE_PATCHES: usize = 16;

struct Dock {
    loop_handle: LoopHandle<'static, Dock>,
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    compositor: CompositorState,
    layer: LayerSurface,
    pool: SlotPool,
    pointer: Option<wl_pointer::WlPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    add_hovered: bool,
    task_editor: crate::panel::layer::TaskPopover,

    config: config::DockConfig,
    app_config: Config,
    palette: Palette,
    store: Store,
    segments: Vec<model::Segment>,
    day: NaiveDate,

    hovered: Option<usize>,
    /// The segment whose full card a click left open.
    pinned: Option<usize>,
    opening: animation::Opening,
    /// Whether the fade-in is still running, so the loop keeps drawing.
    fading: bool,
    button_hovered: bool,
    pointer_position: Option<(f64, f64)>,
    /// The configured logical size, once the compositor has sent one.
    size: Option<(u32, u32)>,
    scale: i32,
    renderer: Renderer,
    /// Kept between frames, so each one only repaints what changed.
    pixmap: Option<Pixmap>,
    /// Shared-memory buffers the pixmap is copied into. The compositor holds
    /// one while the next is filled, so there are normally two.
    frames: Vec<Frame>,
    /// The layout the input and opaque regions were last set for.
    regions_for: Option<Layout>,
    wake: Option<mpsc::Sender<()>>,
    exit: bool,
}

impl Dock {
    fn layout(&self) -> Option<Layout> {
        let (width, height) = self.size?;
        Some(Layout::new(&self.config, width, height, self.scale as u32))
    }

    fn refresh_segments(&mut self) {
        // A pinned card follows its segment, or goes if the segment is gone.
        let pinned = self
            .pinned
            .and_then(|index| self.segments.get(index))
            .map(|segment| {
                (
                    segment.start_min,
                    segment.end_min,
                    segment.card.title.clone(),
                )
            });
        self.segments = model::segments(
            &self.store,
            &self.app_config.calendar,
            self.day,
            self.app_config.appearance.time_format,
        );
        self.pinned = pinned.and_then(|(start, end, title)| {
            self.segments.iter().position(|segment| {
                segment.start_min == start && segment.end_min == end && segment.card.title == title
            })
        });
        self.hovered = match (self.layout(), self.pointer_position) {
            (Some(layout), Some((x, y))) => layout.hit(&self.segments, x, y),
            _ => None,
        };
    }

    /// Quits, fading out first when the dock fades in.
    fn request_exit(&mut self) {
        if self.layout().is_none() || !self.opening.begin_close(Instant::now()) {
            self.exit = true;
            return;
        }
        self.fading = true;
    }

    /// Shows the side panel, then quits.
    fn go_back(&mut self) {
        match launch::close_dock() {
            Ok(()) => self.request_exit(),
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
        let resized =
            !matches!(&self.pixmap, Some(p) if p.width() == width && p.height() == height);
        if resized {
            self.pixmap = Pixmap::new(width, height);
        }
        let Some(pixmap) = self.pixmap.as_mut() else {
            return;
        };

        let now = model::now_minute(Local::now(), self.day, &self.app_config.calendar);
        let scene = Scene {
            segments: &self.segments,
            now_minute: now,
            hovered: self.hovered,
            pinned: self.pinned,
            day_offset_min: self.app_config.calendar.day_end_minutes(),
            hour_lines: self.config.hour_lines,
            button_hovered: self.button_hovered,
            add_hovered: self.add_hovered,
            back_label: "Open the side panel",
            side: self.config.side,
            palette: self.palette,
        };
        let damage = self.renderer.paint(&layout, &scene, pixmap);

        let whole = PixelRect {
            x: 0,
            y: 0,
            width: width as i32,
            height: height as i32,
        };
        if resized {
            self.frames.clear();
        }
        for frame in &mut self.frames {
            frame.stale.extend_from_slice(&damage);
            if frame.stale.len() > MAX_STALE_PATCHES {
                frame.stale = vec![whole];
            }
        }

        let free = self
            .frames
            .iter()
            .position(|frame| frame.buffer.canvas(&mut self.pool).is_some());
        let index = match free {
            Some(index) => index,
            None => {
                let Ok((buffer, _)) = self.pool.create_buffer(
                    width as i32,
                    height as i32,
                    width as i32 * 4,
                    wl_shm::Format::Argb8888,
                ) else {
                    log::warn!("could not allocate a frame");
                    return;
                };
                self.frames.push(Frame {
                    buffer,
                    stale: vec![whole],
                });
                self.frames.len() - 1
            }
        };

        let (opacity, fading) = self.opening.sample(Instant::now());
        let was_fading = std::mem::replace(&mut self.fading, fading);

        let frame = &mut self.frames[index];
        let Some(canvas) = frame.buffer.canvas(&mut self.pool) else {
            return;
        };
        if fading {
            // A faded buffer can't be patched later, so it is rewritten whole
            // now and again on its next use.
            render::copy_patch(pixmap, canvas, whole);
            animation::apply_opacity(canvas, opacity);
            frame.stale = vec![whole];
        } else {
            for patch in frame.stale.drain(..) {
                render::copy_patch(pixmap, canvas, patch);
            }
        }
        let buffer = &frame.buffer;

        let surface = self.layer.wl_surface();
        surface.set_buffer_scale(self.scale);
        if resized || fading || was_fading {
            surface.damage_buffer(0, 0, whole.width, whole.height);
        } else {
            for patch in &damage {
                surface.damage_buffer(patch.x, patch.y, patch.width, patch.height);
            }
        }

        // The task card accepts input only while it is open.
        if self.regions_for != Some(layout) {
            let (x, y, w, h) = layout.strip_rect();
            if let Ok(region) = Region::new(&self.compositor) {
                region.add(x as i32, y as i32, w as i32, h as i32);
                self.layer.set_input_region(Some(region.wl_region()));
                if let Ok(opaque) = Region::new(&self.compositor) {
                    opaque.add(x as i32, y as i32, w as i32, h as i32);
                    self.layer.set_opaque_region(if layout.border_radius > 0.0 {
                        None
                    } else {
                        Some(opaque.wl_region())
                    });
                }
                self.regions_for = Some(layout);
            }
        }

        if buffer.attach_to(surface).is_err() {
            return;
        }
        self.layer.commit();
    }

    fn open_task_editor(&mut self) {
        self.task_editor.toggle();
    }

    /// Follows the pointer, and says whether that changed what is drawn.
    fn pointer_moved(&mut self, position: Option<(f64, f64)>) -> bool {
        self.pointer_position = position;
        let (hovered, button_hovered, add_hovered) = match (self.layout(), position) {
            (Some(layout), Some((x, y))) => (
                layout.hit(&self.segments, x, y),
                layout.hit_button(x, y),
                layout.hit_add(x, y),
            ),
            _ => (None, false, false),
        };
        let changed = hovered != self.hovered
            || button_hovered != self.button_hovered
            || add_hovered != self.add_hovered;
        self.hovered = hovered;
        self.button_hovered = button_hovered;
        self.add_hovered = add_hovered;
        changed
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
        let vertical = self.config.side.is_vertical();
        let width = if width == 0 {
            if vertical {
                self.config.surface_width()
            } else {
                return;
            }
        } else {
            width
        };
        let height = if height == 0 {
            if vertical {
                return;
            }
            self.config.surface_height()
        } else {
            height
        };

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
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            match self.seat_state.get_keyboard_with_repeat(
                qh,
                &seat,
                None,
                self.loop_handle.clone(),
                Box::new(|_, _, _| {}),
            ) {
                Ok(keyboard) => self.keyboard = Some(keyboard),
                Err(error) => log::warn!("could not use the keyboard: {error}"),
            }
        }
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
        if capability == Capability::Keyboard
            && let Some(keyboard) = self.keyboard.take()
        {
            keyboard.release();
        }
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
        // Motion arrives in bursts; draw once for the whole burst.
        let mut changed = false;
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    changed |= self.pointer_moved(Some(event.position));
                }
                PointerEventKind::Leave { .. } => changed |= self.pointer_moved(None),
                PointerEventKind::Press { button, .. }
                    if button == BTN_LEFT && self.button_hovered =>
                {
                    self.go_back();
                }
                PointerEventKind::Press { button, .. } if button == BTN_LEFT => {
                    if self.add_hovered {
                        self.open_task_editor();
                        changed = true;
                    } else if let Some(hovered) = self.hovered {
                        self.pinned = (self.pinned != Some(hovered)).then_some(hovered);
                        changed = true;
                    }
                }
                _ => {}
            }
        }
        if changed {
            self.draw();
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
delegate_keyboard!(Dock);
delegate_layer!(Dock);
delegate_registry!(Dock);

impl ProvidesRegistryState for Dock {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

impl KeyboardHandler for Dock {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: &wl_surface::WlSurface,
        _: u32,
    ) {
    }
    fn press_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        let _ = event;
    }
    fn repeat_key(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        keyboard: &wl_keyboard::WlKeyboard,
        serial: u32,
        event: KeyEvent,
    ) {
        self.press_key(conn, qh, keyboard, serial, event);
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_keyboard::WlKeyboard,
        _: u32,
        modifiers: Modifiers,
        _: RawModifiers,
        _: u32,
    ) {
        let _ = modifiers;
    }
}
