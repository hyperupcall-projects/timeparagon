//! The form for a new event.

use super::*;

pub(super) fn update(state: &mut State, message: EventDraftMessage) -> Task<Message> {
    match message {
        EventDraftMessage::EventDraftMeeting(action) => {
            let Some(draft) = state.calendar.draft.as_mut().filter(|draft| !draft.saving) else {
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
            state.draft_kind_focus = None;
            state.calendar.day = calendar::day_at(start, state.config.calendar.day_start_hour());
            iced::widget::operation::focus("calendar-draft-title")
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
        EventDraftMessage::EventDraftCycleKind {
            reverse,
            title_focused,
        } => {
            if !state
                .calendar
                .draft
                .as_ref()
                .is_some_and(|draft| !draft.editor_open)
            {
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
                draft.rule.apply_action(action, draft.start.date_naive());
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
        EventDraftMessage::EventDraftEndDateChanged(text) => {
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.set_end_date_text(text);
            }
            Task::none()
        }
        EventDraftMessage::EventDraftAllDayToggled(all_day) => {
            if let Some(draft) = state.calendar.draft.as_mut().filter(|draft| !draft.saving) {
                draft.set_all_day(all_day);
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
            crate::inbox::new_task_at(state, draft.title, draft.start, draft.end)
        }
        EventDraftMessage::EventDraftKindSelected(calendar::DraftKind::Habit) => {
            // Creation and editing share the habit details pane.
            let Some(draft) = state.calendar.draft.take() else {
                return Task::none();
            };
            let color_id = None;
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
        EventDraftMessage::EventDraftKindSelected(calendar::DraftKind::TimeBlock) => {
            // A time block is planned in the block panel, not filled in as an
            // event, so choosing it writes the block and opens that panel.
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
            draft.calendar_dropdown_open = false;
            update(state, EventDraftMessage::EventDraftSaveRequested)
        }
        EventDraftMessage::EventDraftKindSelected(kind) => {
            let default_calendar = state.calendar.default_calendar_id(&state.config.calendar);
            if let Some(draft) = state.calendar.draft.as_mut() {
                if draft.saving {
                    return Task::none();
                }
                draft.kind = kind;
                draft.editor_open = true;
                if kind == calendar::DraftKind::Event {
                    draft.calendar_id = default_calendar.clone().unwrap_or_default();
                }
                // An ordinary event uses the normal default calendar.
                if let Some(calendar_id) = match kind {
                    calendar::DraftKind::Event
                    | calendar::DraftKind::Habit
                    | calendar::DraftKind::Task => default_calendar,
                    calendar::DraftKind::Sleep | calendar::DraftKind::TimeBlock => None,
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
            state.draft_kind_focus = None;
            state.calendar.draft = None;
            Task::none()
        }
        EventDraftMessage::EventDraftSaveRequested => {
            if let Some(draft) = state.calendar.draft.as_mut().filter(|draft| !draft.saving) {
                draft.meeting.add_typed_address();
            }
            let Some(draft) = state.calendar.draft.clone() else {
                return Task::none();
            };
            if draft.saving {
                return Task::none();
            }

            if draft.end <= draft.start {
                state.calendar.error = Some(if draft.all_day {
                    "The end date has to be on or after the start".to_owned()
                } else {
                    "The end time has to be after the start".to_owned()
                });
                return Task::none();
            }

            // Time blocks use the app calendar; ordinary events use a regular calendar.
            let is_block = draft.kind == calendar::DraftKind::TimeBlock;
            let all_day = draft.all_day && draft.kind != calendar::DraftKind::TimeBlock;
            let color = if is_block {
                state.config.calendar.time_block_color()
            } else {
                state.calendar.calendar_color(&draft.calendar_id)
            };
            let description = if is_block {
                calendar::event_description(&draft.description)
            } else {
                draft.description.clone()
            };
            let color_id = None;
            let recurrence = match draft.recurrence_lines() {
                Ok(lines) => lines,
                Err(error) => {
                    state.calendar.error = Some(error);
                    return Task::none();
                }
            };
            let known_app = state.config.calendar.app_calendar.clone();
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.saving = true;
            }
            state.calendar.error = None;

            blocking::run(move || {
                let (mut link, mut meeting_link) = (String::new(), String::new());
                // A block resolves (and, if need be, creates) the app's own
                // calendar; an event is written to the calendar it was given.
                let (id, calendar_id) = if draft.kind == calendar::DraftKind::TimeBlock {
                    calendar::create_time_block(
                        known_app.as_deref(),
                        &draft.title,
                        &description,
                        color_id,
                        draft.start,
                        draft.end,
                        &draft.projects.ids,
                    )?
                } else {
                    let created = calendar::create_event(
                        &draft.calendar_id,
                        &draft.title,
                        &description,
                        color_id,
                        draft.start,
                        draft.end,
                        &recurrence,
                        &draft.projects.ids,
                        all_day,
                        Some(&draft.options),
                        (!is_block).then_some(&draft.meeting),
                    )?;
                    link = created.link;
                    meeting_link = created.meeting;
                    (created.id, draft.calendar_id.clone())
                };

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
                    all_day,
                    color,
                    completed: false,
                    details: calendar::Details {
                        options: draft.options.clone(),
                        projects: draft.projects.ids.clone(),
                        description,
                        recurrence: recurrence.clone(),
                        link,
                        meeting: meeting_link,
                        attendance: calendar::meeting::Attendance {
                            guests: if is_block {
                                Vec::new()
                            } else {
                                draft.meeting.guests.clone()
                            },
                            permissions: draft.meeting.permissions,
                            organizer: true,
                            own_email: None,
                        },
                        ..calendar::Details::default()
                    },
                })
            })
            .map(|v0| Message::EventDraft(EventDraftMessage::EventDraftSaved(v0)))
        }
        EventDraftMessage::EventDraftSaved(Ok(event)) => {
            let was_block = state
                .calendar
                .draft
                .as_ref()
                .is_some_and(|draft| draft.kind == calendar::DraftKind::TimeBlock);
            let zoom = state.calendar.draft.as_ref().is_some_and(|draft| {
                !was_block
                    && state.config.calendar.app_calendar.as_deref()
                        != Some(draft.calendar_id.as_str())
                    && draft.meeting.zoom_wanted()
                    && !calendar::meeting::has_zoom_link(&draft.description)
            });
            state.calendar.draft = None;
            if zoom {
                on_event_edit::open_zoom_setup(state, &event.details.link);
            }
            // A block that had to create the TimeSlay calendar makes that
            // calendar the remembered app calendar.
            let remember = was_block && state.config.calendar.app_calendar.is_none();
            if remember {
                state.config.calendar.app_calendar = Some(event.calendar_id.clone());
            }
            let block = was_block.then(|| event.id.clone());
            state.calendar.add_event(event);
            let mut tasks = vec![refresh_week_after_write(state)];
            if remember {
                tasks.push(crate::app::queue_save(state));
            }
            if let Some(id) = block {
                // The block is planned in the right-hand panel, not on the
                // event form it was never sent to.
                tasks.push(Task::done(Message::TimeBlocking(
                    crate::timeblocking::Action::Plan(id),
                )));
            }
            Task::batch(tasks)
        }
        EventDraftMessage::EventDraftSaved(Err(error)) => {
            // The popup stays open so the half-filled event is not lost.
            if let Some(draft) = state.calendar.draft.as_mut() {
                draft.saving = false;
            }
            log::warn!("could not create the event: {error}");
            state.calendar.error = Some(error);
            Task::none()
        }
    }
}
