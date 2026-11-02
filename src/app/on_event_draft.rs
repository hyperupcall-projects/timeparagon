//! The form for a new event.

use super::*;

pub(super) fn update(state: &mut State, message: EventDraftMessage) -> Task<Message> {
    match message {
        EventDraftMessage::EventDraftStarted { start, end } => {
            let Some(calendar_id) = state.calendar.default_calendar_id(&state.config.calendar)
            else {
                state.calendar.error =
                    Some("Connect Google Calendar in Settings to add events".to_owned());
                return Task::none();
            };

            state.calendar.error = None;
            state.calendar.begin_draft(calendar_id, start, end);
            state.calendar.day = calendar::day_at(start, state.config.calendar.day_start_hour());
            Task::none()
        }
        EventDraftMessage::EventDraftProjects(action) => {
            if let calendar::projects::Action::Add(id) = &action
                && state.config.projects.get(*id).is_none()
            {
                return Task::none();
            }
            if let Some(draft) = &mut state.calendar.draft {
                draft.projects.update(action);
            }
            Task::none()
        }
        EventDraftMessage::EventDraftOptions(action) => {
            if let Some(draft) = &mut state.calendar.draft {
                draft.options.update(action);
            }
            Task::none()
        }
        EventDraftMessage::EventDraftTitleChanged(title) => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.title = title;
            }
            Task::none()
        }
        EventDraftMessage::EventDraftDescriptionChanged(description) => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.description = description;
            }
            Task::none()
        }
        EventDraftMessage::EventDraftDateChanged(text) => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.set_date_text(text);
            }
            Task::none()
        }
        EventDraftMessage::EventDraftRecurrence(action) => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                if matches!(&action, crate::recurrence::Action::SkipAdd) {
                    match draft.rule.add_skip() {
                        Ok(()) => state.calendar.error = None,
                        Err(error) => state.calendar.error = Some(error),
                    }
                } else {
                    draft.rule.apply_action(action, draft.start.date_naive());
                }
            }
            Task::none()
        }
        EventDraftMessage::EventDraftStartChanged(text) => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.set_start_text(text);
            }
            Task::none()
        }
        EventDraftMessage::EventDraftEndChanged(text) => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.set_end_text(text);
            }
            Task::none()
        }
        EventDraftMessage::EventDraftKindSelected(calendar::DraftKind::Sleep) => {
            let Some(draft) = state.calendar.draft.take() else {
                return Task::none();
            };
            let start = draft.start;
            let end = start
                + chrono::Duration::hours(i64::from(state.config.calendar.sleep_goal_hours()));
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
        EventDraftMessage::EventDraftKindSelected(calendar::DraftKind::Habit) => {
            // Creation and editing share the habit details pane.
            let Some(draft) = state.calendar.draft.take() else {
                return Task::none();
            };
            let color_id =
                calendar::colors::event_id(state.config.calendar.habit_color()).map(str::to_owned);
            let habit = crate::habits::Draft::for_calendar_with_rule(
                draft.start,
                draft.end,
                draft.title,
                draft.description,
                color_id,
                draft.rule.clone(),
            );
            let task = crate::habits::open_new(state, habit);
            if let Some(edit) = &mut state.calendar.edit_draft {
                edit.options = draft.options;
            }
            task
        }
        EventDraftMessage::EventDraftKindSelected(kind) => {
            let app_calendar = state.config.calendar.app_calendar.clone();
            let default_calendar = state.calendar.default_calendar_id(&state.config.calendar);
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.kind = kind;
                draft.editor_open = true;
                // A time block always lives on the app's TimeSlay calendar;
                // switching back to an event returns to the normal default.
                if let Some(calendar_id) = match kind {
                    calendar::DraftKind::TimeBlock | calendar::DraftKind::Sleep => app_calendar,
                    calendar::DraftKind::Event | calendar::DraftKind::Habit => default_calendar,
                } {
                    draft.calendar_id = calendar_id;
                }
                draft.calendar_dropdown_open = false;
            }
            Task::none()
        }
        EventDraftMessage::EventDraftCalendarSelected(id) => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.calendar_id = id;
                draft.calendar_dropdown_open = false;
            }
            Task::none()
        }
        EventDraftMessage::EventDraftCalendarDropdownToggled => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.calendar_dropdown_open = !draft.calendar_dropdown_open;
            }
            Task::none()
        }
        EventDraftMessage::EventDraftCancelled => {
            state.calendar.draft = None;
            Task::none()
        }
        EventDraftMessage::EventDraftSaveRequested => {
            let Some(draft) = state.calendar.draft.clone() else {
                return Task::none();
            };

            if draft.end <= draft.start {
                state.calendar.error = Some("The end time has to be after the start".to_owned());
                return Task::none();
            }

            // A time block is always written to the app's TimeSlay calendar
            // and labelled and colored as a block; an ordinary event stays on
            // the calendar the picker shows.
            let is_block = draft.kind == calendar::DraftKind::TimeBlock
                || state.config.calendar.app_calendar.as_deref()
                    == Some(draft.calendar_id.as_str());
            let calendar_id = if draft.kind == calendar::DraftKind::TimeBlock {
                state
                    .config
                    .calendar
                    .app_calendar
                    .clone()
                    .unwrap_or_else(|| draft.calendar_id.clone())
            } else {
                draft.calendar_id.clone()
            };
            let color = if is_block {
                state.config.calendar.time_block_color()
            } else {
                state.calendar.calendar_color(&calendar_id)
            };
            let description = if is_block {
                calendar::event_description(&draft.description, "Time Block", &[])
            } else {
                draft.description.clone()
            };
            let color_id = if is_block {
                calendar::colors::event_id(state.config.calendar.time_block_color())
            } else {
                None
            };
            let recurrence = match draft.recurrence_lines() {
                Ok(lines) => lines,
                Err(error) => {
                    state.calendar.error = Some(error);
                    return Task::none();
                }
            };
            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            state.calendar.error = None;

            blocking::run(move || {
                let id = calendar::create_event(
                    &client_id,
                    &client_secret,
                    &calendar_id,
                    &draft.title,
                    &description,
                    color_id,
                    draft.start,
                    draft.end,
                    &recurrence,
                    &draft.projects.ids,
                    Some(&draft.options),
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
                    details: calendar::Details {
                        options: draft.options.clone(),
                        projects: draft.projects.ids.clone(),
                        description,
                        recurrence: recurrence.clone(),
                        ..calendar::Details::default()
                    },
                })
            })
            .map(|v0| Message::EventDraft(EventDraftMessage::EventDraftSaved(v0)))
        }
        EventDraftMessage::EventDraftSaved(Ok(event)) => {
            state.calendar.draft = None;
            state.calendar.add_event(event);
            refresh_week_after_write(state)
        }
        EventDraftMessage::EventDraftSaved(Err(error)) => {
            // The popup stays open so the half-filled event is not lost.
            log::warn!("could not create the event: {error}");
            state.calendar.error = Some(error);
            Task::none()
        }
    }
}
