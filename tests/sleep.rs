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
        "Sleep start",
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
fn sleep_goal_defaults_to_eight_is_bounded_and_persists() {
    let mut state = state();
    assert_eq!(state.config.calendar.sleep_goal_hours(), 8);
    for (input, expected) in [(0, 6), (9, 9), (99, 12)] {
        let _ = app::update(
            &mut state,
            Message::Settings(SettingsMessage::SleepGoalChanged(input)),
        );
        assert_eq!(state.config.calendar.sleep_goal_hours(), expected);
        let saved: Config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
        assert_eq!(saved.calendar.sleep_goal_hours(), expected);
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
fn switching_to_a_cached_day_without_sleep_requests_a_calculation() {
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
    assert_eq!(state.calendar.sync, calendar::SyncState::Syncing);
}
