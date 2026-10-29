use iced::{Size, window};
use iced_test::{Simulator, core::Settings};
use time_slay::{
    app::{self, Message, Screen, State},
    calendar::{self, Calendar, Event, deadline},
    config::{CalendarSettings, Config},
    theme::Catalog,
};

const TASKS: &str = CalendarSettings::GOOGLE_TASKS_ID;

fn state() -> State {
    let mut state = State::new(Config::default(), Catalog::built_in());
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    for (id, name) in [
        ("tasks:one", "First task list"),
        ("tasks:two", "Second task list"),
    ] {
        state.calendar.calendars.push(Calendar {
            id: id.into(),
            name: name.into(),
            color: calendar::colors::TASK,
            primary: false,
            selected_by_google: true,
        });
    }
    state
}

#[test]
fn ordering_individual_lists_retains_hidden_sources_and_survives_restart_and_sync() {
    let mut state = state();
    state.config.calendar.disabled = vec!["work".into()];
    state.config.calendar.show_tasks = false;
    let _ = app::update(
        &mut state,
        Message::CalendarSourceMoved("work".into(), true),
    );
    let _ = app::update(
        &mut state,
        Message::CalendarSourceMoved("tasks:one".into(), true),
    );
    let ids = |state: &State| {
        state
            .calendar
            .display_sources(&state.config.calendar)
            .into_iter()
            .map(|source| source.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(&state),
        ["work", "personal", "tasks:one", "tasks", "tasks:two"]
    );
    let saved = toml::to_string(&state.config).unwrap();
    state.config = toml::from_str(&saved).unwrap();
    state.calendar.calendars.reverse();
    assert_eq!(
        ids(&state),
        ["work", "personal", "tasks:one", "tasks", "tasks:two"]
    );
    let order = state.config.calendar.source_order.clone();
    let _ = app::update(
        &mut state,
        Message::CalendarSourceMoved("work".into(), true),
    );
    let _ = app::update(
        &mut state,
        Message::CalendarSourceMoved("tasks:two".into(), false),
    );
    let _ = app::update(
        &mut state,
        Message::CalendarSourceMoved("missing".into(), false),
    );
    assert_eq!(state.config.calendar.source_order, order);
}

#[test]
fn menu_and_settings_offer_individual_task_lists_and_work_at_narrow_widths() {
    let mut state = state();
    state.calendar.menu_open = true;
    for width in [180.0, 200.0, 420.0] {
        let mut ui = Simulator::with_size(
            Settings::default(),
            Size::new(width, 4000.0),
            app::view(&state, window::Id::unique()),
        );
        assert!(ui.find("Google Tasks").is_err());
        assert!(ui.find("Show Google Tasks").is_err());
        assert!(ui.find("First task list").is_ok());
        assert!(ui.find("Second task list").is_ok());
    }
    state.screen = Screen::Settings;
    state.calendar.menu_open = false;
    for width in [180.0, 200.0, 420.0] {
        let mut ui = Simulator::with_size(
            Settings::default(),
            Size::new(width, 4000.0),
            app::view(&state, window::Id::unique()),
        );
        for label in [
            "Calendar view order",
            "Personal",
            "Work",
            "Tasks",
            "First task list",
            "Second task list",
            "Up",
            "Down",
        ] {
            let bounds = ui.find(label).unwrap().bounds();
            assert!(
                bounds.x >= 0.0 && bounds.x + bounds.width <= width,
                "{label} is outside the window at {width}px"
            );
        }
        ui.click("Down").unwrap();
        assert!(ui.into_messages().any(|message| matches!(message,
            Message::CalendarSourceMoved(id, false) if id == "personal")));
    }
}

#[test]
fn aggregate_toggle_controls_dated_and_timed_tasks_across_lists_without_changing_metadata() {
    let mut state = state();
    let base = state.calendar.events[0].clone();
    for (id, list, notes) in [
        ("due-time", "tasks:one", "DEADLINE: 2026-10-30 17:00\nNotes"),
        ("day-only", "tasks:two", "DUE: 2026-10-30\nNotes"),
    ] {
        let mut task = Event {
            id: format!("task:{id}"),
            calendar_id: list.into(),
            title: id.into(),
            ..base.clone()
        };
        task.details.description = notes.into();
        state.calendar.events.push(task);
    }
    // Give the first task a real Calendar-backed time; a task is on the grid
    // only when its schedule says so, never because of its notes.
    let day = state.calendar.day;
    let first = state
        .calendar
        .events
        .iter_mut()
        .find(|event| event.id == "task:due-time")
        .unwrap();
    first.all_day = false;
    first.start = day
        .and_hms_opt(9, 30, 0)
        .unwrap()
        .and_local_timezone(chrono::Local)
        .unwrap();
    first.end = first.start + chrono::Duration::minutes(45);

    let tasks = |state: &State| {
        state
            .calendar
            .visible_events(&state.config.calendar)
            .filter(|event| event.is_task())
            .cloned()
            .collect::<Vec<_>>()
    };
    let original = tasks(&state);
    assert_eq!(original.len(), 2);
    assert!(!original[0].all_day);
    assert!(
        deadline::read_deadline(&original[0].details.description)
            .unwrap()
            .is_some()
    );
    let _ = app::update(&mut state, Message::ShowTasksToggled(false));
    assert!(tasks(&state).is_empty());
    assert_eq!(
        state
            .calendar
            .visible_events(&state.config.calendar)
            .count(),
        5
    );
    let _ = app::update(&mut state, Message::ShowTasksToggled(true));
    assert_eq!(tasks(&state), original);
    assert_eq!(original[0].source_id(), "tasks:one");
    assert_eq!(original[1].source_id(), "tasks:two");
}

#[test]
fn aggregate_task_color_applies_to_all_lists_and_can_be_reset() {
    let mut state = state();
    state
        .config
        .calendar
        .set_task_color("tasks:one", Some(iced::Color::WHITE));
    let _ = app::update(
        &mut state,
        Message::TaskColorPicked(TASKS.into(), Some(iced::Color::BLACK)),
    );
    for list in ["tasks:one", "tasks:two"] {
        assert_eq!(
            state.config.calendar.task_color(list),
            Some(iced::Color::BLACK)
        );
        assert_eq!(
            state
                .calendar
                .calendars
                .iter()
                .find(|c| c.id == list)
                .unwrap()
                .color,
            iced::Color::BLACK
        );
    }
    let _ = app::update(&mut state, Message::TaskColorPicked(TASKS.into(), None));
    assert!(state.config.calendar.task_colors.is_empty());
    assert_eq!(
        state
            .calendar
            .calendars
            .iter()
            .find(|c| c.id == "tasks:one")
            .unwrap()
            .color,
        calendar::colors::TASK
    );
}

#[test]
fn managed_lists_are_omitted_by_name_and_configured_id() {
    let mut state = state();
    for (id, name) in [
        ("inbox", " Inbox "),
        ("triaged", "triaged"),
        ("blocked", "Time Blocked"),
        ("backlog", "Backlog"),
        ("custom-inbox", "Capture"),
        ("custom-triaged", "Reviewed"),
        ("custom-blocked", "Planned"),
        ("custom-backlog", "Later"),
    ] {
        state.calendar.calendars.push(Calendar {
            id: format!("tasks:{id}"),
            name: name.into(),
            color: calendar::colors::TASK,
            primary: false,
            selected_by_google: true,
        });
    }
    state.config.calendar.inbox_list = Some("custom-inbox".into());
    state.config.calendar.triaged_list = Some("tasks:custom-triaged".into());
    state.config.calendar.time_block_list = Some("custom-blocked".into());
    state.config.calendar.backlog_list = Some("tasks:custom-backlog".into());
    let sources = state.calendar.display_sources(&state.config.calendar);
    let lists: Vec<_> = sources
        .iter()
        .filter(|c| c.is_task_list())
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(lists, ["tasks:one", "tasks:two"]);
    state.calendar.menu_open = true;
    let mut ui = Simulator::with_size(
        Settings::default(),
        Size::new(420.0, 4000.0),
        app::view(&state, window::Id::unique()),
    );
    for name in [
        "Inbox",
        "triaged",
        "Time Blocked",
        "Backlog",
        "Capture",
        "Reviewed",
        "Planned",
        "Later",
    ] {
        assert!(
            ui.find(name).is_err(),
            "managed list {name} appears in menu"
        );
    }
}

#[test]
fn list_colors_and_visibility_are_independent_and_persist() {
    let mut state = state();
    let base = state.calendar.events[0].clone();
    for list in ["one", "two"] {
        state.calendar.events.push(Event {
            id: format!("task:{list}"),
            calendar_id: format!("tasks:{list}"),
            ..base.clone()
        });
    }
    let _ = app::update(
        &mut state,
        Message::TaskColorPicked("tasks:one".into(), Some(iced::Color::BLACK)),
    );
    let _ = app::update(
        &mut state,
        Message::TaskColorPicked("tasks:two".into(), Some(iced::Color::WHITE)),
    );
    let _ = app::update(
        &mut state,
        Message::CalendarToggled("tasks:one".into(), false),
    );
    state.config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
    state.calendar.apply_task_colors(&state.config.calendar);
    for (id, color) in [
        ("tasks:one", iced::Color::BLACK),
        ("tasks:two", iced::Color::WHITE),
    ] {
        assert_eq!(
            state
                .calendar
                .events
                .iter()
                .find(|e| e.calendar_id == id)
                .unwrap()
                .color,
            color
        );
        assert_eq!(
            state
                .calendar
                .display_sources(&state.config.calendar)
                .iter()
                .find(|c| c.id == id)
                .unwrap()
                .color,
            color
        );
    }
    let visible: Vec<_> = state
        .calendar
        .visible_events(&state.config.calendar)
        .filter(|e| e.is_task())
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(visible, ["task:two"]);
    let _ = app::update(
        &mut state,
        Message::TaskColorPicked("tasks:one".into(), None),
    );
    assert_eq!(state.config.calendar.task_color("tasks:one"), None);
    assert_eq!(
        state.config.calendar.task_color("tasks:two"),
        Some(iced::Color::WHITE)
    );
}

#[test]
fn enabling_a_list_after_a_legacy_global_hide_keeps_other_lists_hidden() {
    let mut state = state();
    state.config.calendar.show_tasks = false;
    let _ = app::update(
        &mut state,
        Message::CalendarToggled("tasks:one".into(), true),
    );
    assert!(
        state
            .calendar
            .calendar_enabled("tasks:one", &state.config.calendar)
    );
    assert!(
        !state
            .calendar
            .calendar_enabled("tasks:two", &state.config.calendar)
    );
}

#[test]
fn legacy_shared_source_order_expands_in_place() {
    let mut state = state();
    state.config.calendar.source_order = vec![
        TASKS.into(),
        "personal".into(),
        "work".into(),
        "tasks".into(),
    ];
    let sources: Vec<_> = state
        .calendar
        .display_sources(&state.config.calendar)
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(
        sources,
        ["tasks:one", "tasks:two", "personal", "work", "tasks"]
    );
}
