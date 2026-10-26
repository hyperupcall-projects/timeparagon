//! Days of events on disk, so the cache outlives the process.
//!
//! The in-memory cache in [`Store`](super::Store) is gone the moment the app
//! closes, which leaves a restart staring at an empty grid until its first
//! sync lands. Writing each month out to the XDG cache directory means a day
//! that was open yesterday paints as soon as it is opened today, and stepping
//! to another day in the same month reads from the same file instead of
//! refetching.
//!
//! Nothing in here is allowed to fail loudly: a cache that cannot be read or
//! written costs a re-sync, never the day itself.

use std::fs;
use std::io;
use std::path::Path;

use chrono::{DateTime, Datelike, Local, NaiveDate};
use iced::Color;
use serde::{Deserialize, Serialize};

use super::{day_at, Details, Event};

/// One month's events as the cache file holds them.
///
/// `Event` cannot carry serde derives of its own because `iced::Color` only
/// serializes behind a feature, so the color travels as its four channels.
/// The times go through chrono's own impls, which write RFC 3339.
#[derive(Serialize, Deserialize)]
struct SavedMonth {
    events: Vec<SavedEvent>,
}

/// One event as the cache file holds it.
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
/// written this month or the file turns out to be unreadable. Only events
/// whose start falls within `day`'s window (set by `day_end_hour`) are
/// returned.
pub fn load(directory: &Path, day: NaiveDate, day_end_hour: u32) -> Option<Vec<Event>> {
    let path = directory.join(file_name(day));

    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            log::warn!("{} could not be read: {error}", path.display());
            return None;
        }
    };

    match serde_json::from_slice::<SavedMonth>(&bytes) {
        Ok(saved) => {
            let database = crate::config::project_dirs()
                .map(|dirs| dirs.data_local_dir().join("tasks.sqlite3"))
                .filter(|path| path.exists())
                .and_then(|path| crate::task_db::Database::open(&path).map_err(|e| log::warn!("Cannot load local task data: {e}")).ok());
            let events: Vec<Event> = saved
                .events
                .into_iter()
                .map(Event::from)
                .map(|mut event| {
                    if event.is_task() {
                        if let Some(database) = &database {
                            match database.lookup(&event) {
                                Ok(Some(data)) => {
                                    event.details.task_data = Some(data);
                                    event.details.description = crate::timeblocking::description(&event.details.description);
                                }
                                Err(error) => log::warn!("Cannot load local task data: {error}"),
                                _ => {}
                            }
                        }
                    }
                    event
                })
                .filter(|event| day_at(event.start, day_end_hour) == day)
                .collect();
            Some(events)
        }
        Err(error) => {
            log::warn!(
                "{} could not be parsed, ignoring it: {error}",
                path.display()
            );
            None
        }
    }
}

/// Writes one day's events into the month file, replacing whatever that day
/// held. The whole month is written back out, so a crash mid-write cannot
/// leave a truncated month behind.
pub fn save(
    directory: &Path,
    day: NaiveDate,
    events: &[Event],
    day_end_hour: u32,
) -> io::Result<()> {
    fs::create_dir_all(directory)?;

    let path = directory.join(file_name(day));

    // Load the existing month, or start empty.
    let mut month: SavedMonth = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(SavedMonth {
            events: Vec::new(),
        }),
        Err(_) => SavedMonth {
            events: Vec::new(),
        },
    };

    // Drop any events already stored for `day`, then add the new ones.
    month.events.retain(|saved| day_at(saved.start, day_end_hour) != day);
    month.events.extend(events.iter().map(SavedEvent::from));

    let bytes = serde_json::to_vec(&month)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)
}

/// Every month sitting in the directory, as the first day of each month.
/// Anything not named like a month is left out rather than guessed at, so a
/// stray file survives its neighbours' trim.
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
            let stem = name.strip_suffix(".json")?;
            let mut parts = stem.split('-');
            let year = parts.next()?.parse::<i32>().ok()?;
            let month = parts.next()?.parse::<u32>().ok()?;
            if parts.next().is_some() {
                return None;
            }
            NaiveDate::from_ymd_opt(year, month, 1)
        })
        .collect()
}

/// Takes one month out of the directory. Nothing to do when it is not there.
pub fn remove(directory: &Path, day: NaiveDate) {
    let _ = fs::remove_file(directory.join(file_name(day)));
}

/// Empties the directory, so every month has to be fetched again.
pub fn clear(directory: &Path) {
    match fs::remove_dir_all(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => log::warn!("{} could not be cleared: {error}", directory.display()),
    }
}

/// The file a month is filed under, e.g. `2026-10.json`.
fn file_name(day: NaiveDate) -> String {
    format!("{:04}-{:02}.json", day.year(), day.month())
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
            "timetracker-month-cache-{}-{unique}-{name}",
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
    use chrono::TimeZone;
    use crate::calendar::sample_snapshot;

    #[test]
    fn a_month_round_trips_through_the_directory() {
        let scratch = Scratch::new("round-trip");
        let snapshot = sample_snapshot();
        let day = snapshot.day;

        save(scratch.path(), day, &snapshot.events, 0).unwrap();

        assert_eq!(load(scratch.path(), day, 0).unwrap(), snapshot.events);
    }

    #[test]
    fn a_day_that_was_never_written_reads_as_nothing() {
        let scratch = Scratch::new("missing");
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        assert!(load(scratch.path(), day, 0).is_none());
        assert!(days(scratch.path()).is_empty());
    }

    #[test]
    fn an_unreadable_month_is_ignored_rather_than_raised() {
        let scratch = Scratch::new("broken");
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();

        fs::write(scratch.path().join(file_name(day)), b"not json").unwrap();

        assert!(load(scratch.path(), day, 0).is_none());
        assert_eq!(days(scratch.path()), vec![day.with_day(1).unwrap()], "it is still a month file");
    }

    #[test]
    fn only_files_named_like_a_month_are_listed() {
        let scratch = Scratch::new("listing");
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();

        save(scratch.path(), day, &[], 0).unwrap();
        fs::write(scratch.path().join("notes.txt"), b"mine").unwrap();
        fs::write(scratch.path().join("2026-10.json.tmp"), b"half a month").unwrap();

        assert_eq!(days(scratch.path()), vec![day.with_day(1).unwrap()]);
    }

    #[test]
    fn clearing_takes_every_month_with_it() {
        let scratch = Scratch::new("clearing");
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        save(scratch.path(), day, &[], 0).unwrap();

        clear(scratch.path());

        assert!(days(scratch.path()).is_empty());
        assert!(load(scratch.path(), day, 0).is_none());
    }

    #[test]
    fn saving_a_day_keeps_other_days_in_the_same_month() {
        let scratch = Scratch::new("same-month");
        let oct_8 = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        let oct_9 = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();

        let event_a = Event {
            id: "a".to_owned(),
            calendar_id: "cal".to_owned(),
            title: "A".to_owned(),
            start: Local
                .with_ymd_and_hms(2026, 10, 8, 10, 0, 0)
                .single()
                .unwrap(),
            end: Local
                .with_ymd_and_hms(2026, 10, 8, 11, 0, 0)
                .single()
                .unwrap(),
            all_day: false,
            color: iced::Color::WHITE,
            completed: false,
            details: Details::default(),
        };
        let event_b = Event {
            id: "b".to_owned(),
            calendar_id: "cal".to_owned(),
            title: "B".to_owned(),
            start: Local
                .with_ymd_and_hms(2026, 10, 9, 10, 0, 0)
                .single()
                .unwrap(),
            end: Local
                .with_ymd_and_hms(2026, 10, 9, 11, 0, 0)
                .single()
                .unwrap(),
            all_day: false,
            color: iced::Color::WHITE,
            completed: false,
            details: Details::default(),
        };

        save(scratch.path(), oct_8, &[event_a.clone()], 0).unwrap();
        save(scratch.path(), oct_9, &[event_b.clone()], 0).unwrap();

        assert_eq!(load(scratch.path(), oct_8, 0).unwrap(), vec![event_a]);
        assert_eq!(load(scratch.path(), oct_9, 0).unwrap(), vec![event_b]);
    }

    #[test]
    fn overwriting_a_day_replaces_its_events() {
        let scratch = Scratch::new("overwrite");
        let oct_8 = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();

        let event_a = Event {
            id: "a".to_owned(),
            calendar_id: "cal".to_owned(),
            title: "A".to_owned(),
            start: Local
                .with_ymd_and_hms(2026, 10, 8, 10, 0, 0)
                .single()
                .unwrap(),
            end: Local
                .with_ymd_and_hms(2026, 10, 8, 11, 0, 0)
                .single()
                .unwrap(),
            all_day: false,
            color: iced::Color::WHITE,
            completed: false,
            details: Details::default(),
        };
        let event_b = Event {
            id: "b".to_owned(),
            calendar_id: "cal".to_owned(),
            title: "B".to_owned(),
            start: Local
                .with_ymd_and_hms(2026, 10, 8, 14, 0, 0)
                .single()
                .unwrap(),
            end: Local
                .with_ymd_and_hms(2026, 10, 8, 15, 0, 0)
                .single()
                .unwrap(),
            all_day: false,
            color: iced::Color::WHITE,
            completed: false,
            details: Details::default(),
        };

        save(scratch.path(), oct_8, &[event_a.clone()], 0).unwrap();
        save(scratch.path(), oct_8, &[event_b.clone()], 0).unwrap();

        assert_eq!(load(scratch.path(), oct_8, 0).unwrap(), vec![event_b]);
    }
}
