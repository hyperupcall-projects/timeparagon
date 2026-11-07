//! Durable local edits. Acknowledging one operation never discards a newer edit.
use super::*;
use crate::{config::CalendarSettings, task_db, task_edit};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::{cell::Cell, sync::OnceLock, time::Duration};

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
        original: Event,
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
        #[serde(default)]
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
        task: Event,
        notes: String,
        due: Option<NaiveDate>,
        metadata: Option<crate::timeblocking::Metadata>,
    },
}
#[derive(Debug, Clone)]
pub struct Operation {
    pub id: i64,
    pub kind: String,
    pub entity: String,
    pub created_at: String,
    pub attempts: i64,
    pub error: Option<String>,
    pub payload: String,
}
pub fn list() -> Result<Vec<Operation>, String> {
    let mut db = task_db::open()?;
    let mut q=db.connection().prepare("SELECT id,kind,entity,created_at,attempts,error,payload FROM google_operations ORDER BY id").map_err(|e|e.to_string())?;
    q.query_map([], |r| {
        Ok(Operation {
            id: r.get(0)?,
            kind: r.get(1)?,
            entity: r.get(2)?,
            created_at: r.get(3)?,
            attempts: r.get(4)?,
            error: r.get(5)?,
            payload: r.get(6)?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<_, _>>()
    .map_err(|e| e.to_string())
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
    let mut db = task_db::Database::open(path)?;
    let tx = db
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if let Some(existing) = tx
        .query_row(
            "SELECT id FROM google_entities WHERE json_extract(event,'$.id')=?1 LIMIT 1",
            [id],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())?
    {
        entity = existing;
    }
    (|| -> rusqlite::Result<()> {
        tx.execute("INSERT INTO google_entities(id,event,acknowledged_at) VALUES(?1,?2,NULL) ON CONFLICT(id) DO UPDATE SET event=excluded.event,acknowledged_at=NULL",params![entity,saved])?;
        if event.is_none() {tx.execute("DELETE FROM google_task_records WHERE id=?1",[id])?;}
        if let Some(event)=event {if event.is_task() {tx.execute("INSERT INTO google_task_records(id,event) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET event=excluded.event",params![event.id,serde_json::to_string(event).unwrap()])?;} if let Some(data)=&event.details.task_data {
            tx.execute("INSERT INTO tasks(id,metadata,legacy_key) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET metadata=excluded.metadata", params![data.id,serde_json::to_string(&data.metadata).unwrap(),data.legacy_key])?;
            tx.execute("INSERT INTO task_aliases(source,task_id) VALUES(?1,?2) ON CONFLICT(source) DO UPDATE SET task_id=excluded.task_id",params![crate::timeblocking::block_key(event),data.id])?;
        }}
        tx.execute("DELETE FROM google_operations WHERE entity=?1 AND kind=?2 AND error IS NOT NULL",params![entity,kind])?;
        tx.execute("INSERT INTO google_operations(kind,payload,entity,created_at) VALUES(?1,?2,?3,?4)",params![kind,payload,entity,crate::clock::now().to_rfc3339()])?;
        tx.commit()
    })().map_err(|e|e.to_string())?;
    Ok(())
}
/// Apply the database's desired state after every cache read and Google fetch.
/// Confirmed writes remain protected against fetches that began before the ack.
pub fn overlay(
    database: &std::path::Path,
    events: &mut Vec<Event>,
    fetched_at: Option<DateTime<Local>>,
) -> Result<(), String> {
    let mut db = task_db::Database::open(database)?;
    let aliases = {
        let mut q = db
            .connection()
            .prepare("SELECT local,remote FROM google_id_map")
            .map_err(|e| e.to_string())?;
        let rows = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };
    for event in events.iter_mut() {
        for _ in 0..aliases.len() {
            for (local, remote) in &aliases {
                if event.id == *remote {
                    event.id = local.clone();
                } else if event.id == format!("task:{remote}") {
                    event.id = format!("task:{local}");
                }
            }
        }
    }
    let mut q = db
        .connection()
        .prepare("SELECT id,event,acknowledged_at FROM google_entities")
        .map_err(|e| e.to_string())?;
    let rows = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (id, json, ack) = row.map_err(|e| e.to_string())?;
        if let (Some(fetched), Some(ack)) = (fetched_at, ack.as_deref()) {
            if DateTime::parse_from_rfc3339(ack).is_ok_and(|a| fetched > a) {
                continue;
            }
        }
        let Some((calendar, id)) = id.split_once('\n') else {
            continue;
        };
        events.retain(|e| e.id != id);
        let _ = calendar;
        if let Some(json) = json {
            events.push(serde_json::from_str(&json).map_err(|e| e.to_string())?);
        }
    }
    Ok(())
}
pub fn cached_event(calendar: &str, id: &str) -> Result<Option<Event>, String> {
    let mut db = task_db::open()?;
    let json: Option<String> = db
        .connection()
        .query_row(
            "SELECT event FROM google_entities WHERE json_extract(event,'$.id')=?1 AND (acknowledged_at IS NULL OR acknowledged_at > COALESCE((SELECT json_extract(value,'$.fetched_at') FROM documents WHERE key='calendar_sync_state'),''))",
            [id],
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
    drain_in(&path, execute)
}
fn drain_in(
    path: &std::path::Path,
    run: impl FnOnce(Job) -> Result<Option<(String, String, String)>, String>,
) -> Result<(), String> {
    let mut db = task_db::Database::open(path)?;
    let tx = db
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let item: Option<(i64, String, String, i64)> = tx
        .query_row(
            "SELECT id,payload,entity,lease_until FROM google_operations ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((id, payload, entity, lease)) = item else {
        return Ok(());
    };
    let now = crate::clock::utc_now().timestamp();
    if lease > now {
        return Ok(());
    }
    tx.execute(
        "UPDATE google_operations SET lease_until=?2,attempts=attempts+1 WHERE id=?1",
        params![id, now + 600],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    let mut value: serde_json::Value = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
    let mut q = db
        .connection()
        .prepare("SELECT local,remote,calendar FROM google_id_map")
        .map_err(|e| e.to_string())?;
    let aliases = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(q);
    fn remap(v: &mut serde_json::Value, a: &[(String, String, String)]) {
        if let Some(obj) = v.as_object_mut() {
            for _ in 0..a.len() {
                for (id_key, calendar_key, prefix) in
                    [("id", "calendar_id", "task:"), ("task_id", "list_id", "")]
                {
                    if let Some(id) = obj.get(id_key).and_then(|v| v.as_str()).map(str::to_owned) {
                        if let Some((_, remote, calendar)) = a.iter().find(|(local, remote, _)| {
                            local != remote && (id == *local || id == format!("task:{local}"))
                        }) {
                            let full = id.starts_with("task:");
                            obj.insert(
                                id_key.into(),
                                if full {
                                    format!("task:{remote}")
                                } else {
                                    remote.clone()
                                }
                                .into(),
                            );
                            let calendar = if prefix.is_empty() {
                                calendar.trim_start_matches("tasks:").to_owned()
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
    remap(&mut value, &aliases);
    let job: Job = serde_json::from_value(value).map_err(|e| e.to_string())?;
    let id_for_replay = id;
    REPLAY.with(|r| r.set(true));
    OPERATION_ID.with(|id| id.set(Some(id_for_replay)));
    let result = run(job);
    OPERATION_ID.with(|id| id.set(None));
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
            tx.execute("UPDATE google_entities SET acknowledged_at=?2 WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM google_operations WHERE entity=?1)",params![entity,crate::clock::now().to_rfc3339()]).map_err(|e|e.to_string())?;
        }
        Err(e) => {
            tx.execute("DELETE FROM google_operations WHERE id=?1 AND EXISTS(SELECT 1 FROM google_operations newer WHERE newer.entity=?2 AND newer.kind=(SELECT kind FROM google_operations WHERE id=?1) AND newer.id>?1)",params![id,entity]).map_err(|e|e.to_string())?;
            tx.execute(
                "UPDATE google_operations SET error=?2,lease_until=?3 WHERE id=?1",
                params![id, e, now + 15],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}
fn execute(job: Job) -> Result<Option<(String, String, String)>, String> {
    let s = crate::config::Config::load().calendar;
    let c = &s.client_id;
    let secret = &s.client_secret;
    match job {
        Job::Edit {
            original,
            draft,
            scope,
            description,
            color,
        } => {
            super::save_event_scope(
                c,
                secret,
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
        } => super::set_event_time(c, secret, &calendar, &id, start, end, notify)?,
        Job::Location {
            calendar,
            id,
            location,
        } => super::set_event_location(c, secret, &calendar, &id, &location)?,
        Job::Status { task, status } => {
            super::set_linked_task_status(c, secret, &task, status)?;
        }
        Job::TaskEdit {
            task,
            change,
            order,
        } => {
            let old = task.id.trim_start_matches("task:").to_owned();
            let saved = task_edit::save(task, &s, change, order, task_edit::google_access)?;
            if saved.id.trim_start_matches("task:") != old {
                return Ok(Some((
                    old,
                    saved.id.trim_start_matches("task:").into(),
                    saved.calendar_id,
                )));
            }
        }
        Job::Delete {
            calendar,
            id,
            scope,
            notify,
        } => super::delete_event_scope(c, secret, &calendar, &id, scope, notify)?,
        Job::DeleteTask { mut task, calendar } => {
            let mut db = task_db::open()?;
            let data = db.import(&task)?;
            task.details.scheduled = data.metadata.schedule;
            super::delete_task(c, secret, &task, calendar.as_deref())?
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
            let result = super::create_event(
                c,
                secret,
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
            let result = super::create_habit(c, secret, &draft, color.as_deref(), &habit);
            INSERT_ID.with(|v| *v.borrow_mut() = None);
            let saved = result?;
            return Ok(Some((id, saved.id, saved.calendar_id)));
        }
        Job::Sleep { event, new } => {
            super::sleep::save(&s, event, new)?;
        }
        Job::HabitCompleted { event } => super::set_habit_completed(c, secret, &event)?,
        Job::Reply {
            event,
            email,
            response,
        } => super::reply_to_event(c, secret, &event.calendar_id, &event.id, &email, response)?,
        Job::CreateTask {
            task,
            notes,
            due,
            metadata,
        } => {
            let token = task_edit::google_access(&s)?;
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
                    task.id.trim_start_matches("task:").into(),
                    existing.id,
                    format!("tasks:{list}"),
                )));
            }
            let notes = task_metadata::write(&notes, data)?;
            let saved =
                task_edit::create_inbox_task(&s, &token, &task.title, &notes, due, metadata)?;
            return Ok(Some((
                task.id.trim_start_matches("task:").into(),
                saved.id.trim_start_matches("task:").into(),
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
    let original = task.clone();
    let mut db = task_db::open()?;
    let mut data = task
        .details
        .task_data
        .clone()
        .map(Ok)
        .unwrap_or_else(|| db.import(&task))?;
    use task_edit::Change;
    match &change {
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
        Change::Time(start, end) | Change::Block(_, start, end) => {
            task.start = *start;
            task.end = *end;
            task.all_day = false;
            if let Some(list) = settings.time_block_list.as_deref() {
                task.calendar_id = format!("tasks:{}", list.trim_start_matches("tasks:"));
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
            } else {
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
            task.details.scheduled = None;
            let list = match &change {
                Change::Inbox => settings.inbox_list.as_deref(),
                Change::Triaged => settings.triaged_list.as_deref(),
                Change::Backlog(_) => settings.backlog_list.as_deref(),
                _ => None,
            };
            if let Some(list) = list {
                task.calendar_id = format!("tasks:{}", list.trim_start_matches("tasks:"));
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
        calendar_id: format!("tasks:{list}"),
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
            task: task.clone(),
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
    let mut db = task_db::open()?;
    let tx = db
        .connection()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    if replace {
        tx.execute("DELETE FROM google_task_records", [])
            .map_err(|e| e.to_string())?;
    }
    for event in events.iter().filter(|e| e.is_task()) {
        tx.execute("INSERT INTO google_task_records(id,event) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET event=excluded.event",params![event.id,serde_json::to_string(event).map_err(|e|e.to_string())?]).map_err(|e|e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
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
                original: event.clone(),
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
    #[test]
    fn queued_creation_keeps_all_day_and_loads_older_timed_jobs() {
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
        let legacy: Job = serde_json::from_value(value).unwrap();
        assert!(matches!(legacy, Job::Create { all_day: false, .. }));
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
        let Some((_, id)) = key.split_once('\n') else {
            continue;
        };
        let current = events.iter().find(|e| e.id == id);
        let old = json
            .as_ref()
            .map(|j| serde_json::from_str::<Event>(j))
            .transpose()
            .map_err(|e| e.to_string())?;
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
            tx.execute("UPDATE google_entities SET event=?2,acknowledged_at=?3 WHERE id=?1 AND acknowledged_at=?4",params![key,json,fetched.to_rfc3339(),ack]).map_err(|e|e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}
