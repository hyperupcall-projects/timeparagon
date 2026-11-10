//! Type switching and saving for the sidebar's shared creation form.
use super::*;
use calendar::DraftKind;
use chrono::Timelike;

pub(crate) fn select(state: &mut State, kind: DraftKind) -> Task<Message> {
    let Some(task) = state.inbox.draft.as_ref() else {
        return Task::none();
    };
    if task.saving || task.fetching || state.inbox.voice.active() || state.inbox.image.busy {
        return Task::none();
    }
    crate::image_input::stash(state);
    let task = state.inbox.draft.as_ref().unwrap();
    let previous = task.kind.unwrap_or(DraftKind::Task);
    if previous == kind {
        return Task::none();
    }
    let (title, description) = if previous == DraftKind::Task {
        (
            task.title.clone(),
            task.event
                .as_ref()
                .map(|event| event.details.description.clone())
                .unwrap_or_default(),
        )
    } else {
        let Some(entry) = state.calendar.draft.take() else {
            return Task::none();
        };
        let common = (entry.title.clone(), entry.description.clone());
        state.inbox.entry_drafts.push(entry);
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
    let cached = state
        .inbox
        .entry_drafts
        .iter()
        .position(|entry| entry.kind == kind);
    let mut entry = if let Some(index) = cached {
        state.inbox.entry_drafts.remove(index)
    } else {
        let now = crate::clock::now();
        let start = state
            .inbox
            .draft
            .as_ref()
            .and_then(|task| task.event.as_ref())
            .filter(|event| !event.all_day)
            .map(|event| event.start)
            .unwrap_or_else(|| calendar::at_clock(state.calendar.day, now.hour(), 0));
        let hours = if kind == DraftKind::Sleep { 8 } else { 1 };
        let calendar_id = if kind == DraftKind::Event {
            state
                .calendar
                .default_calendar_id(&state.config.calendar)
                .unwrap_or_default()
        } else {
            state
                .config
                .calendar
                .app_calendar
                .clone()
                .unwrap_or_default()
        };
        let mut entry = calendar::Draft::new(calendar_id, start, start + Duration::hours(hours));
        entry.kind = kind;
        entry.editor_open = true;
        if kind == DraftKind::Habit {
            entry.rule = crate::recurrence::Rule::weekly(start.date_naive());
        }
        entry
    };
    entry.title = title;
    entry.description = description;
    state.calendar.draft = Some(entry);
    state.inbox.draft.as_mut().unwrap().kind = Some(kind);
    Task::none()
}

/// Validate before starting any persistence or network work.
fn validate(draft: &calendar::Draft) -> Result<(), String> {
    if draft.kind != DraftKind::Sleep && draft.title.trim().is_empty() {
        return Err("Enter a title".into());
    }
    if deadline::parse_date(&draft.date_text).is_err()
        || deadline::parse_date(&draft.end_date_text).is_err()
        || (!draft.all_day
            && (calendar::parse_clock(&draft.start_text).is_none()
                || calendar::parse_clock(&draft.end_text).is_none()))
        || draft.end <= draft.start
    {
        return Err("Enter valid dates and times, with the end after the start".into());
    }
    draft.recurrence_lines()?;
    if draft.kind == DraftKind::Event && draft.calendar_id.is_empty() {
        return Err("Connect Google Calendar in Settings to add events".into());
    }
    Ok(())
}

pub(super) fn create(state: &mut State) -> Task<Message> {
    if state.inbox.draft.as_ref().is_none_or(|draft| draft.saving) {
        return Task::none();
    }
    let Some(mut draft) = state.calendar.draft.clone() else {
        return Task::none();
    };
    if let Err(error) = validate(&draft) {
        state.calendar.error = Some(error);
        return Task::none();
    }
    draft.meeting.add_typed_address();
    let (habit, persist) = if draft.kind == DraftKind::Habit {
        let (id, task) = crate::habits::habit_for_title(state, &draft.title);
        (Some(id), task)
    } else {
        (None, Task::none())
    };
    state.inbox.draft.as_mut().unwrap().saving = true;
    state.calendar.draft.as_mut().unwrap().saving = true;
    state.calendar.error = None;
    let settings = state.config.calendar.clone();
    Task::batch([
        persist,
        blocking::run(move || write(&settings, draft, habit))
            .map(|result| Message::Inbox(Action::Created(result))),
    ])
}

fn write(
    settings: &crate::config::CalendarSettings,
    draft: calendar::Draft,
    habit: Option<String>,
) -> Result<Event, String> {
    let mut event = Event {
        id: String::new(),
        calendar_id: draft.calendar_id.clone(),
        title: draft.title.trim().into(),
        start: draft.start,
        end: draft.end,
        all_day: draft.all_day,
        completed: false,
        color: calendar::colors::DEFAULT_EVENT,
        details: calendar::Details {
            description: draft.description.clone(),
            projects: draft.projects.ids.clone(),
            recurrence: draft.recurrence_lines()?,
            options: draft.options.clone(),
            ..Default::default()
        },
    };
    match draft.kind {
        DraftKind::Task => unreachable!("tasks use the task writer"),
        DraftKind::Sleep => {
            event.id = calendar::sleep::NEW_ID.into();
            event.color = calendar::sleep::COLOR;
            event.title = "Sleep".into();
            event.details.sleep = Some(calendar::sleep::Record {
                wake_date: event.end.date_naive(),
                automatic: false,
                estimated_start: false,
                estimated_end: false,
            });
            return calendar::sleep::save(settings, event, true);
        }
        DraftKind::Habit => {
            let mut edit = calendar::EditDraft::from_event(&event);
            edit.rule = draft.rule;
            let habit = habit.ok_or("Choose a habit name")?;
            let saved = calendar::create_habit(&edit, None, &habit)?;
            event.id = saved.id;
            event.calendar_id = saved.calendar_id;
            event.color = settings.habit_color();
            event.details.habit = true;
            event.details.habit_id = Some(habit);
        }
        DraftKind::TimeBlock => {
            let (id, calendar) = calendar::create_time_block(
                settings.app_calendar.as_deref(),
                &event.title,
                &event.details.description,
                None,
                event.start,
                event.end,
                &event.details.projects,
            )?;
            event.id = id;
            event.calendar_id = calendar;
            event.color = settings.time_block_color();
        }
        DraftKind::Event => {
            let saved = calendar::create_event(
                &event.calendar_id,
                &event.title,
                &event.details.description,
                None,
                event.start,
                event.end,
                &event.details.recurrence,
                &event.details.projects,
                event.all_day,
                Some(&draft.options),
                Some(&draft.meeting),
            )?;
            event.id = saved.id;
            event.details.link = saved.link;
            event.details.meeting = saved.meeting;
            event.details.attendance = calendar::meeting::Attendance {
                guests: draft.meeting.guests,
                permissions: draft.meeting.permissions,
                organizer: true,
                own_email: None,
            };
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

    #[test]
    fn switching_types_keeps_common_text_and_each_types_fields() {
        let mut state = state();
        let _ = select(&mut state, DraftKind::Event);
        let event = state.calendar.draft.as_mut().unwrap();
        event.title = "Read a chapter".into();
        event.description = "Take notes".into();
        event.calendar_id = "chosen-calendar".into();
        event.set_start_text("11:15".into());
        let start = event.start;

        let _ = select(&mut state, DraftKind::Habit);
        let habit = state.calendar.draft.as_ref().unwrap();
        assert_eq!(habit.title, "Read a chapter");
        assert_eq!(habit.description, "Take notes");
        assert_eq!(habit.rule.repeat, crate::recurrence::RepeatMode::Weekly);
        let _ = select(&mut state, DraftKind::Event);
        let event = state.calendar.draft.as_ref().unwrap();
        assert_eq!(event.start, start);
        assert_eq!(event.calendar_id, "chosen-calendar");
        assert_eq!(event.rule.repeat, crate::recurrence::RepeatMode::Never);

        let _ = select(&mut state, DraftKind::Task);
        assert!(state.calendar.draft.is_none());
        let task = state.inbox.draft.as_ref().unwrap();
        assert_eq!(task.title, "Read a chapter");
        assert_eq!(
            task.event.as_ref().unwrap().details.description,
            "Take notes"
        );
        let _ = select(&mut state, DraftKind::Event);
        assert_eq!(state.calendar.draft.as_ref().unwrap().start, start);
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
        assert_ne!(
            state.calendar.draft.as_ref().unwrap().start,
            start + Duration::days(1)
        );
        let _ = super::super::update(&mut state, Action::Cancel);
        assert_eq!(state.calendar.draft, Some(old));
    }

    #[test]
    fn invalid_calendar_fields_keep_the_form_open_without_saving() {
        for kind in [
            DraftKind::Event,
            DraftKind::TimeBlock,
            DraftKind::Habit,
            DraftKind::Sleep,
        ] {
            let mut state = state();
            let _ = select(&mut state, kind);
            state.calendar.draft.as_mut().unwrap().date_text = "invalid".into();
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
            assert!(!state.calendar.draft.as_ref().unwrap().saving);
        }
    }

    #[test]
    fn saving_locks_type_switching_and_cancellation_and_errors_allow_retry() {
        let mut state = state();
        let _ = select(&mut state, DraftKind::TimeBlock);
        state.inbox.draft.as_mut().unwrap().saving = true;
        state.calendar.draft.as_mut().unwrap().saving = true;
        let _ = select(&mut state, DraftKind::Task);
        let _ = super::super::update(&mut state, Action::Cancel);
        assert_eq!(
            state.inbox.draft.as_ref().unwrap().kind,
            Some(DraftKind::TimeBlock)
        );
        let _ = super::super::update(&mut state, Action::Created(Err("Could not save".into())));
        assert!(!state.calendar.draft.as_ref().unwrap().saving);
        assert!(!state.inbox.draft.as_ref().unwrap().saving);
        assert_eq!(
            state.inbox.draft.as_ref().unwrap().error.as_deref(),
            Some("Could not save")
        );
        let _ = super::super::update(&mut state, Action::Cancel);
        assert!(state.inbox.draft.is_none());
        assert!(state.calendar.draft.is_none());
        assert!(state.inbox.entry_drafts.is_empty());
    }
}
