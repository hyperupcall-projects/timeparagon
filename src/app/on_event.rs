//! An event that already exists: opening, moving and deleting it.

use super::*;

pub(super) fn update(state: &mut State, message: EventMessage) -> Task<Message> {
	match message {
		EventMessage::EventReply(id, response) => {
			// An invitation in the inbox needn't be in the range on show.
			let Some(event) = state
				.calendar
				.event(&id)
				.or_else(|| state.inbox.invitations.iter().find(|e| e.id == id))
			else {
				return Task::none();
			};
			let attendance = &event.details.attendance;
			let Some(email) = attendance
				.can_reply()
				.then(|| attendance.own_email.clone())
				.flatten()
			else {
				return Task::none();
			};
			if attendance.own_response() == Some(response) {
				return Task::none();
			}
			let series = state
				.calendar
				.edit_draft
				.as_ref()
				.filter(|draft| draft.id == id && draft.meeting.reply_to_series)
				.and(event.details.recurring_event_id.clone());
			let target = series.unwrap_or_else(|| id.clone());
			let calendar_id = event.calendar_id.clone();
			let previous = attendance.own_response();
			set_own_response(state, &id, response);
			// Answered, so it's no longer waiting. A failure brings it back.
			state
				.inbox
				.invitations
				.retain(|invitation| invitation.id != id && invitation.id != target);
			state.calendar.error = None;
			blocking::run(move || calendar::reply_to_event(&calendar_id, &target, &email, response))
				.map(move |result| {
					Message::Event(EventMessage::EventReplied(id.clone(), previous, result))
				})
		}
		EventMessage::EventReplied(_, _, Ok(())) => refresh_week_after_write(state),
		EventMessage::EventReplied(id, previous, Err(error)) => {
			if let Some(previous) = previous {
				set_own_response(state, &id, previous);
			}
			log::warn!("could not answer the invitation: {error}");
			state.calendar.error = Some(error);
			crate::inbox::refresh_invitations(state)
		}
		EventMessage::DescriptionOpen(id) => {
			let value =
				if let Some(draft) = state.calendar.edit_draft.as_ref().filter(|d| d.id == id) {
					draft.description.clone()
				} else if let Some(event) = state.calendar.event(&id) {
					event.details.description.clone()
				} else {
					return Task::none();
				};
			state.description_editor = text_editor::Content::with_text(&value);
			state.description_popup = Some(id);
			Task::none()
		}
		EventMessage::DescriptionEdited(action) => {
			state.description_editor.perform(action);
			Task::none()
		}
		EventMessage::DescriptionClose => {
			state.description_popup = None;
			Task::none()
		}
		EventMessage::DescriptionSave => {
			let Some(id) = state.description_popup.take() else {
				return Task::none();
			};
			let value = state.description_editor.text();
			if let Some(draft) = state.calendar.edit_draft.as_mut().filter(|d| d.id == id) {
				draft.description = value;
				// Keep this with the other staged fields until the editor's
				// Save button is used. Saving a new habit here would close
				// its pane as soon as the creation request finishes.
				Task::none()
			} else {
				crate::timeblocking::update(
					state,
					crate::timeblocking::Action::SaveDescription(id, value),
				)
			}
		}
		EventMessage::HabitDetailsLoaded(id, result) => {
			if state.calendar.open_event.as_deref() == Some(id.as_str()) {
				match result {
					Ok((event, draft)) => {
						state.calendar.open_event = Some(event.id.clone());
						if state.calendar.detail_event.is_some() || event.id != id {
							state.calendar.detail_event = Some(event);
						}
						state.calendar.edit_draft = Some(draft);
						state.calendar.error = None;
					}
					Err(error) => {
						state.calendar.error = Some(error);
					}
				}
			}
			Task::none()
		}
		EventMessage::SeriesRuleLoaded(id, result) => {
			// Only the repeat rule is taken: the rest of the draft may have
			// been edited while the master loaded. A failed load leaves the
			// event editable, just without its schedule shown.
			if state.calendar.open_event.as_deref() == Some(id.as_str())
				&& let Ok((_, loaded)) = result
				&& let Some(draft) = &mut state.calendar.edit_draft
				&& draft.id == id
				&& !draft.recurrence_changed
			{
				draft.rule = loaded.rule;
				draft.series_id = loaded.series_id;
			}
			Task::none()
		}
		EventMessage::EventTitleEdit => {
			if let Some(draft) = &mut state.calendar.edit_draft {
				draft.editing_title = true;
			}
			iced::widget::operation::focus(iced::widget::Id::new("event-title"))
		}
		EventMessage::EventEndDateChanged(value) => {
			if let Some(draft) = &mut state.calendar.edit_draft {
				draft.set_end_date_text(value);
			}
			Task::none()
		}
		EventMessage::EventLabelDraft(value) => {
			if let Some(draft) = &mut state.calendar.edit_draft {
				draft.label_draft = value;
			}
			Task::none()
		}
		EventMessage::EventLabelAdd => {
			if let Some(draft) = &mut state.calendar.edit_draft {
				let label = draft.label_draft.trim().trim_start_matches('#').to_owned();
				if !label.is_empty() && !draft.labels.contains(&label) {
					draft.labels.push(label);
				}
				draft.label_draft.clear();
			}
			Task::none()
		}
		EventMessage::EventLabelRemove(label) => {
			if let Some(draft) = &mut state.calendar.edit_draft {
				draft.labels.retain(|l| l != &label);
			}
			Task::none()
		}
		EventMessage::FormEdited(field, action) => {
			let editor = state.form_editors.get_mut(field);
			editor.perform(action);
			let value = editor.text();
			match field {
				FormField::EventDraft => {
					if let Some(d) = &mut state.calendar.draft {
						d.description = value;
					}
				}
				FormField::EventEdit => {
					if let Some(d) = &mut state.calendar.edit_draft {
						d.description = value;
					}
				}
				FormField::HabitNotes => {
					if let Some(form) = &mut state.habits.form {
						form.notes = value;
						form.error = None;
					}
				}
			}
			Task::none()
		}
		EventMessage::EventOpened(id) => {
			let id = calendar::deadline::source_id(&id).to_owned();
			if let Some(task) = state.calendar.event(&id)
				&& task.task_ref().is_some()
				&& let Some(parent) = crate::timeblocking::block_of(
					task,
					&state.calendar.events,
					crate::timeblocking::day_start_hour(state),
				) && state.timeblocking.block.as_deref() != Some(parent.id.as_str())
				&& state.timeblocking.focus.as_deref() != Some(parent.id.as_str())
			{
				return crate::timeblocking::update(
					state,
					crate::timeblocking::Action::Plan(parent.id.clone()),
				);
			}
			// The task screens list tasks in full whatever the calendar hides,
			// so this only stops a click on the calendar itself.
			let on_calendar = state.range_view_active() || state.screen == Screen::Calendar;
			if on_calendar
				&& state
					.calendar
					.event(&id)
					.is_some_and(|event| state.calendar.search.hides_details(event))
			{
				return Task::none();
			}
			state.timeblocking.backlog_picker = None;
			if state
				.calendar
				.detail_event
				.as_ref()
				.is_some_and(|event| event.id != id)
			{
				state.calendar.detail_event = None;
			}
			state.calendar.error = None;
			state.calendar.menu_open = false;
			state.calendar.search.open = false;
			state.calendar.actions_open = false;
			let mut close_block = None;
			if let Some(event) = state.calendar.event(&id).cloned() {
				let is_task = event.task_ref().is_some();
				let mut draft = calendar::EditDraft::from_event(&event);
				draft.loading = (event.details.habit || event.is_calendar_task())
					&& event.details.recurring_event_id.is_some();
				// A block's panel and the event pane are two different
				// sidebars; opening one closes the other.
				if state.timeblocking.block.is_some() && !is_task {
					close_block = Some(crate::timeblocking::close(state));
				}
				if !is_task {
					// Events are edited in place in the details pane, so the
					// draft opens with the card rather than behind an Edit
					// button.
					state.calendar.edit_draft = Some(draft);
				}
				if is_task {
					state.timeblocking.editing_title = false;
					state.timeblocking.editing_due_time = false;
					state.timeblocking.editing_deadline_time = false;
					state.timeblocking.title = event.title.clone();
					state.timeblocking.title_error = None;
					state.timeblocking.date =
						if event.start.date_naive() == crate::timeblocking::UNDATED {
							String::new()
						} else {
							event.start.date_naive().to_string()
						};
					state.timeblocking.deadline = calendar::deadline::input_text(&event);
					state.timeblocking.deadline_time = state
						.timeblocking
						.deadline
						.get(11..)
						.unwrap_or("09:00")
						.to_owned();
					state.timeblocking.start_time = if event.all_day {
						"09:00".into()
					} else {
						calendar::clock_label(event.start)
					};
					state.timeblocking.end_time = if event.all_day {
						"09:30".into()
					} else {
						calendar::clock_label(event.end)
					};
					state.timeblocking.project_query.clear();
					state.timeblocking.project_picker = None;
					state.timeblocking.label_draft.clear();
					state.timeblocking.label_error = None;
				}
			}
			let habit_load = state
				.calendar
				.event(&id)
				.filter(|event| {
					(event.details.habit || event.is_calendar_task())
						&& event.details.recurring_event_id.is_some()
				})
				.cloned()
				.map(|event| {
					let settings = state.config.calendar.clone();
					let id = id.clone();
					blocking::run(move || {
						calendar::habit_edit_draft(
							&calendar::GoogleClient::from_settings(&settings),
							&event,
						)
					})
					.map(move |result| {
						Message::Event(EventMessage::HabitDetailsLoaded(id.clone(), result))
					})
				});
			// Ordinary recurring events: occurrences carry no recurrence of
			// their own, so fetch the series' rule without blocking the editor.
			let series_load = state
				.calendar
				.event(&id)
				.filter(|event| {
					!event.details.habit
						&& !event.is_calendar_task()
						&& event.task_ref().is_none()
						&& event.details.recurring_event_id.is_some()
				})
				.cloned()
				.map(|event| {
					let settings = state.config.calendar.clone();
					let id = id.clone();
					blocking::run(move || {
						calendar::habit_edit_draft(
							&calendar::GoogleClient::from_settings(&settings),
							&event,
						)
					})
					.map(move |result| {
						Message::Event(EventMessage::SeriesRuleLoaded(id.clone(), result))
					})
				});
			state.calendar.open_event = Some(id);
			// Recorded during this click, before the message is handled.
			state.calendar.details_below = ui::calendar::click_was_in_the_top_half();
			state.inbox.picker = None;
			Task::batch(close_block.into_iter().chain(habit_load).chain(series_load))
		}
		EventMessage::EventClosed => {
			state.calendar.open_event = None;
			if state.timeblocking.block.is_none() && state.timeblocking.focus.is_none() {
				state.calendar.edit_draft = None;
			}
			state.calendar.detail_event = None;
			state.inbox.picker = None;
			Task::none()
		}
		EventMessage::EventTimeChanged { id, start, end } => {
			if state.calendar.pending_move.is_some() {
				return Task::none();
			}
			let Some(event) = state.calendar.event(&id).cloned() else {
				return Task::none();
			};
			if end <= start || (event.start == start && event.end == end) {
				return Task::none();
			}
			if event.task_ref().is_some() {
				return crate::timeblocking::update(
					state,
					crate::timeblocking::Action::Schedule(id, start, end),
				);
			}
			let ask = event.details.attendance.can_manage()
				&& !event.details.attendance.guests.is_empty();
			if !state.calendar.begin_time_change(&id, start, end) {
				return Task::none();
			}
			let recurring = event.is_recurring() || event.details.sleep.is_some();
			if ask || recurring {
				state.calendar.pending_move = Some(calendar::PendingMove {
					original: event,
					scope: (!recurring).then_some(calendar::SaveScope::Current),
					id,
					start,
					end,
				});
				return Task::none();
			}
			write_event_time(
				state,
				event,
				start,
				end,
				calendar::SaveScope::Current,
				false,
			)
		}
		EventMessage::EventMoveScoped(scope) => {
			let Some(pending) = state.calendar.pending_move.as_mut() else {
				return Task::none();
			};
			if pending.original.details.sleep.is_some() && scope == calendar::SaveScope::All {
				return Task::none();
			}
			pending.scope = Some(scope);
			let attendance = &pending.original.details.attendance;
			if attendance.can_manage() && !attendance.guests.is_empty() {
				return Task::none();
			}
			update(state, EventMessage::EventMoveConfirmed(false))
		}
		EventMessage::EventMoveConfirmed(notify) => {
			if state
				.calendar
				.pending_move
				.as_ref()
				.is_none_or(|p| p.scope.is_none())
			{
				return Task::none();
			}
			let pending = state.calendar.pending_move.take().unwrap();
			write_event_time(
				state,
				pending.original,
				pending.start,
				pending.end,
				pending.scope.unwrap(),
				notify,
			)
		}
		EventMessage::EventMoveCancelled => {
			if let Some(pending) = state.calendar.pending_move.take() {
				state.calendar.fail_time_change(&pending.id);
			}
			Task::none()
		}
		EventMessage::SleepTimeSaved(id, scope, result) => {
			let event = match result {
				Ok(event) => event,
				Err(error) => {
					state.calendar.fail_time_change(&id);
					state.calendar.error = Some(error);
					return Task::none();
				}
			};
			state.calendar.confirm_time_change(&id);
			let mut tasks = Vec::new();
			if scope == calendar::SaveScope::CurrentAndFuture {
				let record = event.details.sleep.as_ref().unwrap();
				let from = calendar::sleep::change_from(
					record.wake_date,
					crate::clock::now().date_naive(),
				);
				calendar::sleep::set_weekday_times(
					&mut state.config.calendar,
					from,
					calendar::sleep::Times {
						bedtime: event.start.time(),
						wakeup: event.end.time(),
					},
				);
				state.calendar.invalidate_other_days();
				tasks.push(queue_save(state));
			}
			state.calendar.forget_event_everywhere(&id);
			if state
				.calendar
				.edit_draft
				.as_ref()
				.is_some_and(|draft| draft.id == id)
			{
				state.calendar.edit_draft = Some(calendar::EditDraft::from_event(&event));
			}
			state.calendar.add_event(event);
			tasks.push(refresh_week_after_write(state));
			Task::batch(tasks)
		}
		EventMessage::EventTimeSaved(id, Ok(())) => {
			state.calendar.confirm_time_change(&id);
			let refresh =
				crate::timeblocking::update(state, crate::timeblocking::Action::RefreshBlock(id));
			Task::batch([refresh, refresh_week_after_write(state)])
		}
		EventMessage::EventTimeSaved(id, Err(error)) => {
			state.calendar.fail_time_change(&id);
			log::warn!("could not move the event: {error}");
			state.calendar.error = Some(error);
			Task::none()
		}

		request @ (EventMessage::EventDeleteRequested(_)
		| EventMessage::EventDeleteScoped(_, _)
		| EventMessage::EventDeleteWithTasks(_, _)) => {
			let (id, scope, with_tasks) = match request {
				EventMessage::EventDeleteScoped(id, scope) => (id, scope, false),
				EventMessage::EventDeleteWithTasks(id, scope) => (id, scope, true),
				EventMessage::EventDeleteRequested(id) => (id, calendar::SaveScope::Current, false),
				_ => unreachable!(),
			};
			// Only calendar events can be deleted this way; Google Tasks live
			// under the Tasks API and are ticked off instead.
			let is_task = state
				.calendar
				.event(&id)
				.map(|event| event.task_ref().is_some())
				.unwrap_or(false);
			if is_task {
				return Task::none();
			}
			if let Some(draft) = &mut state.calendar.edit_draft {
				draft.delete_menu_open = false;
				draft.save_menu_open = false;
			}
			state.calendar.pending_delete_scope = scope;
			state.calendar.pending_delete_notify = true;
			state.calendar.pending_delete_with_tasks = with_tasks;
			state.calendar.pending_delete = Some(id);
			Task::none()
		}
		EventMessage::EventDeleteNotify(notify) => {
			state.calendar.pending_delete_notify = notify;
			Task::none()
		}
		EventMessage::EventDeleteConfirmed(id) => {
			if state.calendar.pending_delete.as_deref() != Some(id.as_str()) {
				return Task::none();
			}
			let scope = state.calendar.pending_delete_scope;
			let Some(event) = state
				.calendar
				.event(&id)
				.filter(|event| event.task_ref().is_none())
			else {
				state.calendar.pending_delete = None;
				return Task::none();
			};
			let calendar_id = event.calendar_id.clone();
			let attendance = &event.details.attendance;
			let notify = state.calendar.pending_delete_notify
				&& attendance.can_manage()
				&& !attendance.guests.is_empty();

			let request_id = id.clone();
			let block = state
				.calendar
				.pending_delete_with_tasks
				.then(|| event.clone());
			state.calendar.error = None;

			blocking::run(move || match block {
				Some(block) => calendar::delete_block_with_tasks(&block, scope, notify),
				None => calendar::delete_event_scope(&calendar_id, &request_id, scope, notify),
			})
			.map(move |result| Message::Event(EventMessage::EventDeleted(id.clone(), result)))
		}
		EventMessage::EventDeleteCancelled => {
			state.calendar.pending_delete = None;
			Task::none()
		}
		EventMessage::EventDeleted(id, Ok(())) => {
			state.calendar.pending_delete = None;
			// The delete is on its way to Google. The event, the rest of its
			// series it takes and any planned tasks going with it stay in
			// view, locked like an edit on its way, until Google confirms;
			// the local edits read here say so (see `Overlay::apply`).
			let closing_details = state.calendar.open_event.as_deref() == Some(id.as_str());
			if closing_details {
				state.calendar.open_event = None;
				state.calendar.edit_draft = None;
				state.calendar.detail_event = None;
			}
			let close = if state.timeblocking.block.as_deref() == Some(id.as_str()) {
				crate::timeblocking::close(state)
			} else {
				Task::none()
			};
			if state.screen == Screen::Habits {
				state.habits.series.retain(|series| series.id != id);
			}
			// Through the write refresh, so a sync already under way, which
			// may predate the delete, is fetched again rather than applied.
			Task::batch([close, refresh_week_after_write(state)])
		}
		EventMessage::EventDeleted(_id, Err(error)) => {
			state.calendar.pending_delete = None;
			log::warn!("could not delete the event: {error}");
			state.calendar.error = Some(error);
			Task::none()
		}
	}
}

/// Shows the user's answer on the event, and on its form if that is open.
fn set_own_response(
	state: &mut State,
	id: &str,
	response: calendar::meeting::Response,
) -> Option<calendar::meeting::Response> {
	let event = state.calendar.event_mut_anywhere(id)?;
	let previous = event.details.attendance.set_own_response(response);
	let email = event.details.attendance.own_email.clone()?;
	if let Some(draft) = state.calendar.edit_draft.as_mut().filter(|d| d.id == id) {
		draft.meeting.set_guest_response(&email, response);
	}
	previous
}

/// Sends a time already showing on the chip to Google.
fn write_event_time(
	state: &mut State,
	original: calendar::Event,
	start: chrono::DateTime<chrono::Local>,
	end: chrono::DateTime<chrono::Local>,
	scope: calendar::SaveScope,
	notify: bool,
) -> Task<Message> {
	let id = original.id.clone();
	state.calendar.error = None;
	if original.details.sleep.is_some() {
		let mut event = original;
		event.start = start;
		event.end = end;
		if let Some(draft) = state
			.calendar
			.edit_draft
			.as_ref()
			.filter(|draft| draft.id == id)
		{
			if let Some(record) = &draft.sleep {
				event.details.sleep = Some(record.clone());
			}
		}
		event.details.sleep.as_mut().unwrap().automatic =
			scope == calendar::SaveScope::CurrentAndFuture;
		// Only a single-night edit becomes an exception to the shared schedule.
		let settings = state.config.calendar.clone();
		state.timeblocking.revision += 1;
		return blocking::run(move || calendar::sleep::save(&settings, event, false)).map(
			move |result| Message::Event(EventMessage::SleepTimeSaved(id.clone(), scope, result)),
		);
	}
	blocking::run(move || {
		if !original.is_recurring() {
			return calendar::set_event_time(
				&original.calendar_id,
				&original.id,
				start,
				end,
				notify,
			);
		}
		let mut draft = calendar::EditDraft::from_event(&original);
		draft.start = start;
		draft.end = end;
		draft.meeting.notify = notify;
		// Tasks repeating with a block series are moved with it once Google
		// has the block's new time (see `timeblocking::reconcile`).
		calendar::save_event_scope(&original, &draft, scope, &draft.description, None).map(|_| ())
	})
	.map(move |result| Message::Event(EventMessage::EventTimeSaved(id.clone(), result)))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn assigned_tasks_open_their_block_and_are_edited_from_that_interface() {
		let mut state = State::blank();
		state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
		let parent = state.calendar.event("standup").unwrap().clone();
		state.config.calendar.app_calendar = Some(parent.calendar_id.clone());
		let key = crate::timeblocking::block_key(&parent);
		let child = state.calendar.event_mut("deep").unwrap();
		child.id = "task:child".into();
		child.calendar_id = "tasks:list".into();
		child.details.task_data = Some(crate::task_db::TaskData {
			id: 100,
			metadata: crate::timeblocking::Metadata {
				block: Some(key),
				..Default::default()
			},
		});
		let _ = update(&mut state, EventMessage::EventOpened("task:child".into()));
		assert_eq!(state.timeblocking.block.as_deref(), Some("standup"));
		assert!(state.calendar.open_event.is_none());
		let _ = update(&mut state, EventMessage::EventOpened("task:child".into()));
		assert_eq!(state.calendar.open_event.as_deref(), Some("task:child"));
		assert_eq!(state.timeblocking.block.as_deref(), Some("standup"));
		let _ = update(&mut state, EventMessage::EventClosed);
		assert_eq!(state.calendar.edit_draft.as_ref().unwrap().id, "standup");
	}
}
