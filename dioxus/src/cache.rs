//! One database-first loader shared by Do, Plan and agenda sheets. A navigation
//! reads covered dates; a refresh generation catches up with Google's change
//! stream once, even when several mounted views request that generation.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::NaiveDate;
use dioxus::prelude::*;
use timeparagon_core::calendar::{Snapshot, sleep, view_cache};
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
		let cached = view_cache::load(path, dates, settings.day_start_hour())?;
		let forced = match &refresh.completed {
			Some((seen, previous)) => *seen != revision || previous != settings,
			None => revision != 0,
		};
		let due = cached.as_ref().is_none_or(|snapshot| {
			(timeparagon_core::clock::now() - snapshot.fetched_at)
				.to_std()
				.is_ok_and(|elapsed| elapsed >= CalendarSettings::REFRESH_EVERY)
		});
		if !forced
			&& !due && let Some(snapshot) = cached
		{
			refresh.completed = Some((revision, settings.clone()));
			return Ok((snapshot, false));
		}
		let snapshot = fetch()?;
		view_cache::save(path, &snapshot, dates, settings.day_start_hour())?;
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
				view_cache::load(&path, &wanted, options.day_start_hour())
			})
			.await?;
			if let Some(snapshot) = &saved {
				cached.set(Some((dates.clone(), snapshot.clone())));
			}
			if !active {
				return saved
					.map(|snapshot| (dates, snapshot))
					.ok_or_else(|| "No cached calendar data yet.".to_owned());
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

	fn options() -> CalendarSettings {
		CalendarSettings::default()
	}

	/// Records `day` as complete, as a sync that fetched it would.
	fn cover(path: &Path, day: NaiveDate) {
		timeparagon_core::task_db::Database::open(path).unwrap();
		let bound = |day: NaiveDate| at_clock(day, 0, 0).to_utc().to_rfc3339();
		rusqlite::Connection::open(path)
			.unwrap()
			.execute(
				"INSERT INTO google_sync_windows (start_at,end_at) VALUES (?1,?2)",
				[bound(day), bound(day.succ_opt().unwrap())],
			)
			.unwrap();
	}

	#[test]
	fn navigation_and_reopening_use_persisted_empty_days_and_refresh_once() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		snapshot.fetched_at = timeparagon_core::clock::now();
		snapshot.events.clear();
		let day = snapshot.day;
		cover(&path, day);
		let loader = Loader::default();
		let mut calls = 0;
		loader
			.load_with(&path, &[day], &options(), 0, || {
				calls += 1;
				Ok(snapshot.clone())
			})
			.unwrap();
		assert_eq!(calls, 1);
		// A new loader represents a restarted app, with no in-memory state.
		let loader = Loader::default();
		loader
			.load_with(&path, &[day], &options(), 0, || {
				panic!("cached navigation fetched")
			})
			.unwrap();
		loader
			.load_with(&path, &[day], &options(), 1, || {
				calls += 1;
				Ok(snapshot.clone())
			})
			.unwrap();
		loader
			.load_with(&path, &[day], &options(), 1, || {
				panic!("another view repeated the refresh")
			})
			.unwrap();
		assert_eq!(calls, 2);
		let next = day.succ_opt().unwrap();
		loader
			.load_with(&path, &[next], &options(), 1, || {
				calls += 1;
				let mut next_snapshot = snapshot.clone();
				next_snapshot.day = next;
				Ok(next_snapshot)
			})
			.unwrap();
		assert_eq!(
			calls, 3,
			"an uncovered date still fetches in this generation"
		);
	}

	#[test]
	fn a_failed_refresh_preserves_the_disk_copy_and_can_retry() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("calendar.sqlite3");
		let mut snapshot = sample_snapshot();
		snapshot.fetched_at = timeparagon_core::clock::now() - chrono::Duration::days(1);
		let day = snapshot.day;
		cover(&path, day);
		view_cache::save(&path, &snapshot, &[day], 0).unwrap();
		let before = view_cache::load(&path, &[day], 0).unwrap().unwrap();
		let loader = Loader::default();
		assert!(
			loader
				.load_with(&path, &[day], &options(), 2, || Err("offline".into()))
				.is_err()
		);
		let after = view_cache::load(&path, &[day], 0).unwrap().unwrap();
		assert_eq!(after.events, before.events);
		assert_eq!(after.fetched_at, before.fetched_at);
		snapshot.fetched_at = timeparagon_core::clock::now();
		loader
			.load_with(&path, &[day], &options(), 2, || Ok(snapshot.clone()))
			.unwrap();
		assert_eq!(
			view_cache::load(&path, &[day], 0)
				.unwrap()
				.unwrap()
				.fetched_at,
			snapshot.fetched_at
		);
	}
}
