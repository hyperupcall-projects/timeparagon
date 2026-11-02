use super::*;
#[test]
fn sleep_is_clickable_in_day_and_week_grids_and_obeys_its_source_toggle() {
    let mut state = State::blank();
    let mut snapshot = calendar::sample_snapshot();
    let day = snapshot.day;
    let mut event = snapshot.events[1].clone();
    event.id = "sleep".into();
    event.title = "Sleep".into();
    event.start = calendar::at_clock(day, 1, 0);
    event.end = calendar::at_clock(day, 9, 0);
    event.details.sleep = Some(calendar::sleep::Record::planned(day));
    snapshot.events = vec![event];
    state.calendar = calendar::Store::with_snapshot(snapshot);
    state.week_anchor = day;
    let metrics = Metrics::of(Size::new(420.0, 1200.0));
    for (weekly, hidden, visible) in [
        (false, false, true),
        (true, false, true),
        (false, true, false),
    ] {
        state
            .config
            .calendar
            .set_calendar_enabled(calendar::sleep::SOURCE_ID, !hidden);
        let mut view = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            metrics.size,
            day_canvas(&state, &metrics, crate::clock::now(), day, weekly),
        );
        view.point_at(Point::new(200.0, state.calendar.hour_height * 2.0));
        view.simulate([
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ]);
        assert_eq!(
            view.into_messages().any(
                |m| matches!(m, Message::Event(EventMessage::EventOpened(id)) if id == "sleep")
            ),
            visible
        );
    }
    state
        .config
        .calendar
        .set_calendar_enabled(calendar::sleep::SOURCE_ID, true);
    let mut month = iced_test::Simulator::with_size(
        iced_test::core::Settings::default(),
        metrics.size,
        monthly_view(&state, &metrics),
    );
    assert!(month.find("Sleep").is_err());
    assert!(
        state
            .calendar
            .task_list_events_on(day, &state.config.calendar)
            .is_empty()
    );
}
