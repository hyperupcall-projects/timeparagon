//! Durable task metadata and IDs. SQLite owns inter-process locking and recovery.
use crate::{
    calendar::Event,
    timeblocking::{Metadata, legacy_id, legacy_metadata},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskData {
    pub id: i64,
    pub metadata: Metadata,
    // Preserve keys already used by work sessions without keeping UUIDs in Google.
    pub legacy_key: Option<String>,
}

pub struct Database(Connection);
impl Database {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let db = Connection::open(path).map_err(|e| e.to_string())?;
        db.busy_timeout(Duration::from_secs(10))
            .map_err(|e| e.to_string())?;
        db.execute_batch("PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS counter (singleton INTEGER PRIMARY KEY CHECK(singleton=1), next_id INTEGER NOT NULL CHECK(next_id >= 100));
            INSERT OR IGNORE INTO counter VALUES (1,100);
            CREATE TABLE IF NOT EXISTS tasks (id INTEGER PRIMARY KEY, metadata TEXT NOT NULL, legacy_key TEXT UNIQUE);
            CREATE TABLE IF NOT EXISTS task_aliases (source TEXT PRIMARY KEY, task_id INTEGER NOT NULL REFERENCES tasks(id));")
            .map_err(|e| e.to_string())?;
        Ok(Self(db))
    }

    /// Allocate and record the mapping in one transaction. No ID escapes until commit.
    pub fn import(&mut self, event: &Event) -> Result<TaskData, String> {
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
            let id: i64 =
                tx.query_row("SELECT next_id FROM counter WHERE singleton=1", [], |r| {
                    r.get(0)
                })?;
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
            tx.execute("UPDATE tasks SET metadata=?1 WHERE id=?2", params![json,data.id])?;
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
pub fn open() -> Result<Database, String> {
    let directory = crate::config::project_dirs().ok_or("Cannot locate local data directory")?;
    Database::open(&directory.data_local_dir().join("tasks.sqlite3"))
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
            .join("tasks.sqlite3")
    }
    fn event(id: &str) -> Event {
        let mut event = crate::calendar::sample_snapshot().events.remove(0);
        event.id = format!("task:{id}");
        event.calendar_id = "tasks:test".into();
        event.details = Default::default();
        event
    }
    #[test]
    fn sequential_persistent_ids_and_migration_preserve_metadata_and_history() {
        let path = path("migration");
        let mut db = Database::open(&path).unwrap();
        let mut original = event("old");
        original.details.description = "Human notes\ntt-timeblocking: {\"minutes\":45,\"block\":\"work/block\",\"order\":9}\nttid: 12345678-1234-4234-8234-123456789abc".into();
        let mut data = db.import(&original).unwrap();
        assert_eq!(data.id, 100);
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
        assert_eq!(db.import(&event("another")).unwrap().id, 101);
        // A failed transaction must not consume or publish an ID.
        {
            let tx =
                db.0.transaction_with_behavior(TransactionBehavior::Immediate)
                    .unwrap();
            tx.execute("UPDATE counter SET next_id=5000", []).unwrap();
        }
        assert_eq!(db.import(&event("after-rollback")).unwrap().id, 102);
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
    fn concurrent_processes_get_unique_consecutive_ids_and_one_shared_mapping() {
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
        assert_eq!((count, min, max), (81, 100, 180));
        assert_eq!(
            db.0.query_row("SELECT next_id FROM counter", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            181
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
