//! Task metadata and the pure time-block rules.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::calendar::{self, Event};
use crate::config::CalendarSettings;

pub mod membership;
pub mod reconcile;
pub use membership::{
	block_of, can_repeat_with, holding_blocks, in_block, is_assigned, is_follower, layout, lineage,
	original_start, repeats_with_block, tasks_in,
};

pub const UNDATED: NaiveDate = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub color: Option<String>,
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
	/// Creation instant recorded by TimeParagon; absent for tasks created elsewhere.
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
	event.is_task() && !event.completed && event.task_ref().is_some() && !is_assigned(event)
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

/// Whether a task, from Google Tasks or on the calendar, is unfinished: not
/// completed or ignored, and either past its deadline or planned for a day
/// before the current one. Habits planned as tasks have their own tracking,
/// so they are left out.
pub fn unfinished(
	event: &Event,
	now: chrono::DateTime<chrono::Local>,
	day_start_hour: u32,
) -> bool {
	if !event.is_task()
		|| event.completed
		|| metadata(event).ignored
		|| metadata(event).habit_id.is_some()
	{
		return false;
	}
	if past_due(event, now) {
		return true;
	}
	let date = event.start.date_naive();
	if date == UNDATED {
		return false;
	}
	let day = if event.all_day {
		date
	} else {
		calendar::day_at(event.start, day_start_hour)
	};
	day < calendar::day_at(now, day_start_hour)
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
		&& !is_assigned(event)
		&& !past_due(event, now)
}

/// A day task needs a slot unless the block it is in is among the day's
/// `events`.
pub fn needs_slot(event: &Event, events: &[Event], day_start_hour: u32) -> bool {
	block_of(event, events, day_start_hour).is_none()
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
