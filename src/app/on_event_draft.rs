//! The form for a new event.

use super::*;

pub(super) fn update(state: &mut State, message: EventDraftMessage) -> Task<Message> {
	match message {
		EventDraftMessage::EventDraftStarted { start, end } => {
			// The sidebar owns the active creation draft until Add or Cancel.
			if state.inbox.draft.is_some() {
				return Task::none();
			}
			let Some(calendar_id) = state.calendar.default_calendar_id(&state.config.calendar)
			else {
				state.calendar.error =
					Some("Connect Google Calendar in Settings to add events".to_owned());
				return Task::none();
			};

			state.calendar.error = None;
			state.calendar.begin_draft(calendar_id, start, end);
			state.close_calendar_popup();
			state.draft_kind_focus = None;
			state.calendar.day = calendar::day_at(start, state.config.calendar.day_start_hour());
			iced::widget::operation::focus("calendar-draft-title")
		}
		EventDraftMessage::EventDraftCycleKind {
			reverse,
			title_focused,
		} => {
			if state.calendar.draft.is_none() {
				return Task::none();
			}
			let kinds = [
				calendar::DraftKind::Event,
				calendar::DraftKind::TimeBlock,
				calendar::DraftKind::Task,
				calendar::DraftKind::Habit,
			];
			let current = if title_focused {
				None
			} else {
				state.draft_kind_focus
			};
			let index =
				current.and_then(|kind| kinds.iter().position(|candidate| *candidate == kind));
			let next = match index {
				Some(index) if reverse => (index + kinds.len() - 1) % kinds.len(),
				Some(index) => (index + 1) % kinds.len(),
				None if reverse => kinds.len() - 1,
				None => 0,
			};
			state.draft_kind_focus = Some(kinds[next]);
			iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())
		}
		EventDraftMessage::EventDraftTitleChanged(title) => {
			state.draft_kind_focus = None;
			if let Some(draft) = state.calendar.draft.as_mut() {
				draft.title = title;
			}
			Task::none()
		}
		EventDraftMessage::EventDraftKindSelected(calendar::DraftKind::Sleep) => {
			let Some(draft) = state.calendar.draft.take() else {
				return Task::none();
			};
			let start = draft.start;
			let end = start + chrono::Duration::hours(8);
			let event = calendar::Event {
				id: calendar::sleep::NEW_ID.into(),
				calendar_id: state
					.config
					.calendar
					.app_calendar
					.clone()
					.unwrap_or_default(),
				title: "Sleep".into(),
				start,
				end,
				all_day: false,
				color: calendar::sleep::COLOR,
				completed: false,
				details: calendar::Details {
					sleep: Some(calendar::sleep::Record::planned(end.date_naive())),
					..Default::default()
				},
			};
			state.calendar.detail_event = Some(event);
			super::update(
				state,
				Message::Event(EventMessage::EventOpened(calendar::sleep::NEW_ID.into())),
			)
		}
		EventDraftMessage::EventDraftKindSelected(calendar::DraftKind::Task) => {
			let Some(draft) = state.calendar.draft.take() else {
				return Task::none();
			};
			crate::inbox::new_something_at(state, draft.title, draft.start, draft.end)
		}
		EventDraftMessage::EventDraftKindSelected(calendar::DraftKind::Habit) => {
			// Creation and editing share the habit details pane.
			let Some(draft) = state.calendar.draft.take() else {
				return Task::none();
			};
			let rule = crate::recurrence::Rule::never(draft.start.date_naive());
			let habit = crate::habits::Draft::for_calendar_with_rule(
				draft.start,
				draft.end,
				draft.title,
				String::new(),
				None,
				rule,
			);
			crate::habits::open_new(state, habit)
		}
		EventDraftMessage::EventDraftKindSelected(calendar::DraftKind::TimeBlock) => {
			// A time block is written straight away and filled in afterwards, so
			// choosing it saves the block and then opens its details.
			let app_calendar = state.config.calendar.app_calendar.clone();
			let Some(draft) = state.calendar.draft.as_mut() else {
				return Task::none();
			};
			if draft.saving {
				return Task::none();
			}
			draft.kind = calendar::DraftKind::TimeBlock;
			if let Some(calendar_id) = app_calendar {
				draft.calendar_id = calendar_id;
			}
			update(state, EventDraftMessage::EventDraftSaveRequested)
		}
		EventDraftMessage::EventDraftKindSelected(calendar::DraftKind::Event) => {
			if state.calendar.draft.as_ref().is_some_and(|d| d.saving) {
				return Task::none();
			}
			let Some(draft) = state.calendar.draft.take() else {
				return Task::none();
			};
			open_new_event(state, draft)
		}
		EventDraftMessage::EventDraftCancelled => {
			state.draft_kind_focus = None;
			state.calendar.draft = None;
			Task::none()
		}
		// Only a time block is written straight from the chooser: it is planned
		// in its own panel afterwards.
		EventDraftMessage::EventDraftSaveRequested => {
			let Some(draft) = state.calendar.draft.as_mut().filter(|draft| !draft.saving) else {
				return Task::none();
			};
			if draft.end <= draft.start {
				state.calendar.error = Some("The end time has to be after the start".to_owned());
				return Task::none();
			}
			draft.saving = true;
			let draft = draft.clone();
			let color = state.config.calendar.time_block_color();
			let known_app = state.config.calendar.app_calendar.clone();
			state.calendar.error = None;
			blocking::run(move || {
				let (id, calendar_id) = calendar::create_time_block(
					known_app.as_deref(),
					&draft.title,
					"",
					None,
					draft.start,
					draft.end,
					&[],
				)?;
				Ok(calendar::Event {
					id,
					calendar_id,
					title: if draft.title.trim().is_empty() {
						"(No title)".to_owned()
					} else {
						draft.title.clone()
					},
					start: draft.start,
					end: draft.end,
					all_day: false,
					color,
					completed: false,
					details: calendar::Details::default(),
				})
			})
			.map(|v0| Message::EventDraft(EventDraftMessage::EventDraftSaved(v0)))
		}
		EventDraftMessage::EventDraftSaved(Ok(event)) => {
			state.calendar.draft = None;
			// A block that had to create the TimeParagon calendar makes that
			// calendar the remembered app calendar.
			let remember = state.config.calendar.app_calendar.is_none();
			if remember {
				state.config.calendar.app_calendar = Some(event.calendar_id.clone());
			}
			let block = event.id.clone();
			state.calendar.add_event(event);
			let mut tasks = vec![refresh_week_after_write(state)];
			if remember {
				tasks.push(crate::app::queue_save(state));
			}
			// The details pane edits the saved block.
			tasks.push(Task::done(Message::Event(EventMessage::EventOpened(block))));
			Task::batch(tasks)
		}
		EventDraftMessage::EventDraftSaved(Err(error)) => {
			// The popup stays open so the half-filled event is not lost.
			if let Some(draft) = state.calendar.draft.as_mut() {
				draft.saving = false;
			}
			log::warn!("could not create the time block: {error}");
			state.calendar.error = Some(error);
			Task::none()
		}
	}
}

/// Opens the range chosen on the calendar as a new event, in the form that edits
/// events.
fn open_new_event(state: &mut State, draft: calendar::Draft) -> Task<Message> {
	let event = calendar::Event {
		id: crate::inbox::NEW_EVENT_ID.into(),
		color: state.calendar.calendar_color(&draft.calendar_id),
		calendar_id: draft.calendar_id,
		title: draft.title,
		start: draft.start,
		end: draft.end,
		all_day: false,
		completed: false,
		details: calendar::Details::default(),
	};
	let mut edit = calendar::EditDraft::from_event(&event);
	edit.meeting = calendar::meeting::Form::default();
	edit.editing_title = true;
	let close_block = state
		.timeblocking
		.block
		.is_some()
		.then(|| crate::timeblocking::close(state));
	state.calendar.error = None;
	state.close_calendar_popup();
	state.calendar.detail_event = Some(event);
	state.calendar.edit_draft = Some(edit);
	state.calendar.open_event = Some(crate::inbox::NEW_EVENT_ID.into());
	Task::batch(
		close_block
			.into_iter()
			.chain([iced::widget::operation::focus(iced::widget::Id::new(
				"event-title",
			))]),
	)
}
