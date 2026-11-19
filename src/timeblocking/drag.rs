//! Desktop drag state and conversion of drop targets into planning actions.
use super::{WeekListPlacement, day_start_hour, event, weekly_list_events};
use crate::app::{Message, State};
use crate::calendar::{self, Event};
use crate::timeblocking::update as plan;
use chrono::{DateTime, Local, NaiveDate};
use iced::Task;
use timeparagon_core::task_edit::Change;
use timeparagon_core::timeblocking::BacklogSection;
use timeparagon_core::timeblocking::{
	block_key, block_of, in_block, is_assigned, metadata, tasks_in,
};

#[derive(Debug, Clone)]
pub enum Action {
	DropTime(DateTime<Local>),
	CancelDrag,
	/// Drop the dragged task into a backlog planning horizon.
	DropBacklogSection(BacklogSection),
	Drag(String, iced::Point),
	/// The pointer moved while a task is being dragged, so the floating card
	/// can follow it.
	DragMoved(iced::Point),
	DropBlock(String),
	DropTask(String),
	/// A task row in the calendar, independent of the selected sidebar tab.
	DropCalendarTask(String),
	DropDay,
	/// Drop between two rows, beginning right after the timed row
	/// above, or ending as the timed row below starts. With no timed row on
	/// either side it goes on the day as a whole, as by [`Action::DropDay`].
	DropAt {
		start: Option<DateTime<Local>>,
	},
	/// Insert before/after a board card, or at the end of an empty/day column.
	DropWeekList {
		target: Option<String>,
		after: bool,
	},
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
	let (id, change) = match action {
		Action::DropWeekList { target, after } => return drop_week_list(state, target, after),
		Action::DropAt { start } => {
			return plan(
				state,
				start.map_or(super::Action::DropDay, super::Action::DropTime),
			);
		}
		Action::DropTime(start) => {
			let Some(id) = take_drag(state) else {
				return Task::none();
			};
			let minutes = event(state, &id).map(|t| metadata(t).minutes).unwrap_or(0);
			let minutes = if minutes == 0 { 30 } else { minutes };
			return plan(
				state,
				super::Action::Schedule(
					id,
					start,
					start + chrono::Duration::minutes(i64::from(minutes)),
				),
			);
		}
		Action::CancelDrag => {
			take_drag(state);
			return Task::none();
		}
		Action::Drag(id, position) => {
			state.timeblocking.drag_window = None;
			state.timeblocking.drop_hover = None;
			state.timeblocking.dragging = Some(id);
			state.timeblocking.drag_pointer = Some(position);
			return Task::none();
		}
		Action::DragMoved(position) => {
			if state.timeblocking.dragging.is_some() {
				state.timeblocking.drag_pointer = Some(position);
			}
			return Task::none();
		}
		Action::DropBlock(block) => {
			if let Some(id) = take_drag(state) {
				return plan(state, super::Action::Assign(id, block));
			}
			return Task::none();
		}
		Action::DropBacklogSection(section) => {
			let Some(id) = take_drag(state) else {
				return Task::none();
			};
			state.timeblocking.drag_pointer = None;
			return plan(state, super::Action::SetBacklogSection(id, section));
		}
		Action::DropCalendarTask(target) => {
			let block = event(state, &target)
				.and_then(|task| block_of(task, &state.calendar.events, day_start_hour(state)))
				.map(|block| block.id.clone());
			return plan(
				state,
				block.map_or(super::Action::DropDay, super::Action::DropBlock),
			);
		}
		Action::DropTask(target) => {
			if state.screen == crate::app::Screen::Backlog {
				let Some(task) = event(state, &target) else {
					return Task::none();
				};
				return plan(
					state,
					super::Action::DropBacklogSection(metadata(task).backlog_section),
				);
			}
			let Some(id) = take_drag(state) else {
				return Task::none();
			};
			if id == target {
				return Task::none();
			}
			let Some(target) = event(state, &target) else {
				return Task::none();
			};
			let m = metadata(target);
			let hour = day_start_hour(state);
			let block = block_of(target, &state.calendar.events, hour);
			// Into another block: assigned there, as by dropping on the block.
			if let Some(block) = block
				&& let Some(dragged) = event(state, &id)
				&& !in_block(dragged, block, hour)
			{
				let block = block.id.clone();
				return plan(state, super::Action::Assign(id, block));
			}
			let siblings: Vec<&Event> = match block {
				Some(block) => tasks_in(block, &state.calendar.events, hour),
				None => state
					.calendar
					.events
					.iter()
					.filter(|e| e.is_task() && !is_assigned(e))
					.collect(),
			};
			let previous = siblings
				.into_iter()
				.filter(|e| e.id != id && e.id != target.id)
				.map(metadata)
				.filter(|other| other.order < m.order)
				.map(|other| other.order)
				.max();
			let rank = previous.map_or(m.order.saturating_sub(1_i64 << 32), |previous| {
				(i128::from(previous) + (i128::from(m.order) - i128::from(previous)) / 2) as i64
			});
			let key = block.map(block_key);
			(id, Change::Reorder(key, rank, target.start.date_naive()))
		}
		Action::DropDay => {
			let Some(id) = take_drag(state) else {
				return Task::none();
			};
			(id, Change::Day(state.calendar.day))
		}
	};
	crate::task_operations::edit(state, id, change, None)
}

pub(super) fn take_drag(state: &mut State) -> Option<String> {
	state.timeblocking.drag_pointer = None;
	state.timeblocking.drop_hover = None;
	state.timeblocking.drag_window = None;
	state.timeblocking.dragging.take()
}

pub(crate) fn place_week_card(
	state: &mut State,
	id: &str,
	day: NaiveDate,
	target: Option<&str>,
	after: bool,
) {
	let mut order: Vec<_> = weekly_list_events(state, day)
		.iter()
		.filter(|event| event.id != id)
		.map(|event| event.id.clone())
		.collect();
	let index = target
		.and_then(|target| order.iter().position(|id| id == target))
		.map_or(order.len(), |index| index + usize::from(after));
	order.insert(index, id.to_owned());
	for other in state.config.calendar.weekly_task_order.values_mut() {
		other.retain(|other| other != id);
	}
	state
		.config
		.calendar
		.weekly_task_order
		.insert(day.to_string(), order);
	let _ = crate::app::queue_save(state);
}

fn drop_week_list(state: &mut State, target: Option<String>, after: bool) -> Task<Message> {
	let Some(id) = take_drag(state) else {
		return Task::none();
	};
	if !state.task_operations.busy.is_empty() || target.as_deref() == Some(&id) {
		return Task::none();
	}
	let day = state.calendar.day;
	if target
		.as_ref()
		.is_some_and(|id| !weekly_list_events(state, day).iter().any(|e| &e.id == id))
	{
		return Task::none();
	}
	let Some(task) = event(state, &id).filter(|task| task.is_task()).cloned() else {
		return Task::none();
	};
	if !task.all_day && calendar::day_at(task.start, state.config.calendar.day_start_hour()) == day
	{
		place_week_card(state, &id, day, target.as_deref(), after);
		return Task::none();
	}
	let Some((start, end)) = week_drop_times(&task, day, state.config.calendar.day_start_hour())
	else {
		state.calendar.error = Some("This time does not exist on the destination day".into());
		return Task::none();
	};
	state.task_operations.pending_week_drop = Some(WeekListPlacement {
		task: id.clone(),
		day,
		target,
		after,
	});
	plan(state, super::Action::Schedule(id, start, end))
}

/// Keep local clock time and elapsed duration when crossing days, including
/// the small hours assigned to the previous planning day.
pub(crate) fn week_drop_times(
	task: &Event,
	day: NaiveDate,
	boundary: u32,
) -> Option<(DateTime<Local>, DateTime<Local>)> {
	let (start, duration) = if task.all_day {
		let minutes = metadata(task).minutes;
		(
			calendar::at_clock(day, 9, 0),
			chrono::Duration::minutes(i64::from(if minutes == 0 { 30 } else { minutes })),
		)
	} else {
		let source_day = calendar::day_at(task.start, boundary);
		let offset = task.start.date_naive().signed_duration_since(source_day);
		let date = day.checked_add_signed(offset)?;
		(
			date.and_time(task.start.time())
				.and_local_timezone(Local)
				.earliest()?,
			task.end - task.start,
		)
	};
	Some((start, start + duration))
}
