//! The single local database: remote task metadata, durable ID/alias allocation,
//! and everything else this device keeps between runs.
//!
//! Tasks and cached calendar months have tables of their own. The rest are JSON
//! documents, one per kind of data, named in [`keys`].
//!
//! LEGACY_METADATA_COMPAT: unexported SQLite rows are migration sources only.
//! This marker covers the old inline JSON, `ttid:` ids and SQLite-only metadata.
//! It can all go once every device has synced a build that writes metadata to
//! Google, and no task notes in the user's account start with `ttid:`.
use crate::{
    calendar::Event,
    timeblocking::{Metadata, legacy_id, legacy_metadata},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Once,
    time::Duration,
};

mod legacy_files;

/// The names of the documents in the database.
pub mod keys {
    pub const HABITS: &str = "habits";
    pub const ALARMS: &str = "alarms";
    pub const NOTES: &str = "notes";
    pub const CONTACTS: &str = "contacts";
    pub const ROUTES: &str = "routes";
    pub const SYNC_STATE: &str = "calendar_sync_state";
    /// Android's widget reads this one too: see `DayRenderer.kt`.
    pub const THEME: &str = "dioxus_theme";
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskData {
    pub id: i64,
    pub metadata: Metadata,
    // Preserve keys already used by work sessions, including in remote metadata.
    pub legacy_key: Option<String>,
}

pub struct Database(Connection);
impl Database {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let db = Connection::open(path).map_err(|e| e.to_string())?;
        // Holds credentials and personal details. An in-memory database has no file.
        if let Err(error) = crate::fsutil::make_private(path)
            && error.kind() != io::ErrorKind::NotFound
        {
            return Err(error.to_string());
        }
        db.busy_timeout(Duration::from_secs(10))
            .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS counter (singleton INTEGER PRIMARY KEY CHECK(singleton=1), next_id INTEGER NOT NULL CHECK(next_id >= 100));
            INSERT OR IGNORE INTO counter VALUES (1,100);
            CREATE TABLE IF NOT EXISTS tasks (id INTEGER PRIMARY KEY, metadata TEXT NOT NULL, legacy_key TEXT UNIQUE);
            CREATE TABLE IF NOT EXISTS task_aliases (source TEXT PRIMARY KEY, task_id INTEGER NOT NULL REFERENCES tasks(id));
            CREATE TABLE IF NOT EXISTS documents (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS calendar_months (month TEXT PRIMARY KEY, events TEXT NOT NULL);")
            .map_err(|e| e.to_string())?;
        if path != Path::new(":memory:") {
            let enrolled = db
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE name='timeslay_sync_control'",
                    [],
                    |_| Ok(()),
                )
                .optional()
                .map_err(|e| e.to_string())?
                .is_some();
            if !enrolled {
                crate::database_sync::enroll(path).map_err(|e| e.to_string())?;
            }
        }
        Ok(Self(db))
    }

    pub(crate) fn connection(&mut self) -> &mut Connection {
        &mut self.0
    }

    /// Adopt authoritative remote data even on a fresh installation.
    pub fn cache_remote(&mut self, event: &Event, data: &TaskData) -> Result<(), String> {
        self.save(event, data)
    }

    /// Allocate and record the mapping in one transaction. No ID escapes until commit.
    pub fn import(&mut self, event: &Event) -> Result<TaskData, String> {
        if let Some(data) = crate::calendar::task_metadata::read(&event.details.description)? {
            self.cache_remote(event, &data)?;
            return Ok(data);
        }
        // LEGACY_METADATA_COMPAT: consult SQLite/inline notes only without a remote footer.
        self.import_inner(event).map_err(|e| e.to_string())
    }
    fn import_inner(&mut self, event: &Event) -> rusqlite::Result<TaskData> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let source = crate::timeblocking::block_key(event);
        let legacy = legacy_id(&event.details.description);
        let existing = tx.query_row("SELECT t.id,t.metadata,t.legacy_key FROM tasks t JOIN task_aliases a ON a.task_id=t.id WHERE a.source=?1",
            [&source], read_data).optional()?;
        let existing = match (existing, legacy.as_ref()) {
            (None, Some(key)) => tx
                .query_row(
                    "SELECT id,metadata,legacy_key FROM tasks WHERE legacy_key=?1",
                    [key],
                    read_data,
                )
                .optional()?,
            (value, _) => value,
        };
        let data = if let Some(data) = existing {
            data
        } else {
            let synced = tx
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE name='timeslay_sync_control'",
                    [],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            let id: i64 = if synced {
                loop {
                    let mut bytes = [0u8; 8];
                    getrandom::fill(&mut bytes).map_err(|_| rusqlite::Error::InvalidQuery)?;
                    let candidate = (u64::from_le_bytes(bytes) & ((1u64 << 62) - 1)) as i64;
                    if candidate < 100 {
                        continue;
                    }
                    if !tx
                        .query_row("SELECT 1 FROM tasks WHERE id=?1", [candidate], |_| Ok(()))
                        .optional()?
                        .is_some()
                    {
                        break candidate;
                    }
                }
            } else {
                tx.query_row("SELECT next_id FROM counter WHERE singleton=1", [], |r| {
                    r.get(0)
                })?
            };
            // Refuse overflow rather than wrapping or switching to random IDs.
            let next = id.checked_add(1).ok_or(rusqlite::Error::InvalidQuery)?;
            let metadata = legacy_metadata(&event.details.description);
            let json = serde_json::to_string(&metadata).expect("metadata serializes");
            tx.execute(
                "INSERT INTO tasks VALUES (?1,?2,?3)",
                params![id, json, legacy],
            )?;
            tx.execute("UPDATE counter SET next_id=?1 WHERE singleton=1", [next])?;
            TaskData {
                id,
                metadata,
                legacy_key: legacy,
            }
        };
        tx.execute(
            "INSERT INTO task_aliases VALUES (?1,?2) ON CONFLICT(source) DO NOTHING",
            params![source, data.id],
        )?;
        tx.commit()?;
        Ok(data)
    }

    pub fn lookup(&self, event: &Event) -> Result<Option<TaskData>, String> {
        self.0.query_row("SELECT t.id,t.metadata,t.legacy_key FROM tasks t JOIN task_aliases a ON a.task_id=t.id WHERE a.source=?1",
            [crate::timeblocking::block_key(event)], read_data).optional().map_err(|e| e.to_string())
    }

    /// Keep both old and new Google identifiers when a move changes the task ID.
    pub fn save(&mut self, event: &Event, data: &TaskData) -> Result<(), String> {
        (|| -> rusqlite::Result<()> {
            let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let json = serde_json::to_string(&data.metadata).expect("metadata serializes");
            tx.execute("INSERT INTO tasks (id,metadata,legacy_key) VALUES (?1,?2,?3) ON CONFLICT(id) DO UPDATE SET metadata=excluded.metadata,legacy_key=excluded.legacy_key", params![data.id,json,data.legacy_key])?;
            let next = data.id.checked_add(1).ok_or(rusqlite::Error::InvalidQuery)?;
            tx.execute("UPDATE counter SET next_id=max(next_id,?1) WHERE singleton=1", [next])?;
            tx.execute("INSERT INTO task_aliases VALUES (?1,?2) ON CONFLICT(source) DO UPDATE SET task_id=excluded.task_id", params![crate::timeblocking::block_key(event),data.id])?;
            tx.commit()
        })().map_err(|e| e.to_string())
    }
}
fn read_data(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskData> {
    let json: String = row.get(1)?;
    let metadata = serde_json::from_str(&json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(TaskData {
        id: row.get(0)?,
        metadata,
        legacy_key: row.get(2)?,
    })
}

/// The database on this device, with whatever older builds left in files
/// already moved into it.
pub fn path() -> Option<PathBuf> {
    static IMPORT: Once = Once::new();
    let directories = crate::config::project_dirs()?;
    let path = directories.data_local_dir().join("timeslay.sqlite3");
    IMPORT.call_once(|| {
        if let Err(error) = rename_from_tasks(&path) {
            log::warn!("could not rename tasks.sqlite3: {error}");
        }
        let legacy = legacy_files::Directories {
            config: directories.config_dir().to_owned(),
            data: directories.data_dir().to_owned(),
            state: directories.state_dir().map(Path::to_owned),
            cache: directories.cache_dir().to_owned(),
        };
        if let Err(error) = legacy_files::import(&path, &legacy) {
            log::warn!("could not import older local files: {error}");
        }
        let config = directories.config_dir().join("config.toml");
        if let Err(error) = crate::shared_data::adopt(&config, &path) {
            log::warn!("could not move shared data out of config.toml: {error}");
        }
    });
    Some(path)
}

/// Older builds called the database `tasks.sqlite3`. It is renamed, and on
/// Unix the old name is left as a link to the new one, so an older build still
/// running opens the same database rather than starting an empty one. SQLite
/// names a database's journal after the file a link points to, so the two
/// builds also agree on where the journal is.
fn rename_from_tasks(path: &Path) -> io::Result<()> {
    let old = path.with_file_name("tasks.sqlite3");
    let is_file = std::fs::symlink_metadata(&old).is_ok_and(|metadata| metadata.is_file());
    if !is_file || std::fs::symlink_metadata(path).is_ok() {
        return Ok(());
    }
    // A journal left by a crash must follow its database, or the
    // transaction it would roll back is lost.
    let journal = old.with_file_name("tasks.sqlite3-journal");
    if journal.exists() {
        std::fs::rename(&journal, path.with_file_name("timeslay.sqlite3-journal"))?;
    }
    match std::fs::rename(&old, path) {
        // Another process just did it.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        result => result?,
    }
    #[cfg(unix)]
    match std::os::unix::fs::symlink("timeslay.sqlite3", &old) {
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        result => result?,
    }
    Ok(())
}

pub fn open() -> Result<Database, String> {
    Database::open(&path().ok_or("Cannot locate local data directory")?)
}

/// A document, or `None` when nothing has saved it yet. Contents that do not
/// parse are an error, so a caller can decline to save over them.
pub fn read_document<T: DeserializeOwned>(path: &Path, key: &str) -> io::Result<Option<T>> {
    let database = Database::open(path).map_err(io::Error::other)?;
    let text: Option<String> = database
        .0
        .query_row("SELECT value FROM documents WHERE key=?1", [key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(io::Error::other)?;
    let result = text
        .as_ref()
        .map(|text| {
            serde_json::from_str(text)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        })
        .transpose()?;
    crate::database_sync::projection::remember(path, key, text);
    Ok(result)
}

pub fn write_document<T: Serialize + ?Sized>(path: &Path, key: &str, value: &T) -> io::Result<()> {
    let json = serde_json::to_string(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    write_document_json(path, key, &json)
}

/// [`write_document`] for a value that is already serialized.
pub fn write_document_json(path: &Path, key: &str, json: &str) -> io::Result<()> {
    change_document(path, key, Some(json.to_owned()))
}

pub fn remove_document(path: &Path, key: &str) -> io::Result<()> {
    change_document(path, key, None)
}

fn change_document(path: &Path, key: &str, desired: Option<String>) -> io::Result<()> {
    let mut database = Database::open(path).map_err(io::Error::other)?;
    let tx = database
        .0
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(io::Error::other)?;
    let value = crate::database_sync::projection::prepare(path, &tx, key, &desired)?;
    match value {
        Some(value) => {
            tx.execute("INSERT INTO documents(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value]).map_err(io::Error::other)?;
        }
        None => {
            tx.execute("DELETE FROM documents WHERE key=?1", [key])
                .map_err(io::Error::other)?;
        }
    }
    tx.commit().map_err(io::Error::other)?;
    crate::database_sync::projection::remember(path, key, desired);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!(
                "tt-db-{label}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ))
            .join("timeslay.sqlite3")
    }
    fn event(id: &str) -> Event {
        let mut event = crate::calendar::sample_snapshot().events.remove(0);
        event.id = format!("task:{id}");
        event.calendar_id = "tasks:test".into();
        event.details = Default::default();
        event
    }
    #[test]
    fn persistent_ids_and_migration_preserve_metadata_and_history() {
        let path = path("migration");
        let mut db = Database::open(&path).unwrap();
        let mut original = event("old");
        original.details.description = "Human notes\ntt-timeblocking: {\"minutes\":45,\"block\":\"work/block\",\"order\":9}\nttid: 12345678-1234-4234-8234-123456789abc".into();
        let mut data = db.import(&original).unwrap();
        assert!(data.id >= 100);
        let original_id = data.id;
        assert_eq!(data.metadata.minutes, 45);
        assert_eq!(
            data.legacy_key.as_deref(),
            Some("12345678-1234-4234-8234-123456789abc")
        );
        assert_eq!(db.import(&original).unwrap(), data);
        let mut moved = event("new");
        moved.calendar_id = "tasks:destination".into();
        data.metadata.minutes = 90;
        db.save(&moved, &data).unwrap();
        drop(db);
        let mut db = Database::open(&path).unwrap();
        assert_eq!(db.import(&moved).unwrap(), data);
        assert_eq!(db.import(&original).unwrap(), data); // stale notes cannot overwrite local edits
        let another_id = db.import(&event("another")).unwrap().id;
        assert_ne!(another_id, original_id);
        // A rolled-back transaction must not publish a task.
        {
            let tx =
                db.0.transaction_with_behavior(TransactionBehavior::Immediate)
                    .unwrap();
            tx.execute("INSERT INTO tasks VALUES(5000,'{}',NULL)", [])
                .unwrap();
        }
        assert_eq!(
            db.0.query_row("SELECT count(*) FROM tasks WHERE id=5000", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let after_rollback = db.import(&event("after-rollback")).unwrap().id;
        assert_ne!(after_rollback, original_id);
        assert_ne!(after_rollback, another_id);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn documents_round_trip_and_bad_contents_are_an_error() {
        let path = path("documents");
        assert_eq!(read_document::<String>(&path, "a").unwrap(), None);
        write_document(&path, "a", "first").unwrap();
        write_document(&path, "a", "second").unwrap();
        assert_eq!(
            read_document::<String>(&path, "a").unwrap().as_deref(),
            Some("second")
        );
        write_document_json(&path, "a", "not json").unwrap();
        assert_eq!(
            read_document::<String>(&path, "a").unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        remove_document(&path, "a").unwrap();
        assert_eq!(read_document::<String>(&path, "a").unwrap(), None);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn the_database_is_readable_only_by_this_user() {
        use std::os::unix::fs::PermissionsExt;
        let path = path("private");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        Database::open(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn the_old_name_is_renamed_and_still_leads_to_the_database() {
        let directory = tempfile::tempdir().unwrap();
        let old = directory.path().join("tasks.sqlite3");
        let path = directory.path().join("timeslay.sqlite3");
        let mut db = Database::open(&old).unwrap();
        let data = db.import(&event("kept")).unwrap();
        drop(db);

        rename_from_tasks(&path).unwrap();
        rename_from_tasks(&path).unwrap();

        assert!(std::fs::symlink_metadata(&path).unwrap().is_file());
        assert_eq!(
            Database::open(&path)
                .unwrap()
                .lookup(&event("kept"))
                .unwrap(),
            Some(data.clone())
        );
        // An older build opening the old name reaches the same rows.
        assert_eq!(
            Database::open(&old)
                .unwrap()
                .lookup(&event("kept"))
                .unwrap(),
            Some(data)
        );
    }
    #[test]
    fn allocator_process() {
        let Some(path) = std::env::var_os("TT_TEST_DATABASE") else {
            return;
        };
        let worker = std::env::var("TT_TEST_WORKER").unwrap();
        let mut db = Database::open(Path::new(&path)).unwrap();
        for i in 0..20 {
            db.import(&event(&format!("{worker}-{i}"))).unwrap();
            db.import(&event("shared-task")).unwrap();
        }
    }
    #[test]
    fn concurrent_processes_get_unique_ids_and_one_shared_mapping() {
        let path = path("concurrent");
        let children: Vec<_> = (0..4)
            .map(|worker| {
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "task_db::tests::allocator_process",
                        "--nocapture",
                    ])
                    .env("TT_TEST_DATABASE", &path)
                    .env("TT_TEST_WORKER", worker.to_string())
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for mut child in children {
            assert!(child.wait().unwrap().success());
        }
        let db = Database::open(&path).unwrap();
        let (count, min, max): (i64, i64, i64) =
            db.0.query_row("SELECT count(*),min(id),max(id) FROM tasks", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();
        assert_eq!(count, 81);
        assert!(min >= 100 && max < (1_i64 << 62));
        let aliases: i64 =
            db.0.query_row("SELECT count(*) FROM task_aliases", [], |r| r.get(0))
                .unwrap();
        assert_eq!(aliases, 81, "all processes reuse the same shared task");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
