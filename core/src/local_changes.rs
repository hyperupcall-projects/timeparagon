//! Cheap change detection for the local database.
//!
//! The apps poll the database every couple of seconds for edits made by other
//! TimeParagon processes, the Turso worker and writes to Google. Reading and
//! parsing every document on every poll is what made that poll expensive, so
//! triggers bump a counter whenever a watched table changes, and a poll reads
//! the counters first. Only the parts whose counter moved are read again.
//!
//! The counters live in the database rather than in memory, so a change made
//! by any process (or by an older build without these triggers' knowledge,
//! since the triggers belong to the file) is seen by every other process.

use rusqlite::{Connection, OptionalExtension};
use std::path::Path;

/// Counter for one document of the `documents` table.
fn document(key: &str) -> String {
	format!("doc:{key}")
}

/// Bumped by changes to what [`crate::calendar::operations::load_overlay`]
/// reads: the local snapshots of edited Google items and the local-to-remote
/// ID map.
const OVERLAY: &str = "google.overlay";

/// Bumped by changes to any settings table (see [`crate::settings`]).
const SETTINGS: &str = "settings";

/// Bumped by changes to the stored nights of sleep, including those database
/// sync brings from another device (see `calendar::sleep::store`).
const SLEEP: &str = "sleep";

/// The counters a poll compares against what it last applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counters {
	pub habits: i64,
	pub alarms: i64,
	pub overlay: i64,
	/// Moves whenever anything the apps' `Config` holds changes: a settings
	/// table, or one of the shared documents laid into it.
	pub config: i64,
	/// Moves whenever a stored night of sleep changes.
	pub sleep: i64,
	/// Moves whenever note metadata changes, including changes from sync.
	pub notes: i64,
}

const CREATE: &str = "CREATE TABLE IF NOT EXISTS timeparagon_change_counters(name TEXT PRIMARY KEY, value INTEGER NOT NULL);";

/// A trigger body that adds one to the counter `name` (an SQL expression).
fn bump(name: &str) -> String {
	// Neither UPSERT nor `OR IGNORE` can be relied on in a trigger body: the
	// outer statement's conflict policy replaces the trigger's own, so the
	// row is created with an explicit existence check instead.
	format!(
		"INSERT INTO timeparagon_change_counters SELECT {name},0 WHERE NOT EXISTS (SELECT 1 FROM timeparagon_change_counters WHERE name={name}); \
         UPDATE timeparagon_change_counters SET value=value+1 WHERE name={name};"
	)
}

/// Makes every insert, update and delete on `table` bump `counter`.
fn watch(db: &Connection, table: &str, counter: &str) -> rusqlite::Result<()> {
	for operation in ["INSERT", "UPDATE", "DELETE"] {
		db.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS timeparagon_change_{table}_{op} AFTER {operation} ON {table} BEGIN {body} END;",
            op = operation.to_ascii_lowercase(),
            body = bump(&format!("'{counter}'")),
        ))?;
	}
	Ok(())
}

/// The triggers on the synced data tables, installed during versioned setup
/// and sync enrollment (see `settings::schema::create_data`).
/// Idempotent, and expects the tables to exist.
pub(crate) fn install_data(db: &Connection) -> rusqlite::Result<()> {
	db.execute_batch(CREATE)?;
	watch(db, crate::settings::schema::SLEEP_NIGHTS.name, SLEEP)?;
	watch(db, crate::settings::schema::NOTES.name, "notes")
}

/// Creates the counter table and the triggers that maintain it. Idempotent.
pub(crate) fn install(db: &Connection) -> rusqlite::Result<()> {
	db.execute_batch(CREATE)?;
	for (operation, key) in [
		("INSERT", "NEW.key"),
		("UPDATE", "NEW.key"),
		("DELETE", "OLD.key"),
	] {
		let name = format!("'doc:'||{key}");
		db.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS timeparagon_change_documents_{op} AFTER {operation} ON documents BEGIN {body} END;",
            op = operation.to_ascii_lowercase(),
            body = bump(&name),
        ))?;
	}
	let settings = crate::settings::schema::TABLES.map(|table| (table.name, SETTINGS));
	for (table, counter) in [("google_entities", OVERLAY), ("google_id_map", OVERLAY)]
		.into_iter()
		.chain(settings)
	{
		watch(db, table, counter)?;
	}
	Ok(())
}

/// Reads every counter in one query. A counter that has never been bumped
/// reads as zero.
pub fn read(path: &Path) -> Result<Counters, String> {
	let mut database = crate::task_db::Database::open(path)?;
	read_from(database.connection()).map_err(|e| e.to_string())
}

pub(crate) fn read_from(db: &Connection) -> rusqlite::Result<Counters> {
	let value = |name: &str| -> rusqlite::Result<i64> {
		Ok(db
			.query_row(
				"SELECT value FROM timeparagon_change_counters WHERE name=?1",
				[name],
				|row| row.get(0),
			)
			.optional()?
			.unwrap_or(0))
	};
	let mut config = value(SETTINGS)?;
	for section in crate::shared_data::SECTIONS {
		config += value(&document(&format!("shared.{section}")))?;
	}
	Ok(Counters {
		alarms: value(&document(crate::task_db::keys::ALARMS))?,
		habits: value(&document(crate::task_db::keys::HABITS))?,
		// Settling a series edit unlocks the occurrences it locked, which
		// only its record in the documents tells.
		overlay: value(OVERLAY)? + value(&document(crate::calendar::operations::SERIES_EDITS))?,
		config,
		sleep: value(SLEEP)?,
		notes: value("notes")?,
	})
}

/// Reads the counters again and again over one connection, for a watcher
/// that looks often.
pub struct Reader(crate::task_db::Database);

impl Reader {
	pub fn open(path: &Path) -> Result<Self, String> {
		crate::task_db::Database::open(path).map(Self)
	}

	pub fn read(&mut self) -> Result<Counters, String> {
		read_from(self.0.connection()).map_err(|e| e.to_string())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn each_watched_change_moves_only_its_own_counter() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("timeparagon.sqlite3");
		let start = read(&path).unwrap();

		crate::task_db::write_document_json(&path, crate::task_db::keys::HABITS, "{}").unwrap();
		let after_habits = read(&path).unwrap();
		assert!(after_habits.habits > start.habits);
		assert_eq!(after_habits.overlay, start.overlay);

		// Unrelated documents do not wake the habits reader.
		crate::task_db::write_document_json(&path, "routes", "[]").unwrap();
		let after_routes = read(&path).unwrap();
		assert_eq!(after_routes.habits, after_habits.habits);

		let mut database = crate::task_db::Database::open(&path).unwrap();
		database
			.connection()
			.execute(
				"INSERT INTO google_id_map(local,remote,calendar) VALUES('a','b','c')",
				[],
			)
			.unwrap();
		let after_map = read(&path).unwrap();
		assert!(after_map.overlay > after_routes.overlay);
		assert_eq!(after_map.habits, after_routes.habits);
		assert_eq!(after_map.config, after_routes.config);

		// Settings, and the documents the config holds, move the config's.
		let mut reader = Reader::open(&path).unwrap();
		crate::settings::update(&path, |config: &mut crate::config::Config| {
			config.calendar.sync_minutes = 5;
		})
		.unwrap();
		let after_setting = reader.read().unwrap();
		assert!(after_setting.config > after_map.config);
		assert_eq!(after_setting.overlay, after_map.overlay);
		let notepad = crate::config::NotepadText {
			text: "milk".into(),
		};
		crate::shared_data::save_section(&path, "notepad", &notepad).unwrap();
		let after_notepad = reader.read().unwrap();
		assert!(after_notepad.config > after_setting.config);
		assert_eq!(after_notepad.sleep, after_setting.sleep);

		// Stored nights, written here or brought by sync, move only theirs.
		let mut database = crate::task_db::Database::open(&path).unwrap();
		crate::settings::schema::create_data(database.connection()).unwrap();
		database
			.connection()
			.execute(
				"INSERT INTO sleep_nights VALUES('app','n','2030-11-04','a','b','{}')",
				[],
			)
			.unwrap();
		let after_night = reader.read().unwrap();
		assert!(after_night.sleep > after_notepad.sleep);
		assert_eq!(
			(after_night.config, after_night.overlay, after_night.habits),
			(
				after_notepad.config,
				after_notepad.overlay,
				after_notepad.habits
			)
		);
	}
}
