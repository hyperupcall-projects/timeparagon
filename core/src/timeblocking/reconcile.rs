//! Keeps Google in step with time blocks.
//!
//! A task that repeats with a block is its own Google series, so it shows in
//! Google Calendar, and the block is the source of truth for it: the task
//! series has the block's repeat rule, starts where the block starts plus its
//! place among the block's tasks, and has an occurrence only where the block
//! does. Rather than copy each change to a block across by hand, this module
//! looks at what Google holds and works out the writes that make it so again
//! ([`reconcile`]). Running it twice in a row makes no writes the second time.
//!
//! It also lays every block's tasks out end to end, and takes one-off tasks
//! out of block occurrences that have been deleted.
//!
//! What it leaves alone, on purpose:
//!
//! - Completed occurrences are history and never move or go away.
//! - An occurrence of a task series deleted on its own is a skipped day and
//!   is not put back.
//! - A task series that ends before its block does was ended on purpose and
//!   is not extended. (So extending a block series that had an end date
//!   leaves its tasks' end dates where they were.)

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Local, NaiveDate, TimeZone, Utc};

use super::membership::{
    block_of, is_follower, layout, original_start, planning_day, series_lineage, tasks_in,
};
use super::metadata;
use crate::calendar::{self, Event};
use crate::task_db::TaskData;

/// A series' master event as Google has it.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub calendar: String,
    pub id: String,
    /// Raw RFC 5545 lines: `RRULE:`, `EXDATE:`, `RDATE:`.
    pub recurrence: Vec<String>,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub all_day: bool,
    /// The series this one continues, as `calendar/series`.
    pub split_from: Vec<String>,
    /// A task series' TimeSlay ID and metadata.
    pub task: Option<TaskData>,
}

impl Series {
    fn name(&self) -> String {
        format!("{}/{}", self.calendar, self.id)
    }

    fn lineage(&self) -> Vec<String> {
        series_lineage(&self.calendar, &self.id, &self.split_from)
    }

    fn block_series(&self) -> Option<&str> {
        self.task.as_ref()?.metadata.block_series.as_deref()
    }

    /// The task's place among its block's tasks, as in [`membership::sort_key`].
    fn order(&self) -> (i64, &str) {
        (
            self.task.as_ref().map_or(0, |data| data.metadata.order),
            &self.id,
        )
    }

    fn minutes(&self) -> i64 {
        match self.task.as_ref().map_or(0, |data| data.metadata.minutes) {
            0 => 30,
            minutes => i64::from(minutes),
        }
    }
}

/// What Google holds on one calendar over a stretch of time.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// Every one-off event and occurrence, not counting deleted ones.
    pub events: Vec<Event>,
    /// Occurrences deleted on their own, as their series and when each was
    /// first scheduled.
    pub cancelled: Vec<(String, DateTime<Local>)>,
}

/// A write that brings Google back in step.
#[derive(Debug, Clone, PartialEq)]
pub enum Fix {
    /// Give a task series its block's repeat rule and time.
    Rule {
        calendar: String,
        series: String,
        recurrence: Vec<String>,
        start: DateTime<Local>,
        end: DateTime<Local>,
    },
    /// Carry a task series on with the block series that continues its own,
    /// as a new task series copied from `from`.
    Continue {
        calendar: String,
        from: String,
        block_series: String,
        recurrence: Vec<String>,
        start: DateTime<Local>,
        end: DateTime<Local>,
        /// Days the series being continued skipped, which stay skipped.
        skip: Vec<NaiveDate>,
        /// Days it was already done on, which stay done.
        done: Vec<NaiveDate>,
    },
    /// Delete a task series that no block occurrence is left for.
    Delete { calendar: String, series: String },
    /// Put one task at its place in its block.
    Move {
        calendar: String,
        event: String,
        start: DateTime<Local>,
        end: DateTime<Local>,
    },
    /// Delete one occurrence of a task series whose block occurrence was
    /// deleted.
    Cancel { calendar: String, event: String },
    /// Take a one-off task out of a block occurrence that was deleted.
    Release { task: Box<Event> },
}

impl Fix {
    /// The task series this changes, if it changes one.
    pub fn series(&self) -> Option<&str> {
        match self {
            Fix::Rule { series, .. } | Fix::Delete { series, .. } => Some(series),
            Fix::Continue { from, .. } => Some(from),
            _ => None,
        }
    }
}

/// Google, as far as the reconciler needs it.
pub trait Remote {
    /// What `calendar` holds between `first` and `last`, every series
    /// expanded into its occurrences.
    fn listing(
        &mut self,
        calendar: &str,
        first: DateTime<Local>,
        last: DateTime<Local>,
    ) -> Result<Listing, String>;
    /// A series' master, or `None` once it has been deleted.
    fn series(&mut self, calendar: &str, id: &str) -> Result<Option<Series>, String>;
    /// The first occurrence of a series first scheduled at or after `from`.
    fn first_occurrence(
        &mut self,
        calendar: &str,
        series: &str,
        from: DateTime<Local>,
    ) -> Result<Option<Event>, String>;
    /// The last occurrence of a series that has a repeat count.
    fn last_occurrence(&mut self, calendar: &str, series: &str) -> Result<Option<Event>, String>;
    /// Whether an event, or one occurrence, still exists.
    fn exists(&mut self, calendar: &str, id: &str) -> Result<bool, String>;
    fn apply(&mut self, fix: &Fix) -> Result<(), String>;
}

/// Brings `calendar` back in step between `first` and `last`, and returns
/// the writes it made. `today` is the planning day it is now: a task series
/// whose block series was deleted keeps its occurrences before it.
pub fn reconcile(
    remote: &mut impl Remote,
    calendar: &str,
    first: DateTime<Local>,
    last: DateTime<Local>,
    day_start_hour: u32,
    today: NaiveDate,
) -> Result<Vec<Fix>, String> {
    let mut applied = Vec::new();
    // Series changes first, since they change the occurrences; then the
    // occurrences. Each pass reads Google afresh.
    for _ in 0..6 {
        let listing = remote.listing(calendar, first, last)?;
        let mut fixes = series_fixes(remote, calendar, &listing, day_start_hour, today)?;
        if fixes.is_empty() {
            fixes = occurrence_fixes(remote, calendar, &listing, day_start_hour, today)?;
        }
        if fixes.is_empty() {
            return Ok(applied);
        }
        for fix in &fixes {
            remote.apply(fix)?;
        }
        applied.extend(fixes);
    }
    Err("Time blocks did not settle after several passes".into())
}

/// The parts of a series' one `RRULE:` line, in order.
fn rule_parts(recurrence: &[String]) -> Option<Vec<(String, String)>> {
    let mut rules = recurrence
        .iter()
        .filter_map(|line| line.strip_prefix("RRULE:"));
    let rule = rules.next()?;
    if rules.next().is_some() {
        return None;
    }
    Some(
        rule.split(';')
            .filter_map(|part| part.split_once('='))
            .map(|(k, v)| (k.to_ascii_uppercase(), v.to_owned()))
            .collect(),
    )
}

/// The repeat itself, without where it ends.
fn rule_shape(parts: &[(String, String)]) -> BTreeMap<String, String> {
    parts
        .iter()
        .filter(|(key, _)| key != "UNTIL" && key != "COUNT")
        .cloned()
        .collect()
}

/// The planning day an `UNTIL=` value ends on.
fn until_day(value: &str, day_start_hour: u32) -> Option<NaiveDate> {
    if value.len() == 8 {
        return NaiveDate::parse_from_str(value, "%Y%m%d").ok();
    }
    let naive = chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S"))
        .ok()?;
    let when = if value.ends_with('Z') {
        Utc.from_utc_datetime(&naive).with_timezone(&Local)
    } else {
        Local.from_local_datetime(&naive).earliest()?
    };
    Some(planning_day(when, day_start_hour))
}

/// The `UNTIL=` value that keeps every occurrence on `day` and none after.
fn until_value(day: NaiveDate, all_day: bool, day_start_hour: u32) -> String {
    if all_day {
        return day.format("%Y%m%d").to_string();
    }
    let next = day.succ_opt().unwrap_or(day);
    let end = calendar::at_clock(next, day_start_hour.min(23), 0) - Duration::seconds(1);
    end.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ").to_string()
}

/// The last planning day a series has an occurrence on, or `None` when it
/// goes on for ever.
fn last_day(
    remote: &mut impl Remote,
    series: &Series,
    day_start_hour: u32,
) -> Result<Option<NaiveDate>, String> {
    let Some(parts) = rule_parts(&series.recurrence) else {
        return Ok(None);
    };
    if let Some((_, until)) = parts.iter().find(|(key, _)| key == "UNTIL") {
        return Ok(until_day(until, day_start_hour));
    }
    if parts.iter().any(|(key, _)| key == "COUNT") {
        return Ok(remote
            .last_occurrence(&series.calendar, &series.id)?
            .map(|last| planning_day(original_start(&last), day_start_hour)));
    }
    Ok(None)
}

/// A block series and the planning days it covers.
struct Span {
    series: Series,
    first: NaiveDate,
    last: Option<NaiveDate>,
}

impl Span {
    fn covers(&self, day: NaiveDate) -> bool {
        self.first <= day && self.last.is_none_or(|last| day <= last)
    }
}

/// A task series and the block series it repeats with.
struct Follower {
    series: Series,
    first: NaiveDate,
    last: Option<NaiveDate>,
}

fn series_fixes(
    remote: &mut impl Remote,
    calendar: &str,
    listing: &Listing,
    day_start_hour: u32,
    today: NaiveDate,
) -> Result<Vec<Fix>, String> {
    // The task series seen repeating with a block, and the block series
    // seen at all.
    let mut follower_ids: Vec<String> = Vec::new();
    let mut block_ids: Vec<String> = Vec::new();
    for event in &listing.events {
        let Some(series) = &event.details.recurring_event_id else {
            continue;
        };
        if is_follower(event) {
            if !follower_ids.contains(series) {
                follower_ids.push(series.clone());
            }
        } else if !event.is_task() && !block_ids.contains(series) {
            block_ids.push(series.clone());
        }
    }
    let mut followers: Vec<Series> = Vec::new();
    for id in &follower_ids {
        if let Some(series) = remote.series(calendar, id)?
            && series.block_series().is_some()
        {
            followers.push(series);
        }
    }
    // Every block series a task repeats with, plus those seen continuing
    // one. A series the task names that is gone stays out.
    for follower in &followers {
        if let Some(name) = follower.block_series()
            && let Some((cal, id)) = name.rsplit_once('/')
            && cal == calendar
            && !block_ids.iter().any(|known| known == id)
        {
            block_ids.push(id.to_owned());
        }
    }
    let mut spans: Vec<Span> = Vec::new();
    for id in &block_ids {
        if let Some(series) = remote.series(calendar, id)?
            && rule_parts(&series.recurrence).is_some()
        {
            let first = planning_day(series.start, day_start_hour);
            let last = last_day(remote, &series, day_start_hour)?;
            spans.push(Span {
                series,
                first,
                last,
            });
        }
    }
    spans.sort_by_key(|span| span.first);

    let mut tracked: Vec<Follower> = Vec::new();
    for series in followers {
        let first = planning_day(series.start, day_start_hour);
        let last = last_day(remote, &series, day_start_hour)?;
        tracked.push(Follower {
            series,
            first,
            last,
        });
    }

    let yesterday = today.pred_opt().unwrap_or(today);
    let mut fixes = Vec::new();
    for follower in &tracked {
        let series = &follower.series;
        // Entirely in the past: history, left as it is.
        if follower.last.is_some_and(|last| last < today) {
            continue;
        }
        let history = follower.first < today;
        // Nothing left of its block from here on: keep what is past, drop
        // the rest.
        let end_here = || {
            if history {
                Fix::Rule {
                    calendar: calendar.to_owned(),
                    series: series.id.clone(),
                    recurrence: ended(series, yesterday, day_start_hour),
                    start: series.start,
                    end: series.end,
                }
            } else {
                Fix::Delete {
                    calendar: calendar.to_owned(),
                    series: series.id.clone(),
                }
            }
        };
        let from = follower.first.max(today);
        let name = series.block_series().unwrap_or_default().to_owned();
        let Some(governing) = governing(&spans, &name, from) else {
            fixes.push(end_here());
            continue;
        };
        let span = &spans[governing];
        let Some(first_block) = remote.first_occurrence(
            calendar,
            &span.series.id,
            calendar::at_clock(from.max(span.first), day_start_hour.min(23), 0),
        )?
        else {
            fixes.push(end_here());
            continue;
        };
        let first_day = planning_day(original_start(&first_block), day_start_hour);
        if span.last.is_some_and(|last| first_day > last) {
            fixes.push(end_here());
            continue;
        }
        let order = series.order();
        let place = offset(&tracked, &span.series.lineage(), order, first_day);
        let start = original_start(&first_block) + Duration::minutes(place);
        let end = start + Duration::minutes(series.minutes());
        // Ends with the block series, or earlier if it was ended earlier,
        // but never before today.
        let mut last = min_day(follower.last, span.last);
        if history {
            last = last.map(|last| last.max(yesterday));
        }
        let recurrence = following(&span.series, series, last, day_start_hour);
        if !history {
            if start != series.start
                || end != series.end
                || !same_rule(&recurrence, &series.recurrence, day_start_hour)
            {
                fixes.push(Fix::Rule {
                    calendar: calendar.to_owned(),
                    series: series.id.clone(),
                    recurrence,
                    start,
                    end,
                });
            }
        } else if start.time() != series.start.time()
            || end - start != series.end - series.start
            || rule_parts(&recurrence).map(|p| rule_shape(&p))
                != rule_parts(&series.recurrence).map(|p| rule_shape(&p))
        {
            // Its days or time changed. Rewriting the whole series would
            // rewrite its past too, so it ends yesterday and carries on as
            // a new series from today.
            let (skip, done) = carried(listing, series, first_day, day_start_hour);
            fixes.push(Fix::Continue {
                calendar: calendar.to_owned(),
                from: series.id.clone(),
                block_series: span.series.name(),
                recurrence,
                start,
                end,
                skip,
                done,
            });
            fixes.push(end_here());
            continue;
        } else if !same_rule(&recurrence, &series.recurrence, day_start_hour) {
            fixes.push(Fix::Rule {
                calendar: calendar.to_owned(),
                series: series.id.clone(),
                recurrence,
                start: series.start,
                end: series.end,
            });
        }
        // The block series ends and another carries it on: so does the task.
        let Some(block_last) = span.last else {
            continue;
        };
        if follower.last.is_some_and(|own| own <= block_last) {
            continue;
        }
        let Some(next) = spans.iter().position(|other| {
            other.first > block_last && other.series.lineage().contains(&span.series.name())
        }) else {
            continue;
        };
        let next_span = &spans[next];
        let next_name = next_span.series.name();
        let task_id = series.task.as_ref().map(|data| data.id);
        if tracked.iter().any(|other| {
            other.series.block_series() == Some(next_name.as_str())
                && other.series.task.as_ref().map(|data| data.id) == task_id
        }) {
            continue;
        }
        let Some(next_first) = remote.first_occurrence(
            calendar,
            &next_span.series.id,
            calendar::at_clock(next_span.first.max(today), day_start_hour.min(23), 0),
        )?
        else {
            continue;
        };
        let next_day = planning_day(original_start(&next_first), day_start_hour);
        let place = offset(&tracked, &next_span.series.lineage(), order, next_day);
        let start = original_start(&next_first) + Duration::minutes(place);
        let (skip, done) = carried(listing, series, next_day, day_start_hour);
        fixes.push(Fix::Continue {
            calendar: calendar.to_owned(),
            from: series.id.clone(),
            block_series: next_name,
            recurrence: following(
                &next_span.series,
                series,
                min_day(follower.last, next_span.last),
                day_start_hour,
            ),
            start,
            end: start + Duration::minutes(series.minutes()),
            skip,
            done,
        });
    }
    // A continuation is made before the series it continues is cut short,
    // so a failure between the two cannot lose the task's later days.
    fixes.sort_by_key(|fix| !matches!(fix, Fix::Continue { .. }));
    Ok(fixes)
}

/// The earlier of two last days, where `None` is never.
fn min_day(a: Option<NaiveDate>, b: Option<NaiveDate>) -> Option<NaiveDate> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Among the block series continuing `name`, the one covering `day`, else
/// the first to start after it.
fn governing(spans: &[Span], name: &str, day: NaiveDate) -> Option<usize> {
    let candidates: Vec<usize> = spans
        .iter()
        .enumerate()
        .filter(|(_, span)| span.series.lineage().iter().any(|known| known == name))
        .map(|(i, _)| i)
        .collect();
    candidates
        .iter()
        .copied()
        .find(|&i| spans[i].covers(day))
        .or_else(|| candidates.iter().copied().find(|&i| spans[i].first > day))
}

/// How far into its block a task series starts on `day`: after the tasks
/// repeating with the same block series that come before it.
fn offset(tracked: &[Follower], lineage: &[String], order: (i64, &str), day: NaiveDate) -> i64 {
    tracked
        .iter()
        .filter(|other| {
            other
                .series
                .block_series()
                .is_some_and(|name| lineage.iter().any(|known| known == name))
                && other.first <= day
                && other.last.is_none_or(|last| day <= last)
                && other.series.order() < order
        })
        .map(|other| other.series.minutes())
        .sum()
}

/// The days from `from` on that a task series skipped or was done on, so a
/// series continuing it keeps them.
fn carried(
    listing: &Listing,
    series: &Series,
    from: NaiveDate,
    day_start_hour: u32,
) -> (Vec<NaiveDate>, Vec<NaiveDate>) {
    let skip = listing
        .cancelled
        .iter()
        .filter(|(id, _)| *id == series.id)
        .map(|(_, start)| planning_day(*start, day_start_hour))
        .filter(|day| *day >= from)
        .collect();
    let done = listing
        .events
        .iter()
        .filter(|event| {
            event.completed && event.details.recurring_event_id.as_deref() == Some(&series.id)
        })
        .map(|event| planning_day(original_start(event), day_start_hour))
        .filter(|day| *day >= from)
        .collect();
    (skip, done)
}

/// The repeat rule a task series following `block` has: the block's rule,
/// ending on `last`, with the task's own skipped days.
fn following(
    block: &Series,
    task: &Series,
    last: Option<NaiveDate>,
    day_start_hour: u32,
) -> Vec<String> {
    let parts = rule_parts(&block.recurrence).unwrap_or_default();
    let mut rule: Vec<String> = parts
        .iter()
        .filter(|(key, _)| key != "UNTIL" && key != "COUNT")
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    if let Some(last) = last {
        rule.push(format!(
            "UNTIL={}",
            until_value(last, task.all_day, day_start_hour)
        ));
    }
    let mut lines = vec![format!("RRULE:{}", rule.join(";"))];
    lines.extend(
        task.recurrence
            .iter()
            .filter(|line| line.starts_with("EXDATE"))
            .cloned(),
    );
    lines
}

/// `series`' own rule, ending on `last`.
fn ended(series: &Series, last: NaiveDate, day_start_hour: u32) -> Vec<String> {
    series
        .recurrence
        .iter()
        .map(|line| match line.strip_prefix("RRULE:") {
            Some(rule) => {
                let mut parts: Vec<String> = rule
                    .split(';')
                    .filter(|part| !part.starts_with("UNTIL=") && !part.starts_with("COUNT="))
                    .map(str::to_owned)
                    .collect();
                parts.push(format!(
                    "UNTIL={}",
                    until_value(last, series.all_day, day_start_hour)
                ));
                format!("RRULE:{}", parts.join(";"))
            }
            None => line.clone(),
        })
        .collect()
}

/// Whether two sets of recurrence lines repeat alike and end on the same day.
fn same_rule(wanted: &[String], have: &[String], day_start_hour: u32) -> bool {
    let (Some(wanted), Some(have)) = (rule_parts(wanted), rule_parts(have)) else {
        return false;
    };
    let until = |parts: &[(String, String)]| {
        parts
            .iter()
            .find(|(key, _)| key == "UNTIL")
            .and_then(|(_, value)| until_day(value, day_start_hour))
    };
    rule_shape(&wanted) == rule_shape(&have)
        && until(&wanted) == until(&have)
        && !have.iter().any(|(key, _)| key == "COUNT")
}

fn occurrence_fixes(
    remote: &mut impl Remote,
    calendar: &str,
    listing: &Listing,
    day_start_hour: u32,
    today: NaiveDate,
) -> Result<Vec<Fix>, String> {
    let events = &listing.events;
    let blocks: Vec<&Event> = events.iter().filter(|event| !event.is_task()).collect();
    let mut fixes = Vec::new();
    // The past is history: only today on is put right.
    let upcoming = |event: &Event| planning_day(original_start(event), day_start_hour) >= today;
    for task in events
        .iter()
        .filter(|event| event.is_task() && !event.completed && upcoming(event))
    {
        if block_of(task, blocks.iter().copied(), day_start_hour).is_some() {
            continue;
        }
        if is_follower(task) {
            // Its block occurrence was deleted on its own: so is this one.
            let name = metadata(task).block_series.unwrap_or_default();
            let day = planning_day(original_start(task), day_start_hour);
            let deleted = listing.cancelled.iter().any(|(series, start)| {
                series_lineage(calendar, series, &[]).contains(&name)
                    && planning_day(*start, day_start_hour) == day
            });
            if deleted {
                fixes.push(Fix::Cancel {
                    calendar: calendar.to_owned(),
                    event: task.id.clone(),
                });
            }
            continue;
        }
        // A one-off whose block occurrence on this calendar is gone.
        let Some(key) = metadata(task).block else {
            continue;
        };
        let Some((cal, id)) = key.rsplit_once('/') else {
            continue;
        };
        if cal == calendar && !remote.exists(cal, id)? {
            fixes.push(Fix::Release {
                task: Box::new(task.clone()),
            });
        }
    }
    for block in blocks
        .iter()
        .filter(|block| planning_day(block.start, day_start_hour) >= today)
    {
        let tasks = tasks_in(block, events, day_start_hour);
        if tasks.is_empty() {
            continue;
        }
        for (task, (start, end)) in tasks.iter().zip(layout(block, &tasks)) {
            if !task.completed && (task.all_day || task.start != start || task.end != end) {
                fixes.push(Fix::Move {
                    calendar: calendar.to_owned(),
                    event: task.id.clone(),
                    start,
                    end,
                });
            }
        }
    }
    Ok(fixes)
}

#[cfg(test)]
pub(crate) mod fake;

#[cfg(test)]
mod tests;
