use iced::futures::{StreamExt, executor::block_on};
use iced::{Size, keyboard, window};
use iced_test::{Simulator, core::Settings};
use timeparagon::app::{self, Message, State, SystemMessage};

const SCHEMA: &str = "Unsupported database schema 7; expected 6";

fn update(state: &mut State, message: Message) {
	let _ = app::update(state, message);
}

#[test]
fn every_global_error_opens_a_dialog_in_each_window() {
	for source in 0..7 {
		let mut state = State::blank();
		let field = match source {
			0 => &mut state.config_error,
			1 => &mut state.calendar.error,
			2 => &mut state.inbox.error,
			3 => &mut state.routines.error,
			4 => &mut state.habits.error,
			5 => &mut state.notes.error,
			_ => &mut state.accounts.error,
		};
		*field = Some(SCHEMA.into());
		update(&mut state, Message::Ignored);
		let main = window::Id::unique();
		let editor = window::Id::unique();
		state.main_window = Some(main);
		state.editor_window = Some(editor);
		for window in [main, editor] {
			for size in [Size::new(180.0, 240.0), Size::new(900.0, 700.0)] {
				let mut ui =
					Simulator::with_size(Settings::default(), size, app::view(&state, window));
				assert!(ui.find(SCHEMA).is_ok(), "source {source}");
				let close = ui.find("Close").unwrap().bounds();
				assert!(close.y >= 0.0 && close.y + close.height <= size.height);
				ui.click("Close").unwrap();
				assert!(ui.into_messages().any(|m| matches!(m, Message::CloseError)));
			}
		}
		update(&mut state, Message::CloseError);
		update(&mut state, Message::Ignored);
		for window in [main, editor] {
			let mut ui = Simulator::with_size(
				Settings::default(),
				Size::new(480.0, 700.0),
				app::view(&state, window),
			);
			assert!(
				ui.find(SCHEMA).is_err(),
				"dismissed source {source} reopened"
			);
		}
	}
}

#[test]
fn global_errors_are_above_existing_dialogs_and_block_background_input() {
	let mut state = State::blank();
	update(
		&mut state,
		Message::Inbox(timeparagon::inbox::Action::NewTask),
	);
	state.config_error = Some(SCHEMA.into());
	update(&mut state, Message::Ignored);
	let mut ui = Simulator::with_size(
		Settings::default(),
		Size::new(480.0, 700.0),
		app::view(&state, window::Id::unique()),
	);
	assert!(ui.find(SCHEMA).is_ok());
	ui.typewrite("background must not change");
	ui.click("Close").unwrap();
	let messages: Vec<_> = ui.into_messages().collect();
	assert!(messages.iter().any(|m| matches!(m, Message::CloseError)));
	assert!(!messages.iter().any(|m| matches!(m, Message::Inbox(_))));
	drop(messages);
	update(
		&mut state,
		Message::System(SystemMessage::KeyPressed(
			keyboard::Key::Named(keyboard::key::Named::Escape),
			keyboard::Modifiers::default(),
		)),
	);
	assert!(
		state.inbox.draft.is_some(),
		"Escape closed the background form"
	);
	let mut ui = Simulator::with_size(
		Settings::default(),
		Size::new(480.0, 700.0),
		app::view(&state, window::Id::unique()),
	);
	assert!(ui.find(SCHEMA).is_err());
}

#[test]
fn dock_errors_leave_the_hidden_surface_empty_and_the_open_form_usable() {
	for source in 0..7 {
		let mut state = State::blank();
		let field = match source {
			0 => &mut state.config_error,
			1 => &mut state.calendar.error,
			2 => &mut state.inbox.error,
			3 => &mut state.routines.error,
			4 => &mut state.habits.error,
			5 => &mut state.notes.error,
			_ => &mut state.accounts.error,
		};
		*field = Some(SCHEMA.into());
		// Startup can capture an error before the dock's editor mode is set.
		update(&mut state, Message::Ignored);
		state.task_creation_window = true;
		for open in [false, true] {
			if open {
				update(
					&mut state,
					Message::Inbox(timeparagon::inbox::Action::NewTask),
				);
				state.calendar.error = Some("Calendar sync timed out".into());
				update(&mut state, Message::Ignored);
			}
			let mut ui = Simulator::with_size(
				Settings::default(),
				Size::new(440.0, 900.0),
				app::view(&state, window::Id::unique()),
			);
			assert!(ui.find(SCHEMA).is_err(), "source {source}, open {open}");
			assert!(ui.find("Calendar sync timed out").is_err());
			assert!(ui.find("Error").is_err());
			if open {
				ui.click("Cancel").unwrap();
			} else {
				assert!(ui.find("Cancel").is_err());
			}
			ui.simulate([iced::Event::Window(window::Event::RedrawRequested(
				iced::time::Instant::now(),
			))]);
			let messages: Vec<_> = ui.into_messages().collect();
			if open {
				assert!(
					messages
						.iter()
						.any(|m| matches!(m, Message::Inbox(timeparagon::inbox::Action::Cancel)))
				);
			} else {
				assert!(
					!messages
						.iter()
						.any(|m| matches!(m, Message::System(SystemMessage::TaskPopoverBounds(_))))
				);
			}
		}
		// Suppressed errors must not consume Escape and leave the form open.
		let task = app::update(
			&mut state,
			Message::System(SystemMessage::KeyPressed(
				keyboard::Key::Named(keyboard::key::Named::Escape),
				keyboard::Modifiers::default(),
			)),
		);
		let actions = block_on(
			iced_test::runtime::task::into_stream(task)
				.expect("Escape should cancel the form")
				.collect::<Vec<_>>(),
		);
		assert!(
			actions.iter().any(|action| matches!(
				action,
				iced_test::runtime::Action::Output(Message::Inbox(
					timeparagon::inbox::Action::Cancel
				))
			)),
			"source {source}"
		);
	}
}

#[test]
fn changed_and_recurring_errors_are_shown_after_dismissal() {
	let mut state = State::blank();
	state.config_error = Some(SCHEMA.into());
	update(&mut state, Message::Ignored);
	update(&mut state, Message::CloseError);
	assert_eq!(state.config_error.as_deref(), Some(SCHEMA));
	state.config_error = None;
	update(&mut state, Message::Ignored);
	state.config_error = Some(SCHEMA.into());
	state.calendar.error = Some("Could not sync calendar".into());
	update(&mut state, Message::Ignored);
	let mut ui = Simulator::with_size(
		Settings::default(),
		Size::new(260.0, 400.0),
		app::view(&state, window::Id::unique()),
	);
	assert!(
		ui.find(format!("{SCHEMA}\n\nCould not sync calendar"))
			.is_ok()
	);
}
