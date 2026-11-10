//! The single local database: remote task metadata, durable ID/alias allocation,
//! and everything else this device keeps between runs.
//!
//! Tasks and cached calendar months have tables of their own. The rest are JSON
//! documents, one per kind of data, named in [`keys`].
//!
use crate::calendar::ids;
use crate::{calendar::Event, timeblocking::Metadata};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};

/// The names of the documents in the database.
pub mod keys {
    pub const HABITS: &str = "habits";
    pub const ALARMS: &str = "alarms";
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
}

/// The schema this build sets up, kept in SQLite's `user_version`.
///
/// Schema changes are applied transactionally by [`Database::set_up`].
pub const SCHEMA_VERSION: i64 = 4;

/// Older schemas [`Database::set_up`] can bring up to date.
const UPGRADABLE: std::ops::RangeInclusive<i64> = 2..=3;

fn schema_version(db: &Connection) -> Result<i64, String> {
    db.query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| e.to_string())
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
        // Per connection, so it cannot be part of the one-off setup below.
        db.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| e.to_string())?;
        let memory = path == Path::new(":memory:");
        // Setting up the schema is most of the cost of opening, and the apps
        // open the database many times a minute. Once a file has been brought
        // to the current version it is not set up again.
        let version = schema_version(&db)?;
        if version != 0 && version != SCHEMA_VERSION && !UPGRADABLE.contains(&version) {
            return Err(format!(
                "Unsupported database schema {version}; expected {SCHEMA_VERSION}"
            ));
        }
        if version == 0
            && db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='tasks') AND (SELECT count(*) FROM pragma_table_info('tasks')) != 2",
                    [],
                    |r| r.get::<_, bool>(0),
                )
                .map_err(|e| e.to_string())?
        {
            return Err("Unversioned task databases are no longer supported".into());
        }
        if version == 2 {
            Self::drop_legacy_key(&db).map_err(|e| e.to_string())?;
        }
        if memory || version != SCHEMA_VERSION {
            Self::set_up(&db, path, memory)?;
        }
        Ok(Self(db))
    }

    /// Brings the file to [`SCHEMA_VERSION`]. Every step is idempotent, so two
    /// processes doing this at once, or a crash half way, are both harmless.
    fn set_up(db: &Connection, path: &Path, memory: bool) -> Result<(), String> {
        // Immediate, so a second process waits here and then finds each step
        // already done, instead of racing it.
        let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        tx.execute_batch("
            CREATE TABLE IF NOT EXISTS counter (singleton INTEGER PRIMARY KEY CHECK(singleton=1), next_id INTEGER NOT NULL CHECK(next_id >= 100));
            INSERT OR IGNORE INTO counter VALUES (1,100);
            CREATE TABLE IF NOT EXISTS tasks (id INTEGER PRIMARY KEY, metadata TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS task_aliases (source TEXT PRIMARY KEY, task_id INTEGER NOT NULL REFERENCES tasks(id));
            CREATE TABLE IF NOT EXISTS documents (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS google_task_records (id TEXT PRIMARY KEY, event TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS google_task_backend_state (task_id INTEGER PRIMARY KEY, data TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS google_entities (id TEXT PRIMARY KEY, event TEXT, acknowledged_at TEXT, observed_at TEXT, sending_since TEXT);
            CREATE TABLE IF NOT EXISTS google_id_map (local TEXT PRIMARY KEY, remote TEXT NOT NULL, calendar TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS calendar_months (month TEXT PRIMARY KEY, events TEXT NOT NULL);")
            .map_err(|e| e.to_string())?;
        crate::local_changes::install(&tx).map_err(|e| e.to_string())?;
        Self::remove_write_queue(&tx).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;

        if !memory {
            crate::database_sync::enroll(path).map_err(|e| e.to_string())?;
            crate::database_sync::install_snapshots(db).map_err(|e| e.to_string())?;
            // Readers no longer wait on a writer, so a poll on the UI thread
            // cannot stall behind the sync worker's transaction. The mode is
            // kept by the file. It cannot change while another connection is
            // mid-transaction; that is retried on the next open.
            match db.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0)) {
                Ok(mode) if mode.eq_ignore_ascii_case("wal") => {}
                Ok(mode) => {
                    log::warn!("database stayed in {mode} mode; will retry");
                    return Ok(());
                }
                Err(error) => {
                    log::warn!("could not switch the database to WAL: {error}");
                    return Ok(());
                }
            }
        }
        db.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Schema 3: `tasks` loses `legacy_key`. The column is `UNIQUE`, so SQLite
    /// cannot drop it in place and the table is rebuilt, as SQLite documents,
    /// with foreign keys off so `task_aliases` keeps pointing at its rows.
    /// That pragma cannot change inside a transaction, so this runs on its own
    /// before [`Database::set_up`]. Idempotent: a second process finds the
    /// column already gone.
    fn drop_legacy_key(db: &Connection) -> rusqlite::Result<()> {
        db.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let result = (|| {
            let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
            let has_column = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('tasks') WHERE name='legacy_key')",
                [],
                |r| r.get::<_, bool>(0),
            )?;
            if has_column {
                tx.execute_batch(
                    "CREATE TABLE tasks_new (id INTEGER PRIMARY KEY, metadata TEXT NOT NULL);
                     INSERT INTO tasks_new(id,metadata) SELECT id,metadata FROM tasks;
                     DROP TABLE tasks;
                     ALTER TABLE tasks_new RENAME TO tasks;",
                )?;
                let broken = tx
                    .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
                    .optional()?
                    .is_some();
                if broken {
                    return Err(rusqlite::Error::InvalidQuery);
                }
            }
            tx.commit()
        })();
        db.execute_batch("PRAGMA foreign_keys = ON;")?;
        result
    }

    /// Schema 4: writes go to Google as they are made instead of through a
    /// durable queue. The queue's tables go, and an item marked as sending
    /// gets `google_entities.sending_since`. Edits still waiting in the old
    /// queue are dropped with it, so those items show what Google has.
    fn remove_write_queue(tx: &Connection) -> rusqlite::Result<()> {
        let has_column = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('google_entities') WHERE name='sending_since')",
            [],
            |r| r.get::<_, bool>(0),
        )?;
        if !has_column {
            tx.execute_batch("ALTER TABLE google_entities ADD COLUMN sending_since TEXT;")?;
        }
        let had_queue = tx
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='google_operations'",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if had_queue {
            tx.execute_batch(
                "DELETE FROM google_entities WHERE id IN (SELECT entity FROM google_operations);
                 DELETE FROM google_entities WHERE acknowledged_at IS NULL AND sending_since IS NULL;
                 DROP TRIGGER IF EXISTS timeslay_change_google_operations_insert;
                 DROP TRIGGER IF EXISTS timeslay_change_google_operations_update;
                 DROP TRIGGER IF EXISTS timeslay_change_google_operations_delete;
                 DROP TABLE IF EXISTS google_operation_progress;
                 DROP TABLE IF EXISTS google_operations;
                 DELETE FROM timeslay_change_counters WHERE name='google.queue';",
            )?;
        }
        Ok(())
    }

    pub(crate) fn connection(&mut self) -> &mut Connection {
        &mut self.0
    }

    /// Metadata on its way to Google remains authoritative while Google
    /// refreshes in parallel.
    pub(crate) fn pending_task_data(&self, event: &Event) -> Result<Option<TaskData>, String> {
        let stable = event.details.task_data.as_ref().map(|data| data.id).or(
            crate::calendar::task_metadata::read(&event.details.description)?.map(|data| data.id),
        );
        let json:Option<String>=self.0.query_row("SELECT e.event FROM google_entities e WHERE (json_extract(e.event,'$.id')=?1 OR json_extract(e.event,'$.details.task_data.id')=?2) AND e.sending_since IS NOT NULL",params![event.id,stable],|r|r.get(0)).optional().map_err(|e|e.to_string())?.flatten();
        json.map(|json| {
            serde_json::from_str::<Event>(&json)
                .map(|e| e.details.task_data)
                .map_err(|e| e.to_string())
        })
        .transpose()
        .map(Option::flatten)
    }

    /// Adopt authoritative remote data even on a fresh installation.
    pub fn cache_remote(&mut self, event: &Event, data: &TaskData) -> Result<(), String> {
        self.save(event, data)
    }

    /// Allocate and record the mapping in one transaction. No ID escapes until commit.
    pub fn import(&mut self, event: &Event) -> Result<TaskData, String> {
        if let Some(data) = &event.details.task_data {
            return Ok(data.clone());
        }
        if let Some(data) = crate::calendar::task_metadata::read(&event.details.description)? {
            self.cache_remote(event, &data)?;
            return Ok(data);
        }
        self.import_inner(event).map_err(|e| e.to_string())
    }
    fn import_inner(&mut self, event: &Event) -> rusqlite::Result<TaskData> {
        let tx = self
            .0
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let source = crate::timeblocking::block_key(event);
        let existing = tx.query_row("SELECT t.id,t.metadata FROM tasks t JOIN task_aliases a ON a.task_id=t.id WHERE a.source=?1",
            [&source], read_data).optional()?;
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
            let metadata = Metadata::default();
            let json = serde_json::to_string(&metadata).expect("metadata serializes");
            tx.execute(
                "INSERT INTO tasks(id,metadata) VALUES (?1,?2)",
                params![id, json],
            )?;
            tx.execute("UPDATE counter SET next_id=?1 WHERE singleton=1", [next])?;
            TaskData { id, metadata }
        };
        tx.execute(
            "INSERT INTO task_aliases VALUES (?1,?2) ON CONFLICT(source) DO NOTHING",
            params![source, data.id],
        )?;
        tx.commit()?;
        Ok(data)
    }

    pub fn lookup(&self, event: &Event) -> Result<Option<TaskData>, String> {
        self.0.query_row("SELECT t.id,t.metadata FROM tasks t JOIN task_aliases a ON a.task_id=t.id WHERE a.source=?1",
            [crate::timeblocking::block_key(event)], read_data).optional().map_err(|e| e.to_string())
    }

    /// Keep both old and new Google identifiers when a move changes the task ID.
    pub fn save(&mut self, event: &Event, data: &TaskData) -> Result<(), String> {
        // While a write is being sent, what it saves is what Google now
        // holds. The tasks table keeps the user's wanted state, which a later
        // edit may already have moved on from.
        if crate::calendar::operations::replaying() {
            let json = serde_json::to_string(data).map_err(|e| e.to_string())?;
            let tx = self
                .0
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|e| e.to_string())?;
            tx.execute("INSERT INTO google_task_backend_state(task_id,data) VALUES(?1,?2) ON CONFLICT(task_id) DO UPDATE SET data=excluded.data",params![data.id,json]).map_err(|e|e.to_string())?;
            if event.is_task()
                && let Some(local) = crate::calendar::operations::sending_task()
                && ids::bare_task(&local) != ids::bare_task(&event.id)
            {
                tx.execute("INSERT INTO google_id_map(local,remote,calendar) VALUES(?1,?2,?3) ON CONFLICT(local) DO UPDATE SET remote=excluded.remote,calendar=excluded.calendar",params![ids::bare_task(&local),ids::bare_task(&event.id),event.calendar_id]).map_err(|e|e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())?;
            return Ok(());
        }
        (|| -> rusqlite::Result<()> {
            let tx = self.0.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let json = serde_json::to_string(&data.metadata).expect("metadata serializes");
            tx.execute("INSERT INTO tasks (id,metadata) VALUES (?1,?2) ON CONFLICT(id) DO UPDATE SET metadata=excluded.metadata", params![data.id,json])?;
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
    })
}

/// The database on this device.
pub fn path() -> Option<PathBuf> {
    Some(
        crate::config::project_dirs()?
            .data_local_dir()
            .join("timeslay.sqlite3"),
    )
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
        event.id = ids::event_id(id);
        event.calendar_id = "tasks:test".into();
        event.details = Default::default();
        event
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
    #[test]
    fn schema_2_is_upgraded_keeping_its_tasks() {
        let path = path("schema-2-upgrade");
        Database::open(&path).unwrap();
        {
            // Put the file back as schema 2 left it: `tasks` with a unique
            // `legacy_key`, an alias pointing at it, and the old write queue.
            let db = Connection::open(&path).unwrap();
            db.execute_batch(
                "DROP TABLE task_aliases; DROP TABLE tasks;
                 CREATE TABLE tasks (id INTEGER PRIMARY KEY, metadata TEXT NOT NULL, legacy_key TEXT UNIQUE);
                 CREATE TABLE task_aliases (source TEXT PRIMARY KEY, task_id INTEGER NOT NULL REFERENCES tasks(id));
                 INSERT INTO tasks VALUES (100,'{}','old'),(101,'{}',NULL);
                 INSERT INTO task_aliases VALUES ('a',100),('b',101);
                 CREATE TABLE google_operations (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, payload TEXT NOT NULL, entity TEXT NOT NULL, created_at TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, error TEXT, lease_until INTEGER NOT NULL DEFAULT 0, version INTEGER NOT NULL DEFAULT 1, dead_at TEXT);
                 CREATE TABLE google_operation_progress (operation_id INTEGER PRIMARY KEY REFERENCES google_operations(id) ON DELETE CASCADE, data TEXT NOT NULL);
                 ALTER TABLE google_entities DROP COLUMN sending_since;
                 PRAGMA user_version = 2;",
            )
            .unwrap();
        }
        let mut db = Database::open(&path).unwrap();
        let connection = db.connection();
        assert_eq!(schema_version(connection).unwrap(), SCHEMA_VERSION);
        let columns: Vec<String> = connection
            .prepare("SELECT name FROM pragma_table_info('tasks')")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(columns, ["id", "metadata"]);
        let aliases: Vec<(String, i64)> = connection
            .prepare("SELECT a.source,t.id FROM task_aliases a JOIN tasks t ON t.id=a.task_id ORDER BY a.source")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(aliases, [("a".into(), 100), ("b".into(), 101)]);
        let foreign_keys: bool = connection
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert!(foreign_keys);
        drop(db);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn upgrading_drops_the_old_write_queue_and_its_unsent_edits() {
        let path = path("queue-upgrade");
        Database::open(&path).unwrap();
        {
            // Put the file back as schema 3 left it: a queue holding one
            // unsent edit, and an edit Google had already confirmed.
            let db = Connection::open(&path).unwrap();
            db.execute_batch(
                "CREATE TABLE google_operations (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, payload TEXT NOT NULL, entity TEXT NOT NULL, created_at TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, error TEXT, lease_until INTEGER NOT NULL DEFAULT 0, version INTEGER NOT NULL DEFAULT 1, dead_at TEXT);
                 CREATE TABLE google_operation_progress (operation_id INTEGER PRIMARY KEY REFERENCES google_operations(id) ON DELETE CASCADE, data TEXT NOT NULL);
                 CREATE TRIGGER timeslay_change_google_operations_insert AFTER INSERT ON google_operations BEGIN SELECT 1; END;
                 INSERT INTO google_operations(kind,payload,entity,created_at) VALUES('Edit event','{}','cal
unsent','now');
                 INSERT INTO google_entities(id,event,acknowledged_at) VALUES('cal
unsent','{}',NULL);
                 INSERT INTO google_entities(id,event,acknowledged_at) VALUES('cal
confirmed','{}','2026-01-01T00:00:00+00:00');
                 ALTER TABLE google_entities DROP COLUMN sending_since;
                 PRAGMA user_version = 3;",
            )
            .unwrap();
        }
        let mut db = Database::open(&path).unwrap();
        let connection = db.connection();
        assert_eq!(schema_version(connection).unwrap(), SCHEMA_VERSION);
        let tables: i64 = connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name LIKE 'google_operation%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
        let kept: Vec<String> = connection
            .prepare("SELECT id FROM google_entities WHERE sending_since IS NULL")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(kept, ["cal\nconfirmed"]);
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
