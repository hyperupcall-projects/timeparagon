//! Moving stored state off the project's old name, TimeSlay.
//!
//! The directories and the database file are moved by
//! `config::migrate_legacy_dirs`. This renames what lives inside the
//! database (the tables and triggers prefixed `timeslay_`, and the built-in
//! calendar sources' `timeslay:` IDs) and the sync server's table.
use rusqlite::{Connection, params};

/// The prefix of the tables, indexes and triggers the app created.
const OLD_PREFIX: &str = "timeslay_";
const NEW_PREFIX: &str = "timeparagon_";

/// The sync server's table under the old name.
pub(crate) const OLD_REVISIONS_TABLE: &str = "timeslay_revisions_v1";

/// Renames the app's own tables. Their triggers and indexes are dropped
/// rather than renamed: the callers recreate them under the new names right
/// after, and a renamed trigger would be a duplicate of the new one.
///
/// Run inside the transaction that then creates the schema, so a second
/// process either finds nothing to do or waits. Idempotent.
pub(crate) fn rename_schema(tx: &Connection) -> rusqlite::Result<()> {
	let objects = tx
		.prepare(
			"SELECT type, name FROM sqlite_master
			 WHERE type IN ('table', 'index', 'trigger') AND name LIKE 'timeslay\\_%' ESCAPE '\\'
			 ORDER BY type = 'table'",
		)?
		.query_map([], |row| {
			Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
		})?
		.collect::<rusqlite::Result<Vec<_>>>()?;
	for (kind, name) in objects {
		match kind.as_str() {
			"trigger" => tx.execute_batch(&format!("DROP TRIGGER IF EXISTS \"{name}\";"))?,
			"index" => tx.execute_batch(&format!("DROP INDEX IF EXISTS \"{name}\";"))?,
			_ => {
				let renamed = format!("{NEW_PREFIX}{}", &name[OLD_PREFIX.len()..]);
				tx.execute_batch(&format!("ALTER TABLE \"{name}\" RENAME TO \"{renamed}\";"))?;
				log::info!("renamed table {name} to {renamed}");
			}
		}
	}
	Ok(())
}

/// The built-in calendar sources, by old and new ID.
const SOURCE_IDS: [(&str, &str); 4] = [
	(
		"timeslay:tasks",
		crate::calendar::sources::TimeParagonSource::Tasks.id(),
	),
	(
		"timeslay:time-blocks",
		crate::calendar::sources::TimeParagonSource::TimeBlocks.id(),
	),
	(
		"timeslay:habits",
		crate::calendar::sources::TimeParagonSource::Habits.id(),
	),
	("timeslay:sleep", crate::calendar::sleep::SOURCE_ID),
];

/// Moves each built-in source's settings (hidden, position and color) to its
/// new ID. Runs after sync enrollment, so other devices receive the change.
/// Idempotent.
pub(crate) fn rename_source_ids(db: &Connection) -> rusqlite::Result<()> {
	use crate::settings::mapping::SOURCES;
	let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)?;
	for (old, new) in SOURCE_IDS {
		let (old, new) = (format!("{SOURCES}/{old}"), format!("{SOURCES}/{new}"));
		// Settings already made under the new ID win.
		tx.execute(
			"DELETE FROM settings WHERE key=?1 AND EXISTS (SELECT 1 FROM settings WHERE key=?2)",
			params![old, new],
		)?;
		tx.execute("UPDATE settings SET key=?2 WHERE key=?1", params![old, new])?;
	}
	tx.commit()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn renames_tables_and_drops_their_triggers() {
		let db = Connection::open_in_memory().unwrap();
		db.execute_batch(
			"CREATE TABLE documents(key TEXT PRIMARY KEY, value TEXT);
			 CREATE TABLE timeslay_sync_edits(id TEXT PRIMARY KEY, clock INTEGER);
			 CREATE INDEX timeslay_sync_edits_clock ON timeslay_sync_edits(clock);
			 CREATE TRIGGER timeslay_sync_insert AFTER INSERT ON documents
			 BEGIN INSERT INTO timeslay_sync_edits VALUES(NEW.key, 1); END;
			 INSERT INTO timeslay_sync_edits VALUES('kept', 7);",
		)
		.unwrap();
		rename_schema(&db).unwrap();
		rename_schema(&db).unwrap();
		let names: Vec<String> = db
			.prepare("SELECT name FROM sqlite_master ORDER BY name")
			.unwrap()
			.query_map([], |row| row.get(0))
			.unwrap()
			.collect::<Result<_, _>>()
			.unwrap();
		assert!(
			names.iter().all(|name| !name.starts_with("timeslay")),
			"{names:?}"
		);
		assert!(names.contains(&"timeparagon_sync_edits".to_owned()));
		let clock: i64 = db
			.query_row(
				"SELECT clock FROM timeparagon_sync_edits WHERE id='kept'",
				[],
				|r| r.get(0),
			)
			.unwrap();
		assert_eq!(clock, 7);
	}

	#[test]
	fn moves_source_settings_to_the_new_ids() {
		let db = Connection::open_in_memory().unwrap();
		db.execute_batch(
			"CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
			 INSERT INTO settings VALUES('calendar.sources/timeslay:habits', '{\"hidden\":true,\"position\":2}');
			 INSERT INTO settings VALUES('calendar.sources/timeslay:sleep', '{\"hidden\":true,\"position\":3}');
			 INSERT INTO settings VALUES('calendar.sources/timeparagon:sleep', '{\"hidden\":false,\"position\":4}');
			 INSERT INTO settings VALUES('calendar.sources/work', '{\"hidden\":false,\"position\":1}');",
		)
		.unwrap();
		rename_source_ids(&db).unwrap();
		let rows: Vec<(String, String)> = db
			.prepare("SELECT key, value FROM settings ORDER BY json_extract(value,'$.position')")
			.unwrap()
			.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
			.unwrap()
			.collect::<Result<_, _>>()
			.unwrap();
		assert_eq!(
			rows,
			[
				(
					"calendar.sources/work".to_owned(),
					r#"{"hidden":false,"position":1}"#.to_owned()
				),
				(
					"calendar.sources/timeparagon:habits".to_owned(),
					r#"{"hidden":true,"position":2}"#.to_owned()
				),
				(
					"calendar.sources/timeparagon:sleep".to_owned(),
					r#"{"hidden":false,"position":4}"#.to_owned()
				),
			]
		);
	}
}
