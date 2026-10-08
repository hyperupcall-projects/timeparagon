//! Days of events on disk, so the cache outlives the process.
//!
//! The in-memory cache in [`Store`](super::Store) is gone the moment the app
//! closes, which leaves a restart staring at an empty grid until its first
//! sync lands. Writing each day out to the XDG cache directory means a day
//! that was open yesterday paints as soon as it is opened today.
//!
//! Nothing in here is allowed to fail loudly: a cache that cannot be read or
//! written costs a re-sync, never the day itself.

use std::fs;
use std::io;
use std::path::Path;

use chrono::{DateTime, Local, NaiveDate};
use iced::Color;
use serde::{Deserialize, Serialize};

use super::{Details, Event};

/// One day's events as the cache file holds them.
///
/// `Event` cannot carry serde derives of its own because `iced::Color` only
/// serializes behind a feature, so the color travels as its four channels.
/// The times go through chrono's own impls, which write RFC 3339.
#[derive(Serialize, Deserialize)]
struct SavedEvent {
    id: String,
    calendar_id: String,
    title: String,
    start: DateTime<Local>,
    end: DateTime<Local>,
    all_day: bool,
    color: [f32; 4],
    completed: bool,
    #[serde(default)]
    details: Details,
}

impl From<&Event> for SavedEvent {
    fn from(event: &Event) -> Self {
        Self {
            id: event.id.clone(),
            calendar_id: event.calendar_id.clone(),
            title: event.title.clone(),
            start: event.start,
            end: event.end,
            all_day: event.all_day,
            color: [event.color.r, event.color.g, event.color.b, event.color.a],
            completed: event.completed,
            details: event.details.clone(),
        }
    }
}

impl From<SavedEvent> for Event {
    fn from(saved: SavedEvent) -> Self {
        Self {
            id: saved.id,
            calendar_id: saved.calendar_id,
            title: saved.title,
            start: saved.start,
            end: saved.end,
            all_day: saved.all_day,
            color: Color {
                r: saved.color[0],
                g: saved.color[1],
                b: saved.color[2],
                a: saved.color[3],
            },
            completed: saved.completed,
            details: saved.details,
        }
    }
}

/// The day's events as an earlier run wrote them, or nothing when no run has
/// written this day or the file turns out to be unreadable.
pub fn load(directory: &Path, day: NaiveDate) -> Option<Vec<Event>> {
    let path = directory.join(file_name(day));

    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            log::warn!("{} could not be read: {error}", path.display());
            return None;
        }
    };

    match serde_json::from_slice::<Vec<SavedEvent>>(&bytes) {
        Ok(saved) => Some(saved.into_iter().map(Event::from).collect()),
        Err(error) => {
            log::warn!(
                "{} could not be parsed, ignoring it: {error}",
                path.display()
            );
            None
        }
    }
}

/// Writes one day out, replacing whatever the file held.
///
/// The bytes go through a temporary file that is renamed into place, so a
/// crash mid-write cannot leave a truncated day behind.
pub fn save(directory: &Path, day: NaiveDate, events: &[Event]) -> io::Result<()> {
    fs::create_dir_all(directory)?;

    let saved: Vec<SavedEvent> = events.iter().map(SavedEvent::from).collect();
    let bytes = serde_json::to_vec(&saved)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let path = directory.join(file_name(day));
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)
}

/// Every day sitting in the directory. Anything not named like a day is left
/// out rather than guessed at, so a stray file survives its neighbours' trim.
pub fn days(directory: &Path) -> Vec<NaiveDate> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            log::warn!("{} could not be listed: {error}", directory.display());
            return Vec::new();
        }
    };

    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            NaiveDate::parse_from_str(name.strip_suffix(".json")?, "%Y-%m-%d").ok()
        })
        .collect()
}

/// Takes one day out of the directory. Nothing to do when it is not there.
pub fn remove(directory: &Path, day: NaiveDate) {
    let _ = fs::remove_file(directory.join(file_name(day)));
}

/// Empties the directory, so every day has to be fetched again.
pub fn clear(directory: &Path) {
    match fs::remove_dir_all(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("{} could not be cleared: {error}", directory.display()),
    }
}

/// The file a day is filed under, e.g. `2026-10-08.json`.
fn file_name(day: NaiveDate) -> String {
    format!("{day}.json")
}

/// A throwaway directory under the system's temp dir, removed when the test
/// that made it ends either way.
#[cfg(test)]
pub(super) struct Scratch(std::path::PathBuf);

#[cfg(test)]
impl Scratch {
    pub(super) fn new(name: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or_default();

        let path = std::env::temp_dir().join(format!(
            "timetracker-day-cache-{}-{unique}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("scratch directory");

        Self(path)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

#[cfg(test)]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::sample_snapshot;

    #[test]
    fn a_day_round_trips_through_the_directory() {
        let scratch = Scratch::new("round-trip");
        let snapshot = sample_snapshot();
        let day = snapshot.day;

        save(scratch.path(), day, &snapshot.events).unwrap();

        assert_eq!(load(scratch.path(), day).unwrap(), snapshot.events);
    }

    #[test]
    fn a_day_that_was_never_written_reads_as_nothing() {
        let scratch = Scratch::new("missing");

        let day = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        assert!(load(scratch.path(), day).is_none());
        assert!(days(scratch.path()).is_empty());
    }

    #[test]
    fn an_unreadable_day_is_ignored_rather_than_raised() {
        let scratch = Scratch::new("broken");
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();

        fs::write(scratch.path().join(file_name(day)), b"not json").unwrap();

        assert!(load(scratch.path(), day).is_none());
        assert_eq!(days(scratch.path()), vec![day], "it is still a day file");
    }

    #[test]
    fn only_files_named_like_a_day_are_listed() {
        let scratch = Scratch::new("listing");
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();

        save(scratch.path(), day, &[]).unwrap();
        fs::write(scratch.path().join("notes.txt"), b"mine").unwrap();
        fs::write(scratch.path().join("2026-10-09.json.tmp"), b"half a day").unwrap();

        assert_eq!(days(scratch.path()), vec![day]);
    }

    #[test]
    fn clearing_takes_every_day_with_it() {
        let scratch = Scratch::new("clearing");
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        save(scratch.path(), day, &[]).unwrap();

        clear(scratch.path());

        assert!(days(scratch.path()).is_empty());
        assert!(load(scratch.path(), day).is_none());
    }
}
