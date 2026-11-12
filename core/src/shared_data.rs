//! The notepad's text, brain dumps, projects, work sessions and routines: data the
//! user writes, kept as documents in the local database and shared between
//! devices through Turso by [`crate::database_sync`].
//!
//! [`crate::settings`] lays these documents into the settings tree when it is
//! read, and stores a changed section back here when it is saved.
//!
//! Each section is one document. A section with nothing in it is not stored at
//! all, so a fresh device holds nothing that could replace another device's
//! data when the two are synced.

use std::io;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::task_db::Database;

/// The sections, by their names in the settings tree.
pub const SECTIONS: [&str; 5] = ["notepad", "brain_dump", "projects", "task_work", "routines"];

/// The time the shared data last changed, as milliseconds since the epoch.
const MODIFIED: &str = "shared_data_modified_ms";

/// The key each section's document is filed under.
fn key(section: &str) -> String {
	format!("shared.{section}")
}

fn other(error: impl std::fmt::Display) -> io::Error {
	io::Error::other(error.to_string())
}

/// Whether a value holds nothing worth keeping, including default zero values.
fn blank(value: &Value) -> bool {
	match value {
		Value::Null | Value::Bool(false) => true,
		Value::String(text) => text.is_empty(),
		Value::Number(number) => number.as_f64() == Some(0.0),
		Value::Array(items) => items.iter().all(blank),
		Value::Object(fields) => fields.values().all(blank),
		Value::Bool(true) => false,
	}
}

/// A section as it is stored, taken from a settings tree. The notepad's table
/// also holds its font size, a device setting, so only its text is taken.
pub(crate) fn section(tree: &Map<String, Value>, section: &str) -> Option<Value> {
	if section == "notepad" {
		let text = tree.get("notepad")?.get("text")?.clone();
		let mut fields = Map::new();
		fields.insert("text".to_owned(), text);
		return Some(Value::Object(fields));
	}
	tree.get(section).cloned()
}

fn put(tree: &mut Map<String, Value>, section: &str, value: Value) {
	let Value::Object(value) = value else {
		return;
	};
	let entry = tree
		.entry(section)
		.or_insert_with(|| Value::Object(Map::new()));
	match entry {
		// Keeps the notepad's font size beside its text.
		Value::Object(existing) if section == "notepad" => existing.extend(value),
		_ => *entry = Value::Object(value),
	}
}

fn document(connection: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
	connection
		.query_row("SELECT value FROM documents WHERE key=?1", [key], |row| {
			row.get(0)
		})
		.optional()
}

fn now_ms() -> i64 {
	crate::clock::now().timestamp_millis()
}

/// Puts the saved sections into a settings tree. A section the database does
/// not have is left as the tree has it.
pub(crate) fn overlay(
	connection: &Connection,
	path: &Path,
	tree: &mut Map<String, Value>,
) -> io::Result<()> {
	for section in SECTIONS {
		if let Some(text) = document(connection, &key(section)).map_err(other)? {
			let value: Value = serde_json::from_str(&text)
				.map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
			crate::database_sync::projection::remember(path, &key(section), Some(text));
			put(tree, section, value);
		}
	}
	Ok(())
}

/// What was saved, to be remembered as read once the transaction commits.
#[must_use]
pub(crate) struct Stored(Vec<(String, Option<String>)>);

impl Stored {
	pub(crate) fn remember(self, path: &Path) {
		for (key, value) in self.0 {
			crate::database_sync::projection::remember(path, &key, value);
		}
	}
}

/// Saves `sections` as `tree` has them, inside the caller's transaction. One
/// missing from the tree, or holding nothing, is saved as empty.
pub(crate) fn store_in(
	tx: &Connection,
	path: &Path,
	tree: &Map<String, Value>,
	sections: &[&str],
) -> io::Result<Stored> {
	let values: Vec<(String, Option<String>)> = sections
		.iter()
		.map(|name| {
			let value = section(tree, name).filter(|value| !blank(value));
			(key(name), value.map(|value| value.to_string()))
		})
		.collect();
	let mut changed = false;
	for (key, desired) in &values {
		let value = crate::database_sync::projection::prepare(path, tx, key, desired)?;
		if document(tx, key).map_err(other)? == value {
			continue;
		}
		changed = true;
		match value {
            Some(value) => tx.execute(
                "INSERT INTO documents (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            ),
            None => tx.execute("DELETE FROM documents WHERE key=?1", [key]),
        }
        .map_err(other)?;
	}
	if changed {
		tx.execute(
            "INSERT INTO documents (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![MODIFIED, now_ms().to_string()],
        )
        .map_err(other)?;
	}
	Ok(Stored(values))
}

/// Saves one section, given as it appears in the settings.
pub fn save_section(database: &Path, section: &str, value: &impl Serialize) -> io::Result<()> {
	let value =
		serde_json::to_value(value).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
	let mut tree = Map::new();
	tree.insert(section.to_owned(), value);
	let mut db = Database::open(database).map_err(other)?;
	let tx = db
		.connection()
		.transaction_with_behavior(TransactionBehavior::Immediate)
		.map_err(other)?;
	let stored = store_in(&tx, database, &tree, &[section])?;
	tx.commit().map_err(other)?;
	stored.remember(database);
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn database() -> (tempfile::TempDir, std::path::PathBuf) {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("test.sqlite3");
		(directory, path)
	}

	/// Inspect migrated sections and their timestamp without a sync transport.
	fn snapshot(path: &Path) -> io::Result<(String, i64)> {
		let mut db = Database::open(path).map_err(other)?;
		let connection = db.connection();
		let mut fields = std::collections::BTreeMap::new();
		for section in SECTIONS {
			if let Some(text) = document(connection, &key(section)).map_err(other)? {
				fields.insert(
					section,
					serde_json::from_str::<Value>(&text).map_err(other)?,
				);
			}
		}
		let modified = document(connection, MODIFIED)
			.map_err(other)?
			.and_then(|text| text.parse().ok())
			.unwrap_or(0);
		Ok((serde_json::to_string(&fields).map_err(other)?, modified))
	}

	#[test]
	fn only_the_notepad_text_is_a_document_and_it_comes_back_beside_the_font_size() {
		let (_directory, path) = database();
		save_section(
			&path,
			"notepad",
			&serde_json::json!({"text": "milk", "font_size": 120}),
		)
		.unwrap();
		let (text, _) = snapshot(&path).unwrap();
		assert_eq!(text, r#"{"notepad":{"text":"milk"}}"#);

		let mut tree = serde_json::json!({"notepad": {"font_size": 120}})
			.as_object()
			.unwrap()
			.clone();
		let mut db = Database::open(&path).unwrap();
		overlay(db.connection(), &path, &mut tree).unwrap();
		assert_eq!(tree["notepad"]["text"], "milk");
		assert_eq!(tree["notepad"]["font_size"], 120);
		assert!(tree.get("brain_dump").is_none());
	}

	#[test]
	fn an_empty_section_is_not_stored_and_only_changes_move_the_clock() {
		let (_directory, path) = database();
		save_section(&path, "notepad", &crate::config::NotepadText::default()).unwrap();
		assert_eq!(snapshot(&path).unwrap(), ("{}".to_owned(), 0));

		let notepad = crate::config::NotepadText { text: "a".into() };
		save_section(&path, "notepad", &notepad).unwrap();
		let (text, first) = snapshot(&path).unwrap();
		assert!(text.contains("\"notepad\""));
		assert!(first > 0);

		let mut database = Database::open(&path).unwrap();
		database
			.connection()
			.execute("UPDATE documents SET value='1' WHERE key=?1", [MODIFIED])
			.unwrap();
		save_section(&path, "notepad", &notepad).unwrap();
		assert_eq!(
			snapshot(&path).unwrap().1,
			1,
			"saving the same data is not a change"
		);
	}
}
