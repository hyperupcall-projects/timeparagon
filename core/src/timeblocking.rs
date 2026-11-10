//! Task metadata and the pure time-block rules.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::calendar::{self, Event};
use crate::config::CalendarSettings;

pub const UNDATED: NaiveDate = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    /// Habit library entry represented by this planned task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub habit_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<calendar::deadline::Deadline>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_source: Option<i64>,
    /// The recurring time block series this task repeats with, as
    /// `calendar/series`. Each occurrence belongs to that day's block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_series: Option<String>,
    /// Creation instant recorded by TimeSlay; absent for tasks created elsewhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Planning horizon in the backlog; older and newly created tasks use Someday.
    #[serde(default)]
    pub backlog_section: BacklogSection,
    pub block: Option<String>,
    pub minutes: u32,
    #[serde(default)]
    pub order: i64,
    #[serde(default)]
    pub ignored: bool,
    /// The projects this task is filed under, by `config::Project::id`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<u64>,
    /// Free-form labels, such as "urgent" or "deep work".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    /// Where the task happens, by `config::Place::id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<u64>,
}

/// Ordered planning horizons for parked tasks. These do not change due dates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BacklogSection {
    NextWeekOrTwo,
    NextMonth,
    NextQuarter,
    NextYear,
    #[default]
    Someday,
    Never,
}

impl BacklogSection {
    pub const ALL: [Self; 6] = [
        Self::NextWeekOrTwo,
        Self::NextMonth,
        Self::NextQuarter,
        Self::NextYear,
        Self::Someday,
        Self::Never,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::NextWeekOrTwo => "Next week or two",
            Self::NextMonth => "Next month",
            Self::NextQuarter => "Next quarter",
            Self::NextYear => "Next year",
            Self::Someday => "Someday",
            Self::Never => "Never",
        }
    }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    }
}

pub fn task_label(task: &Event) -> String {
    let title = if metadata(task).habit_id.is_some() {
        format!("{} · Habit", task.title)
    } else {
        task.title.clone()
    };
    match task_status(task) {
        TaskStatus::Active => title,
        TaskStatus::Completed => format!("{title} · Completed ✓"),
        TaskStatus::Ignored => format!("{title} · Ignored"),
    }
}

pub fn metadata(event: &Event) -> Metadata {
    event
        .details
        .task_data
        .as_ref()
        .map(|data| data.metadata.clone())
        .unwrap_or_default()
}

pub fn task_key(event: &Event) -> String {
    event
        .details
        .task_data
        .as_ref()
        .map(|data| {
            if event.is_calendar_task() && event.details.recurring_event_id.is_some() {
                format!("{}/{}", data.id, event.id)
            } else {
                data.id.to_string()
            }
        })
        .unwrap_or_else(|| block_key(event))
}

pub fn block_key(event: &Event) -> String {
    format!("{}/{}", event.calendar_id, event.id)
}

/// The series a recurring block occurrence belongs to, as `calendar/series`.
pub fn block_series(block: &Event) -> Option<String> {
    block
        .details
        .recurring_event_id
        .as_ref()
        .map(|series| format!("{}/{series}", block.calendar_id))
}

/// Google copies a series' metadata to every occurrence, so each occurrence
/// of a task repeating with a block series still names the block it was
/// first put in. Point it at the occurrence of the block on its own day, or
/// at none when that day has no such block.
///
/// Editing a block "this and following" splits its series; the new series
/// names the ones it continues, so their tasks follow it.
pub fn resolve_block_series(events: &mut [Event]) {
    let blocks: Vec<(Vec<String>, NaiveDate, String)> = events
        .iter()
        .filter(|event| !event.is_task())
        .filter_map(|block| {
            let mut series = block.details.split_from.clone();
            series.push(block_series(block)?);
            Some((series, block.start.date_naive(), block_key(block)))
        })
        .collect();
    for task in events
        .iter_mut()
        .filter(|event| event.is_task() && event.details.recurring_event_id.is_some())
    {
        let day = task.start.date_naive();
        let Some(data) = task.details.task_data.as_mut() else {
            continue;
        };
        let Some(series) = data.metadata.block_series.as_ref() else {
            continue;
        };
        data.metadata.block = blocks
            .iter()
            .find(|(s, d, _)| *d == day && s.contains(series))
            .map(|(_, _, key)| key.clone());
    }
}

pub fn is_block(event: &Event, settings: &CalendarSettings) -> bool {
    !event.is_task()
        && !event.details.habit
        && event.details.sleep.is_none()
        && !event.is_calendar_task()
        && !event.all_day
        && settings.app_calendar.as_deref() == Some(&event.calendar_id)
}

/// A task that still needs a time: unscheduled, or waiting on a schedule write.
pub fn in_backlog(event: &Event) -> bool {
    let meta = metadata(event);
    event.is_task() && !event.completed && event.task_ref().is_some() && meta.block.is_none()
}

/// A task whose date is the current day. An undated task is not.
pub fn on_today(event: &Event) -> bool {
    event.start.date_naive() == crate::clock::now().date_naive()
}

pub fn overdue(event: &Event) -> bool {
    past_due(event, crate::clock::now())
}

/// Whether an open task's explicit deadline has passed. A planned date or
/// start time is a scheduling choice, not a deadline.
pub fn past_due(event: &Event, now: chrono::DateTime<chrono::Local>) -> bool {
    event.is_task()
        && !event.completed
        && !metadata(event).ignored
        && metadata(event)
            .deadline
            .is_some_and(|deadline| deadline.is_past(now.naive_local()))
}

/// An active date-only task still awaiting a slot on `day`.
/// Past-due tasks belong in Overdue, even if they have no slot yet.
pub fn unslotted_on(
    event: &Event,
    day: chrono::NaiveDate,
    now: chrono::DateTime<chrono::Local>,
) -> bool {
    let meta = metadata(event);
    event.is_task()
        && !event.completed
        && !meta.ignored
        && event.all_day
        && event.start.date_naive() == day
        && meta.block.is_none()
        && !past_due(event, now)
}

/// A day task needs a slot unless its linked block is among the day's `events`.
pub fn needs_slot(event: &Event, events: &[Event]) -> bool {
    metadata(event).block.as_ref().is_none_or(|key| {
        !events
            .iter()
            .any(|block| !block.is_task() && block_key(block) == *key)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_in_the_past_is_not_a_deadline() {
        let now = crate::clock::now();
        let mut task = calendar::sample_snapshot().events.remove(0);
        task.id = "task:planned".into();
        task.calendar_id = "tasks:list".into();
        task.completed = false;
        task.details = Default::default();
        task.start = now - chrono::Duration::days(2);
        task.end = task.start + chrono::Duration::minutes(30);
        for all_day in [true, false] {
            task.all_day = all_day;
            assert!(!past_due(&task, now));
        }
        task.details.description = "".into();
        task.details
            .task_data
            .get_or_insert_with(|| crate::task_db::TaskData {
                id: 100,
                metadata: Default::default(),
            })
            .metadata
            .deadline = Some(
            crate::calendar::deadline::parse_input(&format!(
                "{}",
                now.date_naive().pred_opt().unwrap()
            ))
            .unwrap(),
        );
        assert!(past_due(&task, now));
        task.completed = true;
        assert!(!past_due(&task, now));
    }

    #[test]
    fn task_occurrences_belong_to_the_block_occurrence_on_their_day() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        let occurrence = |id: &str, days: i64, hour: u32| {
            let start = calendar::at_clock(day + chrono::Duration::days(days), hour, 0);
            Event {
                id: id.into(),
                calendar_id: "app".into(),
                title: id.into(),
                start,
                end: start + chrono::Duration::hours(1),
                all_day: false,
                color: calendar::colors::TASK,
                completed: false,
                details: calendar::Details {
                    recurring_event_id: Some(id.split('_').next().unwrap().into()),
                    ..Default::default()
                },
            }
        };
        let task = |id: &str, days: i64| {
            let mut task = occurrence(id, days, 10);
            task.details.task_data = Some(crate::task_db::TaskData {
                id: 123,
                metadata: Metadata {
                    block: Some("app/block_0".into()),
                    block_series: Some("app/block".into()),
                    minutes: 30,
                    ..Default::default()
                },
            });
            task
        };
        let mut unrelated = task("other_0", 1);
        unrelated
            .details
            .task_data
            .as_mut()
            .unwrap()
            .metadata
            .block_series = None;
        let mut events = vec![
            occurrence("block_0", 0, 10),
            occurrence("block_1", 1, 10),
            task("task_0", 0),
            task("task_1", 1),
            task("task_2", 2),
            unrelated,
        ];
        assert_eq!(block_series(&events[0]).as_deref(), Some("app/block"));
        resolve_block_series(&mut events);
        let block = |id: &str| metadata(events.iter().find(|event| event.id == id).unwrap()).block;
        assert_eq!(block("task_0").as_deref(), Some("app/block_0"));
        assert_eq!(block("task_1").as_deref(), Some("app/block_1"));
        // No block that day: the occurrence stands alone.
        assert_eq!(block("task_2"), None);
        assert_eq!(block("other_0").as_deref(), Some("app/block_0"));

        // Moving the block "this and following" from day 1 ends its series
        // and continues it as a new one; the task follows.
        events.retain(|event| event.id != "block_1");
        let mut split = occurrence("later_1", 1, 11);
        split.details.split_from = vec!["app/block".into()];
        events.push(split);
        resolve_block_series(&mut events);
        let block = |id: &str| metadata(events.iter().find(|event| event.id == id).unwrap()).block;
        assert_eq!(block("task_0").as_deref(), Some("app/block_0"));
        assert_eq!(block("task_1").as_deref(), Some("app/later_1"));
    }

    #[test]
    fn backlog_sections_default_to_someday_and_round_trip() {
        let old: Metadata = serde_json::from_str(r#"{"minutes":30}"#).unwrap();
        assert_eq!(old.backlog_section, BacklogSection::Someday);
        assert_eq!(Metadata::default().backlog_section, BacklogSection::Someday);
        for section in BacklogSection::ALL {
            let data = crate::task_db::TaskData {
                id: 123,
                metadata: Metadata {
                    backlog_section: section,
                    ..Default::default()
                },
            };
            let notes = calendar::task_metadata::write("Keep these notes", &data).unwrap();
            let read = calendar::task_metadata::read(&notes).unwrap().unwrap();
            assert_eq!(read, data);
            assert_eq!(calendar::task_metadata::prose(&notes), "Keep these notes");
        }
    }

    #[test]
    fn metadata_without_projects_or_labels_omits_them() {
        let json = serde_json::to_string(&Metadata::default()).unwrap();
        assert!(!json.contains("project"));
        assert!(!json.contains("labels"));
    }
}
