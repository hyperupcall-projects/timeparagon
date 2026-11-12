//! The Routines screen, rendered headlessly: the tab reaches it, the overview
//! lists every routine, and every step of every routine draws at the widths
//! the panel is used at.

use iced::Size;
use iced_test::core::Settings;
use iced_test::{Error, Simulator};

use timeparagon::app::{self, Message, NavigationMessage, Screen, State};
use timeparagon::routines::{Action, Routine};

fn render(state: &State, size: Size) -> Simulator<'_, Message> {
	Simulator::with_size(
		Settings::default(),
		size,
		app::view(state, iced::window::Id::unique()),
	)
}

fn routines() -> State {
	let mut state = State::blank();
	let _ = app::update(
		&mut state,
		Message::Navigation(NavigationMessage::ScreenSelected(Screen::Routines)),
	);
	state
}

#[test]
fn the_routines_tab_opens_the_overview() -> Result<(), Error> {
	let state = State::blank();
	let mut ui = render(&state, Size::new(420.0, 700.0));
	let _ = ui.click("Routines")?;
	assert!(ui.into_messages().any(|message| matches!(
		message,
		Message::Navigation(NavigationMessage::ScreenSelected(Screen::Routines))
	)));

	let state = routines();
	assert_eq!(state.screen, Screen::Routines);
	let mut ui = render(&state, Size::new(420.0, 1400.0));
	for routine in Routine::ALL {
		assert!(
			ui.find(routine.title()).is_ok(),
			"{} is missing from the overview",
			routine.title()
		);
	}
	Ok(())
}

#[test]
fn starting_a_routine_from_the_overview_opens_its_first_step() -> Result<(), Error> {
	let mut state = routines();
	let mut ui = render(&state, Size::new(420.0, 1400.0));
	let _ = ui.click("Start")?;
	let messages: Vec<Message> = ui.into_messages().collect();
	let start = messages
		.into_iter()
		.find(|message| matches!(message, Message::Routines(Action::Start(_))))
		.expect("Start sends a routine on its way");
	let _ = app::update(&mut state, start);
	let flow = state.routines.flow.as_ref().expect("a routine is open");
	let mut ui = render(&state, Size::new(420.0, 700.0));
	assert!(ui.find(flow.routine.title()).is_ok());
	assert!(ui.find("Next").is_ok() || ui.find("Start").is_ok());
	assert!(ui.find("‹ Routines").is_ok());
	Ok(())
}

#[test]
fn every_step_of_every_routine_draws_at_every_width() {
	for routine in Routine::ALL {
		for width in [180.0, 260.0, 420.0] {
			let mut state = routines();
			let _ = app::update(&mut state, Message::Routines(Action::Start(routine)));
			for index in 0..routine.steps().len() {
				let _ = app::update(&mut state, Message::Routines(Action::Step(index)));
				let mut ui = render(&state, Size::new(width, 700.0));
				assert!(
					ui.find(routine.title()).is_ok(),
					"{} step {index} lost its title at {width}px",
					routine.title()
				);
				assert!(
					ui.find("Back").is_ok(),
					"{} step {index} has no way back at {width}px",
					routine.title()
				);
			}
		}
	}
}

#[test]
fn leaving_a_routine_keeps_it_to_continue() -> Result<(), Error> {
	let mut state = routines();
	let _ = app::update(
		&mut state,
		Message::Routines(Action::Start(Routine::WeeklyPlan)),
	);
	let _ = app::update(&mut state, Message::Routines(Action::Next));
	let mut ui = render(&state, Size::new(420.0, 700.0));
	let _ = ui.click("‹ Routines")?;
	for message in ui.into_messages().collect::<Vec<_>>() {
		let _ = app::update(&mut state, message);
	}
	assert!(state.routines.flow.is_none());
	let mut ui = render(&state, Size::new(420.0, 1400.0));
	assert!(ui.find("Continue").is_ok());
	Ok(())
}
