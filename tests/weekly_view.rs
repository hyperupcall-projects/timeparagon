use chrono::{Duration, Local, NaiveDate};
use iced::futures::{StreamExt, executor::block_on};
use iced::{Element, Size};
use iced_test::{Error, Simulator, core::Settings};
use timetracker::{
    app::{self, Message, Screen, State},
    calendar::{self, Store},
    config::Config,
    theme::Catalog,
};

fn date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
}

fn at(day: u32, hour: u32) -> chrono::DateTime<Local> {
    date(day)
        .and_hms_opt(hour, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .single()
        .unwrap()
}

fn state() -> State {
    let mut state = State::new(Config::default(), Catalog::built_in());
    let mut snapshot = calendar::sample_snapshot();
    snapshot.day = date(15);
    let mut block = snapshot
        .events
        .iter()
        .find(|event| event.id == "standup")
        .unwrap()
        .clone();
    block.start = at(14, 9);
    block.end = at(14, 10);
    block.title = "Wednesday block".into();
    let mut task = block.clone();
    task.id = "task:thursday".into();
    task.calendar_id = "tasks:list".into();
    task.title = "Thursday task".into();
    task.start = at(15, 9);
    task.end = at(15, 10);
    let mut final_event = block.clone();
    final_event.id = "last-event".into();
    final_event.calendar_id = "personal".into();
    final_event.title = "Last day event".into();
    final_event.start = at(22, 9);
    final_event.end = at(22, 10);
    let mut spanning = block.clone();
    spanning.id = "spanning".into();
    spanning.title = "Multi-day event".into();
    spanning.all_day = true;
    spanning.start = at(16, 0);
    spanning.end = at(19, 0);
    snapshot.events = vec![block, task, final_event, spanning];
    state.config.calendar.time_block_calendar = Some("work".into());
    state.calendar = Store::with_snapshot(calendar::Snapshot {
        events: Vec::new(),
        ..snapshot.clone()
    });
    state.weekly = true;
    state.week_anchor = date(15);
    state
        .calendar
        .apply_week_snapshot(snapshot, &calendar::week_dates(date(15), 8));
    state
}

fn render(state: &State, size: Size) -> Simulator<'_, Message> {
    let view: Element<'_, Message> = app::view(state, iced::window::Id::unique());
    Simulator::with_size(Settings::default(), size, view)
}

#[test]
fn week_range_has_previous_day_and_seven_or_eight_selected_days() {
    let eight = calendar::week_dates(date(15), 8);
    assert_eq!(eight, (14..=22).map(date).collect::<Vec<_>>());
    let seven = calendar::week_dates(date(15), 7);
    assert_eq!(seven, (14..=21).map(date).collect::<Vec<_>>());
    let new_year = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
    assert_eq!(
        calendar::week_dates(new_year, 7)[0],
        new_year - Duration::days(1)
    );
}

#[test]
fn expanded_window_fits_every_weekly_heading() {
    use iced_test::runtime::{Action, task, window::Action as WindowAction};
    use timetracker::{app::Sidebar, panel::Side};

    for (days, width, sidebar) in [(7, 100, false), (8, 200, false), (8, 150, true)] {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.config.calendar.weekly_days = days;
        state.config.calendar.weekly_day_width = width;
        state.calendar.set_day(date(15), date(15));
        if sidebar {
            state.sidebar = Some(Sidebar {
                width: 200.0,
                side: Side::Left,
            });
        }
        state.main_window = Some(iced::window::Id::unique());
        let task = app::update(&mut state, Message::CalendarModeSelected(true));
        let size = block_on(async {
            let mut stream = task::into_stream(task).unwrap();
            let mut size = None;
            while let Some(action) = stream.next().await {
                match action {
                    Action::Window(WindowAction::Resize(_, next)) => size = Some(next),
                    Action::Window(WindowAction::GetMonitorSize(_, sender)) => {
                        let _ = sender.send(None);
                    }
                    _ => {}
                }
            }
            size.unwrap()
        });
        let size = Size::new(size.width, 700.0);
        let mut ui = render(&state, size);
        for day in calendar::week_dates(date(15), days) {
            let heading = day.format("%a %b %-d").to_string();
            let bounds = ui.find(heading.as_str()).unwrap().bounds();
            assert!(
                bounds.x >= 0.0 && bounds.x + bounds.width <= size.width + 0.5,
                "{heading} at {bounds:?} is outside {size:?}"
            );
        }
    }
}

#[test]
fn weekly_settings_round_trip_and_clamp_old_or_invalid_values() {
    let defaults: timetracker::config::CalendarSettings = toml::from_str("").unwrap();
    assert_eq!(defaults.weekly_days(), 8);
    assert_eq!(defaults.weekly_day_width(), 150);
    let mut settings = defaults;
    settings.weekly_days = 7;
    settings.weekly_day_width = 220;
    let saved = toml::to_string(&settings).unwrap();
    let loaded: timetracker::config::CalendarSettings = toml::from_str(&saved).unwrap();
    assert_eq!((loaded.weekly_days(), loaded.weekly_day_width()), (7, 220));
    settings.weekly_days = 99;
    settings.weekly_day_width = 0;
    assert_eq!(
        (settings.weekly_days(), settings.weekly_day_width()),
        (8, 100)
    );
}

#[test]
fn range_snapshot_partitions_events_and_keeps_tasks_available() {
    let state = state();
    let events = |day| state.calendar.events_on(date(day));
    assert!(events(14).iter().any(|e| e.id == "standup"));
    assert!(!events(15).iter().any(|e| e.id == "standup"));
    assert!(events(22).iter().any(|e| e.id == "last-event"));
    assert!(events(14).iter().any(|e| e.id == "task:thursday"));
    for day in 16..=18 {
        assert!(
            state
                .calendar
                .all_day_events_on(date(day), &state.config.calendar)
                .any(|e| e.id == "spanning")
        );
    }
    assert!(!events(19).iter().any(|e| e.id == "spanning"));
}

#[test]
fn weekly_day_interactions_open_correct_panes_without_moving_the_week() {
    let mut state = state();
    let _ = app::update(
        &mut state,
        Message::WeekDayAction(date(14), Box::new(Message::EventOpened("standup".into()))),
    );
    assert_eq!(state.week_anchor, date(15));
    assert_eq!(state.calendar.day, date(14));
    assert_eq!(state.timeblocking.block.as_deref(), Some("standup"));
    assert_eq!(state.timeblocking.original_width, None);
    assert!(!state.details_pane_open);
    let mut ui = render(&state, Size::new(1500.0, 700.0));
    let selected = ui.find("Wed Oct 14").unwrap().bounds();
    let pane = ui.find("Planned Tasks").unwrap().bounds();
    assert!(
        pane.x >= selected.x + selected.width - 1.0,
        "selected {selected:?}, pane {pane:?}"
    );
    drop(ui);

    let _ = app::update(
        &mut state,
        Message::WeekDayAction(
            date(15),
            Box::new(Message::EventOpened("task:thursday".into())),
        ),
    );
    assert_eq!(state.calendar.day, date(15));
    assert_eq!(state.week_anchor, date(15));
    assert_eq!(state.calendar.open_event.as_deref(), Some("task:thursday"));
    assert!(state.timeblocking.block.is_none());
    assert!(!state.details_pane_open);
    let mut ui = render(&state, Size::new(1500.0, 700.0));
    let pane = ui.find("Complete task").unwrap().bounds();
    assert!(pane.x >= 14.0 + 2.0 * 150.0, "pane {pane:?}");
    let other_day = ui.find("Wed Oct 14").unwrap().bounds().center();
    ui.point_at(other_day);
    ui.simulate(iced_test::simulator::click());
    assert!(
        ui.into_messages()
            .any(|message| matches!(message, Message::EventClosed))
    );
    let mut ui = render(&state, Size::new(328.0, 900.0));
    let pane = ui.find("Complete task").unwrap().bounds();
    assert!(pane.x + pane.width <= 14.0 + 150.0 + 1.0, "pane {pane:?}");
    drop(ui);

    let _ = app::update(
        &mut state,
        Message::WeekDayAction(
            date(22),
            Box::new(Message::EventOpened("last-event".into())),
        ),
    );
    let mut ui = render(&state, Size::new(1500.0, 700.0));
    let selected = ui.find("Thu Oct 22").unwrap().bounds();
    let pane = ui.find("Last day event").unwrap().bounds();
    assert!(pane.x + pane.width <= selected.x + 1.0);
}

#[test]
fn weekly_navigation_uses_the_anchor_not_the_last_opened_day() {
    let mut state = state();
    let _ = app::update(
        &mut state,
        Message::WeekDayAction(date(14), Box::new(Message::EventOpened("standup".into()))),
    );
    let _ = app::update(&mut state, Message::CalendarDayShift(8));
    assert_eq!(state.week_anchor, date(23));
    assert!(state.timeblocking.block.is_none());
}

#[test]
fn weekly_task_writes_keep_other_days_visible_and_update_cached_copies() {
    let mut state = state();
    let mut task = state.calendar.event("task:thursday").unwrap().clone();
    task.title = "Renamed task".into();
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(timetracker::timeblocking::Action::Saved(
            "task:thursday".into(),
            Ok(task.clone()),
        )),
    );
    assert!(
        state
            .calendar
            .events_on(date(14))
            .iter()
            .any(|e| e.id == "standup")
    );
    assert!(
        state
            .calendar
            .events_on(date(14))
            .iter()
            .any(|e| e.title == "Renamed task")
    );
    assert!(
        state
            .calendar
            .events_on(date(22))
            .iter()
            .any(|e| e.id == "last-event")
    );

    task.completed = true;
    let _ = app::update(
        &mut state,
        Message::TaskCompletionSaved(
            "task:thursday".into(),
            timetracker::timeblocking::TaskStatus::Active,
            Ok(task),
        ),
    );
    assert!(
        state
            .calendar
            .events_on(date(14))
            .iter()
            .any(|e| e.id == "task:thursday" && e.completed)
    );
    let _ = app::update(
        &mut state,
        Message::TaskDeleted("task:thursday".into(), Ok(())),
    );
    assert!(
        !state
            .calendar
            .events_on(date(14))
            .iter()
            .any(|e| e.id == "task:thursday")
    );
    assert!(
        state
            .calendar
            .events_on(date(14))
            .iter()
            .any(|e| e.id == "standup")
    );
    assert!(
        state
            .calendar
            .events_on(date(22))
            .iter()
            .any(|e| e.id == "last-event")
    );
}

#[test]
fn weekly_canvas_click_reports_the_clicked_day() -> Result<(), Error> {
    let state = state();
    let mut ui = render(&state, Size::new(1500.0, 900.0));
    let heading = ui.find("Wed Oct 14")?.bounds();
    let point = iced::Point::new(
        14.0 + 32.0 + 20.0,
        heading.y + heading.height + 110.0 + 9.0 * 48.0 + 30.0,
    );
    ui.point_at(point);
    ui.simulate(iced_test::simulator::click());
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(messages.iter().any(|message| matches!(
        message,
        Message::WeekDayAction(day, action)
            if *day == date(14) && matches!(action.as_ref(), Message::EventOpened(id) if id == "standup")
    )), "click produced {messages:?}");
    Ok(())
}

#[test]
fn weekly_all_day_task_opens_the_task_for_its_column() -> Result<(), Error> {
    let mut state = state();
    state.calendar.event_mut("task:thursday").unwrap().all_day = true;
    state.config.calendar.weekly_day_width = 100;
    let mut ui = render(&state, Size::new(1500.0, 900.0));
    let _ = ui.click("Thursday task")?;
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::WeekDayAction(day, action)
            if day == date(15) && matches!(*action, Message::EventOpened(ref id) if id == "task:thursday")
    )));
    Ok(())
}

#[test]
fn narrow_weekly_block_panel_keeps_task_and_actions_clickable() -> Result<(), Error> {
    let mut state = state();
    state.calendar.set_day(date(14), date(15));
    let task = state.calendar.event_mut("task:thursday").unwrap();
    task.details.task_data = Some(timetracker::task_db::TaskData {
        id: 1,
        metadata: timetracker::timeblocking::Metadata {
            block: Some("work/standup".into()),
            ..Default::default()
        },
        legacy_key: None,
    });
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    let mut ui = render(&state, Size::new(328.0, 900.0));
    let _ = ui.click("Thursday task")?;
    assert!(ui.into_messages().any(|message| matches!(
        message, Message::EventOpened(id) if id == "task:thursday"
    )));
    let mut ui = render(&state, Size::new(328.0, 900.0));
    let _ = ui.click("Close")?;
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::TimeBlocking(timetracker::timeblocking::Action::Close)
    )));
    Ok(())
}

#[test]
fn weekly_picker_emits_hover_and_selection_and_tabs_remain_reachable() -> Result<(), Error> {
    let mut state = state();
    state.calendar.month_open = true;
    state.calendar.month = date(1);
    let mut ui = render(&state, Size::new(900.0, 700.0));
    let point = ui.find("15")?.bounds().center();
    ui.point_at(point);
    ui.simulate([iced::Event::Mouse(iced::mouse::Event::CursorMoved {
        position: point,
    })]);
    let _ = ui.click("15")?;
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::WeekHover(Some(day)) if *day == date(15)))
    );
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::CalendarDaySelected(day) if *day == date(15)))
    );

    state.calendar.month_open = false;
    for width in [180.0, 260.0, 900.0] {
        let mut ui = render(&state, Size::new(width, 700.0));
        let weekly = ui.find("Weekly")?.bounds();
        assert!(weekly.x >= 0.0 && weekly.x + weekly.width <= width);
        let _ = ui.click("Weekly")?;
        assert!(
            ui.into_messages()
                .any(|m| matches!(m, Message::CalendarModeSelected(true)))
        );
    }
    let mut ui = render(&state, Size::new(180.0, 240.0));
    let heading = ui.find("Wed Oct 14")?.bounds();
    assert!(heading.y + heading.height < 240.0, "heading {heading:?}");
    drop(ui);
    state.screen = Screen::Settings;
    let mut ui = render(&state, Size::new(900.0, 1600.0));
    assert!(ui.find("Weekly days: 8 (+ previous day)").is_ok());
    assert!(ui.find("Weekly day width: 150 px").is_ok());
    Ok(())
}
