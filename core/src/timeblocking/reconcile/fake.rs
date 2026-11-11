//! An in-memory Google Calendar for testing the reconciler.
//!
//! It behaves like Google where that matters to time blocks:
//!
//! - A series is expanded from its `RRULE` (daily or weekly, with
//!   `INTERVAL`, `BYDAY`, `UNTIL` and `COUNT`).
//! - Each occurrence's ID is the series ID, `_`, and its original start in
//!   UTC, so retiming a series gives every occurrence a new ID.
//! - Editing or deleting one occurrence makes an exception keyed by its
//!   original start; an exception the rule no longer makes is orphaned.
//! - Google's own "this and following" split names the new series
//!   `old_RYYYYMMDDTHHMMSS`; this app's records `ttSplitFrom` instead.
//! - Every occurrence carries its series' properties (task metadata
//!   included) unless that occurrence was edited on its own.

use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Timelike, Utc, Weekday};

use super::{Fix, Listing, Remote, Series};
use crate::calendar::{self, Details, Event};
use crate::task_db::TaskData;
use crate::timeblocking::Metadata;

pub const CALENDAR: &str = "app";

#[derive(Debug, Clone, Default)]
pub struct Exception {
    pub start: Option<DateTime<Local>>,
    pub end: Option<DateTime<Local>>,
    pub cancelled: bool,
    pub completed: bool,
    pub task: Option<TaskData>,
}

#[derive(Debug, Clone)]
pub struct FakeSeries {
    pub master: Series,
    pub title: String,
    pub cancelled: bool,
    pub exceptions: BTreeMap<DateTime<Local>, Exception>,
}

#[derive(Debug, Clone, Default)]
pub struct FakeGoogle {
    pub series: Vec<FakeSeries>,
    /// One-off events: blocks and tasks that do not repeat.
    pub singles: Vec<Event>,
    pub next_id: u32,
    pub writes: usize,
}

pub fn stamp(when: DateTime<Local>) -> String {
    when.with_timezone(&Utc)
        .format("%Y%m%dT%H%M%SZ")
        .to_string()
}

fn weekday(code: &str) -> Option<Weekday> {
    Some(match code {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

/// The original starts a series' rule makes, up to `horizon`.
fn expand(master: &Series, horizon: NaiveDate) -> Vec<DateTime<Local>> {
    let Some(parts) = super::rule_parts(&master.recurrence) else {
        return vec![master.start];
    };
    let get = |key: &str| {
        parts
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    };
    let interval: i64 = get("INTERVAL").and_then(|v| v.parse().ok()).unwrap_or(1);
    let count: Option<usize> = get("COUNT").and_then(|v| v.parse().ok());
    let until: Option<DateTime<Local>> = get("UNTIL").and_then(|v| {
        if v.len() == 8 {
            NaiveDate::parse_from_str(v, "%Y%m%d")
                .ok()
                .map(|d| calendar::at_clock(d, 23, 59))
        } else {
            chrono::NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%SZ")
                .ok()
                .map(|n| n.and_utc().with_timezone(&Local))
        }
    });
    let freq = get("FREQ").unwrap_or("DAILY");
    let first = master.start.date_naive();
    let days: Vec<Weekday> = get("BYDAY")
        .map(|v| v.split(',').filter_map(weekday).collect())
        .unwrap_or_else(|| vec![first.weekday()]);
    let week_of = |d: NaiveDate| d - Duration::days(d.weekday().num_days_from_monday() as i64);
    let (hour, minute) = (master.start.hour(), master.start.minute());
    let mut starts = Vec::new();
    let mut day = first;
    while day <= horizon {
        let matches = day == first
            || match freq {
                "DAILY" => (day - first).num_days() % interval == 0,
                "WEEKLY" => {
                    ((week_of(day) - week_of(first)).num_days() / 7) % interval == 0
                        && days.contains(&day.weekday())
                }
                _ => false,
            };
        if matches {
            let start = calendar::at_clock(day, hour, minute);
            if until.is_some_and(|until| start > until) {
                break;
            }
            starts.push(start);
            if count.is_some_and(|count| starts.len() >= count) {
                break;
            }
        }
        day = day.succ_opt().unwrap();
    }
    starts
}

fn horizon() -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, 6, 30).unwrap()
}

impl FakeGoogle {
    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    fn find(&self, id: &str) -> Option<&FakeSeries> {
        self.series
            .iter()
            .find(|s| s.master.id == id && !s.cancelled)
    }

    fn find_on(&self, calendar: &str, id: &str) -> Option<&FakeSeries> {
        self.find(id).filter(|s| s.master.calendar == calendar)
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut FakeSeries> {
        self.series
            .iter_mut()
            .find(|s| s.master.id == id && !s.cancelled)
    }

    /// One occurrence as Google would list it, unless it was deleted.
    fn occurrence(&self, series: &FakeSeries, original: DateTime<Local>) -> Option<Event> {
        let exception = series
            .exceptions
            .get(&original)
            .cloned()
            .unwrap_or_default();
        if exception.cancelled {
            return None;
        }
        let length = series.master.end - series.master.start;
        let start = exception.start.unwrap_or(original);
        Some(Event {
            id: format!("{}_{}", series.master.id, stamp(original)),
            calendar_id: series.master.calendar.clone(),
            title: series.title.clone(),
            start,
            end: exception.end.unwrap_or(start + length),
            all_day: series.master.all_day,
            color: calendar::colors::TASK,
            completed: exception.completed,
            details: Details {
                recurring_event_id: Some(series.master.id.clone()),
                original_start: Some(original),
                split_from: series.master.split_from.clone(),
                task_data: exception.task.or_else(|| series.master.task.clone()),
                ..Default::default()
            },
        })
    }

    fn instance(&self, id: &str) -> Option<(usize, DateTime<Local>)> {
        let (series, start) = crate::timeblocking::membership::parse_instance_id(id)?;
        let index = self
            .series
            .iter()
            .position(|s| s.master.id == series && !s.cancelled)?;
        expand(&self.series[index].master, horizon())
            .contains(&start)
            .then_some((index, start))
    }

    // What a person does, in the app or on Google's site.

    pub fn add_block_series(&mut self, start: DateTime<Local>, minutes: i64, rule: &str) -> String {
        self.add_block_series_on(CALENDAR, start, minutes, rule)
    }

    /// A repeating block on a calendar of its own.
    pub fn add_block_series_on(
        &mut self,
        calendar: &str,
        start: DateTime<Local>,
        minutes: i64,
        rule: &str,
    ) -> String {
        let id = self.id("block");
        self.series.push(FakeSeries {
            master: Series {
                calendar: calendar.into(),
                id: id.clone(),
                recurrence: vec![format!("RRULE:{rule}")],
                start,
                end: start + Duration::minutes(minutes),
                all_day: false,
                split_from: Vec::new(),
                task: None,
            },
            title: "Block".into(),
            cancelled: false,
            exceptions: BTreeMap::new(),
        });
        id
    }

    pub fn add_block(&mut self, start: DateTime<Local>, minutes: i64) -> String {
        let id = self.id("single");
        self.singles.push(Event {
            id: id.clone(),
            calendar_id: CALENDAR.into(),
            title: "One block".into(),
            start,
            end: start + Duration::minutes(minutes),
            all_day: false,
            color: calendar::colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        });
        id
    }

    /// The occurrence of `series` first scheduled on `day`.
    pub fn occurrence_on(&self, series: &str, day: NaiveDate) -> Event {
        let fake = self.find(series).expect("series");
        let original = expand(&fake.master, horizon())
            .into_iter()
            .find(|start| start.date_naive() == day)
            .expect("an occurrence that day");
        self.occurrence(fake, original).expect("not deleted")
    }

    /// Repeats a new task with `block`'s series from its occurrence on
    /// `from`, as joining a block series does: the block's rule from there
    /// on, at the block occurrence's start.
    pub fn add_follower(
        &mut self,
        block: &str,
        from: NaiveDate,
        title: &str,
        order: i64,
        minutes: u32,
    ) -> String {
        let occurrence = self.occurrence_on(block, from);
        let block_master = self.find(block).unwrap().master.clone();
        let rule = block_master.recurrence.clone();
        let id = self.id("task");
        let task_id = 1000 + i64::from(self.next_id);
        self.series.push(FakeSeries {
            master: Series {
                calendar: CALENDAR.into(),
                id: id.clone(),
                recurrence: rule,
                start: occurrence.start,
                end: occurrence.start + Duration::minutes(i64::from(minutes.max(1))),
                all_day: false,
                split_from: Vec::new(),
                task: Some(TaskData {
                    id: task_id,
                    metadata: Metadata {
                        block: Some(crate::timeblocking::block_key(&occurrence)),
                        block_series: Some(format!("{}/{block}", block_master.calendar)),
                        order,
                        minutes,
                        ..Default::default()
                    },
                }),
            },
            title: title.into(),
            cancelled: false,
            exceptions: BTreeMap::new(),
        });
        id
    }

    /// A task put in one block occurrence only.
    pub fn add_one_off(&mut self, block: &Event, title: &str, order: i64, minutes: u32) -> String {
        let id = self.id("oneoff");
        let task_id = 1000 + i64::from(self.next_id);
        self.singles.push(Event {
            id: id.clone(),
            calendar_id: CALENDAR.into(),
            title: title.into(),
            start: block.start,
            end: block.start + Duration::minutes(i64::from(minutes.max(1))),
            all_day: false,
            color: calendar::colors::TASK,
            completed: false,
            details: Details {
                task_data: Some(TaskData {
                    id: task_id,
                    metadata: Metadata {
                        block: Some(crate::timeblocking::block_key(block)),
                        order,
                        minutes,
                        ..Default::default()
                    },
                }),
                ..Default::default()
            },
        });
        id
    }

    fn exception_on(&mut self, series: &str, day: NaiveDate) -> &mut Exception {
        let original = {
            let fake = self.find(series).expect("series");
            expand(&fake.master, horizon())
                .into_iter()
                .find(|start| start.date_naive() == day)
                .expect("an occurrence that day")
        };
        self.find_mut(series)
            .unwrap()
            .exceptions
            .entry(original)
            .or_default()
    }

    /// A task on its own on `day`, all day, on the app calendar.
    pub fn add_calendar_task(&mut self, day: NaiveDate, title: &str, minutes: u32) -> Event {
        let id = self.id("ttsingle");
        let task_id = 1000 + i64::from(self.next_id);
        let start = calendar::at_clock(day, 0, 0);
        let task = Event {
            id,
            calendar_id: CALENDAR.into(),
            title: title.into(),
            start,
            end: start + Duration::days(1),
            all_day: true,
            color: calendar::colors::TASK,
            completed: false,
            details: Details {
                task_data: Some(TaskData {
                    id: task_id,
                    metadata: Metadata {
                        minutes,
                        ..Default::default()
                    },
                }),
                ..Default::default()
            },
        };
        self.singles.push(task.clone());
        task
    }

    /// Edit one occurrence's time.
    pub fn move_occurrence(&mut self, series: &str, day: NaiveDate, start: DateTime<Local>) {
        let length = {
            let master = &self.find(series).unwrap().master;
            master.end - master.start
        };
        let exception = self.exception_on(series, day);
        exception.start = Some(start);
        exception.end = Some(start + length);
    }

    /// Delete one occurrence.
    pub fn delete_occurrence(&mut self, series: &str, day: NaiveDate) {
        self.exception_on(series, day).cancelled = true;
    }

    /// Mark one task occurrence done.
    pub fn complete(&mut self, series: &str, day: NaiveDate) {
        self.exception_on(series, day).completed = true;
    }

    /// Edit "all events": the series' time, keeping its first day.
    pub fn retime_series(&mut self, series: &str, hour: u32, minute: u32) {
        let master = &mut self.find_mut(series).unwrap().master;
        let length = master.end - master.start;
        master.start = calendar::at_clock(master.start.date_naive(), hour, minute);
        master.end = master.start + length;
    }

    /// Edit "this and following" from `day`: the series ends the day
    /// before and a new one starting at `start` carries on. `google` makes
    /// the split as Google's site does; otherwise as this app does.
    pub fn split(
        &mut self,
        series: &str,
        day: NaiveDate,
        start: DateTime<Local>,
        rule: Option<&str>,
        google: bool,
    ) -> String {
        let old = self.find(series).unwrap().clone();
        let id = if google {
            format!(
                "{series}_R{}",
                calendar::at_clock(day, 0, 0)
                    .with_timezone(&Utc)
                    .format("%Y%m%dT%H%M%S")
            )
        } else {
            self.id("block")
        };
        self.delete_following(series, day);
        let mut master = old.master.clone();
        master.id = id.clone();
        master.start = start;
        master.end = start + (old.master.end - old.master.start);
        if let Some(rule) = rule {
            master.recurrence = vec![format!("RRULE:{rule}")];
        }
        if !google {
            master.split_from = crate::timeblocking::membership::series_lineage(
                CALENDAR,
                series,
                &old.master.split_from,
            );
        }
        self.series.push(FakeSeries {
            master,
            title: old.title,
            cancelled: false,
            exceptions: BTreeMap::new(),
        });
        id
    }

    /// Delete "this and following" from `day`. As this app and Google do,
    /// the series' `UNTIL` is set a second before the first occurrence it
    /// drops.
    pub fn delete_following(&mut self, series: &str, day: NaiveDate) {
        let master = &mut self.find_mut(series).unwrap().master;
        let boundary = calendar::at_clock(day, master.start.hour(), master.start.minute());
        let until = (boundary - Duration::seconds(1))
            .with_timezone(&Utc)
            .format("%Y%m%dT%H%M%SZ");
        let rule = master.recurrence[0]
            .trim_start_matches("RRULE:")
            .split(';')
            .filter(|part| !part.starts_with("UNTIL=") && !part.starts_with("COUNT="))
            .collect::<Vec<_>>()
            .join(";");
        master.recurrence = vec![format!("RRULE:{rule};UNTIL={until}")];
    }

    /// Delete "all events".
    pub fn delete_series(&mut self, series: &str) {
        self.find_mut(series).unwrap().cancelled = true;
    }

    /// Every occurrence of a series, as listed.
    pub fn occurrences(&self, series: &str) -> Vec<Event> {
        let Some(fake) = self.find(series) else {
            return Vec::new();
        };
        expand(&fake.master, horizon())
            .into_iter()
            .filter_map(|start| self.occurrence(fake, start))
            .collect()
    }

    pub fn single(&self, id: &str) -> &Event {
        self.singles.iter().find(|e| e.id == id).unwrap()
    }

    /// The series following `block` that `task_id` has, by start.
    pub fn followers_of(&self, task_id: i64) -> Vec<&FakeSeries> {
        let mut found: Vec<&FakeSeries> = self
            .series
            .iter()
            .filter(|s| !s.cancelled && s.master.task.as_ref().is_some_and(|t| t.id == task_id))
            .collect();
        found.sort_by_key(|s| s.master.start);
        found
    }

    pub fn task_id(&self, series: &str) -> i64 {
        self.find(series).unwrap().master.task.as_ref().unwrap().id
    }
}

impl Remote for FakeGoogle {
    fn listing(
        &mut self,
        calendar: &str,
        first: DateTime<Local>,
        last: DateTime<Local>,
    ) -> Result<Listing, String> {
        let mut listing = Listing::default();
        for series in self
            .series
            .iter()
            .filter(|s| !s.cancelled && s.master.calendar == calendar)
        {
            for original in expand(&series.master, horizon()) {
                match self.occurrence(series, original) {
                    Some(event) if event.end > first && event.start < last => {
                        listing.events.push(event)
                    }
                    Some(_) => {}
                    None if original >= first && original < last => {
                        listing.cancelled.push((series.master.id.clone(), original))
                    }
                    None => {}
                }
            }
        }
        listing.events.extend(
            self.singles
                .iter()
                .filter(|e| e.calendar_id == calendar && e.end > first && e.start < last)
                .cloned(),
        );
        listing.events.sort_by_key(|e| (e.start, e.id.clone()));
        Ok(listing)
    }

    fn series(&mut self, calendar: &str, id: &str) -> Result<Option<Series>, String> {
        Ok(self.find_on(calendar, id).map(|s| s.master.clone()))
    }

    fn first_occurrence(
        &mut self,
        calendar: &str,
        series: &str,
        from: DateTime<Local>,
    ) -> Result<Option<Event>, String> {
        let Some(fake) = self.find_on(calendar, series) else {
            return Ok(None);
        };
        Ok(expand(&fake.master, horizon())
            .into_iter()
            .filter(|start| *start >= from)
            .find_map(|start| self.occurrence(fake, start)))
    }

    fn last_occurrence(&mut self, calendar: &str, series: &str) -> Result<Option<Event>, String> {
        let Some(fake) = self.find_on(calendar, series) else {
            return Ok(None);
        };
        Ok(expand(&fake.master, horizon())
            .into_iter()
            .rev()
            .find_map(|start| self.occurrence(fake, start)))
    }

    fn exists(&mut self, _calendar: &str, id: &str) -> Result<bool, String> {
        if self.singles.iter().any(|e| e.id == id) {
            return Ok(true);
        }
        Ok(match self.instance(id) {
            Some((index, start)) => self.occurrence(&self.series[index], start).is_some(),
            None => false,
        })
    }

    fn apply(&mut self, fix: &Fix) -> Result<(), String> {
        self.writes += 1;
        match fix {
            Fix::Rule {
                series,
                recurrence,
                start,
                end,
                ..
            } => {
                let master = &mut self.find_mut(series).ok_or("no such series")?.master;
                master.recurrence = recurrence.clone();
                master.start = *start;
                master.end = *end;
            }
            Fix::Continue {
                from,
                block_series,
                recurrence,
                start,
                end,
                skip,
                done,
                ..
            } => {
                let template = self.find(from).ok_or("no such series")?.clone();
                let id = self.id("task");
                let mut master = template.master.clone();
                master.id = id;
                master.recurrence = recurrence.clone();
                master.start = *start;
                master.end = *end;
                if let Some(task) = &mut master.task {
                    task.metadata.block_series = Some(block_series.clone());
                }
                let mut exceptions = BTreeMap::new();
                for original in expand(&master, horizon()) {
                    let day = original.date_naive();
                    if skip.contains(&day) || done.contains(&day) {
                        exceptions.insert(
                            original,
                            Exception {
                                cancelled: skip.contains(&day),
                                completed: done.contains(&day),
                                ..Default::default()
                            },
                        );
                    }
                }
                self.series.push(FakeSeries {
                    master,
                    title: template.title,
                    cancelled: false,
                    exceptions,
                });
            }
            Fix::Delete { series, .. } => {
                self.find_mut(series).ok_or("no such series")?.cancelled = true;
            }
            Fix::Move {
                event, start, end, ..
            } => {
                if let Some(single) = self.singles.iter_mut().find(|e| e.id == *event) {
                    single.start = *start;
                    single.end = *end;
                    single.all_day = false;
                } else {
                    let (index, original) = self.instance(event).ok_or("no such event")?;
                    let exception = self.series[index].exceptions.entry(original).or_default();
                    exception.start = Some(*start);
                    exception.end = Some(*end);
                }
            }
            Fix::Cancel { event, .. } => {
                let (index, original) = self.instance(event).ok_or("no such occurrence")?;
                self.series[index]
                    .exceptions
                    .entry(original)
                    .or_default()
                    .cancelled = true;
            }
            Fix::Release { task } => {
                let single = self
                    .singles
                    .iter_mut()
                    .find(|e| e.id == task.id)
                    .ok_or("no such task")?;
                let day = single.start.date_naive();
                single.all_day = true;
                single.start = calendar::at_clock(day, 0, 0);
                single.end = single.start + Duration::days(1);
                if let Some(data) = &mut single.details.task_data {
                    data.metadata.block = None;
                }
            }
        }
        Ok(())
    }
}

/// The same Google, answering Google Calendar's REST API, so the code that
/// talks to the real one ([`crate::calendar::api::block_sync`]) can be run
/// against it. Only what the reconciler uses is answered.
impl FakeGoogle {
    pub fn answer(
        &mut self,
        method: &str,
        url: &str,
        body: Option<&str>,
    ) -> Result<String, (u16, String)> {
        use serde_json::Value;
        let rest = url
            .strip_prefix(crate::calendar::api::EVENTS)
            .and_then(|rest| rest.strip_prefix('/'))
            .ok_or((400, format!("not a calendar URL: {url}")))?;
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let decode = |text: &str| urlencoding::decode(text).unwrap().into_owned();
        let query: std::collections::HashMap<String, String> = query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(key, value)| (key.to_owned(), decode(value)))
            .collect();
        let time = |key: &str| {
            query.get(key).map(|value| {
                DateTime::parse_from_rfc3339(value)
                    .unwrap()
                    .with_timezone(&Local)
            })
        };
        let parts: Vec<String> = path.split('/').map(decode).collect();
        let body: Option<Value> = body.map(|body| serde_json::from_str(body).unwrap());
        let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
        let reply = |value: Value| Ok(value.to_string());
        match (method, parts.as_slice()) {
            ("GET", [calendar, "events"]) => {
                let first = time("timeMin").unwrap();
                let last = time("timeMax").unwrap();
                let listing = self.listing(calendar, first, last).unwrap();
                let mut items: Vec<Value> = listing.events.iter().map(event_json).collect();
                for (series, original) in listing.cancelled {
                    items.push(serde_json::json!({
                        "id": format!("{series}_{}", stamp(original)),
                        "status": "cancelled",
                        "recurringEventId": series,
                        "originalStartTime": {"dateTime": original.to_rfc3339()},
                    }));
                }
                reply(serde_json::json!({ "items": items }))
            }
            ("GET", [calendar, "events", id]) => {
                if let Some(series) = self
                    .series
                    .iter()
                    .find(|s| s.master.id == *id && s.master.calendar == *calendar)
                {
                    return reply(if series.cancelled {
                        serde_json::json!({"id": id, "status": "cancelled"})
                    } else {
                        master_json(series)
                    });
                }
                if let Some(single) = self.singles.iter().find(|e| e.id == *id) {
                    return reply(event_json(single));
                }
                match self.instance(id) {
                    Some((index, original)) => {
                        reply(match self.occurrence(&self.series[index], original) {
                            Some(event) => event_json(&event),
                            None => serde_json::json!({"id": id, "status": "cancelled"}),
                        })
                    }
                    None => Err((404, "Not Found".into())),
                }
            }
            ("GET", [calendar, "events", id, "instances"]) => {
                let fake = self
                    .find_on(calendar, id)
                    .ok_or((404, "Not Found".to_owned()))?;
                let (first, last) = (time("timeMin"), time("timeMax"));
                let items: Vec<Value> = expand(&fake.master, horizon())
                    .into_iter()
                    .filter_map(|start| self.occurrence(fake, start))
                    .filter(|e| first.is_none_or(|first| e.end > first))
                    .filter(|e| last.is_none_or(|last| e.start < last))
                    .map(|e| event_json(&e))
                    .collect();
                reply(serde_json::json!({ "items": items }))
            }
            ("PATCH", [_, "events", id]) => {
                let body = body.unwrap();
                let start = rest_time(&body["start"]);
                let end = rest_time(&body["end"]);
                if let Some(fake) = self.find_mut(id) {
                    if let Some(lines) = body["recurrence"].as_array() {
                        fake.master.recurrence = lines
                            .iter()
                            .map(|line| line.as_str().unwrap().to_owned())
                            .collect();
                    }
                    if let (Some(start), Some(end)) = (start, end) {
                        fake.master.start = start;
                        fake.master.end = end;
                    }
                    return reply(Value::Null);
                }
                // Given a repeat rule, a one-off event becomes a series.
                let rule: Vec<String> =
                    serde_json::from_value(body["recurrence"].clone()).unwrap_or_default();
                if !rule.is_empty()
                    && let Some(index) = self.singles.iter().position(|e| e.id == *id)
                {
                    let single = self.singles.remove(index);
                    let properties = &body["extendedProperties"];
                    self.series.push(FakeSeries {
                        master: Series {
                            calendar: single.calendar_id.clone(),
                            id: single.id.clone(),
                            recurrence: rule,
                            start: start.unwrap_or(single.start),
                            end: end.unwrap_or(single.end),
                            all_day: false,
                            split_from: Vec::new(),
                            task: crate::calendar::calendar_tasks::read_data(properties)
                                .ok()
                                .or(single.details.task_data),
                        },
                        title: single.title,
                        cancelled: false,
                        exceptions: BTreeMap::new(),
                    });
                    return reply(Value::Null);
                }
                if let Some(single) = self.singles.iter_mut().find(|e| e.id == *id) {
                    if let (Some(start), Some(end)) = (start, end) {
                        single.start = start;
                        single.end = end;
                        single.all_day = body["start"]["date"].is_string();
                    }
                    if body["extendedProperties"]["private"]["ttDataCount"].is_string() {
                        single.details.task_data = Some(
                            crate::calendar::calendar_tasks::read_data(&body["extendedProperties"])
                                .unwrap(),
                        );
                    }
                    return reply(Value::Null);
                }
                let (index, original) = self.instance(id).ok_or((404, "Not Found".to_owned()))?;
                let exception = self.series[index].exceptions.entry(original).or_default();
                if let (Some(start), Some(end)) = (start, end) {
                    exception.start = Some(start);
                    exception.end = Some(end);
                }
                if let Some(done) = body["extendedProperties"]["private"]["ttCompleted"].as_str() {
                    exception.completed = done == "true";
                }
                reply(Value::Null)
            }
            ("POST", [calendar, "events"]) => {
                let body = body.unwrap();
                let id = body["id"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| self.id("posted"));
                if self.series.iter().any(|s| s.master.id == id)
                    || self.singles.iter().any(|e| e.id == id)
                {
                    return Err((409, "The requested identifier already exists.".into()));
                }
                let properties = &body["extendedProperties"];
                let master = Series {
                    calendar: (*calendar).to_owned(),
                    id: id.clone(),
                    recurrence: serde_json::from_value(body["recurrence"].clone())
                        .unwrap_or_default(),
                    start: rest_time(&body["start"]).unwrap(),
                    end: rest_time(&body["end"]).unwrap(),
                    all_day: false,
                    split_from: properties["private"]["ttSplitFrom"]
                        .as_str()
                        .map(|value| serde_json::from_str(value).unwrap())
                        .unwrap_or_default(),
                    task: crate::calendar::calendar_tasks::read_data(properties).ok(),
                };
                self.series.push(FakeSeries {
                    master,
                    title: body["summary"].as_str().unwrap_or_default().to_owned(),
                    cancelled: false,
                    exceptions: BTreeMap::new(),
                });
                reply(serde_json::json!({ "id": id }))
            }
            ("DELETE", [_, "events", id]) => {
                if let Some(fake) = self.find_mut(id) {
                    fake.cancelled = true;
                    return Ok(String::new());
                }
                if let Some(index) = self.singles.iter().position(|e| e.id == *id) {
                    self.singles.remove(index);
                    return Ok(String::new());
                }
                let (index, original) = self.instance(id).ok_or((404, "Not Found".to_owned()))?;
                let exception = self.series[index].exceptions.entry(original).or_default();
                if exception.cancelled {
                    return Err((410, "Resource has been deleted".into()));
                }
                exception.cancelled = true;
                Ok(String::new())
            }
            _ => Err((400, format!("the fake does not answer {method} {url}"))),
        }
    }
}

/// A start or end as Google writes it.
fn rest_time(value: &serde_json::Value) -> Option<DateTime<Local>> {
    if let Some(time) = value["dateTime"].as_str() {
        return Some(
            DateTime::parse_from_rfc3339(time)
                .ok()?
                .with_timezone(&Local),
        );
    }
    let date = NaiveDate::parse_from_str(value["date"].as_str()?, "%Y-%m-%d").ok()?;
    Some(calendar::at_clock(date, 0, 0))
}

/// An event or occurrence as Google lists it.
fn event_json(event: &Event) -> serde_json::Value {
    let time = |when: DateTime<Local>| {
        if event.all_day {
            serde_json::json!({"date": when.date_naive().to_string()})
        } else {
            serde_json::json!({"dateTime": when.to_rfc3339(), "timeZone": "UTC"})
        }
    };
    let mut value = serde_json::json!({
        "id": event.id,
        "status": "confirmed",
        "summary": event.title,
        "start": time(event.start),
        "end": time(event.end),
        "extendedProperties": {"private": {}},
    });
    if let Some(series) = &event.details.recurring_event_id {
        value["recurringEventId"] = series.clone().into();
        value["originalStartTime"] =
            serde_json::json!({"dateTime": event.details.original_start.unwrap().to_rfc3339()});
    }
    properties(
        &mut value,
        &event.details.split_from,
        event.details.task_data.as_ref(),
        event.completed,
    );
    value
}

/// A series' master event as Google gives it.
fn master_json(series: &FakeSeries) -> serde_json::Value {
    let master = &series.master;
    let mut value = serde_json::json!({
        "id": master.id,
        "status": "confirmed",
        "summary": series.title,
        "start": {"dateTime": master.start.to_rfc3339(), "timeZone": "UTC"},
        "end": {"dateTime": master.end.to_rfc3339(), "timeZone": "UTC"},
        "recurrence": master.recurrence,
        "extendedProperties": {"private": {}},
    });
    properties(&mut value, &master.split_from, master.task.as_ref(), false);
    value
}

fn properties(
    value: &mut serde_json::Value,
    split_from: &[String],
    task: Option<&TaskData>,
    completed: bool,
) {
    if !split_from.is_empty() {
        value["extendedProperties"]["private"]["ttSplitFrom"] =
            serde_json::to_string(split_from).unwrap().into();
    }
    if let Some(data) = task {
        value["extendedProperties"]["private"]["ttKind"] = "task".into();
        value["extendedProperties"]["private"]["ttCompleted"] = completed.to_string().into();
        crate::calendar::calendar_tasks::write_data(value, data).unwrap();
    }
}
