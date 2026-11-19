//! Desktop recurring task and habit membership in time blocks.
use super::{
	PendingJoin, PendingRecurring, RecurringItem, add_block_item, block_repeats, day_start_hour,
	event, save,
};
use crate::app::{Message, State};
use crate::calendar::{self, Event};
use crate::{blocking, task_operations::BusyKey};
use chrono::NaiveDate;
use iced::Task;
use timeparagon_core::task_edit::Change;
use timeparagon_core::timeblocking::{
	Metadata, can_repeat_with, is_block, is_follower, membership, metadata,
};

#[derive(Debug, Clone)]
pub enum Action {
	/// Move the recurring item being confirmed into its block. `true` also
	/// deletes the item's future occurrences.
	MoveRecurring(bool),
	CancelRecurring,
	/// Add to a repeating block's occurrence only (`true`), or repeat with
	/// the block (`false`).
	OneOff(bool),
	/// Take a task repeating with its block out of it from this occurrence
	/// on: this occurrence and the later ones are deleted.
	StopRepeating(String),
	/// Have a one-off task or habit (first) in a repeating block (second)
	/// repeat with the block from this occurrence on.
	RepeatWithBlock(String, String),
	/// Go ahead with the habit waiting in [`Workspace::pending_join`].
	ConfirmRepeatWithBlock,
	CancelRepeatWithBlock,
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
	match action {
		Action::CancelRecurring => {
			state.timeblocking.pending_recurring = None;
			return Task::none();
		}
		Action::OneOff(one_off) => {
			state.timeblocking.one_off = one_off;
			return Task::none();
		}
		Action::StopRepeating(id) => return stop_repeating(state, &id),
		Action::RepeatWithBlock(task, block) => {
			return repeat_with_block(state, &task, &block, false);
		}
		Action::ConfirmRepeatWithBlock => {
			let Some(PendingJoin { task, block }) = state.timeblocking.pending_join.take() else {
				return Task::none();
			};
			return repeat_with_block(state, &task, &block, true);
		}
		Action::CancelRepeatWithBlock => {
			state.timeblocking.pending_join = None;
			return Task::none();
		}
		Action::MoveRecurring(future) => return move_recurring(state, future),
	}
}

/// Moves the recurring task or habit awaiting confirmation into its block.
/// Either way the block gets a new task and the occurrence it replaces is
/// deleted. With `future` the ones after it are deleted too, and the new
/// task repeats with the block when the block repeats; otherwise it is a
/// one-off in this occurrence.
pub(super) fn move_recurring(state: &mut State, future: bool) -> Task<Message> {
	if !state.task_operations.busy.is_empty() {
		return Task::none();
	}
	let Some(PendingRecurring { item, block }) = state.timeblocking.pending_recurring.take() else {
		return Task::none();
	};
	let Some((day, repeats)) = event(state, &block)
		.filter(|event| is_block(event, &state.config.calendar))
		.map(|event| (event.start.date_naive(), block_repeats(event)))
	else {
		return Task::none();
	};
	let repeat = future && repeats;
	let scope = if future {
		calendar::SaveScope::CurrentAndFuture
	} else {
		calendar::SaveScope::Current
	};
	match item {
		RecurringItem::Task(id) => {
			let Some(task) = event(state, &id).filter(|task| task.is_task()).cloned() else {
				return Task::none();
			};
			let mut copy = metadata(&task);
			copy.block = None;
			copy.block_series = None;
			copy.repeat_source = None;
			copy.created_at = None;
			add_block_item(
				state,
				&block,
				task.title.clone(),
				calendar::task_metadata::prose(&task.details.description).to_owned(),
				Some(copy),
				vec![(task.calendar_id, task.id, scope)],
				repeat,
			)
		}
		RecurringItem::Habit(habit_id) => {
			let Some(habit) = state.habits.library.habit(&habit_id).cloned() else {
				return Task::none();
			};
			let deletions = habit_occurrences(state, &habit_id, day, future)
				.into_iter()
				.map(|event| (event.calendar_id.clone(), event.id.clone(), scope))
				.collect();
			add_block_item(
				state,
				&block,
				habit.name,
				habit.notes,
				Some(Metadata {
					habit_id: Some(habit_id),
					minutes: 30,
					..Default::default()
				}),
				deletions,
				repeat,
			)
		}
	}
}

/// Ends a task repeating with its block at this occurrence: it and the
/// later ones are deleted, the earlier ones stay as they were.
pub(super) fn stop_repeating(state: &mut State, id: &str) -> Task<Message> {
	if !state.task_operations.busy.is_empty() {
		return Task::none();
	}
	let Some(task) = event(state, id).filter(|task| is_follower(task)).cloned() else {
		return Task::none();
	};
	let scope = calendar::SaveScope::CurrentAndFuture;
	// The occurrences deleted stay in view, locked, until Google confirms
	// (see `Overlay::apply`), as for any other delete.
	state.task_operations.revision += 1;
	crate::task_operations::begin(state, task.id.clone());
	let key = BusyKey::from(task.id.clone());
	blocking::run(move || {
		calendar::delete_event_scope(&task.calendar_id, &task.id, scope, false)?;
		Ok(Vec::new())
	})
	.map(move |result| {
		Message::TaskOperations(crate::task_operations::Action::SavedMany(
			key.clone(),
			result,
		))
	})
}

/// Has a one-off task or habit in a repeating block repeat with the block
/// from this occurrence on. A habit that also repeats on its own is asked
/// about first: once `confirmed`, its own repeat ends from this day, so it
/// is not on the calendar twice.
pub(super) fn repeat_with_block(
	state: &mut State,
	task_id: &str,
	block_id: &str,
	confirmed: bool,
) -> Task<Message> {
	if !state.task_operations.busy.is_empty() {
		return Task::none();
	}
	let hour = day_start_hour(state);
	let Some(block) = event(state, block_id)
		.filter(|block| is_block(block, &state.config.calendar))
		.cloned()
	else {
		return Task::none();
	};
	let Some(task) = event(state, task_id)
		.filter(|task| can_repeat_with(task, &block, hour))
		.cloned()
	else {
		return Task::none();
	};
	let Some(join) = Change::join_block_series(&block) else {
		return Task::none();
	};
	let habit = metadata(&task).habit_id;
	if !confirmed
		&& let Some(habit) = &habit
		&& repeats_on_its_own(state, habit)
	{
		state.timeblocking.pending_join = Some(PendingJoin {
			task: task.id.clone(),
			block: block.id.clone(),
		});
		return Task::none();
	}
	let ended = habit.filter(|_| confirmed);
	let from = calendar::at_clock(membership::planning_day(block.start, hour), hour.min(23), 0);
	let settings = state.config.calendar.clone();
	let order = metadata(&task).order;
	state.task_operations.revision += 1;
	crate::task_operations::begin(state, task.id.clone());
	let key = BusyKey::from(task.id.clone());
	blocking::run(move || {
		// Found before anything is written, so failing to look leaves the
		// habit as it was rather than on the calendar twice.
		let ended = match ended {
			Some(habit) => calendar::habit_repeats_from(&settings, &habit, from)?,
			None => Vec::new(),
		};
		let saved = save(task, &settings, join, order)?;
		for (calendar_id, occurrence) in ended {
			calendar::delete_event_scope(
				&calendar_id,
				&occurrence,
				calendar::SaveScope::CurrentAndFuture,
				false,
			)?;
		}
		Ok(vec![saved])
	})
	.map(move |result| {
		Message::TaskOperations(crate::task_operations::Action::SavedMany(
			key.clone(),
			result,
		))
	})
}

/// Whether the habit has a repeating event of its own on the calendar.
pub fn repeats_on_its_own(state: &State, habit_id: &str) -> bool {
	state.habits.series.iter().any(|series| {
		series.habit_id.as_deref() == Some(habit_id) && !series.recurrence.lines().is_empty()
	})
}

/// Per repeating event of the habit, the occurrence a block on `day` takes
/// the place of: the one that day, or with `future` the first from then on.
fn habit_occurrences<'a>(
	state: &'a State,
	habit_id: &str,
	day: NaiveDate,
	future: bool,
) -> Vec<&'a Event> {
	let mut chosen: Vec<&Event> = Vec::new();
	let mut occurrences: Vec<_> = state
		.calendar
		.events
		.iter()
		.filter(|event| {
			event.details.habit
				&& event.details.recurring_event_id.is_some()
				&& event.details.habit_id.as_deref() == Some(habit_id)
				&& if future {
					event.start.date_naive() >= day
				} else {
					event.start.date_naive() == day
				}
		})
		.collect();
	occurrences.sort_by_key(|event| event.start);
	for event in occurrences {
		if !chosen.iter().any(|other| {
			other.calendar_id == event.calendar_id
				&& other.details.recurring_event_id == event.details.recurring_event_id
		}) {
			chosen.push(event);
		}
	}
	chosen
}

/// Drops a deleted occurrence, and with "this and following" the later ones
/// of its series, from view. The next sync reconciles the cache.
pub(super) fn forget_occurrences(
	state: &mut State,
	calendar_id: &str,
	id: &str,
	scope: calendar::SaveScope,
) {
	let Some(occurrence) = state
		.calendar
		.events
		.iter()
		.find(|event| event.id == id && event.calendar_id == calendar_id)
		.cloned()
	else {
		return;
	};
	let gone: Vec<String> = state
		.calendar
		.events
		.iter()
		.filter(|event| {
			event.id == id
				|| (scope == calendar::SaveScope::CurrentAndFuture
					&& event.calendar_id == calendar_id
					&& event.details.recurring_event_id.is_some()
					&& event.details.recurring_event_id == occurrence.details.recurring_event_id
					&& event.start >= occurrence.start)
		})
		.map(|event| event.id.clone())
		.collect();
	for id in gone {
		if state.range_view_active() {
			state.calendar.forget_event(&id);
		} else {
			state.calendar.events.retain(|event| event.id != id);
		}
	}
}
