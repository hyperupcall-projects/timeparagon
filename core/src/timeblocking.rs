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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited: Option<Box<InheritedMetadata>>,
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

/// Metadata owned by a time block and inherited by its assigned tasks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChildMetadata {
    pub force: bool,
    pub due: Option<calendar::deadline::Deadline>,
    pub deadline: Option<calendar::deadline::Deadline>,
    pub priority: Option<Priority>,
    pub location: Option<u64>,
}

/// The complete inherited snapshot also records repeat rules and event settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InheritedMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    pub fields: ChildMetadata,
    pub projects: Vec<u64>,
    pub labels: Vec<String>,
    pub recurrence: Vec<String>,
    pub options: calendar::event_options::EventOptions,
}

pub fn inherited_metadata(block: &Event) -> InheritedMetadata {
    let mut options = block.details.options.clone();
    options.child_metadata = ChildMetadata::default();
    InheritedMetadata {
        series: if !block.details.recurrence.is_empty()
            || block.details.recurring_event_id.is_some()
        {
            Some(format!(
                "{}/{}",
                block.calendar_id,
                block
                    .details
                    .recurring_event_id
                    .as_deref()
                    .unwrap_or(&block.id)
            ))
        } else {
            None
        },
        fields: block.details.options.child_metadata.clone(),
        projects: block.details.projects.clone(),
        labels: block.details.labels.clone(),
        recurrence: block.details.recurrence.clone(),
        options,
    }
}

/// Identity, membership, estimate, work history and completion belong to the child.
pub fn apply_inherited(
    task: &mut Event,
    data: &mut crate::task_db::TaskData,
    inherited: InheritedMetadata,
) {
    data.metadata.projects = inherited.projects.clone();
    data.metadata.labels = inherited.labels.clone();
    data.metadata.priority = inherited.fields.priority;
    data.metadata.location = inherited.fields.location;
    data.metadata.deadline = inherited.fields.deadline;
    task.details.recurrence = inherited.recurrence.clone();
    task.details.options = inherited.options.clone();
    data.metadata.inherited = Some(Box::new(inherited));
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
    fn forced_metadata_replaces_old_values_and_keeps_child_identity() {
        let start = calendar::at_clock(NaiveDate::from_ymd_opt(2026, 11, 8).unwrap(), 10, 0);
        let mut parent = Event {
            id: "parent".into(),
            calendar_id: "app".into(),
            title: "Block".into(),
            start,
            end: start + chrono::Duration::hours(1),
            all_day: false,
            color: calendar::colors::TASK,
            completed: false,
            details: Default::default(),
        };
        parent.details.projects = vec![7];
        parent.details.labels = vec!["work".into()];
        parent.details.recurrence = vec!["RRULE:FREQ=WEEKLY".into()];
        parent.details.options.child_metadata = ChildMetadata {
            force: true,
            priority: Some(Priority::P1),
            location: Some(3),
            due: Some(calendar::deadline::parse_input("2026-11-10").unwrap()),
            deadline: Some(calendar::deadline::parse_input("2026-11-12 17:00").unwrap()),
        };
        let mut child = parent.clone();
        child.id = "task:child".into();
        child.title = "Child".into();
        child.completed = true;
        let mut data = crate::task_db::TaskData {
            id: 123,
            metadata: Metadata {
                block: Some(block_key(&parent)),
                minutes: 25,
                order: 99,
                projects: vec![1],
                labels: vec!["old".into()],
                priority: Some(Priority::P2),
                ..Default::default()
            },
        };
        child.details.description = "Child notes".into();
        let inherited = inherited_metadata(&parent);
        apply_inherited(&mut child, &mut data, inherited.clone());
        child.details.task_data = Some(data.clone());
        assert_eq!(child.id, "task:child");
        assert_eq!(child.title, "Child");
        assert!(child.completed);
        assert_eq!(child.start, start);
        assert_eq!(data.id, 123);
        assert_eq!(data.metadata.minutes, 25);
        assert_eq!(data.metadata.order, 99);

        assert_eq!(data.metadata.projects, [7]);
        assert_eq!(data.metadata.labels, ["work"]);
        assert_eq!(data.metadata.priority, Some(Priority::P1));
        assert_eq!(data.metadata.location, Some(3));
        let notes = calendar::task_metadata::write(&child.details.description, &data).unwrap();
        let read = calendar::task_metadata::read(&notes).unwrap().unwrap();
        assert_eq!(read.metadata.inherited.as_deref(), Some(&inherited));
        assert_eq!(read.metadata.deadline, inherited.fields.deadline);
        assert_eq!(calendar::task_metadata::prose(&notes), "Child notes");
        assert!(
            crate::task_edit::validate_child_edit(
                &child,
                &crate::task_edit::Change::Priority(None)
            )
            .is_err()
        );
        assert!(
            crate::task_edit::validate_child_edit(&child, &crate::task_edit::Change::Estimate(10))
                .is_ok()
        );
        // Clearing a parent value clears it on the child too.
        parent.details.projects.clear();
        parent.details.labels.clear();
        parent.details.recurrence.clear();
        parent.details.options.child_metadata.priority = None;
        parent.details.options.child_metadata.deadline = None;
        apply_inherited(&mut child, &mut data, inherited_metadata(&parent));
        assert!(data.metadata.projects.is_empty());
        assert!(data.metadata.labels.is_empty());
        assert!(data.metadata.priority.is_none());
        assert!(child.details.recurrence.is_empty());
        assert_eq!(data.metadata.deadline, None);
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
