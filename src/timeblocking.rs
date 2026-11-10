//! Task planning, Calendar scheduling, metadata and work sessions.
use crate::app::CalendarMessage;
use crate::calendar::ids;
use crate::{
    app::{Message, State},
    blocking,
    calendar::{self, Event},
    config::CalendarSettings,
};
use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone};
use iced::Task;
use timeslay_core::calendar::google::GoogleError;
use timeslay_core::task_edit::{self, Change, clean_label};

pub use timeslay_core::timeblocking::*;

pub const PANEL_WIDTH: f32 = 400.0;
/// A save in progress, and what its result will be for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusyKey {
    Task(String),
    /// A todo that does not have a task id until Google creates it.
    AddingTodo,
    AddingHabit,
}

impl BusyKey {
    /// The task this save is for, which matches no task while adding a todo.
    fn task_id(&self) -> &str {
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
use timeslay_core::task_edit::MAX_TITLE_CHARS as MAX_TODO_CHARS;
#[derive(Default)]
pub struct Workspace {
    pub block: Option<String>,
    pub focus: Option<String>,
    pub work_now: Option<chrono::DateTime<chrono::Utc>>,
    /// The work session open in the editor, as an index into the work log.
    pub session_edit: Option<usize>,
    /// The day the edited session started, so its times can be parsed back.
    pub session_date: Option<NaiveDate>,
    /// The start and end being edited, as local `HH:MM`.
    pub session_start: String,
    pub session_end: String,
    pub session_error: Option<String>,
    /// The choice of how long a distraction lasted is open.
    pub distraction_open: bool,
    pub revision: u64,
    pub date: String,
    pub deadline: String,
    pub sync_notice: Option<String>,
    pub start_time: String,
    pub end_time: String,
    pub estimate: String,
    pub estimate_task: Option<String>,
    pub estimate_error: Option<String>,
    /// The name being edited for the open task, before it is saved.
    pub title: String,
    pub editing_title: bool,
    pub editing_due_time: bool,
    pub editing_deadline_time: bool,
    pub deadline_time: String,
    pub title_error: Option<String>,
    /// The task whose rename is in flight, so a failure stays on the name field.
    pub renaming: Option<String>,
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
    /// Placement is committed only after a cross-day schedule write succeeds.
    pub pending_week_drop: Option<WeekListPlacement>,
    pub dragging: Option<String>,
    /// Where the pointer is while a task is being dragged, so the floating
    /// card can follow it. Only meaningful while `dragging` is set.
    pub drag_pointer: Option<iced::Point>,
    /// The surface whose coordinates `drag_pointer` uses.
    pub drag_window: Option<iced::window::Id>,
    pub busy: Busy,
    /// A task whose fields are being saved without closing its details pane.
    pub keep_open: Option<String>,
    /// Text that narrows the backlog by task title or list name.
    pub backlog_query: String,
    /// When set, the backlog shows only tasks from this Google Tasks list.
    pub backlog_list: Option<String>,
    /// Text that narrows the project picker in the task editor.
    pub project_query: String,
    /// The task whose project picker is open.
    pub project_picker: Option<String>,
    /// The task whose backlog destination choices are open.
    pub backlog_picker: Option<String>,
    /// The label being typed for the open task, before it is added.
    pub label_draft: String,
    pub label_error: Option<String>,
}
#[derive(Debug, Clone)]
pub struct WeekListPlacement {
    pub task: String,
    pub day: NaiveDate,
    pub target: Option<String>,
    pub after: bool,
}

// Messages only live until the next update, so a large variant costs nothing lasting.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Action {
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
    /// Ask how long the distraction from the focused block lasted.
    OpenDistraction,
    CloseDistraction,
    /// Record this many distracted minutes against the focused block.
    AddDistraction(u32),
    /// Take back the focused block's latest distraction.
    UndoDistraction,
    HideOverdue(bool),
    Deadline(String),
    StartTime(String),
    EndTime(String),
    ScheduleEntered(String),
    SaveDeadline(String),
    EditTitle,
    EditScheduleTime(bool),
    DeadlineTime(String),
    DeadlineDay(String),
    DeadlineWithTime(String),
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
    Date(String),
    SaveEstimate(String),
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
    Reschedule(String),
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
    /// Insert before/after a board card, or at the end of an empty/day column.
    DropWeekList {
        target: Option<String>,
        after: bool,
    },
    NewWeekTask,
    Saved(BusyKey, Result<Event, String>),
    SavedMany(BusyKey, Result<Vec<Event>, String>),
}
fn take_drag(state: &mut State) -> Option<String> {
    state.timeblocking.drag_pointer = None;
    state.timeblocking.drag_window = None;
    state.timeblocking.dragging.take()
}

pub fn assigned_all<'a>(state: &'a State, block: &Event) -> Vec<&'a Event> {
    let key = block_key(block);
    let mut tasks: Vec<_> = state
        .calendar
        .events
        .iter()
        .filter(|e| e.is_task() && metadata(e).block.as_deref() == Some(&key))
        .collect();
    tasks.sort_by_key(|e| (metadata(e).order, e.id.clone()));
    tasks
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
    state.timeblocking.distraction_open = false;
    state.timeblocking.session_edit = None;
    state.timeblocking.session_error = None;
    state.timeblocking.dragging = None;
    state.timeblocking.drag_pointer = None;
    state.timeblocking.available_query.clear();
    state.timeblocking.available_habit_query.clear();
    state.timeblocking.available_all = false;
    state.timeblocking.available_planned = false;
    // A save still in flight keeps the draft so its result can land on the same field.
    if !state.timeblocking.busy.adding_todo() {
        state.timeblocking.todo_open = false;
        state.timeblocking.todo_draft.clear();
        state.timeblocking.todo_error = None;
    }
    Task::none()
}
pub fn open(state: &mut State, id: String) -> Task<Message> {
    if state.timeblocking.block.as_deref() != Some(id.as_str())
        && !state.timeblocking.busy.adding_todo()
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
    state.timeblocking.distraction_open = false;
    // The panel edits the block in place, so it always opens with a live
    // draft of the block's own fields.
    state.calendar.edit_draft = event(state, &id).map(crate::calendar::EditDraft::from_event);
    state.timeblocking.block = Some(id);
    Task::none()
}

fn focused_block_key(state: &State) -> Option<String> {
    let id = state.timeblocking.focus.as_deref()?;
    state.calendar.event(id).map(block_key)
}

fn advance_pomodoro(state: &mut State, now: chrono::DateTime<chrono::Utc>) -> bool {
    let eligible: std::collections::HashSet<_> = state
        .calendar
        .events
        .iter()
        .filter(|e| e.is_task() && task_status(e) == TaskStatus::Active)
        .map(task_key)
        .collect();
    state
        .config
        .pomodoro
        .advance(&mut state.config.task_work, now, |key| {
            eligible.contains(key)
        })
}

/// What the Pomodoro should start next: a marker, never an active work session.
fn resume_target(task: &Event, now: chrono::DateTime<chrono::Utc>) -> crate::task_work::Session {
    crate::task_work::Session {
        task_key: task_key(task),
        task_title: task.title.clone(),
        block_key: metadata(task).block,
        started_at: now,
        stopped_at: Some(now),
    }
}

/// The task a block is on: the one running, else the one the Pomodoro will
/// resume, else the first still to do.
pub fn current_task<'a>(state: &'a State, block: &Event) -> Option<&'a Event> {
    let open: Vec<_> = assigned_all(state, block)
        .into_iter()
        .filter(|task| task_status(task) == TaskStatus::Active)
        .collect();
    let find = |key: &str| open.iter().copied().find(|task| task_key(task) == key);
    state
        .config
        .task_work
        .active()
        .and_then(|session| find(&session.task_key))
        .or_else(|| {
            let target = state.config.pomodoro.target.as_ref()?;
            find(&target.task_key)
        })
        .or_else(|| open.first().copied())
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
    // Writes started before creation opened still finish in their own workspace.
    if matches!(&action, Action::Saved(_, _) | Action::SavedMany(_, _))
        && let Some(previous) = state.inbox.workspace.take()
    {
        let draft_workspace = std::mem::replace(&mut state.timeblocking, *previous);
        let task = update(state, action);
        let previous = std::mem::replace(&mut state.timeblocking, draft_workspace);
        state.inbox.workspace = Some(Box::new(previous));
        return task;
    }

    // Triaging is a review step: the task cannot leave the Inbox until every
    // required field is filled in. Nothing is sent to Google when it is not.
    if let Action::Triage(id) = &action
        && let Some(error) = triage_error(state, id)
    {
        state.inbox.error = Some(error);
        return Task::none();
    }
    state.inbox.error = None;
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
            state.timeblocking.work_now = Some(crate::clock::utc_now());
            state.calendar.open_event = None;
            state.calendar.menu_open = false;
            state.calendar.search.open = false;
            state.calendar.actions_open = false;
            state.calendar.month_open = false;
            state.overview_month_open = false;
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
            state.calendar.menu_open = false;
            state.calendar.search.open = false;
            state.calendar.actions_open = false;
            return open(state, id);
        }
        Action::CloseView => {
            state.timeblocking.focus = None;
            state.timeblocking.distraction_open = false;
            return Task::none();
        }
        Action::WorkTick => {
            let now = crate::clock::utc_now();
            state.timeblocking.work_now = Some(now);
            if advance_pomodoro(state, now) {
                return crate::app::queue_save(state);
            }
            return Task::none();
        }
        Action::PomodoroToggle => {
            let now = crate::clock::utc_now();
            advance_pomodoro(state, now);
            if state.config.pomodoro.running() {
                // The task stops with the clock and is what the next Start resumes.
                if let Some(active) = state.config.task_work.active() {
                    state.config.pomodoro.target = Some(active.clone());
                }
                state.config.pomodoro.running_since = None;
                state.config.task_work.stop_all(now);
            } else {
                let current = state
                    .timeblocking
                    .focus
                    .as_deref()
                    .and_then(|id| state.calendar.event(id))
                    .and_then(|block| current_task(state, block))
                    .map(|task| resume_target(task, now));
                let eligible: std::collections::HashSet<_> = state
                    .calendar
                    .events
                    .iter()
                    .filter(|e| e.is_task() && task_status(e) == TaskStatus::Active)
                    .map(task_key)
                    .collect();
                let timer = &mut state.config.pomodoro;
                timer.running_since = Some(now);
                if let Some(next) = current.or_else(|| state.config.task_work.active().cloned()) {
                    timer.target = Some(next);
                }
                if timer.phase == crate::pomodoro::Phase::Work {
                    timer.resume_task(&mut state.config.task_work, now, |key| {
                        eligible.contains(key)
                    });
                } else {
                    state.config.task_work.stop_all(now);
                }
            }
            state.timeblocking.work_now = Some(now);
            return crate::app::queue_save(state);
        }
        Action::PomodoroReset => {
            let now = crate::clock::utc_now();
            advance_pomodoro(state, now);
            if state.config.pomodoro.running() {
                state.config.task_work.stop_all(now);
            }
            state.config.pomodoro.reset();
            return crate::app::queue_save(state);
        }
        Action::PomodoroMinutes(work, delta) => {
            let timer = &mut state.config.pomodoro;
            if timer.fresh() {
                let minutes = if work {
                    &mut timer.work_minutes
                } else {
                    &mut timer.break_minutes
                };
                *minutes = (i64::from(*minutes) + i64::from(delta)).clamp(1, 180) as u32;
                timer.remaining = timer.duration();
                return crate::app::queue_save(state);
            }
            return Task::none();
        }
        Action::EditSession(index) => {
            let Some(session) = state.config.task_work.sessions.get(index) else {
                return Task::none();
            };
            let start = session.started_at.with_timezone(&Local);
            state.timeblocking.session_date = Some(start.date_naive());
            state.timeblocking.session_start = start.format("%H:%M").to_string();
            state.timeblocking.session_end = session
                .stopped_at
                .map(|end| end.with_timezone(&Local).format("%H:%M").to_string())
                .unwrap_or_default();
            state.timeblocking.session_error = None;
            state.timeblocking.session_edit = Some(index);
            return iced::widget::operation::focus(iced::widget::Id::new("work-session-start"));
        }
        Action::SessionStart(value) => {
            state.timeblocking.session_start = value;
            state.timeblocking.session_error = None;
            return Task::none();
        }
        Action::SessionEnd(value) => {
            state.timeblocking.session_end = value;
            state.timeblocking.session_error = None;
            return Task::none();
        }
        Action::SaveSession => {
            let Some(index) = state.timeblocking.session_edit else {
                return Task::none();
            };
            let Some(day) = state.timeblocking.session_date else {
                return Task::none();
            };
            let Some((sh, sm)) = calendar::parse_clock(state.timeblocking.session_start.trim())
            else {
                state.timeblocking.session_error = Some("Enter a start time as HH:MM".into());
                return Task::none();
            };
            let Some(start) = Local
                .from_local_datetime(&day.and_hms_opt(sh, sm, 0).unwrap())
                .earliest()
            else {
                state.timeblocking.session_error =
                    Some("This start time does not exist in your time zone".into());
                return Task::none();
            };
            let end_text = state.timeblocking.session_end.trim();
            let end = if end_text.is_empty() {
                None
            } else {
                let Some((eh, em)) = calendar::parse_clock(end_text) else {
                    state.timeblocking.session_error = Some("Enter an end time as HH:MM".into());
                    return Task::none();
                };
                let end_day = if (eh, em) <= (sh, sm) {
                    match day.succ_opt() {
                        Some(day) => day,
                        None => return Task::none(),
                    }
                } else {
                    day
                };
                let Some(end) = Local
                    .from_local_datetime(&end_day.and_hms_opt(eh, em, 0).unwrap())
                    .earliest()
                else {
                    state.timeblocking.session_error =
                        Some("This end time does not exist in your time zone".into());
                    return Task::none();
                };
                if end <= start {
                    state.timeblocking.session_error =
                        Some("The session must end after it starts".into());
                    return Task::none();
                }
                Some(end)
            };
            let Some(session) = state.config.task_work.sessions.get_mut(index) else {
                state.timeblocking.session_edit = None;
                return Task::none();
            };
            session.started_at = start.with_timezone(&chrono::Utc);
            session.stopped_at = end.map(|end| end.with_timezone(&chrono::Utc));
            state.timeblocking.session_edit = None;
            state.timeblocking.session_error = None;
            state.timeblocking.work_now = Some(crate::clock::utc_now());
            return crate::app::queue_save(state);
        }
        Action::CloseSession => {
            state.timeblocking.session_edit = None;
            state.timeblocking.session_error = None;
            return Task::none();
        }
        Action::DeleteSession => {
            let Some(index) = state.timeblocking.session_edit.take() else {
                return Task::none();
            };
            if index < state.config.task_work.sessions.len() {
                state.config.task_work.sessions.remove(index);
                state.timeblocking.session_error = None;
                state.timeblocking.work_now = Some(crate::clock::utc_now());
                return crate::app::queue_save(state);
            }
            return Task::none();
        }
        Action::OpenDistraction => {
            state.timeblocking.distraction_open = state.timeblocking.focus.is_some();
            return Task::none();
        }
        Action::CloseDistraction => {
            state.timeblocking.distraction_open = false;
            return Task::none();
        }
        Action::AddDistraction(minutes) => {
            state.timeblocking.distraction_open = false;
            let Some(key) = focused_block_key(state) else {
                return Task::none();
            };
            state
                .config
                .task_work
                .distractions
                .push(crate::task_work::Distraction {
                    block_key: key,
                    minutes,
                    recorded_at: crate::clock::utc_now(),
                });
            return crate::app::queue_save(state);
        }
        Action::UndoDistraction => {
            let Some(key) = focused_block_key(state) else {
                return Task::none();
            };
            state.config.task_work.undo_distraction(&key);
            return crate::app::queue_save(state);
        }
        Action::StartWork(id) => {
            let now = crate::clock::utc_now();
            advance_pomodoro(state, now);
            let Some(task) = event(state, &id).filter(|e| {
                e.is_task() && !e.completed && !state.timeblocking.busy.contains_task(&id)
            }) else {
                return Task::none();
            };
            if state.config.pomodoro.running()
                && state.config.pomodoro.phase == crate::pomodoro::Phase::Break
            {
                state.config.pomodoro.target = Some(resume_target(task, now));
                return crate::app::queue_save(state);
            }
            state.config.task_work.start(
                task_key(task),
                task.title.clone(),
                metadata(task).block,
                now,
            );
            state.config.pomodoro.target = state.config.task_work.active().cloned();
            state.timeblocking.work_now = Some(now);
            return crate::app::queue_save(state);
        }
        Action::StopWork(id) => {
            let now = crate::clock::utc_now();
            advance_pomodoro(state, now);
            if let Some(task) = event(state, &id) {
                let key = task_key(task);
                state.config.task_work.stop(&key, now);
                if state
                    .config
                    .pomodoro
                    .target
                    .as_ref()
                    .is_some_and(|target| target.task_key == key)
                {
                    state.config.pomodoro.target = None;
                }
                state.timeblocking.work_now = Some(now);
                return crate::app::queue_save(state);
            }
            return Task::none();
        }

        Action::HideOverdue(hide) => {
            state.config.calendar.hide_overdue_tasks = hide;
            return crate::app::queue_save(state);
        }
        Action::StartTime(value) => {
            state.timeblocking.start_time = value;
            return Task::none();
        }
        Action::EndTime(value) => {
            state.timeblocking.end_time = value;
            return Task::none();
        }
        Action::ScheduleEntered(id) => {
            let range = task_edit::schedule_range(
                &state.timeblocking.date,
                &state.timeblocking.start_time,
                &state.timeblocking.end_time,
            );
            return match range {
                Ok((start, end)) => update(state, Action::Schedule(id, start, end)),
                Err(error) => {
                    state.calendar.error = Some(error);
                    Task::none()
                }
            };
        }
        Action::EditTitle => {
            state.timeblocking.editing_title = true;
            return iced::widget::operation::focus(iced::widget::Id::new("task-title"));
        }
        Action::EditScheduleTime(deadline) => {
            if deadline {
                state.timeblocking.editing_deadline_time =
                    !state.timeblocking.editing_deadline_time;
            } else {
                state.timeblocking.editing_due_time = !state.timeblocking.editing_due_time;
            }
            return Task::none();
        }
        Action::DeadlineTime(value) => {
            state.timeblocking.deadline_time = value;
            return Task::none();
        }
        Action::DeadlineDay(id) => {
            state.timeblocking.deadline.truncate(10);
            return update(state, Action::SaveDeadline(id));
        }
        Action::DeadlineWithTime(id) => {
            let day = state.timeblocking.deadline.get(..10).unwrap_or("");
            state.timeblocking.deadline = format!("{day} {}", state.timeblocking.deadline_time);
            return update(state, Action::SaveDeadline(id));
        }
        Action::Deadline(value) => {
            state.timeblocking.deadline = value;
            return Task::none();
        }
        Action::SaveDeadline(id) => {
            let input = state.timeblocking.deadline.trim();
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
        Action::DropWeekList { target, after } => return drop_week_list(state, target, after),
        Action::DropTime(start) => {
            let Some(id) = take_drag(state) else {
                return Task::none();
            };
            let minutes = event(state, &id).map(|t| metadata(t).minutes).unwrap_or(0);
            let minutes = if minutes == 0 { 30 } else { minutes };
            return update(
                state,
                Action::Schedule(
                    id,
                    start,
                    start + chrono::Duration::minutes(i64::from(minutes)),
                ),
            );
        }
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
        Action::CancelDrag => {
            take_drag(state);
            return Task::none();
        }
        Action::Details(id) => {
            state.calendar.open_event = Some(id);
            return Task::none();
        }
        Action::Date(value) => {
            state.timeblocking.date = value;
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
            let Some(task) = event(state, &id).filter(|e| e.is_task()) else {
                return Task::none();
            };
            if state.timeblocking.busy.contains_task(&id) {
                return Task::none();
            }
            state.timeblocking.estimate = metadata(task).minutes.to_string();
            state.timeblocking.estimate_task = Some(id);
            state.timeblocking.estimate_error = None;
            state.timeblocking.dragging = None;
            state.timeblocking.drag_pointer = None;
            return iced::widget::operation::focus(iced::widget::Id::new("task-estimate"));
        }
        Action::CloseEstimate => {
            state.timeblocking.estimate_task = None;
            state.timeblocking.estimate_error = None;
            return Task::none();
        }
        Action::Estimate(value) => {
            state.timeblocking.estimate = value;
            state.timeblocking.estimate_error = None;
            return Task::none();
        }
        Action::Drag(id, position) => {
            state.timeblocking.drag_window = None;
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
                return update(state, Action::Assign(id, block));
            }
            return Task::none();
        }
        Action::DropBacklogSection(section) => {
            let Some(id) = take_drag(state) else {
                return Task::none();
            };
            state.timeblocking.drag_pointer = None;
            return update(state, Action::SetBacklogSection(id, section));
        }
        Action::SetBacklogSection(id, section) => {
            let Some(task) = event(state, &id).filter(|task| {
                task.is_task()
                    && !task.completed
                    && metadata(task).block.is_none()
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
            let block = event(state, &target).and_then(|task| metadata(task).block);
            let block = block.and_then(|key| {
                state
                    .calendar
                    .events
                    .iter()
                    .find(|event| !event.is_task() && block_key(event) == key)
                    .map(|event| event.id.clone())
            });
            return update(state, block.map_or(Action::DropDay, Action::DropBlock));
        }
        Action::DropTask(target) => {
            if state.screen == crate::app::Screen::Backlog {
                let Some(task) = event(state, &target) else {
                    return Task::none();
                };
                return update(
                    state,
                    Action::DropBacklogSection(metadata(task).backlog_section),
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
            let previous = state
                .calendar
                .events
                .iter()
                .filter(|e| e.is_task() && e.id != id && e.id != target.id)
                .map(metadata)
                .filter(|other| other.block == m.block && other.order < m.order)
                .map(|other| other.order)
                .max();
            let rank = previous.map_or(m.order.saturating_sub(1_i64 << 32), |previous| {
                (i128::from(previous) + (i128::from(m.order) - i128::from(previous)) / 2) as i64
            });
            (
                id,
                Change::Reorder(m.block, rank, target.start.date_naive()),
            )
        }
        Action::DropDay => {
            let Some(id) = take_drag(state) else {
                return Task::none();
            };
            (id, Change::Day(state.calendar.day))
        }
        Action::SavedMany(key, result) => {
            state.timeblocking.busy.remove(&key);
            let mut tasks = Vec::new();
            match result {
                Ok(events) => {
                    for (index, event) in events.into_iter().enumerate() {
                        let previous = if index == 0 {
                            key.clone()
                        } else {
                            BusyKey::Task(event.id.clone())
                        };
                        tasks.push(update(state, Action::Saved(previous, Ok(event))));
                    }
                }
                Err(error) => {
                    state.timeblocking.sync_notice = Some(error.clone());
                    tasks.push(update(state, Action::Saved(key, Err(error))));
                }
            }
            // Also reconciles a partially successful cross-service write.
            tasks.push(Task::done(Message::Calendar(
                CalendarMessage::CalendarSyncRequested,
            )));
            return Task::batch(tasks);
        }
        Action::Saved(key, result) => {
            state.timeblocking.revision += 1;
            state.timeblocking.busy.remove(&key);
            let id = key.task_id();
            let mut refresh_week = false;
            let editing_estimate = state.timeblocking.estimate_task.as_deref() == Some(id);
            let renaming = state.timeblocking.renaming.as_deref() == Some(id);
            let keep_open = state
                .timeblocking
                .keep_open
                .take_if(|task| *task == id)
                .is_some();
            match result {
                Ok(event) => {
                    let saved_id = event.id.clone();
                    if renaming {
                        state.timeblocking.renaming = None;
                        state.timeblocking.title_error = None;
                        state.timeblocking.editing_title = false;
                    }
                    if key == BusyKey::AddingTodo {
                        state.timeblocking.todo_draft.clear();
                        state.timeblocking.todo_error = None;
                        state.timeblocking.todo_open = false;
                    }
                    if event.is_calendar_task() && state.config.calendar.app_calendar.is_none() {
                        state.config.calendar.app_calendar = Some(event.calendar_id.clone());
                    }
                    if keep_open && state.calendar.open_event.as_deref() == Some(id) {
                        state.timeblocking.editing_due_time = false;
                        state.timeblocking.editing_deadline_time = false;
                    }
                    let key = task_key(&event);
                    state
                        .calendar
                        .events
                        .retain(|e| e.id != id && (!e.is_task() || task_key(e) != key));
                    state.calendar.replace_cached_task(id, &event);
                    if state.range_view_active() {
                        refresh_week = true;
                    }
                    state.calendar.add_event(event);
                    if let Some(placement) = state
                        .timeblocking
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
                    if editing_estimate {
                        state.timeblocking.estimate_task = None;
                        state.timeblocking.estimate_error = None;
                    } else if !keep_open {
                        state.calendar.open_event = None;
                    }
                }
                Err(error) if editing_estimate => state.timeblocking.estimate_error = Some(error),
                Err(error) if renaming => {
                    state.timeblocking.renaming = None;
                    if state.calendar.open_event.as_deref() == Some(id) {
                        state.timeblocking.title_error = Some(error);
                    } else {
                        state.calendar.error = Some(error);
                    }
                }
                Err(error) if key == BusyKey::AddingTodo => {
                    state.timeblocking.todo_error = Some(error);
                    state.timeblocking.todo_open = true;
                }
                Err(error) => state.calendar.error = Some(error),
            }
            state
                .timeblocking
                .pending_week_drop
                .take_if(|p| p.task == id);
            let save = crate::app::queue_save(state);
            return if refresh_week {
                Task::batch([save, crate::app::refresh_week_after_write(state)])
            } else {
                save
            };
        }
        Action::SaveEstimate(id) => {
            if state.timeblocking.estimate_task.as_deref() != Some(&id) {
                return Task::none();
            }
            let Ok(minutes) = state.timeblocking.estimate.trim().parse::<u32>() else {
                state.timeblocking.estimate_error =
                    Some("Enter an estimate in whole minutes".into());
                return Task::none();
            };
            if minutes == 0 {
                state.timeblocking.estimate_error = Some("An estimate is required".into());
                return Task::none();
            }
            if minutes > 10080 {
                state.timeblocking.estimate_error =
                    Some("Estimate must be at most 10080 minutes".into());
                return Task::none();
            }
            (id, Change::Estimate(minutes))
        }
        Action::BacklogPicker(id) => {
            if !state.timeblocking.busy.is_empty()
                || !event(state, &id).is_some_and(|task| task.is_task())
            {
                return Task::none();
            }
            state.timeblocking.backlog_picker =
                (state.timeblocking.backlog_picker.as_deref() != Some(id.as_str())).then_some(id);
            return Task::none();
        }
        Action::Backlog(id) => (id, Change::Backlog(None)),
        Action::MoveToBacklog(id, section) => {
            if !state.timeblocking.busy.is_empty() {
                return Task::none();
            }
            state.timeblocking.backlog_picker = None;
            (id, Change::Backlog(Some(section)))
        }
        Action::MoveToInbox(id) => {
            if !state.timeblocking.busy.is_empty() {
                return Task::none();
            }
            state.timeblocking.backlog_picker = None;
            (id, Change::Inbox)
        }
        Action::Triage(id) => (id, Change::Triaged),
        Action::Unassign(id) => (id, Change::Unassign),
        Action::Reschedule(id) if state.timeblocking.date.is_empty() => (id, Change::ClearSchedule),
        Action::Reschedule(id) => {
            let Ok(day) = NaiveDate::parse_from_str(&state.timeblocking.date, "%Y-%m-%d") else {
                state.calendar.error = Some("Enter a date as YYYY-MM-DD".into());
                return Task::none();
            };
            (id, Change::Day(day))
        }
        Action::ScheduleDay(id, day) => (id, Change::Day(day)),
        Action::Assign(id, block) => {
            let Some(event) = event(state, &block).filter(|e| is_block(e, &state.config.calendar))
            else {
                return Task::none();
            };
            (id, Change::Block(block_key(event), event.start, event.end))
        }
        Action::Title(value) => {
            state.timeblocking.title = value;
            state.timeblocking.title_error = None;
            return Task::none();
        }
        Action::SaveDescription(id, value) => (id, Change::Description(value)),
        Action::SaveTitle(id) => {
            let title = state.timeblocking.title.trim().to_owned();
            if title.is_empty() {
                state.timeblocking.title_error = Some("A task needs a name".into());
                return Task::none();
            }
            if title.chars().count() > MAX_TODO_CHARS {
                state.timeblocking.title_error = Some(format!(
                    "A task name can be at most {MAX_TODO_CHARS} characters"
                ));
                return Task::none();
            }
            let Some(task) = event(state, &id).filter(|event| event.is_task()) else {
                return Task::none();
            };
            if task.title == title {
                state.timeblocking.title_error = None;
                state.timeblocking.editing_title = false;
                return Task::none();
            }
            state.timeblocking.title_error = None;
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
            state.timeblocking.project_query = value;
            return Task::none();
        }
        Action::ProjectPicker(id, open) => {
            if open {
                state.timeblocking.project_picker = Some(id);
            } else if state.timeblocking.project_picker.as_deref() == Some(id.as_str()) {
                state.timeblocking.project_picker = None;
            }
            return Task::none();
        }
        Action::LabelDraft(value) => {
            state.timeblocking.label_draft = value;
            state.timeblocking.label_error = None;
            return Task::none();
        }
        Action::AddLabel(id, label) => {
            let Some(task) = event(state, &id).filter(|event| event.is_task()) else {
                return Task::none();
            };
            let label = match clean_label(&label) {
                Ok(label) => label,
                Err(error) => {
                    state.timeblocking.label_error = Some(error.to_owned());
                    return Task::none();
                }
            };
            let mut labels = metadata(task).labels;
            if labels
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&label))
            {
                state.timeblocking.label_draft.clear();
                state.timeblocking.label_error = None;
                return Task::none();
            }
            labels.push(label);
            state.timeblocking.label_draft.clear();
            state.timeblocking.label_error = None;
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
        Action::OpenTodo => {
            state.timeblocking.todo_open = true;
            return iced::widget::operation::focus(iced::widget::Id::new("time-block-todo"));
        }
        Action::CloseTodo => {
            if state.timeblocking.busy.adding_todo() {
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
    if id == crate::inbox::DRAFT_ID {
        let Some(draft) = state.inbox.draft.as_mut().filter(|d| !d.saving) else {
            return Task::none();
        };
        let Some(task) = draft.event.as_mut() else {
            return Task::none();
        };
        let data = task.details.task_data.as_mut().unwrap();
        match change {
            Change::Estimate(v) => {
                data.metadata.minutes = v;
                state.timeblocking.estimate_task = None;
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
                state.timeblocking.editing_due_time = false;
            }
            _ => {}
        }
        return Task::none();
    }
    if !state.timeblocking.busy.is_empty() {
        return Task::none();
    }
    let Some(task) = event(state, &id).filter(|e| e.is_task()).cloned() else {
        return Task::none();
    };
    if matches!(change, Change::Title(_)) {
        state.timeblocking.renaming = Some(id.clone());
    }
    if matches!(
        change,
        Change::Description(_)
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
        state.timeblocking.keep_open = Some(id.clone());
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
    state.timeblocking.revision += 1;
    state.timeblocking.busy.insert(id.clone());
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
    blocking::run(move || save_and_reflow(task, settings, change, order, blocks, related))
        .map(move |result| Message::TimeBlocking(Action::SavedMany(id.clone().into(), result)))
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

fn place_week_card(state: &mut State, id: &str, day: NaiveDate, target: Option<&str>, after: bool) {
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
    if !state.timeblocking.busy.is_empty() || target.as_deref() == Some(&id) {
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
    state.timeblocking.pending_week_drop = Some(WeekListPlacement {
        task: id.clone(),
        day,
        target,
        after,
    });
    update(state, Action::Schedule(id, start, end))
}

/// Keep local clock time and elapsed duration when crossing days, including
/// the small hours assigned to the previous planning day.
fn week_drop_times(
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
    add_block_item(state, block_id, title, String::new(), None)
}

fn add_block_item(
    state: &mut State,
    block_id: &str,
    title: String,
    notes: String,
    initial_metadata: Option<Metadata>,
) -> Task<Message> {
    let busy_key = if initial_metadata
        .as_ref()
        .is_some_and(|meta| meta.habit_id.is_some())
    {
        BusyKey::AddingHabit
    } else {
        BusyKey::AddingTodo
    };
    if !state.timeblocking.busy.is_empty() {
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
    state.timeblocking.revision += 1;
    state.timeblocking.busy.insert(busy_key.clone());
    blocking::run(move || {
        create_and_assign(
            title,
            notes,
            initial_metadata,
            settings,
            key,
            start,
            end,
            order,
            blocks,
            related,
        )
    })
    .map(move |result| Message::TimeBlocking(Action::SavedMany(busy_key.clone(), result)))
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
    let task = timeslay_core::task_edit::create_inbox_task(
        &settings,
        "",
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
fn triage_error(state: &State, id: &str) -> Option<String> {
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
    if tasks.is_empty() {
        return Task::none();
    }
    let settings = state.config.calendar.clone();
    let first = tasks[0].id.clone();
    state.timeblocking.revision += 1;
    state.timeblocking.busy.insert(first.clone());
    blocking::run(move || reflow(&block, &tasks, &settings))
        .map(move |result| Message::TimeBlocking(Action::SavedMany(first.clone().into(), result)))
}

/// Every write below stays behind this module's [`access`], so unit tests can
/// never reach a real Google account.
fn save(
    task: Event,
    settings: &CalendarSettings,
    change: Change,
    order: i64,
) -> Result<Event, String> {
    Ok(task_edit::save(task, settings, change, order, access)?)
}

fn save_and_reflow(
    task: Event,
    settings: CalendarSettings,
    change: Change,
    order: i64,
    blocks: Vec<Event>,
    related: Vec<Event>,
) -> Result<Vec<Event>, String> {
    task_edit::save_and_reflow(task, settings, change, order, blocks, related, access)
}

fn reflow(
    block: &Event,
    tasks: &[Event],
    settings: &CalendarSettings,
) -> Result<Vec<Event>, String> {
    task_edit::reflow(block, tasks, settings, access)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            state.timeblocking.pending_week_drop = Some(WeekListPlacement {
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
            assert!(state.timeblocking.pending_week_drop.is_none());
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
        assert!(state.timeblocking.busy.is_empty());
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
        assert!(state.timeblocking.busy.contains_task("task:deep"));
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
    fn deadline_day_removes_time_and_time_save_requires_a_date() {
        let mut state = estimate_state();
        state.timeblocking.deadline = "2026-10-29 14:45".into();
        let _ = update(&mut state, Action::DeadlineDay("task:deep".into()));
        assert_eq!(state.timeblocking.deadline, "2026-10-29");
        assert_eq!(state.timeblocking.keep_open.as_deref(), Some("task:deep"));

        let mut state = estimate_state();
        state.timeblocking.deadline_time = "14:45".into();
        let _ = update(&mut state, Action::DeadlineWithTime("task:deep".into()));
        assert!(state.calendar.error.is_some());
        assert!(state.timeblocking.busy.is_empty());
    }

    #[test]
    fn clearing_a_due_date_keeps_the_editor_and_the_task_list() {
        let mut state = estimate_state();
        state.timeblocking.date.clear();
        let _ = update(&mut state, Action::Reschedule("task:deep".into()));
        assert!(state.calendar.error.is_none());
        assert!(state.timeblocking.busy.contains_task("task:deep"));
        assert_eq!(state.timeblocking.keep_open.as_deref(), Some("task:deep"));
    }

    #[test]
    fn invalid_deadline_input_leaves_the_task_untouched() {
        let mut state = estimate_state();
        state.timeblocking.deadline = "2026-02-30".into();
        let before = state.calendar.event("task:deep").unwrap().clone();
        let _ = update(&mut state, Action::SaveDeadline("task:deep".into()));
        assert_eq!(state.calendar.event("task:deep"), Some(&before));
        assert!(state.timeblocking.busy.is_empty());
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
            .get_or_insert_with(|| timeslay_core::task_db::TaskData {
                id: 100,
                metadata: Default::default(),
            })
            .metadata
            .deadline =
            Some(timeslay_core::calendar::deadline::parse_input(&format!("{yesterday}")).unwrap());
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

    #[test]
    fn a_distraction_is_recorded_against_the_focused_block_and_can_be_undone() {
        let mut state = estimate_state();
        let key = block_key(state.calendar.event("standup").unwrap());
        let _ = update(&mut state, Action::OpenDistraction);
        assert!(state.timeblocking.distraction_open);
        let _ = update(&mut state, Action::CloseDistraction);
        assert!(!state.timeblocking.distraction_open);
        assert!(state.config.task_work.distractions.is_empty());

        for minutes in [5, 10] {
            let _ = update(&mut state, Action::OpenDistraction);
            let _ = update(&mut state, Action::AddDistraction(minutes));
            assert!(!state.timeblocking.distraction_open);
        }
        assert_eq!(state.config.task_work.distracted(&key), (2, 15));

        let _ = update(&mut state, Action::UndoDistraction);
        assert_eq!(state.config.task_work.distracted(&key), (1, 5));
    }

    #[test]
    fn leaving_the_block_closes_the_distraction_choice() {
        let mut state = estimate_state();
        let _ = update(&mut state, Action::OpenDistraction);
        let _ = update(&mut state, Action::CloseView);
        assert!(!state.timeblocking.distraction_open);
        // Without a focused block there is nothing to ask about or record against.
        let _ = update(&mut state, Action::OpenDistraction);
        assert!(!state.timeblocking.distraction_open);
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
                task_key: "task:deep".into(),
                task_title: "Deep work".into(),
                block_key: Some("work/block".into()),
                started_at: start.with_timezone(&chrono::Utc),
                stopped_at: Some(end.with_timezone(&chrono::Utc)),
            });

        let _ = update(&mut state, Action::EditSession(0));
        assert_eq!(state.timeblocking.session_start, "09:00");
        assert_eq!(state.timeblocking.session_end, "10:00");
        assert_eq!(state.timeblocking.session_date, Some(day));

        let _ = update(&mut state, Action::SessionStart("08:30".into()));
        let _ = update(&mut state, Action::SessionEnd("09:15".into()));
        let _ = update(&mut state, Action::SaveSession);
        assert!(state.timeblocking.session_edit.is_none());
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
        assert_eq!(state.timeblocking.estimate, "35");
        let _ = update(&mut state, Action::Estimate("90".into()));
        let _ = update(&mut state, Action::CloseEstimate);
        assert!(state.timeblocking.estimate_task.is_none());
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
            assert!(state.timeblocking.estimate_error.is_some());
            assert!(state.timeblocking.estimate_task.is_some());
            assert!(!state.timeblocking.busy.contains_task("task:deep"));
            assert!(state.calendar.error.is_none());
        }
        let _ = update(&mut state, Action::Estimate("60".into()));
        let _ = update(
            &mut state,
            Action::Saved("task:deep".into(), Err("Could not save".into())),
        );
        assert_eq!(
            state.timeblocking.estimate_error.as_deref(),
            Some("Could not save")
        );
        let mut saved = state.calendar.event("task:deep").unwrap().clone();
        let mut meta = metadata(&saved);
        meta.minutes = 60;
        saved.details.task_data.as_mut().unwrap().metadata = meta;
        let _ = update(&mut state, Action::Saved("task:deep".into(), Ok(saved)));
        assert!(state.timeblocking.estimate_task.is_none());
        assert!(state.timeblocking.estimate_error.is_none());
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
        assert!(state.timeblocking.busy.is_empty());
        assert!(state.timeblocking.todo_error.is_none());

        state.timeblocking.todo_draft = "x".repeat(MAX_TODO_CHARS + 1);
        let _ = update(&mut state, Action::AddTodo("standup".into()));
        assert!(state.timeblocking.busy.is_empty());
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
        state.timeblocking.title = "   ".into();
        let _ = update(&mut state, Action::SaveTitle("task:deep".into()));
        assert_eq!(
            state.timeblocking.title_error.as_deref(),
            Some("A task needs a name")
        );
        assert!(state.timeblocking.busy.is_empty());
        assert_eq!(state.calendar.event("task:deep").unwrap().title, original);

        state.timeblocking.title = "x".repeat(MAX_TODO_CHARS + 1);
        let _ = update(&mut state, Action::SaveTitle("task:deep".into()));
        assert!(state.timeblocking.title_error.is_some());
        assert!(state.timeblocking.busy.is_empty());
        assert_eq!(state.calendar.event("task:deep").unwrap().title, original);

        let _ = update(&mut state, Action::Title(original.clone()));
        assert!(state.timeblocking.title_error.is_none());
        let _ = update(&mut state, Action::SaveTitle("task:deep".into()));
        assert!(state.timeblocking.renaming.is_none());
        assert!(state.timeblocking.busy.is_empty());

        state.timeblocking.title = format!("  {original} revised  ");
        state.timeblocking.renaming = Some("task:deep".into());
        state.timeblocking.busy.insert("task:deep");
        let _ = update(
            &mut state,
            Action::Saved("task:deep".into(), Err("Not connected to Google".into())),
        );
        assert_eq!(state.calendar.event("task:deep").unwrap().title, original);
        assert_eq!(
            state.timeblocking.title_error.as_deref(),
            Some("Not connected to Google")
        );
        assert!(state.calendar.error.is_none());
        assert_eq!(state.calendar.open_event.as_deref(), Some("task:deep"));
        assert!(state.timeblocking.renaming.is_none());

        let mut saved = state.calendar.event("task:deep").unwrap().clone();
        saved.title = format!("{original} revised");
        state.timeblocking.renaming = Some("task:deep".into());
        let _ = update(&mut state, Action::Saved("task:deep".into(), Ok(saved)));
        assert_eq!(
            state.calendar.event("task:deep").unwrap().title,
            format!("{original} revised")
        );
        assert!(state.timeblocking.title_error.is_none());
        assert!(state.timeblocking.renaming.is_none());
    }

    #[test]
    fn a_saved_todo_replaces_the_draft_and_a_failure_keeps_it() {
        let mut state = estimate_state();
        state.timeblocking.todo_open = true;
        state.timeblocking.todo_draft = "Call the client".into();
        state.timeblocking.busy.insert(BusyKey::AddingTodo);
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
        assert!(state.timeblocking.busy.is_empty());
        assert_eq!(
            state.calendar.event("task:fresh").unwrap().title,
            "Call the client"
        );

        state.timeblocking.todo_open = true;
        state.timeblocking.todo_draft = "Call the client".into();
        state.timeblocking.busy.insert(BusyKey::AddingTodo);
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
