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
            calendar_id: CALENDAR.into(),
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
        let id = self.id("block");
        self.series.push(FakeSeries {
            master: Series {
                calendar: CALENDAR.into(),
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
        let rule = self.find(block).unwrap().master.recurrence.clone();
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
                        block_series: Some(format!("{CALENDAR}/{block}")),
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

    /// Delete "this and following" from `day`.
    pub fn delete_following(&mut self, series: &str, day: NaiveDate) {
        let master = &mut self.find_mut(series).unwrap().master;
        let last = day.pred_opt().unwrap();
        let until = (calendar::at_clock(day, 0, 0) - Duration::seconds(1))
            .with_timezone(&Utc)
            .format("%Y%m%dT%H%M%SZ");
        let _ = last;
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
        assert_eq!(calendar, CALENDAR);
        let mut listing = Listing::default();
        for series in self.series.iter().filter(|s| !s.cancelled) {
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
                .filter(|e| e.end > first && e.start < last)
                .cloned(),
        );
        listing.events.sort_by_key(|e| (e.start, e.id.clone()));
        Ok(listing)
    }

    fn series(&mut self, _calendar: &str, id: &str) -> Result<Option<Series>, String> {
        Ok(self.find(id).map(|s| s.master.clone()))
    }

    fn first_occurrence(
        &mut self,
        _calendar: &str,
        series: &str,
        from: DateTime<Local>,
    ) -> Result<Option<Event>, String> {
        let Some(fake) = self.find(series) else {
            return Ok(None);
        };
        Ok(expand(&fake.master, horizon())
            .into_iter()
            .filter(|start| *start >= from)
            .find_map(|start| self.occurrence(fake, start)))
    }

    fn last_occurrence(&mut self, _calendar: &str, series: &str) -> Result<Option<Event>, String> {
        let Some(fake) = self.find(series) else {
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
