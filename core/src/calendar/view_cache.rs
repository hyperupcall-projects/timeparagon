//! Database reads for complete views, including successfully fetched empty
//! days. Incremental views read Google's maintained local copy; full-sync views
//! use month snapshots with explicit coverage, never inferred from events.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use chrono::NaiveDate;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use super::{Event, Snapshot, api::incremental, day_cache, event_on_day, operations, sleep};
use crate::task_db::{self, Database};

const COVERAGE: &str = "calendar_view_coverage";
#[derive(Default, Serialize, Deserialize)]
struct Coverage {
	account: Option<String>,
	day_end_hour: u32,
	days: BTreeSet<NaiveDate>,
}

fn account(calendars: &[super::Calendar]) -> Option<String> {
	calendars
		.iter()
		.find(|calendar| calendar.primary)
		.map(|calendar| calendar.id.clone())
}

/// `None` means some calendar data is missing, even when tasks or other days
/// in the month have been saved. No network or OAuth is needed.
pub fn load(
	path: &Path,
	dates: &[NaiveDate],
	day_end_hour: u32,
	incremental: bool,
) -> Result<Option<Snapshot>, String> {
	let day = *dates.first().ok_or("Calendar range is empty")?;
	let first = *dates.iter().min().unwrap();
	let last = *dates.iter().max().unwrap();
	let Some((fetched_at, calendars)) = day_cache::load_sync_state(path) else {
		return Ok(None);
	};
	let mut events = if incremental {
		let mut database = Database::open(path)?;
		let tx = database
			.connection()
			.unchecked_transaction()
			.map_err(|e| e.to_string())?;
		let Some(mut events) =
			incremental::cached_events(&tx, &calendars, first, last, day_end_hour)?
		else {
			return Ok(None);
		};
		// Tasks have one canonical snapshot, shared by all dates. Calendar
		// tasks come from the sync tables, including pinned off-day tasks.
		let mut query = tx
			.prepare("SELECT event FROM google_task_records")
			.map_err(|e| e.to_string())?;
		let rows = query
			.query_map([], |row| row.get::<_, String>(0))
			.map_err(|e| e.to_string())?;
		for row in rows {
			let task: Event = serde_json::from_str(&row.map_err(|e| e.to_string())?)
				.map_err(|e| e.to_string())?;
			if !task.is_calendar_task() {
				events.retain(|event| event.id != task.id);
				events.push(task);
			}
		}
		events
	} else {
		let coverage: Coverage = task_db::read_document(path, COVERAGE)
			.ok()
			.flatten()
			.unwrap_or_default();
		if coverage.account != account(&calendars)
			|| coverage.day_end_hour != day_end_hour
			|| !dates.iter().all(|day| coverage.days.contains(day))
		{
			return Ok(None);
		}
		let mut events = Vec::new();
		for &day in dates {
			let Some(saved) = day_cache::load(path, day, day_end_hour) else {
				return Ok(None);
			};
			events.extend(saved);
		}
		events
	};
	let mut seen = HashSet::new();
	events.retain(|event| seen.insert((event.calendar_id.clone(), event.id.clone())));
	sleep::store::merge(sleep::stored_nights(path, first, last)?, &mut events);
	// A successful incremental pass updates every covered window. Only edits
	// still pending, or acknowledged after that pass began, may override it.
	// Month snapshots can be older than the last sync of a different view.
	operations::overlay(path, &mut events, incremental.then_some(fetched_at))?;
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
/// cannot discard a completed fetch. Publish coverage only after all contents
/// have been written successfully.
pub fn save(
	path: &Path,
	snapshot: &Snapshot,
	dates: &[NaiveDate],
	day_end_hour: u32,
) -> Result<(), String> {
	if dates.is_empty() {
		return Err("Calendar range is empty".to_owned());
	}
	for &day in dates {
		let events: Vec<_> = snapshot
			.events
			.iter()
			.filter(|event| event_on_day(event, day, day_end_hour))
			.cloned()
			.collect();
		day_cache::save(path, day, &events, day_end_hour).map_err(|e| e.to_string())?;
	}
	let mut database = Database::open(path)?;
	let tx = database
		.connection()
		.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
		.map_err(|e| e.to_string())?;
	let mut coverage: Coverage = tx
		.query_row(
			"SELECT value FROM documents WHERE key=?1",
			[COVERAGE],
			|row| row.get::<_, String>(0),
		)
		.optional()
		.map_err(|e| e.to_string())?
		.and_then(|json| serde_json::from_str(&json).ok())
		.unwrap_or_default();
	// A new account or day boundary invalidates the old month snapshots.
	if coverage.account != account(&snapshot.calendars) || coverage.day_end_hour != day_end_hour {
		coverage = Coverage {
			account: account(&snapshot.calendars),
			day_end_hour,
			..Default::default()
		};
	}
	coverage.days.extend(dates);
	let json = serde_json::to_string(&coverage).map_err(|e| e.to_string())?;
	tx.execute("INSERT INTO documents(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [COVERAGE, &json]).map_err(|e| e.to_string())?;
	tx.commit().map_err(|e| e.to_string())?;
	day_cache::save_sync_state(path, snapshot.fetched_at, &snapshot.calendars)
		.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::calendar::sample_snapshot;

	#[test]
	fn empty_days_survive_reopening_without_claiming_the_rest_of_the_month() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		snapshot.events.clear();
		let day = snapshot.day;
		let next = day.succ_opt().unwrap();
		assert!(load(&path, &[day], 0, false).unwrap().is_none());
		save(&path, &snapshot, &[day], 0).unwrap();
		assert!(
			load(&path, &[day], 0, false)
				.unwrap()
				.unwrap()
				.events
				.is_empty()
		);
		assert!(load(&path, &[next], 0, false).unwrap().is_none());
		assert!(load(&path, &[day, next], 0, false).unwrap().is_none());
		assert!(load(&path, &[day], 3, false).unwrap().is_none());
		day_cache::clear(&path);
		assert!(load(&path, &[day], 0, false).unwrap().is_none());
	}

	#[test]
	fn range_reads_deduplicate_overnight_events_and_keep_every_day() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		let day = snapshot.day;
		let next = day.succ_opt().unwrap();
		snapshot.events.truncate(1);
		let overnight = &mut snapshot.events[0];
		overnight.start = super::super::at_clock(day, 23, 0);
		overnight.end = super::super::at_clock(next, 2, 0);
		let mut tomorrow = overnight.clone();
		tomorrow.id = "tomorrow".into();
		tomorrow.start = super::super::at_clock(next, 12, 0);
		tomorrow.end = super::super::at_clock(next, 13, 0);
		snapshot.events.push(tomorrow);
		save(&path, &snapshot, &[day, next], 0).unwrap();
		let restored = load(&path, &[day, next], 0, false).unwrap().unwrap();
		assert_eq!(restored.events, snapshot.events);
		assert_eq!(restored.calendars, snapshot.calendars);
		assert_eq!(restored.fetched_at, snapshot.fetched_at);
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
		assert!(
			load(&path, &[next], 0, false)
				.unwrap()
				.unwrap()
				.events
				.is_empty()
		);
	}

	#[test]
	fn another_account_does_not_inherit_coverage() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		let day = snapshot.day;
		let next = day.succ_opt().unwrap();
		save(&path, &snapshot, &[day], 0).unwrap();
		snapshot
			.calendars
			.iter_mut()
			.find(|calendar| calendar.primary)
			.unwrap()
			.id = "another-account".into();
		snapshot.events.clear();
		snapshot.day = next;
		save(&path, &snapshot, &[next], 0).unwrap();
		assert!(load(&path, &[day], 0, false).unwrap().is_none());
	}
}
