//! Resolve only Google identities and references, while jobs retain their types.
use super::{Event, Job, ids, task_edit};
use crate::timeblocking::Metadata;
use std::collections::HashSet;

pub(super) fn job(mut job: Job, aliases: &[(String, String, String)]) -> Result<Job, String> {
	let resolver = Resolver(aliases);
	match &mut job {
		Job::Edit {
			original, draft, ..
		} => {
			let source_calendar = original.calendar_id.clone();
			let destination_changed = draft.calendar_id != source_calendar;
			let mut calendar = source_calendar.clone();
			resolver.identity(&mut calendar, &mut draft.id)?;
			if !destination_changed {
				draft.calendar_id = calendar;
			}
			if let Some(series) = &mut draft.series_id {
				resolver.identity(&mut source_calendar.clone(), series)?;
			}
			resolver.event(original)?;
		}
		Job::Time { calendar, id, .. } | Job::Location { calendar, id, .. } => {
			resolver.identity(calendar, id)?
		}
		Job::Delete {
			calendar,
			id,
			with_tasks,
			..
		} => {
			resolver.identity(calendar, id)?;
			if let Some(block) = with_tasks {
				resolver.event(block)?;
			}
		}
		Job::Status { task, .. } | Job::DeleteTask { task, .. } => resolver.event(task)?,
		Job::TaskEdit { task, change, .. } => {
			resolver.event(task)?;
			resolver.change(change)?;
		}
		Job::Sleep { event, new } => {
			if *new {
				resolver.references(event)?;
			} else {
				resolver.event(event)?;
			}
		}
		Job::HabitCompleted { event } | Job::Reply { event, .. } => resolver.event(event)?,
		Job::CreateCalendarTask { task } => resolver.references(task)?,
		Job::CreateTask { task, metadata, .. } => {
			resolver.references(task)?;
			if let Some(metadata) = metadata {
				resolver.metadata(metadata)?;
			}
		}
		// Creation IDs, destinations, and habit library IDs are not references
		// to previously created Google items.
		Job::Create { .. } | Job::CreateHabit { .. } => {}
	}
	Ok(job)
}

/// Moves `calendar` and `id` on to where the item they name is now.
pub(super) fn identity(
	aliases: &[(String, String, String)],
	calendar: &mut String,
	id: &mut String,
) -> Result<(), String> {
	Resolver(aliases).identity(calendar, id)
}

struct Resolver<'a>(&'a [(String, String, String)]);

impl Resolver<'_> {
	fn identity(&self, calendar: &mut String, id: &mut String) -> Result<(), String> {
		let mut seen = HashSet::new();
		loop {
			let bare = ids::bare_task(id);
			let Some((_, remote, destination)) = self
				.0
				.iter()
				.find(|(local, remote, _)| local != remote && ids::bare_task(local) == bare)
			else {
				return Ok(());
			};
			if !seen.insert(bare.to_owned()) {
				return Err(format!("Cycle in Google ID mappings at {id}"));
			}
			*id = if destination.starts_with(ids::TASK_LIST) {
				ids::event_id(remote)
			} else {
				remote.clone()
			};
			*calendar = destination.clone();
		}
	}

	fn key(&self, key: &mut String) -> Result<(), String> {
		if let Some((calendar, id)) = key.rsplit_once('/') {
			let (mut calendar, mut id) = (calendar.to_owned(), id.to_owned());
			self.identity(&mut calendar, &mut id)?;
			*key = format!("{calendar}/{id}");
		}
		Ok(())
	}

	fn metadata(&self, metadata: &mut Metadata) -> Result<(), String> {
		for key in [&mut metadata.block, &mut metadata.block_series]
			.into_iter()
			.flatten()
		{
			self.key(key)?;
		}
		Ok(())
	}

	fn event(&self, event: &mut Event) -> Result<(), String> {
		self.references(event)?;
		self.identity(&mut event.calendar_id, &mut event.id)
	}

	fn references(&self, event: &mut Event) -> Result<(), String> {
		if let Some(series) = &mut event.details.recurring_event_id {
			self.identity(&mut event.calendar_id.clone(), series)?;
		}
		for key in &mut event.details.split_from {
			self.key(key)?;
		}
		if let Some(data) = &mut event.details.task_data {
			self.metadata(&mut data.metadata)?;
		}
		Ok(())
	}

	fn change(&self, change: &mut task_edit::Change) -> Result<(), String> {
		use task_edit::Change;
		match change {
			Change::Block(key, ..) => self.key(key)?,
			Change::Reorder(Some(key), ..) => self.key(key)?,
			Change::JoinBlockSeries {
				calendar, series, ..
			} => self.identity(calendar, series)?,
			Change::Color(_)
			| Change::Description(_)
			| Change::Title(_)
			| Change::Estimate(_)
			| Change::Projects(_)
			| Change::Labels(_)
			| Change::Priority(_)
			| Change::Location(_)
			| Change::Deadline(_)
			| Change::Backlog(_)
			| Change::Inbox
			| Change::BacklogSection(_)
			| Change::Triaged
			| Change::Unassign
			| Change::Day(_)
			| Change::ClearSchedule
			| Change::Reorder(None, ..)
			| Change::Time(..)
			| Change::Reflow(..) => {}
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::calendar::{EditDraft, SaveScope, sample_snapshot};

	fn aliases() -> Vec<(String, String, String)> {
		vec![("local".into(), "remote".into(), "destination".into())]
	}

	#[test]
	fn resolving_an_identity_preserves_location_text_that_matches_it() {
		let resolved = job(
			Job::Location {
				calendar: "source".into(),
				id: "local".into(),
				location: "local".into(),
			},
			&aliases(),
		)
		.unwrap();
		let Job::Location {
			calendar,
			id,
			location,
		} = resolved
		else {
			unreachable!()
		};
		assert_eq!(
			(calendar.as_str(), id.as_str(), location.as_str()),
			("destination", "remote", "local")
		);
	}

	#[test]
	fn edits_preserve_user_text_and_an_explicit_destination() {
		let mut event = sample_snapshot().events.remove(0);
		event.id = "local".into();
		event.calendar_id = "source".into();
		event.title = "local".into();
		event.details.description = "local".into();
		event.details.labels = vec!["local".into()];
		let mut draft = EditDraft::from_event(&event);
		draft.calendar_id = "chosen-calendar".into();
		let resolved = job(
			Job::Edit {
				original: Box::new(event),
				draft,
				scope: SaveScope::Current,
				description: "local".into(),
				color: Some("local".into()),
			},
			&aliases(),
		)
		.unwrap();
		let Job::Edit {
			original,
			draft,
			description,
			color,
			..
		} = resolved
		else {
			unreachable!()
		};
		assert_eq!(original.id, "remote");
		assert_eq!(original.calendar_id, "destination");
		assert_eq!(original.title, "local");
		assert_eq!(original.details.description, "local");
		assert_eq!(original.details.labels, ["local"]);
		assert_eq!(draft.id, "remote");
		assert_eq!(draft.calendar_id, "chosen-calendar");
		assert_eq!(draft.title, "local");
		assert_eq!(description, "local");
		assert_eq!(color.as_deref(), Some("local"));
	}

	#[test]
	fn chains_are_order_independent_and_keep_task_prefixes() {
		let aliases = vec![
			("middle".into(), "last".into(), "tasks:final-list".into()),
			("local".into(), "middle".into(), "calendar".into()),
		];
		let mut task = sample_snapshot().events.remove(0);
		task.id = "task:local".into();
		task.calendar_id = "tasks:source".into();
		let resolved = job(
			Job::TaskEdit {
				task,
				change: task_edit::Change::Title("local".into()),
				order: 0,
				scheduled_as: None,
			},
			&aliases,
		)
		.unwrap();
		let Job::TaskEdit {
			task,
			change: task_edit::Change::Title(title),
			..
		} = resolved
		else {
			unreachable!()
		};
		assert_eq!(task.id, "task:last");
		assert_eq!(task.calendar_id, "tasks:final-list");
		assert_eq!(title, "local");
	}

	#[test]
	fn cycles_fail_but_self_mappings_are_terminal() {
		let location = || Job::Location {
			calendar: "cal".into(),
			id: "local".into(),
			location: "text".into(),
		};
		let aliases = vec![
			("local".into(), "other".into(), "cal".into()),
			("other".into(), "local".into(), "cal".into()),
		];
		assert!(job(location(), &aliases).err().unwrap().contains("Cycle"));
		assert!(
			job(
				location(),
				&[("local".into(), "local".into(), "cal".into())]
			)
			.is_ok()
		);
	}

	#[test]
	fn block_references_are_resolved_without_changing_library_ids_or_labels() {
		let mut task = sample_snapshot().events.remove(0);
		task.details.task_data = Some(crate::task_db::TaskData {
			id: 123,
			metadata: Metadata {
				block: Some("source/local".into()),
				block_series: Some("source/local".into()),
				habit_id: Some("local".into()),
				labels: vec!["local".into()],
				..Default::default()
			},
		});
		let start = task.start;
		let resolved = job(
			Job::TaskEdit {
				task,
				change: task_edit::Change::Block("source/local".into(), start, start),
				order: 0,
				scheduled_as: None,
			},
			&aliases(),
		)
		.unwrap();
		let Job::TaskEdit {
			task,
			change: task_edit::Change::Block(key, ..),
			..
		} = resolved
		else {
			unreachable!()
		};
		let data = task.details.task_data.unwrap();
		assert_eq!(data.metadata.block.as_deref(), Some("destination/remote"));
		assert_eq!(
			data.metadata.block_series.as_deref(),
			Some("destination/remote")
		);
		assert_eq!(data.metadata.habit_id.as_deref(), Some("local"));
		assert_eq!(data.metadata.labels, ["local"]);
		assert_eq!(key, "destination/remote");
	}

	#[test]
	fn creation_keeps_its_new_identity_and_resolves_only_existing_references() {
		let mut task = sample_snapshot().events.remove(0);
		task.id = "local".into();
		task.calendar_id = "chosen".into();
		task.title = "local".into();
		task.details.task_data = Some(crate::task_db::TaskData {
			id: 123,
			metadata: Metadata {
				block: Some("source/local".into()),
				..Default::default()
			},
		});
		let resolved = job(
			Job::CreateCalendarTask {
				task: Box::new(task),
			},
			&aliases(),
		)
		.unwrap();
		let Job::CreateCalendarTask { task } = resolved else {
			unreachable!()
		};
		assert_eq!(task.id, "local");
		assert_eq!(task.calendar_id, "chosen");
		assert_eq!(task.title, "local");
		assert_eq!(
			task.details.task_data.unwrap().metadata.block.as_deref(),
			Some("destination/remote")
		);
	}

	#[test]
	fn reorder_and_join_series_resolve_their_explicit_references() {
		let aliases = aliases();
		let resolver = Resolver(&aliases);
		let start = sample_snapshot().events[0].start;
		let mut reorder =
			task_edit::Change::Reorder(Some("source/local".into()), 4, start.date_naive());
		resolver.change(&mut reorder).unwrap();
		let task_edit::Change::Reorder(Some(key), order, day) = reorder else {
			unreachable!()
		};
		assert_eq!(key, "destination/remote");
		assert_eq!(order, 4);
		assert_eq!(day, start.date_naive());
		let mut join = task_edit::Change::JoinBlockSeries {
			calendar: "source".into(),
			series: "local".into(),
			from: start,
		};
		resolver.change(&mut join).unwrap();
		let task_edit::Change::JoinBlockSeries {
			calendar,
			series,
			from,
		} = join
		else {
			unreachable!()
		};
		assert_eq!(
			(calendar.as_str(), series.as_str()),
			("destination", "remote")
		);
		assert_eq!(from, start);
	}
}
