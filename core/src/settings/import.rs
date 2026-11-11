//! The one-time move of `config.toml` into the database.
//!
//! Each device imports its own file once, the first time this version opens
//! its database, and then renames it `config.toml.imported` so that it is
//! plain it is no longer read.
//!
//! Two devices that both had a file both import it, and where the files
//! disagree on a shared setting only one value can win. The imported rows
//! are therefore journaled as the oldest edits there can be (clock 1): a
//! setting changed in the app on any device afterwards always wins over a
//! file, and where only two files disagree, every device settles on the same
//! one of them. Values a file held at their default are not imported at all,
//! so they never override another device's choice. Places, reminders and the
//! other tables merge row by row, so both devices' places survive.

use std::fs;
use std::io;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{Map, Value};

use super::{decode, mapping, other, tree_of};
use crate::task_db::Database;

/// The document that records when this database imported a settings file.
/// Present even when there was no file, so nothing is imported later.
const MARKER: &str = "settings_imported_at";

fn toml_to_json(value: toml::Value) -> Value {
    match value {
        toml::Value::String(text) => Value::String(text),
        toml::Value::Integer(number) => number.into(),
        // NaN and infinity have no JSON form; they become the default.
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

/// The file's settings as a tree. The sections both apps know are read the
/// way the apps read them, so a partial table gets the same defaults it
/// always had. The desktop's own sections are kept as the file has them; the
/// desktop reads them, and reports anything it cannot use, when it loads.
fn normalize(raw: &Value) -> Value {
    let known =
        tree_of(&decode::<crate::config::Config>(raw.clone(), |_| {}).value).unwrap_or(Value::Null);
    let mut tree = raw.as_object().cloned().unwrap_or_default();
    for (section, fields) in known.as_object().into_iter().flatten() {
        // Documents come from the file as it is: the defaults a section the
        // file never had would get must not be stored as if they were data.
        if crate::shared_data::SECTIONS.contains(&section.as_str()) {
            continue;
        }
        match (tree.get_mut(section), fields) {
            (Some(Value::Object(raw)), Value::Object(fields)) => {
                for (name, value) in fields {
                    raw.insert(name.clone(), value.clone());
                }
            }
            _ => {
                tree.insert(section.clone(), fields.clone());
            }
        }
    }
    Value::Object(tree)
}

fn document(db: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    db.query_row("SELECT value FROM documents WHERE key=?1", [key], |row| {
        row.get(0)
    })
    .optional()
}

/// Stores the settings of `table`, inside the caller's transaction.
fn store(
    tx: &Connection,
    path: &Path,
    table: toml::Table,
) -> io::Result<crate::shared_data::Stored> {
    let raw = toml_to_json(toml::Value::Table(table));
    let tree = normalize(&raw);
    let defaults = mapping::rows(&tree_of(&crate::config::Config::default())?);
    let rows = mapping::without_defaults(mapping::rows(&tree), &defaults);

    let journal = tx
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='timeslay_sync_edits'",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(other)?
        .is_some();
    let journaled_before: i64 = if journal {
        tx.query_row(
            "SELECT coalesce(max(rowid),0) FROM timeslay_sync_edits",
            [],
            |row| row.get(0),
        )
        .map_err(other)?
    } else {
        0
    };

    for ((table, _), row) in &rows {
        let table = super::schema::table(table).expect("rows are only made for settings tables");
        table.upsert(tx, row).map_err(other)?;
    }

    // A document already in the database is newer than the file's copy: the
    // apps have stored these in the database, not the file, for a while.
    let empty = Map::new();
    let file = raw.as_object().unwrap_or(&empty);
    let mut missing = Vec::new();
    for section in crate::shared_data::SECTIONS {
        if crate::shared_data::section(file, section).is_some()
            && document(tx, &format!("shared.{section}"))
                .map_err(other)?
                .is_none()
        {
            missing.push(section);
        }
    }
    let stored = crate::shared_data::store_in(tx, path, file, &missing)?;

    if journal {
        tx.execute(
            "UPDATE timeslay_sync_edits SET clock=1 WHERE rowid>?1",
            [journaled_before],
        )
        .map_err(other)?;
    }
    Ok(stored)
}

/// Imports the settings file at `file` into the database `db` (at `path`),
/// unless this database has imported one already. Returns whether it did.
///
/// A file that cannot be parsed is left where it is, and not imported: its
/// settings are reported in the log and start from their defaults.
pub(crate) fn import_once(db: &Connection, path: &Path, file: &Path) -> io::Result<bool> {
    let tx = Transaction::new_unchecked(db, TransactionBehavior::Immediate).map_err(other)?;
    if document(&tx, MARKER).map_err(other)?.is_some() {
        return Ok(false);
    }
    let mut stored = None;
    match fs::read_to_string(file) {
        Ok(text) => match toml::from_str::<toml::Table>(&text) {
            Ok(table) => stored = Some(store(&tx, path, table)?),
            Err(error) => log::error!(
                "{} could not be parsed, so its settings were not imported: {error}",
                file.display()
            ),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        // Perhaps temporary: try again next time.
        Err(error) => return Err(error),
    }
    let now = serde_json::to_string(&crate::clock::utc_now().to_rfc3339()).map_err(other)?;
    tx.execute(
        "INSERT INTO documents(key,value) VALUES(?1,?2) ON CONFLICT(key) DO NOTHING",
        [MARKER, now.as_str()],
    )
    .map_err(other)?;
    tx.commit().map_err(other)?;
    let Some(stored) = stored else {
        return Ok(false);
    };
    stored.remember(path);
    let mut renamed = file.as_os_str().to_owned();
    renamed.push(".imported");
    match fs::rename(file, &renamed) {
        Ok(()) => log::info!(
            "Imported the settings in {} into the database; the file is now {}",
            file.display(),
            Path::new(&renamed).display()
        ),
        Err(error) => log::warn!(
            "Imported the settings in {}, but could not rename it: {error}",
            file.display()
        ),
    }
    Ok(true)
}

/// Imports a settings file into the database at `database`, as the first
/// start of this version does with `config.toml`. For trying it on a copy.
pub fn import_file(database: &Path, file: &Path) -> io::Result<bool> {
    let mut db = Database::open(database).map_err(other)?;
    import_once(db.connection(), database, file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    const FILE: &str = r##"
[appearance]
theme = "Ocean"
time_format = "compact"
distance_unit = "miles"

[window]
width = 320.0

[calendar]
sync_minutes = 60
client_id = "id"
client_secret = "secret"
disabled = ["work"]
sleep_schedule = [
  { bedtime = "23:00:00", wakeup = "07:00:00" },
  { bedtime = "00:00:00", wakeup = "08:00:00" },
  { bedtime = "00:00:00", wakeup = "08:00:00" },
  { bedtime = "00:00:00", wakeup = "08:00:00" },
  { bedtime = "00:00:00", wakeup = "08:00:00" },
  { bedtime = "00:00:00", wakeup = "08:00:00" },
  { bedtime = "00:00:00", wakeup = "08:00:00" },
]

[notes]
directory = "~/Notes"

[notepad]
font_size = 120
text = "from the file"

[reminders]
enabled = true

[reminders.events]
enabled = false

[travel]
api_key = "pk"
home = 1
places = [{ id = 1, name = "Home", lat = 1.5, lng = 2.5 }]

[travel.event_places]
standup = 1

[views]
dock_width = nan
"##;

    fn setup() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("config.toml");
        fs::write(&file, FILE).unwrap();
        let database = directory.path().join("timeslay.sqlite3");
        (directory, database, file)
    }

    fn rows(path: &Path, sql: &str) -> Vec<(String, String)> {
        let mut db = Database::open(path).unwrap();
        let connection = db.connection();
        let mut query = connection.prepare(sql).unwrap();
        let text = |value: rusqlite::types::ValueRef<'_>| match value {
            rusqlite::types::ValueRef::Integer(number) => number.to_string(),
            value => value.as_str().unwrap_or_default().to_owned(),
        };
        query
            .query_map([], |row| Ok((text(row.get_ref(0)?), text(row.get_ref(1)?))))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn the_file_is_imported_once_without_its_defaults_and_renamed() {
        let (_directory, path, file) = setup();
        assert!(import_file(&path, &file).unwrap());
        assert!(!file.exists());
        assert!(file.with_file_name("config.toml.imported").exists());

        let settings = rows(&path, "SELECT key, value FROM settings ORDER BY key");
        let keys: Vec<&str> = settings.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "appearance.distance_unit",
                "appearance.theme",
                "appearance.time_format",
                "calendar.client_id",
                "calendar.client_secret",
                "reminders.enabled",
                "travel.api_key",
                "travel.home",
            ],
            "sync_minutes = 60 is the default"
        );
        let device = rows(&path, "SELECT key, value FROM device_settings ORDER BY key");
        assert_eq!(
            device,
            [
                ("notepad.font_size".to_owned(), "120".to_owned()),
                ("notes.directory".to_owned(), "\"~/Notes\"".to_owned()),
                ("window.width".to_owned(), "320.0".to_owned()),
            ],
            "the NaN dock width is left at its default"
        );
        // A partial reminder table gets the defaults it always had.
        let reminders = rows(
            &path,
            "SELECT category, json_object('e',enabled,'m',minutes_before) FROM reminders",
        );
        assert_eq!(
            reminders,
            [("events".to_owned(), r#"{"e":0,"m":0}"#.to_owned())]
        );
        assert_eq!(
            rows(&path, "SELECT weekday, bedtime FROM sleep_schedule").len(),
            1
        );

        let config: Config = super::super::load(&path, |_| {}).unwrap().value;
        assert_eq!(config.notepad.text, "from the file");
        assert_eq!(config.travel.places[0].name, "Home");
        assert_eq!(config.travel.event_places["standup"], 1);
        assert_eq!(config.calendar.disabled, ["work"]);
        assert_eq!(config.notes.directory, "~/Notes");

        // Never again, even if a file turns up.
        fs::write(&file, "[calendar]\nsync_minutes = 5\n").unwrap();
        assert!(!import_file(&path, &file).unwrap());
        assert!(file.exists());
    }

    #[test]
    fn imported_rows_are_journaled_as_the_oldest_edits() {
        let (_directory, path, file) = setup();
        crate::task_db::write_document_json(&path, "habits", "{}").unwrap();
        import_file(&path, &file).unwrap();
        crate::settings::update(&path, |config: &mut Config| {
            config.calendar.sync_minutes = 5;
        })
        .unwrap();
        let journal = rows(
            &path,
            "SELECT document, clock FROM timeslay_sync_edits ORDER BY rowid",
        );
        let clock = |document: &str| -> Vec<i64> {
            journal
                .iter()
                .filter(|(name, _)| name.starts_with(document))
                .map(|(_, clock)| clock.parse().unwrap())
                .collect()
        };
        assert!(clock("settings:").len() > 2);
        assert!(
            clock("settings:[\"appearance")
                .iter()
                .all(|clock| *clock == 1)
        );
        assert!(clock("places:").iter().all(|clock| *clock == 1));
        assert_eq!(
            clock("settings:[\"calendar.sync_minutes\"]"),
            [2],
            "an edit after it sorts after it"
        );
        assert!(
            clock("device_settings").is_empty(),
            "device settings are never journaled"
        );
    }

    #[test]
    fn a_document_already_in_the_database_wins_over_the_files_copy() {
        let (_directory, path, file) = setup();
        let notepad = crate::config::NotepadText {
            text: "from the database".into(),
        };
        crate::shared_data::save_section(&path, "notepad", &notepad).unwrap();
        import_file(&path, &file).unwrap();
        let config: Config = super::super::load(&path, |_| {}).unwrap().value;
        assert_eq!(config.notepad.text, "from the database");
    }

    #[test]
    fn a_broken_file_is_left_alone() {
        let (_directory, path, file) = setup();
        fs::write(&file, "[broken\n").unwrap();
        assert!(!import_file(&path, &file).unwrap());
        assert_eq!(fs::read_to_string(&file).unwrap(), "[broken\n");
    }
}
