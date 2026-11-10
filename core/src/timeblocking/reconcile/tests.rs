//! Every way a recurring block changes, against an in-memory Google.
//!
//! Each test makes the change as a person would (in this app or on Google's
//! site), reconciles, and then checks the invariants that must always hold:
//!
//! 1. Every open occurrence of a repeating task is in a block occurrence.
//! 2. Every block's open tasks sit end to end from its start, repeating
//!    ones first.
//! 3. Every repeating task series repeats as its block series does.
//! 4. Reconciling again makes no writes.
//!
//! The first three hold from today (the 10th) on. Before today is history,
//! which the reconciler leaves alone.

use chrono::{DateTime, Local, NaiveDate};

use super::fake::{CALENDAR, FakeGoogle};
use super::*;
use crate::calendar::{self, Event};
use crate::timeblocking::membership::{self, block_of, is_follower, layout, tasks_in};

fn day(d: u32) -> NaiveDate {
    if d > 30 {
        NaiveDate::from_ymd_opt(2026, 12, d - 30).unwrap()
    } else {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }
}

fn at(d: u32, hour: u32, minute: u32) -> DateTime<Local> {
    calendar::at_clock(day(d), hour, minute)
}

const TODAY: u32 = 10;

fn window() -> (DateTime<Local>, DateTime<Local>) {
    (at(1, 0, 0), at(45, 0, 0))
}

/// Reconciles, then checks every invariant and that a second pass is idle.
fn settle(google: &mut FakeGoogle, hour: u32) -> Vec<Fix> {
    let (first, last) = window();
    let fixes = reconcile(google, CALENDAR, first, last, hour, day(TODAY)).expect("reconciles");
    let problems = check(google, hour);
    assert!(problems.is_empty(), "{problems:#?}\nafter {fixes:#?}");
    let again = reconcile(google, CALENDAR, first, last, hour, day(TODAY)).unwrap();
    assert!(again.is_empty(), "a second pass wrote {again:#?}");
    fixes
}

fn upcoming(event: &Event, hour: u32) -> bool {
    membership::planning_day(membership::original_start(event), hour) >= day(TODAY)
}

fn check(google: &mut FakeGoogle, hour: u32) -> Vec<String> {
    let (first, last) = window();
    let listing = google.listing(CALENDAR, first, last).unwrap();
    let events = &listing.events;
    let blocks: Vec<&Event> = events.iter().filter(|e| !e.is_task()).collect();
    let mut problems = Vec::new();
    for task in events
        .iter()
        .filter(|e| is_follower(e) && !e.completed && upcoming(e, hour))
    {
        let Some(block) = block_of(task, blocks.iter().copied(), hour) else {
            problems.push(format!("{} on {} has no block", task.title, task.start));
            continue;
        };
        // Invariant 3: the task's series repeats as its block's does.
        let task_series = google
            .series(
                CALENDAR,
                task.details.recurring_event_id.as_deref().unwrap(),
            )
            .unwrap()
            .unwrap();
        let block_series = google
            .series(
                CALENDAR,
                block.details.recurring_event_id.as_deref().unwrap(),
            )
            .unwrap()
            .unwrap();
        if rule_parts(&task_series.recurrence).map(|p| rule_shape(&p))
            != rule_parts(&block_series.recurrence).map(|p| rule_shape(&p))
        {
            problems.push(format!(
                "{} repeats {:?} but its block repeats {:?}",
                task.title, task_series.recurrence, block_series.recurrence
            ));
        }
    }
    for block in blocks.iter().filter(|b| upcoming(b, hour)) {
        let tasks = tasks_in(block, events, hour);
        for (task, (start, end)) in tasks.iter().zip(layout(block, &tasks)) {
            if !task.completed && (task.start != start || task.end != end) {
                problems.push(format!(
                    "{} is at {}–{} but belongs at {start}–{end}",
                    task.title, task.start, task.end
                ));
            }
        }
    }
    problems
}

/// The open tasks in `block_series`' occurrence first scheduled on `d`, by
/// title, in order.
fn titles_on(google: &mut FakeGoogle, block_series: &str, d: u32) -> Vec<String> {
    let (first, last) = window();
    let listing = google.listing(CALENDAR, first, last).unwrap();
    let block = listing
        .events
        .iter()
        .find(|e| {
            !e.is_task()
                && membership::lineage(e).contains(&format!("{CALENDAR}/{block_series}"))
                && membership::original_start(e).date_naive() == day(d)
        })
        .unwrap_or_else(|| panic!("no block occurrence on {}", day(d)));
    tasks_in(block, &listing.events, 0)
        .into_iter()
        .map(|t| t.title.clone())
        .collect()
}

/// The task's occurrence first scheduled on `d`, whichever of its series
/// has it.
fn on(google: &FakeGoogle, task_id: i64, d: u32) -> Option<Event> {
    google
        .followers_of(task_id)
        .into_iter()
        .flat_map(|series| google.occurrences(&series.master.id))
        .find(|e| membership::original_start(e).date_naive() == day(d))
}

const DAILY: &str = "FREQ=DAILY";

#[test]
fn a_task_joining_a_block_series_is_placed_and_repeats_with_it() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 120, DAILY);
    let first = google.add_follower(&block, day(12), "Inbox zero", 1, 30);
    let second = google.add_follower(&block, day(12), "Plan", 2, 15);
    let (first, second) = (google.task_id(&first), google.task_id(&second));
    settle(&mut google, 0);
    // The second starts after the first, every day from the 12th on.
    assert_eq!(on(&google, second, 14).unwrap().start, at(14, 9, 30));
    assert_eq!(on(&google, first, 14).unwrap().start, at(14, 9, 0));
    assert!(on(&google, first, 11).is_none());
    assert_eq!(titles_on(&mut google, &block, 20), ["Inbox zero", "Plan"]);
}

#[test]
fn one_offs_sit_after_the_repeating_tasks_and_never_push_them() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 120, DAILY);
    let occurrence = google.occurrence_on(&block, day(13));
    // Ordered before the repeating task, but still placed after it.
    let one_off = google.add_one_off(&occurrence, "Call the bank", 0, 20);
    let repeating = google.add_follower(&block, day(12), "Inbox zero", 9, 30);
    let repeating = google.task_id(&repeating);
    settle(&mut google, 0);
    assert_eq!(
        titles_on(&mut google, &block, 13),
        ["Inbox zero", "Call the bank"]
    );
    assert_eq!(google.single(&one_off).start, at(13, 9, 30));
    assert_eq!(on(&google, repeating, 13).unwrap().start, at(13, 9, 0));
}

#[test]
fn moving_one_block_occurrence_takes_its_tasks_along_even_to_another_day() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 120, DAILY);
    let repeating = google.add_follower(&block, day(12), "Inbox zero", 1, 30);
    let repeating = google.task_id(&repeating);
    let one_off = google.add_one_off(&google.occurrence_on(&block, day(15)), "Call", 2, 20);
    settle(&mut google, 0);
    // Later the same day, and the next one to another day.
    google.move_occurrence(&block, day(14), at(14, 14, 0));
    google.move_occurrence(&block, day(15), at(16, 18, 0));
    settle(&mut google, 0);
    assert_eq!(on(&google, repeating, 14).unwrap().start, at(14, 14, 0));
    assert_eq!(on(&google, repeating, 15).unwrap().start, at(16, 18, 0));
    assert_eq!(google.single(&one_off).start, at(16, 18, 30));
    // The other days are untouched.
    assert_eq!(on(&google, repeating, 16).unwrap().start, at(16, 9, 0));
}

#[test]
fn retiming_the_block_series_retimes_its_tasks_from_today_and_keeps_the_past() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 120, DAILY);
    let repeating = google.add_follower(&block, day(5), "Inbox zero", 1, 30);
    let task_id = google.task_id(&repeating);
    google.complete(&repeating, day(6));
    let one_off = google.add_one_off(&google.occurrence_on(&block, day(12)), "Call", 2, 20);
    settle(&mut google, 0);
    google.retime_series(&block, 10, 30);
    settle(&mut google, 0);
    assert_eq!(on(&google, task_id, 15).unwrap().start, at(15, 10, 30));
    // Before today is history: as it was, done day included.
    assert_eq!(on(&google, task_id, 7).unwrap().start, at(7, 9, 0));
    assert!(on(&google, task_id, 6).unwrap().completed);
    // Its occurrence has a new ID now; the one-off still found it.
    assert_eq!(google.single(&one_off).start, at(12, 11, 0));
    assert_eq!(titles_on(&mut google, &block, 12), ["Inbox zero", "Call"]);
}

#[test]
fn a_task_that_has_not_started_yet_is_changed_in_place() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 60, DAILY);
    let repeating = google.add_follower(&block, day(12), "Inbox zero", 1, 30);
    let task_id = google.task_id(&repeating);
    settle(&mut google, 0);
    google.retime_series(&block, 10, 0);
    settle(&mut google, 0);
    assert_eq!(google.followers_of(task_id).len(), 1);
    assert_eq!(on(&google, task_id, 12).unwrap().start, at(12, 10, 0));
}

#[test]
fn splitting_the_block_series_in_this_app_continues_its_tasks() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 60, DAILY);
    let repeating = google.add_follower(&block, day(5), "Inbox zero", 1, 30);
    let task_id = google.task_id(&repeating);
    google.complete(&repeating, day(6));
    google.complete(&repeating, day(22));
    google.delete_occurrence(&repeating, day(23));
    settle(&mut google, 0);
    let later = google.split(&block, day(20), at(20, 13, 0), None, false);
    settle(&mut google, 0);
    let series = google.followers_of(task_id);
    assert_eq!(series.len(), 2, "the task continues as a second series");
    assert!(task_on(&google, &repeating, 19).is_some());
    assert!(task_on(&google, &repeating, 20).is_none());
    assert!(on(&google, task_id, 6).unwrap().completed);
    assert_eq!(on(&google, task_id, 25).unwrap().start, at(25, 13, 0));
    assert_eq!(titles_on(&mut google, &later, 25), ["Inbox zero"]);
    // Done and skipped days after the split stay that way.
    assert!(on(&google, task_id, 22).unwrap().completed);
    assert!(on(&google, task_id, 23).is_none());
}

#[test]
fn a_split_made_on_googles_site_with_new_days_is_followed() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 60, "FREQ=WEEKLY;BYDAY=MO,WE,FR");
    let repeating = google.add_follower(&block, day(11), "Review", 1, 30);
    let task_id = google.task_id(&repeating);
    settle(&mut google, 0);
    // From the 16th (a Monday): Tuesdays and Thursdays at 8.
    let later = google.split(
        &block,
        day(16),
        at(17, 8, 0),
        Some("FREQ=WEEKLY;BYDAY=TU,TH"),
        true,
    );
    settle(&mut google, 0);
    assert_eq!(google.followers_of(task_id).len(), 2);
    assert_eq!(on(&google, task_id, 19).unwrap().start, at(19, 8, 0));
    assert!(on(&google, task_id, 18).is_none());
    assert_eq!(titles_on(&mut google, &later, 24), ["Review"]);
    assert!(on(&google, task_id, 13).is_some());
}

#[test]
fn deleting_one_block_occurrence_drops_its_repeating_tasks_and_frees_one_offs() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 60, DAILY);
    let repeating = google.add_follower(&block, day(5), "Inbox zero", 1, 30);
    let one_off = google.add_one_off(&google.occurrence_on(&block, day(12)), "Call", 2, 20);
    settle(&mut google, 0);
    google.delete_occurrence(&block, day(12));
    let fixes = settle(&mut google, 0);
    assert!(task_on(&google, &repeating, 12).is_none());
    assert!(task_on(&google, &repeating, 13).is_some());
    let freed = google.single(&one_off);
    assert!(freed.all_day && freed.start.date_naive() == day(12));
    assert!(crate::timeblocking::metadata(freed).block.is_none());
    assert!(
        fixes.iter().any(|fix| matches!(fix, Fix::Release { .. })),
        "{fixes:#?}"
    );
}

#[test]
fn ending_the_block_series_ends_its_tasks_and_deleting_it_keeps_the_past() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 60, DAILY);
    let repeating = google.add_follower(&block, day(3), "Inbox zero", 1, 30);
    let future = google.add_follower(&block, day(14), "Later", 2, 30);
    settle(&mut google, 0);
    google.delete_following(&block, day(20));
    settle(&mut google, 0);
    assert!(task_on(&google, &repeating, 19).is_some());
    assert!(task_on(&google, &repeating, 20).is_none());

    google.delete_series(&block);
    settle(&mut google, 0);
    // Before today stays as history; from today on is gone.
    assert!(task_on(&google, &repeating, TODAY - 1).is_some());
    assert!(task_on(&google, &repeating, TODAY).is_none());
    assert!(google.occurrences(&future).is_empty());
}

#[test]
fn a_skipped_day_stays_skipped_and_the_next_task_moves_up() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 120, DAILY);
    let first = google.add_follower(&block, day(12), "Inbox zero", 1, 30);
    let second = google.add_follower(&block, day(12), "Plan", 2, 15);
    settle(&mut google, 0);
    google.delete_occurrence(&first, day(15));
    settle(&mut google, 0);
    assert!(task_on(&google, &first, 15).is_none());
    assert_eq!(task_on(&google, &second, 15).unwrap().start, at(15, 9, 0));
    assert_eq!(task_on(&google, &second, 16).unwrap().start, at(16, 9, 30));
}

#[test]
fn done_occurrences_never_move() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 120, DAILY);
    let repeating = google.add_follower(&block, day(12), "Inbox zero", 1, 30);
    settle(&mut google, 0);
    google.complete(&repeating, day(13));
    google.move_occurrence(&block, day(13), at(13, 15, 0));
    google.move_occurrence(&block, day(14), at(14, 15, 0));
    settle(&mut google, 0);
    assert_eq!(
        task_on(&google, &repeating, 13).unwrap().start,
        at(13, 9, 0)
    );
    assert_eq!(
        task_on(&google, &repeating, 14).unwrap().start,
        at(14, 15, 0)
    );
}

#[test]
fn a_task_ended_early_is_not_extended() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 60, DAILY);
    let repeating = google.add_follower(&block, day(5), "Inbox zero", 1, 30);
    settle(&mut google, 0);
    google.delete_following(&repeating, day(15));
    settle(&mut google, 0);
    assert!(task_on(&google, &repeating, 14).is_some());
    assert!(task_on(&google, &repeating, 15).is_none());
}

#[test]
fn a_block_with_a_repeat_count_ends_its_tasks_on_its_last_day() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 60, "FREQ=DAILY;COUNT=20");
    let repeating = google.add_follower(&block, day(5), "Inbox zero", 1, 30);
    settle(&mut google, 0);
    assert!(task_on(&google, &repeating, 21).is_some());
    assert!(task_on(&google, &repeating, 22).is_none());
}

#[test]
fn a_task_series_on_the_wrong_rule_is_put_back_from_today() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 60, "FREQ=WEEKLY;BYDAY=MO,WE");
    let repeating = google.add_follower(&block, day(2), "Review", 1, 30);
    let task_id = google.task_id(&repeating);
    settle(&mut google, 0);
    // Someone changed the task's own repeat on Google's site.
    google
        .apply(&Fix::Rule {
            calendar: CALENDAR.into(),
            series: repeating.clone(),
            recurrence: vec!["RRULE:FREQ=DAILY".into()],
            start: at(2, 7, 0),
            end: at(2, 7, 30),
        })
        .unwrap();
    settle(&mut google, 0);
    assert!(on(&google, task_id, 12).is_none());
    assert_eq!(on(&google, task_id, 16).unwrap().start, at(16, 9, 0));
}

#[test]
fn a_block_running_past_midnight_keeps_its_small_hours_tasks() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 23, 0), 120, DAILY);
    let first = google.add_follower(&block, day(12), "Wind down", 1, 60);
    let second = google.add_follower(&block, day(12), "Read", 2, 30);
    let (first, second) = (google.task_id(&first), google.task_id(&second));
    settle(&mut google, 4);
    // The 13th's block runs from 11pm to 1am; reading starts at midnight.
    assert_eq!(on(&google, first, 13).unwrap().start, at(13, 23, 0));
    assert_eq!(on(&google, second, 14).unwrap().start, at(14, 0, 0));
}

#[test]
fn a_one_off_block_lays_out_its_tasks_and_deleting_it_frees_them() {
    let mut google = FakeGoogle::default();
    let single = google.add_block(at(12, 14, 0), 60);
    let block = google.singles[0].clone();
    let a = google.add_one_off(&block, "A", 2, 20);
    let b = google.add_one_off(&block, "B", 1, 15);
    google.singles.iter_mut().find(|e| e.id == a).unwrap().start = at(12, 8, 0);
    settle(&mut google, 0);
    assert_eq!(google.single(&b).start, at(12, 14, 0));
    assert_eq!(google.single(&a).start, at(12, 14, 15));
    google.singles.retain(|e| e.id != single);
    settle(&mut google, 0);
    assert!(google.single(&a).all_day);
}

#[test]
fn the_past_is_left_alone() {
    let mut google = FakeGoogle::default();
    let block = google.add_block_series(at(2, 9, 0), 60, DAILY);
    let repeating = google.add_follower(&block, day(3), "Inbox zero", 1, 30);
    let one_off = google.add_one_off(&google.occurrence_on(&block, day(4)), "Old", 2, 20);
    google.move_occurrence(&block, day(5), at(5, 16, 0));
    google.delete_occurrence(&block, day(4));
    let fixes = settle(&mut google, 0);
    assert!(fixes.is_empty(), "{fixes:#?}");
    assert_eq!(task_on(&google, &repeating, 5).unwrap().start, at(5, 9, 0));
    assert!(!google.single(&one_off).all_day);
}

fn task_on(google: &FakeGoogle, series: &str, d: u32) -> Option<Event> {
    google
        .occurrences(series)
        .into_iter()
        .find(|e| membership::original_start(e).date_naive() == day(d))
}
