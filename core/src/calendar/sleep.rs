//! Sleep is an ordinary Google event with private metadata and a separate UI source.
use super::google::GoogleError;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, Timelike, Utc};
use iced_core::Color;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Event, Snapshot, api, scheduled, time_format};
use crate::config::CalendarSettings;

pub(super) mod store;

pub const SOURCE_ID: &str = "timeslay:sleep";
pub const NEW_ID: &str = "new-sleep";
pub const COLOR: Color = Color::from_rgb(0.48, 0.50, 0.53);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
	/// Stable identity of a night, even after its times are edited.
	pub wake_date: NaiveDate,
	pub automatic: bool,
	pub estimated_start: bool,
	pub estimated_end: bool,
	/// When sleep actually began, as started from the Start sleep button or
	/// entered by hand. The event's own times stay the plan.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub actual_start: Option<DateTime<Local>>,
	/// When sleep actually ended. Unset while the night is still running.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub actual_end: Option<DateTime<Local>>,
}
impl Record {
	pub fn planned(wake_date: NaiveDate) -> Self {
		Self {
			wake_date,
			automatic: true,
			estimated_start: true,
			estimated_end: true,
			actual_start: None,
			actual_end: None,
		}
	}
	/// Started with the timer and not finished yet.
	pub fn sleeping(&self) -> bool {
		self.actual_start.is_some() && self.actual_end.is_none()
	}
	/// The recorded length of actual sleep, once both ends are known.
	pub fn actual_minutes(&self) -> Option<i64> {
		Some((self.actual_end? - self.actual_start?).num_minutes())
	}
	pub fn from_properties(properties: &Value) -> Option<Self> {
		if properties["private"]["ttKind"].as_str() != Some("sleep") {
			return None;
		}
		serde_json::from_str(properties["private"]["ttSleep"].as_str()?).ok()
	}
}

pub fn duration(event: &Event) -> String {
	minutes_label((event.end - event.start).num_minutes())
}
/// "8h" or "7h 45m".
pub fn minutes_label(minutes: i64) -> String {
	let minutes = minutes.max(0);
	if minutes % 60 == 0 {
		format!("{}h", minutes / 60)
	} else {
		format!("{}h {}m", minutes / 60, minutes % 60)
	}
}
pub fn title(_event: &Event) -> String {
	"Sleep".into()
}
/// The times written on a night: once it is finished, when sleep actually
/// happened and how long it was against the plan ("11p – 8a\n8h 35m / 9h");
/// until then, the planned times and length ("11p – 8a\n9h").
pub fn time_label(event: &Event, format: time_format::TimeFormat) -> String {
	let actual = event
		.details
		.sleep
		.as_ref()
		.and_then(|record| Some((record.actual_start?, record.actual_end?)));
	match actual {
		Some((start, end)) => format!(
			"{}\n{} / {}",
			time_format::range(start, end, format),
			minutes_label((end - start).num_minutes()),
			duration(event)
		),
		None => format!(
			"{}\n{}",
			time_format::range(event.start, event.end, format),
			duration(event)
		),
	}
}

/// What the sleep timer button on a night does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Timer {
	Start,
	Finish,
}
impl Timer {
	pub fn label(self) -> &'static str {
		match self {
			Self::Start => "Start sleep",
			Self::Finish => "Finish sleep",
		}
	}
}

/// How far ahead of a planned bedtime the Start sleep button is offered.
const START_WINDOW_HOURS: i64 = 12;

/// Which timer button a night offers. A running night can always be
/// finished. Start is offered only on the night about to begin or underway,
/// so the four weeks of planned nights do not all carry a button.
pub fn timer(event: &Event, now: DateTime<Local>) -> Option<Timer> {
	let record = event.details.sleep.as_ref()?;
	if record.sleeping() {
		return Some(Timer::Finish);
	}
	(record.actual_start.is_none()
		&& now < event.end
		&& now >= event.start - Duration::hours(START_WINDOW_HOURS))
	.then_some(Timer::Start)
}

/// The moment the wake-up predictions count from: when sleep actually began,
/// else the planned bedtime.
pub fn prediction_start(event: &Event) -> DateTime<Local> {
	event
		.details
		.sleep
		.as_ref()
		.and_then(|record| record.actual_start)
		.unwrap_or(event.start)
}

/// The fixed amounts of sleep wake-up times are predicted for.
pub const PREDICTED_HOURS: [i64; 4] = [6, 7, 8, 9];

/// Wake-up times for 6, 7, 8 and 9 hours of sleep, then for the planned
/// (target) amount, each as (amount label, wake-up time).
pub fn wake_predictions(event: &Event) -> Vec<(String, DateTime<Local>)> {
	let start = prediction_start(event);
	let target = event.end - event.start;
	PREDICTED_HOURS
		.iter()
		.map(|&hours| (format!("{hours}h"), start + Duration::hours(hours)))
		.chain(std::iter::once((
			format!("Target {}", minutes_label(target.num_minutes())),
			start + target,
		)))
		.collect()
}

/// The predictions on one line, e.g. "6h 5:12a · 7h 6:12a · … · Target 8h 7:12a".
pub fn predictions_line(event: &Event, format: time_format::TimeFormat) -> String {
	wake_predictions(event)
		.into_iter()
		.map(|(label, at)| format!("{label} {}", time_format::clock(at, format)))
		.collect::<Vec<_>>()
		.join(" · ")
}

/// What the sleep notification says once the timer has started or finished,
/// as (summary, body): while sleeping, since when and the wake-up times; once
/// finished, how long was slept against the plan.
pub fn notification(
	event: &Event,
	timer: Timer,
	format: time_format::TimeFormat,
) -> (String, String) {
	let record = event.details.sleep.as_ref();
	match timer {
		Timer::Start => {
			let since = record
				.and_then(|r| r.actual_start)
				.map(|at| time_format::clock(at, format))
				.unwrap_or_default();
			(
				format!("Sleeping since {since}"),
				format!("Wake up for {}", predictions_line(event, format)),
			)
		}
		Timer::Finish => {
			let slept = record
				.and_then(Record::actual_minutes)
				.map(minutes_label)
				.unwrap_or_default();
			(
				format!("Slept {slept}"),
				format!("Planned {}", duration(event)),
			)
		}
	}
}

/// The first night on or after `today` that wakes on `weekday`: where a
/// change to that weekday's schedule in Settings takes effect.
pub fn next_night(today: NaiveDate, weekday: chrono::Weekday) -> NaiveDate {
	today + Duration::days(i64::from(weekday.days_since(today.weekday())))
}

/// Where "this and future" nights start when one night's times are edited.
/// A past night moves the schedule from its next occurrence, since history
/// is never re-planned.
pub fn change_from(wake_date: NaiveDate, today: NaiveDate) -> NaiveDate {
	if wake_date < today {
		next_night(today, wake_date.weekday())
	} else {
		wake_date
	}
}

/// Reads a typed clock time as the moment nearest `anchor`, so an actual
/// bedtime of 23:40 lands the evening before a 00:30 plan, and 01:10 the
/// morning after an 23:30 plan.
pub fn nearest_clock(text: &str, anchor: DateTime<Local>) -> Option<DateTime<Local>> {
	let (hour, minute) = super::parse_clock(text)?;
	let day = anchor.date_naive();
	[day.pred_opt(), Some(day), day.succ_opt()]
		.into_iter()
		.flatten()
		.map(|day| super::at_clock(day, hour, minute))
		.min_by_key(|at| (*at - anchor).num_seconds().abs())
}

/// Clock times for the night ending on this weekday.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Times {
	pub bedtime: NaiveTime,
	pub wakeup: NaiveTime,
}
impl Default for Times {
	fn default() -> Self {
		Self {
			bedtime: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
			wakeup: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
		}
	}
}

/// A weekday schedule change effective from a particular night.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleChange {
	pub from: NaiveDate,
	pub previous: Times,
	pub times: Times,
}

pub fn times_for(settings: &CalendarSettings, day: NaiveDate) -> Times {
	let changes = || {
		settings
			.sleep_schedule_changes
			.iter()
			.filter(|change| change.from.weekday() == day.weekday())
	};
	changes()
		.filter(|change| change.from <= day)
		.max_by_key(|change| change.from)
		.map(|change| change.times)
		.or_else(|| {
			changes()
				.min_by_key(|change| change.from)
				.map(|change| change.previous)
		})
		.unwrap_or(settings.sleep_schedule[day.weekday().num_days_from_monday() as usize])
}

/// Share changes made from the grid with Settings while retaining the schedule
/// before the selected occurrence. A later edit replaces following changes.
pub fn set_weekday_times(settings: &mut CalendarSettings, from: NaiveDate, times: Times) {
	let previous = times_for(settings, from);
	settings
		.sleep_schedule_changes
		.retain(|change| change.from.weekday() != from.weekday() || change.from < from);
	settings.sleep_schedule_changes.push(ScheduleChange {
		from,
		previous,
		times,
	});
	settings.sleep_schedule[from.weekday().num_days_from_monday() as usize] = times;
}

pub fn weekday_label(day: NaiveDate) -> String {
	day.format("%A").to_string().to_lowercase()
}

/// Use local clock times, including across daylight-saving changes.
pub fn plan(wake_date: NaiveDate, schedule: &[Times; 7]) -> (DateTime<Local>, DateTime<Local>) {
	let times = schedule[wake_date.weekday().num_days_from_monday() as usize];
	let bedtime_date = if times.bedtime >= times.wakeup {
		wake_date.pred_opt().unwrap_or(wake_date)
	} else {
		wake_date
	};
	(
		super::at_clock(bedtime_date, times.bedtime.hour(), times.bedtime.minute()),
		super::at_clock(wake_date, times.wakeup.hour(), times.wakeup.minute()),
	)
}

/// Today and the next four weeks, independent of the date being viewed.
fn planning_dates(today: NaiveDate) -> impl Iterator<Item = NaiveDate> {
	(0..=28).filter_map(move |offset| today.checked_add_signed(Duration::days(offset)))
}

/// Description kept on the Google event, regenerated whenever the times change.
pub fn description(event: &Event) -> String {
	time_label(event, time_format::TimeFormat::TwelveHour)
}

/// Starts or finishes the night's sleep timer at `now`.
pub fn apply_timer(event: &mut Event, timer: Timer, now: DateTime<Local>) -> Result<(), String> {
	let record = event
		.details
		.sleep
		.as_mut()
		.ok_or("Missing sleep metadata")?;
	let now = now
		.with_second(0)
		.and_then(|t| t.with_nanosecond(0))
		.unwrap_or(now);
	match timer {
		Timer::Start => {
			record.actual_start = Some(now);
			record.actual_end = None;
		}
		Timer::Finish => {
			let start = record.actual_start.ok_or("Sleep was never started")?;
			record.actual_end = Some(now.max(start));
		}
	}
	event.details.description = description(event);
	Ok(())
}

fn collection(calendar_id: &str) -> String {
	format!(
		"{}/{}/events",
		api::EVENTS,
		urlencoding::encode(calendar_id)
	)
}
fn event_url(calendar_id: &str, id: &str) -> String {
	format!("{}/{}", collection(calendar_id), urlencoding::encode(id))
}
fn event_id(day: NaiveDate) -> String {
	// Google IDs accept base32hex characters; this is stable across devices.
	format!("51ee9{}", day.format("%Y%m%d"))
}
fn body(event: &Event) -> Result<Value, String> {
	let record = event
		.details
		.sleep
		.as_ref()
		.ok_or("Missing sleep metadata")?;
	Ok(json!({
		"summary": title(event), "description": description(event), "colorId": null,
		"start": {"dateTime": event.start.with_timezone(&Utc).to_rfc3339()},
		"end": {"dateTime": event.end.with_timezone(&Utc).to_rfc3339()},
		"extendedProperties": {"private": {
			"ttKind": "sleep", "ttSleepDate": record.wake_date.to_string(),
			"ttSleep": serde_json::to_string(record).map_err(|e| e.to_string())?
		}},
		"reminders": {"useDefault": false}
	}))
}
fn get(token: &str, calendar_id: &str, day: NaiveDate) -> Result<Option<Event>, String> {
	// No time limits: a recorded night may have been moved outside the viewed range.
	let url = format!(
		"{}?privateExtendedProperty=ttSleepDate%3D{}&showDeleted=false",
		collection(calendar_id),
		day
	);
	let value = scheduled::request("GET", &url, token, None).map_err(|e| e.message)?;
	value["items"]
		.as_array()
		.and_then(|items| items.first())
		.cloned()
		.map(|value| api::parse_sleep(value, calendar_id))
		.transpose()
}
/// The span a planning pass needs nights for: from two days before `today`
/// (a night waking today starts the evening before) to two days past the
/// rolling four-week horizon.
fn nights_window(today: NaiveDate) -> (DateTime<Utc>, DateTime<Utc>) {
	let at = |offset| super::at_clock(today + Duration::days(offset), 0, 0).with_timezone(&Utc);
	(at(-2), at(30))
}

/// Every night on the app calendar overlapping `from..to`, in one paged list
/// request instead of one lookup per day.
fn fetch_nights(
	token: &str,
	calendar_id: &str,
	from: DateTime<Utc>,
	to: DateTime<Utc>,
) -> Result<Vec<Event>, String> {
	let mut nights = Vec::new();
	let mut page: Option<String> = None;
	loop {
		let url = format!(
			"{}?privateExtendedProperty=ttKind%3Dsleep&showDeleted=false&maxResults=250&timeMin={}&timeMax={}{}",
			collection(calendar_id),
			urlencoding::encode(&from.to_rfc3339()),
			urlencoding::encode(&to.to_rfc3339()),
			page.as_ref()
				.map(|p| format!("&pageToken={}", urlencoding::encode(p)))
				.unwrap_or_default()
		);
		let value = scheduled::request("GET", &url, token, None).map_err(|e| e.message)?;
		for item in value["items"].as_array().into_iter().flatten() {
			// Skip anything unreadable rather than failing the whole night list.
			if let Ok(event) = api::parse_sleep(item.clone(), calendar_id)
				&& event.details.sleep.is_some()
			{
				nights.push(event);
			}
		}
		page = value["nextPageToken"].as_str().map(str::to_owned);
		if page.is_none() {
			return Ok(nights);
		}
	}
}

/// The nights stored for `first..=last` (as Google last had them, from this
/// device or another one through database sync), with this device's unsent
/// edits on top. For showing nights again when
/// [`Counters::sleep`](crate::local_changes::Counters) moves.
pub fn stored_nights(
	path: &std::path::Path,
	first: NaiveDate,
	last: NaiveDate,
) -> Result<Vec<Event>, String> {
	let around = |day: NaiveDate| super::at_clock(day, 0, 0).to_utc();
	let mut nights = {
		let mut database = crate::task_db::Database::open(path)?;
		store::overlapping(
			database.connection(),
			around(first - Duration::days(1)),
			around(last + Duration::days(2)),
		)
	};
	super::operations::overlay(path, &mut nights, None)?;
	// The overlay can add pending events of any kind; only nights are wanted.
	nights.retain(|night| night.details.sleep.is_some());
	Ok(nights)
}

/// Puts stored nights into `day`'s events, each replacing any older copy of
/// the same night, keeping only what falls on the day.
pub fn merge_stored(events: &mut Vec<Event>, nights: &[Event], day: NaiveDate, day_end_hour: u32) {
	store::merge(nights.iter().cloned(), events);
	events.retain(|event| super::event_on_day(event, day, day_end_hour));
}

/// Records a night as Google now has it in the local store.
fn remember(event: &Event) {
	if let Some(path) = crate::task_db::path()
		&& let Err(error) = store::put(&path, event)
	{
		log::warn!("could not store sleep: {error}");
	}
}

/// Drops a deleted event from the stored nights. Anything that is not a
/// night matches nothing, so every deletion can call this.
pub(crate) fn forget(calendar_id: &str, id: &str) {
	if let Some(path) = crate::task_db::path()
		&& let Err(error) = store::remove(&path, calendar_id, id)
	{
		log::warn!("could not forget stored sleep: {error}");
	}
}

fn write(token: &str, event: &Event, new: bool) -> Result<Event, GoogleError> {
	let mut body = body(event)?;
	let url = if new {
		body["id"] = json!(event.id);
		collection(&event.calendar_id)
	} else {
		event_url(&event.calendar_id, &event.id)
	};
	let value =
		match scheduled::request(if new { "POST" } else { "PATCH" }, &url, token, Some(&body)) {
			Ok(value) => value,
			Err(error) if new && error.status == Some(409) => {
				// Another device may have created this night, or its original ID
				// may be a deleted Google tombstone. Never overwrite a recorded night.
				if let Some(existing) = get(
					token,
					&event.calendar_id,
					event.details.sleep.as_ref().unwrap().wake_date,
				)? {
					return if event.details.sleep.as_ref().unwrap().automatic {
						remember(&existing);
						Ok(existing)
					} else {
						let mut event = event.clone();
						event.id = existing.id;
						write(token, &event, false)
					};
				}
				body.as_object_mut().unwrap().remove("id");
				scheduled::request("POST", &url, token, Some(&body))?
			}
			Err(error) => return Err(error.into()),
		};
	let saved = api::parse_sleep(value, &event.calendar_id)?;
	remember(&saved);
	Ok(saved)
}

/// Reconcile the weekly schedule for today and four weeks ahead, then return
/// the requested view. Browsing past or distant dates never extends planning.
pub fn sync(
	settings: &CalendarSettings,
	anchor: NaiveDate,
	dates: &[NaiveDate],
) -> Result<Snapshot, String> {
	let first = *dates.first().ok_or("Calendar range is empty")?;
	let last = dates
		.last()
		.unwrap()
		.succ_opt()
		.ok_or("Invalid calendar date")?;
	let token = super::GoogleClient::from_settings(settings).access_token()?;
	let now = crate::clock::now();
	let window = nights_window(now.date_naive());
	// The nights are listed alongside the calendar fetch whenever the app
	// calendar is already known, so planning adds no round trip of its own.
	let (snapshot, listed) = std::thread::scope(|scope| {
		let listing = settings.app_calendar.as_deref().map(|id| {
			let token = &token;
			(
				id,
				scope.spawn(move || fetch_nights(token, id, window.0, window.1)),
			)
		});
		let snapshot = api::fetch_range(
			&token,
			&settings.disabled,
			anchor,
			first.pred_opt().unwrap_or(first),
			last,
			0,
			settings.app_calendar.as_deref(),
		);
		let listed = listing.map(|(id, worker)| {
			let nights = worker
				.join()
				.unwrap_or_else(|_| Err("Sleep worker failed".into()));
			(id.to_owned(), nights)
		});
		(snapshot, listed)
	});
	let mut snapshot = snapshot?;
	let calendar_id = settings
		.app_calendar
		.as_ref()
		.filter(|id| snapshot.calendars.iter().any(|c| &c.id == *id))
		.cloned()
		.or_else(|| {
			snapshot
				.calendars
				.iter()
				.find(|c| CalendarSettings::is_app_calendar_name(&c.name))
				.map(|c| c.id.clone())
		});
	if let Some(calendar_id) = calendar_id {
		let nights = match listed {
			Some((id, nights)) if id == calendar_id => nights,
			_ => fetch_nights(&token, &calendar_id, window.0, window.1),
		};
		let planned = nights.and_then(|nights| {
			plan_nights(
				settings,
				&token,
				&calendar_id,
				&mut snapshot.events,
				nights,
				window,
				now,
			)
		});
		if let Err(error) = planned {
			let notice = format!("Could not update Sleep: {error}");
			snapshot.notice = Some(match snapshot.notice.take() {
				Some(old) => format!("{old}\n{notice}"),
				None => notice,
			});
		}
	}
	snapshot.events.retain(|event| {
		dates
			.iter()
			.any(|&day| super::event_on_day(event, day, settings.day_start_hour()))
	});
	Ok(snapshot)
}

/// Stores the nights Google listed for `window`, then brings today and the
/// next four weeks in line with the schedule. Planning looks nights up in
/// that list, never with a request per day.
fn plan_nights(
	settings: &CalendarSettings,
	token: &str,
	calendar_id: &str,
	events: &mut Vec<Event>,
	nights: Vec<Event>,
	window: (DateTime<Utc>, DateTime<Utc>),
	now: DateTime<Local>,
) -> Result<(), String> {
	// Google is the source of truth: what it listed replaces the window. The
	// calendar fetch's own copies are not stored, as they carry unsent edits.
	if let Some(path) = crate::task_db::path()
		&& let Err(error) = store::replace(&path, calendar_id, window.0, window.1, &nights)
	{
		log::warn!("could not store sleep: {error}");
	}
	for day in planning_dates(now.date_naive()) {
		let mut schedule = settings.sleep_schedule;
		schedule[day.weekday().num_days_from_monday() as usize] = times_for(settings, day);
		// A permission/network failure stops here rather than repeating per day.
		reconcile_with(
			calendar_id,
			events,
			day,
			&schedule,
			now,
			|day| {
				Ok(nights
					.iter()
					.find(|e| e.details.sleep.as_ref().is_some_and(|s| s.wake_date == day))
					.cloned())
			},
			|event, new| write(token, event, new).map_err(String::from),
		)?;
	}
	Ok(())
}

fn reconcile_with(
	calendar_id: &str,
	events: &mut Vec<Event>,
	day: NaiveDate,
	schedule: &[Times; 7],
	now: DateTime<Local>,
	mut load: impl FnMut(NaiveDate) -> Result<Option<Event>, String>,
	mut save: impl FnMut(&Event, bool) -> Result<Event, String>,
) -> Result<(), String> {
	// Guard before loading or writing: settings never change historical days,
	// and no caller can create sleep beyond the rolling four-week horizon.
	if day < now.date_naive() || day > now.date_naive() + Duration::days(28) {
		return Ok(());
	}
	let (start, end) = plan(day, schedule);
	let mut existing = events
		.iter()
		.find(|e| {
			e.calendar_id == calendar_id
				&& e.details.sleep.as_ref().is_some_and(|s| s.wake_date == day)
		})
		.cloned();
	if existing.is_none() {
		existing = load(day)?;
	}
	if existing
		.as_ref()
		.is_some_and(|e| e.details.sleep.as_ref().is_none_or(|s| !s.automatic))
	{
		return Ok(());
	}
	let mut event = Event {
		id: existing
			.as_ref()
			.map_or_else(|| event_id(day), |e| e.id.clone()),
		calendar_id: calendar_id.into(),
		title: String::new(),
		start,
		end,
		all_day: false,
		color: COLOR,
		completed: false,
		details: super::Details {
			sleep: Some(Record {
				// A schedule change must not throw away sleep already recorded.
				actual_start: existing
					.as_ref()
					.and_then(|e| e.details.sleep.as_ref())
					.and_then(|s| s.actual_start),
				actual_end: existing
					.as_ref()
					.and_then(|e| e.details.sleep.as_ref())
					.and_then(|s| s.actual_end),
				..Record::planned(day)
			}),
			..Default::default()
		},
	};
	event.title = title(&event);
	event.details.description = description(&event);
	// Also repairs title/description of nights written by older versions.
	if existing.as_ref().is_some_and(|e| {
		e.start == start
			&& e.end == end
			&& e.title == event.title
			&& e.details.description == event.details.description
	}) {
		return Ok(());
	}
	let saved = save(&event, existing.is_none())?;
	events.retain(|e| e.id != saved.id || e.calendar_id != calendar_id);
	events.push(saved);
	Ok(())
}

/// Save user-entered times and confidence flags as a recorded night. Recorded
/// nights are preserved during subsequent weekly schedule changes.
/// Saves a night locally and sends it to Google.
pub fn save(settings: &CalendarSettings, mut event: Event, new: bool) -> Result<Event, String> {
	if event.end <= event.start {
		return Err("Wake-up time must be after sleep start".into());
	}
	if let Some(record) = &event.details.sleep {
		if record.actual_end.is_some() && record.actual_start.is_none() {
			return Err("Enter when sleep actually started".into());
		}
		if record.actual_minutes().is_some_and(|minutes| minutes < 0) {
			return Err("Actual wake-up must be after actual sleep start".into());
		}
	}
	event.details.description = description(&event);
	if new {
		event.calendar_id = settings
			.app_calendar
			.clone()
			.ok_or("Sync Google once to discover the TimeSlay calendar")?;
		event.id = event_id(
			event
				.details
				.sleep
				.as_ref()
				.ok_or("Missing sleep metadata")?
				.wake_date,
		);
	}
	super::operations::submit(
		super::operations::Job::Sleep {
			event: event.clone(),
			new,
		},
		Some(&event),
		&event.calendar_id,
		&event.id,
	)?;
	Ok(event)
}

/// Sends a night saved by [`save`] to Google. Only `operations` calls this.
pub(crate) fn send(
	client: &super::GoogleClient,
	mut event: Event,
	new: bool,
) -> Result<Event, GoogleError> {
	if event.end <= event.start {
		return Err("Wake-up time must be after sleep start".into());
	}
	let token = client.access_token()?;
	if new {
		event.calendar_id = scheduled::app_calendar(&token)?;
		event.id = event_id(
			event
				.details
				.sleep
				.as_ref()
				.ok_or("Missing sleep metadata")?
				.wake_date,
		);
		// Opening a manually added night must not create a second copy.
		if let Some(existing) = get(
			&token,
			&event.calendar_id,
			event.details.sleep.as_ref().unwrap().wake_date,
		)? {
			event.id = existing.id;
			return write(&token, &event, false);
		}
	}
	write(&token, &event, new)
}

#[cfg(test)]
mod tests {
	use super::*;
	fn date() -> NaiveDate {
		NaiveDate::from_ymd_opt(2030, 11, 4).unwrap()
	}
	fn at(hour: u32) -> DateTime<Local> {
		super::super::at_clock(date(), hour, 0)
	}
	fn appointment(hour: u32) -> Event {
		Event {
			id: format!("meeting-{hour}"),
			calendar_id: "work".into(),
			title: "Meeting".into(),
			start: at(hour),
			end: at(hour) + Duration::hours(1),
			all_day: false,
			color: COLOR,
			completed: false,
			details: Default::default(),
		}
	}
	#[test]
	fn confidence_times_and_type_round_trip_through_google_and_cache() {
		let mut event = appointment(1);
		event.calendar_id = "app".into();
		event.end = at(9) + Duration::minutes(30);
		let mut record = Record::planned(date());
		record.automatic = false;
		record.estimated_end = false;
		event.details.sleep = Some(record.clone());
		let mut value = body(&event).unwrap();
		assert_eq!(value.get("colorId"), Some(&Value::Null));
		value["id"] = json!("sleep-id");
		let parsed = api::parse_sleep(value, "app").unwrap();
		assert_eq!(parsed.details.sleep, Some(record));
		assert_eq!((parsed.start, parsed.end), (event.start, event.end));
		assert_eq!(title(&parsed), "Sleep");
		let cached = serde_json::to_string(&parsed.details).unwrap();
		let details: super::super::Details = serde_json::from_str(&cached).unwrap();
		assert_eq!(details, parsed.details);
		assert!(super::super::EditDraft::from_event(&parsed).sleep.is_some());
		assert!(!crate::timeblocking::is_block(
			&parsed,
			&CalendarSettings {
				app_calendar: Some("app".into()),
				..Default::default()
			}
		));
	}
	#[test]
	fn weekly_times_ignore_appointments_and_preserve_minutes() {
		let mut schedule = [Times::default(); 7];
		schedule[date().weekday().num_days_from_monday() as usize] = Times {
			bedtime: NaiveTime::from_hms_opt(22, 45, 0).unwrap(),
			wakeup: NaiveTime::from_hms_opt(7, 15, 0).unwrap(),
		};
		let mut events = vec![appointment(5)];
		reconcile_with(
			"app",
			&mut events,
			date(),
			&schedule,
			at(12),
			|_| Ok(None),
			|event, _| Ok(event.clone()),
		)
		.unwrap();
		let night = events.last().unwrap();
		assert_eq!(night.end, at(7) + Duration::minutes(15));
		assert_eq!(
			night.start,
			super::super::at_clock(date().pred_opt().unwrap(), 22, 45)
		);
		assert_eq!(
			plan(date().succ_opt().unwrap(), &schedule).1,
			super::super::at_clock(date().succ_opt().unwrap(), 8, 0)
		);
	}

	#[test]
	fn reconciliation_is_bounded_and_preserves_recorded_nights() {
		let schedule = [Times::default(); 7];
		let now = at(12);
		for day in [date() - Duration::days(1), date() + Duration::days(29)] {
			reconcile_with(
				"app",
				&mut vec![],
				day,
				&schedule,
				now,
				|_| panic!("must not load outside horizon"),
				|_, _| panic!("must not write outside horizon"),
			)
			.unwrap();
		}
		for offset in [0, 28] {
			let mut events = vec![];
			let day = date() + Duration::days(offset);
			reconcile_with(
				"app",
				&mut events,
				day,
				&schedule,
				now,
				|_| Ok(None),
				|event, new| {
					assert!(new);
					Ok(event.clone())
				},
			)
			.unwrap();
			assert_eq!(events.len(), 1);
			events[0].details.sleep.as_mut().unwrap().automatic = false;
			reconcile_with(
				"app",
				&mut events,
				day,
				&schedule,
				now,
				|_| panic!(),
				|_, _| panic!("recorded night must survive"),
			)
			.unwrap();
		}
		let dates: Vec<_> = planning_dates(date()).collect();
		assert_eq!(dates.len(), 29);
		assert_eq!(dates[0], date());
		assert_eq!(dates[28], date() + Duration::days(28));
	}

	#[test]
	fn schedule_changes_update_existing_nights_without_duplicates() {
		let mut events = vec![];
		let mut schedule = [Times::default(); 7];
		for hour in [8, 9] {
			schedule[date().weekday().num_days_from_monday() as usize].wakeup =
				NaiveTime::from_hms_opt(hour, 30, 0).unwrap();
			reconcile_with(
				"app",
				&mut events,
				date(),
				&schedule,
				at(12),
				|_| Ok(None),
				|event, new| {
					assert_eq!(new, hour == 8);
					Ok(event.clone())
				},
			)
			.unwrap();
			assert_eq!(events.len(), 1);
			assert_eq!(events[0].end, at(hour) + Duration::minutes(30));
		}
		reconcile_with(
			"app",
			&mut events,
			date(),
			&schedule,
			at(12),
			|_| panic!(),
			|_, _| panic!("unchanged schedule must not write"),
		)
		.unwrap();
	}
	#[test]
	fn failed_writes_preserve_the_existing_event() {
		let mut event = appointment(7);
		event.calendar_id = "app".into();
		event.details.sleep = Some(Record::planned(date()));
		let mut events = vec![event];
		let original = events.clone();
		let result = reconcile_with(
			"app",
			&mut events,
			date(),
			&[Times::default(); 7],
			at(12),
			|_| panic!(),
			|_, _| Err("offline".into()),
		);
		assert_eq!(result, Err("offline".into()));
		assert_eq!(events, original);
	}
	#[test]
	fn dated_weekday_changes_preserve_earlier_nights_and_later_edits_replace_following_changes() {
		let mut settings = CalendarSettings::default();
		let first = date();
		let second = first + Duration::days(14);
		let times = |hour| Times {
			bedtime: NaiveTime::from_hms_opt(23, 30, 0).unwrap(),
			wakeup: NaiveTime::from_hms_opt(hour, 15, 0).unwrap(),
		};
		set_weekday_times(&mut settings, first, times(7));
		set_weekday_times(&mut settings, second, times(9));
		assert_eq!(
			times_for(&settings, first - Duration::days(7)),
			Times::default()
		);
		assert_eq!(times_for(&settings, first + Duration::days(7)), times(7));
		assert_eq!(times_for(&settings, second + Duration::days(7)), times(9));
		assert_eq!(settings.sleep_schedule[0], times(9));
		set_weekday_times(&mut settings, first + Duration::days(7), times(8));
		assert_eq!(times_for(&settings, first), times(7));
		assert_eq!(times_for(&settings, second), times(8));
		assert_eq!(
			times_for(&settings, first + Duration::days(1)),
			Times::default()
		);
	}
	fn night() -> Event {
		let mut event = appointment(1);
		event.calendar_id = "app".into();
		event.end = at(9);
		event.details.sleep = Some(Record::planned(date()));
		event
	}

	#[test]
	fn records_without_actual_sleep_still_parse_and_actuals_round_trip() {
		let old = r#"{"wake_date":"2030-11-04","automatic":true,"estimated_start":true,"estimated_end":true}"#;
		let record: Record = serde_json::from_str(old).unwrap();
		assert_eq!(record, Record::planned(date()));
		assert!(!serde_json::to_string(&record).unwrap().contains("actual"));

		let mut event = night();
		apply_timer(&mut event, Timer::Start, at(1) - Duration::minutes(20)).unwrap();
		apply_timer(&mut event, Timer::Finish, at(8) + Duration::minutes(5)).unwrap();
		let mut value = body(&event).unwrap();
		value["id"] = json!("sleep-id");
		let parsed = api::parse_sleep(value, "app").unwrap();
		let record = parsed.details.sleep.as_ref().unwrap();
		assert_eq!(record.actual_minutes(), Some(7 * 60 + 25));
		assert_eq!(parsed.details.description, "12:40 – 8:05AM\n7h 25m / 8h");
		assert_eq!(title(&parsed), "Sleep");
	}

	#[test]
	fn the_timer_offers_start_only_near_the_night_and_finish_while_sleeping() {
		let event = night();
		assert_eq!(timer(&event, at(1) - Duration::hours(13)), None);
		assert_eq!(
			timer(&event, at(1) - Duration::hours(3)),
			Some(Timer::Start)
		);
		assert_eq!(timer(&event, at(5)), Some(Timer::Start));
		assert_eq!(timer(&event, at(9)), None);

		let mut sleeping = event.clone();
		apply_timer(&mut sleeping, Timer::Start, at(2)).unwrap();
		assert!(sleeping.details.sleep.as_ref().unwrap().sleeping());
		// A forgotten timer can still be finished long after the plan ended.
		assert_eq!(timer(&sleeping, at(23)), Some(Timer::Finish));
		apply_timer(&mut sleeping, Timer::Finish, at(10)).unwrap();
		assert_eq!(timer(&sleeping, at(5)), None);
		assert!(apply_timer(&mut night(), Timer::Finish, at(5)).is_err());
	}

	#[test]
	fn wake_predictions_count_from_actual_start_else_the_plan() {
		let mut event = night();
		let times = |event: &Event| {
			wake_predictions(event)
				.into_iter()
				.map(|(label, at)| (label, at.time().to_string()))
				.collect::<Vec<_>>()
		};
		assert_eq!(
			times(&event),
			[
				("6h".into(), "07:00:00".into()),
				("7h".into(), "08:00:00".into()),
				("8h".into(), "09:00:00".into()),
				("9h".into(), "10:00:00".into()),
				("Target 8h".into(), "09:00:00".into()),
			]
		);
		apply_timer(&mut event, Timer::Start, at(2) + Duration::minutes(30)).unwrap();
		assert_eq!(times(&event)[0].1, "08:30:00");
		assert_eq!(times(&event)[4].1, "10:30:00");
	}

	#[test]
	fn notifications_say_since_when_then_how_long() {
		let format = time_format::TimeFormat::TwentyFourHour;
		let mut event = night();
		apply_timer(&mut event, Timer::Start, at(1) - Duration::minutes(20)).unwrap();
		let (summary, body) = notification(&event, Timer::Start, format);
		assert_eq!(summary, "Sleeping since 00:40");
		assert!(body.starts_with("Wake up for 6h 06:40"), "{body}");
		apply_timer(&mut event, Timer::Finish, at(8)).unwrap();
		assert_eq!(
			notification(&event, Timer::Finish, format),
			("Slept 7h 20m".to_owned(), "Planned 8h".to_owned())
		);
	}

	#[test]
	fn schedule_changes_start_on_the_next_matching_night() {
		// 2030-11-04 is a Monday.
		let monday = date();
		assert_eq!(next_night(monday, chrono::Weekday::Mon), monday);
		assert_eq!(
			next_night(monday, chrono::Weekday::Sun),
			monday + Duration::days(6)
		);
		assert_eq!(
			change_from(monday + Duration::days(1), monday),
			monday + Duration::days(1)
		);
		assert_eq!(
			change_from(monday - Duration::days(6), monday),
			monday + Duration::days(1)
		);
	}

	#[test]
	fn typed_actual_times_land_on_the_day_nearest_the_plan() {
		let anchor = at(1);
		assert_eq!(
			nearest_clock("23:40", anchor),
			Some(super::super::at_clock(date().pred_opt().unwrap(), 23, 40))
		);
		assert_eq!(
			nearest_clock("02:10", anchor),
			Some(at(2) + Duration::minutes(10))
		);
		assert_eq!(nearest_clock("nope", anchor), None);
	}

	#[test]
	fn planning_lists_nights_once_and_keeps_the_database_in_step_with_google() {
		use std::{cell::RefCell, rc::Rc};
		let calendar = "planning-store-test";
		let settings = CalendarSettings::default();
		let now = at(12);
		let window = nights_window(date());
		let night_on = |offset| {
			let day = date() + Duration::days(offset);
			let (start, end) = plan(day, &settings.sleep_schedule);
			let mut night = night();
			night.id = event_id(day);
			night.calendar_id = calendar.into();
			(night.start, night.end) = (start, end);
			night.details.sleep = Some(Record::planned(day));
			night.details.description = description(&night);
			let mut value = body(&night).unwrap();
			value["id"] = json!(night.id);
			value
		};
		let stored = || {
			let path = crate::task_db::path().unwrap();
			let mut database = crate::task_db::Database::open(&path).unwrap();
			let mut ids: Vec<_> = store::overlapping(database.connection(), window.0, window.1)
				.into_iter()
				.filter(|n| n.calendar_id == calendar)
				.map(|n| n.id)
				.collect();
			ids.sort();
			ids
		};
		let requests = Rc::new(RefCell::new(Vec::<(String, String)>::new()));
		// Google lists `listed` across two pages; writes echo their body.
		let sync = |listed: Vec<Value>| {
			let seen = requests.clone();
			let (first, second) = listed.split_at(listed.len() / 2);
			let (first, second) = (first.to_vec(), second.to_vec());
			api::with_transport(
				Box::new(move |method, url, body| {
					seen.borrow_mut().push((method.into(), url.into()));
					Ok(match (method, url.contains("pageToken=next")) {
						("GET", false) => json!({"items": first, "nextPageToken": "next"}),
						("GET", true) => json!({"items": second}),
						_ => serde_json::from_str(body.unwrap()).unwrap(),
					}
					.to_string())
				}),
				|| {
					let nights = fetch_nights("token", calendar, window.0, window.1)?;
					plan_nights(
						&settings,
						"token",
						calendar,
						&mut vec![],
						nights,
						window,
						now,
					)
				},
			)
		};
		let writes = |method: &str| {
			requests
				.borrow()
				.iter()
				.filter(|(m, _)| m == method)
				.count()
		};

		// Every planned night exists except the last, which is created.
		sync((0..28).map(night_on).collect()).unwrap();
		let made = requests.take();
		let lists: Vec<_> = made.iter().filter(|(m, _)| m == "GET").collect();
		assert_eq!(lists.len(), 2, "{made:?}");
		assert!(
			lists[0]
				.1
				.contains("privateExtendedProperty=ttKind%3Dsleep")
		);
		assert!(lists[0].1.contains("timeMin=") && lists[0].1.contains("timeMax="));
		assert!(!made.iter().any(|(_, url)| url.contains("ttSleepDate")));
		assert_eq!(made.iter().filter(|(m, _)| m == "POST").count(), 1);
		assert_eq!(made.iter().filter(|(m, _)| m == "PATCH").count(), 0);
		let all: Vec<_> = {
			let mut ids: Vec<_> = (0..=28)
				.map(|offset| event_id(date() + Duration::days(offset)))
				.collect();
			ids.sort();
			ids
		};
		assert_eq!(stored(), all);

		// A deleted event leaves the database.
		forget(calendar, &event_id(date() + Duration::days(28)));
		assert_eq!(stored().len(), 28);

		// Google wins: a night deleted there goes from the database, and is
		// planned again along with the one deleted here.
		let gone = event_id(date() + Duration::days(5));
		sync(
			(0..28)
				.filter(|&offset| offset != 5)
				.map(night_on)
				.collect(),
		)
		.unwrap();
		assert_eq!(writes("POST"), 2);
		assert!(stored().contains(&gone));
		assert_eq!(stored(), all);
	}

	#[test]
	fn schedule_changes_keep_the_actual_sleep_already_recorded() {
		let mut event = night();
		event.end = at(8);
		apply_timer(&mut event, Timer::Start, at(1)).unwrap();
		let mut events = vec![event];
		reconcile_with(
			"app",
			&mut events,
			date(),
			&[Times::default(); 7],
			at(0),
			|_| panic!(),
			|event, _| Ok(event.clone()),
		)
		.unwrap();
		assert_eq!(events.len(), 1);
		assert_eq!(events[0].end, at(8));
		assert_eq!(
			events[0].details.sleep.as_ref().unwrap().actual_start,
			Some(at(1))
		);
	}
}
