//! Task metadata and the pure time-block rules.
//!
//! The iced crate keeps the interactive time-block workspace; this module has
//! only what a read-only day view needs, and what the calendar/task modules
//! themselves use.

use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::calendar::{self, Event};
use crate::config::CalendarSettings;

pub const UNDATED: NaiveDate = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
const META: &str = "tt-timeblocking: ";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Metadata {
    pub block: Option<String>,
    pub minutes: u32,
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub ignored: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<calendar::scheduled::Link>,
    /// The project this task is filed under, by `config::Project::id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Where the task happens, by `config::Place::id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<u64>,
}

/// How urgent a task is. P1 is the most urgent; a task with neither has no
/// priority at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    P1,
    P2,
}

impl Priority {
    pub const ALL: [Priority; 2] = [Priority::P1, Priority::P2];

    pub fn label(self) -> &'static str {
        match self {
            Self::P1 => "P1",
            Self::P2 => "P2",
        }
    }
}

/// Completed and ignored tasks both count as completed in Google Tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Active,
    Completed,
    Ignored,
}

pub fn task_status(task: &Event) -> TaskStatus {
    if !task.completed {
        TaskStatus::Active
    } else if metadata(task).ignored {
        TaskStatus::Ignored
    } else {
        TaskStatus::Completed
    }
}

pub fn apply_status(task: &mut Event, status: TaskStatus) {
    task.completed = status != TaskStatus::Active;
    let ignored = status == TaskStatus::Ignored;
    if let Some(data) = &mut task.details.task_data {
        data.metadata.ignored = ignored;
    } else {
        let mut meta = metadata(task);
        meta.ignored = ignored;
        // Legacy tasks can be edited before their first database import.
        task.details.description = format!(
            "{}\n{META}{}",
            description(&task.details.description),
            serde_json::to_string(&meta).expect("metadata serializes")
        );
    }
}

pub fn task_label(task: &Event) -> String {
    match task_status(task) {
        TaskStatus::Active => task.title.clone(),
        TaskStatus::Completed => format!("{} · Completed ✓", task.title),
        TaskStatus::Ignored => format!("{} · Ignored", task.title),
    }
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
        .filter(|line| !line.starts_with(META) && !line.trim().starts_with("ttid:"))
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

pub fn legacy_metadata(notes: &str) -> Metadata {
    notes
        .lines()
        .find_map(|line| {
            line.strip_prefix(META)
                .and_then(|s| serde_json::from_str(s).ok())
        })
        .unwrap_or_default()
}

pub fn legacy_id(notes: &str) -> Option<String> {
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

pub fn block_key(event: &Event) -> String {
    format!("{}/{}", event.calendar_id, event.id)
}

pub fn is_block(event: &Event, settings: &CalendarSettings) -> bool {
    !event.is_task()
        && event.details.scheduled.is_none()
        && !event.all_day
        && settings.time_block_calendar.as_deref() == Some(&event.calendar_id)
}

/// A task that still needs a time: unscheduled, or waiting on a schedule write.
pub fn in_backlog(event: &Event) -> bool {
    let meta = metadata(event);
    let pending = meta.schedule.as_ref().is_some_and(|link| link.pending);
    event.is_task()
        && !event.completed
        && (pending || (meta.schedule.is_none() && meta.block.is_none()))
}

/// A task whose date is the current day. An undated task is not.
pub fn on_today(event: &Event) -> bool {
    event.start.date_naive() == Local::now().date_naive()
}

pub fn overdue(event: &Event) -> bool {
    event.is_task()
        && !event.completed
        && calendar::deadline::read_deadline(&event.details.description)
            .ok()
            .flatten()
            .is_some_and(|deadline| deadline.is_past(Local::now().naive_local()))
}
