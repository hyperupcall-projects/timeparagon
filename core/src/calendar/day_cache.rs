//! Days of events in the database, so the cache outlives the process.
//!
//! The in-memory cache in [`Store`](super::Store) is gone the moment the app
//! closes, which leaves a restart staring at an empty grid until its first
//! sync lands. Saving each month as a row means a day that was open yesterday
//! paints as soon as it is opened today, and stepping to another day in the
//! same month reads from the same row instead of refetching.
//!
//! Nothing in here is allowed to fail loudly: a cache that cannot be read or
//! written costs a re-sync, never the day itself.

use std::io;
use std::path::Path;

use chrono::{DateTime, Datelike, Local, NaiveDate};
use iced_core::Color;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use super::{Calendar, Details, Event, day_at};
use crate::task_db::{self, Database, keys};

/// One month's events as the cache row holds them.
///
/// `Event` cannot carry serde derives of its own because `iced_core::Color` only
/// serializes behind a feature, so the color travels as its four channels.
/// The times go through chrono's own impls, which write RFC 3339.
#[derive(Serialize, Deserialize)]
struct SavedMonth {
    events: Vec<SavedEvent>,
}

/// Catalog and successful refresh time travel with the event cache, so a
/// restart can show source names and respect the remaining sync interval.
#[derive(Serialize, Deserialize)]
struct SyncState {
    fetched_at: DateTime<Local>,
    calendars: Vec<SavedCalendar>,
}

#[derive(Serialize, Deserialize)]
struct SavedCalendar {
    id: String,
    name: String,
    color: crate::hex::HexColor,
    primary: bool,
    selected_by_google: bool,
}

pub fn load_sync_state(database: &Path) -> Option<(DateTime<Local>, Vec<Calendar>)> {
    let state: SyncState = task_db::read_document(database, keys::SYNC_STATE).ok()??;
    Some((
        state.fetched_at,
        state
            .calendars
            .into_iter()
            .map(|saved| Calendar {
                id: saved.id,
                name: saved.name,
                color: saved.color.0,
                primary: saved.primary,
                selected_by_google: saved.selected_by_google,
            })
            .collect(),
    ))
}

pub fn save_sync_state(
    database: &Path,
    fetched_at: DateTime<Local>,
    calendars: &[Calendar],
) -> io::Result<()> {
    let state = SyncState {
        fetched_at,
        calendars: calendars
            .iter()
            .map(|calendar| SavedCalendar {
                id: calendar.id.clone(),
                name: calendar.name.clone(),
                color: crate::hex::HexColor(calendar.color),
                primary: calendar.primary,
                selected_by_google: calendar.selected_by_google,
            })
            .collect(),
    };
    task_db::write_document(database, keys::SYNC_STATE, &state)
}

/// One event as the cache row holds it.
#[derive(Serialize, Deserialize)]
pub(super) struct SavedEvent {
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

fn open(database: &Path) -> io::Result<Database> {
    Database::open(database).map_err(io::Error::other)
}

fn month_text(connection: &rusqlite::Connection, month: &str) -> rusqlite::Result<Option<String>> {
    connection
        .query_row(
            "SELECT events FROM calendar_months WHERE month=?1",
            [month],
            |row| row.get(0),
        )
        .optional()
}

/// The day's events as an earlier run wrote them, or nothing when no run has
/// written this month or the row turns out to be unreadable. Only events
/// overlapping `day`'s window (set by `day_end_hour`) are returned, along
/// with tasks, whose backlog is shared across all views.
pub fn load(database: &Path, day: NaiveDate, day_end_hour: u32) -> Option<Vec<Event>> {
    let database_path = database;
    let mut database = match open(database) {
        Ok(database) => database,
        Err(error) => {
            log::warn!("cached events could not be read: {error}");
            return None;
        }
    };
    let month = month_key(day);
    let text = match month_text(database.connection(), &month) {
        Ok(Some(text)) => text,
        Ok(None) => {
            let has_local:bool=database.connection().query_row("SELECT EXISTS(SELECT 1 FROM google_entities) OR EXISTS(SELECT 1 FROM google_task_records)",[],|r|r.get(0)).unwrap_or(false);
            if !has_local {
                return None;
            }
            "{\"events\":[]}".into()
        }
        Err(error) => {
            log::warn!("cached events for {month} could not be read: {error}");
            return None;
        }
    };

    match serde_json::from_str::<SavedMonth>(&text) {
        Ok(saved) => {
            // Once a complete task snapshot exists, month rows are calendar
            // caches only. Their old task copies must never resurrect deletions
            // or override a moved task's canonical record.
            let canonical_tasks: bool = database
                .connection()
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM documents WHERE key='calendar_task_snapshot')",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(false);
            let mut events: Vec<Event> = saved
                .events
                .into_iter()
                .map(Event::from)
                .filter(|event| !canonical_tasks || !event.is_task())
                .filter(|event| super::event_on_day(event, day, day_end_hour))
                .collect();
            if let Ok(mut statement) = database
                .connection()
                .prepare("SELECT event FROM google_task_records")
                && let Ok(rows) = statement.query_map([], |r| r.get::<_, String>(0))
            {
                for row in rows.flatten() {
                    if let Ok(task) = serde_json::from_str::<Event>(&row) {
                        events.retain(|e| e.id != task.id);
                        events.push(task);
                    }
                }
            }
            let _ = super::operations::overlay(database_path, &mut events, None);
            events.retain(|e| super::event_on_day(e, day, day_end_hour));
            Some(events)
        }
        Err(error) => {
            log::warn!("cached events for {month} could not be parsed, ignoring them: {error}");
            None
        }
    }
}

/// Writes one day's events into the month's row, replacing whatever that day
/// held. The read and the write share a transaction, so two processes saving
/// the same month cannot lose each other's days.
pub fn save(
    database: &Path,
    day: NaiveDate,
    events: &[Event],
    day_end_hour: u32,
) -> io::Result<()> {
    let mut database = open(database)?;
    let month_name = month_key(day);
    let tx = database
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(io::Error::other)?;

    // Load the existing month, or start empty.
    let mut month: SavedMonth = month_text(&tx, &month_name)
        .map_err(io::Error::other)?
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(SavedMonth { events: Vec::new() });

    // A snapshot also contains tasks from other days. Replace those by id
    // instead of appending another copy for every day in a weekly refresh.
    let incoming: std::collections::HashSet<_> = events
        .iter()
        .map(|event| (event.calendar_id.as_str(), event.id.as_str()))
        .collect();
    month.events.retain(|saved| {
        day_at(saved.start, day_end_hour) != day
            && !incoming.contains(&(saved.calendar_id.as_str(), saved.id.as_str()))
    });
    month.events.extend(events.iter().map(SavedEvent::from));

    write_month(&tx, &month_name, &month)?;
    tx.commit().map_err(io::Error::other)
}

fn write_month(
    connection: &rusqlite::Connection,
    month: &str,
    saved: &SavedMonth,
) -> io::Result<()> {
    let text = serde_json::to_string(saved)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    connection
        .execute(
            "INSERT INTO calendar_months (month,events) VALUES (?1,?2) ON CONFLICT(month) DO UPDATE SET events=excluded.events",
            params![month, text],
        )
        .map(|_| ())
        .map_err(io::Error::other)
}

/// Update known records without throwing away unrelated cached dates.
pub fn update_events(database: &Path, mut update: impl FnMut(&mut Vec<Event>)) {
    let mut database_handle = match open(database) {
        Ok(database) => database,
        Err(error) => {
            log::warn!("could not update cached events: {error}");
            return;
        }
    };
    for month in days(database) {
        let month = month_key(month);
        let mut write = || -> io::Result<()> {
            let tx = database_handle
                .connection()
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(io::Error::other)?;
            let Some(saved) = month_text(&tx, &month)
                .map_err(io::Error::other)?
                .and_then(|text| serde_json::from_str::<SavedMonth>(&text).ok())
            else {
                return Ok(());
            };
            let mut events: Vec<Event> = saved.events.into_iter().map(Event::from).collect();
            let before = events.clone();
            update(&mut events);
            if events == before {
                return Ok(());
            }
            let updated = SavedMonth {
                events: events.iter().map(SavedEvent::from).collect(),
            };
            write_month(&tx, &month, &updated)?;
            tx.commit().map_err(io::Error::other)
        };
        if let Err(error) = write() {
            log::warn!("could not update cached events: {error}");
        }
    }
}

/// Puts a series' freshly fetched occurrences into every cached month the
/// refresh covers, by [`Refresh::apply_to_month`](super::series::Refresh). A
/// month neither naming the series nor gaining an occurrence is not parsed.
pub(super) fn replace_series(database: &Path, refresh: &super::series::Refresh, day_end_hour: u32) {
    let mut database_handle = match open(database) {
        Ok(database) => database,
        Err(error) => {
            log::warn!("could not update the cached series: {error}");
            return;
        }
    };
    for month_start in days(database) {
        let month_end = month_start
            .iter_days()
            .take_while(|day| day.month() == month_start.month())
            .last()
            .unwrap_or(month_start);
        if month_end < refresh.first || month_start > refresh.last {
            continue;
        }
        let month = month_key(month_start);
        let mut write = || -> io::Result<()> {
            let tx = database_handle
                .connection()
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(io::Error::other)?;
            let Some(text) = month_text(&tx, &month).map_err(io::Error::other)? else {
                return Ok(());
            };
            let gains = refresh.events.iter().any(|event| {
                (month_start..=month_end).contains(&day_at(event.start, day_end_hour))
            });
            if !gains
                && !refresh
                    .series
                    .iter()
                    .any(|series| text.contains(&series.id))
            {
                return Ok(());
            }
            let Ok(saved) = serde_json::from_str::<SavedMonth>(&text) else {
                return Ok(());
            };
            let mut events: Vec<Event> = saved.events.into_iter().map(Event::from).collect();
            let before = events.clone();
            refresh.apply_to_month(month_start, &mut events, day_end_hour);
            if events == before {
                return Ok(());
            }
            let updated = SavedMonth {
                events: events.iter().map(SavedEvent::from).collect(),
            };
            write_month(&tx, &month, &updated)?;
            tx.commit().map_err(io::Error::other)
        };
        if let Err(error) = write() {
            log::warn!("could not update the cached series in {month}: {error}");
        }
    }
}

/// Every cached month, as the first day of each month. A row not named like a
/// month is left out rather than guessed at.
pub fn days(database: &Path) -> Vec<NaiveDate> {
    let listed = || -> Result<Vec<String>, String> {
        let mut database = Database::open(database)?;
        let connection = database.connection();
        let mut statement = connection
            .prepare("SELECT month FROM calendar_months")
            .map_err(|error| error.to_string())?;
        statement
            .query_map([], |row| row.get(0))
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<_>>()
            .map_err(|error| error.to_string())
    };
    match listed() {
        Ok(months) => months
            .iter()
            .filter_map(|month| parse_month(month))
            .collect(),
        Err(error) => {
            log::warn!("cached months could not be listed: {error}");
            Vec::new()
        }
    }
}

/// The first day of a month named like `2026-10`.
pub(crate) fn parse_month(name: &str) -> Option<NaiveDate> {
    let mut parts = name.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month, 1)
}

/// Empties the cache, so every month has to be fetched again. The sync state
/// goes too: it says how recently those months were fetched.
pub fn clear(database: &Path) {
    let cleared = open(database).and_then(|mut database| {
        database
            .connection()
            .execute_batch(&format!(
                "BEGIN IMMEDIATE;
                 DELETE FROM calendar_months;
                 DELETE FROM documents WHERE key='{}';
                 COMMIT;",
                keys::SYNC_STATE
            ))
            .map_err(io::Error::other)
    });
    if let Err(error) = cleared {
        log::warn!("the event cache could not be cleared: {error}");
    }
}

/// The name a month is filed under, e.g. `2026-10`.
fn month_key(day: NaiveDate) -> String {
    format!("{:04}-{:02}", day.year(), day.month())
}

/// A throwaway database under the system's temp dir, removed when the test
/// that made it ends either way.
#[cfg(test)]
pub(super) struct Scratch(std::path::PathBuf, std::path::PathBuf);

#[cfg(test)]
impl Scratch {
    pub(super) fn new(name: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or_default();

        let path = std::env::temp_dir().join(format!(
            "timeslay-month-cache-{}-{unique}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch directory");

        let database = path.join("test.sqlite3");
        Self(path, database)
    }

    pub(super) fn path(&self) -> &Path {
        &self.1
    }
}

#[cfg(test)]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::sample_snapshot;
    use chrono::TimeZone;

    fn raw_month(database: &Path, month: &str, text: &str) {
        open(database)
            .unwrap()
            .connection()
            .execute(
                "INSERT INTO calendar_months (month,events) VALUES (?1,?2)",
                [month, text],
            )
            .unwrap();
    }

    #[test]
    fn cached_days_include_overlapping_events_and_undated_tasks() {
        let scratch = Scratch::new("overlap-and-tasks");
        let mut snapshot = sample_snapshot();
        let day = snapshot.day;
        let next = day.succ_opt().unwrap();
        let mut overnight = snapshot.events.remove(0);
        overnight.all_day = false;
        overnight.start = super::super::at_clock(day, 23, 0);
        overnight.end = super::super::at_clock(next, 2, 0);
        let mut task = overnight.clone();
        task.id = "task:undated".into();
        task.calendar_id = "tasks:list".into();
        task.start = super::super::at_clock(crate::timeblocking::UNDATED, 0, 0);
        task.end = task.start + chrono::Duration::days(1);
        let events = vec![overnight, task];
        save(scratch.path(), day, &events, 0).unwrap();
        assert_eq!(load(scratch.path(), next, 0).unwrap(), events);
    }

    #[test]
    fn a_month_round_trips_through_the_database() {
        let scratch = Scratch::new("round-trip");
        let snapshot = sample_snapshot();
        let day = snapshot.day;

        save(scratch.path(), day, &snapshot.events, 0).unwrap();

        assert_eq!(load(scratch.path(), day, 0).unwrap(), snapshot.events);
    }

    #[test]
    fn task_timestamps_survive_a_restart() {
        let scratch = Scratch::new("task-timestamps");
        let snapshot = sample_snapshot();
        let mut event = snapshot.events[0].clone();
        event.id = "task:timestamps".into();
        event.calendar_id = "tasks:inbox".into();
        let timestamp = chrono::DateTime::parse_from_rfc3339("2026-10-29T19:30:12.123Z")
            .unwrap()
            .to_utc();
        event.details.task_updated_at = Some(timestamp);
        event.details.task_data = Some(crate::task_db::TaskData {
            id: 100,
            metadata: crate::timeblocking::Metadata {
                created_at: Some(timestamp),
                ..Default::default()
            },
        });
        save(scratch.path(), snapshot.day, &[event.clone()], 0).unwrap();
        assert_eq!(load(scratch.path(), snapshot.day, 0).unwrap(), vec![event]);
    }

    #[test]
    fn saving_neighboring_days_does_not_duplicate_other_days_tasks() {
        let scratch = Scratch::new("weekly-duplicates");
        let snapshot = sample_snapshot();
        let day = snapshot.day;
        let next = day.succ_opt().unwrap();
        save(scratch.path(), day, &snapshot.events, 0).unwrap();
        save(scratch.path(), next, &snapshot.events, 0).unwrap();
        save(scratch.path(), next, &snapshot.events, 0).unwrap();
        assert_eq!(load(scratch.path(), day, 0).unwrap(), snapshot.events);
        assert!(load(scratch.path(), next, 0).unwrap().is_empty());
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

        raw_month(scratch.path(), &month_key(day), "not json");

        assert!(load(scratch.path(), day, 0).is_none());
        assert_eq!(
            days(scratch.path()),
            vec![day.with_day(1).unwrap()],
            "it is still a month"
        );
    }

    #[test]
    fn only_rows_named_like_a_month_are_listed() {
        let scratch = Scratch::new("listing");
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();

        save(scratch.path(), day, &[], 0).unwrap();
        raw_month(scratch.path(), "notes", "{}");
        raw_month(scratch.path(), "2026-10-extra", "{}");

        assert_eq!(days(scratch.path()), vec![day.with_day(1).unwrap()]);
    }

    #[test]
    fn clearing_takes_every_month_with_it() {
        let scratch = Scratch::new("clearing");
        let day = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        save(scratch.path(), day, &[], 0).unwrap();

        save_sync_state(scratch.path(), crate::clock::now(), &[]).unwrap();

        clear(scratch.path());

        assert!(days(scratch.path()).is_empty());
        assert!(load(scratch.path(), day, 0).is_none());
        assert!(load_sync_state(scratch.path()).is_none());
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
            color: iced_core::Color::WHITE,
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
            color: iced_core::Color::WHITE,
            completed: false,
            details: Details::default(),
        };

        save(scratch.path(), oct_8, std::slice::from_ref(&event_a), 0).unwrap();
        save(scratch.path(), oct_9, std::slice::from_ref(&event_b), 0).unwrap();

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
            color: iced_core::Color::WHITE,
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
            color: iced_core::Color::WHITE,
            completed: false,
            details: Details::default(),
        };

        save(scratch.path(), oct_8, std::slice::from_ref(&event_a), 0).unwrap();
        save(scratch.path(), oct_8, std::slice::from_ref(&event_b), 0).unwrap();

        assert_eq!(load(scratch.path(), oct_8, 0).unwrap(), vec![event_b]);
    }
}
