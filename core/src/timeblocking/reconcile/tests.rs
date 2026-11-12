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
fn a_one_off_stays_in_its_block_once_the_block_starts_repeating() {
	let mut google = FakeGoogle::default();
	let single = google.add_block(at(12, 14, 0), 60);
	let block = google.singles[0].clone();
	let task = google.add_one_off(&block, "A", 1, 20);
	settle(&mut google, 0);
	google.make_repeating(&single, DAILY);
	let fixes = settle(&mut google, 0);
	assert!(fixes.is_empty(), "{fixes:#?}");
	let (first, last) = window();
	let events = google.listing(CALENDAR, first, last).unwrap().events;
	let task = google.single(&task);
	let holder = block_of(task, events.iter().filter(|e| !e.is_task()), 0).expect("in a block");
	assert_eq!(
		holder.details.recurring_event_id.as_deref(),
		Some(single.as_str())
	);
	assert_eq!(holder.start, at(12, 14, 0));
	// Only that day: it does not repeat with the block.
	assert!(!is_follower(task));
	let next = google.occurrence_on(&single, day(13));
	assert!(tasks_in(&next, &events, 0).is_empty());
	// Deleting that first occurrence on its own frees the task.
	google.delete_occurrence(&single, day(12));
	let fixes = settle(&mut google, 0);
	assert!(
		fixes.iter().any(|fix| matches!(fix, Fix::Release { .. })),
		"{fixes:#?}"
	);
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

#[test]
fn deleting_the_block_series_keeps_days_already_done() {
	let mut google = FakeGoogle::default();
	let block = google.add_block_series(at(2, 9, 0), 60, DAILY);
	let repeating = google.add_follower(&block, day(12), "Inbox zero", 1, 30);
	settle(&mut google, 0);
	google.complete(&repeating, day(13));
	google.delete_series(&block);
	settle(&mut google, 0);
	assert!(task_on(&google, &repeating, 13).unwrap().completed);
	assert!(task_on(&google, &repeating, 14).is_none());
}

#[test]
fn a_block_on_another_calendar_is_not_taken_for_deleted() {
	let mut google = FakeGoogle::default();
	let block = google.add_block_series_on("work", at(2, 9, 0), 60, DAILY);
	let repeating = google.add_follower(&block, day(12), "Inbox zero", 1, 30);
	// The task lives on the app calendar; the block does not, so the
	// invariants (which look at one calendar) do not apply here.
	let (first, last) = window();
	let fixes = reconcile(&mut google, CALENDAR, first, last, 0, day(TODAY)).unwrap();
	assert!(
		!fixes.iter().any(|fix| matches!(
			fix,
			Fix::Delete { .. } | Fix::Rule { .. } | Fix::Cancel { .. }
		)),
		"{fixes:#?}"
	);
	assert!(task_on(&google, &repeating, 20).is_some());
}

fn task_on(google: &FakeGoogle, series: &str, d: u32) -> Option<Event> {
	google
		.occurrences(series)
		.into_iter()
		.find(|e| membership::original_start(e).date_naive() == day(d))
}

#[test]
fn a_series_ended_at_an_occurrence_ends_the_day_before_it() {
	let series = |hour: u32, minute: u32| Series {
		calendar: CALENDAR.into(),
		id: "s".into(),
		recurrence: Vec::new(),
		start: at(2, hour, minute),
		end: at(2, hour, minute) + chrono::Duration::hours(1),
		all_day: false,
		split_from: Vec::new(),
		task: None,
	};
	// As this app and Google write it: a second before the first occurrence
	// dropped.
	let until = |when: DateTime<Local>| {
		(when - chrono::Duration::seconds(1))
			.with_timezone(&chrono::Utc)
			.format("%Y%m%dT%H%M%SZ")
			.to_string()
	};
	assert_eq!(
		until_day(&series(9, 0), &until(at(20, 9, 0)), 0),
		Some(day(19))
	);
	assert_eq!(
		until_day(&series(9, 0), &until(at(21, 0, 0)), 0),
		Some(day(20))
	);
	assert_eq!(
		until_day(&series(23, 0), &until(at(20, 23, 0)), 4),
		Some(day(19))
	);
	// Half past midnight, with the planning day starting at 4: the 19th's
	// occurrence is on the 20th's date.
	assert_eq!(
		until_day(&series(0, 30), &until(at(20, 0, 30)), 4),
		Some(day(18))
	);
	assert_eq!(until_day(&series(9, 0), "20261120", 0), Some(day(20)));
}

// The code that talks to the real Google, run against the fake's REST API.

/// Reconciles through `GoogleRemote`, the real Google client, with its
/// requests answered by the fake's REST API.
fn reconcile_through_the_api(google: FakeGoogle) -> (FakeGoogle, Vec<Fix>, Vec<Fix>) {
	use std::{cell::RefCell, rc::Rc};
	let shared = Rc::new(RefCell::new(google));
	let answering = shared.clone();
	let settings = crate::config::CalendarSettings {
		app_calendar: Some(CALENDAR.into()),
		day_end_hour: 0,
		..Default::default()
	};
	let (first, last) = window();
	let (fixes, again) = crate::calendar::api::with_transport(
		Box::new(move |method, url, body| answering.borrow_mut().answer(method, url, body)),
		|| {
			let mut remote = crate::calendar::api::block_sync::GoogleRemote::new(
				"token".into(),
				&settings,
				CALENDAR,
			);
			let fixes = reconcile(&mut remote, CALENDAR, first, last, 0, day(TODAY)).unwrap();
			let again = reconcile(&mut remote, CALENDAR, first, last, 0, day(TODAY)).unwrap();
			(fixes, again)
		},
	);
	let google = Rc::try_unwrap(shared).unwrap().into_inner();
	(google, fixes, again)
}

/// What the calendar holds, by task and time rather than by Google's IDs
/// (a series made through the API is named differently).
fn outcome(google: &mut FakeGoogle) -> Vec<(String, Option<i64>, DateTime<Local>, bool, bool)> {
	let (first, last) = window();
	let mut seen: Vec<_> = google
		.listing(CALENDAR, first, last)
		.unwrap()
		.events
		.into_iter()
		.map(|e| {
			let task = e.details.task_data.as_ref().map(|data| data.id);
			(e.title.clone(), task, e.start, e.all_day, e.completed)
		})
		.collect();
	seen.sort();
	seen
}

/// Sets a calendar up, settles it, then makes a change.
type Scenario = fn(&mut FakeGoogle);

fn scenarios() -> Vec<(&'static str, Scenario)> {
	vec![
		("split in this app, with done and skipped days", |google| {
			let block = google.add_block_series(at(2, 9, 0), 60, DAILY);
			let task = google.add_follower(&block, day(5), "Inbox zero", 1, 30);
			google.add_follower(&block, day(12), "Plan", 2, 15);
			settle(google, 0);
			google.complete(&task, day(22));
			google.delete_occurrence(&task, day(23));
			google.split(&block, day(20), at(20, 13, 0), None, false);
		}),
		("split on Google's site with new days", |google| {
			let block = google.add_block_series(at(2, 9, 0), 60, "FREQ=WEEKLY;BYDAY=MO,WE,FR");
			google.add_follower(&block, day(11), "Review", 1, 30);
			settle(google, 0);
			google.split(
				&block,
				day(16),
				at(17, 8, 0),
				Some("FREQ=WEEKLY;BYDAY=TU,TH"),
				true,
			);
		}),
		("retimed, with history", |google| {
			let block = google.add_block_series(at(2, 9, 0), 120, DAILY);
			google.add_follower(&block, day(5), "Inbox zero", 1, 30);
			google.add_follower(&block, day(14), "Plan", 2, 15);
			settle(google, 0);
			google.retime_series(&block, 10, 30);
		}),
		("an occurrence moved and one deleted", |google| {
			let block = google.add_block_series(at(2, 9, 0), 120, DAILY);
			google.add_follower(&block, day(12), "Inbox zero", 1, 30);
			let moved = google.occurrence_on(&block, day(15));
			google.add_one_off(&moved, "Call", 2, 20);
			let deleted = google.occurrence_on(&block, day(16));
			google.add_one_off(&deleted, "Write", 3, 20);
			settle(google, 0);
			google.move_occurrence(&block, day(15), at(16, 18, 0));
			google.delete_occurrence(&block, day(16));
		}),
		("a repeat count", |google| {
			let block = google.add_block_series(at(2, 9, 0), 60, "FREQ=DAILY;COUNT=20");
			google.add_follower(&block, day(5), "Inbox zero", 1, 30);
		}),
		("the block series deleted", |google| {
			let block = google.add_block_series(at(2, 9, 0), 60, DAILY);
			google.add_follower(&block, day(3), "Inbox zero", 1, 30);
			let later = google.add_follower(&block, day(14), "Later", 2, 30);
			settle(google, 0);
			google.complete(&later, day(15));
			google.delete_series(&block);
		}),
	]
}

#[test]
fn the_google_client_does_what_the_reconciler_asks_of_it() {
	for (name, scenario) in scenarios() {
		let mut direct = FakeGoogle::default();
		scenario(&mut direct);
		let (mut through_api, fixes, again) = reconcile_through_the_api(direct.clone());
		settle(&mut direct, 0);
		assert!(!fixes.is_empty(), "{name}: nothing to do");
		assert!(again.is_empty(), "{name}: a second pass wrote {again:#?}");
		let problems = check(&mut through_api, 0);
		assert!(problems.is_empty(), "{name}: {problems:#?}");
		assert_eq!(
			outcome(&mut through_api),
			outcome(&mut direct),
			"{name}: Google ends up unlike the fake"
		);
	}
}

#[test]
fn a_task_put_in_a_repeating_block_to_repeat_with_it_does_so_on_google() {
	use crate::task_edit::{self, Change};
	use std::{cell::RefCell, rc::Rc};
	let mut google = FakeGoogle::default();
	let block = google.add_block_series(at(2, 9, 0), 120, DAILY);
	let first = google.add_follower(&block, day(5), "Inbox zero", 1, 30);
	settle(&mut google, 0);
	let task = google.add_calendar_task(day(12), "Plan", 15);
	let task_id = task.details.task_data.as_ref().unwrap().id;
	let occurrence = google.occurrence_on(&block, day(12));
	let shared = Rc::new(RefCell::new(google));
	let answering = shared.clone();
	let settings = crate::config::CalendarSettings {
		app_calendar: Some(CALENDAR.into()),
		..Default::default()
	};
	// What the apps send: into the block, then repeating with it.
	crate::calendar::api::with_transport(
		Box::new(move |method, url, body| answering.borrow_mut().answer(method, url, body)),
		|| {
			let key = crate::timeblocking::block_key(&occurrence);
			let placed = task_edit::send(
				task,
				&settings,
				Change::Block(key, occurrence.start, occurrence.end),
				9,
				"token",
			)
			.unwrap();
			let join = Change::join_block_series(&occurrence).unwrap();
			task_edit::send(placed, &settings, join, 9, "token").unwrap();
		},
	);
	let google = Rc::try_unwrap(shared).unwrap().into_inner();
	let (mut google, _, again) = reconcile_through_the_api(google);
	assert!(again.is_empty(), "{again:#?}");
	assert!(
		check(&mut google, 0).is_empty(),
		"{:#?}",
		check(&mut google, 0)
	);
	let plan = google.followers_of(task_id);
	assert_eq!(
		plan.len(),
		1,
		"the task is one series repeating with the block"
	);
	assert_eq!(titles_on(&mut google, &block, 20), ["Inbox zero", "Plan"]);
	assert_eq!(task_on(&google, &first, 20).unwrap().start, at(20, 9, 0));
	assert!(on(&google, task_id, 11).is_none());
	assert_eq!(on(&google, task_id, 20).unwrap().start, at(20, 9, 30));
}
