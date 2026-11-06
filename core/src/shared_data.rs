//! Todos, the notepad's text, brain dumps, projects and work sessions: data the
//! user writes, kept in the local database and shared between devices through
//! Google Drive by [`crate::config_sync`].
//!
//! Older builds kept these as tables in `config.toml`. The configs still carry
//! them in memory, so the apps use them as before. Loading puts them into the
//! config's table before it is parsed, and saving takes them out again
//! before the rest is written to the file.
//!
//! Each section is one document. A section with nothing in it is not stored at
//! all, so a fresh device holds nothing that could replace another device's
//! data when the two are synced.

use std::fs;
use std::io;
use std::path::Path;

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde_json::Value;

use crate::task_db::Database;

/// The sections, by their names in the config.
pub const SECTIONS: [&str; 5] = ["todos", "notepad", "brain_dump", "projects", "task_work"];

/// The time the shared data last changed, as milliseconds since the epoch.
const MODIFIED: &str = "shared_data_modified_ms";

/// The key each section's document is filed under.
fn key(section: &str) -> String {
    format!("shared.{section}")
}

fn other(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

/// Whether a value holds nothing worth keeping. An empty todo list still says
/// which item is current, so a zero counts as nothing too.
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
        if document(&tx, &key).map_err(other)? == value {
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

/// Every section as one JSON object, with when it last changed: what is
/// synced. The keys are sorted, so the same data always reads the same.
pub(crate) fn bundle(database: &Path) -> io::Result<(String, i64)> {
    let mut database = Database::open(database).map_err(other)?;
    let connection = database.connection();
    let mut fields = serde_json::Map::new();
    for section in SECTIONS {
        if let Some(text) = document(connection, &key(section)).map_err(other)? {
            let value = serde_json::from_str(&text)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            fields.insert(section.to_owned(), value);
        }
    }
    let modified = document(connection, MODIFIED)
        .map_err(other)?
        .and_then(|text| text.parse().ok())
        .unwrap_or(0);
    let sorted: std::collections::BTreeMap<_, _> = fields.into_iter().collect();
    Ok((serde_json::to_string(&sorted).map_err(other)?, modified))
}

/// Replaces every section with those in `text`, a [`bundle`] from elsewhere.
pub(crate) fn replace(database: &Path, text: &str, modified_ms: i64) -> io::Result<()> {
    let mut fields: serde_json::Map<String, Value> = serde_json::from_str(text)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut database = Database::open(database).map_err(other)?;
    let tx = database
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(other)?;
    for section in SECTIONS {
        match fields.remove(section).filter(|value| !blank(value)) {
            Some(value) => tx.execute(
                "INSERT INTO documents (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key(section), value.to_string()],
            ),
            None => tx.execute("DELETE FROM documents WHERE key=?1", [key(section)]),
        }
        .map_err(other)?;
    }
    tx.execute(
        "INSERT INTO documents (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![MODIFIED, modified_ms.to_string()],
    )
    .map_err(other)?;
    tx.commit().map_err(other)
}

/// Moves sections that `config.toml` still holds into the database: those an
/// older build wrote, or that a device running one synced here. Where both have
/// a section and they differ, the newer wins, as in the sync: the file's
/// modification time against when the shared data last changed. A section with
/// nothing in it never replaces one with something. The file keeps its
/// modification time, so the config sync does not mistake the move for a newer
/// config.
pub(crate) fn adopt(config: &Path, database: &Path) -> io::Result<()> {
    let holds_sections = |text: &str| {
        toml::from_str::<toml::Table>(text).is_ok_and(|mut table| {
            SECTIONS
                .iter()
                .any(|section| take(&mut table, section).is_some())
        })
    };
    match fs::read_to_string(config) {
        Ok(text) if holds_sections(&text) => {}
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }

    crate::fsutil::locked(config, || {
        let text = fs::read_to_string(config)?;
        let Ok(mut table) = toml::from_str::<toml::Table>(&text) else {
            return Ok(());
        };
        let modified = fs::metadata(config)?.modified()?;
        let modified_ms = chrono::DateTime::<chrono::Utc>::from(modified).timestamp_millis();
        let found: Vec<(&str, Value)> = SECTIONS
            .iter()
            .filter_map(|section| Some((*section, take(&mut table, section)?)))
            .collect();

        let mut handle = Database::open(database).map_err(other)?;
        let tx = handle
            .connection()
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(other)?;
        let recorded: Option<i64> = document(&tx, MODIFIED)
            .map_err(other)?
            .and_then(|text| text.parse().ok());
        let file_is_newer = recorded.is_none_or(|recorded| modified_ms > recorded);
        let mut imported = false;
        for (section, value) in found {
            if blank(&value) {
                continue;
            }
            let text = value.to_string();
            match document(&tx, &key(section)).map_err(other)? {
                Some(existing) if existing == text => continue,
                Some(_) if !file_is_newer => {
                    log::warn!(
                        "config.toml has an older [{section}] than the database; keeping the database's"
                    );
                    continue;
                }
                _ => {}
            }
            tx.execute(
                "INSERT INTO documents (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key(section), text],
            )
            .map_err(other)?;
            imported = true;
        }
        if imported && file_is_newer {
            // As old as the file, so a newer copy elsewhere still wins the sync.
            tx.execute(
                "INSERT INTO documents (key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![MODIFIED, modified_ms.to_string()],
            )
            .map_err(other)?;
        }
        tx.commit().map_err(other)?;

        let stripped = toml::to_string_pretty(&table).map_err(other)?;
        crate::fsutil::write_atomic_with(
            config,
            stripped.as_bytes(),
            crate::fsutil::Kind::Private,
            |file| file.set_modified(modified),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.sqlite3");
        (directory, path)
    }

    fn table(text: &str) -> toml::Table {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn sections_leave_the_table_on_save_and_come_back_on_load() {
        let (_directory, path) = database();
        let mut saved = table(
            "[notepad]\ntext = \"milk\"\nfont_size = 120\n\n[todos]\ncurrent = 0\n\n[[todos.items]]\ntext = \"a\"\n\n[window]\nwidth = 5\n",
        );
        store(&path, &mut saved, &SECTIONS).unwrap();
        assert_eq!(
            saved,
            table("[notepad]\nfont_size = 120\n\n[window]\nwidth = 5\n")
        );

        overlay(&path, &mut saved).unwrap();
        assert_eq!(saved["notepad"]["text"].as_str(), Some("milk"));
        assert_eq!(saved["notepad"]["font_size"].as_integer(), Some(120));
        assert_eq!(saved["todos"]["items"][0]["text"].as_str(), Some("a"));
        assert!(saved.get("brain_dump").is_none());
    }

    #[test]
    fn an_empty_section_is_not_stored_and_only_changes_move_the_clock() {
        let (_directory, path) = database();
        save_section(&path, "todos", &crate::todos::TodoList::default()).unwrap();
        assert_eq!(bundle(&path).unwrap(), ("{}".to_owned(), 0));

        let mut todos = crate::todos::TodoList::default();
        todos.add("a");
        save_section(&path, "todos", &todos).unwrap();
        let (text, first) = bundle(&path).unwrap();
        assert!(text.contains("\"todos\""));
        assert!(first > 0);

        let mut database = Database::open(&path).unwrap();
        database
            .connection()
            .execute("UPDATE documents SET value='1' WHERE key=?1", [MODIFIED])
            .unwrap();
        save_section(&path, "todos", &todos).unwrap();
        assert_eq!(
            bundle(&path).unwrap().1,
            1,
            "saving the same data is not a change"
        );
    }

    #[test]
    fn a_bundle_replaces_every_section() {
        let (_directory, path) = database();
        save_section(
            &path,
            "brain_dump",
            &toml::toml! { entries = [{ date = "2026-11-05", text = "x" }] },
        )
        .unwrap();
        replace(
            &path,
            r#"{"projects":{"items":[{"id":1,"name":"Work"}]}}"#,
            42,
        )
        .unwrap();
        let (text, modified) = bundle(&path).unwrap();
        assert_eq!(text, r#"{"projects":{"items":[{"id":1,"name":"Work"}]}}"#);
        assert_eq!(modified, 42);
    }

    #[test]
    fn an_old_config_moves_into_the_database_and_keeps_its_time() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config.toml");
        let path = directory.path().join("test.sqlite3");
        fs::write(
            &config,
            "[calendar]\nsync_minutes = 5\n\n[notepad]\ntext = \"milk\"\nfont_size = 90\n\n[[projects.items]]\nid = 1\nname = \"Work\"\n",
        )
        .unwrap();
        let stamp = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        fs::File::options()
            .write(true)
            .open(&config)
            .unwrap()
            .set_modified(stamp)
            .unwrap();

        adopt(&config, &path).unwrap();

        let left = table(&fs::read_to_string(&config).unwrap());
        assert_eq!(
            left,
            table("[calendar]\nsync_minutes = 5\n\n[notepad]\nfont_size = 90\n")
        );
        assert_eq!(fs::metadata(&config).unwrap().modified().unwrap(), stamp);
        let (text, modified) = bundle(&path).unwrap();
        assert_eq!(
            text,
            r#"{"notepad":{"text":"milk"},"projects":{"items":[{"id":1,"name":"Work"}]}}"#
        );
        assert_eq!(modified, 1_700_000_000_000);
    }

    #[test]
    fn the_newer_of_an_old_config_and_the_database_wins() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config.toml");
        let path = directory.path().join("test.sqlite3");
        let stamp = |seconds| {
            fs::File::options()
                .write(true)
                .open(&config)
                .unwrap()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
                .unwrap();
        };
        replace(
            &path,
            r#"{"notepad":{"text":"database"}}"#,
            2_000_000_000_000,
        )
        .unwrap();

        fs::write(&config, "[notepad]\ntext = \"older\"\n").unwrap();
        stamp(1_000_000_000);
        adopt(&config, &path).unwrap();
        assert_eq!(
            bundle(&path).unwrap().0,
            r#"{"notepad":{"text":"database"}}"#
        );
        assert!(!fs::read_to_string(&config).unwrap().contains("older"));

        fs::write(
            &config,
            "[notepad]\ntext = \"newer\"\n[projects]\nitems = []\n",
        )
        .unwrap();
        stamp(3_000_000_000);
        adopt(&config, &path).unwrap();
        assert_eq!(
            bundle(&path).unwrap(),
            (
                r#"{"notepad":{"text":"newer"}}"#.to_owned(),
                3_000_000_000_000
            )
        );
    }
}
