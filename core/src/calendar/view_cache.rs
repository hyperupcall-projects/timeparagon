//! Database reads for complete views, including successfully fetched empty
//! days, from the local copy of Google's data that sync maintains.

use std::collections::HashSet;
use std::path::Path;

use chrono::NaiveDate;

use super::{Event, Snapshot, api::incremental, day_cache, event_on_day, operations, sleep};
use crate::task_db::Database;

/// `None` means some calendar data is missing, even when tasks or other days
/// have been saved. No network or OAuth is needed.
pub fn load(
	path: &Path,
	dates: &[NaiveDate],
	day_end_hour: u32,
) -> Result<Option<Snapshot>, String> {
	let day = *dates.first().ok_or("Calendar range is empty")?;
	let first = *dates.iter().min().unwrap();
	let last = *dates.iter().max().unwrap();
	let Some((fetched_at, calendars)) = day_cache::load_sync_state(path) else {
		return Ok(None);
	};
	let mut database = Database::open(path)?;
	let tx = database
		.connection()
		.unchecked_transaction()
		.map_err(|e| e.to_string())?;
	let Some(mut events) = incremental::cached_events(&tx, &calendars, first, last, day_end_hour)?
	else {
		return Ok(None);
	};
	// Merge the canonical task snapshot once, rather than once per date.
	let mut query = tx
		.prepare("SELECT event FROM google_task_records")
		.map_err(|e| e.to_string())?;
	let tasks: Vec<Event> = query
		.query_map([], |row| row.get::<_, String>(0))
		.map_err(|e| e.to_string())?
		.map(|row| {
			serde_json::from_str::<Event>(&row.map_err(|e| e.to_string())?)
				.map_err(|e| e.to_string())
		})
		.collect::<Result<_, _>>()?;
	let tasks: Vec<_> = tasks
		.into_iter()
		.filter(|task| !task.is_calendar_task())
		.collect();
	let ids: HashSet<_> = tasks.iter().map(|task| task.id.as_str()).collect();
	events.retain(|event| !ids.contains(event.id.as_str()));
	events.extend(tasks);
	let mut seen = HashSet::new();
	events.retain(|event| seen.insert((event.calendar_id.clone(), event.id.clone())));
	sleep::store::merge(sleep::stored_nights(path, first, last)?, &mut events);
	// A successful sync updates every covered window. Only edits still pending,
	// or acknowledged after that sync began, may override it.
	operations::overlay(path, &mut events, Some(fetched_at))?;
	events.retain(|event| {
		dates
			.iter()
			.any(|&day| event_on_day(event, day, day_end_hour))
	});
	events.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
	Ok(Some(Snapshot {
		calendars,
		events,
		day,
		fetched_at,
		notice: None,
	}))
}

/// Save on the worker before delivering a result to the UI. Navigating away
/// cannot discard a completed fetch.
pub fn save(
	path: &Path,
	snapshot: &Snapshot,
	dates: &[NaiveDate],
	day_end_hour: u32,
) -> Result<(), String> {
	if dates.is_empty() {
		return Err("Calendar range is empty".to_owned());
	}
	let mut database = Database::open(path)?;
	let tx = database
		.connection()
		.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
		.map_err(|e| e.to_string())?;
	day_cache::save_days(&tx, dates, &snapshot.events, day_end_hour).map_err(|e| e.to_string())?;
	day_cache::save_sync_state_in(&tx, snapshot.fetched_at, &snapshot.calendars)
		.map_err(|e| e.to_string())?;
	tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::calendar::sample_snapshot;

	#[test]
	fn saved_months_do_not_count_as_synced_days() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let snapshot = sample_snapshot();
		let day = snapshot.day;
		assert!(load(&path, &[day], 0).unwrap().is_none());
		save(&path, &snapshot, &[day], 0).unwrap();
		assert!(
			load(&path, &[day], 0).unwrap().is_none(),
			"only the sync windows say which days are complete"
		);
	}

	#[test]
	fn refreshing_an_empty_day_removes_an_overnight_event_deleted_on_google() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		let day = snapshot.day;
		let next = day.succ_opt().unwrap();
		snapshot.events.truncate(1);
		snapshot.events[0].start = super::super::at_clock(day, 23, 0);
		snapshot.events[0].end = super::super::at_clock(next, 2, 0);
		save(&path, &snapshot, &[day, next], 0).unwrap();
		snapshot.events.clear();
		snapshot.day = next;
		save(&path, &snapshot, &[next], 0).unwrap();
		assert!(day_cache::load(&path, next, 0).unwrap().is_empty());
	}

	#[test]
	fn range_save_writes_each_month_once_and_preserves_unrequested_days() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		snapshot.events.truncate(1);
		let untouched = snapshot.events[0].clone();
		let first = snapshot.day;
		save(&path, &snapshot, &[first], 0).unwrap();
		let mut db = Database::open(&path).unwrap();
		db.connection().execute_batch("
            CREATE TABLE cache_writes(month TEXT);
            CREATE TRIGGER count_month_insert AFTER INSERT ON calendar_months BEGIN INSERT INTO cache_writes VALUES(NEW.month); END;
            CREATE TRIGGER count_month_update AFTER UPDATE ON calendar_months BEGIN INSERT INTO cache_writes VALUES(NEW.month); END;
        ").unwrap();
		let start = first + chrono::Duration::days(20);
		let dates: Vec<_> = (0..35)
			.map(|offset| start + chrono::Duration::days(offset))
			.collect();
		let overnight = first + chrono::Duration::days(30);
		snapshot.events[0].start = super::super::at_clock(overnight, 23, 0);
		snapshot.events[0].end = snapshot.events[0].start + chrono::Duration::hours(3);
		snapshot.events[0].id = "overnight".into();
		save(&path, &snapshot, &dates, 0).unwrap();
		let writes: i64 = db
			.connection()
			.query_row("SELECT count(*) FROM cache_writes", [], |row| row.get(0))
			.unwrap();
		assert_eq!(
			writes, 2,
			"a date range spanning two months writes two rows"
		);
		assert_eq!(day_cache::load(&path, first, 0).unwrap(), vec![untouched]);
		assert_eq!(
			day_cache::load(&path, overnight, 0).unwrap(),
			snapshot.events
		);
	}

	#[test]
	fn failed_range_save_rolls_back_contents_and_sync_time_together() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		let day = snapshot.day;
		save(&path, &snapshot, &[day], 0).unwrap();
		let before = day_cache::load(&path, day, 0).unwrap();
		let (fetched_at, _) = day_cache::load_sync_state(&path).unwrap();
		let mut db = Database::open(&path).unwrap();
		db.connection().execute_batch("CREATE TRIGGER fail_next_month BEFORE INSERT ON calendar_months WHEN NEW.month='2026-11' BEGIN SELECT RAISE(ABORT,'test failure'); END;").unwrap();
		snapshot.events.clear();
		snapshot.fetched_at += chrono::Duration::hours(1);
		let next = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
		assert!(save(&path, &snapshot, &[day, next], 0).is_err());
		assert_eq!(day_cache::load(&path, day, 0).unwrap(), before);
		assert_eq!(day_cache::load_sync_state(&path).unwrap().0, fetched_at);
		assert!(day_cache::load(&path, next, 0).is_none());
	}
}
