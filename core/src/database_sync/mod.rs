//! Durable SQLite edit journal -> native Turso replica -> SQLite projection.
//! The existing SQLite file remains readable by the Android widget and older
//! local processes. Only the worker holding the OS lock opens the Turso replica.
//! Credentials, Google caches and device preferences never enter replication.
pub mod model;
pub(crate) mod projection;
use model::Edit;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::{
    fs, io,
    path::Path,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

const TABLE: &str = "timeslay_revisions_v1";
const SHARED: &str = "'habits','notes','shared.todos','shared.notepad','shared.brain_dump','shared.projects','shared.task_work'";
static STARTED: OnceLock<()> = OnceLock::new();
static STATUS: Mutex<String> = Mutex::new(String::new());
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// What sync did, at the level a person cares about, newest last.
static LOG: Mutex<Vec<LogEntry>> = Mutex::new(Vec::new());
static LOG_SEQUENCE: AtomicU64 = AtomicU64::new(0);
/// How many entries [`LOG`] keeps.
const LOG_LIMIT: usize = 100;

/// One thing sync did, such as sending three edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Increases with every entry, so a reader can ask for what is new.
    pub sequence: u64,
    pub at: chrono::DateTime<chrono::Local>,
    pub text: String,
}

/// Entries written after `sequence`, oldest first.
pub fn log_since(sequence: u64) -> Vec<LogEntry> {
    LOG.lock()
        .map(|log| {
            log.iter()
                .filter(|entry| entry.sequence > sequence)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn log_activity(text: String) {
    if let Ok(mut log) = LOG.lock() {
        let sequence = LOG_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
        log.push(LogEntry {
            sequence,
            at: crate::clock::now(),
            text,
        });
        let excess = log.len().saturating_sub(LOG_LIMIT);
        log.drain(..excess);
    }
}

/// Local edits not yet sent to the sync server. Any process can ask, not
/// only the one running sync.
pub fn unsent_edits() -> usize {
    crate::task_db::path()
        .and_then(|path| {
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()
        })
        .and_then(|db| {
            db.query_row(
                "SELECT COUNT(*) FROM timeslay_sync_edits WHERE uploaded=0",
                [],
                |r| r.get::<_, i64>(0),
            )
            .ok()
        })
        .map_or(0, |count| usize::try_from(count).unwrap_or(0))
}

/// Whether sync is currently stuck: connecting, offline, or waiting to retry.
/// Edits are kept locally meanwhile.
pub fn waiting() -> bool {
    let status = status();
    status.starts_with("Database sync waiting")
        || status.starts_with("Database sync offline")
        || status.starts_with("Database sync connecting")
        || status.starts_with("Database sync starting")
}

/// "3 habit and note edits": a count and what the edits were to.
fn describe_edits(edits: &[&Edit]) -> String {
    let mut kinds: Vec<&str> = Vec::new();
    for edit in edits {
        let kind = match edit.document.as_str() {
            "habits" => "habit",
            "notes" => "note",
            "shared.todos" => "todo",
            "shared.notepad" => "notepad",
            "shared.brain_dump" => "brain dump",
            "shared.projects" => "project",
            "shared.task_work" => "task work",
            _ => "data",
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    let what = match kinds.as_slice() {
        [] => String::new(),
        [one] => format!("{one} "),
        [rest @ .., last] => format!("{} and {last} ", rest.join(", ")),
    };
    let plural = if edits.len() == 1 { "" } else { "s" };
    format!("{} {what}edit{plural}", edits.len())
}

pub fn generation() -> u64 {
    crate::task_db::path()
        .and_then(|path| {
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).ok()
        })
        .and_then(|db| {
            db.query_row(
                "SELECT generation FROM timeslay_sync_control WHERE singleton=1",
                [],
                |r| r.get::<_, u32>(0),
            )
            .ok()
        })
        .map(u64::from)
        .unwrap_or_else(|| GENERATION.load(Ordering::Relaxed))
}
pub fn status() -> String {
    STATUS
        .lock()
        .map(|s| s.clone())
        .unwrap_or_else(|_| "Database sync unavailable".into())
}
/// Returns whether the status changed.
fn set_status(message: String) -> bool {
    if let Ok(mut status) = STATUS.lock()
        && *status != message
    {
        log::info!("{message}");
        *status = message;
        return true;
    }
    false
}
/// A failure goes in the log once, not on every retry.
fn set_failure(message: String) {
    if set_status(message.clone()) {
        log_activity(message);
    }
}
fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}
fn open(path: &Path) -> io::Result<Connection> {
    let db = Connection::open(path).map_err(other)?;
    db.busy_timeout(Duration::from_secs(10)).map_err(other)?;
    Ok(db)
}
/// Called only after legacy file and credential handling has finished. Adding
/// the journal, triggers and initial records is one atomic transaction.
pub fn enroll(path: &Path) -> io::Result<()> {
    let mut db = open(path)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(other)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS timeslay_sync_control(singleton INTEGER PRIMARY KEY CHECK(singleton=1), projecting INTEGER NOT NULL DEFAULT 0, generation INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS timeslay_sync_edits(id TEXT PRIMARY KEY, document TEXT NOT NULL, before_json TEXT, after_json TEXT, clock INTEGER NOT NULL, uploaded INTEGER NOT NULL DEFAULT 0);").map_err(other)?;
    let enrolled = tx
        .query_row(
            "SELECT 1 FROM timeslay_sync_control WHERE singleton=1",
            [],
            |r| r.get::<_, i64>(0),
        )
        .optional()
        .map_err(other)?
        .is_some();
    if !enrolled {
        tx.execute("INSERT INTO timeslay_sync_control VALUES(1,0,0)", [])
            .map_err(other)?;
        tx.execute_batch(&format!("INSERT INTO timeslay_sync_edits SELECT lower(hex(randomblob(16))),key,NULL,value,1,0 FROM documents WHERE key IN ({SHARED});")).map_err(other)?;
    }
    for (name, kind, condition, key, before, after) in [
        ("insert", "INSERT", "1", "NEW.key", "NULL", "NEW.value"),
        (
            "update",
            "UPDATE",
            "OLD.value IS NOT NEW.value",
            "NEW.key",
            "OLD.value",
            "NEW.value",
        ),
        ("delete", "DELETE", "1", "OLD.key", "OLD.value", "NULL"),
    ] {
        tx.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS timeslay_sync_{name} AFTER {kind} ON documents
          WHEN {key} IN ({SHARED}) AND ({condition}) AND (SELECT projecting FROM timeslay_sync_control WHERE singleton=1)=0
          BEGIN INSERT INTO timeslay_sync_edits VALUES(lower(hex(randomblob(16))),{key},{before},{after},(SELECT coalesce(max(clock),0)+1 FROM timeslay_sync_edits),0); END;")).map_err(other)?;
    }
    tx.commit().map_err(other)?;
    crate::fsutil::make_private(path)
}
fn edits(db: &Connection, pending: bool) -> io::Result<Vec<Edit>> {
    let sql = if pending {
        "SELECT id,document,before_json,after_json,clock FROM timeslay_sync_edits WHERE uploaded=0 ORDER BY clock,id"
    } else {
        "SELECT id,document,before_json,after_json,clock FROM timeslay_sync_edits ORDER BY clock,id"
    };
    db.prepare(sql)
        .map_err(other)?
        .query_map([], |r| {
            Ok(Edit {
                id: r.get(0)?,
                document: r.get(1)?,
                before: r.get(2)?,
                after: r.get(3)?,
                clock: r.get(4)?,
            })
        })
        .map_err(other)?
        .collect::<Result<_, _>>()
        .map_err(other)
}
fn shared(key: &str) -> bool {
    matches!(
        key,
        "habits"
            | "notes"
            | "shared.todos"
            | "shared.notepad"
            | "shared.brain_dump"
            | "shared.projects"
            | "shared.task_work"
    )
}
/// Ingest and materialize under the same writer lock as ordinary app saves.
/// Saves made during the network request are included, rather than overwritten.
/// Returns the incoming edits that were new here.
fn apply<'a>(path: &Path, incoming: &'a [Edit], uploaded: &[Edit]) -> io::Result<Vec<&'a Edit>> {
    let mut received = Vec::new();
    let mut db = open(path)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(other)?;
    for edit in incoming {
        if !shared(&edit.document) || edit.clock < 1 {
            return Err(other("unsupported replicated record"));
        }
        let existing: Option<(String, Option<String>, Option<String>, i64)> = tx
            .query_row(
                "SELECT document,before_json,after_json,clock FROM timeslay_sync_edits WHERE id=?1",
                [&edit.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(other)?;
        if let Some(old) = existing {
            if old
                != (
                    edit.document.clone(),
                    edit.before.clone(),
                    edit.after.clone(),
                    edit.clock,
                )
            {
                return Err(other("replication ID collision; original data retained"));
            }
        } else {
            tx.execute(
                "INSERT INTO timeslay_sync_edits VALUES(?1,?2,?3,?4,?5,1)",
                params![edit.id, edit.document, edit.before, edit.after, edit.clock],
            )
            .map_err(other)?;
            received.push(edit);
        }
    }
    let projection = model::project(&edits(&tx, false)?).map_err(other)?;
    let mut changed = false;
    tx.execute(
        "UPDATE timeslay_sync_control SET projecting=1 WHERE singleton=1",
        [],
    )
    .map_err(other)?;
    for (key, value) in projection {
        let old: Option<String> = tx
            .query_row("SELECT value FROM documents WHERE key=?1", [&key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(other)?;
        if old == value {
            continue;
        }
        changed = true;
        match value {
            Some(value) => {
                tx.execute("INSERT INTO documents VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![key,value]).map_err(other)?;
            }
            None => {
                tx.execute("DELETE FROM documents WHERE key=?1", [key])
                    .map_err(other)?;
            }
        }
    }
    tx.execute(
        "UPDATE timeslay_sync_control SET projecting=0 WHERE singleton=1",
        [],
    )
    .map_err(other)?;
    for edit in uploaded {
        tx.execute(
            "UPDATE timeslay_sync_edits SET uploaded=1 WHERE id=?1",
            [&edit.id],
        )
        .map_err(other)?;
    }
    if changed {
        tx.execute(
            "UPDATE timeslay_sync_control SET generation=generation+1 WHERE singleton=1",
            [],
        )
        .map_err(other)?;
    }
    tx.commit().map_err(other)?;
    if changed {
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    Ok(received)
}
/// Export keeps every original and superseded version, including deletions.
/// Useful for conflict recovery; it never opens or changes the native replica.
pub fn history(path: &Path) -> io::Result<Vec<Edit>> {
    edits(&open(path)?, false)
}

pub fn start() {
    // Test runs must never push their fixtures to the user's sync server.
    if crate::config::isolated() {
        return;
    }
    STARTED.get_or_init(|| {
        set_status("Database sync starting…".into());
        if let Err(e) = std::thread::Builder::new()
            .name("timeslay-database-sync".into())
            .spawn(run)
        {
            set_failure(format!("Database sync could not start: {e}"));
        }
    });
}
fn run() {
    let Some(path) = crate::task_db::path() else {
        set_status("Database sync: no data directory".into());
        return;
    };
    let parent = path.parent().expect("database directory");
    let lock_path = parent.join("timeslay-sync.owner");
    loop {
        let url = crate::config::DatabaseSyncSettings::load()
            .url
            .trim()
            .to_owned();
        if url.is_empty() {
            set_status("Database sync disabled; configure a server URL in Settings".into());
            std::thread::sleep(Duration::from_secs(5));
            continue;
        }
        let attempt = (|| -> io::Result<()> {
            let lock = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(&lock_path)?;
            crate::fsutil::make_private(&lock_path)?;
            // Unlike fsutil::locked, failure must never permit two owners.
            if let Err(error) = lock.try_lock() {
                set_status("Database sync running in another TimeSlay process".into());
                return Err(other(error));
            }
            set_status("Database sync connecting…".into());
            enroll(&path)?;
            let _ = rustls::crypto::ring::default_provider().install_default();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(transport(&path, &url))?;
            drop(lock);
            Ok(())
        })();
        if let Err(e) = attempt
            && status() != "Database sync running in another TimeSlay process"
        {
            set_failure(format!("Database sync waiting; local data saved: {e}"));
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}
async fn transport(path: &Path, url: &str) -> io::Result<()> {
    let replica_directory = path
        .parent()
        .ok_or_else(|| other("missing database directory"))?
        .join("turso");
    fs::create_dir_all(&replica_directory)?;
    crate::fsutil::make_private(&replica_directory)?;
    let replica_path = replica_directory.join("replica.db");
    let future = turso::sync::Builder::new_remote(
        replica_path
            .to_str()
            .ok_or_else(|| other("invalid database path"))?,
    )
    .with_remote_url(url)
    .build();
    let replica = tokio::time::timeout(Duration::from_secs(30), future)
        .await
        .map_err(other)?
        .map_err(other)?;
    crate::fsutil::make_private(&replica_path)?;
    let conn = replica.connect().await.map_err(other)?;
    conn.execute(
        &format!("CREATE TABLE IF NOT EXISTS {TABLE}(id TEXT PRIMARY KEY,payload TEXT NOT NULL)"),
        (),
    )
    .await
    .map_err(other)?;
    loop {
        if crate::config::DatabaseSyncSettings::load().url.trim() != url {
            return Ok(());
        }
        let cycle = async {
            replica.pull().await.map_err(other)?;
            let pending = edits(&open(path)?, true)?;
            // Validate before sending anything, so corrupt input never replaces
            // a good remote projection and can be repaired in place.
            model::project(&pending).map_err(other)?;
            for edit in &pending {
                conn.execute(
                    &format!("INSERT OR IGNORE INTO {TABLE} VALUES(?1,?2)"),
                    (edit.id.clone(), serde_json::to_string(edit).map_err(other)?),
                )
                .await
                .map_err(other)?;
            }
            replica.push().await.map_err(other)?;
            replica.pull().await.map_err(other)?;
            let mut rows = conn
                .query(&format!("SELECT id,payload FROM {TABLE}"), ())
                .await
                .map_err(other)?;
            let mut incoming = Vec::new();
            while let Some(row) = rows.next().await.map_err(other)? {
                let id: String = row.get(0).map_err(other)?;
                let payload: String = row.get(1).map_err(other)?;
                let edit: Edit = serde_json::from_str(&payload).map_err(other)?;
                if edit.id != id {
                    return Err(other("replication payload identity mismatch"));
                }
                incoming.push(edit);
            }
            drop(rows);
            let received = apply(path, &incoming, &pending)?;
            replica.checkpoint().await.map_err(other)?;
            if !pending.is_empty() {
                let sent: Vec<&Edit> = pending.iter().collect();
                log_activity(format!("Sent {} to the cloud", describe_edits(&sent)));
            }
            if !received.is_empty() {
                log_activity(format!(
                    "Received {} from the cloud",
                    describe_edits(&received)
                ));
            }
            Ok::<_, io::Error>(())
        };
        match tokio::time::timeout(Duration::from_secs(45), cycle).await {
            Ok(Ok(())) => {
                if set_status("Database synced".into()) {
                    log_activity("Connected to database sync".into());
                }
            }
            Ok(Err(e)) => {
                set_failure(format!("Database sync offline; edits saved locally: {e}"));
            }
            Err(e) => return Err(other(e)),
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        crate::task_db::Database::open(&path).unwrap();
        (dir, path)
    }
    #[test]
    fn enrollment_and_projection_are_retryable_and_do_not_rejournal_remote_writes() {
        let (_d, path) = db();
        crate::task_db::write_document_json(&path, "habits", r#"{"habits":[]}"#).unwrap();
        enroll(&path).unwrap();
        enroll(&path).unwrap();
        assert_eq!(history(&path).unwrap().len(), 1);
        crate::task_db::write_document_json(&path, "habits", r#"{"habits":[{"id":"a"}]}"#).unwrap();
        let local = history(&path).unwrap();
        assert_eq!(local.len(), 2);
        let foreign = Edit {
            id: "foreign".into(),
            document: "habits".into(),
            before: Some(r#"{"habits":[]}"#.into()),
            after: Some(r#"{"habits":[{"id":"b"}]}"#.into()),
            clock: 2,
        };
        apply(&path, std::slice::from_ref(&foreign), &local).unwrap();
        apply(&path, &[foreign], &local).unwrap();
        assert_eq!(history(&path).unwrap().len(), 3);
        let data: serde_json::Value = crate::task_db::read_document(&path, "habits")
            .unwrap()
            .unwrap();
        assert_eq!(data["habits"].as_array().unwrap().len(), 2);
        assert!(edits(&open(&path).unwrap(), true).unwrap().is_empty());
    }
    #[test]
    fn credentials_caches_and_theme_are_not_replicated_and_deletions_are_durable() {
        let (_d, path) = db();
        enroll(&path).unwrap();
        for key in [
            "google_tokens",
            "gmail_accounts",
            "calendar_sync_state",
            "contacts",
            "routes",
            "dioxus_theme",
        ] {
            crate::task_db::write_document_json(&path, key, "{}").unwrap();
        }
        assert!(history(&path).unwrap().is_empty());
        crate::task_db::write_document_json(&path, "shared.notepad", r#"{"text":"keep"}"#).unwrap();
        crate::task_db::remove_document(&path, "shared.notepad").unwrap();
        let local = history(&path).unwrap();
        assert_eq!(local.len(), 2);
        assert!(local[1].after.is_none());
        apply(&path, &[], &local).unwrap();
        assert!(
            crate::task_db::read_document::<serde_json::Value>(&path, "shared.notepad")
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn malformed_remote_records_and_identity_collisions_roll_back() {
        let (_d, path) = db();
        enroll(&path).unwrap();
        crate::task_db::write_document_json(&path, "habits", "{}").unwrap();
        let mut conflicting = history(&path).unwrap()[0].clone();
        conflicting.after = Some("[]".into());
        assert!(apply(&path, &[conflicting], &[]).is_err());
        let bad = Edit {
            id: "bad".into(),
            document: "habits".into(),
            before: None,
            after: Some("not json".into()),
            clock: 2,
        };
        assert!(apply(&path, &[bad], &[]).is_err());
        assert_eq!(history(&path).unwrap().len(), 1);
    }
}
