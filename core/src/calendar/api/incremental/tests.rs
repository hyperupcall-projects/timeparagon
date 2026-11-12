//! Incremental sync against a stand-in for Google that keeps a change log,
//! so sync tokens, expiry and `updatedMin` behave as Google's do.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::{Value, json};

use super::*;
use crate::calendar::api::with_transport;

const PRIMARY: &str = "me@example.com";
const APP: &str = "app-calendar";

#[derive(Default)]
struct Fake {
	primary: String,
	/// Every event as Google holds it now, by calendar and ID.
	events: BTreeMap<(String, String), Value>,
	/// What `instances` answers for each series.
	instances: BTreeMap<String, Vec<Value>>,
	/// Changes in order: (sequence, calendar, event).
	log: Vec<(u64, String, Value)>,
	sequence: u64,
	/// Google has expired every sync token.
	expired: bool,
	/// Tasks by list and ID.
	tasks: BTreeMap<(String, String), Value>,
	lists: Vec<String>,
	requests: Vec<String>,
}

impl Fake {
	fn new() -> Rc<RefCell<Self>> {
		Rc::new(RefCell::new(Self {
			primary: PRIMARY.into(),
			..Self::default()
		}))
	}

	/// Saves `event` on `calendar`, as an edit made anywhere would.
	fn put(&mut self, calendar: &str, event: Value) {
		self.sequence += 1;
		let id = event["id"].as_str().unwrap().to_owned();
		self.log
			.push((self.sequence, calendar.to_owned(), event.clone()));
		self.events.insert((calendar.to_owned(), id), event);
	}

	fn cancel(&mut self, calendar: &str, id: &str) {
		self.put(calendar, json!({"id": id, "status": "cancelled"}));
	}

	fn put_task(&mut self, list: &str, task: Value) {
		let id = task["id"].as_str().unwrap().to_owned();
		self.tasks.insert((list.to_owned(), id), task);
	}

	fn answer(&mut self, url: &str) -> Result<String, (u16, String)> {
		self.requests.push(url.to_owned());
		let (path, query) = url.split_once('?').unwrap_or((url, ""));
		let params: BTreeMap<String, String> = query
			.split('&')
			.filter_map(|pair| pair.split_once('='))
			.map(|(key, value)| {
				(
					key.to_owned(),
					urlencoding::decode(value).unwrap().into_owned(),
				)
			})
			.collect();
		let time = |key: &str| {
			params
				.get(key)
				.and_then(|value| DateTime::parse_from_rfc3339(value).ok())
				.map(|at| at.with_timezone(&Utc))
		};
		let in_window = |event: &Value| {
			let start = DateTime::parse_from_rfc3339(event["start"]["dateTime"].as_str().unwrap())
				.unwrap()
				.with_timezone(&Utc);
			let end = DateTime::parse_from_rfc3339(event["end"]["dateTime"].as_str().unwrap())
				.unwrap()
				.with_timezone(&Utc);
			time("timeMin").is_none_or(|min| end > min)
				&& time("timeMax").is_none_or(|max| start < max)
		};

		if path.ends_with("/users/me/calendarList") {
			return Ok(json!({"items": [
				{"id": self.primary, "summary": "Me", "primary": true},
				{"id": APP, "summary": CalendarSettings::APP_CALENDAR_NAME},
			]})
			.to_string());
		}
		if path.ends_with("/users/@me/lists") {
			let items: Vec<Value> = self
				.lists
				.iter()
				.map(|id| json!({"id": id, "title": id}))
				.collect();
			return Ok(json!({ "items": items }).to_string());
		}
		if let Some(rest) = path.strip_prefix(&format!("{TASKS}/")) {
			let list = urlencoding::decode(rest.trim_end_matches("/tasks"))
				.unwrap()
				.into_owned();
			let since = time("updatedMin");
			let items: Vec<Value> = self
				.tasks
				.iter()
				.filter(|((on, _), _)| *on == list)
				.map(|(_, task)| task.clone())
				.filter(|task| {
					since.is_none_or(|since| {
						DateTime::parse_from_rfc3339(task["updated"].as_str().unwrap()).unwrap()
							>= since
					})
				})
				.collect();
			return Ok(json!({ "items": items }).to_string());
		}
		let rest = path.strip_prefix(&format!("{EVENTS}/")).unwrap();
		let (calendar, rest) = rest.split_once('/').unwrap();
		let calendar = urlencoding::decode(calendar).unwrap().into_owned();
		if let Some(series) = rest
			.strip_prefix("events/")
			.and_then(|rest| rest.strip_suffix("/instances"))
		{
			let items: Vec<Value> = self
				.instances
				.get(series)
				.into_iter()
				.flatten()
				.filter(|event| in_window(event))
				.cloned()
				.collect();
			return Ok(json!({ "items": items }).to_string());
		}
		assert_eq!(rest, "events");
		if let Some(token) = params.get("syncToken") {
			if self.expired {
				return Err((410, "Sync token is no longer valid".into()));
			}
			let since: u64 = token.parse().unwrap();
			let items: Vec<Value> = self
				.log
				.iter()
				.filter(|(at, on, _)| *at > since && *on == calendar)
				.map(|(_, _, event)| event.clone())
				.collect();
			return Ok(
				json!({"items": items, "nextSyncToken": self.sequence.to_string()}).to_string(),
			);
		}
		if params.get("singleEvents").map(String::as_str) == Some("true") {
			let mut items: Vec<Value> = self
				.events
				.iter()
				.filter(|((on, _), event)| {
					*on == calendar
						&& event["status"] != "cancelled"
						&& event.get("recurrence").is_none()
				})
				.map(|(_, event)| event.clone())
				.filter(|event| in_window(event))
				.collect();
			for ((on, id), event) in &self.events {
				if *on == calendar && event.get("recurrence").is_some() {
					items.extend(
						self.instances
							.get(id)
							.into_iter()
							.flatten()
							.filter(|event| in_window(event))
							.cloned(),
					);
				}
			}
			return Ok(json!({ "items": items }).to_string());
		}
		// A listing for a new sync token.
		Ok(json!({"items": [], "nextSyncToken": self.sequence.to_string()}).to_string())
	}
}

fn day() -> NaiveDate {
	NaiveDate::from_ymd_opt(2026, 11, 11).unwrap()
}

fn at(day: NaiveDate, hour: u32) -> DateTime<Utc> {
	crate::calendar::at_clock(day, hour, 0).to_utc()
}

fn event(id: &str, day: NaiveDate, hour: u32) -> Value {
	json!({
		"id": id,
		"summary": id,
		"start": {"dateTime": at(day, hour).to_rfc3339()},
		"end": {"dateTime": at(day, hour + 1).to_rfc3339()},
	})
}

fn occurrence(series: &str, id: &str, day: NaiveDate, hour: u32) -> Value {
	let mut value = event(id, day, hour);
	value["recurringEventId"] = json!(series);
	value
}

struct Harness {
	_directory: tempfile::TempDir,
	path: std::path::PathBuf,
	google: Rc<RefCell<Fake>>,
}

impl Harness {
	fn new() -> Self {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("timeparagon.sqlite3");
		Self {
			_directory: directory,
			path,
			google: Fake::new(),
		}
	}

	fn sync(&self, first: NaiveDate, last: NaiveDate) -> Synced {
		let google = self.google.clone();
		self.google.borrow_mut().requests.clear();
		with_transport(
			Box::new(move |_, url, _| google.borrow_mut().answer(url)),
			|| sync(&self.path, "token", first, last, 0, Some(APP), at(day(), 0)).unwrap(),
		)
	}

	fn requested(&self, part: &str) -> usize {
		self.google
			.borrow()
			.requests
			.iter()
			.filter(|url| url.contains(part))
			.count()
	}

	/// Listings of a whole calendar for a new sync token: a full sync.
	fn full_listings(&self) -> usize {
		self.google
			.borrow()
			.requests
			.iter()
			.filter(|url| {
				url.contains("/events?maxResults=2500")
					&& !url.contains("syncToken=")
					&& !url.contains("singleEvents=")
			})
			.count()
	}

	fn tasks(&self, list: &str) -> Vec<String> {
		let mut database = open(&self.path).unwrap();
		stored_tasks(database.connection(), list)
			.unwrap()
			.into_iter()
			.map(|task| task.id)
			.collect()
	}
}

fn ids(synced: &Synced) -> Vec<&str> {
	let mut ids: Vec<&str> = synced.events.iter().map(|e| e.id.as_str()).collect();
	ids.sort();
	ids
}

#[test]
fn the_first_sync_is_full_and_later_ones_only_ask_for_changes() {
	let harness = Harness::new();
	harness
		.google
		.borrow_mut()
		.put(PRIMARY, event("a", day(), 10));

	let first = harness.sync(day(), day());
	assert_eq!(ids(&first), ["a"]);
	assert_eq!(harness.full_listings(), 2, "a token per calendar");
	assert_eq!(
		harness.requested("singleEvents=true"),
		2,
		"the day, per calendar"
	);

	{
		let mut google = harness.google.borrow_mut();
		google.put(PRIMARY, event("b", day(), 12));
		google.cancel(PRIMARY, "a");
	}
	let second = harness.sync(day(), day());
	assert_eq!(ids(&second), ["b"]);
	assert_eq!(harness.requested("syncToken="), 2);
	assert_eq!(
		harness.requested("singleEvents=true"),
		0,
		"nothing listed again"
	);
	assert_eq!(harness.full_listings(), 0, "no full sync");

	// Nothing changed: the copy answers on its own.
	let third = harness.sync(day(), day());
	assert_eq!(ids(&third), ["b"]);
	assert_eq!(harness.requested("singleEvents=true"), 0);
}

#[test]
fn an_event_moved_to_another_day_leaves_the_day_it_was_on() {
	let harness = Harness::new();
	harness
		.google
		.borrow_mut()
		.put(PRIMARY, event("a", day(), 10));
	harness.sync(day(), day());

	let later = day() + Duration::days(20);
	harness
		.google
		.borrow_mut()
		.put(PRIMARY, event("a", later, 10));
	assert!(harness.sync(day(), day()).events.is_empty());
}

#[test]
fn an_expired_token_brings_a_full_sync_of_the_windows() {
	let harness = Harness::new();
	harness
		.google
		.borrow_mut()
		.put(PRIMARY, event("a", day(), 10));
	harness.sync(day(), day());

	{
		let mut google = harness.google.borrow_mut();
		google.expired = true;
		// A deletion the expired token can no longer report.
		google.events.clear();
		google.put(PRIMARY, event("b", day(), 9));
	}
	let synced = harness.sync(day(), day());
	harness.google.borrow_mut().expired = false;
	assert_eq!(ids(&synced), ["b"]);
	assert_eq!(harness.full_listings(), 2);

	let again = harness.sync(day(), day());
	assert_eq!(ids(&again), ["b"]);
	assert_eq!(harness.full_listings(), 0);
}

#[test]
fn a_changed_series_has_its_occurrences_fetched_again_and_a_deleted_one_goes() {
	let harness = Harness::new();
	{
		let mut google = harness.google.borrow_mut();
		let mut master = event("s", day(), 8);
		master["recurrence"] = json!(["RRULE:FREQ=DAILY"]);
		google.put(APP, master);
		google
			.instances
			.insert("s".into(), vec![occurrence("s", "s_1", day(), 8)]);
	}
	let first = harness.sync(day(), day());
	assert_eq!(ids(&first), ["s_1"]);
	assert_eq!(first.events[0].start.to_utc(), at(day(), 8));

	{
		let mut google = harness.google.borrow_mut();
		let mut master = event("s", day(), 14);
		master["recurrence"] = json!(["RRULE:FREQ=DAILY"]);
		google.put(APP, master);
		google
			.instances
			.insert("s".into(), vec![occurrence("s", "s_1", day(), 14)]);
	}
	let moved = harness.sync(day(), day());
	assert_eq!(moved.events[0].start.to_utc(), at(day(), 14));
	assert_eq!(harness.requested("/events/s/instances"), 1);

	// One occurrence deleted, then the whole series.
	harness.google.borrow_mut().put(
		APP,
		json!({"id": "s_1", "status": "cancelled", "recurringEventId": "s"}),
	);
	assert!(harness.sync(day(), day()).events.is_empty());
	harness.google.borrow_mut().cancel(APP, "s");
	assert!(harness.sync(day(), day()).events.is_empty());
}

#[test]
fn days_outside_the_windows_are_fetched_on_their_own() {
	let harness = Harness::new();
	let tomorrow = day() + Duration::days(1);
	{
		let mut google = harness.google.borrow_mut();
		google.put(PRIMARY, event("today", day(), 10));
		google.put(PRIMARY, event("tomorrow", tomorrow, 10));
	}
	assert_eq!(ids(&harness.sync(day(), day())), ["today"]);

	let both = harness.sync(day(), tomorrow);
	assert_eq!(ids(&both), ["today", "tomorrow"]);
	let gap = format!(
		"timeMin={}",
		urlencoding::encode(&at(tomorrow, 0).to_rfc3339())
	);
	assert_eq!(harness.requested(&gap), 2, "only tomorrow, per calendar");
	assert_eq!(harness.full_listings(), 0);

	assert_eq!(ids(&harness.sync(tomorrow, tomorrow)), ["tomorrow"]);
	assert_eq!(harness.requested("singleEvents=true"), 0);
}

#[test]
fn another_account_starts_again_with_a_full_sync() {
	let harness = Harness::new();
	harness
		.google
		.borrow_mut()
		.put(PRIMARY, event("a", day(), 10));
	harness.sync(day(), day());

	{
		let mut google = harness.google.borrow_mut();
		google.primary = "someone@example.com".into();
		google.events.clear();
	}
	assert!(harness.sync(day(), day()).events.is_empty());
	assert_eq!(harness.full_listings(), 2);
}

#[test]
fn tasks_are_fetched_whole_once_then_since_the_latest_change() {
	let harness = Harness::new();
	let at = |minute: i64| (at(day(), 9) + Duration::minutes(minute)).to_rfc3339();
	{
		let mut google = harness.google.borrow_mut();
		google.lists = vec!["one".into(), "two".into()];
		google.put_task("one", json!({"id": "t1", "title": "T1", "updated": at(0)}));
	}
	let first = harness.sync(day(), day());
	assert_eq!(first.task_lists.unwrap().len(), 2);
	assert_eq!(harness.tasks("one"), ["t1"]);
	assert_eq!(harness.requested("updatedMin="), 0);

	{
		let mut google = harness.google.borrow_mut();
		google.put_task("one", json!({"id": "t2", "title": "T2", "updated": at(10)}));
		google.put_task(
			"one",
			json!({"id": "t1", "deleted": true, "updated": at(11)}),
		);
	}
	harness.sync(day(), day());
	assert_eq!(harness.tasks("one"), ["t2"]);
	let since = format!(
		"updatedMin={}",
		urlencoding::encode(
			&(DateTime::parse_from_rfc3339(&at(0)).unwrap().to_utc() - TASKS_OVERLAP).to_rfc3339()
		)
	);
	assert_eq!(
		harness.requested(&since),
		1,
		"list one since its last change"
	);

	// Moved to another list: the list it left says nothing.
	{
		let mut google = harness.google.borrow_mut();
		google.tasks.remove(&("one".to_owned(), "t2".to_owned()));
		google.put_task("two", json!({"id": "t2", "title": "T2", "updated": at(20)}));
	}
	harness.sync(day(), day());
	assert!(harness.tasks("one").is_empty());
	assert_eq!(harness.tasks("two"), ["t2"]);

	// A list that is gone takes its tasks with it.
	harness.google.borrow_mut().lists = vec!["one".into()];
	harness.sync(day(), day());
	assert!(harness.tasks("two").is_empty());
}

#[test]
fn windows_merge_and_leave_gaps_only_where_nothing_is_held() {
	let hour = |h: i64| at(day(), 0) + Duration::hours(h);
	let window = |start, end| Window {
		start: hour(start),
		end: hour(end),
	};
	let covered = merge(vec![window(10, 12), window(0, 4), window(4, 6)]);
	assert_eq!(covered, [window(0, 6), window(10, 12)]);
	assert_eq!(
		gaps(window(2, 14), &covered),
		[window(6, 10), window(12, 14)]
	);
	assert!(gaps(window(1, 5), &covered).is_empty());

	let far = window(24 * 400, 24 * 401);
	let kept = prune(vec![window(0, 6), far], window(0, 1), hour(0));
	assert_eq!(kept, [window(0, 6)]);
}
