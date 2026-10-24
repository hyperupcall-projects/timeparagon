//! Google Calendar HTTP calls.
//!
//! All of this is blocking on purpose: it runs on a worker thread via
//! [`crate::blocking::run`] so a slow API never stalls a frame.

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use iced::Color;
use serde::Deserialize;

use super::colors;
use super::{Calendar, Event, Snapshot};
use crate::config::CalendarSettings;
use crate::hex;

const CALENDAR_LIST: &str = "https://www.googleapis.com/calendar/v3/users/me/calendarList";
const EVENTS: &str = "https://www.googleapis.com/calendar/v3/calendars";

#[derive(Debug, Deserialize)]
struct CalendarList {
    #[serde(default)]
    items: Vec<CalendarListEntry>,
}

#[derive(Debug, Deserialize)]
struct CalendarListEntry {
    id: String,
    #[serde(default)]
    summary: String,
    #[serde(default, rename = "backgroundColor")]
    background_color: Option<String>,
    #[serde(default, rename = "colorId")]
    color_id: Option<String>,
    #[serde(default)]
    selected: Option<bool>,
    #[serde(default)]
    primary: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct EventList {
    #[serde(default)]
    items: Vec<EventEntry>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct EventEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    location: String,
    #[serde(default, rename = "htmlLink")]
    html_link: String,
    #[serde(default, rename = "hangoutLink")]
    hangout_link: String,
    #[serde(default)]
    start: Option<EventTime>,
    #[serde(default)]
    end: Option<EventTime>,
    #[serde(default, rename = "colorId")]
    color_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    organizer: Option<Person>,
    #[serde(default)]
    attendees: Vec<Person>,
    #[serde(default, rename = "conferenceData")]
    conference: Option<Conference>,
}

#[derive(Debug, Default, Deserialize)]
struct Person {
    #[serde(default)]
    email: String,
    #[serde(default, rename = "displayName")]
    display_name: String,
    #[serde(default, rename = "responseStatus")]
    response_status: String,
}

#[derive(Debug, Default, Deserialize)]
struct Conference {
    #[serde(default, rename = "entryPoints")]
    entry_points: Vec<EntryPoint>,
}

#[derive(Debug, Default, Deserialize)]
struct EntryPoint {
    #[serde(default, rename = "entryPointType")]
    kind: String,
    #[serde(default)]
    uri: String,
}

#[derive(Debug, Deserialize)]
struct EventTime {
    #[serde(default, rename = "dateTime")]
    date_time: Option<String>,
    #[serde(default)]
    date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    #[serde(default)]
    error: ApiError,
}

#[derive(Debug, Default, Deserialize)]
struct ApiError {
    #[serde(default)]
    message: String,
}

/// Pulls every calendar and the events of `day` into a [`Snapshot`].
///
/// The window is anchored to the boundary rather than to midnight, so a day
/// that ends at 3am still includes the small hours of the next morning.
pub fn fetch_day(
    access_token: &str,
    disabled: &[String],
    day: NaiveDate,
    day_end_hour: u32,
) -> Result<Snapshot, String> {
    let mut calendars = fetch_calendars(access_token)?;
    let mut events = Vec::new();

    // Every calendar is fetched; the hamburger menu hides by ID in the
    // view so toggling one back on does not need another round trip.
    let _ = disabled;
    for calendar in &calendars {
        events.extend(fetch_events(access_token, calendar, day, day_end_hour)?);
    }

    let (task_calendars, task_events, notice) = match fetch_tasks(access_token, day) {
        Ok(tasks) => tasks,
        Err(error) => (Vec::new(), Vec::new(), Some(tasks_notice(&error))),
    };
    calendars.extend(task_calendars);
    events.extend(task_events);
    events.sort_by(|left, right| left.start.cmp(&right.start).then(right.end.cmp(&left.end)));

    Ok(Snapshot {
        calendars,
        events,
        day,
        fetched_at: Local::now(),
        notice,
    })
}

fn fetch_calendars(access_token: &str) -> Result<Vec<Calendar>, String> {
    let list: CalendarList = get(CALENDAR_LIST, access_token)?;

    let mut calendars: Vec<Calendar> = list
        .items
        .into_iter()
        .map(|entry| {
            let color = entry
                .background_color
                .as_deref()
                .and_then(|value| hex::parse(value).ok())
                .or_else(|| entry.color_id.as_deref().and_then(colors::calendar))
                .unwrap_or(colors::DEFAULT_EVENT);

            Calendar {
                id: entry.id,
                name: if entry.summary.is_empty() {
                    "Untitled".to_owned()
                } else {
                    entry.summary
                },
                color,
                primary: entry.primary.unwrap_or(false),
                selected_by_google: entry.selected.unwrap_or(true),
            }
        })
        .collect();

    calendars.sort_by(|left, right| right.primary.cmp(&left.primary).then(left.name.cmp(&right.name)));
    Ok(calendars)
}

fn fetch_events(
    access_token: &str,
    calendar: &Calendar,
    day: NaiveDate,
    day_end_hour: u32,
) -> Result<Vec<Event>, String> {
    let (time_min, time_max) = day_bounds(day, day_end_hour);
    let calendar_id = urlencoding::encode(&calendar.id);
    let mut page_token: Option<String> = None;
    let mut events = Vec::new();

    loop {
        let mut url = format!(
            "{EVENTS}/{calendar_id}/events?singleEvents=true&orderBy=startTime&maxResults=250&timeMin={}&timeMax={}",
            urlencoding::encode(&time_min),
            urlencoding::encode(&time_max),
        );
        if let Some(token) = &page_token {
            url.push_str("&pageToken=");
            url.push_str(&urlencoding::encode(token));
        }

        let list: EventList = get(&url, access_token)?;

        for entry in list.items {
            if entry.status.as_deref() == Some("cancelled") {
                continue;
            }

            if let Some(event) = parse_event(entry, calendar, day) {
                events.push(event);
            }
        }

        match list.next_page_token {
            Some(next) => page_token = Some(next),
            None => break,
        }
    }

    Ok(events)
}

const TASK_LISTS: &str = "https://tasks.googleapis.com/tasks/v1/users/@me/lists";
const TASKS: &str = "https://tasks.googleapis.com/tasks/v1/lists";


#[derive(Debug, Deserialize)]
struct TaskLists {
    #[serde(default)]
    items: Vec<TaskListEntry>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TaskListEntry {
    id: String,
    #[serde(default)]
    title: String,
}

#[derive(Debug, Deserialize)]
struct TaskPage {
    #[serde(default)]
    items: Vec<TaskEntry>,
    #[serde(default, rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TaskEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    due: Option<String>,
    #[serde(default, rename = "webViewLink")]
    web_view_link: String,
}

/// Task lists become calendars you can toggle, and each task due on `day`
/// becomes an all-day chip. Tasks are not calendar events, so the Calendar
/// API never returns them.
fn fetch_tasks(
    access_token: &str,
    day: NaiveDate,
) -> Result<(Vec<Calendar>, Vec<Event>, Option<String>), String> {
    let mut lists: TaskLists = get(&format!("{TASK_LISTS}?maxResults=100"), access_token)?;
    while let Some(token) = lists.next_page_token.take() {
        let next: TaskLists = get(&format!("{TASK_LISTS}?maxResults=100&pageToken={}", urlencoding::encode(&token)), access_token)?;
        lists.items.extend(next.items);
        lists.next_page_token = next.next_page_token;
    }
    let color = colors::TASK;
    let mut calendars = Vec::new();
    let mut events = Vec::new();

    for list in lists.items {
        let name = if list.title.is_empty() {
            "Tasks".to_owned()
        } else {
            list.title
        };
        let id = format!("tasks:{}", list.id);
        calendars.push(Calendar {
            id: id.clone(),
            name,
            color,
            primary: false,
            selected_by_google: true,
        });

        events.extend(fetch_task_items(access_token, &list.id, &id, color, day)?);
    }

    Ok((calendars, events, None))
}

fn fetch_task_items(
    access_token: &str,
    list_id: &str,
    calendar_id: &str,
    color: Color,
    _day: NaiveDate,
) -> Result<Vec<Event>, String> {
    let mut page_token: Option<String> = None;
    let mut events = Vec::new();

    loop {
        let mut url = format!(
            "{TASKS}/{}/tasks?showCompleted=true&showHidden=true&maxResults=100",
            urlencoding::encode(list_id),
        );
        if let Some(token) = &page_token {
            url.push_str("&pageToken=");
            url.push_str(&urlencoding::encode(token));
        }

        let page: TaskPage = get(&url, access_token)?;
        for mut task in page.items {
            // Keep completed block tasks in the focused timeline after sync/restart.
            if task.status.as_deref() == Some("completed") && crate::timeblocking::metadata(&task.notes).block.is_none() { continue; }
            let normalized = crate::timeblocking::notes(&task.notes, &crate::timeblocking::metadata(&task.notes))?;
            if normalized != task.notes {
                let url = format!("{TASKS}/{}/tasks/{}", urlencoding::encode(list_id), urlencoding::encode(&task.id));
                patch(&url, access_token, &serde_json::json!({"notes":normalized}).to_string(), "Google Tasks tracking")?;
                task.notes = normalized;
            }
            let due = task.due.as_deref().and_then(task_due_date)
                .unwrap_or(crate::timeblocking::UNDATED);
            if let Some(event) = parse_task(task, calendar_id, color, due) {
                events.push(event);
            }
        }

        match page.next_page_token {
            Some(next) => page_token = Some(next),
            None => break,
        }
    }

    Ok(events)
}

fn parse_task(task: TaskEntry, calendar_id: &str, color: Color, day: NaiveDate) -> Option<Event> {
    let due = task.due.as_deref().and_then(task_due_date).unwrap_or(crate::timeblocking::UNDATED);
    if due != day {
        return None;
    }

    let completed = task.status.as_deref() == Some("completed");
    let link = task_link(&task);
    let title = if task.title.is_empty() {
        "(No title)".to_owned()
    } else {
        task.title
    };
    Some(Event {
        id: format!("task:{}", task.id),
        calendar_id: calendar_id.to_owned(),
        title,
        start: date_at(day, 0, 0),
        end: date_at(day.succ_opt().unwrap_or(day), 0, 0),
        all_day: true,
        color,
        completed,
        details: crate::calendar::Details {
            description: task.notes,
            link,
            ..crate::calendar::Details::default()
        },
    })
}

/// Flips a task between complete and not in Google Tasks.
///
/// The tasks scope is read/write, so this is one of the two places the app
/// changes anything on Google's side. Blocking: call it from
/// [`crate::blocking::run`].
pub fn set_task_completed(
    access_token: &str,
    list_id: &str,
    task_id: &str,
    completed: bool,
) -> Result<(), String> {
    let url = format!(
        "{TASKS}/{}/tasks/{}",
        urlencoding::encode(list_id),
        urlencoding::encode(task_id),
    );
    let body = if completed {
        r#"{"status":"completed"}"#
    } else {
        r#"{"status":"needsAction"}"#
    };

    patch(&url, access_token, body, "Google Tasks")
}

/// Deletes a single task instance from Google Tasks.
///
/// For a recurring task this removes just the one occurrence identified by
/// `task_id`; the series and its other instances are untouched.
pub fn delete_task(
    access_token: &str,
    list_id: &str,
    task_id: &str,
) -> Result<(), String> {
    let url = format!(
        "{TASKS}/{}/tasks/{}",
        urlencoding::encode(list_id),
        urlencoding::encode(task_id),
    );

    delete(&url, access_token, "Google Tasks")
}

/// Moves an event's start and end on Google's side.
///
/// Only the two times are sent, so the title, description and guests are left
/// exactly as they were. The times are written as UTC instants, which is what
/// the API expects for a timed event. Blocking: call it from
/// [`crate::blocking::run`].
pub fn patch_event_time(
    access_token: &str,
    calendar_id: &str,
    event_id: &str,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<(), String> {
    let url = format!(
        "{EVENTS}/{}/events/{}",
        urlencoding::encode(calendar_id),
        urlencoding::encode(event_id),
    );
    let body = serde_json::json!({
        "start": { "dateTime": start.with_timezone(&Utc).to_rfc3339() },
        "end": { "dateTime": end.with_timezone(&Utc).to_rfc3339() },
    })
    .to_string();

    patch(&url, access_token, &body, "Google Calendar")
}

/// Updates an existing event on Google: title, description, start and end.
/// Blocking: call it from [`crate::blocking::run`].
pub fn patch_event(
    access_token: &str,
    calendar_id: &str,
    event_id: &str,
    title: &str,
    description: &str,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<(), String> {
    let url = format!(
        "{EVENTS}/{}/events/{}",
        urlencoding::encode(calendar_id),
        urlencoding::encode(event_id),
    );
    let mut body = serde_json::json!({
        "summary": title,
        "start": { "dateTime": start.with_timezone(&Utc).to_rfc3339() },
        "end": { "dateTime": end.with_timezone(&Utc).to_rfc3339() },
    });
    if !description.is_empty() {
        body["description"] = serde_json::Value::String(description.to_owned());
    }

    patch(&url, access_token, &body.to_string(), "Google Calendar")
}

/// Creates a timed event on Google and returns its id.
///
/// Only the fields the add-event popup collects are sent: the title, an
/// optional description, and the two times. Blocking: call it from
/// [`crate::blocking::run`].
pub fn insert_event(
    access_token: &str,
    calendar_id: &str,
    title: &str,
    description: &str,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<String, String> {
    let url = format!("{EVENTS}/{}/events", urlencoding::encode(calendar_id));
    let mut body = serde_json::json!({
        "summary": title,
        "start": { "dateTime": start.with_timezone(&Utc).to_rfc3339() },
        "end": { "dateTime": end.with_timezone(&Utc).to_rfc3339() },
    });
    if !description.is_empty() {
        body["description"] = serde_json::Value::String(description.to_owned());
    }

    let response = post(&url, access_token, &body.to_string(), "Google Calendar")?;
    let created: CreatedEvent = serde_json::from_str(&response)
        .map_err(|error| format!("Google Calendar response: {error}"))?;

    if created.id.is_empty() {
        return Err("Google Calendar did not return the new event".to_owned());
    }
    Ok(created.id)
}

/// Deletes a single event on Google. A successful call returns `204 No
/// Content` and there is no body to parse. Blocking: call it from
/// [`crate::blocking::run`].
pub fn delete_event(
    access_token: &str,
    calendar_id: &str,
    event_id: &str,
) -> Result<(), String> {
    let url = format!(
        "{EVENTS}/{}/events/{}",
        urlencoding::encode(calendar_id),
        urlencoding::encode(event_id),
    );

    delete(&url, access_token, "Google Calendar")
}

#[derive(Debug, Deserialize)]
struct CreatedEvent {
    #[serde(default)]
    id: String,
}

/// `webViewLink` opens the task in the Google Tasks UI. The API id is the
/// fallback when that field is missing.
fn task_link(task: &TaskEntry) -> String {
    if !task.web_view_link.is_empty() {
        return task.web_view_link.clone();
    }
    if task.id.is_empty() {
        return String::new();
    }
    format!("https://tasks.google.com/task/{}", task.id)
}

/// Google stores a task due date as midnight UTC. The calendar day is the
/// date in that string, not the date after converting to local time, or a
/// task due on the 1st would land on the 30th in the US.
fn task_due_date(due: &str) -> Option<NaiveDate> {
    let date = due.get(..10)?;
    NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()
}

fn tasks_notice(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("insufficient") || lower.contains("scope") {
        "Reconnect Google so Tasks are included".to_owned()
    } else if lower.contains("has not been used") || lower.contains("tasks api") || lower.contains("accessnotconfigured") {
        "Enable the Google Tasks API, then sync again".to_owned()
    } else {
        format!("Tasks: {error}")
    }
}

fn parse_event(entry: EventEntry, calendar: &Calendar, day: NaiveDate) -> Option<Event> {
    let start = entry.start.as_ref()?;
    let end = entry.end.as_ref()?;

    let (start_dt, end_dt, all_day) = match (&start.date_time, &start.date, &end.date_time, &end.date) {
        (Some(start), _, Some(end), _) => {
            (parse_date_time(start)?, parse_date_time(end)?, false)
        }
        (_, Some(start), _, Some(end)) => {
            let start_date = NaiveDate::parse_from_str(start, "%Y-%m-%d").ok()?;
            let end_date = NaiveDate::parse_from_str(end, "%Y-%m-%d").ok()?;
            // Google's all-day end is exclusive. An event on `day` is kept if
            // it covers that date.
            if day < start_date || day >= end_date {
                return None;
            }
            (
                date_at(start_date, 0, 0),
                date_at(end_date, 0, 0),
                true,
            )
        }
        _ => return None,
    };

    let color = entry
        .color_id
        .as_deref()
        .and_then(colors::event)
        .unwrap_or(calendar.color);
    let details = details_of(&entry);

    Some(Event {
        id: entry.id,
        calendar_id: calendar.id.clone(),
        title: if entry.summary.is_empty() {
            "(No title)".to_owned()
        } else {
            entry.summary
        },
        start: start_dt,
        end: end_dt,
        all_day,
        color,
        completed: false,
        details,
    })
}

fn parse_date_time(value: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Local))
        .ok()
        .or_else(|| {
            DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f%z")
                .map(|parsed| parsed.with_timezone(&Local))
                .ok()
        })
}

fn date_at(date: NaiveDate, hour: u32, minute: u32) -> DateTime<Local> {
    let naive = date.and_hms_opt(hour, minute, 0).unwrap_or_default();
    match Local.from_local_datetime(&naive) {
        chrono::offset::LocalResult::Single(when) | chrono::offset::LocalResult::Ambiguous(when, _) => {
            when
        }
        chrono::offset::LocalResult::None => naive.and_utc().with_timezone(&Local),
    }
}

fn details_of(entry: &EventEntry) -> crate::calendar::Details {
    let meeting = if !entry.hangout_link.is_empty() {
        entry.hangout_link.clone()
    } else {
        entry
            .conference
            .as_ref()
            .and_then(|conference| {
                conference
                    .entry_points
                    .iter()
                    .find(|point| point.kind == "video" && !point.uri.is_empty())
                    .or_else(|| {
                        conference
                            .entry_points
                            .iter()
                            .find(|point| !point.uri.is_empty())
                    })
                    .map(|point| point.uri.clone())
            })
            .unwrap_or_default()
    };

    crate::calendar::Details {
        location: entry.location.trim().to_owned(),
        description: plain_text(&entry.description),
        status: match entry.status.as_deref() {
            Some("confirmed") => "Confirmed".to_owned(),
            Some("tentative") => "Tentative".to_owned(),
            Some("cancelled") => "Cancelled".to_owned(),
            Some(other) if !other.is_empty() => other.to_owned(),
            _ => String::new(),
        },
        link: entry.html_link.clone(),
        meeting,
        organizer: person_label(entry.organizer.as_ref(), false),
        attendees: entry
            .attendees
            .iter()
            .map(|person| person_label(Some(person), true))
            .filter(|label| !label.is_empty())
            .collect(),
    }
}

fn person_label(person: Option<&Person>, with_response: bool) -> String {
    let Some(person) = person else {
        return String::new();
    };
    let name = if !person.display_name.is_empty() {
        person.display_name.clone()
    } else {
        person.email.clone()
    };
    if name.is_empty() {
        return String::new();
    }
    if !with_response {
        return name;
    }
    let response = match person.response_status.as_str() {
        "accepted" => "accepted",
        "declined" => "declined",
        "tentative" => "tentative",
        "needsAction" => "no reply",
        _ => "",
    };
    if response.is_empty() {
        name
    } else {
        format!("{name} ({response})")
    }
}

/// Google descriptions are plain text or a small amount of HTML.
/// Keep anchor destinations as text so the shared link renderer can open them.
fn plain_text(value: &str) -> String {
    let mut text = String::with_capacity(value.len());
    let mut rest = value;
    let mut anchor: Option<(String, usize)> = None;
    while let Some(start) = rest.find('<') {
        text.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>').map(|end| start + end) else {
            text.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let tag = rest[start + 1..end].trim();
        let name = tag.split_whitespace().next().unwrap_or("");
        if name.eq_ignore_ascii_case("a") {
            anchor = html_href(&tag[name.len()..]).map(|url| (url, text.len()));
        } else if name.eq_ignore_ascii_case("/a") {
            if let Some((url, label_start)) = anchor.take() {
                let url = decode_entities(&url);
                if decode_entities(text[label_start..].trim()) != url {
                    text.push_str(" (");
                    text.push_str(&url);
                    text.push(')');
                }
            }
        } else if ["br", "br/", "/p", "/div", "/li"].iter().any(|name| tag.eq_ignore_ascii_case(name)) {
            text.push('\n');
        } else if tag.starts_with("https://") || tag.starts_with("http://") {
            text.push('<');
            text.push_str(tag);
            text.push('>');
        }
        rest = &rest[end + 1..];
    }
    text.push_str(rest);
    decode_entities(&text).trim().to_owned()
}

/// Read quoted or unquoted HTML attributes without confusing data-href or
/// another attribute's value with the actual destination.
fn html_href(mut attributes: &str) -> Option<String> {
    while !attributes.trim_start().is_empty() {
        attributes = attributes.trim_start();
        let end = attributes.find(|ch: char| ch.is_whitespace() || ch == '=').unwrap_or(attributes.len());
        let name = &attributes[..end];
        attributes = attributes[end..].trim_start();
        if !attributes.starts_with('=') {
            continue;
        }
        attributes = attributes[1..].trim_start();
        let (value, rest) = if let Some(quote) = attributes.chars().next().filter(|ch| *ch == '\'' || *ch == '"') {
            let tail = &attributes[1..];
            let end = tail.find(quote).unwrap_or(tail.len());
            (&tail[..end], tail.get(end + 1..).unwrap_or(""))
        } else {
            let end = attributes.find(char::is_whitespace).unwrap_or(attributes.len());
            (&attributes[..end], &attributes[end..])
        };
        if name.eq_ignore_ascii_case("href") {
            return Some(value.to_owned());
        }
        attributes = rest;
    }
    None
}

fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
}

fn day_bounds(day: NaiveDate, day_end_hour: u32) -> (String, String) {
    let hour = day_end_hour.min(CalendarSettings::MAX_DAY_END_HOUR);
    let start = date_at(day, hour, 0);
    let end = date_at(day.succ_opt().unwrap_or(day), hour, 0);
    (
        start.with_timezone(&Utc).to_rfc3339(),
        end.with_timezone(&Utc).to_rfc3339(),
    )
}

pub(crate) fn get<T: serde::de::DeserializeOwned>(url: &str, access_token: &str) -> Result<T, String> {
    let result = ureq::get(url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("Accept", "application/json")
        .call();

    match result {
        Ok(response) => response
            .into_json()
            .map_err(|error| format!("Google Calendar response: {error}")),
        Err(ureq::Error::Status(code, response)) => {
            let detail = response
                .into_json::<ApiErrorBody>()
                .ok()
                .map(|body| body.error.message)
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| format!("HTTP {code}"));
            Err(format!("Google Calendar: {detail}"))
        }
        Err(error) => Err(format!("Google Calendar: {error}")),
    }
}

/// Sends a small JSON body to Google and discards the response.
///
/// `context` names the API in the error, so a failed write reads as coming
/// from Calendar or Tasks rather than from whichever call site happened to
/// share this helper.
pub(crate) fn patch(url: &str, access_token: &str, body: &str, context: &str) -> Result<(), String> {
    let result = ureq::request("PATCH", url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("Content-Type", "application/json")
        .send_string(body);

    match result {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, response)) => {
            let detail = response
                .into_json::<ApiErrorBody>()
                .ok()
                .map(|body| body.error.message)
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| format!("HTTP {code}"));
            Err(format!("{context}: {detail}"))
        }
        Err(error) => Err(format!("{context}: {error}")),
    }
}

/// Sends a `DELETE` to Google and discards the response.
///
/// A successful deletion comes back as `204 No Content` with no body, so there
/// is nothing to parse. Blocking: call it from [`crate::blocking::run`].
fn delete(url: &str, access_token: &str, context: &str) -> Result<(), String> {
    let result = ureq::request("DELETE", url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .call();

    match result {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, response)) => {
            let detail = response
                .into_json::<ApiErrorBody>()
                .ok()
                .map(|body| body.error.message)
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| format!("HTTP {code}"));
            Err(format!("{context}: {detail}"))
        }
        Err(error) => Err(format!("{context}: {error}")),
    }
}

/// Sends a JSON body to Google and returns the response text.
///
/// Used for the one call that needs something back: creating an event, whose
/// id the caller keeps so the new chip can be drawn without a full sync.
pub(crate) fn post(url: &str, access_token: &str, body: &str, context: &str) -> Result<String, String> {
    let result = ureq::post(url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("Content-Type", "application/json")
        .send_string(body);

    match result {
        Ok(response) => response
            .into_string()
            .map_err(|error| format!("{context}: {error}")),
        Err(ureq::Error::Status(code, response)) => {
            let detail = response
                .into_json::<ApiErrorBody>()
                .ok()
                .map(|body| body.error.message)
                .filter(|message| !message.is_empty())
                .unwrap_or_else(|| format!("HTTP {code}"));
            Err(format!("{context}: {detail}"))
        }
        Err(error) => Err(format!("{context}: {error}")),
    }
}

pub fn color_or_default(value: Option<Color>) -> Color {
    value.unwrap_or(colors::DEFAULT_EVENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_keep_html_link_targets_and_line_breaks() {
        assert_eq!(
            plain_text("<p>Join <a href=\"https://example.org/?a=1&amp;b=2\">the meeting</a></p><br>Bring notes"),
            "Join the meeting (https://example.org/?a=1&b=2)\n\nBring notes"
        );
        assert_eq!(
            plain_text("<A title='more info' HREF='https://example.org'>https://example.org</A>"),
            "https://example.org"
        );
        assert_eq!(plain_text("<a href=https://example.org>Visit</a>"), "Visit (https://example.org)");
        assert_eq!(plain_text("See <https://example.org>"), "See <https://example.org>");
    }

    #[test]
    fn all_day_events_use_exclusive_ends() {
        let calendar = Calendar {
            id: "primary".into(),
            name: "Personal".into(),
            color: colors::DEFAULT_EVENT,
            primary: true,
            selected_by_google: true,
        };
        let day = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let entry = EventEntry {
            id: "1".into(),
            summary: "Holiday".into(),
            start: Some(EventTime {
                date_time: None,
                date: Some("2026-10-01".into()),
            }),
            end: Some(EventTime {
                date_time: None,
                date: Some("2026-10-02".into()),
            }),
            color_id: None,
            status: None,
            ..EventEntry::default()
        };

        let event = parse_event(entry, &calendar, day).unwrap();
        assert!(event.all_day);
        assert_eq!(event.title, "Holiday");
    }

    #[test]
    fn cancelled_events_are_not_parsed_by_the_status_guard() {
        // parse_event itself does not look at status; fetch_events drops them.
        // This keeps the parser focused on times.
        let calendar = Calendar {
            id: "primary".into(),
            name: "Personal".into(),
            color: colors::DEFAULT_EVENT,
            primary: true,
            selected_by_google: true,
        };
        let day = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let entry = EventEntry {
            id: "1".into(),
            summary: "Gone".into(),
            start: Some(EventTime {
                date_time: Some("2026-10-01T09:00:00-07:00".into()),
                date: None,
            }),
            end: Some(EventTime {
                date_time: Some("2026-10-01T10:00:00-07:00".into()),
                date: None,
            }),
            color_id: Some("11".into()),
            status: Some("cancelled".into()),
            ..EventEntry::default()
        };

        let event = parse_event(entry, &calendar, day).unwrap();
        assert!(!event.all_day);
        assert_eq!(event.color, colors::event("11").unwrap());
    }

    #[test]
    fn event_details_keep_the_fields_worth_reading() {
        let calendar = Calendar {
            id: "primary".into(),
            name: "Personal".into(),
            color: colors::DEFAULT_EVENT,
            primary: true,
            selected_by_google: true,
        };
        let day = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let entry = EventEntry {
            id: "1".into(),
            summary: "Sync".into(),
            description: "<b>Bring notes</b> &amp; coffee".into(),
            location: "Room 4".into(),
            html_link: "https://calendar.google.com/event?eid=1".into(),
            hangout_link: "https://meet.google.com/abc-defg-hij".into(),
            start: Some(EventTime {
                date_time: Some("2026-10-01T09:00:00-07:00".into()),
                date: None,
            }),
            end: Some(EventTime {
                date_time: Some("2026-10-01T10:00:00-07:00".into()),
                date: None,
            }),
            status: Some("confirmed".into()),
            organizer: Some(Person {
                email: "ada@example.com".into(),
                display_name: "Ada".into(),
                response_status: String::new(),
            }),
            attendees: vec![Person {
                email: "grace@example.com".into(),
                display_name: "Grace".into(),
                response_status: "accepted".into(),
            }],
            ..EventEntry::default()
        };

        let event = parse_event(entry, &calendar, day).unwrap();
        assert_eq!(event.details.location, "Room 4");
        assert_eq!(event.details.description, "Bring notes & coffee");
        assert_eq!(event.details.status, "Confirmed");
        assert_eq!(event.details.organizer, "Ada");
        assert_eq!(event.details.attendees, vec!["Grace (accepted)".to_owned()]);
        assert_eq!(event.details.meeting, "https://meet.google.com/abc-defg-hij");
        assert!(event.details.link.contains("calendar.google.com"));
    }

    #[test]
    fn a_task_due_at_utc_midnight_stays_on_that_calendar_day() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let task = TaskEntry {
            id: "abc".into(),
            title: "File taxes".into(),
            notes: String::new(),
            status: Some("needsAction".into()),
            due: Some("2026-10-01T00:00:00.000Z".into()),
            web_view_link: "https://tasks.google.com/task/abc".into(),
        };

        let event = parse_task(task, "tasks:list", colors::DEFAULT_EVENT, day).unwrap();
        assert!(event.all_day);
        assert!(!event.completed);
        assert_eq!(event.title, "File taxes");
        assert_eq!(event.calendar_id, "tasks:list");
        assert!(event.details.status.is_empty());
        assert_eq!(event.details.link, "https://tasks.google.com/task/abc");
    }

    #[test]
    fn a_task_for_another_day_is_left_out() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let task = TaskEntry {
            id: "abc".into(),
            title: "Tomorrow".into(),
            notes: String::new(),
            status: Some("needsAction".into()),
            due: Some("2026-10-02T00:00:00.000Z".into()),
            web_view_link: String::new(),
        };

        assert!(parse_task(task, "tasks:list", colors::DEFAULT_EVENT, day).is_none());
    }

    #[test]
    fn a_missing_tasks_scope_asks_for_a_reconnect() {
        let notice = tasks_notice("Request had insufficient authentication scopes.");
        assert_eq!(notice, "Reconnect Google so Tasks are included");
    }
}
