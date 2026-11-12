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

#[test]
fn sleep_dragging_moves_the_whole_night_or_either_edge_on_day_and_week_grids() {
	for weekly in [false, true] {
		for edge in ["move", "start", "end"] {
			let mut state = State::blank();
			let mut snapshot = calendar::sample_snapshot();
			let day = snapshot.day;
			let mut event = snapshot.events[1].clone();
			event.id = "sleep".into();
			event.start = calendar::at_clock(day, 1, 0);
			event.end = calendar::at_clock(day, 9, 0);
			event.details.sleep = Some(calendar::sleep::Record::planned(day));
			snapshot.events = vec![event.clone()];
			state.calendar = calendar::Store::with_snapshot(snapshot);
			let metrics = Metrics::of(Size::new(420.0, 1200.0));
			let h = state.calendar.hour_height;
			let y = match edge {
				"start" => h + 2.0,
				"end" => 9.0 * h - 2.0,
				_ => 5.0 * h,
			};
			let mut ui = iced_test::Simulator::with_size(
				iced_test::core::Settings::default(),
				metrics.size,
				day_canvas(&state, &metrics, crate::clock::now(), day, weekly),
			);
			ui.point_at(Point::new(200.0, y));
			ui.simulate([iced::Event::Mouse(mouse::Event::ButtonPressed(
				mouse::Button::Left,
			))]);
			let position = Point::new(200.0, y + h / 4.0);
			ui.point_at(position);
			ui.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position })]);
			ui.simulate([iced::Event::Mouse(mouse::Event::ButtonReleased(
				mouse::Button::Left,
			))]);
			let (start, end) = ui
				.into_messages()
				.find_map(|message| match message {
					Message::Event(EventMessage::EventTimeChanged { id, start, end })
						if id == "sleep" =>
					{
						Some((start, end))
					}
					_ => None,
				})
				.expect("sleep drag must emit a time change");
			let step = chrono::Duration::minutes(15);
			assert_eq!(
				start,
				event.start
					+ if edge == "end" {
						chrono::Duration::zero()
					} else {
						step
					}
			);
			assert_eq!(
				end,
				event.end
					+ if edge == "start" {
						chrono::Duration::zero()
					} else {
						step
					}
			);
		}
	}
}

#[test]
fn sleep_scope_prompt_uses_weekday_labels_and_offers_no_all_events_choice() {
	for (label, scope) in [
		("This monday", calendar::SaveScope::Current),
		(
			"This and future mondays",
			calendar::SaveScope::CurrentAndFuture,
		),
	] {
		let mut state = State::blank();
		let mut snapshot = calendar::sample_snapshot();
		let mut event = snapshot.events[1].clone();
		event.details.sleep = Some(calendar::sleep::Record::planned(
			chrono::NaiveDate::from_ymd_opt(2030, 11, 4).unwrap(),
		));
		snapshot.events = vec![event.clone()];
		state.calendar = calendar::Store::with_snapshot(snapshot);
		state.calendar.pending_move = Some(calendar::PendingMove {
			original: event.clone(),
			scope: None,
			id: event.id,
			start: event.start,
			end: event.end,
		});
		let metrics = Metrics::of(Size::new(360.0, 640.0));
		let mut ui = iced_test::Simulator::with_size(
			iced_test::core::Settings::default(),
			metrics.size,
			move_confirmation(&state, &metrics),
		);
		assert!(ui.find("All events").is_err());
		ui.click(label).unwrap();
		assert!(ui.into_messages().any(|message| matches!(message,
            Message::Event(EventMessage::EventMoveScoped(chosen)) if chosen == scope)));
	}
}
