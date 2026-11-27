//! Desktop work sessions, distractions, and Pomodoro coordination.
use crate::app::{Message, State};
use crate::calendar::{self, Event};
use crate::timeblocking::{assigned_all, block_key_of, event};
use chrono::{Local, NaiveDate, TimeZone};
use iced::Task;
use timeparagon_core::task_db::TaskData;
use timeparagon_core::timeblocking::{TaskStatus, block_key, task_key, task_status};

#[derive(Debug, Clone)]
pub enum Action {
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
	/// The task with this ID, which had no TimeParagon ID, has been given one
	/// to start work under (see [`identify`]). It goes on the loaded task,
	/// and work on it starts.
	Identified(String, Result<TaskData, String>),
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
	match action {
		Action::WorkTick => {
			let now = crate::clock::utc_now();
			state.work_tracking.work_now = Some(now);
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
					.map(|task| resume_target(state, task, now));
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
			state.work_tracking.work_now = Some(now);
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
			state.work_tracking.session_date = Some(start.date_naive());
			state.work_tracking.session_start = start.format("%H:%M").to_string();
			state.work_tracking.session_end = session
				.stopped_at
				.map(|end| end.with_timezone(&Local).format("%H:%M").to_string())
				.unwrap_or_default();
			state.work_tracking.session_error = None;
			state.work_tracking.session_edit = Some(index);
			return iced::widget::operation::focus(iced::widget::Id::new("work-session-start"));
		}
		Action::SessionStart(value) => {
			state.work_tracking.session_start = value;
			state.work_tracking.session_error = None;
			return Task::none();
		}
		Action::SessionEnd(value) => {
			state.work_tracking.session_end = value;
			state.work_tracking.session_error = None;
			return Task::none();
		}
		Action::SaveSession => {
			let Some(index) = state.work_tracking.session_edit else {
				return Task::none();
			};
			let Some(day) = state.work_tracking.session_date else {
				return Task::none();
			};
			let Some((sh, sm)) = calendar::parse_clock(state.work_tracking.session_start.trim())
			else {
				state.work_tracking.session_error = Some("Enter a start time as HH:MM".into());
				return Task::none();
			};
			let Some(start) = Local
				.from_local_datetime(&day.and_hms_opt(sh, sm, 0).unwrap())
				.earliest()
			else {
				state.work_tracking.session_error =
					Some("This start time does not exist in your time zone".into());
				return Task::none();
			};
			let end_text = state.work_tracking.session_end.trim();
			let end = if end_text.is_empty() {
				None
			} else {
				let Some((eh, em)) = calendar::parse_clock(end_text) else {
					state.work_tracking.session_error = Some("Enter an end time as HH:MM".into());
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
					state.work_tracking.session_error =
						Some("This end time does not exist in your time zone".into());
					return Task::none();
				};
				if end <= start {
					state.work_tracking.session_error =
						Some("The session must end after it starts".into());
					return Task::none();
				}
				Some(end)
			};
			let Some(session) = state.config.task_work.sessions.get_mut(index) else {
				state.work_tracking.session_edit = None;
				return Task::none();
			};
			session.started_at = start.with_timezone(&chrono::Utc);
			session.stopped_at = end.map(|end| end.with_timezone(&chrono::Utc));
			state.work_tracking.session_edit = None;
			state.work_tracking.session_error = None;
			state.work_tracking.work_now = Some(crate::clock::utc_now());
			return crate::app::queue_save(state);
		}
		Action::CloseSession => {
			state.work_tracking.session_edit = None;
			state.work_tracking.session_error = None;
			return Task::none();
		}
		Action::DeleteSession => {
			let Some(index) = state.work_tracking.session_edit.take() else {
				return Task::none();
			};
			if index < state.config.task_work.sessions.len() {
				state.config.task_work.sessions.remove(index);
				state.work_tracking.session_error = None;
				state.work_tracking.work_now = Some(crate::clock::utc_now());
				return crate::app::queue_save(state);
			}
			return Task::none();
		}
		Action::OpenDistraction => {
			state.work_tracking.distraction_open = state.timeblocking.focus.is_some();
			state.work_tracking.distraction_cause = None;
			return Task::none();
		}
		Action::CloseDistraction => {
			state.work_tracking.distraction_open = false;
			return Task::none();
		}
		Action::ChooseDistraction(cause) => {
			state.work_tracking.distraction_cause = Some(cause);
			return Task::none();
		}
		Action::AddDistraction(minutes) => {
			state.work_tracking.distraction_open = false;
			let cause = state.work_tracking.distraction_cause.take();
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
			let saved = crate::app::queue_save(state);
			return match cause {
				Some(DistractionCause::Habit(id)) => {
					Task::batch([saved, crate::habits::open_happened_now(state, id)])
				}
				_ => saved,
			};
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
				e.is_task() && !e.completed && !state.task_operations.busy.contains_task(&id)
			}) else {
				return Task::none();
			};
			if task.details.task_data.is_none() {
				return identify(task.clone());
			}
			if state.config.pomodoro.running()
				&& state.config.pomodoro.phase == crate::pomodoro::Phase::Break
			{
				state.config.pomodoro.target = Some(resume_target(state, task, now));
				return crate::app::queue_save(state);
			}
			let block = block_key_of(state, task);
			state
				.config
				.task_work
				.start(task_key(task), task.title.clone(), block, now);
			state.config.pomodoro.target = state.config.task_work.active().cloned();
			state.work_tracking.work_now = Some(now);
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
				state.work_tracking.work_now = Some(now);
				return crate::app::queue_save(state);
			}
			return Task::none();
		}
		Action::Identified(id, Ok(data)) => {
			let loaded = state
				.calendar
				.events
				.iter_mut()
				.chain(state.calendar.detail_event.iter_mut())
				.chain(
					state
						.inbox
						.draft
						.as_mut()
						.and_then(|draft| draft.event.as_mut()),
				)
				.filter(|event| event.id == id);
			for task in loaded {
				task.details.task_data.get_or_insert_with(|| data.clone());
			}
			// Only once the task has its ID, so a task that has gone in the
			// meantime is not sent to get one again.
			if event(state, &id).is_some_and(|task| task.details.task_data.is_some()) {
				return update(state, Action::StartWork(id));
			}
			return Task::none();
		}
		Action::Identified(_, Err(error)) => {
			log::warn!("could not give the task an ID to record work against: {error}");
			state.calendar.error = Some(error);
			return Task::none();
		}
	}
}

/// Work is recorded against a task's TimeParagon ID, which stays the same
/// when the task moves, where its Google identity does not. A Google Task
/// that has no ID yet is given one first, off the interface thread.
fn identify(task: Event) -> Task<Message> {
	let id = task.id.clone();
	crate::blocking::run(move || crate::task_db::open()?.import(&task))
		.map(move |result| Action::Identified(id.clone(), result).into())
}

fn focused_block_key(state: &State) -> Option<String> {
	let id = state.timeblocking.focus.as_deref()?;
	state.calendar.event(id).map(block_key)
}

pub(crate) fn advance_pomodoro(state: &mut State, now: chrono::DateTime<chrono::Utc>) -> bool {
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
fn resume_target(
	state: &State,
	task: &Event,
	now: chrono::DateTime<chrono::Utc>,
) -> crate::task_work::Session {
	crate::task_work::Session {
		task_key: task_key(task),
		task_title: task.title.clone(),
		block_key: block_key_of(state, task),
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

#[derive(Debug, Default, Clone)]
pub struct Workspace {
	pub work_now: Option<chrono::DateTime<chrono::Utc>>,
	/// The work session open in the editor, as an index into the work log.
	pub session_edit: Option<usize>,
	/// The day the edited session started, so its times can be parsed back.
	pub session_date: Option<NaiveDate>,
	/// The start and end being edited, as local `HH:MM`.
	pub session_start: String,
	pub session_end: String,
	pub session_error: Option<String>,
	/// The questions about a distraction are open.
	pub distraction_open: bool,
	/// What the open distraction was, once that has been answered.
	pub distraction_cause: Option<DistractionCause>,
}

/// The answer to what a distraction was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DistractionCause {
	/// One of the distraction habits, by id.
	Habit(String),
	/// Something no distraction habit covers.
	Other,
}

impl From<Action> for Message {
	fn from(action: Action) -> Self {
		Self::WorkTracking(action)
	}
}
