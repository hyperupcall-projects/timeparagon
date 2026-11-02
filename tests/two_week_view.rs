use chrono::{Duration, Local, NaiveDate, TimeZone};
use iced::{Element, Size};
use iced_test::{Simulator, core::Settings};
use timeslay::app::{CalendarMessage, EventDraftMessage, EventMessage};
use timeslay::{
    app::{self, Message, State},
    calendar::{self, Store},
};

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// A Thursday, in a fortnight that runs from Mon Oct 26 to Sun Nov 8.
fn anchor() -> NaiveDate {
    date(2026, 10, 29)
}

fn state(events_per_day: u32) -> State {
    let mut state = State::blank();
    let mut snapshot = calendar::sample_snapshot();
    snapshot.day = anchor();
    let template = snapshot
        .events
        .iter()
        .find(|e| e.id == "standup")
        .unwrap()
        .clone();
    snapshot.events = calendar::two_week_dates(snapshot.day)
        .into_iter()
        .flat_map(|day| (0..events_per_day).map(move |index| (day, index)))
        .map(|(day, index)| {
            let mut event = template.clone();
            event.id = format!("{day}-{index}");
            event.title = format!("Meeting {index} on {day}");
            event.start = Local
                .from_local_datetime(&day.and_hms_opt(8 + index % 12, 0, 0).unwrap())
                .single()
                .unwrap();
            event.end = event.start + Duration::minutes(30);
            event
        })
        .collect();
    state.calendar = Store::with_snapshot(calendar::Snapshot {
        events: vec![],
        ..snapshot.clone()
    });
    state.view = timeslay::app::CalendarView::TwoWeek;
    state.week_anchor = snapshot.day;
    let dates = state.calendar_dates();
    state.calendar.apply_week_snapshot(snapshot, &dates);
    state
}

fn render(state: &State, size: Size) -> Simulator<'_, Message> {
    let view: Element<'_, Message> = timeslay::ui::editor_window(state);
    Simulator::with_size(Settings::default(), size, view)
}

#[test]
fn shows_two_whole_weeks_starting_on_the_anchors_monday() {
    let state = state(1);
    let dates = state.calendar_dates();
    assert_eq!(dates.len(), 14);
    assert_eq!(dates[0], date(2026, 10, 26));
    assert_eq!(dates[13], date(2026, 11, 8));
    let mut ui = render(&state, Size::new(1400.0, 1000.0));
    assert!(ui.find("Oct 26 – Nov 8, 2026 ▾").is_ok());
    for label in ["2 Weeks", "MON", "SUN", "26", "Nov 1", "8", "Today", "Menu"] {
        assert!(ui.find(label).is_ok(), "missing {label}");
    }
    let clock = calendar::time_format::clock(
        chrono::NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
        state.config.appearance.time_format,
    );
    for day in dates {
        assert!(ui.find(format!("{clock} Meeting 0 on {day}")).is_ok());
    }
}

#[test]
fn stepping_moves_a_whole_fortnight_and_keeps_the_view() {
    let mut state = state(1);
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarDayShift(14)),
    );
    assert!(state.two_weekly());
    assert_eq!(state.calendar_dates()[0], date(2026, 11, 9));
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarDayShift(-28)),
    );
    assert_eq!(state.calendar_dates()[0], date(2026, 10, 12));
}

#[test]
fn a_full_cell_offers_the_rest_in_a_popup() {
    let mut state = state(30);
    let mut ui = render(&state, Size::new(1400.0, 700.0));
    let day = date(2026, 11, 3);
    let clock = |index: u32| {
        calendar::time_format::clock(
            chrono::NaiveTime::from_hms_opt(8 + index % 12, 0, 0).unwrap(),
            state.config.appearance.time_format,
        )
    };
    let last = format!("{} Meeting 29 on {day}", clock(29));
    assert!(ui.find(last.as_str()).is_err());
    assert!(ui.find(format!("{} Meeting 0 on {day}", clock(0))).is_ok());
    drop(ui);

    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarTwoWeekMore(Some(day))),
    );
    let mut ui = render(&state, Size::new(1400.0, 700.0));
    assert!(ui.find("Tue, Nov 3").is_ok());
    assert!(ui.find(last.as_str()).is_ok());
    drop(ui);

    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarDayShift(14)),
    );
    assert_eq!(state.two_week_more, None);
}

#[test]
fn every_row_of_a_short_day_fits_without_a_more_link() {
    let state = state(2);
    let mut ui = render(&state, Size::new(1400.0, 1000.0));
    assert!(ui.find("2 more").is_err());
    assert!(ui.find("1 more").is_err());
}

#[test]
fn pressing_an_empty_cell_drafts_an_event_on_that_day() {
    let mut state = state(1);
    let day = date(2026, 11, 3);
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarTwoWeekCellPressed(day)),
    );
    assert_eq!(state.calendar.day, day);
    assert!(state.two_weekly());
    assert_eq!(state.calendar_dates()[0], date(2026, 10, 26));
    assert!(state.calendar.draft.is_some());
    assert!(
        render(&state, Size::new(1400.0, 1000.0))
            .find("Add title")
            .is_ok()
    );
    let _ = app::update(
        &mut state,
        Message::EventDraft(EventDraftMessage::EventDraftCancelled),
    );
    assert!(state.calendar.draft.is_none());
}

fn pressed_cell(messages: &[Message]) -> Option<NaiveDate> {
    messages.iter().find_map(|message| match message {
        Message::Calendar(CalendarMessage::CalendarTwoWeekCellPressed(day)) => Some(*day),
        _ => None,
    })
}

#[test]
fn a_click_starts_a_draft_only_on_empty_space() {
    let mut state = state(1);
    let size = Size::new(1400.0, 1000.0);
    let day = date(2026, 11, 1);

    let mut ui = render(&state, size);
    let date_label = ui.find("Nov 1").unwrap().bounds();
    ui.point_at(iced::Point::new(
        date_label.center_x(),
        date_label.y + 200.0,
    ));
    ui.simulate(iced_test::simulator::click());
    let messages: Vec<_> = ui.into_messages().collect();
    assert_eq!(pressed_cell(&messages), Some(day));

    let clock = calendar::time_format::clock(
        chrono::NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
        state.config.appearance.time_format,
    );
    let mut ui = render(&state, size);
    ui.click(format!("{clock} Meeting 0 on {day}")).unwrap();
    let messages: Vec<_> = ui.into_messages().collect();
    assert_eq!(pressed_cell(&messages), None);
    assert!(messages.iter().any(|message| matches!(
        message,
        Message::Calendar(CalendarMessage::WeekDayAction(d, inner))
            if *d == day && matches!(inner.as_ref(), Message::Event(EventMessage::EventOpened(_)))
    )));

    // Clicking away from the popup closes it, without adding an event under it.
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarTwoWeekMore(Some(day))),
    );
    let mut ui = render(&state, size);
    ui.point_at(iced::Point::new(60.0, 400.0));
    ui.simulate(iced_test::simulator::click());
    let messages: Vec<_> = ui.into_messages().collect();
    assert_eq!(pressed_cell(&messages), None);
    assert!(messages.iter().any(|message| matches!(
        message,
        Message::Calendar(CalendarMessage::CalendarTwoWeekMore(None))
    )));
}

#[test]
fn the_view_is_exclusive_with_the_others_and_closes_back_to_the_day() {
    let mut state = state(1);
    assert!(state.two_weekly() && !state.weekly() && !state.monthly());
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarMonthViewSelected),
    );
    assert!(state.monthly() && !state.two_weekly());
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarTwoWeekViewSelected),
    );
    assert!(state.two_weekly() && !state.monthly());
    let _ = app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarModeSelected(false)),
    );
    assert!(!state.range_view_active());
}

#[test]
fn navigation_remains_reachable_at_small_widths() {
    let state = state(1);
    for width in [180.0, 260.0, 420.0, 1400.0] {
        let mut ui = render(&state, Size::new(width, 900.0));
        for label in ["2 Weeks", "Today", "Menu", "Oct 26 – Nov 8, 2026 ▾"] {
            assert!(ui.find(label).is_ok(), "{label} missing at {width}");
        }
    }
}
