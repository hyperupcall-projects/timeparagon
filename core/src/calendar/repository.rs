//! The only reader of calendar views. Confirmed source rows and local desired
//! operations are kept separately; a view is assembled once in one read transaction.

use super::{Calendar, Event, Snapshot, at_clock, on_any_day, operations, range_bounds};
use crate::task_db::{Database, keys};
use chrono::{DateTime, Local, NaiveDate};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};

#[derive(Debug, Clone)]
pub struct Read {
	pub snapshot: Snapshot,
	/// Successfully fetched dates, including dates with no events.
	pub covered: Vec<NaiveDate>,
	pub revision: i64,
}

impl Read {
	pub fn complete(&self, dates: &[NaiveDate]) -> bool {
		dates.iter().all(|day| self.covered.contains(day))
	}
}

#[derive(Serialize, Deserialize)]
pub(super) struct SyncState {
	pub fetched_at: DateTime<Local>,
	pub calendars: Vec<Calendar>,
	#[serde(default)]
	pub notice: Option<String>,
}

pub(super) fn state(connection: &Connection) -> Result<Option<SyncState>, String> {
	let json: Option<String> = connection
		.query_row(
			"SELECT value FROM documents WHERE key=?1",
			[keys::SYNC_STATE],
			|r| r.get(0),
		)
		.optional()
		.map_err(|e| e.to_string())?;
	json.map(|json| serde_json::from_str(&json).map_err(|e| e.to_string()))
		.transpose()
}

pub(super) fn save_state(connection: &Connection, state: &SyncState) -> Result<(), String> {
	connection.execute("INSERT INTO documents(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![keys::SYNC_STATE, serde_json::to_string(state).map_err(|e| e.to_string())?]).map_err(|e|e.to_string())?;
	Ok(())
}

/// Read everything available locally, even if some requested dates are uncovered.
/// A missing range can still contain local creations or stored sleep.
pub fn read(path: &Path, dates: &[NaiveDate], hour: u32) -> Result<Read, String> {
	let day = *dates.first().ok_or("Calendar range is empty")?;
	let mut database = Database::open(path)?;
	let tx = database
		.connection()
		.transaction()
		.map_err(|e| e.to_string())?;
	let state = state(&tx)?;
	let mut events = events(&tx)?;
	let bounds = range_bounds(dates, hour);
	events.retain(|event| on_any_day(event, &bounds));
	let mut covered = Vec::new();
	for &day in dates {
		if super::api::incremental::covers(
			&tx,
			at_clock(day, hour, 0).to_utc(),
			at_clock(day.succ_opt().unwrap_or(day), hour, 0).to_utc(),
		)? {
			covered.push(day);
		}
	}
	let revision = crate::local_changes::read_from(&tx)
		.map_err(|e| e.to_string())?
		.calendar;
	let (calendars, fetched_at, notice) = state
		.map(|s| (s.calendars, s.fetched_at, s.notice))
		.unwrap_or_else(|| (Vec::new(), DateTime::UNIX_EPOCH.with_timezone(&Local), None));
	Ok(Read {
		snapshot: Snapshot {
			revision,
			calendars,
			events,
			day,
			fetched_at,
			notice,
		},
		covered,
		revision,
	})
}

/// Source ownership is exclusive: Calendar rows own calendar tasks, Tasks rows
/// own Google Tasks, and the stored sleep table owns nights. Local operations
/// are the sole authority over changes that Google has not yet superseded.
pub(super) fn events(connection: &Connection) -> Result<Vec<Event>, String> {
	let mut events = Vec::new();
	for (table, accepts) in [
		(
			"google_sync_events",
			(|event: &Event| event.details.sleep.is_none()) as fn(&Event) -> bool,
		),
		("google_task_records", |event: &Event| {
			!event.is_calendar_task()
		}),
		("sleep_nights", |event: &Event| {
			event.details.sleep.is_some()
		}),
	] {
		let mut query = connection
			.prepare(&format!("SELECT event FROM {table}"))
			.map_err(|e| e.to_string())?;
		for row in query
			.query_map([], |r| r.get::<_, String>(0))
			.map_err(|e| e.to_string())?
		{
			let event: Event = serde_json::from_str(&row.map_err(|e| e.to_string())?)
				.map_err(|e| e.to_string())?;
			if accepts(&event) {
				events.push(event);
			}
		}
	}
	operations::read_overlay(connection)?.apply(&mut events);
	let mut seen = HashSet::new();
	events.retain(|event| seen.insert((event.calendar_id.clone(), event.id.clone())));
	events.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
	Ok(events)
}

/// Normalize a successful Tasks feed into confirmed records. Local operations
/// remain separate and are projected only when a consumer reads a view.
pub(super) fn save_tasks(
	path: &std::path::Path,
	events: &[Event],
	replace: bool,
) -> Result<(), String> {
	let mut db = Database::open(path)?;
	let tx = db
		.connection()
		.transaction_with_behavior(TransactionBehavior::Immediate)
		.map_err(|e| e.to_string())?;

	if replace {
		tx.execute("DELETE FROM google_task_records", [])
			.map_err(|e| e.to_string())?;
	}
	for event in events
		.iter()
		.filter(|e| e.is_task() && !e.is_calendar_task())
	{
		tx.execute("INSERT INTO google_task_records(id,event) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET event=excluded.event",params![event.id,serde_json::to_string(event).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
	}
	tx.commit().map_err(|e| e.to_string())
}

/// All locally known events, for consumers such as the complete trips list.
/// Uses exactly the same source ownership and local-operation projection as a range.
pub fn all_events(path: &Path) -> Result<Vec<Event>, String> {
	let mut database = Database::open(path)?;
	let tx = database
		.connection()
		.transaction()
		.map_err(|e| e.to_string())?;
	events(&tx)
}

pub fn event(path: &Path, calendar: &str, id: &str) -> Result<Option<Event>, String> {
	Ok(all_events(path)?
		.into_iter()
		.find(|event| event.calendar_id == calendar && event.id == id))
}

#[cfg(test)]
pub(super) fn seed(path: &Path, snapshot: &Snapshot, dates: &[NaiveDate], hour: u32) {
	let mut database = Database::open(path).unwrap();
	let tx = database.connection().transaction().unwrap();
	for event in &snapshot.events {
		if event.details.sleep.is_some() {
			continue;
		}
		if event.is_task() && !event.is_calendar_task() {
			tx.execute(
				"INSERT OR REPLACE INTO google_task_records VALUES(?1,?2)",
				params![event.id, serde_json::to_string(event).unwrap()],
			)
			.unwrap();
		} else {
			super::api::incremental::store(&tx, event, event.is_calendar_task()).unwrap();
		}
	}
	for &day in dates {
		tx.execute(
			"INSERT INTO google_sync_windows VALUES(?1,?2)",
			params![
				at_clock(day, hour, 0)
					.to_utc()
					.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
				at_clock(day.succ_opt().unwrap(), hour, 0)
					.to_utc()
					.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
			],
		)
		.unwrap();
	}
	save_state(
		&tx,
		&SyncState {
			fetched_at: snapshot.fetched_at,
			calendars: snapshot.calendars.clone(),
			notice: snapshot.notice.clone(),
		},
	)
	.unwrap();
	tx.commit().unwrap();
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::calendar::{Store, sample_snapshot};
	use chrono::Duration;

	#[test]
	fn an_empty_fetched_day_has_coverage_after_restart_and_navigation_is_read_only() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		snapshot.events.clear();
		let day = snapshot.day;
		seed(&path, &snapshot, &[day], 0);
		let before = read(&path, &[day], 0).unwrap();
		assert!(before.complete(&[day]));
		assert!(before.snapshot.events.is_empty());
		assert!(
			!read(&path, &[day.succ_opt().unwrap()], 0)
				.unwrap()
				.complete(&[day.succ_opt().unwrap()])
		);
		let mut store = Store::new(0);
		store.day = day;
		store.restore(Some(path.clone()));
		store.set_day(day.succ_opt().unwrap(), day);
		store.set_day(day, day);
		assert!(store.is_covered(day));
		assert!(store.events.is_empty());
		assert_eq!(read(&path, &[day], 0).unwrap().revision, before.revision);
	}

	#[test]
	fn range_views_hold_one_copy_of_overnight_events_and_shared_tasks() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		let day = snapshot.day;
		let next = day.succ_opt().unwrap();
		let mut overnight = snapshot.events[0].clone();
		overnight.all_day = false;
		overnight.start = at_clock(day, 23, 0);
		overnight.end = at_clock(next, 2, 0);
		let mut task = overnight.clone();
		task.id = "task:shared".into();
		task.calendar_id = "tasks:list".into();
		task.all_day = true;
		snapshot.events = vec![overnight, task];
		seed(&path, &snapshot, &[day, next], 0);
		let result = read(&path, &[day, next], 0).unwrap();
		assert_eq!(result.snapshot.events.len(), 2);
		let mut store = Store::with_snapshot(snapshot);
		store.install(result, &[day, next]);
		assert_eq!(store.events.len(), 2);
		assert_eq!(store.events_on(day).len(), 2);
		assert_eq!(store.events_on(next).len(), 2);
		assert!(std::ptr::eq(
			store.events_on(day)[0],
			store.events_on(next)[0]
		));
	}

	#[test]
	fn pending_moves_win_without_modifying_confirmed_records_and_rejection_restores_them() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		snapshot.events.truncate(1);
		let day = snapshot.day;
		let next = day.succ_opt().unwrap();
		seed(&path, &snapshot, &[day, next], 0);
		let before = read(&path, &[day, next], 0).unwrap();
		let mut moved = snapshot.events[0].clone();
		moved.start += Duration::days(1);
		moved.end += Duration::days(1);
		moved.title = "Local edit".into();
		let mut database = Database::open(&path).unwrap();
		database
			.connection()
			.execute(
				"INSERT INTO google_entities(id,event,sending_since) VALUES(?1,?2,?3)",
				params![
					format!("{}\n{}", moved.calendar_id, moved.id),
					serde_json::to_string(&moved).unwrap(),
					crate::clock::utc_now().to_rfc3339()
				],
			)
			.unwrap();
		assert!(read(&path, &[day], 0).unwrap().snapshot.events.is_empty());
		let current = read(&path, &[next], 0).unwrap();
		assert_eq!(current.snapshot.events[0].title, "Local edit");
		assert!(current.snapshot.events[0].details.sending);
		let json: String = database
			.connection()
			.query_row("SELECT event FROM google_sync_events", [], |r| r.get(0))
			.unwrap();
		assert_eq!(
			serde_json::from_str::<Event>(&json).unwrap().title,
			snapshot.events[0].title
		);
		let mut store = Store::with_snapshot(snapshot.clone());
		store.install(current, &[next]);
		store.apply_snapshot(before.snapshot);
		assert_eq!(
			store.events[0].title, "Local edit",
			"older worker results cannot replace newer data"
		);
		database
			.connection()
			.execute("DELETE FROM google_entities", [])
			.unwrap();
		assert_eq!(
			read(&path, &[day], 0).unwrap().snapshot.events,
			snapshot.events
		);
	}

	#[test]
	fn a_local_creation_is_visible_even_when_its_day_has_never_been_fetched() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let snapshot = sample_snapshot();
		let event = &snapshot.events[0];
		let mut db = Database::open(&path).unwrap();
		db.connection()
			.execute(
				"INSERT INTO google_entities(id,event) VALUES(?1,?2)",
				params![
					format!("{}\n{}", event.calendar_id, event.id),
					serde_json::to_string(event).unwrap()
				],
			)
			.unwrap();
		let result = read(&path, &[snapshot.day], 0).unwrap();
		assert!(!result.complete(&[snapshot.day]));
		assert_eq!(result.snapshot.events, vec![event.clone()]);
	}

	#[test]
	fn deleted_cached_sleep_is_never_resurrected_from_a_calendar_row() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut night = sample_snapshot().events.remove(0);
		night.details.sleep = Some(super::super::sleep::Record::planned(night.end.date_naive()));
		let mut db = Database::open(&path).unwrap();
		super::super::api::incremental::store(db.connection(), &night, false).unwrap();
		assert_eq!(
			read(&path, &[night.start.date_naive()], 0)
				.unwrap()
				.snapshot
				.events
				.len(),
			1
		);
		db.connection()
			.execute("DELETE FROM sleep_nights", [])
			.unwrap();
		assert!(
			read(&path, &[night.start.date_naive()], 0)
				.unwrap()
				.snapshot
				.events
				.is_empty()
		);
	}
}
