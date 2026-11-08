//! Picking up what other processes changed in the local database.
//!
//! The Turso worker, the Google queue, the dock and the phone's synced edits
//! all write to the database behind the panel's back, so the panel looks every
//! couple of seconds. That look used to happen on the UI thread and reread
//! everything each time: the queued edits for every cached day, the whole
//! habits and notes documents, and the queue. Now a worker thread reads the
//! change counters (see [`timeslay_core::local_changes`]) and rereads only
//! what moved, and the UI thread only applies the result.
//!
//! Because the read happens off the UI thread, the user can change something
//! while it is in flight. A read is therefore only applied when nothing it
//! could overwrite has changed locally since it started; otherwise it is
//! dropped and read again on the next tick.

use std::path::PathBuf;

use iced::Task;
use timeslay_core::calendar::operations::{self, Operation, Overlay};
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
    queue: Option<i64>,
    habits: Option<i64>,
    notes: Option<i64>,
    /// Read the queued edits again even though their counter has not moved,
    /// because this process just wrote some and wants them shown at once.
    force_overlay: bool,
}

/// One poll's reads. Each part is `None` when it was not reread.
#[derive(Debug, Clone)]
pub struct Polled {
    counters: Counters,
    overlay: Option<Result<Overlay, String>>,
    operations: Option<Result<Vec<Operation>, String>>,
    habits: Option<Result<Option<timeslay_core::habits::Library>, String>>,
    /// The habits' and notes' local revisions when the read started.
    habits_revision: u64,
    notes_revision: u64,
    notes: Option<Result<Option<timeslay_core::notes::Library>, String>>,
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
        .shared_path()
        .filter(|_| state.notes.accepts_shared())
        .map(PathBuf::from);
    let previous = (
        state.poll.overlay,
        state.poll.queue,
        state.poll.habits,
        state.poll.notes,
    );
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
        let (overlay, queue, habits, notes) = previous;
        let moved = |seen: Option<i64>, now: i64| seen != Some(now);
        Some(Polled {
            counters,
            overlay: overlay_path
                .filter(|_| force_overlay || moved(overlay, counters.overlay))
                .map(|path| operations::load_overlay(&path)),
            operations: moved(queue, counters.queue).then(|| operations::list_in(&database)),
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
                .filter(|_| moved(notes, counters.notes))
                .map(|path| {
                    timeslay_core::task_db::read_document(
                        &path,
                        timeslay_core::task_db::keys::NOTES,
                    )
                    .map_err(|error| error.to_string())
                }),
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
        overlay,
        operations,
        habits,
        habits_revision,
        notes_revision,
        notes,
    } = *polled;

    match overlay {
        Some(Ok(overlay)) => {
            state.calendar.apply_overlay(&overlay);
            state.poll.overlay = Some(counters.overlay);
        }
        Some(Err(error)) => log::warn!("Could not load committed edits: {error}"),
        None => {}
    }

    if let Some(operations) = operations {
        if operations.is_ok() {
            state.poll.queue = Some(counters.queue);
        }
        state.operations = operations;
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
            if let Some(library) = library {
                state.notes.adopt_shared(library);
            }
            state.poll.notes = Some(counters.notes);
        }
        Some(Err(error)) => log::warn!("could not reload notes: {error}"),
        None => {}
    }
    // A write asked for its edits while this read was already under way.
    if state.poll.force_overlay {
        return start(state);
    }
    Task::none()
}

/// Shows this process's own queued edits straight away, rather than at the
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
            overlay: None,
            operations: None,
            habits: Some(Ok(Some(habits))),
            habits_revision: revision,
            notes_revision: state.notes.revision(),
            notes: None,
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
