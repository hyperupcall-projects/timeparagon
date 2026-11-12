//! State, messages and the update loop.

use crate::calendar::time_format::TimeFormat;
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use anyhow::Result as AnyhowResult;

use iced::keyboard::{self, Key};
use iced::widget::text_editor;
use iced::{Element, Size, Subscription, Task, Theme, window};

use chrono::{DateTime, Local};

use crate::audio::{self, Audio, Pcm};
use crate::calendar::{self, Connection, SyncState};
use crate::config::{self, Config};
use crate::theme::Catalog;
use crate::{alarm, alarms, blocking, notify, panel, theme, timeblocking, ui};

mod on_calendar;
mod on_event;
mod on_event_draft;
mod on_event_edit;
mod on_navigation;
mod on_notepad;
mod on_settings;
mod on_system;
mod on_task;
#[cfg(test)]
mod tests;

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

/// Which span of time the Calendar screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CalendarView {
    #[default]
    Day,
    Week,
    TwoWeek,
    Month,
    Schedule,
}

/// The sidebar shown beside a planning overview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanTab {
    Schedule,
    Overdue,
    Inbox,
    Today,
    Backlog,
}

/// The two month pickers: the day panel's and the range overview's. Each opens,
/// steps and picks on its own, so using one leaves the other where it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonthPicker {
    Panel,
    Overview,
}

/// The boxes in the shared border settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderLayer {
    Outer,
    Inner,
}

/// The multi-line text fields of the event and habit forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    EventDraft,
    EventEdit,
    HabitNotes,
}

/// One editor per [`FormField`]. Each keeps its cursor and selection between
/// redraws, while the form's draft holds the saved copy of the text.
pub struct FormEditors {
    pub event_draft: text_editor::Content,
    pub event_edit: text_editor::Content,
    pub habit_notes: text_editor::Content,
}

impl FormEditors {
    fn new() -> Self {
        Self {
            event_draft: text_editor::Content::new(),
            event_edit: text_editor::Content::new(),
            habit_notes: text_editor::Content::new(),
        }
    }

    fn get_mut(&mut self, field: FormField) -> &mut text_editor::Content {
        match field {
            FormField::EventDraft => &mut self.event_draft,
            FormField::EventEdit => &mut self.event_edit,
            FormField::HabitNotes => &mut self.habit_notes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Calendar,
    /// Incomplete tasks whose scheduled time or deadline has passed.
    Overdue,
    /// Tasks that have not been triaged yet.
    Inbox,
    /// Incomplete tasks dated today that still need a time slot.
    Today,
    /// Tasks that have been triaged but not yet scheduled.
    Triaged,
    Backlog,
    Life,
    /// The places events happen at, and the trips between them.
    Travel,
    /// Recurring events on the TimeSlay calendar: habits and their rules.
    Habits,
    /// The notepad: a page of free text kept between sessions.
    Notepad,
    /// One brain dump page per day, with search across all of them.
    BrainDump,
    /// Markdown documents stored locally and synced through Turso.
    Notes,
    /// Contacts from every signed-in Google account, and their directories.
    People,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutField {
    CalendarWidth,
    CalendarWeekWidth,
    CalendarMonthWidth,
    TimeBlockWidth,
    DetailWidth,
    ScreenMargin,
    BorderRadius,
    DockBorderRadius,
    PanelBorderRadius,
    DockWidth,
    DockLeftMargin,
    DockRightMargin,
    DockBorderOuterWidth,
    DockBorderInnerWidth,
    PanelBorderOuterWidth,
    PanelBorderInnerWidth,
    FadeDuration,
}

impl LayoutField {
    pub fn value(self, state: &State) -> f32 {
        match self {
            Self::CalendarWidth => state.calendar_width(),
            Self::CalendarWeekWidth => state
                .config
                .views
                .calendar_week_width(ui::calendar::weekly_view_width(&state.config.calendar)),
            Self::CalendarMonthWidth => state.config.views.calendar_month_width(),
            Self::TimeBlockWidth => state.config.views.time_block_width(),
            Self::DetailWidth => state.config.views.detail_width(),
            Self::ScreenMargin => state.config.views.margin(),
            Self::BorderRadius => state.config.views.border_radius(),
            Self::DockBorderRadius => state.config.views.dock_border_radius(),
            Self::PanelBorderRadius => state.config.views.panel_border_radius(),
            Self::DockWidth => state.config.views.dock_width(),
            Self::DockLeftMargin => state.config.views.dock_left_margin(),
            Self::DockRightMargin => state.config.views.dock_right_margin(),
            Self::DockBorderOuterWidth => state.config.views.panel_border_outer_width(),
            Self::DockBorderInnerWidth => state.config.views.panel_border_inner_width(),
            Self::PanelBorderOuterWidth => state.config.views.panel_border_outer_width(),
            Self::PanelBorderInnerWidth => state.config.views.panel_border_inner_width(),
            Self::FadeDuration => state.config.views.fade_duration_ms() as f32,
        }
    }

    pub fn bounds(self) -> (f32, f32) {
        match self {
            Self::ScreenMargin => (0.0, config::ViewSettings::MAX_MARGIN),
            Self::BorderRadius | Self::DockBorderRadius | Self::PanelBorderRadius => {
                (0.0, config::ViewSettings::MAX_BORDER_RADIUS)
            }
            Self::DockWidth => (
                config::ViewSettings::MIN_DOCK_WIDTH,
                config::ViewSettings::MAX_DOCK_WIDTH,
            ),
            Self::DockLeftMargin | Self::DockRightMargin => {
                (0.0, config::ViewSettings::MAX_DOCK_MARGIN)
            }
            Self::DockBorderOuterWidth
            | Self::DockBorderInnerWidth
            | Self::PanelBorderOuterWidth
            | Self::PanelBorderInnerWidth => (0.0, config::ViewSettings::MAX_BORDER_WIDTH),
            Self::FadeDuration => (0.0, config::ViewSettings::MAX_FADE_DURATION_MS as f32),
            _ => (
                config::ViewSettings::MIN_WIDTH,
                config::ViewSettings::MAX_WIDTH,
            ),
        }
    }

    pub fn changed(self, value: f32) -> Message {
        match self {
            Self::CalendarWidth => Message::Settings(SettingsMessage::CalendarWidthChanged(value)),
            Self::CalendarWeekWidth => {
                Message::Settings(SettingsMessage::CalendarWeekWidthChanged(value))
            }
            Self::CalendarMonthWidth => {
                Message::Settings(SettingsMessage::CalendarMonthWidthChanged(value))
            }
            Self::TimeBlockWidth => {
                Message::Settings(SettingsMessage::TimeBlockWidthChanged(value))
            }
            Self::DetailWidth => Message::Settings(SettingsMessage::DetailWidthChanged(value)),
            Self::ScreenMargin => Message::Settings(SettingsMessage::ScreenMarginChanged(value)),
            Self::BorderRadius => Message::Settings(SettingsMessage::BorderRadiusChanged(value)),
            Self::DockBorderRadius => {
                Message::Settings(SettingsMessage::DockBorderRadiusChanged(value))
            }
            Self::PanelBorderRadius => {
                Message::Settings(SettingsMessage::PanelBorderRadiusChanged(value))
            }
            Self::DockWidth => Message::Settings(SettingsMessage::DockWidthChanged(value)),
            Self::DockLeftMargin => {
                Message::Settings(SettingsMessage::DockLeftMarginChanged(value))
            }
            Self::DockRightMargin => {
                Message::Settings(SettingsMessage::DockRightMarginChanged(value))
            }
            Self::DockBorderOuterWidth => {
                Message::Settings(SettingsMessage::DockBorderOuterWidthChanged(value))
            }
            Self::DockBorderInnerWidth => {
                Message::Settings(SettingsMessage::DockBorderInnerWidthChanged(value))
            }
            Self::PanelBorderOuterWidth => {
                Message::Settings(SettingsMessage::PanelBorderOuterWidthChanged(value))
            }
            Self::PanelBorderInnerWidth => {
                Message::Settings(SettingsMessage::PanelBorderInnerWidthChanged(value))
            }
            Self::FadeDuration => Message::Settings(SettingsMessage::FadeDurationChanged(value)),
        }
    }
}

/// A flash overlay that is currently on screen.
pub struct Flash {
    pub window: window::Id,
    pub started: Instant,
    pub settings: config::Flash,
}

/// Geometry for the side panel; absent for a task popover.
pub struct Sidebar {
    /// Width the panel is pinned to, in logical pixels.
    pub width: f32,
    /// Which edge it is anchored to.
    pub side: panel::Side,
}

/// The state of one action's row in the shortcut settings.
#[derive(Debug, Default)]
pub struct ShortcutEditor {
    /// What has been typed into the "add" box.
    pub input: String,
    /// A change is being applied.
    pub busy: bool,
    /// How the last change went.
    pub result: Option<Result<String, String>>,
}

/// A command sent from the shortcut daemon to the panel process over its IPC
/// socket, telling it to show or hide its window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelCommand {
    Show,
    Hide,
    ToggleReservation,
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
    pub picker: Option<crate::ui::controls::Picker>,
    /// Background work under way, and the log of what it has done.
    pub activity: crate::activity::Activity,
    /// Local edits not yet sent by database sync, as last read.
    pub database_unsent: usize,
    /// Keyboard choice in the calendar quick-create popup; None means title entry.
    pub draft_kind_focus: Option<calendar::DraftKind>,
    /// Persistent cursor and selection state for new events, event edits and habits.
    pub task_description: text_editor::Content,
    pub task_creation_window: bool,
    pub description_popup: Option<String>,
    pub description_editor: text_editor::Content,
    pub form_editors: FormEditors,
    pub config: Config,
    pub catalog: Catalog,
    /// The tone the system is using, for the high contrast follow-the-system
    /// theme. It is kept up to date by [`SystemMessage::SystemThemeMode`].
    pub system_theme: iced::theme::Mode,

    /// The clock reading used for this frame, so every widget agrees.
    pub now: Instant,

    /// Geometry for the side panel; absent for a task popover.
    /// It pins the window to its configured width and edge, and keeps the panel
    /// from writing its own geometry back over the user's saved window.
    pub sidebar: Option<Sidebar>,

    /// The alarms set on the calendar, and the state of setting them.
    pub alarms: alarms::Alarms,
    pub reminders: crate::reminders::Reminders,

    pub screen: Screen,
    pub view: CalendarView,
    /// Last period chosen for the Plan split button.
    pub plan_view: CalendarView,
    /// Whether the Plan menu under the left-hand navigation group is open.
    pub view_menu_open: bool,
    /// Whether the More menu under the right-hand navigation group is open.
    pub more_menu_open: bool,
    pub week_anchor: chrono::NaiveDate,
    pub week_hover: Option<chrono::NaiveDate>,
    pub week_scroll_x: f32,
    /// How far down the calendar grid is scrolled, and how much of it shows.
    /// A zoom with no pointer to hold still holds the middle of this instead.
    pub grid_scroll: (f32, f32),
    /// Whether the month picker is showing over the range overview. The day
    /// panel's own picker is `calendar.month_open`.
    pub overview_month_open: bool,
    /// The first day of the month the overview's picker is showing.
    pub overview_month: chrono::NaiveDate,
    /// The day panel moved to a day outside the range in view, which a range
    /// sync would not fetch.
    pub panel_day_wanted: bool,
    /// The day whose full list is open from its "N more" in the two-week view.
    pub two_week_more: Option<chrono::NaiveDate>,
    /// What the Projects tab is in the middle of.
    pub life: crate::life::Workspace,
    /// Places, place searches and trips: the Travel tab and travel on the day view.
    pub travel: crate::travel::Workspace,
    pub main_window: Option<window::Id>,
    pub main_size: Size,
    /// Logical size of the monitor, when the compositor reports it.
    pub monitor_size: Option<Size>,
    /// Receiver for show/hide commands from the panel daemon, when this process
    /// is the side panel.
    pub panel_command_rx: Option<mpsc::Receiver<PanelCommand>>,
    /// Tasks added over IPC by the browser extension or the Slack app, waiting
    /// to be folded into the calendar on the UI thread.
    pub inbox_added_rx: Option<mpsc::Receiver<calendar::Event>>,
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

    /// The "add a shortcut" boxes of the settings page, one per [`panel::Action`].
    pub shortcut_editors: [ShortcutEditor; panel::Action::ALL.len()],
    /// Whether the panel window is on screen, as the shortcut daemon sees it
    /// when it asks. Only meaningful when this process is the side panel.
    pub panel_visible: Arc<AtomicBool>,
    /// Keep empty or incomplete numbers editable until they are submitted.
    pub view_layout_input: Option<(LayoutField, String)>,
    /// The panel border colour box being typed into, and the text as it
    /// currently reads.
    pub panel_border_input: Option<(BorderLayer, String)>,
    /// Whether the shared details editor is visible.
    pub details_pane_open: bool,
    /// Floating range view or details editor, independent of the day panel.
    pub editor_window: Option<window::Id>,
    pub(crate) editor_window_size: Size,

    /// Set while a config write is already queued, so a slider drag does not
    /// schedule one save per pixel.
    save_queued: bool,
    /// Last disk snapshot, including our own writes, to ignore save echoes.
    disk_config: Config,
    /// Why settings are not reaching the disk, shown above the app until a
    /// good config can be read or written.
    pub config_error: Option<String>,

    /// The notepad's text as the editor widget holds it.
    ///
    /// The widget keeps its own copy so the cursor and the selection survive
    /// a redraw; the config keeps the saved copy, and the two are reconciled
    /// on every edit.
    pub notepad: text_editor::Content<iced::Renderer>,

    pub calendar: calendar::Store,
    /// Visible empty days awaiting an in-flight calendar request.
    calendar_fetch_pending: Vec<chrono::NaiveDate>,
    pub timeblocking: crate::timeblocking::Workspace,
    /// The Brain dump screen and the page open in its editor.
    pub brain_dump: crate::brain_dump::Workspace,
    /// The Habits tab: the habit masters and the rule editor.
    pub habits: crate::habits::Store,
    pub notes: crate::notes::Store,
    pub people: crate::people::Store,
    pub inbox: crate::inbox::Inbox,
    /// Google accounts connected for the Gmail integration.
    pub accounts: crate::integrations::accounts::Accounts,
    /// The look for changes other processes made to the database.
    pub poll: crate::local_poll::Poll,
}

// Messages only live until the next update, so a large variant costs nothing lasting.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Message {
    Picker(crate::ui::controls::Action),
    TimeBlocking(crate::timeblocking::Action),
    BrainDump(crate::brain_dump::Action),
    Inbox(crate::inbox::Action),
    Accounts(crate::integrations::accounts::Action),
    People(crate::people::Action),
    Alarms(alarms::Action),
    Reminders(crate::reminders::Action),
    Life(crate::life::Action),
    Travel(crate::travel::Action),
    Habits(crate::habits::Action),
    /// An event we deliberately do not act on, such as a key release or a
    /// press that only needs to be swallowed.
    Ignored,
    OpenEventLink(String),
    Navigation(NavigationMessage),
    Notepad(NotepadMessage),
    Settings(SettingsMessage),
    System(SystemMessage),
    Notes(crate::notes::Action),
    Calendar(CalendarMessage),
    Task(TaskMessage),
    Event(EventMessage),
    EventDraft(EventDraftMessage),
    EventEdit(EventEditMessage),
}

/// Moving between screens.
#[derive(Debug, Clone)]
pub enum NavigationMessage {
    ScreenSelected(Screen),
    PlanTabSelected(PlanTab),
    PlanDaySelected,
    ViewMenuToggled,
    MoreMenuToggled,
    /// The log of background work, opened from the foot of the panel.
    ActivityLogToggled,
}

/// The notepad page.
#[derive(Debug, Clone)]
pub enum NotepadMessage {
    /// The notepad was edited.
    NotepadEdited(text_editor::Action),
    /// The text on the notepad was made bigger or smaller, by a number of
    /// percentage points.
    NotepadFontSizeChanged(i32),
}

/// Changes made on the Settings page.
// Messages only live until the next update, so a large variant costs nothing lasting.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum SettingsMessage {
    NotesDirectoryChanged(String),
    NotesDirectoryApply,
    NotesDirectoryBrowse,
    NotesDirectoryPicked(Option<String>),
    DatabaseSyncUrlChanged(String),
    ThemeSelected(String),
    TimeFormatSelected(TimeFormat),
    DistanceUnitSelected(config::DistanceUnit),
    FirstDayOfWeekSelected(calendar::first_day::FirstDayOfWeek),
    AlwaysOnTopToggled(bool),
    CalendarWidthChanged(f32),
    CalendarWeekWidthChanged(f32),
    CalendarMonthWidthChanged(f32),
    TimeBlockWidthChanged(f32),
    DetailWidthChanged(f32),
    ScreenMarginChanged(f32),
    DockReserveSpaceToggled(bool),
    PanelReserveSpaceToggled(bool),
    DockWidthChanged(f32),
    DockLeftMarginChanged(f32),
    DockRightMarginChanged(f32),
    DockSideSelected(config::DockSide),
    DockBorderOuterWidthChanged(f32),
    DockBorderInnerWidthChanged(f32),
    DockBorderColorInput(BorderLayer, String),
    DockBorderColorSubmitted(BorderLayer),
    PanelBorderOuterWidthChanged(f32),
    PanelBorderInnerWidthChanged(f32),
    PanelBorderColorInput(BorderLayer, String),
    PanelBorderColorSubmitted(BorderLayer),
    FadeDurationChanged(f32),
    WindowAnimationSelected(config::WindowAnimation),
    DockHourLinesToggled(bool),
    /// Quit and start again from the binary on disk.
    RestartRequested,
    BorderRadiusChanged(f32),
    DockBorderRadiusChanged(f32),
    PanelBorderRadiusChanged(f32),
    ViewLayoutInputChanged(LayoutField, String),
    ViewLayoutInputSubmitted(LayoutField),
    ShortcutInputChanged(panel::Action, String),
    ShortcutAdded(panel::Action),
    ShortcutRemoved(panel::Action, usize),
    /// The shortcuts of an action were applied, or could not be.
    ShortcutsApplied(panel::Action, Result<Vec<String>, String>),
    /// Show or hide Google Tasks in the day view.
    ShowTasksToggled(bool),
    /// Open or close the color picker for a calendar or Google Tasks list.
    SourceColorEditorToggled(String),
    /// Give a Google Tasks list a custom color, or `None` to reset it.
    TaskColorPicked(String, Option<iced::Color>),
    /// Give a regular calendar a local color override, or reset to Google's color.
    CalendarColorPicked(String, Option<iced::Color>),
    /// Color one of the TimeSlay calendar's event kinds; `None` resets it.
    TimeSlayColorPicked(String, Option<iced::Color>),
    GoogleClientIdChanged(String),
    GoogleClientSecretChanged(String),
    GoogleConnectRequested,
    GoogleConnected(Result<calendar::oauth::Tokens, String>),
    GoogleDisconnectRequested,
    CalendarSyncMinutesChanged(u32),
    /// The hour the calendar day ends, so late-night events stay with the
    /// evening they belong to.
    CalendarDayEndChanged(u32),
    SleepTimeChanged(chrono::Weekday, bool, String),
    SleepSaveRequested,
    SleepSaved(Result<calendar::Event, String>),
    /// The actual sleep start field of the open night was edited.
    SleepActualStartChanged(String),
    /// The actual wake-up field of the open night was edited.
    SleepActualEndChanged(String),
    /// The Start sleep or Finish sleep button of the night with this id.
    SleepTimerPressed(String),
    SleepTimerSaved(String, Result<calendar::Event, String>),
    CalendarWeekRangeChanged(config::WeekRange),

    /// The calendar new events are created on was changed.
    DefaultCalendarSelected(String),
}

/// Windows, timers, the config file and other things the app reacts to.
// Messages only live until the next update, so a large variant costs nothing lasting.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum SystemMessage {
    TaskPopoverBounds(iced::Rectangle),
    FlashFrame(Instant),
    Dismiss,

    /// A sound finished rendering and is ready to hand to PipeWire.
    SoundRendered(Result<Pcm, String>, PlaySpec),
    NotificationPosted(Result<(), String>),
    ConfigSaved(Result<(), String>),
    SaveRequested,
    ConfigReloaded(Config),
    /// Time to look for changes other processes made to the database.
    DatabaseSyncPolled,
    /// What that look found, read on a worker thread.
    DatabasePolled(Option<Box<crate::local_poll::Polled>>),
    /// Time to show the next of several things under way.
    ActivityTick,
    AudioStatusPolled,

    /// The system's light/dark tone, for the high contrast system theme.
    SystemThemeMode(iced::theme::Mode),

    WindowOpened(window::Id),
    EditorWindowOpened(window::Id),
    /// The monitor size came back, so the window can open at its default width.
    MonitorSized(window::Id, Option<Size>),
    WeekMonitorSized(window::Id, Option<Size>),
    WindowClosed(window::Id),
    WindowResized(window::Id, Size),
    FlashFinished,
    /// A poll tick that checks for show/hide commands from the panel daemon.
    PanelCommandPolled,
    /// A poll tick that checks for tasks added over IPC.
    InboxAddedPolled,

    KeyPressed(Key, keyboard::Modifiers),
    OpenGoogleCalendar,
    /// Swap this window for the dock strip on the screen edge.
    DockRequested,
}

/// What the calendar shows and keeping it in sync.
#[derive(Debug, Clone)]
pub enum CalendarMessage {
    CalendarModeSelected(bool),
    CalendarTaskListToggled(bool),
    CalendarMonthViewSelected,
    CalendarScheduleViewSelected,
    CalendarTwoWeekViewSelected,
    /// Open a day's full list from the two-week view, or close it.
    CalendarTwoWeekMore(Option<chrono::NaiveDate>),
    /// A press on a two-week cell's empty space, which starts a new event.
    CalendarTwoWeekCellPressed(chrono::NaiveDate),
    CalendarViewMonthShift(i32),
    CalendarDayOpened(chrono::NaiveDate),
    WeekDayAction(chrono::NaiveDate, Box<Message>),
    WeekHover(Option<chrono::NaiveDate>),
    WeekScrolled(f32),
    /// The vertical scroll of whichever grid is showing: its top and height.
    CalendarGridScrolled(f32, f32),

    CalendarMenuToggled,
    /// The Actions button in the calendar header.
    CalendarActionsToggled,
    CalendarSearch(calendar::search::Action),
    CalendarToggled(String, bool),
    /// Move a calendar-view source one position up (left) or down (right).
    CalendarSourceMoved(String, bool),
    CalendarSyncRequested,
    CalendarAutoSyncRequested,
    /// Drop cached days and pull the day in view from Google again.
    CalendarResyncRequested,
    CalendarSynced(Result<calendar::Snapshot, String>),
    CalendarSyncedAt(u64, Result<calendar::Snapshot, String>),
    CalendarRangeSynced(
        u64,
        Vec<chrono::NaiveDate>,
        Result<calendar::Snapshot, String>,
    ),
    CalendarTick,
    /// Multiplies the calendar hour height. Above 1 spreads the hours apart.
    ///
    /// `anchor` is the pixel row under the pointer, measured from the top of
    /// the grid, so the zoom can keep that row still under the mouse. The keys
    /// and the menu have no pointer, so they leave it out and the middle of
    /// the view holds still.
    CalendarZoomed {
        factor: f32,
        anchor: Option<f32>,
    },
    CalendarZoomReset,
    /// Move the day in view by a number of days, positive or negative.
    CalendarDayShift(i64),
    /// Jump the day in view to a specific date, from the month picker.
    CalendarDaySelected(chrono::NaiveDate),
    /// Open or close a month picker.
    CalendarMonthToggled(MonthPicker),
    /// Move a month picker by a number of months, positive or negative.
    CalendarMonthShift(MonthPicker, i32),
    /// A day tapped in a month picker. It moves only what that picker belongs to.
    CalendarMonthDayPicked(MonthPicker, chrono::NaiveDate),
}

/// Tasks: dragging, completing and deleting them.
// Messages only live until the next update, so a large variant costs nothing lasting.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum TaskMessage {
    TaskDragMoved(window::Id, iced::Point),
    WeekDragScroll,
    TaskDragReleased(window::Id),
    TaskDragDropped(String, Option<Box<Message>>),
    /// What dropping the task here would do: the task, the plan view point
    /// that was asked about, and the drop it answered with.
    TaskDragHovered(String, iced::Point, Option<Box<Message>>),
    /// Tick a Google Task off, or put it back.
    TaskToggled(String),
    /// Ignore a task, or restore an ignored task to active.
    TaskIgnoreToggled(String),
    TaskCompletionSaved(
        String,
        timeblocking::TaskStatus,
        Result<calendar::Event, String>,
    ),
    /// Delete a single Google Task instance (one occurrence of a recurring task).
    TaskDeleteRequested(String),
    /// The user confirmed the task deletion prompt.
    TaskDeleteConfirmed(String),
    /// Google's answer to a single-task deletion.
    TaskDeleted(String, Result<(), String>),
}

/// An event that already exists: opening, moving and deleting it.
// Messages only live until the next update, so a large variant costs nothing lasting.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum EventMessage {
    /// Answer the invitation to this event.
    EventReply(String, calendar::meeting::Response),
    EventReplied(
        String,
        Option<calendar::meeting::Response>,
        Result<(), String>,
    ),
    DescriptionOpen(String),
    DescriptionEdited(text_editor::Action),
    DescriptionClose,
    DescriptionSave,
    EventTitleEdit,
    HabitDetailsLoaded(
        String,
        Result<(calendar::Event, calendar::EditDraft), String>,
    ),
    /// The repeat schedule of an ordinary recurring event's series. Google
    /// only stores it on the master, not on the occurrence that was opened.
    SeriesRuleLoaded(
        String,
        Result<(calendar::Event, calendar::EditDraft), String>,
    ),
    EventEndDateChanged(String),
    EventLabelDraft(String),
    EventLabelAdd,
    EventLabelRemove(String),
    FormEdited(FormField, text_editor::Action),
    EventOpened(String),
    EventClosed,
    /// The user moved or resized an event on the day grid. Preview the time,
    /// then ask for recurrence scope and guest notifications before saving.
    EventTimeChanged {
        id: String,
        start: DateTime<Local>,
        end: DateTime<Local>,
    },
    /// Google's answer to a dragged time change.
    EventTimeSaved(String, Result<(), String>),
    SleepTimeSaved(String, calendar::SaveScope, Result<calendar::Event, String>),
    /// The user clicked Delete on an event's details card. The confirmation
    /// prompt opens before anything is sent to Google.
    /// Whether the delete being confirmed emails the guests.
    EventDeleteNotify(bool),
    /// Write the dragged meeting's new time, emailing the guests or not.
    EventMoveConfirmed(bool),
    /// Choose which occurrences receive the dragged time change.
    EventMoveScoped(calendar::SaveScope),
    /// Put the dragged meeting back.
    EventMoveCancelled,
    EventDeleteRequested(String),
    EventDeleteScoped(String, calendar::SaveScope),
    /// The user confirmed the deletion prompt.
    EventDeleteConfirmed(String),
    /// The user dismissed the deletion prompt without deleting.
    EventDeleteCancelled,
    /// Google's answer to a single-event deletion.
    EventDeleted(String, Result<(), String>),
}

/// The form for a new event.
// Messages only live until the next update, so a large variant costs nothing lasting.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum EventDraftMessage {
    /// A click or drag on empty grid space asked for a new event. The popup
    /// opens on the default calendar, spanning these times.
    EventDraftStarted {
        start: DateTime<Local>,
        end: DateTime<Local>,
    },
    /// The new event's title was edited.
    EventDraftTitleChanged(String),
    EventDraftCycleKind {
        reverse: bool,
        title_focused: bool,
    },
    EventDraftOptions(calendar::event_options::Action),
    EventDraftProjects(calendar::projects::Action),
    /// The new event's description was edited.
    EventDraftDescriptionChanged(String),
    /// The new event's date was edited, as `YYYY-MM-DD`.
    EventDraftDateChanged(String),
    /// The start field was edited, as `HH:MM`.
    EventDraftStartChanged(String),
    /// The end field was edited, as `HH:MM`.
    EventDraftEndChanged(String),
    /// The end date field was edited, as `YYYY-MM-DD`: the last day an
    /// all-day event covers, or the day a timed one ends.
    EventDraftEndDateChanged(String),
    /// The new event was switched between all-day and timed.
    EventDraftAllDayToggled(bool),
    /// The new entry's kind was switched between an event and a time block.
    EventDraftKindSelected(calendar::DraftKind),
    /// A change was made in the new event's shared repeat controls.
    EventDraftRecurrence(crate::recurrence::Action),
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
    EventDraftMeeting(calendar::meeting::Action),
}

/// The form for editing an event.
#[derive(Debug, Clone)]
pub enum EventEditMessage {
    EventEditDeadlineChanged(String),
    EventEditDeleteMenuToggled,
    EventEditOptions(calendar::event_options::Action),

    /// The user asked to edit an existing event, from the details card.
    EventEditRequested(String),
    /// The edited event's title was changed.
    EventEditTitleChanged(String),
    EventEditProjects(calendar::projects::Action),
    /// The edited event's description was changed.
    EventEditDescriptionChanged(String),
    /// The edited event's date was changed, as `YYYY-MM-DD`.
    EventEditDateChanged(String),
    /// The edited event's start field was edited, as `HH:MM`.
    EventEditStartChanged(String),
    /// The edited event's end field was edited, as `HH:MM`.
    EventEditEndChanged(String),
    /// The edited event was switched between all-day and timed.
    EventEditAllDayToggled(bool),
    /// A change was made in the edited event's shared repeat controls.
    EventEditRecurrence(crate::recurrence::Action),
    /// A calendar was chosen for the edited event, staged until Save.
    EventEditCalendarSelected(String),
    /// The edited event's calendar dropdown was opened or dismissed.
    EventEditCalendarDropdownToggled,
    /// The edit-event popup was dismissed without saving.
    EventEditCancelled,
    /// Write the edited event to Google.
    EventEditSaveRequested,
    EventEditSaveMenuToggled,
    EventEditSaveScoped(calendar::SaveScope),
    /// Google's answer to updating the event.
    EventEditSaved(Result<calendar::api::Saved, String>),
    EventEditMeeting(calendar::meeting::Action),
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
            picker: None,
            activity: crate::activity::Activity::default(),
            database_unsent: 0,
            draft_kind_focus: None,
            task_description: text_editor::Content::new(),
            task_creation_window: false,
            description_popup: None,
            description_editor: text_editor::Content::new(),
            form_editors: FormEditors::new(),
            now: Instant::now(),
            sidebar: None,
            alarms: alarms::Alarms::default(),
            reminders: crate::reminders::Reminders::default(),
            screen: Screen::Calendar,
            view: CalendarView::default(),
            plan_view: CalendarView::Day,
            view_menu_open: false,
            more_menu_open: false,
            week_anchor: calendar::day_at(crate::clock::now(), day_end_hour),
            week_hover: None,
            week_scroll_x: 0.0,
            grid_scroll: (0.0, 0.0),
            overview_month_open: false,
            overview_month: calendar::first_of_month(calendar::day_at(
                crate::clock::now(),
                day_end_hour,
            )),
            panel_day_wanted: false,
            two_week_more: None,
            life: Default::default(),
            travel: crate::travel::Workspace::starting(config.travel.show),
            main_window: None,
            main_size: Size::new(
                config.views.calendar_width(config.window.width),
                config.window.height,
            ),
            monitor_size: None,
            panel_command_rx: None,
            inbox_added_rx: None,
            panel_config: None,
            panel_expected_close: false,
            flash: None,
            alarm_active: false,
            audio: Audio::new(),
            audio_error: None,
            shortcut_editors: Default::default(),
            panel_visible: Arc::new(AtomicBool::new(true)),
            view_layout_input: None,
            panel_border_input: None,
            details_pane_open: false,
            editor_window: None,
            editor_window_size: Size::ZERO,
            save_queued: false,
            disk_config: config.clone(),
            config_error: None,
            catalog,
            system_theme: iced::theme::Mode::None,
            config,
            notepad,
            notes: crate::notes::Store::default(),
            people: crate::people::Store::default(),
            calendar: calendar::Store::new(day_end_hour),
            calendar_fetch_pending: Vec::new(),
            timeblocking: crate::timeblocking::Workspace::default(),
            brain_dump: crate::brain_dump::Workspace::default(),
            habits: crate::habits::Store::default(),
            inbox: crate::inbox::Inbox::default(),
            accounts: crate::integrations::accounts::Accounts::load(),
            poll: Default::default(),
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

    pub fn screen_margin_width(&self) -> f32 {
        self.screen_margin() * 2.0
    }

    /// A habit is open beside the Habits list, or being added.
    pub fn habit_panel_open(&self) -> bool {
        self.screen == Screen::Habits
            && (self.habits.selected.is_some() || self.habits.form.is_some())
    }

    /// A life area is open beside the Life list.
    pub fn life_panel_open(&self) -> bool {
        self.screen == Screen::Life
            && self
                .life
                .open
                .as_ref()
                .is_some_and(|open| self.config.projects.area(open.area).is_some())
    }

    /// The map picker is shown in the floating window at the right of the
    /// panel, never in the panel itself. The dock has no such window, so
    /// there it covers the dock instead.
    pub fn map_in_editor_window(&self) -> bool {
        self.travel.map.active && !self.task_creation_window && self.main_window.is_some()
    }

    pub fn floating_editors(&self) -> bool {
        !self.task_creation_window
            && (self.people_panel_open()
                || self.habit_panel_open()
                || self.life_panel_open()
                || (shows_details(self.screen) && !self.range_view_active()))
    }

    /// A contact is open beside the People list. Contacts have their own pane
    /// rather than the shared details editor, which belongs to events and tasks.
    pub fn people_panel_open(&self) -> bool {
        self.screen == Screen::People && self.people.draft.is_some()
    }

    /// A calendar fetch is actually under way, for the footer and the month
    /// view. Days merely queued in `calendar_fetch_pending` do not count: no
    /// request is running yet, and saying "Loading from cloud" while the
    /// queue waits on something else (a pending move, a time block write)
    /// left the footer stuck on loading after the fetch had finished.
    pub(crate) fn calendar_loading(&self) -> bool {
        self.calendar.is_busy()
    }

    /// A state touching nothing outside itself: default settings and the
    /// built-in themes. It is what tests and previews start from.
    pub fn blank() -> Self {
        Self::new(Config::default(), Catalog::built_in())
    }

    /// Background work under way now, in the order it is rotated through.
    pub fn current_activities(&self) -> Vec<crate::activity::Current> {
        use crate::activity::{Current, Kind};
        let mut current = Vec::new();
        let sending = timeslay_core::calendar::operations::sending();
        if sending > 0 {
            current.push(Current {
                kind: Kind::SavingToCloud,
                label: format!("Saving to cloud ({sending})"),
            });
        }
        if self.calendar_loading() {
            current.push(Current {
                kind: Kind::LoadingFromCloud,
                label: "Loading from cloud".into(),
            });
        }
        if self.database_unsent > 0 {
            let label = if timeslay_core::database_sync::waiting() {
                format!("Waiting for database sync ({})", self.database_unsent)
            } else {
                format!("Syncing database ({})", self.database_unsent)
            };
            current.push(Current {
                kind: Kind::DatabaseSync,
                label,
            });
        }
        current
    }

    pub fn weekly(&self) -> bool {
        self.view == CalendarView::Week
    }

    pub fn monthly(&self) -> bool {
        self.view == CalendarView::Month
    }

    pub fn two_weekly(&self) -> bool {
        self.view == CalendarView::TwoWeek
    }

    pub fn range_view_active(&self) -> bool {
        self.view != CalendarView::Day
    }

    /// Day planning keeps the task list in the panel and the calendar beside it.
    pub fn day_plan_active(&self) -> bool {
        !self.task_creation_window
            && self.view == CalendarView::Day
            && matches!(
                self.screen,
                Screen::Overdue | Screen::Inbox | Screen::Today | Screen::Triaged | Screen::Backlog
            )
    }

    pub fn calendar_dates(&self) -> Vec<chrono::NaiveDate> {
        match self.view {
            CalendarView::Schedule => calendar::schedule::dates(self.week_anchor),
            CalendarView::Month => {
                calendar::month_grid(self.week_anchor, self.config.calendar.first_day_of_week)
            }
            CalendarView::TwoWeek => {
                calendar::two_week_dates(self.week_anchor, self.config.calendar.first_day_of_week)
            }
            _ => self.config.calendar.week_dates(self.week_anchor),
        }
    }

    fn weekly_window_width(&self) -> f32 {
        // The two-week grid has the month's seven columns, so it takes its width.
        let month_shaped = self.monthly() || self.two_weekly();
        let view_width = if month_shaped {
            self.config.views.calendar_month_width()
        } else {
            self.config
                .views
                .calendar_week_width(ui::calendar::weekly_view_width(&self.config.calendar))
        };
        let configured = if month_shaped {
            self.config.views.calendar_month_width
        } else {
            self.config.views.calendar_week_width
        };
        // Preserve the previous automatic sizing until a width is chosen.
        let view_width = if configured.is_some() {
            view_width
        } else {
            view_width.max(self.calendar_width())
        };
        let width = view_width + self.screen_margin_width();
        match self.monitor_size {
            Some(size) if size.width.is_finite() && size.width > 0.0 => {
                let panel_width = self.calendar_width() + self.screen_margin_width();
                width.min((size.width - panel_width).max(1.0))
            }
            _ => width,
        }
    }
}

/// Resize the day panel, keeping its transparent screen inset outside the
/// configured width. Floating views have their own geometry.
fn apply_view_sizes(state: &mut State) -> Task<Message> {
    if state.task_creation_window {
        return Task::none();
    }
    let base = state.calendar_width() + state.screen_margin_width();
    let width = base;
    let Some(id) = state.main_window else {
        return Task::none();
    };
    let resize = window::resize(id, Size::new(width, state.main_size.height));
    if state.sidebar.is_some() {
        // Relax the previous fixed width before asking the compositor to resize.
        window::set_max_size(id, None)
            .chain(window::set_min_size(
                id,
                Some(Size::new(base, MIN_SIZE.height)),
            ))
            .chain(resize)
            .chain(window::set_max_size(id, Some(Size::new(base, 100_000.0))))
    } else {
        resize
    }
}

/// Asks iced for the system's current light/dark tone. The subscription only
/// reports changes, so this seeds [`State::system_theme`] at startup.
fn system_theme_task() -> Task<Message> {
    iced::system::theme().map(|mode| Message::System(SystemMessage::SystemThemeMode(mode)))
}

/// Opens the task editor as a layer-shell popover.
pub fn boot_add_something() -> (State, Task<Message>) {
    let mut state = boot_state();
    state.task_creation_window = true;
    state.main_size = Size::new(480.0, 800.0);
    let edit = crate::inbox::update(&mut state, crate::inbox::Action::NewTask);
    let (_id, open) = window::open(window::Settings {
        size: state.main_size,
        transparent: true,
        min_size: Some(MIN_SIZE),
        exit_on_close_request: true,
        ..Default::default()
    });
    (
        state,
        Task::batch([
            open.map(|v0| Message::System(SystemMessage::WindowOpened(v0))),
            edit,
            system_theme_task(),
        ]),
    )
}

/// Opens the clock as the side panel: a separate, narrow instance pinned to one
/// screen edge and kept above other windows.
///
/// The width and edge come from the `panel.*` settings, and the width is also fixed by
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
    state.reminders.active = true;

    let (tx, rx) = mpsc::channel();
    state.panel_command_rx = Some(rx);
    let (inbox_tx, inbox_rx) = mpsc::channel();
    state.inbox_added_rx = Some(inbox_rx);

    match crate::ipc::bind(crate::ipc::SIDEBAR) {
        Ok(listener) => {
            let visible = Arc::clone(&state.panel_visible);
            std::thread::Builder::new()
                .name("panel-ipc".into())
                .spawn(move || {
                    if let Err(error) = panel_ipc_listener(listener, tx, visible, inbox_tx) {
                        log::error!("panel IPC listener stopped: {error:#}");
                    }
                })
                .expect("failed to spawn the panel IPC listener");
        }
        Err(error) => log::error!("panel IPC is unavailable: {error:#}"),
    }

    state.main_size = Size::new(
        state.calendar_width() + state.screen_margin_width(),
        PANEL_FILL,
    );
    let (_id, open) = window::open(sidebar_settings(&panel, &state.config.views));
    let device_sounds = crate::reminders::load_device_sounds(&state);

    (
        state,
        Task::batch([
            open.map(|v0| Message::System(SystemMessage::WindowOpened(v0))),
            system_theme_task(),
            device_sounds,
        ]),
    )
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
    let width = views.calendar_width(panel.width as f32) + views.margin() * 2.0;
    window::Settings {
        size: Size::new(width, PANEL_FILL),
        min_size: Some(Size::new(width, MIN_SIZE.height)),
        max_size: Some(Size::new(width, 100_000.0)),
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

/// Builds the state that is read from disk at startup.
///
/// Shared by the task popover and the side panel, so both open on the same
/// config, the same cached days and the same running track. It deliberately
/// opens no window: the two callers want different ones.
fn boot_state() -> State {
    let (config, problem) = Config::load_checked();
    let mut state = State::new(config, Catalog::load());
    state.config_error = problem;

    // Start sync only for the running app; hand-built test states stay offline.
    crate::database_sync::start();

    // Days are kept under the XDG cache directory as well as in memory, so a
    // restart opens on what the last run saw rather than an empty grid. This
    // is deliberately not part of `State::new`: a state built by hand in a
    // test must not read or rewrite the user's cache.
    state.calendar.apply_task_colors(&state.config.calendar);
    let database = crate::task_db::path();
    state.calendar.cache_days_in(database.clone());
    state.notes.go_live(timeslay_core::notes::directory());
    state.people.go_live(database.clone());
    state.travel.cache_in(database.clone());

    state.alarms.keep_in(database.clone());
    state.habits.keep_in(database);

    state
}

/// The desktop-file basename, `timeslay.desktop`.
///
/// On Wayland the title-bar icon is not the pixmap passed to `set_window_icon`
/// (winit ignores that). KWin looks up this id, finds the desktop entry, and
/// paints `Icon=timeslay`. An empty id leaves the generic Wayland logo, the
/// yellow W, in the corner while the dock still shows the hourglass.
const APPLICATION_ID: &str = "timeslay";

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
    let ticking = matches!(
        message,
        Message::System(SystemMessage::FlashFrame(_)) | Message::Task(TaskMessage::WeekDragScroll)
    );
    state.habits.roll_over();
    let task = dispatch(state, message);
    let draft = state.calendar.draft.as_ref();
    let edit = state.calendar.edit_draft.as_ref();
    for (editor, value) in [
        (
            &mut state.form_editors.event_draft,
            draft.map(|d| d.description.as_str()),
        ),
        (
            &mut state.form_editors.event_edit,
            edit.map(|d| d.description.as_str()),
        ),
        (
            &mut state.form_editors.habit_notes,
            state.habits.form.as_ref().map(|form| form.notes.as_str()),
        ),
    ] {
        let value = value.unwrap_or("");
        if editor.text() != value {
            *editor = text_editor::Content::with_text(value);
        }
    }
    let travel = if ticking {
        Task::none()
    } else {
        crate::travel::reconcile(state)
    };
    state.details_pane_open = detail_pane_visible(state);
    let editor = reconcile_editor_window(state);
    let fetch = if !state.calendar_fetch_pending.is_empty()
        && !state.calendar.is_busy()
        && state.timeblocking.busy.is_empty()
    {
        show_cached_calendar(state)
    } else {
        Task::none()
    };
    let activities = state.current_activities();
    state.activity.observe(activities, Instant::now());
    Task::batch([task, editor, travel, fetch])
}

/// Keep the floating range view/editor separate from panel geometry and reservation.
fn reconcile_editor_window(state: &mut State) -> Task<Message> {
    let visible =
        state.floating_editors() && state.main_window.is_some() && !state.panel_expected_close;
    let width = if visible && state.people_panel_open() {
        state.config.views.detail_width()
    } else if visible && state.life_panel_open() {
        ui::life::PANEL_WIDTH
    } else if visible && state.habit_panel_open() {
        ui::habits::PANEL_WIDTH
            + if state.calendar.open_event.is_some() {
                state.config.views.detail_width()
            } else {
                0.0
            }
    } else if state.range_view_active()
        && state.main_window.is_some()
        && !state.panel_expected_close
    {
        state.weekly_window_width()
    } else if visible {
        (if state.day_plan_active() {
            state.calendar_width()
        } else {
            0.0
        }) + (if state.timeblocking.block.is_some() {
            state.config.views.time_block_width()
        } else {
            0.0
        }) + if state.details_pane_open {
            state.config.views.detail_width()
        } else {
            0.0
        }
    } else {
        0.0
    };
    // The map opens in this window even when nothing else would have it open,
    // as when Home is chosen from the settings.
    let width = if width == 0.0 && state.map_in_editor_window() {
        state.config.views.detail_width()
    } else {
        width
    };
    if width == 0.0 {
        return state
            .editor_window
            .take()
            .map(window::close)
            .unwrap_or_else(Task::none);
    }
    let size = Size::new(
        width,
        (state.main_size.height - state.screen_margin_width()).max(1.0),
    );
    if let Some(id) = state.editor_window {
        if state.editor_window_size != size {
            state.editor_window_size = size;
            return window::resize(id, size);
        }
        return Task::none();
    }
    let (id, task) = window::open(window::Settings {
        size,
        decorations: false,
        resizable: false,
        level: window::Level::AlwaysOnTop,
        exit_on_close_request: false,
        ..Default::default()
    });
    state.editor_window = Some(id);
    state.editor_window_size = size;
    task.map(|v0| Message::System(SystemMessage::EditorWindowOpened(v0)))
}

fn detail_pane_visible(state: &State) -> bool {
    (state.calendar.open_event.is_some()
        || (state.inbox.draft.is_none()
            && state.calendar.draft.as_ref().is_some_and(|d| d.editor_open)))
        && shows_details(state.screen)
        && !state.range_view_active()
}

fn range_view_transition(
    state: &mut State,
    was_weekly: bool,
    close: Task<Message>,
) -> Task<Message> {
    if !state.range_view_active() {
        state.overview_month_open = false;
        state.panel_day_wanted = false;
    }
    if was_weekly == state.range_view_active() {
        return close;
    }

    // Range views use the floating surface; the day panel never resizes.
    if state.range_view_active()
        && let Some(id) = state.main_window
    {
        return close.chain(
            window::monitor_size(id)
                .map(move |size| Message::System(SystemMessage::WeekMonitorSized(id, size))),
        );
    }
    close
}

/// Destinations that support the shared details editor.
pub(crate) fn shows_details(screen: Screen) -> bool {
    matches!(
        screen,
        Screen::Calendar
            | Screen::Backlog
            | Screen::Overdue
            | Screen::Inbox
            | Screen::Today
            | Screen::Triaged
            | Screen::Habits
    )
}

fn dispatch(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::TimeBlocking(action) => crate::timeblocking::update(state, action),
        Message::BrainDump(action) => crate::brain_dump::update(state, action),
        Message::Inbox(action) => {
            let task = crate::inbox::update(state, action);
            if state.task_creation_window && state.inbox.draft.is_none() {
                return Task::batch([task, iced::exit()]);
            }
            task
        }
        Message::Accounts(action) => crate::integrations::accounts::update(state, action),
        Message::People(action) => crate::people::update(state, action),
        Message::Alarms(action) => alarms::update(state, action),
        Message::Reminders(action) => crate::reminders::update(state, action),
        Message::Life(action) => crate::life::update(state, action),
        Message::Travel(action) => crate::travel::update(state, action),
        Message::Habits(action) => crate::habits::update(state, action),
        Message::Picker(action) => crate::ui::controls::update(state, action),
        Message::Ignored => Task::none(),
        Message::OpenEventLink(url) => {
            if let Err(error) = calendar::oauth::open_in_browser(&url) {
                state.calendar.error = Some(error);
            }
            Task::none()
        }
        Message::Navigation(message) => on_navigation::update(state, message),
        Message::Notepad(message) => on_notepad::update(state, message),
        Message::Settings(message) => on_settings::update(state, message),
        Message::System(message) => on_system::update(state, message),
        Message::Notes(action) => crate::notes::update(state, action),
        Message::Calendar(message) => on_calendar::update(state, message),
        Message::Task(message) => on_task::update(state, message),
        Message::Event(message) => on_event::update(state, message),
        Message::EventDraft(message) => on_event_draft::update(state, message),
        Message::EventEdit(message) => on_event_edit::update(state, message),
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

    if state.editor_window == Some(window) {
        return ui::editor_window(state);
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

    let name = state.config.appearance.theme.as_str();
    match name {
        // Returning `None` is how iced is told to follow the system color
        // scheme, which is what the old follow-system toggle did.
        theme::SYSTEM => None,
        theme::SYSTEM_HIGH_CONTRAST => {
            let name = match state.system_theme {
                iced::theme::Mode::Dark => "Dark high contrast",
                iced::theme::Mode::None | iced::theme::Mode::Light => "Light high contrast",
            };
            Some(state.catalog.resolve(name).theme.clone())
        }
        _ => Some(state.catalog.resolve(name).theme.clone()),
    }
}

pub fn title(state: &State, window: window::Id) -> String {
    if state.task_creation_window {
        return "New task — TimeSlay".into();
    }
    if state
        .flash
        .as_ref()
        .is_some_and(|flash| flash.window == window)
    {
        return "TimeSlay".to_owned();
    }

    // The panel carries a stable, distinctive title so a window rule can pick
    // it out from the main window; see the packaging README.
    if state.sidebar.is_some() {
        return "TimeSlay Sidebar".to_owned();
    }

    "TimeSlay".to_owned()
}

pub fn subscription(state: &State) -> Subscription<Message> {
    let mut subscriptions = vec![
        crate::travel::map::subscription(state),
        iced::time::every(std::time::Duration::from_secs(2))
            .map(|_| Message::System(SystemMessage::DatabaseSyncPolled)),
        if state.activity.rotating() {
            iced::time::every(std::time::Duration::from_millis(500))
                .map(|_| Message::System(SystemMessage::ActivityTick))
        } else {
            Subscription::none()
        },
        config::watch().map(|v0| Message::System(SystemMessage::ConfigReloaded(v0))),
        iced::system::theme_changes()
            .map(|mode| Message::System(SystemMessage::SystemThemeMode(mode))),
        window::close_events().map(|v0| Message::System(SystemMessage::WindowClosed(v0))),
        window::resize_events()
            .map(|(id, size)| Message::System(SystemMessage::WindowResized(id, size))),
        if state
            .calendar
            .draft
            .as_ref()
            .is_some_and(|draft| !draft.editor_open)
        {
            // Text inputs capture Escape when focused. The popup must still close.
            iced::event::listen_with(|event, status, _| match event {
                iced::Event::Keyboard(event)
                    if status == iced::event::Status::Ignored
                        || matches!(
                            event,
                            keyboard::Event::KeyPressed {
                                key: Key::Named(keyboard::key::Named::Escape),
                                ..
                            }
                        ) =>
                {
                    Some(map_key(event))
                }
                _ => None,
            })
        } else {
            keyboard::listen().map(map_key)
        },
        iced::event::listen_with(|event, status, window| {
            if status == iced::event::Status::Ignored
                && matches!(
                    event,
                    iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                        iced::mouse::Button::Left
                    ))
                )
            {
                Some(Message::Task(TaskMessage::TaskDragReleased(window)))
            } else {
                None
            }
        }),
    ];

    // Letting go of a habit row anywhere, even outside the list, ends the drag.
    if state.habits.dragging.is_some() {
        subscriptions.push(iced::event::listen_with(|event, status, _window| {
            (status == iced::event::Status::Ignored
                && matches!(
                    event,
                    iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
                        iced::mouse::Button::Left
                    ))
                ))
            .then_some(Message::Habits(crate::habits::Action::DragEnded))
        }));
    }

    // While a task is in the air, every move of the pointer is fed back so the
    // floating card can follow it. It is only listened for during a drag, so
    // an idle window still costs nothing.
    if state.timeblocking.dragging.is_some() {
        subscriptions.push(iced::event::listen_with(
            |event, _status, window| match event {
                iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                    Some(Message::Task(TaskMessage::TaskDragMoved(window, position)))
                }
                _ => None,
            },
        ));
    }

    if state.weekly()
        && state.config.calendar.weekly_task_list
        && state.timeblocking.dragging.is_some()
    {
        subscriptions.push(
            iced::time::every(Duration::from_millis(50))
                .map(|_| Message::Task(TaskMessage::WeekDragScroll)),
        );
    }

    // Refresh the calendar needle and the Overdue list as time passes.
    if state.range_view_active()
        || state.day_plan_active()
        || matches!(state.screen, Screen::Calendar | Screen::Overdue)
    {
        subscriptions.push(
            iced::time::every(NEEDLE_TICK)
                .map(|_| Message::Calendar(CalendarMessage::CalendarTick)),
        );
    }

    // Alarms are checked once a second, and only while there is one waiting,
    // so an idle TimeSlay with nothing set still costs nothing.
    if alarms::needs_poll(&state.alarms) {
        subscriptions.push(
            iced::time::every(Duration::from_secs(1))
                .map(|_| Message::Alarms(alarms::Action::Poll)),
        );
    }
    // Reminders read the day cache on a worker thread every poll. With them
    // switched off nothing is polled at all.
    if crate::reminders::needs_poll(state) {
        subscriptions.push(
            iced::time::every(crate::reminders::POLL_INTERVAL)
                .map(|_| Message::Reminders(crate::reminders::Action::Poll)),
        );
    }
    if alarms::needs_flush(&state.alarms) {
        subscriptions.push(
            iced::time::every(config::SAVE_DEBOUNCE)
                .map(|_| Message::Alarms(alarms::Action::Flush)),
        );
    }

    if state.timeblocking.focus.is_some()
        || state.config.task_work.active().is_some()
        || state.config.pomodoro.running()
    {
        subscriptions.push(
            iced::time::every(Duration::from_secs(1))
                .map(|_| Message::TimeBlocking(crate::timeblocking::Action::WorkTick)),
        );
    }
    if state.calendar.is_signed_in() {
        subscriptions.push(
            iced::time::every(state.config.calendar.sync_every())
                .map(|_| Message::Calendar(CalendarMessage::CalendarAutoSyncRequested)),
        );
    }

    if state.flash.is_some() {
        subscriptions.push(
            iced::time::every(FLASH_FRAME).map(|v0| Message::System(SystemMessage::FlashFrame(v0))),
        );
    }

    if matches!(state.screen, Screen::Settings) {
        subscriptions.push(
            iced::time::every(STATUS_POLL)
                .map(|_| Message::System(SystemMessage::AudioStatusPolled)),
        );
    }

    if state.notes.wants_tick() {
        subscriptions.push(
            iced::time::every(config::SAVE_DEBOUNCE)
                .map(|_| Message::Notes(crate::notes::Action::Tick)),
        );
    }

    // The debounce itself: the write lands once the interval elapses, and
    // further edits in the meantime ride along on the same pending save.
    if state.save_queued {
        subscriptions.push(
            iced::time::every(config::SAVE_DEBOUNCE)
                .map(|_| Message::System(SystemMessage::SaveRequested)),
        );
    }

    // The side panel polls for show/hide commands from the panel daemon.
    if state.panel_command_rx.is_some() {
        subscriptions.push(
            iced::time::every(Duration::from_millis(200))
                .map(|_| Message::System(SystemMessage::PanelCommandPolled)),
        );
    }

    if state.inbox.voice.active() {
        subscriptions.push(
            iced::time::every(Duration::from_millis(if state.inbox.voice.recording() {
                16
            } else {
                80
            }))
            .map(|_| Message::Inbox(crate::inbox::Action::Voice(crate::voice::Action::Poll))),
        );
    }
    // The extension and the Slack app add tasks from their own processes.
    if state.inbox_added_rx.is_some() {
        subscriptions.push(
            iced::time::every(Duration::from_millis(200))
                .map(|_| Message::System(SystemMessage::InboxAddedPolled)),
        );
    }

    Subscription::batch(subscriptions)
}

fn map_key(event: keyboard::Event) -> Message {
    match event {
        keyboard::Event::KeyPressed { key, modifiers, .. } => {
            Message::System(SystemMessage::KeyPressed(key, modifiers))
        }
        _ => Message::Ignored,
    }
}

fn on_key(state: &mut State, key: Key, modifiers: keyboard::Modifiers) -> Task<Message> {
    if state.description_popup.is_some() {
        return if key == keyboard::Key::Named(keyboard::key::Named::Escape) {
            update(state, Message::Event(EventMessage::DescriptionClose))
        } else {
            Task::none()
        };
    }
    if state
        .calendar
        .draft
        .as_ref()
        .is_some_and(|draft| !draft.editor_open)
        && key == Key::Named(keyboard::key::Named::Tab)
    {
        return iced::widget::operation::is_focused("calendar-draft-title").map(
            move |title_focused| {
                Message::EventDraft(EventDraftMessage::EventDraftCycleKind {
                    reverse: modifiers.shift(),
                    title_focused,
                })
            },
        );
    }
    if state
        .calendar
        .draft
        .as_ref()
        .is_some_and(|draft| !draft.editor_open)
        && matches!(
            key.as_ref(),
            Key::Named(keyboard::key::Named::Enter) | Key::Named(keyboard::key::Named::Space)
        )
        && let Some(kind) = state.draft_kind_focus
    {
        return iced::widget::operation::is_focused("calendar-draft-title").map(move |focused| {
            if focused {
                Message::Ignored
            } else {
                Message::EventDraft(EventDraftMessage::EventDraftKindSelected(kind))
            }
        });
    }
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
    if state.calendar.pending_move.is_some() {
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::Event(EventMessage::EventMoveCancelled)),
            _ => None,
        };
    }
    if state
        .calendar
        .draft
        .as_ref()
        .is_some_and(|draft| !draft.editor_open)
    {
        return match key.as_ref() {
            Key::Named(Named::Escape) => {
                Some(Message::EventDraft(EventDraftMessage::EventDraftCancelled))
            }
            Key::Named(Named::Enter) | Key::Named(Named::Space) => state
                .draft_kind_focus
                .map(|kind| Message::EventDraft(EventDraftMessage::EventDraftKindSelected(kind))),
            _ => None,
        };
    }
    if state.travel.map.active {
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::Travel(crate::travel::Action::Map(
                crate::travel::map::Action::Close,
            ))),
            _ => None,
        };
    }
    if state.inbox.voice.debug_open {
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::Inbox(crate::inbox::Action::Voice(
                crate::voice::Action::Debug(false),
            ))),
            _ => None,
        };
    }
    if let Some(picker) = &state.picker {
        use crate::ui::controls::Action;
        return match key {
            Key::Named(Named::Escape) => Some(Message::Picker(Action::Close)),
            Key::Named(Named::ArrowLeft) => Some(Message::Picker(Action::Move(-1))),
            Key::Named(Named::ArrowRight) => Some(Message::Picker(Action::Move(1))),
            Key::Named(Named::ArrowUp) => Some(Message::Picker(Action::Move(-7))),
            Key::Named(Named::ArrowDown) => Some(Message::Picker(Action::Move(7))),
            Key::Named(Named::Enter) => Some(Message::Picker(Action::Select(picker.focus))),
            _ => None,
        };
    }

    // A task that is in the air is put back where it started before anything
    // else looks at the key, so Escape cancels the drag instead of closing the
    // view underneath it.
    if state.timeblocking.dragging.is_some() {
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::TimeBlocking(
                crate::timeblocking::Action::CancelDrag,
            )),
            _ => None,
        };
    }

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
    if state.view_menu_open {
        return match key.as_ref() {
            Key::Named(Named::Escape) => {
                Some(Message::Navigation(NavigationMessage::ViewMenuToggled))
            }
            _ => None,
        };
    }
    if state.more_menu_open {
        return match key.as_ref() {
            Key::Named(Named::Escape) => {
                Some(Message::Navigation(NavigationMessage::MoreMenuToggled))
            }
            _ => None,
        };
    }
    if state.activity.open {
        return match key.as_ref() {
            Key::Named(Named::Escape) => {
                Some(Message::Navigation(NavigationMessage::ActivityLogToggled))
            }
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
    if state.habits.day_dialog.is_some() {
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::Habits(crate::habits::Action::CloseDay)),
            _ => None,
        };
    }
    if state.habits.confirm_delete.is_some() {
        return match key.as_ref() {
            Key::Named(Named::Escape) => {
                Some(Message::Habits(crate::habits::Action::DeleteCancelled))
            }
            _ => None,
        };
    }
    if state.habits.section_edit.is_some() {
        // Naming a section must not reach the clock shortcuts.
        return match key.as_ref() {
            Key::Named(Named::Escape) => {
                Some(Message::Habits(crate::habits::Action::CancelSection))
            }
            _ => None,
        };
    }
    if state.habits.form.is_some() {
        // Typing into the habit form must not reach the clock shortcuts.
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::Habits(crate::habits::Action::CancelEdit)),
            _ => None,
        };
    }
    if state.inbox.draft.is_some() {
        // Typing into the popup must not reach the clock shortcuts. Escape
        // leaves the calendar preview first, back to the form it came from.
        return match key.as_ref() {
            Key::Named(Named::Escape) if state.inbox.preview.is_some() => Some(Message::Inbox(
                crate::inbox::Action::Preview(crate::inbox::preview::PreviewAction::Close),
            )),
            Key::Named(Named::Escape) => Some(Message::Inbox(crate::inbox::Action::Cancel)),
            _ => None,
        };
    }
    if state.brain_dump.open.is_some() {
        // The page is an editor: only Escape leaves it, and no shortcut may
        // steal a space while it is being written.
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::BrainDump(crate::brain_dump::Action::Close)),
            _ => None,
        };
    }
    if state.timeblocking.distraction_open {
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::TimeBlocking(
                crate::timeblocking::Action::CloseDistraction,
            )),
            _ => None,
        };
    }
    if state.timeblocking.session_edit.is_some() {
        return match key.as_ref() {
            Key::Named(Named::Escape) => Some(Message::TimeBlocking(
                crate::timeblocking::Action::CloseSession,
            )),
            _ => None,
        };
    }
    if state.timeblocking.focus.is_some() {
        return match key.as_ref() {
            // A task opened from the view closes before the view does.
            Key::Named(Named::Escape) if state.calendar.open_event.is_some() => {
                Some(Message::Event(EventMessage::EventClosed))
            }
            Key::Named(Named::Escape) => Some(Message::TimeBlocking(
                crate::timeblocking::Action::CloseView,
            )),
            _ => None,
        };
    }
    if state.floating_editors()
        && state.calendar.open_event.is_none()
        && state.timeblocking.block.is_some()
        && key == Key::Named(Named::Escape)
    {
        return Some(Message::TimeBlocking(crate::timeblocking::Action::Close));
    }
    if state.calendar.draft.is_some() || state.calendar.edit_draft.is_some() {
        return match key.as_ref() {
            Key::Named(Named::Escape) if state.calendar.draft.is_some() => {
                Some(Message::EventDraft(EventDraftMessage::EventDraftCancelled))
            }
            Key::Named(Named::Escape) => Some(Message::Event(EventMessage::EventClosed)),
            _ => None,
        };
    }
    if state.two_week_more.is_some()
        && state.calendar.open_event.is_none()
        && key == Key::Named(Named::Escape)
    {
        return Some(Message::Calendar(CalendarMessage::CalendarTwoWeekMore(
            None,
        )));
    }
    if modifiers.command() {
        // A range view stands in for whichever screen is selected, so the grid
        // is on show in those as well as on the Calendar screen itself.
        let grid_showing = state.range_view_active()
            || state.day_plan_active()
            || state.screen == Screen::Calendar;
        let zoom = |factor| {
            Some(Message::Calendar(CalendarMessage::CalendarZoomed {
                factor,
                anchor: None,
            }))
        };
        return match key.as_ref() {
            Key::Character(",") => Some(Message::Navigation(NavigationMessage::ScreenSelected(
                Screen::Settings,
            ))),
            // `=` and `_` are the same keys as `+` and `-`, with Shift the other
            // way round, so nobody has to hold Shift to zoom in.
            Key::Character("+" | "=") if grid_showing => zoom(calendar::ZOOM_STEP),
            Key::Character("-" | "_") if grid_showing => zoom(1.0 / calendar::ZOOM_STEP),
            Key::Character("0") if grid_showing => {
                Some(Message::Calendar(CalendarMessage::CalendarZoomReset))
            }
            _ => None,
        };
    }

    // The notepad is for writing, so the shortcuts are left off while it is on
    // show: a space would otherwise silence an alarm halfway through a word.
    if matches!(
        state.screen,
        Screen::Notepad
            | Screen::BrainDump
            | Screen::Notes
            | Screen::People
            | Screen::Overdue
            | Screen::Inbox
            | Screen::Today
            | Screen::Triaged
    ) {
        return match key.as_ref() {
            // An open note comes away first; a second press leaves the screen.
            Key::Named(Named::Escape)
                if state.screen == Screen::Notes && state.notes.open.is_some() =>
            {
                Some(Message::Notes(crate::notes::Action::Back))
            }
            // An open contact comes away first, the same as a task's details.
            Key::Named(Named::Escape) if state.people_panel_open() => {
                Some(Message::People(crate::people::Action::Close))
            }
            // A task's details come away first; a second press leaves.
            Key::Named(Named::Escape)
                if matches!(
                    state.screen,
                    Screen::Overdue | Screen::Inbox | Screen::Today | Screen::Triaged
                ) && state.calendar.open_event.is_some() =>
            {
                Some(Message::Event(EventMessage::EventClosed))
            }
            Key::Named(Named::Escape) => Some(Message::Navigation(
                NavigationMessage::ScreenSelected(Screen::Calendar),
            )),
            _ => None,
        };
    }

    match key.as_ref() {
        // The clock no longer has a place on the calendar, so a space only
        // has something to do while an alarm is ringing.
        Key::Named(Named::Space) if state.alarm_active => {
            Some(Message::System(SystemMessage::Dismiss))
        }
        Key::Named(Named::Escape) => {
            if state.life_panel_open() {
                Some(Message::Life(crate::life::Action::Close))
            } else if state.calendar.open_event.is_some() {
                Some(Message::Event(EventMessage::EventClosed))
            } else if state.timeblocking.block.is_some() {
                Some(Message::TimeBlocking(crate::timeblocking::Action::Close))
            } else if state.calendar.menu_open {
                state.calendar.menu_open = false;
                None
            } else if state.calendar.search.open {
                state.calendar.search.open = false;
                None
            } else if state.calendar.actions_open {
                state.calendar.actions_open = false;
                None
            } else if state.alarm_active {
                Some(Message::System(SystemMessage::Dismiss))
            } else if state.screen != Screen::Calendar {
                Some(Message::Navigation(NavigationMessage::ScreenSelected(
                    Screen::Calendar,
                )))
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
pub(crate) fn fire(state: &mut State, profile: &config::AlarmProfile) -> Task<Message> {
    let actions = alarm::on_finished(profile);

    state.alarm_active = state.alarm_active || actions.is_dismissable();

    perform(state, actions)
}

pub(crate) fn perform(state: &mut State, actions: alarm::Actions) -> Task<Message> {
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
                .map(move |result| Message::System(SystemMessage::SoundRendered(result, spec))),
        );
    }

    if let Some(notification) = actions.notification {
        tasks.push(
            blocking::run(move || notify::show(&notification))
                .map(|v0| Message::System(SystemMessage::NotificationPosted(v0))),
        );
    }

    if let Some(settings) = actions.flash {
        tasks.push(open_flash(state, settings));
    }

    if actions.raise
        && let Some(id) = state.main_window
    {
        tasks.push(window::gain_focus(id));
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
/// Binds `shortcuts` to `action`. The config only takes them once the desktop
/// has accepted them, so the settings never list a key that is not bound.
fn apply_shortcuts(
    state: &mut State,
    action: panel::Action,
    shortcuts: Vec<String>,
) -> Task<Message> {
    let editor = &mut state.shortcut_editors[action.index()];
    editor.busy = true;
    editor.result = None;

    blocking::run(move || {
        panel::rebind(action, &shortcuts).map_err(|error| format!("{error:#}"))?;
        Ok(shortcuts)
    })
    .map(move |result| Message::Settings(SettingsMessage::ShortcutsApplied(action, result)))
}

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

    let day = calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
    let displayed = if state.range_view_active() {
        state.week_anchor
    } else {
        state.calendar.day
    };
    if day != displayed {
        state.calendar.set_day(day, day);
        if state.range_view_active() {
            state.week_anchor = day;
        }
        return show_cached_calendar(state);
    }
    Task::none()
}

pub(crate) fn refresh_week_after_write(state: &mut State) -> Task<Message> {
    let overlay = crate::local_poll::refresh_overlay(state);
    state.timeblocking.revision += 1;
    Task::batch([
        overlay,
        Task::done(Message::Calendar(CalendarMessage::CalendarSyncRequested)),
    ])
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
        let now = crate::clock::utc_now();
        state.config.task_work.stop(&key, now);
        state.timeblocking.work_now = Some(now);
        queue_save(state)
    } else {
        Task::none()
    }
}

/// Remembers the id of the app-owned TimeSlay calendar from a fresh calendar
/// list, so time blocks and scheduled tasks are recognized even after the id
/// changes (a re-created calendar, or a different account). Returns whether the
/// id changed and needs saving.
fn remember_app_calendar(state: &mut State, snapshot: &calendar::Snapshot) -> bool {
    let app = |calendar: &&calendar::Calendar| {
        !calendar.is_task_list()
            && calendar.name == crate::config::CalendarSettings::APP_CALENDAR_NAME
    };
    let found = snapshot.calendars.iter().find(app);
    match found {
        Some(app) if state.config.calendar.app_calendar.as_deref() != Some(app.id.as_str()) => {
            state.config.calendar.app_calendar = Some(app.id.clone());
            true
        }
        _ => false,
    }
}

fn load_cached_calendar(state: &mut State) {
    if state.range_view_active() {
        let days = state.calendar_dates();
        state.calendar.load_cached_days(&days);
    } else {
        state.calendar.load_cached_days(&[]);
    }
}

fn empty_calendar_days(state: &State) -> Vec<chrono::NaiveDate> {
    let days = if state.range_view_active() {
        state.calendar_dates()
    } else {
        vec![state.calendar.day]
    };
    days.into_iter()
        .filter(|&day| !state.calendar.has_calendar_events_on(day))
        .collect()
}

fn show_cached_calendar(state: &mut State) -> Task<Message> {
    load_cached_calendar(state);
    if !state.calendar.is_signed_in() {
        return Task::none();
    }
    let missing = empty_calendar_days(state);
    if missing.is_empty() {
        state.calendar_fetch_pending.clear();
        return Task::none();
    }
    state.calendar_fetch_pending = missing;
    sync_calendar(state)
}

fn fetch_pending_calendar_days(state: &mut State, fetched: &[chrono::NaiveDate]) -> Task<Message> {
    if state.panel_day_wanted {
        return sync_calendar(state);
    }
    let pending = std::mem::take(&mut state.calendar_fetch_pending);
    // Only navigation to dates outside the completed request needs a follow-up.
    // A successful, genuinely empty day must not start an endless fetch loop.
    if pending.iter().any(|day| !fetched.contains(day)) {
        show_cached_calendar(state)
    } else {
        Task::none()
    }
}

fn auto_sync_calendar(state: &mut State) -> Task<Message> {
    if state
        .calendar
        .sync_due(state.config.calendar.sync_every(), crate::clock::now())
    {
        sync_calendar(state)
    } else {
        show_cached_calendar(state)
    }
}

fn sync_calendar(state: &mut State) -> Task<Message> {
    load_cached_calendar(state);
    if !state.calendar.is_signed_in()
        || state.calendar.is_busy()
        || state.calendar.pending_move.is_some()
        || state.calendar.events.iter().any(|event| {
            event.details.sleep.is_some() && state.calendar.pending_time(&event.id).is_some()
        })
        || !state.timeblocking.busy.is_empty()
        || state
            .calendar
            .edit_draft
            .as_ref()
            .is_some_and(|d| d.sleep.is_some() && d.saving)
    {
        return Task::none();
    }

    // The range's own pending days wait for the panel's day, not the other way.
    let panel_day = std::mem::take(&mut state.panel_day_wanted);
    if !panel_day {
        state.calendar_fetch_pending.clear();
    }
    state.calendar.sync = SyncState::Syncing;
    state.calendar.error = None;

    let settings = state.config.calendar.clone();
    let day = state.calendar.day;
    let revision = state.timeblocking.revision;
    if state.range_view_active() && !panel_day {
        let anchor = state.week_anchor;
        let dates = state.calendar_dates();
        let fetched_dates = dates.clone();
        return blocking::run(move || calendar::sleep::sync(&settings, anchor, &fetched_dates))
            .map(move |result| {
                Message::Calendar(CalendarMessage::CalendarRangeSynced(
                    revision,
                    dates.clone(),
                    result,
                ))
            });
    }
    blocking::run(move || calendar::sleep::sync(&settings, day, &[day]))
        .map(move |result| Message::Calendar(CalendarMessage::CalendarSyncedAt(revision, result)))
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
fn panel_ipc_listener(
    listener: UnixListener,
    tx: mpsc::Sender<PanelCommand>,
    visible: Arc<AtomicBool>,
    inbox: mpsc::Sender<calendar::Event>,
) -> AnyhowResult<()> {
    log::info!("panel IPC socket listening");

    crate::ipc::serve(listener, |command| {
        if command.starts_with('{') {
            let handled = crate::webextension::handle_ipc(command);
            if let Some(event) = handled.event {
                let _ = inbox.send(event);
            }
            return handled.reply;
        }
        match command {
            "toggle-reservation" => {
                let _ = tx.send(PanelCommand::ToggleReservation);
                crate::ipc::OK.to_owned()
            }
            "show" => {
                // Answered from here rather than from the window, so that a
                // `status` straight afterwards is already right.
                visible.store(true, Ordering::Relaxed);
                let _ = tx.send(PanelCommand::Show);
                crate::ipc::OK.to_owned()
            }
            "hide" => {
                visible.store(false, Ordering::Relaxed);
                let _ = tx.send(PanelCommand::Hide);
                crate::ipc::OK.to_owned()
            }
            "status" if visible.load(Ordering::Relaxed) => crate::ipc::VISIBLE.to_owned(),
            "status" => crate::ipc::HIDDEN.to_owned(),
            other => {
                log::warn!("unknown panel IPC command: {other:?}");
                String::new()
            }
        }
    })
}
