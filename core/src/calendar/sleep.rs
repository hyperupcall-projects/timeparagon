//! Sleep is an ordinary Google event with private metadata and a separate UI source.
use super::google::GoogleError;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveTime, Timelike, Utc};
use iced_core::Color;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Event, Snapshot, api, scheduled, time_format};
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

/// Clock times for the night ending on this weekday.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Times {
    pub bedtime: NaiveTime,
    pub wakeup: NaiveTime,
}
impl Default for Times {
    fn default() -> Self {
        Self {
            bedtime: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            wakeup: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
        }
    }
}

/// A weekday schedule change effective from a particular night.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleChange {
    pub from: NaiveDate,
    pub previous: Times,
    pub times: Times,
}

pub fn times_for(settings: &CalendarSettings, day: NaiveDate) -> Times {
    let changes = || {
        settings
            .sleep_schedule_changes
            .iter()
            .filter(|change| change.from.weekday() == day.weekday())
    };
    changes()
        .filter(|change| change.from <= day)
        .max_by_key(|change| change.from)
        .map(|change| change.times)
        .or_else(|| {
            changes()
                .min_by_key(|change| change.from)
                .map(|change| change.previous)
        })
        .unwrap_or(settings.sleep_schedule[day.weekday().num_days_from_monday() as usize])
}

/// Share changes made from the grid with Settings while retaining the schedule
/// before the selected occurrence. A later edit replaces following changes.
pub fn set_weekday_times(settings: &mut CalendarSettings, from: NaiveDate, times: Times) {
    let previous = times_for(settings, from);
    settings
        .sleep_schedule_changes
        .retain(|change| change.from.weekday() != from.weekday() || change.from < from);
    settings.sleep_schedule_changes.push(ScheduleChange {
        from,
        previous,
        times,
    });
    settings.sleep_schedule[from.weekday().num_days_from_monday() as usize] = times;
}

pub fn weekday_label(day: NaiveDate) -> String {
    day.format("%A").to_string().to_lowercase()
}

/// Use local clock times, including across daylight-saving changes.
pub fn plan(wake_date: NaiveDate, schedule: &[Times; 7]) -> (DateTime<Local>, DateTime<Local>) {
    let times = schedule[wake_date.weekday().num_days_from_monday() as usize];
    let bedtime_date = if times.bedtime >= times.wakeup {
        wake_date.pred_opt().unwrap_or(wake_date)
    } else {
        wake_date
    };
    (
        super::at_clock(bedtime_date, times.bedtime.hour(), times.bedtime.minute()),
        super::at_clock(wake_date, times.wakeup.hour(), times.wakeup.minute()),
    )
}

/// Today and the next four weeks, independent of the date being viewed.
fn planning_dates(today: NaiveDate) -> impl Iterator<Item = NaiveDate> {
    (0..=28).filter_map(move |offset| today.checked_add_signed(Duration::days(offset)))
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
        "summary": title(event), "description": description(event), "colorId": null,
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
fn write(token: &str, event: &Event, new: bool) -> Result<Event, GoogleError> {
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
                scheduled::request("POST", &url, token, Some(&body))?
            }
            Err(error) => return Err(error.into()),
        };
    Ok(api::parse_sleep(value, &event.calendar_id)?)
}

/// Reconcile the weekly schedule for today and four weeks ahead, then return
/// the requested view. Browsing past or distant dates never extends planning.
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
    let token = super::GoogleClient::from_settings(settings).access_token()?;
    let mut snapshot = api::fetch_range(
        &token,
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
        let now = crate::clock::now();
        for day in planning_dates(now.date_naive()) {
            let mut schedule = settings.sleep_schedule;
            schedule[day.weekday().num_days_from_monday() as usize] = times_for(settings, day);
            if let Err(error) = reconcile(
                &token,
                &calendar_id,
                &mut snapshot.events,
                day,
                &schedule,
                now,
            ) {
                let notice = format!("Could not update Sleep: {error}");
                snapshot.notice = Some(match snapshot.notice.take() {
                    Some(old) => format!("{old}\n{notice}"),
                    None => notice,
                });
                // A permission/network failure should not repeat for every day.
                break;
            }
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
    schedule: &[Times; 7],
    now: DateTime<Local>,
) -> Result<(), String> {
    reconcile_with(
        calendar_id,
        events,
        day,
        schedule,
        now,
        |day| get(token, calendar_id, day),
        |event, new| write(token, event, new).map_err(String::from),
    )
}

fn reconcile_with(
    calendar_id: &str,
    events: &mut Vec<Event>,
    day: NaiveDate,
    schedule: &[Times; 7],
    now: DateTime<Local>,
    mut load: impl FnMut(NaiveDate) -> Result<Option<Event>, String>,
    mut save: impl FnMut(&Event, bool) -> Result<Event, String>,
) -> Result<(), String> {
    // Guard before loading or writing: settings never change historical days,
    // and no caller can create sleep beyond the rolling four-week horizon.
    if day < now.date_naive() || day > now.date_naive() + Duration::days(28) {
        return Ok(());
    }
    let (start, end) = plan(day, schedule);
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
        .is_some_and(|e| e.details.sleep.as_ref().is_none_or(|s| !s.automatic))
    {
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
/// nights are preserved during subsequent weekly schedule changes.
/// Saves a night locally and sends it to Google.
pub fn save(settings: &CalendarSettings, mut event: Event, new: bool) -> Result<Event, String> {
    if event.end <= event.start {
        return Err("Wake-up time must be after sleep start".into());
    }
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
    super::operations::submit(
        super::operations::Job::Sleep {
            event: event.clone(),
            new,
        },
        Some(&event),
        &event.calendar_id,
        &event.id,
    )?;
    Ok(event)
}

/// Sends a night saved by [`save`] to Google. Only `operations` calls this.
pub(crate) fn send(
    client: &super::GoogleClient,
    mut event: Event,
    new: bool,
) -> Result<Event, GoogleError> {
    if event.end <= event.start {
        return Err("Wake-up time must be after sleep start".into());
    }
    let token = client.access_token()?;
    if new {
        event.calendar_id = scheduled::app_calendar(&token)?;
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
            &token,
            &event.calendar_id,
            event.details.sleep.as_ref().unwrap().wake_date,
        )? {
            event.id = existing.id;
            return write(&token, &event, false);
        }
    }
    write(&token, &event, new)
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
        assert_eq!(value.get("colorId"), Some(&Value::Null));
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
    fn weekly_times_ignore_appointments_and_preserve_minutes() {
        let mut schedule = [Times::default(); 7];
        schedule[date().weekday().num_days_from_monday() as usize] = Times {
            bedtime: NaiveTime::from_hms_opt(22, 45, 0).unwrap(),
            wakeup: NaiveTime::from_hms_opt(7, 15, 0).unwrap(),
        };
        let mut events = vec![appointment(5)];
        reconcile_with(
            "app",
            &mut events,
            date(),
            &schedule,
            at(12),
            |_| Ok(None),
            |event, _| Ok(event.clone()),
        )
        .unwrap();
        let night = events.last().unwrap();
        assert_eq!(night.end, at(7) + Duration::minutes(15));
        assert_eq!(
            night.start,
            super::super::at_clock(date().pred_opt().unwrap(), 22, 45)
        );
        assert_eq!(
            plan(date().succ_opt().unwrap(), &schedule).1,
            super::super::at_clock(date().succ_opt().unwrap(), 8, 0)
        );
    }

    #[test]
    fn reconciliation_is_bounded_and_preserves_recorded_nights() {
        let schedule = [Times::default(); 7];
        let now = at(12);
        for day in [date() - Duration::days(1), date() + Duration::days(29)] {
            reconcile_with(
                "app",
                &mut vec![],
                day,
                &schedule,
                now,
                |_| panic!("must not load outside horizon"),
                |_, _| panic!("must not write outside horizon"),
            )
            .unwrap();
        }
        for offset in [0, 28] {
            let mut events = vec![];
            let day = date() + Duration::days(offset);
            reconcile_with(
                "app",
                &mut events,
                day,
                &schedule,
                now,
                |_| Ok(None),
                |event, new| {
                    assert!(new);
                    Ok(event.clone())
                },
            )
            .unwrap();
            assert_eq!(events.len(), 1);
            events[0].details.sleep.as_mut().unwrap().automatic = false;
            reconcile_with(
                "app",
                &mut events,
                day,
                &schedule,
                now,
                |_| panic!(),
                |_, _| panic!("recorded night must survive"),
            )
            .unwrap();
        }
        let dates: Vec<_> = planning_dates(date()).collect();
        assert_eq!(dates.len(), 29);
        assert_eq!(dates[0], date());
        assert_eq!(dates[28], date() + Duration::days(28));
    }

    #[test]
    fn schedule_changes_update_existing_nights_without_duplicates() {
        let mut events = vec![];
        let mut schedule = [Times::default(); 7];
        for hour in [8, 9] {
            schedule[date().weekday().num_days_from_monday() as usize].wakeup =
                NaiveTime::from_hms_opt(hour, 30, 0).unwrap();
            reconcile_with(
                "app",
                &mut events,
                date(),
                &schedule,
                at(12),
                |_| Ok(None),
                |event, new| {
                    assert_eq!(new, hour == 8);
                    Ok(event.clone())
                },
            )
            .unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].end, at(hour) + Duration::minutes(30));
        }
        reconcile_with(
            "app",
            &mut events,
            date(),
            &schedule,
            at(12),
            |_| panic!(),
            |_, _| panic!("unchanged schedule must not write"),
        )
        .unwrap();
    }
    #[test]
    fn failed_writes_preserve_the_existing_event() {
        let mut event = appointment(7);
        event.calendar_id = "app".into();
        event.details.sleep = Some(Record::planned(date()));
        let mut events = vec![event];
        let original = events.clone();
        let result = reconcile_with(
            "app",
            &mut events,
            date(),
            &[Times::default(); 7],
            at(12),
            |_| panic!(),
            |_, _| Err("offline".into()),
        );
        assert_eq!(result, Err("offline".into()));
        assert_eq!(events, original);
    }
    #[test]
    fn dated_weekday_changes_preserve_earlier_nights_and_later_edits_replace_following_changes() {
        let mut settings = CalendarSettings::default();
        let first = date();
        let second = first + Duration::days(14);
        let times = |hour| Times {
            bedtime: NaiveTime::from_hms_opt(23, 30, 0).unwrap(),
            wakeup: NaiveTime::from_hms_opt(hour, 15, 0).unwrap(),
        };
        set_weekday_times(&mut settings, first, times(7));
        set_weekday_times(&mut settings, second, times(9));
        assert_eq!(
            times_for(&settings, first - Duration::days(7)),
            Times::default()
        );
        assert_eq!(times_for(&settings, first + Duration::days(7)), times(7));
        assert_eq!(times_for(&settings, second + Duration::days(7)), times(9));
        assert_eq!(settings.sleep_schedule[0], times(9));
        set_weekday_times(&mut settings, first + Duration::days(7), times(8));
        assert_eq!(times_for(&settings, first), times(7));
        assert_eq!(times_for(&settings, second), times(8));
        assert_eq!(
            times_for(&settings, first + Duration::days(1)),
            Times::default()
        );
    }
}
