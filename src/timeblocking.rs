//! Desktop block planning and a compatibility router for planning UI actions.
//! Task forms, pending writes, and work tracking own their state separately.
use crate::calendar::ids;
use crate::{
	app::{Message, State},
	blocking,
	calendar::{self, Event},
	config::CalendarSettings,
};
use chrono::{DateTime, Local, NaiveDate, NaiveTime};
use iced::Task;
use timeparagon_core::calendar::google::GoogleError;
use timeparagon_core::task_edit::{self, Change};

pub use timeparagon_core::timeblocking::*;

mod drag;
mod recurring;
#[cfg(test)]
use crate::focus::advance_pomodoro;
pub use crate::focus::current_task;
#[cfg(test)]
use crate::task_editor::end_keeping_length;
pub(crate) use drag::place_week_card;
pub(crate) use drag::week_drop_times;
pub use recurring::repeats_on_its_own;
use recurring::*;

pub const PANEL_WIDTH: f32 = 400.0;
pub use crate::task_operations::{Busy, BusyKey};
use timeparagon_core::task_edit::MAX_TITLE_CHARS as MAX_TODO_CHARS;
/// What a recurring item being added to a time block is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecurringItem {
	/// An occurrence of a recurring task, by event id.
	Task(String),
	/// A habit from the library, by id, with a repeating habit event.
	Habit(String),
}

/// A recurring task or habit waiting on whether its future occurrences stay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRecurring {
	pub item: RecurringItem,
	pub block: String,
}

/// A one-off habit in a repeating block, waiting on whether it repeats with
/// the block, which ends the habit's own repeat from that day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingJoin {
	pub task: String,
	pub block: String,
}

pub use crate::focus::DistractionCause;

#[derive(Default)]
pub struct Workspace {
	/// The recurring item whose move into a block is being confirmed.
	pub pending_recurring: Option<PendingRecurring>,
	/// The one-off habit whose repeating with its block is being confirmed.
	pub pending_join: Option<PendingJoin>,
	/// What is added to a repeating block goes in this occurrence only,
	/// rather than repeating with the block.
	pub one_off: bool,
	pub block: Option<String>,
	pub focus: Option<String>,
	/// The line being typed into the open time block, before it is saved.
	pub todo_draft: String,
	pub todo_error: Option<String>,
	/// The planned-tasks box is open so a task title can be typed.
	pub todo_open: bool,
	/// Text that narrows the available tasks under the open time block.
	pub available_query: String,
	pub available_habit_query: String,
	/// Include tasks from all dates instead of only the current day.
	pub available_all: bool,
	/// Include tasks that already have a due time or calendar schedule.
	pub available_planned: bool,
	pub dragging: Option<String>,
	/// Where the pointer is while a task is being dragged, so the floating
	/// card can follow it. Only meaningful while `dragging` is set.
	pub drag_pointer: Option<iced::Point>,
	/// The surface whose coordinates `drag_pointer` uses.
	pub drag_window: Option<iced::window::Id>,
	/// What letting go of the task in the air would do right now, as found by
	/// asking the plan view under the pointer. It drives the drop preview, so
	/// the preview always agrees with where the task actually lands.
	pub drop_hover: Option<DropHover>,
	/// Text that narrows the backlog by task title or description.
	pub backlog_query: String,
	/// When set, the backlog shows only tasks from this Google Tasks list.
	pub backlog_list: Option<String>,
}
#[derive(Debug, Clone)]
pub struct WeekListPlacement {
	pub task: String,
	pub day: NaiveDate,
	pub target: Option<String>,
	pub after: bool,
}

/// The drop a plan view would make if the task in the air were let go now.
#[derive(Debug, Clone)]
pub struct DropHover {
	/// The day of the range view column or cell under the pointer. `None`
	/// when the view under it is the single day being planned.
	pub day: Option<NaiveDate>,
	pub action: Action,
}

/// Planning actions, including compatibility entry points for existing callers.
/// New task forms and work controls send their typed messages directly to the app.
// Messages only live until the next update, so a large variant costs nothing lasting.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Action {
	ColorInput(String, String),
	SaveColor(String, String),
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
	View(String),
	Plan(String),
	CloseView,
	StartWork(String),
	StopWork(String),
	WorkTick,
	PomodoroToggle,
	PomodoroReset,
	PomodoroMinutes(bool, i32),
	/// Open the editor for the work session at this index in the log.
	EditSession(usize),
	SessionStart(String),
	SessionEnd(String),
	SaveSession,
	CloseSession,
	DeleteSession,
	/// Ask what distracted from the focused block, and for how long.
	OpenDistraction,
	CloseDistraction,
	/// Answer what the distraction was, which moves on to how long it lasted.
	ChooseDistraction(DistractionCause),
	/// Record this many distracted minutes against the focused block, then
	/// open the habit that caused them, if one did.
	AddDistraction(u32),
	/// Take back the focused block's latest distraction.
	UndoDistraction,
	HideOverdue(bool),
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
	Schedule(String, DateTime<Local>, DateTime<Local>),
	DropTime(DateTime<Local>),
	RefreshBlock(String),
	BacklogDestination(Option<String>),
	/// The Google Tasks list triaged tasks are moved to, or `None` for the
	/// default.
	TriagedDestination(Option<String>),
	/// Require an estimate before a task can be triaged.
	RequireEstimate(bool),
	/// Require a project before a task can be triaged.
	RequireProject(bool),
	/// Require a priority before a task can be triaged.
	RequirePriority(bool),
	Close,
	Details(String),
	CancelDrag,
	EditEstimate(String),
	CloseEstimate,
	Estimate(String),
	SaveEstimate(String),
	/// Give this task an estimate in minutes, without the estimate popup.
	SetEstimate(String, u32),
	/// Set or clear this task's deadline, without the deadline field.
	SetDeadline(String, Option<calendar::deadline::Deadline>),
	Backlog(String),
	BacklogPicker(String),
	MoveToBacklog(String, BacklogSection),
	/// Return a task to the inbox for review, clearing its schedule.
	MoveToInbox(String),
	/// Drop the dragged task into a backlog planning horizon.
	DropBacklogSection(BacklogSection),
	SetBacklogSection(String, BacklogSection),
	/// Confirm a task out of the Inbox and onto the triaged list.
	Triage(String),
	/// Remove a task from its block without deleting it or moving lists.
	Unassign(String),
	/// The backlog search box changed.
	BacklogQuery(String),
	/// Restrict the backlog to one task list, or all lists when `None`.
	BacklogList(Option<String>),
	/// Plan the task for this day, keeping its list.
	ScheduleDay(String, NaiveDate),
	Assign(String, String),
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
	/// Open the planned-tasks box so a task can be typed.
	OpenTodo,
	/// Close the typed-task box without saving.
	CloseTodo,
	/// The free-text todo being typed into a time block changed.
	TodoDraft(String),
	/// Save the typed line as a Google Task in this time block.
	AddTodo(String),
	/// The available-tasks search box changed.
	AvailableQuery(String),
	AvailableHabitQuery(String),
	AddHabit(String, String),
	/// Include available tasks from all dates, or only tasks dated today.
	AvailableAll(bool),
	/// Include planned tasks in the available-tasks list.
	AvailablePlanned(bool),
	Drag(String, iced::Point),
	/// The pointer moved while a task is being dragged, so the floating card
	/// can follow it.
	DragMoved(iced::Point),
	DropBlock(String),
	DropTask(String),
	/// A task row in the calendar, independent of the selected sidebar tab.
	DropCalendarTask(String),
	DropDay,
	/// Drop between two rows of a day's list. `after` names the row the gap
	/// follows (none for the top of the list), so the preview can show there.
	/// `start` is when the task would then begin: right after the timed row
	/// above, or ending as the timed row below starts. With no timed row on
	/// either side it goes on the day as a whole, as by [`Action::DropDay`].
	DropAt {
		after: Option<String>,
		start: Option<DateTime<Local>>,
	},
	/// Insert before/after a board card, or at the end of an empty/day column.
	DropWeekList {
		target: Option<String>,
		after: bool,
	},
	NewWeekTask,
	Saved(BusyKey, Result<Event, String>),
	SavedMany(BusyKey, Result<Vec<Event>, String>),
}
/// The tasks in `block`, in the order they are laid out (see
/// [`timeparagon_core::timeblocking::membership`]).
pub fn assigned_all<'a>(state: &'a State, block: &Event) -> Vec<&'a Event> {
	tasks_in(block, &state.calendar.events, day_start_hour(state))
}

/// The hour the planning day starts at, which decides which day an
/// occurrence counts towards.
pub fn day_start_hour(state: &State) -> u32 {
	state.config.calendar.day_start_hour()
}

/// The key of the loaded block `task` is in.
pub fn block_key_of(state: &State, task: &Event) -> Option<String> {
	block_of(task, &state.calendar.events, day_start_hour(state)).map(block_key)
}

/// Whether `block` repeats, so what goes in it is asked to repeat with it
/// or stay a one-off.
pub fn block_repeats(block: &Event) -> bool {
	block.details.recurring_event_id.is_some()
}
pub fn assigned<'a>(state: &'a State, block: &Event) -> Vec<&'a Event> {
	assigned_all(state, block)
}
/// The Google Tasks list a task belongs to, using the list title when it is known.
pub fn task_list_name(state: &State, calendar_id: &str) -> String {
	state
		.calendar
		.calendars
		.iter()
		.find(|calendar| calendar.id == calendar_id)
		.map(|calendar| calendar.name.trim())
		.filter(|name| !name.is_empty())
		.map(str::to_owned)
		.unwrap_or_else(|| {
			calendar_id
				.strip_prefix(ids::TASK_LIST)
				.filter(|id| !id.is_empty())
				.unwrap_or("Tasks")
				.to_owned()
		})
}

/// Whether `calendar_id` is the list that holds backlog tasks.
pub fn is_backlog_list(state: &State, calendar_id: &str) -> bool {
	if let Some(chosen) = &state.config.calendar.backlog_list {
		return ids::bare_list(chosen) == ids::bare_list(calendar_id);
	}
	let default = CalendarSettings::DEFAULT_BACKLOG_LIST;
	calendar_id
		.trim_start_matches("tasks:")
		.eq_ignore_ascii_case(default)
		|| state.calendar.calendars.iter().any(|calendar| {
			calendar.id == calendar_id
				&& calendar.is_task_list()
				&& calendar.name.trim().eq_ignore_ascii_case(default)
		})
}

/// When a task is due: its metadata deadline if it has one, otherwise the
/// Google Tasks due day. `None` for a task with neither.
pub fn due(event: &Event) -> Option<calendar::deadline::Deadline> {
	if let Some(deadline) = calendar::deadline::get(event) {
		return Some(deadline);
	}
	let date = event.start.date_naive();
	(date != UNDATED).then_some(calendar::deadline::Deadline { date, time: None })
}

/// Backlog order: soonest due first, then later ones, then tasks with no due
/// date. Ties keep the manual order.
pub fn backlog_order(event: &Event) -> (bool, Option<NaiveDate>, Option<NaiveTime>, i64) {
	let due = due(event);
	(
		due.is_none(),
		due.map(|due| due.date),
		due.and_then(|due| due.time),
		metadata(event).order,
	)
}

pub fn close(state: &mut State) -> Task<Message> {
	// Closing the block's panel drops its inline draft with it.
	let block = state.timeblocking.block.clone();
	if state
		.calendar
		.edit_draft
		.as_ref()
		.zip(block.as_deref())
		.is_some_and(|(draft, block)| draft.id == block)
	{
		state.calendar.edit_draft = None;
	}
	state.timeblocking.block = None;
	state.timeblocking.focus = None;
	state.work_tracking.distraction_open = false;
	state.work_tracking.session_edit = None;
	state.work_tracking.session_error = None;
	state.timeblocking.dragging = None;
	state.timeblocking.drag_pointer = None;
	state.timeblocking.available_query.clear();
	state.timeblocking.available_habit_query.clear();
	state.timeblocking.available_all = false;
	state.timeblocking.available_planned = false;
	// A save still in flight keeps the draft so its result can land on the same field.
	if !state.task_operations.busy.adding_todo() {
		state.timeblocking.todo_open = false;
		state.timeblocking.todo_draft.clear();
		state.timeblocking.todo_error = None;
	}
	Task::none()
}
pub fn open(state: &mut State, id: String) -> Task<Message> {
	if state.timeblocking.block.as_deref() != Some(id.as_str())
		&& !state.task_operations.busy.adding_todo()
	{
		state.timeblocking.todo_open = false;
		state.timeblocking.todo_draft.clear();
		state.timeblocking.todo_error = None;
		state.timeblocking.available_query.clear();
		state.timeblocking.available_habit_query.clear();
		state.timeblocking.available_all = false;
		state.timeblocking.available_planned = false;
	}
	state.timeblocking.focus = None;
	state.work_tracking.distraction_open = false;
	// The panel edits the block in place, so it always opens with a live
	// draft of the block's own fields.
	state.calendar.edit_draft = event(state, &id).map(crate::calendar::EditDraft::from_event);
	state.timeblocking.block = Some(id);
	Task::none()
}

/// Resolve staged tasks without adding them to calendar snapshots or caches.
pub fn event<'a>(state: &'a State, id: &str) -> Option<&'a Event> {
	state
		.inbox
		.draft
		.as_ref()
		.and_then(|d| d.event.as_ref())
		.filter(|e| e.id == id)
		.or_else(|| state.calendar.event(id))
}

pub(crate) fn schedule_created(
	task: Event,
	settings: &CalendarSettings,
	start: DateTime<Local>,
	end: DateTime<Local>,
) -> Result<Event, String> {
	save(task, settings, Change::Time(start, end), 0)
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
	// Triaging is a review step: the task cannot leave the Inbox until every
	// required field is filled in. Nothing is sent to Google when it is not.
	if let Action::Triage(id) = &action
		&& let Some(error) = triage_error(state, id)
	{
		state.inbox.error = Some(error);
		return Task::none();
	}
	state.inbox.error = None;
	// Set when the change puts a task in a recurring block, to repeat with it.
	let mut join = None;
	let (id, change) = match action {
		Action::View(id) => {
			if !event(state, &id).is_some_and(|e| is_block(e, &state.config.calendar)) {
				return Task::none();
			}
			let resize = close(state);
			// The block's fields are edited in place in the panel, so seed the
			// draft as the panel opens rather than behind an Edit button.
			state.calendar.edit_draft =
				event(state, &id).map(crate::calendar::EditDraft::from_event);
			state.timeblocking.focus = Some(id);
			state.work_tracking.work_now = Some(crate::clock::utc_now());
			state.calendar.open_event = None;
			state.clear_calendar_overlays();
			let unfocus = iced::advanced::widget::operate(
				iced::advanced::widget::operation::focusable::unfocus(),
			);
			return Task::batch([resize, unfocus]);
		}
		Action::Plan(id) => {
			if !event(state, &id).is_some_and(|e| is_block(e, &state.config.calendar)) {
				return Task::none();
			}
			state.calendar.open_event = None;
			state.calendar.detail_event = None;
			state.close_calendar_popup();
			return open(state, id);
		}
		Action::CloseView => {
			state.timeblocking.focus = None;
			state.work_tracking.distraction_open = false;
			return Task::none();
		}
		Action::WorkTick => return crate::focus::update(state, crate::focus::Action::WorkTick),
		Action::PomodoroToggle => {
			return crate::focus::update(state, crate::focus::Action::PomodoroToggle);
		}
		Action::PomodoroReset => {
			return crate::focus::update(state, crate::focus::Action::PomodoroReset);
		}
		Action::PomodoroMinutes(work, delta) => {
			return crate::focus::update(state, crate::focus::Action::PomodoroMinutes(work, delta));
		}
		Action::EditSession(index) => {
			return crate::focus::update(state, crate::focus::Action::EditSession(index));
		}
		Action::SessionStart(value) => {
			return crate::focus::update(state, crate::focus::Action::SessionStart(value));
		}
		Action::SessionEnd(value) => {
			return crate::focus::update(state, crate::focus::Action::SessionEnd(value));
		}
		Action::SaveSession => {
			return crate::focus::update(state, crate::focus::Action::SaveSession);
		}
		Action::CloseSession => {
			return crate::focus::update(state, crate::focus::Action::CloseSession);
		}
		Action::DeleteSession => {
			return crate::focus::update(state, crate::focus::Action::DeleteSession);
		}
		Action::OpenDistraction => {
			return crate::focus::update(state, crate::focus::Action::OpenDistraction);
		}
		Action::CloseDistraction => {
			return crate::focus::update(state, crate::focus::Action::CloseDistraction);
		}
		Action::ChooseDistraction(cause) => {
			return crate::focus::update(state, crate::focus::Action::ChooseDistraction(cause));
		}
		Action::AddDistraction(minutes) => {
			return crate::focus::update(state, crate::focus::Action::AddDistraction(minutes));
		}
		Action::UndoDistraction => {
			return crate::focus::update(state, crate::focus::Action::UndoDistraction);
		}
		Action::StartWork(id) => {
			return crate::focus::update(state, crate::focus::Action::StartWork(id));
		}
		Action::StopWork(id) => {
			return crate::focus::update(state, crate::focus::Action::StopWork(id));
		}
		Action::HideOverdue(hide) => {
			state.config.calendar.hide_overdue_tasks = hide;
			return crate::app::queue_save(state);
		}
		Action::DueDate(id, value) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::DueDate(id, value),
			);
		}
		Action::DueAllDay(id, all_day) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::DueAllDay(id, all_day),
			);
		}
		Action::DueStart(id, value) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::DueStart(id, value),
			);
		}
		Action::DueEnd(id, value) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::DueEnd(id, value),
			);
		}
		Action::EditTitle => {
			return crate::task_editor::update(state, crate::task_editor::Action::EditTitle);
		}
		Action::Deadline(id, value) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::Deadline(id, value),
			);
		}
		Action::SaveDeadline(id) => {
			return crate::task_editor::update(state, crate::task_editor::Action::SaveDeadline(id));
		}
		Action::NewWeekTask => {
			let day = state.calendar.day;
			let open = crate::inbox::update(state, crate::inbox::Action::NewTask);
			let start = calendar::at_clock(day, 9, 0);
			let due = crate::inbox::update(state, crate::inbox::Action::Due(day.to_string()));
			let schedule = update(
				state,
				Action::Schedule(
					crate::inbox::DRAFT_ID.into(),
					start,
					start + chrono::Duration::minutes(30),
				),
			);
			return Task::batch([open, due, schedule]);
		}
		Action::DropWeekList { target, after } => {
			return drag::update(state, drag::Action::DropWeekList { target, after });
		}
		Action::DropAt { start, .. } => return drag::update(state, drag::Action::DropAt { start }),
		Action::DropTime(start) => return drag::update(state, drag::Action::DropTime(start)),
		Action::Schedule(id, start, end) => {
			if end <= start {
				state.calendar.error = Some("The task must end after it starts".into());
				return Task::none();
			}
			(id, Change::Time(start, end))
		}
		Action::RefreshBlock(id) => return refresh_block(state, &id),
		Action::BacklogDestination(id) => {
			state.config.calendar.backlog_list = id;
			return crate::app::queue_save(state);
		}
		Action::TriagedDestination(id) => {
			state.config.calendar.triaged_list = id;
			return crate::app::queue_save(state);
		}
		Action::RequireEstimate(required) => {
			state.config.calendar.triage.estimate = required;
			return crate::app::queue_save(state);
		}
		Action::RequireProject(required) => {
			state.config.calendar.triage.project = required;
			return crate::app::queue_save(state);
		}
		Action::RequirePriority(required) => {
			state.config.calendar.triage.priority = required;
			return crate::app::queue_save(state);
		}
		Action::Close => return close(state),
		Action::CancelDrag => return drag::update(state, drag::Action::CancelDrag),
		Action::Details(id) => {
			state.calendar.open_event = Some(id);
			return Task::none();
		}
		Action::BacklogQuery(query) => {
			state.timeblocking.backlog_query = query;
			return Task::none();
		}
		Action::BacklogList(list) => {
			state.timeblocking.backlog_list = list;
			return Task::none();
		}
		Action::EditEstimate(id) => {
			return crate::task_editor::update(state, crate::task_editor::Action::EditEstimate(id));
		}
		Action::CloseEstimate => {
			return crate::task_editor::update(state, crate::task_editor::Action::CloseEstimate);
		}
		Action::Estimate(value) => {
			return crate::task_editor::update(state, crate::task_editor::Action::Estimate(value));
		}
		Action::Drag(id, position) => return drag::update(state, drag::Action::Drag(id, position)),
		Action::DragMoved(position) => {
			return drag::update(state, drag::Action::DragMoved(position));
		}
		Action::DropBlock(block) => return drag::update(state, drag::Action::DropBlock(block)),
		Action::DropBacklogSection(section) => {
			return drag::update(state, drag::Action::DropBacklogSection(section));
		}
		Action::SetBacklogSection(id, section) => {
			let Some(task) = event(state, &id).filter(|task| {
				task.is_task()
					&& !task.completed
					&& !is_assigned(task)
					&& is_backlog_list(state, &task.calendar_id)
			}) else {
				return Task::none();
			};
			if metadata(task).backlog_section == section {
				return Task::none();
			}
			(id, Change::BacklogSection(section))
		}
		Action::DropCalendarTask(target) => {
			return drag::update(state, drag::Action::DropCalendarTask(target));
		}
		Action::DropTask(target) => return drag::update(state, drag::Action::DropTask(target)),
		Action::DropDay => return drag::update(state, drag::Action::DropDay),
		Action::SavedMany(key, result) => {
			return crate::task_operations::saved_many(state, key, result);
		}
		Action::Saved(key, result) => return crate::task_operations::saved(state, key, result),
		Action::SaveEstimate(id) => {
			return crate::task_editor::update(state, crate::task_editor::Action::SaveEstimate(id));
		}
		Action::SetEstimate(id, minutes) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::SetEstimate(id, minutes),
			);
		}
		Action::SetDeadline(id, deadline) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::SetDeadline(id, deadline),
			);
		}
		Action::BacklogPicker(id) => {
			if !state.task_operations.busy.is_empty()
				|| !event(state, &id).is_some_and(|task| task.is_task())
			{
				return Task::none();
			}
			state.task_editor_mut().backlog_picker =
				(state.task_editor().backlog_picker.as_deref() != Some(id.as_str())).then_some(id);
			return Task::none();
		}
		Action::Backlog(id) => (id, Change::Backlog(None)),
		Action::MoveToBacklog(id, section) => {
			if !state.task_operations.busy.is_empty() {
				return Task::none();
			}
			state.task_editor_mut().backlog_picker = None;
			(id, Change::Backlog(Some(section)))
		}
		Action::MoveToInbox(id) => {
			if !state.task_operations.busy.is_empty() {
				return Task::none();
			}
			state.task_editor_mut().backlog_picker = None;
			(id, Change::Inbox)
		}
		Action::Triage(id) => (id, Change::Triaged),
		Action::Unassign(id) => (id, Change::Unassign),
		Action::ScheduleDay(id, day) => (id, Change::Day(day)),
		Action::Assign(id, block) => {
			let Some(event) = event(state, &block).filter(|e| is_block(e, &state.config.calendar))
			else {
				return Task::none();
			};
			let key = block_key(event);
			let hour = day_start_hour(state);
			if self::event(state, &id).is_some_and(|task| in_block(task, event, hour)) {
				return Task::none();
			}
			// Something that repeats is asked about: just this occurrence,
			// or this one and the later ones.
			if self::event(state, &id).is_some_and(|task| task.is_task() && task.is_recurring()) {
				if !state.task_operations.busy.is_empty() {
					return Task::none();
				}
				state.timeblocking.pending_recurring = Some(PendingRecurring {
					item: RecurringItem::Task(id),
					block,
				});
				return Task::none();
			}
			if block_repeats(event) && !state.timeblocking.one_off {
				join = Change::join_block_series(event);
			}
			(id, Change::Block(key, event.start, event.end))
		}
		Action::CancelRecurring => {
			return recurring::update(state, recurring::Action::CancelRecurring);
		}
		Action::OneOff(one_off) => {
			return recurring::update(state, recurring::Action::OneOff(one_off));
		}
		Action::StopRepeating(id) => {
			return recurring::update(state, recurring::Action::StopRepeating(id));
		}
		Action::RepeatWithBlock(task, block) => {
			return recurring::update(state, recurring::Action::RepeatWithBlock(task, block));
		}
		Action::ConfirmRepeatWithBlock => {
			return recurring::update(state, recurring::Action::ConfirmRepeatWithBlock);
		}
		Action::CancelRepeatWithBlock => {
			return recurring::update(state, recurring::Action::CancelRepeatWithBlock);
		}
		Action::MoveRecurring(future) => {
			return recurring::update(state, recurring::Action::MoveRecurring(future));
		}
		Action::Title(value) => {
			return crate::task_editor::update(state, crate::task_editor::Action::Title(value));
		}
		Action::ColorInput(id, value) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::ColorInput(id, value),
			);
		}
		Action::SaveColor(id, value) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::SaveColor(id, value),
			);
		}
		Action::SaveDescription(id, value) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::SaveDescription(id, value),
			);
		}
		Action::SaveTitle(id) => {
			return crate::task_editor::update(state, crate::task_editor::Action::SaveTitle(id));
		}
		Action::ProjectAdd(id, project) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::ProjectAdd(id, project),
			);
		}
		Action::ProjectRemove(id, project) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::ProjectRemove(id, project),
			);
		}
		Action::ProjectQuery(value) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::ProjectQuery(value),
			);
		}
		Action::ProjectPicker(id, open) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::ProjectPicker(id, open),
			);
		}
		Action::LabelDraft(value) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::LabelDraft(value),
			);
		}
		Action::AddLabel(id, label) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::AddLabel(id, label),
			);
		}
		Action::RemoveLabel(id, label) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::RemoveLabel(id, label),
			);
		}
		Action::Priority(id, priority) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::Priority(id, priority),
			);
		}
		Action::Location(id, location) => {
			return crate::task_editor::update(
				state,
				crate::task_editor::Action::Location(id, location),
			);
		}
		Action::OpenTodo => {
			state.timeblocking.todo_open = true;
			return iced::widget::operation::focus(iced::widget::Id::new("time-block-todo"));
		}
		Action::CloseTodo => {
			if state.task_operations.busy.adding_todo() {
				return Task::none();
			}
			state.timeblocking.todo_open = false;
			state.timeblocking.todo_draft.clear();
			state.timeblocking.todo_error = None;
			return Task::none();
		}
		Action::TodoDraft(value) => {
			state.timeblocking.todo_draft = value;
			state.timeblocking.todo_error = None;
			return Task::none();
		}
		Action::AddTodo(block_id) => return add_todo(state, &block_id),
		Action::AddHabit(habit_id, block_id) => {
			let Some(habit) = state.habits.library.habit(&habit_id).cloned() else {
				return Task::none();
			};
			let Some(block) = event(state, &block_id) else {
				return Task::none();
			};
			if assigned_all(state, block)
				.iter()
				.any(|task| metadata(task).habit_id.as_deref() == Some(&habit_id))
			{
				return Task::none();
			}
			if state.calendar.events.iter().any(|event| {
				event.details.habit
					&& event.details.recurring_event_id.is_some()
					&& event.details.habit_id.as_deref() == Some(&habit_id)
			}) {
				if state.task_operations.busy.is_empty() {
					state.timeblocking.pending_recurring = Some(PendingRecurring {
						item: RecurringItem::Habit(habit_id),
						block: block_id,
					});
				}
				return Task::none();
			}
			let repeat = !state.timeblocking.one_off && block_repeats(block);
			return add_block_item(
				state,
				&block_id,
				habit.name,
				habit.notes,
				Some(Metadata {
					habit_id: Some(habit_id),
					minutes: 30,
					..Default::default()
				}),
				Vec::new(),
				repeat,
			);
		}
		Action::AvailableHabitQuery(query) => {
			state.timeblocking.available_habit_query = query;
			return Task::none();
		}
		Action::AvailableQuery(query) => {
			state.timeblocking.available_query = query;
			return Task::none();
		}
		Action::AvailableAll(all) => {
			state.timeblocking.available_all = all;
			return Task::none();
		}
		Action::AvailablePlanned(show) => {
			state.timeblocking.available_planned = show;
			return Task::none();
		}
	};
	crate::task_operations::edit(state, id, change, join)
}

/// Timed tasks and calendar events, with locally saved board placement applied.
/// This ordering belongs to the board; it never changes time-block membership.
pub fn weekly_list_events(state: &State, day: NaiveDate) -> Vec<&Event> {
	let mut events = state
		.calendar
		.task_list_events_on(day, &state.config.calendar);
	events.retain(|event| !event.is_task() || !event.all_day);
	if let Some(order) = state
		.config
		.calendar
		.weekly_task_order
		.get(&day.to_string())
	{
		events.sort_by_key(|event| {
			order
				.iter()
				.position(|id| id == &event.id)
				.unwrap_or(usize::MAX)
		});
	}
	events
}

/// Saves the typed line as a Google Task and assigns it to the open block.
///
/// Time-block items are tasks, so a free-text todo is a task whose title is
/// whatever was typed. The write happens off the UI thread.
fn add_todo(state: &mut State, block_id: &str) -> Task<Message> {
	let title = state.timeblocking.todo_draft.trim().to_owned();
	if title.is_empty() {
		return Task::none();
	}
	if title.chars().count() > MAX_TODO_CHARS {
		state.timeblocking.todo_error =
			Some(format!("A todo can be at most {MAX_TODO_CHARS} characters"));
		return Task::none();
	}
	let repeat = !state.timeblocking.one_off && event(state, block_id).is_some_and(block_repeats);
	add_block_item(
		state,
		block_id,
		title,
		String::new(),
		None,
		Vec::new(),
		repeat,
	)
}

/// An occurrence to delete, and whether the ones after it go too.
type Deletion = (String, String, calendar::SaveScope);

/// Creates a task in the block, repeating with it when `repeat` and the
/// block repeats, then deletes `deletions`: the recurring occurrences it
/// replaces.
pub(crate) fn add_block_item(
	state: &mut State,
	block_id: &str,
	title: String,
	notes: String,
	initial_metadata: Option<Metadata>,
	deletions: Vec<Deletion>,
	repeat: bool,
) -> Task<Message> {
	let busy_key = if initial_metadata
		.as_ref()
		.is_some_and(|meta| meta.habit_id.is_some())
	{
		BusyKey::AddingHabit
	} else {
		BusyKey::AddingTodo
	};
	if !state.task_operations.busy.is_empty() {
		return Task::none();
	}
	let Some(block) = event(state, block_id)
		.filter(|event| is_block(event, &state.config.calendar))
		.cloned()
	else {
		return Task::none();
	};
	let key = block_key(&block);
	let start = block.start;
	let end = block.end;
	let join = repeat.then(|| Change::join_block_series(&block)).flatten();
	let settings = state.config.calendar.clone();
	let order = state
		.calendar
		.events
		.iter()
		.filter(|event| event.is_task())
		.map(|event| metadata(event).order)
		.max()
		.unwrap_or(0)
		.saturating_add(1_i64 << 32);
	let blocks: Vec<_> = state
		.calendar
		.events
		.iter()
		.filter(|event| is_block(event, &settings))
		.cloned()
		.collect();
	let related: Vec<_> = state
		.calendar
		.events
		.iter()
		.filter(|event| event.is_task())
		.cloned()
		.collect();
	state.timeblocking.todo_error = None;
	state.task_operations.revision += 1;
	crate::task_operations::begin(state, busy_key.clone());
	for (calendar_id, id, scope) in &deletions {
		forget_occurrences(state, calendar_id, id, *scope);
	}
	blocking::run(move || {
		let changed = create_and_assign(
			title,
			notes,
			initial_metadata,
			settings.clone(),
			key,
			start,
			end,
			order,
			blocks,
			related,
		)?;
		let changed = join_series(changed, &settings, join)?;
		for (calendar_id, id, scope) in deletions {
			calendar::delete_event_scope(&calendar_id, &id, scope, false)?;
		}
		Ok(changed)
	})
	.map(move |result| {
		Message::TaskOperations(crate::task_operations::Action::SavedMany(
			busy_key.clone(),
			result,
		))
	})
}

#[allow(clippy::too_many_arguments)]
fn create_and_assign(
	title: String,
	notes: String,
	metadata: Option<Metadata>,
	settings: CalendarSettings,
	key: String,
	start: DateTime<Local>,
	end: DateTime<Local>,
	order: i64,
	blocks: Vec<Event>,
	related: Vec<Event>,
) -> Result<Vec<Event>, String> {
	let task = timeparagon_core::task_edit::create_inbox_task(
		&settings,
		&title,
		&notes,
		Some(start.date_naive()),
		metadata,
	)?;
	save_and_reflow(
		task,
		settings,
		Change::Block(key, start, end),
		order,
		blocks,
		related,
	)
}

#[cfg(test)]
fn new_todo_event(
	list_id: &str,
	created: &calendar::api::CreatedTask,
	when: DateTime<Local>,
) -> Event {
	Event {
		id: format!("task:{}", created.id),
		calendar_id: ids::calendar_id(list_id),
		title: created.title.clone(),
		start: when,
		end: when + chrono::Duration::minutes(30),
		all_day: true,
		color: calendar::colors::TASK,
		completed: false,
		details: calendar::Details {
			task_updated_at: created.updated_at,
			link: created.link.clone(),
			..Default::default()
		},
	}
}

/// Why a task cannot be triaged yet, from the Inbox's required-field settings,
/// or `None` when it is ready to leave the Inbox.
pub(crate) fn triage_error(state: &State, id: &str) -> Option<String> {
	let task = event(state, id).filter(|event| event.is_task())?;
	task_edit::triage_error(task, &state.config.calendar.triage)
}

pub(crate) fn access(settings: &CalendarSettings) -> Result<String, GoogleError> {
	// Unit tests must never reach the user's real Google account.
	#[cfg(test)]
	{
		let _ = settings;
		Err(GoogleError::auth("Google access is disabled in tests"))
	}
	#[cfg(not(test))]
	access_google(settings)
}

#[cfg(not(test))]
fn access_google(settings: &CalendarSettings) -> Result<String, GoogleError> {
	task_edit::google_access(settings)
}

fn refresh_block(state: &mut State, id: &str) -> Task<Message> {
	let Some(block) = event(state, id)
		.filter(|e| is_block(e, &state.config.calendar))
		.cloned()
	else {
		return Task::none();
	};
	let tasks: Vec<_> = assigned_all(state, &block).into_iter().cloned().collect();
	// Tasks repeating with the block are moved after it by the reconciler
	// (see `task_edit::reflow`); only the one-offs are written here.
	let Some(first) = tasks
		.iter()
		.find(|task| !is_follower(task))
		.map(|task| task.id.clone())
	else {
		return Task::none();
	};
	let settings = state.config.calendar.clone();
	state.task_operations.revision += 1;
	crate::task_operations::begin(state, first.clone());
	blocking::run(move || reflow(&block, &tasks, &settings)).map(move |result| {
		Message::TaskOperations(crate::task_operations::Action::SavedMany(
			first.clone().into(),
			result,
		))
	})
}

/// Record task edits locally; only the operations worker sends them to Google.
fn save(
	task: Event,
	settings: &CalendarSettings,
	change: Change,
	order: i64,
) -> Result<Event, String> {
	Ok(task_edit::save(task, settings, change, order)?)
}

pub(crate) fn save_and_reflow(
	task: Event,
	settings: CalendarSettings,
	change: Change,
	order: i64,
	blocks: Vec<Event>,
	related: Vec<Event>,
) -> Result<Vec<Event>, String> {
	task_edit::save_and_reflow(task, settings, change, order, blocks, related)
}

/// Has the task saved first in `changed` repeat with its block's series.
pub(crate) fn join_series(
	mut changed: Vec<Event>,
	settings: &CalendarSettings,
	join: Option<Change>,
) -> Result<Vec<Event>, String> {
	if let Some(join) = join
		&& let Some(task) = changed.first_mut()
	{
		let order = metadata(task).order;
		*task = save(task.clone(), settings, join, order)?;
	}
	Ok(changed)
}

fn reflow(
	block: &Event,
	tasks: &[Event],
	settings: &CalendarSettings,
) -> Result<Vec<Event>, String> {
	task_edit::reflow(block, tasks, settings, false)
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::TimeZone;

	#[test]
	fn save_feedback_reaches_its_editor_while_creation_owns_another_form() {
		for success in [false, true] {
			let mut state = estimate_state();
			let id = "task:deep";
			state.calendar.open_event = Some(id.into());
			state.task_editor.title = "Edited title".into();
			state.task_editor.editing_title = true;
			state.task_editor.renaming = Some(id.into());
			state.task_editor.keep_open = Some(id.into());
			crate::task_operations::begin(&mut state, id);
			let _ = crate::inbox::update(&mut state, crate::inbox::Action::NewTask);
			state.task_editor_mut().date = "2026-12-01".into();
			state.task_editor_mut().title = "Creation title".into();
			let creation_id = state.task_editor().id;
			let revision = state.task_operations.revision;
			let mut saved = state.calendar.event(id).unwrap().clone();
			saved.title = "Edited title".into();
			let result = if success {
				Ok(vec![saved])
			} else {
				Err("Offline".into())
			};
			let _ = crate::task_operations::saved_many(&mut state, id.into(), result);
			assert!(state.inbox.draft.is_some());
			assert_eq!(state.task_editor().id, creation_id);
			assert_eq!(state.task_editor().title, "Creation title");
			assert_eq!(state.task_editor().date, "2026-12-01");
			assert!(state.task_editor().title_error.is_none());
			assert!(state.task_operations.busy.is_empty());
			assert_eq!(state.task_operations.revision, revision + 1);
			assert_eq!(state.timeblocking.focus.as_deref(), Some("standup"));
			assert_eq!(state.calendar.open_event.as_deref(), Some(id));
			assert!(state.task_editor.renaming.is_none());
			if success {
				assert_eq!(state.calendar.event(id).unwrap().title, "Edited title");
				assert!(!state.task_editor.editing_title);
			} else {
				assert_eq!(state.task_editor.title_error.as_deref(), Some("Offline"));
			}
			let _ = crate::inbox::update(&mut state, crate::inbox::Action::Cancel);
			assert_eq!(state.task_editor().title, "Edited title");
		}
	}

	#[test]
	fn a_late_save_updates_data_without_changing_a_newer_editor() {
		let mut state = estimate_state();
		let id = "task:deep";
		crate::task_operations::begin(&mut state, id);
		let mut saved = state.calendar.event(id).unwrap().clone();
		saved.title = "Saved in background".into();
		// Reopening even the same task creates a distinct editor instance.
		let _ = crate::app::update(
			&mut state,
			Message::Event(crate::app::EventMessage::EventOpened(id.into())),
		);
		state.task_editor.title = "New unsaved title".into();
		state.task_editor.editing_title = true;
		let _ = crate::task_operations::saved(&mut state, id.into(), Ok(saved));
		assert_eq!(
			state.calendar.event(id).unwrap().title,
			"Saved in background"
		);
		assert_eq!(state.calendar.open_event.as_deref(), Some(id));
		assert_eq!(state.task_editor.title, "New unsaved title");
		assert!(state.task_editor.editing_title);
		assert!(state.task_operations.busy.is_empty());
	}

	#[test]
	fn reflow_results_do_not_close_an_unrelated_details_pane() {
		let mut state = estimate_state();
		state.calendar.open_event = Some("standup".into());
		let saved = state.calendar.event("task:deep").unwrap().clone();
		let related = state.calendar.event("lunch").unwrap().clone();
		crate::task_operations::begin(&mut state, "task:deep");
		let _ = crate::task_operations::saved_many(
			&mut state,
			"task:deep".into(),
			Ok(vec![saved, related]),
		);
		assert_eq!(state.calendar.open_event.as_deref(), Some("standup"));
	}

	#[test]
	fn creation_does_not_suspend_the_focused_block_or_work_clock() {
		let mut state = estimate_state();
		state.timeblocking.block = Some("standup".into());
		state.timeblocking.dragging = Some("task:deep".into());
		let _ = crate::inbox::update(&mut state, crate::inbox::Action::NewTask);
		let _ = crate::focus::update(&mut state, crate::focus::Action::WorkTick);
		let tick = state.work_tracking.work_now;
		assert!(tick.is_some());
		assert_eq!(state.timeblocking.focus.as_deref(), Some("standup"));
		assert_eq!(state.timeblocking.block.as_deref(), Some("standup"));
		let _ = crate::inbox::update(&mut state, crate::inbox::Action::Cancel);
		assert_eq!(state.work_tracking.work_now, tick);
		assert_eq!(state.timeblocking.dragging.as_deref(), Some("task:deep"));
	}

	#[test]
	fn week_list_moves_keep_clock_duration_and_the_planning_day_boundary() {
		use chrono::Timelike;
		let mut state = estimate_state();
		let task = state.calendar.event_mut("task:deep").unwrap();
		let source = NaiveDate::from_ymd_opt(2026, 10, 14).unwrap();
		let destination = source + chrono::Duration::days(3);
		task.all_day = false;
		task.start = calendar::at_clock(source, 23, 30);
		task.end = task.start + chrono::Duration::minutes(90);
		let (start, end) = week_drop_times(task, destination, 0).unwrap();
		assert_eq!(start.date_naive(), destination);
		assert_eq!(start.time(), task.start.time());
		assert_eq!(end - start, task.end - task.start);
		task.start = calendar::at_clock(source.succ_opt().unwrap(), 1, 30);
		task.end = task.start + chrono::Duration::minutes(90);
		let (start, end) = week_drop_times(task, destination, 3).unwrap();
		assert_eq!(calendar::day_at(start, 3), destination);
		assert_eq!(start.hour(), 1);
		assert_eq!(end - start, task.end - task.start);
		// Tasks dragged from the inbox become timed cards rather than disappearing.
		task.all_day = true;
		let (start, end) = week_drop_times(task, destination, 0).unwrap();
		assert_eq!(start, calendar::at_clock(destination, 9, 0));
		assert_eq!((end - start).num_minutes(), 35);
	}

	#[test]
	fn week_list_cross_day_placement_commits_only_after_a_successful_save() {
		for success in [false, true] {
			let mut state = estimate_state();
			let source = state.calendar.day;
			let destination = source.succ_opt().unwrap();
			let id = "task:deep".to_owned();
			let original = state.calendar.event(&id).unwrap().clone();
			state
				.config
				.calendar
				.weekly_task_order
				.insert(source.to_string(), vec![id.clone()]);
			state.task_operations.pending_week_drop = Some(WeekListPlacement {
				task: id.clone(),
				day: destination,
				target: None,
				after: true,
			});
			let result = if success {
				let mut moved = original.clone();
				let (start, end) = week_drop_times(&moved, destination, 0).unwrap();
				moved.id = "task:new-list-id".into();
				moved.start = start;
				moved.end = end;
				moved.all_day = false;
				Ok(moved)
			} else {
				Err("Could not save".to_owned())
			};
			let _ = update(&mut state, Action::Saved(id.clone().into(), result));
			assert!(state.task_operations.pending_week_drop.is_none());
			if success {
				assert_eq!(
					state.config.calendar.weekly_task_order[&destination.to_string()],
					vec!["task:new-list-id"]
				);
				assert!(
					!state.config.calendar.weekly_task_order[&source.to_string()].contains(&id)
				);
			} else {
				assert!(
					!state
						.config
						.calendar
						.weekly_task_order
						.contains_key(&destination.to_string())
				);
				assert_eq!(state.calendar.event(&id), Some(&original));
				assert_eq!(state.calendar.error.as_deref(), Some("Could not save"));
			}
		}
	}

	#[test]
	fn backlog_destination_setting_can_be_customized_and_reset() {
		let mut state = estimate_state();
		assert_eq!(state.config.calendar.backlog_list, None);
		let _ = update(
			&mut state,
			Action::BacklogDestination(Some("tasks:custom".into())),
		);
		let serialized = toml::to_string(&state.config).unwrap();
		let restored: crate::config::Config = toml::from_str(&serialized).unwrap();
		assert_eq!(
			restored.calendar.backlog_list.as_deref(),
			Some("tasks:custom")
		);
		let _ = update(&mut state, Action::BacklogDestination(None));
		assert_eq!(state.config.calendar.backlog_list, None);
	}

	#[test]
	fn the_triaged_list_and_required_fields_round_trip() {
		let mut state = estimate_state();
		let _ = update(
			&mut state,
			Action::TriagedDestination(Some("tasks:custom".into())),
		);
		let _ = update(&mut state, Action::RequireEstimate(false));
		let _ = update(&mut state, Action::RequireProject(false));
		let _ = update(&mut state, Action::RequirePriority(false));
		let serialized = toml::to_string(&state.config).unwrap();
		let restored: crate::config::Config = toml::from_str(&serialized).unwrap();
		assert_eq!(
			restored.calendar.triaged_list.as_deref(),
			Some("tasks:custom")
		);
		assert!(!restored.calendar.triage.estimate);
		assert!(!restored.calendar.triage.project);
		assert!(!restored.calendar.triage.priority);
	}

	#[test]
	fn triaging_lists_every_missing_required_field_and_keeps_the_task_in_the_inbox() {
		let mut state = estimate_state();
		// The fixture has an estimate but neither a project nor a priority.
		let _ = update(&mut state, Action::Triage("task:deep".into()));
		assert!(state.task_operations.busy.is_empty());
		let error = state.inbox.error.as_deref().unwrap();
		assert!(error.contains("a project"), "{error}");
		assert!(error.contains("a priority"), "{error}");
		assert!(!error.contains("an estimate"), "{error}");

		// Relaxing the requirements lets it through.
		let _ = update(&mut state, Action::RequireProject(false));
		let _ = update(&mut state, Action::RequirePriority(false));
		let _ = update(&mut state, Action::RequireEstimate(false));
		let _ = update(&mut state, Action::Triage("task:deep".into()));
		assert!(state.inbox.error.is_none());
		assert!(state.task_operations.busy.contains_task("task:deep"));
	}

	#[test]
	fn backlog_and_triaged_lists_match_their_default_names_or_a_chosen_list() {
		let mut state = estimate_state();
		assert!(is_backlog_list(&state, "tasks:Backlog"));
		assert!(!is_backlog_list(&state, "tasks:Inbox"));
		state.config.calendar.backlog_list = Some("tasks:park".into());
		assert!(is_backlog_list(&state, "tasks:park"));
		assert!(!is_backlog_list(&state, "tasks:Backlog"));
	}

	#[test]
	fn a_changed_deadline_is_saved_and_keeps_the_task_open() {
		let mut state = estimate_state();
		let _ = update(
			&mut state,
			Action::Deadline("task:deep".into(), "2026-10-29 14:45".into()),
		);
		assert_eq!(state.task_editor().deadline, "2026-10-29 14:45");
		assert!(state.task_operations.busy.contains_task("task:deep"));
		assert_eq!(state.task_editor().keep_open.as_deref(), Some("task:deep"));
	}

	#[test]
	fn the_due_fields_save_a_day_or_a_time_as_they_change() {
		let mut state = estimate_state();
		let task = state.calendar.event("task:deep").unwrap().clone();
		state.task_editor_mut().seed_schedule(&task);
		let _ = update(
			&mut state,
			Action::DueDate("task:deep".into(), "2026-10-29".into()),
		);
		assert!(state.calendar.error.is_none());
		assert!(state.task_operations.busy.contains_task("task:deep"));
		assert_eq!(state.task_editor().keep_open.as_deref(), Some("task:deep"));

		// A second change waits for the first, and does not claim to have happened.
		let _ = update(
			&mut state,
			Action::DueDate("task:deep".into(), "2026-11-01".into()),
		);
		assert_eq!(state.task_editor().date, "2026-10-29");

		let mut state = estimate_state();
		state.task_editor_mut().date = "2026-10-29".into();
		state.task_editor_mut().start_time = String::new();
		let _ = update(&mut state, Action::DueAllDay("task:deep".into(), false));
		assert_eq!(state.task_editor().start_time, "09:00");
		assert_eq!(state.task_editor().end_time, "09:30");
		assert!(state.task_operations.busy.contains_task("task:deep"));
	}

	#[test]
	fn moving_a_due_start_keeps_the_tasks_length() {
		assert_eq!(
			end_keeping_length("09:00", "09:45", "14:30").as_deref(),
			Some("15:15")
		);
		assert_eq!(
			end_keeping_length("09:00", "10:00", "23:30").as_deref(),
			Some("00:30")
		);
		assert_eq!(end_keeping_length("", "10:00", "11:00"), None);
	}

	#[test]
	fn clearing_a_due_date_keeps_the_editor_and_the_task_list() {
		let mut state = estimate_state();
		let _ = update(
			&mut state,
			Action::DueDate("task:deep".into(), String::new()),
		);
		assert!(state.calendar.error.is_none());
		assert!(state.task_operations.busy.contains_task("task:deep"));
		assert_eq!(state.task_editor().keep_open.as_deref(), Some("task:deep"));
	}

	#[test]
	fn invalid_deadline_input_leaves_the_task_untouched() {
		let mut state = estimate_state();
		state.task_editor_mut().deadline = "2026-02-30".into();
		let before = state.calendar.event("task:deep").unwrap().clone();
		let _ = update(&mut state, Action::SaveDeadline("task:deep".into()));
		assert_eq!(state.calendar.event("task:deep"), Some(&before));
		assert!(state.task_operations.busy.is_empty());
		assert!(
			state
				.calendar
				.error
				.as_deref()
				.unwrap()
				.contains("YYYY-MM-DD")
		);
	}

	#[test]
	fn overdue_uses_metadata_deadline_independently_of_scheduling() {
		let mut task = calendar::sample_snapshot().events.remove(0);
		task.id = "task:deadline".into();
		task.calendar_id = "tasks:list".into();
		task.start = crate::clock::now() + chrono::Duration::days(5);
		let yesterday = (crate::clock::now() - chrono::Duration::days(1)).date_naive();
		task.details.description = "Notes".into();
		task.details
			.task_data
			.get_or_insert_with(|| timeparagon_core::task_db::TaskData {
				id: 100,
				metadata: Default::default(),
			})
			.metadata
			.deadline = Some(
			timeparagon_core::calendar::deadline::parse_input(&format!("{yesterday}")).unwrap(),
		);
		assert!(overdue(&task));
		task.details.task_data.as_mut().unwrap().metadata.deadline = None;
		task.start = crate::clock::now() - chrono::Duration::days(5);
		assert!(!overdue(&task));
	}

	fn estimate_state() -> State {
		let mut state = State::blank();
		state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
		let task = state.calendar.event_mut("deep").unwrap();
		task.id = "task:deep".into();
		task.calendar_id = "tasks:list".into();
		task.details.description = "Notes".into();
		task.details.task_data = Some(crate::task_db::TaskData {
			id: 100,
			metadata: Metadata {
				minutes: 35,
				..Default::default()
			},
		});
		state.timeblocking.focus = Some("standup".into());
		state
	}

	/// "standup" as a time block, with "deep" as today's occurrence of a
	/// recurring calendar task and "deep_next" as tomorrow's.
	fn recurring_state() -> State {
		let mut state = estimate_state();
		let block = state.calendar.event("standup").unwrap().clone();
		state.config.calendar.app_calendar = Some(block.calendar_id.clone());
		let task = state.calendar.event_mut("task:deep").unwrap();
		task.id = "deep".into();
		task.calendar_id = block.calendar_id.clone();
		task.details.recurring_event_id = Some("deep-series".into());
		let mut next = task.clone();
		next.id = "deep_next".into();
		next.start += chrono::Duration::days(1);
		next.end += chrono::Duration::days(1);
		state.calendar.events.push(next);
		state
	}

	fn habit_occurrence(state: &State, id: &str, days: i64) -> Event {
		let mut habit = state.calendar.event("standup").unwrap().clone();
		habit.id = id.into();
		habit.start += chrono::Duration::days(days);
		habit.end += chrono::Duration::days(days);
		habit.details.habit = true;
		habit.details.habit_id = Some("read".into());
		habit.details.recurring_event_id = Some("read-series".into());
		habit
	}

	#[test]
	fn a_recurring_task_asks_before_moving_into_a_block_and_cancel_keeps_it() {
		let mut state = recurring_state();
		let _ = update(&mut state, Action::Assign("deep".into(), "standup".into()));
		assert_eq!(
			state.timeblocking.pending_recurring,
			Some(PendingRecurring {
				item: RecurringItem::Task("deep".into()),
				block: "standup".into(),
			})
		);
		assert!(state.task_operations.busy.is_empty());
		let _ = update(&mut state, Action::CancelRecurring);
		assert!(state.timeblocking.pending_recurring.is_none());
		assert!(state.calendar.event("deep").is_some());
		assert!(state.calendar.event("deep_next").is_some());
		assert!(
			metadata(state.calendar.event("deep").unwrap())
				.block
				.is_none()
		);
	}

	/// `recurring_state`, with the block repeating and "deep" repeating with
	/// it: one occurrence today in the block, one tomorrow.
	fn following_state() -> State {
		let mut state = recurring_state();
		let block = state.calendar.event_mut("standup").unwrap();
		block.details.recurring_event_id = Some("standup-series".into());
		let calendar = block.calendar_id.clone();
		for id in ["deep", "deep_next"] {
			let task = state.calendar.event_mut(id).unwrap();
			task.details.task_data = Some(crate::task_db::TaskData {
				id: 100,
				metadata: Metadata {
					block_series: Some(format!("{calendar}/standup-series")),
					minutes: 30,
					..Default::default()
				},
			});
		}
		state
	}

	#[test]
	fn a_task_repeating_with_its_block_is_in_that_days_occurrence_only() {
		let state = following_state();
		let block = state.calendar.event("standup").unwrap();
		let ids: Vec<_> = assigned(&state, block)
			.into_iter()
			.map(|task| task.id.as_str())
			.collect();
		assert_eq!(ids, ["deep"]);
		assert_eq!(
			block_key_of(&state, state.calendar.event("deep").unwrap()),
			Some(block_key(block))
		);
	}

	#[test]
	fn assigning_a_task_to_the_block_it_is_in_does_nothing() {
		let mut state = following_state();
		let _ = update(&mut state, Action::Assign("deep".into(), "standup".into()));
		assert!(state.timeblocking.pending_recurring.is_none());
		assert!(state.task_operations.busy.is_empty());
	}

	#[test]
	fn stopping_a_repeating_task_deletes_this_and_later_occurrences() {
		let mut state = following_state();
		let _ = update(&mut state, Action::StopRepeating("deep".into()));
		assert!(state.task_operations.busy.contains_task("deep"));
		// They stay in view, locked by the local edits, until Google
		// confirms the delete.
		assert!(state.calendar.event("deep").is_some());
		assert!(state.calendar.event("deep_next").is_some());
		// Only a task repeating with a block can be stopped this way.
		let mut state = recurring_state();
		let _ = update(&mut state, Action::StopRepeating("deep".into()));
		assert!(state.task_operations.busy.is_empty());
	}

	/// `recurring_state`, with the block repeating and "deep" a one-off in
	/// today's occurrence of it.
	fn one_off_state() -> State {
		let mut state = recurring_state();
		state
			.calendar
			.events
			.retain(|event| event.id != "deep_next");
		let block = state.calendar.event_mut("standup").unwrap();
		block.details.recurring_event_id = Some("standup-series".into());
		let key = block_key(state.calendar.event("standup").unwrap());
		let task = state.calendar.event_mut("deep").unwrap();
		task.details.recurring_event_id = None;
		task.details.task_data.as_mut().unwrap().metadata.block = Some(key);
		state
	}

	fn repeat_deep(state: &mut State) {
		let _ = update(
			state,
			Action::RepeatWithBlock("deep".into(), "standup".into()),
		);
	}

	#[test]
	fn a_one_off_in_a_repeating_block_can_start_repeating_with_it() {
		let mut state = one_off_state();
		repeat_deep(&mut state);
		assert!(state.timeblocking.pending_join.is_none());
		assert!(state.task_operations.busy.contains_task("deep"));

		// Not a task repeating with the block already, nor one in a block
		// that does not repeat.
		let mut following = following_state();
		repeat_deep(&mut following);
		let mut single = one_off_state();
		single
			.calendar
			.event_mut("standup")
			.unwrap()
			.details
			.recurring_event_id = None;
		repeat_deep(&mut single);
		for state in [following, single] {
			assert!(state.task_operations.busy.is_empty());
			assert!(state.timeblocking.pending_join.is_none());
		}
	}

	#[test]
	fn a_habit_repeating_on_its_own_asks_before_repeating_with_the_block() {
		let habit_one_off = || {
			let mut state = one_off_state();
			let task = state.calendar.event_mut("deep").unwrap();
			task.details.task_data.as_mut().unwrap().metadata.habit_id = Some("read".into());
			state
		};
		// Without a repeating event of its own, it goes ahead at once.
		let mut plain = habit_one_off();
		repeat_deep(&mut plain);
		assert!(plain.timeblocking.pending_join.is_none());
		assert!(plain.task_operations.busy.contains_task("deep"));

		let mut state = habit_one_off();
		let occurrence = habit_occurrence(&state, "read_1", 1);
		state.habits.series.push(crate::habits::Series {
			id: "read-series".into(),
			calendar_id: occurrence.calendar_id.clone(),
			title: "Read".into(),
			notes: String::new(),
			start: occurrence.start,
			end: occurrence.end,
			recurrence: crate::habits::Recurrence::from_lines(&["RRULE:FREQ=DAILY".into()]),
			color_id: None,
			habit_id: Some("read".into()),
		});
		repeat_deep(&mut state);
		let pending = Some(PendingJoin {
			task: "deep".into(),
			block: "standup".into(),
		});
		assert_eq!(state.timeblocking.pending_join, pending);
		assert!(state.task_operations.busy.is_empty());
		let _ = update(&mut state, Action::CancelRepeatWithBlock);
		assert!(state.timeblocking.pending_join.is_none());
		assert!(state.task_operations.busy.is_empty());

		repeat_deep(&mut state);
		let _ = update(&mut state, Action::ConfirmRepeatWithBlock);
		assert!(state.timeblocking.pending_join.is_none());
		assert!(state.task_operations.busy.contains_task("deep"));
	}

	#[test]
	fn the_repeat_choice_is_remembered() {
		let mut state = recurring_state();
		assert!(!state.timeblocking.one_off);
		let _ = update(&mut state, Action::OneOff(true));
		assert!(state.timeblocking.one_off);
	}

	#[test]
	fn a_task_that_does_not_repeat_moves_without_asking() {
		let mut state = recurring_state();
		let task = state.calendar.event_mut("deep").unwrap();
		task.details.recurring_event_id = None;
		let _ = update(&mut state, Action::Assign("deep".into(), "standup".into()));
		assert!(state.timeblocking.pending_recurring.is_none());
		assert!(state.task_operations.busy.contains_task("deep"));
	}

	#[test]
	fn moving_a_recurring_task_only_drops_just_that_occurrence() {
		let mut state = recurring_state();
		let _ = update(&mut state, Action::Assign("deep".into(), "standup".into()));
		let _ = update(&mut state, Action::MoveRecurring(false));
		assert!(state.timeblocking.pending_recurring.is_none());
		assert!(state.calendar.event("deep").is_none());
		assert!(state.calendar.event("deep_next").is_some());
		assert!(!state.task_operations.busy.is_empty());
	}

	#[test]
	fn moving_a_recurring_task_and_deleting_future_drops_the_later_occurrences() {
		let mut state = recurring_state();
		let _ = update(&mut state, Action::Assign("deep".into(), "standup".into()));
		let _ = update(&mut state, Action::MoveRecurring(true));
		assert!(state.calendar.event("deep").is_none());
		assert!(state.calendar.event("deep_next").is_none());
	}

	/// `recurring_state` with a "read" habit whose repeating event has
	/// occurrences yesterday, on the block's day, and in two days.
	fn habit_state() -> State {
		let mut state = recurring_state();
		state
			.habits
			.library
			.habits
			.push(crate::habits::Habit::new("read", "Read a book"));
		let earlier = habit_occurrence(&state, "read_earlier", -1);
		let today = habit_occurrence(&state, "read_today", 0);
		let later = habit_occurrence(&state, "read_later", 2);
		state.calendar.events.extend([earlier, today, later]);
		state
	}

	#[test]
	fn a_habit_with_a_repeating_event_asks_and_replaces_its_occurrences() {
		let mut only = habit_state();
		let _ = update(&mut only, Action::AddHabit("read".into(), "standup".into()));
		assert_eq!(
			only.timeblocking.pending_recurring,
			Some(PendingRecurring {
				item: RecurringItem::Habit("read".into()),
				block: "standup".into(),
			})
		);
		assert!(only.task_operations.busy.is_empty());
		let _ = update(&mut only, Action::MoveRecurring(false));
		assert!(only.calendar.event("read_today").is_none());
		assert!(only.calendar.event("read_earlier").is_some());
		assert!(only.calendar.event("read_later").is_some());
		assert!(only.task_operations.busy.adding_habit());

		let mut future = habit_state();
		let _ = update(
			&mut future,
			Action::AddHabit("read".into(), "standup".into()),
		);
		let _ = update(&mut future, Action::MoveRecurring(true));
		assert!(future.calendar.event("read_today").is_none());
		assert!(future.calendar.event("read_later").is_none());
		assert!(future.calendar.event("read_earlier").is_some());
	}

	#[test]
	fn a_habit_without_a_repeating_event_is_added_without_asking() {
		let mut state = recurring_state();
		state
			.habits
			.library
			.habits
			.push(crate::habits::Habit::new("read", "Read a book"));
		let _ = update(
			&mut state,
			Action::AddHabit("read".into(), "standup".into()),
		);
		assert!(state.timeblocking.pending_recurring.is_none());
		assert!(state.task_operations.busy.adding_habit());
	}

	#[test]
	fn a_distraction_is_recorded_against_the_focused_block_and_can_be_undone() {
		let mut state = estimate_state();
		let key = block_key(state.calendar.event("standup").unwrap());
		let _ = update(&mut state, Action::OpenDistraction);
		assert!(state.work_tracking.distraction_open);
		let _ = update(&mut state, Action::CloseDistraction);
		assert!(!state.work_tracking.distraction_open);
		assert!(state.config.task_work.distractions.is_empty());

		for minutes in [5, 10] {
			let _ = update(&mut state, Action::OpenDistraction);
			let _ = update(&mut state, Action::AddDistraction(minutes));
			assert!(!state.work_tracking.distraction_open);
		}
		assert_eq!(state.config.task_work.distracted(&key), (2, 15));

		let _ = update(&mut state, Action::UndoDistraction);
		assert_eq!(state.config.task_work.distracted(&key), (1, 5));
	}

	/// A focused block with one distraction habit, "phone".
	fn distraction_habit_state() -> State {
		let mut state = estimate_state();
		let mut habit = crate::habits::Habit::new("phone", "Check phone");
		habit.kind = crate::habits::Kind::No;
		habit.distraction = true;
		state.habits.library.habits.push(habit);
		state
	}

	#[test]
	fn a_distraction_habit_is_chosen_timed_and_then_filled_out() {
		let mut state = distraction_habit_state();
		let key = block_key(state.calendar.event("standup").unwrap());
		let _ = update(&mut state, Action::OpenDistraction);
		let _ = update(
			&mut state,
			Action::ChooseDistraction(DistractionCause::Habit("phone".into())),
		);
		assert!(state.work_tracking.distraction_open);
		assert!(state.habits.day_dialog.is_none());

		let _ = update(&mut state, Action::AddDistraction(10));
		assert!(!state.work_tracking.distraction_open);
		assert_eq!(state.config.task_work.distracted(&key), (1, 10));
		let dialog = state.habits.day_dialog.as_ref().unwrap();
		assert_eq!(dialog.habit_id, "phone");
		assert_eq!(dialog.happened.len(), 1, "an entry for now is ready");
		assert!(dialog.shows_happened());
		let today = crate::clock::now().date_naive();
		assert_eq!(
			state.habits.library.habit("phone").unwrap().times_on(today),
			0,
			"nothing reaches the habit until the dialog is saved"
		);

		let _ = crate::habits::update(&mut state, crate::habits::Action::SaveDay);
		assert!(state.habits.day_dialog.is_none());
		assert_eq!(
			state.habits.library.habit("phone").unwrap().times_on(today),
			1
		);
	}

	#[test]
	fn a_distraction_no_habit_covers_is_only_timed() {
		let mut state = distraction_habit_state();
		let key = block_key(state.calendar.event("standup").unwrap());
		let _ = update(&mut state, Action::OpenDistraction);
		let _ = update(
			&mut state,
			Action::ChooseDistraction(DistractionCause::Other),
		);
		let _ = update(&mut state, Action::AddDistraction(5));
		assert_eq!(state.config.task_work.distracted(&key), (1, 5));
		assert!(state.habits.day_dialog.is_none());
	}

	#[test]
	fn reopening_the_distraction_questions_starts_from_the_first() {
		let mut state = distraction_habit_state();
		let _ = update(&mut state, Action::OpenDistraction);
		let _ = update(
			&mut state,
			Action::ChooseDistraction(DistractionCause::Habit("phone".into())),
		);
		let _ = update(&mut state, Action::CloseDistraction);
		let _ = update(&mut state, Action::OpenDistraction);
		assert_eq!(state.work_tracking.distraction_cause, None);
	}

	#[test]
	fn leaving_the_block_closes_the_distraction_choice() {
		let mut state = estimate_state();
		let _ = update(&mut state, Action::OpenDistraction);
		let _ = update(&mut state, Action::CloseView);
		assert!(!state.work_tracking.distraction_open);
		// Without a focused block there is nothing to ask about or record against.
		let _ = update(&mut state, Action::OpenDistraction);
		assert!(!state.work_tracking.distraction_open);
		let _ = update(&mut state, Action::AddDistraction(5));
		assert!(state.config.task_work.distractions.is_empty());
	}

	#[test]
	fn pomodoro_stops_its_task_with_the_clock_and_selects_tasks_on_break() {
		let mut state = estimate_state();
		let _ = update(&mut state, Action::StartWork("task:deep".into()));
		assert!(state.config.task_work.active().is_some());
		assert!(!state.config.pomodoro.running());
		let _ = update(&mut state, Action::PomodoroMinutes(true, -24));
		assert_eq!(state.config.pomodoro.remaining, 60);
		let _ = update(&mut state, Action::PomodoroToggle);
		assert!(state.config.pomodoro.running());
		let _ = update(&mut state, Action::PomodoroToggle);
		assert!(!state.config.pomodoro.running());
		assert!(state.config.task_work.active().is_none());
		let _ = update(&mut state, Action::PomodoroToggle);
		assert!(state.config.task_work.active().is_some());
		let now = crate::clock::utc_now();
		state.config.pomodoro.running_since = Some(now);
		state.config.pomodoro.remaining = 60;
		assert!(advance_pomodoro(
			&mut state,
			now + chrono::Duration::seconds(60)
		));
		assert!(state.config.task_work.active().is_none());
		assert_eq!(state.config.pomodoro.phase, crate::pomodoro::Phase::Break);
		let _ = update(&mut state, Action::StartWork("task:deep".into()));
		assert!(state.config.task_work.active().is_none());
		assert_eq!(
			state.config.pomodoro.target.as_ref().unwrap().task_key,
			"100"
		);
		assert!(advance_pomodoro(
			&mut state,
			now + chrono::Duration::seconds(360)
		));
		assert!(state.config.task_work.active().is_some());
		let _ = update(&mut state, Action::CloseView);
		assert!(state.config.pomodoro.running());
	}

	#[test]
	fn work_on_a_task_without_an_id_waits_for_one_and_is_recorded_under_it() {
		let mut state = estimate_state();
		state
			.calendar
			.event_mut("task:deep")
			.unwrap()
			.details
			.task_data = None;
		let _ = update(&mut state, Action::StartWork("task:deep".into()));
		// Nothing is recorded under its Google identity meanwhile.
		assert!(state.config.task_work.sessions.is_empty());
		let data = crate::task_db::TaskData {
			id: 2875,
			metadata: Default::default(),
		};
		let _ = crate::focus::update(
			&mut state,
			crate::focus::Action::Identified("task:deep".into(), Ok(data.clone())),
		);
		let task = state.calendar.event("task:deep").unwrap();
		assert_eq!(task.details.task_data, Some(data));
		assert_eq!(state.config.task_work.active().unwrap().task_key, "2875");
		assert!(state.config.task_work.running(&task_key(task)));
		// Nothing starts if the ID could not be given, or the task has gone
		// by the time it has one.
		state.config.task_work = Default::default();
		let _ = crate::focus::update(
			&mut state,
			crate::focus::Action::Identified("task:deep".into(), Err("database is locked".into())),
		);
		assert_eq!(state.calendar.error.as_deref(), Some("database is locked"));
		let _ = crate::focus::update(
			&mut state,
			crate::focus::Action::Identified(
				"task:gone".into(),
				Ok(crate::task_db::TaskData {
					id: 2876,
					metadata: Default::default(),
				}),
			),
		);
		assert!(state.config.task_work.sessions.is_empty());
	}

	#[test]
	fn a_work_session_can_have_its_start_and_end_edited_or_removed() {
		let mut state = State::blank();
		let day = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
		let start = Local.with_ymd_and_hms(2026, 10, 28, 9, 0, 0).unwrap();
		let end = Local.with_ymd_and_hms(2026, 10, 28, 10, 0, 0).unwrap();
		state
			.config
			.task_work
			.sessions
			.push(crate::task_work::Session {
				id: "session".into(),
				task_key: "task:deep".into(),
				task_title: "Deep work".into(),
				block_key: Some("work/block".into()),
				started_at: start.with_timezone(&chrono::Utc),
				stopped_at: Some(end.with_timezone(&chrono::Utc)),
			});

		let _ = update(&mut state, Action::EditSession(0));
		assert_eq!(state.work_tracking.session_start, "09:00");
		assert_eq!(state.work_tracking.session_end, "10:00");
		assert_eq!(state.work_tracking.session_date, Some(day));

		let _ = update(&mut state, Action::SessionStart("08:30".into()));
		let _ = update(&mut state, Action::SessionEnd("09:15".into()));
		let _ = update(&mut state, Action::SaveSession);
		assert!(state.work_tracking.session_edit.is_none());
		let session = &state.config.task_work.sessions[0];
		assert_eq!(
			session
				.started_at
				.with_timezone(&Local)
				.format("%H:%M")
				.to_string(),
			"08:30"
		);
		assert_eq!(
			session
				.stopped_at
				.unwrap()
				.with_timezone(&Local)
				.format("%H:%M")
				.to_string(),
			"09:15"
		);

		let _ = update(&mut state, Action::EditSession(0));
		let _ = update(&mut state, Action::DeleteSession);
		assert!(state.config.task_work.sessions.is_empty());
	}

	#[test]
	fn estimate_dialog_prefills_and_cancel_preserves_the_view_and_task() {
		let mut state = estimate_state();
		let _ = update(&mut state, Action::EditEstimate("task:deep".into()));
		assert_eq!(state.task_editor().estimate, "35");
		let _ = update(&mut state, Action::Estimate("90".into()));
		let _ = update(&mut state, Action::CloseEstimate);
		assert!(state.task_editor().estimate_task.is_none());
		assert_eq!(state.timeblocking.focus.as_deref(), Some("standup"));
		assert_eq!(
			metadata(state.calendar.event("task:deep").unwrap()).minutes,
			35
		);
	}

	#[test]
	fn estimate_errors_stay_in_the_dialog_and_a_successful_retry_closes_it() {
		let mut state = estimate_state();
		state.calendar.open_event = Some("task:deep".into());
		let _ = update(&mut state, Action::EditEstimate("task:deep".into()));
		for invalid in ["abc", "-1", "10081"] {
			let _ = update(&mut state, Action::Estimate(invalid.into()));
			let _ = update(&mut state, Action::SaveEstimate("task:deep".into()));
			assert!(state.task_editor().estimate_error.is_some());
			assert!(state.task_editor().estimate_task.is_some());
			assert!(!state.task_operations.busy.contains_task("task:deep"));
			assert!(state.calendar.error.is_none());
		}
		let _ = update(&mut state, Action::Estimate("60".into()));
		let _ = update(
			&mut state,
			Action::Saved("task:deep".into(), Err("Could not save".into())),
		);
		assert_eq!(
			state.task_editor().estimate_error.as_deref(),
			Some("Could not save")
		);
		let mut saved = state.calendar.event("task:deep").unwrap().clone();
		let mut meta = metadata(&saved);
		meta.minutes = 60;
		saved.details.task_data.as_mut().unwrap().metadata = meta;
		let _ = update(&mut state, Action::Saved("task:deep".into(), Ok(saved)));
		assert!(state.task_editor().estimate_task.is_none());
		assert!(state.task_editor().estimate_error.is_none());
		assert_eq!(state.calendar.open_event.as_deref(), Some("task:deep"));
		assert_eq!(state.timeblocking.focus.as_deref(), Some("standup"));
		assert_eq!(
			metadata(state.calendar.event("task:deep").unwrap()).minutes,
			60
		);
	}

	#[test]
	fn a_blank_todo_is_not_added_and_a_long_one_is_refused() {
		let mut state = estimate_state();
		state.timeblocking.todo_draft = "   ".into();
		let _ = update(&mut state, Action::AddTodo("standup".into()));
		assert!(state.task_operations.busy.is_empty());
		assert!(state.timeblocking.todo_error.is_none());

		state.timeblocking.todo_draft = "x".repeat(MAX_TODO_CHARS + 1);
		let _ = update(&mut state, Action::AddTodo("standup".into()));
		assert!(state.task_operations.busy.is_empty());
		assert!(state.timeblocking.todo_error.is_some());
		assert_eq!(
			state.timeblocking.todo_draft.chars().count(),
			MAX_TODO_CHARS + 1
		);
	}

	#[test]
	fn typing_a_todo_clears_the_previous_error() {
		let mut state = estimate_state();
		state.timeblocking.todo_error = Some("Could not save".into());
		let _ = update(&mut state, Action::TodoDraft("Call the client".into()));
		assert_eq!(state.timeblocking.todo_draft, "Call the client");
		assert!(state.timeblocking.todo_error.is_none());
	}

	#[test]
	fn a_task_can_be_renamed_after_it_is_created() {
		let mut state = estimate_state();
		let original = state.calendar.event("task:deep").unwrap().title.clone();
		state.calendar.open_event = Some("task:deep".into());
		state.task_editor_mut().title = "   ".into();
		let _ = update(&mut state, Action::SaveTitle("task:deep".into()));
		assert_eq!(
			state.task_editor().title_error.as_deref(),
			Some("A task needs a name")
		);
		assert!(state.task_operations.busy.is_empty());
		assert_eq!(state.calendar.event("task:deep").unwrap().title, original);

		state.task_editor_mut().title = "x".repeat(MAX_TODO_CHARS + 1);
		let _ = update(&mut state, Action::SaveTitle("task:deep".into()));
		assert!(state.task_editor().title_error.is_some());
		assert!(state.task_operations.busy.is_empty());
		assert_eq!(state.calendar.event("task:deep").unwrap().title, original);

		let _ = update(&mut state, Action::Title(original.clone()));
		assert!(state.task_editor().title_error.is_none());
		let _ = update(&mut state, Action::SaveTitle("task:deep".into()));
		assert!(state.task_editor().renaming.is_none());
		assert!(state.task_operations.busy.is_empty());

		state.task_editor_mut().title = format!("  {original} revised  ");
		state.task_editor_mut().renaming = Some("task:deep".into());
		crate::task_operations::begin(&mut state, "task:deep");
		let _ = update(
			&mut state,
			Action::Saved("task:deep".into(), Err("Not connected to Google".into())),
		);
		assert_eq!(state.calendar.event("task:deep").unwrap().title, original);
		assert_eq!(
			state.task_editor().title_error.as_deref(),
			Some("Not connected to Google")
		);
		assert!(state.calendar.error.is_none());
		assert_eq!(state.calendar.open_event.as_deref(), Some("task:deep"));
		assert!(state.task_editor().renaming.is_none());

		let mut saved = state.calendar.event("task:deep").unwrap().clone();
		saved.title = format!("{original} revised");
		state.task_editor_mut().renaming = Some("task:deep".into());
		let _ = update(&mut state, Action::Saved("task:deep".into(), Ok(saved)));
		assert_eq!(
			state.calendar.event("task:deep").unwrap().title,
			format!("{original} revised")
		);
		assert!(state.task_editor().title_error.is_none());
		assert!(state.task_editor().renaming.is_none());
	}

	#[test]
	fn a_saved_todo_replaces_the_draft_and_a_failure_keeps_it() {
		let mut state = estimate_state();
		state.timeblocking.todo_open = true;
		state.timeblocking.todo_draft = "Call the client".into();
		crate::task_operations::begin(&mut state, BusyKey::AddingTodo);
		let event = new_todo_event(
			"has-time",
			&calendar::api::CreatedTask {
				updated_at: None,
				id: "fresh".into(),
				title: "Call the client".into(),
				link: "https://tasks.google.com/task/fresh".into(),
			},
			crate::clock::now(),
		);
		assert!(event.is_task());
		let _ = update(&mut state, Action::Saved(BusyKey::AddingTodo, Ok(event)));
		assert!(state.timeblocking.todo_draft.is_empty());
		assert!(!state.timeblocking.todo_open);
		assert!(state.task_operations.busy.is_empty());
		assert_eq!(
			state.calendar.event("task:fresh").unwrap().title,
			"Call the client"
		);

		state.timeblocking.todo_open = true;
		state.timeblocking.todo_draft = "Call the client".into();
		crate::task_operations::begin(&mut state, BusyKey::AddingTodo);
		let _ = update(
			&mut state,
			Action::Saved(BusyKey::AddingTodo, Err("Not connected to Google".into())),
		);
		assert!(state.timeblocking.todo_open);
		assert_eq!(state.timeblocking.todo_draft, "Call the client");
		assert_eq!(
			state.timeblocking.todo_error.as_deref(),
			Some("Not connected to Google")
		);
		assert!(state.calendar.error.is_none());
	}
}

impl From<Action> for Message {
	fn from(action: Action) -> Self {
		Self::TimeBlocking(action)
	}
}
