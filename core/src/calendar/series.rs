//! Keeps cached occurrences of a recurring series in step with Google after a
//! write that changes many of them at once.
//!
//! Saving or deleting "this and following" or "all" events changes
//! occurrences on days other than the one being edited. The local overlay
//! (see [`operations`](super::operations)) only covers the edited occurrence,
//! and the sync asked for straight after saving can reach Google before the
//! write does. So once Google confirms such a write, the worker asks Google
//! for the series' occurrences across the cached dates and swaps them into the
//! cache: Google expands the series, so its rules are never reproduced here.
//!
//! The disk cache is updated on the worker thread. The interface's in-memory
//! days are updated when it next collects the results with [`take_refreshed`].

use super::*;
use std::{cell::RefCell, path::Path, sync::Mutex};

/// A recurring series a write changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesRef {
	/// The calendar the series is on now, where its occurrences are fetched.
	pub calendar: String,
	/// Google's ID of the series' master event.
	pub id: String,
	/// The calendars whose cached copies of the series are replaced: where it
	/// is now, and where it was before a move.
	pub calendars: Vec<String>,
}

impl SeriesRef {
	fn contains(&self, event: &Event) -> bool {
		self.calendars.contains(&event.calendar_id)
			&& (event.id == self.id
				|| event.details.recurring_event_id.as_deref() == Some(self.id.as_str()))
	}
}

/// The occurrences of some series between two days, as Google had them just
/// after a write.
#[derive(Debug, Clone)]
pub struct Refresh {
	pub series: Vec<SeriesRef>,
	pub first: NaiveDate,
	pub last: NaiveDate,
	pub events: Vec<Event>,
}

impl Refresh {
	/// Whether `event` is an occurrence (or the master) of a refreshed series.
	pub fn contains(&self, event: &Event) -> bool {
		!event.is_task() && self.series.iter().any(|series| series.contains(event))
	}

	fn covers(&self, day: NaiveDate) -> bool {
		(self.first..=self.last).contains(&day)
	}

	/// Replaces the series' occurrences in one cached day's events with
	/// `fresh`. A day is only given new occurrences if it already held
	/// calendar events: one holding nothing yet has not been fetched, and the
	/// occurrences alone would pass it off as loaded.
	pub(super) fn apply_to_day(
		&self,
		day: NaiveDate,
		events: &mut Vec<Event>,
		fresh: &[Event],
		day_end_hour: u32,
	) {
		if !self.covers(day) {
			return;
		}
		let loaded = events
			.iter()
			.any(|event| !event.is_task() && event_on_day(event, day, day_end_hour));
		let before = events.len();
		events.retain(|event| !self.contains(event));
		let mut changed = events.len() != before;
		if loaded {
			for event in fresh
				.iter()
				.filter(|event| event_on_day(event, day, day_end_hour))
			{
				if !events
					.iter()
					.any(|e| e.id == event.id && e.calendar_id == event.calendar_id)
				{
					events.push(event.clone());
					changed = true;
				}
			}
		}
		if changed {
			events
				.sort_by(|left, right| left.start.cmp(&right.start).then(right.end.cmp(&left.end)));
		}
	}

	/// Replaces the series' occurrences in one cached month's events, by the
	/// same rule as [`Refresh::apply_to_day`] for each of its days.
	pub(super) fn apply_to_month(
		&self,
		month: NaiveDate,
		events: &mut Vec<Event>,
		day_end_hour: u32,
	) {
		let days: Vec<NaiveDate> = month
			.iter_days()
			.take_while(|day| day.month() == month.month())
			.filter(|&day| self.covers(day))
			.collect();
		let loaded: Vec<NaiveDate> = days
			.iter()
			.copied()
			.filter(|&day| {
				events
					.iter()
					.any(|event| !event.is_task() && event_on_day(event, day, day_end_hour))
			})
			.collect();
		events.retain(|event| {
			!(self.contains(event) && days.contains(&day_at(event.start, day_end_hour)))
		});
		for event in &self.events {
			if loaded
				.iter()
				.any(|&day| event_on_day(event, day, day_end_hour))
				&& !events
					.iter()
					.any(|e| e.id == event.id && e.calendar_id == event.calendar_id)
			{
				events.push(event.clone());
			}
		}
	}
}

thread_local! {
	/// The series the write being sent on this thread has changed so far.
	static NOTED: RefCell<Vec<SeriesRef>> = const { RefCell::new(Vec::new()) };
}

/// Records that the write being sent changed the series `id`, now on
/// `calendar`. Call it only once Google has accepted the change.
pub(super) fn note(calendar: &str, id: &str, calendars: Vec<String>) {
	NOTED.with(|noted| {
		noted.borrow_mut().push(SeriesRef {
			calendar: calendar.to_owned(),
			id: id.to_owned(),
			calendars,
		})
	});
}

/// The series noted on this thread since the last call.
pub(super) fn take_noted() -> Vec<SeriesRef> {
	NOTED.with(|noted| std::mem::take(&mut *noted.borrow_mut()))
}

static REFRESHED: Mutex<Vec<Refresh>> = Mutex::new(Vec::new());

/// The refreshes the worker has made since the last call, oldest first, for
/// the interface to apply to the days it holds in memory.
pub fn take_refreshed() -> Vec<Refresh> {
	REFRESHED
		.lock()
		.map(|mut refreshed| std::mem::take(&mut *refreshed))
		.unwrap_or_default()
}

/// Fetches the series' occurrences across the cached months and puts them in
/// the disk cache, then queues them for the interface. `entity` is the
/// `google_entities` row of the write that changed them. Failure only costs
/// waiting for the next sync, so it is logged rather than returned.
pub(super) fn refresh(path: &Path, entity: &str, series: Vec<SeriesRef>) {
	if series.is_empty() {
		return;
	}
	if let Err(error) = refresh_in(path, entity, series) {
		log::warn!("could not refresh the changed series: {error}");
	}
}

fn refresh_in(path: &Path, entity: &str, series: Vec<SeriesRef>) -> Result<(), String> {
	let months = day_cache::days(path);
	let (Some(&first), Some(&last_month)) = (months.iter().min(), months.iter().max()) else {
		return Ok(());
	};
	let last = last_month
		.iter_days()
		.take_while(|day| day.month() == last_month.month())
		.last()
		.unwrap_or(last_month);
	let settings = crate::config::Config::load().calendar;
	let day_end_hour = settings.day_start_hour();
	let token = GoogleClient::from_settings(&settings)
		.access_token()
		.map_err(|error| error.to_string())?;
	let calendars = day_cache::load_sync_state(path)
		.map(|(_, calendars)| calendars)
		.unwrap_or_default();
	// Taken after the write was acknowledged, so the overlay knows this fetch
	// already shows it.
	let fetched_at = crate::clock::now();
	let mut events = Vec::new();
	for series in &series {
		let calendar = calendars
			.iter()
			.find(|calendar| calendar.id == series.calendar)
			.cloned()
			.unwrap_or_else(|| Calendar {
				id: series.calendar.clone(),
				name: String::new(),
				color: colors::DEFAULT_EVENT,
				primary: false,
				selected_by_google: true,
			});
		events.extend(
			api::fetch_series_instances(&token, &calendar, &series.id, first, last, day_end_hour)
				.map_err(|error| error.to_string())?,
		);
	}
	let mut refresh = Refresh {
		series,
		first,
		last,
		events: Vec::new(),
	};
	operations::observe_series(
		path,
		entity,
		|event| refresh.series.iter().any(|series| series.contains(event)),
		&events,
		first,
		last,
		fetched_at,
		day_end_hour,
	)?;
	// Other edits to the series still on their way to Google stay shown.
	operations::overlay(path, &mut events, Some(fetched_at))?;
	events.retain(|event| refresh.contains(event));
	refresh.events = events;
	day_cache::replace_series(path, &refresh, day_end_hour);
	if let Ok(mut refreshed) = REFRESHED.lock() {
		refreshed.push(refresh);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn day(d: u32) -> NaiveDate {
		NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
	}

	fn occurrence(id: &str, series: &str, d: u32, hour: u32, title: &str) -> Event {
		let mut event = sample_snapshot().events[0].clone();
		event.id = id.into();
		event.calendar_id = "primary".into();
		event.title = title.into();
		event.all_day = false;
		event.start = at_clock(day(d), hour, 0);
		event.end = at_clock(day(d), hour + 1, 0);
		event.details.recurring_event_id = Some(series.into());
		event.details.recurrence.clear();
		event.details.task_data = None;
		event
	}

	fn other(id: &str, d: u32) -> Event {
		let mut event = occurrence(id, "", d, 14, "Lunch");
		event.details.recurring_event_id = None;
		event
	}

	fn refresh(events: Vec<Event>) -> Refresh {
		Refresh {
			series: vec![SeriesRef {
				calendar: "primary".into(),
				id: "standup".into(),
				calendars: vec!["primary".into()],
			}],
			first: day(1),
			last: day(30),
			events,
		}
	}

	#[test]
	fn a_cached_day_takes_the_new_title_and_time_of_its_occurrence() {
		let mut events = vec![
			occurrence("standup_10", "standup", 10, 9, "Standup"),
			other("lunch", 10),
		];
		let fresh = vec![occurrence("standup_10", "standup", 10, 11, "Sync")];
		refresh(fresh.clone()).apply_to_day(day(10), &mut events, &fresh, 0);
		let titles: Vec<_> = events.iter().map(|e| e.title.as_str()).collect();
		assert_eq!(titles, ["Sync", "Lunch"]);
		assert_eq!(events[0].start, at_clock(day(10), 11, 0));
	}

	#[test]
	fn deleted_occurrences_leave_and_a_split_series_takes_their_place() {
		let mut events = vec![
			occurrence("standup_10", "standup", 10, 9, "Standup"),
			other("lunch", 10),
		];
		let mut split = refresh(Vec::new());
		split.series.push(SeriesRef {
			calendar: "primary".into(),
			id: "standup_new".into(),
			calendars: vec!["primary".into()],
		});
		let fresh = vec![occurrence("standup_new_10", "standup_new", 10, 9, "Daily")];
		split.apply_to_day(day(10), &mut events, &fresh, 0);
		let ids: Vec<_> = events.iter().map(|e| e.id.as_str()).collect();
		assert_eq!(ids, ["standup_new_10", "lunch"]);

		let mut events = vec![occurrence("standup_10", "standup", 10, 9, "Standup")];
		refresh(Vec::new()).apply_to_day(day(10), &mut events, &[], 0);
		assert!(events.is_empty());
	}

	#[test]
	fn a_day_never_fetched_is_not_passed_off_as_loaded() {
		let mut events = Vec::new();
		let fresh = vec![occurrence("standup_12", "standup", 12, 9, "Standup")];
		refresh(fresh.clone()).apply_to_day(day(12), &mut events, &fresh, 0);
		assert!(events.is_empty());
	}

	#[test]
	fn other_series_and_days_outside_the_window_are_left_alone() {
		let mut events = vec![
			occurrence("review_10", "review", 10, 9, "Review"),
			occurrence("standup_10", "standup", 10, 10, "Standup"),
		];
		let mut narrow = refresh(Vec::new());
		narrow.first = day(11);
		narrow.apply_to_day(day(10), &mut events, &[], 0);
		assert_eq!(events.len(), 2);
		refresh(Vec::new()).apply_to_day(day(10), &mut events, &[], 0);
		let ids: Vec<_> = events.iter().map(|e| e.id.as_str()).collect();
		assert_eq!(ids, ["review_10"]);
	}

	#[test]
	fn the_disk_cache_takes_the_series_on_every_loaded_day() {
		let scratch = day_cache::Scratch::new("series-refresh");
		let path = scratch.path();
		day_cache::save(
			path,
			day(10),
			&[
				occurrence("standup_10", "standup", 10, 9, "Standup"),
				other("lunch_10", 10),
			],
			0,
		)
		.unwrap();
		day_cache::save(
			path,
			day(17),
			&[occurrence("standup_17", "standup", 17, 9, "Standup")],
			0,
		)
		.unwrap();
		let moved = refresh(vec![
			occurrence("standup_10", "standup", 10, 11, "Sync"),
			occurrence("standup_17", "standup", 17, 11, "Sync"),
			occurrence("standup_24", "standup", 24, 11, "Sync"),
		]);
		day_cache::replace_series(path, &moved, 0);
		let titles = |d| {
			let mut events = day_cache::load(path, day(d), 0).unwrap();
			events.sort_by_key(|e| e.start);
			events
				.into_iter()
				.map(|e| (e.title, e.start))
				.collect::<Vec<_>>()
		};
		assert_eq!(
			titles(10),
			[
				("Sync".into(), at_clock(day(10), 11, 0)),
				("Lunch".into(), at_clock(day(10), 14, 0))
			]
		);
		assert_eq!(titles(17), [("Sync".into(), at_clock(day(17), 11, 0))]);
		assert!(titles(24).is_empty());

		day_cache::replace_series(path, &refresh(Vec::new()), 0);
		assert_eq!(titles(10), [("Lunch".into(), at_clock(day(10), 14, 0))]);
		assert!(titles(17).is_empty());
	}

	#[test]
	fn the_store_updates_the_day_in_view_and_every_cached_day() {
		let mut store = Store::with_snapshot(sample_snapshot());
		store.day = day(10);
		store.events = vec![occurrence("standup_10", "standup", 10, 9, "Standup")];
		store.cache.insert(
			day(17),
			vec![
				occurrence("standup_17", "standup", 17, 9, "Standup"),
				other("lunch_17", 17),
			],
		);
		store.open_event = Some("standup_17".into());
		store.apply_series_refresh(&refresh(Vec::new()));
		// Deleting the series from here on leaves only the other events.
		assert!(store.events.is_empty());
		let ids: Vec<_> = store
			.events_on(day(17))
			.iter()
			.map(|e| e.id.as_str())
			.collect();
		assert_eq!(ids, ["lunch_17"]);
		assert!(store.open_event.is_none());

		let mut split = refresh(vec![occurrence(
			"standup_new_17",
			"standup_new",
			17,
			9,
			"Daily",
		)]);
		split.series.push(SeriesRef {
			calendar: "primary".into(),
			id: "standup_new".into(),
			calendars: vec!["primary".into()],
		});
		store.apply_series_refresh(&split);
		let ids: Vec<_> = store
			.events_on(day(17))
			.iter()
			.map(|e| e.id.as_str())
			.collect();
		assert_eq!(ids, ["standup_new_17", "lunch_17"]);
	}

	#[test]
	fn a_month_updates_only_the_days_it_had_loaded() {
		let mut events = vec![
			occurrence("standup_10", "standup", 10, 9, "Standup"),
			other("lunch", 17),
		];
		let fresh = refresh(vec![
			occurrence("standup_10", "standup", 10, 9, "Sync"),
			occurrence("standup_17", "standup", 17, 9, "Sync"),
			occurrence("standup_24", "standup", 24, 9, "Sync"),
		]);
		fresh.apply_to_month(day(1), &mut events, 0);
		let mut ids: Vec<_> = events.iter().map(|e| e.id.as_str()).collect();
		ids.sort();
		// Day 10 lost its only event before the new one arrived, but it was
		// loaded; day 24 never was.
		assert_eq!(ids, ["lunch", "standup_10", "standup_17"]);
	}
}
