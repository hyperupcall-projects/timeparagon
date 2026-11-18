//! Type switching and saving for the sidebar's shared creation form. Events,
//! time blocks and habits are filled in by the form that edits them.
use super::*;
use calendar::DraftKind;
use chrono::Timelike;

fn entry_id(kind: DraftKind) -> &'static str {
	match kind {
		DraftKind::TimeBlock => ENTRY_BLOCK_ID,
		DraftKind::Habit => ENTRY_HABIT_ID,
		_ => ENTRY_EVENT_ID,
	}
}

pub(crate) fn select(state: &mut State, kind: DraftKind) -> Task<Message> {
	// Sleep is recorded from the calendar itself.
	if kind == DraftKind::Sleep {
		return Task::none();
	}
	let Some(task) = state.inbox.draft.as_ref() else {
		return Task::none();
	};
	if task.saving || task.fetching || state.inbox.voice.active() || state.inbox.image.busy {
		return Task::none();
	}
	let previous = task.kind.unwrap_or(DraftKind::Task);
	if previous == kind {
		return Task::none();
	}
	// An answer on its way to the open event's draft has to find it there.
	if previous == DraftKind::Task
		&& state
			.calendar
			.edit_draft
			.as_ref()
			.is_some_and(|draft| draft.saving || draft.loading)
	{
		return Task::none();
	}
	crate::image_input::stash(state);
	let task = state.inbox.draft.as_ref().unwrap();
	let (title, description) = if previous == DraftKind::Task {
		(
			task.title.clone(),
			task.event
				.as_ref()
				.map(|event| event.details.description.clone())
				.unwrap_or_default(),
		)
	} else {
		let entry = state.calendar.edit_draft.take();
		let common = entry
			.as_ref()
			.map(|entry| (entry.title.clone(), entry.description.clone()))
			.unwrap_or_default();
		state.inbox.entry_drafts.extend(entry);
		state.calendar.edit_draft = state.inbox.outer_edit_draft.take();
		common
	};
	state.picker = None;
	state.timeblocking.estimate_task = None;
	state.calendar.error = None;
	state.inbox.draft.as_mut().unwrap().error = None;
	if kind == DraftKind::Task {
		let task = state.inbox.draft.as_mut().unwrap();
		task.set_title(title.clone());
		task.kind = None;
		if let Some(event) = &mut task.event {
			event.title = title;
			event.details.description = description.clone();
			state.task_description = iced::widget::text_editor::Content::with_text(&description);
		}
		return operation::focus(TITLE_FIELD);
	}
	open_entry(state, kind, title, description);
	// Every type's title takes the same place, so typing carries on there.
	operation::focus(TITLE_FIELD)
}

/// Where a new item starts: when the task being written is planned, or else
/// this hour on the day in view.
fn default_start(state: &State) -> chrono::DateTime<Local> {
	state
		.inbox
		.draft
		.as_ref()
		.and_then(|task| task.event.as_ref())
		.filter(|event| !event.all_day)
		.map(|event| event.start)
		.unwrap_or_else(|| calendar::at_clock(state.calendar.day, crate::clock::now().hour(), 0))
}

/// The item being created, as the form that edits it reads it.
fn entry_event(state: &State, kind: DraftKind) -> Event {
	let start = default_start(state);
	let settings = &state.config.calendar;
	let app_calendar = settings.app_calendar.clone().unwrap_or_default();
	let (calendar_id, color) = match kind {
		DraftKind::Event => {
			let id = state
				.calendar
				.default_calendar_id(settings)
				.unwrap_or_default();
			let color = state.calendar.calendar_color(&id);
			(id, color)
		}
		DraftKind::Habit => (app_calendar, settings.habit_color()),
		_ => (app_calendar, settings.time_block_color()),
	};
	Event {
		id: entry_id(kind).into(),
		calendar_id,
		title: String::new(),
		start,
		end: start + Duration::hours(1),
		all_day: false,
		completed: false,
		color,
		details: calendar::Details {
			habit: kind == DraftKind::Habit,
			..Default::default()
		},
	}
}

/// A draft of something that does not exist yet.
fn entry_draft(event: &Event) -> calendar::EditDraft {
	let mut draft = calendar::EditDraft::from_event(event);
	draft.meeting = calendar::meeting::Form::default();
	if event.details.habit {
		draft.rule = crate::recurrence::Rule::weekly(event.start.date_naive());
	}
	draft
}

/// Shows `kind` in the form that edits it, on a draft of its own.
fn open_entry(state: &mut State, kind: DraftKind, title: String, description: String) {
	let mut event = entry_event(state, kind);
	let cached = state
		.inbox
		.entry_drafts
		.iter()
		.position(|draft| draft.id == event.id);
	let mut draft = match cached {
		Some(index) => state.inbox.entry_drafts.remove(index),
		None => entry_draft(&event),
	};
	draft.title = title;
	draft.description = description;
	state.inbox.outer_edit_draft = state.calendar.edit_draft.replace(draft);
	let form = state.inbox.draft.as_mut().unwrap();
	event.details.habit_id = form.chosen_habit.clone();
	form.entry = Some(event);
	form.kind = Some(kind);
}

/// A new event read from a picture, waiting to be reviewed in the form.
pub(crate) fn picture_draft(
	calendar_id: String,
	start: chrono::DateTime<Local>,
	end: chrono::DateTime<Local>,
	all_day: bool,
) -> calendar::EditDraft {
	entry_draft(&Event {
		id: ENTRY_EVENT_ID.into(),
		calendar_id,
		title: String::new(),
		start,
		end,
		all_day,
		completed: false,
		color: calendar::colors::DEFAULT_EVENT,
		details: Default::default(),
	})
}

/// Starts the new event over, once the last picture's proposal is removed.
pub(crate) fn reset_event_entry(state: &mut State) {
	if entry(state).is_some_and(|draft| draft.id == ENTRY_EVENT_ID) {
		let event = entry_event(state, DraftKind::Event);
		state.calendar.edit_draft = Some(entry_draft(&event));
	}
}

fn is_entry(draft: &calendar::EditDraft) -> bool {
	[ENTRY_EVENT_ID, ENTRY_BLOCK_ID, ENTRY_HABIT_ID].contains(&draft.id.as_str())
}

/// The draft of the item being created, if one is.
pub(crate) fn entry(state: &State) -> Option<&calendar::EditDraft> {
	state.calendar.edit_draft.as_ref().filter(|d| is_entry(d))
}

pub(super) fn entry_mut(state: &mut State) -> Option<&mut calendar::EditDraft> {
	state.calendar.edit_draft.as_mut().filter(|d| is_entry(d))
}

/// Gives the edit draft back to whatever was open behind the form.
pub(super) fn release_edit_draft(state: &mut State) {
	if entry(state).is_some() {
		state.calendar.edit_draft = state.inbox.outer_edit_draft.take();
	}
	state.inbox.outer_edit_draft = None;
}

/// Keeps the habit picked for a new habit event until it is added. Answers
/// whether the creation form had one to keep it for.
pub(crate) fn choose_habit(state: &mut State, habit_id: Option<String>) -> bool {
	let name = habit_id
		.as_deref()
		.and_then(|id| state.habits.library.habit(id))
		.map(|habit| habit.name.clone());
	let Some(form) = state
		.inbox
		.draft
		.as_mut()
		.filter(|form| form.kind == Some(DraftKind::Habit))
	else {
		return false;
	};
	form.chosen_habit.clone_from(&habit_id);
	if let Some(entry) = form.entry.as_mut() {
		entry.details.habit_id = habit_id;
	}
	if let (Some(name), Some(draft)) = (name, entry_mut(state)) {
		draft.title = name;
	}
	true
}

/// Whether saving this new event still leaves Zoom to be added on Google.
pub(crate) fn wants_zoom(
	settings: &crate::config::CalendarSettings,
	draft: &calendar::EditDraft,
) -> bool {
	settings.app_calendar.as_deref() != Some(draft.calendar_id.as_str())
		&& draft.meeting.zoom_wanted()
		&& !calendar::meeting::has_zoom_link(&draft.description)
}

pub(super) fn entry_wants_zoom(state: &State) -> bool {
	entry(state)
		.filter(|draft| draft.id == ENTRY_EVENT_ID)
		.is_some_and(|draft| wants_zoom(&state.config.calendar, draft))
}

/// Validate before starting any persistence or network work.
fn validate(kind: DraftKind, draft: &calendar::EditDraft) -> Result<(), String> {
	if deadline::parse_date(&draft.date_text).is_err()
		|| deadline::parse_date(&draft.end_date_text).is_err()
	{
		return Err("Enter valid dates and times, with the end after the start".into());
	}
	draft.validate()?;
	if kind == DraftKind::Event && draft.calendar_id.is_empty() {
		return Err("Connect Google Calendar in Settings to add events".into());
	}
	Ok(())
}

pub(super) fn create(state: &mut State) -> Task<Message> {
	let Some(kind) = state
		.inbox
		.draft
		.as_ref()
		.filter(|form| !form.saving)
		.and_then(|form| form.kind)
	else {
		return Task::none();
	};
	let Some(entry) = entry_mut(state) else {
		return Task::none();
	};
	entry.meeting.add_typed_address();
	let draft = entry.clone();
	if let Err(error) = validate(kind, &draft) {
		state.calendar.error = Some(error);
		return Task::none();
	}
	let (habit, persist) = if kind == DraftKind::Habit {
		let chosen = state
			.inbox
			.draft
			.as_ref()
			.and_then(|form| form.chosen_habit.clone())
			.filter(|id| state.habits.library.habit(id).is_some());
		let (id, task) = match chosen {
			Some(id) => (id, Task::none()),
			None => crate::habits::habit_for_title(state, &draft.title),
		};
		(Some(id), task)
	} else {
		(None, Task::none())
	};
	state.inbox.draft.as_mut().unwrap().saving = true;
	entry_mut(state).unwrap().saving = true;
	state.calendar.error = None;
	let settings = state.config.calendar.clone();
	let color = entry_event(state, kind).color;
	Task::batch([
		persist,
		blocking::run(move || write_entry(&settings, draft, kind, habit, color))
			.map(|result| Message::Inbox(Action::Created(result))),
	])
}

/// Writes a new event, time block or habit from the draft that filled it in, and
/// builds the local [`Event`] for it.
pub(crate) fn write_entry(
	settings: &crate::config::CalendarSettings,
	draft: calendar::EditDraft,
	kind: DraftKind,
	habit: Option<String>,
	color: iced::Color,
) -> Result<Event, String> {
	let description = if kind == DraftKind::TimeBlock {
		calendar::event_description(&draft.description)
	} else {
		draft.description.clone()
	};
	let mut event = Event {
		id: String::new(),
		calendar_id: draft.calendar_id.clone(),
		title: draft.title.trim().into(),
		start: draft.start,
		end: draft.end,
		all_day: draft.all_day,
		completed: false,
		color,
		details: calendar::Details {
			description: description.clone(),
			color_id: calendar::colors::selection(&draft.color_input)?,
			projects: draft.projects.ids.clone(),
			labels: draft.labels.clone(),
			recurrence: draft.recurrence_lines()?,
			options: draft.options.clone(),
			..Default::default()
		},
	};
	match kind {
		DraftKind::Habit => {
			let habit = habit.ok_or("Choose a habit name")?;
			let color = calendar::colors::selection(&draft.color_input)?;
			let saved = calendar::create_habit(&draft, color.as_deref(), &habit)?;
			event.id = saved.id;
			event.calendar_id = saved.calendar_id;
			event.details.habit = true;
			event.details.habit_id = Some(habit);
		}
		DraftKind::TimeBlock => {
			let (id, calendar) =
				calendar::create_time_block_from(settings.app_calendar.as_deref(), &draft)?;
			event.id = id;
			event.calendar_id = calendar;
		}
		DraftKind::Event => {
			let saved = calendar::create_event_from(
				&draft.calendar_id,
				&draft,
				&description,
				Some(&draft.meeting),
			)?;
			event.id = saved.id;
			event.details.link = saved.link;
			event.details.meeting = saved.meeting;
			event.details.attendance = calendar::meeting::Attendance {
				guests: draft.meeting.guests.clone(),
				permissions: draft.meeting.permissions,
				organizer: true,
				own_email: None,
			};
		}
		DraftKind::Task | DraftKind::Sleep => {
			return Err("This type is not created from this form".into());
		}
	}
	Ok(event)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn state() -> State {
		let mut state = State::blank();
		let _ = super::super::update(&mut state, Action::NewTask);
		let _ = super::super::update(&mut state, Action::Title("Study".into()));
		state
	}

	fn open(state: &State) -> &calendar::EditDraft {
		state.calendar.edit_draft.as_ref().unwrap()
	}

	#[test]
	fn switching_types_keeps_common_text_and_each_types_fields() {
		let mut state = state();
		let _ = select(&mut state, DraftKind::Event);
		let event = state.calendar.edit_draft.as_mut().unwrap();
		event.title = "Read a chapter".into();
		event.description = "Take notes".into();
		event.calendar_id = "chosen-calendar".into();
		event.set_start_text("11:15".into());
		let start = event.start;

		let _ = select(&mut state, DraftKind::Habit);
		assert_eq!(open(&state).id, ENTRY_HABIT_ID);
		assert_eq!(open(&state).title, "Read a chapter");
		assert_eq!(open(&state).description, "Take notes");
		assert_eq!(
			open(&state).rule.repeat,
			crate::recurrence::RepeatMode::Weekly
		);
		let _ = select(&mut state, DraftKind::Event);
		assert_eq!(open(&state).start, start);
		assert_eq!(open(&state).calendar_id, "chosen-calendar");
		assert_eq!(
			open(&state).rule.repeat,
			crate::recurrence::RepeatMode::Never
		);

		let _ = select(&mut state, DraftKind::Task);
		assert!(state.calendar.edit_draft.is_none());
		let task = state.inbox.draft.as_ref().unwrap();
		assert_eq!(task.title, "Read a chapter");
		assert_eq!(
			task.event.as_ref().unwrap().details.description,
			"Take notes"
		);
		let _ = select(&mut state, DraftKind::Event);
		assert_eq!(open(&state).start, start);
	}

	#[test]
	fn sidebar_creation_preserves_an_existing_calendar_draft() {
		let mut state = State::blank();
		let start = crate::clock::now();
		let mut old = calendar::Draft::new("original".into(), start, start + Duration::hours(1));
		old.title = "Unfinished event".into();
		state.calendar.draft = Some(old.clone());
		let _ = super::super::update(&mut state, Action::NewTask);
		let _ = select(&mut state, DraftKind::Event);
		let _ = crate::app::update(
			&mut state,
			Message::EventDraft(crate::app::EventDraftMessage::EventDraftStarted {
				start: start + Duration::days(1),
				end: start + Duration::days(1) + Duration::hours(1),
			}),
		);
		assert!(state.calendar.draft.is_none(), "the form owns the moment");
		let _ = super::super::update(&mut state, Action::Cancel);
		assert_eq!(state.calendar.draft, Some(old));
	}

	#[test]
	fn invalid_fields_keep_the_form_open_without_saving() {
		for kind in [DraftKind::Event, DraftKind::TimeBlock, DraftKind::Habit] {
			let mut state = state();
			let _ = select(&mut state, kind);
			state.calendar.edit_draft.as_mut().unwrap().date_text = "invalid".into();
			let _ = create(&mut state);
			assert!(
				state
					.calendar
					.error
					.as_deref()
					.unwrap()
					.contains("valid dates")
			);
			assert!(!state.inbox.draft.as_ref().unwrap().saving);
			assert!(!open(&state).saving);
		}
	}

	#[test]
	fn a_new_event_needs_a_calendar_to_go_on() {
		let mut state = state();
		let _ = select(&mut state, DraftKind::Event);
		state
			.calendar
			.edit_draft
			.as_mut()
			.unwrap()
			.calendar_id
			.clear();
		let _ = create(&mut state);
		assert!(state.calendar.error.as_deref().unwrap().contains("Connect"));
		assert!(!state.inbox.draft.as_ref().unwrap().saving);
	}

	#[test]
	fn saving_locks_type_switching_and_cancellation_and_errors_allow_retry() {
		let mut state = state();
		let _ = select(&mut state, DraftKind::TimeBlock);
		state.inbox.draft.as_mut().unwrap().saving = true;
		state.calendar.edit_draft.as_mut().unwrap().saving = true;
		let _ = select(&mut state, DraftKind::Task);
		let _ = super::super::update(&mut state, Action::Cancel);
		assert_eq!(
			state.inbox.draft.as_ref().unwrap().kind,
			Some(DraftKind::TimeBlock)
		);
		let _ = super::super::update(&mut state, Action::Created(Err("Could not save".into())));
		assert!(!open(&state).saving);
		assert!(!state.inbox.draft.as_ref().unwrap().saving);
		assert_eq!(
			state.inbox.draft.as_ref().unwrap().error.as_deref(),
			Some("Could not save")
		);
		let _ = super::super::update(&mut state, Action::Cancel);
		assert!(state.inbox.draft.is_none());
		assert!(state.calendar.draft.is_none());
		assert!(state.calendar.edit_draft.is_none());
		assert!(state.inbox.entry_drafts.is_empty());
	}

	#[test]
	fn every_type_is_filled_in_on_the_draft_that_edits_it() {
		for (kind, id) in [
			(DraftKind::Event, ENTRY_EVENT_ID),
			(DraftKind::TimeBlock, ENTRY_BLOCK_ID),
			(DraftKind::Habit, ENTRY_HABIT_ID),
		] {
			let mut state = state();
			let _ = select(&mut state, kind);
			assert!(state.calendar.draft.is_none());
			assert_eq!(open(&state).id, id);
			assert_eq!(open(&state).title, "Study");
			assert!(open(&state).meeting.can_manage, "{kind:?}");
			state
				.calendar
				.edit_draft
				.as_mut()
				.unwrap()
				.labels
				.push("deep".into());
			let start = open(&state).start;

			// Other types carry the title along, and this one's own fields wait.
			let _ = crate::app::update(
				&mut state,
				Message::EventEdit(crate::app::EventEditMessage::EventEditTitleChanged(
					"Revise".into(),
				)),
			);
			let _ = select(&mut state, DraftKind::Task);
			assert_eq!(state.inbox.draft.as_ref().unwrap().title, "Revise");
			let _ = select(&mut state, kind);
			assert_eq!((open(&state).start, open(&state).labels.len()), (start, 1));
			assert_eq!(open(&state).title, "Revise");
		}
	}

	#[test]
	fn a_habit_picked_for_a_new_habit_event_is_kept_across_types() {
		let mut state = state();
		let id = crate::habits::habit_for_title(&mut state, "Stretch").0;
		let _ = select(&mut state, DraftKind::Habit);
		assert!(choose_habit(&mut state, Some(id.clone())));
		assert_eq!(open(&state).title, "Stretch");
		let _ = select(&mut state, DraftKind::Event);
		assert!(!choose_habit(&mut state, None), "only a habit has one");
		let _ = select(&mut state, DraftKind::Habit);
		let form = state.inbox.draft.as_ref().unwrap();
		let entry = form.entry.as_ref().unwrap();
		assert_eq!(entry.details.habit_id.as_deref(), Some(id.as_str()));
	}

	#[test]
	fn a_new_item_gives_the_open_events_draft_back() {
		let mut state = state();
		let event = calendar::sample_snapshot().events.remove(0);
		let opened = calendar::EditDraft::from_event(&event);
		state.calendar.edit_draft = Some(opened.clone());
		let _ = select(&mut state, DraftKind::TimeBlock);
		assert_eq!(open(&state).id, ENTRY_BLOCK_ID);
		let _ = select(&mut state, DraftKind::Event);
		assert_eq!(open(&state).id, ENTRY_EVENT_ID);
		let _ = select(&mut state, DraftKind::Task);
		assert_eq!(state.calendar.edit_draft.as_ref(), Some(&opened));
		let _ = select(&mut state, DraftKind::Habit);
		let _ = super::super::update(&mut state, Action::Cancel);
		assert_eq!(state.calendar.edit_draft, Some(opened));
	}

	#[test]
	fn the_open_events_draft_is_not_taken_while_it_is_being_saved() {
		let mut state = state();
		let event = calendar::sample_snapshot().events.remove(0);
		let mut opened = calendar::EditDraft::from_event(&event);
		opened.saving = true;
		state.calendar.edit_draft = Some(opened);
		let _ = select(&mut state, DraftKind::TimeBlock);
		assert_eq!(state.inbox.draft.as_ref().unwrap().kind, None);
		assert_eq!(open(&state).id, event.id);
	}

	#[test]
	fn a_new_item_is_checked_like_an_edited_one() {
		let mut state = state();
		let _ = select(&mut state, DraftKind::TimeBlock);
		let block = state.calendar.edit_draft.as_mut().unwrap();
		block.end = block.start;
		let _ = create(&mut state);
		assert_eq!(
			state.calendar.error.as_deref(),
			Some("The end time has to be after the start")
		);
		assert!(!state.inbox.draft.as_ref().unwrap().saving);

		let block = state.calendar.edit_draft.as_mut().unwrap();
		block.end = block.start + Duration::hours(1);
		let _ = create(&mut state);
		assert!(state.inbox.draft.as_ref().unwrap().saving);
		assert!(open(&state).saving);
	}
}
