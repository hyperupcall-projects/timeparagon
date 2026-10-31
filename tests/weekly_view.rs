use chrono::{Duration, Local, NaiveDate};
use iced::futures::{StreamExt, executor::block_on};
use iced::{Element, Size};
use iced_test::{Error, Simulator, core::Settings};
use timeslay::{
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
    final_event.start = at(21, 9);
    final_event.end = at(21, 10);
    let mut spanning = block.clone();
    spanning.id = "spanning".into();
    spanning.title = "Multi-day event".into();
    spanning.all_day = true;
    spanning.start = at(16, 0);
    spanning.end = at(19, 0);
    snapshot.events = vec![block, task, final_event, spanning];
    state.config.calendar.app_calendar = Some("work".into());
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
    let view: Element<'_, Message> = if state.weekly || state.monthly {
        timeslay::ui::editor_window(state)
    } else {
        app::view(state, iced::window::Id::unique())
    };
    Simulator::with_size(Settings::default(), size, view)
}

fn render_main(state: &State, size: Size) -> Simulator<'_, Message> {
    Simulator::with_size(
        Settings::default(),
        size,
        app::view(state, iced::window::Id::unique()),
    )
}

#[test]
fn week_range_shows_seven_from_today_or_eight_from_yesterday() {
    let eight = calendar::week_dates(date(15), 8);
    assert_eq!(eight, (14..=21).map(date).collect::<Vec<_>>());
    let seven = calendar::week_dates(date(15), 7);
    assert_eq!(seven, (15..=21).map(date).collect::<Vec<_>>());
    let new_year = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
    assert_eq!(calendar::week_dates(new_year, 7)[0], new_year);
    assert_eq!(
        calendar::week_dates(new_year, 8)[0],
        new_year - Duration::days(1)
    );
}

#[test]
fn floating_window_fits_every_weekly_heading() {
    use iced_test::runtime::{Action, task, window::Action as WindowAction};
    use timeslay::{app::Sidebar, panel::Side};

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
                    Action::Window(WindowAction::Open(id, settings, sender)) => {
                        size = Some(settings.size);
                        let _ = sender.send(id);
                    }
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
    let defaults: timeslay::config::CalendarSettings = toml::from_str("").unwrap();
    assert_eq!(defaults.weekly_days(), 8);
    assert_eq!(defaults.weekly_day_width(), 150);
    let mut settings = defaults;
    settings.weekly_days = 7;
    settings.weekly_day_width = 220;
    let saved = toml::to_string(&settings).unwrap();
    let loaded: timeslay::config::CalendarSettings = toml::from_str(&saved).unwrap();
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
    assert!(events(21).iter().any(|e| e.id == "last-event"));
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
    assert!(state.editor_window.is_none());
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
            date(21),
            Box::new(Message::EventOpened("last-event".into())),
        ),
    );
    let mut ui = render(&state, Size::new(1500.0, 700.0));
    let selected = ui.find("Wed Oct 21").unwrap().bounds();
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
        Message::TimeBlocking(timeslay::timeblocking::Action::Saved(
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
            .events_on(date(21))
            .iter()
            .any(|e| e.id == "last-event")
    );

    task.completed = true;
    let _ = app::update(
        &mut state,
        Message::TaskCompletionSaved(
            "task:thursday".into(),
            timeslay::timeblocking::TaskStatus::Active,
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
            .events_on(date(21))
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
    assert!(
        messages.iter().any(|message| matches!(
            message,
            Message::WeekDayAction(day, action)
                if *day == date(14)
                    && matches!(
                        action.as_ref(),
                        Message::EventOpened(id) if id == "standup"
                    ) || *day == date(14) && matches!(
                        action.as_ref(),
                        Message::TimeBlocking(timeslay::timeblocking::Action::View(id)) if id == "standup"
                    )
        )),
        "click produced {messages:?}"
    );
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
    task.details.task_data = Some(timeslay::task_db::TaskData {
        id: 1,
        metadata: timeslay::timeblocking::Metadata {
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
        Message::TimeBlocking(timeslay::timeblocking::Action::Close)
    )));
    Ok(())
}

#[test]
fn overview_buttons_toggle_from_every_screen() -> Result<(), Error> {
    for width in [180.0, 260.0, 420.0, 900.0] {
        for screen in [
            Screen::Calendar,
            Screen::Overdue,
            Screen::Inbox,
            Screen::Triaged,
            Screen::Backlog,
            Screen::Projects,
            Screen::Notepad,
            Screen::Travel,
            Screen::Habits,
            Screen::KeepNotes,
            Screen::BrainDump,
            Screen::Settings,
        ] {
            for monthly in [false, true] {
                let mut state = state();
                state.weekly = false;
                state.monthly = false;
                state.screen = screen;
                let show = if monthly { "Show month" } else { "Show week" };
                let hide = if monthly { "Hide month" } else { "Hide week" };
                let mut ui = render_main(&state, Size::new(width, 900.0));
                let week = ui.find("Show week")?.bounds();
                let month = ui.find("Show month")?.bounds();
                let schedule = ui.find("Schedule")?.bounds();
                assert_eq!(week.height, 22.0);
                assert_eq!(month.height, 22.0);
                assert_eq!(week.y, month.y);
                assert!((week.width - month.width).abs() < 0.5);
                assert!(week.x + week.width <= month.x);
                assert!(month.x + month.width <= width);
                assert!(week.y + week.height <= schedule.y);
                ui.click(show)?;
                let message = ui
                    .into_messages()
                    .find(|m| {
                        matches!(
                            m,
                            Message::CalendarModeSelected(true)
                                | Message::CalendarMonthViewSelected
                        )
                    })
                    .unwrap();
                let _ = app::update(&mut state, message);
                assert_eq!(state.screen, screen);
                assert_eq!(state.monthly, monthly);
                assert_eq!(state.weekly, !monthly);
                let mut ui = render_main(&state, Size::new(width, 900.0));
                ui.click(hide)?;
                let message = ui
                    .into_messages()
                    .find(|m| matches!(m, Message::CalendarModeSelected(false)))
                    .unwrap();
                let _ = app::update(&mut state, message);
                assert!(!state.weekly && !state.monthly);
            }
        }
    }
    Ok(())
}

#[test]
fn schedule_opens_day_panel_without_hiding_the_overview() -> Result<(), Error> {
    for monthly in [false, true] {
        let mut state = state();
        state.weekly = !monthly;
        state.monthly = monthly;
        let mut ui = render_main(&state, Size::new(180.0, 700.0));
        ui.click("Schedule")?;
        let message = ui
            .into_messages()
            .find(|m| matches!(m, Message::ScreenSelected(Screen::Calendar)))
            .unwrap();
        let _ = app::update(&mut state, message);
        assert_eq!(state.screen, Screen::Calendar);
        assert_eq!(state.weekly, !monthly);
        assert_eq!(state.monthly, monthly);
    }
    Ok(())
}

#[test]
fn first_overview_click_preserves_the_sidebar_through_window_startup() -> Result<(), Error> {
    for monthly in [false, true] {
        for screen in [
            Screen::Inbox,
            Screen::Backlog,
            Screen::Projects,
            Screen::Notepad,
            Screen::Settings,
            Screen::Travel,
            Screen::Habits,
            Screen::KeepNotes,
            Screen::BrainDump,
            Screen::Overdue,
            Screen::Triaged,
        ] {
            let mut state = state();
            state.weekly = false;
            state.monthly = false;
            let main = iced::window::Id::unique();
            state.main_window = Some(main);
            let _ = app::update(&mut state, Message::ScreenSelected(screen));
            state.timeblocking.available_query = "keep my filter".into();
            state.timeblocking.available_all = true;
            state.timeblocking.available_planned = true;
            let mut panel = render_main(&state, Size::new(320.0, 900.0));
            panel.click(if monthly { "Show month" } else { "Show week" })?;
            // Apply every emitted message so an accidental navigation cannot
            // be hidden by filtering the simulator's output.
            let messages: Vec<_> = panel.into_messages().collect();
            for message in messages {
                let _ = app::update(&mut state, message);
            }
            let floating = state.editor_window.expect("overview window");
            let _ = app::update(&mut state, Message::WindowOpened(floating));
            let _ = app::update(&mut state, Message::EditorWindowOpened(floating));
            assert_eq!(state.screen, screen);
            assert_eq!(state.main_window, Some(main));
            assert_eq!(state.timeblocking.available_query, "keep my filter");
            assert!(state.timeblocking.available_all);
            assert!(state.timeblocking.available_planned);
            assert_eq!(state.monthly, monthly);
            assert_eq!(state.weekly, !monthly);
        }
    }
    Ok(())
}

#[test]
fn overview_stays_open_across_all_panel_tabs() -> Result<(), Error> {
    for monthly in [false, true] {
        let mut state = state();
        state.main_window = Some(iced::window::Id::unique());
        state.weekly = false;
        state.monthly = false;
        let _ = app::update(
            &mut state,
            if monthly {
                Message::CalendarMonthViewSelected
            } else {
                Message::CalendarModeSelected(true)
            },
        );
        let floating = state.editor_window.unwrap();
        let anchor = state.week_anchor;
        for screen in [
            Screen::Inbox,
            Screen::Overdue,
            Screen::Triaged,
            Screen::Backlog,
            Screen::Projects,
            Screen::Notepad,
            Screen::Travel,
            Screen::Habits,
            Screen::KeepNotes,
            Screen::BrainDump,
            Screen::Settings,
            Screen::Calendar,
        ] {
            let _ = app::update(&mut state, Message::ScreenSelected(screen));
            assert_eq!(state.screen, screen);
            assert_eq!(state.editor_window, Some(floating));
            assert_eq!(state.week_anchor, anchor);
            assert_eq!(state.weekly, !monthly);
            assert_eq!(state.monthly, monthly);
            let mut panel = render_main(&state, Size::new(260.0, 900.0));
            assert!(
                panel
                    .find(if monthly { "Hide month" } else { "Hide week" })
                    .is_ok()
            );
            drop(panel);
            let mut overview = Simulator::with_size(
                Settings::default(),
                Size::new(1100.0, 900.0),
                timeslay::ui::editor_window(&state),
            );
            assert!(
                overview
                    .find(if monthly { "Month" } else { "Week" })
                    .is_ok()
            );
            assert!(overview.find("Close view").is_err());
        }
        let mut panel = render_main(&state, Size::new(260.0, 900.0));
        panel.click(if monthly { "Hide month" } else { "Hide week" })?;
        let hide = panel
            .into_messages()
            .find(|m| matches!(m, Message::CalendarModeSelected(false)))
            .unwrap();
        let _ = app::update(&mut state, hide);
        assert!(!state.weekly && !state.monthly);
        assert!(state.editor_window.is_none());
    }
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
        let mut ui = render_main(&state, Size::new(width, 700.0));
        let switcher = timeslay::ui::calendar_view_label(&state);
        let weekly = ui.find(switcher)?.bounds();
        assert!(weekly.x >= 0.0 && weekly.x + weekly.width <= width);
        let hide = ui.find("Hide week")?.bounds();
        assert!(hide.x >= 0.0 && hide.x + hide.width <= width);
        let _ = ui.click("Hide week")?;
        assert!(
            ui.into_messages()
                .any(|m| matches!(m, Message::CalendarModeSelected(false)))
        );
    }
    let mut ui = render(&state, Size::new(180.0, 240.0));
    let heading = ui.find("Wed Oct 14")?.bounds();
    assert!(heading.y + heading.height < 240.0, "heading {heading:?}");
    drop(ui);
    state.screen = Screen::Settings;
    let mut ui = render_main(&state, Size::new(900.0, 1600.0));
    assert!(ui.find("Days shown").is_ok());
    assert!(ui.find("Weekly day width: 150 px").is_ok());
    Ok(())
}

#[test]
fn task_list_mixes_tasks_and_events_in_start_order_and_keeps_each_day_separate() {
    let mut state = state();
    let mut early = state.calendar.event("task:thursday").unwrap().clone();
    early.id = "task:early".into();
    early.title = "Earlier task".into();
    early.start = at(14, 8);
    early.end = at(14, 9);
    // A task linked to a block must still appear as its own list entry.
    let mut data = timeslay::timeblocking::metadata(&early);
    data.block = Some(timeslay::timeblocking::block_key(
        state
            .calendar
            .events_on(date(14))
            .iter()
            .find(|e| e.id == "standup")
            .unwrap(),
    ));
    early.details.task_data = Some(timeslay::task_db::TaskData {
        id: 999,
        metadata: data,
        legacy_key: None,
    });
    early.details.scheduled = Some(calendar::scheduled::Link::new("work".into(), &early).unwrap());
    state.calendar.replace_cached_task("task:early", &early);
    let entries = state
        .calendar
        .task_list_events_on(date(14), &state.config.calendar);
    assert_eq!(
        entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        vec!["task:early", "standup"]
    );
    let entries = state
        .calendar
        .task_list_events_on(date(15), &state.config.calendar);
    assert_eq!(
        entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        vec!["task:thursday"]
    );
    for day in 16..=18 {
        assert!(
            state
                .calendar
                .task_list_events_on(date(day), &state.config.calendar)
                .iter()
                .any(|e| e.id == "spanning")
        );
    }
    assert!(
        state
            .calendar
            .task_list_events_on(date(19), &state.config.calendar)
            .is_empty()
    );
}

#[test]
fn weekly_task_view_omits_all_day_tasks_without_hiding_scheduled_cards() -> Result<(), Error> {
    let mut state = state();
    state.config.calendar.weekly_task_list = true;
    let template = state.calendar.event("task:thursday").unwrap().clone();
    for (id, day, completed) in [
        ("first", date(14), false),
        ("second", date(15), false),
        ("third", date(15), false),
        ("completed", date(15), true),
        ("undated", timeslay::timeblocking::UNDATED, false),
        ("overdue", date(13), false),
        ("linked", date(14), false),
    ] {
        let mut task = template.clone();
        task.id = format!("task:{id}");
        task.title = format!("Day task {id}");
        task.all_day = true;
        task.completed = completed;
        task.start = day
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap();
        task.end = task.start + Duration::days(1);
        if id == "overdue" {
            task.details.description = "DUE: 2020-01-01".into();
        }
        if id == "linked" {
            let mut metadata = timeslay::timeblocking::metadata(&task);
            metadata.block = Some("work/standup".into());
            task.details.task_data = Some(timeslay::task_db::TaskData {
                id: 999,
                metadata,
                legacy_key: None,
            });
        }
        let task_id = task.id.clone();
        state.calendar.replace_cached_task(&task_id, &task);
        state.calendar.events.push(task);
    }
    let mut ui = render(&state, Size::new(1500.0, 900.0));
    assert!(ui.find("Wednesday").is_ok());
    assert!(ui.find("October 14").is_ok());
    for count in 0..=2 {
        assert!(ui.find(format!("{count} not slotted in")).is_err());
    }
    for id in [
        "first",
        "second",
        "third",
        "completed",
        "undated",
        "overdue",
        "linked",
    ] {
        assert!(ui.find(format!("Day task {id}")).is_err());
    }
    assert!(ui.find("Thursday task").is_ok());
    assert!(ui.find("Wednesday block").is_ok());
    assert!(ui.find("Multi-day event").is_ok());
    Ok(())
}

#[test]
fn task_list_switch_preserves_week_and_cards_show_start_times_and_open_details() -> Result<(), Error>
{
    let mut state = state();
    state.config.appearance.time_format =
        timeslay::calendar::time_format::TimeFormat::TwentyFourHour;
    let _ = app::update(&mut state, Message::CalendarTaskListToggled(true));
    assert!(state.config.calendar.weekly_task_list);
    assert_eq!(state.week_anchor, date(15));
    let saved = toml::to_string(&state.config.calendar).unwrap();
    let loaded: timeslay::config::CalendarSettings = toml::from_str(&saved).unwrap();
    assert!(loaded.weekly_task_list);
    let mut ui = render(&state, Size::new(1500.0, 900.0));
    assert!(ui.find("▦ Board").is_ok());
    let title = ui.find("Wednesday block")?.bounds();
    let start = ui.find("09:00")?.bounds();
    assert!(
        start.y < title.y && (start.x - title.x).abs() < 6.0,
        "start {start:?}, title {title:?}"
    );
    let _ = ui.click("Thursday task")?;
    assert!(ui.into_messages().any(|message| matches!(message,
        Message::WeekDayAction(day, action) if day == date(15)
            && matches!(*action, Message::EventOpened(ref id) if id == "task:thursday")
    )));
    let _ = app::update(&mut state, Message::CalendarTaskListToggled(false));
    assert!(!state.config.calendar.weekly_task_list);
    assert_eq!(state.week_anchor, date(15));
    Ok(())
}

fn board_state() -> State {
    let mut state = state();
    state.config.calendar.weekly_task_list = true;
    for (id, title, day, hour) in [
        ("first", "First board task", 15, 8),
        ("last", "Last board task", 15, 11),
        ("other", "Other day task", 14, 11),
    ] {
        let mut task = state.calendar.event("task:thursday").unwrap().clone();
        task.id = format!("task:{id}");
        task.title = title.into();
        task.start = at(day, hour);
        task.end = task.start + Duration::minutes(45);
        state.calendar.replace_cached_task(&task.id, &task);
        state.calendar.events.push(task);
    }
    state
}

#[test]
fn board_cards_drag_from_title_badges_and_footer_without_opening_or_completing() -> Result<(), Error>
{
    use iced::{Event, Point, Vector, mouse};
    use timeslay::timeblocking::Action;
    // A title, its time badges and its footer all lift the same card.
    for offset in [
        Vector::new(0.0, 0.0),
        Vector::new(15.0, -20.0),
        Vector::new(-15.0, 23.0),
    ] {
        let state = board_state();
        let mut ui = render(&state, Size::new(1500.0, 900.0));
        let title = ui.find("First board task")?.bounds();
        let origin = Point::new(title.center_x(), title.center_y()) + offset;
        ui.point_at(origin);
        ui.simulate([Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left,
        ))]);
        let position = origin + Vector::new(10.0, 0.0);
        ui.point_at(position);
        ui.simulate([Event::Mouse(mouse::Event::CursorMoved { position })]);
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(messages.iter().any(|m| matches!(m, Message::WeekDayAction(day, inner)
            if *day == date(15) && matches!(&**inner, Message::TimeBlocking(Action::Drag(id, _)) if id == "task:first"))), "{messages:?}");
        assert!(
            !messages
                .iter()
                .any(|m| matches!(m, Message::WeekDayAction(_, inner)
            if matches!(&**inner, Message::EventOpened(_) | Message::TaskToggled(_))))
        );
    }
    Ok(())
}

#[test]
fn board_drop_targets_cover_before_after_events_and_empty_days() -> Result<(), Error> {
    use iced::{Event, Point, mouse};
    use timeslay::timeblocking::Action;
    let mut state = board_state();
    state.timeblocking.dragging = Some("task:first".into());
    for (title, expected_day, target) in [
        ("Thursday task", date(15), "task:thursday"),
        ("Wednesday block", date(14), "standup"),
        ("Other day task", date(14), "task:other"),
    ] {
        // The title is in the upper half; the footer is in the lower half.
        for after in [false, true] {
            let mut ui = render(&state, Size::new(1500.0, 900.0));
            let title_bounds = ui.find(title)?.bounds();
            let point = Point::new(
                title_bounds.center_x(),
                if after {
                    title_bounds.y + title_bounds.height + 12.0
                } else {
                    title_bounds.y - 12.0
                },
            );
            ui.point_at(point);
            ui.simulate([Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left,
            ))]);
            let messages: Vec<_> = ui.into_messages().collect();
            assert!(
                matches!(&messages[..], [Message::WeekDayAction(day, inner)]
                if *day == expected_day && matches!(&**inner, Message::TimeBlocking(Action::DropWeekList { target: Some(id), after: a }) if id == target && *a == after)),
                "{point:?}: {messages:?}"
            );
        }
    }
    let mut ui = render(&state, Size::new(1500.0, 900.0));
    let empty_day = ui.find("October 19")?.bounds();
    ui.point_at(Point::new(empty_day.center_x(), 600.0));
    ui.simulate([Event::Mouse(mouse::Event::ButtonReleased(
        mouse::Button::Left,
    ))]);
    assert!(ui.into_messages().any(|m| matches!(m, Message::WeekDayAction(day, inner)
        if day == date(19) && matches!(*inner, Message::TimeBlocking(Action::DropWeekList { target: None, .. })))));
    Ok(())
}

#[test]
fn board_reordering_is_persistent_and_keeps_schedule_and_block_membership() {
    use timeslay::timeblocking::{self as tb, Action};
    let mut state = board_state();
    // The board works regardless of the sidebar tab.
    state.screen = Screen::Backlog;
    let original = state.calendar.event("task:first").unwrap().clone();
    for (target, after, expected) in [
        (
            Some("task:last"),
            true,
            vec!["task:thursday", "task:last", "task:first"],
        ),
        (
            Some("task:thursday"),
            false,
            vec!["task:first", "task:thursday", "task:last"],
        ),
        (None, true, vec!["task:thursday", "task:last", "task:first"]),
    ] {
        state.timeblocking.dragging = Some("task:first".into());
        let _ = app::update(
            &mut state,
            Message::WeekDayAction(
                date(15),
                Box::new(Message::TimeBlocking(Action::DropWeekList {
                    target: target.map(str::to_owned),
                    after,
                })),
            ),
        );
        assert_eq!(
            tb::weekly_list_events(&state, date(15))
                .iter()
                .map(|e| e.id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(state.calendar.event("task:first").unwrap(), &original);
        assert!(state.timeblocking.dragging.is_none());
        assert!(state.timeblocking.busy.is_empty());
    }
    let serialized = toml::to_string(&state.config.calendar).unwrap();
    let loaded = toml::from_str::<timeslay::config::CalendarSettings>(&serialized).unwrap();
    assert_eq!(
        loaded.weekly_task_order,
        state.config.calendar.weekly_task_order
    );
    assert_eq!(state.week_anchor, date(15));
    assert_eq!(state.screen, Screen::Backlog);
}

#[test]
fn add_task_on_a_board_day_opens_a_timed_task_for_that_day() {
    use timeslay::timeblocking::Action;
    let mut state = board_state();
    let _ = app::update(
        &mut state,
        Message::WeekDayAction(
            date(14),
            Box::new(Message::TimeBlocking(Action::NewWeekTask)),
        ),
    );
    let task = state.inbox.draft.as_ref().unwrap().event.as_ref().unwrap();
    assert!(!task.all_day);
    assert_eq!(task.start, at(14, 9));
    assert_eq!(task.end, at(14, 9) + Duration::minutes(30));
    assert_eq!(state.week_anchor, date(15));
}

#[test]
fn board_layout_renders_at_reference_size_and_in_dark_mode() -> Result<(), Error> {
    let mut state = state();
    state.config.calendar.weekly_task_list = true;
    state.config.appearance.time_format =
        timeslay::calendar::time_format::TimeFormat::TwentyFourHour;
    let template = state.calendar.event("task:thursday").unwrap().clone();
    let mut events = Vec::new();
    for (index, title, day, hour, minutes, notes, project) in [
        (
            0,
            "Build daily notes feature",
            14,
            6,
            240,
            "- [ ] Mockups\n- [ ] Data model\n- [ ] Basic functionality",
            1,
        ),
        (
            1,
            "Document customer feedback",
            14,
            10,
            90,
            "- [ ] Summarize customer churn surveys\n- [ ] Review top posts",
            1,
        ),
        (
            2,
            "Investigate secondary growth channels",
            14,
            12,
            60,
            "",
            2,
        ),
        (3, "Product demo with Jenn", 14, 14, 90, "", 3),
        (4, "Answer customer support tickets", 15, 8, 30, "", 3),
        (5, "Investigate secondary growth channels", 15, 9, 30, "", 3),
        (6, "Review prototype of new feature", 15, 10, 120, "", 1),
        (7, "1:1 with Tomoa", 15, 11, 30, "", 3),
    ] {
        let mut task = template.clone();
        task.id = format!("task:reference-{index}");
        task.title = title.into();
        task.start = at(day, hour);
        task.end = task.start + Duration::minutes(minutes);
        task.details.description = notes.into();
        task.details.task_data = Some(timeslay::task_db::TaskData {
            id: index + 1,
            metadata: timeslay::timeblocking::Metadata {
                projects: vec![project],
                ..Default::default()
            },
            legacy_key: None,
        });
        events.push(task);
    }
    state.config.projects.items = [
        (1, "product", "#a282ff"),
        (2, "planning", "#6acddd"),
        (3, "growth", "#ffb34e"),
    ]
    .into_iter()
    .map(|(id, name, color)| timeslay::config::Project {
        id,
        name: name.into(),
        color: Some(timeslay::hex::parse(color).unwrap().into()),
        directory: None,
    })
    .collect();
    state.calendar.apply_week_snapshot(
        calendar::Snapshot {
            calendars: state.calendar.calendars.clone(),
            events,
            day: date(15),
            fetched_at: Local::now(),
            notice: None,
        },
        &calendar::week_dates(date(15), 8),
    );
    for (name, theme) in [("light", iced::Theme::Light), ("dark", iced::Theme::Dark)] {
        let mut ui = render(&state, Size::new(492.0, 653.0));
        let day = ui.find("Wednesday")?.bounds();
        let date = ui.find("October 14")?.bounds();
        let card = ui.find("Build daily notes feature")?.bounds();
        assert!(day.height > date.height && date.y > day.y && card.y > date.y);
        assert!(ui.find("Mockups").is_ok());
        assert!(ui.find("4:00").is_ok());
        assert!(ui.find("All day").is_err());
        if let Ok(directory) = std::env::var("TIMESLAY_BOARD_PREVIEW") {
            ui.snapshot(&theme)?.matches_image(
                std::path::Path::new(&directory).join(format!("week-board-{name}")),
            )?;
        }
    }
    Ok(())
}
