//! Writes to Google, sent as soon as they are made.
//!
//! An edit is recorded locally first (the item's wanted state in
//! `google_entities`, marked as sending) so the interface can show it, greyed
//! out and locked, straight away. A worker thread then sends it to Google.
//! Success clears the mark; failure drops the local copy so the item falls
//! back to what Google has. Nothing is retried and nothing waits in a queue:
//! every write is logged (see [`log_since`]) instead.
use super::*;
use crate::{config::CalendarSettings, task_db, task_edit};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::{
    cell::Cell,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

thread_local! {
    static REPLAY: Cell<bool> = const { Cell::new(false) };
    static INSERT_ID: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// The local ID of the task the write being sent is for.
    static SENDING_TASK: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}
pub(super) fn insert_id() -> Option<String> {
    INSERT_ID.with(|id| id.borrow().clone())
}
/// The local ID of the task whose write is being sent on this thread, if any.
pub(crate) fn sending_task() -> Option<String> {
    SENDING_TASK.with(|id| id.borrow().clone())
}
/// This thread is sending a write to Google, so the calls it makes must talk
/// to Google rather than record another write.
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
        /// The block as shown, when the tasks planned in the occurrences
        /// deleted go too rather than being taken out of the block.
        #[serde(default)]
        with_tasks: Option<Box<Event>>,
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
    CreateCalendarTask {
        task: Box<Event>,
    },
    CreateTask {
        task: Box<Event>,
        notes: String,
        due: Option<NaiveDate>,
        metadata: Option<crate::timeblocking::Metadata>,
    },
}
/// A write that has been recorded locally and is on its way to Google.
#[derive(Clone)]
struct Write {
    /// `google_entities.id` of the item it changes.
    entity: String,
    /// What it does, for the log: "Edit task Title “Buy milk”".
    label: String,
    job: Job,
    /// The cached task record it replaced, restored if Google refuses it:
    /// the record's ID and its JSON before (`None` when there was none).
    previous_task: Option<(String, Option<String>)>,
}

/// One write the log mentions: being sent, saved, or refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// Increases with every entry, so a reader can ask for what is new.
    pub sequence: u64,
    pub at: DateTime<Local>,
    pub text: String,
    pub failed: bool,
}

static LOG: Mutex<Vec<LogEntry>> = Mutex::new(Vec::new());
static LOG_SEQUENCE: AtomicU64 = AtomicU64::new(0);
/// How many entries [`LOG`] keeps.
const LOG_LIMIT: usize = 200;

/// Log entries written after `sequence`, oldest first.
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

fn log_write(text: String, failed: bool) {
    if failed {
        log::warn!("Google: {text}");
    } else {
        log::info!("Google: {text}");
    }
    if let Ok(mut log) = LOG.lock() {
        log.push(LogEntry {
            sequence: LOG_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1,
            at: crate::clock::now(),
            text,
            failed,
        });
        let excess = log.len().saturating_sub(LOG_LIMIT);
        log.drain(..excess);
    }
}

/// Writes recorded by this process and not yet answered, per database and
/// item. An item's mark is only cleared when its last write is answered.
static IN_FLIGHT: Mutex<Option<HashMap<(PathBuf, String), usize>>> = Mutex::new(None);

fn begin(path: &Path, entity: &str) {
    if let Ok(mut map) = IN_FLIGHT.lock() {
        *map.get_or_insert_with(HashMap::new)
            .entry((path.to_owned(), entity.to_owned()))
            .or_default() += 1;
    }
}

/// Counts one write for `entity` as answered. True when it was the last.
fn end(path: &Path, entity: &str) -> bool {
    let Ok(mut map) = IN_FLIGHT.lock() else {
        return true;
    };
    let map = map.get_or_insert_with(HashMap::new);
    let key = (path.to_owned(), entity.to_owned());
    match map.get_mut(&key) {
        Some(count) if *count > 1 => {
            *count -= 1;
            false
        }
        _ => {
            map.remove(&key);
            true
        }
    }
}

/// How many writes this process is still waiting on Google for.
pub fn sending() -> usize {
    IN_FLIGHT
        .lock()
        .ok()
        .and_then(|map| map.as_ref().map(|map| map.values().sum()))
        .unwrap_or(0)
}

/// A write still marked as sending after this long was cut off (the process
/// that sent it quit or crashed). Its item is unlocked and falls back to what
/// Google has.
const STALE_SENDING: chrono::Duration = chrono::Duration::minutes(10);

fn stale_cutoff() -> String {
    (crate::clock::utc_now() - STALE_SENDING).to_rfc3339()
}

/// Forgets local copies whose write was cut off before Google answered.
fn release_stale(tx: &rusqlite::Connection) -> rusqlite::Result<usize> {
    tx.execute(
        "DELETE FROM google_entities WHERE sending_since IS NOT NULL AND sending_since < ?1",
        [stale_cutoff()],
    )
}

pub fn new_id() -> Result<String, String> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| e.to_string())?;
    Ok(b.iter().map(|v| format!("{v:02x}")).collect())
}
fn key(calendar: &str, id: &str) -> String {
    format!("{calendar}\n{id}")
}

/// Shows `event` (or, for `None`, the item's removal) locally at once, locked
/// until Google answers, and sends `job` to Google in the background.
pub fn submit(job: Job, event: Option<&Event>, calendar: &str, id: &str) -> Result<(), String> {
    let path = task_db::path().ok_or("Cannot locate database")?;
    let writes = record_in(&path, job, event, calendar, id)?;
    for write in writes {
        log_write(format!("Saving to Google: {}", write.label), false);
        // Test runs keep everything local: nothing may reach Google.
        if crate::config::isolated() {
            send_in(&path, write, |_| Ok(None))?;
        } else {
            worker(&path).send(Work::Write(Box::new(write)));
        }
    }
    Ok(())
}

/// What the worker thread is asked to do.
enum Work {
    Write(Box<Write>),
    /// Bring time blocks back in step with Google once the writes are sent.
    Reconcile,
}

/// The thread that sends this process's writes, one at a time and in the
/// order they were made, so an edit cannot overtake its item's creation.
/// Once it has sent everything it was given, and a write touched the app's
/// calendar or a sync asked for it, it brings time blocks back in step (see
/// [`crate::timeblocking::reconcile`]).
struct Worker(Mutex<mpsc::Sender<Work>>);

impl Worker {
    fn send(&self, work: Work) {
        let sent = self.0.lock().is_ok_and(|sender| sender.send(work).is_ok());
        if !sent {
            log_write("Could not hand a write to the Google worker".into(), true);
        }
    }
}

fn worker(path: &Path) -> &'static Worker {
    static WORKER: OnceLock<Worker> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Work>();
        let path = path.to_owned();
        std::thread::spawn(move || {
            let mut stale = false;
            while let Ok(mut work) = receiver.recv() {
                loop {
                    match work {
                        Work::Write(write) => {
                            let app = crate::config::Config::load().calendar.app_calendar;
                            stale |= touches_blocks(&write.job, app.as_deref());
                            let entity = write.entity.clone();
                            super::series::take_noted();
                            let result = send_in(&path, *write, |job| {
                                let settings = crate::config::Config::load().calendar;
                                execute(&GoogleClient::from_settings(&settings), &settings, job)
                            });
                            if let Err(error) = result {
                                log_write(
                                    format!("Could not record a Google answer: {error}"),
                                    true,
                                );
                            }
                            // A write that changed a whole series has been
                            // acknowledged by now, so a fetch from here on
                            // shows it on every day.
                            super::series::refresh(&path, &entity, super::series::take_noted());
                        }
                        Work::Reconcile => stale = true,
                    }
                    match receiver.try_recv() {
                        Ok(next) => work = next,
                        Err(_) => break,
                    }
                }
                if std::mem::take(&mut stale) {
                    reconcile_blocks(&path);
                }
            }
        });
        Worker(Mutex::new(sender))
    })
}

/// Whether a write can leave time blocks out of step: anything on the app's
/// calendar (`app`), where blocks and the tasks repeating with them live,
/// and any task edit.
fn touches_blocks(job: &Job, app: Option<&str>) -> bool {
    let on_app = |calendar: &str| app == Some(calendar);
    match job {
        Job::Edit { original, .. } => on_app(&original.calendar_id),
        Job::Time { calendar, .. }
        | Job::Delete { calendar, .. }
        | Job::Create { calendar, .. } => on_app(calendar),
        Job::TaskEdit { .. } | Job::CreateCalendarTask { .. } | Job::DeleteTask { .. } => true,
        Job::Location { .. }
        | Job::Status { .. }
        | Job::Sleep { .. }
        | Job::HabitCompleted { .. }
        | Job::Reply { .. }
        | Job::CreateHabit { .. }
        | Job::CreateTask { .. } => false,
    }
}

/// How far ahead time blocks are kept in step. Further days are put right
/// once they come this close.
const RECONCILE_DAYS: i64 = 56;

/// When a sync last asked for time blocks to be brought in step, so a fast
/// sync interval does not reconcile every time.
static LAST_REQUEST: Mutex<Option<DateTime<Local>>> = Mutex::new(None);

/// Asks the worker to bring time blocks back in step with Google, as after a
/// sync, which may have brought changes made elsewhere. At most every few
/// minutes; writes from this app always do it.
pub fn request_reconcile() {
    if crate::config::isolated() {
        return;
    }
    let now = crate::clock::now();
    {
        let Ok(mut last) = LAST_REQUEST.lock() else {
            return;
        };
        if last.is_some_and(|last| now - last < chrono::Duration::minutes(5)) {
            return;
        }
        *last = Some(now);
    }
    if let Some(path) = task_db::path() {
        worker(&path).send(Work::Reconcile);
    }
}

/// Brings the app calendar's time blocks back in step with Google from
/// today on, then fetches the series it changed again for the caches.
/// Failure only means waiting for the next try, so it is logged.
fn reconcile_blocks(path: &Path) {
    let settings = crate::config::Config::load().calendar;
    let Some(calendar) = settings.app_calendar.clone() else {
        return;
    };
    let hour = settings.day_start_hour();
    let today = day_at(crate::clock::now(), hour);
    let first = at_clock(today, hour, 0);
    let last = first + chrono::Duration::days(RECONCILE_DAYS);
    let token = match GoogleClient::from_settings(&settings).access_token() {
        Ok(token) => token,
        Err(error) => {
            log::warn!("time blocks not reconciled: {error}");
            return;
        }
    };
    let mut remote = api::block_sync::GoogleRemote::new(token, &settings, &calendar);
    REPLAY.with(|r| r.set(true));
    let result =
        crate::timeblocking::reconcile::reconcile(&mut remote, &calendar, first, last, hour, today);
    REPLAY.with(|r| r.set(false));
    match result {
        Ok(fixes) if fixes.is_empty() => {}
        Ok(fixes) => log_write(
            format!("Kept {} time block change(s) in step", fixes.len()),
            false,
        ),
        Err(error) => log_write(format!("Could not keep time blocks in step: {error}"), true),
    }
    let changed: Vec<super::series::SeriesRef> = remote
        .changed
        .iter()
        .map(|id| super::series::SeriesRef {
            calendar: calendar.clone(),
            id: id.clone(),
            calendars: vec![calendar.clone()],
        })
        .collect();
    super::series::refresh(path, "", changed);
}

/// Records the wanted state locally, marked as sending, in one transaction,
/// and returns the writes that carry it to Google.
fn record_in(
    path: &Path,
    job: Job,
    event: Option<&Event>,
    calendar: &str,
    id: &str,
) -> Result<Vec<Write>, String> {
    let mut db = task_db::Database::open(path)?;
    let tx = db
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let mut writes = vec![record_tx(&tx, job.clone(), event, calendar, id)?];
    if let Job::Delete {
        calendar,
        id,
        scope,
        with_tasks,
        ..
    } = &job
    {
        // Removing a container releases its children; deleting the container
        // and recording all child edits is one local transaction. Which tasks
        // are its one-offs is decided as everywhere else, so one placed
        // before its series was retimed still counts. Tasks repeating with
        // it are not released: the reconciler deletes that day of them.
        let block = with_tasks
            .as_deref()
            .cloned()
            .unwrap_or_else(|| deleted_block(calendar, id));
        let hour = crate::config::Config::load().calendar.day_start_hour();
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
        if with_tasks.is_some() {
            // The one-offs in every occurrence deleted go with them. Tasks
            // repeating with the block are their own series, which sending
            // the delete ends (see `send::delete_followers`).
            for task in children.iter().filter(|child| {
                !crate::timeblocking::is_follower(child)
                    && crate::timeblocking::membership::deleted_with(child, &block, *scope, hour)
            }) {
                writes.push(record_tx(
                    &tx,
                    Job::DeleteTask {
                        task: task.clone(),
                        calendar: None,
                    },
                    None,
                    &task.calendar_id,
                    &task.id,
                )?);
            }
        }
        for original in children.into_iter().filter(|child| {
            with_tasks.is_none()
                && !crate::timeblocking::is_follower(child)
                && crate::timeblocking::in_block(child, &block, hour)
        }) {
            let mut released = original.clone();
            let data = released
                .details
                .task_data
                .as_mut()
                .ok_or("Assigned task has no metadata")?;
            data.metadata.block = None;
            data.metadata.repeat_source = None;
            released.all_day = true;
            released.start = at_clock(original.start.date_naive(), 0, 0);
            released.end = released.start + chrono::Duration::days(1);
            writes.push(record_tx(
                &tx,
                Job::TaskEdit {
                    task: original.clone(),
                    change: task_edit::Change::Unassign,
                    order: 0,
                },
                Some(&released),
                &original.calendar_id,
                &original.id,
            )?);
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    for write in &writes {
        begin(path, &write.entity);
    }
    Ok(writes)
}

/// Enough of a deleted event, known only by its ID, to tell which tasks
/// were in it: for an occurrence, its series and original start, which its
/// ID holds.
fn deleted_block(calendar: &str, id: &str) -> Event {
    let instance = crate::timeblocking::membership::parse_instance_id(id);
    let start = instance.map_or_else(crate::clock::now, |(_, start)| start);
    Event {
        id: id.to_owned(),
        calendar_id: calendar.to_owned(),
        title: String::new(),
        start,
        end: start,
        all_day: false,
        color: colors::DEFAULT_EVENT,
        completed: false,
        details: Details {
            recurring_event_id: instance.map(|(series, _)| series.to_owned()),
            original_start: instance.map(|(_, start)| start),
            ..Default::default()
        },
    }
}

/// What a job does, as the log says it.
fn describe(job: &Job, event: Option<&Event>) -> String {
    let kind = match job {
        Job::Edit { .. } => "Edit event",
        Job::Time { .. } => "Change time",
        Job::Location { .. } => "Change location",
        Job::Status { .. } => "Task status",
        Job::TaskEdit { .. } => "Edit task",
        Job::Delete { .. } => "Delete event",
        Job::DeleteTask { .. } => "Delete task",
        Job::Create { .. } => "Create event",
        Job::CreateTask { .. } | Job::CreateCalendarTask { .. } => "Create task",
        Job::Sleep { .. } => "Save sleep",
        Job::HabitCompleted { .. } => "Habit completion",
        Job::Reply { .. } => "Reply to invitation",
        Job::CreateHabit { .. } => "Create habit",
    };
    let lane = match job {
        Job::TaskEdit { change, .. } => serde_json::to_value(change)
            .ok()
            .and_then(|value| {
                value
                    .as_object()
                    .and_then(|o| o.keys().next().cloned())
                    .or_else(|| value.as_str().map(str::to_owned))
            })
            .unwrap_or_default(),
        Job::Edit { scope, .. } => format!("{scope:?}"),
        _ => String::new(),
    };
    let title = match job {
        Job::DeleteTask { task, .. } | Job::Status { task, .. } => Some(task.title.as_str()),
        Job::Create { title, .. } => Some(title.as_str()),
        _ => event.map(|event| event.title.as_str()),
    }
    .filter(|title| !title.trim().is_empty());
    let mut label = format!("{kind} {lane}").trim().to_owned();
    if let Some(title) = title {
        label.push_str(&format!(" “{}”", title.trim()));
    }
    label
}

fn record_tx(
    tx: &rusqlite::Connection,
    job: Job,
    event: Option<&Event>,
    calendar: &str,
    id: &str,
) -> Result<Write, String> {
    let mut entity = key(calendar, id);
    let label = describe(&job, event);
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
    // One row per item. A row recorded for this item under an ID Google has
    // since renamed it from is the same item: this write takes that row over,
    // and any others are dropped, so no older copy can be shown beside it.
    if let Some(wanted) = event {
        let hour = crate::config::CalendarSettings::stored_day_start_hour(tx);
        let rows = {
            let mut q = tx
                .prepare("SELECT id,event FROM google_entities WHERE event IS NOT NULL")
                .map_err(|e| e.to_string())?;
            q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        let mut same: Vec<String> = rows
            .into_iter()
            .filter(|(key, json)| {
                *key != entity
                    && serde_json::from_str::<Event>(json).is_ok_and(|saved| {
                        crate::timeblocking::membership::same_item(&saved, wanted, hour)
                    })
            })
            .map(|(key, _)| key)
            .collect();
        let own_row = tx
            .query_row(
                "SELECT 1 FROM google_entities WHERE id=?1",
                [&entity],
                |_| Ok(()),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .is_some();
        if !own_row && !same.is_empty() {
            entity = same.remove(0);
        }
        for other in same {
            tx.execute("DELETE FROM google_entities WHERE id=?1", [other])
                .map_err(|e| e.to_string())?;
        }
    }
    let record_id = event.filter(|e| e.is_task()).map_or(id, |e| e.id.as_str());
    let previous_task = tx
        .query_row(
            "SELECT event FROM google_task_records WHERE id=?1",
            [record_id],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let touches_task = previous_task.is_some() || event.is_some_and(Event::is_task);
    (|| -> rusqlite::Result<()> {
        tx.execute(
            "INSERT INTO google_entities(id,event,acknowledged_at,observed_at,sending_since) VALUES(?1,?2,NULL,NULL,?3) ON CONFLICT(id) DO UPDATE SET event=excluded.event,acknowledged_at=NULL,observed_at=NULL,sending_since=excluded.sending_since",
            params![entity, saved, crate::clock::utc_now().to_rfc3339()],
        )?;
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
        Ok(())
    })()
    .map_err(|e| e.to_string())?;
    Ok(Write {
        entity,
        label,
        job,
        previous_task: touches_task.then(|| (record_id.to_owned(), previous_task)),
    })
}
/// What [`overlay`] applies, read from the database in one go so the reading
/// can happen on a worker thread and the applying on the UI thread.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Overlay {
    /// Remote ID to the local ID it was created under, so an item keeps the
    /// ID the interface already knows it by.
    aliases: HashMap<(String, String), String>,
    entities: Vec<OverlayEntity>,
    /// Decides which planning day an occurrence is on, and so which renamed
    /// occurrence is the same item (see [`apply`](Self::apply)).
    day_start_hour: u32,
}

#[derive(Debug, Clone, PartialEq)]
struct OverlayEntity {
    calendar: String,
    id: String,
    /// `None` for an item deleted locally.
    event: Option<Event>,
    acknowledged_at: Option<DateTime<chrono::FixedOffset>>,
    /// Its write is on its way to Google, so the item is shown locked.
    sending: bool,
    /// A fetch begun after Google confirmed the write has shown the item.
    observed: bool,
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
            .prepare(
                "SELECT id,event,acknowledged_at,sending_since,observed_at IS NOT NULL FROM google_entities",
            )
            .map_err(|e| e.to_string())?;
        let cutoff = stale_cutoff();
        for row in q
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, bool>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?
        {
            let (key, json, ack, sending_since, observed) = row.map_err(|e| e.to_string())?;
            // A write cut off long ago no longer counts: the item is unlocked
            // and shows what Google has.
            if sending_since.as_ref().is_some_and(|since| *since < cutoff) {
                continue;
            }
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
                sending: sending_since.is_some(),
                observed,
            });
        }
    }
    Ok(Overlay {
        aliases,
        entities,
        day_start_hour: crate::config::CalendarSettings::stored_day_start_hour(tx),
    })
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
    /// the fetch already shows it, or something newer. A confirmed edit is
    /// still applied over a fetch that started before it was confirmed.
    /// Without `fetched_at`, every row applies.
    ///
    /// Items are matched with [`same_item`](crate::timeblocking::membership::same_item),
    /// so an occurrence Google renamed is replaced rather than shown twice.
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
        let applies = |entity: &&OverlayEntity| match (fetched_at, entity.acknowledged_at) {
            (Some(fetched), Some(ack)) => fetched <= ack,
            _ => true,
        };
        // There is one row per item (see `record_tx`), but rows written
        // before that held may still overlap. The most current goes last so
        // it is the one left standing: an edit on its way, then one Google
        // confirmed, then what a fetch showed.
        let mut entities: Vec<&OverlayEntity> = self.entities.iter().filter(applies).collect();
        entities.sort_by_key(|entity| (!entity.observed, entity.sending));
        let hour = self.day_start_hour;
        for entity in entities {
            events.retain(|e| {
                let by_id = e.id == entity.id
                    && (e.calendar_id == entity.calendar
                        || entity
                            .event
                            .as_ref()
                            .is_some_and(|saved| saved.calendar_id == e.calendar_id)
                        || remapped.contains(&(
                            entity.calendar.clone(),
                            e.calendar_id.clone(),
                            e.id.clone(),
                        )));
                let same = entity.event.as_ref().is_some_and(|saved| {
                    crate::timeblocking::membership::same_item(saved, e, hour)
                });
                !by_id && !same
            });
            if let Some(event) = &entity.event {
                let mut event = event.clone();
                event.details.sending = entity.sending;
                events.push(event);
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
/// A successful write that gave something a new ID: the local ID, Google's,
/// and the calendar or task list it is on.
type Mapping = Option<(String, String, String)>;

/// Sends one recorded write and records Google's answer: on success the item
/// is unlocked, on failure its local copy is dropped so it shows what Google
/// has. Either way the outcome is logged.
fn send_in(
    path: &Path,
    write: Write,
    run: impl FnOnce(Job) -> Result<Mapping, google::GoogleError>,
) -> Result<(), String> {
    let Write {
        entity,
        label,
        job,
        previous_task,
    } = write;
    let task = match &job {
        Job::TaskEdit { task, .. } | Job::Status { task, .. } => Some(task.id.clone()),
        Job::CreateTask { task, .. } => Some(task.id.clone()),
        _ => None,
    };
    let outcome = remapped(path, job)
        .map_err(google::GoogleError::from)
        .and_then(|job| {
            REPLAY.with(|r| r.set(true));
            SENDING_TASK.with(|t| *t.borrow_mut() = task);
            let result = run(job);
            SENDING_TASK.with(|t| *t.borrow_mut() = None);
            REPLAY.with(|r| r.set(false));
            result
        });
    let last = end(path, &entity);
    let recorded = finish(path, &entity, &outcome, last, previous_task);
    match &outcome {
        Ok(_) => log_write(format!("Saved to Google: {label}"), false),
        Err(error) => log_write(format!("Could not save to Google: {label}: {error}"), true),
    }
    recorded
}

fn finish(
    path: &Path,
    entity: &str,
    outcome: &Result<Mapping, google::GoogleError>,
    last: bool,
    previous_task: Option<(String, Option<String>)>,
) -> Result<(), String> {
    let mut db = task_db::Database::open(path)?;
    let tx = db
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    match outcome {
        Ok(mapping) => {
            if let Some((local, remote, calendar)) = mapping {
                tx.execute("INSERT INTO google_id_map(local,remote,calendar) VALUES(?1,?2,?3) ON CONFLICT(local) DO UPDATE SET remote=excluded.remote,calendar=excluded.calendar",params![local,remote,calendar]).map_err(|e|e.to_string())?;
            }
            // A later write for the same item is still on its way, and its
            // wanted state is the one to keep showing, locked.
            if last {
                tx.execute(
                    "UPDATE google_entities SET acknowledged_at=?2,sending_since=NULL WHERE id=?1",
                    params![entity, crate::clock::now().to_rfc3339()],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        Err(_) if last => {
            tx.execute("DELETE FROM google_entities WHERE id=?1", [entity])
                .map_err(|e| e.to_string())?;
            match previous_task {
                Some((id, Some(json))) => tx.execute(
                    "INSERT INTO google_task_records(id,event) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET event=excluded.event",
                    params![id, json],
                ),
                Some((id, None)) => {
                    tx.execute("DELETE FROM google_task_records WHERE id=?1", [id])
                }
                None => Ok(0),
            }
            .map_err(|e| e.to_string())?;
        }
        Err(_) => {}
    }
    tx.commit().map_err(|e| e.to_string())
}

/// The job with any local IDs replaced by the IDs Google has given those
/// items since it was made (an edit made just after a creation names the
/// item by the ID it was created under).
fn remapped(path: &Path, job: Job) -> Result<Job, String> {
    let mut db = task_db::Database::open(path)?;
    let aliases = {
        let mut q = db
            .connection()
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
    if aliases.is_empty() {
        return Ok(job);
    }
    let mut value = serde_json::to_value(&job).map_err(|e| e.to_string())?;
    remap(&mut value, &aliases);
    serde_json::from_value(value).map_err(|e| e.to_string())
}
/// Rewrites local IDs in a job to the IDs Google gave the items since.
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
                    let full = calendar.starts_with(ids::TASK_LIST);
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
            with_tasks,
        } => {
            send::delete_event_scope(client, &calendar, &id, scope, notify)?;
            super::sleep::forget(&calendar, &id);
            if with_tasks.is_some() {
                send::delete_followers(client, s, &calendar, &id, scope)?;
            }
        }
        Job::DeleteTask { mut task, calendar } => {
            let mut db = task_db::open()?;
            let data = db.import(&task)?;
            task.details.task_data = Some(data);
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
        Job::CreateCalendarTask { mut task } => {
            let old = task.id.clone();
            let token = client.access_token()?;
            super::calendar_tasks::persist(&token, &task.clone(), &mut task, s)?;
            return Ok(Some((old, task.id, task.calendar_id)));
        }
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
    if leaves_series(&task, &change) {
        return detach(task, settings, change, order);
    }
    if matches!(change, task_edit::Change::JoinBlockSeries { .. })
        && task.details.recurring_event_id.is_some()
    {
        return Err("A task that already repeats cannot also repeat with a time block".into());
    }
    let mut db = task_db::open()?;
    if task.details.task_data.is_none() {
        task.details.task_data = Some(db.import(&task)?);
    }
    let original = task.clone();
    task = super::calendar_tasks::apply(task, settings, &change, order)?;
    submit(
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
/// Whether `change` takes one occurrence of a repeating task out of its
/// series. An item in a time block either repeats with the block or is a
/// one-off, so:
///
/// - an occurrence of a task repeating with a block that leaves the block
///   (for another block, another day, a time of its own, or the task lists)
///   leaves its series too, and
/// - an occurrence of a task with a repeat of its own goes into a block only
///   as a one-off.
fn leaves_series(task: &Event, change: &task_edit::Change) -> bool {
    use task_edit::Change;
    if !task.is_task() || task.details.recurring_event_id.is_none() {
        return false;
    }
    let block = matches!(change, Change::Block(..));
    if crate::timeblocking::is_follower(task) {
        block
            || matches!(
                change,
                Change::Unassign
                    | Change::Day(_)
                    | Change::Time(..)
                    | Change::ClearSchedule
                    | Change::Backlog(_)
                    | Change::Inbox
                    | Change::Triaged
            )
    } else {
        block
    }
}

/// Takes one occurrence out of its series: a task standing on its own takes
/// its place, with `change` made, and the occurrence is deleted.
fn detach(
    task: Event,
    settings: &CalendarSettings,
    change: task_edit::Change,
    order: i64,
) -> Result<Event, String> {
    let mut metadata = crate::timeblocking::metadata(&task);
    metadata.block = None;
    metadata.block_series = None;
    metadata.repeat_source = None;
    metadata.created_at = None;
    let notes = super::task_metadata::prose(&task.details.description).to_owned();
    let created = create_task(
        settings,
        &task.title,
        &notes,
        Some(task.start.date_naive()),
        Some(metadata),
    )?;
    submit(
        Job::Delete {
            calendar: task.calendar_id.clone(),
            id: task.id.clone(),
            scope: SaveScope::Current,
            notify: false,
            with_tasks: None,
        },
        None,
        &task.calendar_id,
        &task.id,
    )?;
    if matches!(change, task_edit::Change::Unassign) {
        return Ok(created);
    }
    edit_task(created, settings, change, order)
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
    if let Some(day) = due {
        task = super::calendar_tasks::apply(task, settings, &task_edit::Change::Day(day), 0)?;
        submit(
            Job::CreateCalendarTask {
                task: Box::new(task.clone()),
            },
            Some(&task),
            &task.calendar_id,
            &task.id,
        )?;
        return Ok(task);
    }
    submit(
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

/// What Google now holds for the item a `google_entities` row is about:
/// the event with the row's ID or, when Google has renamed it, the same item
/// under its new ID (see [`same_item`](crate::timeblocking::membership::same_item)).
///
/// Recording only that the old ID is gone would make the row delete the item
/// from any fetch that began before the rename (which has the old ID and not
/// the new one). The row carries the renamed item instead.
fn observed<'a>(
    calendar: &str,
    id: &str,
    old: Option<&Event>,
    events: &'a [Event],
    day_start_hour: u32,
) -> Option<&'a Event> {
    let calendar = old.map_or(calendar, |e| e.calendar_id.as_str());
    if let Some(current) = events
        .iter()
        .find(|e| e.id == id && e.calendar_id == calendar)
    {
        return Some(current);
    }
    let old = old?;
    events
        .iter()
        .find(|e| crate::timeblocking::membership::same_item(old, e, day_start_hour))
        .or_else(|| {
            events
                .iter()
                .find(|e| first_occurrence(old, e, day_start_hour))
        })
}

/// Whether `event` is the occurrence a series' master `master` is shown as:
/// a write that made something a series is seen again as occurrences, none
/// of which has the master's ID.
fn first_occurrence(master: &Event, event: &Event, day_start_hour: u32) -> bool {
    use crate::timeblocking::membership::{original_start, planning_day};
    master.details.recurring_event_id.is_none()
        && !master.details.recurrence.is_empty()
        && event.calendar_id == master.calendar_id
        && event.details.recurring_event_id.as_deref() == Some(master.id.as_str())
        && planning_day(original_start(event), day_start_hour)
            == planning_day(master.start, day_start_hour)
}

/// Advance acknowledged projections only from a fetch that began after the
/// acknowledgement. Older concurrent fetches continue to see the newer row.
pub(super) fn observe_range(
    path: &std::path::Path,
    events: &[Event],
    first: NaiveDate,
    last: NaiveDate,
    fetched: DateTime<Local>,
    day_start_hour: u32,
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
        let current = observed(calendar, id, old.as_ref(), events, day_start_hour);
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
    release_stale(&tx).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

/// [`observe_range`] for a fetch of some series' occurrences between `first`
/// and `last`: the item `entity` and every item on those series (`on_series`).
/// Items elsewhere may simply not have been asked for, so they are left alone.
///
/// Every row on the series is brought up to date, not only the write's own.
/// A write to the whole series changes occurrences other edits were made to
/// before, and their rows would otherwise go on showing them as they were.
#[allow(clippy::too_many_arguments)]
pub(super) fn observe_series(
    path: &std::path::Path,
    entity: &str,
    on_series: impl Fn(&Event) -> bool,
    events: &[Event],
    first: NaiveDate,
    last: NaiveDate,
    fetched: DateTime<Local>,
    day_start_hour: u32,
) -> Result<(), String> {
    let mut db = task_db::Database::open(path)?;
    let tx = db
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let rows = {
        let mut q = tx
            .prepare("SELECT id,event,acknowledged_at FROM google_entities WHERE acknowledged_at IS NOT NULL AND sending_since IS NULL")
            .map_err(|e| e.to_string())?;
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
    for (key, json, ack) in rows {
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
        if key != entity && !old.as_ref().is_some_and(&on_series) {
            continue;
        }
        let current = observed(calendar, id, old.as_ref(), events, day_start_hour);
        let covered = current.is_some()
            || old
                .as_ref()
                .is_some_and(|e| e.start.date_naive() <= last && e.end.date_naive() >= first);
        if covered {
            let json = current
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| e.to_string())?;
            tx.execute("UPDATE google_entities SET event=?2,acknowledged_at=?3,observed_at=COALESCE(observed_at,?5) WHERE id=?1 AND acknowledged_at=?4",params![key,json,fetched.to_rfc3339(),ack,fetched.with_timezone(&Utc).to_rfc3339()]).map_err(|e|e.to_string())?;
        }
    }
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
        "DELETE FROM google_entities WHERE observed_at IS NOT NULL AND observed_at < ?1 AND sending_since IS NULL",
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
    fn edit_job(event: &Event) -> Job {
        Job::Edit {
            original: Box::new(event.clone()),
            draft: EditDraft::from_event(event),
            scope: SaveScope::Current,
            description: String::new(),
            color: None,
        }
    }
    /// Records an edit of the sample event, returning it and its write.
    fn edit(path: &Path, title: &str) -> (Event, Write) {
        edit_other(path, &super::super::sample_snapshot().events[0].id, title)
    }
    fn edit_other(path: &Path, id: &str, title: &str) -> (Event, Write) {
        let mut event = super::super::sample_snapshot().events.remove(0);
        event.id = id.into();
        event.title = title.into();
        let mut writes = record_in(
            path,
            edit_job(&event),
            Some(&event),
            &event.calendar_id,
            &event.id,
        )
        .unwrap();
        (event, writes.remove(0))
    }
    fn entities(path: &Path) -> i64 {
        task_db::Database::open(path)
            .unwrap()
            .connection()
            .query_row("SELECT count(*) FROM google_entities", [], |r| r.get(0))
            .unwrap()
    }
    /// The events as they would be without the sending marks, for comparing
    /// with the events they were made from.
    fn settled(mut events: Vec<Event>) -> Vec<Event> {
        for event in &mut events {
            event.details.sending = false;
        }
        events
    }
    #[test]
    fn an_edit_is_shown_locked_at_once_and_unlocked_when_google_accepts_it() {
        let (_dir, path) = db();
        let (event, write) = edit(&path, "locked");
        let mut shown = Vec::new();
        overlay(&path, &mut shown, None).unwrap();
        assert_eq!(shown[0].title, "locked");
        assert!(shown[0].details.sending);
        send_in(&path, write, |_| Ok(None)).unwrap();
        let mut shown = vec![event.clone()];
        overlay(&path, &mut shown, None).unwrap();
        assert_eq!(shown, [event]);
        assert!(!shown[0].details.sending);
    }
    #[test]
    fn a_refused_write_drops_the_local_copy_so_google_shows_through() {
        let (_dir, path) = db();
        let (_, write) = edit(&path, "refused");
        send_in(&path, write, |_| {
            Err(google::GoogleError::from_status(
                "Google Calendar",
                404,
                "Not Found",
            ))
        })
        .unwrap();
        assert_eq!(entities(&path), 0);
        let mut fetched = Vec::new();
        overlay(&path, &mut fetched, None).unwrap();
        assert!(fetched.is_empty());
    }
    #[test]
    fn a_refused_task_write_restores_the_cached_task() {
        let (_dir, path) = db();
        let mut task = super::super::sample_snapshot().events.remove(0);
        task.id = "task:cached".into();
        task.calendar_id = "tasks:list".into();
        cache_tasks_in(&path, std::slice::from_ref(&task), true).unwrap();
        let mut edited = task.clone();
        edited.title = "never reached Google".into();
        let mut writes = record_in(
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
        send_in(&path, writes.remove(0), |_| Err("offline".into())).unwrap();
        let restored = super::super::day_cache::load(&path, task.start.date_naive(), 0).unwrap();
        let restored = restored.iter().find(|e| e.id == task.id).unwrap();
        assert_eq!(restored.title, task.title);
        assert!(!restored.details.sending);
    }
    #[test]
    fn an_item_stays_locked_until_its_last_write_is_answered() {
        let (_dir, path) = db();
        let (_, first) = edit(&path, "first");
        let (_, second) = edit(&path, "second");
        // The first is refused, but the second still carries what is wanted.
        send_in(&path, first, |_| Err("refused".into())).unwrap();
        let mut shown = Vec::new();
        overlay(&path, &mut shown, None).unwrap();
        assert_eq!(shown[0].title, "second");
        assert!(shown[0].details.sending);
        send_in(&path, second, |_| Ok(None)).unwrap();
        let mut shown = Vec::new();
        overlay(&path, &mut shown, None).unwrap();
        assert_eq!(shown[0].title, "second");
        assert!(!shown[0].details.sending);
    }
    #[test]
    fn edit_while_sending_survives_the_older_acknowledgement() {
        let (_dir, path) = db();
        let (first, write) = edit(&path, "first");
        let mut later = None;
        send_in(&path, write, |_| {
            later = Some(edit(&path, "edited during network request").1);
            Ok(None)
        })
        .unwrap();
        let mut fetched = vec![first];
        overlay(&path, &mut fetched, Some(crate::clock::now())).unwrap();
        assert_eq!(fetched[0].title, "edited during network request");
        assert!(fetched[0].details.sending);
        send_in(&path, later.unwrap(), |_| Ok(None)).unwrap();
        let mut fetched = Vec::new();
        overlay(&path, &mut fetched, None).unwrap();
        assert!(!fetched[0].details.sending);
    }
    #[test]
    fn a_write_cut_off_long_ago_unlocks_its_item() {
        let (_dir, path) = db();
        let (event, _write) = edit(&path, "the app quit before Google answered");
        task_db::Database::open(&path)
            .unwrap()
            .connection()
            .execute(
                "UPDATE google_entities SET sending_since='2000-01-01T00:00:00+00:00'",
                [],
            )
            .unwrap();
        let mut fetched = Vec::new();
        overlay(&path, &mut fetched, None).unwrap();
        assert!(fetched.is_empty());
        let day = event.start.date_naive();
        observe_range(&path, &[], day, day, crate::clock::now(), 0).unwrap();
        assert_eq!(entities(&path), 0);
    }
    #[test]
    fn writes_are_labelled_with_what_they_do() {
        let (_dir, path) = db();
        let (_, write) = edit_other(&path, "a", "Standup");
        assert_eq!(write.label, "Edit event Current “Standup”");
        let task = super::super::sample_snapshot().events.remove(0);
        assert_eq!(
            describe(
                &Job::DeleteTask {
                    task: task.clone(),
                    calendar: None
                },
                None
            ),
            format!("Delete task “{}”", task.title)
        );
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
            day_start_hour: 0,
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
        let (a, _) = edit(&path, "calendar A edit");
        let mut b = a.clone();
        b.calendar_id = "other-calendar".into();
        b.title = "calendar B edit".into();
        record_in(&path, edit_job(&b), Some(&b), &b.calendar_id, &b.id).unwrap();
        let mut events = vec![a.clone(), b.clone()];
        overlay(&path, &mut events, None).unwrap();
        let events = settled(events);
        assert_eq!(events.len(), 2);
        assert!(events.contains(&a));
        assert!(events.contains(&b));
        record_in(
            &path,
            Job::Delete {
                calendar: a.calendar_id.clone(),
                id: a.id.clone(),
                scope: SaveScope::Current,
                notify: false,
                with_tasks: None,
            },
            None,
            &a.calendar_id,
            &a.id,
        )
        .unwrap();
        let mut events = events;
        overlay(&path, &mut events, None).unwrap();
        assert_eq!(settled(events), [b]);
    }

    #[test]
    fn moved_ids_on_other_calendars_are_not_remapped_or_removed() {
        let (_dir, path) = db();
        let (local, write) = edit(&path, "local");
        send_in(&path, write, |_| {
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
        let (original, _) = edit(&path, "original");
        let mut moved = original.clone();
        moved.calendar_id = "destination".into();
        record_in(
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
        record_in(
            &path,
            edit_job(&other),
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
        let events = settled(events);
        assert_eq!(events.len(), 2);
        assert!(events.contains(&moved));
        assert!(events.contains(&other));
    }

    /// An occurrence of `series` first scheduled at `hour:minute` on 12 Nov,
    /// named the way Google names it.
    fn occurrence_at(series: &str, hour: u32, minute: u32) -> Event {
        let mut event = super::super::sample_snapshot().events.remove(0);
        let day = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        event.start = at_clock(day, hour, minute);
        event.end = event.start + chrono::Duration::minutes(45);
        event.all_day = false;
        event.details.recurring_event_id = Some(series.into());
        event.details.original_start = Some(event.start);
        event.id = format!(
            "{series}_{}",
            event.start.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ")
        );
        event
    }

    /// Moving every occurrence of a series renames them all on Google. The
    /// moved occurrence must stay on screen throughout: it used to vanish
    /// once Google confirmed, until the renamed occurrence arrived by
    /// another route, which left a time block's tasks drawn on their own.
    #[test]
    fn an_occurrence_renamed_by_retiming_its_series_never_disappears() {
        let (_dir, path) = db();
        let before = occurrence_at("block", 20, 45);
        let mut moved = before.clone();
        moved.start += chrono::Duration::minutes(30);
        moved.end += chrono::Duration::minutes(30);
        let mut writes = record_in(
            &path,
            Job::Edit {
                original: Box::new(before.clone()),
                draft: EditDraft::from_event(&moved),
                scope: SaveScope::All,
                description: String::new(),
                color: None,
            },
            Some(&moved),
            &before.calendar_id,
            &before.id,
        )
        .unwrap();
        let entity = writes[0].entity.clone();
        // A sync asked for when the move was saved starts before Google has it.
        let stale_fetch = crate::clock::now() - chrono::Duration::seconds(1);
        send_in(&path, writes.remove(0), |_| Ok(None)).unwrap();
        let renamed = occurrence_at("block", 21, 15);
        assert_ne!(renamed.id, before.id);
        let refreshed = crate::clock::now() + chrono::Duration::seconds(1);
        let day = renamed.start.date_naive();
        observe_series(
            &path,
            &entity,
            |_| false,
            std::slice::from_ref(&renamed),
            day,
            day,
            refreshed,
            0,
        )
        .unwrap();

        // The interface still holds the moved occurrence under its old ID.
        let mut shown = vec![moved.clone()];
        overlay(&path, &mut shown, None).unwrap();
        assert_eq!(settled(shown), std::slice::from_ref(&renamed));
        // The renamed occurrence already arrived: it is not doubled.
        let mut shown = vec![moved.clone(), renamed.clone()];
        overlay(&path, &mut shown, None).unwrap();
        assert_eq!(settled(shown), std::slice::from_ref(&renamed));
        // The stale fetch lands: Google still had the old ID at the old time.
        let mut shown = vec![before.clone()];
        overlay(&path, &mut shown, Some(stale_fetch)).unwrap();
        assert_eq!(settled(shown), std::slice::from_ref(&renamed));
        // A fetch from after the rename shows it as it is.
        let mut shown = vec![renamed.clone()];
        overlay(
            &path,
            &mut shown,
            Some(refreshed + chrono::Duration::seconds(1)),
        )
        .unwrap();
        assert_eq!(shown, std::slice::from_ref(&renamed));

        // A full sync from after the rename observes the row too, and keeps
        // carrying the renamed occurrence for fetches still on their way.
        observe_range(
            &path,
            std::slice::from_ref(&renamed),
            day,
            day,
            refreshed + chrono::Duration::seconds(2),
            0,
        )
        .unwrap();
        let mut shown = vec![before.clone()];
        overlay(&path, &mut shown, Some(stale_fetch)).unwrap();
        assert_eq!(settled(shown), std::slice::from_ref(&renamed));

        // Editing the renamed occurrence records a newer row for it, which
        // the carried copy defers to rather than putting the old time back.
        let mut edited = renamed.clone();
        edited.title = "edited".into();
        record_in(
            &path,
            edit_job(&edited),
            Some(&edited),
            &edited.calendar_id,
            &edited.id,
        )
        .unwrap();
        let mut shown = vec![before.clone()];
        overlay(&path, &mut shown, None).unwrap();
        assert_eq!(settled(shown), [edited]);
    }

    /// Deleting an occurrence is not a rename: the next one of its series is
    /// on another day and is never put in its place.
    #[test]
    fn a_deleted_occurrence_is_not_replaced_by_another_day() {
        let (_dir, path) = db();
        let gone = occurrence_at("block", 20, 45);
        let mut next = gone.clone();
        next.start += chrono::Duration::days(1);
        next.end += chrono::Duration::days(1);
        next.details.original_start = Some(next.start);
        next.id = format!(
            "block_{}",
            next.start.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ")
        );
        let mut writes = record_in(
            &path,
            edit_job(&gone),
            Some(&gone),
            &gone.calendar_id,
            &gone.id,
        )
        .unwrap();
        let entity = writes[0].entity.clone();
        send_in(&path, writes.remove(0), |_| Ok(None)).unwrap();
        let fetched = crate::clock::now() + chrono::Duration::seconds(1);
        let first = gone.start.date_naive();
        observe_series(
            &path,
            &entity,
            |_| false,
            std::slice::from_ref(&next),
            first,
            next.start.date_naive(),
            fetched,
            0,
        )
        .unwrap();
        let mut shown = vec![gone.clone(), next.clone()];
        overlay(&path, &mut shown, None).unwrap();
        assert_eq!(settled(shown), [next]);
    }

    fn moved_by(event: &Event, minutes: i64) -> Event {
        let mut moved = event.clone();
        moved.start += chrono::Duration::minutes(minutes);
        moved.end += chrono::Duration::minutes(minutes);
        moved
    }

    fn rows(path: &Path) -> Vec<String> {
        let mut db = task_db::Database::open(path).unwrap();
        let mut q = db
            .connection()
            .prepare("SELECT id FROM google_entities")
            .unwrap();
        q.query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    /// Moved again before any fetch has shown the first move, the block is
    /// already shown under the ID Google renamed it to. The second move takes
    /// over the first one's row: two rows would each put their own copy of
    /// the block on screen, at two different times.
    #[test]
    fn a_renamed_occurrence_moved_again_keeps_one_row() {
        let (_dir, path) = db();
        let before = occurrence_at("block", 20, 45);
        let first = moved_by(&before, 30);
        let mut writes = record_in(
            &path,
            edit_job(&first),
            Some(&first),
            &before.calendar_id,
            &before.id,
        )
        .unwrap();
        send_in(&path, writes.remove(0), |_| Ok(None)).unwrap();
        let renamed = occurrence_at("block", 21, 15);
        let second = moved_by(&renamed, -15);
        record_in(
            &path,
            edit_job(&second),
            Some(&second),
            &renamed.calendar_id,
            &renamed.id,
        )
        .unwrap();
        assert_eq!(rows(&path).len(), 1);
        let mut shown = vec![renamed.clone()];
        overlay(&path, &mut shown, None).unwrap();
        assert_eq!(settled(shown), [second]);
    }

    /// A series-wide write made from one occurrence renames the occurrences
    /// other edits were made to before. Their rows follow, or they would put
    /// those occurrences back as they were beside the renamed ones.
    #[test]
    fn a_write_to_the_whole_series_brings_its_other_rows_up_to_date() {
        let (_dir, path) = db();
        let earlier = occurrence_at("block", 20, 45);
        let mut retitled = earlier.clone();
        retitled.title = "retitled".into();
        let mut writes = record_in(
            &path,
            edit_job(&retitled),
            Some(&retitled),
            &earlier.calendar_id,
            &earlier.id,
        )
        .unwrap();
        send_in(&path, writes.remove(0), |_| Ok(None)).unwrap();
        let day = earlier.start.date_naive();
        observe_range(
            &path,
            std::slice::from_ref(&retitled),
            day,
            day,
            crate::clock::now() + chrono::Duration::seconds(1),
            0,
        )
        .unwrap();
        // Another occurrence moves the whole series half an hour later.
        let mut renamed = occurrence_at("block", 21, 15);
        renamed.title = "retitled".into();
        let series = super::super::series::SeriesRef {
            calendar: renamed.calendar_id.clone(),
            id: "block".into(),
            calendars: vec![renamed.calendar_id.clone()],
        };
        observe_series(
            &path,
            "another occurrence's row",
            |event| {
                event.calendar_id == series.calendar
                    && event.details.recurring_event_id.as_deref() == Some(series.id.as_str())
            },
            std::slice::from_ref(&renamed),
            day,
            day,
            crate::clock::now() + chrono::Duration::seconds(2),
            0,
        )
        .unwrap();
        let mut shown = vec![renamed.clone()];
        overlay(&path, &mut shown, None).unwrap();
        assert_eq!(settled(shown), [renamed]);
    }

    /// Rows for one item written before there was one row per item: an item
    /// is still shown once, as the write on its way to Google has it.
    #[test]
    fn overlapping_rows_show_their_item_once_as_the_newest_write_has_it() {
        let (_dir, path) = db();
        let before = occurrence_at("block", 20, 45);
        let confirmed = moved_by(&before, 30);
        let renamed = occurrence_at("block", 21, 15);
        let sending = moved_by(&renamed, -15);
        let mut db = task_db::Database::open(&path).unwrap();
        for (event, ack, since) in [
            (&confirmed, Some(crate::clock::now().to_rfc3339()), None),
            (&sending, None, Some(crate::clock::utc_now().to_rfc3339())),
        ] {
            db.connection()
                .execute(
                    "INSERT INTO google_entities(id,event,acknowledged_at,sending_since) VALUES(?1,?2,?3,?4)",
                    params![
                        key(&event.calendar_id, &event.id),
                        serde_json::to_string(event).unwrap(),
                        ack,
                        since
                    ],
                )
                .unwrap();
        }
        drop(db);
        let mut shown = vec![before.clone()];
        overlay(&path, &mut shown, None).unwrap();
        assert_eq!(settled(shown), [sending]);
    }

    // A write that made something a series (a task joining a block's series,
    // say) is seen again only as occurrences, none with the item's own ID.
    // The row takes the first of them rather than hiding the item until the
    // next sync.
    #[test]
    fn a_write_that_made_a_series_is_observed_as_its_first_occurrence() {
        let (_dir, path) = db();
        let mut one_off = super::super::sample_snapshot().events.remove(0);
        one_off.id = "series".into();
        let mut master = one_off.clone();
        master.details.recurrence = vec!["RRULE:FREQ=DAILY".into()];
        let mut writes = record_in(
            &path,
            edit_job(&master),
            Some(&master),
            &master.calendar_id,
            &master.id,
        )
        .unwrap();
        send_in(&path, writes.remove(0), |_| Ok(None)).unwrap();
        let occurrence = |id: &str, days: i64| {
            let mut event = one_off.clone();
            event.id = id.into();
            event.start += chrono::Duration::days(days);
            event.end += chrono::Duration::days(days);
            event.details.recurring_event_id = Some("series".into());
            event.details.original_start = Some(event.start);
            event
        };
        let first = occurrence("series_1", 0);
        let next = occurrence("series_2", 1);
        let day = master.start.date_naive();
        let fetched = crate::clock::now() + chrono::Duration::seconds(1);
        observe_range(
            &path,
            &[first.clone(), next],
            day,
            day + chrono::Duration::days(1),
            fetched,
            0,
        )
        .unwrap();
        // What was shown before, the one-off, gives way to the occurrence.
        let mut shown = vec![one_off];
        overlay(&path, &mut shown, None).unwrap();
        assert_eq!(settled(shown), [first]);
    }

    #[test]
    fn snapshots_google_has_since_confirmed_are_pruned_after_an_hour() {
        let (_dir, path) = db();
        let (event, write) = edit(&path, "confirmed");
        send_in(&path, write, |_| Ok(None)).unwrap();
        assert_eq!(entities(&path), 1);
        let day = event.start.date_naive();
        let fetched = crate::clock::now() + chrono::Duration::seconds(1);
        observe_range(&path, std::slice::from_ref(&event), day, day, fetched, 0).unwrap();
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
            0,
        )
        .unwrap();
        assert_eq!(entities(&path), 1);
        observe_range(
            &path,
            std::slice::from_ref(&event),
            day,
            day,
            fetched + chrono::Duration::hours(2),
            0,
        )
        .unwrap();
        assert_eq!(entities(&path), 0);
    }
    #[test]
    fn creation_requires_an_explicit_all_day_flag() {
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
    fn stale_google_data_does_not_hide_the_latest_local_edit() {
        let (_dir, path) = db();
        let (old, _) = edit(&path, "first");
        let (latest, _) = edit(&path, "second");
        let mut fetched = vec![old];
        overlay(&path, &mut fetched, Some(crate::clock::now())).unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].title, latest.title);
    }
    #[test]
    fn deleting_a_block_releases_children_atomically() {
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
            with_tasks: None,
        };
        database.connection().execute_batch("CREATE TRIGGER reject_child BEFORE INSERT ON google_entities WHEN instr(NEW.id,'task:child')>0 BEGIN SELECT RAISE(ABORT,'disk failure'); END;").unwrap();
        assert!(record_in(&path, delete.clone(), None, &block.calendar_id, &block.id).is_err());
        assert_eq!(entities(&path), 0);
        database
            .connection()
            .execute_batch("DROP TRIGGER reject_child")
            .unwrap();
        let writes = record_in(&path, delete, None, &block.calendar_id, &block.id).unwrap();
        assert_eq!(writes.len(), 2);
        assert!(writes[1].label.starts_with("Edit task Unassign"));
        let mut restored = vec![block.clone(), child];
        overlay(&path, &mut restored, None).unwrap();
        assert_eq!(restored.len(), 1);
        assert!(restored[0].all_day);
        assert!(restored[0].details.sending);
        assert!(crate::timeblocking::metadata(&restored[0]).block.is_none());
        assert_eq!(restored[0].start.date_naive(), block.start.date_naive());
    }

    #[test]
    fn deleting_a_block_with_its_tasks_deletes_the_one_offs_in_what_it_deletes() {
        let day = |d| NaiveDate::from_ymd_opt(2030, 11, d).unwrap();
        let stamp = |d| {
            at_clock(day(d), 9, 0)
                .with_timezone(&Utc)
                .format("%Y%m%dT%H%M%SZ")
                .to_string()
        };
        let template = super::super::sample_snapshot().events.remove(0);
        let mut block = template.clone();
        block.id = format!("blockseries_{}", stamp(5));
        block.calendar_id = "app".into();
        block.start = at_clock(day(5), 9, 0);
        block.end = block.start + chrono::Duration::hours(1);
        block.details.recurring_event_id = Some("blockseries".into());
        block.details.original_start = Some(block.start);
        // Deletes the occurrence on the 5th with `scope`, with one-offs
        // planned on the 4th, the 5th and the 6th, and returns the one-offs
        // deleted and those still shown.
        let run = |scope| {
            let (_dir, path) = db();
            let mut database = task_db::Database::open(&path).unwrap();
            let children: Vec<Event> = [4, 5, 6]
                .into_iter()
                .map(|d| {
                    let mut child = template.clone();
                    child.id = format!("task:child{d}");
                    child.calendar_id = "tasks:list".into();
                    let mut data = database.import(&child).unwrap();
                    data.metadata.block = Some(format!("app/blockseries_{}", stamp(d)));
                    child.details.task_data = Some(data);
                    child
                })
                .collect();
            cache_tasks_in(&path, &children, true).unwrap();
            let delete = Job::Delete {
                calendar: "app".into(),
                id: block.id.clone(),
                scope,
                notify: false,
                with_tasks: Some(Box::new(block.clone())),
            };
            let writes = record_in(&path, delete, None, "app", &block.id).unwrap();
            assert!(
                writes
                    .iter()
                    .all(|write| !write.label.starts_with("Edit task Unassign")),
                "nothing is taken out of the block instead"
            );
            let deleted: Vec<String> = writes
                .iter()
                .filter_map(|write| match &write.job {
                    Job::DeleteTask { task, .. } => Some(task.id.clone()),
                    _ => None,
                })
                .collect();
            let mut shown = children;
            overlay(&path, &mut shown, None).unwrap();
            let shown: Vec<String> = shown.into_iter().map(|e| e.id).collect();
            (deleted, shown)
        };
        let (deleted, shown) = run(SaveScope::CurrentAndFuture);
        assert_eq!(deleted, ["task:child5", "task:child6"]);
        assert_eq!(shown, ["task:child4"]);
        let (deleted, shown) = run(SaveScope::All);
        assert_eq!(deleted, ["task:child4", "task:child5", "task:child6"]);
        assert!(shown.is_empty());
        let (deleted, _) = run(SaveScope::Current);
        assert_eq!(deleted, ["task:child5"]);
    }

    #[test]
    fn deleting_a_retimed_occurrence_releases_the_one_offs_placed_before() {
        let (_dir, path) = db();
        let day = NaiveDate::from_ymd_opt(2030, 11, 4).unwrap();
        let stamp = |hour| {
            at_clock(day, hour, 0)
                .with_timezone(&Utc)
                .format("%Y%m%dT%H%M%SZ")
                .to_string()
        };
        let mut child = super::super::sample_snapshot().events.remove(0);
        child.id = "task:placed".into();
        child.calendar_id = "tasks:list".into();
        let mut database = task_db::Database::open(&path).unwrap();
        let mut data = database.import(&child).unwrap();
        // Placed when the series ran at 9; it runs at 10 now.
        data.metadata.block = Some(format!("app/blockseries_{}", stamp(9)));
        child.details.task_data = Some(data);
        cache_tasks_in(&path, &[child.clone()], true).unwrap();
        let retimed = format!("blockseries_{}", stamp(10));
        let delete = Job::Delete {
            calendar: "app".into(),
            id: retimed.clone(),
            scope: SaveScope::Current,
            notify: false,
            with_tasks: None,
        };
        let writes = record_in(&path, delete, None, "app", &retimed).unwrap();
        assert_eq!(writes.len(), 2, "the one-off is released");
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
        record_in(
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
        record_in(
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
    fn a_repeating_occurrence_leaving_its_block_becomes_a_task_of_its_own() {
        let settings = CalendarSettings {
            app_calendar: Some("app".into()),
            ..Default::default()
        };
        let start = at_clock(NaiveDate::from_ymd_opt(2030, 11, 4).unwrap(), 9, 0);
        let occurrence = |n: u32, block_series: Option<&str>| Event {
            id: format!("ttrepeat{n}_20301104T170000Z"),
            calendar_id: "app".into(),
            title: "Inbox zero".into(),
            start,
            end: start + chrono::Duration::minutes(30),
            all_day: false,
            color: colors::TASK,
            completed: false,
            details: Details {
                recurring_event_id: Some(format!("ttrepeat{n}")),
                task_data: Some(task_db::TaskData {
                    id: 424_200 + i64::from(n),
                    metadata: crate::timeblocking::Metadata {
                        block_series: block_series.map(str::to_owned),
                        block: Some("app/block_x".into()),
                        minutes: 30,
                        order: 7,
                        ..Default::default()
                    },
                }),
                ..Default::default()
            },
        };
        let path = task_db::path().unwrap();
        let gone = |event: &Event| {
            let mut fetched = vec![event.clone()];
            overlay(&path, &mut fetched, None).unwrap();
            !fetched.iter().any(|shown| shown.id == event.id)
        };

        // Out of its block: a task of its own that day.
        let first = occurrence(1, Some("app/block"));
        let alone = edit_task(first.clone(), &settings, task_edit::Change::Unassign, 0).unwrap();
        assert_ne!(alone.id, first.id);
        assert!(alone.details.recurring_event_id.is_none());
        assert!(alone.all_day && alone.start.date_naive() == start.date_naive());
        let meta = crate::timeblocking::metadata(&alone);
        assert!(meta.block.is_none() && meta.block_series.is_none());
        assert_eq!((meta.minutes, alone.title.as_str()), (30, "Inbox zero"));
        assert!(gone(&first), "the occurrence is deleted");

        // Into another block: a one-off there.
        let second = occurrence(2, Some("app/block"));
        let later = start + chrono::Duration::hours(3);
        let change = task_edit::Change::Block("app/other".into(), later, later);
        let placed = edit_task(second.clone(), &settings, change, 0).unwrap();
        assert!(placed.details.recurring_event_id.is_none());
        assert_eq!(
            crate::timeblocking::metadata(&placed).block.as_deref(),
            Some("app/other")
        );
        assert!(gone(&second));

        // A task with a repeat of its own goes into a block as a one-off.
        let own = occurrence(3, None);
        let change = task_edit::Change::Block("app/other".into(), later, later);
        let placed = edit_task(own.clone(), &settings, change, 0).unwrap();
        assert!(placed.details.recurring_event_id.is_none());
        assert!(gone(&own));

        // Laid out again inside its block, it keeps repeating with it.
        let fourth = occurrence(4, Some("app/block"));
        let change = task_edit::Change::Reflow(later, later + chrono::Duration::minutes(30));
        let kept = edit_task(fourth.clone(), &settings, change, 0).unwrap();
        assert_eq!(kept.id, fourth.id);
        assert!(crate::timeblocking::is_follower(&kept));

        // Something that repeats already cannot also repeat with a block.
        let join = task_edit::Change::JoinBlockSeries {
            calendar: "app".into(),
            series: "block".into(),
            from: start,
        };
        assert!(edit_task(occurrence(5, None), &settings, join, 0).is_err());
    }

    #[test]
    fn deletion_hides_the_item_while_it_is_sent() {
        let (_dir, path) = db();
        let (event, _) = edit(&path, "delete me");
        record_in(
            &path,
            Job::Delete {
                calendar: event.calendar_id.clone(),
                id: event.id.clone(),
                scope: SaveScope::Current,
                notify: false,
                with_tasks: None,
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
    fn task_id_changes_are_resolved_through_multiple_moves() {
        let (_dir, path) = db();
        let (local, write) = edit(&path, "first");
        let local_id = local.id.clone();
        send_in(&path, write, |_| {
            Ok(Some((
                local_id.clone(),
                "remote-one".into(),
                "first-list".into(),
            )))
        })
        .unwrap();
        let (_, write) = edit(&path, "second");
        send_in(&path, write, |job| {
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
        let (_, write) = edit(&path, "third");
        send_in(&path, write, |job| {
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
    fn confirmed_remote_changes_update_the_disk_projection_and_reject_older_fetches() {
        let (_dir, path) = db();
        let (mut remote, write) = edit(&path, "local edit");
        send_in(&path, write, |_| Ok(None)).unwrap();
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
            0,
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
        let (latest, _) = edit(&path, "immediate edit");
        store.apply_local_operations().unwrap();
        let shown = store.event(&latest.id).unwrap();
        assert_eq!(shown.title, "immediate edit");
        assert!(shown.details.sending);
    }
}
