//! Sleep is an ordinary Google event with private metadata and a separate UI source.
use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use iced_core::Color;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Event, Snapshot, api, oauth, scheduled, time_format};
use crate::config::CalendarSettings;

pub const SOURCE_ID: &str = "timeslay:sleep";
pub const NEW_ID: &str = "new-sleep";
pub const COLOR: Color = Color::from_rgb(0.48, 0.50, 0.53);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Stable identity of a night, even after its times are edited.
    pub wake_date: NaiveDate,
    pub automatic: bool,
    pub estimated_start: bool,
    pub estimated_end: bool,
}
impl Record {
    pub fn planned(wake_date: NaiveDate) -> Self {
        Self {
            wake_date,
            automatic: true,
            estimated_start: true,
            estimated_end: true,
        }
    }
    pub fn from_properties(properties: &Value) -> Option<Self> {
        if properties["private"]["ttKind"].as_str() != Some("sleep") {
            return None;
        }
        serde_json::from_str(properties["private"]["ttSleep"].as_str()?).ok()
    }
}

pub fn duration(event: &Event) -> String {
    let minutes = (event.end - event.start).num_minutes();
    if minutes % 60 == 0 {
        format!("{}h", minutes / 60)
    } else {
        format!("{}h {}m", minutes / 60, minutes % 60)
    }
}
pub fn title(event: &Event) -> String {
    format!("Sleep · {}", duration(event))
}
pub fn time_label(event: &Event, format: time_format::TimeFormat) -> String {
    format!(
        "Bedtime {}\nWake up {}",
        time_format::clock(event.start, format),
        time_format::clock(event.end, format)
    )
}

/// The user's sleep preferences that decide when a night starts and ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rules {
    pub goal: u32,
    pub day_start: u32,
    pub latest_wakeup: u32,
}
impl Rules {
    pub fn new(settings: &CalendarSettings) -> Self {
        Self {
            goal: settings.sleep_goal_hours(),
            day_start: settings.sleep_day_start_hour(),
            latest_wakeup: settings.latest_wakeup_hour(),
        }
    }
}

/// Wake up at the first timed event between the sleep-day start and the
/// latest wake-up time on `wake_date`; with no such event, at the latest
/// wake-up time. All-day entries and due-only tasks do not anchor sleep.
pub fn plan(
    events: &[Event],
    wake_date: NaiveDate,
    rules: Rules,
) -> (DateTime<Local>, DateTime<Local>) {
    let from = super::at_clock(wake_date, rules.day_start, 0);
    let latest = super::at_clock(wake_date, rules.latest_wakeup.max(rules.day_start), 0);
    let wake = events
        .iter()
        .filter(|e| e.details.sleep.is_none() && !e.all_day)
        .filter(|e| !e.is_task() || e.details.scheduled.is_some())
        .map(|e| e.start)
        .filter(|&start| start >= from && start < latest)
        .min()
        .unwrap_or(latest);
    // Subtract elapsed hours, so the actual sleep goal survives DST transitions.
    (
        wake - Duration::hours(i64::from(rules.goal.clamp(6, 12))),
        wake,
    )
}

/// Description kept on the Google event, regenerated whenever the times change.
pub fn description(event: &Event) -> String {
    time_label(event, time_format::TimeFormat::TwelveHour)
}

fn collection(calendar_id: &str) -> String {
    format!(
        "{}/{}/events",
        api::EVENTS,
        urlencoding::encode(calendar_id)
    )
}
fn event_url(calendar_id: &str, id: &str) -> String {
    format!("{}/{}", collection(calendar_id), urlencoding::encode(id))
}
fn event_id(day: NaiveDate) -> String {
    // Google IDs accept base32hex characters; this is stable across devices.
    format!("51ee9{}", day.format("%Y%m%d"))
}
fn body(event: &Event) -> Result<Value, String> {
    let record = event
        .details
        .sleep
        .as_ref()
        .ok_or("Missing sleep metadata")?;
    Ok(json!({
        "summary": title(event), "description": description(event), "colorId": "8",
        "start": {"dateTime": event.start.with_timezone(&Utc).to_rfc3339()},
        "end": {"dateTime": event.end.with_timezone(&Utc).to_rfc3339()},
        "extendedProperties": {"private": {
            "ttKind": "sleep", "ttSleepDate": record.wake_date.to_string(),
            "ttSleep": serde_json::to_string(record).map_err(|e| e.to_string())?
        }},
        "reminders": {"useDefault": false}
    }))
}
fn get(token: &str, calendar_id: &str, day: NaiveDate) -> Result<Option<Event>, String> {
    // No time limits: a recorded night may have been moved outside the viewed range.
    let url = format!(
        "{}?privateExtendedProperty=ttSleepDate%3D{}&showDeleted=false",
        collection(calendar_id),
        day
    );
    let value = scheduled::request("GET", &url, token, None).map_err(|e| e.message)?;
    value["items"]
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .map(|value| api::parse_sleep(value, calendar_id))
        .transpose()
}
fn write(token: &str, event: &Event, new: bool) -> Result<Event, String> {
    let mut body = body(event)?;
    let url = if new {
        body["id"] = json!(event.id);
        collection(&event.calendar_id)
    } else {
        event_url(&event.calendar_id, &event.id)
    };
    let value =
        match scheduled::request(if new { "POST" } else { "PATCH" }, &url, token, Some(&body)) {
            Ok(value) => value,
            Err(error) if new && error.status == Some(409) => {
                // Another device may have created this night, or its original ID
                // may be a deleted Google tombstone. Never overwrite a recorded night.
                if let Some(existing) = get(
                    token,
                    &event.calendar_id,
                    event.details.sleep.as_ref().unwrap().wake_date,
                )? {
                    return if event.details.sleep.as_ref().unwrap().automatic {
                        Ok(existing)
                    } else {
                        let mut event = event.clone();
                        event.id = existing.id;
                        write(token, &event, false)
                    };
                }
                body.as_object_mut().unwrap().remove("id");
                scheduled::request("POST", &url, token, Some(&body)).map_err(|e| e.message)?
            }
            Err(error) => return Err(error.message),
        };
    api::parse_sleep(value, &event.calendar_id)
}

/// Fetch one extra day to know tomorrow's first appointment, then reconcile
/// managed sleep events before the snapshot is installed or cached.
pub fn sync(
    settings: &CalendarSettings,
    anchor: NaiveDate,
    dates: &[NaiveDate],
) -> Result<Snapshot, String> {
    let first = *dates.first().ok_or("Calendar range is empty")?;
    let last = dates
        .last()
        .unwrap()
        .succ_opt()
        .ok_or("Invalid calendar date")?;
    let tokens = oauth::load_tokens().ok_or("Not connected to Google Calendar")?;
    let tokens = oauth::ensure_access(&settings.client_id, &settings.client_secret, tokens)?;
    let mut snapshot = api::fetch_range(
        &tokens.access_token,
        &settings.disabled,
        anchor,
        first.pred_opt().unwrap_or(first),
        last,
        0,
        settings.app_calendar.as_deref(),
    )?;
    let calendar_id = settings
        .app_calendar
        .as_ref()
        .filter(|id| snapshot.calendars.iter().any(|c| &c.id == *id))
        .cloned()
        .or_else(|| {
            snapshot
                .calendars
                .iter()
                .find(|c| CalendarSettings::is_app_calendar_name(&c.name))
                .map(|c| c.id.clone())
        });
    if let Some(calendar_id) = calendar_id {
        let mut day = first;
        while day <= last {
            if let Err(error) = reconcile(
                &tokens.access_token,
                &calendar_id,
                &mut snapshot.events,
                day,
                Rules::new(settings),
                crate::clock::now(),
            ) {
                let notice = format!("Could not update Sleep: {error}");
                snapshot.notice = Some(match snapshot.notice.take() {
                    Some(old) => format!("{old}\n{notice}"),
                    None => notice,
                });
                // A permission/network failure should not repeat for every day.
                break;
            }
            day = match day.succ_opt() {
                Some(day) => day,
                None => break,
            };
        }
    }
    snapshot.events.retain(|event| {
        dates
            .iter()
            .any(|&day| super::event_on_day(event, day, settings.day_start_hour()))
    });
    Ok(snapshot)
}

fn reconcile(
    token: &str,
    calendar_id: &str,
    events: &mut Vec<Event>,
    day: NaiveDate,
    rules: Rules,
    now: DateTime<Local>,
) -> Result<(), String> {
    reconcile_with(
        calendar_id,
        events,
        day,
        rules,
        now,
        |day| get(token, calendar_id, day),
        |event, new| write(token, event, new),
    )
}

fn reconcile_with(
    calendar_id: &str,
    events: &mut Vec<Event>,
    day: NaiveDate,
    rules: Rules,
    now: DateTime<Local>,
    mut load: impl FnMut(NaiveDate) -> Result<Option<Event>, String>,
    mut save: impl FnMut(&Event, bool) -> Result<Event, String>,
) -> Result<(), String> {
    let (start, end) = plan(events, day, rules);
    let mut existing = events
        .iter()
        .find(|e| {
            e.calendar_id == calendar_id
                && e.details.sleep.as_ref().is_some_and(|s| s.wake_date == day)
        })
        .cloned();
    if existing.is_none() {
        existing = load(day)?;
    }
    if existing
        .as_ref()
        .is_some_and(|e| e.end <= now || e.details.sleep.as_ref().is_none_or(|s| !s.automatic))
    {
        return Ok(());
    }
    // Never create a night whose wake-up is already behind us.
    if existing.is_none() && end <= now {
        return Ok(());
    }
    let mut event = Event {
        id: existing
            .as_ref()
            .map_or_else(|| event_id(day), |e| e.id.clone()),
        calendar_id: calendar_id.into(),
        title: String::new(),
        start,
        end,
        all_day: false,
        color: COLOR,
        completed: false,
        details: super::Details {
            sleep: Some(Record::planned(day)),
            ..Default::default()
        },
    };
    event.title = title(&event);
    event.details.description = description(&event);
    // Also repairs title/description of nights written by older versions.
    if existing.as_ref().is_some_and(|e| {
        e.start == start
            && e.end == end
            && e.title == event.title
            && e.details.description == event.details.description
    }) {
        return Ok(());
    }
    let saved = save(&event, existing.is_none())?;
    events.retain(|e| e.id != saved.id || e.calendar_id != calendar_id);
    events.push(saved);
    Ok(())
}

/// Save user-entered times and confidence flags as a recorded night. Recorded
/// nights are preserved during subsequent schedule/goal changes.
pub fn save(settings: &CalendarSettings, mut event: Event, new: bool) -> Result<Event, String> {
    if event.end <= event.start {
        return Err("Wake-up time must be after sleep start".into());
    }
    if !super::operations::replaying() {
        if new {
            event.calendar_id = settings
                .app_calendar
                .clone()
                .ok_or("Sync Google once to discover the TimeSlay calendar")?;
            event.id = event_id(
                event
                    .details
                    .sleep
                    .as_ref()
                    .ok_or("Missing sleep metadata")?
                    .wake_date,
            );
        }
        super::operations::enqueue(
            super::operations::Job::Sleep {
                event: event.clone(),
                new,
            },
            Some(&event),
            &event.calendar_id,
            &event.id,
        )?;
        return Ok(event);
    }
    let tokens = oauth::load_tokens().ok_or("Not connected to Google Calendar")?;
    let tokens = oauth::ensure_access(&settings.client_id, &settings.client_secret, tokens)?;
    if new {
        event.calendar_id = scheduled::app_calendar(&tokens.access_token)?;
        event.id = event_id(
            event
                .details
                .sleep
                .as_ref()
                .ok_or("Missing sleep metadata")?
                .wake_date,
        );
        // Opening a manually added night must not create a second copy.
        if let Some(existing) = get(
            &tokens.access_token,
            &event.calendar_id,
            event.details.sleep.as_ref().unwrap().wake_date,
        )? {
            event.id = existing.id;
            return write(&tokens.access_token, &event, false);
        }
    }
    write(&tokens.access_token, &event, new)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2030, 11, 4).unwrap()
    }
    fn at(hour: u32) -> DateTime<Local> {
        super::super::at_clock(date(), hour, 0)
    }
    fn appointment(hour: u32) -> Event {
        Event {
            id: format!("meeting-{hour}"),
            calendar_id: "work".into(),
            title: "Meeting".into(),
            start: at(hour),
            end: at(hour) + Duration::hours(1),
            all_day: false,
            color: COLOR,
            completed: false,
            details: Default::default(),
        }
    }
    fn rules(goal: u32) -> Rules {
        Rules {
            goal,
            day_start: 0,
            latest_wakeup: 8,
        }
    }
    fn update(events: &mut Vec<Event>, goal: u32) -> Vec<bool> {
        let mut writes = Vec::new();
        reconcile_with(
            "app",
            events,
            date(),
            rules(goal),
            at(0) - Duration::days(1),
            |_| Ok(None),
            |event, new| {
                writes.push(new);
                Ok(event.clone())
            },
        )
        .unwrap();
        writes
    }
    #[test]
    fn first_timed_event_drives_sleep_and_recalculates_without_duplicates() {
        let mut events = vec![appointment(10), appointment(7)];
        assert_eq!(update(&mut events, 8), [true]);
        let night = events.last().unwrap();
        assert_eq!(night.end, at(7));
        assert_eq!(night.start.date_naive(), date().pred_opt().unwrap());
        assert_eq!(night.end - night.start, Duration::hours(8));
        let id = night.id.clone();
        assert!(update(&mut events, 8).is_empty());
        events.retain(|e| e.id != "meeting-7");
        assert_eq!(update(&mut events, 8), [false]);
        let night = events.iter().find(|e| e.id == id).unwrap();
        assert_eq!(night.end, at(8));
        assert_eq!(update(&mut events, 12), [false]);
        assert_eq!(
            events.iter().filter(|e| e.details.sleep.is_some()).count(),
            1
        );
        let night = events.iter().find(|e| e.id == id).unwrap();
        assert_eq!(night.end - night.start, Duration::hours(12));
    }
    #[test]
    fn recorded_sleep_and_completed_nights_survive_schedule_and_goal_changes() {
        let mut events = vec![appointment(7)];
        update(&mut events, 8);
        events
            .last_mut()
            .unwrap()
            .details
            .sleep
            .as_mut()
            .unwrap()
            .automatic = false;
        let recorded = events.last().unwrap().clone();
        events[0] = appointment(10);
        assert!(update(&mut events, 12).is_empty());
        assert_eq!(events.last().unwrap(), &recorded);
        events
            .last_mut()
            .unwrap()
            .details
            .sleep
            .as_mut()
            .unwrap()
            .automatic = true;
        let past = events.last().unwrap().clone();
        reconcile_with(
            "app",
            &mut events,
            date(),
            rules(6),
            at(12),
            |_| panic!(),
            |_, _| panic!(),
        )
        .unwrap();
        assert_eq!(events.last().unwrap(), &past);
    }
    #[test]
    fn empty_days_get_estimated_sleep_and_reuse_it_after_appointments_are_removed() {
        let mut events = Vec::new();
        assert_eq!(update(&mut events, 8), [true]);
        assert_eq!(events[0].end, at(8));
        assert_eq!(events[0].end - events[0].start, Duration::hours(8));
        assert!(update(&mut events, 8).is_empty());
        events.push(appointment(7));
        assert_eq!(update(&mut events, 8), [false]);
        events.retain(|event| event.details.sleep.is_some());
        assert_eq!(update(&mut events, 8), [false]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].end, at(8));
    }

    #[test]
    fn missing_nights_are_created_only_for_present_and_future_dates() {
        let mut events = Vec::new();
        for day in [date(), date().succ_opt().unwrap()] {
            reconcile_with(
                "app",
                &mut events,
                day,
                rules(9),
                at(0) - Duration::days(10),
                |_| Ok(None),
                |event, new| {
                    assert!(new);
                    Ok(event.clone())
                },
            )
            .unwrap();
        }
        assert_eq!(events.len(), 2);
        assert_ne!(events[0].id, events[1].id);
        assert_eq!(events[0].end, at(8));
        assert_eq!(events[1].start, super::super::at_clock(date(), 23, 0));
        // Once the wake-up time has passed, nothing is created.
        let mut past = Vec::new();
        reconcile_with(
            "app",
            &mut past,
            date(),
            rules(9),
            at(12),
            |_| Ok(None),
            |_, _| panic!("past nights must not be created"),
        )
        .unwrap();
        assert!(past.is_empty());
    }

    #[test]
    fn all_day_tasks_other_dates_and_sleep_do_not_anchor_a_night() {
        let mut all_day = appointment(1);
        all_day.all_day = true;
        let mut task = appointment(2);
        task.id = "task:due-only".into();
        let mut other_day = appointment(3);
        other_day.start -= Duration::days(1);
        let mut sleep = appointment(4);
        sleep.details.sleep = Some(Record::planned(date()));
        assert_eq!(
            plan(
                &[all_day, task, other_day, sleep, appointment(6)],
                date(),
                rules(8)
            )
            .1,
            at(6)
        );
        for (goal, hours) in [(0, 6), (8, 8), (99, 12)] {
            let (start, end) = plan(&[appointment(7)], date(), rules(goal));
            assert_eq!((end - start).num_hours(), hours);
        }
    }
    #[test]
    fn wakeup_uses_first_event_in_the_sleep_day_else_the_latest_wakeup() {
        let custom = Rules {
            goal: 8,
            day_start: 4,
            latest_wakeup: 10,
        };
        // Before the sleep day starts: ignored. After the latest wake-up: ignored.
        assert_eq!(
            plan(&[appointment(2), appointment(12)], date(), custom).1,
            at(10)
        );
        assert_eq!(
            plan(
                &[appointment(2), appointment(9), appointment(5)],
                date(),
                custom
            )
            .1,
            at(5)
        );
        assert_eq!(plan(&[appointment(10)], date(), custom).1, at(10));
    }
    #[test]
    fn retitles_stale_nights_and_describes_the_times() {
        let mut events = vec![appointment(7)];
        update(&mut events, 8);
        let night = events.last().unwrap();
        assert_eq!(night.title, "Sleep · 8h");
        assert!(night.details.description.contains("Wake up 7:00AM"));
        events.last_mut().unwrap().title = "Sleep".into();
        assert_eq!(update(&mut events, 8), [false]);
        assert_eq!(events.last().unwrap().title, "Sleep · 8h");
    }
    #[test]
    fn confidence_times_and_type_round_trip_through_google_and_cache() {
        let mut event = appointment(1);
        event.calendar_id = "app".into();
        event.end = at(9) + Duration::minutes(30);
        let mut record = Record::planned(date());
        record.automatic = false;
        record.estimated_end = false;
        event.details.sleep = Some(record.clone());
        let mut value = body(&event).unwrap();
        value["id"] = json!("sleep-id");
        let parsed = api::parse_sleep(value, "app").unwrap();
        assert_eq!(parsed.details.sleep, Some(record));
        assert_eq!((parsed.start, parsed.end), (event.start, event.end));
        assert_eq!(title(&parsed), "Sleep · 8h 30m");
        let cached = serde_json::to_string(&parsed.details).unwrap();
        let details: super::super::Details = serde_json::from_str(&cached).unwrap();
        assert_eq!(details, parsed.details);
        assert!(super::super::EditDraft::from_event(&parsed).sleep.is_some());
        assert!(!crate::timeblocking::is_block(
            &parsed,
            &CalendarSettings {
                app_calendar: Some("app".into()),
                ..Default::default()
            }
        ));
    }
    #[test]
    fn a_failed_write_preserves_the_existing_calendar_event() {
        let mut events = vec![appointment(7)];
        update(&mut events, 8);
        events[0] = appointment(9);
        let original = events.clone();
        let result = reconcile_with(
            "app",
            &mut events,
            date(),
            rules(8),
            at(0),
            |_| Ok(None),
            |_, _| Err("offline".into()),
        );
        assert_eq!(result, Err("offline".into()));
        assert_eq!(events, original);
    }
}
