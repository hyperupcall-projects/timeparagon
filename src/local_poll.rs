//! Pick up database changes made by Google sync, write workers and other devices.
//! A worker checks revision counters and rereads the selected calendar range
//! through the shared repository. The UI installs that snapshot directly.
//! Reads overtaken by navigation or another local write are retried.

use std::path::PathBuf;

use iced::Task;
use timeparagon_core::calendar::repository;
use timeparagon_core::local_changes::Counters;

use crate::app::{Message, State, SystemMessage};
use crate::blocking;

/// What the poll has already applied, and whether a read is in flight.
#[derive(Debug, Default)]
pub struct Poll {
	in_flight: bool,
	/// The counters each part was last read at. `None` until the first read,
	/// so everything is read once at start.
	calendar: Option<i64>,
	habits: Option<i64>,
	alarms: Option<i64>,
	/// The notes directory's [`timeparagon_core::notes::fingerprint`].
	notes: Option<u64>,
	/// Read the local edits again even though their counter has not moved,
	/// because this process just wrote some and wants them shown at once.
	force_calendar: bool,
}

/// One poll's reads. Each part is `None` when it was not reread.
#[derive(Debug, Clone)]
pub struct Polled {
	counters: Counters,
	alarms: Option<Result<timeparagon_core::alarms::Snapshot, String>>,
	alarms_before: Vec<crate::alarms::Alarm>,
	calendar: Option<Result<repository::Read, String>>,
	calendar_dates: Vec<chrono::NaiveDate>,
	habits: Option<Result<Option<timeparagon_core::habits::Library>, String>>,
	/// The habits' and notes' local revisions when the read started.
	habits_revision: u64,
	notes_revision: u64,
	notes: Option<Result<timeparagon_core::notes::Library, String>>,
	notes_fingerprint: u64,
	/// Local edits database sync has not sent yet.
	database_unsent: usize,
}

/// Starts a poll unless one is already running.
pub fn start(state: &mut State) -> Task<Message> {
	if state.poll.in_flight {
		return Task::none();
	}
	let Some(database) = timeparagon_core::task_db::path() else {
		return Task::none();
	};
	let calendar_path: Option<PathBuf> = state.calendar.local_database().map(Into::into);
	let habits_path = state.habits.path().map(PathBuf::from);
	let notes_path = state
		.notes
		.directory()
		.filter(|_| state.notes.accepts_shared())
		.map(PathBuf::from);
	let previous = (state.poll.calendar, state.poll.habits, state.poll.notes);
	let previous_alarms = state.poll.alarms;
	let calendar_dates = state.calendar.loaded_days();
	let hour = state.calendar.day_end_hour();
	let alarms_before = state.alarms.items().to_vec();
	let force_calendar = std::mem::take(&mut state.poll.force_calendar);
	let habits_revision = state.habits.revision();
	let notes_revision = state.notes.revision();
	state.poll.in_flight = true;

	blocking::run(move || {
		let counters = match timeparagon_core::local_changes::read(&database) {
			Ok(counters) => counters,
			Err(error) => {
				log::warn!("could not read local changes: {error}");
				return None;
			}
		};
		let (calendar, habits, notes) = previous;
		let moved = |seen: Option<i64>, now: i64| seen != Some(now);
		let notes_fingerprint = notes_path
			.as_deref()
			.map_or(0, timeparagon_core::notes::fingerprint);
		Some(Polled {
			counters,
			alarms: moved(previous_alarms, counters.alarms)
				.then(|| timeparagon_core::alarms::poll(&database).map_err(|e| e.to_string())),
			alarms_before,
			calendar: calendar_path
				.filter(|_| force_calendar || moved(calendar, counters.calendar))
				.map(|path| repository::read(&path, &calendar_dates, hour)),
			calendar_dates,
			habits: habits_path
				.filter(|_| moved(habits, counters.habits))
				.map(|path| {
					timeparagon_core::task_db::read_document(
						&path,
						timeparagon_core::task_db::keys::HABITS,
					)
					.map_err(|error| error.to_string())
				}),
			habits_revision,
			notes_revision,
			notes: notes_path
				.filter(|_| notes != Some(notes_fingerprint))
				.map(|path| {
					timeparagon_core::notes::Library::read(&path).map_err(|error| error.to_string())
				}),
			notes_fingerprint,
			database_unsent: timeparagon_core::database_sync::unsent_edits(),
		})
	})
	.map(|polled| Message::System(SystemMessage::DatabasePolled(polled.map(Box::new))))
}

/// Applies a finished poll on the UI thread. Touches no files.
pub fn apply(state: &mut State, polled: Option<Box<Polled>>) -> Task<Message> {
	state.poll.in_flight = false;
	let Some(polled) = polled else {
		return Task::none();
	};
	let Polled {
		counters,
		alarms,
		alarms_before,
		calendar,
		calendar_dates,
		habits,
		habits_revision,
		notes_revision,
		notes,
		notes_fingerprint,
		database_unsent,
	} = *polled;
	state.database_unsent = database_unsent;
	state.activity.pull_database_log();
	// A write Google refused has had its local copy dropped; fetch what
	// Google has so the item shows that again.
	let refused = if state.activity.pull_google_log() {
		Task::done(Message::Calendar(
			crate::app::CalendarMessage::CalendarSyncRequested,
		))
	} else {
		Task::none()
	};

	match calendar {
		Some(Ok(read))
			if !state.poll.force_calendar && state.calendar.loaded_days() == calendar_dates =>
		{
			state.calendar.install(read, &calendar_dates);
			state.poll.calendar = Some(counters.calendar);
		}
		Some(Err(error)) => log::warn!("Could not read calendar: {error}"),
		_ => {}
	}

	if state.alarms.accepts_shared() && state.alarms.items() == alarms_before {
		match alarms {
			Some(Ok(snapshot)) => {
				if let Some(path) = timeparagon_core::task_db::path() {
					snapshot.observe(&path);
				}
				state.alarms.adopt_shared(snapshot.library.alarms);
				state.poll.alarms = Some(counters.alarms);
			}
			Some(Err(error)) => log::warn!("could not reload alarms: {error}"),
			None => {}
		}
	}

	match habits {
		// A habit changed here since the read started, or is still being
		// saved: the read may predate it, so it waits for the next tick.
		Some(_) if state.habits.revision() != habits_revision || state.habits.saving() => {}
		Some(Ok(library)) => {
			if let Some(library) = library {
				state.habits.adopt_shared(library);
			}
			state.poll.habits = Some(counters.habits);
		}
		Some(Err(error)) => log::warn!("could not reload habits: {error}"),
		None => {}
	}

	match notes {
		Some(_) if !state.notes.accepts_shared() || state.notes.revision() != notes_revision => {}
		Some(Ok(library)) => {
			state.notes.adopt_shared(library);
			state.poll.notes = Some(notes_fingerprint);
		}
		Some(Err(error)) => log::warn!("could not reload notes: {error}"),
		None => {}
	}
	// A write asked for its edits while this read was already under way.
	if state.poll.force_calendar {
		return Task::batch([refused, start(state)]);
	}
	refused
}

/// Shows this process's own local edits straight away, rather than at the
/// next tick.
pub fn refresh_calendar(state: &mut State) -> Task<Message> {
	state.poll.force_calendar = true;
	start(state)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn polled(
		state: &State,
		habits: timeparagon_core::habits::Library,
		revision: u64,
	) -> Box<Polled> {
		Box::new(Polled {
			counters: Counters {
				habits: 7,
				..Default::default()
			},
			alarms: None,
			alarms_before: state.alarms.items().to_vec(),
			calendar: None,
			calendar_dates: state.calendar.loaded_days(),
			habits: Some(Ok(Some(habits))),
			habits_revision: revision,
			notes_revision: state.notes.revision(),
			notes: None,
			notes_fingerprint: 0,
			database_unsent: 0,
		})
	}

	fn renamed(state: &State) -> timeparagon_core::habits::Library {
		let mut library = state.habits.library.clone();
		library.score_bucket = match library.score_bucket {
			timeparagon_core::habits::Bucket::Day => timeparagon_core::habits::Bucket::Week,
			_ => timeparagon_core::habits::Bucket::Day,
		};
		library
	}

	#[test]
	fn a_read_from_before_a_local_change_is_dropped_and_read_again() {
		let mut state = State::blank();
		let read_at = state.habits.revision();
		let remote = renamed(&state);
		// The user changes a habit while the read is in flight, and the save
		// even finishes before the read arrives.
		state.habits.begin_save_for_test();
		let _ = crate::habits::update(&mut state, crate::habits::Action::Persisted(Ok(())));
		assert!(!state.habits.saving());
		let local = state.habits.library.clone();
		state.poll.in_flight = true;
		let read = polled(&state, remote.clone(), read_at);
		let _ = apply(&mut state, Some(read));
		assert!(!state.poll.in_flight);
		assert_eq!(state.habits.library, local, "the newer local change stays");
		assert_eq!(state.poll.habits, None, "so the next tick reads again");
	}

	#[test]
	fn a_current_read_is_applied_and_remembered() {
		let mut state = State::blank();
		let remote = renamed(&state);
		let revision = state.habits.revision();
		let read = polled(&state, remote.clone(), revision);
		let _ = apply(&mut state, Some(read));
		assert_eq!(state.habits.library, remote);
		assert_eq!(state.poll.habits, Some(7));
	}

	#[test]
	fn a_failed_read_still_lets_the_next_tick_start() {
		let mut state = State::blank();
		state.poll.in_flight = true;
		let _ = apply(&mut state, None);
		assert!(!state.poll.in_flight);
	}
}
