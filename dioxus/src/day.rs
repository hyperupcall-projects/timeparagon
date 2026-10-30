//! The day view's data model.
//!
//! Sync, OAuth, Google Tasks metadata and time-block assignment all come from
//! the existing `timeslay` library, so this crate only has to arrange that
//! data for the screen: which events are timed, which are all-day, which task
//! belongs to which block, and how overlaps share a column.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Timelike};
use timeslay_core::calendar::layout::{self, Span};
use timeslay_core::calendar::{self, Event, Snapshot};
use timeslay_core::config::{CalendarSettings, Config};
use timeslay_core::timeblocking as tb;

/// Reads the same configuration the iced app uses.
pub fn load_config() -> Config {
    Config::load()
}

/// Which colour scheme the view uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// Follow the operating system's light/dark setting.
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn next(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Self::System => "◐",
            Self::Light => "☀",
            Self::Dark => "☾",
        }
    }

    fn parse(text: &str) -> Self {
        match text.trim() {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    /// The preference lives beside `config.toml` but in its own file: it is a
    /// per-device choice, so it must not travel with the synced config.
    fn path() -> Option<std::path::PathBuf> {
        Some(
            timeslay_core::config::project_dirs()?
                .config_dir()
                .join("dioxus-theme"),
        )
    }

    /// The saved choice, or `System` when nothing has been saved.
    pub fn load() -> Self {
        Self::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }

    pub fn save(self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(path, self.name());
    }
}

/// The calendar day that `now` belongs to, honoring the configured end of day.
pub fn today(settings: &CalendarSettings) -> NaiveDate {
    calendar::day_at(Local::now(), settings.day_start_hour())
}

/// Pulls calendars, events and Google Tasks for one day.
///
/// Blocking: callers run this on a worker thread. It uses the same OAuth token
/// file as the iced app, so signing in there is enough for the Dioxus view.
pub fn load_day(day: NaiveDate, settings: &CalendarSettings) -> Result<Snapshot, String> {
    calendar::sync_day(
        &settings.client_id,
        &settings.client_secret,
        &settings.disabled,
        day,
        settings.day_start_hour(),
        settings.app_calendar.as_deref(),
    )
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

/// Opens the browser sign-in flow and stores the returned token.
///
/// Blocking: this waits for Google's localhost redirect.
pub fn connect_google(settings: &CalendarSettings) -> Result<(), String> {
    calendar::connect(settings.client_id.clone(), settings.client_secret.clone())?;
    // Signing in is what lets the config sync reach Google Drive.
    timeslay_core::config_sync::poke();
    Ok(())
}

pub fn shift_day(day: NaiveDate, days: i64) -> NaiveDate {
    day + Duration::days(days)
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
    /// Minutes from the top of the displayed day window, when the needle
    /// belongs to this day.
    pub now_min: Option<i32>,
    pub hour_start: u32,
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
    pub time_block: bool,
    pub task_count: usize,
    pub planned_minutes: u32,
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
}

#[derive(Clone, Debug, PartialEq)]
pub struct BlockRef {
    pub event_id: String,
    pub title: String,
    pub time: String,
    pub start_min: i32,
}

pub fn build_model(snapshot: &Snapshot, settings: &CalendarSettings) -> DayModel {
    let day = snapshot.day;
    let hour_start = settings.day_start_hour();
    let offset = settings.day_end_minutes();
    let (view_start, view_end) = view_bounds(day, offset);

    let mut events = snapshot.events.clone();
    for event in &mut events {
        if event.calendar_id.starts_with("tasks:") && event.details.scheduled.is_none() {
            event.color = settings
                .task_color(&event.calendar_id)
                .unwrap_or(calendar::colors::TASK);
        }
    }

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
    let assigned_keys: HashSet<String> = events
        .iter()
        .filter(|event| event.is_task())
        .filter_map(|event| tb::metadata(event).block)
        .collect();
    let loaded_block_keys: HashSet<String> = events
        .iter()
        .filter(|event| is_time_block(event, settings, &assigned_keys))
        .map(tb::block_key)
        .collect();

    let blocks = build_blocks(&events, snapshot, settings, &assigned_keys);
    let block_refs: HashMap<String, BlockRef> = blocks
        .iter()
        .map(|block| (tb::block_key(&block.event), block_ref(block, day, offset)))
        .collect();

    let all_day = all_day_events(&events, settings, &loaded_block_keys, &priority);
    let timed = timed_events(
        &events,
        snapshot,
        settings,
        day,
        offset,
        view_start,
        view_end,
        &loaded_block_keys,
        &priority,
        &blocks,
    );
    let (task_groups, tasks_hidden) = task_groups(&events, snapshot, settings, &block_refs);

    let today = calendar::day_at(Local::now(), hour_start);
    let now_min =
        (day == today).then(|| minutes_into_day(Local::now(), day, offset).clamp(0, 24 * 60));

    DayModel {
        day,
        notice: snapshot.notice.clone(),
        timed,
        all_day,
        blocks,
        task_groups,
        tasks_hidden,
        now_min,
        hour_start,
    }
}

fn view_bounds(day: NaiveDate, offset: i32) -> (DateTime<Local>, DateTime<Local>) {
    let start = Local
        .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .unwrap_or_else(Local::now)
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
        && event.details.scheduled.is_none()
        && (settings.app_calendar.as_deref() == Some(event.calendar_id.as_str())
            || assigned_keys.contains(&tb::block_key(event)))
}

/// Mirrors `Store::calendar_enabled` for a source id.
fn source_enabled(id: &str, settings: &CalendarSettings) -> bool {
    if id.starts_with("tasks:") {
        settings.show_tasks
    } else {
        !settings.disabled.iter().any(|hidden| hidden == id)
    }
}

/// Mirrors `Store::visible_events`.
fn event_enabled(event: &Event, settings: &CalendarSettings) -> bool {
    source_enabled(event.source_id(), settings)
        && event
            .details
            .scheduled
            .as_ref()
            .is_none_or(|link| source_enabled(&link.calendar_id, settings))
}

/// Mirrors `omit_managed_tasks_event`: app-owned scheduled copies are shown
/// beside their task or inside their block, not as a second calendar chip.
fn managed_copy(event: &Event, settings: &CalendarSettings, block_keys: &HashSet<String>) -> bool {
    let Some(managed) = settings.app_calendar.as_deref() else {
        return false;
    };
    event
        .details
        .scheduled
        .as_ref()
        .is_some_and(|link| link.calendar_id == managed)
        && tb::metadata(event)
            .block
            .as_ref()
            .is_some_and(|key| block_keys.contains(key))
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
            id.strip_prefix("tasks:")
                .filter(|name| !name.is_empty())
                .unwrap_or("Tasks")
                .to_owned()
        })
}

fn all_day_events(
    events: &[Event],
    settings: &CalendarSettings,
    block_keys: &HashSet<String>,
    priority: &impl Fn(&Event) -> usize,
) -> Vec<Event> {
    let mut all_day: Vec<_> = events
        .iter()
        .filter(|event| !event.is_task() && event.all_day)
        .filter(|event| event_enabled(event, settings))
        .filter(|event| !managed_copy(event, settings, block_keys))
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
    block_keys: &HashSet<String>,
    priority: &impl Fn(&Event) -> usize,
    blocks: &[TimeBlock],
) -> Vec<TimedEvent> {
    let blocks_by_key: HashMap<String, &TimeBlock> = blocks
        .iter()
        .map(|block| (tb::block_key(&block.event), block))
        .collect();

    let mut visible: Vec<&Event> = events
        .iter()
        .filter(|event| !event.all_day)
        .filter(|event| event.start < view_end && event.end > view_start)
        .filter(|event| event_enabled(event, settings))
        .filter(|event| !managed_copy(event, settings, block_keys))
        // A task that already sits inside a visible block is read in that
        // block's popup, not drawn a second time as a chip on the grid.
        .filter(|event| {
            !(event.is_task()
                && tb::metadata(event)
                    .block
                    .as_ref()
                    .is_some_and(|key| blocks_by_key.contains_key(key)))
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
                title: if event.is_task() {
                    tb::task_label(event)
                } else {
                    event.title.clone()
                },
                calendar_name: calendar_name(snapshot, &event.calendar_id),
                start_min: span.start_min,
                end_min: span.end_min,
                column: slot.column,
                columns: slot.columns,
                time_block: block.is_some(),
                task_count: block.map_or(0, |block| block.tasks.len()),
                planned_minutes: block.map_or(0, |block| block.planned_minutes),
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
        .filter(|event| event_enabled(event, settings))
        .map(|block| {
            let key = tb::block_key(block);
            let mut tasks: Vec<&Event> = events
                .iter()
                .filter(|event| event.is_task())
                .filter(|event| tb::metadata(event).block.as_deref() == Some(key.as_str()))
                .collect();
            tasks.sort_by_key(|event| (tb::metadata(event).order, event.id.clone()));
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
) -> (Vec<TaskGroup>, bool) {
    let mut groups: Vec<TaskGroup> = Vec::new();

    for event in events.iter().filter(|event| event.is_task()) {
        // The task rail is the complete view of Google Tasks for the day, so
        // it follows the single Tasks toggle rather than the visibility of the
        // calendars a task happens to be scheduled on.
        if !settings.show_tasks {
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
        let block = metadata
            .block
            .as_ref()
            .and_then(|key| blocks.get(key))
            .cloned();
        groups[group_index].tasks.push(TaskRow {
            event: event.clone(),
            status: tb::task_status(event),
            minutes: metadata.minutes,
            block,
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
            &format!("task:{id}"),
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
                schedule: None,
                projects: Vec::new(),
                labels: Vec::new(),
                priority: None,
                location: None,
            },
            legacy_key: None,
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
    fn every_task_survives_a_hidden_schedule_calendar() {
        let mut assigned = task("a", "Write", None, 1, 45, false);
        assigned.details.scheduled = Some(calendar::scheduled::Link {
            calendar_id: "scheduled".into(),
            event_id: "event".into(),
            list_id: "work".into(),
            task_id: "a".into(),
            pending: false,
            block: None,
            order: 0,
            color_id: None,
        });
        let snapshot = Snapshot {
            calendars: vec![
                calendar("scheduled", "Tasks"),
                calendar("tasks:work", "Work"),
            ],
            events: vec![assigned],
            day: day(),
            fetched_at: at(10, 0),
            notice: None,
        };
        let settings = CalendarSettings {
            app_calendar: Some("scheduled".into()),
            disabled: vec!["scheduled".into()],
            ..CalendarSettings::default()
        };
        let model = build_model(&snapshot, &settings);

        assert_eq!(model.task_groups.len(), 1);
        assert_eq!(model.task_groups[0].tasks.len(), 1);
        assert!(model.timed.is_empty());
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
