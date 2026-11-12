//! Picking up what other processes changed in the local database.
//!
//! The Turso worker, writes to Google, the dock and the phone's synced edits
//! all write to the database behind the panel's back, so the panel looks every
//! couple of seconds. That look used to happen on the UI thread and reread
//! everything each time: the local edits for every cached day, and the whole
//! habits and notes. Now a worker thread reads the change
//! counters (see [`timeslay_core::local_changes`]) and a fingerprint of the
//! notes directory, rereads only what moved, and the UI thread only applies
//! the result.
//!
//! Because the read happens off the UI thread, the user can change something
//! while it is in flight. A read is therefore only applied when nothing it
//! could overwrite has changed locally since it started; otherwise it is
//! dropped and read again on the next tick.

use std::path::PathBuf;

use iced::Task;
use timeslay_core::calendar::operations::{self, Overlay};
use timeslay_core::local_changes::Counters;

use crate::app::{Message, State, SystemMessage};
use crate::blocking;

/// What the poll has already applied, and whether a read is in flight.
#[derive(Debug, Default)]
pub struct Poll {
	in_flight: bool,
	/// The counters each part was last read at. `None` until the first read,
	/// so everything is read once at start.
	overlay: Option<i64>,
	habits: Option<i64>,
	alarms: Option<i64>,
	sleep: Option<i64>,
	/// The notes directory's [`timeslay_core::notes::fingerprint`].
	notes: Option<u64>,
	/// Read the local edits again even though their counter has not moved,
	/// because this process just wrote some and wants them shown at once.
	force_overlay: bool,
}

/// One poll's reads. Each part is `None` when it was not reread.
#[derive(Debug, Clone)]
pub struct Polled {
	counters: Counters,
	alarms: Option<Result<timeslay_core::alarms::Snapshot, String>>,
	alarms_before: Vec<crate::alarms::Alarm>,
	overlay: Option<Result<Overlay, String>>,
	/// The stored nights of the days in memory, unsent edits applied.
	nights: Option<Result<Vec<timeslay_core::calendar::Event>, String>>,
	habits: Option<Result<Option<timeslay_core::habits::Library>, String>>,
	/// The habits' and notes' local revisions when the read started.
	habits_revision: u64,
	notes_revision: u64,
	notes: Option<Result<timeslay_core::notes::Library, String>>,
	notes_fingerprint: u64,
	/// Local edits database sync has not sent yet.
	database_unsent: usize,
}

/// Starts a poll unless one is already running.
pub fn start(state: &mut State) -> Task<Message> {
	if state.poll.in_flight {
		return Task::none();
	}
	let Some(database) = timeslay_core::task_db::path() else {
		return Task::none();
	};
	let overlay_path: Option<PathBuf> = state.calendar.local_database().map(Into::into);
	let habits_path = state.habits.path().map(PathBuf::from);
	let notes_path = state
		.notes
		.directory()
		.filter(|_| state.notes.accepts_shared())
		.map(PathBuf::from);
	let previous = (state.poll.overlay, state.poll.habits, state.poll.notes);
	let previous_alarms = state.poll.alarms;
	let previous_sleep = state.poll.sleep;
	// Nights on every day in memory, should another device have changed one.
	let nights_path = overlay_path.clone();
	let loaded = state.calendar.loaded_days();
	let nights_range = loaded.first().copied().zip(loaded.last().copied());
	let alarms_before = state.alarms.items().to_vec();
	let force_overlay = std::mem::take(&mut state.poll.force_overlay);
	let habits_revision = state.habits.revision();
	let notes_revision = state.notes.revision();
	state.poll.in_flight = true;

	blocking::run(move || {
		let counters = match timeslay_core::local_changes::read(&database) {
			Ok(counters) => counters,
			Err(error) => {
				log::warn!("could not read local changes: {error}");
				return None;
			}
		};
		let (overlay, habits, notes) = previous;
		let moved = |seen: Option<i64>, now: i64| seen != Some(now);
		let notes_fingerprint = notes_path
			.as_deref()
			.map_or(0, timeslay_core::notes::fingerprint);
		Some(Polled {
			counters,
			alarms: moved(previous_alarms, counters.alarms)
				.then(|| timeslay_core::alarms::poll(&database).map_err(|e| e.to_string())),
			alarms_before,
			overlay: overlay_path
				.filter(|_| force_overlay || moved(overlay, counters.overlay))
				.map(|path| operations::load_overlay(&path)),
			nights: nights_path
				.zip(nights_range)
				.filter(|_| moved(previous_sleep, counters.sleep))
				.map(|(path, (first, last))| {
					timeslay_core::calendar::sleep::stored_nights(&path, first, last)
				}),
			habits: habits_path
				.filter(|_| moved(habits, counters.habits))
				.map(|path| {
					timeslay_core::task_db::read_document(
						&path,
						timeslay_core::task_db::keys::HABITS,
					)
					.map_err(|error| error.to_string())
				}),
			habits_revision,
			notes_revision,
			notes: notes_path
				.filter(|_| notes != Some(notes_fingerprint))
				.map(|path| {
					timeslay_core::notes::Library::read(&path).map_err(|error| error.to_string())
				}),
			notes_fingerprint,
			database_unsent: timeslay_core::database_sync::unsent_edits(),
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
		overlay,
		nights,
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

	match overlay {
		Some(Ok(overlay)) => {
			state.calendar.apply_overlay(&overlay);
			state.poll.overlay = Some(counters.overlay);
		}
		Some(Err(error)) => log::warn!("Could not load committed edits: {error}"),
		None => {}
	}

	// A night changed in the database, perhaps by another device through
	// database sync: show it now rather than at the next Google sync. The
	// read carried the unsent edits of its moment; reading them again next
	// tick puts back any made since.
	match nights {
		Some(Ok(nights)) => {
			state.calendar.apply_stored_nights(&nights);
			state.poll.sleep = Some(counters.sleep);
			state.poll.force_overlay = true;
		}
		Some(Err(error)) => log::warn!("Could not load stored sleep: {error}"),
		None => {}
	}

	// A whole series was saved or deleted and Google has confirmed it. Its
	// occurrences on other cached days are swapped in, and the days in view
	// are fetched again: the sync asked for when saving may have reached
	// Google before the write did.
	let series = timeslay_core::calendar::series::take_refreshed();
	let refused = if series.is_empty() {
		refused
	} else {
		for refresh in &series {
			state.calendar.apply_series_refresh(refresh);
		}
		Task::batch([refused, crate::app::refresh_week_after_write(state)])
	};

	if state.alarms.accepts_shared() && state.alarms.items() == alarms_before {
		match alarms {
			Some(Ok(snapshot)) => {
				if let Some(path) = timeslay_core::task_db::path() {
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
	if state.poll.force_overlay {
		return Task::batch([refused, start(state)]);
	}
	refused
}

/// Shows this process's own local edits straight away, rather than at the
/// next tick.
pub fn refresh_overlay(state: &mut State) -> Task<Message> {
	state.poll.force_overlay = true;
	start(state)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn polled(state: &State, habits: timeslay_core::habits::Library, revision: u64) -> Box<Polled> {
		Box::new(Polled {
			counters: Counters {
				habits: 7,
				..Default::default()
			},
			alarms: None,
			alarms_before: state.alarms.items().to_vec(),
			overlay: None,
			nights: None,
			habits: Some(Ok(Some(habits))),
			habits_revision: revision,
			notes_revision: state.notes.revision(),
			notes: None,
			notes_fingerprint: 0,
			database_unsent: 0,
		})
	}

	fn renamed(state: &State) -> timeslay_core::habits::Library {
		let mut library = state.habits.library.clone();
		library.score_bucket = match library.score_bucket {
			timeslay_core::habits::Bucket::Day => timeslay_core::habits::Bucket::Week,
			_ => timeslay_core::habits::Bucket::Day,
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
