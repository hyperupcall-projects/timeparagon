//! Durable local edits. Acknowledging one operation never discards a newer edit.
use super::*;
use crate::{config::CalendarSettings, task_db, task_edit};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::{cell::Cell, collections::HashMap, sync::OnceLock, time::Duration};

thread_local! { static OPERATION_ID:Cell<Option<i64>>=const {Cell::new(None)};
    static REPLAY: Cell<bool> = const { Cell::new(false) };
static INSERT_ID: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) }; }
pub(super) fn insert_id() -> Option<String> {
    INSERT_ID.with(|id| id.borrow().clone())
}
pub(crate) fn operation_id() -> Option<i64> {
    OPERATION_ID.with(Cell::get)
}
pub fn replaying() -> bool {
    REPLAY.with(Cell::get)
}
#[derive(Clone, Serialize, Deserialize)]
pub enum Job {
    Edit {
        original: Box<Event>,
        draft: EditDraft,
        scope: SaveScope,
        description: String,
        color: Option<String>,
    },
    Time {
        calendar: String,
        id: String,
        start: DateTime<Local>,
        end: DateTime<Local>,
        notify: bool,
    },
    Location {
        calendar: String,
        id: String,
        location: String,
    },
    Status {
        task: Event,
        status: crate::timeblocking::TaskStatus,
    },
    TaskEdit {
        task: Event,
        change: task_edit::Change,
        order: i64,
    },
    Delete {
        calendar: String,
        id: String,
        scope: SaveScope,
        notify: bool,
    },
    DeleteTask {
        task: Event,
        calendar: Option<String>,
    },
    Create {
        id: String,
        calendar: String,
        title: String,
        description: String,
        color: Option<String>,
        start: DateTime<Local>,
        end: DateTime<Local>,
        recurrence: Vec<String>,
        projects: Vec<u64>,
        all_day: bool,
        options: Option<event_options::EventOptions>,
        meeting: Option<meeting::Form>,
    },
    Sleep {
        event: Event,
        new: bool,
    },
    HabitCompleted {
        event: Event,
    },
    Reply {
        event: Event,
        email: String,
        response: meeting::Response,
    },
    CreateHabit {
        id: String,
        draft: EditDraft,
        color: Option<String>,
        habit: String,
    },
    CreateTask {
        task: Box<Event>,
        notes: String,
        due: Option<NaiveDate>,
        metadata: Option<crate::timeblocking::Metadata>,
    },
}
/// One queued write, as the Operations screen shows it.
#[derive(Debug, Clone)]
pub struct Operation {
    pub id: i64,
    pub kind: String,
    pub entity: String,
    pub created_at: String,
    pub attempts: i64,
    pub error: Option<String>,
    pub payload: String,
    /// When it was given up on. Such a write stays listed, with its error,
    /// until it is retried or discarded; it no longer holds up the queue.
    pub failed_at: Option<String>,
    /// Seconds since the epoch before which it will not be tried again.
    pub retry_at: i64,
}

impl Operation {
    /// Still waiting to reach Google, as opposed to given up on.
    pub fn pending(&self) -> bool {
        self.failed_at.is_none()
    }
}

pub fn list() -> Result<Vec<Operation>, String> {
    list_in(&task_db::path().ok_or("Cannot locate database")?)
}

pub fn list_in(path: &std::path::Path) -> Result<Vec<Operation>, String> {
    let mut db = task_db::Database::open(path)?;
    let mut q=db.connection().prepare("SELECT id,kind,entity,created_at,attempts,error,payload,dead_at,lease_until FROM google_operations ORDER BY id").map_err(|e|e.to_string())?;
    q.query_map([], |r| {
        Ok(Operation {
            id: r.get(0)?,
            kind: r.get(1)?,
            entity: r.get(2)?,
            created_at: r.get(3)?,
            attempts: r.get(4)?,
            error: r.get(5)?,
            payload: r.get(6)?,
            failed_at: r.get(7)?,
            retry_at: r.get(8)?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<_, _>>()
    .map_err(|e| e.to_string())
}

/// Gives a failed write another go, straight away.
pub fn retry(id: i64) -> Result<(), String> {
    let path = task_db::path().ok_or("Cannot locate database")?;
    retry_in(&path, id)?;
    start();
    Ok(())
}

fn retry_in(path: &std::path::Path, id: i64) -> Result<(), String> {
    let mut db = task_db::Database::open(path)?;
    db.connection()
        .execute(
            "UPDATE google_operations SET dead_at=NULL,lease_until=0,attempts=0 WHERE id=?1",
            [id],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Drops a failed write. The local copy of what it changed goes too once no
/// other write for the same item is waiting, so the next sync shows Google's
/// version instead of an edit that will never arrive.
pub fn discard(id: i64) -> Result<(), String> {
    discard_in(&task_db::path().ok_or("Cannot locate database")?, id)
}

fn discard_in(path: &std::path::Path, id: i64) -> Result<(), String> {
    let mut db = task_db::Database::open(path)?;
    let tx = db
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let entity: Option<String> = tx
        .query_row(
            "SELECT entity FROM google_operations WHERE id=?1 AND dead_at IS NOT NULL",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(entity) = entity else {
        return Err("Only a write that failed can be discarded".into());
    };
    tx.execute("DELETE FROM google_operations WHERE id=?1", [id])
        .map_err(|e| e.to_string())?;
    release_entity(&tx, &entity).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

/// Forgets the local snapshot of `entity` unless a write for it is still
/// waiting, so the overlay stops showing a change Google never got.
fn release_entity(tx: &rusqlite::Connection, entity: &str) -> rusqlite::Result<()> {
    tx.execute(
        "DELETE FROM google_entities WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM google_operations WHERE entity=?1 AND dead_at IS NULL)",
        [entity],
    )?;
    Ok(())
}
pub fn new_id() -> Result<String, String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(b.iter().map(|v| format!("{v:02x}")).collect())
}
fn key(calendar: &str, id: &str) -> String {
    format!("{calendar}\n{id}")
}
pub fn enqueue(job: Job, event: Option<&Event>, calendar: &str, id: &str) -> Result<(), String> {
    let path = task_db::path().ok_or("Cannot locate database")?;
    enqueue_in(&path, job, event, calendar, id)?;
    start();
    Ok(())
}
fn enqueue_in(
    path: &std::path::Path,
    job: Job,
    event: Option<&Event>,
    calendar: &str,
    id: &str,
) -> Result<(), String> {
    let mut db = task_db::Database::open(path)?;
    let tx = db
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    enqueue_tx(&tx, &job, event, calendar, id)?;
    if let Job::Delete { calendar, id, .. } = &job {
        // Removing a container releases its children; deleting the container
        // and recording all child edits is one local transaction.
        let block = format!("{calendar}/{id}");
        let children = {
            let mut q = tx
                .prepare("SELECT event FROM google_task_records")
                .map_err(|e| e.to_string())?;
            q.query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .map(|row| {
                    serde_json::from_str::<Event>(&row.map_err(|e| e.to_string())?)
                        .map_err(|e| e.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        for original in children
            .into_iter()
            .filter(|child| crate::timeblocking::metadata(child).block.as_deref() == Some(&block))
        {
            let mut released = original.clone();
            let data = released
                .details
                .task_data
                .as_mut()
                .ok_or("Assigned task has no metadata")?;
            data.metadata.block = None;
            data.metadata.schedule = None;
            data.metadata.inherited = None;
            data.metadata.repeat_source = None;
            released.details.scheduled = None;
            released.all_day = true;
            released.start = at_clock(original.start.date_naive(), 0, 0);
            released.end = released.start + chrono::Duration::days(1);
            enqueue_tx(
                &tx,
                &Job::TaskEdit {
                    task: original.clone(),
                    change: task_edit::Change::Unassign,
                    order: 0,
                },
                Some(&released),
                &original.calendar_id,
                &original.id,
            )?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

fn enqueue_tx(
    tx: &rusqlite::Connection,
    job: &Job,
    event: Option<&Event>,
    calendar: &str,
    id: &str,
) -> Result<(), String> {
    let mut entity = key(calendar, id);
    let mut kind = match &job {
        Job::Edit { .. } => "Edit event",
        Job::Time { .. } => "Change time",
        Job::Location { .. } => "Change location",
        Job::Status { .. } => "Task status",
        Job::TaskEdit { .. } => "Edit task",
        Job::Delete { .. } | Job::DeleteTask { .. } => "Delete",
        Job::Create { .. } => "Create event",
        Job::CreateTask { .. } => "Create task",
        Job::Sleep { .. } => "Save sleep",
        Job::HabitCompleted { .. } => "Habit completion",
        Job::Reply { .. } => "Reply to invitation",
        Job::CreateHabit { .. } => "Create habit",
    };
    let lane = match &job {
        Job::TaskEdit { change, .. } => {
            let value = serde_json::to_value(change).map_err(|e| e.to_string())?;
            value
                .as_object()
                .and_then(|o| o.keys().next().cloned())
                .or_else(|| value.as_str().map(str::to_owned))
                .unwrap_or_default()
        }
        Job::Edit { scope, .. } => format!("{scope:?}"),
        _ => String::new(),
    };
    let kind_owned = format!("{kind} {lane}").trim().to_owned();
    kind = &kind_owned;
    let payload = serde_json::to_string(&job).map_err(|e| e.to_string())?;
    let saved = event
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| e.to_string())?;
    if let Some(existing) = tx
        .query_row(
            "SELECT id FROM google_entities WHERE json_extract(event,'$.id')=?1 AND (id=?2 OR json_extract(event,'$.calendar_id')=?3) LIMIT 1",
            params![id, entity, calendar],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
    {
        entity = existing;
    }
    (|| -> rusqlite::Result<()> {
        tx.execute("INSERT INTO google_entities(id,event,acknowledged_at) VALUES(?1,?2,NULL) ON CONFLICT(id) DO UPDATE SET event=excluded.event,acknowledged_at=NULL,observed_at=NULL",params![entity,saved])?;
        if event.is_none() {
            tx.execute("DELETE FROM google_task_records WHERE id=?1", [id])?;
        }
        if let Some(event) = event {
            if event.is_task() {
                tx.execute(
                    "INSERT INTO google_task_records(id,event) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET event=excluded.event",
                    params![event.id, serde_json::to_string(event).unwrap()],
                )?;
            }
            if let Some(data) = &event.details.task_data {
                tx.execute(
                    "INSERT INTO tasks(id,metadata) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET metadata=excluded.metadata",
                    params![data.id, serde_json::to_string(&data.metadata).unwrap()],
                )?;
                tx.execute(
                    "INSERT INTO task_aliases(source,task_id) VALUES(?1,?2) ON CONFLICT(source) DO UPDATE SET task_id=excluded.task_id",
                    params![crate::timeblocking::block_key(event), data.id],
                )?;
            }
        }
        tx.execute("DELETE FROM google_operations WHERE entity=?1 AND kind=?2 AND error IS NOT NULL",params![entity,kind])?;
        tx.execute("INSERT INTO google_operations(kind,payload,entity,created_at,version) VALUES(?1,?2,?3,?4,?5)",params![kind,payload,entity,crate::clock::now().to_rfc3339(),JOB_VERSION])?;
        Ok(())
    })().map_err(|e|e.to_string())?;
    Ok(())
}
/// What [`overlay`] applies, read from the database in one go so the reading
/// can happen on a worker thread and the applying on the UI thread.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    /// Remote ID to the local ID it was created under, so an item keeps the
    /// ID the interface already knows it by.
    aliases: HashMap<(String, String), String>,
    entities: Vec<OverlayEntity>,
}

#[derive(Debug, Clone, PartialEq)]
struct OverlayEntity {
    calendar: String,
    id: String,
    /// `None` for an item deleted locally.
    event: Option<Event>,
    acknowledged_at: Option<DateTime<chrono::FixedOffset>>,
}

/// Reads the local snapshots of edited items and the ID map.
pub fn load_overlay(database: &std::path::Path) -> Result<Overlay, String> {
    let mut db = task_db::Database::open(database)?;
    let tx = db.connection().transaction().map_err(|e| e.to_string())?;
    read_overlay(&tx)
}

fn read_overlay(tx: &rusqlite::Connection) -> Result<Overlay, String> {
    let mut aliases = HashMap::new();
    {
        let mut q = tx
            .prepare("SELECT local,remote,calendar FROM google_id_map")
            .map_err(|e| e.to_string())?;
        for row in q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
        {
            let (local, remote, calendar) = row.map_err(|e| e.to_string())?;
            if local != remote {
                aliases.entry((calendar, remote)).or_insert(local);
            }
        }
    }
    let mut entities = Vec::new();
    {
        let mut q = tx
            .prepare("SELECT id,event,acknowledged_at FROM google_entities")
            .map_err(|e| e.to_string())?;
        for row in q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
        {
            let (key, json, ack) = row.map_err(|e| e.to_string())?;
            let Some((calendar, id)) = key.split_once('\n') else {
                continue;
            };
            entities.push(OverlayEntity {
                calendar: calendar.to_owned(),
                id: id.to_owned(),
                event: json
                    .map(|json| serde_json::from_str(&json))
                    .transpose()
                    .map_err(|e| e.to_string())?,
                acknowledged_at: ack.and_then(|ack| DateTime::parse_from_rfc3339(&ack).ok()),
            });
        }
    }
    Ok(Overlay { aliases, entities })
}

impl Overlay {
    /// The ID the interface knows `id` by: the local ID it was created under,
    /// following moves that changed it more than once. Task IDs are mapped
    /// without their `task:` prefix.
    fn local_id(&self, calendar: &str, id: &str) -> Option<String> {
        if let Some(local) = self.follow(calendar, id) {
            return Some(local);
        }
        let bare = id.strip_prefix(ids::TASK)?;
        self.follow(calendar, bare)
            .map(|local| ids::event_id(&local))
    }

    fn follow(&self, calendar: &str, id: &str) -> Option<String> {
        let mut current = self.aliases.get(&(calendar.to_owned(), id.to_owned()))?;
        // A chain is at most as long as the map; a cycle stops there too.
        for _ in 0..self.aliases.len() {
            // Intermediate moves may have used a different calendar. Only
            // follow an unambiguous predecessor once the first hop is scoped.
            let mut predecessors = self
                .aliases
                .iter()
                .filter(|((_, remote), _)| remote == current)
                .map(|(_, local)| local);
            match (predecessors.next(), predecessors.next()) {
                (Some(next), None) if next != current => current = next,
                _ => break,
            }
        }
        Some(current.clone())
    }

    /// Puts the database's desired state over `events`.
    ///
    /// With `fetched_at`, the events came from a Google fetch started then,
    /// and an edit Google had confirmed before that is not applied over them:
    /// the fetch already shows it, or something newer.
    pub fn apply(&self, events: &mut Vec<Event>, fetched_at: Option<DateTime<Local>>) {
        let mut remapped = std::collections::HashSet::new();
        if !self.aliases.is_empty() {
            for event in events.iter_mut() {
                if let Some(local) = self.local_id(&event.calendar_id, &event.id) {
                    event.id = local;
                    let candidates: Vec<_> = self
                        .entities
                        .iter()
                        .filter(|entity| entity.id == event.id)
                        .collect();
                    let origin = candidates
                        .iter()
                        .copied()
                        .find(|entity| {
                            entity.calendar == event.calendar_id
                                || entity
                                    .event
                                    .as_ref()
                                    .is_some_and(|saved| saved.calendar_id == event.calendar_id)
                        })
                        .or_else(|| (candidates.len() == 1).then(|| candidates[0]));
                    if let Some(origin) = origin {
                        remapped.insert((
                            origin.calendar.clone(),
                            event.calendar_id.clone(),
                            event.id.clone(),
                        ));
                    }
                }
            }
        }
        for entity in &self.entities {
            if let (Some(fetched), Some(ack)) = (fetched_at, entity.acknowledged_at)
                && fetched > ack
            {
                continue;
            }
            events.retain(|e| {
                e.id != entity.id
                    || !(e.calendar_id == entity.calendar
                        || entity
                            .event
                            .as_ref()
                            .is_some_and(|saved| saved.calendar_id == e.calendar_id)
                        || remapped.contains(&(
                            entity.calendar.clone(),
                            e.calendar_id.clone(),
                            e.id.clone(),
                        )))
            });
            if let Some(event) = &entity.event {
                events.push(event.clone());
            }
        }
    }
}

/// Apply the database's desired state after every cache read and Google fetch.
/// Confirmed writes remain protected against fetches that began before the ack.
pub fn overlay(
    database: &std::path::Path,
    events: &mut Vec<Event>,
    fetched_at: Option<DateTime<Local>>,
) -> Result<(), String> {
    load_overlay(database)?.apply(events, fetched_at);
    Ok(())
}

pub fn cached_event(calendar: &str, id: &str) -> Result<Option<Event>, String> {
    let mut db = task_db::open()?;
    let json: Option<String> = db
        .connection()
        .query_row(
            "SELECT event FROM google_entities WHERE json_extract(event,'$.id')=?1 AND (id=?2 OR json_extract(event,'$.calendar_id')=?3) AND (acknowledged_at IS NULL OR acknowledged_at > COALESCE((SELECT json_extract(value,'$.fetched_at') FROM documents WHERE key='calendar_sync_state'),''))",
            params![id, key(calendar, id), calendar],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .flatten();
    if let Some(json) = json {
        return serde_json::from_str(&json)
            .map(Some)
            .map_err(|e| e.to_string());
    }
    let mut q = db
        .connection()
        .prepare("SELECT events FROM calendar_months")
        .map_err(|e| e.to_string())?;
    let rows = q
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    for row in rows {
        let json = row.map_err(|e| e.to_string())?;
        let month: serde_json::Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
        for value in month["events"].as_array().into_iter().flatten() {
            if value["id"] == id && value["calendar_id"] == calendar {
                let saved: super::day_cache::SavedEvent =
                    serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
                return Ok(Some(saved.into()));
            }
        }
    }
    Ok(None)
}
/// The payload format [`enqueue`] writes, kept in each row's `version`.
///
/// A change to [`Job`] that older rows cannot be read as bumps this, and
/// [`decode`] learns to read the previous version, so writes queued by an
/// older build still go out after an update. A row from a newer build is left
/// for that build rather than misread.
pub const JOB_VERSION: i64 = 1;

/// Why a queued payload could not be turned back into a [`Job`].
#[derive(Debug)]
enum Undecodable {
    /// Written by a newer build; that build will send it.
    Newer(i64),
    /// Cannot be read by any build: it will never be sent.
    Broken(String),
}

fn decode(version: i64, value: serde_json::Value) -> Result<Job, Undecodable> {
    match version {
        1 => serde_json::from_value(value).map_err(|e| Undecodable::Broken(e.to_string())),
        newer if newer > JOB_VERSION => Err(Undecodable::Newer(newer)),
        older => Err(Undecodable::Broken(format!(
            "unknown queue format version {older}"
        ))),
    }
}

/// What to do with a write Google did not accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    /// Try again after this many seconds.
    Retry(i64),
    /// Give up: trying again would get the same answer.
    Fail,
}

/// The first retry waits this long; each further one waits twice as long, up
/// to [`MAX_BACKOFF`].
const FIRST_BACKOFF: i64 = 15;
const MAX_BACKOFF: i64 = 30 * 60;
/// An error whose cause is unknown is retried at least this many times, and
/// for at least [`UNKNOWN_PATIENCE`], before it is given up on.
const UNKNOWN_ATTEMPTS: i64 = 12;
const UNKNOWN_PATIENCE: i64 = 24 * 60 * 60;

fn backoff(attempts: i64) -> i64 {
    let doublings = attempts.saturating_sub(1).clamp(0, 20) as u32;
    FIRST_BACKOFF
        .saturating_mul(1 << doublings)
        .min(MAX_BACKOFF)
}

/// Decides between retrying and giving up from what went wrong.
///
/// Anything the user or the network can fix (no connection, a rate limit, a
/// server error, an expired or missing sign-in, a scope or API to enable) is
/// retried for as long as it takes, so edits made offline or while signed out
/// are never lost. A refusal that will not change (the item is gone, the
/// request is invalid, permission is denied) is given up on at once, so it
/// stops holding up later writes for the same item. An error from a layer that
/// does not say which it was is retried patiently first.
fn disposition(error: &google::GoogleError, attempts: i64, age_seconds: i64) -> Disposition {
    use google::Kind;
    let retry = Disposition::Retry(backoff(attempts));
    match &error.kind {
        Kind::Transport | Kind::Auth | Kind::MissingScope | Kind::ApiDisabled => retry,
        Kind::RateLimited { retry_after } => {
            Disposition::Retry(backoff(attempts).max(retry_after.map_or(0, |d| d.as_secs() as i64)))
        }
        Kind::NotFound => Disposition::Fail,
        Kind::Http => match error.status {
            Some(408 | 425 | 429) | Some(500..=599) | None => retry,
            Some(_) => Disposition::Fail,
        },
        Kind::Other => {
            if attempts >= UNKNOWN_ATTEMPTS && age_seconds >= UNKNOWN_PATIENCE {
                Disposition::Fail
            } else {
                retry
            }
        }
    }
}

pub fn start() {
    // Test runs keep their queue local: it must never reach Google.
    if crate::config::isolated() {
        return;
    }
    static START: OnceLock<()> = OnceLock::new();
    START.get_or_init(|| {
        std::thread::spawn(|| {
            loop {
                if let Err(e) = drain_one() {
                    log::warn!("Google queue: {e}");
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    });
}
fn drain_one() -> Result<(), String> {
    let path = task_db::path().ok_or("Cannot locate database")?;
    drain_in(&path, |job| {
        let settings = crate::config::Config::load().calendar;
        execute(&GoogleClient::from_settings(&settings), &settings, job)
    })
}

/// A successful write that gave something a new ID: the local ID, Google's,
/// and the calendar or task list it is on.
type Mapping = Option<(String, String, String)>;

/// Sends the oldest due write with no earlier pending write for its item.
///
/// Preserve order for each item, so edits cannot overtake its creation or
/// move. A failed write waiting to retry does not hold up unrelated items.
fn drain_in(
    path: &std::path::Path,
    run: impl FnOnce(Job) -> Result<Mapping, google::GoogleError>,
) -> Result<(), String> {
    let mut db = task_db::Database::open(path)?;
    let tx = db
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let now = crate::clock::utc_now().timestamp();
    let item: Option<(i64, String, String, i64, i64, i64, String)> = tx
        .query_row(
            "SELECT op.id,op.payload,op.entity,op.lease_until,op.version,op.attempts,op.created_at FROM google_operations op WHERE op.dead_at IS NULL AND op.lease_until<=?1 AND NOT EXISTS(SELECT 1 FROM google_operations earlier WHERE earlier.entity=op.entity AND earlier.id<op.id AND earlier.dead_at IS NULL) ORDER BY op.id LIMIT 1",
            [now],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((id, payload, entity, lease, version, attempts, created_at)) = item else {
        return Ok(());
    };
    if lease > now {
        return Ok(());
    }
    let age =
        DateTime::parse_from_rfc3339(&created_at).map_or(0, |created| now - created.timestamp());
    let mut value: serde_json::Value = match serde_json::from_str(&payload) {
        Ok(value) => value,
        Err(error) => {
            fail(
                &tx,
                id,
                &entity,
                &format!("This change could not be read: {error}"),
            )?;
            return tx.commit().map_err(|e| e.to_string());
        }
    };
    let aliases = {
        let mut q = tx
            .prepare("SELECT local,remote,calendar FROM google_id_map")
            .map_err(|e| e.to_string())?;
        q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
    };
    remap(&mut value, &aliases);
    let job = match decode(version, value) {
        Ok(job) => job,
        Err(Undecodable::Newer(newer)) => {
            // Leave it for the newer build, without counting an attempt. It
            // stays first, so later writes keep their order behind it.
            tx.execute(
                "UPDATE google_operations SET error=?2 WHERE id=?1",
                params![
                    id,
                    format!("Waiting for a newer version of TimeSlay (queue format {newer})")
                ],
            )
            .map_err(|e| e.to_string())?;
            return tx.commit().map_err(|e| e.to_string());
        }
        Err(Undecodable::Broken(error)) => {
            fail(
                &tx,
                id,
                &entity,
                &format!("This change was saved in a form TimeSlay can no longer send: {error}"),
            )?;
            return tx.commit().map_err(|e| e.to_string());
        }
    };
    tx.execute(
        "UPDATE google_operations SET lease_until=?2,attempts=attempts+1 WHERE id=?1",
        params![id, now + 600],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;

    REPLAY.with(|r| r.set(true));
    OPERATION_ID.with(|op| op.set(Some(id)));
    let result = run(job);
    OPERATION_ID.with(|op| op.set(None));
    REPLAY.with(|r| r.set(false));

    // All edits to an entity commit independently. Only delete the exact job
    // that succeeded, then acknowledge if no newer job exists.
    let tx = db
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let progress: Option<String> = tx
        .query_row(
            "SELECT data FROM google_operation_progress WHERE operation_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(json) = progress {
        let data: crate::task_db::TaskData =
            serde_json::from_str(&json).map_err(|e| e.to_string())?;
        tx.execute("INSERT INTO google_task_backend_state(task_id,data) VALUES(?1,?2) ON CONFLICT(task_id) DO UPDATE SET data=excluded.data",params![data.id,json]).map_err(|e|e.to_string())?;
    }
    match result {
        Ok(mapping) => {
            if let Some((local, remote, calendar)) = mapping {
                tx.execute("INSERT INTO google_id_map(local,remote,calendar) VALUES(?1,?2,?3) ON CONFLICT(local) DO UPDATE SET remote=excluded.remote,calendar=excluded.calendar",params![local,remote,calendar]).map_err(|e|e.to_string())?;
            }
            tx.execute("DELETE FROM google_operations WHERE id=?1", [id])
                .map_err(|e| e.to_string())?;
            tx.execute("UPDATE google_entities SET acknowledged_at=?2 WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM google_operations WHERE entity=?1 AND dead_at IS NULL)",params![entity,crate::clock::now().to_rfc3339()]).map_err(|e|e.to_string())?;
        }
        Err(error) => {
            let message = error.to_string();
            // A newer write of the same kind for the same item supersedes this
            // one, so it does not need to succeed at all.
            let superseded = tx.execute("DELETE FROM google_operations WHERE id=?1 AND EXISTS(SELECT 1 FROM google_operations newer WHERE newer.entity=?2 AND newer.kind=(SELECT kind FROM google_operations WHERE id=?1) AND newer.id>?1 AND newer.dead_at IS NULL)",params![id,entity]).map_err(|e|e.to_string())?;
            if superseded == 0 {
                match disposition(&error, attempts + 1, age) {
                    Disposition::Retry(delay) => {
                        tx.execute(
                            "UPDATE google_operations SET error=?2,lease_until=?3 WHERE id=?1",
                            params![id, message, now + delay],
                        )
                        .map_err(|e| e.to_string())?;
                    }
                    Disposition::Fail => {
                        log::warn!("Google queue: giving up on write {id}: {message}");
                        fail(&tx, id, &entity, &message)?;
                    }
                }
            }
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

/// Sets a write aside as failed, with the reason, and stops showing the local
/// change it carried unless a later write for the same item is still waiting.
fn fail(tx: &rusqlite::Connection, id: i64, entity: &str, error: &str) -> Result<(), String> {
    tx.execute(
        "UPDATE google_operations SET error=?2,dead_at=?3,lease_until=0 WHERE id=?1",
        params![id, error, crate::clock::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    release_entity(tx, entity).map_err(|e| e.to_string())
}

/// Rewrites local IDs in a queued payload to the IDs Google gave the items
/// since it was queued.
fn remap(v: &mut serde_json::Value, a: &[(String, String, String)]) {
    if let Some(obj) = v.as_object_mut() {
        for _ in 0..a.len() {
            for (id_key, calendar_key, prefix) in
                [("id", "calendar_id", "task:"), ("task_id", "list_id", "")]
            {
                if let Some(id) = obj.get(id_key).and_then(|v| v.as_str()).map(str::to_owned)
                    && let Some((_, remote, calendar)) = a.iter().find(|(local, remote, _)| {
                        local != remote && (id == *local || id == ids::event_id(local))
                    })
                {
                    let full = id.starts_with(ids::TASK);
                    obj.insert(
                        id_key.into(),
                        if full {
                            ids::event_id(remote)
                        } else {
                            remote.clone()
                        }
                        .into(),
                    );
                    let calendar = if prefix.is_empty() {
                        ids::bare_list(calendar).to_owned()
                    } else {
                        calendar.clone()
                    };
                    if obj.contains_key(calendar_key) {
                        obj.insert(calendar_key.into(), calendar.clone().into());
                    }
                    if obj.contains_key("calendar") {
                        obj.insert("calendar".into(), calendar.into());
                    }
                }
            }
        }
        for v in obj.values_mut() {
            remap(v, a);
        }
    } else {
        match v {
            serde_json::Value::String(s) => {
                for (l, r, _) in a {
                    if s == l {
                        *s = r.clone();
                    }
                }
            }
            serde_json::Value::Array(v) => {
                for v in v {
                    remap(v, a)
                }
            }
            _ => {}
        }
    }
}

fn execute(
    client: &GoogleClient,
    s: &CalendarSettings,
    job: Job,
) -> Result<Mapping, google::GoogleError> {
    match job {
        Job::Edit {
            original,
            draft,
            scope,
            description,
            color,
        } => {
            send::event_scope(
                client,
                &original,
                &draft,
                scope,
                &description,
                color.as_deref(),
            )?;
        }
        Job::Time {
            calendar,
            id,
            start,
            end,
            notify,
        } => send::event_time(client, &calendar, &id, start, end, notify)?,
        Job::Location {
            calendar,
            id,
            location,
        } => send::event_location(client, &calendar, &id, &location)?,
        Job::Status { task, status } => {
            send::task_status(client, &task, status)?;
        }
        Job::TaskEdit {
            task,
            change,
            order,
        } => {
            let old = ids::bare_task(&task.id).to_owned();
            let saved = task_edit::save(task, s, change, order, task_edit::google_access)?;
            if ids::bare_task(&saved.id) != old {
                return Ok(Some((
                    old,
                    ids::bare_task(&saved.id).into(),
                    saved.calendar_id,
                )));
            }
        }
        Job::Delete {
            calendar,
            id,
            scope,
            notify,
        } => send::delete_event_scope(client, &calendar, &id, scope, notify)?,
        Job::DeleteTask { mut task, calendar } => {
            let mut db = task_db::open()?;
            let data = db.import(&task)?;
            task.details.scheduled = data.metadata.schedule;
            send::delete_task(client, &task, calendar.as_deref())?
        }
        Job::Create {
            id,
            calendar,
            title,
            description,
            color,
            start,
            end,
            recurrence,
            projects,
            all_day,
            options,
            meeting,
        } => {
            INSERT_ID.with(|v| *v.borrow_mut() = Some(id.clone()));
            let result = send::create_event(
                client,
                &calendar,
                &title,
                &description,
                color.as_deref(),
                start,
                end,
                &recurrence,
                &projects,
                all_day,
                options.as_ref(),
                meeting.as_ref(),
            );
            INSERT_ID.with(|v| *v.borrow_mut() = None);
            let saved = result?;
            return Ok(Some((id, saved.id, calendar)));
        }
        Job::CreateHabit {
            id,
            draft,
            color,
            habit,
        } => {
            INSERT_ID.with(|v| *v.borrow_mut() = Some(id.clone()));
            let result = send::create_habit(client, &draft, color.as_deref(), &habit);
            INSERT_ID.with(|v| *v.borrow_mut() = None);
            let saved = result?;
            return Ok(Some((id, saved.id, saved.calendar_id)));
        }
        Job::Sleep { event, new } => {
            super::sleep::send(client, event, new)?;
        }
        Job::HabitCompleted { event } => send::habit_completed(client, &event)?,
        Job::Reply {
            event,
            email,
            response,
        } => send::reply(client, &event.calendar_id, &event.id, &email, response)?,
        Job::CreateTask {
            task,
            notes,
            due,
            metadata,
        } => {
            let token = client.access_token()?;
            let list = scheduled::inbox_destination(&token, s.inbox_list.as_deref())?;
            let data = task
                .details
                .task_data
                .as_ref()
                .ok_or("Missing task metadata")?;
            let existing = api::fetch_task_list(&token, &list)?
                .into_iter()
                .find(|entry| {
                    task_metadata::read(&entry.notes)
                        .ok()
                        .flatten()
                        .is_some_and(|d| d.id == data.id)
                });
            if let Some(existing) = existing {
                return Ok(Some((
                    ids::bare_task(&task.id).into(),
                    existing.id,
                    ids::calendar_id(&list),
                )));
            }
            let notes = task_metadata::write(&notes, data)?;
            let saved =
                task_edit::create_inbox_task(s, &token, &task.title, &notes, due, metadata)?;
            return Ok(Some((
                ids::bare_task(&task.id).into(),
                ids::bare_task(&saved.id).into(),
                saved.calendar_id,
            )));
        }
    }
    Ok(None)
}

pub fn edit_task(
    mut task: Event,
    settings: &CalendarSettings,
    change: task_edit::Change,
    order: i64,
) -> Result<Event, String> {
    task_edit::validate_child_edit(&task, &change)?;
    let original = task.clone();
    let mut db = task_db::open()?;
    let mut data = task
        .details
        .task_data
        .clone()
        .map(Ok)
        .unwrap_or_else(|| db.import(&task))?;
    use task_edit::Change;
    task_edit::reset_repeat_source(&mut data.metadata, &change);
    match &change {
        Change::ReleaseInheritance => data.metadata.inherited = None,
        Change::Inherit(inherited) => {
            crate::timeblocking::apply_inherited(&mut task, &mut data, inherited.clone())
        }
        Change::Title(v) => task.title = v.clone(),
        Change::Description(v) => {
            let deadline = deadline::read_deadline(&task.details.description)?;
            task.details.description = deadline::write(v, deadline);
        }
        Change::Estimate(v) => {
            data.metadata.minutes = *v;
            if !task.all_day && *v > 0 {
                task.end = task.start + chrono::Duration::minutes(i64::from(*v));
            }
        }
        Change::Projects(v) => data.metadata.projects = v.clone(),
        Change::Labels(v) => data.metadata.labels = v.clone(),
        Change::Priority(v) => data.metadata.priority = *v,
        Change::Location(v) => data.metadata.location = *v,
        Change::BacklogSection(v) => data.metadata.backlog_section = *v,
        Change::Deadline(v) => {
            task.details.description = deadline::write(&task.details.description, *v)
        }
        Change::Time(start, end) | Change::Block(_, start, end) | Change::Reflow(start, end) => {
            task.start = *start;
            task.end = *end;
            task.all_day = false;
            if let Some(list) = settings.time_block_list.as_deref() {
                task.calendar_id = ids::calendar_id(list);
            }
            if let Change::Block(key, _, _) = &change {
                data.metadata.block = Some(key.clone());
                data.metadata.order = order;
                task.end = task.start
                    + chrono::Duration::minutes(i64::from(if data.metadata.minutes == 0 {
                        30
                    } else {
                        data.metadata.minutes
                    }));
            } else if matches!(change, Change::Time(_, _)) {
                data.metadata.block = None;
                let moved = !original.all_day && *end - *start == original.end - original.start;
                if moved && data.metadata.minutes > 0 {
                    task.end = *start + chrono::Duration::minutes(i64::from(data.metadata.minutes));
                } else {
                    data.metadata.minutes = (*end - *start).num_minutes().clamp(0, 10080) as u32;
                }
            }
        }
        Change::Reorder(block, order, _) => {
            data.metadata.block = block.clone();
            data.metadata.order = *order;
        }
        Change::Day(_)
        | Change::ClearSchedule
        | Change::Unassign
        | Change::Inbox
        | Change::Triaged
        | Change::Backlog(_) => {
            let day = match &change {
                Change::Day(day) => *day,
                Change::Unassign => task.start.date_naive(),
                _ => crate::timeblocking::UNDATED,
            };
            task.start = at_clock(day, 0, 0);
            task.end = task.start + chrono::Duration::days(1);
            task.all_day = true;
            data.metadata.schedule = None;
            data.metadata.block = None;
            data.metadata.inherited = None;
            task.details.scheduled = None;
            let list = match &change {
                Change::Inbox => settings.inbox_list.as_deref(),
                Change::Triaged => settings.triaged_list.as_deref(),
                Change::Backlog(_) => settings.backlog_list.as_deref(),
                _ => None,
            };
            if let Some(list) = list {
                task.calendar_id = ids::calendar_id(list);
            }
            if let Change::Backlog(Some(section)) = change {
                data.metadata.backlog_section = section;
            }
        }
    }
    task.details.task_data = Some(data);
    enqueue(
        Job::TaskEdit {
            task: original.clone(),
            change,
            order,
        },
        Some(&task),
        &original.calendar_id,
        &original.id,
    )?;
    Ok(task)
}
pub fn create_task(
    settings: &CalendarSettings,
    title: &str,
    notes: &str,
    due: Option<NaiveDate>,
    metadata: Option<crate::timeblocking::Metadata>,
) -> Result<Event, String> {
    let list = settings
        .inbox_list
        .as_deref()
        .unwrap_or("local-inbox")
        .trim_start_matches("tasks:");
    let start = at_clock(due.unwrap_or(crate::timeblocking::UNDATED), 0, 0);
    let mut task = Event {
        id: format!("task:{}", new_id()?),
        calendar_id: ids::calendar_id(list),
        title: title.into(),
        start,
        end: start + chrono::Duration::days(1),
        all_day: true,
        color: colors::TASK,
        completed: false,
        details: Details {
            description: notes.into(),
            ..Default::default()
        },
    };
    let mut db = task_db::open()?;
    let mut data = db.import(&task)?;
    if let Some(metadata) = &metadata {
        data.metadata = metadata.clone();
    }
    data.metadata.created_at = Some(crate::clock::utc_now());
    task.details.task_data = Some(data);
    enqueue(
        Job::CreateTask {
            task: Box::new(task.clone()),
            notes: notes.into(),
            due,
            metadata,
        },
        Some(&task),
        &task.calendar_id,
        &task.id,
    )?;
    Ok(task)
}

pub(super) fn cache_tasks(events: &[Event], replace: bool) -> Result<(), String> {
    cache_tasks_in(
        &task_db::path().ok_or("Cannot locate database")?,
        events,
        replace,
    )
}

fn cache_tasks_in(path: &std::path::Path, events: &[Event], replace: bool) -> Result<(), String> {
    let mut db = task_db::Database::open(path)?;
    let tx = db
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    // Re-read the durable overlay under the write transaction. A UI edit or
    // deletion may have committed since this fetch applied its first overlay.
    let mut events = events.to_vec();
    read_overlay(&tx)?.apply(&mut events, None);
    if replace {
        tx.execute("INSERT INTO documents(key,value) VALUES('calendar_task_snapshot','true') ON CONFLICT(key) DO UPDATE SET value=excluded.value", [])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM google_task_records", [])
            .map_err(|e| e.to_string())?;
    }
    for event in events.iter().filter(|e| e.is_task()) {
        tx.execute("INSERT INTO google_task_records(id,event) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET event=excluded.event",params![event.id,serde_json::to_string(event).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

/// Advance acknowledged projections only from a fetch that began after the
/// acknowledgement. Older concurrent fetches continue to see the newer row.
pub(super) fn observe_range(
    path: &std::path::Path,
    events: &[Event],
    first: NaiveDate,
    last: NaiveDate,
    fetched: DateTime<Local>,
) -> Result<(), String> {
    let mut db = task_db::Database::open(path)?;
    let tx = db
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let records = {
        let mut q=tx.prepare("SELECT id,event,acknowledged_at FROM google_entities WHERE acknowledged_at IS NOT NULL").map_err(|e|e.to_string())?;
        q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?
    };
    for (key, json, ack) in records {
        if !DateTime::parse_from_rfc3339(&ack).is_ok_and(|a| fetched > a) {
            continue;
        }
        let Some((calendar, id)) = key.split_once('\n') else {
            continue;
        };
        let old = json
            .as_ref()
            .map(|j| serde_json::from_str::<Event>(j))
            .transpose()
            .map_err(|e| e.to_string())?;
        let current = events.iter().find(|e| {
            e.id == id && e.calendar_id == old.as_ref().map_or(calendar, |e| &e.calendar_id)
        });
        let covered = current.is_some()
            || old.as_ref().is_some_and(|e| {
                e.is_task() || (e.start.date_naive() <= last && e.end.date_naive() >= first)
            });
        if covered {
            if let Some(data) = current.and_then(|e| e.details.task_data.as_ref()) {
                tx.execute("INSERT INTO google_task_backend_state(task_id,data) VALUES(?1,?2) ON CONFLICT(task_id) DO UPDATE SET data=excluded.data",params![data.id,serde_json::to_string(data).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
            }
            let json = current
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| e.to_string())?;
            tx.execute("UPDATE google_entities SET event=?2,acknowledged_at=?3,observed_at=COALESCE(observed_at,?5) WHERE id=?1 AND acknowledged_at=?4",params![key,json,fetched.to_rfc3339(),ack,fetched.with_timezone(&Utc).to_rfc3339()]).map_err(|e|e.to_string())?;
        }
    }
    prune_observed(&tx, fetched).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

/// How long a local snapshot is kept after a Google fetch showed the item as
/// Google has it. It only guards against a fetch that started before the edit
/// was confirmed finishing after it, and no request takes anywhere near this.
const OBSERVED_RETENTION: chrono::Duration = chrono::Duration::hours(1);

/// Forgets local snapshots that a fetch has since shown Google agrees with.
/// Without this every item ever edited kept a row that every poll re-read.
fn prune_observed(tx: &rusqlite::Connection, now: DateTime<Local>) -> rusqlite::Result<usize> {
    let cutoff = (now - OBSERVED_RETENTION).with_timezone(&Utc).to_rfc3339();
    tx.execute(
        "DELETE FROM google_entities WHERE observed_at IS NOT NULL AND observed_at < ?1 AND NOT EXISTS(SELECT 1 FROM google_operations o WHERE o.entity=google_entities.id AND o.dead_at IS NULL)",
        [cutoff],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("timeslay.sqlite3");
        task_db::Database::open(&path).unwrap();
        (dir, path)
    }
    fn edit(path: &std::path::Path, title: &str) -> Event {
        let mut event = super::super::sample_snapshot().events.remove(0);
        event.title = title.into();
        let draft = EditDraft::from_event(&event);
        enqueue_in(
            path,
            Job::Edit {
                original: Box::new(event.clone()),
                draft,
                scope: SaveScope::Current,
                description: String::new(),
                color: None,
            },
            Some(&event),
            &event.calendar_id,
            &event.id,
        )
        .unwrap();
        event
    }
    fn count(path: &std::path::Path) -> i64 {
        let mut db = task_db::Database::open(path).unwrap();
        db.connection()
            .query_row("SELECT count(*) FROM google_operations", [], |r| r.get(0))
            .unwrap()
    }
    fn edit_other(path: &std::path::Path, id: &str, title: &str) -> Event {
        let mut event = super::super::sample_snapshot().events.remove(0);
        event.id = id.into();
        event.title = title.into();
        let draft = EditDraft::from_event(&event);
        enqueue_in(
            path,
            Job::Edit {
                original: Box::new(event.clone()),
                draft,
                scope: SaveScope::Current,
                description: String::new(),
                color: None,
            },
            Some(&event),
            &event.calendar_id,
            &event.id,
        )
        .unwrap();
        event
    }
    fn due_now(path: &std::path::Path) {
        task_db::Database::open(path)
            .unwrap()
            .connection()
            .execute("UPDATE google_operations SET lease_until=0", [])
            .unwrap();
    }
    fn titles_sent(path: &std::path::Path) -> Vec<String> {
        let mut sent = Vec::new();
        for _ in 0..4 {
            due_now(path);
            drain_in(path, |job| {
                if let Job::Edit { draft, .. } = job {
                    sent.push(draft.title);
                }
                Ok(None)
            })
            .unwrap();
        }
        sent
    }
    #[test]
    fn retrying_an_item_does_not_block_other_items_or_reorder_its_edits() {
        let (_dir, path) = db();
        let event = edit(&path, "first edit");
        drain_in(&path, |_| Err("temporary failure".into())).unwrap();
        let mut draft = EditDraft::from_event(&event);
        draft.title = "later edit of same item".into();
        // A different scope is not coalesced with the failed edit.
        enqueue_in(
            &path,
            Job::Edit {
                original: Box::new(event.clone()),
                draft,
                scope: SaveScope::All,
                description: String::new(),
                color: None,
            },
            Some(&event),
            &event.calendar_id,
            &event.id,
        )
        .unwrap();
        edit_other(&path, "unrelated", "unrelated edit");

        let mut sent = Vec::new();
        drain_in(&path, |job| {
            if let Job::Edit { draft, .. } = job {
                sent.push(draft.title);
            }
            Ok(None)
        })
        .unwrap();
        assert_eq!(sent, ["unrelated edit"]);
        assert_eq!(list_in(&path).unwrap().len(), 2);
        drain_in(&path, |_| panic!("same item must wait for its first write")).unwrap();
        assert_eq!(
            titles_sent(&path),
            ["first edit", "later edit of same item"]
        );
    }

    #[test]
    fn invalid_google_task_list_is_skipped_as_a_permanent_failure() {
        let (_dir, path) = db();
        edit(&path, "invalid list");
        edit_other(&path, "valid", "valid edit");
        drain_in(&path, |_| {
            Err(google::GoogleError::from_status(
                "Google Tasks",
                400,
                "Invalid task list ID",
            ))
        })
        .unwrap();
        assert!(!list_in(&path).unwrap()[0].pending());
        assert_eq!(titles_sent(&path), ["valid edit"]);
    }

    #[test]
    fn a_refusal_that_will_not_change_is_set_aside_and_unblocks_the_queue() {
        let (_dir, path) = db();
        edit_other(&path, "gone", "edited after deletion elsewhere");
        edit_other(&path, "fine", "still wanted");
        drain_in(&path, |_| {
            Err(google::GoogleError::from_status(
                "Google Calendar",
                404,
                "Not Found",
            ))
        })
        .unwrap();
        let rows = list_in(&path).unwrap();
        assert!(!rows[0].pending());
        assert_eq!(rows[0].error.as_deref(), Some("Google Calendar: Not Found"));
        assert!(rows[1].pending());
        // The failed edit's local copy is no longer shown over Google's data.
        let mut fetched = Vec::new();
        overlay(&path, &mut fetched, None).unwrap();
        assert_eq!(
            fetched.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
            ["fine"]
        );
        // The write behind it goes out; the failed one stays listed.
        assert_eq!(titles_sent(&path), ["still wanted"]);
        assert_eq!(list_in(&path).unwrap().len(), 1);
    }
    #[test]
    fn offline_and_signed_out_writes_are_retried_for_as_long_as_it_takes() {
        let (_dir, path) = db();
        edit(&path, "made offline");
        for error in [
            google::GoogleError::transport("Google Calendar", "Connection Failed"),
            google::GoogleError::auth("Not connected to Google Calendar"),
            google::GoogleError::from_status("Google Calendar", 503, "Backend Error"),
        ] {
            for _ in 0..30 {
                due_now(&path);
                drain_in(&path, |_| Err(error.clone())).unwrap();
            }
            assert!(list_in(&path).unwrap()[0].pending(), "{error}");
        }
        // An error with no known cause is given up on, but only eventually.
        task_db::Database::open(&path)
            .unwrap()
            .connection()
            .execute(
                "UPDATE google_operations SET created_at='2000-01-01T00:00:00+00:00'",
                [],
            )
            .unwrap();
        due_now(&path);
        drain_in(&path, |_| Err("something odd".into())).unwrap();
        assert!(!list_in(&path).unwrap()[0].pending());
    }
    #[test]
    fn retries_back_off_up_to_a_limit() {
        assert_eq!(backoff(1), FIRST_BACKOFF);
        assert_eq!(backoff(2), FIRST_BACKOFF * 2);
        assert_eq!(backoff(100), MAX_BACKOFF);
        let limited = google::GoogleError {
            kind: google::Kind::RateLimited {
                retry_after: Some(Duration::from_secs(5000)),
            },
            status: Some(429),
            context: String::new(),
            detail: String::new(),
        };
        assert_eq!(disposition(&limited, 1, 0), Disposition::Retry(5000));
        let fresh_unknown: google::GoogleError = "odd".into();
        assert!(matches!(
            disposition(&fresh_unknown, UNKNOWN_ATTEMPTS, 60),
            Disposition::Retry(_)
        ));
        assert_eq!(
            disposition(&fresh_unknown, UNKNOWN_ATTEMPTS, UNKNOWN_PATIENCE),
            Disposition::Fail
        );
        let invalid = google::GoogleError::from_status("Google Calendar", 400, "Invalid");
        assert_eq!(disposition(&invalid, 1, 0), Disposition::Fail);
    }
    #[test]
    fn a_payload_from_a_newer_build_waits_and_a_broken_one_is_set_aside() {
        let (_dir, path) = db();
        edit(&path, "from the future");
        edit_other(&path, "later", "behind it");
        task_db::Database::open(&path)
            .unwrap()
            .connection()
            .execute(
                "UPDATE google_operations SET version=?1 WHERE id=(SELECT min(id) FROM google_operations)",
                [JOB_VERSION + 1],
            )
            .unwrap();
        drain_in(&path, |_| panic!("a newer payload must not be read")).unwrap();
        let rows = list_in(&path).unwrap();
        assert!(rows[0].pending());
        assert_eq!(rows[0].attempts, 0);
        assert!(rows[0].error.as_deref().unwrap().contains("newer version"));

        // A payload no build can read is set aside instead of blocking forever.
        task_db::Database::open(&path)
            .unwrap()
            .connection()
            .execute(
                "UPDATE google_operations SET version=?1, payload='{\"Unknown\":{}}' WHERE id=(SELECT min(id) FROM google_operations)",
                [JOB_VERSION],
            )
            .unwrap();
        drain_in(&path, |_| panic!("an unreadable payload must not run")).unwrap();
        assert!(!list_in(&path).unwrap()[0].pending());
        assert_eq!(titles_sent(&path), ["behind it"]);
    }
    #[test]
    fn a_failed_write_can_be_retried_or_discarded() {
        let (_dir, path) = db();
        edit(&path, "refused");
        drain_in(&path, |_| {
            Err(google::GoogleError::from_status(
                "Google Calendar",
                403,
                "Forbidden",
            ))
        })
        .unwrap();
        let id = list_in(&path).unwrap()[0].id;
        retry_in(&path, id).unwrap();
        let row = &list_in(&path).unwrap()[0];
        assert!(row.pending());
        assert_eq!(row.attempts, 0);
        drain_in(&path, |_| {
            Err(google::GoogleError::from_status(
                "Google Calendar",
                403,
                "Forbidden",
            ))
        })
        .unwrap();
        discard_in(&path, id).unwrap();
        assert!(list_in(&path).unwrap().is_empty());
        let mut fetched = Vec::new();
        overlay(&path, &mut fetched, None).unwrap();
        assert!(fetched.is_empty());
        // Only failed writes can be discarded; a waiting one is never dropped.
        edit(&path, "waiting");
        let waiting = list_in(&path).unwrap()[0].id;
        assert!(discard_in(&path, waiting).is_err());
    }
    #[test]
    fn ids_follow_chains_of_moves_only_from_the_matching_calendar() {
        let overlay = Overlay {
            aliases: [("b", "a"), ("c", "b"), ("x", "x")]
                .into_iter()
                .map(|(remote, local)| {
                    (("calendar".to_owned(), remote.to_owned()), local.to_owned())
                })
                .collect(),
            entities: Vec::new(),
        };
        assert_eq!(overlay.local_id("calendar", "c").as_deref(), Some("a"));
        assert_eq!(
            overlay.local_id("calendar", "task:c").as_deref(),
            Some("task:a")
        );
        assert_eq!(overlay.local_id("calendar", "b").as_deref(), Some("a"));
        assert_eq!(overlay.local_id("calendar", "unknown"), None);
        assert_eq!(overlay.local_id("other", "c"), None);
        // A self-mapping cannot loop.
        assert_eq!(overlay.local_id("calendar", "x").as_deref(), Some("x"));
    }
    #[test]
    fn edits_and_deletions_preserve_matching_ids_on_other_calendars() {
        let (_dir, path) = db();
        let a = edit(&path, "calendar A edit");
        let mut b = a.clone();
        b.calendar_id = "other-calendar".into();
        b.title = "calendar B edit".into();
        enqueue_in(
            &path,
            Job::Edit {
                original: Box::new(b.clone()),
                draft: EditDraft::from_event(&b),
                scope: SaveScope::Current,
                description: String::new(),
                color: None,
            },
            Some(&b),
            &b.calendar_id,
            &b.id,
        )
        .unwrap();
        let mut events = vec![a.clone(), b.clone()];
        overlay(&path, &mut events, None).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events.contains(&a));
        assert!(events.contains(&b));
        enqueue_in(
            &path,
            Job::Delete {
                calendar: a.calendar_id.clone(),
                id: a.id.clone(),
                scope: SaveScope::Current,
                notify: false,
            },
            None,
            &a.calendar_id,
            &a.id,
        )
        .unwrap();
        overlay(&path, &mut events, None).unwrap();
        assert_eq!(events, [b]);
    }

    #[test]
    fn moved_ids_on_other_calendars_are_not_remapped_or_removed() {
        let (_dir, path) = db();
        let local = edit(&path, "local");
        drain_in(&path, |_| {
            Ok(Some((
                local.id.clone(),
                "remote".into(),
                "destination".into(),
            )))
        })
        .unwrap();
        let mut moved = local.clone();
        moved.id = "remote".into();
        moved.calendar_id = "destination".into();
        let mut other = moved.clone();
        other.calendar_id = "unrelated".into();
        let mut events = vec![moved, other.clone()];
        overlay(&path, &mut events, None).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events.contains(&local));
        assert!(events.contains(&other));
    }
    #[test]
    fn a_moved_overlay_does_not_consume_another_calendars_local_id() {
        let (_dir, path) = db();
        let original = edit(&path, "original");
        let mut moved = original.clone();
        moved.calendar_id = "destination".into();
        enqueue_in(
            &path,
            Job::Edit {
                original: Box::new(original.clone()),
                draft: EditDraft::from_event(&moved),
                scope: SaveScope::Current,
                description: String::new(),
                color: None,
            },
            Some(&moved),
            &original.calendar_id,
            &original.id,
        )
        .unwrap();
        let mut other = original.clone();
        other.calendar_id = "unrelated".into();
        enqueue_in(
            &path,
            Job::Edit {
                original: Box::new(other.clone()),
                draft: EditDraft::from_event(&other),
                scope: SaveScope::Current,
                description: String::new(),
                color: None,
            },
            Some(&other),
            &other.calendar_id,
            &other.id,
        )
        .unwrap();
        task_db::Database::open(&path).unwrap().connection().execute(
            "INSERT INTO google_id_map(local,remote,calendar) VALUES(?1,'remote','destination')",
            [&original.id],
        ).unwrap();
        let mut remote = moved.clone();
        remote.id = "remote".into();
        let mut events = vec![remote, other.clone()];
        overlay(&path, &mut events, None).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events.contains(&moved));
        assert!(events.contains(&other));
    }

    #[test]
    fn snapshots_google_has_since_confirmed_are_pruned_after_an_hour() {
        let (_dir, path) = db();
        let event = edit(&path, "confirmed");
        drain_in(&path, |_| Ok(None)).unwrap();
        let entities = |path: &std::path::Path| -> i64 {
            task_db::Database::open(path)
                .unwrap()
                .connection()
                .query_row("SELECT count(*) FROM google_entities", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(entities(&path), 1);
        let day = event.start.date_naive();
        let fetched = crate::clock::now() + chrono::Duration::seconds(1);
        observe_range(&path, std::slice::from_ref(&event), day, day, fetched).unwrap();
        assert_eq!(
            entities(&path),
            1,
            "kept while an older fetch may still land"
        );
        // Fetches keep confirming it; the first confirmation starts the clock.
        observe_range(
            &path,
            std::slice::from_ref(&event),
            day,
            day,
            fetched + chrono::Duration::minutes(30),
        )
        .unwrap();
        assert_eq!(entities(&path), 1);
        observe_range(
            &path,
            std::slice::from_ref(&event),
            day,
            day,
            fetched + chrono::Duration::hours(2),
        )
        .unwrap();
        assert_eq!(entities(&path), 0);
    }
    #[test]
    fn queued_creation_requires_an_explicit_all_day_flag() {
        let event = super::super::sample_snapshot().events.remove(0);
        let job = Job::Create {
            id: event.id,
            calendar: event.calendar_id,
            title: event.title,
            description: String::new(),
            color: None,
            start: event.start,
            end: event.end,
            recurrence: vec![],
            projects: vec![],
            all_day: true,
            options: None,
            meeting: None,
        };
        let mut value = serde_json::to_value(job).unwrap();
        let restored: Job = serde_json::from_value(value.clone()).unwrap();
        assert!(matches!(restored, Job::Create { all_day: true, .. }));

        value["Create"].as_object_mut().unwrap().remove("all_day");
        assert!(serde_json::from_value::<Job>(value).is_err());
    }
    #[test]
    fn restart_and_stale_google_data_preserve_the_latest_local_edit() {
        let (_dir, path) = db();
        let old = edit(&path, "first");
        let latest = edit(&path, "second");
        assert_eq!(count(&path), 2);
        let mut fetched = vec![old];
        overlay(&path, &mut fetched, Some(crate::clock::now())).unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].title, latest.title);
    }
    #[test]
    fn edit_while_uploading_survives_the_older_acknowledgement() {
        let (_dir, path) = db();
        let first = edit(&path, "first");
        drain_in(&path, |_| {
            edit(&path, "edited during network request");
            Ok(None)
        })
        .unwrap();
        assert_eq!(count(&path), 1);
        let mut fetched = vec![first];
        overlay(&path, &mut fetched, Some(crate::clock::now())).unwrap();
        assert_eq!(fetched[0].title, "edited during network request");
        drain_in(&path, |_| Ok(None)).unwrap();
        assert_eq!(count(&path), 0);
    }
    #[test]
    fn deleting_a_block_releases_children_durably_and_atomically() {
        let (_dir, path) = db();
        let block = super::super::sample_snapshot().events.remove(0);
        let mut child = block.clone();
        child.id = "task:child".into();
        child.calendar_id = "tasks:list".into();
        let mut database = task_db::Database::open(&path).unwrap();
        let mut data = database.import(&child).unwrap();
        data.metadata.block = Some(crate::timeblocking::block_key(&block));
        child.details.task_data = Some(data);
        cache_tasks_in(&path, &[child.clone()], true).unwrap();
        let delete = Job::Delete {
            calendar: block.calendar_id.clone(),
            id: block.id.clone(),
            scope: SaveScope::Current,
            notify: false,
        };
        database.connection().execute_batch("CREATE TRIGGER reject_child BEFORE INSERT ON google_operations WHEN NEW.kind='Edit task Unassign' BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
        assert!(enqueue_in(&path, delete.clone(), None, &block.calendar_id, &block.id).is_err());
        assert_eq!(count(&path), 0);
        database
            .connection()
            .execute_batch("DROP TRIGGER reject_child")
            .unwrap();
        enqueue_in(&path, delete, None, &block.calendar_id, &block.id).unwrap();
        assert_eq!(count(&path), 2);
        let mut restored = vec![block.clone(), child];
        overlay(&path, &mut restored, None).unwrap();
        assert_eq!(restored.len(), 1);
        assert!(restored[0].all_day);
        assert!(crate::timeblocking::metadata(&restored[0]).block.is_none());
        assert_eq!(restored[0].start.date_naive(), block.start.date_naive());
    }

    #[test]
    fn complete_task_cache_keeps_times_and_cannot_overwrite_edits_or_deletions() {
        let (_dir, path) = db();
        let mut task = super::super::sample_snapshot().events.remove(0);
        task.id = "task:cached".into();
        task.calendar_id = "tasks:list".into();
        task.all_day = false;
        cache_tasks_in(&path, &[task.clone()], true).unwrap();
        // A stale fetch finishes after an edit has already committed.
        let mut edited = task.clone();
        edited.title = "new local title".into();
        enqueue_in(
            &path,
            Job::Status {
                task: task.clone(),
                status: crate::timeblocking::TaskStatus::Active,
            },
            Some(&edited),
            &task.calendar_id,
            &task.id,
        )
        .unwrap();
        cache_tasks_in(&path, &[task.clone()], true).unwrap();
        let restored = super::super::day_cache::load(&path, task.start.date_naive(), 0).unwrap();
        let restored = restored.iter().find(|e| e.id == task.id).unwrap();
        assert_eq!(restored.title, edited.title);
        assert!(!restored.all_day);
        enqueue_in(
            &path,
            Job::DeleteTask {
                task: task.clone(),
                calendar: None,
            },
            None,
            &task.calendar_id,
            &task.id,
        )
        .unwrap();
        cache_tasks_in(&path, &[task.clone()], true).unwrap();
        let restored = super::super::day_cache::load(&path, task.start.date_naive(), 0).unwrap();
        assert!(!restored.iter().any(|e| e.id == task.id));
    }

    #[test]
    fn complete_task_snapshot_supersedes_task_copies_in_old_months() {
        let (_dir, path) = db();
        let mut task = super::super::sample_snapshot().events.remove(0);
        task.id = "task:deleted-remotely".into();
        task.calendar_id = "tasks:list".into();
        let day = task.start.date_naive();
        super::super::day_cache::save(&path, day, &[task.clone()], 0).unwrap();
        cache_tasks_in(&path, &[], true).unwrap();
        assert!(
            !super::super::day_cache::load(&path, day, 0)
                .unwrap()
                .iter()
                .any(Event::is_task)
        );
    }

    #[test]
    fn failed_upload_keeps_both_the_transaction_and_local_value() {
        let (_dir, path) = db();
        let event = edit(&path, "saved offline");
        drain_in(&path, |_| Err("network unavailable".into())).unwrap();
        assert_eq!(count(&path), 1);
        let mut db = task_db::Database::open(&path).unwrap();
        let error: String = db
            .connection()
            .query_row("SELECT error FROM google_operations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(error, "network unavailable");
        let mut fetched = Vec::new();
        overlay(&path, &mut fetched, None).unwrap();
        assert_eq!(fetched[0].title, event.title);
        drain_in(&path, |_| panic!("retry backoff must be respected")).unwrap();
        db.connection()
            .execute("UPDATE google_operations SET lease_until=0", [])
            .unwrap();
        drain_in(&path, |_| Ok(None)).unwrap();
        assert_eq!(count(&path), 0);
    }
    #[test]
    fn deletion_is_a_durable_tombstone_until_google_accepts_it() {
        let (_dir, path) = db();
        let event = edit(&path, "delete me");
        enqueue_in(
            &path,
            Job::Delete {
                calendar: event.calendar_id.clone(),
                id: event.id.clone(),
                scope: SaveScope::Current,
                notify: false,
            },
            None,
            &event.calendar_id,
            &event.id,
        )
        .unwrap();
        let mut fetched = vec![event];
        overlay(&path, &mut fetched, None).unwrap();
        assert!(fetched.is_empty());
    }
    #[test]
    fn sql_failure_rolls_back_the_projection_and_queue_together() {
        let (_dir, path) = db();
        let mut db = task_db::Database::open(&path).unwrap();
        db.connection().execute_batch("CREATE TRIGGER reject_queue BEFORE INSERT ON google_operations BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
        let event = super::super::sample_snapshot().events.remove(0);
        assert!(
            enqueue_in(
                &path,
                Job::Delete {
                    calendar: event.calendar_id.clone(),
                    id: event.id.clone(),
                    scope: SaveScope::Current,
                    notify: false
                },
                None,
                &event.calendar_id,
                &event.id
            )
            .is_err()
        );
        assert_eq!(count(&path), 0);
        let entities: i64 = db
            .connection()
            .query_row("SELECT count(*) FROM google_entities", [], |r| r.get(0))
            .unwrap();
        assert_eq!(entities, 0);
    }
    #[test]
    fn task_id_changes_are_resolved_through_multiple_moves() {
        let (_dir, path) = db();
        let local = edit(&path, "first");
        let local_id = local.id.clone();
        drain_in(&path, |_| {
            Ok(Some((
                local_id.clone(),
                "remote-one".into(),
                "first-list".into(),
            )))
        })
        .unwrap();
        edit(&path, "second");
        drain_in(&path, |job| {
            let Job::Edit { original, .. } = job else {
                panic!("expected edit")
            };
            assert_eq!(original.id, "remote-one");
            assert_eq!(original.calendar_id, "first-list");
            Ok(Some((
                "remote-one".into(),
                "remote-two".into(),
                "second-list".into(),
            )))
        })
        .unwrap();
        edit(&path, "third");
        drain_in(&path, |job| {
            let Job::Edit { original, .. } = job else {
                panic!("expected edit")
            };
            assert_eq!(original.id, "remote-two");
            assert_eq!(original.calendar_id, "second-list");
            Ok(None)
        })
        .unwrap();
        let mut remote = local.clone();
        remote.id = "remote-two".into();
        remote.calendar_id = "second-list".into();
        let mut fetched = vec![remote];
        overlay(&path, &mut fetched, None).unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].id, local_id);
        assert_eq!(fetched[0].title, "third");
    }
    #[test]
    fn corrected_value_can_replace_a_failed_inflight_value() {
        let (_dir, path) = db();
        edit(&path, "invalid old value");
        drain_in(&path, |_| {
            edit(&path, "corrected value");
            Err("Google rejected the old value".into())
        })
        .unwrap();
        assert_eq!(count(&path), 1);
        drain_in(&path, |_| Ok(None)).unwrap();
        assert_eq!(count(&path), 0);
    }
    #[test]
    fn confirmed_remote_changes_update_the_disk_projection_and_reject_older_fetches() {
        let (_dir, path) = db();
        let mut remote = edit(&path, "local edit");
        drain_in(&path, |_| Ok(None)).unwrap();
        let fetched = crate::clock::now() + chrono::Duration::seconds(1);
        remote.title = "later Google edit".into();
        let mut events = vec![remote.clone()];
        overlay(&path, &mut events, Some(fetched)).unwrap();
        observe_range(
            &path,
            &events,
            remote.start.date_naive(),
            remote.end.date_naive(),
            fetched,
        )
        .unwrap();
        let mut old = vec![remote.clone()];
        old[0].title = "stale response".into();
        overlay(
            &path,
            &mut old,
            Some(fetched - chrono::Duration::seconds(2)),
        )
        .unwrap();
        assert_eq!(old[0].title, "later Google edit");
        let mut offline = Vec::new();
        overlay(&path, &mut offline, None).unwrap();
        assert_eq!(offline[0].title, "later Google edit");
    }
    #[test]
    fn committed_edits_are_visible_without_a_google_refresh() {
        let (_dir, path) = db();
        let snapshot = super::super::sample_snapshot();
        let mut store = Store::with_snapshot(snapshot);
        store.cache_days_in(Some(path.clone()));
        let latest = edit(&path, "immediate edit");
        store.apply_local_operations().unwrap();
        assert_eq!(store.event(&latest.id).unwrap().title, "immediate edit");
    }
    #[test]
    fn retries_keep_reserved_schedule_ids_without_overwriting_local_metadata() {
        let (_dir, path) = db();
        let mut db = task_db::Database::open(&path).unwrap();
        let mut task = super::super::sample_snapshot().events.remove(0);
        task.id = "task:local".into();
        task.calendar_id = "tasks:inbox".into();
        let data = db.import(&task).unwrap();
        task.details.task_data = Some(data.clone());
        enqueue_in(
            &path,
            Job::TaskEdit {
                task: task.clone(),
                change: task_edit::Change::Title("new title".into()),
                order: 0,
            },
            Some(&task),
            &task.calendar_id,
            &task.id,
        )
        .unwrap();
        let mut reserved = None;
        drain_in(&path, |_| {
            let mut db = task_db::Database::open(&path)?;
            let mut progress = db.import(&task)?;
            let mut moved = task.clone();
            moved.id = "task:moved".into();
            moved.calendar_id = "tasks:destination".into();
            let link = scheduled::Link::new("calendar".into(), &moved)?;
            reserved = Some(link.event_id.clone());
            progress.metadata.schedule = Some(link);
            db.save(&moved, &progress)?;
            Err("Calendar timed out after Tasks moved".into())
        })
        .unwrap();
        assert!(
            db.lookup(&task)
                .unwrap()
                .unwrap()
                .metadata
                .schedule
                .is_none()
        );
        db.connection()
            .execute("UPDATE google_operations SET lease_until=0", [])
            .unwrap();
        drain_in(&path, |job| {
            let Job::TaskEdit { task, .. } = job else {
                panic!("expected task edit")
            };
            assert_eq!(task.id, "task:moved");
            assert_eq!(task.calendar_id, "tasks:destination");
            let mut db = task_db::Database::open(&path)?;
            assert_eq!(
                db.import(&task)?.metadata.schedule.unwrap().event_id,
                reserved.unwrap()
            );
            Ok(None)
        })
        .unwrap();
        let left: i64 = db
            .connection()
            .query_row("SELECT count(*) FROM google_operation_progress", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(left, 0);
    }
    #[test]
    fn later_queued_edits_inherit_backend_links_without_losing_their_newer_fields() {
        let (_dir, path) = db();
        let mut db = task_db::Database::open(&path).unwrap();
        let mut task = super::super::sample_snapshot().events.remove(0);
        task.id = "task:local".into();
        task.calendar_id = "tasks:inbox".into();
        task.details.task_data = Some(db.import(&task).unwrap());
        let job = Job::TaskEdit {
            task: task.clone(),
            change: task_edit::Change::Title("first".into()),
            order: 0,
        };
        enqueue_in(&path, job, Some(&task), &task.calendar_id, &task.id).unwrap();
        let mut newer = task.clone();
        newer.details.task_data.as_mut().unwrap().metadata.minutes = 90;
        enqueue_in(
            &path,
            Job::TaskEdit {
                task: newer.clone(),
                change: task_edit::Change::Title("second".into()),
                order: 0,
            },
            Some(&newer),
            &task.calendar_id,
            &task.id,
        )
        .unwrap();
        drain_in(&path, |_| {
            let mut db = task_db::Database::open(&path)?;
            let mut data = db.import(&task)?;
            data.metadata.schedule = Some(scheduled::Link::new("calendar".into(), &task)?);
            db.save(&task, &data)?;
            Err("Earlier request failed after reserving its calendar ID".into())
        })
        .unwrap();
        drain_in(&path, |job| {
            let Job::TaskEdit { task, .. } = job else {
                panic!("expected task edit")
            };
            let mut db = task_db::Database::open(&path)?;
            let data = db.import(&task)?;
            assert!(data.metadata.schedule.is_some());
            assert_eq!(data.metadata.minutes, 90);
            Ok(None)
        })
        .unwrap();
    }
}
