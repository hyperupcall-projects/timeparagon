//! One database-first loader shared by Do, Plan and agenda sheets. A navigation
//! reads covered dates; a refresh generation catches up with Google's change
//! stream once, even when several mounted views request that generation.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::NaiveDate;
use dioxus::prelude::*;
use timeparagon_core::calendar::{Snapshot, repository, sleep};
use timeparagon_core::config::CalendarSettings;

#[derive(Clone, Default)]
pub struct Loader(Arc<Mutex<Refresh>>);

#[derive(Clone, Copy)]
pub struct Changes(pub Signal<u64>);

#[derive(Default)]
struct Refresh {
	completed: Option<(u32, CalendarSettings)>,
}

impl Loader {
	fn load(
		&self,
		dates: &[NaiveDate],
		settings: &CalendarSettings,
		revision: u32,
	) -> Result<(Snapshot, bool), String> {
		let path = timeparagon_core::task_db::path().ok_or("Cannot locate the local database")?;
		self.load_with(&path, dates, settings, revision, || {
			if !crate::day::signed_in() {
				return Err(
					"Not connected to Google Calendar. Connect an account to load your day."
						.to_owned(),
				);
			}
			sleep::sync(settings, dates[0], dates)
		})
	}

	fn load_with(
		&self,
		path: &Path,
		dates: &[NaiveDate],
		settings: &CalendarSettings,
		revision: u32,
		fetch: impl FnOnce() -> Result<Snapshot, String>,
	) -> Result<(Snapshot, bool), String> {
		// Hold the lock through persistence, so a request from another view
		// observes both the new records and the completed refresh generation.
		let mut refresh = self.0.lock().map_err(|_| "The calendar worker stopped")?;
		let cached = repository::read(path, dates, settings.day_start_hour());
		let forced = match &refresh.completed {
			Some((seen, previous)) => *seen != revision || previous != settings,
			None => revision != 0,
		};
		let due = cached.as_ref().map_or(true, |read| {
			!read.complete(dates)
				|| (timeparagon_core::clock::now() - read.snapshot.fetched_at)
					.to_std()
					.is_ok_and(|elapsed| elapsed >= CalendarSettings::REFRESH_EVERY)
		});
		if !forced && !due {
			refresh.completed = Some((revision, settings.clone()));
			return Ok((cached?.snapshot, false));
		}

		let snapshot = fetch()?;
		refresh.completed = Some((revision, settings.clone()));
		Ok((snapshot, true))
	}
}

pub type CachedView = Signal<Option<(Vec<NaiveDate>, Snapshot)>>;

/// Publish the disk copy before waiting for the network. Retain it through a
/// refresh failure, and key it by the requested dates to prevent stale headings.
pub fn use_calendar(
	dates: Vec<NaiveDate>,
	settings: CalendarSettings,
	revision: Signal<u32>,
	active: bool,
) -> (
	Resource<Result<(Vec<NaiveDate>, Snapshot), String>>,
	CachedView,
) {
	let loader = use_context::<Loader>();
	let Changes(mut changes) = use_context::<Changes>();
	let mut cached = use_signal(|| None);
	let loaded = use_resource(use_reactive!(|(dates, settings, active)| {
		let revision = revision();
		let _ = changes();
		let loader = loader.clone();
		async move {
			let wanted = dates.clone();
			let options = settings.clone();
			let saved = crate::editor::worker(move || {
				let path =
					timeparagon_core::task_db::path().ok_or("Cannot locate the local database")?;
				repository::read(&path, &wanted, options.day_start_hour()).map(|read| read.snapshot)
			})
			.await;
			if let Ok(snapshot) = &saved {
				cached.set(Some((dates.clone(), snapshot.clone())));
			}
			if !active {
				return saved.map(|snapshot| (dates, snapshot));
			}

			let wanted = dates.clone();
			let (snapshot, fetched) =
				crate::editor::worker(move || loader.load(&wanted, &settings, revision)).await?;
			if fetched {
				let next = *changes.peek() + 1;
				changes.set(next);
			}
			Ok((dates, snapshot))
		}
	}));
	(loaded, cached)
}

#[cfg(test)]
mod tests {
	use super::*;
	use timeparagon_core::calendar::{at_clock, sample_snapshot};

	fn persist(path: &Path, snapshot: &Snapshot, dates: &[NaiveDate]) {
		repository::read(path, dates, 0).unwrap();
		let mut db = rusqlite::Connection::open(path).unwrap();
		let tx = db.transaction().unwrap();
		tx.execute(
			"INSERT OR REPLACE INTO caches VALUES('calendar_sync_state',?1)",
			[serde_json::json!({
				"fetched_at":snapshot.fetched_at, "calendars":snapshot.calendars, "notice":snapshot.notice,
			})
			.to_string()],
		)
		.unwrap();
		for day in dates {
			tx.execute(
				"INSERT INTO google_sync_windows VALUES(?1,?2)",
				[
					at_clock(*day, 0, 0)
						.to_utc()
						.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
					at_clock(day.succ_opt().unwrap(), 0, 0)
						.to_utc()
						.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
				],
			)
			.unwrap();
		}
		tx.commit().unwrap();
	}

	#[test]
	fn empty_covered_dates_survive_restart_and_refresh_once_per_generation() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		snapshot.events.clear();
		snapshot.fetched_at = timeparagon_core::clock::now();
		let day = snapshot.day;
		persist(&path, &snapshot, &[day]);
		let loader = Loader::default();
		loader
			.load_with(&path, &[day], &CalendarSettings::default(), 0, || {
				panic!("covered navigation fetched")
			})
			.unwrap();
		loader
			.load_with(&path, &[day], &CalendarSettings::default(), 1, || {
				Ok(snapshot.clone())
			})
			.unwrap();
		loader
			.load_with(&path, &[day], &CalendarSettings::default(), 1, || {
				panic!("same refresh repeated")
			})
			.unwrap();
		let next = day.succ_opt().unwrap();
		let mut fetched = false;
		loader
			.load_with(&path, &[next], &CalendarSettings::default(), 1, || {
				fetched = true;
				Ok(snapshot.clone())
			})
			.unwrap();
		assert!(
			fetched,
			"an uncovered range fetches even in the same generation"
		);
	}

	#[test]
	fn a_failed_refresh_preserves_confirmed_data_and_can_retry() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		snapshot.events.clear();
		snapshot.fetched_at = timeparagon_core::clock::now() - chrono::Duration::days(1);
		let dates = [snapshot.day];
		persist(&path, &snapshot, &dates);
		let before = repository::read(&path, &dates, 0).unwrap();
		let loader = Loader::default();
		assert!(
			loader
				.load_with(&path, &dates, &CalendarSettings::default(), 2, || Err(
					"offline".into()
				))
				.is_err()
		);
		let after = repository::read(&path, &dates, 0).unwrap();
		assert_eq!(before.snapshot.events, after.snapshot.events);
		assert_eq!(before.snapshot.fetched_at, after.snapshot.fetched_at);
		snapshot.fetched_at = timeparagon_core::clock::now();
		loader
			.load_with(&path, &dates, &CalendarSettings::default(), 2, || {
				persist(&path, &snapshot, &dates);
				Ok(snapshot.clone())
			})
			.unwrap();
		assert_eq!(
			repository::read(&path, &dates, 0)
				.unwrap()
				.snapshot
				.fetched_at,
			snapshot.fetched_at
		);
	}
}
