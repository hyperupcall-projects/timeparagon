//! Desktop task fields and validation.
use crate::app::{Message, State};
use crate::calendar::{self, Event};
use crate::timeblocking::event;
use chrono::NaiveDate;
use iced::Task;
use timeparagon_core::task_edit::{self, Change, clean_label};
use timeparagon_core::timeblocking::{Priority, UNDATED, is_block, metadata};

#[derive(Debug, Clone)]
pub enum Action {
	ColorInput(String, String),
	SaveColor(String, String),
	/// This task's due date field changed. Empty clears it.
	DueDate(String, String),
	/// This task's "All day" box changed.
	DueAllDay(String, bool),
	/// This task's start time field changed.
	DueStart(String, String),
	/// This task's end time field changed.
	DueEnd(String, String),
	/// This task's deadline field changed. Empty clears it.
	Deadline(String, String),
	SaveDeadline(String),
	EditTitle,
	EditEstimate(String),
	CloseEstimate,
	Estimate(String),
	SaveEstimate(String),
	/// Give this task an estimate in minutes, without the estimate popup.
	SetEstimate(String, u32),
	/// Set or clear this task's deadline, without the deadline field.
	SetDeadline(String, Option<calendar::deadline::Deadline>),
	/// The name being typed for an existing task changed.
	Title(String),
	/// Save the edited name of this task.
	SaveTitle(String),
	SaveDescription(String, String),
	/// Add a project to this task.
	ProjectAdd(String, u64),
	/// Remove a project from this task.
	ProjectRemove(String, u64),
	/// The project search box changed.
	ProjectQuery(String),
	/// Open or close the project picker for this task.
	ProjectPicker(String, bool),
	/// The label being typed for this task changed.
	LabelDraft(String),
	/// Add this label to the task.
	AddLabel(String, String),
	/// Remove a label from this task.
	RemoveLabel(String, String),
	/// Give this task a priority, or clear it.
	Priority(String, Option<Priority>),
	/// Set where this task happens, by `config::Place::id`, or clear it.
	Location(String, Option<u64>),
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
	state.inbox.error = None;
	let (id, change) = match action {
		Action::DueDate(id, value) => {
			if schedule_locked(state, &id) {
				return Task::none();
			}
			state.task_editor_mut().date = value;
			let timed = due_is_timed(state, &id);
			let Some(change) = due_change(state, timed) else {
				return Task::none();
			};
			(id, change)
		}
		Action::DueAllDay(id, all_day) => {
			if schedule_locked(state, &id) {
				return Task::none();
			}
			let schedule = state.task_editor_mut();
			if calendar::parse_clock(&schedule.start_time).is_none()
				|| calendar::parse_clock(&schedule.end_time).is_none()
			{
				schedule.start_time = "09:00".into();
				schedule.end_time = "09:30".into();
			}
			let Some(change) = due_change(state, !all_day) else {
				return Task::none();
			};
			(id, change)
		}
		Action::DueStart(id, value) => {
			if schedule_locked(state, &id) {
				return Task::none();
			}
			let schedule = state.task_editor_mut();
			if let Some(end) = end_keeping_length(&schedule.start_time, &schedule.end_time, &value)
			{
				schedule.end_time = end;
			}
			schedule.start_time = value;
			let Some(change) = due_change(state, true) else {
				return Task::none();
			};
			(id, change)
		}
		Action::DueEnd(id, value) => {
			if schedule_locked(state, &id) {
				return Task::none();
			}
			state.task_editor_mut().end_time = value;
			let Some(change) = due_change(state, true) else {
				return Task::none();
			};
			(id, change)
		}
		Action::EditTitle => {
			if state
				.calendar
				.open_event
				.as_deref()
				.and_then(|id| state.calendar.event(id))
				.and_then(|event| state.habits.habit_of(event))
				.is_some()
			{
				return Task::none();
			}
			state.task_editor_mut().editing_title = true;
			return iced::widget::operation::focus(iced::widget::Id::new("task-title"));
		}
		Action::Deadline(id, value) => {
			if schedule_locked(state, &id) {
				return Task::none();
			}
			state.task_editor_mut().deadline = value;
			return update(state, Action::SaveDeadline(id));
		}
		Action::SaveDeadline(id) => {
			let input = state.task_editor().deadline.trim();
			let date = if input.is_empty() {
				None
			} else {
				match calendar::deadline::parse_input(input) {
					Ok(deadline) => Some(deadline),
					Err(error) => {
						state.calendar.error = Some(error);
						return Task::none();
					}
				}
			};
			(id, Change::Deadline(date))
		}
		Action::EditEstimate(id) => {
			let Some(task) = event(state, &id).filter(|e| e.is_task()) else {
				return Task::none();
			};
			if state.task_operations.busy.contains_task(&id) {
				return Task::none();
			}
			state.task_editor_mut().estimate = metadata(task).minutes.to_string();
			state.task_editor_mut().estimate_task = Some(id);
			state.task_editor_mut().estimate_error = None;
			state.timeblocking.dragging = None;
			state.timeblocking.drag_pointer = None;
			return iced::widget::operation::focus(iced::widget::Id::new("task-estimate"));
		}
		Action::CloseEstimate => {
			state.task_editor_mut().estimate_task = None;
			state.task_editor_mut().estimate_error = None;
			return Task::none();
		}
		Action::Estimate(value) => {
			state.task_editor_mut().estimate = value;
			state.task_editor_mut().estimate_error = None;
			return Task::none();
		}
		Action::SaveEstimate(id) => {
			if state.task_editor().estimate_task.as_deref() != Some(&id) {
				return Task::none();
			}
			let Ok(minutes) = state.task_editor().estimate.trim().parse::<u32>() else {
				state.task_editor_mut().estimate_error =
					Some("Enter an estimate in whole minutes".into());
				return Task::none();
			};
			if minutes == 0 {
				state.task_editor_mut().estimate_error = Some("An estimate is required".into());
				return Task::none();
			}
			if minutes > 10080 {
				state.task_editor_mut().estimate_error =
					Some("Estimate must be at most 10080 minutes".into());
				return Task::none();
			}
			(id, Change::Estimate(minutes))
		}
		Action::SetEstimate(id, minutes) => {
			if minutes == 0 || minutes > 10080 {
				return Task::none();
			}
			(id, Change::Estimate(minutes))
		}
		Action::SetDeadline(id, deadline) => (id, Change::Deadline(deadline)),
		Action::Title(value) => {
			state.task_editor_mut().title = value;
			state.task_editor_mut().title_error = None;
			return Task::none();
		}
		Action::ColorInput(id, value) => {
			state.task_editor_mut().color_input = Some((id, value));
			return Task::none();
		}
		Action::SaveColor(id, value) => {
			let choice = match crate::calendar::colors::selection(&value) {
				Ok(choice) => choice,
				Err(error) => {
					state.calendar.error = Some(error);
					return Task::none();
				}
			};
			state.task_editor_mut().color_input = None;
			if let Some(block) = event(state, &id)
				.filter(|e| is_block(e, &state.config.calendar))
				.cloned()
			{
				if block.details.sending
					|| state
						.calendar
						.edit_draft
						.as_ref()
						.is_some_and(|d| d.id == id && d.saving)
				{
					return Task::none();
				}
				let mut draft = state
					.calendar
					.edit_draft
					.take()
					.filter(|d| d.id == id)
					.unwrap_or_else(|| calendar::EditDraft::from_event(&block));
				draft.color_input = choice.unwrap_or_default();
				state.calendar.edit_draft = Some(draft);
				return crate::app::update(
					state,
					Message::EventEdit(crate::app::EventEditMessage::EventEditSaveRequested),
				);
			}
			(id, Change::Color(choice))
		}
		Action::SaveDescription(id, value) => (id, Change::Description(value)),
		Action::SaveTitle(id) => {
			if event(state, &id)
				.and_then(|event| state.habits.habit_of(event))
				.is_some()
			{
				return Task::none();
			}
			let title = state.task_editor().title.trim().to_owned();
			if title.is_empty() {
				state.task_editor_mut().title_error = Some("A task needs a name".into());
				return Task::none();
			}
			if title.chars().count() > task_edit::MAX_TITLE_CHARS {
				state.task_editor_mut().title_error = Some(format!(
					"A task name can be at most {} characters",
					task_edit::MAX_TITLE_CHARS
				));
				return Task::none();
			}
			let Some(task) = event(state, &id).filter(|event| event.is_task()) else {
				return Task::none();
			};
			if task.title == title {
				state.task_editor_mut().title_error = None;
				state.task_editor_mut().editing_title = false;
				return Task::none();
			}
			state.task_editor_mut().title_error = None;
			(id, Change::Title(title))
		}
		Action::ProjectAdd(id, project) => {
			if state.config.projects.get(project).is_none() {
				return Task::none();
			}
			let Some(task) = event(state, &id).filter(|event| event.is_task()) else {
				return Task::none();
			};
			let mut projects = metadata(task).projects;
			if projects.contains(&project) {
				return Task::none();
			}
			projects.push(project);
			projects.sort_unstable();
			(id, Change::Projects(projects))
		}
		Action::ProjectRemove(id, project) => {
			let Some(task) = event(state, &id).filter(|event| event.is_task()) else {
				return Task::none();
			};
			let mut projects = metadata(task).projects;
			if !projects.contains(&project) {
				return Task::none();
			}
			projects.retain(|id| *id != project);
			(id, Change::Projects(projects))
		}
		Action::ProjectQuery(value) => {
			state.task_editor_mut().project_query = value;
			return Task::none();
		}
		Action::ProjectPicker(id, open) => {
			if open {
				state.task_editor_mut().project_picker = Some(id);
			} else if state.task_editor().project_picker.as_deref() == Some(id.as_str()) {
				state.task_editor_mut().project_picker = None;
			}
			return Task::none();
		}
		Action::LabelDraft(value) => {
			state.task_editor_mut().label_draft = value;
			state.task_editor_mut().label_error = None;
			return Task::none();
		}
		Action::AddLabel(id, label) => {
			let Some(task) = event(state, &id).filter(|event| event.is_task()) else {
				return Task::none();
			};
			let label = match clean_label(&label) {
				Ok(label) => label,
				Err(error) => {
					state.task_editor_mut().label_error = Some(error.to_owned());
					return Task::none();
				}
			};
			let mut labels = metadata(task).labels;
			if labels
				.iter()
				.any(|existing| existing.eq_ignore_ascii_case(&label))
			{
				state.task_editor_mut().label_draft.clear();
				state.task_editor_mut().label_error = None;
				return Task::none();
			}
			labels.push(label);
			state.task_editor_mut().label_draft.clear();
			state.task_editor_mut().label_error = None;
			(id, Change::Labels(labels))
		}
		Action::RemoveLabel(id, label) => {
			let Some(task) = event(state, &id).filter(|event| event.is_task()) else {
				return Task::none();
			};
			let mut labels = metadata(task).labels;
			if !labels
				.iter()
				.any(|existing| existing.eq_ignore_ascii_case(&label))
			{
				return Task::none();
			}
			labels.retain(|existing| !existing.eq_ignore_ascii_case(&label));
			(id, Change::Labels(labels))
		}
		Action::Priority(id, priority) => {
			let Some(task) = event(state, &id).filter(|event| event.is_task()) else {
				return Task::none();
			};
			if metadata(task).priority == priority {
				return Task::none();
			}
			(id, Change::Priority(priority))
		}
		Action::Location(id, location) => {
			let Some(task) = event(state, &id).filter(|event| event.is_task()) else {
				return Task::none();
			};
			if metadata(task).location == location {
				return Task::none();
			}
			(id, Change::Location(location))
		}
	};
	crate::task_operations::edit(state, id, change, None)
}

impl Editor {
	pub(crate) fn seed_schedule(&mut self, task: &Event) {
		self.date = if task.start.date_naive() == UNDATED {
			String::new()
		} else {
			task.start.date_naive().to_string()
		};
		self.deadline = calendar::deadline::input_text(task);
		self.start_time = if task.all_day {
			"09:00".into()
		} else {
			calendar::clock_label(task.start)
		};
		self.end_time = if task.all_day {
			"09:30".into()
		} else {
			calendar::clock_label(task.end)
		};
	}
}

/// Another write holds the queue, so a change to this existing task could not
/// be sent. Leaving the field alone keeps it saying what the task says.
fn schedule_locked(state: &State, id: &str) -> bool {
	id != crate::inbox::DRAFT_ID && !state.task_operations.busy.is_empty()
}

fn due_is_timed(state: &State, id: &str) -> bool {
	let task = if id == crate::inbox::DRAFT_ID {
		state.inbox.draft.as_ref().and_then(|d| d.event.as_ref())
	} else {
		event(state, id)
	};
	task.is_some_and(|task| !task.all_day)
}

/// What the due fields say now, as a change to the task. A problem with them is
/// reported, and leaves the task as it is.
fn due_change(state: &mut State, timed: bool) -> Option<Change> {
	let schedule = state.task_editor();
	let date = schedule.date.trim();
	let change = if date.is_empty() {
		Ok(Change::ClearSchedule)
	} else {
		match NaiveDate::parse_from_str(date, "%Y-%m-%d") {
			Err(_) => Err("Enter a date as YYYY-MM-DD".to_owned()),
			Ok(day) if !timed => Ok(Change::Day(day)),
			Ok(_) => task_edit::schedule_range(date, &schedule.start_time, &schedule.end_time)
				.and_then(|(start, end)| {
					if end > start {
						Ok(Change::Time(start, end))
					} else {
						Err("The task must end after it starts".to_owned())
					}
				}),
		}
	};
	change
		.map_err(|error| state.calendar.error = Some(error))
		.ok()
}

/// The end time that keeps a task's length when its start moves to `moved`.
pub(crate) fn end_keeping_length(start: &str, end: &str, moved: &str) -> Option<String> {
	const DAY: i64 = 24 * 60;
	let minutes = |clock: &str| {
		calendar::parse_clock(clock).map(|(hour, minute)| i64::from(hour) * 60 + i64::from(minute))
	};
	let length = (minutes(end)? - minutes(start)?).rem_euclid(DAY);
	let total = (minutes(moved)? + length).rem_euclid(DAY);
	Some(format!("{:02}:{:02}", total / 60, total % 60))
}

#[derive(Debug, Default, Clone)]
pub struct Editor {
	pub(crate) id: u64,
	pub date: String,
	pub deadline: String,
	pub start_time: String,
	pub end_time: String,
	pub estimate: String,
	pub estimate_task: Option<String>,
	pub estimate_error: Option<String>,
	pub color_input: Option<(String, String)>,
	/// The name being edited for the open task, before it is saved.
	pub title: String,
	pub editing_title: bool,
	pub title_error: Option<String>,
	/// The task whose rename is in flight, so a failure stays on the name field.
	pub renaming: Option<String>,
	/// Text that narrows the project picker in the task editor.
	pub project_query: String,
	/// The task whose project picker is open.
	pub project_picker: Option<String>,
	/// The task whose backlog destination choices are open.
	pub backlog_picker: Option<String>,
	/// The label being typed for the open task, before it is added.
	pub label_draft: String,
	pub label_error: Option<String>,
	/// A task whose fields are being saved without closing its details pane.
	pub keep_open: Option<String>,
}

impl Editor {
	pub(crate) fn new(pending: &mut crate::task_operations::Pending) -> Self {
		Self {
			id: pending.next_editor(),
			..Self::default()
		}
	}
}

pub(crate) fn edit_draft(state: &mut State, change: Change) -> Task<Message> {
	let Some(draft) = state.inbox.draft.as_mut().filter(|d| !d.saving) else {
		return Task::none();
	};
	let Some(task) = draft.event.as_mut() else {
		return Task::none();
	};
	let data = task.details.task_data.as_mut().unwrap();
	match change {
		Change::Color(v) => data.metadata.color = v,
		Change::Estimate(v) => {
			data.metadata.minutes = v;
			draft.editor.estimate_task = None;
		}
		Change::Projects(v) => data.metadata.projects = v,
		Change::Labels(v) => data.metadata.labels = v,
		Change::Priority(v) => data.metadata.priority = v,
		Change::Location(v) => data.metadata.location = v,
		Change::Deadline(v) => {
			draft.deadline = v.map(|v| v.value()).unwrap_or_default();
		}
		Change::Day(day) => {
			task.start = calendar::at_clock(day, 0, 0);
			task.end = task.start + chrono::Duration::days(1);
			task.all_day = true;
		}
		Change::ClearSchedule => {
			task.start = calendar::at_clock(UNDATED, 0, 0);
			task.end = task.start + chrono::Duration::days(1);
			task.all_day = true;
		}
		Change::Time(start, end) => {
			task.start = start;
			task.end = end;
			task.all_day = false;
			draft.editor.start_time = calendar::clock_label(start);
			draft.editor.end_time = calendar::clock_label(end);
		}
		_ => {}
	}
	return Task::none();
}

#[derive(Default)]
pub(crate) struct SaveFeedback {
	pub close_details: bool,
	pub error: Option<String>,
}

impl Editor {
	pub(crate) fn saved(
		&mut self,
		id: &str,
		result: &Result<Event, String>,
		previous: Option<&Event>,
		showing: bool,
	) -> SaveFeedback {
		let editing_estimate = self.estimate_task.as_deref() == Some(id);
		let renaming = self.renaming.as_deref() == Some(id);
		let keep_open = self.keep_open.take_if(|task| task == id).is_some();
		let mut feedback = SaveFeedback::default();
		match result {
			Ok(event) => {
				if renaming {
					self.renaming = None;
					self.title_error = None;
					self.editing_title = false;
				}
				if showing && keep_open {
					self.seed_schedule(event);
				}
				if editing_estimate {
					self.estimate_task = None;
					self.estimate_error = None;
				} else {
					feedback.close_details = !keep_open;
				}
			}
			Err(error) => {
				if showing
					&& keep_open
					&& let Some(task) = previous
				{
					self.seed_schedule(task);
				}
				if editing_estimate {
					self.estimate_error = Some(error.clone());
				} else if renaming {
					self.renaming = None;
					if showing {
						self.title_error = Some(error.clone());
					} else {
						feedback.error = Some(error.clone());
					}
				} else {
					feedback.error = Some(error.clone());
				}
			}
		}
		feedback
	}
}

impl From<Action> for Message {
	fn from(action: Action) -> Self {
		Self::TaskEditor(action)
	}
}
