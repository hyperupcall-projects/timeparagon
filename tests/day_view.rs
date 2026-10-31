use chrono::{Duration, Local, TimeZone};
use iced::Size;
use iced_test::{Simulator, core::Settings};
use timeslay::{app::State, calendar, config::Config, theme::Catalog, ui};

#[test]
fn day_view_summarizes_untimed_tasks_and_keeps_all_day_events() {
    let mut state = State::new(Config::default(), Catalog::built_in());
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    state.calendar.day = Local::now().date_naive();
    let start = Local
        .from_local_datetime(&state.calendar.day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .unwrap();
    let mut all_day = state.calendar.events[0].clone();
    all_day.id = "offsite".into();
    all_day.title = "Team offsite".into();
    all_day.all_day = true;
    all_day.start = start;
    all_day.end = start + Duration::days(1);
    state.calendar.events = vec![all_day.clone()];

    for count in [0, 1, 3] {
        state.calendar.events.truncate(1);
        for index in 0..count {
            let mut task = all_day.clone();
            task.id = format!("task:unslotted-{index}");
            task.calendar_id = "tasks:work".into();
            task.title = format!("Unslotted task {index}");
            state.calendar.events.push(task);
        }
        let mut view: Simulator<'_, timeslay::app::Message> = Simulator::with_size(
            Settings::default(),
            Size::new(720.0, 700.0),
            ui::view(&state),
        );
        assert!(
            view.find(format!("0 overdue.  0 in inbox.  {count} not slotted in"))
                .is_ok()
        );
        assert!(view.find("Team offsite").is_ok());
        for index in 0..count {
            assert!(view.find(format!("Unslotted task {index}")).is_err());
        }
    }

    state.weekly = true;
    state.week_anchor = state.calendar.day;
    let mut week: Simulator<'_, timeslay::app::Message> = Simulator::with_size(
        Settings::default(),
        Size::new(1200.0, 700.0),
        ui::editor_window(&state),
    );
    assert!(week.find("Unslotted task 0").is_ok());
    assert!(
        week.find("0 overdue.  0 in inbox.  3 not slotted in")
            .is_err()
    );
    drop(week);

    state.weekly = false;
    state.config.calendar.inbox_list = Some("tasks:inbox".into());
    for (id, day) in [
        ("inbox", timeslay::timeblocking::UNDATED),
        ("overdue-1", state.calendar.day - Duration::days(1)),
        ("overdue-2", state.calendar.day - Duration::days(2)),
    ] {
        let mut task = all_day.clone();
        task.id = format!("task:{id}");
        task.calendar_id = "tasks:inbox".into();
        task.start = Local
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .unwrap();
        task.end = task.start + Duration::days(1);
        state.calendar.events.push(task);
    }
    let mut day = Simulator::with_size(
        Settings::default(),
        Size::new(720.0, 700.0),
        ui::view(&state),
    );
    assert!(
        day.find("2 overdue.  1 in inbox.  3 not slotted in")
            .is_ok()
    );
}

#[test]
fn range_views_keep_the_day_panel_visible_and_render_in_the_floating_host() {
    for monthly in [false, true] {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
        state.calendar.events.clear();
        state.week_anchor = state.calendar.day;
        state.weekly = !monthly;
        state.monthly = monthly;
        let mut day = Simulator::with_size(
            Settings::default(),
            Size::new(400.0, 900.0),
            ui::view(&state),
        );
        assert!(
            day.find("0 overdue.  0 in inbox.  0 not slotted in")
                .is_ok()
        );
        assert!(day.find("Close view").is_err());
        let mut floating = Simulator::with_size(
            Settings::default(),
            Size::new(1200.0, 900.0),
            ui::editor_window(&state),
        );
        assert!(
            floating
                .find(if monthly { "Month" } else { "Week" })
                .is_ok()
        );
        assert!(floating.find("Close view").is_err());
        assert!(
            floating
                .find("0 overdue.  0 in inbox.  0 not slotted in")
                .is_err()
        );
    }
}
