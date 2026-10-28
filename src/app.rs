//! State, messages and the update loop.

use std::io::Read;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result as AnyhowResult};

use iced::keyboard::{self, Key};
use iced::widget::text_editor;
use iced::{Element, Point, Size, Subscription, Task, Theme, window};

use chrono::{DateTime, Local};

use crate::audio::{self, Audio, Pcm};
use crate::calendar::{self, Connection, SyncState};
use crate::config::{self, Config};
use crate::theme::Catalog;
use crate::{alarm, alarms, blocking, notify, panel, theme, timeblocking, ui};

/// The smallest window we are willing to be asked to draw.
///
/// The compact layout is built for 200px; this leaves a little room below
/// that rather than letting a tiling compositor squeeze it into nonsense.
pub const MIN_SIZE: Size = Size::new(180.0, 240.0);

/// How often the flash overlay redraws while it is animating.
///
/// This is a timer rather than `window::frames` because iced only redraws
/// when something changes, so an animation has to keep producing messages to
/// stay in motion. It only runs for the second or two the flash is up.
const FLASH_FRAME: Duration = Duration::from_millis(33);

/// How often the settings pane refreshes the audio status note, so that
/// "PipeWire unavailable" appears without needing a mouse wiggle.
const STATUS_POLL: Duration = Duration::from_millis(500);

/// How often the current-time needle is redrawn on the calendar.
const NEEDLE_TICK: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Calendar,
    /// Tasks that have not been triaged yet.
    Inbox,
    Backlog,
    Projects,
    /// The places events happen at, and the trips between them.
    Travel,
    /// The notepad: a page of free text kept between sessions.
    Notepad,
    KeepNotes,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutField {
    CalendarWidth,
    TimeBlockWidth,
    DetailWidth,
    ScreenMargin,
    BorderRadius,
    DockWidth,
}

impl LayoutField {
    pub fn value(self, state: &State) -> f32 {
        match self {
            Self::CalendarWidth => state.calendar_width(),
            Self::TimeBlockWidth => state.config.views.time_block_width(),
            Self::DetailWidth => state.config.views.detail_width(),
            Self::ScreenMargin => state.config.views.margin(),
            Self::BorderRadius => state.config.views.border_radius(),
            Self::DockWidth => state.config.views.dock_width(),
        }
    }

    pub fn bounds(self) -> (f32, f32) {
        match self {
            Self::ScreenMargin => (0.0, config::ViewSettings::MAX_MARGIN),
            Self::BorderRadius => (0.0, config::ViewSettings::MAX_BORDER_RADIUS),
            Self::DockWidth => (
                config::ViewSettings::MIN_DOCK_WIDTH,
                config::ViewSettings::MAX_DOCK_WIDTH,
            ),
            _ => (
                config::ViewSettings::MIN_WIDTH,
                config::ViewSettings::MAX_WIDTH,
            ),
        }
    }

    pub fn changed(self, value: f32) -> Message {
        match self {
            Self::CalendarWidth => Message::CalendarWidthChanged(value),
            Self::TimeBlockWidth => Message::TimeBlockWidthChanged(value),
            Self::DetailWidth => Message::DetailWidthChanged(value),
            Self::ScreenMargin => Message::ScreenMarginChanged(value),
            Self::BorderRadius => Message::BorderRadiusChanged(value),
            Self::DockWidth => Message::DockWidthChanged(value),
        }
    }
}

/// A flash overlay that is currently on screen.
pub struct Flash {
    pub window: window::Id,
    pub started: Instant,
    pub settings: config::Flash,
}

/// Set when this process is the side panel rather than the main window.
pub struct Sidebar {
    /// Width the panel is pinned to, in logical pixels.
    pub width: f32,
    /// Which edge it is anchored to.
    pub side: panel::Side,
}

/// A command sent from the panel daemon to the panel process over its IPC
/// socket, telling it to show or hide its window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelCommand {
    Show,
    Hide,
}

impl Flash {
    /// How opaque the overlay should be right now, or `None` once it is done.
    ///
    /// Each pulse is a full raised-cosine fade in and out, so the flash never
    /// snaps on or off.
    pub fn alpha(&self, now: Instant) -> Option<f32> {
        let elapsed = now.saturating_duration_since(self.started);

        if elapsed >= self.settings.total_duration() {
            return None;
        }

        let period = self.settings.pulse_period().as_secs_f32();
        let phase = (elapsed.as_secs_f32() % period) / period;
        let envelope = 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos();

        Some(envelope * self.settings.peak())
    }
}

pub struct State {
    pub config: Config,
    pub catalog: Catalog,

    /// The clock reading used for this frame, so every widget agrees.
    pub now: Instant,

    /// Set when this process is the side panel rather than the main window.
    /// It pins the window to its configured width and edge, and keeps the panel
    /// from writing its own geometry back over the user's saved window.
    pub sidebar: Option<Sidebar>,

    /// The alarms set on the calendar, and the state of setting them.
    pub alarms: alarms::Alarms,

    pub screen: Screen,
    pub weekly: bool,
    pub week_anchor: chrono::NaiveDate,
    pub week_hover: Option<chrono::NaiveDate>,
    pub week_scroll_x: f32,
    /// What the Projects tab is in the middle of.
    pub projects: crate::projects::Workspace,
    /// Places, place searches and trips: the Travel tab and travel on the day view.
    pub travel: crate::travel::Workspace,
    pub main_window: Option<window::Id>,
    pub main_size: Size,
    /// Logical size of the monitor, when the compositor reports it.
    pub monitor_size: Option<Size>,
    weekly_resize_pending: bool,
    /// Suppress queued weekly sizes until the restored width arrives or settles.
    weekly_width_restoring: Option<(f32, Instant)>,
    /// Receiver for show/hide commands from the panel daemon, when this process
    /// is the side panel.
    pub panel_command_rx: Option<mpsc::Receiver<PanelCommand>>,
    /// The panel config, kept so the window can be reopened after being closed.
    pub panel_config: Option<crate::panel::PanelConfig>,
    /// Set when the panel window is closed programmatically (to hide), so that
    /// `WindowClosed` does not exit the process.
    pub panel_expected_close: bool,
    pub flash: Option<Flash>,
    /// True while a sound or flash is still asking to be dismissed.
    pub alarm_active: bool,

    pub audio: Audio,
    /// The last thing that went wrong with a sound, shown in settings.
    pub audio_error: Option<String>,

    pub panel_shortcut_input: String,
    pub panel_shortcut_busy: bool,
    pub panel_shortcut_result: Option<Result<String, String>>,
    /// Keep empty or incomplete numbers editable until they are submitted.
    pub view_layout_input: Option<(LayoutField, String)>,
    /// Whether the window has been widened for the event details pane. Kept so
    /// the pane can be opened and closed from anywhere without resizing twice.
    pub details_pane_open: bool,

    /// Set while a config write is already queued, so a slider drag does not
    /// schedule one save per pixel.
    save_queued: bool,
    /// Last disk snapshot, including our own writes, to ignore save echoes.
    disk_config: Config,

    /// The notepad's text as the editor widget holds it.
    ///
    /// The widget keeps its own copy so the cursor and the selection survive
    /// a redraw; the config keeps the saved copy, and the two are reconciled
    /// on every edit.
    pub notepad: text_editor::Content<iced::Renderer>,

    pub calendar: calendar::Store,
    pub timeblocking: crate::timeblocking::Workspace,
    pub keep: crate::keep::Store,
    pub inbox: crate::inbox::Inbox,
    /// Google accounts connected for the Gmail integration.
    pub accounts: crate::integrations::accounts::Accounts,
}

#[derive(Debug, Clone)]
pub enum Message {
    TimeBlocking(crate::timeblocking::Action),
    Inbox(crate::inbox::Action),
    Accounts(crate::integrations::accounts::Action),
    FlashFrame(Instant),

    /// The notepad was edited.
    NotepadEdited(text_editor::Action),
    /// The text on the notepad was made bigger or smaller, by a number of
    /// percentage points.
    NotepadFontSizeChanged(i32),
    Alarms(alarms::Action),

    ScreenSelected(Screen),
    CalendarModeSelected(bool),
    WeekDayAction(chrono::NaiveDate, Box<Message>),
    WeekHover(Option<chrono::NaiveDate>),
    WeekScrolled(f32),
    Projects(crate::projects::Action),
    Travel(crate::travel::Action),
    KeepRefresh,
    KeepSignin,
    KeepLoaded(Result<crate::keep::Snapshot, String>),
    KeepSignedIn(Result<(), String>),
    KeepQueryChanged(String),
    KeepLabelChanged(Option<String>),
    KeepScopeChanged(crate::keep::Scope),
    KeepEdit(String),
    Dismiss,

    ThemeSelected(String),
    FollowSystemToggled(bool),
    AlwaysOnTopToggled(bool),
    CalendarWidthChanged(f32),
    TimeBlockWidthChanged(f32),
    DetailWidthChanged(f32),
    ScreenMarginChanged(f32),
    DockWidthChanged(f32),
    BorderRadiusChanged(f32),
    ViewLayoutInputChanged(LayoutField, String),
    ViewLayoutInputSubmitted(LayoutField),
    PanelShortcutChanged(String),
    PanelShortcutApply,
    PanelShortcutApplied(Result<String, String>),

    /// A sound finished rendering and is ready to hand to PipeWire.
    SoundRendered(Result<Pcm, String>, PlaySpec),
    NotificationPosted(Result<(), String>),
    ConfigSaved(Result<(), String>),
    SaveRequested,
    ConfigReloaded(Config),
    AudioStatusPolled,

    WindowOpened(window::Id),
    /// The monitor size came back, so the window can open at its default width.
    MonitorSized(window::Id, Option<Size>),
    WeekMonitorSized(window::Id, Option<Size>),
    WindowClosed(window::Id),
    WindowResized(window::Id, Size),
    FlashFinished,
    /// A poll tick that checks for show/hide commands from the panel daemon.
    PanelCommandPolled,

    KeyPressed(Key, keyboard::Modifiers),
    /// An event we deliberately do not act on, such as a key release or a
    /// press that only needs to be swallowed.
    Ignored,

    CalendarMenuToggled,
    CalendarToggled(String, bool),
    /// Move a calendar-view source one position up (left) or down (right).
    CalendarSourceMoved(String, bool),
    CalendarSyncRequested,
    /// Drop cached days and pull the day in view from Google again.
    CalendarResyncRequested,
    CalendarSynced(Result<calendar::Snapshot, String>),
    CalendarSyncedAt(u64, Result<calendar::Snapshot, String>),
    CalendarWeekSynced(
        u64,
        chrono::NaiveDate,
        u32,
        Result<calendar::Snapshot, String>,
    ),
    CalendarTick,
    OpenGoogleCalendar,
    /// Swap this window for the dock strip on the screen edge.
    DockRequested,
    EventOpened(String),
    EventClosed,
    OpenEventLink(String),
    /// The user dragged an event's time on the day grid. The new times are
    /// applied to the view at once and written back to Google.
    EventTimeChanged {
        id: String,
        start: DateTime<Local>,
        end: DateTime<Local>,
    },
    /// Google's answer to a dragged time change.
    EventTimeSaved(String, Result<(), String>),
    /// The user clicked Delete on an event's details card. The confirmation
    /// prompt opens before anything is sent to Google.
    EventDeleteRequested(String),
    /// The user confirmed the deletion prompt.
    EventDeleteConfirmed(String),
    /// The user dismissed the deletion prompt without deleting.
    EventDeleteCancelled,
    /// Google's answer to a single-event deletion.
    EventDeleted(String, Result<(), String>),
    /// Tick a Google Task off, or put it back.
    TaskToggled(String),
    /// Ignore a task, or restore an ignored task to active.
    TaskIgnoreToggled(String),
    TaskCompletionSaved(
        String,
        timeblocking::TaskStatus,
        Result<calendar::Event, String>,
    ),
    /// Show or hide Google Tasks in the day view.
    ShowTasksToggled(bool),
    /// Open or close the color picker for a Google Tasks list.
    TaskColorEditorToggled(String),
    /// Give a Google Tasks list a custom color, or `None` to reset it.
    TaskColorPicked(String, Option<iced::Color>),
    /// Delete a single Google Task instance (one occurrence of a recurring task).
    TaskDeleteRequested(String),
    /// The user confirmed the task deletion prompt.
    TaskDeleteConfirmed(String),
    /// Google's answer to a single-task deletion.
    TaskDeleted(String, Result<(), String>),
    GoogleClientIdChanged(String),
    GoogleClientSecretChanged(String),
    GoogleConnectRequested,
    GoogleConnected(Result<calendar::oauth::Tokens, String>),
    GoogleDisconnectRequested,
    CalendarSyncMinutesChanged(u32),
    /// The hour the calendar day ends, so late-night events stay with the
    /// evening they belong to.
    CalendarDayEndChanged(u32),
    CalendarWeeklyDaysChanged(u32),
    CalendarWeeklyWidthChanged(u32),
    /// Multiplies the calendar hour height. Above 1 spreads the hours apart.
    ///
    /// `anchor` is the pixel row under the pointer, measured from the top of
    /// the day grid, so the zoom can keep that row still under the mouse.
    CalendarZoomed {
        factor: f32,
        anchor: f32,
    },
    /// Move the day in view by a number of days, positive or negative.
    CalendarDayShift(i64),
    /// Jump the day in view to a specific date, from the month picker.
    CalendarDaySelected(chrono::NaiveDate),
    /// Open or close the month picker.
    CalendarMonthToggled,
    /// Opens the pending-tasks popup for a day, or closes it.
    PendingTasksShown(Option<chrono::NaiveDate>),
    /// Move the month picker by a number of months, positive or negative.
    CalendarMonthShift(i32),

    /// A click or drag on empty grid space asked for a new event. The popup
    /// opens on the default calendar, spanning these times.
    EventDraftStarted {
        start: DateTime<Local>,
        end: DateTime<Local>,
    },
    /// The new event's title was edited.
    EventDraftTitleChanged(String),
    /// The new event's description was edited.
    EventDraftDescriptionChanged(String),
    /// The start field was edited, as `HH:MM`.
    EventDraftStartChanged(String),
    /// The end field was edited, as `HH:MM`.
    EventDraftEndChanged(String),
    /// The new event was moved to another calendar.
    EventDraftCalendarSelected(String),
    /// The calendar dropdown in the add-event popup was opened or closed.
    EventDraftCalendarDropdownToggled,
    /// The add-event popup was dismissed without saving.
    EventDraftCancelled,
    /// Write the new event to Google.
    EventDraftSaveRequested,
    /// Google's answer to creating the event, carrying the chip to draw.
    EventDraftSaved(Result<calendar::Event, String>),

    /// The user asked to edit an existing event, from the details card.
    EventEditRequested(String),
    /// The edited event's title was changed.
    EventEditTitleChanged(String),
    /// The edited event's description was changed.
    EventEditDescriptionChanged(String),
    /// The edited event's start field was edited, as `HH:MM`.
    EventEditStartChanged(String),
    /// The edited event's end field was edited, as `HH:MM`.
    EventEditEndChanged(String),
    /// The edited event was moved to another calendar.
    EventEditCalendarSelected(String),
    /// The edit-event popup was dismissed without saving.
    EventEditCancelled,
    /// Write the edited event to Google.
    EventEditSaveRequested,
    /// Google's answer to updating the event.
    EventEditSaved(Result<(), String>),

    /// The calendar new events are created on was changed.
    DefaultCalendarSelected(String),
}

/// The parts of a play request that survive the trip through rendering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaySpec {
    pub gain: f32,
    pub repeats: u8,
    pub fade_in: Duration,
}

impl State {
    /// Builds a state from settings that are already in hand.
    ///
    /// Separate from [`boot`] so tests can assemble a state without reading
    /// or writing anything in the user's config directory.
    pub fn new(config: Config, catalog: Catalog) -> Self {
        let day_end_hour = config.calendar.day_start_hour();

        // The editor is seeded with the saved text, so the notepad opens on
        // what was written last time.
        let notepad = text_editor::Content::with_text(&config.notepad.text);

        Self {
            now: Instant::now(),
            sidebar: None,
            alarms: alarms::Alarms::default(),
            screen: Screen::Calendar,
            weekly: false,
            week_anchor: calendar::day_at(Local::now(), day_end_hour),
            week_hover: None,
            week_scroll_x: 0.0,
            projects: Default::default(),
            travel: Default::default(),
            main_window: None,
            main_size: Size::new(
                config.views.calendar_width(config.window.width),
                config.window.height,
            ),
            monitor_size: None,
            weekly_resize_pending: false,
            weekly_width_restoring: None,
            panel_command_rx: None,
            panel_config: None,
            panel_expected_close: false,
            flash: None,
            alarm_active: false,
            audio: Audio::new(),
            audio_error: None,
            panel_shortcut_input: String::new(),
            panel_shortcut_busy: false,
            panel_shortcut_result: None,
            view_layout_input: None,
            details_pane_open: false,
            save_queued: false,
            disk_config: config.clone(),
            catalog,
            config,
            notepad,
            keep: crate::keep::Store::default(),
            calendar: calendar::Store::new(day_end_hour),
            timeblocking: crate::timeblocking::Workspace::default(),
            inbox: crate::inbox::Inbox::default(),
            accounts: crate::integrations::accounts::Accounts::load(),
        }
    }

    /// Replaces the notepad's text, as if it had been typed.
    ///
    /// The editor holds its own copy of the text so that the cursor survives a
    /// redraw, so this puts the same text in both places.
    pub fn set_notepad(&mut self, text: impl Into<String>) {
        let text = text.into();

        self.notepad = text_editor::Content::with_text(&text);
        self.config.notepad.text = text;
    }

    pub fn calendar_width(&self) -> f32 {
        self.config.views.calendar_width(
            self.sidebar
                .as_ref()
                .map_or(self.config.window.width, |panel| panel.width),
        )
    }

    pub fn screen_margin(&self) -> f32 {
        if self.sidebar.is_some() {
            self.config.views.margin()
        } else {
            0.0
        }
    }

    fn weekly_active(&self) -> bool {
        self.screen == Screen::Calendar && self.weekly
    }

    fn weekly_window_width(&self) -> f32 {
        let width = (ui::calendar::weekly_view_width(&self.config.calendar) + self.screen_margin())
            .max(self.calendar_width() + self.screen_margin());
        match self.monitor_size {
            Some(size) if size.width.is_finite() && size.width > 0.0 => width.min(size.width),
            _ => width,
        }
    }
}

/// Resize the panes together, keeping the panel's transparent screen inset
/// outside the calendar's configured width.
fn apply_view_sizes(state: &mut State) -> Task<Message> {
    let weekly = state.weekly_active();
    let base = if weekly {
        state.weekly_window_width()
    } else {
        state.calendar_width() + state.screen_margin()
    };
    let block_open = state.timeblocking.original_width.is_some();
    let mut width = if block_open {
        state.timeblocking.original_width = Some(base);
        crate::timeblocking::editor_width(state, base)
    } else {
        base
    };
    if state.details_pane_open {
        width += state.config.views.detail_width();
    }
    let widened = block_open || state.details_pane_open;
    let Some(id) = state.main_window else {
        return Task::none();
    };
    if weekly && (state.main_size.width - width).abs() > 0.5 {
        state.weekly_resize_pending = true;
    }
    let resize = window::resize(id, Size::new(width, state.main_size.height));
    if state.sidebar.is_some() {
        // Relax the previous fixed width before asking the compositor to resize.
        window::set_max_size(id, None)
            .chain(window::set_min_size(
                id,
                Some(Size::new(
                    if weekly {
                        (MIN_SIZE.width + state.screen_margin()).min(base)
                    } else {
                        base
                    },
                    MIN_SIZE.height,
                )),
            ))
            .chain(resize)
            .chain(window::set_max_size(
                id,
                (!widened).then_some(Size::new(base, 100_000.0)),
            ))
    } else {
        resize
    }
}

pub fn boot() -> (State, Task<Message>) {
    let state = boot_state();

    // A daemon opens no window by itself, which is what lets the flash
    // overlay be a peer window rather than a child of the clock.
    let (_id, open) = window::open(window::Settings {
        size: state.main_size,
        transparent: true,
        min_size: Some(MIN_SIZE),
        level: level(state.config.window.always_on_top),
        exit_on_close_request: true,
        icon: window_icon(),
        platform_specific: window_platform(),
        ..Default::default()
    });

    (state, open.map(Message::WindowOpened))
}

/// Opens the clock as the side panel: a separate, narrow instance pinned to one
/// screen edge and kept above other windows.
///
/// The width and edge come from the `[panel]` table of `config.toml`, and the width is also fixed by
/// the window's minimum and maximum so the compositor cannot stretch it. The
/// height is filled in once the monitor size is known, in [`Message::MonitorSized`].
///
/// Where the window lands is the compositor's call on Wayland, where a client
/// cannot dictate its own position; see the packaging README.
pub fn boot_sidebar() -> (State, Task<Message>) {
    let panel = panel::PanelConfig::load();

    let mut state = boot_state();
    state.sidebar = Some(Sidebar {
        width: panel.width as f32,
        side: panel.side,
    });

    state.panel_config = Some(panel.clone());

    let (tx, rx) = mpsc::channel();
    state.panel_command_rx = Some(rx);

    if let Some(socket_path) = crate::panel::panel_socket_path() {
        std::thread::Builder::new()
            .name("panel-ipc".into())
            .spawn(move || {
                if let Err(error) = panel_ipc_listener(socket_path, tx) {
                    log::error!("panel IPC listener stopped: {error:#}");
                }
            })
            .expect("failed to spawn the panel IPC listener");
    }

    state.main_size = Size::new(state.calendar_width() + state.screen_margin(), PANEL_FILL);
    let (_id, open) = window::open(sidebar_settings(&panel, &state.config.views));

    (state, open.map(Message::WindowOpened))
}

/// The height a panel asks for before the compositor clamps it.
///
/// A Wayland client cannot learn how tall the output is (iced's
/// `window::monitor_size` reports `None` here), so the panel asks for a strip
/// taller than any screen and lets the compositor fit it — KWin clamps a window
/// to the screen. Weekly also permits the compositor to clamp the width.
const PANEL_FILL: f32 = 10_000.0;

/// The window a panel instance opens with.
fn sidebar_settings(panel: &panel::PanelConfig, views: &config::ViewSettings) -> window::Settings {
    let width = views.calendar_width(panel.width as f32) + views.margin();
    let side = panel.side;

    window::Settings {
        size: Size::new(width, PANEL_FILL),
        min_size: Some(Size::new(width, MIN_SIZE.height)),
        max_size: Some(Size::new(width, 100_000.0)),
        position: match side {
            panel::Side::Left => window::Position::SpecificWith(left_edge),
            panel::Side::Right => window::Position::SpecificWith(right_edge),
        },
        decorations: false,
        // Transparent gutters provide the screen margin even on Wayland,
        // where clients cannot position their own windows.
        transparent: true,
        level: window::Level::AlwaysOnTop,
        exit_on_close_request: true,
        icon: window_icon(),
        platform_specific: window_platform(),
        ..Default::default()
    }
}

/// The top-left corner of the monitor. `SpecificWith` wants a plain `fn`, so
/// the edge is chosen with a pair of functions rather than a captured closure.
fn left_edge(_window: Size, _monitor: Size) -> Point {
    Point::new(0.0, 0.0)
}

fn right_edge(window: Size, monitor: Size) -> Point {
    Point::new(monitor.width - window.width, 0.0)
}

/// Builds the state that is read from disk at startup.
///
/// Shared by the main window and the side panel, so both open on the same
/// config, the same cached days and the same running track. It deliberately
/// opens no window: the two callers want different ones.
fn boot_state() -> State {
    let mut state = State::new(Config::load(), Catalog::load());

    // Follow the config in Google Drive's app folder. This is also not part of
    // `State::new`: a hand-built test state must not touch the network.
    crate::config_sync::start();

    // Days are kept under the XDG cache directory as well as in memory, so a
    // restart opens on what the last run saw rather than an empty grid. This
    // is deliberately not part of `State::new`: a state built by hand in a
    // test must not read or rewrite the user's cache.
    state.calendar.cache_days_in(config::cache_days_dir());
    state.keep.cache_in(config::cache_keep_path());
    state.travel.cache_in(config::cache_travel_path());

    state.alarms.keep_in(alarms::path());

    state
}

/// The desktop-file basename, `timetracker.desktop`.
///
/// On Wayland the title-bar icon is not the pixmap passed to `set_window_icon`
/// (winit ignores that). KWin looks up this id, finds the desktop entry, and
/// paints `Icon=timetracker`. An empty id leaves the generic Wayland logo, the
/// yellow W, in the corner while the dock still shows the hourglass.
const APPLICATION_ID: &str = "timetracker";

fn window_platform() -> window::settings::PlatformSpecific {
    window::settings::PlatformSpecific {
        application_id: APPLICATION_ID.to_owned(),
        ..Default::default()
    }
}

/// The window icon, embedded so it survives being installed anywhere.
///
/// A missing or unreadable icon is not worth failing a start over, so it is
/// logged and the window opens without one.
fn window_icon() -> Option<window::Icon> {
    let data = include_bytes!("../assets/noto-hourglass-done.png");

    match window::icon::from_file_data(data, None) {
        Ok(icon) => Some(icon),
        Err(error) => {
            log::warn!("could not load the window icon: {error}");
            None
        }
    }
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    // The flash animation redraws many times a second and changes nothing
    // travel reads.
    let ticking = matches!(message, Message::FlashFrame(_));
    let task = dispatch(state, message);
    let travel = if ticking {
        Task::none()
    } else {
        crate::travel::reconcile(state)
    };
    Task::batch([task, reconcile_detail_pane(state), travel])
}

/// Opens or closes the event details pane by widening or narrowing the window
/// to match, whatever code changed which event is open.
fn reconcile_detail_pane(state: &mut State) -> Task<Message> {
    let open = detail_pane_visible(state);
    if open == state.details_pane_open {
        return Task::none();
    }
    state.details_pane_open = open;
    apply_view_sizes(state)
}

fn detail_pane_visible(state: &State) -> bool {
    state.calendar.open_event.is_some() && shows_details(state.screen) && !state.weekly_active()
}

fn resize_for_week_transition(
    state: &mut State,
    was_weekly: bool,
    close: Task<Message>,
) -> Task<Message> {
    if was_weekly == state.weekly_active() {
        return close;
    }

    state.details_pane_open = detail_pane_visible(state);
    let restored_width = state.calendar_width()
        + state.screen_margin()
        + if state.details_pane_open {
            state.config.views.detail_width()
        } else {
            0.0
        };
    let restoring = was_weekly
        && state.main_window.is_some()
        && (state.weekly_resize_pending || (state.main_size.width - restored_width).abs() > 0.5);
    state.weekly_width_restoring = restoring.then_some((restored_width, Instant::now()));
    state.weekly_resize_pending = false;
    let resize = close.chain(apply_view_sizes(state));
    if state.weekly_active() {
        if let Some(id) = state.main_window {
            return resize.chain(
                window::monitor_size(id).map(move |size| Message::WeekMonitorSized(id, size)),
            );
        }
    }
    resize
}

/// The screens that show the event details pane beside their content.
pub(crate) fn shows_details(screen: Screen) -> bool {
    matches!(screen, Screen::Calendar | Screen::Backlog | Screen::Inbox)
}

fn dispatch(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::TimeBlocking(action) => crate::timeblocking::update(state, action),
        Message::Inbox(action) => crate::inbox::update(state, action),
        Message::Accounts(action) => crate::integrations::accounts::update(state, action),
        Message::FlashFrame(now) => {
            state.now = now;

            match &state.flash {
                Some(flash) if flash.alpha(now).is_none() => Task::done(Message::FlashFinished),
                _ => Task::none(),
            }
        }

        Message::NotepadEdited(action) => {
            state.notepad.perform(action);

            // The editor is the one holding the text while it is being typed,
            // so the saved copy is taken from it after every change.
            state.config.notepad.text = state.notepad.text();

            queue_save(state)
        }

        Message::NotepadFontSizeChanged(delta) => {
            let size = state.config.notepad.font_size;
            let next = size.saturating_add_signed(delta as i8).clamp(
                config::Notepad::MIN_FONT_SIZE,
                config::Notepad::MAX_FONT_SIZE,
            );

            state.config.notepad.font_size = next;

            queue_save(state)
        }

        Message::Alarms(action) => alarms::update(state, action),

        Message::ScreenSelected(screen) => {
            let was_weekly = state.weekly_active();
            let returning_to_week =
                screen == Screen::Calendar && state.screen != Screen::Calendar && state.weekly;
            state.view_layout_input = None;
            // Alarm setting belongs to the day view, so leaving it ends both.
            state.alarms.mode = false;
            state.alarms.editor = None;
            // Keep the day grid's scroll position when switching screens.
            state.calendar.menu_open = false;
            // Whatever was open belongs to the screen being left: the Inbox
            // shows its details pane only for its own tasks.
            if screen != state.screen && (screen == Screen::Inbox || state.screen == Screen::Inbox)
            {
                state.calendar.open_event = None;
                state.inbox.picker = None;
            }
            state.screen = screen;
            let close = crate::timeblocking::close(state);
            let close = resize_for_week_transition(state, was_weekly, close);
            if returning_to_week {
                Task::batch([close, sync_calendar(state)])
            } else if screen == Screen::KeepNotes {
                Task::batch([close, refresh_keep(state)])
            } else {
                close
            }
        }

        Message::CalendarModeSelected(weekly) => {
            let was_weekly = state.weekly_active();
            let changed = state.weekly != weekly || state.screen != Screen::Calendar;
            let close = crate::timeblocking::close(state);
            state.calendar.open_event = None;
            state.calendar.month_open = false;
            state.calendar.menu_open = false;
            state.week_hover = None;
            state.screen = Screen::Calendar;
            state.weekly = weekly;
            let close = resize_for_week_transition(state, was_weekly, close);
            if changed {
                state.week_anchor = state.calendar.day;
                state.week_scroll_x = 0.0;
                let scroll = if weekly {
                    ui::calendar::scroll_to_now(
                        state.calendar.hour_height,
                        state.config.calendar.day_end_minutes(),
                    )
                } else {
                    Task::none()
                };
                Task::batch([close, scroll, sync_calendar(state)])
            } else {
                close
            }
        }

        Message::WeekHover(day) => {
            state.week_hover = day;
            Task::none()
        }

        Message::WeekScrolled(x) => {
            state.week_scroll_x = x;
            Task::none()
        }

        Message::WeekDayAction(day, message) => {
            let selects_day = matches!(
                &*message,
                Message::EventOpened(_)
                    | Message::EventTimeChanged { .. }
                    | Message::TaskToggled(_)
                    | Message::EventDraftStarted { .. }
                    | Message::TimeBlocking(
                        crate::timeblocking::Action::View(_)
                            | crate::timeblocking::Action::DropDay
                            | crate::timeblocking::Action::DropTime(_)
                            | crate::timeblocking::Action::DropTask(_)
                            | crate::timeblocking::Action::DropBlock(_)
                    )
            );
            if selects_day && day != state.calendar.day {
                let today = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
                let follow_today = state.calendar.follow_today;
                state.calendar.set_day(day, today);
                state.calendar.follow_today = follow_today;
                if matches!(
                    &*message,
                    Message::EventOpened(_)
                        | Message::TimeBlocking(crate::timeblocking::Action::View(_))
                ) {
                    let _ = crate::timeblocking::close(state);
                }
            }
            dispatch(state, *message)
        }

        Message::Projects(action) => crate::projects::update(state, action),
        Message::Travel(action) => crate::travel::update(state, action),

        Message::KeepRefresh => refresh_keep(state),
        Message::KeepSignin => {
            if state.keep.loading {
                return Task::none();
            }
            state.keep.loading = true;
            state.keep.error = None;
            blocking::run(crate::keep::signin).map(Message::KeepSignedIn)
        }
        Message::KeepSignedIn(result) => {
            state.keep.loading = false;
            match result {
                Ok(()) => refresh_keep(state),
                Err(error) => {
                    state.keep.error = Some(error);
                    Task::none()
                }
            }
        }
        Message::KeepLoaded(result) => {
            state.keep.loading = false;
            match result {
                Ok(snapshot) => {
                    state.keep.snapshot = snapshot;
                    state.keep.error = None;
                }
                Err(error) => state.keep.error = Some(error),
            }
            Task::none()
        }
        Message::KeepQueryChanged(query) => {
            state.keep.query = query;
            Task::none()
        }
        Message::KeepLabelChanged(label) => {
            state.keep.label = label;
            Task::none()
        }
        Message::KeepScopeChanged(scope) => {
            state.keep.scope = scope;
            Task::none()
        }
        Message::KeepEdit(url) => {
            if url.starts_with("https://keep.google.com/") {
                if let Err(error) = calendar::oauth::open_in_browser(&url) {
                    state.keep.error = Some(error);
                }
            }
            Task::none()
        }

        Message::Dismiss => dismiss(state),

        Message::PanelShortcutChanged(value) => {
            state.panel_shortcut_input = value;
            state.panel_shortcut_result = None;
            Task::none()
        }

        Message::PanelShortcutApply => {
            if state.panel_shortcut_busy || state.panel_shortcut_input.trim().is_empty() {
                return Task::none();
            }
            state.panel_shortcut_busy = true;
            state.panel_shortcut_result = None;
            let shortcut = state.panel_shortcut_input.trim().to_owned();
            blocking::run(move || {
                panel::rebind(&shortcut).map_err(|error| format!("{error:#}"))?;
                Ok(shortcut)
            })
            .map(Message::PanelShortcutApplied)
        }

        Message::PanelShortcutApplied(result) => {
            state.panel_shortcut_busy = false;
            state.panel_shortcut_result = Some(result);
            Task::none()
        }

        Message::ViewLayoutInputChanged(field, input) => {
            if !input.chars().all(|c| c.is_ascii_digit()) {
                return Task::none();
            }
            let (min, max) = field.bounds();
            if let Ok(value) = input.parse::<f32>()
                && (min..=max).contains(&value)
            {
                update(state, field.changed(value))
            } else {
                state.view_layout_input = Some((field, input));
                Task::none()
            }
        }

        Message::ViewLayoutInputSubmitted(field) => {
            let value = state
                .view_layout_input
                .take()
                .filter(|(edited, _)| *edited == field)
                .and_then(|(_, input)| input.parse::<f32>().ok())
                .filter(|value| value.is_finite())
                .unwrap_or_else(|| field.value(state));
            update(state, field.changed(value))
        }

        Message::CalendarWidthChanged(width) => {
            state.view_layout_input = None;
            if !width.is_finite() {
                return Task::none();
            }
            let pane = state.config.views.time_block_width();
            state.config.views.calendar_width = Some(width.clamp(
                config::ViewSettings::MIN_WIDTH,
                config::ViewSettings::MAX_WIDTH,
            ));
            // A remembered total must not override a newly chosen pane size.
            state.config.window.time_block_width = state
                .timeblocking
                .original_width
                .map(|_| state.calendar_width() + state.screen_margin() + pane);
            let resize = apply_view_sizes(state);
            Task::batch([resize, queue_save(state)])
        }

        Message::TimeBlockWidthChanged(width) => {
            state.view_layout_input = None;
            if !width.is_finite() {
                return Task::none();
            }
            state.config.views.time_block_width = width.clamp(
                config::ViewSettings::MIN_WIDTH,
                config::ViewSettings::MAX_WIDTH,
            );
            state.config.window.time_block_width = None;
            let resize = apply_view_sizes(state);
            Task::batch([resize, queue_save(state)])
        }

        Message::DetailWidthChanged(width) => {
            state.view_layout_input = None;
            if !width.is_finite() {
                return Task::none();
            }
            state.config.views.detail_width = width.clamp(
                config::ViewSettings::MIN_WIDTH,
                config::ViewSettings::MAX_WIDTH,
            );
            let resize = apply_view_sizes(state);
            Task::batch([resize, queue_save(state)])
        }

        Message::ScreenMarginChanged(margin) => {
            state.view_layout_input = None;
            if !margin.is_finite() {
                return Task::none();
            }
            let before = state.screen_margin();
            state.config.views.margin = margin.clamp(0.0, config::ViewSettings::MAX_MARGIN);
            let delta = state.screen_margin() - before;
            if let Some(width) = &mut state.config.window.time_block_width {
                *width += delta;
            }
            let resize = if state.sidebar.is_some() {
                apply_view_sizes(state)
            } else {
                Task::none()
            };
            Task::batch([resize, queue_save(state)])
        }

        Message::DockWidthChanged(width) => {
            state.view_layout_input = None;
            if !width.is_finite() {
                return Task::none();
            }
            state.config.views.dock_width = width.clamp(
                config::ViewSettings::MIN_DOCK_WIDTH,
                config::ViewSettings::MAX_DOCK_WIDTH,
            );
            queue_save(state)
        }

        Message::BorderRadiusChanged(radius) => {
            state.view_layout_input = None;
            if !radius.is_finite() {
                return Task::none();
            }
            state.config.views.border_radius =
                radius.clamp(0.0, config::ViewSettings::MAX_BORDER_RADIUS);
            queue_save(state)
        }

        Message::ThemeSelected(name) => {
            state.config.appearance.theme = name;
            state.config.appearance.follow_system = false;
            queue_save(state)
        }

        Message::FollowSystemToggled(follow) => {
            state.config.appearance.follow_system = follow;
            queue_save(state)
        }

        Message::AlwaysOnTopToggled(on_top) => {
            state.config.window.always_on_top = on_top;

            let apply = state
                .main_window
                .map(|id| window::set_level(id, level(on_top)))
                .unwrap_or(Task::none());

            Task::batch([apply, queue_save(state)])
        }

        Message::SoundRendered(Ok(pcm), spec) => {
            state.audio_error = None;
            state.audio.set_gain(spec.gain);
            state.audio.play(audio::Play {
                pcm,
                repeats: spec.repeats,
                fade_in: spec.fade_in,
            });

            Task::none()
        }

        Message::SoundRendered(Err(error), _) => {
            log::warn!("could not prepare the alarm sound: {error}");
            state.audio_error = Some(error);
            Task::none()
        }

        Message::NotificationPosted(Err(error)) => {
            log::warn!("could not post the notification: {error}");
            Task::none()
        }

        Message::NotificationPosted(Ok(())) => Task::none(),

        Message::ConfigSaved(result) => {
            if let Err(error) = result {
                log::warn!("could not save the config: {error}");
            }

            Task::none()
        }

        Message::ConfigReloaded(config) => {
            if config == state.disk_config {
                return Task::none();
            }
            state.disk_config = config.clone();
            if config == state.config {
                return Task::none();
            }
            let previous = std::mem::replace(&mut state.config, config);
            if previous.views != state.config.views {
                state.view_layout_input = None;
            }
            // An external edit wins over a save that has not started yet.
            state.save_queued = false;
            if previous.notepad.text != state.config.notepad.text {
                state.notepad = text_editor::Content::with_text(&state.config.notepad.text);
            }
            state.audio.set_gain(state.config.alarm.sound.gain());
            state.calendar.apply_task_colors(&state.config.calendar);
            if previous.calendar.day_start_hour() != state.config.calendar.day_start_hour() {
                let day = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
                state.calendar.set_day(day, day);
                state.calendar.forget_cached_days();
            }
            let mut tasks = Vec::new();
            let pane_widths_changed = previous.views.calendar_width
                != state.config.views.calendar_width
                || previous.views.time_block_width != state.config.views.time_block_width
                || previous.views.detail_width != state.config.views.detail_width;
            let views_changed = pane_widths_changed
                || (state.sidebar.is_some() && previous.views.margin != state.config.views.margin);
            let weekly_size_changed = state.weekly_active()
                && (previous.calendar.weekly_days() != state.config.calendar.weekly_days()
                    || previous.calendar.weekly_day_width()
                        != state.config.calendar.weekly_day_width());
            if pane_widths_changed
                && previous.window.time_block_width == state.config.window.time_block_width
            {
                state.config.window.time_block_width = None;
            } else if state.sidebar.is_some()
                && previous.views.margin != state.config.views.margin
                && previous.window.time_block_width == state.config.window.time_block_width
            {
                if let Some(width) = &mut state.config.window.time_block_width {
                    *width += state.config.views.margin() - previous.views.margin();
                }
            }
            if views_changed || weekly_size_changed {
                if state.sidebar.is_none() {
                    state.main_size.height = state.config.window.height;
                }
                tasks.push(apply_view_sizes(state));
            }
            if state.sidebar.is_none() {
                if let Some(id) = state.main_window {
                    if previous.window.always_on_top != state.config.window.always_on_top {
                        tasks.push(window::set_level(
                            id,
                            level(state.config.window.always_on_top),
                        ));
                    }
                    if !views_changed
                        && !weekly_size_changed
                        && (previous.window.width != state.config.window.width
                            || previous.window.height != state.config.window.height
                            || (state.timeblocking.original_width.is_some()
                                && previous.window.time_block_width
                                    != state.config.window.time_block_width))
                    {
                        state.main_size.height = state.config.window.height;
                        tasks.push(apply_view_sizes(state));
                    }
                }
            }
            if previous.calendar != state.config.calendar {
                tasks.push(sync_calendar(state));
            }
            log::info!("reloaded config from disk");
            Task::batch(tasks)
        }

        Message::SaveRequested => {
            if !state.save_queued {
                return Task::none();
            }
            state.save_queued = false;
            state.disk_config = state.config.clone();
            let config = state.config.clone();

            blocking::run(move || config.save().map_err(|error| error.to_string()))
                .map(Message::ConfigSaved)
        }

        Message::AudioStatusPolled => {
            if let audio::Status::Unavailable(reason) = state.audio.status() {
                state.audio_error = Some(reason);
            }

            Task::none()
        }

        Message::PanelCommandPolled => {
            let Some(command) = state
                .panel_command_rx
                .as_ref()
                .and_then(|rx| rx.try_recv().ok())
            else {
                return Task::none();
            };

            match command {
                PanelCommand::Show => {
                    if state.main_window.is_some() {
                        return Task::none(); // already visible
                    }
                    let Some(panel) = &state.panel_config else {
                        return Task::none();
                    };
                    let mut settings = sidebar_settings(panel, &state.config.views);
                    if state.weekly_active() {
                        let width = state.weekly_window_width();
                        settings.size = Size::new(width, PANEL_FILL);
                        settings.min_size = Some(Size::new(
                            (MIN_SIZE.width + state.screen_margin()).min(width),
                            MIN_SIZE.height,
                        ));
                        settings.max_size = Some(Size::new(width, 100_000.0));
                    } else {
                        // The editor and details pane can stay open while hidden.
                        if let Some(base) = state.timeblocking.original_width {
                            let width = crate::timeblocking::editor_width(state, base);
                            settings.size = Size::new(width, PANEL_FILL);
                            settings.max_size = None;
                        }
                        if state.details_pane_open {
                            settings.size.width += state.config.views.detail_width();
                            settings.max_size = None;
                        }
                    }
                    let (_id, open) = window::open(settings);
                    open.map(Message::WindowOpened)
                }
                PanelCommand::Hide => {
                    let Some(id) = state.main_window else {
                        return Task::none(); // already hidden
                    };
                    state.panel_expected_close = true;
                    window::close(id)
                }
            }
        }

        Message::WindowOpened(id) => {
            state.main_window = Some(id);

            // The window opens at whatever size we last saved; once the
            // monitor size is known it is widened to the default fraction of
            // the screen. The height stays as the user left it.
            let measure = window::monitor_size(id).map(move |size| Message::MonitorSized(id, size));

            // The calendar is the front page now, so it opens on the hour the
            // needle is at rather than on midnight.
            let scroll = ui::calendar::scroll_to_now(
                state.calendar.hour_height,
                state.config.calendar.day_end_minutes(),
            );

            let sync = if state.calendar.is_signed_in() {
                sync_calendar(state)
            } else {
                Task::none()
            };

            Task::batch([measure, sync, scroll])
        }

        Message::MonitorSized(id, size) => {
            if state.main_window != Some(id) {
                return Task::none();
            }

            state.monitor_size = size;
            if state.weekly_active() {
                if state.sidebar.is_some() {
                    if let Some(size) =
                        size.filter(|size| size.height.is_finite() && size.height > 0.0)
                    {
                        state.main_size.height = size.height;
                    }
                }
                return apply_view_sizes(state);
            }

            let Some(size) = size else {
                return Task::none();
            };

            // The panel is pinned to its configured width and the full height
            // of the monitor, and never widens to the default fraction.
            if let Some(sidebar) = &state.sidebar {
                let mut width = match state.timeblocking.original_width {
                    Some(base) => crate::timeblocking::editor_width(state, base),
                    None => {
                        state.config.views.calendar_width(sidebar.width) + state.screen_margin()
                    }
                };
                if state.details_pane_open {
                    width += state.config.views.detail_width();
                }
                log::debug!("panel filling {}x{}", width, size.height);
                return window::resize(id, Size::new(width, size.height));
            }

            // Only a window that has never been sized gets the default
            // fraction. A width the user chose, or the one saved with the
            // editor open, must survive a relaunch.
            if state.config.window.width != Config::default().window.width
                || state.config.views.calendar_width.is_some()
                || state.timeblocking.original_width.is_some()
            {
                return Task::none();
            }

            let width = launch_width(size.width);
            let height = state.config.window.height;
            state.config.window.width = width;

            Task::batch([
                window::resize(id, Size::new(width, height)),
                queue_save(state),
            ])
        }

        Message::WeekMonitorSized(id, size) => {
            if state.main_window != Some(id) || !state.weekly_active() {
                return Task::none();
            }
            state.monitor_size = size;
            apply_view_sizes(state)
        }

        Message::WindowResized(id, size) => {
            if state.main_window != Some(id) {
                return Task::none();
            }

            state.main_size = size;
            log::debug!("window is now {}x{}", size.width, size.height);

            if state.weekly_active() {
                state.weekly_resize_pending = false;
                if state.sidebar.is_none() {
                    state.config.window.height = size.height;
                    return queue_save(state);
                }
                return Task::none();
            }
            if let Some((target, started)) = state.weekly_width_restoring {
                if started.elapsed() < Duration::from_secs(1) {
                    if (size.width - target).abs() <= 0.5 {
                        state.weekly_width_restoring = None;
                    }
                    if state.sidebar.is_none() {
                        state.config.window.height = size.height;
                        return queue_save(state);
                    }
                    return Task::none();
                }
                state.weekly_width_restoring = None;
            }

            // The expanded editor has its own saved width. Keep the width to
            // restore on close, including in the side-panel companion. The
            // details pane, when open beside it, is not part of that width.
            if state.timeblocking.original_width.is_some() {
                let detail = if state.details_pane_open {
                    state.config.views.detail_width()
                } else {
                    0.0
                };
                state.config.window.time_block_width = Some((size.width - detail).max(0.0));
                if state.sidebar.is_none() {
                    state.config.window.height = size.height;
                }
                return queue_save(state);
            }

            // The details pane widens the window on its own. The width the user
            // drags out is the pane's, not the window's, so it is not saved as
            // the normal window size.
            if state.details_pane_open {
                if state.sidebar.is_some() {
                    return Task::none();
                }
                state.config.window.height = size.height;
                return queue_save(state);
            }

            // The panel's geometry is not the user's saved window, so it must
            // not be written back over it.
            if state.sidebar.is_some() {
                return Task::none();
            }

            state.config.window.width = size.width;
            state.config.window.height = size.height;

            queue_save(state)
        }

        Message::WindowClosed(id) => {
            if state.main_window == Some(id) {
                if state.panel_expected_close {
                    state.panel_expected_close = false;
                    state.main_window = None;
                    return Task::none();
                }
                // Closing the clock quits, even though a daemon would
                // otherwise keep running with no windows left.
                if !state.config.task_work.is_empty() {
                    let config = state.config.clone();
                    return blocking::run(move || config.save()).then(|_| iced::exit());
                }
                return iced::exit();
            }

            if state.flash.as_ref().is_some_and(|flash| flash.window == id) {
                state.flash = None;
            }

            Task::none()
        }

        Message::FlashFinished => {
            let Some(flash) = state.flash.take() else {
                return Task::none();
            };

            window::close(flash.window)
        }

        Message::KeyPressed(key, modifiers) => on_key(state, key, modifiers),

        Message::Ignored => Task::none(),

        Message::PendingTasksShown(day) => {
            state.calendar.pending_open = day;
            Task::none()
        }

        Message::CalendarMenuToggled => {
            state.calendar.menu_open = !state.calendar.menu_open;
            // Dismissing the menu also puts away any open color picker, so it
            // does not pop back up the next time the menu is opened.
            state.calendar.color_editor = None;
            Task::none()
        }

        Message::CalendarToggled(id, enabled) => {
            state.config.calendar.set_calendar_enabled(&id, enabled);
            queue_save(state)
        }

        Message::CalendarSourceMoved(id, up) => {
            let mut order: Vec<_> = state
                .calendar
                .display_sources(&state.config.calendar)
                .into_iter()
                .map(|source| source.id)
                .collect();
            let Some(index) = order.iter().position(|source| source == &id) else {
                return Task::none();
            };
            let next = if up {
                index.checked_sub(1)
            } else {
                index.checked_add(1).filter(|&next| next < order.len())
            };
            let Some(next) = next else {
                return Task::none();
            };
            order.swap(index, next);
            // Keep preferences for calendars temporarily absent from the account.
            for saved in &state.config.calendar.source_order {
                if !order.contains(saved) {
                    order.push(saved.clone());
                }
            }
            state.config.calendar.source_order = order;
            queue_save(state)
        }

        Message::CalendarSyncRequested => sync_calendar(state),

        Message::CalendarResyncRequested => {
            // Days parked in memory and on disk would paint before the next
            // visit fetched them. Drop them so the resync is the source of
            // truth. The day in view stays up until its own fetch lands.
            state.calendar.invalidate_other_days();
            sync_calendar(state)
        }

        Message::CalendarSyncedAt(revision, result) => {
            if revision != state.timeblocking.revision
                || (state.screen == Screen::Calendar && state.weekly)
            {
                state.calendar.sync = SyncState::Idle;
                return sync_calendar(state);
            }
            Task::done(Message::CalendarSynced(result))
        }
        Message::CalendarWeekSynced(revision, anchor, days, result) => {
            if revision != state.timeblocking.revision
                || state.screen != Screen::Calendar
                || !state.weekly
                || anchor != state.week_anchor
                || days != state.config.calendar.weekly_days()
            {
                state.calendar.sync = SyncState::Idle;
                return sync_calendar(state);
            }
            match result {
                Ok(mut snapshot) => {
                    if let Some(notice) = state.timeblocking.sync_notice.take() {
                        snapshot.notice = Some(match snapshot.notice.take() {
                            Some(existing) => format!("{notice}\n{existing}"),
                            None => notice,
                        });
                    }
                    let dates = calendar::week_dates(anchor, days);
                    state.calendar.apply_week_snapshot(snapshot, &dates);
                    state.calendar.apply_task_colors(&state.config.calendar);
                    reconcile_completed_task(state)
                }
                Err(error) => dispatch(state, Message::CalendarSynced(Err(error))),
            }
        }
        Message::CalendarSynced(Ok(mut snapshot)) => {
            // The user may have stepped to another day while this was in
            // flight. The events are for the day they left, so drop them and
            // fetch the day now in view rather than snapping the view back.
            if snapshot.day != state.calendar.day {
                state.calendar.sync = SyncState::Idle;
                return sync_calendar(state);
            }

            if let Some(notice) = state.timeblocking.sync_notice.take() {
                snapshot.notice = Some(match snapshot.notice.take() {
                    Some(existing) => format!("{notice}\n{existing}"),
                    None => notice,
                });
            }
            state.calendar.apply_snapshot(snapshot);
            state.calendar.apply_task_colors(&state.config.calendar);
            reconcile_completed_task(state)
        }

        Message::CalendarSynced(Err(error)) => {
            log::warn!("calendar sync failed: {error}");
            state.calendar.sync = SyncState::Idle;
            if matches!(error.as_str(), "Not connected to Google Calendar") {
                state.calendar.connection = Connection::SignedOut;
            }
            state.calendar.error = Some(error);
            Task::none()
        }

        Message::CalendarTick => refresh_day(state),

        Message::OpenGoogleCalendar => {
            if let Err(error) = calendar::oauth::open_in_browser(calendar::WEB_URL) {
                state.calendar.error = Some(error);
            }
            Task::none()
        }

        Message::DockRequested => {
            let origin = if state.sidebar.is_some() {
                crate::dock::launch::Origin::Sidebar
            } else {
                crate::dock::launch::Origin::Window
            };
            if let Err(error) = crate::dock::launch::open_dock(origin) {
                state.calendar.error = Some(format!("Could not open the dock: {error:#}"));
                return Task::none();
            }

            state.calendar.menu_open = false;
            let Some(id) = state.main_window else {
                return Task::none();
            };
            // The panel process stays up with its window hidden, so it can be
            // brought back; the regular window just quits.
            state.panel_expected_close = state.sidebar.is_some();
            window::close(id)
        }

        Message::EventOpened(id) => {
            state.calendar.menu_open = false;
            if let Some(event) = state.calendar.event(&id) {
                if crate::timeblocking::is_block(event, &state.config.calendar) {
                    state.calendar.open_event = None;
                    return crate::timeblocking::open(state, id);
                }
                if event.is_task() {
                    state.timeblocking.title = event.title.clone();
                    state.timeblocking.title_error = None;
                    state.timeblocking.date =
                        if event.start.date_naive() == crate::timeblocking::UNDATED {
                            state.calendar.day.to_string()
                        } else {
                            event.start.date_naive().to_string()
                        };
                    state.timeblocking.deadline =
                        calendar::deadline::input_text(&event.details.description);
                    state.timeblocking.start_time = if event.all_day {
                        "09:00".into()
                    } else {
                        calendar::clock_label(event.start)
                    };
                    state.timeblocking.end_time = if event.all_day {
                        "09:30".into()
                    } else {
                        calendar::clock_label(event.end)
                    };
                }
            }
            state.calendar.open_event = Some(id);
            // Recorded during this click, before the message is handled.
            state.calendar.details_below = ui::calendar::click_was_in_the_top_half();
            state.inbox.picker = None;
            Task::none()
        }

        Message::EventClosed => {
            state.calendar.open_event = None;
            state.inbox.picker = None;
            Task::none()
        }

        Message::OpenEventLink(url) => {
            if let Err(error) = calendar::oauth::open_in_browser(&url) {
                state.calendar.error = Some(error);
            }
            Task::none()
        }

        Message::EventTimeChanged { id, start, end } => {
            if state
                .calendar
                .event(&id)
                .is_some_and(|event| event.is_task())
            {
                return crate::timeblocking::update(
                    state,
                    crate::timeblocking::Action::Schedule(id, start, end),
                );
            }
            let Some(calendar_id) = state
                .calendar
                .event(&id)
                .filter(|event| !event.is_task())
                .map(|event| event.calendar_id.clone())
            else {
                return Task::none();
            };

            // The chip moves now; the lighter shade and the write follow. A
            // second drag on the same event is refused until this one lands.
            if !state.calendar.begin_time_change(&id, start, end) {
                return Task::none();
            }

            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            let request_id = id.clone();
            state.calendar.error = None;

            blocking::run(move || {
                calendar::set_event_time(
                    &client_id,
                    &client_secret,
                    &calendar_id,
                    &request_id,
                    start,
                    end,
                )
            })
            .map(move |result| Message::EventTimeSaved(id.clone(), result))
        }

        Message::EventTimeSaved(id, Ok(())) => {
            state.calendar.confirm_time_change(&id);
            let refresh =
                crate::timeblocking::update(state, crate::timeblocking::Action::RefreshBlock(id));
            Task::batch([refresh, refresh_week_after_write(state)])
        }

        Message::EventTimeSaved(id, Err(error)) => {
            state.calendar.fail_time_change(&id);
            log::warn!("could not move the event: {error}");
            state.calendar.error = Some(error);
            Task::none()
        }

        Message::EventDeleteRequested(id) => {
            // Only calendar events can be deleted this way; Google Tasks live
            // under the Tasks API and are ticked off instead.
            let is_task = state
                .calendar
                .event(&id)
                .map(|event| event.is_task())
                .unwrap_or(false);
            if is_task {
                return Task::none();
            }
            state.calendar.pending_delete = Some(id);
            Task::none()
        }

        Message::EventDeleteConfirmed(id) => {
            let Some(calendar_id) = state
                .calendar
                .event(&id)
                .filter(|event| !event.is_task())
                .map(|event| event.calendar_id.clone())
            else {
                state.calendar.pending_delete = None;
                return Task::none();
            };

            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            let request_id = id.clone();
            state.calendar.error = None;

            blocking::run(move || {
                calendar::delete_event(&client_id, &client_secret, &calendar_id, &request_id)
            })
            .map(move |result| Message::EventDeleted(id.clone(), result))
        }

        Message::EventDeleteCancelled => {
            state.calendar.pending_delete = None;
            Task::none()
        }

        Message::EventDeleted(id, Ok(())) => {
            state.calendar.pending_delete = None;
            // The event is gone on Google's side, so drop it from the day in
            // view. The next sync reconciles the cache.
            if state.screen == Screen::Calendar && state.weekly {
                state.calendar.forget_event_everywhere(&id);
            } else {
                state.calendar.events.retain(|event| event.id != id);
            }
            let closing_details = state.calendar.open_event.as_deref() == Some(id.as_str());
            if closing_details {
                state.calendar.open_event = None;
            }
            let close = if state.timeblocking.block.as_deref() == Some(id.as_str()) {
                let close = crate::timeblocking::close(state);
                if closing_details && state.details_pane_open {
                    // Detail-pane reconciliation will resize both panes together.
                    Task::none()
                } else {
                    close
                }
            } else {
                Task::none()
            };
            Task::batch([close, refresh_week_after_write(state)])
        }

        Message::EventDeleted(_id, Err(error)) => {
            state.calendar.pending_delete = None;
            log::warn!("could not delete the event: {error}");
            state.calendar.error = Some(error);
            Task::none()
        }

        Message::TaskDeleteRequested(id) => {
            // Only Google Tasks can be deleted this way; calendar events use
            // EventDeleteRequested instead.
            let is_task = state
                .calendar
                .event(&id)
                .map(|event| event.is_task())
                .unwrap_or(false);
            if !is_task {
                return Task::none();
            }
            state.calendar.pending_delete = Some(id);
            Task::none()
        }

        Message::TaskDeleteConfirmed(id) => {
            if state.calendar.pending_delete.as_deref() != Some(id.as_str()) {
                return Task::none();
            }
            if !state.timeblocking.busy.is_empty() {
                return Task::none();
            }
            state.timeblocking.revision += 1;
            let Some(task) = state
                .calendar
                .event(&id)
                .filter(|event| event.is_task())
                .cloned()
            else {
                state.calendar.pending_delete = None;
                return Task::none();
            };

            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            let tasks_calendar = state.config.calendar.tasks_calendar.clone();
            state.calendar.error = None;
            state.timeblocking.busy.insert(id.clone());

            blocking::run(move || {
                calendar::delete_task(&client_id, &client_secret, &task, tasks_calendar.as_deref())
            })
            .map(move |result| Message::TaskDeleted(id.clone(), result))
        }

        Message::TaskDeleted(id, Ok(())) => {
            state.timeblocking.busy.remove(&id);
            state.timeblocking.revision += 1;
            state.calendar.pending_delete = None;
            if let Some(task) = state.calendar.event(&id) {
                state
                    .config
                    .task_work
                    .stop(&timeblocking::task_key(task), chrono::Utc::now());
            }
            // The task is gone on Google's side. Drop every chip that was
            // showing it, including a stale copy on the list it was moved
            // from and the calendar block that held its time.
            state.calendar.forget_task(&id);
            if state.screen == Screen::Calendar && state.weekly {
                state.calendar.clear_disk_cache();
            } else {
                state.calendar.invalidate_other_days();
            }
            let save = queue_save(state);
            Task::batch([save, refresh_week_after_write(state)])
        }

        Message::TaskDeleted(id, Err(error)) => {
            state.timeblocking.busy.remove(&id);
            state.timeblocking.revision += 1;
            state.calendar.pending_delete = None;
            log::warn!("could not delete the task: {error}");
            state.calendar.error = Some(error);
            Task::none()
        }

        Message::TaskToggled(_) | Message::TaskIgnoreToggled(_) => {
            let ignoring = matches!(&message, Message::TaskIgnoreToggled(_));
            let (Message::TaskToggled(id) | Message::TaskIgnoreToggled(id)) = message else {
                unreachable!()
            };
            if !state.timeblocking.busy.is_empty() {
                return Task::none();
            }
            state.timeblocking.revision += 1;
            let Some(task) = state
                .calendar
                .event(&id)
                .filter(|event| event.is_task())
                .cloned()
            else {
                return Task::none();
            };
            let previous = timeblocking::task_status(&task);
            let status = if ignoring {
                if previous == timeblocking::TaskStatus::Ignored {
                    timeblocking::TaskStatus::Active
                } else {
                    timeblocking::TaskStatus::Ignored
                }
            } else if previous == timeblocking::TaskStatus::Completed {
                timeblocking::TaskStatus::Active
            } else {
                timeblocking::TaskStatus::Completed
            };

            // Update the chip straight away, then put it back if Google
            // refuses the write.
            state.timeblocking.busy.insert(id.clone());
            let completed = status != timeblocking::TaskStatus::Active;
            if completed && let Some(task) = state.calendar.event(&id) {
                state
                    .config
                    .task_work
                    .stop(&crate::timeblocking::task_key(task), chrono::Utc::now());
                state.timeblocking.work_now = Some(chrono::Utc::now());
                let _ = queue_save(state);
            }
            if let Some(event) = state.calendar.event_mut(&id) {
                timeblocking::apply_status(event, status);
            }

            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            state.calendar.error = None;

            blocking::run(move || {
                calendar::set_linked_task_status(&client_id, &client_secret, &task, status)
            })
            .map(move |result| Message::TaskCompletionSaved(id.clone(), previous, result))
        }

        Message::TaskCompletionSaved(id, _, Ok(task)) => {
            if let Some(event) = state.calendar.event_mut(&id) {
                *event = task.clone();
            }
            state.timeblocking.busy.remove(&id);
            state.timeblocking.revision += 1;
            if state.screen == Screen::Calendar && state.weekly {
                state.calendar.replace_cached_task(&id, &task);
            } else {
                state.calendar.invalidate_other_days();
            }
            Task::done(Message::CalendarSyncRequested)
        }

        Message::TaskCompletionSaved(id, previous, Err(error)) => {
            state.timeblocking.busy.remove(&id);
            state.timeblocking.revision += 1;
            if let Some(event) = state.calendar.event_mut(&id) {
                timeblocking::apply_status(event, previous);
            }

            log::warn!("could not update the task: {error}");
            state.calendar.error = Some(error);
            Task::none()
        }

        Message::ShowTasksToggled(show) => {
            state.config.calendar.show_tasks = show;
            queue_save(state)
        }

        Message::TaskColorEditorToggled(id) => {
            state.calendar.color_editor =
                (state.calendar.color_editor.as_deref() != Some(id.as_str())).then_some(id);
            Task::none()
        }

        Message::TaskColorPicked(id, color) => {
            state.config.calendar.set_task_color(&id, color);
            state.calendar.apply_task_colors(&state.config.calendar);
            state.calendar.color_editor = None;
            queue_save(state)
        }

        Message::GoogleClientIdChanged(id) => {
            state.config.calendar.client_id = id;
            queue_save(state)
        }

        Message::GoogleClientSecretChanged(secret) => {
            state.config.calendar.client_secret = secret;
            queue_save(state)
        }

        Message::GoogleConnectRequested => {
            if state.calendar.is_busy() {
                return Task::none();
            }

            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            state.calendar.sync = SyncState::Connecting;
            state.calendar.error = None;

            blocking::run(move || calendar::connect(client_id, client_secret))
                .map(Message::GoogleConnected)
        }

        Message::GoogleConnected(Ok(_tokens)) => {
            // Signing in is what makes config sync possible.
            crate::config_sync::poke();
            state.calendar.connection = Connection::SignedIn;
            state.calendar.sync = SyncState::Idle;
            state.calendar.error = None;
            sync_calendar(state)
        }

        Message::GoogleConnected(Err(error)) => {
            log::warn!("Google sign-in failed: {error}");
            state.calendar.sync = SyncState::Idle;
            state.calendar.connection = Connection::SignedOut;
            state.calendar.error = Some(error);
            Task::none()
        }

        Message::GoogleDisconnectRequested => {
            calendar::disconnect();
            state.calendar = calendar::Store::new(state.config.calendar.day_start_hour());
            // The reset takes the store with it, so the directory is put
            // straight back: days keep being written across a disconnect.
            state.calendar.cache_days_in(config::cache_days_dir());
            state.calendar.connection = Connection::SignedOut;
            Task::none()
        }

        Message::CalendarSyncMinutesChanged(minutes) => {
            state.config.calendar.sync_minutes = minutes.clamp(
                config::CalendarSettings::MIN_SYNC_MINUTES,
                config::CalendarSettings::MAX_SYNC_MINUTES,
            );
            queue_save(state)
        }

        Message::CalendarDayEndChanged(hour) => {
            state.config.calendar.day_end_hour =
                hour.min(config::CalendarSettings::MAX_DAY_END_HOUR);
            // The day's window moved, so the day in view and the events that
            // belong to it may have changed too. This also puts the view back
            // on today if the user had navigated away.
            let day = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
            state.calendar.set_day(day, day);
            state.week_anchor = day;
            // The day's window moved, so the cached days no longer describe
            // the days they are filed under.
            state.calendar.forget_cached_days();
            let save = queue_save(state);
            Task::batch([save, sync_calendar(state)])
        }

        Message::CalendarWeeklyDaysChanged(days) => {
            state.config.calendar.weekly_days = days.clamp(7, 8);
            let save = queue_save(state);
            let resize = if state.weekly_active() {
                apply_view_sizes(state)
            } else {
                Task::none()
            };
            Task::batch([save, resize, sync_calendar(state)])
        }

        Message::CalendarWeeklyWidthChanged(width) => {
            state.config.calendar.weekly_day_width = width.clamp(100, 400);
            let save = queue_save(state);
            let resize = if state.weekly_active() {
                apply_view_sizes(state)
            } else {
                Task::none()
            };
            Task::batch([save, resize])
        }

        Message::CalendarZoomed { factor, anchor } => {
            let before = state.calendar.hour_height;
            state.calendar.zoom(factor);
            let after = state.calendar.hour_height;

            if after == before {
                return Task::none();
            }

            // Keep the row under the pointer fixed. The grid grows by `ratio`,
            // so the anchored row slides down by `anchor * (ratio - 1)` and the
            // scroll has to follow it by the same amount.
            let delta = crate::ui::calendar::zoom_scroll_delta(anchor, before, after);
            crate::ui::calendar::scroll_by(delta)
        }

        Message::CalendarDayShift(days) => {
            let today = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
            let origin = if state.weekly {
                state.week_anchor
            } else {
                state.calendar.day
            };
            let day = origin + chrono::Duration::days(days);
            let close = if state.weekly {
                state.calendar.open_event = None;
                crate::timeblocking::close(state)
            } else {
                Task::none()
            };
            state.calendar.set_day(day, today);
            if state.weekly {
                state.week_anchor = day;
                state.week_scroll_x = 0.0;
            }
            state.calendar.menu_open = false;
            state.calendar.month_open = false;
            let scroll = if state.weekly {
                ui::calendar::scroll_week_to_start()
            } else {
                Task::none()
            };
            Task::batch([close, scroll, sync_calendar(state)])
        }

        Message::CalendarDaySelected(day) => {
            let today = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
            let close = if state.weekly {
                state.calendar.open_event = None;
                crate::timeblocking::close(state)
            } else {
                Task::none()
            };
            state.calendar.set_day(day, today);
            if state.weekly {
                state.week_anchor = day;
                state.week_hover = None;
                state.week_scroll_x = 0.0;
            }
            state.calendar.month_open = false;
            let scroll = if state.weekly {
                ui::calendar::scroll_week_to_start()
            } else {
                Task::none()
            };
            Task::batch([close, scroll, sync_calendar(state)])
        }

        Message::CalendarMonthToggled => {
            state.calendar.month_open = !state.calendar.month_open;
            // The picker opens on the month of the day in view, and the menu
            // gets out of its way.
            if state.calendar.month_open {
                state.calendar.month = calendar::first_of_month(if state.weekly {
                    state.week_anchor
                } else {
                    state.calendar.day
                });
                state.calendar.menu_open = false;
            }
            Task::none()
        }

        Message::CalendarMonthShift(months) => {
            let step = chrono::Months::new(months.unsigned_abs());
            let shifted = if months >= 0 {
                state.calendar.month.checked_add_months(step)
            } else {
                state.calendar.month.checked_sub_months(step)
            };
            if let Some(month) = shifted {
                state.calendar.month = month;
            }
            Task::none()
        }

        Message::EventDraftStarted { start, end } => {
            let Some(calendar_id) = state.calendar.default_calendar_id(&state.config.calendar)
            else {
                state.calendar.error =
                    Some("Connect Google Calendar in Settings to add events".to_owned());
                return Task::none();
            };

            state.calendar.error = None;
            state.calendar.begin_draft(calendar_id, start, end);
            Task::none()
        }

        Message::EventDraftTitleChanged(title) => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.title = title;
            }
            Task::none()
        }

        Message::EventDraftDescriptionChanged(description) => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.description = description;
            }
            Task::none()
        }

        Message::EventDraftStartChanged(text) => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.set_start_text(text);
            }
            Task::none()
        }

        Message::EventDraftEndChanged(text) => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.set_end_text(text);
            }
            Task::none()
        }

        Message::EventDraftCalendarSelected(id) => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.calendar_id = id;
                draft.calendar_dropdown_open = false;
            }
            Task::none()
        }

        Message::EventDraftCalendarDropdownToggled => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.calendar_dropdown_open = !draft.calendar_dropdown_open;
            }
            Task::none()
        }

        Message::EventDraftCancelled => {
            state.calendar.draft = None;
            Task::none()
        }

        Message::EventDraftSaveRequested => {
            let Some(draft) = state.calendar.draft.clone() else {
                return Task::none();
            };

            if draft.end <= draft.start {
                state.calendar.error = Some("The end time has to be after the start".to_owned());
                return Task::none();
            }

            let color = state.calendar.calendar_color(&draft.calendar_id);
            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            state.calendar.error = None;

            blocking::run(move || {
                let id = calendar::create_event(
                    &client_id,
                    &client_secret,
                    &draft.calendar_id,
                    &draft.title,
                    &draft.description,
                    draft.start,
                    draft.end,
                )?;

                Ok(calendar::Event {
                    id,
                    calendar_id: draft.calendar_id.clone(),
                    title: if draft.title.trim().is_empty() {
                        "(No title)".to_owned()
                    } else {
                        draft.title.clone()
                    },
                    start: draft.start,
                    end: draft.end,
                    all_day: false,
                    color,
                    completed: false,
                    details: calendar::Details {
                        description: draft.description.clone(),
                        ..calendar::Details::default()
                    },
                })
            })
            .map(Message::EventDraftSaved)
        }

        Message::EventDraftSaved(Ok(event)) => {
            state.calendar.draft = None;
            state.calendar.add_event(event);
            refresh_week_after_write(state)
        }

        Message::EventDraftSaved(Err(error)) => {
            // The popup stays open so the half-filled event is not lost.
            log::warn!("could not create the event: {error}");
            state.calendar.error = Some(error);
            Task::none()
        }

        Message::EventEditRequested(id) => {
            state.calendar.begin_edit(&id);
            Task::none()
        }

        Message::EventEditTitleChanged(title) => {
            if let Some(draft) = state.calendar.edit_draft.as_mut() {
                draft.title = title;
            }
            Task::none()
        }

        Message::EventEditDescriptionChanged(description) => {
            if let Some(draft) = state.calendar.edit_draft.as_mut() {
                draft.description = description;
            }
            Task::none()
        }

        Message::EventEditStartChanged(text) => {
            if let Some(draft) = state.calendar.edit_draft.as_mut() {
                draft.set_start_text(text);
            }
            Task::none()
        }

        Message::EventEditEndChanged(text) => {
            if let Some(draft) = state.calendar.edit_draft.as_mut() {
                draft.set_end_text(text);
            }
            Task::none()
        }

        Message::EventEditCalendarSelected(id) => {
            if let Some(draft) = state.calendar.edit_draft.as_mut() {
                draft.calendar_id = id;
            }
            Task::none()
        }

        Message::EventEditCancelled => {
            state.calendar.edit_draft = None;
            Task::none()
        }

        Message::EventEditSaveRequested => {
            let Some(draft) = state.calendar.edit_draft.clone() else {
                return Task::none();
            };

            if draft.end <= draft.start {
                state.calendar.error = Some("The end time has to be after the start".to_owned());
                return Task::none();
            }

            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            state.calendar.error = None;

            blocking::run(move || {
                calendar::update_event(
                    &client_id,
                    &client_secret,
                    &draft.calendar_id,
                    &draft.id,
                    &draft.title,
                    &draft.description,
                    draft.start,
                    draft.end,
                )?;
                Ok(())
            })
            .map(Message::EventEditSaved)
        }

        Message::EventEditSaved(Ok(())) => {
            state.calendar.edit_draft = None;
            refresh_week_after_write(state)
        }

        Message::EventEditSaved(Err(error)) => {
            // The popup stays open so the half-filled edit is not lost.
            log::warn!("could not update the event: {error}");
            state.calendar.error = Some(error);
            Task::none()
        }

        Message::DefaultCalendarSelected(id) => {
            state.config.calendar.default_calendar = if id.is_empty() { None } else { Some(id) };
            queue_save(state)
        }
    }
}

pub fn view(state: &State, window: window::Id) -> Element<'_, Message> {
    if state
        .flash
        .as_ref()
        .is_some_and(|flash| flash.window == window)
    {
        return ui::flash::view(state);
    }

    ui::view(state)
}

/// The theme for a given window.
///
/// Returning `None` is how iced is told to follow the system color scheme.
pub fn active_theme(state: &State, window: window::Id) -> Option<Theme> {
    if state
        .flash
        .as_ref()
        .is_some_and(|flash| flash.window == window)
    {
        return Some(theme::FLASH.clone());
    }

    if state.config.appearance.follow_system {
        return None;
    }

    Some(
        state
            .catalog
            .resolve(&state.config.appearance.theme)
            .theme
            .clone(),
    )
}

pub fn title(state: &State, window: window::Id) -> String {
    if state
        .flash
        .as_ref()
        .is_some_and(|flash| flash.window == window)
    {
        return "TimeTracker".to_owned();
    }

    // The panel carries a stable, distinctive title so a window rule can pick
    // it out from the main window; see the packaging README.
    if state.sidebar.is_some() {
        return "TimeTracker Sidebar".to_owned();
    }

    "TimeTracker".to_owned()
}

pub fn subscription(state: &State) -> Subscription<Message> {
    let mut subscriptions = vec![
        config::watch().map(Message::ConfigReloaded),
        window::close_events().map(Message::WindowClosed),
        window::resize_events().map(|(id, size)| Message::WindowResized(id, size)),
        keyboard::listen().map(map_key),
        iced::event::listen_with(|event, status, _| {
            if status == iced::event::Status::Ignored
                && matches!(
                    event,
                    iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                        iced::mouse::Button::Left
                    ))
                )
            {
                Some(Message::TimeBlocking(
                    crate::timeblocking::Action::CancelDrag,
                ))
            } else {
                None
            }
        }),
    ];

    // The needle has to move even when the clock is idle, but only while
    // the day view is actually on screen.
    if state.screen == Screen::Calendar {
        subscriptions.push(iced::time::every(NEEDLE_TICK).map(|_| Message::CalendarTick));
    }

    // Alarms are checked once a second, and only while there is one waiting,
    // so an idle TimeTracker with nothing set still costs nothing.
    if alarms::needs_poll(&state.alarms) {
        subscriptions.push(
            iced::time::every(Duration::from_secs(1))
                .map(|_| Message::Alarms(alarms::Action::Poll)),
        );
    }
    if alarms::needs_flush(&state.alarms) {
        subscriptions.push(
            iced::time::every(config::SAVE_DEBOUNCE)
                .map(|_| Message::Alarms(alarms::Action::Flush)),
        );
    }

    if state.timeblocking.focus.is_some() || state.config.task_work.active().is_some() {
        subscriptions.push(
            iced::time::every(Duration::from_secs(1))
                .map(|_| Message::TimeBlocking(crate::timeblocking::Action::WorkTick)),
        );
    }
    if state.calendar.is_signed_in() {
        subscriptions.push(
            iced::time::every(state.config.calendar.sync_every())
                .map(|_| Message::CalendarSyncRequested),
        );
    }

    if state.flash.is_some() {
        subscriptions.push(iced::time::every(FLASH_FRAME).map(Message::FlashFrame));
    }

    if matches!(state.screen, Screen::Settings) {
        subscriptions.push(iced::time::every(STATUS_POLL).map(|_| Message::AudioStatusPolled));
    }

    // The debounce itself: the write lands once the interval elapses, and
    // further edits in the meantime ride along on the same pending save.
    if state.save_queued {
        subscriptions
            .push(iced::time::every(config::SAVE_DEBOUNCE).map(|_| Message::SaveRequested));
    }

    // The side panel polls for show/hide commands from the panel daemon.
    if state.panel_command_rx.is_some() {
        subscriptions.push(
            iced::time::every(Duration::from_millis(200)).map(|_| Message::PanelCommandPolled),
        );
    }

    Subscription::batch(subscriptions)
}

fn map_key(event: keyboard::Event) -> Message {
    match event {
        keyboard::Event::KeyPressed { key, modifiers, .. } => Message::KeyPressed(key, modifiers),
        _ => Message::Ignored,
    }
}

fn on_key(state: &mut State, key: Key, modifiers: keyboard::Modifiers) -> Task<Message> {
    match key_message(state, key, modifiers) {
        Some(message) => Task::done(message),
        None => Task::none(),
    }
}

/// What a key press means, if anything.
///
/// Kept apart from the task it turns into, so that what a key does can be
/// looked at without running anything.
fn key_message(state: &mut State, key: Key, modifiers: keyboard::Modifiers) -> Option<Message> {
    use keyboard::key::Named;

    // The alarm popup and alarm setting mode are put away with Escape before
    // anything behind them is.
    if state.alarms.editor.is_some() {
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::Alarms(alarms::Action::CloseEditor)),
            _ => None,
        };
    }
    if state.alarms.mode {
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::Alarms(alarms::Action::ModeToggled)),
            _ => None,
        };
    }

    if state.timeblocking.estimate_task.is_some() {
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::TimeBlocking(
                crate::timeblocking::Action::CloseEstimate,
            )),
            _ => None,
        };
    }
    if state.travel.open.is_some() {
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::Travel(crate::travel::Action::CloseLeg)),
            _ => None,
        };
    }
    if state.inbox.draft.is_some() {
        // Typing into the popup must not reach the clock shortcuts.
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::Inbox(crate::inbox::Action::Cancel)),
            _ => None,
        };
    }
    if state.timeblocking.focus.is_some() {
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::TimeBlocking(
                crate::timeblocking::Action::CloseView,
            )),
            _ => None,
        };
    }
    if modifiers.command() {
        return match key.as_ref() {
            Key::Character(",") => Some(Message::ScreenSelected(Screen::Settings)),
            _ => None,
        };
    }

    // The notepad is for writing, so the shortcuts are left off while it is on
    // show: a space would otherwise silence an alarm halfway through a word.
    if matches!(
        state.screen,
        Screen::Notepad | Screen::KeepNotes | Screen::Inbox
    ) {
        return match key.as_ref() {
            // A task's details come away first; a second press leaves.
            Key::Named(Named::Escape)
                if state.screen == Screen::Inbox && state.calendar.open_event.is_some() =>
            {
                Some(Message::EventClosed)
            }
            Key::Named(Named::Escape) => Some(Message::ScreenSelected(Screen::Calendar)),
            _ => None,
        };
    }

    match key.as_ref() {
        // The clock no longer has a place on the calendar, so a space only
        // has something to do while an alarm is ringing.
        Key::Named(Named::Space) if state.alarm_active => Some(Message::Dismiss),
        Key::Named(Named::Escape) => {
            if state.calendar.open_event.is_some() {
                Some(Message::EventClosed)
            } else if state.timeblocking.block.is_some() {
                Some(Message::TimeBlocking(crate::timeblocking::Action::Close))
            } else if state.calendar.menu_open {
                state.calendar.menu_open = false;
                None
            } else if state.alarm_active {
                Some(Message::Dismiss)
            } else if state.screen != Screen::Calendar {
                Some(Message::ScreenSelected(Screen::Calendar))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Runs one alarm profile, for an alarm set on the calendar.
///
/// An alarm already ringing stays dismissable if a quieter one goes off on
/// top of it.
pub(crate) fn fire(
    state: &mut State,
    profile: &config::AlarmProfile,
) -> Task<Message> {
    let actions = alarm::on_finished(profile);

    state.alarm_active = state.alarm_active || actions.is_dismissable();

    perform(state, actions)
}

pub(crate) fn perform(
    state: &mut State,
    actions: alarm::Actions,
) -> Task<Message> {
    let mut tasks = Vec::new();

    if let Some(sound) = actions.sound {
        let spec = PlaySpec {
            gain: sound.gain,
            repeats: sound.repeats,
            fade_in: sound.fade_in,
        };
        let choice = sound.choice;

        // Rendering happens off the UI thread: synthesis is quick but
        // decoding a file the user picked is not necessarily.
        tasks.push(
            blocking::run(move || audio::render(&choice))
                .map(move |result| Message::SoundRendered(result, spec)),
        );
    }

    if let Some(notification) = actions.notification {
        tasks.push(
            blocking::run(move || notify::show(&notification))
                .map(Message::NotificationPosted),
        );
    }

    if let Some(settings) = actions.flash {
        tasks.push(open_flash(state, settings));
    }

    if actions.raise {
        if let Some(id) = state.main_window {
            tasks.push(window::gain_focus(id));
        }
    }

    Task::batch(tasks)
}

fn open_flash(state: &mut State, settings: config::Flash) -> Task<Message> {
    // Replace any flash still on screen rather than stacking a second one.
    let close_existing = state
        .flash
        .take()
        .map(|flash| window::close::<Message>(flash.window))
        .unwrap_or(Task::none());

    let passthrough = settings.passthrough;

    let (id, open) = window::open(window::Settings {
        fullscreen: true,
        transparent: true,
        decorations: false,
        resizable: false,
        level: window::Level::AlwaysOnTop,
        exit_on_close_request: false,
        platform_specific: window_platform(),
        ..Default::default()
    });

    state.flash = Some(Flash {
        window: id,
        started: Instant::now(),
        settings,
    });

    let configure = if passthrough {
        // Purely visual: pointer events fall through to whatever is
        // underneath, so the flash cannot swallow a click.
        window::enable_mouse_passthrough(id)
    } else {
        Task::none()
    };

    Task::batch([close_existing, open.discard(), configure])
}

/// Silences the alarm and takes the flash off the screen.
fn dismiss(state: &mut State) -> Task<Message> {
    state.alarm_active = false;
    state.audio.stop();

    match state.flash.take() {
        Some(flash) => window::close(flash.window),
        None => Task::none(),
    }
}

/// Marks the config dirty.
///
/// Dragging a slider produces a message per pixel; without the debounce that
/// would be a config write per pixel too. The flag turns on a subscription in
/// [`subscription`] that fires the write once things have settled, and edits
/// made while it is pending are all captured by that one write.
pub(crate) fn queue_save(state: &mut State) -> Task<Message> {
    state.save_queued = true;

    Task::none()
}

/// Re-syncs when the day in view has rolled over at the configured boundary.
///
/// The day ends at the configured hour, not at midnight, so a 3am boundary
/// keeps the small hours on the day that is ending. When the boundary passes,
/// the events in hand are for the wrong day and have to be pulled again.
///
/// Only the day that is following today rolls over; once the user has navigated
/// to another day, the boundary passing must not pull them back to today.
fn refresh_day(state: &mut State) -> Task<Message> {
    if !state.calendar.follow_today {
        return Task::none();
    }

    let day = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
    let displayed = if state.weekly && state.screen == Screen::Calendar {
        state.week_anchor
    } else {
        state.calendar.day
    };
    if day != displayed {
        state.calendar.set_day(day, day);
        if state.weekly {
            state.week_anchor = day;
        }
        return sync_calendar(state);
    }
    Task::none()
}

pub(crate) fn refresh_week_after_write(state: &mut State) -> Task<Message> {
    if state.screen == Screen::Calendar && state.weekly {
        state.timeblocking.revision += 1;
        state.calendar.clear_disk_cache();
        Task::done(Message::CalendarSyncRequested)
    } else {
        Task::none()
    }
}

fn reconcile_completed_task(state: &mut State) -> Task<Message> {
    let completed_key = state.config.task_work.active().and_then(|session| {
        state
            .calendar
            .events
            .iter()
            .any(|event| {
                event.is_task()
                    && event.completed
                    && crate::timeblocking::task_key(event) == session.task_key
            })
            .then(|| session.task_key.clone())
    });
    if let Some(key) = completed_key {
        let now = chrono::Utc::now();
        state.config.task_work.stop(&key, now);
        state.timeblocking.work_now = Some(now);
        queue_save(state)
    } else {
        Task::none()
    }
}

fn sync_calendar(state: &mut State) -> Task<Message> {
    if state.screen == Screen::Calendar && state.weekly {
        let days = calendar::week_dates(state.week_anchor, state.config.calendar.weekly_days());
        state.calendar.load_cached_days(&days);
    }
    if !state.calendar.is_signed_in()
        || state.calendar.is_busy()
        || !state.timeblocking.busy.is_empty()
    {
        return Task::none();
    }

    state.calendar.sync = SyncState::Syncing;
    state.calendar.error = None;

    let client_id = state.config.calendar.client_id.clone();
    let client_secret = state.config.calendar.client_secret.clone();
    let disabled = state.config.calendar.disabled.clone();
    let day = state.calendar.day;
    let day_end_hour = state.config.calendar.day_start_hour();

    let revision = state.timeblocking.revision;
    let tasks_calendar = state.config.calendar.tasks_calendar.clone();
    if state.screen == Screen::Calendar && state.weekly {
        let anchor = state.week_anchor;
        let days = state.config.calendar.weekly_days();
        return blocking::run(move || {
            calendar::sync_week(
                &client_id,
                &client_secret,
                &disabled,
                anchor,
                days,
                day_end_hour,
                tasks_calendar.as_deref(),
            )
        })
        .map(move |result| Message::CalendarWeekSynced(revision, anchor, days, result));
    }
    blocking::run(move || {
        calendar::sync_day(
            &client_id,
            &client_secret,
            &disabled,
            day,
            day_end_hour,
            tasks_calendar.as_deref(),
        )
    })
    .map(move |result| Message::CalendarSyncedAt(revision, result))
}

fn level(always_on_top: bool) -> window::Level {
    if always_on_top {
        window::Level::AlwaysOnTop
    } else {
        window::Level::Normal
    }
}

/// The width the clock opens at: two thirds of the monitor, but never so
/// narrow that the layout folds below the minimum it supports.
fn launch_width(monitor_width: f32) -> f32 {
    (monitor_width * 2.0 / 3.0).max(MIN_SIZE.width)
}

/// Listens on the panel IPC socket and forwards show/hide commands to the
/// main thread via `tx`. Runs in a background thread spawned by `boot_sidebar`.
fn panel_ipc_listener(socket_path: PathBuf, tx: mpsc::Sender<PanelCommand>) -> AnyhowResult<()> {
    // Remove any stale socket from a previous run.
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("failed to bind panel socket at {}", socket_path.display()))?;

    log::info!("panel IPC socket listening at {}", socket_path.display());

    for stream in listener.incoming() {
        let mut stream = stream.context("failed to accept panel IPC connection")?;
        let mut buffer = [0u8; 16];
        match stream.read(&mut buffer) {
            Ok(0) => continue, // client disconnected without sending
            Ok(n) => {
                let command = String::from_utf8_lossy(&buffer[..n]).trim().to_string();
                match command.as_str() {
                    "show" => {
                        let _ = tx.send(PanelCommand::Show);
                    }
                    "hide" => {
                        let _ = tx.send(PanelCommand::Hide);
                    }
                    _ => log::warn!("unknown panel IPC command: {command:?}"),
                }
            }
            Err(error) => log::warn!("failed to read panel IPC command: {error}"),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::futures::{StreamExt, executor::block_on};

    #[derive(Debug)]
    enum GeometryAction {
        Resize(Size),
        Min(Option<Size>),
        Max(Option<Size>),
        Open(window::Settings),
    }

    fn geometry_actions(task: Task<Message>) -> Vec<GeometryAction> {
        use iced_test::runtime::{Action, task, window::Action as WindowAction};
        block_on(async {
            let Some(mut stream) = task::into_stream(task) else {
                return Vec::new();
            };
            let mut actions = Vec::new();
            while let Some(action) = stream.next().await {
                match action {
                    Action::Window(WindowAction::Resize(_, size)) => {
                        actions.push(GeometryAction::Resize(size));
                    }
                    Action::Window(WindowAction::SetMinSize(_, size)) => {
                        actions.push(GeometryAction::Min(size));
                    }
                    Action::Window(WindowAction::SetMaxSize(_, size)) => {
                        actions.push(GeometryAction::Max(size));
                    }
                    Action::Window(WindowAction::GetMonitorSize(_, sender)) => {
                        let _ = sender.send(None);
                    }
                    Action::Window(WindowAction::Open(id, settings, sender)) => {
                        actions.push(GeometryAction::Open(settings));
                        let _ = sender.send(id);
                    }
                    _ => {}
                }
            }
            actions
        })
    }

    fn flash(settings: config::Flash) -> Flash {
        Flash {
            window: window::Id::unique(),
            started: Instant::now(),
            settings,
        }
    }

    #[test]
    fn external_config_updates_live_state_and_cancels_pending_save() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.save_queued = true;
        let mut external = state.config.clone();
        external.notepad.text = "edited outside the app".into();
        external.appearance.theme = "Ocean".into();
        external.calendar.day_end_hour = 3;
        let _ = update(&mut state, Message::ConfigReloaded(external.clone()));
        assert_eq!(state.config, external);
        assert_eq!(state.notepad.text(), "edited outside the app");
        assert!(!state.save_queued);
    }

    #[test]
    fn own_save_echo_does_not_undo_newer_unsaved_edits() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        let saved = state.disk_config.clone();
        state.config.notepad.text = "new unsaved text".into();
        state.save_queued = true;
        let _ = update(&mut state, Message::ConfigReloaded(saved));
        assert_eq!(state.config.notepad.text, "new unsaved text");
        assert!(state.save_queued);
    }

    #[test]
    fn the_window_icon_decodes() {
        assert!(window_icon().is_some(), "the embedded icon should decode");
    }

    #[test]
    fn the_flash_fades_in_and_out_within_a_pulse() {
        let flash = flash(config::Flash {
            pulses: 1,
            pulse_ms: 1_000,
            peak_opacity: 1.0,
            ..config::Flash::default()
        });

        let at = |millis| flash.alpha(flash.started + Duration::from_millis(millis));

        assert_eq!(at(0), Some(0.0));
        assert!(at(500).unwrap() > 0.99, "should peak mid-pulse");
        assert!(at(950).unwrap() < 0.05, "should fade back out");
    }

    #[test]
    fn the_flash_reports_when_it_is_over() {
        let flash = flash(config::Flash {
            pulses: 2,
            pulse_ms: 500,
            ..config::Flash::default()
        });

        let total = flash.settings.total_duration();
        assert_eq!(total, Duration::from_millis(1_000));

        assert!(
            flash
                .alpha(flash.started + total - Duration::from_millis(1))
                .is_some()
        );
        assert!(flash.alpha(flash.started + total).is_none());
    }

    #[test]
    fn a_flash_configured_faster_than_the_safety_cap_is_slowed_down() {
        let flash = flash(config::Flash {
            pulses: 2,
            pulse_ms: 20,
            ..config::Flash::default()
        });

        // The config asked for 50 pulses a second; it gets three.
        assert_eq!(
            flash.settings.total_duration(),
            config::Flash::MIN_PULSE * 2
        );
    }

    #[test]
    fn the_flash_never_exceeds_the_configured_peak() {
        let flash = flash(config::Flash {
            pulses: 3,
            peak_opacity: 0.35,
            ..config::Flash::default()
        });

        for millis in (0..3_000).step_by(7) {
            if let Some(alpha) = flash.alpha(flash.started + Duration::from_millis(millis)) {
                assert!((0.0..=0.35).contains(&alpha), "alpha {alpha} out of range");
            }
        }
    }

    #[test]
    fn a_zero_opacity_flash_draws_nothing() {
        let flash = flash(config::Flash {
            peak_opacity: 0.0,
            ..config::Flash::default()
        });

        assert_eq!(
            flash.alpha(flash.started + Duration::from_millis(100)),
            Some(0.0)
        );
    }

    #[test]
    fn showing_tasks_is_remembered() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        assert!(state.config.calendar.show_tasks);

        let _ = update(&mut state, Message::ShowTasksToggled(false));

        assert!(!state.config.calendar.show_tasks);
        assert!(state.save_queued);
    }

    #[test]
    fn the_notepad_keeps_what_was_written() {
        let mut state = State::new(Config::default(), Catalog::built_in());

        for letter in "milk".chars() {
            let _ = update(
                &mut state,
                Message::NotepadEdited(text_editor::Action::Edit(text_editor::Edit::Insert(
                    letter,
                ))),
            );
        }

        assert_eq!(state.config.notepad.text, "milk");
        assert!(state.save_queued);
    }

    #[test]
    fn the_notepad_opens_on_what_was_written_last_time() {
        let mut config = Config::default();
        config.notepad.text = "remember the milk".to_owned();

        let state = State::new(config, Catalog::built_in());

        assert_eq!(state.notepad.text(), "remember the milk");
    }

    #[test]
    fn the_notepad_text_size_is_remembered() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        assert_eq!(state.config.notepad.font_size, 100);

        let _ = update(&mut state, Message::NotepadFontSizeChanged(10));
        assert_eq!(state.config.notepad.font_size, 110);

        let _ = update(&mut state, Message::NotepadFontSizeChanged(-20));
        assert_eq!(state.config.notepad.font_size, 90);
        assert!(state.save_queued);
    }

    #[test]
    fn the_notepad_text_size_stops_at_the_ends_of_its_range() {
        let mut state = State::new(Config::default(), Catalog::built_in());

        for _ in 0..40 {
            let _ = update(&mut state, Message::NotepadFontSizeChanged(10));
        }
        assert_eq!(
            state.config.notepad.font_size,
            config::Notepad::MAX_FONT_SIZE
        );

        for _ in 0..40 {
            let _ = update(&mut state, Message::NotepadFontSizeChanged(-10));
        }
        assert_eq!(
            state.config.notepad.font_size,
            config::Notepad::MIN_FONT_SIZE
        );
    }

    #[test]
    fn the_notepad_is_a_tab_of_its_own() {
        let mut state = State::new(Config::default(), Catalog::built_in());

        let _ = update(&mut state, Message::ScreenSelected(Screen::Notepad));
        assert_eq!(state.screen, Screen::Notepad);

        let _ = update(&mut state, Message::ScreenSelected(Screen::Calendar));
        assert_eq!(state.screen, Screen::Calendar);
    }

    #[test]
    fn the_clock_shortcuts_are_left_off_while_writing() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.screen = Screen::Notepad;

        let space = key_message(
            &mut state,
            Key::Named(keyboard::key::Named::Space),
            keyboard::Modifiers::default(),
        );
        let reset = key_message(
            &mut state,
            Key::Character("r".into()),
            keyboard::Modifiers::default(),
        );

        assert!(space.is_none(), "a space would start the clock mid-word");
        assert!(reset.is_none(), "an r would reset the clock mid-word");

        // Escape still leaves the page.
        let escape = key_message(
            &mut state,
            Key::Named(keyboard::key::Named::Escape),
            keyboard::Modifiers::default(),
        );
        assert!(
            matches!(escape, Some(Message::ScreenSelected(Screen::Calendar))),
            "escape should leave the notepad"
        );
    }

    #[test]
    fn a_space_on_the_calendar_only_silences_a_ringing_alarm() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        let space = |state: &mut State| {
            key_message(
                state,
                Key::Named(keyboard::key::Named::Space),
                keyboard::Modifiers::default(),
            )
        };

        assert!(
            space(&mut state).is_none(),
            "there is no clock on the calendar for a space to start"
        );

        state.alarm_active = true;

        assert!(matches!(space(&mut state), Some(Message::Dismiss)));
    }

    #[test]
    fn changing_the_end_of_day_moves_the_day_in_view() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        assert_eq!(state.config.calendar.day_end_hour, 0);

        let _ = update(&mut state, Message::CalendarDayEndChanged(3));

        assert_eq!(state.config.calendar.day_end_hour, 3);
        assert!(state.save_queued);
        // The day in view is recomputed from the new boundary, not left on the
        // date the old one produced.
        assert_eq!(state.calendar.day, calendar::day_at(Local::now(), 3));
    }

    #[test]
    fn an_out_of_range_end_of_day_is_clamped() {
        let mut state = State::new(Config::default(), Catalog::built_in());

        let _ = update(&mut state, Message::CalendarDayEndChanged(99));

        assert_eq!(
            state.config.calendar.day_end_hour,
            config::CalendarSettings::MAX_DAY_END_HOUR
        );
    }

    #[test]
    fn picking_a_task_color_recolors_the_list_and_its_events() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());

        // The sample uses a shortened id; a real task list is `tasks:<list>`.
        state.calendar.calendars[2].id = "tasks:list".into();
        let deep = state
            .calendar
            .events
            .iter_mut()
            .find(|event| event.id == "deep")
            .unwrap();
        deep.calendar_id = "tasks:list".into();

        let red = iced::Color::from_rgb8(0xDC, 0x21, 0x27);
        let _ = update(
            &mut state,
            Message::TaskColorPicked("tasks:list".into(), Some(red)),
        );

        assert_eq!(state.config.calendar.task_color("tasks:list"), Some(red));
        assert!(state.save_queued);
        assert_eq!(state.calendar.calendars[2].color, red);
        assert_eq!(
            state.calendar.event("deep").unwrap().color,
            red,
            "the task's own chip should follow the list color"
        );

        // Resetting has to repaint the chip, not just forget the override.
        let _ = update(
            &mut state,
            Message::TaskColorPicked("tasks:list".into(), None),
        );
        assert_eq!(state.config.calendar.task_color("tasks:list"), None);
        assert_eq!(
            state.calendar.event("deep").unwrap().color,
            calendar::colors::TASK
        );
    }

    #[test]
    fn the_task_color_editor_opens_and_closes() {
        let mut state = State::new(Config::default(), Catalog::built_in());

        let _ = update(
            &mut state,
            Message::TaskColorEditorToggled("tasks:list".into()),
        );
        assert_eq!(state.calendar.color_editor.as_deref(), Some("tasks:list"));

        let _ = update(
            &mut state,
            Message::TaskColorEditorToggled("tasks:list".into()),
        );
        assert_eq!(state.calendar.color_editor, None);
    }

    #[test]
    fn the_window_opens_at_two_thirds_of_the_monitor() {
        assert_eq!(launch_width(1920.0), 1280.0);
        assert_eq!(launch_width(3840.0), 2560.0);
        // A tiny monitor must not fold the layout below its minimum.
        assert_eq!(launch_width(120.0), MIN_SIZE.width);
    }

    #[test]
    fn the_panel_window_is_pinned_to_its_width() {
        let panel = panel::PanelConfig {
            width: 200,
            ..Default::default()
        };
        let settings = sidebar_settings(&panel, &config::ViewSettings::default());

        // The width is fixed from both sides, so the compositor cannot stretch
        // the strip, but the height is free to fill the screen.
        assert_eq!(settings.min_size.unwrap().width, 208.0);
        assert_eq!(settings.max_size.unwrap().width, 208.0);
        assert!(settings.min_size.unwrap().height < settings.max_size.unwrap().height);
        assert!(!settings.decorations);
        assert!(settings.transparent);
        assert_eq!(settings.level, window::Level::AlwaysOnTop);
        assert_eq!(settings.platform_specific.application_id, APPLICATION_ID);
    }

    #[test]
    fn weekly_width_includes_every_column_padding_margin_and_monitor_limit() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.config.calendar.weekly_days = 7;
        state.config.calendar.weekly_day_width = 100;
        assert_eq!(state.weekly_window_width(), 828.0);
        state.config.calendar.weekly_days = 8;
        state.config.calendar.weekly_day_width = 220;
        assert_eq!(state.weekly_window_width(), 2008.0);
        state.sidebar = Some(Sidebar {
            width: 200.0,
            side: panel::Side::Left,
        });
        assert_eq!(state.weekly_window_width(), 2016.0);
        state.monitor_size = Some(Size::new(1200.0, 900.0));
        assert_eq!(state.weekly_window_width(), 1200.0);
        state.monitor_size = Some(Size::new(f32::NAN, 900.0));
        assert_eq!(state.weekly_window_width(), 2016.0);
        state.sidebar = None;
        state.monitor_size = None;
        state.config.window.width = 3000.0;
        assert_eq!(state.weekly_window_width(), 3000.0);
    }

    #[test]
    fn weekly_sidebar_relaxes_constraints_and_restores_daily_width() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.main_window = Some(window::Id::unique());
        state.sidebar = Some(Sidebar {
            width: 200.0,
            side: panel::Side::Left,
        });
        state.main_size = Size::new(208.0, 900.0);

        let actions = geometry_actions(update(&mut state, Message::CalendarModeSelected(true)));
        assert!(matches!(actions.as_slice(), [
            GeometryAction::Max(None),
            GeometryAction::Min(Some(min)),
            GeometryAction::Resize(size),
            GeometryAction::Max(Some(max)),
        ] if min.width < 208.0 && size.width == 1386.0 && max.width == size.width));

        let actions = geometry_actions(update(&mut state, Message::CalendarModeSelected(false)));
        assert!(matches!(actions.as_slice(), [
            GeometryAction::Max(None),
            GeometryAction::Min(Some(min)),
            GeometryAction::Resize(size),
            GeometryAction::Max(Some(max)),
        ] if min.width == 208.0 && size.width == 208.0 && max.width == 208.0));
    }

    #[test]
    fn weekly_does_not_replace_the_saved_daily_width() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        let id = window::Id::unique();
        state.main_window = Some(id);
        let _ = update(
            &mut state,
            Message::WindowResized(id, Size::new(640.0, 700.0)),
        );
        let _ = geometry_actions(update(&mut state, Message::CalendarModeSelected(true)));
        let _ = update(
            &mut state,
            Message::WindowResized(id, Size::new(1378.0, 700.0)),
        );
        assert_eq!(state.config.window.width, 640.0);

        let actions = geometry_actions(update(&mut state, Message::CalendarModeSelected(false)));
        assert!(
            matches!(actions.as_slice(), [GeometryAction::Resize(size)] if size.width == 640.0)
        );
        // A delayed notification from the expanded window can arrive after switching.
        let _ = update(
            &mut state,
            Message::WindowResized(id, Size::new(1378.0, 700.0)),
        );
        let _ = update(
            &mut state,
            Message::WindowResized(id, Size::new(1200.0, 700.0)),
        );
        assert_eq!(state.config.window.width, 640.0);
        let _ = update(
            &mut state,
            Message::WindowResized(id, Size::new(640.0, 700.0)),
        );
        assert_eq!(state.config.window.width, 640.0);
        let _ = update(
            &mut state,
            Message::WindowResized(id, Size::new(720.0, 700.0)),
        );
        assert_eq!(state.config.window.width, 720.0);
    }

    #[test]
    fn leaving_weekly_at_an_unchanged_width_does_not_discard_a_later_manual_resize() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        let id = window::Id::unique();
        state.main_window = Some(id);
        let _ = update(
            &mut state,
            Message::WindowResized(id, Size::new(1600.0, 700.0)),
        );
        let _ = geometry_actions(update(&mut state, Message::CalendarModeSelected(true)));
        let _ = geometry_actions(update(&mut state, Message::CalendarModeSelected(false)));
        let _ = update(
            &mut state,
            Message::WindowResized(id, Size::new(1550.0, 700.0)),
        );
        assert_eq!(state.config.window.width, 1550.0);
    }

    #[test]
    fn a_clamped_restore_does_not_prevent_later_manual_resizes() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        let id = window::Id::unique();
        state.main_window = Some(id);
        state.config.window.width = 1200.0;
        state.weekly_width_restoring = Some((1200.0, Instant::now() - Duration::from_secs(2)));
        let _ = update(
            &mut state,
            Message::WindowResized(id, Size::new(900.0, 700.0)),
        );
        assert_eq!(state.config.window.width, 900.0);
    }

    #[test]
    fn weekly_resizes_on_tab_return_and_external_settings_changes() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.main_window = Some(window::Id::unique());
        let _ = geometry_actions(update(&mut state, Message::CalendarModeSelected(true)));
        let actions = geometry_actions(update(
            &mut state,
            Message::ScreenSelected(Screen::Settings),
        ));
        assert!(
            matches!(actions.as_slice(), [GeometryAction::Resize(size)] if size.width == state.calendar_width())
        );
        let _ = update(&mut state, Message::CalendarWeeklyDaysChanged(7));
        let _ = update(&mut state, Message::CalendarWeeklyWidthChanged(200));
        let actions = geometry_actions(update(
            &mut state,
            Message::ScreenSelected(Screen::Calendar),
        ));
        assert!(
            matches!(actions.as_slice(), [GeometryAction::Resize(size)] if size.width == 1628.0)
        );

        let mut external = state.config.clone();
        external.calendar.weekly_day_width = 250;
        let actions = geometry_actions(update(&mut state, Message::ConfigReloaded(external)));
        assert!(
            matches!(actions.as_slice(), [GeometryAction::Resize(size)] if size.width == 2028.0)
        );
    }

    #[test]
    fn weekly_sidebar_reopens_wide_and_respects_monitor_responses() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.sidebar = Some(Sidebar {
            width: 200.0,
            side: panel::Side::Left,
        });
        state.panel_config = Some(panel::PanelConfig::default());
        state.weekly = true;
        state.monitor_size = Some(Size::new(1000.0, 900.0));
        let (sender, receiver) = mpsc::channel();
        state.panel_command_rx = Some(receiver);
        sender.send(PanelCommand::Show).unwrap();
        let actions = geometry_actions(update(&mut state, Message::PanelCommandPolled));
        assert!(
            matches!(actions.as_slice(), [GeometryAction::Open(settings)]
            if settings.size.width == 1000.0
                && settings.min_size.unwrap().width < 1000.0
                && settings.max_size.unwrap().width == 1000.0)
        );

        let id = window::Id::unique();
        state.main_window = Some(id);
        let actions = geometry_actions(update(
            &mut state,
            Message::MonitorSized(id, Some(Size::new(1100.0, 800.0))),
        ));
        assert!(matches!(actions.as_slice(), [
            GeometryAction::Max(None), GeometryAction::Min(_),
            GeometryAction::Resize(size), GeometryAction::Max(_)
        ] if *size == Size::new(1100.0, 800.0)));
        let actions = geometry_actions(update(&mut state, Message::WeekMonitorSized(id, None)));
        assert!(matches!(actions.as_slice(), [
            GeometryAction::Max(None), GeometryAction::Min(Some(min)),
            GeometryAction::Resize(size), GeometryAction::Max(_)
        ] if size.width == 1386.0 && min.width < 1000.0));

        let _ = geometry_actions(update(&mut state, Message::CalendarModeSelected(false)));
        let actions = geometry_actions(update(
            &mut state,
            Message::WeekMonitorSized(id, Some(Size::new(1000.0, 800.0))),
        ));
        assert!(actions.is_empty());
    }

    #[test]
    fn entering_weekly_closes_daily_panes_before_the_final_resize() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.main_window = Some(window::Id::unique());
        state.sidebar = Some(Sidebar {
            width: 200.0,
            side: panel::Side::Left,
        });
        state.timeblocking.original_width = Some(208.0);
        state.details_pane_open = true;
        state.calendar.open_event = Some("event".into());
        let actions = geometry_actions(update(&mut state, Message::CalendarModeSelected(true)));
        let widths: Vec<_> = actions
            .iter()
            .filter_map(|action| match action {
                GeometryAction::Resize(size) => Some(size.width),
                _ => None,
            })
            .collect();
        assert_eq!(widths.last(), Some(&1386.0));
        assert!(!state.details_pane_open);
        assert_eq!(state.timeblocking.original_width, None);
    }

    #[test]
    fn the_panel_pins_to_the_edge_it_was_asked_for() {
        let window = Size::new(200.0, 1080.0);
        let monitor = Size::new(1920.0, 1080.0);

        assert_eq!(left_edge(window, monitor), Point::new(0.0, 0.0));
        assert_eq!(right_edge(window, monitor), Point::new(1720.0, 0.0));
    }

    #[test]
    fn the_panel_does_not_widen_to_the_default_fraction() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        let id = window::Id::unique();
        state.main_window = Some(id);
        state.sidebar = Some(Sidebar {
            width: 200.0,
            side: panel::Side::Left,
        });

        let before = state.config.window.width;
        let _ = update(
            &mut state,
            Message::MonitorSized(id, Some(Size::new(1920.0, 1080.0))),
        );

        // The main window would have adopted `launch_width` (1280px) here; the
        // panel keeps the width it was told to.
        assert_eq!(state.config.window.width, before);
    }

    #[test]
    fn the_panel_does_not_write_its_geometry_over_the_window() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        let id = window::Id::unique();
        state.main_window = Some(id);
        state.sidebar = Some(Sidebar {
            width: 200.0,
            side: panel::Side::Left,
        });

        let before = state.config.window.clone();
        let _ = update(
            &mut state,
            Message::WindowResized(id, Size::new(200.0, 1080.0)),
        );

        assert_eq!(
            state.config.window, before,
            "the panel must leave the saved window alone"
        );
    }

    #[test]
    fn a_main_window_still_remembers_its_own_size() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        let id = window::Id::unique();
        state.main_window = Some(id);

        let _ = update(
            &mut state,
            Message::WindowResized(id, Size::new(640.0, 480.0)),
        );

        assert_eq!(state.config.window.width, 640.0);
        assert_eq!(state.config.window.height, 480.0);
    }

    #[test]
    fn opening_an_event_in_the_top_half_puts_the_details_below() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        ui::calendar::record_pointer(20.0, 500.0);

        let _ = update(&mut state, Message::EventOpened("standup".into()));

        assert_eq!(state.calendar.open_event.as_deref(), Some("standup"));
        assert!(state.calendar.details_below);
    }

    #[test]
    fn opening_an_event_in_the_bottom_half_keeps_the_details_on_top() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        ui::calendar::record_pointer(480.0, 500.0);

        let _ = update(&mut state, Message::EventOpened("lunch".into()));

        assert_eq!(state.calendar.open_event.as_deref(), Some("lunch"));
        assert!(!state.calendar.details_below);
    }

    #[test]
    fn escape_closes_the_event_details_pane() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar.open_event = Some("standup".into());

        let escape = key_message(
            &mut state,
            Key::Named(keyboard::key::Named::Escape),
            keyboard::Modifiers::default(),
        );

        assert!(
            matches!(escape, Some(Message::EventClosed)),
            "escape should close the details pane, got {escape:?}"
        );
    }

    #[test]
    fn stepping_the_day_moves_the_view_and_drops_the_old_events() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
        let start = state.calendar.day;

        let _ = update(&mut state, Message::CalendarDayShift(1));

        assert_eq!(state.calendar.day, start.succ_opt().unwrap());
        assert!(
            state.calendar.events.is_empty(),
            "the old day's events must not paint onto the new day"
        );
        assert!(!state.calendar.follow_today);
    }

    #[test]
    fn stepping_back_to_a_day_paints_its_cached_events_at_once() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
        let start = state.calendar.day;
        let cached = state.calendar.events.len();
        assert!(cached > 0);

        let _ = update(&mut state, Message::CalendarDayShift(1));
        assert!(
            state.calendar.events.is_empty(),
            "a day not seen yet starts empty"
        );

        let _ = update(&mut state, Message::CalendarDayShift(-1));

        assert_eq!(state.calendar.day, start);
        assert_eq!(
            state.calendar.events.len(),
            cached,
            "the day we came back to should paint from the cache"
        );
    }

    #[test]
    fn selecting_today_makes_the_view_follow_it_again() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        let today = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
        state.calendar.day = today;
        state.calendar.follow_today = false;

        let _ = update(&mut state, Message::CalendarDaySelected(today));

        assert!(state.calendar.follow_today);
    }

    #[test]
    fn the_month_picker_opens_on_the_month_of_the_day_in_view() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar.day = chrono::NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();

        let _ = update(&mut state, Message::CalendarMonthToggled);

        assert!(state.calendar.month_open);
        assert_eq!(
            state.calendar.month,
            chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
        );

        let _ = update(&mut state, Message::CalendarMonthToggled);
        assert!(!state.calendar.month_open);
    }

    #[test]
    fn stepping_the_month_crosses_the_year_boundary() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar.month = chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();

        let _ = update(&mut state, Message::CalendarMonthShift(1));
        assert_eq!(
            state.calendar.month,
            chrono::NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
        );

        let _ = update(&mut state, Message::CalendarMonthShift(-2));
        assert_eq!(
            state.calendar.month,
            chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
        );
    }

    #[test]
    fn picking_a_day_closes_the_month_picker_and_moves_the_view() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar.month_open = true;
        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();

        let _ = update(&mut state, Message::CalendarDaySelected(day));

        assert_eq!(state.calendar.day, day);
        assert!(!state.calendar.month_open);
    }

    #[test]
    fn a_snapshot_for_a_day_we_left_does_not_snap_the_view_back() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
        let moved_to = state.calendar.day.succ_opt().unwrap();
        state.calendar.day = moved_to;
        state.calendar.sync = SyncState::Syncing;

        // The sample snapshot is for the day we navigated away from.
        let _ = update(
            &mut state,
            Message::CalendarSynced(Ok(calendar::sample_snapshot())),
        );

        assert_eq!(state.calendar.day, moved_to);
    }

    #[test]
    fn resync_forgets_cached_days() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
        // Stay signed out so the fetch is not started against a real account.
        // The cache drop still has to happen; a signed-in store would also
        // move into Syncing, which `sync_calendar` owns.
        state.calendar.connection = Connection::SignedOut;
        let today = state.calendar.day;
        let tomorrow = today.succ_opt().unwrap();
        state.calendar.set_day(tomorrow, today);

        let _ = update(&mut state, Message::CalendarResyncRequested);

        state.calendar.set_day(today, today);
        assert!(
            state.calendar.events.is_empty(),
            "a resync should forget days that were only cached"
        );
        assert_eq!(state.calendar.sync, SyncState::Idle);
    }

    fn at_in(minutes: i64) -> DateTime<Local> {
        alarms::minute_of(Local::now()) + chrono::Duration::minutes(minutes)
    }

    fn place(state: &mut State, minutes: i64) {
        let _ = update(
            state,
            Message::Alarms(alarms::Action::Place(at_in(minutes))),
        );
    }

    #[test]
    fn set_alarm_enters_and_leaves_alarm_mode() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar.menu_open = true;

        let _ = update(&mut state, Message::Alarms(alarms::Action::ModeToggled));

        assert!(state.alarms.mode);
        assert!(!state.calendar.menu_open, "the menu gets out of the way");

        let _ = update(&mut state, Message::Alarms(alarms::Action::ModeToggled));

        assert!(!state.alarms.mode);
    }

    #[test]
    fn escape_leaves_alarm_mode() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.alarms.mode = true;

        let escape = key_message(
            &mut state,
            Key::Named(keyboard::key::Named::Escape),
            keyboard::Modifiers::default(),
        );

        assert!(matches!(
            escape,
            Some(Message::Alarms(alarms::Action::ModeToggled))
        ));
    }

    #[test]
    fn clicking_a_future_time_sets_the_alarm_and_opens_its_options() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.alarms.mode = true;

        place(&mut state, 45);

        assert_eq!(state.alarms.items().len(), 1);
        assert!(!state.alarms.mode, "placing an alarm ends the mode");

        let editor = state.alarms.editor.as_ref().expect("the popup opens");
        assert_eq!(editor.id, state.alarms.items()[0].id);
        assert_eq!(
            alarms::minutes_between(Local::now(), state.alarms.items()[0].at),
            45
        );
    }

    #[test]
    fn an_alarm_cannot_be_set_in_the_past() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.alarms.mode = true;

        place(&mut state, -30);
        place(&mut state, 0);

        assert!(state.alarms.is_empty());
        assert!(state.alarms.editor.is_none());
    }

    #[test]
    fn a_new_alarm_starts_from_the_alarm_profile_without_the_timer_wording() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.config.alarm.sound.volume = 33;

        place(&mut state, 10);

        let alarm = &state.alarms.items()[0];
        assert_eq!(alarm.profile.sound.volume, 33);
        assert_eq!(alarm.profile.notification.summary, "Alarm");
    }

    #[test]
    fn opening_an_alarm_shows_its_time_and_closing_puts_it_away() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        place(&mut state, 90);
        let id = state.alarms.items()[0].id;

        let _ = update(&mut state, Message::Alarms(alarms::Action::CloseEditor));
        assert!(state.alarms.editor.is_none());

        let _ = update(&mut state, Message::Alarms(alarms::Action::Open(id)));
        assert_eq!(
            state.alarms.editor.as_ref().map(|e| e.time_text.clone()),
            Some(state.alarms.items()[0].at.format("%H:%M").to_string())
        );
    }

    #[test]
    fn an_alarm_can_be_removed_before_it_goes_off() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        place(&mut state, 60);
        place(&mut state, 120);
        let id = state
            .alarms
            .editor
            .as_ref()
            .expect("the last one is open")
            .id;

        let _ = update(&mut state, Message::Alarms(alarms::Action::Delete(id)));

        assert_eq!(state.alarms.items().len(), 1);
        assert!(state.alarms.get(id).is_none());
        assert!(state.alarms.editor.is_none(), "its popup goes with it");
    }

    #[test]
    fn editing_an_alarm_changes_what_it_does_and_nothing_else() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        place(&mut state, 60);
        let before = state.alarms.items()[0].at;

        for edit in [
            alarms::Edit::Label("Tea".into()),
            alarms::Edit::Sound(false),
            alarms::Edit::Volume(12),
            alarms::Edit::Flash(false),
        ] {
            let _ = update(&mut state, Message::Alarms(alarms::Action::Edit(edit)));
        }

        let alarm = &state.alarms.items()[0];
        assert_eq!(alarm.label, "Tea");
        assert!(!alarm.profile.sound.enabled);
        assert_eq!(alarm.profile.sound.volume, 12);
        assert!(!alarm.profile.flash.enabled);
        assert_eq!(alarm.at, before);
        assert_eq!(
            state.config.alarm,
            Config::default().alarm,
            "defaults stay put"
        );
    }

    #[test]
    fn an_alarm_time_can_be_edited_but_not_into_the_past() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        place(&mut state, 24 * 60 + 30);
        let alarm = state.alarms.items()[0].clone();

        // The same time of day, tomorrow, is still ahead.
        let later = (alarm.at + chrono::Duration::minutes(5))
            .format("%H:%M")
            .to_string();
        let _ = update(
            &mut state,
            Message::Alarms(alarms::Action::Edit(alarms::Edit::Time(later.clone()))),
        );
        assert_eq!(
            state.alarms.items()[0].at.format("%H:%M").to_string(),
            later
        );
        assert!(state.alarms.editor.as_ref().unwrap().time_error.is_none());

        // Nonsense is refused and the alarm stays where it was.
        let kept = state.alarms.items()[0].at;
        let _ = update(
            &mut state,
            Message::Alarms(alarms::Action::Edit(alarms::Edit::Time("soon".into()))),
        );
        assert_eq!(state.alarms.items()[0].at, kept);
        assert!(state.alarms.editor.as_ref().unwrap().time_error.is_some());
    }

    #[test]
    fn an_alarm_for_today_cannot_be_edited_into_the_past() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        place(&mut state, 2);
        let alarm = state.alarms.items()[0].clone();
        if alarm.at.date_naive() != Local::now().date_naive() {
            return; // set a hair before midnight, so it landed on tomorrow
        }

        // Midnight has always passed by the time today has an alarm in it.
        let _ = update(
            &mut state,
            Message::Alarms(alarms::Action::Edit(alarms::Edit::Time("00:00".into()))),
        );

        assert_eq!(
            state.alarms.items()[0].at,
            alarm.at,
            "the alarm did not move"
        );
        assert!(state.alarms.editor.as_ref().unwrap().time_error.is_some());
    }

    #[test]
    fn an_alarm_that_comes_due_goes_off_and_leaves_the_calendar() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        place(&mut state, 60);
        state.alarms.add(
            Local::now() - chrono::Duration::seconds(1),
            config::AlarmProfile::default(),
        );
        assert_eq!(state.alarms.items().len(), 2);

        let _ = update(&mut state, Message::Alarms(alarms::Action::Poll));

        assert_eq!(state.alarms.items().len(), 1, "only the due one went");
        assert!(state.alarm_active, "it rings until dismissed");
    }

    #[test]
    fn an_alarm_going_off_closes_its_own_popup() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        let id = state.alarms.add(
            Local::now() - chrono::Duration::seconds(1),
            config::AlarmProfile::default(),
        );
        let _ = update(&mut state, Message::Alarms(alarms::Action::Open(id)));
        assert!(state.alarms.editor.is_some());

        let _ = update(&mut state, Message::Alarms(alarms::Action::Poll));

        assert!(state.alarms.editor.is_none());
    }

    #[test]
    fn leaving_the_calendar_ends_alarm_setting() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.alarms.mode = true;

        let _ = update(&mut state, Message::ScreenSelected(Screen::Notepad));

        assert!(!state.alarms.mode);
    }

    #[test]
    fn the_alarm_poll_only_runs_while_an_alarm_is_waiting() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        assert!(!alarms::needs_poll(&state.alarms));

        place(&mut state, 5);

        assert!(alarms::needs_poll(&state.alarms));
    }

    /// A state with the standup already dragged an hour later and pending.
    fn state_with_a_pending_drag() -> (State, DateTime<Local>, DateTime<Local>) {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());

        let event = state.calendar.event("standup").unwrap();
        let start = event.start + chrono::Duration::hours(1);
        let end = event.end + chrono::Duration::hours(1);
        state.calendar.begin_time_change("standup", start, end);
        (state, start, end)
    }

    #[test]
    fn a_confirmed_drag_makes_the_chip_final() {
        let (mut state, start, _) = state_with_a_pending_drag();

        let _ = update(
            &mut state,
            Message::EventTimeSaved("standup".into(), Ok(())),
        );

        assert!(state.calendar.pending_time("standup").is_none());
        assert_eq!(state.calendar.event("standup").unwrap().start, start);
    }

    #[test]
    fn a_refused_drag_puts_the_old_time_back_and_says_so() {
        let (mut state, start, _) = state_with_a_pending_drag();
        let before = calendar::sample_snapshot()
            .events
            .into_iter()
            .find(|event| event.id == "standup")
            .unwrap()
            .start;

        let _ = update(
            &mut state,
            Message::EventTimeSaved("standup".into(), Err("Google Calendar: nope".into())),
        );

        assert!(state.calendar.pending_time("standup").is_none());
        assert_eq!(state.calendar.event("standup").unwrap().start, before);
        assert_ne!(state.calendar.event("standup").unwrap().start, start);
        assert_eq!(
            state.calendar.error.as_deref(),
            Some("Google Calendar: nope")
        );
    }

    #[test]
    fn a_task_time_change_uses_task_scheduling_instead_of_a_calendar_event_write() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());

        // The sample's `deep` event stands in for a Google Task.
        let deep = state.calendar.event_mut("deep").unwrap();
        deep.id = "task:deep".into();
        deep.calendar_id = "tasks:list".into();
        let start = deep.start + chrono::Duration::hours(1);
        let end = deep.end + chrono::Duration::hours(1);

        let _ = update(
            &mut state,
            Message::EventTimeChanged {
                id: "task:deep".into(),
                start,
                end,
            },
        );

        assert!(state.calendar.pending_time("task:deep").is_none());
        assert!(state.timeblocking.busy.contains("task:deep"));
        assert_ne!(state.calendar.event("task:deep").unwrap().start, start);
    }
    #[test]
    fn estimate_popup_takes_keyboard_priority_over_the_time_block_view() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.timeblocking.focus = Some("standup".into());
        state.timeblocking.estimate_task = Some("task:deep".into());
        assert!(
            key_message(
                &mut state,
                Key::Named(keyboard::key::Named::Space),
                keyboard::Modifiers::default()
            )
            .is_none()
        );
        assert!(matches!(
            key_message(
                &mut state,
                Key::Named(keyboard::key::Named::Escape),
                keyboard::Modifiers::default()
            ),
            Some(Message::TimeBlocking(
                crate::timeblocking::Action::CloseEstimate
            ))
        ));
    }

    #[test]
    fn keep_search_does_not_trigger_timer_shortcuts() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.screen = Screen::KeepNotes;
        assert!(
            key_message(
                &mut state,
                Key::Named(keyboard::key::Named::Space),
                keyboard::Modifiers::default()
            )
            .is_none()
        );
        assert!(matches!(
            key_message(
                &mut state,
                Key::Named(keyboard::key::Named::Escape),
                keyboard::Modifiers::default()
            ),
            Some(Message::ScreenSelected(Screen::Calendar))
        ));
    }

    #[test]
    fn keep_failed_refresh_preserves_last_snapshot_and_filters() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.keep.snapshot.notes.push(crate::keep::Note {
            title: "Saved".into(),
            ..Default::default()
        });
        state.keep.query = "saved".into();
        state.keep.loading = true;
        let _ = update(&mut state, Message::KeepLoaded(Err("Reconnect".into())));
        assert!(!state.keep.loading);
        assert_eq!(state.keep.snapshot.notes[0].title, "Saved");
        assert_eq!(state.keep.query, "saved");
        assert_eq!(state.keep.error.as_deref(), Some("Reconnect"));
    }
}

fn refresh_keep(state: &mut State) -> Task<Message> {
    if state.keep.loading {
        return Task::none();
    }
    state.keep.loading = true;
    state.keep.error = None;
    let cache_path = state.keep.cache_path();
    blocking::run(move || crate::keep::fetch(cache_path)).map(Message::KeepLoaded)
}
