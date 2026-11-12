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
	let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)?;
	for (old, new) in SOURCE_IDS {
		// Settings already made under the new ID win.
		tx.execute(
			"DELETE FROM calendar_sources WHERE id=?1 AND EXISTS (SELECT 1 FROM calendar_sources WHERE id=?2)",
			params![old, new],
		)?;
		tx.execute(
			"UPDATE calendar_sources SET id=?2 WHERE id=?1",
			params![old, new],
		)?;
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
			"CREATE TABLE calendar_sources (id TEXT PRIMARY KEY, hidden INTEGER NOT NULL DEFAULT 0, position INTEGER, color TEXT);
			 INSERT INTO calendar_sources VALUES('timeslay:habits', 1, 2, '#123456');
			 INSERT INTO calendar_sources VALUES('timeslay:sleep', 1, 3, NULL);
			 INSERT INTO calendar_sources VALUES('timeparagon:sleep', 0, 4, NULL);
			 INSERT INTO calendar_sources VALUES('work', 0, 1, NULL);",
		)
		.unwrap();
		rename_source_ids(&db).unwrap();
		let rows: Vec<(String, i64, i64)> = db
			.prepare("SELECT id, hidden, position FROM calendar_sources ORDER BY position")
			.unwrap()
			.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
			.unwrap()
			.collect::<Result<_, _>>()
			.unwrap();
		assert_eq!(
			rows,
			[
				("work".to_owned(), 0, 1),
				("timeparagon:habits".to_owned(), 1, 2),
				("timeparagon:sleep".to_owned(), 0, 4),
			]
		);
	}
}
