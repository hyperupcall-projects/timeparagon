use chrono::{Duration, Local, NaiveDate, TimeZone};
use iced::Size;
use iced_test::{Simulator, core::Settings};
use timeslay::app::{
    CalendarMessage, EventDraftMessage, EventEditMessage, EventMessage, SettingsMessage,
};
use timeslay::{
    app::{self, Message, State},
    calendar::{self, sleep},
    config::Config,
    ui,
};

fn state() -> State {
    let mut state = State::blank();
    let mut snapshot = calendar::sample_snapshot();
    snapshot.day = NaiveDate::from_ymd_opt(2030, 11, 4).unwrap();
    let mut event = snapshot.events[1].clone();
    event.id = "sleep".into();
    event.calendar_id = "app".into();
    event.title = "Sleep".into();
    event.start = Local
        .from_local_datetime(&snapshot.day.and_hms_opt(1, 0, 0).unwrap())
        .earliest()
        .unwrap();
    event.end = event.start + Duration::hours(8);
    event.details.sleep = Some(sleep::Record::planned(snapshot.day));
    state.config.calendar.app_calendar = Some("app".into());
    snapshot.events = vec![event];
    state.calendar = calendar::Store::with_snapshot(snapshot);
    state.week_anchor = state.calendar.day;
    state
}

#[test]
fn sleep_opens_its_editor_with_times_and_independent_estimate_toggles() {
    let mut state = state();
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened("sleep".into())),
    );
    assert!(state.timeblocking.block.is_none());
    let metrics = ui::layout::Metrics::of(Size::new(420.0, 800.0));
    let mut view = Simulator::with_size(
        Settings::default(),
        metrics.size,
        ui::calendar::details_pane(&state, &metrics),
    );
    for label in [
        "Bedtime",
        "8h",
        "Nov. 4, 2030",
        "Wake-up time",
        "Estimated Sleep Start",
        "Estimated Sleep End",
        "Save sleep",
    ] {
        assert!(view.find(label).is_ok(), "missing {label}");
    }
    view.click("Estimated Sleep Start").unwrap();
    assert!(view.into_messages().any(|m| matches!(
        m,
        Message::Settings(SettingsMessage::SleepEstimatedStart(false))
    )));
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::SleepEstimatedStart(false)),
    );
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::SleepEstimatedEnd(true)),
    );
    let _ = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditStartChanged("00:30".into())),
    );
    let _ = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditEndChanged("09:15".into())),
    );
    let draft = state.calendar.edit_draft.as_ref().unwrap();
    assert!(!draft.sleep.as_ref().unwrap().estimated_start);
    assert!(draft.sleep.as_ref().unwrap().estimated_end);
    assert_eq!(draft.start_text, "00:30");
    assert_eq!(draft.end_text, "09:15");
    let _ = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditStartChanged("bad".into())),
    );
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::SleepSaveRequested),
    );
    assert!(state.calendar.error.is_some());
    assert!(!state.calendar.edit_draft.as_ref().unwrap().saving);
}

#[test]
fn bedtime_date_follows_the_clock_relative_to_wakeup() {
    let mut state = state();
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened("sleep".into())),
    );
    for (bedtime, wakeup, expected_date) in [
        ("23:00", "09:00", "2030-11-03"),
        ("01:00", "09:00", "2030-11-04"),
        ("00:00", "09:00", "2030-11-04"),
        ("08:00", "07:00", "2030-11-03"),
        ("08:00", "09:00", "2030-11-04"),
    ] {
        let _ = app::update(
            &mut state,
            Message::EventEdit(EventEditMessage::EventEditStartChanged(bedtime.into())),
        );
        let _ = app::update(
            &mut state,
            Message::EventEdit(EventEditMessage::EventEditEndChanged(wakeup.into())),
        );
        let draft = state.calendar.edit_draft.as_ref().unwrap();
        assert_eq!(draft.date_text, expected_date);
        assert_eq!(draft.start.date_naive().to_string(), expected_date);
        assert_eq!(draft.end_date_text, "2030-11-04");
        assert_eq!(draft.end.date_naive().to_string(), "2030-11-04");
        assert!(draft.start < draft.end);
    }
}

#[test]
fn sleep_has_a_fixed_gray_source_and_an_independent_persisted_toggle() {
    let mut state = state();
    let sleep = state.calendar.event("sleep").unwrap();
    assert_eq!(sleep.source_id(), sleep::SOURCE_ID);
    assert_eq!(sleep.display_color(&state.config.calendar), sleep::COLOR);
    assert!(!timeslay::timeblocking::is_block(
        sleep,
        &state.config.calendar
    ));
    assert!(
        state
            .calendar
            .display_sources(&state.config.calendar)
            .iter()
            .any(|source| source.id == sleep::SOURCE_ID && source.name == "Sleep")
    );
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarToggled("app".into(), false)),
    );
    assert_eq!(
        state.calendar.timed_events(&state.config.calendar).count(),
        1
    );
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarToggled(
            sleep::SOURCE_ID.into(),
            false,
        )),
    );
    assert_eq!(
        state.calendar.timed_events(&state.config.calendar).count(),
        0
    );
    let saved: Config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
    assert!(saved.calendar.disabled.contains(&sleep::SOURCE_ID.into()));
}

#[test]
fn weekly_sleep_times_persist_and_invalid_input_is_ignored() {
    let mut state = state();
    let original = state.config.calendar.sleep_schedule;
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::SleepTimeChanged(
            chrono::Weekday::Tue,
            true,
            "22:45".into(),
        )),
    );
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::SleepTimeChanged(
            chrono::Weekday::Tue,
            false,
            "07:15".into(),
        )),
    );
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::SleepTimeChanged(
            chrono::Weekday::Tue,
            false,
            "invalid".into(),
        )),
    );
    let saved: Config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
    assert_eq!(
        saved.calendar.sleep_schedule[1].bedtime.to_string(),
        "22:45:00"
    );
    assert_eq!(
        saved.calendar.sleep_schedule[1].wakeup.to_string(),
        "07:15:00"
    );
    for index in [0, 2, 3, 4, 5, 6] {
        assert_eq!(saved.calendar.sleep_schedule[index], original[index]);
    }
}

#[test]
fn a_new_sleep_type_opens_the_sleep_editor() {
    let mut state = state();
    let event = state.calendar.events[0].clone();
    let _ = app::update(
        &mut state,
        Message::EventDraft(EventDraftMessage::EventDraftStarted {
            start: event.start,
            end: event.end,
        }),
    );
    let _ = app::update(
        &mut state,
        Message::EventDraft(EventDraftMessage::EventDraftKindSelected(
            calendar::DraftKind::Sleep,
        )),
    );
    assert!(state.calendar.draft.is_none());
    assert_eq!(state.calendar.open_event.as_deref(), Some(sleep::NEW_ID));
    let draft = state.calendar.edit_draft.as_ref().unwrap();
    assert_eq!(draft.end - draft.start, Duration::hours(8));
    assert!(draft.sleep.is_some());
}

#[test]
fn switching_to_a_cached_day_without_sleep_does_not_request_detection() {
    let mut state = state();
    state.calendar.events.clear();
    let snapshot = calendar::sample_snapshot();
    let mut event = snapshot.events[1].clone();
    event.start = Local
        .from_local_datetime(&state.calendar.day.and_hms_opt(9, 0, 0).unwrap())
        .earliest()
        .unwrap();
    event.end = event.start + Duration::hours(1);
    state.calendar.events.push(event);
    assert!(state.calendar.has_calendar_events_on(state.calendar.day));
    state.calendar.connection = calendar::Connection::SignedIn;
    state.calendar.last_sync = Some(Local::now());
    let day = state.calendar.day;
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarDaySelected(day)),
    );
    assert_eq!(state.calendar.sync, calendar::SyncState::Idle);
}

#[test]
fn sleep_moves_and_both_resize_edges_preview_then_cancel_without_changing_settings() {
    for edge in ["move", "start", "end"] {
        let mut state = state();
        let original = state.calendar.event("sleep").unwrap().clone();
        let config = state.config.calendar.clone();
        let step = Duration::minutes(15);
        let start = original.start
            + if edge == "end" {
                Duration::zero()
            } else {
                step
            };
        let end = original.end
            + if edge == "start" {
                Duration::zero()
            } else {
                step
            };
        let _ = app::update(
            &mut state,
            Message::Event(EventMessage::EventTimeChanged {
                id: original.id.clone(),
                start,
                end,
            }),
        );
        let pending = state.calendar.pending_move.as_ref().unwrap();
        assert_eq!(pending.scope, None);
        assert_eq!(pending.original, original);
        assert_eq!(state.calendar.event("sleep").unwrap().start, start);
        let _ = app::update(
            &mut state,
            Message::Event(EventMessage::EventMoveConfirmed(false)),
        );
        assert!(state.calendar.pending_move.is_some());
        let _ = app::update(&mut state, Message::Event(EventMessage::EventMoveCancelled));
        assert_eq!(state.calendar.event("sleep").unwrap(), &original);
        assert_eq!(state.config.calendar, config);
    }
}

#[test]
fn sleep_scope_success_updates_only_the_selected_weekday_from_the_selected_night() {
    for scope in [
        calendar::SaveScope::Current,
        calendar::SaveScope::CurrentAndFuture,
    ] {
        let mut state = state();
        let mut event = state.calendar.event("sleep").unwrap().clone();
        let day = event.details.sleep.as_ref().unwrap().wake_date;
        let before = sleep::times_for(&state.config.calendar, day);
        event.start += Duration::minutes(30);
        event.end += Duration::minutes(15);
        event.details.sleep.as_mut().unwrap().automatic =
            scope == calendar::SaveScope::CurrentAndFuture;
        let _ = app::update(
            &mut state,
            Message::Event(EventMessage::SleepTimeSaved(
                event.id.clone(),
                scope,
                Ok(event.clone()),
            )),
        );
        assert_eq!(
            sleep::times_for(&state.config.calendar, day - Duration::days(7)),
            before
        );
        let expected = if scope == calendar::SaveScope::CurrentAndFuture {
            sleep::Times {
                bedtime: event.start.time(),
                wakeup: event.end.time(),
            }
        } else {
            before
        };
        assert_eq!(sleep::times_for(&state.config.calendar, day), expected);
        assert_eq!(
            sleep::times_for(&state.config.calendar, day + Duration::days(7)),
            expected
        );
        assert_eq!(state.config.calendar.sleep_schedule[0], expected);
        assert_eq!(
            sleep::times_for(&state.config.calendar, day + Duration::days(1)),
            sleep::Times::default()
        );
        assert_eq!(
            state
                .calendar
                .event("sleep")
                .unwrap()
                .details
                .sleep
                .as_ref()
                .unwrap()
                .automatic,
            scope == calendar::SaveScope::CurrentAndFuture
        );
        let saved: Config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
        assert_eq!(sleep::times_for(&saved.calendar, day), expected);
    }
}

#[test]
fn failed_sleep_drag_restores_times_and_leaves_the_schedule_unchanged() {
    let mut state = state();
    let original = state.calendar.event("sleep").unwrap().clone();
    let settings = state.config.calendar.clone();
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::EventTimeChanged {
            id: original.id.clone(),
            start: original.start + Duration::hours(1),
            end: original.end + Duration::hours(1),
        }),
    );
    let _ = app::update(&mut state, Message::Event(EventMessage::EventMoveCancelled));
    state.calendar.begin_time_change(
        "sleep",
        original.start + Duration::hours(1),
        original.end + Duration::hours(1),
    );
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::SleepTimeSaved(
            "sleep".into(),
            calendar::SaveScope::CurrentAndFuture,
            Err("offline".into()),
        )),
    );
    assert_eq!(state.calendar.event("sleep").unwrap(), &original);
    assert_eq!(state.config.calendar, settings);
}

#[test]
fn sleep_editor_time_changes_use_the_same_scope_prompt() {
    let mut state = state();
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened("sleep".into())),
    );
    let _ = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditEndChanged("09:30".into())),
    );
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::SleepSaveRequested),
    );
    assert!(
        state
            .calendar
            .pending_move
            .as_ref()
            .unwrap()
            .scope
            .is_none()
    );
    assert_eq!(
        state.calendar.pending_move.as_ref().unwrap().end,
        calendar::at_clock(state.calendar.day, 9, 30)
    );
    assert!(!state.calendar.edit_draft.as_ref().unwrap().saving);
}
