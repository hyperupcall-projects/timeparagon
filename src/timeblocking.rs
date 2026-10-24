//! Google Tasks metadata and time-block operations. Notes remain portable across devices.
use crate::{
    app::{Message, State},
    blocking,
    calendar::{self, Event},
    config::CalendarSettings,
};
use chrono::{Local, NaiveDate, TimeZone};
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
    pub estimate: String,
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
    List(String),
    Close,
    Details(String),
    CancelDrag,
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
}
pub fn description(notes: &str) -> String {
    notes
        .lines()
        .filter(|l| !l.starts_with(META) && !l.trim().starts_with("ttid:"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}
pub fn metadata(notes: &str) -> Metadata {
    notes
        .lines()
        .find_map(|line| {
            line.strip_prefix(META)
                .and_then(|s| serde_json::from_str(s).ok())
        })
        .unwrap_or_default()
}
fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}
pub fn notes(notes: &str, meta: &Metadata) -> Result<String, String> {
    let existing = notes
        .lines()
        .filter_map(|l| l.trim().strip_prefix("ttid:"))
        .map(str::trim)
        .find(|v| valid_uuid(v));
    let id = if let Some(id) = existing {
        id.to_owned()
    } else {
        let mut b = [0u8; 16];
        getrandom::fill(&mut b).map_err(|e| e.to_string())?;
        b[6] = (b[6] & 15) | 64;
        b[8] = (b[8] & 63) | 128;
        let h: String = b.iter().map(|v| format!("{v:02x}")).collect();
        format!(
            "{}-{}-{}-{}-{}",
            &h[..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..]
        )
    };
    let prose = notes
        .lines()
        .filter(|l| !l.trim().starts_with("ttid:") && !l.starts_with(META))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!(
        "{}{}{}{}\nttid: {}",
        prose.trim_end(),
        if prose.trim_end().is_empty() {
            ""
        } else {
            "\n\n"
        },
        META,
        serde_json::to_string(meta).unwrap(),
        id
    ))
}
pub fn task_key(event: &Event) -> String {
    event
        .details
        .description
        .lines()
        .filter_map(|l| l.trim().strip_prefix("ttid:"))
        .map(str::trim)
        .find(|id| valid_uuid(id))
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}/{}", event.calendar_id, event.id))
}
pub fn assigned_all<'a>(state: &'a State, block: &Event) -> Vec<&'a Event> {
    let key = block_key(block);
    let mut tasks: Vec<_> = state
        .calendar
        .events
        .iter()
        .filter(|e| e.is_task() && metadata(&e.details.description).block.as_deref() == Some(&key))
        .collect();
    tasks.sort_by_key(|e| (metadata(&e.details.description).order, e.id.clone()));
    tasks
}
pub fn block_key(event: &Event) -> String {
    format!("{}/{}", event.calendar_id, event.id)
}
pub fn is_block(event: &Event, settings: &CalendarSettings) -> bool {
    !event.is_task()
        && !event.all_day
        && settings.time_block_calendar.as_deref() == Some(&event.calendar_id)
}
pub fn assigned<'a>(state: &'a State, block: &Event) -> Vec<&'a Event> {
    let key = block_key(block);
    let mut tasks: Vec<_> = state
        .calendar
        .events
        .iter()
        .filter(|e| {
            e.is_task()
                && !e.completed
                && metadata(&e.details.description).block.as_deref() == Some(&key)
        })
        .collect();
    tasks.sort_by_key(|e| (metadata(&e.details.description).order, e.id.clone()));
    tasks
}
pub fn overdue(event: &Event) -> bool {
    event.is_task()
        && !event.completed
        && event.start.date_naive() != UNDATED
        && event.start.date_naive() < Local::now().date_naive()
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
                metadata(&task.details.description).block,
                now,
            );
            state.config.timer.current_task = task.title.clone();
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
        Action::Estimate(value) => {
            state.timeblocking.estimate = value;
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
            let m = metadata(&target.details.description);
            let previous = state
                .calendar
                .events
                .iter()
                .filter(|e| e.is_task() && e.id != id && e.id != target.id)
                .map(|e| metadata(&e.details.description))
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
        Action::Saved(id, result) => {
            state.timeblocking.revision += 1;
            state.timeblocking.busy.remove(&id);
            match result {
                Ok(event) => {
                    state.calendar.events.retain(|e| e.id != id);
                    state.calendar.add_event(event);
                    state.calendar.open_event = None;
                    state.calendar.invalidate_other_days();
                }
                Err(error) => state.calendar.error = Some(error),
            }
            return Task::none();
        }
        Action::SaveEstimate(id) => {
            let Ok(minutes) = state.timeblocking.estimate.parse::<u32>() else {
                state.calendar.error = Some("Enter an estimate in whole minutes".into());
                return Task::none();
            };
            if minutes > 10080 {
                state.calendar.error = Some("Estimate must be at most 10080 minutes".into());
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
            if state.config.calendar.time_block_list.is_none() {
                state.calendar.error =
                    Some("Select a Google Tasks list in Time Blocking settings".into());
                return Task::none();
            }
            (
                id,
                Change::Block(block_key(event), event.start.date_naive()),
            )
        }
    };
    if state.timeblocking.busy.contains(&id) {
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
        .map(|e| metadata(&e.details.description).order)
        .max()
        .unwrap_or(0)
        .saturating_add(1_i64 << 32);
    state.timeblocking.revision += 1;
    state.timeblocking.busy.insert(id.clone());
    blocking::run(move || save(task, settings, change, order))
        .map(move |result| Message::TimeBlocking(Action::Saved(id.clone(), result)))
}
enum Change {
    Estimate(u32),
    Backlog,
    Day(NaiveDate),
    Block(String, NaiveDate),
    Reorder(Option<String>, i64, NaiveDate),
}
fn save(
    mut task: Event,
    settings: CalendarSettings,
    change: Change,
    order: i64,
) -> Result<Event, String> {
    let tokens = calendar::oauth::load_tokens().ok_or("Not connected to Google")?;
    let tokens =
        calendar::oauth::ensure_access(&settings.client_id, &settings.client_secret, tokens)?;
    let original_notes = task.details.description.clone();
    let original_day = task.start.date_naive();
    let mut m = metadata(&task.details.description);
    let mut due = None;
    let mut destination = None;
    match change {
        Change::Estimate(minutes) => m.minutes = minutes,
        Change::Backlog => {
            m.block = None;
            due = Some(UNDATED);
        }
        Change::Day(day) => {
            m.block = None;
            due = Some(day);
        }
        Change::Block(key, day) => {
            m.block = Some(key);
            m.order = order;
            due = Some(day);
            destination = settings.time_block_list;
        }
        Change::Reorder(block, order, day) => {
            m.block = block;
            m.order = order;
            if m.block.is_some() {
                destination = settings.time_block_list;
                due = Some(day);
            }
        }
    }
    task.details.description = notes(&task.details.description, &m)?;
    let list = task
        .calendar_id
        .strip_prefix("tasks:")
        .ok_or("Invalid task list")?;
    let id = task.id.strip_prefix("task:").ok_or("Invalid task")?;
    let base = format!(
        "https://tasks.googleapis.com/tasks/v1/lists/{}/tasks/{}",
        urlencoding::encode(list),
        urlencoding::encode(id)
    );
    let mut body = serde_json::json!({"notes":task.details.description});
    if let Some(day) = due {
        body["due"] = if day == UNDATED {
            serde_json::Value::Null
        } else {
            serde_json::json!(format!("{day}T00:00:00.000Z"))
        };
    }
    calendar::api::patch(
        &base,
        &tokens.access_token,
        &body.to_string(),
        "Google Tasks",
    )?;
    if let Some(day) = due {
        task.all_day = true;
        task.start = Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .ok_or("Invalid local date")?;
        task.end = task.start + chrono::Duration::days(1);
    }
    if let Some(dest) = destination {
        let dest = dest.strip_prefix("tasks:").unwrap_or(&dest);
        if dest != list {
            // Google may return a new ID when moving between lists.
            let url = format!(
                "{base}/move?destinationTasklist={}",
                urlencoding::encode(dest)
            );
            let response = calendar::api::post(&url, &tokens.access_token, "", "Move Google Task")
                .map_err(|e| {
                    let original_due = if original_day == UNDATED { serde_json::Value::Null } else { serde_json::json!(format!("{original_day}T00:00:00.000Z")) };
                    let rollback = serde_json::json!({"notes":original_notes, "due":original_due});
                    match calendar::api::patch(&base, &tokens.access_token, &rollback.to_string(), "Restore Google Task") {
                        Ok(()) => format!("Moving the task failed; its original notes and due date were restored: {e}"),
                        Err(rollback_error) => format!("Task notes saved, but moving lists failed: {e}. Restoring the original task also failed: {rollback_error}. Sync before retrying."),
                    }
                })?;
            let moved: serde_json::Value =
                serde_json::from_str(&response).map_err(|e| e.to_string())?;
            if let Some(id) = moved["id"].as_str() {
                task.id = format!("task:{id}");
            }
            task.calendar_id = format!("tasks:{dest}");
        }
    }
    Ok(task)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn marker_is_unique_last_and_stable() {
        let old = "human notes\nttid: 12345678-1234-4234-8234-123456789abc\nmore notes\nttid: bad";
        let m = Metadata {
            minutes: 60,
            block: Some("calendar/event".into()),
            order: 1,
        };
        let normalized = notes(old, &m).unwrap();
        assert!(normalized.ends_with("ttid: 12345678-1234-4234-8234-123456789abc"));
        assert_eq!(normalized.matches("ttid:").count(), 1);
        assert!(normalized.contains("human notes\nmore notes"));
        assert_eq!(metadata(&normalized), m);
        assert_eq!(notes(&normalized, &m).unwrap(), normalized);
    }
    #[test]
    fn creates_v4_uuid() {
        let n = notes("", &Metadata::default()).unwrap();
        let id = n.lines().last().unwrap().strip_prefix("ttid: ").unwrap();
        assert!(valid_uuid(id));
        assert_eq!(&id[14..15], "4");
    }
}
