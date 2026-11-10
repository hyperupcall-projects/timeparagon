//! Which time block a task is in.
//!
//! This is the one place that answers it. Everything else (the day views on
//! both apps, the dock, search, layout, and the reconciler that keeps Google
//! in step) asks here rather than comparing stored keys itself.
//!
//! A task in a block is one of two kinds:
//!
//! - **One-off**: its metadata names one block occurrence by key,
//!   `calendar/event`. For an occurrence of a recurring block that event ID
//!   is Google's instance ID, `series_YYYYMMDDTHHMMSSZ`, which Google changes
//!   when the series' time is edited or the series is split. So a key that no
//!   longer matches exactly still matches the occurrence of the same series
//!   (or a series continuing it) on the same planning day.
//! - **Repeating with the block**: the task is its own Google series with the
//!   block's repeat rule, and its metadata names the block's series in
//!   `block_series`. Each of its occurrences belongs to the block occurrence
//!   on the same planning day. The `block` key Google copies onto every
//!   occurrence is ignored.
//!
//! Days are compared by when each occurrence was first scheduled (Google's
//! `originalStartTime`), so moving one occurrence of a block to another day
//! takes its tasks along.

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};

use super::{block_key, metadata};
use crate::calendar::{self, Event};

/// Google's ID for one occurrence: the series ID, `_`, then the original
/// start as `YYYYMMDD` (all-day) or `YYYYMMDDTHHMMSSZ` (UTC).
pub fn parse_instance_id(id: &str) -> Option<(&str, DateTime<Local>)> {
    let (series, stamp) = id.rsplit_once('_')?;
    if series.is_empty() {
        return None;
    }
    let start = if stamp.len() == 8 && stamp.bytes().all(|b| b.is_ascii_digit()) {
        let date = NaiveDate::parse_from_str(stamp, "%Y%m%d").ok()?;
        calendar::at_clock(date, 0, 0)
    } else if stamp.len() == 16 && stamp.ends_with('Z') {
        let naive = NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%SZ").ok()?;
        Utc.from_utc_datetime(&naive).with_timezone(&Local)
    } else {
        return None;
    };
    Some((series, start))
}

/// When this occurrence was first scheduled: Google's `originalStartTime`,
/// else what its instance ID says, else its start.
pub fn original_start(event: &Event) -> DateTime<Local> {
    event
        .details
        .original_start
        .or_else(|| {
            event
                .details
                .recurring_event_id
                .as_ref()
                .and_then(|_| parse_instance_id(&event.id))
                .map(|(_, start)| start)
        })
        .unwrap_or(event.start)
}

/// The planning day an instant falls on: before `day_start_hour` it still
/// counts as the day before.
pub fn planning_day(when: DateTime<Local>, day_start_hour: u32) -> NaiveDate {
    calendar::day_at(when, day_start_hour)
}

/// A series that Google Calendar split off ("this and following" edited on
/// Google's site) is named `base_RYYYYMMDD…`. Returns `base`.
fn google_split_base(series: &str) -> Option<&str> {
    let (base, stamp) = series.rsplit_once("_R")?;
    let digits = stamp.trim_end_matches('Z');
    let valid = match digits.len() {
        8 => digits.bytes().all(|b| b.is_ascii_digit()),
        15 => {
            digits.as_bytes()[8] == b'T'
                && digits
                    .bytes()
                    .enumerate()
                    .all(|(i, b)| i == 8 || b.is_ascii_digit())
        }
        _ => false,
    };
    (valid && !base.is_empty()).then_some(base)
}

/// The series `series` (on `calendar`) is, and every series it continues,
/// as `calendar/series`: a split recorded by this app in `split_from`, and
/// one Google made, whose ID says which series it came from.
pub fn series_lineage(calendar: &str, series: &str, split_from: &[String]) -> Vec<String> {
    let mut names: Vec<String> = split_from.to_vec();
    let mut current = series;
    loop {
        let name = format!("{calendar}/{current}");
        if !names.contains(&name) {
            names.push(name);
        }
        match google_split_base(current) {
            Some(base) => current = base,
            None => break,
        }
    }
    names
}

/// For an occurrence of a recurring block: its series and every series it
/// continues. Empty for a block that does not repeat.
pub fn lineage(block: &Event) -> Vec<String> {
    block
        .details
        .recurring_event_id
        .as_deref()
        .map(|series| series_lineage(&block.calendar_id, series, &block.details.split_from))
        .unwrap_or_default()
}

/// An occurrence of a task that repeats with a block series.
pub fn is_follower(task: &Event) -> bool {
    task.is_task()
        && task.details.recurring_event_id.is_some()
        && metadata(task).block_series.is_some()
}

/// Repeats with a block, or is about to once Google has the block's rule.
pub fn repeats_with_block(task: &Event) -> bool {
    task.is_task() && metadata(task).block_series.is_some()
}

/// The task belongs in some block, whether or not that block is loaded.
pub fn is_assigned(task: &Event) -> bool {
    task.is_task() && (metadata(task).block.is_some() || is_follower(task))
}

/// Whether the stored key names `block` exactly.
fn exact(task: &Event, block: &Event) -> bool {
    !is_follower(task) && metadata(task).block.as_deref() == Some(block_key(block).as_str())
}

/// Whether `task` belongs in `block`.
pub fn in_block(task: &Event, block: &Event, day_start_hour: u32) -> bool {
    if !task.is_task() || block.is_task() {
        return false;
    }
    let meta = metadata(task);
    if is_follower(task) {
        let series = meta.block_series.as_deref().unwrap_or_default();
        return lineage(block).iter().any(|name| name == series)
            && planning_day(original_start(task), day_start_hour)
                == planning_day(original_start(block), day_start_hour);
    }
    let Some(key) = meta.block else {
        return false;
    };
    if key == block_key(block) {
        return true;
    }
    // The occurrence's ID has changed since the task was put in it: its
    // series was retimed or split. Match the same series on the same day.
    let Some((calendar_id, id)) = key.rsplit_once('/') else {
        return false;
    };
    let Some((series, start)) = parse_instance_id(id) else {
        return false;
    };
    let name = format!("{calendar_id}/{series}");
    block.calendar_id == calendar_id
        && lineage(block).contains(&name)
        && planning_day(start, day_start_hour)
            == planning_day(original_start(block), day_start_hour)
}

/// The block `task` is in, among `events`. An exact key match wins over a
/// block that only matches by series and day.
pub fn block_of<'a>(
    task: &Event,
    events: impl IntoIterator<Item = &'a Event>,
    day_start_hour: u32,
) -> Option<&'a Event> {
    let mut found = None;
    for block in events {
        if exact(task, block) {
            return Some(block);
        }
        if found.is_none() && in_block(task, block, day_start_hour) {
            found = Some(block);
        }
    }
    found
}

/// The order tasks sit in a block: those repeating with it first, so a
/// one-off added on one day never pushes them later, then each by its
/// order. Ties go by series, so every day of a block lays its repeating
/// tasks out alike.
pub fn sort_key(task: &Event) -> (bool, i64, String) {
    let id = if is_follower(task) {
        task.details.recurring_event_id.clone().unwrap_or_default()
    } else {
        task.id.clone()
    };
    (!repeats_with_block(task), metadata(task).order, id)
}

/// The tasks in `block`, in the order they are laid out.
pub fn tasks_in<'a>(
    block: &Event,
    events: impl IntoIterator<Item = &'a Event>,
    day_start_hour: u32,
) -> Vec<&'a Event> {
    let mut tasks: Vec<&Event> = events
        .into_iter()
        .filter(|task| in_block(task, block, day_start_hour))
        .collect();
    tasks.sort_by_key(|task| sort_key(task));
    tasks
}

/// How long a task takes in a block: its estimate, or half an hour.
pub fn task_minutes(task: &Event) -> i64 {
    match metadata(task).minutes {
        0 => 30,
        minutes => i64::from(minutes),
    }
}

/// Where each of `tasks` (already in block order) goes: end to end from the
/// block's start.
pub fn layout(block: &Event, tasks: &[&Event]) -> Vec<(DateTime<Local>, DateTime<Local>)> {
    let mut start = block.start;
    tasks
        .iter()
        .map(|task| {
            let end = start + chrono::Duration::minutes(task_minutes(task));
            let slot = (start, end);
            start = end;
            slot
        })
        .collect()
}

/// Keys of the events in `events` that some task is in, so an event on
/// another calendar that holds tasks still shows as a block.
pub fn holding_blocks(events: &[Event], day_start_hour: u32) -> std::collections::HashSet<String> {
    let tasks: Vec<&Event> = events.iter().filter(|e| is_assigned(e)).collect();
    events
        .iter()
        .filter(|block| !block.is_task())
        .filter(|block| {
            tasks
                .iter()
                .any(|task| in_block(task, block, day_start_hour))
        })
        .map(block_key)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::Details;
    use crate::task_db::TaskData;
    use crate::timeblocking::Metadata;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    fn stamp(when: DateTime<Local>) -> String {
        when.with_timezone(&Utc)
            .format("%Y%m%dT%H%M%SZ")
            .to_string()
    }

    fn occurrence(series: &str, d: u32, hour: u32) -> Event {
        let start = calendar::at_clock(day(d), hour, 0);
        Event {
            id: format!("{series}_{}", stamp(start)),
            calendar_id: "app".into(),
            title: series.into(),
            start,
            end: start + chrono::Duration::hours(1),
            all_day: false,
            color: calendar::colors::TASK,
            completed: false,
            details: Details {
                recurring_event_id: Some(series.into()),
                original_start: Some(start),
                ..Default::default()
            },
        }
    }

    fn task(meta: Metadata) -> Event {
        let mut event = occurrence("task", 9, 9);
        event.details.recurring_event_id = None;
        event.details.original_start = None;
        event.id = "task".into();
        event.details.task_data = Some(TaskData {
            id: 100,
            metadata: meta,
        });
        event
    }

    #[test]
    fn instance_ids_name_their_series_and_original_start() {
        let start = calendar::at_clock(day(9), 9, 0);
        let id = format!("abc_{}", stamp(start));
        assert_eq!(parse_instance_id(&id), Some(("abc", start)));
        assert_eq!(
            parse_instance_id("abc_20261109"),
            Some(("abc", calendar::at_clock(day(9), 0, 0)))
        );
        assert_eq!(parse_instance_id("plain"), None);
        assert_eq!(parse_instance_id("abc_notastamp"), None);
    }

    #[test]
    fn a_split_series_continues_the_one_it_came_from() {
        assert_eq!(
            series_lineage("app", "abc_R20261109T170000", &[]),
            ["app/abc_R20261109T170000", "app/abc"]
        );
        assert_eq!(
            series_lineage("app", "new", &["app/old".into()]),
            ["app/old", "app/new"]
        );
        assert_eq!(series_lineage("app", "abc_Rxyz", &[]), ["app/abc_Rxyz"]);
    }

    #[test]
    fn a_one_off_follows_its_occurrence_through_a_retimed_or_split_series() {
        let placed = occurrence("block", 9, 9);
        let task = task(Metadata {
            block: Some(block_key(&placed)),
            ..Default::default()
        });
        assert!(in_block(&task, &placed, 0));
        // The series moved to 10am: every occurrence has a new ID.
        let retimed = occurrence("block", 9, 10);
        assert_ne!(block_key(&retimed), block_key(&placed));
        assert!(in_block(&task, &retimed, 0));
        // Not the next day's occurrence, nor another series that day.
        assert!(!in_block(&task, &occurrence("block", 10, 9), 0));
        assert!(!in_block(&task, &occurrence("other", 9, 9), 0));
        // Edited "this and following" here, by this app or on Google's site.
        let mut split = occurrence("later", 9, 11);
        split.details.split_from = vec!["app/block".into()];
        assert!(in_block(&task, &split, 0));
        assert!(in_block(
            &task,
            &occurrence("block_R20261109T090000", 9, 11),
            0
        ));
        // Moving just that occurrence to another day takes the task along.
        let mut moved = placed.clone();
        moved.start = calendar::at_clock(day(11), 15, 0);
        moved.end = moved.start + chrono::Duration::hours(1);
        assert!(in_block(&task, &moved, 0));
        // The exact match wins when two blocks could take it.
        let events = [retimed.clone(), placed.clone()];
        assert_eq!(block_of(&task, &events, 0).unwrap().id, placed.id);
    }

    #[test]
    fn a_repeating_task_belongs_to_the_block_occurrence_of_its_own_day() {
        let mut task = occurrence("task", 10, 9);
        task.details.task_data = Some(TaskData {
            id: 100,
            metadata: Metadata {
                block_series: Some("app/block".into()),
                // Copied from the series to every occurrence; ignored.
                block: Some("app/block_x".into()),
                ..Default::default()
            },
        });
        assert!(is_follower(&task) && is_assigned(&task));
        assert!(in_block(&task, &occurrence("block", 10, 8), 0));
        assert!(!in_block(&task, &occurrence("block", 9, 8), 0));
        assert!(!in_block(&task, &occurrence("other", 10, 8), 0));
        // The block occurrence was moved to the next day: the task follows.
        let mut moved = occurrence("block", 10, 8);
        moved.start += chrono::Duration::days(1);
        assert!(in_block(&task, &moved, 0));
        // A block running past midnight keeps its small-hours tasks when
        // the planning day ends later than midnight.
        let mut late = occurrence("task", 11, 0);
        late.start += chrono::Duration::minutes(30);
        late.details.original_start = Some(late.start);
        late.details.task_data = task.details.task_data.clone();
        let evening = occurrence("block", 10, 23);
        assert!(in_block(&late, &evening, 4));
        assert!(!in_block(&late, &evening, 0));
    }

    #[test]
    fn repeating_tasks_come_first_and_are_laid_end_to_end() {
        let block = occurrence("block", 9, 9);
        let one_off = task(Metadata {
            block: Some(block_key(&block)),
            order: 1,
            minutes: 15,
            ..Default::default()
        });
        let mut repeating = occurrence("task", 9, 9);
        repeating.id = "repeating".into();
        repeating.details.task_data = Some(TaskData {
            id: 101,
            metadata: Metadata {
                block_series: Some("app/block".into()),
                order: 5,
                ..Default::default()
            },
        });
        let events = [one_off.clone(), repeating.clone()];
        let tasks = tasks_in(&block, &events, 0);
        assert_eq!(
            tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["repeating", "task"]
        );
        let slots = layout(&block, &tasks);
        assert_eq!(
            slots[0],
            (block.start, block.start + chrono::Duration::minutes(30))
        );
        assert_eq!(slots[1].1, block.start + chrono::Duration::minutes(45));
        assert_eq!(
            holding_blocks(&[block.clone(), one_off], 0),
            [block_key(&block)].into()
        );
    }
}
