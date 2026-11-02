use iced::{Size, window};
use iced_test::{Simulator, core::Settings};
use timeslay::app::{CalendarMessage, SettingsMessage};
use timeslay::{
    app::{self, Message, Screen, State},
    calendar::{self, Calendar, Event, deadline},
    config::CalendarSettings,
};

const TASKS: &str = CalendarSettings::GOOGLE_TASKS_ID;

fn state() -> State {
    let mut state = State::blank();
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
fn regular_calendar_colors_survive_sync_and_reset_without_changing_event_colors() {
    let mut state = state();
    let original = state.calendar.event("standup").unwrap().color;
    let color = calendar::colors::calendar("24").unwrap();
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::CalendarColorPicked(
            "work".into(),
            Some(color),
        )),
    );
    let check = |state: &State| {
        assert_eq!(
            state
                .calendar
                .event("standup")
                .unwrap()
                .display_color(&state.config.calendar),
            color
        );
        assert_eq!(
            state
                .calendar
                .display_sources(&state.config.calendar)
                .iter()
                .find(|c| c.id == "work")
                .unwrap()
                .color,
            color
        );
    };
    check(&state);
    // Preferences persist, while raw event colors remain available for reset.
    state.config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
    let snapshot = calendar::sample_snapshot();
    state.calendar.day = snapshot.day;
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarSynced(Ok(snapshot))),
    );
    check(&state);
    let mut explicit = state.calendar.event("standup").unwrap().clone();
    explicit.details.color_id = Some("4".into());
    explicit.color = calendar::colors::event("4").unwrap();
    assert_eq!(
        explicit.display_color(&state.config.calendar),
        explicit.color
    );
    state.config.calendar.app_calendar = Some("work".into());
    assert_eq!(
        state
            .calendar
            .event("standup")
            .unwrap()
            .display_color(&state.config.calendar),
        original
    );
    state.config.calendar.app_calendar = None;
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::CalendarColorPicked("work".into(), None)),
    );
    assert_eq!(
        state
            .calendar
            .event("standup")
            .unwrap()
            .display_color(&state.config.calendar),
        original
    );
    assert!(!state.config.calendar.calendar_colors.contains_key("work"));
}

#[test]
fn regular_calendar_menu_shows_a_picker_at_narrow_widths() {
    let mut state = state();
    state.calendar.menu_open = true;
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::SourceColorEditorToggled("work".into())),
    );
    for width in [180.0, 200.0, 420.0] {
        state.main_size = Size::new(width, 4000.0);
        let mut ui = Simulator::with_size(
            Settings::default(),
            Size::new(width, 4000.0),
            app::view(&state, window::Id::unique()),
        );
        assert!(ui.find("Calendar color in TimeSlay").is_ok());
        ui.click("Reset to default").unwrap();
        assert!(ui.into_messages().any(
            |message| matches!(message, Message::Settings(SettingsMessage::CalendarColorPicked(id, None)) if id == "work")
        ));
    }
}

#[test]
fn ordering_individual_lists_retains_hidden_sources_and_survives_restart_and_sync() {
    let mut state = state();
    state.config.calendar.disabled = vec!["work".into()];
    state.config.calendar.show_tasks = false;
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarSourceMoved("work".into(), true)),
    );
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarSourceMoved(
            "tasks:one".into(),
            true,
        )),
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
        [
            "work",
            "personal",
            "tasks:one",
            "tasks",
            "tasks:two",
            "timeslay:sleep"
        ]
    );
    let saved = toml::to_string(&state.config).unwrap();
    state.config = toml::from_str(&saved).unwrap();
    state.calendar.calendars.reverse();
    assert_eq!(
        ids(&state),
        [
            "work",
            "personal",
            "tasks:one",
            "tasks",
            "tasks:two",
            "timeslay:sleep"
        ]
    );
    let order = state.config.calendar.source_order.clone();
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarSourceMoved("work".into(), true)),
    );
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarSourceMoved(
            "timeslay:sleep".into(),
            false,
        )),
    );
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarSourceMoved(
            "missing".into(),
            false,
        )),
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
            Message::Calendar(CalendarMessage::CalendarSourceMoved(id, false)) if id == "personal")));
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
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::ShowTasksToggled(false)),
    );
    assert!(tasks(&state).is_empty());
    assert_eq!(
        state
            .calendar
            .visible_events(&state.config.calendar)
            .count(),
        5
    );
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::ShowTasksToggled(true)),
    );
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
        Message::Settings(SettingsMessage::TaskColorPicked(
            TASKS.into(),
            Some(iced::Color::BLACK),
        )),
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
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::TaskColorPicked(TASKS.into(), None)),
    );
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
        Message::Settings(SettingsMessage::TaskColorPicked(
            "tasks:one".into(),
            Some(iced::Color::BLACK),
        )),
    );
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::TaskColorPicked(
            "tasks:two".into(),
            Some(iced::Color::WHITE),
        )),
    );
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarToggled("tasks:one".into(), false)),
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
        Message::Settings(SettingsMessage::TaskColorPicked("tasks:one".into(), None)),
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
        Message::Calendar(CalendarMessage::CalendarToggled("tasks:one".into(), true)),
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
        [
            "tasks:one",
            "tasks:two",
            "personal",
            "work",
            "tasks",
            "timeslay:sleep"
        ]
    );
}

#[test]
fn menu_groups_google_sources_before_all_timeslay_types() {
    let mut state = state();
    state
        .calendar
        .calendars
        .iter_mut()
        .find(|source| source.id == "tasks")
        .unwrap()
        .name = "Legacy tasks calendar".into();
    state.config.calendar.app_calendar = Some("app".into());
    // Exclude by remembered ID even if renamed, and recognize legacy names.
    for (id, name) in [("app", "Renamed app calendar"), ("old", "TimeSlayer")] {
        state.calendar.calendars.push(Calendar {
            id: id.into(),
            name: name.into(),
            color: calendar::colors::TIME_BLOCK,
            primary: false,
            selected_by_google: true,
        });
    }
    state.calendar.menu_open = true;
    for width in [180.0, 200.0, 420.0] {
        state.main_size = Size::new(width, 4000.0);
        let mut ui = Simulator::with_size(
            Settings::default(),
            state.main_size,
            timeslay::ui::calendar::view(
                &state,
                &timeslay::ui::layout::Metrics::of(state.main_size),
            ),
        );
        let calendars_y = ui.find("Google Calendars").unwrap().bounds().y;
        let tasks_y = ui.find("Google Task Lists").unwrap().bounds().y;
        let timeslay_y = ui.find("TimeSlay").unwrap().bounds().y;
        assert!(calendars_y < tasks_y && tasks_y < timeslay_y);
        for label in ["Personal", "Work"] {
            let y = ui.find(label).unwrap().bounds().y;
            assert!(y > calendars_y && y < tasks_y);
        }
        for label in ["First task list", "Second task list"] {
            let y = ui.find(label).unwrap().bounds().y;
            assert!(y > tasks_y && y < timeslay_y);
        }
        for source in calendar::sources::TimeSlaySource::ALL {
            let bounds = ui.find(source.label()).unwrap().bounds();
            assert!(
                bounds.y > timeslay_y,
                "{} is above TimeSlay at {width}px",
                source.label()
            );
            assert!(bounds.x >= 0.0 && bounds.x + bounds.width <= width);
            // Click the toggle immediately to the right of the label.
            ui.point_at(iced::Point::new(
                bounds.x + bounds.width + 20.0,
                bounds.center_y(),
            ));
            ui.simulate(iced_test::simulator::click());
        }
        assert!(ui.find("Renamed app calendar").is_err());
        assert!(ui.find("TimeSlayer").is_err());
        let messages: Vec<_> = ui.into_messages().collect();
        for source in calendar::sources::TimeSlaySource::ALL {
            assert!(
                messages.iter().any(|message| matches!(message,
                    Message::Calendar(CalendarMessage::CalendarToggled(id, false)) if id == source.id()
                )),
                "missing toggle for {} at {width}px",
                source.label()
            );
        }
    }
}

#[test]
fn timeslay_colors_are_chosen_from_the_menu_and_not_from_settings() {
    use calendar::sources::TimeSlaySource;

    let mut state = state();
    state.config.calendar.app_calendar = Some("app".into());
    let base = state.calendar.events[0].clone();
    state.calendar.events.clear();
    for source in TimeSlaySource::ALL {
        let mut event = Event {
            id: source.id().into(),
            calendar_id: "app".into(),
            all_day: source == TimeSlaySource::Events,
            details: Default::default(),
            ..base.clone()
        };
        match source {
            TimeSlaySource::Habits => event.details.habit = true,
            TimeSlaySource::Sleep => {
                event.details.sleep = Some(calendar::sleep::Record::planned(state.calendar.day));
            }
            TimeSlaySource::Tasks => {
                event.id = "task:scheduled".into();
                event.calendar_id = "tasks:one".into();
                event.details.scheduled =
                    Some(calendar::scheduled::Link::new("app".into(), &event).unwrap());
            }
            _ => {}
        }
        state.calendar.events.push(event);
    }

    let palette = calendar::colors::palette();
    for (index, source) in TimeSlaySource::ALL.into_iter().enumerate() {
        let color = palette[index];
        let _ = app::update(
            &mut state,
            Message::Settings(SettingsMessage::TimeSlayColorPicked(
                source.id().into(),
                Some(color),
            )),
        );
        assert_eq!(source.custom_color(&state.config.calendar), Some(color));
        assert_eq!(source.color(&state.config.calendar), color);
        let event = state
            .calendar
            .events
            .iter()
            .find(|event| TimeSlaySource::of(event, "app") == Some(source))
            .unwrap();
        assert_eq!(
            event.display_color(&state.config.calendar),
            color,
            "{} was not recolored",
            source.label()
        );
    }

    // The menu swatch opens the same picker as a Google source, and resetting
    // it clears the custom color.
    state.calendar.menu_open = true;
    for source in TimeSlaySource::ALL {
        state.calendar.color_editor = Some(source.id().into());
        state.main_size = Size::new(420.0, 4000.0);
        let mut ui = Simulator::with_size(
            Settings::default(),
            state.main_size,
            timeslay::ui::calendar::view(
                &state,
                &timeslay::ui::layout::Metrics::of(state.main_size),
            ),
        );
        let title = format!("Color for {}", source.label());
        assert!(ui.find(title.as_str()).is_ok(), "no picker for {title}");
        ui.click("Reset to default").unwrap();
        assert!(ui.into_messages().any(|message| matches!(message,
            Message::Settings(SettingsMessage::TimeSlayColorPicked(id, None)) if id == source.id()
        )));
        let _ = app::update(
            &mut state,
            Message::Settings(SettingsMessage::TimeSlayColorPicked(
                source.id().into(),
                None,
            )),
        );
        assert_eq!(source.custom_color(&state.config.calendar), None);
    }

    // The old settings section is gone; colors live in the calendar menu.
    state.calendar.menu_open = false;
    state.screen = Screen::Settings;
    let mut ui = Simulator::with_size(
        Settings::default(),
        Size::new(420.0, 4000.0),
        app::view(&state, window::Id::unique()),
    );
    assert!(ui.find("TimeSlay colors").is_err());
}

#[test]
fn timeslay_type_toggles_filter_independently_and_persist_across_sync() {
    use calendar::sources::TimeSlaySource;
    let mut state = state();
    state.config.calendar.app_calendar = Some("app".into());
    let base = state.calendar.events[0].clone();
    state.calendar.events.clear();
    for source in TimeSlaySource::ALL {
        let mut event = Event {
            id: source.id().into(),
            calendar_id: "app".into(),
            all_day: source == TimeSlaySource::Events,
            details: Default::default(),
            ..base.clone()
        };
        match source {
            TimeSlaySource::Habits => event.details.habit = true,
            TimeSlaySource::Sleep => {
                event.details.sleep = Some(calendar::sleep::Record::planned(state.calendar.day));
            }
            TimeSlaySource::Tasks => {
                event.id = "task:scheduled".into();
                event.calendar_id = "tasks:one".into();
                event.details.scheduled =
                    Some(calendar::scheduled::Link::new("app".into(), &event).unwrap());
            }
            _ => {}
        }
        state.calendar.events.push(event);
    }
    // A task and an event on other Google sources must remain independent.
    state.calendar.events.push(base.clone());
    state.calendar.events.push(Event {
        id: "task:external".into(),
        calendar_id: "tasks:two".into(),
        ..base
    });
    let original = state.calendar.events.clone();
    for source in TimeSlaySource::ALL {
        let _ = app::update(
            &mut state,
            Message::Calendar(CalendarMessage::CalendarToggled(source.id().into(), false)),
        );
        state.config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
        let mut snapshot = calendar::sample_snapshot();
        snapshot.events = original.clone();
        state.calendar.apply_snapshot(snapshot);
        let visible: Vec<_> = state
            .calendar
            .visible_events(&state.config.calendar)
            .collect();
        assert_eq!(visible.len(), original.len() - 1, "{}", source.label());
        assert!(
            !visible
                .iter()
                .any(|event| TimeSlaySource::of(event, "app") == Some(source))
        );
        // Check the grid and agenda filtering as well as the shared iterator.
        for event in state
            .calendar
            .timed_events_on(state.calendar.day, &state.config.calendar)
            .into_iter()
            .chain(
                state
                    .calendar
                    .task_list_events_on(state.calendar.day, &state.config.calendar),
            )
            .chain(
                state
                    .calendar
                    .all_day_events_on(state.calendar.day, &state.config.calendar),
            )
        {
            assert_ne!(TimeSlaySource::of(event, "app"), Some(source));
        }
        let _ = app::update(
            &mut state,
            Message::Calendar(CalendarMessage::CalendarToggled(source.id().into(), true)),
        );
        assert_eq!(
            state
                .calendar
                .visible_events(&state.config.calendar)
                .count(),
            original.len()
        );
    }
}

#[test]
fn opening_the_grouped_menu_preserves_the_old_timeslay_calendar_hide() {
    use calendar::sources::TimeSlaySource;
    let mut state = state();
    state.config.calendar.app_calendar = Some("app".into());
    state.config.calendar.disabled = vec!["app".into()];
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarMenuToggled),
    );
    assert!(!state.config.calendar.disabled.iter().any(|id| id == "app"));
    for source in TimeSlaySource::ALL {
        assert_eq!(
            state
                .calendar
                .calendar_enabled(source.id(), &state.config.calendar),
            source == TimeSlaySource::Sleep,
        );
    }
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarToggled(
            TimeSlaySource::Habits.id().into(),
            true,
        )),
    );
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarMenuToggled),
    );
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarMenuToggled),
    );
    assert!(
        state
            .calendar
            .calendar_enabled(TimeSlaySource::Habits.id(), &state.config.calendar)
    );
    assert!(
        !state
            .calendar
            .calendar_enabled(TimeSlaySource::TimeBlocks.id(), &state.config.calendar)
    );
}
