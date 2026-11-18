use chrono::Duration;
use iced::{Size, window};
use iced_test::{Simulator, core::Settings};
use timeparagon::alarms::{Action, Alarm};
use timeparagon::app::{self, Message, NavigationMessage, Screen, State};
use timeparagon::config::AlarmProfile;

fn render(state: &State, width: f32) -> Simulator<'_, Message> {
	Simulator::with_size(
		Settings::default(),
		Size::new(width, 240.0),
		app::view(state, window::Id::unique()),
	)
}

#[test]
fn more_alarms_lists_pending_alarms_and_cancels_only_the_chosen_one() {
	for width in [180.0, 420.0] {
		let mut state = State::blank();
		let now = chrono::Local::now();
		let alarm = |id, label: &str, minutes| Alarm {
			id,
			label: label.to_owned(),
			at: now + Duration::minutes(minutes),
			profile: AlarmProfile::default(),
		};
		state
			.alarms
			.adopt_shared(vec![alarm(2, "Stretch", 120), alarm(1, "Tea", 60)]);
		state.more_menu_open = true;
		let mut ui = render(&state, width);
		let item = ui.find("Alarms").unwrap().bounds();
		assert!(item.y >= 0.0 && item.y + item.height <= 240.0);
		ui.click("Alarms").unwrap();
		let messages: Vec<_> = ui.into_messages().collect();
		assert!(messages.iter().any(|message| matches!(
			message,
			Message::Navigation(NavigationMessage::ScreenSelected(Screen::Alarms))
		)));
		for message in messages {
			let _ = app::update(&mut state, message);
		}
		assert_eq!(state.screen, Screen::Alarms);
		assert!(!state.more_menu_open);
		assert!(!state.alarms.mode);
		assert!(state.alarms.editor.is_none());

		let mut ui = render(&state, width);
		let tea = ui.find("Tea").unwrap().bounds();
		let stretch = ui.find("Stretch").unwrap().bounds();
		assert!(tea.y < stretch.y, "alarms are listed in time order");
		for unwanted in ["Add alarm", "Edit", "Delete", "Label", "When it goes off"] {
			assert!(ui.find(unwanted).is_err());
		}
		let cancel = ui.find("Cancel").unwrap().bounds();
		assert!(cancel.x >= 0.0 && cancel.x + cancel.width <= width);
		assert!(cancel.y >= 0.0 && cancel.y + cancel.height <= 240.0);
		ui.click("Cancel").unwrap();
		let messages: Vec<_> = ui.into_messages().collect();
		assert!(
			messages
				.iter()
				.any(|message| matches!(message, Message::Alarms(Action::Delete(1))))
		);
		for message in messages {
			let _ = app::update(&mut state, message);
		}
		assert!(state.alarms.get(1).is_none());
		assert!(state.alarms.get(2).is_some());

		let mut ui = render(&state, width);
		ui.click("Cancel").unwrap();
		let messages: Vec<_> = ui.into_messages().collect();
		for message in messages {
			let _ = app::update(&mut state, message);
		}
		assert!(state.alarms.is_empty());
		let mut ui = render(&state, width);
		assert!(ui.find("No current alarms.").is_ok());
		assert!(ui.find("Cancel").is_err());
	}
}
