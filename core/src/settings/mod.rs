//! Settings: every preference both apps keep, in the local database.
//!
//! They used to be `config.toml`, one file per device. Now they are rows of
//! the tables in [`schema`], most of them shared between devices by
//! [`crate::database_sync`] like any other synced data. Those in
//! `device_settings` describe this device (see [`mapping::DEVICE_SECTIONS`])
//! and stay on it. The first start of this version imports `config.toml`
//! (see [`import`]).
//!
//! The apps keep working with their `Config` types. This module turns one
//! into rows and back, through the JSON tree serde gives it, so the desktop's
//! config and the phone's smaller one read and write the same rows.
//!
//! # Saving only what changed
//!
//! A save is given what was loaded as well as what is wanted, and writes only
//! the rows (and, within a row, only the columns) that differ between the two.
//! Settings another device or process changed in the meantime are left as
//! they are, rather than put back to what this process last saw. A value
//! at its default is not stored at all, so a device that never touched a
//! setting cannot override another device's choice.

mod import;
mod mapping;
pub(crate) mod schema;

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

pub use import::import_file;
pub(crate) use import::import_once;
use mapping::Rows;

use crate::task_db::Database;

fn other(error: impl std::fmt::Display) -> io::Error {
	io::Error::other(error.to_string())
}

fn invalid(error: impl std::fmt::Display) -> io::Error {
	io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

/// Settings read from the database.
#[derive(Debug)]
pub struct Loaded<T> {
	pub value: T,
	/// Stored settings that could not be read, each with why. They were left
	/// at their defaults, and stay in the database untouched.
	pub problems: Vec<String>,
}

fn tree_of<T: Serialize>(value: &T) -> io::Result<Value> {
	serde_json::to_value(value).map_err(invalid)
}

/// Every stored setting as one tree, documents included.
fn read_tree(db: &Connection, path: &Path) -> io::Result<Value> {
	let rows = mapping::read(db).map_err(other)?;
	let mut tree = mapping::tree(&rows);
	if let Value::Object(sections) = &mut tree {
		crate::shared_data::overlay(db, path, sections)?;
	}
	Ok(tree)
}

fn deserialize<T: DeserializeOwned>(
	tree: Value,
	ignored: &mut dyn FnMut(String),
) -> Result<T, serde_json::Error> {
	serde_ignored::deserialize(tree, |path| ignored(path.to_string()))
}

/// `T` from `tree`. A field that does not fit `T` is left at its default and
/// reported, rather than taking every other setting down with it. Fields `T`
/// does not have are passed to `ignored`.
pub(crate) fn decode<T: DeserializeOwned + Default>(
	tree: Value,
	mut ignored: impl FnMut(String),
) -> Loaded<T> {
	let mut unknown = Vec::new();
	if let Ok(value) = deserialize(tree.clone(), &mut |path| unknown.push(path)) {
		unknown.into_iter().for_each(&mut ignored);
		return Loaded {
			value,
			problems: Vec::new(),
		};
	}
	let mut clean = Map::new();
	let mut problems = Vec::new();
	let mut fits = |name: &str, field: Option<(&str, &Value)>, value: &Value| {
		let mut single = Map::new();
		match field {
			Some((field, value)) => {
				let mut fields = Map::new();
				fields.insert(field.to_owned(), value.clone());
				single.insert(name.to_owned(), Value::Object(fields));
			}
			None => {
				single.insert(name.to_owned(), value.clone());
			}
		}
		match deserialize::<T>(Value::Object(single), &mut |_| {}) {
			Ok(_) => true,
			Err(error) => {
				let path = field.map_or(name.to_owned(), |(field, _)| format!("{name}.{field}"));
				problems.push(format!("{path}: {error}"));
				false
			}
		}
	};
	for (name, value) in tree.as_object().into_iter().flatten() {
		match value {
			Value::Object(fields) => {
				let mut kept = Map::new();
				for (field, value) in fields {
					if fits(name, Some((field, value)), value) {
						kept.insert(field.clone(), value.clone());
					}
				}
				clean.insert(name.clone(), Value::Object(kept));
			}
			value => {
				if fits(name, None, value) {
					clean.insert(name.clone(), value.clone());
				}
			}
		}
	}
	let mut unknown = Vec::new();
	match deserialize(Value::Object(clean), &mut |path| unknown.push(path)) {
		Ok(value) => {
			unknown.into_iter().for_each(&mut ignored);
			Loaded { value, problems }
		}
		Err(error) => {
			problems.push(error.to_string());
			Loaded {
				value: T::default(),
				problems,
			}
		}
	}
}

/// The rows a tree is stored as, leaving out values at their default.
fn stored_rows(tree: &Value, defaults: &Rows) -> Rows {
	mapping::without_defaults(mapping::rows(tree), defaults)
}

/// Writes the rows that differ between `before` and `after`. Of a row that
/// is in both, only the columns that differ are written.
fn write_rows(tx: &Connection, before: &Rows, after: &Rows) -> rusqlite::Result<()> {
	let keys: BTreeSet<_> = before.keys().chain(after.keys()).collect();
	for key in keys {
		let table = schema::table(key.0).expect("rows are only made for settings tables");
		match (before.get(key), after.get(key)) {
			(before, after) if before == after => {}
			(Some(before), Some(after)) => {
				let columns: Vec<&str> = table
					.columns
					.iter()
					.filter(|column| before.get(column.name) != after.get(column.name))
					.map(|column| column.name)
					.collect();
				// Gone meanwhile, deleted elsewhere: what was wanted wins.
				if !table.update(tx, after, &columns)? {
					table.upsert(tx, after)?;
				}
			}
			(_, Some(after)) => table.upsert(tx, after)?,
			(Some(before), None) => table.delete(tx, &table.key_of(before))?,
			(None, None) => unreachable!("the key came from one of the two"),
		}
	}
	Ok(())
}

/// Writes what changed between the trees `before` and `after`.
fn write_tree(
	tx: &Connection,
	path: &Path,
	before: &Value,
	after: &Value,
	defaults: &Value,
) -> io::Result<crate::shared_data::Stored> {
	let defaults = mapping::rows(defaults);
	write_rows(
		tx,
		&stored_rows(before, &defaults),
		&stored_rows(after, &defaults),
	)
	.map_err(other)?;
	let empty = Map::new();
	let before = before.as_object().unwrap_or(&empty);
	let after = after.as_object().unwrap_or(&empty);
	let changed: Vec<&str> = crate::shared_data::SECTIONS
		.into_iter()
		.filter(|name| {
			crate::shared_data::section(before, name) != crate::shared_data::section(after, name)
		})
		.collect();
	crate::shared_data::store_in(tx, path, after, &changed)
}

/// Reads the settings in the database at `path` into `T`, over `T`'s
/// defaults. Stored settings `T` has no field for are passed to `ignored`:
/// another app's, or a newer version's.
pub fn load<T: DeserializeOwned + Default>(
	path: &Path,
	ignored: impl FnMut(String),
) -> io::Result<Loaded<T>> {
	let mut db = Database::open(path).map_err(other)?;
	let tree = read_tree(db.connection(), path)?;
	Ok(decode(tree, ignored))
}

/// Saves what differs between `baseline`, the settings as they were loaded
/// (or last saved), and `desired`.
pub fn save<T: Serialize + Default>(path: &Path, baseline: &T, desired: &T) -> io::Result<()> {
	let defaults = tree_of(&T::default())?;
	let before = tree_of(baseline)?;
	let after = tree_of(desired)?;
	let mut db = Database::open(path).map_err(other)?;
	let tx = db
		.connection()
		.transaction_with_behavior(TransactionBehavior::Immediate)
		.map_err(other)?;
	let stored = write_tree(&tx, path, &before, &after, &defaults)?;
	tx.commit().map_err(other)?;
	stored.remember(path);
	Ok(())
}

/// Reads the settings, lets `change` change them, and saves what it changed,
/// with other writers held off in between.
pub fn update<T, R>(path: &Path, change: impl FnOnce(&mut T) -> R) -> io::Result<R>
where
	T: Serialize + DeserializeOwned + Default,
{
	let defaults = tree_of(&T::default())?;
	let mut db = Database::open(path).map_err(other)?;
	let tx = db
		.connection()
		.transaction_with_behavior(TransactionBehavior::Immediate)
		.map_err(other)?;
	let mut value: T = decode(read_tree(&tx, path)?, |_| {}).value;
	let before = tree_of(&value)?;
	let result = change(&mut value);
	let after = tree_of(&value)?;
	let stored = write_tree(&tx, path, &before, &after, &defaults)?;
	tx.commit().map_err(other)?;
	stored.remember(path);
	Ok(result)
}

/// Whether `a` and `b` are stored the same. Lists whose order storage does
/// not keep (hidden calendars, places, …) may differ in order and still be
/// the same. A process can tell its own save coming back this way.
pub fn same_stored<T: Serialize>(a: &T, b: &T) -> bool {
	let (Ok(a), Ok(b)) = (tree_of(a), tree_of(b)) else {
		return false;
	};
	let empty = Map::new();
	let sections = |tree: &Value, name: &str| {
		crate::shared_data::section(tree.as_object().unwrap_or(&empty), name)
	};
	mapping::rows(&a) == mapping::rows(&b)
		&& crate::shared_data::SECTIONS
			.into_iter()
			.all(|name| sections(&a, name) == sections(&b, name))
}

/// One of this device's settings by its `section.field` name, without reading
/// the rest.
pub fn device_value(path: &Path, key: &str) -> io::Result<Option<Value>> {
	let mut db = Database::open(path).map_err(other)?;
	let text: Option<String> = db
		.connection()
		.query_row(
			"SELECT value FROM device_settings WHERE key=?1",
			[key],
			|row| row.get(0),
		)
		.optional()
		.map_err(other)?;
	text.map(|text| serde_json::from_str(&text).map_err(invalid))
		.transpose()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::config::Config;

	fn database() -> (tempfile::TempDir, std::path::PathBuf) {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("timeparagon.sqlite3");
		(directory, path)
	}

	fn count(path: &Path, table: &str) -> i64 {
		let mut db = Database::open(path).unwrap();
		db.connection()
			.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
				row.get(0)
			})
			.unwrap()
	}

	#[test]
	fn nothing_is_stored_for_settings_left_at_their_defaults() {
		let (_directory, path) = database();
		let config = Config::default();
		save(&path, &config, &config).unwrap();
		update(&path, |_: &mut Config| {}).unwrap();
		for table in schema::TABLES {
			assert_eq!(count(&path, table.name), 0, "{}", table.name);
		}

		update(&path, |config: &mut Config| {
			config.calendar.sync_minutes = 30
		})
		.unwrap();
		assert_eq!(count(&path, "settings"), 1);
		update(&path, |config: &mut Config| {
			config.calendar.sync_minutes = 60
		})
		.unwrap();
		assert_eq!(
			count(&path, "settings"),
			0,
			"back at the default, it is removed"
		);
	}

	#[test]
	fn a_save_from_a_stale_view_keeps_what_others_changed_meanwhile() {
		let (_directory, path) = database();
		let found = crate::places::Found {
			place_id: "mapbox-1".into(),
			name: "Library".into(),
			address: "1 Main St".into(),
			lat: 1.5,
			lng: 2.5,
		};
		let id = update(&path, |config: &mut Config| {
			config.calendar.sync_minutes = 30;
			crate::places::add_place(
				&mut config.travel.places,
				&found.name,
				&found.address,
				&found.place_id,
				found.lat,
				found.lng,
			)
		})
		.unwrap();
		let stale: Config = load(&path, |_| {}).unwrap().value;

		// Another process, or another device, changes things.
		update(&path, |config: &mut Config| {
			config.calendar.sync_minutes = 45;
			config.travel.places[0].favorite = true;
			config.travel.event_places.insert("standup".into(), id);
		})
		.unwrap();

		// This one only renamed the place.
		let mut desired = stale.clone();
		desired.travel.places[0].name = "Campus".into();
		save(&path, &stale, &desired).unwrap();

		let saved: Config = load(&path, |_| {}).unwrap().value;
		assert_eq!(saved.calendar.sync_minutes, 45);
		assert_eq!(saved.travel.places[0].name, "Campus");
		assert!(
			saved.travel.places[0].favorite,
			"the other column of the row is kept"
		);
		assert_eq!(saved.travel.event_places["standup"], id);
	}

	#[test]
	fn settings_round_trip_through_their_tables() {
		use crate::calendar::sleep;
		let (_directory, path) = database();
		let from = chrono::NaiveDate::from_ymd_opt(2030, 11, 4).unwrap();
		let times = sleep::Times {
			bedtime: chrono::NaiveTime::from_hms_opt(23, 15, 0).unwrap(),
			wakeup: chrono::NaiveTime::from_hms_opt(7, 30, 0).unwrap(),
		};
		let wanted = update(&path, |config: &mut Config| {
			sleep::set_weekday_times(&mut config.calendar, from, times);
			config.calendar.set_calendar_enabled("work", false);
			config.calendar.source_order = vec!["b".into(), "a".into()];
			config
				.calendar
				.set_task_color("tasks:list", Some(crate::Color::WHITE));
			config
				.calendar
				.weekly_task_order
				.insert("2030-11-04".into(), vec!["x".into()]);
			config.reminders.enabled = true;
			config.reminders.travel.sound = crate::reminders::Sound::Silent;
			config.travel.modes.push(crate::places::RouteMode {
				from: 1,
				to: 2,
				mode: crate::places::TravelMode::Car,
			});
			config.alarm.sound.volume = 25;
			config.notes.directory = "~/Notes".into();
			config.clone()
		})
		.unwrap();
		let loaded: Loaded<Config> = load(&path, |_| {}).unwrap();
		assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
		assert_eq!(loaded.value, wanted);
		assert_eq!(count(&path, "sleep_schedule"), 1);
		assert_eq!(count(&path, "sleep_schedule_changes"), 1);
		assert_eq!(count(&path, "calendar_sources"), 4);
		assert_eq!(count(&path, "reminders"), 1);
		assert_eq!(count(&path, "device_settings"), 1);
		assert_eq!(
			device_value(&path, "notes.directory").unwrap(),
			Some(Value::from("~/Notes"))
		);
	}

	#[test]
	fn a_damaged_setting_falls_back_alone_and_is_not_overwritten() {
		let (_directory, path) = database();
		update(&path, |config: &mut Config| {
			config.calendar.day_end_hour = 3
		})
		.unwrap();
		{
			let mut db = Database::open(&path).unwrap();
			// JSON that does not parse can only come from outside the apps:
			// the journal's triggers refuse it.
			let connection = db.connection();
			assert!(
				connection
					.execute(
						"INSERT INTO settings VALUES('calendar.weekly_days','not json')",
						[]
					)
					.is_err()
			);
			connection
				.execute_batch(
					"INSERT INTO settings VALUES('calendar.sync_minutes','\"often\"');
                     INSERT INTO settings VALUES('future.setting','1');
                     UPDATE timeparagon_sync_control SET projecting=1;
                     INSERT INTO settings VALUES('calendar.weekly_days','not json');
                     UPDATE timeparagon_sync_control SET projecting=0;",
				)
				.unwrap();
		}
		let mut ignored = Vec::new();
		let loaded: Loaded<Config> = load(&path, |field| ignored.push(field)).unwrap();
		assert_eq!(loaded.value.calendar.day_end_hour, 3);
		assert_eq!(loaded.value.calendar.sync_minutes, 60);
		assert_eq!(loaded.problems.len(), 1, "{:?}", loaded.problems);
		assert!(loaded.problems[0].starts_with("calendar.sync_minutes"));
		assert_eq!(ignored, ["future"]);

		update(&path, |config: &mut Config| {
			config.calendar.show_tasks = false
		})
		.unwrap();
		let mut db = Database::open(&path).unwrap();
		let kept: i64 = db
            .connection()
            .query_row(
                "SELECT count(*) FROM settings WHERE key IN ('calendar.sync_minutes','calendar.weekly_days','future.setting')",
                [],
                |row| row.get(0),
            )
            .unwrap();
		assert_eq!(kept, 3, "a save only writes what it changed");
	}

	#[test]
	fn reordering_a_list_kept_as_a_set_is_stored_the_same() {
		let mut a = Config::default();
		a.calendar.disabled = vec!["x".into(), "y".into()];
		let mut b = a.clone();
		b.calendar.disabled.reverse();
		assert!(same_stored(&a, &b));
		b.calendar.disabled.pop();
		assert!(!same_stored(&a, &b));
		let mut c = a.clone();
		c.notepad.text = "changed".into();
		assert!(!same_stored(&a, &c));
	}
}
