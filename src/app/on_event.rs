//! An event that already exists: opening, moving and deleting it.

use super::*;

pub(super) fn update(state: &mut State, message: EventMessage) -> Task<Message> {
    match message {
        EventMessage::DescriptionOpen(id) => {
            let value =
                if let Some(draft) = state.calendar.edit_draft.as_ref().filter(|d| d.id == id) {
                    draft.description.clone()
                } else if let Some(event) = state.calendar.event(&id) {
                    calendar::notes::Notes::new(&crate::timeblocking::description(
                        &event.details.description,
                    ))
                    .without(&["DUE:", "DEADLINE:"])
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
                    if !state.habits.busy
                        && let Some(d) = &mut state.habits.draft
                    {
                        d.notes = value;
                    }
                }
            }
            Task::none()
        }
        EventMessage::EventOpened(id) => {
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
            let mut close_block = None;
            if let Some(event) = state.calendar.event(&id).cloned() {
                let is_task = event.is_task();
                let mut draft = calendar::EditDraft::from_event(&event);
                draft.loading = event.details.habit && event.details.recurring_event_id.is_some();
                // A block's panel and the event pane are two different
                // sidebars; opening one closes the other.
                if state.timeblocking.block.is_some() {
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
                    state.timeblocking.deadline =
                        calendar::deadline::input_text(&event.details.description);
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
                .filter(|event| event.details.habit && event.details.recurring_event_id.is_some())
                .cloned()
                .map(|event| {
                    let settings = state.config.calendar.clone();
                    let id = id.clone();
                    blocking::run(move || {
                        calendar::habit_edit_draft(
                            &settings.client_id,
                            &settings.client_secret,
                            &event,
                        )
                    })
                    .map(move |result| {
                        Message::Event(EventMessage::HabitDetailsLoaded(id.clone(), result))
                    })
                });
            state.calendar.open_event = Some(id);
            // Recorded during this click, before the message is handled.
            state.calendar.details_below = ui::calendar::click_was_in_the_top_half();
            state.inbox.picker = None;
            Task::batch(close_block.into_iter().chain(habit_load))
        }
        EventMessage::EventClosed => {
            state.calendar.open_event = None;
            state.calendar.edit_draft = None;
            state.calendar.detail_event = None;
            state.inbox.picker = None;
            Task::none()
        }
        EventMessage::EventTimeChanged { id, start, end } => {
            if state
                .calendar
                .event(&id)
                .is_some_and(|event| event.is_task())
            {
                return crate::timeblocking::update(
                    state,
                    crate::timeblocking::Action::Schedule(id, start, end),
                );
            }
            let Some(calendar_id) = state
                .calendar
                .event(&id)
                .filter(|event| !event.is_task())
                .map(|event| event.calendar_id.clone())
            else {
                return Task::none();
            };

            // The chip moves now; the lighter shade and the write follow. A
            // second drag on the same event is refused until this one lands.
            if !state.calendar.begin_time_change(&id, start, end) {
                return Task::none();
            }

            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            let request_id = id.clone();
            state.calendar.error = None;

            blocking::run(move || {
                calendar::set_event_time(
                    &client_id,
                    &client_secret,
                    &calendar_id,
                    &request_id,
                    start,
                    end,
                )
            })
            .map(move |result| Message::Event(EventMessage::EventTimeSaved(id.clone(), result)))
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
        | EventMessage::EventDeleteScoped(_, _)) => {
            let (id, scope) = match request {
                EventMessage::EventDeleteScoped(id, scope) => (id, scope),
                EventMessage::EventDeleteRequested(id) => (id, calendar::SaveScope::Current),
                _ => unreachable!(),
            };
            // Only calendar events can be deleted this way; Google Tasks live
            // under the Tasks API and are ticked off instead.
            let is_task = state
                .calendar
                .event(&id)
                .map(|event| event.is_task())
                .unwrap_or(false);
            if is_task {
                return Task::none();
            }
            if let Some(draft) = &mut state.calendar.edit_draft {
                draft.delete_menu_open = false;
                draft.save_menu_open = false;
            }
            state.calendar.pending_delete_scope = scope;
            state.calendar.pending_delete = Some(id);
            Task::none()
        }
        EventMessage::EventDeleteConfirmed(id) => {
            if state.calendar.pending_delete.as_deref() != Some(id.as_str()) {
                return Task::none();
            }
            let scope = state.calendar.pending_delete_scope;
            let Some(calendar_id) = state
                .calendar
                .event(&id)
                .filter(|event| !event.is_task())
                .map(|event| event.calendar_id.clone())
            else {
                state.calendar.pending_delete = None;
                return Task::none();
            };

            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            let request_id = id.clone();
            state.calendar.error = None;

            blocking::run(move || {
                calendar::delete_event_scope(
                    &client_id,
                    &client_secret,
                    &calendar_id,
                    &request_id,
                    scope,
                )
            })
            .map(move |result| Message::Event(EventMessage::EventDeleted(id.clone(), result)))
        }
        EventMessage::EventDeleteCancelled => {
            state.calendar.pending_delete = None;
            Task::none()
        }
        EventMessage::EventDeleted(id, Ok(())) => {
            state.calendar.pending_delete = None;
            // The event is gone on Google's side, so drop it from the day in
            // view. The next sync reconciles the cache.
            if state.range_view_active() {
                state.calendar.forget_event_everywhere(&id);
            } else {
                state.calendar.events.retain(|event| event.id != id);
            }
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
                state.habits.habits.retain(|habit| habit.id != id);
            }
            Task::batch([
                close,
                Task::done(Message::Calendar(CalendarMessage::CalendarSyncRequested)),
            ])
        }
        EventMessage::EventDeleted(_id, Err(error)) => {
            state.calendar.pending_delete = None;
            log::warn!("could not delete the event: {error}");
            state.calendar.error = Some(error);
            Task::none()
        }
    }
}
