//! The day view's data model.
//!
//! Sync, OAuth, Google Tasks metadata and time-block assignment all come from
//! the existing `timeslay` library, so this crate only has to arrange that
//! data for the screen: which events are timed, which are all-day, which task
//! belongs to which block, and how overlaps share a column.

use std::collections::{HashMap, HashSet};
use timeslay_core::calendar::ids;

use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Timelike};
use timeslay_core::calendar::first_day::FirstDayOfWeek;
use timeslay_core::calendar::layout::{self, Span};
use timeslay_core::calendar::search::Search;
use timeslay_core::calendar::sources::TimeSlaySource;
use timeslay_core::calendar::time_format::TimeFormat;
use timeslay_core::calendar::{self, Event, Snapshot};
use timeslay_core::config::{CalendarSettings, Config, ProjectsSettings};
use timeslay_core::places::{self, Found, Place, PlaceSettings};
use timeslay_core::task_edit::{self, Change};
use timeslay_core::task_input;
use timeslay_core::timeblocking as tb;

/// Reads the same configuration the iced app uses.
pub fn load_config() -> Config {
    Config::load()
}

/// Which colour scheme the view uses: the choices the desktop's theme picker
/// offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    Light,
    LightHighContrast,
    Dark,
    DarkHighContrast,
    /// Follow the operating system's light/dark setting.
    #[default]
    System,
    SystemHighContrast,
}

impl Theme {
    /// In the order the desktop's theme picker lists them.
    pub const ALL: [Self; 6] = [
        Self::Light,
        Self::LightHighContrast,
        Self::Dark,
        Self::DarkHighContrast,
        Self::System,
        Self::SystemHighContrast,
    ];

    /// What is saved, and what the stylesheet selects on. The Android widget
    /// reads the part before the first `-`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::LightHighContrast => "light-high-contrast",
            Self::Dark => "dark",
            Self::DarkHighContrast => "dark-high-contrast",
            Self::System => "system",
            Self::SystemHighContrast => "system-high-contrast",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::LightHighContrast => "Light high contrast",
            Self::Dark => "Dark",
            Self::DarkHighContrast => "Dark high contrast",
            Self::System => "System",
            Self::SystemHighContrast => "System high contrast",
        }
    }

    pub fn parse(text: &str) -> Self {
        let text = text.trim();
        Self::ALL
            .into_iter()
            .find(|theme| theme.name() == text)
            .unwrap_or_default()
    }

    /// A per-device choice, so it is in the database and not the synced config.
    /// The saved choice, or `System` when nothing has been saved.
    pub fn load() -> Self {
        timeslay_core::task_db::path()
            .and_then(|path| {
                timeslay_core::task_db::read_document::<String>(
                    &path,
                    timeslay_core::task_db::keys::THEME,
                )
                .ok()
                .flatten()
            })
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn save(self) {
        let Some(path) = timeslay_core::task_db::path() else {
            return;
        };
        let _ = timeslay_core::task_db::write_document(
            &path,
            timeslay_core::task_db::keys::THEME,
            self.name(),
        );
    }
}

/// The calendar day that `now` belongs to, honoring the configured end of day.
pub fn today(settings: &CalendarSettings) -> NaiveDate {
    calendar::day_at(timeslay_core::clock::now(), settings.day_start_hour())
}

/// Pulls calendars, events and Google Tasks for one day.
///
/// Blocking: callers run this on a worker thread. It uses the same OAuth token
/// file as the iced app, so signing in there is enough for the Dioxus view.
pub fn load_day(day: NaiveDate, settings: &CalendarSettings) -> Result<Snapshot, String> {
    calendar::sleep::sync(settings, day, &[day])
}

/// A day's snapshot narrowed to what a search shows. Only what is drawn goes
/// through this: writes lay out around the whole day, not the filtered one.
pub fn filter_snapshot(
    snapshot: &Snapshot,
    search: &Search,
    settings: &CalendarSettings,
) -> Snapshot {
    let mut filtered = snapshot.clone();
    filtered
        .events
        .retain(|event| search.shows(event, &snapshot.events, settings));
    filtered
}

/// The labels and locations a search can choose from on this day.
pub fn search_choices(events: &[Event]) -> (Vec<String>, Vec<String>) {
    let mut labels: Vec<String> = events
        .iter()
        .flat_map(|event| tb::metadata(event).labels)
        .collect();
    labels.sort_by_key(|label| label.to_lowercase());
    labels.dedup();
    let mut locations: Vec<String> = events
        .iter()
        .map(|event| event.details.location.trim().to_owned())
        .filter(|location| !location.is_empty())
        .collect();
    locations.sort();
    locations.dedup();
    (labels, locations)
}

/// A task as the quick-add box reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuickTask {
    pub title: String,
    pub due: Option<NaiveDate>,
    pub deadline: Option<NaiveDate>,
    pub metadata: tb::Metadata,
}

/// Reads the keywords out of what was typed, like the desktop's add something box.
pub fn quick_task(text: &str, today: NaiveDate, projects: &ProjectsSettings) -> QuickTask {
    let tokens = task_input::detect(text, today, projects);
    let mut task = QuickTask {
        title: task_input::remove(text, tokens.iter().map(|token| token.range.clone())),
        ..QuickTask::default()
    };
    for token in tokens {
        match token.value {
            task_input::Value::Due(day) => task.due = Some(day),
            task_input::Value::Deadline(day) => task.deadline = Some(day),
            task_input::Value::Priority(priority) => task.metadata.priority = Some(priority),
            task_input::Value::Minutes(minutes) => task.metadata.minutes = minutes,
            task_input::Value::Label(label) => {
                if !task.metadata.labels.contains(&label) {
                    task.metadata.labels.push(label);
                }
            }
            task_input::Value::Project(id) => {
                if !task.metadata.projects.contains(&id) {
                    task.metadata.projects.push(id);
                }
            }
        }
    }
    task
}

/// Adds a task to the inbox list, or to `day` when it names no day of its own.
///
/// Blocking: callers run this on a worker thread.
pub fn create_task(
    mut task: QuickTask,
    day: Option<NaiveDate>,
    settings: &CalendarSettings,
) -> Result<Event, String> {
    let title = task.title.trim();
    if title.is_empty() {
        return Err("A task needs a name".to_owned());
    }
    if title.chars().count() > task_edit::MAX_TITLE_CHARS {
        return Err(format!(
            "A task name can be at most {} characters",
            task_edit::MAX_TITLE_CHARS
        ));
    }
    task.metadata.deadline = task
        .deadline
        .map(|date| calendar::deadline::Deadline { date, time: None });
    let token = task_edit::google_access(settings)?;
    task_edit::create_inbox_task(
        settings,
        &token,
        title,
        "",
        task.due.or(day),
        Some(task.metadata),
    )
}

/// What the new event and time block forms hold.
#[derive(Clone, Debug, PartialEq)]
pub struct NewEvent {
    pub title: String,
    pub description: String,
    pub date: String,
    /// The last day, when it is not the start day. Empty means the same day,
    /// or the next one for a timed event whose end is before its start.
    pub end_date: String,
    pub start: String,
    pub end: String,
    /// It covers whole days, so the clock times are ignored.
    pub all_day: bool,
}

impl NewEvent {
    /// When it starts and ends, or why that cannot be saved. An all-day
    /// event runs from the first midnight to the one after its last day,
    /// which is how Google stores it.
    pub fn range(&self) -> Result<(DateTime<Local>, DateTime<Local>), String> {
        if self.title.trim().is_empty() {
            return Err("Enter a title".to_owned());
        }
        let first = calendar::deadline::parse_date(self.date.trim())?;
        let last = match self.end_date.trim() {
            "" => None,
            text => Some(calendar::deadline::parse_date(text)?),
        };
        if last.is_some_and(|last| last < first) {
            return Err("The end date has to be on or after the start".to_owned());
        }
        if self.all_day {
            let last = last.unwrap_or(first);
            let after = last.succ_opt().ok_or("Invalid end date")?;
            return Ok((
                calendar::at_clock(first, 0, 0),
                calendar::at_clock(after, 0, 0),
            ));
        }
        let (start, mut end) = task_edit::schedule_range(&self.date, &self.start, &self.end)?;
        if let Some(last) = last.filter(|last| *last != first) {
            end = calendar::at_clock(last, end.hour(), end.minute());
        }
        if end <= start {
            return Err("The end time has to be after the start".to_owned());
        }
        Ok((start, end))
    }
}

/// Blocking: callers run this on a worker thread.
pub fn create_event(new: &NewEvent, calendar_id: &str) -> Result<(), String> {
    let (start, end) = new.range()?;
    calendar::create_event(
        calendar_id,
        new.title.trim(),
        &new.description,
        None,
        start,
        end,
        &[],
        &[],
        new.all_day,
        None,
        None,
    )
    .map(|_| ())
}

/// Blocking: callers run this on a worker thread.
pub fn create_time_block(new: &NewEvent, settings: &CalendarSettings) -> Result<(), String> {
    let (start, end) = new.range()?;
    calendar::create_time_block(
        settings.app_calendar.as_deref(),
        new.title.trim(),
        &calendar::event_description(&new.description),
        None,
        start,
        end,
        &[],
    )
    .map(|_| ())
}

/// One entry of the agenda.
#[derive(Clone, Debug, PartialEq)]
pub struct AgendaRow {
    pub event: Event,
    pub time: String,
    pub color: timeslay_core::Color,
}

/// Blocking: callers run this on a worker thread.
pub fn load_agenda(dates: &[NaiveDate], settings: &CalendarSettings) -> Result<Snapshot, String> {
    let first = *dates.first().ok_or("The range is empty")?;
    calendar::sync_range(
        &calendar::GoogleClient::from_settings(settings),
        &settings.disabled,
        first,
        dates,
        settings.day_start_hour(),
        settings.app_calendar.as_deref(),
    )
}

/// The days of a range with what is on each, all-day items first. A task that
/// sits inside a block is left to that block, and days with nothing are kept
/// so the range still reads as a calendar.
pub fn agenda(
    snapshot: &Snapshot,
    dates: &[NaiveDate],
    settings: &CalendarSettings,
) -> Vec<(NaiveDate, Vec<AgendaRow>)> {
    agenda_rows(snapshot, dates, settings, false)
}

pub fn schedule_agenda(
    snapshot: &Snapshot,
    dates: &[NaiveDate],
    settings: &CalendarSettings,
) -> Vec<(NaiveDate, Vec<AgendaRow>)> {
    agenda_rows(snapshot, dates, settings, true)
}

fn agenda_rows(
    snapshot: &Snapshot,
    dates: &[NaiveDate],
    settings: &CalendarSettings,
    schedule: bool,
) -> Vec<(NaiveDate, Vec<AgendaRow>)> {
    let hour = settings.day_start_hour();
    let assigned = tb::holding_blocks(&snapshot.events, hour);
    let blocks: Vec<Event> = snapshot
        .events
        .iter()
        .filter(|event| is_time_block(event, settings, &assigned))
        .cloned()
        .collect();
    let mut display_events = snapshot.events.clone();
    display_events.extend(
        snapshot
            .events
            .iter()
            .filter_map(calendar::deadline::marker),
    );
    dates
        .iter()
        .map(|&date| {
            let mut events: Vec<&Event> = display_events
                .iter()
                .filter(|event| calendar::event_enabled(&snapshot.calendars, event, settings))
                .filter(|event| !managed_copy(event, settings, &blocks))
                .filter(|event| {
                    !(!calendar::deadline::is_marker(event)
                        && in_loaded_block(event, &blocks, settings))
                })
                .filter(|event| {
                    if schedule {
                        calendar::schedule::occurs_on(event, date)
                    } else if event.all_day {
                        // An all-day item's end is the morning after it.
                        event.start.date_naive() <= date && date < event.end.date_naive()
                    } else {
                        calendar::day_at(event.start, hour) == date
                    }
                })
                .collect();
            events.sort_by_key(|event| (!event.all_day, event.start, event.title.clone()));
            let rows = events
                .into_iter()
                .map(|event| AgendaRow {
                    event: event.clone(),
                    time: if event.all_day {
                        "All day".to_owned()
                    } else {
                        calendar::event_time_label(event)
                    },
                    color: event.display_color(settings),
                })
                .collect();
            (date, rows)
        })
        .collect()
}

/// Whether a Google token is already on disk.
pub fn signed_in() -> bool {
    calendar::oauth::load_tokens().is_some()
}

/// Saves the Google OAuth client entered on the device and returns the config
/// as it now reads.
pub fn save_google_client(client_id: &str, client_secret: &str) -> Result<Config, String> {
    if client_id.trim().is_empty() {
        return Err("Enter the Google OAuth client ID.".to_owned());
    }
    timeslay_core::config::save_google_client(client_id, client_secret)
        .map_err(|error| format!("Could not save the Google client: {error}"))?;
    Ok(load_config())
}

/// Saves which day weeks start on and returns the config as it now reads.
pub fn set_first_day_of_week(first_day: FirstDayOfWeek) -> Result<Config, String> {
    timeslay_core::config::save_first_day_of_week(first_day)
        .map_err(|error| format!("Could not save the first day of the week: {error}"))?;
    Ok(load_config())
}

/// Switches one source on or off and returns the config as it now reads. This
/// starts from the file, so it cannot undo a change made by another process.
pub fn set_source_enabled(
    calendars: &[calendar::Calendar],
    id: &str,
    enabled: bool,
) -> Result<Config, String> {
    let mut config = load_config();
    calendar::set_source_enabled(&mut config.calendar, calendars, id, enabled);
    timeslay_core::config::save_calendar_visibility(&config.calendar)
        .map_err(|error| format!("Could not save the calendar settings: {error}"))?;
    Ok(config)
}

/// Opens the browser sign-in flow and stores the returned token.
///
/// Blocking: this waits for Google's localhost redirect.
pub fn connect_google(settings: &CalendarSettings) -> Result<(), String> {
    calendar::connect(settings.client_id.clone(), settings.client_secret.clone())?;
    Ok(())
}

/// A day's worth of screen-ready data.
#[derive(Clone, Debug, PartialEq)]
pub struct DayModel {
    pub day: NaiveDate,
    pub notice: Option<String>,
    /// Timed events and time blocks, already placed in columns.
    pub timed: Vec<TimedEvent>,
    /// Non-task all-day events.
    pub all_day: Vec<Event>,
    /// Every configured/assigned block, with the tasks inside it.
    pub blocks: Vec<TimeBlock>,
    /// Google Tasks grouped by list, including block-assigned and completed ones.
    pub task_groups: Vec<TaskGroup>,
    /// Tasks are switched off in the calendar settings.
    pub tasks_hidden: bool,
    pub summary: DaySummary,
    /// Minutes from the top of the displayed day window, when the needle
    /// belongs to this day.
    pub now_min: Option<i32>,
    pub hour_start: u32,
}

/// The tasks that want attention, counted the way the desktop's day view does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DaySummary {
    pub overdue: usize,
    pub inbox: usize,
    pub unslotted: usize,
}

impl DaySummary {
    pub fn needs_attention(self) -> bool {
        self.overdue > 0 || self.inbox > 0 || self.unslotted > 0
    }
}

/// One event placed on the timed grid.
#[derive(Clone, Debug, PartialEq)]
pub struct TimedEvent {
    pub event: Event,
    pub title: String,
    pub calendar_name: String,
    pub start_min: i32,
    pub end_min: i32,
    pub column: usize,
    pub columns: usize,
    pub column_span: usize,
    pub time_block: bool,
    /// The color to draw it in, with the calendar and kind settings applied.
    pub color: timeslay_core::Color,
    /// A time block's tasks, in order.
    pub tasks: Vec<AssignedTask>,
}

/// A time block and the tasks assigned to it.
#[derive(Clone, Debug, PartialEq)]
pub struct TimeBlock {
    pub event: Event,
    pub tasks: Vec<AssignedTask>,
    pub capacity_minutes: i64,
    pub planned_minutes: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssignedTask {
    pub event: Event,
    pub status: tb::TaskStatus,
    pub minutes: u32,
    pub list_name: String,
}

/// A trip's gray box on the grid, hanging off the start of the event it leads to.
#[derive(Clone, Debug, PartialEq)]
pub struct TravelBox {
    pub event_id: String,
    pub label: String,
    /// How long the trip takes, or 0 when the desktop has not worked it out.
    pub minutes: u32,
    pub anchor_min: i32,
    pub column: usize,
    pub columns: usize,
    pub column_span: usize,
    pub time_block: bool,
}

/// Tasks from one Google Tasks list.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskGroup {
    pub list_id: String,
    pub list_name: String,
    pub tasks: Vec<TaskRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TaskRow {
    pub event: Event,
    pub status: tb::TaskStatus,
    pub minutes: u32,
    pub block: Option<BlockRef>,
    pub overdue: bool,
    pub inbox: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlockRef {
    pub event_id: String,
    pub title: String,
    pub time: String,
    pub start_min: i32,
}

/// One switch in the calendars sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceRow {
    pub id: String,
    pub name: String,
    pub color: timeslay_core::Color,
    pub enabled: bool,
}

/// The calendars sheet, grouped like the iced app's calendars menu. `sources`
/// comes from `Store::display_sources`.
pub fn source_sections(
    sources: &[calendar::Calendar],
    settings: &CalendarSettings,
) -> Vec<(&'static str, Vec<SourceRow>)> {
    let google = |is_tasks: bool| {
        sources
            .iter()
            .filter(|source| {
                source.id != calendar::sleep::SOURCE_ID
                    && source.is_task_list() == is_tasks
                    && !source.is_app_calendar(settings)
            })
            .map(|source| SourceRow {
                id: source.id.clone(),
                name: source.name.clone(),
                color: source.color,
                enabled: calendar::calendar_enabled(&source.id, settings),
            })
            .collect()
    };
    let time_slay = TimeSlaySource::ALL
        .into_iter()
        .map(|source| SourceRow {
            id: source.id().to_owned(),
            name: source.label().to_owned(),
            color: source.color(settings),
            enabled: calendar::calendar_enabled(source.id(), settings),
        })
        .collect();
    vec![
        ("Google Calendars", google(false)),
        ("Google Task Lists", google(true)),
        ("TimeSlay", time_slay),
    ]
}

pub fn build_model(snapshot: &Snapshot, settings: &CalendarSettings) -> DayModel {
    let day = snapshot.day;
    let hour_start = settings.day_start_hour();
    let offset = settings.day_end_minutes();
    let (view_start, view_end) = view_bounds(day, offset);

    let events = snapshot.events.clone();

    let source_order = source_order(snapshot, settings);
    let priority = |event: &Event| {
        source_order
            .iter()
            .position(|id| id == event.source_id())
            .unwrap_or(usize::MAX)
    };

    // Tasks remember their block by key, including tasks the calendar settings
    // currently hide. Detect those blocks even if the time-block calendar is no
    // longer configured, so the day view can still show what was planned.
    let assigned_keys = tb::holding_blocks(&events, settings.day_start_hour());
    let loaded_blocks: Vec<Event> = events
        .iter()
        .filter(|event| is_time_block(event, settings, &assigned_keys))
        .cloned()
        .collect();

    let blocks = build_blocks(&events, snapshot, settings, &assigned_keys);
    let block_refs: HashMap<String, BlockRef> = blocks
        .iter()
        .map(|block| (tb::block_key(&block.event), block_ref(block, day, offset)))
        .collect();

    let mut display_events = events.clone();
    display_events.extend(
        events
            .iter()
            .filter_map(calendar::deadline::marker)
            .filter(|event| calendar::deadline::on_day(event, day, hour_start)),
    );
    let all_day = all_day_events(
        &display_events,
        &snapshot.calendars,
        settings,
        &loaded_blocks,
        &priority,
    );
    let timed = timed_events(
        &display_events,
        snapshot,
        settings,
        day,
        offset,
        view_start,
        view_end,
        &loaded_blocks,
        &priority,
        &blocks,
    );
    let shown_blocks: Vec<Event> = blocks.iter().map(|block| block.event.clone()).collect();
    let (task_groups, tasks_hidden) =
        task_groups(&events, snapshot, settings, &block_refs, &shown_blocks);

    let summary = day_summary(snapshot, &events, settings);

    let today = calendar::day_at(timeslay_core::clock::now(), hour_start);
    let now_min = (day == today)
        .then(|| minutes_into_day(timeslay_core::clock::now(), day, offset).clamp(0, 24 * 60));

    DayModel {
        day,
        notice: snapshot.notice.clone(),
        timed,
        all_day,
        blocks,
        task_groups,
        tasks_hidden,
        summary,
        now_min,
        hour_start,
    }
}

fn day_summary(snapshot: &Snapshot, events: &[Event], settings: &CalendarSettings) -> DaySummary {
    let now = timeslay_core::clock::now();
    let tasks = || events.iter().filter(|event| event.is_task());
    let unslotted = tasks()
        .filter(|task| tb::unslotted_on(task, snapshot.day, now))
        .count();

    DaySummary {
        overdue: tasks().filter(|task| tb::past_due(task, now)).count(),
        inbox: tasks()
            .filter(|task| untriaged_in(task, settings, &snapshot.calendars))
            .count(),
        unslotted,
    }
}

fn view_bounds(day: NaiveDate, offset: i32) -> (DateTime<Local>, DateTime<Local>) {
    let start = Local
        .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .unwrap_or_else(timeslay_core::clock::now)
        + Duration::minutes(i64::from(offset));
    (start, start + Duration::days(1))
}

fn source_order(snapshot: &Snapshot, settings: &CalendarSettings) -> Vec<String> {
    let mut sources: Vec<String> = snapshot
        .calendars
        .iter()
        .filter(|calendar| !calendar.is_task_list())
        .map(|calendar| calendar.id.clone())
        .collect();
    sources.push(CalendarSettings::GOOGLE_TASKS_ID.to_owned());
    sources.sort_by_key(|id| {
        settings
            .source_order
            .iter()
            .position(|ordered| ordered == id)
            .unwrap_or(usize::MAX)
    });
    sources
}

fn is_time_block(
    event: &Event,
    settings: &CalendarSettings,
    assigned_keys: &HashSet<String>,
) -> bool {
    !event.is_task()
        && !event.all_day
        && !event.is_calendar_task()
        && event.details.sleep.is_none()
        && (settings.app_calendar.as_deref() == Some(event.calendar_id.as_str())
            || assigned_keys.contains(&tb::block_key(event)))
}

/// Mirrors `omit_managed_tasks_event`: app-owned scheduled copies are shown
/// beside their task or inside their block, not as a second calendar chip.
fn managed_copy(event: &Event, settings: &CalendarSettings, blocks: &[Event]) -> bool {
    if calendar::deadline::is_marker(event) {
        return false;
    }
    let Some(managed) = settings.app_calendar.as_deref() else {
        return false;
    };
    event.is_calendar_task()
        && event.calendar_id == managed
        && in_loaded_block(event, blocks, settings)
}

/// Whether `event` is a task in one of the loaded `blocks`.
fn in_loaded_block(event: &Event, blocks: &[Event], settings: &CalendarSettings) -> bool {
    event.is_task() && tb::block_of(event, blocks, settings.day_start_hour()).is_some()
}

fn calendar_name(snapshot: &Snapshot, id: &str) -> String {
    snapshot
        .calendars
        .iter()
        .find(|calendar| calendar.id == id)
        .map(|calendar| calendar.name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            id.strip_prefix(ids::TASK_LIST)
                .filter(|name| !name.is_empty())
                .unwrap_or("Tasks")
                .to_owned()
        })
}

fn all_day_events(
    events: &[Event],
    calendars: &[calendar::Calendar],
    settings: &CalendarSettings,
    block_events: &[Event],
    priority: &impl Fn(&Event) -> usize,
) -> Vec<Event> {
    let mut all_day: Vec<_> = events
        .iter()
        .filter(|event| (!event.is_task() || calendar::deadline::is_marker(event)) && event.all_day)
        .filter(|event| calendar::event_enabled(calendars, event, settings))
        .filter(|event| !managed_copy(event, settings, block_events))
        .cloned()
        .collect();
    all_day.sort_by_key(|event| (priority(event), event.title.clone()));
    all_day
}

#[allow(clippy::too_many_arguments)]
fn timed_events(
    events: &[Event],
    snapshot: &Snapshot,
    settings: &CalendarSettings,
    day: NaiveDate,
    offset: i32,
    view_start: DateTime<Local>,
    view_end: DateTime<Local>,
    block_events: &[Event],
    priority: &impl Fn(&Event) -> usize,
    blocks: &[TimeBlock],
) -> Vec<TimedEvent> {
    let blocks_by_key: HashMap<String, &TimeBlock> = blocks
        .iter()
        .map(|block| (tb::block_key(&block.event), block))
        .collect();
    let shown_blocks: Vec<Event> = blocks.iter().map(|block| block.event.clone()).collect();

    let mut visible: Vec<&Event> = events
        .iter()
        .filter(|event| !event.all_day)
        .filter(|event| event.start < view_end && event.end > view_start)
        .filter(|event| calendar::event_enabled(&snapshot.calendars, event, settings))
        .filter(|event| !managed_copy(event, settings, block_events))
        // A task that already sits inside a visible block is read in that
        // block's popup, not drawn a second time as a chip on the grid.
        .filter(|event| {
            !(!calendar::deadline::is_marker(event)
                && in_loaded_block(event, &shown_blocks, settings))
        })
        .collect();
    visible.sort_by_key(|event| (event.start, std::cmp::Reverse(event.end)));

    let spans: Vec<Span> = visible
        .iter()
        .map(|event| {
            let start = minutes_into_day(event.start, day, offset);
            let end = minutes_into_day(event.end, day, offset);
            let (start_min, end_min) = layout::clamp_day(start, end).unwrap_or_else(|| {
                let start = start.clamp(0, 24 * 60);
                (start, (start + 15).min(24 * 60))
            });
            Span { start_min, end_min }
        })
        .collect();
    let priorities: Vec<_> = visible.iter().map(|event| priority(event)).collect();
    let packed = layout::pack_prioritized(&spans, &priorities);

    visible
        .iter()
        .zip(spans)
        .zip(packed)
        .filter(|((_, span), _)| span.end_min > span.start_min)
        .map(|((event, span), slot)| {
            let key = tb::block_key(event);
            let block = blocks_by_key.get(&key).copied();
            TimedEvent {
                event: (*event).clone(),
                title: if event.details.sleep.is_some() {
                    calendar::sleep::title(event)
                } else if event.is_task() {
                    tb::task_label(event)
                } else {
                    event.title.clone()
                },
                calendar_name: calendar_name(snapshot, &event.calendar_id),
                start_min: span.start_min,
                end_min: span.end_min,
                column: slot.column,
                columns: slot.columns,
                column_span: slot.column_span,
                time_block: block.is_some(),
                color: event.display_color(settings),
                tasks: block.map_or_else(Vec::new, |block| block.tasks.clone()),
            }
        })
        .collect()
}

fn build_blocks(
    events: &[Event],
    snapshot: &Snapshot,
    settings: &CalendarSettings,
    assigned_keys: &HashSet<String>,
) -> Vec<TimeBlock> {
    let mut blocks: Vec<TimeBlock> = events
        .iter()
        .filter(|event| is_time_block(event, settings, assigned_keys))
        .filter(|event| calendar::event_enabled(&snapshot.calendars, event, settings))
        .map(|block| {
            let tasks = tb::tasks_in(block, events, settings.day_start_hour());
            let tasks: Vec<AssignedTask> = tasks
                .into_iter()
                .map(|task| AssignedTask {
                    event: task.clone(),
                    status: tb::task_status(task),
                    minutes: tb::metadata(task).minutes,
                    list_name: calendar_name(snapshot, &task.calendar_id),
                })
                .collect();
            let planned_minutes = tasks.iter().map(|task| task.minutes).sum();
            TimeBlock {
                event: block.clone(),
                tasks,
                capacity_minutes: (block.end - block.start).num_minutes().max(0),
                planned_minutes,
            }
        })
        .collect();
    blocks.sort_by_key(|block| (block.event.start, block.event.title.clone()));
    blocks
}

fn block_ref(block: &TimeBlock, day: NaiveDate, offset: i32) -> BlockRef {
    BlockRef {
        event_id: block.event.id.clone(),
        title: block.event.title.clone(),
        time: calendar::event_time_label(&block.event),
        start_min: minutes_into_day(block.event.start, day, offset),
    }
}

fn task_groups(
    events: &[Event],
    snapshot: &Snapshot,
    settings: &CalendarSettings,
    blocks: &HashMap<String, BlockRef>,
    block_events: &[Event],
) -> (Vec<TaskGroup>, bool) {
    let mut groups: Vec<TaskGroup> = Vec::new();

    for event in events.iter().filter(|event| event.is_task()) {
        // The task rail is the complete view of the day's shown task lists, so
        // it follows each list's toggle rather than the visibility of the
        // calendars a task happens to be scheduled on.
        if !calendar::calendar_enabled(&event.calendar_id, settings) {
            continue;
        }
        let group_index = match groups
            .iter()
            .position(|group| group.list_id == event.calendar_id)
        {
            Some(index) => index,
            None => {
                let list_name = calendar_name(snapshot, &event.calendar_id);
                groups.push(TaskGroup {
                    list_id: event.calendar_id.clone(),
                    list_name,
                    tasks: Vec::new(),
                });
                groups.len() - 1
            }
        };

        let metadata = tb::metadata(event);
        let block = tb::block_of(event, block_events, settings.day_start_hour())
            .and_then(|block| blocks.get(&tb::block_key(block)))
            .cloned();
        groups[group_index].tasks.push(TaskRow {
            event: event.clone(),
            status: tb::task_status(event),
            minutes: metadata.minutes,
            block,
            overdue: tb::past_due(event, timeslay_core::clock::now()),
            inbox: untriaged_in(event, settings, &snapshot.calendars),
        });
    }

    let calendar_position = |id: &str| {
        snapshot
            .calendars
            .iter()
            .position(|calendar| calendar.id == id)
            .unwrap_or(usize::MAX)
    };
    groups.sort_by_key(|group| (calendar_position(&group.list_id), group.list_name.clone()));
    for group in &mut groups {
        group.tasks.sort_by_key(|task| {
            let active = task.status == tb::TaskStatus::Active;
            let block_start = task
                .block
                .as_ref()
                .map_or(i32::MAX, |block| block.start_min);
            (
                !active,
                block_start,
                tb::metadata(&task.event).order,
                task.event.title.clone(),
            )
        });
    }

    (groups, !settings.show_tasks)
}

/// Minutes from the start of the displayed day window.
///
/// With the default midnight boundary this is simply minutes from midnight.
/// With a 3am boundary, 1am belongs to the previous calendar date and lands
/// near the foot of that day's grid.
pub fn minutes_into_day(when: DateTime<Local>, day: NaiveDate, offset: i32) -> i32 {
    let days = (when.date_naive() - day).num_days() as i32;
    days * 24 * 60 + when.hour() as i32 * 60 + when.minute() as i32 - offset
}

/// The hour label at a grid row, wrapping through the configured boundary.
pub fn hour_at(hour_start: u32, row: u32) -> u32 {
    (hour_start + row) % 24
}

pub fn duration_label(minutes: u32) -> String {
    match minutes {
        0 => "no estimate".to_owned(),
        minutes if minutes < 60 => format!("{minutes}m"),
        minutes if minutes % 60 == 0 => format!("{}h", minutes / 60),
        minutes => format!("{}h {}m", minutes / 60, minutes % 60),
    }
}

pub fn capacity_label(minutes: i64) -> String {
    let minutes = minutes.max(0) as u32;
    duration_label(minutes)
}

/// The trips on the grid, with the times the desktop app worked out and synced.
/// Nothing is asked of Mapbox here.
pub fn travel_boxes(
    model: &DayModel,
    travel: &PlaceSettings,
    unit: places::DistanceUnit,
) -> Vec<TravelBox> {
    if !travel.show {
        return Vec::new();
    }
    // The order the desktop walks a day's events in, so both find the same trips.
    let mut timed: Vec<&TimedEvent> = model.timed.iter().collect();
    timed.sort_by_key(|timed| (timed.event.start, timed.event.end));
    let stops: Vec<places::Stop> = timed
        .iter()
        .map(|timed| places::Stop {
            id: timed.event.id.clone(),
            title: timed.event.title.clone(),
            place: places::place_of(&travel.places, &travel.event_places, &timed.event),
        })
        .collect();

    places::hops(&travel.places, travel.home, &stops)
        .into_iter()
        .filter_map(|hop| {
            let timed = timed.iter().find(|timed| timed.event.id == hop.before)?;
            let mode = travel.mode_for(hop.leg.from, hop.leg.to);
            let route = travel.route(hop.leg);
            Some(TravelBox {
                event_id: hop.before,
                label: match route {
                    Some(route) => format!("{mode} · {}", places::route_label(route, unit)),
                    None => format!("{mode} · no time yet"),
                },
                minutes: route.map_or(0, |route| route.seconds.div_ceil(60)),
                anchor_min: timed.start_min,
                column: timed.column,
                columns: timed.columns,
                column_span: timed.column_span,
                time_block: timed.time_block,
            })
        })
        .collect()
}

/// Shows or hides travel and returns the config as it now reads.
pub fn set_travel_shown(show: bool) -> Result<Config, String> {
    timeslay_core::config::save_travel_show(show)
        .map_err(|error| format!("Could not save the travel setting: {error}"))?;
    Ok(load_config())
}

/// What the editors need to know beyond the item being edited.
#[derive(Clone, Debug, PartialEq)]
pub struct EditContext {
    pub settings: CalendarSettings,
    pub projects: ProjectsSettings,
    pub places: PlaceSettings,
    pub calendars: Vec<calendar::Calendar>,
    /// Everything loaded for the day, which a task write lays out around.
    pub events: Vec<Event>,
    /// How clock times are written, as in Settings.
    pub time_format: TimeFormat,
}

impl EditContext {
    pub fn new(config: &Config, snapshot: &Snapshot) -> Self {
        Self {
            settings: config.calendar.clone(),
            projects: config.projects.clone(),
            places: config.travel.clone(),
            calendars: snapshot.calendars.clone(),
            events: snapshot.events.clone(),
            time_format: config.appearance.time_format,
        }
    }
}

/// Where an item comes from, as the editor heading's chip shows it: an
/// app-specific kind in the kind's configured color, or for an event on
/// another calendar, the calendar's name in the event's color.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub name: String,
    pub color: timeslay_core::Color,
}

/// How the editor describes where an item comes from, like the iced details pane.
pub fn source_label(event: &Event, context: &EditContext) -> Source {
    let settings = &context.settings;
    let (name, color) =
        event
            .kind(settings, &context.calendars)
            .chip(event, settings, &context.calendars);
    Source { name, color }
}

/// A task's fields as they are being typed, before each one is saved.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskFields {
    pub title: String,
    pub description: String,
    pub estimate: String,
    pub date: String,
    pub start: String,
    pub end: String,
    pub deadline: String,
    pub deadline_time: String,
}

impl TaskFields {
    pub fn new(task: &Event) -> Self {
        let deadline = calendar::deadline::input_text(task);
        Self {
            title: task.title.clone(),
            description: task.details.description.clone(),
            estimate: tb::metadata(task).minutes.to_string(),
            date: if task.start.date_naive() == tb::UNDATED {
                String::new()
            } else {
                task.start.date_naive().to_string()
            },
            start: if task.all_day {
                "09:00".to_owned()
            } else {
                calendar::clock_label(task.start)
            },
            end: if task.all_day {
                "09:30".to_owned()
            } else {
                calendar::clock_label(task.end)
            },
            deadline: deadline.get(..10).unwrap_or_default().to_owned(),
            deadline_time: deadline.get(11..).unwrap_or("09:00").to_owned(),
        }
    }

    /// The change that saves the due date on its own. A blank date clears it.
    pub fn day_change(&self) -> Result<Change, String> {
        if self.date.is_empty() {
            return Ok(Change::ClearSchedule);
        }
        calendar::deadline::parse_date(self.date.trim()).map(Change::Day)
    }

    /// The change that saves the due date with a time. A task with an estimate
    /// lasts exactly that long, whatever the end field holds, because sync would
    /// put it back to that length anyway.
    pub fn time_change(&self, estimate: u32) -> Result<Change, String> {
        if estimate > 0 {
            let (start, _) = task_edit::schedule_range(&self.date, &self.start, &self.start)?;
            return Ok(Change::Time(
                start,
                start + Duration::minutes(i64::from(estimate)),
            ));
        }
        let (start, end) = task_edit::schedule_range(&self.date, &self.start, &self.end)?;
        if end <= start {
            return Err("The task must end after it starts".to_owned());
        }
        Ok(Change::Time(start, end))
    }

    /// The change that saves the deadline, with its time when `timed`. A blank
    /// date clears it.
    pub fn deadline_change(&self, timed: bool) -> Result<Change, String> {
        let day = self.deadline.trim();
        if day.is_empty() {
            return Ok(Change::Deadline(None));
        }
        let input = if timed {
            format!("{day} {}", self.deadline_time.trim())
        } else {
            day.to_owned()
        };
        calendar::deadline::parse_input(&input).map(|deadline| Change::Deadline(Some(deadline)))
    }

    pub fn estimate_change(&self) -> Result<u32, String> {
        let minutes = self
            .estimate
            .trim()
            .parse::<u32>()
            .map_err(|_| "Enter an estimate in whole minutes".to_owned())?;
        if minutes == 0 {
            return Err("An estimate is required".to_owned());
        }
        if minutes > 10080 {
            return Err("Estimate must be at most 10080 minutes".to_owned());
        }
        Ok(minutes)
    }

    pub fn title_change(&self) -> Result<String, String> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err("A task needs a name".to_owned());
        }
        if title.chars().count() > task_edit::MAX_TITLE_CHARS {
            return Err(format!(
                "A task name can be at most {} characters",
                task_edit::MAX_TITLE_CHARS
            ));
        }
        Ok(title.to_owned())
    }
}

/// Labels already used on the day's tasks, offered so they need no retyping.
pub fn label_suggestions(events: &[Event], current: &[String], query: &str) -> Vec<String> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut seen: Vec<String> = Vec::new();
    for event in events.iter().filter(|event| event.is_task()) {
        for label in tb::metadata(event).labels {
            let known = |existing: &String| existing.eq_ignore_ascii_case(&label);
            if !current.iter().any(known)
                && !seen.iter().any(known)
                && label.to_lowercase().contains(&query)
            {
                seen.push(label);
            }
        }
    }
    seen.truncate(5);
    seen
}

/// The task's labels with `label` added, or `None` when it already has it.
pub fn with_label(task: &Event, label: &str) -> Result<Option<Vec<String>>, String> {
    let label = task_edit::clean_label(label).map_err(str::to_owned)?;
    let mut labels = tb::metadata(task).labels;
    if labels
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&label))
    {
        return Ok(None);
    }
    labels.push(label);
    Ok(Some(labels))
}

/// One line of the Habits sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct HabitRow {
    pub title: String,
    pub schedule: String,
    pub time: String,
    pub color: timeslay_core::Color,
    /// What opens in the editor.
    pub event: Event,
}

/// A habit's master as the editor expects it. It names itself as its series, so
/// opening it loads the full master through the same path as an occurrence.
pub fn habit_row(habit: calendar::api::HabitEvent, settings: &CalendarSettings) -> HabitRow {
    let anchor = habit.start.date_naive();
    let recurrence = timeslay_core::recurrence::Recurrence::from_lines(&habit.recurrence);
    let schedule = if recurrence.is_empty() {
        "Does not repeat".to_owned()
    } else {
        timeslay_core::recurrence::Rule::from_recurrence(anchor, &recurrence).summary(anchor)
    };
    let color = habit
        .color_id
        .as_deref()
        .and_then(calendar::colors::event)
        .unwrap_or_else(|| settings.habit_color());
    HabitRow {
        title: habit.title.clone(),
        schedule,
        time: format!(
            "{} – {}",
            calendar::clock_label(habit.start),
            calendar::clock_label(habit.end)
        ),
        color,
        event: Event {
            id: habit.id.clone(),
            calendar_id: habit.calendar_id,
            title: habit.title,
            start: habit.start,
            end: habit.end,
            all_day: false,
            color,
            completed: false,
            details: calendar::Details {
                habit: true,
                habit_id: habit.habit_id,
                description: habit.notes,
                recurrence: habit.recurrence,
                recurring_event_id: Some(habit.id),
                color_id: habit.color_id,
                ..Default::default()
            },
        },
    }
}

/// Whether a task is still waiting in the Inbox to be triaged.
pub fn untriaged(task: &Event, context: &EditContext) -> bool {
    untriaged_in(task, &context.settings, &context.calendars)
}

pub(crate) fn untriaged_in(
    task: &Event,
    settings: &CalendarSettings,
    calendars: &[calendar::Calendar],
) -> bool {
    let inbox = match &settings.inbox_list {
        Some(chosen) => ids::bare_list(chosen) == ids::bare_list(&task.calendar_id),
        None => calendars.iter().any(|calendar| {
            calendar.id == task.calendar_id
                && calendar.is_task_list()
                && calendar
                    .name
                    .trim()
                    .eq_ignore_ascii_case(calendar::scheduled::DEFAULT_LIST)
        }),
    };
    inbox && task.start.date_naive() == tb::UNDATED && tb::in_backlog(task)
}

/// Open tasks of the day that a block could take on.
pub fn assignable_tasks(events: &[Event]) -> Vec<Event> {
    let mut tasks: Vec<Event> = events
        .iter()
        .filter(|event| event.is_task())
        .filter(|event| tb::task_status(event) == tb::TaskStatus::Active)
        .filter(|event| !tb::is_assigned(event))
        .cloned()
        .collect();
    tasks.sort_by_key(|task| (tb::metadata(task).order, task.title.clone()));
    tasks
}

/// Writes one change to a task and returns the task as it now stands. Its id
/// can differ from the one passed in, because moving lists renames it.
///
/// Blocking: callers run this on a worker thread.
pub fn save_task(task: Event, change: Change, context: &EditContext) -> Result<Event, String> {
    let settings = context.settings.clone();
    // New assignments go to the end of a block.
    let order = context
        .events
        .iter()
        .filter(|event| event.is_task())
        .map(|event| tb::metadata(event).order)
        .max()
        .unwrap_or(0)
        .saturating_add(1_i64 << 32);
    let blocks = block_events(&context.events, &settings);
    let related = context
        .events
        .iter()
        .filter(|event| event.is_task())
        .cloned()
        .collect();
    let mut changed = task_edit::save_and_reflow(
        task,
        settings,
        change,
        order,
        blocks,
        related,
        task_edit::google_access,
    )?;
    // The task itself always comes first.
    Ok(changed.swap_remove(0))
}

/// Every event of the day that holds, or could hold, assigned tasks.
fn block_events(events: &[Event], settings: &CalendarSettings) -> Vec<Event> {
    let assigned = tb::holding_blocks(events, settings.day_start_hour());
    events
        .iter()
        .filter(|event| is_time_block(event, settings, &assigned))
        .cloned()
        .collect()
}

/// Saves an edited event, habit or time block, and lays a block's tasks out
/// again from its new start.
///
/// Blocking: callers run this on a worker thread.
pub fn save_event(
    original: &Event,
    draft: &calendar::EditDraft,
    scope: calendar::SaveScope,
    context: &EditContext,
) -> Result<(), String> {
    if draft.title.trim().is_empty() {
        return Err("Enter a title".to_owned());
    }
    if draft.end <= draft.start {
        return Err("The end time has to be after the start".to_owned());
    }
    draft.recurrence_lines()?;

    let settings = &context.settings;
    // A habit event opened from a habit's page, which is created on Google
    // counting towards that habit.
    if original.id == crate::habits::NEW_HABIT_ID {
        let habit_id = original
            .details
            .habit_id
            .as_deref()
            .ok_or("Open a habit to add an event to it")?;
        calendar::create_habit(draft, original.details.color_id.as_deref(), habit_id)?;
        return Ok(());
    }
    let labelled_block = tb::is_block(original, settings);
    let description = if labelled_block {
        calendar::event_description(&draft.description)
    } else {
        draft.description.clone()
    };
    let color_id = None;
    calendar::save_event_scope(original, draft, scope, &description, color_id)?;

    let key = tb::block_key(original);
    if block_events(&context.events, settings)
        .iter()
        .any(|block| tb::block_key(block) == key)
    {
        let mut moved = original.clone();
        moved.start = draft.start;
        moved.end = draft.end;
        task_edit::reflow(&moved, &context.events, settings, task_edit::google_access)?;
    }
    Ok(())
}

/// Gives a calendar event a place, or takes it away, both on Google and in the
/// config the iced app reads it from.
///
/// Blocking: callers run this on a worker thread.
pub fn save_event_place(event: &Event, place: Option<&Place>) -> Result<(), String> {
    let text = place.map(places::location_text).unwrap_or_default();
    calendar::set_event_location(&event.calendar_id, &event.id, &text)?;
    timeslay_core::config::save_event_place(&event.id, place.map(|place| place.id))
        .map_err(|error| format!("Could not save the location: {error}"))
}

/// Saves a place Mapbox found, so tasks and events can refer to it.
pub fn save_found_place(found: &Found) -> Result<Place, String> {
    let id = timeslay_core::config::save_found_place(found)
        .map_err(|error| format!("Could not save the place: {error}"))?;
    Ok(Place {
        id,
        name: found.name.clone(),
        address: found.address.clone(),
        place_id: found.place_id.clone(),
        lat: found.lat,
        lng: found.lng,
        favorite: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use timeslay_core::calendar::{Calendar, Details};
    use timeslay_core::task_db::TaskData;
    use timeslay_core::timeblocking::Metadata;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 26).unwrap()
    }

    fn at(hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .from_local_datetime(&day().and_hms_opt(hour, minute, 0).unwrap())
            .single()
            .unwrap()
    }

    fn calendar(id: &str, name: &str) -> Calendar {
        Calendar {
            id: id.to_owned(),
            name: name.to_owned(),
            color: calendar::colors::DEFAULT_EVENT,
            primary: false,
            selected_by_google: true,
        }
    }

    fn event(
        id: &str,
        calendar_id: &str,
        title: &str,
        start: DateTime<Local>,
        end: DateTime<Local>,
    ) -> Event {
        Event {
            id: id.to_owned(),
            calendar_id: calendar_id.to_owned(),
            title: title.to_owned(),
            start,
            end,
            all_day: false,
            color: calendar::colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        }
    }

    fn task(
        id: &str,
        title: &str,
        block: Option<&str>,
        order: i64,
        minutes: u32,
        completed: bool,
    ) -> Event {
        let mut event = event(
            &ids::event_id(id),
            "tasks:work",
            title,
            at(0, 0),
            at(0, 0) + Duration::days(1),
        );
        event.all_day = true;
        event.completed = completed;
        event.details.task_data = Some(TaskData {
            id: order,
            metadata: Metadata {
                block: block.map(str::to_owned),
                minutes,
                order,
                ignored: false,
                projects: Vec::new(),
                labels: Vec::new(),
                priority: None,
                location: None,
                ..Metadata::default()
            },
        });
        event
    }

    fn settings() -> CalendarSettings {
        CalendarSettings {
            app_calendar: Some("blocks".into()),
            ..CalendarSettings::default()
        }
    }

    fn snapshot(events: Vec<Event>) -> Snapshot {
        Snapshot {
            calendars: vec![calendar("blocks", "Focus"), calendar("tasks:work", "Work")],
            events,
            day: day(),
            fetched_at: at(10, 0),
            notice: None,
        }
    }

    #[test]
    fn a_search_narrows_what_is_drawn_and_keeps_a_block_for_its_matching_task() {
        let block = event("b1", "blocks", "Deep work", at(9, 0), at(11, 0));
        let inside = task("a", "Write report", Some("blocks/b1"), 1, 45, false);
        let lunch = event("l", "blocks", "Lunch", at(12, 0), at(13, 0));
        let snapshot = snapshot(vec![block, inside, lunch]);
        let search = Search {
            query: "report".to_owned(),
            ..Search::default()
        };

        let filtered = filter_snapshot(&snapshot, &search, &settings());

        let titles: Vec<&str> = filtered.events.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["Deep work", "Write report"]);
        // The search being off changes nothing.
        let everything = filter_snapshot(&snapshot, &Search::default(), &settings());
        assert_eq!(everything.events.len(), 3);
    }

    #[test]
    fn quick_add_takes_the_keywords_out_of_the_title() {
        let today = day();
        let task = quick_task(
            "Pay rent tomorrow p1 @bills ~30m",
            today,
            &ProjectsSettings::default(),
        );
        assert_eq!(task.due, today.succ_opt());
        assert_eq!(task.metadata.priority, Some(tb::Priority::P1));
        assert_eq!(task.metadata.minutes, 30);
        assert_eq!(task.metadata.labels, ["bills"]);
        assert_eq!(task.title.trim(), "Pay rent");
    }

    #[test]
    fn a_new_event_needs_a_title_and_to_end_after_it_starts() {
        let new = |title: &str, start: &str, end: &str| NewEvent {
            title: title.to_owned(),
            description: String::new(),
            date: "2026-11-04".to_owned(),
            end_date: String::new(),
            start: start.to_owned(),
            end: end.to_owned(),
            all_day: false,
        };
        assert!(new(" ", "09:00", "10:00").range().is_err());
        assert!(new("Call", "10:00", "10:00").range().is_err());
        let (start, end) = new("Call", "09:00", "10:30").range().unwrap();
        assert_eq!((end - start).num_minutes(), 90);
        // An end before the start runs past midnight, as it does for tasks.
        let (start, end) = new("Late", "23:00", "01:00").range().unwrap();
        assert_eq!((end - start).num_minutes(), 120);
    }

    #[test]
    fn a_new_event_can_cover_whole_days_or_run_over_several() {
        let new = |end_date: &str, all_day: bool| NewEvent {
            title: "Trip".to_owned(),
            description: String::new(),
            date: "2026-11-04".to_owned(),
            end_date: end_date.to_owned(),
            start: "09:00".to_owned(),
            end: "17:00".to_owned(),
            all_day,
        };
        let day = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        // One day: Google's end is the following midnight.
        let (start, end) = new("", true).range().unwrap();
        assert_eq!((start.date_naive(), end.date_naive()), (day(4), day(5)));
        assert_eq!((start.hour(), end.hour()), (0, 0));
        // The end date is the last day covered.
        let (start, end) = new("2026-11-06", true).range().unwrap();
        assert_eq!((start.date_naive(), end.date_naive()), (day(4), day(7)));
        // A timed event can end on a later day.
        let (start, end) = new("2026-11-06", false).range().unwrap();
        assert_eq!(
            (start.hour(), end.date_naive(), end.hour()),
            (9, day(6), 17)
        );
        assert!(new("2026-11-03", true).range().is_err());
    }

    #[test]
    fn the_agenda_lists_each_day_and_leaves_block_tasks_to_their_block() {
        let block = event("b1", "blocks", "Deep work", at(9, 0), at(11, 0));
        let inside = task("a", "Write", Some("blocks/b1"), 1, 45, false);
        let loose = task("b", "Review", None, 2, 20, false);
        let snapshot = snapshot(vec![block, inside, loose]);
        let next = day().succ_opt().unwrap();

        let days = agenda(&snapshot, &[day(), next], &settings());

        assert_eq!(days.len(), 2);
        let titles: Vec<&str> = days[0]
            .1
            .iter()
            .map(|row| row.event.title.as_str())
            .collect();
        // All-day items come first.
        assert_eq!(titles, ["Review", "Deep work"]);
        assert_eq!(days[0].1[0].time, "All day");
        assert!(days[1].1.is_empty());
    }

    #[test]
    fn a_task_row_knows_when_it_is_overdue() {
        let mut late = task("d", "Late", None, 4, 30, false);
        late.start = at(0, 0) - Duration::days(400);
        late.end = late.start + Duration::days(1);
        late.details.description = "".into();
        late.details
            .task_data
            .get_or_insert_with(|| timeslay_core::task_db::TaskData {
                id: 100,
                metadata: Default::default(),
            })
            .metadata
            .deadline = Some(
            timeslay_core::calendar::deadline::parse_input(&format!("{}", late.start.date_naive()))
                .unwrap(),
        );
        let model = build_model(&snapshot(vec![late]), &settings());
        assert!(model.task_groups[0].tasks[0].overdue);
        assert!(!model.task_groups[0].tasks[0].inbox);
    }

    #[test]
    fn every_theme_survives_being_saved_and_anything_else_follows_the_system() {
        for theme in Theme::ALL {
            assert_eq!(Theme::parse(&format!("{}\n", theme.name())), theme);
        }
        assert_eq!(Theme::parse("sakura"), Theme::System);
    }

    #[test]
    fn the_summary_counts_tasks_past_due_and_without_a_slot() {
        let block = event("b1", "blocks", "Deep work", at(9, 0), at(11, 0));
        let slotted = task("a", "Write", Some("blocks/b1"), 1, 45, false);
        let unslotted = task("b", "Review", None, 2, 30, false);
        let finished = task("c", "Filed", None, 3, 30, true);
        let mut late = task("d", "Late", None, 4, 30, false);
        late.start = at(0, 0) - Duration::days(400);
        late.end = late.start + Duration::days(1);
        late.details.description = "".into();
        late.details
            .task_data
            .get_or_insert_with(|| timeslay_core::task_db::TaskData {
                id: 100,
                metadata: Default::default(),
            })
            .metadata
            .deadline = Some(
            timeslay_core::calendar::deadline::parse_input(&format!("{}", late.start.date_naive()))
                .unwrap(),
        );
        let snapshot = snapshot(vec![block, slotted, unslotted, finished, late]);

        let summary = build_model(&snapshot, &settings()).summary;

        // Only the explicit deadline is overdue; planning dates are independent.
        assert_eq!(summary.overdue, 1);
        assert_eq!(summary.inbox, 0);
        // The date-only open task still needs a slot on the fixture day.
        assert_eq!(summary.unslotted, 1);
        assert!(summary.needs_attention());
    }

    #[test]
    fn the_summary_separates_today_unslotted_tasks_from_overdue_tasks() {
        let today = timeslay_core::clock::now().date_naive();
        let mut open = task("open", "Open", None, 1, 30, false);
        open.start = calendar::at_clock(today, 0, 0);
        open.end = open.start + Duration::days(1);
        let mut late = open.clone();
        late.id = "task:late".into();
        late.details.description = "".into();
        late.details
            .task_data
            .get_or_insert_with(|| timeslay_core::task_db::TaskData {
                id: 100,
                metadata: Default::default(),
            })
            .metadata
            .deadline = Some(
            timeslay_core::calendar::deadline::parse_input(&format!(
                "{} 00:00",
                today.pred_opt().unwrap()
            ))
            .unwrap(),
        );
        let mut finished = open.clone();
        finished.id = "task:finished".into();
        finished.completed = true;
        let mut snapshot = snapshot(vec![open, late, finished]);
        snapshot.day = today;

        let summary = day_summary(&snapshot, &snapshot.events, &settings());

        assert_eq!(summary.unslotted, 1);
        assert_eq!(summary.overdue, 1);
    }

    #[test]
    fn an_event_is_drawn_in_its_calendars_custom_color() {
        let custom = timeslay_core::hex::parse("#123456").unwrap();
        let mut settings = settings();
        settings
            .calendar_colors
            .insert("cal".to_owned(), custom.into());
        let snapshot = snapshot(vec![event("e", "cal", "Lunch", at(12, 0), at(13, 0))]);

        assert_eq!(build_model(&snapshot, &settings).timed[0].color, custom);
    }

    #[test]
    fn a_block_lists_every_assigned_task_in_planned_order() {
        let block = event("b1", "blocks", "Deep work", at(9, 0), at(11, 0));
        let first = task("a", "Write", Some("blocks/b1"), 1, 45, false);
        let second = task("b", "Review", Some("blocks/b1"), 2, 70, true);
        let loose = task("c", "Inbox zero", None, 3, 20, false);
        let snapshot = snapshot(vec![block, first, second, loose]);
        let model = build_model(&snapshot, &settings());

        assert_eq!(model.blocks.len(), 1);
        let block = &model.blocks[0];
        assert_eq!(block.tasks.len(), 2);
        assert_eq!(
            block
                .tasks
                .iter()
                .map(|task| task.event.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Write", "Review"]
        );
        assert_eq!(block.planned_minutes, 115);
        assert_eq!(block.capacity_minutes, 120);
        assert_eq!(block.tasks[0].list_name, "Work");
        assert!(!model.tasks_hidden);
    }

    #[test]
    fn a_task_repeating_with_a_block_sits_in_that_days_occurrence() {
        let mut block = event("b_1", "blocks", "Deep work", at(9, 0), at(11, 0));
        block.details.recurring_event_id = Some("b".into());
        // Moved from the day before: its tasks of that day come with it.
        block.details.original_start = Some(at(9, 0) - Duration::days(1));
        let repeating = |id: &str, original| {
            let mut task = event(id, "blocks", "Inbox zero", at(9, 0), at(9, 30));
            task.details.recurring_event_id = Some("t".into());
            task.details.original_start = Some(original);
            task.details.task_data = Some(TaskData {
                id: 100,
                metadata: Metadata {
                    block_series: Some("blocks/b".into()),
                    // Google copies the first day's key to every occurrence.
                    block: Some("blocks/elsewhere".into()),
                    minutes: 30,
                    ..Metadata::default()
                },
            });
            task
        };
        let yesterdays = repeating("t_1", at(9, 0) - Duration::days(1));
        let todays = repeating("t_2", at(9, 0));
        let snapshot = snapshot(vec![block, yesterdays, todays.clone()]);
        let model = build_model(&snapshot, &settings());
        let ids: Vec<_> = model.blocks[0]
            .tasks
            .iter()
            .map(|task| task.event.id.as_str())
            .collect();
        assert_eq!(ids, ["t_1"]);
        assert!(!model.timed.iter().any(|timed| timed.event.id == "t_1"));
        // Neither is offered to a block again.
        assert!(assignable_tasks(&snapshot.events).is_empty());
        assert!(tb::is_follower(&todays));
    }

    #[test]
    fn every_task_is_listed_but_block_tasks_are_not_drawn_twice() {
        let block = event("b1", "blocks", "Deep work", at(9, 0), at(11, 0));
        let assigned = task("a", "Write", Some("blocks/b1"), 1, 45, false);
        let standalone = task("b", "Inbox zero", None, 2, 20, false);
        let snapshot = snapshot(vec![block, assigned, standalone]);
        let model = build_model(&snapshot, &settings());

        let titles: Vec<_> = model
            .task_groups
            .iter()
            .flat_map(|group| group.tasks.iter().map(|task| task.event.title.as_str()))
            .collect();
        assert!(titles.contains(&"Write"));
        assert!(titles.contains(&"Inbox zero"));

        let grid_titles: Vec<_> = model
            .timed
            .iter()
            .map(|event| event.title.as_str())
            .collect();
        assert!(grid_titles.contains(&"Deep work"));
        assert!(!grid_titles.contains(&"Write"));
    }

    #[test]
    fn all_day_events_and_tasks_stay_out_of_the_timed_grid() {
        let mut all_day = event(
            "offsite",
            "work",
            "Offsite",
            at(0, 0),
            at(0, 0) + Duration::days(1),
        );
        all_day.all_day = true;
        let task = task("a", "Write", None, 1, 45, false);
        let snapshot = Snapshot {
            calendars: vec![
                calendar("work", "Work"),
                calendar("tasks:work", "Work tasks"),
            ],
            events: vec![all_day, task],
            day: day(),
            fetched_at: at(10, 0),
            notice: None,
        };
        let model = build_model(&snapshot, &settings());

        assert_eq!(model.all_day.len(), 1);
        assert_eq!(model.all_day[0].title, "Offsite");
        assert!(model.timed.is_empty());
        assert_eq!(model.task_groups[0].tasks[0].event.title, "Write");
    }

    #[test]
    fn hidden_calendars_are_left_out_but_assigned_tasks_still_reach_the_block() {
        let block = event("b1", "blocks", "Deep work", at(9, 0), at(11, 0));
        let assigned = task("a", "Write", Some("blocks/b1"), 1, 45, false);
        let snapshot = snapshot(vec![block, assigned]);
        let mut settings = settings();
        settings.disabled.push("blocks".into());
        let model = build_model(&snapshot, &settings);

        assert!(model.blocks.is_empty());
        assert!(model.timed.is_empty());
        // Tasks remain because the Google Tasks source itself is still on.
        assert_eq!(model.task_groups[0].tasks.len(), 1);
        assert!(model.task_groups[0].tasks[0].block.is_none());
    }

    #[test]
    fn a_hidden_block_does_not_swallow_its_timed_task() {
        let block = event("b1", "blocks", "Deep work", at(9, 0), at(11, 0));
        let mut assigned = task("a", "Write", Some("blocks/b1"), 1, 45, false);
        assigned.all_day = false;
        assigned.start = at(9, 0);
        assigned.end = at(9, 45);
        let snapshot = snapshot(vec![block, assigned]);
        let mut settings = settings();
        settings.disabled.push("blocks".into());
        let model = build_model(&snapshot, &settings);

        assert!(model.blocks.is_empty());
        assert_eq!(model.timed.len(), 1);
        assert_eq!(model.timed[0].event.title, "Write");
    }

    #[test]
    fn hiding_tasks_keeps_the_tasks_inside_a_block_popup() {
        let block = event("b1", "blocks", "Deep work", at(9, 0), at(11, 0));
        let assigned = task("a", "Write", Some("blocks/b1"), 1, 45, false);
        let snapshot = snapshot(vec![block, assigned]);
        let mut settings = settings();
        settings.show_tasks = false;
        let model = build_model(&snapshot, &settings);

        assert!(model.tasks_hidden);
        assert!(model.task_groups.is_empty());
        assert_eq!(model.blocks[0].tasks.len(), 1);
        assert_eq!(model.blocks[0].tasks[0].event.title, "Write");
    }

    #[test]
    fn hiding_a_task_list_removes_only_that_list_from_the_rail() {
        let block = event("b1", "blocks", "Deep work", at(9, 0), at(11, 0));
        let assigned = task("a", "Write", Some("blocks/b1"), 1, 45, false);
        let list = assigned.calendar_id.clone();
        let mut other = task("b", "Call", None, 2, 10, false);
        other.calendar_id = "tasks:home".into();
        let snapshot = snapshot(vec![block, assigned, other]);
        let mut settings = settings();
        settings.disabled.push(list);
        let model = build_model(&snapshot, &settings);

        assert!(!model.tasks_hidden);
        assert_eq!(model.task_groups.len(), 1);
        assert_eq!(model.task_groups[0].list_id, "tasks:home");
        assert_eq!(model.blocks[0].tasks.len(), 1);
    }

    #[test]
    fn hiding_a_time_slay_kind_leaves_the_calendar_alone() {
        let block = event("b1", "blocks", "Deep work", at(9, 0), at(11, 0));
        let meeting = event("m1", "work", "Standup", at(12, 0), at(12, 30));
        let snapshot = snapshot(vec![block, meeting]);
        let mut settings = settings();
        settings
            .disabled
            .push(TimeSlaySource::TimeBlocks.id().into());
        let model = build_model(&snapshot, &settings);

        assert!(model.blocks.is_empty());
        assert_eq!(model.timed.len(), 1);
        assert_eq!(model.timed[0].event.title, "Standup");
    }

    #[test]
    fn source_sections_match_the_calendars_menu() {
        let sources = vec![
            calendar("work", "Work"),
            calendar("blocks", "Focus"),
            calendar("tasks:home", "Home"),
            calendar(calendar::sleep::SOURCE_ID, "Sleep"),
        ];
        let mut settings = settings();
        settings.disabled.push("work".into());
        settings.disabled.push(TimeSlaySource::Habits.id().into());
        let sections = source_sections(&sources, &settings);

        let names = |rows: &[SourceRow]| -> Vec<String> {
            rows.iter().map(|row| row.name.clone()).collect()
        };
        assert_eq!(sections[0].0, "Google Calendars");
        assert_eq!(names(&sections[0].1), ["Work"]);
        assert!(!sections[0].1[0].enabled);
        assert_eq!(sections[1].0, "Google Task Lists");
        assert_eq!(names(&sections[1].1), ["Home"]);
        assert!(sections[1].1[0].enabled);
        assert_eq!(sections[2].0, "TimeSlay");
        assert_eq!(
            names(&sections[2].1),
            ["Tasks", "Time Blocks", "Habits", "Sleep"]
        );
        assert!(!sections[2].1[2].enabled);
        assert!(sections[2].1[3].enabled);
    }

    #[test]
    fn a_plain_event_on_the_app_calendar_is_drawn_as_a_time_block() {
        let block = event("b1", "scheduled", "Write", at(9, 0), at(9, 45));
        let snapshot = Snapshot {
            calendars: vec![
                calendar("scheduled", "TimeSlay"),
                calendar("tasks:work", "Work"),
            ],
            events: vec![block],
            day: day(),
            fetched_at: at(10, 0),
            notice: None,
        };
        let settings = CalendarSettings {
            app_calendar: Some("scheduled".into()),
            ..CalendarSettings::default()
        };
        let model = build_model(&snapshot, &settings);
        assert_eq!(model.blocks.len(), 1);
        assert_eq!(model.blocks[0].event.title, "Write");
    }

    #[test]
    fn minutes_follow_the_end_of_day_boundary() {
        assert_eq!(minutes_into_day(at(9, 30), day(), 0), 570);
        assert_eq!(minutes_into_day(at(9, 30), day(), 3 * 60), 390);
        // 1am on the next calendar date is still late night on this day.
        let late = at(1, 0) + Duration::days(1);
        assert_eq!(minutes_into_day(late, day(), 3 * 60), 22 * 60);
        assert_eq!(hour_at(3, 0), 3);
        assert_eq!(hour_at(3, 23), 2);
    }

    #[test]
    fn overlapping_events_share_columns() {
        let a = event("a", "work", "A", at(9, 0), at(10, 0));
        let b = event("b", "work", "B", at(9, 30), at(10, 30));
        let snapshot = Snapshot {
            calendars: vec![calendar("work", "Work")],
            events: vec![a, b],
            day: day(),
            fetched_at: at(10, 0),
            notice: None,
        };
        let model = build_model(&snapshot, &settings());
        assert_eq!(model.timed.len(), 2);
        assert_eq!(model.timed[0].columns, 2);
        assert_eq!(model.timed[1].columns, 2);
        assert_ne!(model.timed[0].column, model.timed[1].column);
    }

    #[test]
    fn task_fields_start_from_the_task_and_turn_into_changes() {
        let mut timed = task("a", "Write", None, 1, 45, false);
        timed.all_day = false;
        timed.start = at(9, 0);
        timed.end = at(9, 45);
        timed.details.description = "Notes".into();
        timed
            .details
            .task_data
            .get_or_insert_with(|| timeslay_core::task_db::TaskData {
                id: 100,
                metadata: Default::default(),
            })
            .metadata
            .deadline =
            Some(timeslay_core::calendar::deadline::parse_input("2026-11-05 17:30").unwrap());
        let mut fields = TaskFields::new(&timed);
        assert_eq!(fields.description, "Notes");
        assert_eq!(fields.estimate, "45");
        assert_eq!(fields.date, "2026-10-26");
        assert_eq!(
            (fields.start.as_str(), fields.end.as_str()),
            ("09:00", "09:45")
        );
        assert_eq!(fields.deadline, "2026-11-05");
        assert_eq!(fields.deadline_time, "17:30");

        assert!(matches!(fields.day_change(), Ok(Change::Day(day)) if day == day_of(&timed)));
        assert!(matches!(
            fields.time_change(45),
            Ok(Change::Time(start, end)) if start == at(9, 0) && end == at(9, 45)
        ));
        // The estimate decides the length; the end field only counts without one.
        fields.end = "11:00".into();
        assert!(matches!(
            fields.time_change(45),
            Ok(Change::Time(_, end)) if end == at(9, 45)
        ));
        assert!(matches!(
            fields.time_change(0),
            Ok(Change::Time(_, end)) if end == at(11, 0)
        ));
        // A day-only save drops the time; a timed one keeps it.
        let Ok(Change::Deadline(Some(deadline))) = fields.deadline_change(false) else {
            panic!("expected a deadline");
        };
        assert_eq!(deadline.value(), "2026-11-05");
        let Ok(Change::Deadline(Some(deadline))) = fields.deadline_change(true) else {
            panic!("expected a deadline");
        };
        assert_eq!(deadline.value(), "2026-11-05 17:30");

        fields.date.clear();
        fields.deadline.clear();
        assert!(matches!(fields.day_change(), Ok(Change::ClearSchedule)));
        assert!(matches!(
            fields.deadline_change(true),
            Ok(Change::Deadline(None))
        ));
    }

    fn day_of(event: &Event) -> NaiveDate {
        event.start.date_naive()
    }

    #[test]
    fn task_fields_refuse_what_google_would_not_keep() {
        let mut fields = TaskFields::new(&task("a", "Write", None, 1, 45, false));
        fields.estimate = "soon".into();
        assert!(fields.estimate_change().is_err());
        fields.estimate = "10081".into();
        assert!(fields.estimate_change().is_err());
        fields.estimate = " 90 ".into();
        assert_eq!(fields.estimate_change(), Ok(90));
        fields.title = "   ".into();
        assert!(fields.title_change().is_err());
        fields.title = " Write ".into();
        assert_eq!(fields.title_change().unwrap(), "Write");
    }

    #[test]
    fn labels_are_cleaned_deduplicated_and_suggested_from_other_tasks() {
        let mut first = task("a", "Write", None, 1, 45, false);
        first.details.task_data.as_mut().unwrap().metadata.labels = vec!["Deep work".into()];
        let mut second = task("b", "Review", None, 2, 10, false);
        second.details.task_data.as_mut().unwrap().metadata.labels =
            vec!["deep WORK".into(), "Urgent".into()];

        assert_eq!(with_label(&first, " deep   work "), Ok(None));
        assert_eq!(
            with_label(&first, "  new  label "),
            Ok(Some(vec!["Deep work".to_owned(), "new label".to_owned()]))
        );
        assert!(with_label(&first, "   ").is_err());

        let events = [first, second];
        assert_eq!(
            label_suggestions(&events, &[], "deep"),
            vec!["Deep work".to_owned()]
        );
        assert!(label_suggestions(&events, &["DEEP WORK".to_owned()], "deep").is_empty());
        assert!(label_suggestions(&events, &[], "  ").is_empty());
    }

    #[test]
    fn only_open_unassigned_tasks_can_join_a_block() {
        let tasks = assignable_tasks(&[
            event("m1", "work", "Standup", at(9, 0), at(10, 0)),
            task("late", "Second", None, 2, 10, false),
            task("early", "First", None, 1, 10, false),
            task("done", "Done", None, 3, 10, true),
            task("taken", "Taken", Some("blocks/b1"), 4, 10, false),
        ]);
        let titles: Vec<_> = tasks.iter().map(|task| task.title.as_str()).collect();
        assert_eq!(titles, ["First", "Second"]);
    }

    #[test]
    fn untriaged_follows_the_inbox_list_and_the_missing_date() {
        let mut waiting = task("a", "Write", None, 1, 45, false);
        waiting.calendar_id = "tasks:inbox".into();
        waiting.start = calendar::at_clock(tb::UNDATED, 0, 0);
        waiting.end = waiting.start + Duration::days(1);
        let mut context = EditContext {
            settings: settings(),
            projects: ProjectsSettings::default(),
            places: PlaceSettings::default(),
            calendars: vec![
                calendar("tasks:inbox", "Inbox"),
                calendar("tasks:work", "Work"),
            ],
            events: Vec::new(),
            time_format: TimeFormat::TwelveHour,
        };
        assert!(untriaged(&waiting, &context));

        let mut dated = waiting.clone();
        dated.start = at(0, 0);
        assert!(!untriaged(&dated, &context));

        context.settings.inbox_list = Some("tasks:work".into());
        assert!(!untriaged(&waiting, &context));
    }

    #[test]
    fn the_source_label_names_the_kind() {
        let context = EditContext {
            settings: settings(),
            projects: ProjectsSettings::default(),
            places: PlaceSettings::default(),
            calendars: vec![calendar("blocks", "Focus"), calendar("work", "Work")],
            events: Vec::new(),
            time_format: TimeFormat::TwelveHour,
        };
        let block = event("b1", "blocks", "Deep work", at(9, 0), at(11, 0));
        let kind = |event: &Event| source_label(event, &context).name;
        assert_eq!(kind(&block), "Time Block");
        assert_eq!(
            source_label(&block, &context),
            Source {
                name: "Time Block".into(),
                color: context.settings.time_block_color()
            }
        );
        let meeting = event("m1", "work", "Standup", at(12, 0), at(12, 30));
        assert_eq!(
            source_label(&meeting, &context),
            Source {
                name: "Event".into(),
                color: meeting.display_color(&context.settings)
            }
        );
        let mut habit = meeting.clone();
        habit.details.habit = true;
        assert_eq!(kind(&habit), "Habit");
        let write = task("a", "Write", None, 1, 45, false);
        assert_eq!(kind(&write), "Task");
    }

    #[test]
    fn a_habit_opens_as_its_own_series_with_a_readable_schedule() {
        let habit = calendar::api::HabitEvent {
            id: "h1".into(),
            calendar_id: "blocks".into(),
            title: "Stretch".into(),
            notes: "Ten minutes".into(),
            start: at(7, 0),
            end: at(7, 15),
            recurrence: vec!["RRULE:FREQ=DAILY".into()],
            color_id: None,
            habit_id: None,
        };
        let row = habit_row(habit, &settings());
        assert_eq!(row.title, "Stretch");
        assert_eq!(row.time, "07:00 – 07:15");
        assert_ne!(row.schedule, "Does not repeat");
        assert!(row.event.details.habit);
        assert_eq!(row.event.details.recurring_event_id.as_deref(), Some("h1"));
        assert_eq!(row.event.details.description, "Ten minutes");
        assert_eq!(row.color, settings().habit_color());
    }

    fn travel_settings() -> PlaceSettings {
        let place = |id: u64, name: &str| Place {
            id,
            name: name.to_owned(),
            address: String::new(),
            place_id: String::new(),
            lat: 0.0,
            lng: 0.0,
            favorite: false,
        };
        PlaceSettings {
            home: Some(1),
            places: vec![place(1, "Home"), place(2, "Campus"), place(3, "Gym")],
            default_mode: places::TravelMode::Bike,
            show: true,
            routes: vec![
                places::SavedRoute {
                    from: 1,
                    to: 2,
                    mode: places::TravelMode::Bike,
                    seconds: 1190,
                    meters: 5300,
                },
                places::SavedRoute {
                    from: 2,
                    to: 3,
                    mode: places::TravelMode::Car,
                    seconds: 300,
                    meters: 2000,
                },
            ],
            ..PlaceSettings::default()
        }
    }

    fn located(id: &str, place: &str, hour: u32) -> Event {
        let mut event = event(id, "work", id, at(hour, 0), at(hour + 1, 0));
        event.details.location = place.to_owned();
        event
    }

    #[test]
    fn a_trip_shows_the_synced_time_above_the_event_it_leads_to() {
        let snapshot = snapshot(vec![located("lecture", "Campus", 9)]);
        let model = build_model(&snapshot, &settings());
        let boxes = travel_boxes(&model, &travel_settings(), places::DistanceUnit::Kilometers);

        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].event_id, "lecture");
        assert_eq!(boxes[0].label, "Bike · 20 min · 5.3 km");
        assert_eq!(boxes[0].minutes, 20);
        assert_eq!(boxes[0].anchor_min, 9 * 60);
        let miles = travel_boxes(&model, &travel_settings(), places::DistanceUnit::Miles);
        assert_eq!(miles[0].label, "Bike · 20 min · 3.3 mi");
        assert_eq!(miles[0].minutes, boxes[0].minutes);
        assert_eq!(miles[0].anchor_min, boxes[0].anchor_min);
    }

    #[test]
    fn a_trip_with_no_synced_time_for_its_method_says_so() {
        // Only the car time from Campus to the gym is known, and this goes by bike.
        let snapshot = snapshot(vec![
            located("lecture", "Campus", 9),
            located("gym", "Gym", 11),
        ]);
        let model = build_model(&snapshot, &settings());
        let boxes = travel_boxes(&model, &travel_settings(), places::DistanceUnit::Kilometers);

        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[1].event_id, "gym");
        assert_eq!(boxes[1].label, "Bike · no time yet");
        assert_eq!(boxes[1].minutes, 0);
    }

    #[test]
    fn travel_stays_off_until_it_is_switched_on() {
        let snapshot = snapshot(vec![located("lecture", "Campus", 9)]);
        let model = build_model(&snapshot, &settings());
        let travel = PlaceSettings {
            show: false,
            ..travel_settings()
        };
        assert!(travel_boxes(&model, &travel, places::DistanceUnit::Kilometers).is_empty());
    }

    #[test]
    fn a_blocks_assigned_tasks_are_found_without_a_configured_block_calendar() {
        let block = event("b1", "blocks", "Deep work", at(9, 0), at(11, 0));
        let assigned = task("a", "Write", Some("blocks/b1"), 1, 45, false);
        let snapshot = snapshot(vec![block, assigned]);
        let model = build_model(&snapshot, &CalendarSettings::default());

        assert_eq!(model.blocks.len(), 1);
        assert_eq!(model.blocks[0].tasks.len(), 1);
        assert!(model.timed[0].time_block);
    }
}

#[cfg(test)]
mod deadline_display_tests {
    use super::*;
    #[test]
    fn calendar_keeps_the_scheduled_task_and_adds_its_deadline() {
        let mut snapshot = calendar::sample_snapshot();
        let mut task = snapshot
            .events
            .into_iter()
            .find(|event| event.id == "standup")
            .unwrap();
        task.details.task_data = Some(timeslay_core::task_db::TaskData {
            id: 73,
            metadata: Default::default(),
        });
        snapshot.day = task.start.date_naive();
        let deadline_day = snapshot.day.succ_opt().unwrap();
        task.details.task_data.as_mut().unwrap().metadata.deadline =
            Some(calendar::deadline::parse_input(&format!("{deadline_day} 17:00")).unwrap());
        let id = task.id.clone();
        let start = task.start;
        snapshot.events = vec![task.clone()];
        let settings = CalendarSettings::default();
        let scheduled = build_model(&snapshot, &settings);
        assert!(
            scheduled
                .timed
                .iter()
                .any(|entry| entry.event.id == id && entry.event.start == start)
        );
        assert!(
            !scheduled
                .timed
                .iter()
                .any(|entry| calendar::deadline::is_marker(&entry.event))
        );
        snapshot.day = deadline_day;
        let deadline = build_model(&snapshot, &settings);
        assert_eq!(
            deadline
                .timed
                .iter()
                .filter(|entry| calendar::deadline::is_marker(&entry.event))
                .count(),
            1
        );
        snapshot.events[0]
            .details
            .task_data
            .as_mut()
            .unwrap()
            .metadata
            .deadline
            .as_mut()
            .unwrap()
            .time = None;
        let dated = build_model(&snapshot, &settings);
        assert!(dated.all_day.iter().any(calendar::deadline::is_marker));
        let rows = schedule_agenda(&snapshot, &[deadline_day], &settings);
        assert!(
            rows[0]
                .1
                .iter()
                .any(|row| calendar::deadline::is_marker(&row.event))
        );
        assert_eq!(snapshot.events[0].start, start);
    }
}
