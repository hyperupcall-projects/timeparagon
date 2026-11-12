//! Calendar selection and multiline forms exercised through the real widget tree.
use iced::{Size, widget::text_editor};
use iced_test::{Simulator, core::Settings};
use timeparagon::app::{EventDraftMessage, EventMessage};
use timeparagon::{
	app::{self, Message, State},
	ui,
};

fn state() -> State {
	State::blank()
}

#[test]
fn task_dates_open_calendar_and_day_buttons_select_real_dates() {
	for width in [200.0, 320.0, 720.0] {
		let mut state = state();
		let _ = app::update(
			&mut state,
			Message::Inbox(timeparagon::inbox::Action::NewTask),
		);
		let mut form: Simulator<'_, Message> = Simulator::with_size(
			Settings::default(),
			Size::new(width, 700.0),
			ui::view(&state),
		);
		form.click("Choose date").unwrap();
		let open = form
			.into_messages()
			.find(|m| matches!(m, Message::Picker(_)))
			.unwrap();
		let _ = app::update(&mut state, open);
		let mut calendar = Simulator::with_size(
			Settings::default(),
			Size::new(width, 700.0),
			ui::view(&state),
		);
		calendar.click("15").unwrap();
		assert!(calendar.into_messages().any(|m| matches!(m,Message::Picker(ui::controls::Action::Select(day)) if chrono::Datelike::day(&day)==15)));
	}
}

#[test]
fn multiline_descriptions_survive_editing_and_habit_handoff() {
	let mut state = state();
	let now = chrono::Local::now();
	state.calendar.draft = Some(timeparagon::calendar::Draft::new(
		"calendar".into(),
		now,
		now + chrono::Duration::hours(1),
	));
	let _ = app::update(
		&mut state,
		Message::EventDraft(EventDraftMessage::EventDraftDescriptionChanged(
			"First line\nSecond line".into(),
		)),
	);
	assert_eq!(
		state.form_editors.event_draft.text(),
		"First line\nSecond line"
	);
	let _ = app::update(
		&mut state,
		Message::Event(EventMessage::FormEdited(
			timeparagon::app::FormField::EventDraft,
			text_editor::Action::Edit(text_editor::Edit::Insert('!')),
		)),
	);
	let expected = state.form_editors.event_draft.text();
	assert!(expected.contains('\n'));
	assert_eq!(state.calendar.draft.as_ref().unwrap().description, expected);
	let _ = app::update(
		&mut state,
		Message::EventDraft(EventDraftMessage::EventDraftKindSelected(
			timeparagon::calendar::DraftKind::Habit,
		)),
	);
	assert_eq!(state.form_editors.event_edit.text(), expected);
	assert_eq!(
		state.calendar.edit_draft.as_ref().unwrap().description,
		expected
	);
	let id = state.calendar.open_event.clone().unwrap();
	let _ = app::update(
		&mut state,
		Message::Event(EventMessage::DescriptionOpen(id)),
	);
	assert_eq!(state.description_editor.text(), expected);
}

#[test]
fn time_picker_retains_the_other_segment() {
	for (x, expected) in [(40.0, "22:47"), (160.0, "23:46")] {
		let control = ui::controls::time(
			timeparagon::calendar::time_format::TimeFormat::TwentyFourHour,
			"23:47",
			true,
			13.0,
			|v0| Message::EventDraft(EventDraftMessage::EventDraftStartChanged(v0)),
		);
		let mut form: Simulator<'_, Message> = Simulator::with_size(
			Settings::default(),
			Size::new(320.0, 700.0),
			iced::widget::container(control).width(200),
		);
		form.point_at(iced::Point::new(x, 15.0));
		form.simulate([
			iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(
				iced::keyboard::Modifiers::CTRL,
			)),
			iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
				delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
			}),
		]);
		assert!(
            form.into_messages()
                .any(|m| matches!(m, Message::EventDraft(EventDraftMessage::EventDraftStartChanged(value)) if value==expected))
        );
	}
}

#[test]
fn twelve_hour_picker_keeps_noon_and_midnight_unambiguous() {
	for (value, x, expected) in [
		("14:47", 30.0, "13:47"),
		("12:47", 30.0, "23:47"),
		("00:47", 30.0, "11:47"),
		("13:47", 160.0, "13:46"),
		("12:47", 260.0, "00:47"),
	] {
		let control = ui::controls::time(
			timeparagon::calendar::time_format::TimeFormat::TwelveHour,
			value,
			true,
			13.0,
			|v0| Message::EventDraft(EventDraftMessage::EventDraftStartChanged(v0)),
		);
		let mut form: Simulator<'_, Message> = Simulator::with_size(
			Settings::default(),
			Size::new(320.0, 700.0),
			iced::widget::container(control).width(300),
		);
		form.point_at(iced::Point::new(x, 15.0));
		form.simulate([
			iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(
				iced::keyboard::Modifiers::CTRL,
			)),
			iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
				delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
			}),
		]);
		assert!(
            form.into_messages().any(
                |m| matches!(m, Message::EventDraft(EventDraftMessage::EventDraftStartChanged(actual)) if actual == expected)
            ),
            "{value} -> {expected}"
        );
	}
}

#[test]
fn compact_picker_displays_short_periods_and_preserves_stored_time() {
	let control = ui::controls::time(
		timeparagon::calendar::time_format::TimeFormat::Compact,
		"13:30",
		true,
		13.0,
		|v0| Message::EventDraft(EventDraftMessage::EventDraftStartChanged(v0)),
	);
	let mut form: Simulator<'_, Message> = Simulator::with_size(
		Settings::default(),
		Size::new(320.0, 700.0),
		iced::widget::container(control).width(300),
	);
	assert!(form.find(":").is_ok());
	form.point_at(iced::Point::new(260.0, 15.0));
	form.simulate([
		iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(
			iced::keyboard::Modifiers::CTRL,
		)),
		iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
			delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
		}),
	]);
	assert!(
		form.into_messages().any(|m| matches!(m,
        Message::EventDraft(EventDraftMessage::EventDraftStartChanged(value)) if value == "01:30"))
	);
}
