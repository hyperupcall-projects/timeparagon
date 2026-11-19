//! Pending desktop task writes and application of their results.
use crate::app::{CalendarMessage, Message, State};
use crate::blocking;
use crate::calendar::Event;
use crate::timeblocking::{WeekListPlacement, event, join_series, place_week_card};
use iced::Task;
use std::collections::HashMap;
use timeparagon_core::task_edit::{self, Change};
use timeparagon_core::timeblocking::{is_block, metadata, task_key};

#[derive(Debug, Clone)]
pub enum Action {
	Saved(BusyKey, Result<Event, String>),
	SavedMany(BusyKey, Result<Vec<Event>, String>),
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
	match action {
		Action::Saved(key, result) => saved(state, key, result),
		Action::SavedMany(key, result) => saved_many(state, key, result),
	}
}

/// A save in progress, and what its result will be for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BusyKey {
	Task(String),
	/// A todo that does not have a task id until Google creates it.
	AddingTodo,
	AddingHabit,
}

impl BusyKey {
	/// The task this save is for, which matches no task while adding a todo.
	pub(crate) fn task_id(&self) -> &str {
		match self {
			Self::Task(id) => id,
			Self::AddingTodo | Self::AddingHabit => "",
		}
	}
}

impl From<String> for BusyKey {
	fn from(id: String) -> Self {
		Self::Task(id)
	}
}

impl From<&str> for BusyKey {
	fn from(id: &str) -> Self {
		Self::Task(id.to_owned())
	}
}

/// Which saves are in flight.
#[derive(Debug, Default)]
pub struct Busy {
	tasks: std::collections::HashSet<String>,
	adding_todo: bool,
	adding_habit: bool,
}

impl Busy {
	pub fn insert(&mut self, key: impl Into<BusyKey>) {
		match key.into() {
			BusyKey::Task(id) => {
				self.tasks.insert(id);
			}
			BusyKey::AddingTodo => self.adding_todo = true,
			BusyKey::AddingHabit => self.adding_habit = true,
		}
	}

	pub fn remove(&mut self, key: &BusyKey) {
		match key {
			BusyKey::Task(id) => {
				self.tasks.remove(id);
			}
			BusyKey::AddingTodo => self.adding_todo = false,
			BusyKey::AddingHabit => self.adding_habit = false,
		}
	}

	pub fn remove_task(&mut self, id: &str) {
		self.tasks.remove(id);
	}

	pub fn contains_task(&self, id: &str) -> bool {
		self.tasks.contains(id)
	}

	pub fn adding_habit(&self) -> bool {
		self.adding_habit
	}

	pub fn adding_todo(&self) -> bool {
		self.adding_todo
	}

	pub fn is_empty(&self) -> bool {
		self.tasks.is_empty() && !self.adding_todo && !self.adding_habit
	}

	pub fn len(&self) -> usize {
		self.tasks.len() + usize::from(self.adding_todo) + usize::from(self.adding_habit)
	}
}

#[derive(Debug, Default)]
pub struct Pending {
	pub revision: u64,
	pub sync_notice: Option<String>,
	pub busy: Busy,
	/// Placement is committed only after a cross-day schedule write succeeds.
	pub pending_week_drop: Option<WeekListPlacement>,
	origins: HashMap<BusyKey, u64>,
	next_editor: u64,
}

pub fn edit(state: &mut State, id: String, change: Change, join: Option<Change>) -> Task<Message> {
	if id == crate::inbox::DRAFT_ID {
		return crate::task_editor::edit_draft(state, change);
	}
	if !state.task_operations.busy.is_empty() {
		return Task::none();
	}
	let Some(task) = event(state, &id).filter(|e| e.is_task()).cloned() else {
		return Task::none();
	};
	if matches!(change, Change::Title(_)) {
		state.task_editor.renaming = Some(id.clone());
	}
	if matches!(
		change,
		Change::Description(_)
			| Change::Color(_)
			| Change::Projects(_)
			| Change::Labels(_)
			| Change::BacklogSection(_)
			| Change::Priority(_)
			| Change::Location(_)
			| Change::Title(_)
			| Change::Deadline(_)
			| Change::Day(_)
			| Change::Time(_, _)
			| Change::ClearSchedule
	) {
		state.task_editor.keep_open = Some(id.clone());
	}
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
	state.task_operations.revision += 1;
	crate::task_operations::begin(state, id.clone());
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
	blocking::run(move || {
		let changed =
			task_edit::save_and_reflow(task, settings.clone(), change, order, blocks, related)?;
		join_series(changed, &settings, join)
	})
	.map(move |result| {
		Message::TaskOperations(crate::task_operations::Action::SavedMany(
			id.clone().into(),
			result,
		))
	})
}

/// Register UI ownership separately from the lifetime of the visible form.
pub(crate) fn begin(state: &mut State, key: impl Into<BusyKey>) {
	let key = key.into();
	state
		.task_operations
		.origins
		.insert(key.clone(), state.task_editor.id);
	state.task_operations.busy.insert(key);
}

pub fn saved_many(
	state: &mut State,
	key: BusyKey,
	result: Result<Vec<Event>, String>,
) -> Task<Message> {
	let origin = state
		.task_operations
		.origins
		.remove(&key)
		.unwrap_or(state.task_editor.id);
	state.task_operations.busy.remove(&key);
	let mut tasks = Vec::new();
	match result {
		Ok(events) => {
			for (index, event) in events.into_iter().enumerate() {
				let (key, origin) = if index == 0 {
					(key.clone(), Some(origin))
				} else {
					(BusyKey::Task(event.id.clone()), None)
				};
				tasks.push(apply_result(state, key, Ok(event), origin));
			}
		}
		Err(error) => {
			state.task_operations.sync_notice = Some(error.clone());
			tasks.push(apply_result(state, key, Err(error), Some(origin)));
		}
	}
	// Refresh also reconciles partially successful cross-service writes.
	tasks.push(crate::local_poll::refresh_calendar(state));
	tasks.push(Task::done(Message::Calendar(
		CalendarMessage::CalendarSyncRequested,
	)));
	Task::batch(tasks)
}

pub fn saved(state: &mut State, key: BusyKey, result: Result<Event, String>) -> Task<Message> {
	let origin = state
		.task_operations
		.origins
		.remove(&key)
		.unwrap_or(state.task_editor.id);
	apply_result(state, key, result, Some(origin))
}

fn apply_result(
	state: &mut State,
	key: BusyKey,
	result: Result<Event, String>,
	origin: Option<u64>,
) -> Task<Message> {
	state.task_operations.revision += 1;
	state.task_operations.busy.remove(&key);
	let id = key.task_id();
	// Editor feedback goes only to the instance that submitted this write.
	// The calendar update below is independent of whether that editor is open.
	if matches!(key, BusyKey::Task(_)) && origin == Some(state.task_editor.id) {
		let showing = state.calendar.open_event.as_deref() == Some(id);
		let previous = state.calendar.event(id);
		let feedback = state.task_editor.saved(id, &result, previous, showing);
		if feedback.close_details && showing && state.inbox.draft.is_none() {
			state.calendar.open_event = None;
		}
		if let Some(error) = feedback.error {
			state.calendar.error = Some(error);
		}
	} else if matches!(key, BusyKey::Task(_))
		&& let Err(error) = &result
	{
		state.calendar.error = Some(error.clone());
	}
	let mut refresh_week = false;
	match result {
		Ok(event) => {
			if key == BusyKey::AddingTodo {
				state.timeblocking.todo_draft.clear();
				state.timeblocking.todo_error = None;
				state.timeblocking.todo_open = false;
			}
			if event.is_calendar_task() && state.config.calendar.app_calendar.is_none() {
				state.config.calendar.app_calendar = Some(event.calendar_id.clone());
			}
			let saved_id = event.id.clone();
			let task_key = task_key(&event);
			state
				.calendar
				.events
				.retain(|e| e.id != id && (!e.is_task() || self::task_key(e) != task_key));
			state.calendar.replace_task(id, &event);
			refresh_week = state.range_view_active();
			if let Some(placement) = state
				.task_operations
				.pending_week_drop
				.take_if(|p| p.task == id)
			{
				for order in state.config.calendar.weekly_task_order.values_mut() {
					order.retain(|old| old != id);
				}
				place_week_card(
					state,
					&saved_id,
					placement.day,
					placement.target.as_deref(),
					placement.after,
				);
			}
		}
		Err(error) if key == BusyKey::AddingTodo => {
			state.timeblocking.todo_error = Some(error);
			state.timeblocking.todo_open = true;
		}
		Err(_) => {}
	}
	state
		.task_operations
		.pending_week_drop
		.take_if(|p| p.task == id);
	let save = crate::app::queue_save(state);
	if refresh_week {
		Task::batch([save, crate::app::refresh_week_after_write(state)])
	} else {
		save
	}
}

impl Pending {
	pub(crate) fn next_editor(&mut self) -> u64 {
		self.next_editor += 1;
		self.next_editor
	}
}
