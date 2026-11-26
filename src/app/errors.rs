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
	// The dock keeps its task editor alive while hidden. Background failures
	// belong in the log, not in a modal on that otherwise invisible surface.
	let show_dialog = !state.task_creation_window;
	if !show_dialog {
		state.errors.close();
	}
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
			if show_dialog && !state.errors.messages.contains(message) {
				state.errors.messages.push(message.clone());
			}
		}
	}
}
