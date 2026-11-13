//! Nights of sleep in the local database, mirroring Google Calendar.
//!
//! Google is the source of truth: each calendar sync makes the nights in its
//! planning window what Google listed, and each night written to or deleted
//! from Google is written here too. Between syncs, cached days read their
//! nights from here, so a night planned weeks ahead shows without a fetch.
//!
//! Database sync replicates the rows (see `settings::schema::SLEEP_NIGHTS`),
//! so a night timed on one device shows on the others before they next ask
//! Google. Every change is journaled for them, so a row is only written when
//! it actually differs: an unchanged sync costs no sync traffic.
//!
//! The versioned database setup creates the table and its change and sync
//! triggers. Like the event cache, a failure here costs a re-sync, never the
//! night itself.

use std::collections::HashSet;
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::super::Event;
use super::super::day_cache::SavedEvent;
use crate::task_db::Database;

/// A UTC timestamp in one fixed format, so stored times compare as text.
fn stamp(at: DateTime<Utc>) -> String {
	at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse(text: &str) -> Option<Event> {
	serde_json::from_str::<SavedEvent>(text)
		.ok()
		.map(Event::from)
}

/// Writes a night, replacing any other copy of the same night: one row per
/// wake-up date, as on Google. A night stored exactly as given is left alone.
fn upsert(connection: &Connection, night: &Event) -> Result<(), String> {
	let Some(record) = night.details.sleep.as_ref() else {
		return Ok(());
	};
	let wake_date = record.wake_date.to_string();
	connection
		.execute(
			"DELETE FROM sleep_nights WHERE calendar_id=?1 AND wake_date=?2 AND id!=?3",
			params![night.calendar_id, wake_date, night.id],
		)
		.map_err(|e| e.to_string())?;
	let stored: Option<String> = connection
		.query_row(
			"SELECT event FROM sleep_nights WHERE calendar_id=?1 AND id=?2",
			params![night.calendar_id, night.id],
			|row| row.get(0),
		)
		.optional()
		.map_err(|e| e.to_string())?;
	if stored.as_deref().and_then(parse).as_ref() == Some(night) {
		return Ok(());
	}
	let event = serde_json::to_string(&SavedEvent::from(night)).map_err(|e| e.to_string())?;
	connection
		.execute(
			"INSERT INTO sleep_nights (calendar_id,id,wake_date,start_at,end_at,event)
             VALUES (?1,?2,?3,?4,?5,?6)
             ON CONFLICT(calendar_id,id) DO UPDATE SET wake_date=excluded.wake_date,
               start_at=excluded.start_at,end_at=excluded.end_at,event=excluded.event",
			params![
				night.calendar_id,
				night.id,
				wake_date,
				stamp(night.start.to_utc()),
				stamp(night.end.to_utc()),
				event
			],
		)
		.map(|_| ())
		.map_err(|e| e.to_string())
}

/// Makes the stored nights overlapping `from..to` exactly those Google just
/// listed for that span. Nights on any other calendar are dropped: only the
/// app calendar holds sleep.
pub(in crate::calendar) fn replace(
	path: &Path,
	calendar_id: &str,
	from: DateTime<Utc>,
	to: DateTime<Utc>,
	nights: &[Event],
) -> Result<(), String> {
	let mut database = Database::open(path)?;
	let tx = database
		.connection()
		.transaction_with_behavior(TransactionBehavior::Immediate)
		.map_err(|e| e.to_string())?;
	let listed: HashSet<&str> = nights
		.iter()
		.filter(|n| n.calendar_id == calendar_id)
		.map(|n| n.id.as_str())
		.collect();
	let stored: Vec<String> = tx
		.prepare(
			"SELECT id FROM sleep_nights WHERE calendar_id=?1 AND start_at < ?3 AND end_at > ?2",
		)
		.and_then(|mut statement| {
			statement
				.query_map(params![calendar_id, stamp(from), stamp(to)], |row| {
					row.get(0)
				})?
				.collect()
		})
		.map_err(|e| e.to_string())?;
	tx.execute(
		"DELETE FROM sleep_nights WHERE calendar_id!=?1",
		[calendar_id],
	)
	.map_err(|e| e.to_string())?;
	for id in stored.iter().filter(|id| !listed.contains(id.as_str())) {
		tx.execute(
			"DELETE FROM sleep_nights WHERE calendar_id=?1 AND id=?2",
			params![calendar_id, id],
		)
		.map_err(|e| e.to_string())?;
	}
	for night in nights.iter().filter(|n| n.calendar_id == calendar_id) {
		upsert(&tx, night)?;
	}
	tx.commit().map_err(|e| e.to_string())
}

/// Stores one night as Google now has it.
pub(in crate::calendar) fn put(path: &Path, night: &Event) -> Result<(), String> {
	let mut database = Database::open(path)?;
	let tx = database
		.connection()
		.transaction_with_behavior(TransactionBehavior::Immediate)
		.map_err(|e| e.to_string())?;
	upsert(&tx, night)?;
	tx.commit().map_err(|e| e.to_string())
}

/// Forgets an event deleted from Google. Anything that is not a night
/// matches no row, so this is safe to call for every deletion.
pub(in crate::calendar) fn remove(path: &Path, calendar_id: &str, id: &str) -> Result<(), String> {
	Database::open(path)?
		.connection()
		.execute(
			"DELETE FROM sleep_nights WHERE calendar_id=?1 AND id=?2",
			params![calendar_id, id],
		)
		.map(|_| ())
		.map_err(|e| e.to_string())
}

/// The stored nights overlapping `from..to`. A database without the table
/// yet simply has no nights.
pub(in crate::calendar) fn overlapping(
	connection: &Connection,
	from: DateTime<Utc>,
	to: DateTime<Utc>,
) -> Vec<Event> {
	let rows = || -> rusqlite::Result<Vec<String>> {
		connection
			.prepare("SELECT event FROM sleep_nights WHERE start_at < ?2 AND end_at > ?1")?
			.query_map(params![stamp(from), stamp(to)], |row| row.get(0))?
			.collect()
	};
	match rows() {
		Ok(rows) => rows
			.iter()
			.filter_map(|row| serde_json::from_str::<SavedEvent>(row).ok())
			.map(Event::from)
			.collect(),
		Err(error) => {
			if !error.to_string().contains("no such table") {
				log::warn!("stored sleep could not be read: {error}");
			}
			Vec::new()
		}
	}
}

/// Puts the stored nights into a cached day's events, in place of any older
/// copy of the same night the month row kept.
pub(in crate::calendar) fn merge_into(
	connection: &Connection,
	from: DateTime<Utc>,
	to: DateTime<Utc>,
	events: &mut Vec<Event>,
) {
	merge(overlapping(connection, from, to), events);
}

/// Puts `nights` into `events`, each replacing any copy of the same night:
/// the same event, or another event for the same wake-up date.
pub(in crate::calendar) fn merge(nights: impl IntoIterator<Item = Event>, events: &mut Vec<Event>) {
	for night in nights {
		let wake_date = night.details.sleep.as_ref().map(|s| s.wake_date);
		events.retain(|event| {
			event.calendar_id != night.calendar_id
				|| (event.id != night.id
					&& event.details.sleep.as_ref().map(|s| s.wake_date) != wake_date)
		});
		events.push(night);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::calendar::sleep::{COLOR, Record};
	use chrono::{Duration, Local, NaiveDate, TimeZone};

	fn night(calendar: &str, id: &str, day: u32, hours: i64) -> Event {
		let wake = Local.with_ymd_and_hms(2030, 11, day, 8, 0, 0).unwrap();
		Event {
			id: id.into(),
			calendar_id: calendar.into(),
			title: "Sleep".into(),
			start: wake - Duration::hours(hours),
			end: wake,
			all_day: false,
			color: COLOR,
			completed: false,
			details: crate::calendar::Details {
				sleep: Some(Record::planned(
					NaiveDate::from_ymd_opt(2030, 11, day).unwrap(),
				)),
				..Default::default()
			},
		}
	}

	fn ever() -> (DateTime<Utc>, DateTime<Utc>) {
		let year = |y| Utc.with_ymd_and_hms(y, 1, 1, 0, 0, 0).unwrap();
		(year(2000), year(2100))
	}

	fn all(path: &Path) -> Vec<Event> {
		let mut nights = overlapping(
			Database::open(path).unwrap().connection(),
			ever().0,
			ever().1,
		);
		nights.sort_by_key(|n| n.start);
		nights
	}

	#[test]
	fn google_replaces_the_window_and_writes_keep_one_row_per_night() {
		let scratch = crate::calendar::day_cache::Scratch::new("sleep-store");
		let path = scratch.path();
		let window = |a: u32, b: u32| {
			(
				Local
					.with_ymd_and_hms(2030, 11, a, 0, 0, 0)
					.unwrap()
					.to_utc(),
				Local
					.with_ymd_and_hms(2030, 11, b, 0, 0, 0)
					.unwrap()
					.to_utc(),
			)
		};
		let (from, to) = window(4, 8);
		let old = night("app", "old", 2, 8);
		put(path, &old).unwrap();
		put(path, &night("elsewhere", "x", 5, 8)).unwrap();
		put(path, &night("app", "gone", 5, 8)).unwrap();
		replace(path, "app", from, to, &[night("app", "a", 6, 8)]).unwrap();
		// Outside the window survives; another calendar and deleted nights go.
		assert_eq!(
			all(path).iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
			["old", "a"]
		);

		// A write updates the night, and a new id for the same night replaces it.
		put(path, &night("app", "a", 6, 9)).unwrap();
		put(path, &night("app", "b", 6, 7)).unwrap();
		let stored = all(path);
		assert_eq!(stored.len(), 2);
		assert_eq!(stored[1].id, "b");
		assert_eq!(stored[1], night("app", "b", 6, 7));

		remove(path, "app", "b").unwrap();
		assert_eq!(all(path), vec![old]);
	}

	#[test]
	fn only_real_changes_reach_the_sync_journal() {
		let scratch = crate::calendar::day_cache::Scratch::new("sleep-store-journal");
		let path = scratch.path();
		// Opening a database file sets it up, and enrolls it for sync.
		Database::open(path).unwrap();
		let journaled = || {
			crate::database_sync::history(path)
				.unwrap()
				.iter()
				.filter(|edit| edit.document.starts_with("sleep_nights:"))
				.count()
		};
		let (from, to) = ever();
		let listed = [night("app", "a", 5, 8), night("app", "b", 6, 8)];
		replace(path, "app", from, to, &listed).unwrap();
		assert_eq!(journaled(), 2);
		// The same listing again, and a write Google echoed unchanged: nothing.
		replace(path, "app", from, to, &listed).unwrap();
		put(path, &listed[0]).unwrap();
		assert_eq!(journaled(), 2);
		// One night moved and one gone on Google: one edit each.
		replace(path, "app", from, to, &[night("app", "a", 5, 9)]).unwrap();
		assert_eq!(journaled(), 4);
	}

	#[test]
	fn a_night_stored_by_another_device_shows_on_the_days_in_memory() {
		let scratch = crate::calendar::day_cache::Scratch::new("sleep-store-reread");
		let path = scratch.path();
		let day = |d| NaiveDate::from_ymd_opt(2030, 11, d).unwrap();
		let mut store = crate::calendar::Store::new(0);
		store.cache_days_in(Some(path.to_path_buf()));
		store.set_day(day(5), day(5));
		store.set_day(day(6), day(5));
		store.events = vec![night("app", "old", 6, 8)];

		// As database sync would write it: a new copy of the same night.
		put(path, &night("app", "new", 6, 9)).unwrap();
		let (first, last) = {
			let days = store.loaded_days();
			(days[0], *days.last().unwrap())
		};
		let nights = crate::calendar::sleep::stored_nights(path, first, last).unwrap();
		store.apply_stored_nights(&nights);
		assert_eq!(store.events_on(day(6)), [night("app", "new", 6, 9)]);
		// 9h before 08:00 on the 6th starts on the 5th, so it shows there too.
		assert!(store.events_on(day(5)).iter().any(|e| e.id == "new"));
		assert!(store.events_on(day(5)).iter().all(|e| e.id != "old"));
	}

	#[test]
	fn a_cached_day_takes_the_stored_copy_of_its_night() {
		let scratch = crate::calendar::day_cache::Scratch::new("sleep-store-merge");
		let path = scratch.path();
		put(path, &night("app", "fresh", 6, 9)).unwrap();
		let mut events = vec![night("app", "stale", 6, 8), night("other", "y", 6, 8)];
		merge_into(
			Database::open(path).unwrap().connection(),
			ever().0,
			ever().1,
			&mut events,
		);
		assert_eq!(
			events.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(),
			["y", "fresh"]
		);
	}
}
