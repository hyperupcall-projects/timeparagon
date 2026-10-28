//! Local task metadata and Google Tasks time-block operations.
use crate::{
    app::{Message, State},
    blocking,
    calendar::{self, Event},
    config::CalendarSettings,
};
use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone};
use iced::{Size, Task, window};

mod model;
pub use model::*;

pub const PANEL_WIDTH: f32 = 400.0;
/// Busy-set key for a todo that does not have a task id until Google creates it.
pub(crate) const ADDING_TODO: &str = "adding-todo";
const MAX_TODO_CHARS: usize = 1024;
#[derive(Default)]
pub struct Workspace {
    pub block: Option<String>,
    pub focus: Option<String>,
    pub history_open: bool,
    pub work_now: Option<chrono::DateTime<chrono::Utc>>,
    /// The work session open in the editor, as an index into the work log.
    pub session_edit: Option<usize>,
    /// The day the edited session started, so its times can be parsed back.
    pub session_date: Option<NaiveDate>,
    /// The start and end being edited, as local `HH:MM`.
    pub session_start: String,
    pub session_end: String,
    pub session_error: Option<String>,
    pub revision: u64,
    pub original_width: Option<f32>,
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
    /// Include tasks from all dates instead of only the current day.
    pub available_all: bool,
    /// Include tasks that already have a due time or calendar schedule.
    pub available_planned: bool,
    pub dragging: Option<String>,
    /// Where the pointer is while a task is being dragged, so the floating
    /// card can follow it. Only meaningful while `dragging` is set.
    pub drag_pointer: Option<iced::Point>,
    pub busy: std::collections::HashSet<String>,
    /// A task whose project or priority is being saved. Saving those leaves
    /// the details open, since they are picked with a click and the next one
    /// often follows.
    pub keep_open: Option<String>,
    /// Text that narrows the backlog by task title or list name.
    pub backlog_query: String,
    /// When set, the backlog shows only tasks from this Google Tasks list.
    pub backlog_list: Option<String>,
    /// Text that narrows the project picker in the task editor.
    pub project_query: String,
    /// The task whose project picker is open.
    pub project_picker: Option<String>,
    /// The label being typed for the open task, before it is added.
    pub label_draft: String,
    pub label_error: Option<String>,
}
#[derive(Debug, Clone)]
pub enum Action {
    View(String),
    CloseView,
    ToggleHistory,
    StartWork(String),
    StopWork(String),
    WorkTick,
    /// Open the editor for the work session at this index in the log.
    EditSession(usize),
    SessionStart(String),
    SessionEnd(String),
    SaveSession,
    CloseSession,
    DeleteSession,
    HideOverdue(bool),
    Deadline(String),
    StartTime(String),
    EndTime(String),
    ScheduleEntered(String),
    SaveDeadline(String),
    Schedule(String, DateTime<Local>, DateTime<Local>),
    DropTime(DateTime<Local>),
    RefreshBlock(String),
    List(String),
    BacklogDestination(Option<String>),
    Close,
    Details(String),
    CancelDrag,
    EditEstimate(String),
    CloseEstimate,
    Estimate(String),
    Date(String),
    SaveEstimate(String),
    Backlog(String),
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
    DropDay,
    Saved(String, Result<Event, String>),
    SavedMany(String, Result<Vec<Event>, String>),
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
                .strip_prefix("tasks:")
                .filter(|id| !id.is_empty())
                .unwrap_or("Tasks")
                .to_owned()
        })
}

/// When a task is due: its notes deadline if it has one, otherwise the
/// Google Tasks due day. `None` for a task with neither.
pub fn due(event: &Event) -> Option<calendar::deadline::Deadline> {
    if let Ok(Some(deadline)) = calendar::deadline::read_deadline(&event.details.description) {
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
    state.timeblocking.session_edit = None;
    state.timeblocking.session_error = None;
    state.timeblocking.dragging = None;
    state.timeblocking.drag_pointer = None;
    state.timeblocking.available_query.clear();
    state.timeblocking.available_all = false;
    state.timeblocking.available_planned = false;
    // A save still in flight keeps the draft so its result can land on the same field.
    if !state.timeblocking.busy.contains(ADDING_TODO) {
        state.timeblocking.todo_open = false;
        state.timeblocking.todo_draft.clear();
        state.timeblocking.todo_error = None;
    }
    match (state.main_window, state.timeblocking.original_width.take()) {
        (Some(id), Some(width)) => {
            let width = width
                + if state.details_pane_open {
                    state.config.views.detail_width()
                } else {
                    0.0
                };
            let resize = window::resize(id, Size::new(width, state.main_size.height));
            if state.sidebar.is_some() && !state.details_pane_open {
                resize.chain(window::set_max_size(id, Some(Size::new(width, 100_000.0))))
            } else {
                resize
            }
        }
        _ => Task::none(),
    }
}
pub(crate) fn editor_width(state: &State, base_width: f32) -> f32 {
    let pane_width = state.config.views.time_block_width();
    state
        .config
        .window
        .time_block_width
        .filter(|width| width.is_finite())
        .unwrap_or(base_width + pane_width)
        .max(base_width + pane_width)
}
pub fn open(state: &mut State, id: String) -> Task<Message> {
    if state.timeblocking.block.as_deref() != Some(id.as_str())
        && !state.timeblocking.busy.contains(ADDING_TODO)
    {
        state.timeblocking.todo_open = false;
        state.timeblocking.todo_draft.clear();
        state.timeblocking.todo_error = None;
        state.timeblocking.available_query.clear();
        state.timeblocking.available_all = false;
        state.timeblocking.available_planned = false;
    }
    state.timeblocking.focus = None;
    // The panel edits the block in place, so it always opens with a live
    // draft of the block's own fields.
    state.calendar.edit_draft = state
        .calendar
        .event(&id)
        .map(crate::calendar::EditDraft::from_event);
    state.timeblocking.block = Some(id);
    if state.screen == crate::app::Screen::Calendar && state.weekly {
        return Task::none();
    }
    if state.timeblocking.original_width.is_none() {
        let width = state.main_size.width;
        state.timeblocking.original_width = Some(width);
        if let Some(id) = state.main_window {
            let resize = window::resize(
                id,
                Size::new(editor_width(state, width), state.main_size.height),
            );
            return if state.sidebar.is_some() {
                window::set_max_size(id, None).chain(resize)
            } else {
                resize
            };
        }
    }
    Task::none()
}
pub fn update(state: &mut State, action: Action) -> Task<Message> {
    let (id, change) = match action {
        Action::View(id) => {
            if !state
                .calendar
                .event(&id)
                .is_some_and(|e| is_block(e, &state.config.calendar))
            {
                return Task::none();
            }
            let resize = close(state);
            // The block's fields are edited in place in the panel, so seed the
            // draft as the panel opens rather than behind an Edit button.
            state.calendar.edit_draft = state
                .calendar
                .event(&id)
                .map(crate::calendar::EditDraft::from_event);
            state.timeblocking.focus = Some(id);
            state.timeblocking.history_open = false;
            state.timeblocking.work_now = Some(chrono::Utc::now());
            state.calendar.open_event = None;
            state.calendar.menu_open = false;
            state.calendar.month_open = false;
            let unfocus = iced::advanced::widget::operate(
                iced::advanced::widget::operation::focusable::unfocus(),
            );
            return Task::batch([resize, unfocus]);
        }
        Action::CloseView => {
            state.timeblocking.focus = None;
            return Task::none();
        }
        Action::ToggleHistory => {
            state.timeblocking.history_open = !state.timeblocking.history_open;
            return Task::none();
        }
        Action::WorkTick => {
            state.timeblocking.work_now = Some(chrono::Utc::now());
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
            state.timeblocking.work_now = Some(chrono::Utc::now());
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
                state.timeblocking.work_now = Some(chrono::Utc::now());
                return crate::app::queue_save(state);
            }
            return Task::none();
        }
        Action::StartWork(id) => {
            let Some(task) = state
                .calendar
                .event(&id)
                .filter(|e| e.is_task() && !e.completed && !state.timeblocking.busy.contains(&id))
            else {
                return Task::none();
            };
            let now = chrono::Utc::now();
            state.config.task_work.start(
                task_key(task),
                task.title.clone(),
                metadata(task).block,
                now,
            );
            state.timeblocking.work_now = Some(now);
            return crate::app::queue_save(state);
        }
        Action::StopWork(id) => {
            if let Some(task) = state.calendar.event(&id) {
                state
                    .config
                    .task_work
                    .stop(&task_key(task), chrono::Utc::now());
                state.timeblocking.work_now = Some(chrono::Utc::now());
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
            let range = (|| {
                let day = calendar::deadline::parse_date(state.timeblocking.date.trim())?;
                let (sh, sm) = calendar::parse_clock(&state.timeblocking.start_time)
                    .ok_or("Enter a start time as HH:MM")?;
                let (eh, em) = calendar::parse_clock(&state.timeblocking.end_time)
                    .ok_or("Enter an end time as HH:MM")?;
                let start = Local
                    .from_local_datetime(&day.and_hms_opt(sh, sm, 0).unwrap())
                    .earliest()
                    .ok_or("This start time does not exist in your time zone")?;
                let end_day = if (eh, em) < (sh, sm) {
                    day.succ_opt().ok_or("Invalid end date")?
                } else {
                    day
                };
                let end = Local
                    .from_local_datetime(&end_day.and_hms_opt(eh, em, 0).unwrap())
                    .earliest()
                    .ok_or("This end time does not exist in your time zone")?;
                Ok::<_, String>((start, end))
            })();
            return match range {
                Ok((start, end)) => update(state, Action::Schedule(id, start, end)),
                Err(error) => {
                    state.calendar.error = Some(error);
                    Task::none()
                }
            };
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
        Action::DropTime(start) => {
            let Some(id) = state.timeblocking.dragging.take() else {
                return Task::none();
            };
            let minutes = state
                .calendar
                .event(&id)
                .map(|t| metadata(t).minutes)
                .unwrap_or(0);
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
        Action::List(id) => {
            state.config.calendar.time_block_list = Some(id);
            return crate::app::queue_save(state);
        }
        Action::BacklogDestination(id) => {
            state.config.calendar.backlog_list = id;
            return crate::app::queue_save(state);
        }
        Action::Close => return close(state),
        Action::CancelDrag => {
            state.timeblocking.dragging = None;
            state.timeblocking.drag_pointer = None;
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
            let Some(task) = state.calendar.event(&id).filter(|e| e.is_task()) else {
                return Task::none();
            };
            if state.timeblocking.busy.contains(&id) {
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
            if let Some(id) = state.timeblocking.dragging.take() {
                return update(state, Action::Assign(id, block));
            }
            return Task::none();
        }
        Action::DropTask(target) => {
            let Some(id) = state.timeblocking.dragging.take() else {
                return Task::none();
            };
            if id == target {
                return Task::none();
            }
            let Some(target) = state.calendar.event(&target) else {
                return Task::none();
            };
            let m = metadata(target);
            let previous = state
                .calendar
                .events
                .iter()
                .filter(|e| e.is_task() && e.id != id && e.id != target.id)
                .map(|e| metadata(e))
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
            let Some(id) = state.timeblocking.dragging.take() else {
                return Task::none();
            };
            (id, Change::Day(state.calendar.day))
        }
        Action::SavedMany(id, result) => {
            state.timeblocking.busy.remove(&id);
            let mut tasks = Vec::new();
            match result {
                Ok(events) => {
                    for (index, event) in events.into_iter().enumerate() {
                        let previous_id = if index == 0 {
                            id.clone()
                        } else {
                            event.id.clone()
                        };
                        tasks.push(update(state, Action::Saved(previous_id, Ok(event))));
                    }
                }
                Err(error) => {
                    state.timeblocking.sync_notice = Some(error.clone());
                    tasks.push(update(state, Action::Saved(id, Err(error))));
                }
            }
            // Also reconciles a partially successful cross-service write.
            tasks.push(Task::done(Message::CalendarSyncRequested));
            return Task::batch(tasks);
        }
        Action::Saved(id, result) => {
            state.timeblocking.revision += 1;
            state.timeblocking.busy.remove(&id);
            let mut refresh_week = false;
            let editing_estimate = state.timeblocking.estimate_task.as_deref() == Some(&id);
            let renaming = state.timeblocking.renaming.as_deref() == Some(&id);
            let keep_open = state
                .timeblocking
                .keep_open
                .take_if(|task| *task == id)
                .is_some();
            match result {
                Ok(event) => {
                    if renaming {
                        state.timeblocking.renaming = None;
                        state.timeblocking.title_error = None;
                    }
                    if id == ADDING_TODO {
                        state.timeblocking.todo_draft.clear();
                        state.timeblocking.todo_error = None;
                        state.timeblocking.todo_open = false;
                    }
                    if let Some(link) = &event.details.scheduled {
                        // Remember the app calendar the first time an event
                        // lands on it, so blocks and tasks are recognized.
                        if state.config.calendar.app_calendar.is_none() {
                            state.config.calendar.app_calendar = Some(link.calendar_id.clone());
                        }
                        if state.config.calendar.time_block_list.is_none() {
                            state.config.calendar.time_block_list =
                                Some(format!("tasks:{}", link.list_id));
                        }
                    }
                    let key = task_key(&event);
                    state
                        .calendar
                        .events
                        .retain(|e| e.id != id && (!e.is_task() || task_key(e) != key));
                    if state.screen == crate::app::Screen::Calendar && state.weekly {
                        state.calendar.replace_cached_task(&id, &event);
                        refresh_week = true;
                    }
                    state.calendar.add_event(event);
                    if editing_estimate {
                        state.timeblocking.estimate_task = None;
                        state.timeblocking.estimate_error = None;
                    } else if !keep_open {
                        state.calendar.open_event = None;
                    }
                    if !refresh_week {
                        state.calendar.invalidate_other_days();
                    }
                }
                Err(error) if editing_estimate => state.timeblocking.estimate_error = Some(error),
                Err(error) if renaming => {
                    state.timeblocking.renaming = None;
                    if state.calendar.open_event.as_deref() == Some(&id) {
                        state.timeblocking.title_error = Some(error);
                    } else {
                        state.calendar.error = Some(error);
                    }
                }
                Err(error) if id == ADDING_TODO => {
                    state.timeblocking.todo_error = Some(error);
                    state.timeblocking.todo_open = true;
                }
                Err(error) => state.calendar.error = Some(error),
            }
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
            if minutes > 10080 {
                state.timeblocking.estimate_error =
                    Some("Estimate must be at most 10080 minutes".into());
                return Task::none();
            }
            (id, Change::Estimate(minutes))
        }
        Action::Backlog(id) => (id, Change::Backlog),
        Action::Unassign(id) => (id, Change::Unassign),
        Action::Reschedule(id) => {
            let Ok(day) = NaiveDate::parse_from_str(&state.timeblocking.date, "%Y-%m-%d") else {
                state.calendar.error = Some("Enter a date as YYYY-MM-DD".into());
                return Task::none();
            };
            (id, Change::Day(day))
        }
        Action::ScheduleDay(id, day) => (id, Change::Day(day)),
        Action::Assign(id, block) => {
            let Some(event) = state
                .calendar
                .event(&block)
                .filter(|e| is_block(e, &state.config.calendar))
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
            let Some(task) = state.calendar.event(&id).filter(|event| event.is_task()) else {
                return Task::none();
            };
            if task.title == title {
                state.timeblocking.title_error = None;
                return Task::none();
            }
            state.timeblocking.title_error = None;
            (id, Change::Title(title))
        }
        Action::ProjectAdd(id, project) => {
            if state.config.projects.get(project).is_none() {
                return Task::none();
            }
            let Some(task) = state.calendar.event(&id).filter(|event| event.is_task()) else {
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
            let Some(task) = state.calendar.event(&id).filter(|event| event.is_task()) else {
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
            let Some(task) = state.calendar.event(&id).filter(|event| event.is_task()) else {
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
            let Some(task) = state.calendar.event(&id).filter(|event| event.is_task()) else {
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
            let Some(task) = state.calendar.event(&id).filter(|event| event.is_task()) else {
                return Task::none();
            };
            if metadata(task).priority == priority {
                return Task::none();
            }
            (id, Change::Priority(priority))
        }
        Action::Location(id, location) => {
            let Some(task) = state.calendar.event(&id).filter(|event| event.is_task()) else {
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
            if state.timeblocking.busy.contains(ADDING_TODO) {
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
    if !state.timeblocking.busy.is_empty() {
        return Task::none();
    }
    let Some(task) = state.calendar.event(&id).filter(|e| e.is_task()).cloned() else {
        return Task::none();
    };
    if matches!(change, Change::Title(_)) {
        state.timeblocking.renaming = Some(id.clone());
    }
    if matches!(
        change,
        Change::Projects(_) | Change::Labels(_) | Change::Priority(_) | Change::Location(_)
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
        .map(move |result| Message::TimeBlocking(Action::SavedMany(id.clone(), result)))
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
    if !state.timeblocking.busy.is_empty() {
        return Task::none();
    }
    let Some(block) = state
        .calendar
        .event(block_id)
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
    state.timeblocking.busy.insert(ADDING_TODO.to_owned());
    blocking::run(move || {
        create_and_assign(title, settings, key, start, end, order, blocks, related)
    })
    .map(|result| Message::TimeBlocking(Action::SavedMany(ADDING_TODO.to_owned(), result)))
}

fn create_and_assign(
    title: String,
    settings: CalendarSettings,
    key: String,
    start: DateTime<Local>,
    end: DateTime<Local>,
    order: i64,
    blocks: Vec<Event>,
    related: Vec<Event>,
) -> Result<Vec<Event>, String> {
    let token = access(&settings)?;
    let (_, list_id) = calendar::scheduled::destinations(
        &token,
        settings.app_calendar.as_deref(),
        settings.time_block_list.as_deref(),
    )?;
    let created = calendar::api::insert_task(&token, &list_id, &title)?;
    let task = new_todo_event(&list_id, &created, start);
    save_and_reflow(
        task,
        settings,
        Change::Block(key, start, end),
        order,
        blocks,
        related,
    )
}

fn new_todo_event(
    list_id: &str,
    created: &calendar::api::CreatedTask,
    when: DateTime<Local>,
) -> Event {
    Event {
        id: format!("task:{}", created.id),
        calendar_id: format!("tasks:{list_id}"),
        title: created.title.clone(),
        start: when,
        end: when + chrono::Duration::minutes(30),
        all_day: true,
        color: calendar::colors::TASK,
        completed: false,
        details: calendar::Details {
            link: created.link.clone(),
            ..Default::default()
        },
    }
}

/// Trims a typed label and collapses runs of whitespace. Labels are short
/// enough to fit on a task card, so very long ones are rejected.
fn clean_label(input: &str) -> Result<String, &'static str> {
    let label = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if label.is_empty() {
        Err("A label needs a name")
    } else if label.chars().count() > 40 {
        Err("A label can be at most 40 characters")
    } else {
        Ok(label)
    }
}

enum Change {
    Title(String),
    Estimate(u32),
    Projects(Vec<u64>),
    Labels(Vec<String>),
    Priority(Option<Priority>),
    Location(Option<u64>),
    Deadline(Option<calendar::deadline::Deadline>),
    Backlog,
    Unassign,
    Day(NaiveDate),
    Block(String, DateTime<Local>, DateTime<Local>),
    Reorder(Option<String>, i64, NaiveDate),
    Time(DateTime<Local>, DateTime<Local>),
}

pub(crate) fn access(settings: &CalendarSettings) -> Result<String, String> {
    let tokens = calendar::oauth::load_tokens().ok_or("Not connected to Google")?;
    calendar::oauth::ensure_access(&settings.client_id, &settings.client_secret, tokens)
        .map(|tokens| tokens.access_token)
}

fn save_and_reflow(
    task: Event,
    settings: CalendarSettings,
    change: Change,
    order: i64,
    blocks: Vec<Event>,
    mut related: Vec<Event>,
) -> Result<Vec<Event>, String> {
    let original_id = task.id.clone();
    let old_block = metadata(&task).block;
    let saved = save(task, &settings, change, order)?;
    let new_block = metadata(&saved).block;
    related.retain(|e| e.id != original_id);
    related.push(saved.clone());
    let mut changed = vec![saved];
    for block in blocks.iter().filter(|b| {
        let key = block_key(b);
        old_block.as_ref() == Some(&key) || new_block.as_ref() == Some(&key)
    }) {
        for event in reflow(block, &related, &settings)? {
            if let Some(previous) = changed.iter_mut().find(|e| e.id == event.id) {
                *previous = event;
            } else {
                changed.push(event);
            }
        }
    }
    Ok(changed)
}

fn refresh_block(state: &mut State, id: &str) -> Task<Message> {
    let Some(block) = state
        .calendar
        .event(id)
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
        .map(move |result| Message::TimeBlocking(Action::SavedMany(first.clone(), result)))
}

fn reflow(
    block: &Event,
    tasks: &[Event],
    settings: &CalendarSettings,
) -> Result<Vec<Event>, String> {
    let key = block_key(block);
    let mut tasks: Vec<_> = tasks
        .iter()
        .filter(|e| metadata(e).block.as_deref() == Some(&key))
        .cloned()
        .collect();
    tasks.sort_by_key(|e| (metadata(e).order, e.id.clone()));
    let mut start = block.start;
    let mut changed = Vec::new();
    let mut token = None;
    for task in tasks {
        let minutes = metadata(&task).minutes;
        let end =
            start + chrono::Duration::minutes(i64::from(if minutes == 0 { 30 } else { minutes }));
        // Completed blocks are history and are never shifted by later planning.
        if !task.completed && (task.all_day || task.start != start || task.end != end) {
            let token = match &token {
                Some(token) => token,
                None => token.insert(access(settings)?),
            };
            let mut database = crate::task_db::open()?;
            let mut task = task;
            let mut data = database.import(&task)?;
            schedule(
                &mut task,
                &mut data,
                &mut database,
                settings,
                token,
                start,
                end,
            )?;
            changed.push(task);
        }
        start = end;
    }
    Ok(changed)
}

fn save(
    mut task: Event,
    settings: &CalendarSettings,
    change: Change,
    order: i64,
) -> Result<Event, String> {
    let mut database = crate::task_db::open()?;
    let mut data = database.import(&task)?;
    // A previous attempt may have moved the task before Calendar failed. The
    // durable link retains its new identity so retrying the old chip is safe.
    if let Some(link) = &data.metadata.schedule {
        task.id = format!("task:{}", link.task_id);
        task.calendar_id = format!("tasks:{}", link.list_id);
        task.details.scheduled = Some(link.clone());
    }
    task.details.description = description(&task.details.description);
    match change {
        // Tasks own the name. A scheduled block copies that name onto its calendar event.
        Change::Title(title) => {
            let token = access(settings)?;
            calendar::api::patch(
                &task_url(&task)?,
                &token,
                &serde_json::json!({ "title": title }).to_string(),
                "Rename task",
            )?;
            task.title = title;
            if let Some(link) = &data.metadata.schedule {
                if !link.pending {
                    calendar::scheduled::request(
                        "PATCH",
                        &link.event_url(),
                        &token,
                        Some(&calendar::scheduled::content(&task, link)),
                    )
                    .map_err(|error| error.message)?;
                }
            }
        }
        Change::Estimate(minutes) => {
            data.metadata.minutes = minutes;
            database.save(&task, &data)?;
        }
        Change::Projects(projects) => {
            data.metadata.projects = projects;
            database.save(&task, &data)?;
        }
        Change::Labels(labels) => {
            data.metadata.labels = labels;
            database.save(&task, &data)?;
        }
        Change::Priority(priority) => {
            data.metadata.priority = priority;
            database.save(&task, &data)?;
        }
        Change::Location(location) => {
            data.metadata.location = location;
            database.save(&task, &data)?;
        }
        Change::Deadline(deadline) => {
            let token = access(settings)?;
            let notes = calendar::deadline::write(&task.details.description, deadline);
            let base = task_url(&task)?;
            calendar::api::patch(
                &base,
                &token,
                &serde_json::json!({"notes": notes}).to_string(),
                "Save task deadline",
            )?;
            task.details.description = notes;
            if let Some(link) = &data.metadata.schedule {
                calendar::scheduled::request(
                    "PATCH",
                    &link.event_url(),
                    &token,
                    Some(&calendar::scheduled::content(&task, link)),
                )
                .map_err(|e| e.message)?;
            }
        }
        Change::Backlog | Change::Unassign | Change::Day(_) => {
            let day = match change {
                Change::Day(day) => Some(day),
                Change::Unassign => Some(task.start.date_naive()),
                _ => None,
            };
            let token = access(settings)?;
            unschedule_with(
                &mut task,
                &mut data,
                &mut database,
                settings,
                &calendar::scheduled::Google { token: &token },
                day,
            )?;
        }
        Change::Block(key, start, _end) => {
            data.metadata.block = Some(key);
            data.metadata.order = order;
            let minutes = if data.metadata.minutes == 0 {
                30
            } else {
                data.metadata.minutes
            };
            let token = access(settings)?;
            schedule(
                &mut task,
                &mut data,
                &mut database,
                settings,
                &token,
                start,
                start + chrono::Duration::minutes(i64::from(minutes)),
            )?;
        }
        Change::Time(start, end) => {
            data.metadata.block = None;
            let token = access(settings)?;
            schedule(
                &mut task,
                &mut data,
                &mut database,
                settings,
                &token,
                start,
                end,
            )?;
        }
        Change::Reorder(block, order, day) => {
            data.metadata.block = block;
            data.metadata.order = order;
            if data.metadata.block.is_some() {
                let token = access(settings)?;
                let start = if task.all_day {
                    Local
                        .from_local_datetime(&day.and_hms_opt(9, 0, 0).unwrap())
                        .earliest()
                        .ok_or("Invalid local date")?
                } else {
                    task.start
                };
                let minutes = if data.metadata.minutes == 0 {
                    30
                } else {
                    data.metadata.minutes
                };
                schedule(
                    &mut task,
                    &mut data,
                    &mut database,
                    settings,
                    &token,
                    start,
                    start + chrono::Duration::minutes(i64::from(minutes)),
                )?;
            } else {
                database.save(&task, &data)?;
            }
        }
    }
    task.details.task_data = Some(data);
    Ok(task)
}

/// Remove the schedule, then move backlog tasks while retaining their local ID.
/// Day-only rescheduling keeps the current Google Tasks list.
fn unschedule_with(
    task: &mut Event,
    data: &mut crate::task_db::TaskData,
    database: &mut crate::task_db::Database,
    settings: &CalendarSettings,
    remote: &impl calendar::scheduled::Remote,
    day: Option<NaiveDate>,
) -> Result<(), String> {
    // Resolve/create the list before changing the task's existing schedule.
    let destination = if day.is_none() {
        Some(remote.backlog_list(settings.backlog_list.as_deref())?)
    } else {
        None
    };
    let mut link = match &data.metadata.schedule {
        Some(link) => link.clone(),
        None => calendar::scheduled::Link::new(String::new(), task)?,
    };
    if data.metadata.schedule.is_some() {
        remote.remove_event(&link)?;
    }
    remote.set_day(&link, day)?;
    data.metadata.block = None;
    data.metadata.schedule = None;
    task.details.scheduled = None;
    task.all_day = true;
    let day = day.unwrap_or(UNDATED);
    task.start = Local
        .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .ok_or("Invalid local date")?;
    task.end = task.start + chrono::Duration::days(1);
    // If moving lists fails, sync still sees the successfully cleared schedule.
    database.save(task, data)?;
    if let Some(destination) = destination.filter(|destination| *destination != link.list_id) {
        let response = remote.move_task(&link, &destination)?;
        link.task_id = response["id"]
            .as_str()
            .ok_or("Google did not return the moved task ID")?
            .to_owned();
        task.id = format!("task:{}", link.task_id);
        task.calendar_id = format!("tasks:{destination}");
        task.details.link = response["webViewLink"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "https://tasks.google.com/task/{}",
                    urlencoding::encode(&link.task_id)
                )
            });
        task.color = settings
            .task_color(&task.calendar_id)
            .unwrap_or(calendar::colors::TASK);
        database.save(task, data)?;
    }
    Ok(())
}

fn task_url(task: &Event) -> Result<String, String> {
    let list = task
        .calendar_id
        .strip_prefix("tasks:")
        .ok_or("Invalid task list")?;
    let id = task.id.strip_prefix("task:").ok_or("Invalid task")?;
    Ok(format!(
        "{}/{}/tasks/{}",
        calendar::api::TASKS,
        urlencoding::encode(list),
        urlencoding::encode(id)
    ))
}

fn schedule(
    task: &mut Event,
    data: &mut crate::task_db::TaskData,
    database: &mut crate::task_db::Database,
    settings: &CalendarSettings,
    token: &str,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<(), String> {
    schedule_with(
        task,
        data,
        database,
        settings,
        &calendar::scheduled::Google { token },
        start,
        end,
    )
}

fn schedule_with(
    task: &mut Event,
    data: &mut crate::task_db::TaskData,
    database: &mut crate::task_db::Database,
    settings: &CalendarSettings,
    remote: &impl calendar::scheduled::Remote,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<(), String> {
    let old = data.metadata.schedule.clone();
    if let Some(link) = &old {
        if task.id != format!("task:{}", link.task_id)
            || task.calendar_id != format!("tasks:{}", link.list_id)
        {
            task.id = format!("task:{}", link.task_id);
            task.calendar_id = format!("tasks:{}", link.list_id);
            task.details.link = format!(
                "https://tasks.google.com/task/{}",
                urlencoding::encode(&link.task_id)
            );
        }
    }
    let (destination_calendar, destination_list) = remote.destinations(
        settings
            .app_calendar
            .as_deref()
            .or_else(|| old.as_ref().map(|l| l.calendar_id.as_str())),
        settings
            .time_block_list
            .as_deref()
            .or_else(|| old.as_ref().map(|l| l.list_id.as_str())),
    )?;
    let mut link = match old.clone() {
        Some(link) => link,
        None => calendar::scheduled::Link::new(destination_calendar.clone(), task)?,
    };
    // Reserve a stable event ID before any remote write. Preserve aliases and
    // this reservation even if a later API call fails.
    link.calendar_id = destination_calendar;
    // Keep Google's event color in step with the chosen task color.
    link.color_id = calendar::colors::event_id(settings.task_event_color()).map(str::to_owned);
    link.pending = true;
    link.block = data.metadata.block.clone();
    link.order = data.metadata.order;
    data.metadata.schedule = Some(link.clone());
    database.save(task, data)?;
    if link.list_id != destination_list {
        let response = remote.move_task(&link, &destination_list)?;
        link.task_id = response["id"]
            .as_str()
            .ok_or("Google did not return the moved task ID")?
            .to_owned();
        link.list_id = destination_list;
        task.id = format!("task:{}", link.task_id);
        task.calendar_id = format!("tasks:{}", link.list_id);
        task.details.link = response["webViewLink"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "https://tasks.google.com/task/{}",
                    urlencoding::encode(&link.task_id)
                )
            });
        data.metadata.schedule = Some(link.clone());
        database.save(task, data)?;
    }
    // The native Tasks date is the planned day, never the description deadline.
    remote.set_day(&link, Some(start.date_naive()))?;
    remote.save_event(task, &link, start, end).map_err(|e| {
        format!("Task saved, but its calendar block could not be saved: {e}. Sync or retry.")
    })?;
    if let Some(old) = old.filter(|old| old.calendar_id != link.calendar_id) {
        remote.remove_event(&old)?;
    }
    link.pending = false;
    task.start = start;
    task.end = end;
    task.all_day = false;
    task.details.scheduled = Some(link.clone());
    data.metadata.schedule = Some(link);
    database.save(task, data)?;
    task.details.task_data = Some(data.clone());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeGoogle {
        calls: std::cell::RefCell<Vec<String>>,
        fail_calendar: std::cell::Cell<bool>,
        event_ids: std::cell::RefCell<Vec<String>>,
    }
    impl calendar::scheduled::Remote for FakeGoogle {
        fn destinations(
            &self,
            calendar: Option<&str>,
            list: Option<&str>,
        ) -> Result<(String, String), String> {
            Ok((
                calendar.unwrap_or("custom-calendar").to_owned(),
                list.unwrap_or("has-time")
                    .trim_start_matches("tasks:")
                    .to_owned(),
            ))
        }
        fn move_task(
            &self,
            link: &calendar::scheduled::Link,
            destination: &str,
        ) -> Result<serde_json::Value, String> {
            assert_eq!(link.list_id, "original-list");
            assert_eq!(destination, "has-time");
            self.calls.borrow_mut().push("move".into());
            Ok(
                serde_json::json!({"id":"moved-task", "webViewLink":"https://tasks.google.com/task/moved-task"}),
            )
        }
        fn backlog_list(&self, _list: Option<&str>) -> Result<String, String> {
            unreachable!("Scheduling does not use the backlog list")
        }
        fn set_day(
            &self,
            link: &calendar::scheduled::Link,
            _day: Option<NaiveDate>,
        ) -> Result<(), String> {
            assert_eq!(link.task_id, "moved-task");
            self.calls.borrow_mut().push("day".into());
            Ok(())
        }
        fn save_event(
            &self,
            task: &Event,
            link: &calendar::scheduled::Link,
            _start: DateTime<Local>,
            _end: DateTime<Local>,
        ) -> Result<(), String> {
            assert_eq!(task.id, "task:moved-task");
            assert_eq!(
                task.details.description,
                "DUE: 2026-12-31\n\nKeep these notes."
            );
            assert_eq!(link.calendar_id, "custom-calendar");
            self.calls.borrow_mut().push("event".into());
            self.event_ids.borrow_mut().push(link.event_id.clone());
            if self.fail_calendar.get() {
                Err("Calendar is offline".into())
            } else {
                Ok(())
            }
        }
        fn remove_event(&self, _link: &calendar::scheduled::Link) -> Result<(), String> {
            panic!("History must not be removed")
        }
    }

    #[test]
    fn a_failed_calendar_write_can_retry_after_a_list_move_without_losing_identity_or_deadline() {
        let mut task = calendar::sample_snapshot().events.remove(0);
        task.id = "task:original-task".into();
        task.calendar_id = "tasks:original-list".into();
        task.all_day = true;
        task.details.description = "DUE: 2026-12-31\n\nKeep these notes.".into();
        let original = task.clone();
        let scratch = std::env::temp_dir().join(format!(
            "tt-schedule-{}-{}.sqlite",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let mut database = crate::task_db::Database::open(&scratch).unwrap();
        let mut data = database.import(&task).unwrap();
        let local_id = data.id;
        data.metadata.block = Some("work/block".into());
        let start = task.start;
        let end = start + chrono::Duration::minutes(45);
        let remote = FakeGoogle::default();
        remote.fail_calendar.set(true);
        let settings = CalendarSettings {
            app_calendar: Some("custom-calendar".into()),
            time_block_list: Some("tasks:has-time".into()),
            ..Default::default()
        };
        let error = schedule_with(
            &mut task,
            &mut data,
            &mut database,
            &settings,
            &remote,
            start,
            end,
        )
        .unwrap_err();
        assert!(error.contains("Calendar is offline"));
        let persisted = database.lookup(&original).unwrap().unwrap();
        assert_eq!(persisted.id, local_id);
        assert_eq!(
            persisted.metadata.schedule.as_ref().unwrap().task_id,
            "moved-task"
        );
        assert!(persisted.metadata.schedule.as_ref().unwrap().pending);
        assert_eq!(database.lookup(&task).unwrap().unwrap().id, local_id);

        // Simulate retrying the old chip after an app restart.
        task = original;
        data = persisted;
        remote.fail_calendar.set(false);
        schedule_with(
            &mut task,
            &mut data,
            &mut database,
            &settings,
            &remote,
            start,
            end,
        )
        .unwrap();
        assert_eq!(
            remote
                .calls
                .borrow()
                .iter()
                .filter(|call| call.as_str() == "move")
                .count(),
            1
        );
        assert_eq!(remote.event_ids.borrow()[0], remote.event_ids.borrow()[1]);
        assert_eq!(task.id, "task:moved-task");
        assert_eq!(task.calendar_id, "tasks:has-time");
        assert!(!task.all_day);
        assert!(!data.metadata.schedule.as_ref().unwrap().pending);
        assert_eq!(task_key(&task), local_id.to_string());
        assert_eq!(
            task.details.description,
            "DUE: 2026-12-31\n\nKeep these notes."
        );
        assert_eq!(task.start, start);
        assert_eq!(task.end, end);
        drop(database);
        std::fs::remove_file(scratch).unwrap();
    }

    #[derive(Default)]
    struct BacklogGoogle {
        calls: std::cell::RefCell<Vec<String>>,
        fail_move: std::cell::Cell<bool>,
        fail_remove: std::cell::Cell<bool>,
    }

    impl calendar::scheduled::Remote for BacklogGoogle {
        fn destinations(
            &self,
            _: Option<&str>,
            _: Option<&str>,
        ) -> Result<(String, String), String> {
            panic!("Returning to the backlog must not create a calendar")
        }
        fn backlog_list(&self, list: Option<&str>) -> Result<String, String> {
            let list = list
                .unwrap_or("tasks:not-time-blocked")
                .trim_start_matches("tasks:");
            self.calls.borrow_mut().push(format!("list:{list}"));
            Ok(list.to_owned())
        }
        fn move_task(
            &self,
            link: &calendar::scheduled::Link,
            destination: &str,
        ) -> Result<serde_json::Value, String> {
            self.calls
                .borrow_mut()
                .push(format!("move:{}:{destination}", link.task_id));
            if self.fail_move.get() {
                return Err("Move failed".into());
            }
            Ok(serde_json::json!({"id": "backlog-task"}))
        }
        fn set_day(
            &self,
            link: &calendar::scheduled::Link,
            day: Option<NaiveDate>,
        ) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("day:{}:{day:?}", link.task_id));
            Ok(())
        }
        fn save_event(
            &self,
            _: &Event,
            _: &calendar::scheduled::Link,
            _: DateTime<Local>,
            _: DateTime<Local>,
        ) -> Result<(), String> {
            panic!("Returning to the backlog must not write a calendar event")
        }
        fn remove_event(&self, _: &calendar::scheduled::Link) -> Result<(), String> {
            self.calls.borrow_mut().push("remove".into());
            if self.fail_remove.get() {
                Err("Calendar is offline".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn backlog_moves_scheduled_and_existing_backlog_tasks_to_the_default_or_custom_list() {
        for (scheduled, destination) in [(true, None), (false, Some("tasks:custom-backlog"))] {
            let mut task = calendar::sample_snapshot().events.remove(0);
            task.id = "task:original-task".into();
            task.calendar_id = "tasks:has-time".into();
            task.details.description =
                "DEADLINE: 2026-12-31\nSome other note.\nKeep these notes.".into();
            task.details.task_data = None;
            let original = task.clone();
            let mut database =
                crate::task_db::Database::open(std::path::Path::new(":memory:")).unwrap();
            let mut data = database.import(&task).unwrap();
            data.metadata.minutes = 45;
            data.metadata.order = 123;
            if scheduled {
                data.metadata.block = Some("work/block".into());
                data.metadata.schedule =
                    Some(calendar::scheduled::Link::new("calendar".into(), &task).unwrap());
            }
            database.save(&task, &data).unwrap();
            let local_id = data.id;
            let settings = CalendarSettings {
                backlog_list: destination.map(str::to_owned),
                ..Default::default()
            };
            let remote = BacklogGoogle::default();
            unschedule_with(
                &mut task,
                &mut data,
                &mut database,
                &settings,
                &remote,
                None,
            )
            .unwrap();
            task.details.task_data = Some(data.clone());
            let destination = destination.unwrap_or("tasks:not-time-blocked");
            assert_eq!(task.calendar_id, destination);
            assert_eq!(task.id, "task:backlog-task");
            assert_eq!(
                task.details.link,
                "https://tasks.google.com/task/backlog-task"
            );
            assert_eq!(task.title, original.title);
            assert_eq!(task.details.description, original.details.description);
            assert_eq!(task_key(&task), local_id.to_string());
            assert_eq!(data.metadata.minutes, 45);
            assert_eq!(data.metadata.order, 123);
            assert!(data.metadata.block.is_none());
            assert!(data.metadata.schedule.is_none());
            assert!(task.details.scheduled.is_none());
            assert!(task.all_day);
            assert_eq!(task.start.date_naive(), UNDATED);
            assert!(in_backlog(&task));
            assert_eq!(database.lookup(&original).unwrap(), Some(data.clone()));
            assert_eq!(database.lookup(&task).unwrap(), Some(data.clone()));
            let calls = remote.calls.borrow().clone();
            assert_eq!(calls.iter().any(|call| call == "remove"), scheduled);
            assert!(calls.iter().any(|call| call == "day:original-task:None"));
            assert!(calls.last().unwrap().starts_with("move:original-task:"));

            // Repeating the action on a task already on the target list does not move it again.
            remote.calls.borrow_mut().clear();
            unschedule_with(
                &mut task,
                &mut data,
                &mut database,
                &settings,
                &remote,
                None,
            )
            .unwrap();
            assert!(
                !remote
                    .calls
                    .borrow()
                    .iter()
                    .any(|call| call.starts_with("move:"))
            );
        }
    }

    #[test]
    fn removing_a_planned_task_retains_its_day_identity_status_and_list() {
        for status in [
            TaskStatus::Active,
            TaskStatus::Completed,
            TaskStatus::Ignored,
        ] {
            let mut task = calendar::sample_snapshot().events.remove(0);
            task.id = "task:remove".into();
            task.calendar_id = "tasks:has-time".into();
            let mut database =
                crate::task_db::Database::open(std::path::Path::new(":memory:")).unwrap();
            let mut data = database.import(&task).unwrap();
            data.metadata.block = Some("work/block".into());
            data.metadata.schedule =
                Some(calendar::scheduled::Link::new("calendar".into(), &task).unwrap());
            task.details.task_data = Some(data.clone());
            apply_status(&mut task, status);
            data = task.details.task_data.clone().unwrap();
            let original = task.clone();
            let remote = BacklogGoogle::default();
            let day = task.start.date_naive();
            unschedule_with(
                &mut task,
                &mut data,
                &mut database,
                &CalendarSettings::default(),
                &remote,
                Some(day),
            )
            .unwrap();
            task.details.task_data = Some(data.clone());
            assert_eq!(task.id, original.id);
            assert_eq!(task.calendar_id, original.calendar_id);
            assert_eq!(task.start.date_naive(), day);
            assert_eq!(task_status(&task), status);
            assert_eq!(task_key(&task), task_key(&original));
            assert!(data.metadata.block.is_none());
            assert!(data.metadata.schedule.is_none());
            assert_eq!(
                *remote.calls.borrow(),
                ["remove".to_owned(), format!("day:remove:Some({day})")]
            );
        }
    }

    #[test]
    fn a_day_only_change_keeps_the_google_tasks_list() {
        let mut task = calendar::sample_snapshot().events.remove(0);
        task.id = "task:original-task".into();
        task.calendar_id = "tasks:has-time".into();
        let mut database =
            crate::task_db::Database::open(std::path::Path::new(":memory:")).unwrap();
        let mut data = database.import(&task).unwrap();
        data.metadata.schedule =
            Some(calendar::scheduled::Link::new("calendar".into(), &task).unwrap());
        let remote = BacklogGoogle::default();
        let day = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
        unschedule_with(
            &mut task,
            &mut data,
            &mut database,
            &CalendarSettings::default(),
            &remote,
            Some(day),
        )
        .unwrap();
        assert_eq!(task.id, "task:original-task");
        assert_eq!(task.calendar_id, "tasks:has-time");
        assert_eq!(task.start.date_naive(), day);
        assert_eq!(
            *remote.calls.borrow(),
            ["remove", "day:original-task:Some(2026-12-01)"]
        );
    }

    #[test]
    fn a_failed_backlog_move_can_retry_and_calendar_failure_leaves_the_task_untouched() {
        let mut task = calendar::sample_snapshot().events.remove(0);
        task.id = "task:original-task".into();
        task.calendar_id = "tasks:has-time".into();
        let mut database =
            crate::task_db::Database::open(std::path::Path::new(":memory:")).unwrap();
        let mut data = database.import(&task).unwrap();
        data.metadata.block = Some("work/block".into());
        data.metadata.schedule =
            Some(calendar::scheduled::Link::new("calendar".into(), &task).unwrap());
        database.save(&task, &data).unwrap();
        let remote = BacklogGoogle::default();
        remote.fail_remove.set(true);
        let original = task.clone();
        let original_data = data.clone();
        let settings = CalendarSettings::default();
        assert!(
            unschedule_with(
                &mut task,
                &mut data,
                &mut database,
                &settings,
                &remote,
                None
            )
            .is_err()
        );
        assert_eq!(task, original);
        assert_eq!(data, original_data);
        assert_eq!(remote.calls.borrow().len(), 2); // Resolve list, then delete calendar event.

        remote.fail_remove.set(false);
        remote.fail_move.set(true);
        assert!(
            unschedule_with(
                &mut task,
                &mut data,
                &mut database,
                &settings,
                &remote,
                None
            )
            .is_err()
        );
        assert_eq!(task.calendar_id, "tasks:has-time");
        let persisted = database.lookup(&original).unwrap().unwrap();
        assert!(persisted.metadata.block.is_none());
        assert!(persisted.metadata.schedule.is_none());
        assert_eq!(persisted.id, original_data.id);
        remote.fail_move.set(false);
        unschedule_with(
            &mut task,
            &mut data,
            &mut database,
            &settings,
            &remote,
            None,
        )
        .unwrap();
        assert_eq!(task.calendar_id, "tasks:not-time-blocked");
        assert_eq!(
            database.lookup(&task).unwrap().unwrap().id,
            original_data.id
        );
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
    fn overdue_uses_the_description_deadline_independently_of_scheduling() {
        let mut task = calendar::sample_snapshot().events.remove(0);
        task.id = "task:deadline".into();
        task.calendar_id = "tasks:list".into();
        task.start = Local::now() + chrono::Duration::days(5);
        let yesterday = (Local::now() - chrono::Duration::days(1)).date_naive();
        task.details.description = format!("DUE: {yesterday}\nNotes");
        assert!(overdue(&task));
        task.details.description.clear();
        task.start = Local::now() - chrono::Duration::days(5);
        assert!(!overdue(&task));
    }

    fn estimate_state() -> State {
        let mut state = State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        );
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
            legacy_key: None,
        });
        state.timeblocking.focus = Some("standup".into());
        state
    }

    #[test]
    fn a_work_session_can_have_its_start_and_end_edited_or_removed() {
        let mut state = State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        );
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
            assert!(!state.timeblocking.busy.contains("task:deep"));
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
    fn legacy_markers_are_importable_and_removed_from_prose() {
        let old = "human notes\nttid: 12345678-1234-4234-8234-123456789abc\nmore notes\ntt-timeblocking: {\"minutes\":60,\"block\":null,\"order\":1}";
        assert_eq!(description(old), "human notes\nmore notes");
        assert_eq!(legacy_metadata(old).minutes, 60);
        assert_eq!(
            legacy_id(old).as_deref(),
            Some("12345678-1234-4234-8234-123456789abc")
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
        state.timeblocking.busy.insert("task:deep".into());
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
        state.timeblocking.busy.insert(ADDING_TODO.to_owned());
        let event = new_todo_event(
            "has-time",
            &calendar::api::CreatedTask {
                id: "fresh".into(),
                title: "Call the client".into(),
                link: "https://tasks.google.com/task/fresh".into(),
            },
            Local::now(),
        );
        assert!(event.is_task());
        let _ = update(&mut state, Action::Saved(ADDING_TODO.into(), Ok(event)));
        assert!(state.timeblocking.todo_draft.is_empty());
        assert!(!state.timeblocking.todo_open);
        assert!(state.timeblocking.busy.is_empty());
        assert_eq!(
            state.calendar.event("task:fresh").unwrap().title,
            "Call the client"
        );

        state.timeblocking.todo_open = true;
        state.timeblocking.todo_draft = "Call the client".into();
        state.timeblocking.busy.insert(ADDING_TODO.to_owned());
        let _ = update(
            &mut state,
            Action::Saved(ADDING_TODO.into(), Err("Not connected to Google".into())),
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
