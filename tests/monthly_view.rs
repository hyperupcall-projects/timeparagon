use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use iced::{Element, Size};
use iced_test::{Simulator, core::Settings};
use timeslay::{
    app::{self, Message, State},
    calendar::{self, Store},
    config::Config,
    theme::Catalog,
};

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

fn state() -> State {
    let mut state = State::new(Config::default(), Catalog::built_in());
    let mut snapshot = calendar::sample_snapshot();
    snapshot.day = date(2026, 8, 15); // Six-week month, including July and September.
    let template = snapshot
        .events
        .iter()
        .find(|e| e.id == "standup")
        .unwrap()
        .clone();
    snapshot.events = calendar::month_grid(snapshot.day)
        .into_iter()
        .map(|day| {
            let mut event = template.clone();
            event.id = day.to_string();
            event.title = format!("Meeting on {day}");
            event.start = Local
                .from_local_datetime(&day.and_hms_opt(9, 0, 0).unwrap())
                .single()
                .unwrap();
            event.end = event.start + Duration::hours(1);
            event
        })
        .collect();
    state.calendar = Store::with_snapshot(calendar::Snapshot {
        events: vec![],
        ..snapshot.clone()
    });
    state.monthly = true;
    state.week_anchor = snapshot.day;
    let dates = state.calendar_dates();
    state.calendar.apply_week_snapshot(snapshot, &dates);
    state
}

fn render(state: &State, size: Size) -> Simulator<'_, Message> {
    let view: Element<'_, Message> = app::view(state, iced::window::Id::unique());
    Simulator::with_size(Settings::default(), size, view)
}

#[test]
fn whole_month_survives_cache_pruning_and_day_interactions() {
    let mut state = state();
    let dates = state.calendar_dates();
    assert_eq!(dates.len(), 42);
    for day in &dates {
        assert!(
            state
                .calendar
                .events_on(*day)
                .iter()
                .any(|event| event.id == day.to_string()),
            "missing {day}"
        );
    }
    let last = *dates.last().unwrap();
    let _ = app::update(
        &mut state,
        Message::WeekDayAction(last, Box::new(Message::EventOpened(last.to_string()))),
    );
    assert_eq!(state.calendar.day, last);
    assert_eq!(state.week_anchor, date(2026, 8, 15));
    assert_eq!(state.calendar_dates(), dates);
    for day in &dates {
        assert!(!state.calendar.events_on(*day).is_empty(), "lost {day}");
    }
}

#[test]
fn navigation_steps_calendar_months_across_years_and_leap_february() {
    let mut state = state();
    state.calendar.follow_today = false;
    for (origin, shift, expected) in [
        (date(2026, 12, 31), 1, date(2027, 1, 1)),
        (date(2028, 3, 31), -1, date(2028, 2, 1)),
        (date(2028, 2, 29), 1, date(2028, 3, 1)),
    ] {
        state.week_anchor = origin;
        let _ = app::update(&mut state, Message::CalendarViewMonthShift(shift));
        assert_eq!(state.week_anchor, expected);
        assert_eq!(state.calendar.day, expected);
        assert!(state.monthly);
        assert!(!state.weekly);
    }
}

#[test]
fn month_event_click_selects_its_day_and_opens_details() {
    let mut state = state();
    let day = date(2026, 7, 27);
    let mut ui = render(&state, Size::new(1100.0, 1100.0));
    ui.click("9:00am Meeting on 2026-07-27").unwrap();
    let action = ui.into_messages().find(|m| matches!(m, Message::WeekDayAction(d, inner) if *d == day && matches!(inner.as_ref(), Message::EventOpened(_)))).unwrap();
    let _ = app::update(&mut state, action);
    assert_eq!(state.calendar.day, day);
    assert_eq!(state.week_anchor.month(), 8);
    let mut ui = render(&state, Size::new(1100.0, 1100.0));
    assert!(ui.find("Meeting on 2026-07-27").is_ok());
}

#[test]
fn date_opens_daily_timeline_and_modes_are_exclusive() {
    let mut state = state();
    let day = date(2026, 9, 6);
    let _ = app::update(&mut state, Message::CalendarDayOpened(day));
    assert_eq!(state.calendar.day, day);
    assert!(!state.monthly && !state.weekly);
    let _ = app::update(&mut state, Message::CalendarMonthViewSelected);
    assert!(state.monthly && !state.weekly);
    let _ = app::update(&mut state, Message::CalendarModeSelected(true));
    assert!(state.weekly && !state.monthly);
}

#[test]
fn month_navigation_remains_reachable_at_small_widths() {
    let state = state();
    for width in [180.0, 260.0, 420.0, 1100.0] {
        let mut ui = render(&state, Size::new(width, 900.0));
        for label in [
            timeslay::ui::calendar_view_label(&state),
            "Today",
            "Menu",
            "August 2026 ▾",
        ] {
            let bounds = ui.find(label).unwrap().bounds();
            assert!(
                bounds.x >= 0.0 && bounds.x + bounds.width <= width + 0.5,
                "{label} at {width}: {bounds:?}"
            );
        }
    }
}
