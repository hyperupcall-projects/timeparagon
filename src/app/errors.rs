//! Global errors retain their source state, but are announced only when changed.
use super::State;

#[derive(Debug, Default)]
pub(crate) struct Errors {
	seen: [Option<String>; 7],
	pub messages: Vec<String>,
}

impl Errors {
	pub fn is_open(&self) -> bool {
		!self.messages.is_empty()
	}

	pub fn close(&mut self) {
		self.messages.clear();
	}
}

pub(crate) fn capture(state: &mut State) {
	let sources = [
		&state.config_error,
		&state.calendar.error,
		&state.inbox.error,
		&state.routines.error,
		&state.habits.error,
		&state.notes.error,
		&state.accounts.error,
	];
	let mut announced = Vec::new();
	for (seen, source) in state.errors.seen.iter_mut().zip(sources) {
		if seen == source {
			continue;
		}
		seen.clone_from(source);
		if let Some(message) = source {
			if !announced.contains(message) {
				log::error!("{message}");
				announced.push(message.clone());
			}
			if !state.errors.messages.contains(message) {
				state.errors.messages.push(message.clone());
			}
		}
	}
}
