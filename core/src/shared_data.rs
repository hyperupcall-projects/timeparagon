//! The notepad's text, brain dumps, projects and work sessions: data the
//! user writes, kept in the local database and shared between devices through
//! Turso by [`crate::database_sync`].
//!
//! Loading overlays these documents into the in-memory configuration. Saving
//! stores them separately from device settings.
//!
//! Each section is one document. A section with nothing in it is not stored at
//! all, so a fresh device holds nothing that could replace another device's
//! data when the two are synced.

use std::io;
use std::path::Path;

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde_json::Value;

use crate::task_db::Database;

/// The sections, by their names in the config.
pub const SECTIONS: [&str; 4] = ["notepad", "brain_dump", "projects", "task_work"];

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

fn toml_to_json(value: toml::Value) -> Value {
    match value {
        toml::Value::String(text) => Value::String(text),
        toml::Value::Integer(number) => number.into(),
        toml::Value::Float(number) => serde_json::Number::from_f64(number)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        toml::Value::Boolean(flag) => Value::Bool(flag),
        toml::Value::Datetime(time) => Value::String(time.to_string()),
        toml::Value::Array(items) => Value::Array(items.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(table) => Value::Object(
            table
                .into_iter()
                .map(|(name, value)| (name, toml_to_json(value)))
                .collect(),
        ),
    }
}

/// TOML has no null, so a null field is left out, as serializing to TOML does.
fn json_to_toml(value: Value) -> Option<toml::Value> {
    Some(match value {
        Value::Null => return None,
        Value::Bool(flag) => toml::Value::Boolean(flag),
        Value::Number(number) => match number.as_i64() {
            Some(integer) => toml::Value::Integer(integer),
            None => toml::Value::Float(number.as_f64()?),
        },
        Value::String(text) => toml::Value::String(text),
        Value::Array(items) => {
            toml::Value::Array(items.into_iter().filter_map(json_to_toml).collect())
        }
        Value::Object(fields) => toml::Value::Table(
            fields
                .into_iter()
                .filter_map(|(name, value)| Some((name, json_to_toml(value)?)))
                .collect(),
        ),
    })
}

/// Takes a section out of a config's table. The notepad's table also holds its
/// font size, a setting, so only its text is taken.
fn take(table: &mut toml::Table, section: &str) -> Option<Value> {
    if section == "notepad" {
        let notepad = table.get_mut("notepad")?.as_table_mut()?;
        let text = notepad.remove("text")?;
        if notepad.is_empty() {
            table.remove("notepad");
        }
        let mut fields = toml::Table::new();
        fields.insert("text".to_owned(), text);
        return Some(toml_to_json(toml::Value::Table(fields)));
    }
    table.remove(section).map(toml_to_json)
}

fn put(table: &mut toml::Table, section: &str, value: Value) {
    let Some(toml::Value::Table(value)) = json_to_toml(value) else {
        return;
    };
    let entry = table
        .entry(section)
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    match entry {
        // Keeps the notepad's font size beside its text.
        toml::Value::Table(existing) if section == "notepad" => existing.extend(value),
        _ => *entry = toml::Value::Table(value),
    }
}

fn document(connection: &rusqlite::Connection, key: &str) -> rusqlite::Result<Option<String>> {
    connection
        .query_row("SELECT value FROM documents WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .optional()
}

fn now_ms() -> i64 {
    crate::clock::now().timestamp_millis()
}

/// Puts the saved sections into a config's table, ready to be parsed. A section
/// the database does not have is left as the table has it.
pub fn overlay(database: &Path, table: &mut toml::Table) -> io::Result<()> {
    let path = database;
    let mut database = Database::open(database).map_err(other)?;
    let connection = database.connection();
    for section in SECTIONS {
        if let Some(text) = document(connection, &key(section)).map_err(other)? {
            let value: Value = serde_json::from_str(&text)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            crate::database_sync::projection::remember(path, &key(section), Some(text));
            put(table, section, value);
        }
    }
    Ok(())
}

/// Takes `sections` out of a config's table and saves them. One missing from
/// the table, or holding nothing, is saved as empty.
pub fn store(database: &Path, table: &mut toml::Table, sections: &[&str]) -> io::Result<()> {
    let values: Vec<(String, Option<String>)> = sections
        .iter()
        .map(|section| {
            let value = take(table, section).filter(|value| !blank(value));
            (key(section), value.map(|value| value.to_string()))
        })
        .collect();
    let path = database;
    let mut database = Database::open(database).map_err(other)?;
    let tx = database
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(other)?;
    let mut changed = false;
    for (key, desired) in &values {
        let value = crate::database_sync::projection::prepare(path, &tx, key, desired)?;
        if document(&tx, key).map_err(other)? == value {
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
    tx.commit().map_err(other)?;
    for (key, value) in values {
        crate::database_sync::projection::remember(path, &key, value);
    }
    Ok(())
}

/// Saves one section, given as it appears in the config.
pub fn save_section(database: &Path, section: &str, value: &impl Serialize) -> io::Result<()> {
    let value =
        toml::Value::try_from(value).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut table = toml::Table::new();
    table.insert(section.to_owned(), value);
    store(database, &mut table, &[section])
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

    fn table(text: &str) -> toml::Table {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn sections_leave_the_table_on_save_and_come_back_on_load() {
        let (_directory, path) = database();
        let mut saved =
            table("[notepad]\ntext = \"milk\"\nfont_size = 120\n\n[window]\nwidth = 5\n");
        store(&path, &mut saved, &SECTIONS).unwrap();
        assert_eq!(
            saved,
            table("[notepad]\nfont_size = 120\n\n[window]\nwidth = 5\n")
        );

        overlay(&path, &mut saved).unwrap();
        assert_eq!(saved["notepad"]["text"].as_str(), Some("milk"));
        assert_eq!(saved["notepad"]["font_size"].as_integer(), Some(120));
        assert!(saved.get("brain_dump").is_none());
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
