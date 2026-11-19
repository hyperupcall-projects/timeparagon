//! Refresh confirmed occurrences after a write changes a recurring series.
//!
//! Google expands the series over the locally covered dates. The worker updates
//! `google_sync_events`; the same database revision wakes all calendar readers.
//! Coverage is retained when a series deletion leaves a previously fetched day empty.

use super::*;
use std::{cell::RefCell, path::Path};

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

/// Fetches occurrences over the covered dates and updates confirmed rows.
/// `entity` is the
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
	let _syncing = api::incremental::sync_guard()?;
	let mut database = crate::task_db::Database::open(path)?;
	let Some((first, last)) = api::incremental::covered_dates(database.connection())? else {
		return Ok(());
	};
	let settings = crate::config::Config::load().calendar;
	let day_end_hour = settings.day_start_hour();
	let token = GoogleClient::from_settings(&settings)
		.access_token()
		.map_err(|e| e.to_string())?;
	let calendars = repository::state(database.connection())?
		.map(|s| s.calendars)
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
	refresh.events = events;
	persist(path, &refresh)?;

	Ok(())
}

/// Replace the confirmed series rows; coverage remains unchanged, even when
/// this series was the only event of an already fetched day.
fn persist(path: &Path, refresh: &Refresh) -> Result<(), String> {
	let mut database = crate::task_db::Database::open(path)?;
	let tx = database
		.connection()
		.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
		.map_err(|e| e.to_string())?;
	for series in &refresh.series {
		for calendar in &series.calendars {
			tx.execute(
				"DELETE FROM google_sync_events WHERE calendar_id=?1 AND (id=?2 OR series=?2)",
				rusqlite::params![calendar, series.id],
			)
			.map_err(|e| e.to_string())?;
		}
	}
	for event in &refresh.events {
		api::incremental::store(&tx, event, event.is_calendar_task())?;
	}
	tx.commit().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn series_refresh_replaces_confirmed_rows_and_preserves_empty_day_coverage() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		snapshot.events.truncate(1);
		let day = snapshot.day;
		let next = day.succ_opt().unwrap();
		snapshot.events[0].details.recurring_event_id = Some("series".into());
		let series = SeriesRef {
			calendar: snapshot.events[0].calendar_id.clone(),
			id: "series".into(),
			calendars: vec![snapshot.events[0].calendar_id.clone()],
		};
		repository::seed(&path, &snapshot, &[day, next], 0);
		let mut moved = snapshot.events[0].clone();
		moved.start += chrono::Duration::days(1);
		moved.end += chrono::Duration::days(1);
		moved.id = "renamed".into();
		let mut refresh = Refresh {
			series: vec![series],
			first: day,
			last: next,
			events: vec![moved.clone()],
		};
		persist(&path, &refresh).unwrap();
		let old = repository::read(&path, &[day], 0).unwrap();
		assert!(old.complete(&[day]));
		assert!(old.snapshot.events.is_empty());
		assert_eq!(
			repository::read(&path, &[next], 0).unwrap().snapshot.events,
			vec![moved]
		);
		refresh.events.clear();
		persist(&path, &refresh).unwrap();
		let empty = repository::read(&path, &[next], 0).unwrap();
		assert!(empty.complete(&[next]));
		assert!(empty.snapshot.events.is_empty());
	}
}
