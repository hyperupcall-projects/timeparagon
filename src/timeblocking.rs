//! Local task metadata and Google Tasks time-block operations.
use crate::{
    app::{Message, State},
    blocking,
    calendar::{self, Event},
    config::CalendarSettings,
};
use chrono::{DateTime, Local, NaiveDate, TimeZone};
use iced::{Size, Task, window};
use serde::{Deserialize, Serialize};

pub const UNDATED: NaiveDate = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
const META: &str = "tt-timeblocking: ";
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Metadata {
    pub block: Option<String>,
    pub minutes: u32,
    #[serde(default)]
    pub order: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<calendar::scheduled::Link>,
}
#[derive(Default)]
pub struct Workspace {
    pub block: Option<String>,
    pub focus: Option<String>,
    pub history_open: bool,
    pub work_now: Option<chrono::DateTime<chrono::Utc>>,
    pub revision: u64,
    pub original_width: Option<f32>,
    pub date: String,
    pub deadline: String,
    pub sync_notice: Option<String>,
    pub start_time: String,
    pub end_time: String,
    pub estimate: String,
    pub estimate_task: Option<String>,
    pub estimate_error: Option<String>,
    pub dragging: Option<String>,
    pub busy: std::collections::HashSet<String>,
}
#[derive(Debug, Clone)]
pub enum Action {
    View(String),
    CloseView,
    ToggleHistory,
    StartWork(String),
    StopWork(String),
    WorkTick,
    HideOverdue(bool),
    Calendar(String),
    TaskCalendar(String),
    Deadline(String),
    StartTime(String),
    EndTime(String),
    ScheduleEntered(String),
    SaveDeadline(String),
    Schedule(String, DateTime<Local>, DateTime<Local>),
    DropTime(DateTime<Local>),
    RefreshBlock(String),
    List(String),
    Close,
    Details(String),
    CancelDrag,
    EditEstimate(String),
    CloseEstimate,
    Estimate(String),
    Date(String),
    SaveEstimate(String),
    Backlog(String),
    Reschedule(String),
    Assign(String, String),
    Drag(String),
    DropBlock(String),
    DropTask(String),
    DropDay,
    Saved(String, Result<Event, String>),
    SavedMany(String, Result<Vec<Event>, String>),
}
pub fn description(notes: &str) -> String {
    if !notes
        .lines()
        .any(|line| line.starts_with(META) || line.trim().starts_with("ttid:"))
    {
        return notes.to_owned();
    }
    notes
        .lines()
        .filter(|l| !l.starts_with(META) && !l.trim().starts_with("ttid:"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}
pub fn metadata(event: &Event) -> Metadata {
    event
        .details
        .task_data
        .as_ref()
        .map(|data| data.metadata.clone())
        .unwrap_or_else(|| legacy_metadata(&event.details.description))
}
pub(crate) fn legacy_metadata(notes: &str) -> Metadata {
    notes
        .lines()
        .find_map(|line| {
            line.strip_prefix(META)
                .and_then(|s| serde_json::from_str(s).ok())
        })
        .unwrap_or_default()
}
pub(crate) fn legacy_id(notes: &str) -> Option<String> {
    notes
        .lines()
        .filter_map(|line| line.trim().strip_prefix("ttid:"))
        .map(str::trim)
        .find(|id| {
            (id.len() == 36
                && id.bytes().enumerate().all(|(i, c)| {
                    if [8, 13, 18, 23].contains(&i) {
                        c == b'-'
                    } else {
                        c.is_ascii_hexdigit()
                    }
                }))
                || (id.bytes().all(|c| c.is_ascii_digit())
                    && id.parse::<i64>().is_ok_and(|n| n >= 100))
        })
        .map(str::to_owned)
}
pub fn task_key(event: &Event) -> String {
    if let Some(data) = &event.details.task_data {
        return data
            .legacy_key
            .clone()
            .unwrap_or_else(|| data.id.to_string());
    }
    legacy_id(&event.details.description).unwrap_or_else(|| block_key(event))
}

pub fn assigned_all<'a>(state: &'a State, block: &Event) -> Vec<&'a Event> {
    let key = block_key(block);
    let mut tasks: Vec<_> = state
        .calendar
        .events
        .iter()
        .filter(|e| e.is_task() && metadata(e).block.as_deref() == Some(&key))
        .collect();
    tasks.sort_by_key(|e| (metadata(e).order, e.id.clone()));
    tasks
}
pub fn block_key(event: &Event) -> String {
    format!("{}/{}", event.calendar_id, event.id)
}
pub fn is_block(event: &Event, settings: &CalendarSettings) -> bool {
    !event.is_task()
        && event.details.scheduled.is_none()
        && !event.all_day
        && settings.time_block_calendar.as_deref() == Some(&event.calendar_id)
}
pub fn assigned<'a>(state: &'a State, block: &Event) -> Vec<&'a Event> {
    let key = block_key(block);
    let mut tasks: Vec<_> = state
        .calendar
        .events
        .iter()
        .filter(|e| e.is_task() && !e.completed && metadata(e).block.as_deref() == Some(&key))
        .collect();
    tasks.sort_by_key(|e| (metadata(e).order, e.id.clone()));
    tasks
}
pub fn overdue(event: &Event) -> bool {
    event.is_task()
        && !event.completed
        && calendar::deadline::read(&event.details.description)
            .ok()
            .flatten()
            .is_some_and(|date| date < Local::now().date_naive())
}
pub fn close(state: &mut State) -> Task<Message> {
    state.timeblocking.block = None;
    state.timeblocking.focus = None;
    state.timeblocking.dragging = None;
    match (state.main_window, state.timeblocking.original_width.take()) {
        (Some(id), Some(width)) => {
            let resize = window::resize(id, Size::new(width, state.main_size.height));
            if state.sidebar.is_some() {
                resize.chain(window::set_max_size(id, Some(Size::new(width, 100_000.0))))
            } else {
                resize
            }
        }
        _ => Task::none(),
    }
}
pub fn open(state: &mut State, id: String) -> Task<Message> {
    state.timeblocking.focus = None;
    state.timeblocking.block = Some(id);
    if state.timeblocking.original_width.is_none() {
        let width = state.main_size.width;
        state.timeblocking.original_width = Some(width);
        if let Some(id) = state.main_window {
            let resize = window::resize(id, Size::new(width + 400.0, state.main_size.height));
            return if state.sidebar.is_some() {
                window::set_max_size(id, Some(Size::new(width + 400.0, 100_000.0))).chain(resize)
            } else {
                resize
            };
        }
    }
    Task::none()
}
pub fn update(state: &mut State, action: Action) -> Task<Message> {
    let (id, change) = match action {
        Action::View(id) => {
            if !state
                .calendar
                .event(&id)
                .is_some_and(|e| is_block(e, &state.config.calendar))
            {
                return Task::none();
            }
            let resize = close(state);
            state.timeblocking.focus = Some(id);
            state.timeblocking.history_open = false;
            state.timeblocking.work_now = Some(chrono::Utc::now());
            state.calendar.open_event = None;
            state.calendar.menu_open = false;
            state.calendar.month_open = false;
            let unfocus = iced::advanced::widget::operate(
                iced::advanced::widget::operation::focusable::unfocus(),
            );
            return Task::batch([resize, unfocus]);
        }
        Action::CloseView => {
            state.timeblocking.focus = None;
            return Task::none();
        }
        Action::ToggleHistory => {
            state.timeblocking.history_open = !state.timeblocking.history_open;
            return Task::none();
        }
        Action::WorkTick => {
            state.timeblocking.work_now = Some(chrono::Utc::now());
            return Task::none();
        }
        Action::StartWork(id) => {
            let Some(task) = state
                .calendar
                .event(&id)
                .filter(|e| e.is_task() && !e.completed && !state.timeblocking.busy.contains(&id))
            else {
                return Task::none();
            };
            let now = chrono::Utc::now();
            state.config.task_work.start(
                task_key(task),
                task.title.clone(),
                metadata(task).block,
                now,
            );
            state.timeblocking.work_now = Some(now);
            return crate::app::queue_save(state);
        }
        Action::StopWork(id) => {
            if let Some(task) = state.calendar.event(&id) {
                state
                    .config
                    .task_work
                    .stop(&task_key(task), chrono::Utc::now());
                state.timeblocking.work_now = Some(chrono::Utc::now());
                return crate::app::queue_save(state);
            }
            return Task::none();
        }

        Action::HideOverdue(hide) => {
            state.config.calendar.hide_overdue_tasks = hide;
            return crate::app::queue_save(state);
        }
        Action::TaskCalendar(id) => {
            state.config.calendar.tasks_calendar = Some(id);
            return crate::app::queue_save(state);
        }
        Action::StartTime(value) => {
            state.timeblocking.start_time = value;
            return Task::none();
        }
        Action::EndTime(value) => {
            state.timeblocking.end_time = value;
            return Task::none();
        }
        Action::ScheduleEntered(id) => {
            let range = (|| {
                let day = calendar::deadline::parse_date(state.timeblocking.date.trim())?;
                let (sh, sm) = calendar::parse_clock(&state.timeblocking.start_time)
                    .ok_or("Enter a start time as HH:MM")?;
                let (eh, em) = calendar::parse_clock(&state.timeblocking.end_time)
                    .ok_or("Enter an end time as HH:MM")?;
                let start = Local
                    .from_local_datetime(&day.and_hms_opt(sh, sm, 0).unwrap())
                    .earliest()
                    .ok_or("This start time does not exist in your time zone")?;
                let end_day = if (eh, em) < (sh, sm) {
                    day.succ_opt().ok_or("Invalid end date")?
                } else {
                    day
                };
                let end = Local
                    .from_local_datetime(&end_day.and_hms_opt(eh, em, 0).unwrap())
                    .earliest()
                    .ok_or("This end time does not exist in your time zone")?;
                Ok::<_, String>((start, end))
            })();
            return match range {
                Ok((start, end)) => update(state, Action::Schedule(id, start, end)),
                Err(error) => {
                    state.calendar.error = Some(error);
                    Task::none()
                }
            };
        }
        Action::Deadline(value) => {
            state.timeblocking.deadline = value;
            return Task::none();
        }
        Action::SaveDeadline(id) => {
            let input = state.timeblocking.deadline.trim();
            let date = if input.is_empty() {
                None
            } else {
                match calendar::deadline::parse_date(input) {
                    Ok(date) => Some(date),
                    Err(error) => {
                        state.calendar.error = Some(error);
                        return Task::none();
                    }
                }
            };
            (id, Change::Deadline(date))
        }
        Action::DropTime(start) => {
            let Some(id) = state.timeblocking.dragging.take() else {
                return Task::none();
            };
            let minutes = state
                .calendar
                .event(&id)
                .map(|t| metadata(t).minutes)
                .unwrap_or(0);
            let minutes = if minutes == 0 { 30 } else { minutes };
            return update(
                state,
                Action::Schedule(
                    id,
                    start,
                    start + chrono::Duration::minutes(i64::from(minutes)),
                ),
            );
        }
        Action::Schedule(id, start, end) => {
            if end <= start {
                state.calendar.error = Some("The task must end after it starts".into());
                return Task::none();
            }
            (id, Change::Time(start, end))
        }
        Action::RefreshBlock(id) => return refresh_block(state, &id),
        Action::Calendar(id) => {
            state.config.calendar.time_block_calendar = Some(id);
            // Reveal the newly enabled View/Edit controls without opening a sidebar.
            state.calendar.open_event = None;
            return crate::app::queue_save(state);
        }
        Action::List(id) => {
            state.config.calendar.time_block_list = Some(id);
            return crate::app::queue_save(state);
        }
        Action::Close => return close(state),
        Action::CancelDrag => {
            state.timeblocking.dragging = None;
            return Task::none();
        }
        Action::Details(id) => {
            state.calendar.open_event = Some(id);
            return Task::none();
        }
        Action::Date(value) => {
            state.timeblocking.date = value;
            return Task::none();
        }
        Action::EditEstimate(id) => {
            let Some(task) = state.calendar.event(&id).filter(|e| e.is_task()) else {
                return Task::none();
            };
            if state.timeblocking.busy.contains(&id) {
                return Task::none();
            }
            state.timeblocking.estimate = metadata(task).minutes.to_string();
            state.timeblocking.estimate_task = Some(id);
            state.timeblocking.estimate_error = None;
            state.timeblocking.dragging = None;
            return iced::widget::operation::focus(iced::widget::Id::new("task-estimate"));
        }
        Action::CloseEstimate => {
            state.timeblocking.estimate_task = None;
            state.timeblocking.estimate_error = None;
            return Task::none();
        }
        Action::Estimate(value) => {
            state.timeblocking.estimate = value;
            state.timeblocking.estimate_error = None;
            return Task::none();
        }
        Action::Drag(id) => {
            state.timeblocking.dragging = Some(id);
            return Task::none();
        }
        Action::DropBlock(block) => {
            if let Some(id) = state.timeblocking.dragging.take() {
                return update(state, Action::Assign(id, block));
            }
            return Task::none();
        }
        Action::DropTask(target) => {
            let Some(id) = state.timeblocking.dragging.take() else {
                return Task::none();
            };
            if id == target {
                return Task::none();
            }
            let Some(target) = state.calendar.event(&target) else {
                return Task::none();
            };
            let m = metadata(target);
            let previous = state
                .calendar
                .events
                .iter()
                .filter(|e| e.is_task() && e.id != id && e.id != target.id)
                .map(|e| metadata(e))
                .filter(|other| other.block == m.block && other.order < m.order)
                .map(|other| other.order)
                .max();
            let rank = previous.map_or(m.order.saturating_sub(1_i64 << 32), |previous| {
                (i128::from(previous) + (i128::from(m.order) - i128::from(previous)) / 2) as i64
            });
            (
                id,
                Change::Reorder(m.block, rank, target.start.date_naive()),
            )
        }
        Action::DropDay => {
            let Some(id) = state.timeblocking.dragging.take() else {
                return Task::none();
            };
            (id, Change::Day(state.calendar.day))
        }
        Action::SavedMany(id, result) => {
            state.timeblocking.busy.remove(&id);
            let mut tasks = Vec::new();
            match result {
                Ok(events) => {
                    for (index, event) in events.into_iter().enumerate() {
                        let previous_id = if index == 0 {
                            id.clone()
                        } else {
                            event.id.clone()
                        };
                        tasks.push(update(state, Action::Saved(previous_id, Ok(event))));
                    }
                }
                Err(error) => {
                    state.timeblocking.sync_notice = Some(error.clone());
                    tasks.push(update(state, Action::Saved(id, Err(error))));
                }
            }
            // Also reconciles a partially successful cross-service write.
            tasks.push(Task::done(Message::CalendarSyncRequested));
            return Task::batch(tasks);
        }
        Action::Saved(id, result) => {
            state.timeblocking.revision += 1;
            state.timeblocking.busy.remove(&id);
            let editing_estimate = state.timeblocking.estimate_task.as_deref() == Some(&id);
            match result {
                Ok(event) => {
                    if let Some(link) = &event.details.scheduled {
                        if state.config.calendar.tasks_calendar.is_none() {
                            state.config.calendar.tasks_calendar = Some(link.calendar_id.clone());
                        }
                        if state.config.calendar.time_block_list.is_none() {
                            state.config.calendar.time_block_list =
                                Some(format!("tasks:{}", link.list_id));
                        }
                    }
                    let key = task_key(&event);
                    state
                        .calendar
                        .events
                        .retain(|e| e.id != id && (!e.is_task() || task_key(e) != key));
                    state.calendar.add_event(event);
                    if editing_estimate {
                        state.timeblocking.estimate_task = None;
                        state.timeblocking.estimate_error = None;
                    } else {
                        state.calendar.open_event = None;
                    }
                    state.calendar.invalidate_other_days();
                }
                Err(error) if editing_estimate => state.timeblocking.estimate_error = Some(error),
                Err(error) => state.calendar.error = Some(error),
            }
            return crate::app::queue_save(state);
        }
        Action::SaveEstimate(id) => {
            if state.timeblocking.estimate_task.as_deref() != Some(&id) {
                return Task::none();
            }
            let Ok(minutes) = state.timeblocking.estimate.trim().parse::<u32>() else {
                state.timeblocking.estimate_error =
                    Some("Enter an estimate in whole minutes".into());
                return Task::none();
            };
            if minutes > 10080 {
                state.timeblocking.estimate_error =
                    Some("Estimate must be at most 10080 minutes".into());
                return Task::none();
            }
            (id, Change::Estimate(minutes))
        }
        Action::Backlog(id) => (id, Change::Backlog),
        Action::Reschedule(id) => {
            let Ok(day) = NaiveDate::parse_from_str(&state.timeblocking.date, "%Y-%m-%d") else {
                state.calendar.error = Some("Enter a date as YYYY-MM-DD".into());
                return Task::none();
            };
            (id, Change::Day(day))
        }
        Action::Assign(id, block) => {
            let Some(event) = state
                .calendar
                .event(&block)
                .filter(|e| is_block(e, &state.config.calendar))
            else {
                return Task::none();
            };
            (id, Change::Block(block_key(event), event.start, event.end))
        }
    };
    if !state.timeblocking.busy.is_empty() {
        return Task::none();
    }
    let Some(task) = state.calendar.event(&id).filter(|e| e.is_task()).cloned() else {
        return Task::none();
    };
    let settings = state.config.calendar.clone();
    let order = state
        .calendar
        .events
        .iter()
        .filter(|e| e.is_task())
        .map(|e| metadata(e).order)
        .max()
        .unwrap_or(0)
        .saturating_add(1_i64 << 32);
    state.timeblocking.revision += 1;
    state.timeblocking.busy.insert(id.clone());
    let blocks: Vec<_> = state
        .calendar
        .events
        .iter()
        .filter(|e| is_block(e, &settings))
        .cloned()
        .collect();
    let related: Vec<_> = state
        .calendar
        .events
        .iter()
        .filter(|e| e.is_task())
        .cloned()
        .collect();
    blocking::run(move || save_and_reflow(task, settings, change, order, blocks, related))
        .map(move |result| Message::TimeBlocking(Action::SavedMany(id.clone(), result)))
}
enum Change {
    Estimate(u32),
    Deadline(Option<NaiveDate>),
    Backlog,
    Day(NaiveDate),
    Block(String, DateTime<Local>, DateTime<Local>),
    Reorder(Option<String>, i64, NaiveDate),
    Time(DateTime<Local>, DateTime<Local>),
}

fn access(settings: &CalendarSettings) -> Result<String, String> {
    let tokens = calendar::oauth::load_tokens().ok_or("Not connected to Google")?;
    calendar::oauth::ensure_access(&settings.client_id, &settings.client_secret, tokens)
        .map(|tokens| tokens.access_token)
}

fn save_and_reflow(
    task: Event,
    settings: CalendarSettings,
    change: Change,
    order: i64,
    blocks: Vec<Event>,
    mut related: Vec<Event>,
) -> Result<Vec<Event>, String> {
    let original_id = task.id.clone();
    let old_block = metadata(&task).block;
    let saved = save(task, &settings, change, order)?;
    let new_block = metadata(&saved).block;
    related.retain(|e| e.id != original_id);
    related.push(saved.clone());
    let mut changed = vec![saved];
    for block in blocks.iter().filter(|b| {
        let key = block_key(b);
        old_block.as_ref() == Some(&key) || new_block.as_ref() == Some(&key)
    }) {
        for event in reflow(block, &related, &settings)? {
            if let Some(previous) = changed.iter_mut().find(|e| e.id == event.id) {
                *previous = event;
            } else {
                changed.push(event);
            }
        }
    }
    Ok(changed)
}

fn refresh_block(state: &mut State, id: &str) -> Task<Message> {
    let Some(block) = state
        .calendar
        .event(id)
        .filter(|e| is_block(e, &state.config.calendar))
        .cloned()
    else {
        return Task::none();
    };
    let tasks: Vec<_> = assigned_all(state, &block).into_iter().cloned().collect();
    if tasks.is_empty() {
        return Task::none();
    }
    let settings = state.config.calendar.clone();
    let first = tasks[0].id.clone();
    state.timeblocking.revision += 1;
    state.timeblocking.busy.insert(first.clone());
    blocking::run(move || reflow(&block, &tasks, &settings))
        .map(move |result| Message::TimeBlocking(Action::SavedMany(first.clone(), result)))
}

fn reflow(
    block: &Event,
    tasks: &[Event],
    settings: &CalendarSettings,
) -> Result<Vec<Event>, String> {
    let key = block_key(block);
    let mut tasks: Vec<_> = tasks
        .iter()
        .filter(|e| metadata(e).block.as_deref() == Some(&key))
        .cloned()
        .collect();
    tasks.sort_by_key(|e| (metadata(e).order, e.id.clone()));
    let mut start = block.start;
    let mut changed = Vec::new();
    let mut token = None;
    for task in tasks {
        let minutes = metadata(&task).minutes;
        let end =
            start + chrono::Duration::minutes(i64::from(if minutes == 0 { 30 } else { minutes }));
        // Completed blocks are history and are never shifted by later planning.
        if !task.completed && (task.all_day || task.start != start || task.end != end) {
            let token = match &token {
                Some(token) => token,
                None => token.insert(access(settings)?),
            };
            let mut database = crate::task_db::open()?;
            let mut task = task;
            let mut data = database.import(&task)?;
            schedule(
                &mut task,
                &mut data,
                &mut database,
                settings,
                token,
                start,
                end,
            )?;
            changed.push(task);
        }
        start = end;
    }
    Ok(changed)
}

fn save(
    mut task: Event,
    settings: &CalendarSettings,
    change: Change,
    order: i64,
) -> Result<Event, String> {
    let mut database = crate::task_db::open()?;
    let mut data = database.import(&task)?;
    // A previous attempt may have moved the task before Calendar failed. The
    // durable link retains its new identity so retrying the old chip is safe.
    if let Some(link) = &data.metadata.schedule {
        task.id = format!("task:{}", link.task_id);
        task.calendar_id = format!("tasks:{}", link.list_id);
        task.details.scheduled = Some(link.clone());
    }
    task.details.description = description(&task.details.description);
    match change {
        Change::Estimate(minutes) => {
            data.metadata.minutes = minutes;
            database.save(&task, &data)?;
        }
        Change::Deadline(date) => {
            let token = access(settings)?;
            let notes = calendar::deadline::write(&task.details.description, date);
            let base = task_url(&task)?;
            calendar::api::patch(
                &base,
                &token,
                &serde_json::json!({"notes": notes}).to_string(),
                "Save task deadline",
            )?;
            task.details.description = notes;
            if let Some(link) = &data.metadata.schedule {
                calendar::scheduled::request(
                    "PATCH",
                    &link.event_url(),
                    &token,
                    Some(&calendar::scheduled::content(&task, link)),
                )
                .map_err(|e| e.message)?;
            }
        }
        Change::Backlog | Change::Day(_) => {
            let day = if let Change::Day(day) = change {
                day
            } else {
                UNDATED
            };
            let token = access(settings)?;
            // Delete only the calendar block, preserving the task and deadline.
            if let Some(link) = &data.metadata.schedule {
                calendar::scheduled::remove(&token, link)?;
            }
            let body = serde_json::json!({"due": if day == UNDATED { serde_json::Value::Null }
                else { serde_json::json!(format!("{day}T00:00:00.000Z")) }});
            calendar::api::patch(
                &task_url(&task)?,
                &token,
                &body.to_string(),
                "Schedule task day",
            )?;
            data.metadata.block = None;
            data.metadata.schedule = None;
            task.details.scheduled = None;
            task.all_day = true;
            task.start = Local
                .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
                .earliest()
                .ok_or("Invalid local date")?;
            task.end = task.start + chrono::Duration::days(1);
            database.save(&task, &data)?;
        }
        Change::Block(key, start, _end) => {
            data.metadata.block = Some(key);
            data.metadata.order = order;
            let minutes = if data.metadata.minutes == 0 {
                30
            } else {
                data.metadata.minutes
            };
            let token = access(settings)?;
            schedule(
                &mut task,
                &mut data,
                &mut database,
                settings,
                &token,
                start,
                start + chrono::Duration::minutes(i64::from(minutes)),
            )?;
        }
        Change::Time(start, end) => {
            data.metadata.block = None;
            let token = access(settings)?;
            schedule(
                &mut task,
                &mut data,
                &mut database,
                settings,
                &token,
                start,
                end,
            )?;
        }
        Change::Reorder(block, order, day) => {
            data.metadata.block = block;
            data.metadata.order = order;
            if data.metadata.block.is_some() {
                let token = access(settings)?;
                let start = if task.all_day {
                    Local
                        .from_local_datetime(&day.and_hms_opt(9, 0, 0).unwrap())
                        .earliest()
                        .ok_or("Invalid local date")?
                } else {
                    task.start
                };
                let minutes = if data.metadata.minutes == 0 {
                    30
                } else {
                    data.metadata.minutes
                };
                schedule(
                    &mut task,
                    &mut data,
                    &mut database,
                    settings,
                    &token,
                    start,
                    start + chrono::Duration::minutes(i64::from(minutes)),
                )?;
            } else {
                database.save(&task, &data)?;
            }
        }
    }
    task.details.task_data = Some(data);
    Ok(task)
}

fn task_url(task: &Event) -> Result<String, String> {
    let list = task
        .calendar_id
        .strip_prefix("tasks:")
        .ok_or("Invalid task list")?;
    let id = task.id.strip_prefix("task:").ok_or("Invalid task")?;
    Ok(format!(
        "{}/{}/tasks/{}",
        calendar::api::TASKS,
        urlencoding::encode(list),
        urlencoding::encode(id)
    ))
}

fn schedule(
    task: &mut Event,
    data: &mut crate::task_db::TaskData,
    database: &mut crate::task_db::Database,
    settings: &CalendarSettings,
    token: &str,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<(), String> {
    schedule_with(
        task,
        data,
        database,
        settings,
        &calendar::scheduled::Google { token },
        start,
        end,
    )
}

fn schedule_with(
    task: &mut Event,
    data: &mut crate::task_db::TaskData,
    database: &mut crate::task_db::Database,
    settings: &CalendarSettings,
    remote: &impl calendar::scheduled::Remote,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<(), String> {
    let old = data.metadata.schedule.clone();
    if let Some(link) = &old {
        if task.id != format!("task:{}", link.task_id)
            || task.calendar_id != format!("tasks:{}", link.list_id)
        {
            task.id = format!("task:{}", link.task_id);
            task.calendar_id = format!("tasks:{}", link.list_id);
            task.details.link = format!(
                "https://tasks.google.com/task/{}",
                urlencoding::encode(&link.task_id)
            );
        }
    }
    let (destination_calendar, destination_list) = remote.destinations(
        settings
            .tasks_calendar
            .as_deref()
            .or_else(|| old.as_ref().map(|l| l.calendar_id.as_str())),
        settings
            .time_block_list
            .as_deref()
            .or_else(|| old.as_ref().map(|l| l.list_id.as_str())),
    )?;
    let mut link = match old.clone() {
        Some(link) => link,
        None => calendar::scheduled::Link::new(destination_calendar.clone(), task)?,
    };
    // Reserve a stable event ID before any remote write. Preserve aliases and
    // this reservation even if a later API call fails.
    link.calendar_id = destination_calendar;
    link.pending = true;
    link.block = data.metadata.block.clone();
    link.order = data.metadata.order;
    data.metadata.schedule = Some(link.clone());
    database.save(task, data)?;
    if link.list_id != destination_list {
        let response = remote.move_task(&link, &destination_list)?;
        link.task_id = response["id"]
            .as_str()
            .ok_or("Google did not return the moved task ID")?
            .to_owned();
        link.list_id = destination_list;
        task.id = format!("task:{}", link.task_id);
        task.calendar_id = format!("tasks:{}", link.list_id);
        task.details.link = response["webViewLink"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "https://tasks.google.com/task/{}",
                    urlencoding::encode(&link.task_id)
                )
            });
        data.metadata.schedule = Some(link.clone());
        database.save(task, data)?;
    }
    // The native Tasks date is the planned day, never the description deadline.
    remote.set_day(&link, start.date_naive())?;
    remote.save_event(task, &link, start, end).map_err(|e| {
        format!("Task saved, but its calendar block could not be saved: {e}. Sync or retry.")
    })?;
    if let Some(old) = old.filter(|old| old.calendar_id != link.calendar_id) {
        remote.remove_event(&old)?;
    }
    link.pending = false;
    task.start = start;
    task.end = end;
    task.all_day = false;
    task.details.scheduled = Some(link.clone());
    data.metadata.schedule = Some(link);
    database.save(task, data)?;
    task.details.task_data = Some(data.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeGoogle {
        calls: std::cell::RefCell<Vec<String>>,
        fail_calendar: std::cell::Cell<bool>,
        event_ids: std::cell::RefCell<Vec<String>>,
    }
    impl calendar::scheduled::Remote for FakeGoogle {
        fn destinations(
            &self,
            calendar: Option<&str>,
            list: Option<&str>,
        ) -> Result<(String, String), String> {
            Ok((
                calendar.unwrap_or("custom-calendar").to_owned(),
                list.unwrap_or("has-time")
                    .trim_start_matches("tasks:")
                    .to_owned(),
            ))
        }
        fn move_task(
            &self,
            link: &calendar::scheduled::Link,
            destination: &str,
        ) -> Result<serde_json::Value, String> {
            assert_eq!(link.list_id, "original-list");
            assert_eq!(destination, "has-time");
            self.calls.borrow_mut().push("move".into());
            Ok(
                serde_json::json!({"id":"moved-task", "webViewLink":"https://tasks.google.com/task/moved-task"}),
            )
        }
        fn set_day(&self, link: &calendar::scheduled::Link, _day: NaiveDate) -> Result<(), String> {
            assert_eq!(link.task_id, "moved-task");
            self.calls.borrow_mut().push("day".into());
            Ok(())
        }
        fn save_event(
            &self,
            task: &Event,
            link: &calendar::scheduled::Link,
            _start: DateTime<Local>,
            _end: DateTime<Local>,
        ) -> Result<(), String> {
            assert_eq!(task.id, "task:moved-task");
            assert_eq!(
                task.details.description,
                "DUE: 2026-12-31\n\nKeep these notes."
            );
            assert_eq!(link.calendar_id, "custom-calendar");
            self.calls.borrow_mut().push("event".into());
            self.event_ids.borrow_mut().push(link.event_id.clone());
            if self.fail_calendar.get() {
                Err("Calendar is offline".into())
            } else {
                Ok(())
            }
        }
        fn remove_event(&self, _link: &calendar::scheduled::Link) -> Result<(), String> {
            panic!("History must not be removed")
        }
    }

    #[test]
    fn a_failed_calendar_write_can_retry_after_a_list_move_without_losing_identity_or_deadline() {
        let mut task = calendar::sample_snapshot().events.remove(0);
        task.id = "task:original-task".into();
        task.calendar_id = "tasks:original-list".into();
        task.all_day = true;
        task.details.description = "DUE: 2026-12-31\n\nKeep these notes.".into();
        let original = task.clone();
        let scratch = std::env::temp_dir().join(format!(
            "tt-schedule-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let mut database = crate::task_db::Database::open(&scratch).unwrap();
        let mut data = database.import(&task).unwrap();
        let local_id = data.id;
        data.metadata.block = Some("work/block".into());
        let start = task.start;
        let end = start + chrono::Duration::minutes(45);
        let remote = FakeGoogle::default();
        remote.fail_calendar.set(true);
        let settings = CalendarSettings {
            tasks_calendar: Some("custom-calendar".into()),
            time_block_list: Some("tasks:has-time".into()),
            ..Default::default()
        };
        let error = schedule_with(
            &mut task,
            &mut data,
            &mut database,
            &settings,
            &remote,
            start,
            end,
        )
        .unwrap_err();
        assert!(error.contains("Calendar is offline"));
        let persisted = database.lookup(&original).unwrap().unwrap();
        assert_eq!(persisted.id, local_id);
        assert_eq!(
            persisted.metadata.schedule.as_ref().unwrap().task_id,
            "moved-task"
        );
        assert!(persisted.metadata.schedule.as_ref().unwrap().pending);
        assert_eq!(database.lookup(&task).unwrap().unwrap().id, local_id);

        // Simulate retrying the old chip after an app restart.
        task = original;
        data = persisted;
        remote.fail_calendar.set(false);
        schedule_with(
            &mut task,
            &mut data,
            &mut database,
            &settings,
            &remote,
            start,
            end,
        )
        .unwrap();
        assert_eq!(
            remote
                .calls
                .borrow()
                .iter()
                .filter(|call| call.as_str() == "move")
                .count(),
            1
        );
        assert_eq!(remote.event_ids.borrow()[0], remote.event_ids.borrow()[1]);
        assert_eq!(task.id, "task:moved-task");
        assert_eq!(task.calendar_id, "tasks:has-time");
        assert!(!task.all_day);
        assert!(!data.metadata.schedule.as_ref().unwrap().pending);
        assert_eq!(task_key(&task), local_id.to_string());
        assert_eq!(
            task.details.description,
            "DUE: 2026-12-31\n\nKeep these notes."
        );
        assert_eq!(task.start, start);
        assert_eq!(task.end, end);
        drop(database);
        std::fs::remove_file(scratch).unwrap();
    }

    #[test]
    fn invalid_deadline_input_leaves_the_task_untouched() {
        let mut state = estimate_state();
        state.timeblocking.deadline = "2026-02-30".into();
        let before = state.calendar.event("task:deep").unwrap().clone();
        let _ = update(&mut state, Action::SaveDeadline("task:deep".into()));
        assert_eq!(state.calendar.event("task:deep"), Some(&before));
        assert!(state.timeblocking.busy.is_empty());
        assert!(
            state
                .calendar
                .error
                .as_deref()
                .unwrap()
                .contains("YYYY-MM-DD")
        );
    }

    #[test]
    fn overdue_uses_the_description_deadline_independently_of_scheduling() {
        let mut task = calendar::sample_snapshot().events.remove(0);
        task.id = "task:deadline".into();
        task.calendar_id = "tasks:list".into();
        task.start = Local::now() + chrono::Duration::days(5);
        let yesterday = (Local::now() - chrono::Duration::days(1)).date_naive();
        task.details.description = format!("DUE: {yesterday}\nNotes");
        assert!(overdue(&task));
        task.details.description.clear();
        task.start = Local::now() - chrono::Duration::days(5);
        assert!(!overdue(&task));
    }

    fn estimate_state() -> State {
        let mut state = State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        );
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
        let task = state.calendar.event_mut("deep").unwrap();
        task.id = "task:deep".into();
        task.calendar_id = "tasks:list".into();
        task.details.description = "Notes".into();
        task.details.task_data = Some(crate::task_db::TaskData {
            id: 100,
            metadata: Metadata {
                minutes: 35,
                ..Default::default()
            },
            legacy_key: None,
        });
        state.timeblocking.focus = Some("standup".into());
        state
    }

    #[test]
    fn estimate_dialog_prefills_and_cancel_preserves_the_view_and_task() {
        let mut state = estimate_state();
        let _ = update(&mut state, Action::EditEstimate("task:deep".into()));
        assert_eq!(state.timeblocking.estimate, "35");
        let _ = update(&mut state, Action::Estimate("90".into()));
        let _ = update(&mut state, Action::CloseEstimate);
        assert!(state.timeblocking.estimate_task.is_none());
        assert_eq!(state.timeblocking.focus.as_deref(), Some("standup"));
        assert_eq!(
            metadata(state.calendar.event("task:deep").unwrap()).minutes,
            35
        );
    }

    #[test]
    fn estimate_errors_stay_in_the_dialog_and_a_successful_retry_closes_it() {
        let mut state = estimate_state();
        state.calendar.open_event = Some("task:deep".into());
        let _ = update(&mut state, Action::EditEstimate("task:deep".into()));
        for invalid in ["abc", "-1", "10081"] {
            let _ = update(&mut state, Action::Estimate(invalid.into()));
            let _ = update(&mut state, Action::SaveEstimate("task:deep".into()));
            assert!(state.timeblocking.estimate_error.is_some());
            assert!(state.timeblocking.estimate_task.is_some());
            assert!(!state.timeblocking.busy.contains("task:deep"));
            assert!(state.calendar.error.is_none());
        }
        let _ = update(&mut state, Action::Estimate("60".into()));
        let _ = update(
            &mut state,
            Action::Saved("task:deep".into(), Err("Could not save".into())),
        );
        assert_eq!(
            state.timeblocking.estimate_error.as_deref(),
            Some("Could not save")
        );
        let mut saved = state.calendar.event("task:deep").unwrap().clone();
        let mut meta = metadata(&saved);
        meta.minutes = 60;
        saved.details.task_data.as_mut().unwrap().metadata = meta;
        let _ = update(&mut state, Action::Saved("task:deep".into(), Ok(saved)));
        assert!(state.timeblocking.estimate_task.is_none());
        assert!(state.timeblocking.estimate_error.is_none());
        assert_eq!(state.calendar.open_event.as_deref(), Some("task:deep"));
        assert_eq!(state.timeblocking.focus.as_deref(), Some("standup"));
        assert_eq!(
            metadata(state.calendar.event("task:deep").unwrap()).minutes,
            60
        );
    }

    #[test]
    fn legacy_markers_are_importable_and_removed_from_prose() {
        let old = "human notes\nttid: 12345678-1234-4234-8234-123456789abc\nmore notes\ntt-timeblocking: {\"minutes\":60,\"block\":null,\"order\":1}";
        assert_eq!(description(old), "human notes\nmore notes");
        assert_eq!(legacy_metadata(old).minutes, 60);
        assert_eq!(
            legacy_id(old).as_deref(),
            Some("12345678-1234-4234-8234-123456789abc")
        );
    }
}
