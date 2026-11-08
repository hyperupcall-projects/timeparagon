//! Cheap change detection for the local database.
//!
//! The apps poll the database every couple of seconds for edits made by other
//! TimeSlay processes, the Turso worker and the Google queue. Reading and
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
/// Bumped by changes to the queue of edits waiting for Google, including a
/// retry being scheduled, so the queue's status stays current.
const QUEUE: &str = "google.queue";

/// The counters a poll compares against what it last applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counters {
    pub habits: i64,
    pub notes: i64,
    pub overlay: i64,
    pub queue: i64,
}

/// Creates the counter table and the triggers that maintain it. Idempotent.
pub(crate) fn install(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS timeslay_change_counters(name TEXT PRIMARY KEY, value INTEGER NOT NULL);",
    )?;
    // Neither UPSERT nor `OR IGNORE` can be relied on in a trigger body: the
    // outer statement's conflict policy replaces the trigger's own, so the
    // row is created with an explicit existence check instead.
    let bump = |name: &str| {
        format!(
            "INSERT INTO timeslay_change_counters SELECT {name},0 WHERE NOT EXISTS (SELECT 1 FROM timeslay_change_counters WHERE name={name}); \
             UPDATE timeslay_change_counters SET value=value+1 WHERE name={name};"
        )
    };
    for (operation, key) in [
        ("INSERT", "NEW.key"),
        ("UPDATE", "NEW.key"),
        ("DELETE", "OLD.key"),
    ] {
        let name = format!("'doc:'||{key}");
        db.execute_batch(&format!(
            "CREATE TRIGGER IF NOT EXISTS timeslay_change_documents_{op} AFTER {operation} ON documents BEGIN {body} END;",
            op = operation.to_ascii_lowercase(),
            body = bump(&name),
        ))?;
    }
    for (table, counter) in [
        ("google_entities", OVERLAY),
        ("google_id_map", OVERLAY),
        ("google_operations", QUEUE),
    ] {
        for operation in ["INSERT", "UPDATE", "DELETE"] {
            db.execute_batch(&format!(
                "CREATE TRIGGER IF NOT EXISTS timeslay_change_{table}_{op} AFTER {operation} ON {table} BEGIN {body} END;",
                op = operation.to_ascii_lowercase(),
                body = bump(&format!("'{counter}'")),
            ))?;
        }
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
                "SELECT value FROM timeslay_change_counters WHERE name=?1",
                [name],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0))
    };
    Ok(Counters {
        habits: value(&document(crate::task_db::keys::HABITS))?,
        notes: value(&document(crate::task_db::keys::NOTES))?,
        overlay: value(OVERLAY)?,
        queue: value(QUEUE)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_watched_change_moves_only_its_own_counter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timeslay.sqlite3");
        let start = read(&path).unwrap();

        crate::task_db::write_document_json(&path, crate::task_db::keys::HABITS, "{}").unwrap();
        let after_habits = read(&path).unwrap();
        assert!(after_habits.habits > start.habits);
        assert_eq!(after_habits.notes, start.notes);
        assert_eq!(after_habits.overlay, start.overlay);

        crate::task_db::write_document_json(&path, crate::task_db::keys::NOTES, "{}").unwrap();
        let after_notes = read(&path).unwrap();
        assert!(after_notes.notes > after_habits.notes);
        assert_eq!(after_notes.habits, after_habits.habits);

        // Unrelated documents do not wake the habits or notes readers.
        crate::task_db::write_document_json(&path, "routes", "[]").unwrap();
        let after_routes = read(&path).unwrap();
        assert_eq!(after_routes.habits, after_notes.habits);
        assert_eq!(after_routes.notes, after_notes.notes);

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
        assert_eq!(after_map.queue, after_routes.queue);

        database
            .connection()
            .execute(
                "INSERT INTO google_operations(kind,payload,entity,created_at) VALUES('k','{}','e','now')",
                [],
            )
            .unwrap();
        assert!(read(&path).unwrap().queue > after_map.queue);
    }
}
