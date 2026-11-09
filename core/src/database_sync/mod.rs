//! Durable SQLite edit journal -> native Turso replica -> SQLite projection.
//! The existing SQLite file remains readable by the Android widget and older
//! local processes. Only the worker holding the OS lock opens the Turso replica.
//! Credentials, Google caches and device preferences never enter replication.
//!
//! # Bounded history
//!
//! Every save of a shared document is journaled with the whole document before
//! and after, so the journal used to grow without limit and every sync cycle
//! replayed all of it. Now, once a document has more than [`KEEP`] +
//! [`BATCH`] edits on the server, the worker folds the oldest into a snapshot
//! (see [`model::plan_snapshot`]), which by construction projects to the same
//! document. The folded rows are kept for [`GRACE`] so that a snapshot made at
//! the same moment on another device cannot lose anything (see
//! [`model`]'s `lineage`), then deleted here and on the server. Their IDs stay
//! behind as tombstones for [`TOMBSTONE_LIFETIME`], so a stale copy arriving
//! late is recognized.
pub mod model;
pub(crate) mod projection;
use model::Edit;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs, io,
    path::Path,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

const TABLE: &str = "timeslay_revisions_v1";
/// The documents shared between devices. Everything else stays local.
const SHARED_KEYS: [&str; 6] = [
    "habits",
    "alarms",
    "shared.notepad",
    "shared.brain_dump",
    "shared.projects",
    "shared.task_work",
];
/// Edits per document left unfolded, for recovery and so a fresh save always
/// has recent history to merge against.
const KEEP: usize = 20;
/// How many edits beyond [`KEEP`] build up before a fold, so folding is a
/// batch job rather than something every save sets off.
const BATCH: usize = 20;
/// How long folded rows stay after their snapshot is the only one, in seconds.
/// It only needs to outlast the time for a concurrent snapshot to reach every
/// online device, which is a few sync cycles.
const GRACE: i64 = 60 * 60;
/// How long the ID of a deleted row is remembered, in seconds.
const TOMBSTONE_LIFETIME: i64 = 30 * 24 * 60 * 60;

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
                &format!("SELECT COUNT(*) FROM timeslay_sync_edits WHERE uploaded=0 AND document IN ({})", shared_sql()),
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
            "alarms" => "alarm",
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

/// The shared keys as an SQL list, for the journal triggers.
fn shared_sql() -> String {
    SHARED_KEYS
        .iter()
        .map(|key| format!("'{key}'"))
        .collect::<Vec<_>>()
        .join(",")
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
fn now() -> i64 {
    crate::clock::utc_now().timestamp()
}

/// The tables holding snapshots and the IDs of deleted rows. Idempotent.
pub(crate) fn install_snapshots(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS timeslay_sync_snapshots(id TEXT PRIMARY KEY, document TEXT NOT NULL, after_json TEXT, clock INTEGER NOT NULL, folds TEXT NOT NULL, uploaded INTEGER NOT NULL DEFAULT 0, seen_at INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS timeslay_sync_folded(id TEXT PRIMARY KEY, remote_deleted INTEGER NOT NULL DEFAULT 0, folded_at INTEGER NOT NULL);",
    )
}

/// Adds the journal, triggers and initial records in one atomic transaction.
pub fn enroll(path: &Path) -> io::Result<()> {
    let mut db = open(path)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(other)?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS timeslay_sync_control(singleton INTEGER PRIMARY KEY CHECK(singleton=1), projecting INTEGER NOT NULL DEFAULT 0, generation INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS timeslay_sync_edits(id TEXT PRIMARY KEY, document TEXT NOT NULL, before_json TEXT, after_json TEXT, clock INTEGER NOT NULL, uploaded INTEGER NOT NULL DEFAULT 0);").map_err(other)?;
    install_snapshots(&tx).map_err(other)?;
    let shared = shared_sql();
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
        tx.execute_batch(&format!("INSERT INTO timeslay_sync_edits SELECT lower(hex(randomblob(16))),key,NULL,value,1,0 FROM documents WHERE key IN ({shared});")).map_err(other)?;
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
        // Older installations journaled the retired scratchpad. Replace their
        // triggers so it stays local even when an older process writes it.
        let trigger = format!("timeslay_sync_{name}");
        let existing: Option<String> = tx
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
                [&trigger],
                |r| r.get(0),
            )
            .optional()
            .map_err(other)?;
        if existing.is_some_and(|sql| sql.contains("'shared.todos'")) {
            tx.execute_batch(&format!("DROP TRIGGER {trigger};"))
                .map_err(other)?;
        }
        // The clock also counts snapshots, so a save after a fold still sorts
        // after everything this device has seen.
        tx.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS timeslay_sync_{name} AFTER {kind} ON documents
          WHEN {key} IN ({shared}) AND ({condition}) AND (SELECT projecting FROM timeslay_sync_control WHERE singleton=1)=0
          BEGIN INSERT INTO timeslay_sync_edits VALUES(lower(hex(randomblob(16))),{key},{before},{after},(SELECT coalesce(max(clock),0)+1 FROM timeslay_sync_edits),0); END;")).map_err(other)?;
    }
    tx.commit().map_err(other)?;
    crate::fsutil::make_private(path)
}

/// Whether an edit has reached the server, and when a snapshot arrived here.
#[derive(Clone, Copy)]
struct Local {
    uploaded: bool,
    seen_at: Option<i64>,
}

/// Every journaled edit and snapshot, or only those not yet uploaded.
fn edits(db: &Connection, pending: bool) -> io::Result<Vec<Edit>> {
    Ok(rows(db, pending)?
        .into_iter()
        .map(|(edit, _)| edit)
        .collect())
}

fn rows(db: &Connection, pending: bool) -> io::Result<Vec<(Edit, Local)>> {
    // Keep retired history on disk, but never replay or upload it.
    let filter = format!(
        "WHERE document IN ({}){}",
        shared_sql(),
        if pending { " AND uploaded=0" } else { "" }
    );
    let mut out: Vec<(Edit, Local)> = db
        .prepare(&format!(
            "SELECT id,document,before_json,after_json,clock,uploaded FROM timeslay_sync_edits {filter} ORDER BY clock,id"
        ))
        .map_err(other)?
        .query_map([], |r| {
            Ok((
                Edit {
                    id: r.get(0)?,
                    document: r.get(1)?,
                    before: r.get(2)?,
                    after: r.get(3)?,
                    clock: r.get(4)?,
                    ..Default::default()
                },
                Local {
                    uploaded: r.get::<_, i64>(5)? != 0,
                    seen_at: None,
                },
            ))
        })
        .map_err(other)?
        .collect::<Result<_, _>>()
        .map_err(other)?;
    let snapshots = db
        .prepare(&format!(
            "SELECT id,document,after_json,clock,folds,uploaded,seen_at FROM timeslay_sync_snapshots {filter} ORDER BY clock,id"
        ))
        .map_err(other)?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
            ))
        })
        .map_err(other)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(other)?;
    for (id, document, after, clock, folds, uploaded, seen_at) in snapshots {
        out.push((
            Edit {
                id,
                document,
                before: None,
                after,
                clock,
                snapshot: true,
                folds: serde_json::from_str(&folds).map_err(other)?,
            },
            Local {
                uploaded: uploaded != 0,
                seen_at: Some(seen_at),
            },
        ));
    }
    Ok(out)
}

fn shared(key: &str) -> bool {
    SHARED_KEYS.contains(&key)
}

/// Every ID this device knows: journaled, snapshotted or deleted.
fn known_ids(db: &Connection) -> io::Result<HashSet<String>> {
    let mut known = HashSet::new();
    for table in [
        "timeslay_sync_edits",
        "timeslay_sync_snapshots",
        "timeslay_sync_folded",
    ] {
        let mut q = db
            .prepare(&format!("SELECT id FROM {table}"))
            .map_err(other)?;
        for id in q.query_map([], |r| r.get::<_, String>(0)).map_err(other)? {
            known.insert(id.map_err(other)?);
        }
    }
    Ok(known)
}

fn delete_row(tx: &Connection, id: &str, remote_deleted: bool, at: i64) -> io::Result<()> {
    tx.execute("DELETE FROM timeslay_sync_edits WHERE id=?1", [id])
        .map_err(other)?;
    tx.execute("DELETE FROM timeslay_sync_snapshots WHERE id=?1", [id])
        .map_err(other)?;
    tx.execute(
        "INSERT INTO timeslay_sync_folded(id,remote_deleted,folded_at) VALUES(?1,?2,?3) ON CONFLICT(id) DO NOTHING",
        params![id, remote_deleted, at],
    )
    .map_err(other)?;
    Ok(())
}

/// What a sync cycle has to do on the server after [`apply`].
#[derive(Debug, Default)]
struct Applied {
    changed: bool,
    /// IDs folded away here that the server may still hold.
    remote_deletions: Vec<String>,
    /// Incoming edits that were new here.
    received: Vec<Edit>,
}

/// Ingest, tidy and materialize under the same writer lock as ordinary app
/// saves. Saves made during the network request are included, rather than
/// overwritten.
///
/// `remote` is every ID on the server, when the caller has just read it. The
/// tidying that depends on knowing the server's contents (dropping stale rows,
/// settling and making snapshots) only happens then.
fn apply(
    path: &Path,
    incoming: &[Edit],
    remote: Option<&HashSet<String>>,
    at: i64,
) -> io::Result<Applied> {
    let mut received = Vec::new();
    let mut db = open(path)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(other)?;
    let tombstones: HashSet<String> = {
        let mut q = tx
            .prepare("SELECT id FROM timeslay_sync_folded")
            .map_err(other)?;
        q.query_map([], |r| r.get::<_, String>(0))
            .map_err(other)?
            .collect::<Result<_, _>>()
            .map_err(other)?
    };
    for edit in incoming {
        // Old devices and servers can still hold scratchpad records. They
        // must not prevent the remaining documents from syncing.
        if edit.document == "shared.todos" {
            continue;
        }
        if !shared(&edit.document) || edit.clock < 1 {
            return Err(other("unsupported replicated record"));
        }
        if tombstones.contains(&edit.id) {
            continue;
        }
        if edit.snapshot {
            let folds = serde_json::to_string(&edit.folds).map_err(other)?;
            let existing: Option<(String, Option<String>, i64, String)> = tx
                .query_row(
                    "SELECT document,after_json,clock,folds FROM timeslay_sync_snapshots WHERE id=?1",
                    [&edit.id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()
                .map_err(other)?;
            match existing {
                Some(old)
                    if old
                        != (
                            edit.document.clone(),
                            edit.after.clone(),
                            edit.clock,
                            folds.clone(),
                        ) =>
                {
                    return Err(other("replication ID collision; original data retained"));
                }
                Some(_) => {}
                None => {
                    tx.execute(
                        "INSERT INTO timeslay_sync_snapshots(id,document,after_json,clock,folds,uploaded,seen_at) VALUES(?1,?2,?3,?4,?5,1,?6)",
                        params![edit.id, edit.document, edit.after, edit.clock, folds, at],
                    )
                    .map_err(other)?;
                }
            }
            continue;
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
            received.push(edit.clone());
        }
    }

    let mut applied = Applied {
        received,
        ..Applied::default()
    };
    if let Some(remote) = remote {
        tidy(&tx, remote, at)?;
    }

    let all = edits(&tx, false)?;
    let projection = model::project(&all).map_err(other)?;
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
        applied.changed = true;
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
    if applied.changed {
        tx.execute(
            "UPDATE timeslay_sync_control SET generation=generation+1 WHERE singleton=1",
            [],
        )
        .map_err(other)?;
    }

    if let Some(remote) = remote {
        compact(&tx, remote, at)?;
    }
    applied.remote_deletions = {
        let mut q = tx
            .prepare("SELECT id FROM timeslay_sync_folded WHERE remote_deleted=0")
            .map_err(other)?;
        q.query_map([], |r| r.get::<_, String>(0))
            .map_err(other)?
            .collect::<Result<_, _>>()
            .map_err(other)?
    };
    tx.commit().map_err(other)?;
    if applied.changed {
        GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    Ok(applied)
}

/// Drops what the server's contents show to be stale or settled.
fn tidy(tx: &Connection, remote: &HashSet<String>, at: i64) -> io::Result<()> {
    let all = rows(tx, false)?;
    let edits_only: Vec<Edit> = all.iter().map(|(e, _)| e.clone()).collect();
    let local: HashMap<&str, Local> = all.iter().map(|(e, l)| (e.id.as_str(), *l)).collect();
    let documents: BTreeSet<&str> = edits_only.iter().map(|e| e.document.as_str()).collect();

    // A save that reached the server but was not marked as uploaded (the
    // worker stopped in between) may since have been folded. It must not be
    // sent again, or it would reappear after its fold.
    let folded_anywhere: HashSet<&str> = edits_only
        .iter()
        .filter(|e| e.snapshot)
        .flat_map(|e| e.folds.iter().map(String::as_str))
        .collect();
    for (edit, state) in &all {
        if !state.uploaded && folded_anywhere.contains(edit.id.as_str()) {
            tx.execute(
                "UPDATE timeslay_sync_edits SET uploaded=1 WHERE id=?1",
                [&edit.id],
            )
            .map_err(other)?;
        }
    }

    for document in documents {
        let heads = model::heads(&edits_only, document);
        // A snapshot made here that lost a race to one already on the server
        // is withdrawn before it is ever sent: what it folded is still here.
        for head in &heads {
            let unsent = local.get(head.id.as_str()).is_some_and(|l| !l.uploaded);
            let rival = heads
                .iter()
                .any(|other| other.id != head.id && !head.folds.contains(&other.id));
            if unsent && rival {
                tx.execute(
                    "DELETE FROM timeslay_sync_snapshots WHERE id=?1",
                    [&head.id],
                )
                .map_err(other)?;
            }
        }

        // Rows that were on the server and are gone from it, older than a
        // snapshot that is there, were folded on another device. A server
        // without snapshots (a new sync URL, say) deletes nothing.
        let newest_remote_head = heads
            .iter()
            .filter(|h| remote.contains(&h.id))
            .map(|h| h.clock)
            .max();
        if let Some(horizon) = newest_remote_head {
            for (edit, state) in &all {
                if edit.document == document
                    && state.uploaded
                    && !remote.contains(&edit.id)
                    && edit.clock <= horizon
                {
                    delete_row(tx, &edit.id, true, at)?;
                }
            }
        }

        // Once a snapshot has been the only one on the server for the grace
        // period, what it covers is deleted here and queued for the server.
        if let [head] = heads.as_slice()
            && remote.contains(&head.id)
            && local
                .get(head.id.as_str())
                .and_then(|l| l.seen_at)
                .is_some_and(|seen| seen <= at - GRACE)
        {
            for id in model::covered(&edits_only, document) {
                delete_row(tx, &id, false, at)?;
            }
        }
    }
    tx.execute(
        "DELETE FROM timeslay_sync_folded WHERE remote_deleted=1 AND folded_at < ?1",
        [at - TOMBSTONE_LIFETIME],
    )
    .map_err(other)?;
    Ok(())
}

/// Folds the oldest edits of any document over the threshold into a snapshot,
/// to be uploaded by the cycle that called this.
fn compact(tx: &Connection, remote: &HashSet<String>, at: i64) -> io::Result<()> {
    let all = rows(tx, false)?;
    let edits_only: Vec<Edit> = all.iter().map(|(e, _)| e.clone()).collect();
    let unsent_snapshot: HashSet<&str> = all
        .iter()
        .filter(|(e, l)| e.snapshot && !l.uploaded)
        .map(|(e, _)| e.document.as_str())
        .collect();
    let documents: BTreeSet<&str> = edits_only.iter().map(|e| e.document.as_str()).collect();
    for document in documents {
        if unsent_snapshot.contains(document) {
            continue;
        }
        let covered: HashSet<String> = model::covered(&edits_only, document).into_iter().collect();
        let live = edits_only
            .iter()
            .filter(|e| e.document == document && !e.snapshot && !covered.contains(&e.id))
            .count();
        if live <= KEEP + BATCH {
            continue;
        }
        let plan = model::plan_snapshot(&edits_only, document, KEEP, |edit| {
            remote.contains(&edit.id)
        })
        .map_err(other)?;
        let Some(mut snapshot) = plan else { continue };
        snapshot.id = format!(
            "snapshot-{}",
            crate::calendar::operations::new_id().map_err(other)?
        );
        tx.execute(
            "INSERT INTO timeslay_sync_snapshots(id,document,after_json,clock,folds,uploaded,seen_at) VALUES(?1,?2,?3,?4,?5,0,?6)",
            params![
                snapshot.id,
                snapshot.document,
                snapshot.after,
                snapshot.clock,
                serde_json::to_string(&snapshot.folds).map_err(other)?,
                at
            ],
        )
        .map_err(other)?;
        log::info!(
            "Database sync: folded {} edits of {document} into {}",
            snapshot.folds.len(),
            snapshot.id
        );
    }
    Ok(())
}

/// Records that `sent` reached the server and `deleted` left it.
fn mark_sent(path: &Path, sent: &[Edit], deleted: &[String]) -> io::Result<()> {
    let mut db = open(path)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(other)?;
    for edit in sent {
        let table = if edit.snapshot {
            "timeslay_sync_snapshots"
        } else {
            "timeslay_sync_edits"
        };
        tx.execute(
            &format!("UPDATE {table} SET uploaded=1 WHERE id=?1"),
            [&edit.id],
        )
        .map_err(other)?;
    }
    for id in deleted {
        tx.execute(
            "UPDATE timeslay_sync_folded SET remote_deleted=1 WHERE id=?1",
            [id],
        )
        .map_err(other)?;
    }
    tx.commit().map_err(other)
}

/// Export keeps the snapshots and every edit not yet folded into one,
/// including deletions. Useful for conflict recovery; it never opens or
/// changes the native replica.
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

/// Every ID on the server, and the records this device has never seen.
///
/// Only the IDs are read every cycle; payloads are fetched for new IDs alone,
/// so an idle cycle costs one indexed scan rather than parsing all history.
async fn read_remote(
    conn: &turso::Connection,
    known: &HashSet<String>,
) -> io::Result<(HashSet<String>, Vec<Edit>)> {
    let mut remote = HashSet::new();
    let mut rows = conn
        .query(&format!("SELECT id FROM {TABLE}"), ())
        .await
        .map_err(other)?;
    while let Some(row) = rows.next().await.map_err(other)? {
        remote.insert(row.get::<String>(0).map_err(other)?);
    }
    drop(rows);
    let mut incoming = Vec::new();
    for id in remote.iter().filter(|id| !known.contains(*id)) {
        let mut rows = conn
            .query(
                &format!("SELECT payload FROM {TABLE} WHERE id=?1"),
                [id.clone()],
            )
            .await
            .map_err(other)?;
        if let Some(row) = rows.next().await.map_err(other)? {
            let payload: String = row.get(0).map_err(other)?;
            let edit: Edit = serde_json::from_str(&payload).map_err(other)?;
            if edit.id != *id {
                return Err(other("replication payload identity mismatch"));
            }
            incoming.push(edit);
        }
    }
    Ok((remote, incoming))
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
            let known = known_ids(&open(path)?)?;
            let (remote, incoming) = read_remote(&conn, &known).await?;
            // Ingest before sending, so a snapshot made here that lost a race
            // is withdrawn instead of sent.
            let applied = apply(path, &incoming, Some(&remote), now())?;
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
            for id in &applied.remote_deletions {
                conn.execute(&format!("DELETE FROM {TABLE} WHERE id=?1"), [id.clone()])
                    .await
                    .map_err(other)?;
            }
            if !pending.is_empty() || !applied.remote_deletions.is_empty() {
                replica.push().await.map_err(other)?;
            }
            mark_sent(path, &pending, &applied.remote_deletions)?;
            replica.checkpoint().await.map_err(other)?;
            if !pending.is_empty() {
                let sent: Vec<&Edit> = pending.iter().collect();
                log_activity(format!("Sent {} to the cloud", describe_edits(&sent)));
            }
            if !applied.received.is_empty() {
                let received: Vec<&Edit> = applied.received.iter().collect();
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
    /// Plays the server: everything uploaded so far, as a sync cycle sends it.
    fn send(path: &Path, server: &mut HashMap<String, Edit>) {
        let applied = apply(path, &[], Some(&server.keys().cloned().collect()), now()).unwrap();
        let pending = edits(&open(path).unwrap(), true).unwrap();
        for edit in &pending {
            server
                .entry(edit.id.clone())
                .or_insert_with(|| edit.clone());
        }
        for id in &applied.remote_deletions {
            server.remove(id);
        }
        mark_sent(path, &pending, &applied.remote_deletions).unwrap();
    }
    fn habits(path: &Path) -> serde_json::Value {
        crate::task_db::read_document(path, "habits")
            .unwrap()
            .unwrap()
    }
    fn save_habits(path: &Path, n: usize) {
        let marks: serde_json::Map<String, serde_json::Value> =
            (0..n).map(|i| (format!("d{i}"), i.into())).collect();
        let doc = serde_json::json!({"habits":[{"id":"a","marks":marks}]});
        crate::task_db::write_document(path, "habits", &doc).unwrap();
    }
    #[test]
    fn retired_scratchpad_history_and_old_clients_do_not_affect_sync() {
        let (_directory, path) = db();
        let db = open(&path).unwrap();
        // Simulate an existing database with the old trigger and unsent data.
        let sql: String = db
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='timeslay_sync_insert'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        db.execute_batch("DROP TRIGGER timeslay_sync_insert;")
            .unwrap();
        db.execute_batch(&sql.replace("'habits'", "'shared.todos','habits'"))
            .unwrap();
        db.execute(
            "INSERT INTO documents VALUES('shared.todos',?1)",
            ["{\"items\":[{\"text\":\"old\",\"indent\":0}],\"current\":0}"],
        )
        .unwrap();
        enroll(&path).unwrap();
        assert!(edits(&db, true).unwrap().is_empty());
        db.execute("DELETE FROM documents WHERE key='shared.todos'", [])
            .unwrap();
        db.execute("INSERT INTO documents VALUES('shared.todos','{}')", [])
            .unwrap();
        let count: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM timeslay_sync_edits WHERE document='shared.todos'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "retired documents no longer create journal entries"
        );

        let legacy = Edit {
            id: "old-device".into(),
            document: "shared.todos".into(),
            after: Some("unreadable retired data".into()),
            clock: 1,
            ..Default::default()
        };
        let active = Edit {
            id: "active".into(),
            document: "shared.notepad".into(),
            after: Some(r#"{"text":"still syncing"}"#.into()),
            clock: 1,
            ..Default::default()
        };
        apply(&path, &[legacy, active.clone()], None, now()).unwrap();
        assert_eq!(history(&path).unwrap(), vec![active]);
        let text: serde_json::Value = crate::task_db::read_document(&path, "shared.notepad")
            .unwrap()
            .unwrap();
        assert_eq!(text["text"], "still syncing");
    }
    #[test]
    fn alarms_merge_offline_additions_edits_and_deletions_between_devices() {
        use crate::alarms::{self, Alarm, AlarmProfile, Library};
        let (_a, first) = db();
        let (_b, second) = db();
        let at = crate::clock::now() + chrono::Duration::hours(1);
        let make = |label: &str| Alarm {
            id: alarms::new_id(),
            at,
            label: label.into(),
            profile: AlarmProfile::default(),
        };
        let a = make("Desktop");
        let b = make("Phone");
        assert_ne!(a.id, b.id);
        alarms::write(
            &first,
            &Library {
                alarms: vec![a.clone()],
            },
        )
        .unwrap();
        alarms::write(
            &second,
            &Library {
                alarms: vec![b.clone()],
            },
        )
        .unwrap();
        let mut server = HashMap::new();
        send(&first, &mut server);
        send(&second, &mut server);
        let remote = server.values().cloned().collect::<Vec<_>>();
        apply(&first, &remote, None, now()).unwrap();
        apply(&second, &remote, None, now()).unwrap();
        assert_eq!(alarms::read(&first).unwrap().alarms.len(), 2);
        let mut desktop = alarms::read(&first).unwrap();
        desktop.alarms.retain(|item| item.id != a.id);
        let mut phone = alarms::read(&second).unwrap();
        phone
            .alarms
            .iter_mut()
            .find(|item| item.id == b.id)
            .unwrap()
            .label = "Edited on phone".into();
        alarms::write(&first, &desktop).unwrap();
        alarms::write(&second, &phone).unwrap();
        send(&first, &mut server);
        send(&second, &mut server);
        let remote = server.values().cloned().collect::<Vec<_>>();
        for path in [&first, &second] {
            apply(path, &remote, None, now()).unwrap();
            let result = alarms::read(path).unwrap();
            assert_eq!(result.alarms.len(), 1);
            assert_eq!(result.alarms[0].id, b.id);
            assert_eq!(result.alarms[0].label, "Edited on phone");
        }
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
        mark_sent(&path, &local, &[]).unwrap();
        let foreign = Edit {
            id: "foreign".into(),
            document: "habits".into(),
            before: Some(r#"{"habits":[]}"#.into()),
            after: Some(r#"{"habits":[{"id":"b"}]}"#.into()),
            clock: 2,
            ..Default::default()
        };
        apply(&path, std::slice::from_ref(&foreign), None, now()).unwrap();
        apply(&path, &[foreign], None, now()).unwrap();
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
        apply(&path, &[], None, now()).unwrap();
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
        assert!(apply(&path, &[conflicting], None, now()).is_err());
        let bad = Edit {
            id: "bad".into(),
            document: "habits".into(),
            before: None,
            after: Some("not json".into()),
            clock: 2,
            ..Default::default()
        };
        assert!(apply(&path, &[bad], None, now()).is_err());
        assert_eq!(history(&path).unwrap().len(), 1);
    }
    #[test]
    fn history_is_folded_once_on_the_server_and_deleted_after_the_grace_period() {
        let (_d, path) = db();
        enroll(&path).unwrap();
        let mut server = HashMap::new();
        let saves = KEEP + BATCH + 5;
        for i in 0..saves {
            save_habits(&path, i + 1);
            send(&path, &mut server);
        }
        let expected = habits(&path);
        let snapshots: Vec<Edit> = history(&path)
            .unwrap()
            .into_iter()
            .filter(|e| e.snapshot)
            .collect();
        assert_eq!(snapshots.len(), 1, "one fold once over the threshold");
        let snapshot = &snapshots[0];
        assert!(server.contains_key(&snapshot.id));
        assert_eq!(snapshot.folds.len(), KEEP + BATCH + 1 - KEEP);
        assert_eq!(habits(&path), expected);
        // Within the grace period nothing is deleted.
        assert_eq!(history(&path).unwrap().len(), saves + 1);
        // After it, the folded rows go, here and on the server.
        let applied = apply(
            &path,
            &[],
            Some(&server.keys().cloned().collect()),
            now() + GRACE + 1,
        )
        .unwrap();
        assert_eq!(applied.remote_deletions.len(), snapshot.folds.len());
        for id in &applied.remote_deletions {
            server.remove(id);
        }
        mark_sent(&path, &[], &applied.remote_deletions).unwrap();
        let left = saves - snapshot.folds.len() + 1;
        assert_eq!(history(&path).unwrap().len(), left);
        assert_eq!(server.len(), left);
        assert_eq!(habits(&path), expected);
        // The deletions are not sent again.
        let again = apply(
            &path,
            &[],
            Some(&server.keys().cloned().collect()),
            now() + GRACE + 2,
        )
        .unwrap();
        assert!(again.remote_deletions.is_empty());
        // New saves still merge and sort after the folded history.
        save_habits(&path, 100);
        apply(&path, &[], None, now()).unwrap();
        assert_eq!(
            habits(&path)["habits"][0]["marks"]
                .as_object()
                .unwrap()
                .len(),
            100
        );
        let newest = history(&path)
            .unwrap()
            .into_iter()
            .max_by_key(|e| e.clock)
            .unwrap();
        assert!(!newest.snapshot && newest.clock > snapshot.clock);
    }
    #[test]
    fn a_second_device_drops_rows_another_device_folded() {
        let (_d, first) = db();
        let (_e, second) = db();
        enroll(&first).unwrap();
        enroll(&second).unwrap();
        let mut server = HashMap::new();
        for i in 0..(KEEP + BATCH + 5) {
            save_habits(&first, i + 1);
            send(&first, &mut server);
        }
        // The second device catches up with everything, including history.
        let all: Vec<Edit> = server.values().cloned().collect();
        apply(&second, &all, None, now()).unwrap();
        let expected = habits(&first);
        assert_eq!(habits(&second), expected);
        let held = history(&second).unwrap().len();
        // The first deletes the folded rows after the grace period.
        let applied = apply(
            &first,
            &[],
            Some(&server.keys().cloned().collect()),
            now() + GRACE + 1,
        )
        .unwrap();
        assert!(!applied.remote_deletions.is_empty());
        for id in &applied.remote_deletions {
            server.remove(id);
        }
        // The second notices they left the server and drops its copies.
        apply(&second, &[], Some(&server.keys().cloned().collect()), now()).unwrap();
        let left = history(&second).unwrap().len();
        assert!(left < held);
        assert_eq!(left, server.len());
        assert_eq!(habits(&second), expected);
        // It does not send any of its stale copies back.
        assert!(edits(&open(&second).unwrap(), true).unwrap().is_empty());
    }
    #[test]
    fn a_new_empty_server_never_deletes_local_history() {
        let (_d, path) = db();
        enroll(&path).unwrap();
        let mut server = HashMap::new();
        for i in 0..(KEEP + BATCH + 5) {
            save_habits(&path, i + 1);
            send(&path, &mut server);
        }
        let held = history(&path).unwrap().len();
        let expected = habits(&path);
        // A different sync URL: none of the uploaded rows are there.
        apply(&path, &[], Some(&HashSet::new()), now() + GRACE + 1).unwrap();
        assert_eq!(history(&path).unwrap().len(), held);
        assert_eq!(habits(&path), expected);
    }
    #[test]
    fn an_unsent_snapshot_that_lost_a_race_is_withdrawn() {
        let (_d, path) = db();
        enroll(&path).unwrap();
        let mut server = HashMap::new();
        // Upload without the tidying a full cycle does, so nothing folds yet.
        for i in 0..(KEEP + BATCH + 5) {
            save_habits(&path, i + 1);
            let pending = edits(&open(&path).unwrap(), true).unwrap();
            for edit in &pending {
                server.insert(edit.id.clone(), edit.clone());
            }
            mark_sent(&path, &pending, &[]).unwrap();
        }
        let expected = habits(&path);
        // Make a snapshot here but do not send it.
        apply(&path, &[], Some(&server.keys().cloned().collect()), now()).unwrap();
        let mine: Vec<Edit> = edits(&open(&path).unwrap(), true).unwrap();
        assert_eq!(mine.len(), 1);
        assert!(mine[0].snapshot);
        // Meanwhile another device's snapshot reached the server.
        let mut theirs = mine[0].clone();
        theirs.id = "snapshot-theirs".into();
        theirs.folds.pop();
        server.insert(theirs.id.clone(), theirs.clone());
        apply(
            &path,
            &[theirs],
            Some(&server.keys().cloned().collect()),
            now(),
        )
        .unwrap();
        let pending = edits(&open(&path).unwrap(), true).unwrap();
        assert!(pending.iter().all(|e| e.id != mine[0].id));
        assert_eq!(habits(&path), expected);
    }
}
