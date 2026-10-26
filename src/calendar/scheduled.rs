//! Calendar-backed task times. Google Tasks owns the text and completion;
//! Calendar owns start/end. IDs, rather than task titles, connect the two.
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Event, api};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub calendar_id: String,
    pub event_id: String,
    pub list_id: String,
    pub task_id: String,
    #[serde(default)]
    pub pending: bool,
    #[serde(default)]
    pub block: Option<String>,
    #[serde(default)]
    pub order: i64,
}

impl Link {
    pub fn new(calendar_id: String, task: &Event) -> Result<Self, String> {
        let mut entropy = [0u8; 16];
        getrandom::fill(&mut entropy)
            .map_err(|e| format!("Could not allocate a calendar event ID: {e}"))?;
        Ok(Self {
            calendar_id,
            // Hex is a subset of Calendar's permitted base32hex event ID alphabet.
            // Reserve once in SQLite, reuse for retries. A new scheduling
            // after an explicit deletion needs a fresh ID (Calendar tombstones IDs).
            event_id: format!("tt{:032x}", u128::from_be_bytes(entropy)),
            list_id: task.calendar_id.trim_start_matches("tasks:").to_owned(),
            task_id: task.id.trim_start_matches("task:").to_owned(),
            pending: true,
            block: None,
            order: 0,
        })
    }

    pub fn event_url(&self) -> String {
        format!(
            "{}/{}/events/{}",
            api::EVENTS,
            urlencoding::encode(&self.calendar_id),
            urlencoding::encode(&self.event_id)
        )
    }

    pub fn task_url(&self) -> String {
        format!(
            "{}/{}/tasks/{}",
            api::TASKS,
            urlencoding::encode(&self.list_id),
            urlencoding::encode(&self.task_id)
        )
    }

    pub fn from_properties(calendar_id: &str, event_id: &str, properties: &Value) -> Option<Self> {
        let p = &properties["private"];
        if p["ttKind"].as_str()? != "task" {
            return None;
        }
        Some(Self {
            calendar_id: calendar_id.to_owned(),
            event_id: event_id.to_owned(),
            list_id: p["ttTaskList"].as_str()?.to_owned(),
            task_id: p["ttTask"].as_str()?.to_owned(),
            pending: false,
            block: p["ttBlock"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            order: p["ttOrder"]
                .as_str()
                .and_then(|s| s.parse().ok())
                .unwrap_or_default(),
        })
    }
}

pub fn description(task: &Event) -> String {
    let notes = crate::timeblocking::description(&task.details.description);
    if task.details.link.is_empty() {
        return notes;
    }
    format!(
        "{notes}{}Google Task: {}",
        if notes.is_empty() { "" } else { "\n\n" },
        task.details.link
    )
}

pub fn content(task: &Event, link: &Link) -> Value {
    json!({
        "summary": if task.completed { format!("✓ {}", task.title) } else { task.title.clone() },
        "description": description(task),
        "extendedProperties": { "private": {
            "ttKind": "task", "ttTaskList": link.list_id, "ttTask": link.task_id,
            "ttBlock": link.block.as_deref().unwrap_or(""), "ttOrder": link.order.to_string(),
            "ttCompleted": task.completed.to_string(),
        }},
    })
}

pub fn event_body(
    task: &Event,
    link: &Link,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Value {
    let mut body = content(task, link);
    body["start"] = json!({"dateTime": start.with_timezone(&Utc).to_rfc3339()});
    body["end"] = json!({"dateTime": end.with_timezone(&Utc).to_rfc3339()});
    body
}

/// Replace each linked calendar copy with one task-backed block. Calendar owns
/// its time, Tasks owns its content; completed blocks stay in the result.
pub fn merge(
    events: Vec<Event>,
    tasks: Vec<Event>,
    loaded_calendars: &std::collections::HashSet<String>,
) -> (Vec<Event>, Vec<(Link, Value)>) {
    let mut tasks: std::collections::HashMap<_, _> = tasks
        .into_iter()
        .map(|task| ((task.calendar_id.clone(), task.id.clone()), task))
        .collect();
    let mut result = Vec::new();
    let mut changes = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for event in events {
        let Some(link) = event.details.scheduled.as_ref() else {
            result.push(event);
            continue;
        };
        let key = (
            format!("tasks:{}", link.list_id),
            format!("task:{}", link.task_id),
        );
        if !seen.insert(key.clone()) {
            continue;
        }
        let Some(mut task) = tasks.remove(&key) else {
            // Deleted tasks and temporarily unavailable Tasks still leave a
            // calendar record. Do not delete history or infer completion.
            result.push(event);
            continue;
        };
        task.start = event.start;
        task.end = event.end;
        task.all_day = false;
        task.color = event.color;
        task.details.scheduled = Some(link.clone());
        if let Some(data) = &mut task.details.task_data {
            data.metadata.block = link.block.clone();
            data.metadata.order = link.order;
            data.metadata.schedule = Some(link.clone());
        }
        let body = content(&task, link);
        if body["summary"].as_str() != Some(event.title.as_str())
            || body["description"].as_str() != Some(event.details.description.as_str())
            || task.completed != event.completed
        {
            changes.push((link.clone(), body));
        }
        result.push(task);
    }
    result.extend(
        tasks
            .into_values()
            .map(|mut task| {
                // A complete query for the task's calendar proves the event was
                // removed externally. A failed query or an unfinished write does not.
                if let Some(data) = &mut task.details.task_data {
                    if data.metadata.schedule.as_ref().is_some_and(|link| {
                        !link.pending && loaded_calendars.contains(&link.calendar_id)
                    }) {
                        data.metadata.schedule = None;
                        data.metadata.block = None;
                        task.details.scheduled = None;
                    }
                }
                task
            })
            .filter(|task| !task.completed || crate::timeblocking::metadata(task).block.is_some()),
    );
    (result, changes)
}

/// Status remains available to distinguish an absent event from permission or
/// network failures. In particular, a retry must never create a duplicate.
#[derive(Debug)]
pub struct Error {
    pub status: Option<u16>,
    pub message: String,
}

pub fn request(method: &str, url: &str, token: &str, body: Option<&Value>) -> Result<Value, Error> {
    let request = api::agent()
        .request(method, url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json");
    let result = match body {
        Some(body) => request.send_json(body),
        None => request.call(),
    };
    match result {
        Ok(response) if response.status() == 204 => Ok(Value::Null),
        Ok(response) => response.into_json().map_err(|e| Error {
            status: None,
            message: format!("Google response: {e}"),
        }),
        Err(ureq::Error::Status(status, response)) => {
            let detail = response
                .into_json::<Value>()
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("HTTP {status}"));
            let message = if status == 403 && detail.to_lowercase().contains("scope") {
                "Reconnect Google in Settings to allow task scheduling and calendar creation"
                    .to_owned()
            } else {
                format!("Google: {detail}")
            };
            Err(Error {
                status: Some(status),
                message,
            })
        }
        Err(error) => Err(Error {
            status: None,
            message: format!("Google: {error}"),
        }),
    }
}

/// PATCH first, then insert with the reserved ID. A lost insert response or a
/// competing retry results in 409 and another PATCH, not another event.
pub fn upsert(
    token: &str,
    task: &Event,
    link: &Link,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<(), String> {
    let body = event_body(task, link, start, end);
    let url = link.event_url();
    upsert_at(
        token,
        &url,
        &format!(
            "{}/{}/events",
            api::EVENTS,
            urlencoding::encode(&link.calendar_id)
        ),
        &link.event_id,
        body,
    )
}

fn upsert_at(
    token: &str,
    url: &str,
    collection: &str,
    id: &str,
    body: Value,
) -> Result<(), String> {
    upsert_with(id, body, |method, body| {
        request(
            method,
            if method == "POST" { collection } else { url },
            token,
            Some(body),
        )
    })
}

fn upsert_with(
    id: &str,
    mut body: Value,
    mut request: impl FnMut(&str, &Value) -> Result<Value, Error>,
) -> Result<(), String> {
    match request("PATCH", &body) {
        Ok(_) => Ok(()),
        Err(error) if error.status == Some(404) => {
            body["id"] = json!(id);
            match request("POST", &body) {
                Ok(_) => Ok(()),
                Err(error) if error.status == Some(409) => {
                    body.as_object_mut().unwrap().remove("id");
                    request("PATCH", &body).map(|_| ()).map_err(|e| e.message)
                }
                Err(error) => Err(error.message),
            }
        }
        Err(error) => Err(error.message),
    }
}

pub fn remove(token: &str, link: &Link) -> Result<(), String> {
    match request("DELETE", &link.event_url(), token, None) {
        Ok(_) => Ok(()),
        Err(error) if matches!(error.status, Some(404 | 410)) => Ok(()),
        Err(error) => Err(error.message),
    }
}

/// The cross-service scheduling transaction is tested against this boundary,
/// including failures after Google has changed a task's identity.
pub trait Remote {
    fn destinations(
        &self,
        calendar: Option<&str>,
        list: Option<&str>,
    ) -> Result<(String, String), String>;
    fn move_task(&self, link: &Link, destination: &str) -> Result<Value, String>;
    fn set_day(&self, link: &Link, day: chrono::NaiveDate) -> Result<(), String>;
    fn save_event(
        &self,
        task: &Event,
        link: &Link,
        start: DateTime<Local>,
        end: DateTime<Local>,
    ) -> Result<(), String>;
    fn remove_event(&self, link: &Link) -> Result<(), String>;
}

pub struct Google<'a> {
    pub token: &'a str,
}
impl Remote for Google<'_> {
    fn destinations(
        &self,
        calendar: Option<&str>,
        list: Option<&str>,
    ) -> Result<(String, String), String> {
        destinations(self.token, calendar, list)
    }
    fn move_task(&self, link: &Link, destination: &str) -> Result<Value, String> {
        request(
            "POST",
            &format!(
                "{}/move?destinationTasklist={}",
                link.task_url(),
                urlencoding::encode(destination)
            ),
            self.token,
            None,
        )
        .map_err(|e| e.message)
    }
    fn set_day(&self, link: &Link, day: chrono::NaiveDate) -> Result<(), String> {
        api::patch(
            &link.task_url(),
            self.token,
            &json!({"due": format!("{day}T00:00:00.000Z")}).to_string(),
            "Schedule Google Task",
        )
    }
    fn save_event(
        &self,
        task: &Event,
        link: &Link,
        start: DateTime<Local>,
        end: DateTime<Local>,
    ) -> Result<(), String> {
        upsert(self.token, task, link, start, end)
    }
    fn remove_event(&self, link: &Link) -> Result<(), String> {
        remove(self.token, link)
    }
}

/// Explicitly configured destinations win. Otherwise reuse the named default
/// or create it at the first scheduling operation, never during a read-only sync.
pub fn destinations(
    token: &str,
    calendar: Option<&str>,
    list: Option<&str>,
) -> Result<(String, String), String> {
    let calendar = match calendar {
        Some(id) => id.to_owned(),
        None => find_or_create(
            token,
            "https://www.googleapis.com/calendar/v3/users/me/calendarList",
            api::EVENTS,
            "summary",
            "Tasks",
        )?,
    };
    let list = match list {
        Some(id) => id.trim_start_matches("tasks:").to_owned(),
        None => find_or_create(token, api::TASK_LISTS, api::TASK_LISTS, "title", "Has Time")?,
    };
    Ok((calendar, list))
}

fn find_or_create(
    token: &str,
    collection: &str,
    create_url: &str,
    name_field: &str,
    name: &str,
) -> Result<String, String> {
    let mut page = None;
    loop {
        let url = format!(
            "{collection}?maxResults=100{}",
            page.as_ref()
                .map(|p: &String| format!("&pageToken={}", urlencoding::encode(p)))
                .unwrap_or_default()
        );
        let response = request("GET", &url, token, None).map_err(|e| e.message)?;
        if let Some(id) = response["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|item| {
                item[name_field].as_str() == Some(name)
                    && (name_field != "summary"
                        || matches!(item["accessRole"].as_str(), Some("owner" | "writer")))
            })
            .and_then(|item| item["id"].as_str())
        {
            return Ok(id.to_owned());
        }
        page = response["nextPageToken"].as_str().map(str::to_owned);
        if page.is_none() {
            break;
        }
    }
    let mut body = json!({});
    body[name_field] = json!(name);
    request("POST", create_url, token, Some(&body)).map_err(|e| e.message)?["id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "Google did not return an ID for the scheduling destination".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::timeblocking::{Metadata, task_key};

    fn make_task(id: &str, completed: bool) -> Event {
        let mut task = super::super::sample_snapshot().events.remove(1);
        task.id = format!("task:{id}");
        task.calendar_id = "tasks:has-time".into();
        task.title = "Same title".into();
        task.completed = completed;
        task.all_day = true;
        task.details.description = "DUE: 2026-10-30\n\nHuman notes\n".into();
        task.details.link = format!("https://tasks.google.com/task/{id}");
        task.details.task_data = Some(crate::task_db::TaskData {
            id: if id == "one" { 100 } else { 101 },
            metadata: Metadata::default(),
            legacy_key: None,
        });
        task
    }

    fn mirror(task: &Event) -> Event {
        let mut link = Link::new("task-calendar".into(), task).unwrap();
        link.pending = false;
        link.block = Some("work/block".into());
        link.order = 123;
        let mut event = task.clone();
        event.id = link.event_id.clone();
        event.calendar_id = link.calendar_id.clone();
        event.all_day = false;
        event.details.task_data = None;
        event.details.description = description(task);
        event.details.scheduled = Some(link);
        if task.completed {
            event.title = format!("✓ {}", task.title);
        }
        event
    }

    #[test]
    fn calendar_times_and_task_deadlines_merge_by_identity_and_keep_work_history() {
        let task = make_task("one", false);
        let other = make_task("two", false);
        let mut event = mirror(&task);
        event.color = iced::Color::from_rgb8(17, 145, 72);
        event.start += chrono::Duration::hours(4);
        event.end += chrono::Duration::hours(4);
        let start = event.start;
        let (events, changes) = merge(vec![event], vec![task.clone(), other], &Default::default());
        assert!(changes.is_empty());
        assert_eq!(events.len(), 2);
        let merged = events.iter().find(|e| e.id == "task:one").unwrap();
        assert!(!merged.all_day);
        assert_eq!(merged.start, start);
        assert_eq!(merged.color, iced::Color::from_rgb8(17, 145, 72));
        assert_eq!(merged.details.description, task.details.description);
        assert_eq!(task_key(merged), "100");
        assert_eq!(
            crate::timeblocking::metadata(merged).block.as_deref(),
            Some("work/block")
        );
        assert!(events.iter().find(|e| e.id == "task:two").unwrap().all_day);
    }

    #[test]
    fn completed_tasks_keep_their_calendar_blocks_and_mirror_content_without_moving_time() {
        let mut task = make_task("one", false);
        let event = mirror(&task);
        let start = event.start;
        task.completed = true;
        task.details.description = "DUE: 2026-11-05\nUpdated from another device".into();
        let (events, changes) = merge(
            vec![event],
            vec![task.clone(), make_task("two", true)],
            &Default::default(),
        );
        assert_eq!(events.len(), 1);
        assert!(events[0].completed);
        assert_eq!(events[0].start, start);
        assert_eq!(changes.len(), 1);
        let body = &changes[0].1;
        assert_eq!(body["summary"], "✓ Same title");
        assert!(
            body["description"]
                .as_str()
                .unwrap()
                .starts_with("DUE: 2026-11-05\n")
        );
        assert_eq!(body["extendedProperties"]["private"]["ttCompleted"], "true");
        assert!(body.get("start").is_none());
        assert!(body.get("end").is_none());
    }

    #[test]
    fn missing_calendar_data_does_not_unschedule_tasks_but_confirmed_deletion_does() {
        let mut task = make_task("one", false);
        let link = mirror(&task).details.scheduled.unwrap();
        task.details.task_data.as_mut().unwrap().metadata.schedule = Some(link.clone());
        task.details.task_data.as_mut().unwrap().metadata.block = link.block.clone();
        let (events, _) = merge(vec![], vec![task.clone()], &Default::default());
        assert!(crate::timeblocking::metadata(&events[0]).schedule.is_some());
        let loaded = std::collections::HashSet::from([link.calendar_id.clone()]);
        let (events, _) = merge(vec![], vec![task.clone()], &loaded);
        assert!(crate::timeblocking::metadata(&events[0]).schedule.is_none());
        assert!(crate::timeblocking::metadata(&events[0]).block.is_none());
        task.details
            .task_data
            .as_mut()
            .unwrap()
            .metadata
            .schedule
            .as_mut()
            .unwrap()
            .pending = true;
        let (events, _) = merge(vec![], vec![task], &loaded);
        assert!(crate::timeblocking::metadata(&events[0]).schedule.is_some());
    }

    #[test]
    fn properties_round_trip_task_identity_even_after_titles_change() {
        let task = make_task("one", false);
        let mut link = Link::new("custom-calendar".into(), &task).unwrap();
        link.pending = false;
        link.order = i64::MAX;
        link.block = Some("work/time-block".into());
        let body = event_body(&task, &link, task.start, task.end);
        assert_eq!(
            Link::from_properties(
                &link.calendar_id,
                &link.event_id,
                &body["extendedProperties"]
            ),
            Some(link)
        );
        assert!(
            body["description"]
                .as_str()
                .unwrap()
                .contains(&task.details.link)
        );
    }

    #[test]
    fn an_insert_conflict_reuses_the_reserved_id_instead_of_duplicating_the_event() {
        let mut methods = Vec::new();
        upsert_with("reserved-id", json!({"summary":"Task"}), |method, body| {
            methods.push(method.to_owned());
            match methods.len() {
                1 => Err(Error {
                    status: Some(404),
                    message: "missing".into(),
                }),
                2 => {
                    assert_eq!(body["id"], "reserved-id");
                    Err(Error {
                        status: Some(409),
                        message: "already inserted".into(),
                    })
                }
                _ => {
                    assert!(body.get("id").is_none());
                    Ok(json!({}))
                }
            }
        })
        .unwrap();
        assert_eq!(methods, ["PATCH", "POST", "PATCH"]);
    }

    #[test]
    fn permission_and_network_errors_never_trigger_an_insert() {
        for status in [Some(403), None] {
            let mut count = 0;
            let error = upsert_with("id", json!({}), |method, _| {
                count += 1;
                assert_eq!(method, "PATCH");
                Err(Error {
                    status,
                    message: "refused".into(),
                })
            })
            .unwrap_err();
            assert_eq!(error, "refused");
            assert_eq!(count, 1);
        }
    }
    #[test]
    fn scheduled_blocks_are_filtered_by_day_and_by_their_actual_calendar() {
        let mut snapshot = super::super::sample_snapshot();
        let mut task = make_task("one", false);
        task.all_day = false;
        task.details.scheduled = mirror(&task).details.scheduled;
        let mut other_day = task.clone();
        other_day.id = "task:other-day".into();
        other_day.start += chrono::Duration::days(1);
        other_day.end += chrono::Duration::days(1);
        snapshot.events = vec![task, other_day];
        let mut store = super::super::Store::with_snapshot(snapshot);
        let mut settings = crate::config::CalendarSettings::default();
        settings.show_tasks = false;
        store.apply_task_colors(&settings);
        assert_eq!(store.timed_events(&settings).count(), 1);
        settings.disabled.push("task-calendar".into());
        assert_eq!(store.timed_events(&settings).count(), 0);
    }
}
