//! Incremental sync with Google: Calendar sync tokens and Tasks' `updatedMin`.
//!
//! The first sync of a calendar is a full one: it lists the calendar once to
//! get a sync token, then fetches the occurrences of the days asked for. From
//! then on each sync asks Google only for what changed since that token, and
//! applies the changes to a local copy of the events (`google_sync_events`).
//! A snapshot is then read from that copy, so the apps see the same
//! [`Snapshot`] whichever way it was made.
//!
//! The copy only holds the days that have been asked for (its *windows*),
//! plus every one-off task event on the app's calendar, which the task lists
//! need wherever they are dated. Asking for days outside the windows fetches
//! just those days and adds them. A changed recurring series has its
//! occurrences in the windows fetched again, since Google reports the series
//! rather than each occurrence.
//!
//! Google Tasks has no sync tokens: each list is asked for the tasks updated
//! since the latest change already seen (its *watermark*), deletions
//! included, and the tasks are kept in `google_sync_tasks`.
//!
//! A full sync happens only when there is nothing to carry on from: the
//! first sync, a calendar that has just appeared, a sync token Google has
//! expired (`410 Gone`), another account, or [`reset`] (the Full sync
//! button). Every table here stays on this device: database sync never sees
//! them, as each device keeps its own place in Google's change stream.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{
	EVENTS, EventEntry, EventList, FetchedTasks, Range, TASK_LISTS, TASKS, TaskEntry, TaskLists,
	TaskPage, day_bounds, fetch_calendars, finish, get, parse_event_in_range, resolve_app_calendar,
	task_events,
};
use crate::calendar::colors;
use crate::calendar::day_cache::SavedEvent;
use crate::calendar::google::GoogleError;
use crate::calendar::{Calendar, Event, Snapshot};
use crate::config::CalendarSettings;
use crate::task_db::Database;

/// One sync at a time: two at once would read the same tokens and apply the
/// same changes twice, and the later one could store the older token.
static SYNCING: Mutex<()> = Mutex::new(());

/// Kept with the tables, created on first use so older builds can still open
/// the database.
const CREATE: &str = "
    CREATE TABLE IF NOT EXISTS google_sync_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS google_sync_calendars (calendar_id TEXT PRIMARY KEY, sync_token TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS google_sync_windows (start_at TEXT NOT NULL, end_at TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS google_sync_events (
        calendar_id TEXT NOT NULL,
        id TEXT NOT NULL,
        series TEXT,
        start_at TEXT NOT NULL,
        end_at TEXT NOT NULL,
        pinned INTEGER NOT NULL,
        event TEXT NOT NULL,
        PRIMARY KEY (calendar_id, id)
    );
    CREATE INDEX IF NOT EXISTS google_sync_events_series ON google_sync_events (calendar_id, series);
    CREATE TABLE IF NOT EXISTS google_sync_task_lists (list_id TEXT PRIMARY KEY, watermark TEXT);
    CREATE TABLE IF NOT EXISTS google_sync_tasks (
        list_id TEXT NOT NULL,
        task_id TEXT NOT NULL,
        entry TEXT NOT NULL,
        PRIMARY KEY (list_id, task_id)
    );";

const TABLES: [&str; 6] = [
	"google_sync_meta",
	"google_sync_calendars",
	"google_sync_windows",
	"google_sync_events",
	"google_sync_task_lists",
	"google_sync_tasks",
];

/// How far a Tasks watermark is wound back, so a task saved at the same
/// moment as the last one seen, but listed a little later, is not missed.
/// Tasks seen again are simply stored again.
const TASKS_OVERLAP: Duration = Duration::seconds(60);

/// Days kept in the windows that are not in view: from this long ago...
const KEPT_PAST: Duration = Duration::days(90);
/// ...to this far ahead. Further windows are dropped, so a changed series
/// is not fetched again for days nobody is looking at.
const KEPT_FUTURE: Duration = Duration::days(365);

/// Whether syncs are incremental: the `calendar.incremental_sync` setting.
pub fn enabled() -> bool {
	let Some(path) = crate::task_db::path() else {
		return false;
	};
	match Database::open(&path) {
		Ok(mut database) => CalendarSettings::stored_incremental_sync(database.connection()),
		Err(_) => CalendarSettings::default().incremental_sync,
	}
}

/// Forgets every sync token, window and stored event and task, so the next
/// sync is a full one. What the Full sync button does.
pub fn reset() -> Result<(), String> {
	let path = crate::task_db::path().ok_or("Cannot locate the local database")?;
	let _syncing = SYNCING.lock().map_err(|_| "Sync is unavailable")?;
	let mut database = open(&path)?;
	clear(database.connection()).map_err(|e| e.to_string())
}

fn clear(connection: &Connection) -> rusqlite::Result<()> {
	for table in TABLES {
		connection.execute(&format!("DELETE FROM {table}"), [])?;
	}
	Ok(())
}

/// Drops a deleted task from the stored lists, so the next snapshot does not
/// show it again before Google's own report of the deletion arrives.
pub(super) fn forget_task(task_id: &str) {
	let Some(path) = crate::task_db::path() else {
		return;
	};
	if let Ok(mut database) = open(&path)
		&& let Err(error) = database
			.connection()
			.execute("DELETE FROM google_sync_tasks WHERE task_id=?1", [task_id])
	{
		log::warn!("could not forget a synced task: {error}");
	}
}

fn open(path: &Path) -> Result<Database, String> {
	let mut database = Database::open(path)?;
	database
		.connection()
		.execute_batch(CREATE)
		.map_err(|e| e.to_string())?;
	Ok(database)
}

/// A UTC instant in one fixed format, so stored times compare as text.
fn stamp(at: DateTime<Utc>) -> String {
	at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_stamp(text: &str) -> Option<DateTime<Utc>> {
	DateTime::parse_from_rfc3339(text)
		.ok()
		.map(|at| at.with_timezone(&Utc))
}

/// A stretch of time the local copy holds every event of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Window {
	start: DateTime<Utc>,
	end: DateTime<Utc>,
}

impl Window {
	fn overlaps(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
		// Google's own rule for timeMin and timeMax, so the copy holds
		// exactly what a listing of the window would.
		start < self.end && end > self.start
	}
}

/// `windows` sorted, with overlapping or touching ones joined.
fn merge(mut windows: Vec<Window>) -> Vec<Window> {
	windows.sort();
	let mut merged: Vec<Window> = Vec::new();
	for window in windows {
		match merged.last_mut() {
			Some(last) if window.start <= last.end => last.end = last.end.max(window.end),
			_ => merged.push(window),
		}
	}
	merged
}

/// The parts of `wanted` that no window in `covered` (merged) holds.
fn gaps(wanted: Window, covered: &[Window]) -> Vec<Window> {
	let mut gaps = Vec::new();
	let mut from = wanted.start;
	for window in covered {
		if window.end <= from || window.start >= wanted.end {
			continue;
		}
		if window.start > from {
			gaps.push(Window {
				start: from,
				end: window.start,
			});
		}
		from = from.max(window.end);
		if from >= wanted.end {
			break;
		}
	}
	if from < wanted.end {
		gaps.push(Window {
			start: from,
			end: wanted.end,
		});
	}
	gaps
}

/// The windows worth keeping: those near `now`, and always `wanted`.
fn prune(windows: Vec<Window>, wanted: Window, now: DateTime<Utc>) -> Vec<Window> {
	let near = Window {
		start: now - KEPT_PAST,
		end: now + KEPT_FUTURE,
	};
	windows
		.into_iter()
		.filter(|window| {
			window.overlaps(wanted.start, wanted.end) || window.overlaps(near.start, near.end)
		})
		.collect()
}

fn in_windows(event: &Event, windows: &[Window]) -> bool {
	let (start, end) = (event.start.to_utc(), event.end.to_utc());
	windows.iter().any(|window| window.overlaps(start, end))
}

/// A one-off task event on the app's calendar: kept wherever it is dated,
/// since the task lists and block planning need every one of them.
fn pinned(event: &Event, app_calendar: Option<&str>) -> bool {
	Some(event.calendar_id.as_str()) == app_calendar
		&& event.details.task_data.is_some()
		&& !event.is_recurring()
}

/// A Google event as the app sees it, whatever days it falls on.
fn parse(entry: EventEntry, calendar: &Calendar) -> Option<Event> {
	parse_event_in_range(entry, calendar, NaiveDate::MIN, NaiveDate::MAX)
}

fn cancelled(entry: &EventEntry) -> bool {
	entry.status.as_deref() == Some("cancelled")
}

fn calendar_url(calendar: &Calendar) -> String {
	format!("{EVENTS}/{}/events", urlencoding::encode(&calendar.id))
}

fn with_page(url: &str, page: Option<&str>) -> String {
	match page {
		Some(page) => format!("{url}&pageToken={}", urlencoding::encode(page)),
		None => url.to_owned(),
	}
}

/// What one calendar's sync brought back, to be stored once every calendar
/// has answered.
enum CalendarSync {
	/// Changes since the stored token, and the events of days newly asked for.
	Changes {
		token: String,
		changes: Vec<Change>,
		added: Vec<Event>,
	},
	/// Everything the windows hold, replacing what was stored.
	Full { token: String, events: Vec<Event> },
}

enum Change {
	/// A one-off event, or one occurrence of a series edited on its own.
	Upsert(Box<Event>),
	/// A deleted event: a one-off, one occurrence, or a whole series.
	Remove(String),
	/// A series that changed, with all its occurrences in the windows now.
	Series(String, Vec<Event>),
}

/// Every event of `calendar` between the window's ends, one per occurrence.
/// `path` is `events` or a series' `events/<id>/instances`.
fn list_window(
	access_token: &str,
	calendar: &Calendar,
	path: &str,
	window: Window,
) -> Result<Vec<Event>, GoogleError> {
	let mut url = format!(
		"{EVENTS}/{}/{path}?maxResults=2500&timeMin={}&timeMax={}",
		urlencoding::encode(&calendar.id),
		urlencoding::encode(&window.start.to_rfc3339()),
		urlencoding::encode(&window.end.to_rfc3339()),
	);
	if path == "events" {
		url.push_str("&singleEvents=true");
	}
	let mut events = Vec::new();
	let mut page: Option<String> = None;
	loop {
		let list: EventList = get(&with_page(&url, page.as_deref()), access_token)?;
		for mut entry in list.items {
			if cancelled(&entry) {
				continue;
			}
			entry.options.default_reminders = list.default_reminders.clone();
			if let Some(event) = parse(entry, calendar) {
				events.push(event);
			}
		}
		page = list.next_page_token;
		if page.is_none() {
			return Ok(events);
		}
	}
}

fn list_windows(
	access_token: &str,
	calendar: &Calendar,
	windows: &[Window],
) -> Result<Vec<Event>, GoogleError> {
	let mut events = Vec::new();
	for window in windows {
		events.extend(list_window(access_token, calendar, "events", *window)?);
	}
	Ok(events)
}

/// Every occurrence of the series `id` in the windows. A series Google no
/// longer has has none.
fn series_instances(
	access_token: &str,
	calendar: &Calendar,
	id: &str,
	windows: &[Window],
) -> Result<Vec<Event>, GoogleError> {
	let path = format!("events/{}/instances", urlencoding::encode(id));
	let mut events = Vec::new();
	for window in windows {
		match list_window(access_token, calendar, &path, *window) {
			Ok(found) => events.extend(found),
			Err(error) if matches!(error.status, Some(404 | 410)) => return Ok(Vec::new()),
			Err(error) => return Err(error),
		}
	}
	Ok(events)
}

/// A new sync token for `calendar`, by listing all of it. Only the token is
/// wanted, so the events are left out of the answer, except on the app's
/// calendar, where its one-off task events come back with it.
fn new_token(
	access_token: &str,
	calendar: &Calendar,
	keep_tasks: bool,
) -> Result<(String, Vec<Event>), GoogleError> {
	let mut url = format!("{}?maxResults=2500", calendar_url(calendar));
	if !keep_tasks {
		url.push_str("&fields=nextPageToken%2CnextSyncToken");
	}
	let mut tasks = Vec::new();
	let mut page: Option<String> = None;
	loop {
		let list: EventList = get(&with_page(&url, page.as_deref()), access_token)?;
		for mut entry in list.items {
			if cancelled(&entry)
				|| !entry.recurrence.is_empty()
				|| entry.recurring_event_id.is_some()
				|| entry.extended_properties["private"]["ttKind"].as_str() != Some("task")
			{
				continue;
			}
			entry.options.default_reminders = list.default_reminders.clone();
			if let Some(event) = parse(entry, calendar) {
				tasks.push(event);
			}
		}
		if let Some(token) = list.next_sync_token {
			return Ok((token, tasks));
		}
		page = list.next_page_token;
		if page.is_none() {
			return Err(GoogleError::from_status(
				"Google Calendar",
				500,
				"Google gave no sync token",
			));
		}
	}
}

/// The changes to `calendar` since `token`, and the token to carry on from.
/// `None` when Google has expired the token and a full sync is needed.
fn changes_since(
	access_token: &str,
	calendar: &Calendar,
	token: &str,
) -> Result<Option<(Vec<EventEntry>, String)>, GoogleError> {
	let url = format!(
		"{}?maxResults=2500&syncToken={}",
		calendar_url(calendar),
		urlencoding::encode(token)
	);
	let mut entries = Vec::new();
	let mut page: Option<String> = None;
	loop {
		let list: EventList = match get(&with_page(&url, page.as_deref()), access_token) {
			Ok(list) => list,
			Err(error) if error.status == Some(410) => return Ok(None),
			Err(error) => return Err(error),
		};
		for mut entry in list.items {
			entry.options.default_reminders = list.default_reminders.clone();
			entries.push(entry);
		}
		if let Some(next) = list.next_sync_token {
			return Ok(Some((entries, next)));
		}
		page = list.next_page_token;
		if page.is_none() {
			return Err(GoogleError::from_status(
				"Google Calendar",
				500,
				"Google gave no sync token",
			));
		}
	}
}

/// Lists `calendar` from scratch: a new token first, so a change made while
/// the windows are listed comes with the next sync.
fn full_calendar(
	access_token: &str,
	calendar: &Calendar,
	windows: &[Window],
	app_calendar: Option<&str>,
) -> Result<CalendarSync, GoogleError> {
	let (token, mut events) = new_token(
		access_token,
		calendar,
		Some(calendar.id.as_str()) == app_calendar,
	)?;
	events.extend(list_windows(access_token, calendar, windows)?);
	Ok(CalendarSync::Full { token, events })
}

/// Brings one calendar up to date: changes since its token when it has one,
/// otherwise a full listing. `covered` is every window the copy will hold,
/// `added` the parts of it new with this sync.
fn sync_calendar(
	access_token: &str,
	calendar: &Calendar,
	token: Option<&str>,
	covered: &[Window],
	added: &[Window],
	app_calendar: Option<&str>,
) -> Result<CalendarSync, GoogleError> {
	let Some(token) = token else {
		return full_calendar(access_token, calendar, covered, app_calendar);
	};
	let Some((entries, token)) = changes_since(access_token, calendar, token)? else {
		log::info!(
			"Google expired the sync token of {}; syncing it in full",
			calendar.name
		);
		return full_calendar(access_token, calendar, covered, app_calendar);
	};
	let mut changes = Vec::new();
	let mut series = BTreeSet::new();
	for entry in entries {
		if cancelled(&entry) {
			// A deleted series comes back as its bare ID, like any deletion.
			changes.push(Change::Remove(entry.id));
		} else if !entry.recurrence.is_empty() {
			series.insert(entry.id);
		} else {
			let id = entry.id.clone();
			changes.push(match parse(entry, calendar) {
				Some(event) => Change::Upsert(Box::new(event)),
				None => Change::Remove(id),
			});
		}
	}
	for id in series {
		let events = series_instances(access_token, calendar, &id, covered)?;
		changes.push(Change::Series(id, events));
	}
	let added = list_windows(access_token, calendar, added)?;
	Ok(CalendarSync::Changes {
		token,
		changes,
		added,
	})
}

fn store(connection: &Connection, event: &Event, pinned: bool) -> Result<(), String> {
	let json = serde_json::to_string(&SavedEvent::from(event)).map_err(|e| e.to_string())?;
	connection
		.execute(
			"INSERT INTO google_sync_events (calendar_id,id,series,start_at,end_at,pinned,event)
             VALUES (?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(calendar_id,id) DO UPDATE SET series=excluded.series,
               start_at=excluded.start_at,end_at=excluded.end_at,pinned=excluded.pinned,
               event=excluded.event",
			params![
				event.calendar_id,
				event.id,
				event.details.recurring_event_id,
				stamp(event.start.to_utc()),
				stamp(event.end.to_utc()),
				pinned,
				json,
			],
		)
		.map(|_| ())
		.map_err(|e| e.to_string())
}

/// Keeps `event` when it belongs in the copy, and drops it otherwise: an
/// event moved out of every window is no longer wanted.
fn keep(
	connection: &Connection,
	event: &Event,
	windows: &[Window],
	app_calendar: Option<&str>,
) -> Result<(), String> {
	let pinned = pinned(event, app_calendar);
	if pinned || in_windows(event, windows) {
		store(connection, event, pinned)
	} else {
		connection
			.execute(
				"DELETE FROM google_sync_events WHERE calendar_id=?1 AND id=?2",
				params![event.calendar_id, event.id],
			)
			.map(|_| ())
			.map_err(|e| e.to_string())
	}
}

fn apply(
	connection: &Connection,
	calendar: &Calendar,
	sync: CalendarSync,
	windows: &[Window],
	app_calendar: Option<&str>,
) -> Result<(), String> {
	let token = match sync {
		CalendarSync::Full { token, events } => {
			connection
				.execute(
					"DELETE FROM google_sync_events WHERE calendar_id=?1",
					[&calendar.id],
				)
				.map_err(|e| e.to_string())?;
			for event in &events {
				keep(connection, event, windows, app_calendar)?;
			}
			token
		}
		CalendarSync::Changes {
			token,
			changes,
			added,
		} => {
			for change in changes {
				match change {
					Change::Upsert(event) => keep(connection, &event, windows, app_calendar)?,
					Change::Remove(id) => {
						connection
							.execute(
								"DELETE FROM google_sync_events WHERE calendar_id=?1 AND (id=?2 OR series=?2)",
								params![calendar.id, id],
							)
							.map_err(|e| e.to_string())?;
					}
					Change::Series(id, events) => {
						connection
							.execute(
								"DELETE FROM google_sync_events WHERE calendar_id=?1 AND (id=?2 OR series=?2)",
								params![calendar.id, id],
							)
							.map_err(|e| e.to_string())?;
						for event in &events {
							keep(connection, event, windows, app_calendar)?;
						}
					}
				}
			}
			for event in &added {
				keep(connection, event, windows, app_calendar)?;
			}
			token
		}
	};
	connection
		.execute(
			"INSERT INTO google_sync_calendars (calendar_id,sync_token) VALUES (?1,?2)
             ON CONFLICT(calendar_id) DO UPDATE SET sync_token=excluded.sync_token",
			params![calendar.id, token],
		)
		.map(|_| ())
		.map_err(|e| e.to_string())
}

/// What a Google Tasks list's sync brought back.
struct ListSync {
	id: String,
	/// Every task on the list, replacing what was stored, rather than changes.
	full: bool,
	tasks: Vec<TaskEntry>,
	watermark: Option<DateTime<Utc>>,
}

/// The latest change among `tasks`, or `previous` when none is later.
fn watermark(previous: Option<DateTime<Utc>>, tasks: &[TaskEntry]) -> Option<DateTime<Utc>> {
	tasks
		.iter()
		.filter_map(|task| task.updated)
		.chain(previous)
		.max()
}

/// The tasks on `list` updated since `since`, or every task when there is no
/// watermark yet. Deleted and hidden tasks are included so they can be
/// removed and kept track of.
fn sync_list(
	access_token: &str,
	list: &str,
	since: Option<DateTime<Utc>>,
) -> Result<ListSync, GoogleError> {
	let mut url = format!(
		"{TASKS}/{}/tasks?showCompleted=true&showHidden=true&showDeleted=true&maxResults=100",
		urlencoding::encode(list)
	);
	if let Some(since) = since {
		url.push_str("&updatedMin=");
		url.push_str(&urlencoding::encode(&(since - TASKS_OVERLAP).to_rfc3339()));
	}
	let mut tasks = Vec::new();
	let mut page: Option<String> = None;
	loop {
		let found: TaskPage = get(&with_page(&url, page.as_deref()), access_token)?;
		tasks.extend(found.items.into_iter().filter(|task| !task.id.is_empty()));
		page = found.next_page_token;
		if page.is_none() {
			break;
		}
	}
	Ok(ListSync {
		id: list.to_owned(),
		full: since.is_none(),
		watermark: watermark(since, &tasks),
		tasks,
	})
}

/// The task lists, in Google's order, and each one's sync.
fn sync_tasks(access_token: &str, watermarks: &HashMap<String, Option<DateTime<Utc>>>) -> TaskSync {
	let mut lists: TaskLists = get(&format!("{TASK_LISTS}?maxResults=100"), access_token)?;
	while let Some(token) = lists.next_page_token.take() {
		let next: TaskLists = get(
			&format!(
				"{TASK_LISTS}?maxResults=100&pageToken={}",
				urlencoding::encode(&token)
			),
			access_token,
		)?;
		lists.items.extend(next.items);
		lists.next_page_token = next.next_page_token;
	}
	let mut named = Vec::new();
	let mut synced = Vec::new();
	for list in lists.items {
		let since = watermarks.get(&list.id).copied().flatten();
		synced.push(sync_list(access_token, &list.id, since)?);
		let name = if list.title.is_empty() {
			"Tasks".to_owned()
		} else {
			list.title
		};
		named.push((name, list.id));
	}
	Ok((named, synced))
}

fn apply_lists(
	connection: &Connection,
	lists: &[(String, String)],
	synced: Vec<ListSync>,
) -> Result<(), String> {
	let sql = |result: rusqlite::Result<usize>| result.map(|_| ()).map_err(|e| e.to_string());
	// Lists that are gone take their tasks with them.
	let live: BTreeSet<&str> = lists.iter().map(|(_, id)| id.as_str()).collect();
	let stored: Vec<String> = {
		let mut query = connection
            .prepare("SELECT list_id FROM google_sync_task_lists UNION SELECT DISTINCT list_id FROM google_sync_tasks")
            .map_err(|e| e.to_string())?;
		query
			.query_map([], |row| row.get(0))
			.map_err(|e| e.to_string())?
			.collect::<Result<_, _>>()
			.map_err(|e| e.to_string())?
	};
	for list in stored.iter().filter(|id| !live.contains(id.as_str())) {
		sql(connection.execute("DELETE FROM google_sync_tasks WHERE list_id=?1", [list]))?;
		sql(connection.execute(
			"DELETE FROM google_sync_task_lists WHERE list_id=?1",
			[list],
		))?;
	}
	for list in synced {
		if list.full {
			sql(connection.execute("DELETE FROM google_sync_tasks WHERE list_id=?1", [&list.id]))?;
		}
		for task in &list.tasks {
			if task.deleted {
				sql(connection.execute(
					"DELETE FROM google_sync_tasks WHERE list_id=?1 AND task_id=?2",
					params![list.id, task.id],
				))?;
				continue;
			}
			// A task moved to this list is not reported deleted on the list
			// it left, so it is dropped there.
			sql(connection.execute(
				"DELETE FROM google_sync_tasks WHERE task_id=?1 AND list_id!=?2",
				params![task.id, list.id],
			))?;
			let entry = serde_json::to_string(task).map_err(|e| e.to_string())?;
			sql(connection.execute(
				"INSERT INTO google_sync_tasks (list_id,task_id,entry) VALUES (?1,?2,?3)
                 ON CONFLICT(list_id,task_id) DO UPDATE SET entry=excluded.entry",
				params![list.id, task.id, entry],
			))?;
		}
		sql(connection.execute(
			"INSERT INTO google_sync_task_lists (list_id,watermark) VALUES (?1,?2)
             ON CONFLICT(list_id) DO UPDATE SET watermark=excluded.watermark",
			params![list.id, list.watermark.map(stamp)],
		))?;
	}
	Ok(())
}

fn stored_tasks(connection: &Connection, list: &str) -> Result<Vec<TaskEntry>, String> {
	let mut query = connection
		.prepare("SELECT entry FROM google_sync_tasks WHERE list_id=?1 ORDER BY task_id")
		.map_err(|e| e.to_string())?;
	let rows: Vec<String> = query
		.query_map([list], |row| row.get(0))
		.map_err(|e| e.to_string())?
		.collect::<Result<_, _>>()
		.map_err(|e| e.to_string())?;
	Ok(rows
		.iter()
		.filter_map(|json| serde_json::from_str(json).ok())
		.collect())
}

/// The task lists as calendars and their tasks as chips, from the stored
/// tasks: what the full fetch makes of the same tasks.
fn task_snapshot(
	access_token: &str,
	path: &Path,
	lists: &[(String, String)],
	app_calendar: Option<&str>,
) -> Result<FetchedTasks, GoogleError> {
	let mut database = open(path)?;
	let mut calendars = Vec::new();
	let mut events = Vec::new();
	for (name, list) in lists {
		let id = format!("tasks:{list}");
		calendars.push(Calendar {
			id: id.clone(),
			name: name.clone(),
			color: colors::TASK,
			primary: false,
			selected_by_google: true,
		});
		let tasks = stored_tasks(database.connection(), list)?;
		events.extend(task_events(
			access_token,
			tasks,
			&id,
			colors::TASK,
			app_calendar,
		)?);
	}
	Ok((calendars, events, None))
}

fn load_windows(connection: &Connection) -> Result<Vec<Window>, String> {
	let mut query = connection
		.prepare("SELECT start_at,end_at FROM google_sync_windows")
		.map_err(|e| e.to_string())?;
	let rows: Vec<(String, String)> = query
		.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
		.map_err(|e| e.to_string())?
		.collect::<Result<_, _>>()
		.map_err(|e| e.to_string())?;
	Ok(merge(
		rows.iter()
			.filter_map(|(start, end)| {
				Some(Window {
					start: parse_stamp(start)?,
					end: parse_stamp(end)?,
				})
			})
			.collect(),
	))
}

fn save_windows(connection: &Connection, windows: &[Window]) -> Result<(), String> {
	connection
		.execute("DELETE FROM google_sync_windows", [])
		.map_err(|e| e.to_string())?;
	for window in windows {
		connection
			.execute(
				"INSERT INTO google_sync_windows (start_at,end_at) VALUES (?1,?2)",
				params![stamp(window.start), stamp(window.end)],
			)
			.map_err(|e| e.to_string())?;
	}
	// Events left outside every window are no longer looked after.
	connection
        .execute(
            "DELETE FROM google_sync_events WHERE pinned=0 AND NOT EXISTS (
                SELECT 1 FROM google_sync_windows w
                WHERE google_sync_events.start_at < w.end_at AND google_sync_events.end_at > w.start_at)",
            [],
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn string_map(connection: &Connection, sql: &str) -> Result<HashMap<String, String>, String> {
	let mut query = connection.prepare(sql).map_err(|e| e.to_string())?;
	query
		.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
		.map_err(|e| e.to_string())?
		.collect::<Result<_, _>>()
		.map_err(|e| e.to_string())
}

/// The account the stored copy belongs to: its primary calendar's ID, which
/// is the account's address. A copy from another account is thrown away.
fn check_account(connection: &Connection, calendars: &[Calendar]) -> Result<(), String> {
	let Some(primary) = calendars.iter().find(|calendar| calendar.primary) else {
		return Ok(());
	};
	let stored: Option<String> = connection
		.query_row(
			"SELECT value FROM google_sync_meta WHERE key='account'",
			[],
			|row| row.get(0),
		)
		.optional()
		.map_err(|e| e.to_string())?;
	if stored.as_deref() != Some(primary.id.as_str()) {
		if stored.is_some() {
			log::info!("another Google account is connected; syncing it in full");
		}
		clear(connection).map_err(|e| e.to_string())?;
		connection
			.execute(
				"INSERT INTO google_sync_meta (key,value) VALUES ('account',?1)",
				[&primary.id],
			)
			.map_err(|e| e.to_string())?;
	}
	Ok(())
}

/// The stored events of `calendar` in `wanted`, plus its pinned ones,
/// colored as the calendar is now. An all-day event counts by its dates, as
/// in a full fetch.
fn stored_events(
	connection: &Connection,
	calendar: &Calendar,
	wanted: Window,
	first: NaiveDate,
	last: NaiveDate,
) -> Result<Vec<Event>, String> {
	let mut query = connection
		.prepare(
			"SELECT event,pinned FROM google_sync_events WHERE calendar_id=?1
             AND (pinned=1 OR (start_at < ?3 AND end_at > ?2))",
		)
		.map_err(|e| e.to_string())?;
	let rows: Vec<(String, bool)> = query
		.query_map(
			params![calendar.id, stamp(wanted.start), stamp(wanted.end)],
			|row| Ok((row.get(0)?, row.get(1)?)),
		)
		.map_err(|e| e.to_string())?
		.collect::<Result<_, _>>()
		.map_err(|e| e.to_string())?;
	Ok(rows
		.into_iter()
		.filter_map(|(json, pinned)| {
			let event = Event::from(serde_json::from_str::<SavedEvent>(&json).ok()?);
			let in_range = !event.all_day
				|| (event.start.date_naive() <= last && event.end.date_naive() > first);
			(pinned || in_range).then_some(event)
		})
		.map(|mut event| {
			event.color = event
				.details
				.color_id
				.as_deref()
				.and_then(colors::resolve)
				.unwrap_or(calendar.color);
			event
		})
		.collect())
}

/// The task lists by name and ID, and each one's sync.
type TaskSync = Result<(Vec<(String, String)>, Vec<ListSync>), GoogleError>;
type CalendarSyncs<'a> = Vec<(&'a Calendar, CalendarSync)>;

/// Asks Google about every calendar and the tasks: the tasks alongside the
/// calendars, and up to four calendars at once. Tests ask one at a time, as
/// their stand-in for Google answers on its own thread only.
fn fetch_all<'a>(
	access_token: &str,
	calendars: &'a [Calendar],
	tokens: &HashMap<String, String>,
	watermarks: &HashMap<String, Option<DateTime<Utc>>>,
	windows: &[Window],
	added: &[Window],
	app_calendar: Option<&str>,
) -> Result<(CalendarSyncs<'a>, TaskSync), String> {
	let one = |calendar: &'a Calendar| {
		let token = tokens.get(&calendar.id).map(String::as_str);
		sync_calendar(access_token, calendar, token, windows, added, app_calendar)
			.map(|sync| (calendar, sync))
	};
	if cfg!(test) {
		let tasks = sync_tasks(access_token, watermarks);
		let synced = calendars.iter().map(one).collect::<Result<_, _>>()?;
		return Ok((synced, tasks));
	}
	std::thread::scope(|scope| {
		let tasks = scope.spawn(|| sync_tasks(access_token, watermarks));
		let mut synced = Vec::new();
		for chunk in calendars.chunks(4) {
			let workers: Vec<_> = chunk
				.iter()
				.map(|calendar| scope.spawn(move || one(calendar)))
				.collect();
			for worker in workers {
				synced.push(worker.join().map_err(|_| "Calendar worker failed")??);
			}
		}
		Ok((synced, tasks.join().map_err(|_| "Tasks worker failed")?))
	})
}

/// [`super::fetch_range`] by incremental sync: brings the stored copy up to
/// date with Google's changes and answers from it.
pub(super) fn fetch_range(
	access_token: &str,
	day: NaiveDate,
	first: NaiveDate,
	last: NaiveDate,
	day_end_hour: u32,
	app_calendar: Option<&str>,
) -> Result<Snapshot, String> {
	let path = crate::task_db::path().ok_or("Cannot locate the local database")?;
	let _syncing = SYNCING.lock().map_err(|_| "Sync is unavailable")?;
	let fetch_started = crate::clock::now();
	let synced = sync(
		&path,
		access_token,
		first,
		last,
		day_end_hour,
		app_calendar,
		fetch_started.to_utc(),
	)?;
	let task_result = match synced.task_lists {
		Ok(lists) => task_snapshot(access_token, &path, &lists, synced.app_calendar.as_deref()),
		Err(error) => Err(error),
	};
	finish(
		access_token,
		synced.calendars,
		synced.events,
		task_result,
		Range {
			day,
			first,
			last,
			day_end_hour,
			fetched_at: fetch_started,
		},
	)
}

/// What a sync leaves for the snapshot: the calendars, their events in the
/// range, and the task lists (whose tasks are stored), or why Tasks failed.
struct Synced {
	calendars: Vec<Calendar>,
	events: Vec<Event>,
	app_calendar: Option<String>,
	task_lists: Result<Vec<(String, String)>, GoogleError>,
}

/// Brings the copy in the database at `path` up to date and reads the
/// events between `first` and `last` back from it.
fn sync(
	path: &Path,
	access_token: &str,
	first: NaiveDate,
	last: NaiveDate,
	day_end_hour: u32,
	app_calendar: Option<&str>,
	now: DateTime<Utc>,
) -> Result<Synced, String> {
	let mut calendars = fetch_calendars(access_token)?;
	let resolved = resolve_app_calendar(access_token, &mut calendars, app_calendar);
	let app_calendar = resolved.as_deref();

	let mut database = open(path)?;
	let (covered, tokens, watermarks) = {
		let connection = database.connection();
		check_account(connection, &calendars)?;
		let tokens = string_map(
			connection,
			"SELECT calendar_id,sync_token FROM google_sync_calendars",
		)?;
		let watermarks: HashMap<String, Option<DateTime<Utc>>> = string_map(
			connection,
			"SELECT list_id,COALESCE(watermark,'') FROM google_sync_task_lists",
		)?
		.into_iter()
		.map(|(list, watermark)| (list, parse_stamp(&watermark)))
		.collect();
		(load_windows(connection)?, tokens, watermarks)
	};
	let wanted = {
		let (start, _) = day_bounds(first, day_end_hour);
		let (_, end) = day_bounds(last, day_end_hour);
		Window {
			start: parse_stamp(&start).ok_or("Invalid calendar range")?,
			end: parse_stamp(&end).ok_or("Invalid calendar range")?,
		}
	};
	let added = gaps(wanted, &covered);
	let windows = merge(prune(
		covered.iter().copied().chain([wanted]).collect(),
		wanted,
		now,
	));

	let (synced, mut task_result) = fetch_all(
		access_token,
		&calendars,
		&tokens,
		&watermarks,
		&windows,
		&added,
		app_calendar,
	)?;

	{
		let transaction = database
			.connection()
			.transaction_with_behavior(TransactionBehavior::Immediate)
			.map_err(|e| e.to_string())?;
		for (calendar, sync) in synced {
			apply(&transaction, calendar, sync, &windows, app_calendar)?;
		}
		// Calendars that are gone, or no longer shared, take their events
		// and tokens with them.
		let live: BTreeSet<&str> = calendars.iter().map(|c| c.id.as_str()).collect();
		let stored: Vec<String> = string_map(
            &transaction,
            "SELECT calendar_id,'' FROM google_sync_calendars UNION SELECT DISTINCT calendar_id,'' FROM google_sync_events",
        )?
        .into_keys()
        .collect();
		for gone in stored.iter().filter(|id| !live.contains(id.as_str())) {
			transaction
				.execute(
					"DELETE FROM google_sync_events WHERE calendar_id=?1",
					[gone],
				)
				.map_err(|e| e.to_string())?;
			transaction
				.execute(
					"DELETE FROM google_sync_calendars WHERE calendar_id=?1",
					[gone],
				)
				.map_err(|e| e.to_string())?;
		}
		save_windows(&transaction, &windows)?;
		if let Ok((lists, synced)) = &mut task_result {
			apply_lists(&transaction, lists, std::mem::take(synced))?;
		}
		transaction.commit().map_err(|e| e.to_string())?;
	}

	let mut events = Vec::new();
	for calendar in &calendars {
		events.extend(stored_events(
			database.connection(),
			calendar,
			wanted,
			first,
			last,
		)?);
	}
	Ok(Synced {
		calendars,
		events,
		app_calendar: resolved.clone(),
		task_lists: task_result.map(|(lists, _)| lists),
	})
}

#[cfg(test)]
mod tests;
