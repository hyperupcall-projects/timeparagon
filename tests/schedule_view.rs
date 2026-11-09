use chrono::{Duration, Local, NaiveDate, TimeZone};
use iced::Size;
use iced_test::{Simulator, core::Settings};
use timeslay::{
    app::{self, CalendarMessage, CalendarView, EventMessage, Message, MonthPicker, State},
    calendar::{self, Store},
};

fn date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
}

fn state() -> State {
    let mut state = State::blank();
    let mut snapshot = calendar::sample_snapshot();
    snapshot.day = date(8);
    let mut event = snapshot
        .events
        .iter()
        .find(|e| e.id == "standup")
        .unwrap()
        .clone();
    event.title = "Overnight study".into();
    event.start = Local.with_ymd_and_hms(2026, 11, 8, 22, 30, 0).unwrap();
    event.end = event.start + Duration::hours(8);
    snapshot.events = vec![event];
    state.view = CalendarView::Schedule;
    state.plan_view = CalendarView::Schedule;
    state.week_anchor = date(8);
    state.calendar = Store::with_snapshot(snapshot.clone());
    state
        .calendar
        .apply_week_snapshot(snapshot, &state.calendar_dates());
    state
}

#[test]
fn schedule_renders_overnight_occurrences_at_both_widths_and_opens_event() {
    for width in [390.0, 1200.0] {
        let mut state = state();
        let size = Size::new(width, 1000.0);
        let metrics = timeslay::ui::layout::Metrics::of(size);
        let mut ui = Simulator::with_size(
            Settings::default(),
            size,
            timeslay::ui::calendar::schedule_view(&state, &metrics),
        );
        assert!(ui.find("Overnight study (Day 1/2)").is_ok());
        assert!(ui.find("Overnight study (Day 2/2)").is_ok());
        ui.click("Overnight study (Day 2/2)").unwrap();
        let message = ui.into_messages().find(|m| matches!(m, Message::Calendar(CalendarMessage::WeekDayAction(_, inner)) if matches!(inner.as_ref(), Message::Event(EventMessage::EventOpened(_))))).unwrap();
        let _ = app::update(&mut state, message);
        assert_eq!(state.calendar.day, date(9));
        assert_eq!(state.week_anchor, date(8));
        assert!(state.calendar.open_event.is_some());
    }
}

#[test]
fn date_picker_jumps_schedule_without_switching_views() {
    let mut state = state();
    let target = NaiveDate::from_ymd_opt(2027, 1, 31).unwrap();
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarMonthDayPicked(
            MonthPicker::Overview,
            target,
        )),
    );
    assert_eq!(state.view, CalendarView::Schedule);
    assert_eq!(state.calendar_dates()[0], target);
    assert_eq!(state.calendar_dates().len(), 30);
    assert!(!state.overview_month_open);
}

#[test]
fn schedule_selection_closes_menu_and_preserves_date_when_reselected() {
    let mut state = State::blank();
    let selected = state.calendar.day;
    state.view_menu_open = true;
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarScheduleViewSelected),
    );
    assert_eq!(state.view, CalendarView::Schedule);
    assert_eq!(state.plan_view, CalendarView::Schedule);
    assert!(!state.view_menu_open);
    assert_eq!(state.week_anchor, selected);
    state.week_anchor = selected + Duration::days(10);
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarScheduleViewSelected),
    );
    assert_eq!(state.week_anchor, selected + Duration::days(10));
}
