//! Google Calendar, as the time block reconciler sees it.
//!
//! See [`crate::timeblocking::reconcile`] for what the reconciler does. This
//! is only how it reads and writes Google. It runs on the write worker
//! thread, which is already sending to Google.

use super::*;
use crate::task_edit::{self, Change};
use crate::timeblocking::membership::{parse_instance_id, planning_day};
use crate::timeblocking::reconcile::{Fix, Listing, Remote, Series};

pub struct GoogleRemote<'a> {
    token: String,
    settings: &'a CalendarSettings,
    calendar: Calendar,
    /// The task series a fix changed, so the caches can fetch them again.
    pub changed: Vec<String>,
}

impl<'a> GoogleRemote<'a> {
    pub fn new(token: String, settings: &'a CalendarSettings, calendar_id: &str) -> Self {
        Self {
            token,
            settings,
            calendar: Calendar {
                id: calendar_id.to_owned(),
                name: CalendarSettings::APP_CALENDAR_NAME.to_owned(),
                color: colors::DEFAULT_EVENT,
                primary: false,
                selected_by_google: true,
            },
            changed: Vec::new(),
        }
    }

    fn event_url(&self, calendar: &str, id: &str) -> String {
        format!(
            "{EVENTS}/{}/events/{}",
            urlencoding::encode(calendar),
            urlencoding::encode(id)
        )
    }

    fn note(&mut self, series: &str) {
        if !self.changed.iter().any(|known| known == series) {
            self.changed.push(series.to_owned());
        }
    }

    /// An event's JSON, or `None` when it was deleted.
    fn fetch(&self, calendar: &str, id: &str) -> Result<Option<serde_json::Value>, String> {
        match get::<serde_json::Value>(&self.event_url(calendar, id), &self.token) {
            Ok(value) if value["status"].as_str() == Some("cancelled") => Ok(None),
            Ok(value) => Ok(Some(value)),
            Err(error) if matches!(error.status, Some(404 | 410)) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    /// The occurrences of a series, oldest first, from `from` on and at most
    /// `limit` pages of them.
    fn instances(
        &self,
        calendar: &str,
        series: &str,
        from: Option<DateTime<Local>>,
        until: Option<DateTime<Local>>,
        limit: usize,
    ) -> Result<Vec<Event>, String> {
        let mut events = Vec::new();
        let mut page: Option<String> = None;
        let wide = (
            NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(9999, 1, 1).unwrap(),
        );
        for _ in 0..limit {
            let mut url = format!(
                "{}/instances?maxResults=250",
                self.event_url(calendar, series)
            );
            if let Some(from) = from {
                url.push_str(&format!(
                    "&timeMin={}",
                    urlencoding::encode(&from.with_timezone(&Utc).to_rfc3339())
                ));
            }
            if let Some(until) = until {
                url.push_str(&format!(
                    "&timeMax={}",
                    urlencoding::encode(&until.with_timezone(&Utc).to_rfc3339())
                ));
            }
            if let Some(token) = &page {
                url.push_str("&pageToken=");
                url.push_str(&urlencoding::encode(token));
            }
            let list: EventList = match get(&url, &self.token) {
                Ok(list) => list,
                Err(error) if matches!(error.status, Some(404 | 410)) => return Ok(events),
                Err(error) => return Err(error.to_string()),
            };
            for entry in list.items {
                if entry.status.as_deref() == Some("cancelled") {
                    continue;
                }
                if let Some(event) = parse_event_in_range(entry, &self.calendar, wide.0, wide.1) {
                    events.push(event);
                }
            }
            match list.next_page_token {
                Some(next) => page = Some(next),
                None => break,
            }
        }
        events.sort_by_key(crate::timeblocking::original_start);
        Ok(events)
    }

    fn time(&self, when: DateTime<Local>) -> serde_json::Value {
        let zone = iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into());
        serde_json::json!({"dateTime": when.to_rfc3339(), "timeZone": zone, "date": null})
    }

    /// Makes the occurrences of a new series on `skip` days deleted and on
    /// `done` days done, as they were in the series it continues.
    fn carry(
        &mut self,
        calendar: &str,
        series: &str,
        skip: &[NaiveDate],
        done: &[NaiveDate],
    ) -> Result<(), String> {
        let (Some(first), Some(last)) = (
            skip.iter().chain(done).min().copied(),
            skip.iter().chain(done).max().copied(),
        ) else {
            return Ok(());
        };
        let hour = self.settings.day_start_hour();
        let from = crate::calendar::at_clock(first, hour, 0) - chrono::Duration::days(1);
        let until = crate::calendar::at_clock(last, hour, 0) + chrono::Duration::days(2);
        for occurrence in self.instances(calendar, series, Some(from), Some(until), 20)? {
            let day = planning_day(crate::timeblocking::original_start(&occurrence), hour);
            let url = self.event_url(calendar, &occurrence.id);
            if skip.contains(&day) {
                delete(&url, &self.token, "Time blocks").map_err(String::from)?;
            } else if done.contains(&day) {
                let mut properties = self
                    .fetch(calendar, &occurrence.id)?
                    .map(|value| value["extendedProperties"].clone())
                    .unwrap_or_else(|| serde_json::json!({}));
                properties["private"]["ttCompleted"] = "true".into();
                patch(
                    &url,
                    &self.token,
                    &serde_json::json!({"extendedProperties": properties}).to_string(),
                    "Time blocks",
                )
                .map_err(String::from)?;
            }
        }
        Ok(())
    }
}

impl Remote for GoogleRemote<'_> {
    fn listing(
        &mut self,
        calendar: &str,
        first: DateTime<Local>,
        last: DateTime<Local>,
    ) -> Result<Listing, String> {
        let mut listing = Listing::default();
        let mut page: Option<String> = None;
        loop {
            let mut url = format!(
                "{EVENTS}/{}/events?singleEvents=true&showDeleted=true&maxResults=2500&timeMin={}&timeMax={}",
                urlencoding::encode(calendar),
                urlencoding::encode(&first.with_timezone(&Utc).to_rfc3339()),
                urlencoding::encode(&last.with_timezone(&Utc).to_rfc3339()),
            );
            if let Some(token) = &page {
                url.push_str("&pageToken=");
                url.push_str(&urlencoding::encode(token));
            }
            let list: EventList = get(&url, &self.token).map_err(String::from)?;
            for entry in list.items {
                if entry.status.as_deref() == Some("cancelled") {
                    if let (Some(series), Some(original)) = (
                        entry.recurring_event_id.clone(),
                        entry.original_start_time.as_ref().and_then(event_time),
                    ) {
                        listing.cancelled.push((series, original));
                    }
                    continue;
                }
                if let Some(event) = parse_event_in_range(
                    entry,
                    &self.calendar,
                    first.date_naive(),
                    last.date_naive(),
                ) {
                    listing.events.push(event);
                }
            }
            match list.next_page_token {
                Some(next) => page = Some(next),
                None => break,
            }
        }
        Ok(listing)
    }

    fn series(&mut self, calendar: &str, id: &str) -> Result<Option<Series>, String> {
        let Some(value) = self.fetch(calendar, id)? else {
            return Ok(None);
        };
        let recurrence: Vec<String> =
            serde_json::from_value(value["recurrence"].clone()).unwrap_or_default();
        let properties = &value["extendedProperties"];
        Ok(Some(Series {
            calendar: calendar.to_owned(),
            id: id.to_owned(),
            recurrence,
            start: json_event_start(&value["start"])?,
            end: json_event_start(&value["end"])?,
            all_day: value["start"]["date"].is_string(),
            split_from: split_from(properties),
            task: (properties["private"]["ttKind"].as_str() == Some("task"))
                .then(|| crate::calendar::calendar_tasks::read_data(properties).ok())
                .flatten(),
        }))
    }

    fn first_occurrence(
        &mut self,
        calendar: &str,
        series: &str,
        from: DateTime<Local>,
    ) -> Result<Option<Event>, String> {
        // Google filters occurrences by when they are now, not when they
        // were first scheduled, so look a little earlier.
        let earlier = from - chrono::Duration::days(1);
        Ok(self
            .instances(calendar, series, Some(earlier), None, 2)?
            .into_iter()
            .find(|event| crate::timeblocking::original_start(event) >= from))
    }

    fn last_occurrence(&mut self, calendar: &str, series: &str) -> Result<Option<Event>, String> {
        Ok(self.instances(calendar, series, None, None, 40)?.pop())
    }

    fn exists(&mut self, calendar: &str, id: &str) -> Result<bool, String> {
        Ok(self.fetch(calendar, id)?.is_some())
    }

    fn apply(&mut self, fix: &Fix) -> Result<(), String> {
        match fix {
            Fix::Rule {
                calendar,
                series,
                recurrence,
                start,
                end,
            } => {
                let body = serde_json::json!({
                    "recurrence": recurrence,
                    "start": self.time(*start),
                    "end": self.time(*end),
                });
                patch(
                    &self.event_url(calendar, series),
                    &self.token,
                    &body.to_string(),
                    "Time blocks",
                )
                .map_err(String::from)?;
                self.note(series);
            }
            Fix::Continue {
                calendar,
                from,
                block_series,
                recurrence,
                start,
                end,
                skip,
                done,
            } => {
                let mut copy = self
                    .fetch(calendar, from)?
                    .ok_or("The repeating task to carry on was deleted")?;
                for key in [
                    "id",
                    "etag",
                    "kind",
                    "htmlLink",
                    "created",
                    "updated",
                    "iCalUID",
                    "sequence",
                    "organizer",
                    "creator",
                    "recurringEventId",
                    "originalStartTime",
                    "status",
                ] {
                    copy.as_object_mut().map(|object| object.remove(key));
                }
                copy["recurrence"] = serde_json::json!(recurrence);
                copy["start"] = self.time(*start);
                copy["end"] = self.time(*end);
                let mut data =
                    crate::calendar::calendar_tasks::read_data(&copy["extendedProperties"])?;
                data.metadata.block_series = Some(block_series.clone());
                data.metadata.block = None;
                crate::calendar::calendar_tasks::write_data(&mut copy, &data)?;
                let response = post(
                    &format!("{EVENTS}/{}/events", urlencoding::encode(calendar)),
                    &self.token,
                    &copy.to_string(),
                    "Time blocks",
                )
                .map_err(String::from)?;
                let created: serde_json::Value =
                    serde_json::from_str(&response).map_err(|error| error.to_string())?;
                let id = created["id"]
                    .as_str()
                    .ok_or("Google did not return the new repeating task")?
                    .to_owned();
                self.carry(calendar, &id, skip, done)?;
                self.note(from);
                self.note(&id);
            }
            Fix::Delete { calendar, series } => {
                match delete(
                    &self.event_url(calendar, series),
                    &self.token,
                    "Time blocks",
                ) {
                    Err(error) if !matches!(error.status, Some(404 | 410)) => {
                        return Err(error.to_string());
                    }
                    _ => {}
                }
                self.note(series);
            }
            Fix::Move {
                calendar,
                event,
                start,
                end,
            } => {
                let body = serde_json::json!({"start": self.time(*start), "end": self.time(*end)});
                patch(
                    &self.event_url(calendar, event),
                    &self.token,
                    &body.to_string(),
                    "Time blocks",
                )
                .map_err(String::from)?;
                if let Some((series, _)) = parse_instance_id(event) {
                    self.note(series);
                }
            }
            Fix::Cancel { calendar, event } => {
                match delete(&self.event_url(calendar, event), &self.token, "Time blocks") {
                    Err(error) if !matches!(error.status, Some(404 | 410)) => {
                        return Err(error.to_string());
                    }
                    _ => {}
                }
                if let Some((series, _)) = parse_instance_id(event) {
                    self.note(series);
                }
            }
            Fix::Release { task } => {
                let order = crate::timeblocking::metadata(task).order;
                task_edit::save(
                    (**task).clone(),
                    self.settings,
                    Change::Unassign,
                    order,
                    task_edit::google_access,
                )
                .map_err(String::from)?;
            }
        }
        Ok(())
    }
}
