//! One-off migration of the legacy per-purpose calendars into the single
//! TimeSlay calendar.
//!
//! Before the app used one calendar it wrote time blocks to a calendar of the
//! user's choosing (often "Time Blocks") and scheduled task events to another
//! ("Tasks" / "TimeTracker Tasks"). It could also have left more than one
//! calendar named TimeSlay/TimeSlayer behind. This moves every event from
//! those calendars into the one canonical TimeSlay calendar, then removes the
//! now-empty sources.
//!
//! Run it with `cargo run --example migrate_calendar` (plan only) or
//! `... -- --apply` to make the changes. It reuses the signed-in token the app
//! already stores, so run the app once first if you have not connected Google.

use serde_json::{Value, json};

use crate::calendar::{api, oauth, scheduled};
use crate::config::CalendarSettings;

/// Calendar names that used to hold app events and should be emptied into the
/// TimeSlay calendar. The misspelled "Timem Blocks" is included because it is
/// easy to create by hand.
pub const LEGACY_CALENDAR_NAMES: [&str; 3] = ["Time Blocks", "TimeTracker Tasks", "Timem Blocks"];

const CALENDAR_LIST: &str = "https://www.googleapis.com/calendar/v3/users/me/calendarList";
const CALENDARS: &str = "https://www.googleapis.com/calendar/v3/calendars";

/// What the migration did, or would do in a dry run.
#[derive(Debug, Default)]
pub struct Report {
    /// True when nothing was written to Google.
    pub dry_run: bool,
    /// The name of the calendar everything is moved into.
    pub target: String,
    /// One entry per source calendar that was emptied.
    pub moved: Vec<MovedCalendar>,
    /// The names of the source calendars deleted after they were emptied.
    pub deleted: Vec<String>,
    /// Problems that did not stop the rest of the migration.
    pub errors: Vec<String>,
}

#[derive(Debug)]
pub struct MovedCalendar {
    pub name: String,
    pub events: usize,
}

/// Moves every event out of the legacy calendars and into the single TimeSlay
/// calendar. With `apply` false it only reports what it would do.
pub fn run(apply: bool) -> Result<Report, String> {
    let settings = crate::config::Config::load().calendar;
    let tokens = oauth::load_tokens().ok_or("Not connected to Google Calendar")?;
    let tokens = oauth::ensure_access(&settings.client_id, &settings.client_secret, tokens)?;
    let token = tokens.access_token;

    let calendars = list_calendars(&token)?;
    let mut report = Report {
        dry_run: !apply,
        target: CalendarSettings::APP_CALENDAR_NAME.to_owned(),
        ..Report::default()
    };

    // The target is the canonical calendar, else a former name (renamed), else
    // a new calendar. Without `apply` there is nothing to create, so the plan
    // just names what would be.
    let canonical = calendars.iter().find(|calendar| {
        calendar_name(calendar) == CalendarSettings::APP_CALENDAR_NAME
    });
    let alias = calendars
        .iter()
        .find(|calendar| CalendarSettings::is_app_calendar_name(calendar_name(calendar)));
    let target = match (canonical, alias) {
        (Some(calendar), _) => Some(calendar_id(calendar).to_owned()),
        (None, Some(calendar)) => {
            let id = calendar_id(calendar).to_owned();
            if apply {
                rename_calendar(&token, &id, CalendarSettings::APP_CALENDAR_NAME)?;
            }
            Some(id)
        }
        (None, None) if apply => Some(create_calendar(&token, CalendarSettings::APP_CALENDAR_NAME)?),
        (None, None) => None,
    };

    for calendar in &calendars {
        let id = calendar_id(calendar);
        let name = calendar_name(calendar);
        if Some(id) == target.as_deref() {
            continue;
        }
        if !is_source(name) {
            continue;
        }

        let events = list_events(&token, id)?;
        // Cancelled events and the exceptions of a recurring series are not
        // moved on their own: the series master carries them along.
        let movable: Vec<&Value> = events
            .iter()
            .filter(|event| event["status"].as_str() != Some("cancelled"))
            .filter(|event| event.get("recurringEventId").is_none())
            .collect();
        report.moved.push(MovedCalendar {
            name: name.to_owned(),
            events: movable.len(),
        });

        if !apply {
            // The plan empties every source, so every source would go.
            report.deleted.push(name.to_owned());
            continue;
        }
        let Some(target_id) = target.as_deref() else {
            continue;
        };
        let mut remaining = 0usize;
        for event in movable {
            let Some(event_id) = event["id"].as_str() else {
                continue;
            };
            match move_event(&token, id, event_id, target_id) {
                Ok(()) => {}
                Err(error) => {
                    remaining += 1;
                    report.errors.push(format!("{name}: {error}"));
                }
            }
        }
        // Only remove a source once nothing is left in it, so a failed move
        // never loses an event.
        if remaining == 0 {
            match delete_calendar(&token, id) {
                Ok(()) => report.deleted.push(name.to_owned()),
                Err(error) => report.errors.push(format!("{name}: {error}")),
            }
        }
    }

    Ok(report)
}

fn is_source(name: &str) -> bool {
    let name = name.trim();
    LEGACY_CALENDAR_NAMES.contains(&name) || CalendarSettings::is_app_calendar_name(name)
}

fn calendar_id(calendar: &Value) -> &str {
    calendar["id"].as_str().unwrap_or_default()
}

fn calendar_name(calendar: &Value) -> &str {
    calendar["summary"].as_str().unwrap_or_default()
}

fn list_calendars(token: &str) -> Result<Vec<Value>, String> {
    let mut page: Option<String> = None;
    let mut items = Vec::new();
    loop {
        let url = format!(
            "{CALENDAR_LIST}?maxResults=250{}",
            page.as_ref()
                .map(|p| format!("&pageToken={}", urlencoding::encode(p)))
                .unwrap_or_default()
        );
        let response = scheduled::request("GET", &url, token, None).map_err(|error| error.message)?;
        items.extend(
            response["items"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        );
        page = response["nextPageToken"].as_str().map(str::to_owned);
        if page.is_none() {
            return Ok(items);
        }
    }
}

fn list_events(token: &str, calendar_id: &str) -> Result<Vec<Value>, String> {
    let mut page: Option<String> = None;
    let mut items = Vec::new();
    loop {
        let url = format!(
            "{}/{}/events?maxResults=2500&showDeleted=false&singleEvents=false{}",
            api::EVENTS,
            urlencoding::encode(calendar_id),
            page.as_ref()
                .map(|p| format!("&pageToken={}", urlencoding::encode(p)))
                .unwrap_or_default()
        );
        let response = scheduled::request("GET", &url, token, None).map_err(|error| error.message)?;
        items.extend(response["items"].as_array().cloned().unwrap_or_default());
        page = response["nextPageToken"].as_str().map(str::to_owned);
        if page.is_none() {
            return Ok(items);
        }
    }
}

/// Moves one event onto the target calendar. Calendar's own move keeps every
/// field — title, notes, times and the app's extended properties — intact.
fn move_event(
    token: &str,
    source: &str,
    event_id: &str,
    target: &str,
) -> Result<(), String> {
    let url = format!(
        "{}/{}/events/{}/move?destination={}",
        api::EVENTS,
        urlencoding::encode(source),
        urlencoding::encode(event_id),
        urlencoding::encode(target),
    );
    scheduled::request("POST", &url, token, None).map(|_| ()).map_err(|error| error.message)
}

fn create_calendar(token: &str, name: &str) -> Result<String, String> {
    let body = json!({ "summary": name });
    scheduled::request("POST", CALENDARS, token, Some(&body)).map_err(|error| error.message)?["id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "Google did not return an ID for the TimeSlay calendar".to_owned())
}

fn rename_calendar(token: &str, id: &str, name: &str) -> Result<(), String> {
    let url = format!("{CALENDARS}/{}", urlencoding::encode(id));
    let body = json!({ "summary": name });
    scheduled::request("PATCH", &url, token, Some(&body))
        .map(|_| ())
        .map_err(|error| error.message)
}

fn delete_calendar(token: &str, id: &str) -> Result<(), String> {
    let url = format!("{CALENDARS}/{}", urlencoding::encode(id));
    scheduled::request("DELETE", &url, token, None)
        .map(|_| ())
        .map_err(|error| error.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_and_alias_names_are_migrated_but_a_normal_calendar_is_not() {
        for name in ["Time Blocks", "TimeTracker Tasks", "Timem Blocks", "TimeSlay", "TimeSlayer"] {
            assert!(is_source(name), "{name} should be a migration source");
        }
        for name in ["Personal", "Work", "Tasks", "Birthdays"] {
            assert!(!is_source(name), "{name} should be left alone");
        }
    }

    #[test]
    fn a_name_is_matched_with_surrounding_space() {
        assert!(is_source("  Time Blocks "));
        assert!(CalendarSettings::is_app_calendar_name(" TimeSlay "));
    }
}
