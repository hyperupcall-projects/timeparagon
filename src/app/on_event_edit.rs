//! The form for editing an event.

use super::*;

pub(super) fn update(state: &mut State, message: EventEditMessage) -> Task<Message> {
	match message {
		EventEditMessage::EventEditDeadlineChanged(value) => {
			if let Some(draft) = &mut state.calendar.edit_draft {
				draft.deadline_text = value;
			}
			Task::none()
		}
		EventEditMessage::EventEditMeeting(action) => {
			let Some(draft) = state
				.calendar
				.edit_draft
				.as_mut()
				.filter(|draft| !draft.loading && !draft.saving)
			else {
				return Task::none();
			};
			let search = match &action {
				calendar::meeting::Action::Query(query) => Some(query.clone()),
				_ => None,
			};
			draft.meeting.update(action);
			search.map_or_else(Task::none, |query| {
				crate::people::search_guests(state, &query)
			})
		}
		EventEditMessage::EventEditOptions(action) => {
			if let Some(draft) = &mut state.calendar.edit_draft
				&& !draft.loading
				&& !draft.saving
			{
				draft.options.update(action);
			}
			Task::none()
		}
		EventEditMessage::EventEditRequested(id) => {
			state.calendar.begin_edit(&id);
			Task::none()
		}
		EventEditMessage::EventEditProjects(action) => {
			if let calendar::projects::Action::Add(id) = &action
				&& state.config.projects.get(*id).is_none()
			{
				return Task::none();
			}
			if let Some(draft) = &mut state.calendar.edit_draft {
				draft.projects.update(action);
			}
			Task::none()
		}
		EventEditMessage::EventEditTitleChanged(title) => {
			if let Some(draft) = state.calendar.edit_draft.as_mut() {
				draft.title = title;
			}
			Task::none()
		}
		EventEditMessage::EventEditDescriptionChanged(description) => {
			if let Some(draft) = state.calendar.edit_draft.as_mut() {
				draft.description = description;
			}
			Task::none()
		}
		EventEditMessage::EventEditDateChanged(text) => {
			let habit = state
				.calendar
				.open_event
				.as_deref()
				.and_then(|id| state.calendar.event(id))
				.is_some_and(|event| event.details.habit);
			if let Some(draft) = state.calendar.edit_draft.as_mut() {
				let duration = draft.end - draft.start;
				draft.set_date_text(text);
				if habit {
					draft.end = draft.start + duration;
					draft.end_date_text = draft.end.date_naive().to_string();
				}
			}
			Task::none()
		}
		EventEditMessage::EventEditRecurrence(action) => {
			if let Some(draft) = state.calendar.edit_draft.as_mut() {
				draft.recurrence_changed = true;
				draft.rule.apply_action(action, draft.start.date_naive());
			}
			Task::none()
		}
		EventEditMessage::EventEditStartChanged(text) => {
			let habit = state
				.calendar
				.open_event
				.as_deref()
				.and_then(|id| state.calendar.event(id))
				.is_some_and(|event| event.details.habit);
			if let Some(draft) = state.calendar.edit_draft.as_mut() {
				let duration = draft.end - draft.start;
				draft.set_start_text(text);
				draft.all_day = false;
				if habit {
					draft.end = draft.start + duration;
					draft.end_text = calendar::clock_label(draft.end);
					draft.end_date_text = draft.end.date_naive().to_string();
				}
			}
			Task::none()
		}
		EventEditMessage::EventEditAllDayToggled(all_day) => {
			if let Some(draft) = state
				.calendar
				.edit_draft
				.as_mut()
				.filter(|draft| !draft.saving && !draft.loading)
			{
				draft.set_all_day(all_day);
			}
			Task::none()
		}
		EventEditMessage::EventEditEndChanged(text) => {
			if let Some(draft) = state.calendar.edit_draft.as_mut() {
				draft.set_end_text(text);
				draft.all_day = false;
			}
			Task::none()
		}
		EventEditMessage::EventEditCalendarSelected(id) => {
			let allowed = state
				.calendar
				.edit_draft
				.as_ref()
				.filter(|draft| !draft.saving && !draft.loading)
				.and_then(|draft| state.calendar.event(&draft.id))
				.is_some_and(|event| {
					state
						.calendar
						.move_targets(event, &state.config.calendar)
						.iter()
						.any(|calendar| calendar.id == id)
				});
			if let Some(draft) = state.calendar.edit_draft.as_mut() {
				draft.calendar_dropdown_open = false;
				if allowed {
					draft.calendar_id = id;
				}
			}
			Task::none()
		}
		EventEditMessage::EventEditCalendarDropdownToggled => {
			if let Some(draft) = state.calendar.edit_draft.as_mut() {
				draft.calendar_dropdown_open = !draft.calendar_dropdown_open;
			}
			Task::none()
		}
		EventEditMessage::EventEditCancelled => {
			state.calendar.edit_draft = None;
			Task::none()
		}
		EventEditMessage::EventEditDeleteMenuToggled => {
			if let Some(draft) = &mut state.calendar.edit_draft {
				draft.delete_menu_open = !draft.delete_menu_open;
				draft.save_menu_open = false;
			}
			Task::none()
		}
		EventEditMessage::EventEditSaveMenuToggled => {
			if let Some(draft) = &mut state.calendar.edit_draft {
				draft.save_menu_open = !draft.save_menu_open;
				draft.delete_menu_open = false;
			}
			Task::none()
		}

		request @ (EventEditMessage::EventEditSaveRequested
		| EventEditMessage::EventEditSaveScoped(_)) => {
			let scope = match request {
				EventEditMessage::EventEditSaveScoped(scope) => scope,
				_ => calendar::SaveScope::Current,
			};
			if let Some(draft) = &mut state.calendar.edit_draft {
				draft.save_menu_open = false;
				draft.delete_menu_open = false;
				draft.meeting.add_typed_address();
			}
			let Some(draft) = state.calendar.edit_draft.clone() else {
				return Task::none();
			};

			if draft.loading || draft.saving {
				return Task::none();
			}
			if draft.title.trim().is_empty() {
				state.calendar.error = Some("Enter a title".into());
				return Task::none();
			}
			if draft.end <= draft.start {
				state.calendar.error = Some("The end time has to be after the start".to_owned());
				return Task::none();
			}

			if draft.id == crate::habits::NEW_HABIT_ID {
				if state.habits.busy {
					return Task::none();
				}
				if let Err(error) = draft.recurrence_lines() {
					state.calendar.error = Some(error);
					return Task::none();
				}
				let color_id = state
					.calendar
					.event(&draft.id)
					.and_then(|event| event.details.color_id.clone());
				let (habit_id, created) = crate::habits::habit_for_new_event(state, &draft.title);
				state.habits.busy = true;
				state.habits.writing.insert(habit_id.clone());
				state.calendar.error = None;
				let write = blocking::run(move || {
					calendar::create_habit(&draft, color_id.as_deref(), &habit_id)
				})
				.map(|result| Message::Habits(crate::habits::Action::Saved(result)));
				return Task::batch([created, write]);
			}

			let is_block = state
				.calendar
				.event(&draft.id)
				.is_some_and(|event| crate::timeblocking::is_block(event, &state.config.calendar));
			let description = if is_block {
				calendar::event_description(&draft.description)
			} else {
				draft.description.clone()
			};
			let color_id = None;
			let Some(original) = state.calendar.event(&draft.id).cloned() else {
				return Task::none();
			};
			if let Err(error) = draft.recurrence_lines() {
				state.calendar.error = Some(error);
				return Task::none();
			}
			state.calendar.error = None;
			if let Some(draft) = &mut state.calendar.edit_draft {
				draft.saving = true;
			}
			blocking::run(move || {
				calendar::save_event_scope(&original, &draft, scope, &description, color_id)
			})
			.map(|v0| Message::EventEdit(EventEditMessage::EventEditSaved(v0)))
		}
		EventEditMessage::EventEditSaved(Ok(saved)) => {
			// Only the draft still marked as saving is the one Google just
			// wrote; another event may have been opened in the meantime.
			let mut moved = None;
			if let Some(draft) = &mut state.calendar.edit_draft {
				if draft.saving {
					moved = Some((draft.id.clone(), draft.calendar_id.clone()));
					draft.meeting.mark_saved();
				}
				draft.editing_title = false;
				draft.saving = false;
			}
			if let Some((id, calendar_id)) = moved {
				state.calendar.event_moved(&id, &calendar_id);
				if saved.zoom_needed {
					open_zoom_setup(state, &saved.link);
				}
			}
			// Keep the inline draft as the user left it until the calendar
			// sync brings the saved event back, so fields do not flicker.
			if state.screen == Screen::Habits {
				Task::batch([
					Task::done(Message::Habits(crate::habits::Action::Refresh)),
					refresh_week_after_write(state),
				])
			} else {
				refresh_week_after_write(state)
			}
		}
		EventEditMessage::EventEditSaved(Err(error)) => {
			if let Some(draft) = &mut state.calendar.edit_draft {
				draft.saving = false;
			}
			// The popup stays open so the half-filled edit is not lost.
			log::warn!("could not update the event: {error}");
			state.calendar.error = Some(error);
			Task::none()
		}
	}
}

/// Zoom for Google Workspace only runs inside Google Calendar, so the user is
/// taken to the event there, signed in as the account that holds it.
pub(super) fn open_zoom_setup(state: &mut State, link: &str) {
	let account = state
		.calendar
		.calendars
		.iter()
		.find(|calendar| calendar.primary && !calendar.is_task_list())
		.map(|calendar| calendar.id.as_str());
	let opened = calendar::meeting::zoom_setup_url(link, account)
		.ok_or_else(|| "Could not find this event on Google Calendar to add Zoom".to_owned())
		.and_then(|url| calendar::oauth::open_in_browser(&url));
	if let Err(error) = opened {
		state.calendar.error = Some(error);
	}
}
