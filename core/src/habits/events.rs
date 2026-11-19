//! The connection between habit records and calendar or task completion.

use chrono::NaiveDate;

use super::{Habit, Kind, Library, Status};
use crate::calendar::Event;
use crate::timeblocking::{self, TaskStatus};

impl Library {
	/// Resolves a linked habit. An occurrence given a habit of its own (see
	/// [`super::OCCURRENCE_LINK_PROPERTY`]) counts towards that. Otherwise a
	/// known series overrides the occurrence's old properties, including when
	/// the series was explicitly unlinked. Planned tasks use their task
	/// metadata instead.
	pub fn habit_of<'a>(
		&self,
		event: &'a Event,
		series: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
	) -> Option<&'a str> {
		let link = if event.is_task() {
			event.linked_habit_id()
		} else if let Some(own) = event.occurrence_habit() {
			own
		} else {
			let series_id = event
				.details
				.recurring_event_id
				.as_deref()
				.unwrap_or(&event.id);
			series
				.into_iter()
				.find(|(id, _)| *id == series_id)
				.map_or(event.details.habit_id.as_deref(), |(_, link)| link)
		};
		link.filter(|id| self.habit(id).is_some())
	}

	/// Records one completion transition. Callers invoke this only after a
	/// successful write that changed completion, so retries cannot count twice.
	/// Ignored tasks undo a repetition, and habits to avoid are left alone.
	pub fn record_completion(&mut self, id: &str, event: &Event) -> bool {
		let Some(habit) = self.habit_mut(id).filter(|habit| habit.kind == Kind::Yes) else {
			return false;
		};
		let completed = if event.is_task() {
			timeblocking::task_status(event) == TaskStatus::Completed
		} else {
			event.completed
		};
		if completed {
			habit.add_rep(event.start.date_naive());
		} else {
			habit.remove_rep(event.start.date_naive());
		}
		true
	}
}

impl Habit {
	/// Brings loaded occurrences linked to this habit into line with its record.
	/// Completes the earliest open occurrences and reopens the latest completed
	/// ones. Callers supply occurrences in display order, excluding pending writes.
	pub fn occurrence_changes<'a>(
		&self,
		day: NaiveDate,
		today: NaiveDate,
		occurrences: impl IntoIterator<Item = &'a Event>,
	) -> Vec<Event> {
		if self.kind != Kind::Yes {
			return Vec::new();
		}
		let wanted = match self.compute(today).status(day) {
			Status::Yes | Status::Partial(_) => self.times_on(day) as usize,
			_ => 0,
		};
		let occurrences: Vec<_> = occurrences
			.into_iter()
			.filter(|event| event.details.habit && event.start.date_naive() == day)
			.collect();
		let done = occurrences.iter().filter(|event| event.completed).count();
		let changes: Vec<_> = if done < wanted {
			occurrences
				.into_iter()
				.filter(|event| !event.completed)
				.take(wanted - done)
				.collect()
		} else {
			occurrences
				.into_iter()
				.rev()
				.filter(|event| event.completed)
				.take(done - wanted)
				.collect()
		};
		changes
			.into_iter()
			.map(|event| {
				let mut event = event.clone();
				event.completed = done < wanted;
				event.details.habit_completed_at = event.completed.then(crate::clock::now);
				event
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::calendar::{self, Details};
	use crate::habits::{Frequency, Mark};
	use chrono::Duration;

	fn day() -> NaiveDate {
		NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
	}

	fn event(id: &str, completed: bool) -> Event {
		let start = calendar::at_clock(day(), 9, 0);
		Event {
			id: id.into(),
			calendar_id: "calendar".into(),
			title: "Run".into(),
			start,
			end: start + Duration::minutes(30),
			all_day: false,
			color: calendar::colors::DEFAULT_EVENT,
			completed,
			details: Details {
				habit: true,
				habit_id: Some("run".into()),
				..Default::default()
			},
		}
	}

	fn library() -> Library {
		Library {
			habits: vec![Habit::new("run", "Run"), Habit::new("read", "Read")],
			..Default::default()
		}
	}

	#[test]
	fn series_links_override_stale_occurrences_even_when_removed_or_missing() {
		let library = library();
		let mut event = event("occurrence", true);
		event.details.recurring_event_id = Some("series".into());
		assert_eq!(library.habit_of(&event, []), Some("run"));
		assert_eq!(
			library.habit_of(&event, [("series", Some("read"))]),
			Some("read")
		);
		assert_eq!(library.habit_of(&event, [("series", None)]), None);
		assert_eq!(
			library.habit_of(&event, [("series", Some("deleted"))]),
			None
		);
		event.details.habit_id = Some("deleted".into());
		assert_eq!(library.habit_of(&event, []), None);
	}

	#[test]
	fn a_habit_saved_for_this_event_only_outranks_its_series() {
		let library = library();
		let mut event = event("occurrence", true);
		event.details.recurring_event_id = Some("series".into());
		let series = [("series", Some("run"))];
		event.details.habit_override = Some("read".into());
		assert_eq!(library.habit_of(&event, series), Some("read"));
		assert_eq!(event.linked_habit_id(), Some("read"));
		event.details.habit_override = Some(String::new());
		assert_eq!(library.habit_of(&event, series), None);
		assert_eq!(event.linked_habit_id(), None);
		// Only an occurrence has a series to differ from.
		event.details.recurring_event_id = None;
		assert_eq!(library.habit_of(&event, []), Some("run"));
	}

	#[test]
	fn planned_tasks_use_their_metadata_and_ignored_tasks_undo_completion() {
		let mut library = library();
		let mut task = event("task:run", false);
		task.details.task_data = Some(crate::task_db::TaskData {
			id: 1,
			metadata: timeblocking::Metadata {
				habit_id: Some("read".into()),
				..Default::default()
			},
		});
		assert_eq!(
			library.habit_of(&task, [("task:run", Some("run"))]),
			Some("read")
		);
		for (status, count) in [
			(TaskStatus::Completed, 1),
			(TaskStatus::Ignored, 0),
			(TaskStatus::Active, 0),
		] {
			timeblocking::apply_status(&mut task, status);
			assert!(library.record_completion("read", &task));
			assert_eq!(library.habit("read").unwrap().times_on(day()), count);
		}
	}

	#[test]
	fn completing_and_reopening_occurrences_only_changes_habits_to_do() {
		let mut library = library();
		assert!(library.record_completion("run", &event("a", true)));
		assert!(library.record_completion("run", &event("b", true)));
		assert_eq!(library.habit("run").unwrap().times_on(day()), 2);
		assert!(library.record_completion("run", &event("a", false)));
		assert_eq!(library.habit("run").unwrap().times_on(day()), 1);
		library.habit_mut("run").unwrap().kind = Kind::No;
		let before = library.clone();
		assert!(!library.record_completion("run", &event("a", true)));
		assert!(!library.record_completion("missing", &event("a", true)));
		assert_eq!(library, before);
	}

	#[test]
	fn partial_days_complete_first_open_occurrences_and_reopen_last_done_ones() {
		let mut habit = Habit::new("run", "Run");
		habit.frequency = Frequency::TimesPerDay { times: 3 };
		habit.marks.insert(day(), Mark::Reps(2));
		let events = [event("a", true), event("b", false), event("c", false)];
		let changes = habit.occurrence_changes(day(), day(), &events);
		assert_eq!(changes.len(), 1);
		assert_eq!(changes[0].id, "b");
		assert!(changes[0].completed);
		assert!(changes[0].details.habit_completed_at.is_some());
		let events = [event("a", true), event("b", true), event("c", true)];
		let changes = habit.occurrence_changes(day(), day(), &events);
		assert_eq!(changes.len(), 1);
		assert_eq!(changes[0].id, "c");
		assert!(!changes[0].completed);
		assert_eq!(changes[0].details.habit_completed_at, None);
		assert!(
			habit
				.occurrence_changes(day(), day(), &events[..2])
				.is_empty()
		);
	}

	#[test]
	fn skipped_days_reopen_occurrences_but_avoidance_and_other_days_are_untouched() {
		let mut habit = Habit::new("run", "Run");
		habit.marks.insert(day(), Mark::Skip);
		let mut yesterday = event("yesterday", true);
		yesterday.start -= Duration::days(1);
		let mut ordinary = event("ordinary", true);
		ordinary.details.habit = false;
		let events = [event("today", true), yesterday, ordinary];
		let changes = habit.occurrence_changes(day(), day(), &events);
		assert_eq!(changes.len(), 1);
		assert_eq!(changes[0].id, "today");
		assert!(!changes[0].completed);
		habit.kind = Kind::No;
		assert!(habit.occurrence_changes(day(), day(), &events).is_empty());
	}
}
