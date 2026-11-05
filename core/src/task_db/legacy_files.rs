//! Moves the files older builds kept into the database.
//!
//! Each file is imported only if the database has nothing under its name yet, so
//! this can run on every launch and never overwrites newer data. A file is
//! retired only after the transaction holding its contents has committed. One
//! that does not parse is left where it is, to be repaired by hand.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::params;

use super::{Database, keys};

/// Where each kind of file used to live.
pub(super) struct Directories {
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: Option<PathBuf>,
    pub cache: PathBuf,
}

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Json,
    /// A bare string, stored as a JSON string.
    Text,
}

#[derive(Clone, Copy, PartialEq)]
enum Retire {
    /// Could not be fetched again, so it stays beside the original name as
    /// `<name>.migrated` until the user removes it.
    Keep,
    /// A later sync rebuilds it.
    Delete,
}

enum Target {
    Document(&'static str),
    Month(String),
}

struct Source {
    file: PathBuf,
    format: Format,
    retire: Retire,
    target: Target,
}

fn sources(directories: &Directories) -> Vec<Source> {
    let source = |file: PathBuf, format, retire, key| Source {
        file,
        format,
        retire,
        target: Target::Document(key),
    };
    let Directories {
        config,
        data,
        state,
        cache,
    } = directories;
    let days = cache.join("days");

    let mut sources = vec![
        source(
            config.join("dioxus-theme"),
            Format::Text,
            Retire::Keep,
            keys::THEME,
        ),
        // Android kept notes in its data directory, and the desktop in its cache.
        source(
            data.join("notes").join("notes.json"),
            Format::Json,
            Retire::Keep,
            keys::NOTES,
        ),
        source(
            cache.join("notes").join("notes.json"),
            Format::Json,
            Retire::Keep,
            keys::NOTES,
        ),
        source(
            cache.join("people").join("contacts.json"),
            Format::Json,
            Retire::Delete,
            keys::CONTACTS,
        ),
        source(
            cache.join("travel").join("routes.json"),
            Format::Json,
            Retire::Delete,
            keys::ROUTES,
        ),
        source(
            days.join("sync-state.json"),
            Format::Json,
            Retire::Delete,
            keys::SYNC_STATE,
        ),
    ];
    if let Some(state) = state {
        sources.push(source(
            state.join("habits.json"),
            Format::Json,
            Retire::Keep,
            keys::HABITS,
        ));
        sources.push(source(
            state.join("alarms.json"),
            Format::Json,
            Retire::Keep,
            keys::ALARMS,
        ));
    }
    if let Ok(entries) = fs::read_dir(&days) {
        sources.extend(entries.filter_map(Result::ok).filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let month = name.strip_suffix(".json")?;
            crate::calendar::day_cache::parse_month(month)?;
            Some(Source {
                file: entry.path(),
                format: Format::Json,
                retire: Retire::Delete,
                target: Target::Month(month.to_owned()),
            })
        }));
    }
    sources
}

/// The file's contents as the JSON the database stores. `None` when there is no
/// such file or it cannot be understood.
fn read_json(source: &Source) -> Option<String> {
    let text = match fs::read_to_string(&source.file) {
        Ok(text) => text,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("could not read {}: {error}", source.file.display());
            }
            return None;
        }
    };
    let parsed = match source.format {
        Format::Json => serde_json::from_str::<serde_json::Value>(&text).map_err(|e| e.to_string()),
        Format::Text => Ok(serde_json::Value::String(text.trim().to_owned())),
    };
    match parsed {
        Ok(value) => Some(value.to_string()),
        Err(error) => {
            log::warn!(
                "{} could not be parsed, so it was not imported: {error}",
                source.file.display()
            );
            None
        }
    }
}

fn retire(source: &Source) {
    let result = match source.retire {
        Retire::Keep => {
            let mut name = source.file.clone().into_os_string();
            name.push(".migrated");
            fs::rename(&source.file, name)
        }
        Retire::Delete => fs::remove_file(&source.file),
    };
    if let Err(error) = result
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("could not retire {}: {error}", source.file.display());
    }
    let mut lock = source.file.clone().into_os_string();
    lock.push(".lock");
    let _ = fs::remove_file(lock);
    // Only succeeds once nothing else is in there.
    if source.retire == Retire::Delete
        && let Some(parent) = source.file.parent()
    {
        let _ = fs::remove_dir(parent);
    }
}

pub(super) fn import(database: &Path, directories: &Directories) -> Result<(), String> {
    let found: Vec<(Source, String)> = sources(directories)
        .into_iter()
        .filter_map(|source| {
            let json = read_json(&source)?;
            Some((source, json))
        })
        .collect();
    if found.is_empty() {
        return Ok(());
    }

    let mut database = Database::open(database)?;
    let imported = (|| -> rusqlite::Result<Vec<Source>> {
        let tx = database
            .connection()
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut imported = Vec::new();
        for (source, json) in found {
            let changed = match &source.target {
                Target::Document(key) => tx.execute(
                    "INSERT INTO documents (key,value) VALUES (?1,?2) ON CONFLICT(key) DO NOTHING",
                    params![key, json],
                )?,
                Target::Month(month) => tx.execute(
                    "INSERT INTO calendar_months (month,events) VALUES (?1,?2) ON CONFLICT(month) DO NOTHING",
                    params![month, json],
                )?,
            };
            if changed == 1 {
                imported.push(source);
            }
        }
        tx.commit()?;
        Ok(imported)
    })()
    .map_err(|error| error.to_string())?;

    for source in &imported {
        retire(source);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_db::{read_document, write_document};

    struct Home {
        directory: tempfile::TempDir,
    }

    impl Home {
        fn new() -> Self {
            Self {
                directory: tempfile::tempdir().unwrap(),
            }
        }
        fn directories(&self) -> Directories {
            let root = self.directory.path();
            Directories {
                config: root.join("config"),
                data: root.join("data"),
                state: Some(root.join("state")),
                cache: root.join("cache"),
            }
        }
        fn database(&self) -> PathBuf {
            self.directory.path().join("data").join("timeslay.sqlite3")
        }
        fn write(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.directory.path().join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, text).unwrap();
            path
        }
    }

    fn value(home: &Home, key: &str) -> Option<serde_json::Value> {
        read_document(&home.database(), key).unwrap()
    }

    #[test]
    fn every_kind_of_file_lands_in_the_database() {
        let home = Home::new();
        home.write("state/habits.json", r#"{"habits":[1]}"#);
        home.write("state/alarms.json", r#"{"alarms":[]}"#);
        home.write("cache/notes/notes.json", r#"{"notes":[]}"#);
        home.write("cache/notes/notes.json.lock", "");
        home.write("cache/people/contacts.json", r#"{"accounts":[]}"#);
        home.write("cache/travel/routes.json", "[]");
        home.write("cache/days/sync-state.json", r#"{"calendars":[]}"#);
        home.write("cache/days/2026-11.json", r#"{"events":[1,2]}"#);
        home.write("config/dioxus-theme", "dark\n");
        let token = home.write("config/google-token.toml", "access_token = \"x\"\n");

        import(&home.database(), &home.directories()).unwrap();

        assert_eq!(value(&home, keys::HABITS).unwrap()["habits"][0], 1);
        assert!(value(&home, keys::ALARMS).is_some());
        assert!(value(&home, keys::NOTES).is_some());
        assert!(value(&home, keys::CONTACTS).is_some());
        assert!(value(&home, keys::ROUTES).is_some());
        assert!(value(&home, keys::SYNC_STATE).is_some());
        assert_eq!(value(&home, keys::THEME).unwrap(), "dark");
        assert!(token.exists(), "credentials are for crate::secrets");
        let mut database = Database::open(&home.database()).unwrap();
        let month: String = database
            .connection()
            .query_row(
                "SELECT events FROM calendar_months WHERE month='2026-11'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(month, r#"{"events":[1,2]}"#);

        let root = home.directory.path();
        // What cannot be refetched is kept aside, and what can is gone.
        assert!(root.join("state/habits.json.migrated").exists());
        assert!(!root.join("state/habits.json").exists());
        assert!(root.join("cache/notes/notes.json.migrated").exists());
        assert!(!root.join("cache/notes/notes.json.lock").exists());
        assert!(!root.join("cache/days").exists());
        assert!(!root.join("cache/travel").exists());
    }

    #[test]
    fn newer_data_in_the_database_is_never_overwritten() {
        let home = Home::new();
        write_document(&home.database(), keys::HABITS, &"newer").unwrap();
        let file = home.write("state/habits.json", r#"{"habits":[]}"#);

        import(&home.database(), &home.directories()).unwrap();

        assert_eq!(value(&home, keys::HABITS).unwrap(), "newer");
        assert!(file.exists(), "it was not imported, so it stays");
    }

    #[test]
    fn a_file_that_does_not_parse_is_left_alone() {
        let home = Home::new();
        let file = home.write("state/habits.json", "not json");

        import(&home.database(), &home.directories()).unwrap();

        assert_eq!(fs::read_to_string(file).unwrap(), "not json");
        assert_eq!(value(&home, keys::HABITS), None);
    }

    #[test]
    fn a_second_import_changes_nothing() {
        let home = Home::new();
        home.write("state/habits.json", r#"{"habits":[1]}"#);
        import(&home.database(), &home.directories()).unwrap();
        write_document(&home.database(), keys::HABITS, &"edited").unwrap();

        import(&home.database(), &home.directories()).unwrap();

        assert_eq!(value(&home, keys::HABITS).unwrap(), "edited");
    }

    #[test]
    fn files_not_named_like_a_month_stay_out_of_the_cache() {
        let home = Home::new();
        let stray = home.write("cache/days/notes.json", "{}");
        let half = home.write("cache/days/2026-10.json.tmp", "{}");

        import(&home.database(), &home.directories()).unwrap();

        assert!(stray.exists());
        assert!(half.exists());
    }
}
