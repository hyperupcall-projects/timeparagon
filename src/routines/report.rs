//! The numbers a reflection shows: what got done in a period, where the time
//! went, and how the habits and the routines themselves held up.
//!
//! Everything is worked out from what the app already has: the tasks (every
//! fetch brings every task, whatever its date), the events of the period's
//! days as they were last fetched, the work log, the habits and what the
//! routines wrote down. [`build`] reads nothing from disk; the caller loads
//! the days (see [`super::load_days`]).

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Duration, Local, NaiveDate};

use crate::app::State;
use crate::calendar::{self, Event};
use crate::timeblocking::{self as tb, Priority, UNDATED};

use super::{Highlight, Period, Scale};

/// A count and the minutes it adds up to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Span {
	pub count: usize,
	pub minutes: u32,
}

/// One day of the period.
#[derive(Debug, Clone, PartialEq)]
pub struct Day {
	pub day: NaiveDate,
	pub done: usize,
	pub tracked_minutes: u32,
	pub meeting_minutes: u32,
	/// Whether this day's events are known.
	pub known: bool,
}

/// A task finished in the period.
#[derive(Debug, Clone, PartialEq)]
pub struct Done {
	pub key: String,
	/// The event id, to open its details.
	pub id: String,
	pub title: String,
	pub day: NaiveDate,
	/// The estimate, in minutes. Zero when it had none.
	pub estimate: u32,
	/// Time recorded working on it in the period, in seconds.
	pub tracked: u64,
	pub projects: Vec<u64>,
	pub priority: Option<Priority>,
}

/// Time recorded on one task.
#[derive(Debug, Clone, PartialEq)]
pub struct Tracked {
	pub key: String,
	pub title: String,
	pub seconds: u64,
}

/// Where the time and the finished tasks went, by project.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectTime {
	/// `None` for tasks without a project.
	pub project: Option<u64>,
	pub seconds: u64,
	pub done: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Habits {
	/// Days a habit was kept: done, or on an "avoid" habit, avoided.
	pub kept: u32,
	/// Days a habit was expected and was not kept.
	pub missed: u32,
	/// The habits kept most often, with how many days.
	pub best: Vec<(String, u32)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sleep {
	pub nights: u32,
	pub average_minutes: u32,
}

/// What the morning's plan committed to, and how much of it got done.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PlanCheck {
	pub planned: usize,
	pub done: usize,
	pub minutes: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report {
	pub period: Period,
	pub days: Vec<Day>,
	/// Newest first.
	pub done: Vec<Done>,
	/// Tasks planned for a day in the period, finished or not.
	pub planned: usize,
	/// Of those, how many were finished.
	pub planned_done: usize,
	/// Tasks of the period's days so far that are still open.
	pub open: usize,
	/// Tasks of the period that were let go.
	pub ignored: usize,
	/// The estimates of the finished tasks, in minutes.
	pub done_minutes: u32,
	/// Time recorded working, in seconds.
	pub tracked: u64,
	/// Most first.
	pub by_task: Vec<Tracked>,
	/// Most time first.
	pub by_project: Vec<ProjectTime>,
	pub meetings: Span,
	pub blocks: Span,
	pub distractions: Span,
	pub habits: Habits,
	pub sleep: Option<Sleep>,
	/// Each day's rating, from its reflection.
	pub ratings: Vec<(NaiveDate, u8)>,
	/// The highlights picked in each day's reflection.
	pub highlights: Vec<(NaiveDate, Highlight)>,
	/// Each day's intention, and whether it was met.
	pub intentions: Vec<(NaiveDate, String, Option<bool>)>,
	/// Days planned, refreshed and reflected on with the routines.
	pub planned_days: usize,
	pub refreshed_days: usize,
	pub reflected_days: usize,
	/// For a single day: the plan made that morning.
	pub plan: Option<PlanCheck>,
	/// Days in the period up to today.
	pub elapsed_days: usize,
	/// Of those, how many have their events loaded.
	pub known_days: usize,
}

impl Report {
	/// Finished out of planned, as a percentage, when anything was planned.
	pub fn completion(&self) -> Option<u32> {
		(self.planned > 0)
			.then(|| (self.planned_done as f64 * 100.0 / self.planned as f64).round() as u32)
	}

	/// The average of the days' ratings.
	pub fn average_rating(&self) -> Option<f64> {
		(!self.ratings.is_empty()).then(|| {
			self.ratings
				.iter()
				.map(|(_, rating)| f64::from(*rating))
				.sum::<f64>()
				/ self.ratings.len() as f64
		})
	}

	/// Kept out of expected, as a percentage.
	pub fn habit_rate(&self) -> Option<u32> {
		let total = self.habits.kept + self.habits.missed;
		(total > 0).then(|| (f64::from(self.habits.kept) * 100.0 / f64::from(total)).round() as u32)
	}
}

/// Whether a task counts in the routines: habits planned as tasks have their
/// own tracking, so they are left out.
pub fn countable(task: &Event) -> bool {
	task.is_task() && tb::metadata(task).habit_id.is_none()
}

/// The planning day a task is on, or `None` while it has no date.
pub fn task_day(task: &Event, day_start_hour: u32) -> Option<NaiveDate> {
	let date = task.start.date_naive();
	if date == UNDATED {
		None
	} else if task.all_day {
		Some(date)
	} else {
		Some(calendar::day_at(task.start, day_start_hour))
	}
}

/// The day a finished task counts towards: the day it was planned for, or,
/// for one that never had a day, the day it was last changed, which for a
/// finished task is when it was ticked off.
pub fn done_day(task: &Event, day_start_hour: u32) -> Option<NaiveDate> {
	task_day(task, day_start_hour).or_else(|| {
		task.details
			.task_updated_at
			.map(|at| calendar::day_at(at.with_timezone(&Local), day_start_hour))
	})
}

/// Where a period starts and ends, at the configured day boundary.
pub fn window(period: Period, day_start_hour: u32) -> (DateTime<Local>, DateTime<Local>) {
	(
		calendar::at_clock(period.first, day_start_hour, 0),
		calendar::at_clock(period.last() + Duration::days(1), day_start_hour, 0),
	)
}

/// Minutes of `(start, end)` that fall inside `window`.
fn clipped(
	start: DateTime<Local>,
	end: DateTime<Local>,
	window: (DateTime<Local>, DateTime<Local>),
) -> i64 {
	(end.min(window.1) - start.max(window.0))
		.num_minutes()
		.max(0)
}

/// Whether an event takes up time the way a meeting does: timed, not a task,
/// a habit, a night of sleep or a time block, and not declined.
pub fn is_meeting(event: &Event, settings: &crate::config::CalendarSettings) -> bool {
	!event.is_task()
		&& !event.all_day
		&& !event.details.habit
		&& event.details.sleep.is_none()
		&& event.details.status != "cancelled"
		&& event.details.attendance.own_response() != Some(calendar::meeting::Response::Declined)
		&& !tb::is_block(event, settings)
}

/// Sorted, non-overlapping spans covering every interval given.
pub fn merged(mut spans: super::Spans) -> super::Spans {
	spans.sort();
	let mut out: super::Spans = Vec::new();
	for (start, end) in spans {
		match out.last_mut() {
			Some(last) if start <= last.1 => last.1 = last.1.max(end),
			_ => out.push((start, end)),
		}
	}
	out
}

pub fn build(state: &State, period: Period, days: &BTreeMap<NaiveDate, Vec<Event>>) -> Report {
	let settings = &state.config.calendar;
	let hour = settings.day_start_hour();
	let today = super::today(state);
	let span = window(period, hour);

	// Every task once, the freshest copy first: the loaded tasks, then the
	// occurrences of repeating ones only the day caches hold.
	let mut seen = HashSet::new();
	let tasks: Vec<&Event> = state
		.calendar
		.events
		.iter()
		.chain(days.values().flatten())
		.filter(|event| countable(event) && seen.insert(tb::task_key(event)))
		.collect();
	let by_key: HashMap<String, &Event> = tasks
		.iter()
		.map(|task| (tb::task_key(task), *task))
		.collect();

	// Time recorded in the period, by task.
	let now = crate::clock::utc_now();
	let mut tracked_by: HashMap<String, Tracked> = HashMap::new();
	let mut tracked_by_day: HashMap<NaiveDate, u64> = HashMap::new();
	for session in &state.config.task_work.sessions {
		let start = session.started_at.with_timezone(&Local);
		let end = session.stopped_at.unwrap_or(now).with_timezone(&Local);
		let minutes = clipped(start, end, span);
		if minutes <= 0 {
			continue;
		}
		let seconds = minutes as u64 * 60;
		*tracked_by_day
			.entry(calendar::day_at(start.max(span.0), hour))
			.or_default() += seconds;
		let entry = tracked_by
			.entry(session.task_key.clone())
			.or_insert_with(|| Tracked {
				key: session.task_key.clone(),
				title: session.task_title.clone(),
				seconds: 0,
			});
		entry.seconds += seconds;
		entry.title = session.task_title.clone();
	}

	let mut done = Vec::new();
	let (mut planned, mut planned_done, mut open, mut ignored) = (0, 0, 0, 0);
	for task in &tasks {
		let status = tb::task_status(task);
		let planned_day = task_day(task, hour).filter(|day| period.contains(*day));
		if let Some(day) = planned_day {
			match status {
				tb::TaskStatus::Ignored => ignored += 1,
				tb::TaskStatus::Completed => {
					planned += 1;
					planned_done += 1;
				}
				tb::TaskStatus::Active => {
					planned += 1;
					if day <= today {
						open += 1;
					}
				}
			}
		}
		if status != tb::TaskStatus::Completed {
			continue;
		}
		let Some(day) = done_day(task, hour).filter(|day| period.contains(*day)) else {
			continue;
		};
		let metadata = tb::metadata(task);
		let key = tb::task_key(task);
		done.push(Done {
			tracked: tracked_by.get(&key).map_or(0, |t| t.seconds),
			key,
			id: task.id.clone(),
			title: task.title.clone(),
			day,
			estimate: metadata.minutes,
			projects: metadata.projects,
			priority: metadata.priority,
		});
	}
	done.sort_by(|a, b| b.day.cmp(&a.day).then_with(|| a.title.cmp(&b.title)));
	let done_minutes = done.iter().map(|task| task.estimate).sum();

	let mut by_task: Vec<Tracked> = tracked_by.into_values().collect();
	by_task.sort_by(|a, b| {
		b.seconds
			.cmp(&a.seconds)
			.then_with(|| a.title.cmp(&b.title))
	});
	let tracked = by_task.iter().map(|task| task.seconds).sum();

	// By the first project each task is filed under.
	let project_of = |key: &str| {
		by_key
			.get(key)
			.and_then(|task| tb::metadata(task).projects.first().copied())
	};
	let mut projects: HashMap<Option<u64>, ProjectTime> = HashMap::new();
	for task in &by_task {
		let project = project_of(&task.key);
		projects
			.entry(project)
			.or_insert(ProjectTime {
				project,
				seconds: 0,
				done: 0,
			})
			.seconds += task.seconds;
	}
	for task in &done {
		let project = task.projects.first().copied();
		projects
			.entry(project)
			.or_insert(ProjectTime {
				project,
				seconds: 0,
				done: 0,
			})
			.done += 1;
	}
	let mut by_project: Vec<ProjectTime> = projects.into_values().collect();
	by_project.sort_by(|a, b| {
		b.seconds
			.cmp(&a.seconds)
			.then(b.done.cmp(&a.done))
			.then(a.project.is_none().cmp(&b.project.is_none()))
			.then(a.project.cmp(&b.project))
	});

	// Meetings and time blocks, from the events of each known day.
	let mut meetings = Span::default();
	let mut blocks = Span::default();
	let mut meeting_minutes_by_day: HashMap<NaiveDate, u32> = HashMap::new();
	let mut nights: HashMap<NaiveDate, i64> = HashMap::new();
	for (&day, events) in days.iter().filter(|(day, _)| period.contains(**day)) {
		let day_window = (
			calendar::at_clock(day, hour, 0),
			calendar::at_clock(day + Duration::days(1), hour, 0),
		);
		let mut seen = HashSet::new();
		let mut spans = Vec::new();
		for event in events {
			if !seen.insert((&event.calendar_id, &event.id)) {
				continue;
			}
			if let Some(sleep) = &event.details.sleep
				&& period.contains(sleep.wake_date)
				&& let (Some(start), Some(end)) = (sleep.actual_start, sleep.actual_end)
			{
				nights.insert(sleep.wake_date, (end - start).num_minutes().max(0));
			}
			let minutes = clipped(event.start, event.end, day_window);
			if minutes <= 0 || event.all_day {
				continue;
			}
			if is_meeting(event, settings) {
				meetings.count += 1;
				spans.push((event.start.max(day_window.0), event.end.min(day_window.1)));
			} else if !event.is_task() && tb::is_block(event, settings) {
				blocks.count += 1;
				blocks.minutes += minutes as u32;
			}
		}
		let busy: i64 = merged(spans)
			.into_iter()
			.map(|(start, end)| (end - start).num_minutes())
			.sum();
		meetings.minutes += busy as u32;
		meeting_minutes_by_day.insert(day, busy as u32);
	}
	let sleep = (!nights.is_empty()).then(|| Sleep {
		nights: nights.len() as u32,
		average_minutes: (nights.values().sum::<i64>() / nights.len() as i64) as u32,
	});

	let distractions = state
		.config
		.task_work
		.distractions
		.iter()
		.filter(|d| {
			let at = d.recorded_at.with_timezone(&Local);
			span.0 <= at && at < span.1
		})
		.fold(Span::default(), |span, d| Span {
			count: span.count + 1,
			minutes: span.minutes + d.minutes,
		});

	let elapsed: Vec<NaiveDate> = period.days().into_iter().filter(|d| *d <= today).collect();
	let habits = habits(state, &elapsed);

	// What the routines wrote for each day.
	let routines = &state.config.routines;
	let (mut ratings, mut highlights, mut intentions) = (Vec::new(), Vec::new(), Vec::new());
	let (mut planned_days, mut refreshed_days, mut reflected_days) = (0, 0, 0);
	for day in period.days() {
		let Some(entry) = routines.get(Period::day(day)) else {
			continue;
		};
		if let Some(rating) = entry.rating {
			ratings.push((day, rating));
		}
		highlights.extend(entry.highlights.iter().map(|h| (day, h.clone())));
		if !entry.intention.trim().is_empty() {
			intentions.push((day, entry.intention.clone(), entry.intention_met));
		}
		planned_days += usize::from(entry.planned_at.is_some());
		refreshed_days += usize::from(!entry.refreshes.is_empty());
		reflected_days += usize::from(entry.reflected_at.is_some());
	}
	let plan = (period.scale == Scale::Day)
		.then(|| routines.get(period))
		.flatten()
		.filter(|entry| entry.planned_at.is_some())
		.map(|entry| PlanCheck {
			planned: entry.planned_tasks.len(),
			done: entry
				.planned_tasks
				.iter()
				.filter(|key| {
					by_key
						.get(key.as_str())
						.is_some_and(|task| tb::task_status(task) == tb::TaskStatus::Completed)
				})
				.count(),
			minutes: entry.planned_minutes,
		});

	let days_out = period
		.days()
		.into_iter()
		.map(|day| Day {
			day,
			done: done.iter().filter(|task| task.day == day).count(),
			tracked_minutes: (tracked_by_day.get(&day).copied().unwrap_or(0) / 60) as u32,
			meeting_minutes: meeting_minutes_by_day.get(&day).copied().unwrap_or(0),
			known: days.contains_key(&day),
		})
		.collect();

	Report {
		period,
		days: days_out,
		planned,
		planned_done,
		open,
		ignored,
		done_minutes,
		tracked,
		by_task,
		by_project,
		meetings,
		blocks,
		distractions,
		habits,
		sleep,
		ratings,
		highlights,
		intentions,
		planned_days,
		refreshed_days,
		reflected_days,
		plan,
		known_days: elapsed.iter().filter(|day| days.contains_key(day)).count(),
		elapsed_days: elapsed.len(),
		done,
	}
}

/// How the habits held up on `days`.
fn habits(state: &State, days: &[NaiveDate]) -> Habits {
	use timeparagon_core::habits::Status;
	let mut out = Habits::default();
	let mut best = Vec::new();
	for habit in state
		.habits
		.library
		.habits
		.iter()
		.filter(|habit| !habit.archived)
	{
		let Some(computed) = state.habits.computed(&habit.id) else {
			continue;
		};
		let mut kept = 0;
		for &day in days {
			match computed.status(day) {
				Status::Yes | Status::Clean => kept += 1,
				Status::No | Status::Partial(_) | Status::Slipped(_) => out.missed += 1,
				Status::Auto | Status::Skip | Status::Unknown => {}
			}
		}
		out.kept += kept;
		if kept > 0 {
			best.push((habit.name.clone(), kept));
		}
	}
	best.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
	best.truncate(3);
	out.best = best;
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::task_db::TaskData;
	use chrono::TimeZone;

	fn day(d: u32) -> NaiveDate {
		NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
	}

	fn task(id: i64, title: &str, on: Option<NaiveDate>, completed: bool) -> Event {
		let date = on.unwrap_or(UNDATED);
		let start = calendar::at_clock(date, 0, 0);
		Event {
			id: format!("task:{id}"),
			calendar_id: "tasks:list".into(),
			title: title.into(),
			start,
			end: start + Duration::days(1),
			all_day: true,
			color: calendar::colors::TASK,
			completed,
			details: calendar::Details {
				task_data: Some(TaskData {
					id,
					metadata: tb::Metadata {
						minutes: 30,
						..Default::default()
					},
				}),
				..Default::default()
			},
		}
	}

	fn meeting(id: &str, on: NaiveDate, from: u32, to: u32) -> Event {
		Event {
			id: id.into(),
			calendar_id: "work".into(),
			title: id.into(),
			start: calendar::at_clock(on, from, 0),
			end: calendar::at_clock(on, to, 0),
			all_day: false,
			color: calendar::colors::TASK,
			completed: false,
			details: Default::default(),
		}
	}

	#[test]
	fn a_day_counts_what_was_planned_finished_and_left_open() {
		let mut state = State::blank();
		state.calendar.events = vec![
			task(100, "Write report", Some(day(12)), true),
			task(101, "Call bank", Some(day(12)), false),
			task(102, "Tomorrow's thing", Some(day(13)), false),
			// An undated task ticked off that day counts towards it.
			{
				let mut done = task(103, "Inbox zero", None, true);
				done.details.task_updated_at = Some(
					Local
						.from_local_datetime(&day(12).and_hms_opt(15, 0, 0).unwrap())
						.unwrap()
						.with_timezone(&chrono::Utc),
				);
				done
			},
			{
				let mut ignored = task(104, "Let go", Some(day(12)), true);
				tb::apply_status(&mut ignored, tb::TaskStatus::Ignored);
				ignored
			},
		];
		let report = build(&state, Period::day(day(12)), &BTreeMap::new());
		let titles: Vec<_> = report.done.iter().map(|t| t.title.as_str()).collect();
		assert_eq!(titles, ["Inbox zero", "Write report"]);
		assert_eq!(report.planned, 2);
		assert_eq!(report.planned_done, 1);
		assert_eq!(report.ignored, 1);
		assert_eq!(report.completion(), Some(50));
		assert_eq!(report.done_minutes, 60);
	}

	#[test]
	fn overlapping_meetings_count_once_and_blocks_apart() {
		let mut state = State::blank();
		state.config.calendar.app_calendar = Some("app".into());
		let mut block = meeting("block", day(12), 13, 15);
		block.calendar_id = "app".into();
		let days = BTreeMap::from([(
			day(12),
			vec![
				meeting("standup", day(12), 9, 10),
				meeting("overlap", day(12), 9, 11),
				block,
			],
		)]);
		let report = build(&state, Period::day(day(12)), &days);
		assert_eq!(report.meetings.count, 2);
		assert_eq!(report.meetings.minutes, 120);
		assert_eq!(report.blocks.count, 1);
		assert_eq!(report.blocks.minutes, 120);
	}

	#[test]
	fn work_sessions_are_clipped_to_the_period_and_grouped_by_task() {
		let mut state = State::blank();
		let at = |h: u32| {
			Local
				.from_local_datetime(&day(12).and_hms_opt(h, 0, 0).unwrap())
				.unwrap()
				.with_timezone(&chrono::Utc)
		};
		state.config.task_work.sessions = vec![
			crate::task_work::Session {
				task_key: "100".into(),
				task_title: "Write report".into(),
				block_key: None,
				started_at: at(9),
				stopped_at: Some(at(11)),
			},
			crate::task_work::Session {
				task_key: "101".into(),
				task_title: "Email".into(),
				block_key: None,
				started_at: at(14),
				stopped_at: Some(at(15)),
			},
		];
		let report = build(&state, Period::day(day(12)), &BTreeMap::new());
		assert_eq!(report.tracked, 3 * 3600);
		assert_eq!(report.by_task[0].title, "Write report");
		assert_eq!(report.by_task[0].seconds, 2 * 3600);
		let other = build(&state, Period::day(day(13)), &BTreeMap::new());
		assert_eq!(other.tracked, 0);
	}

	#[test]
	fn a_week_gathers_the_days_ratings_and_highlights() {
		let mut state = State::blank();
		let week = Period::of(
			Scale::Week,
			day(12),
			crate::calendar::first_day::FirstDayOfWeek::Monday,
		);
		let entry = state.config.routines.entry_mut(Period::day(day(10)));
		entry.rating = Some(4);
		entry.highlights.push(Highlight {
			id: "100".into(),
			title: "Shipped".into(),
		});
		entry.planned_at = Some(chrono::Utc::now());
		state.config.routines.entry_mut(Period::day(day(11))).rating = Some(2);
		// Outside the week.
		state.config.routines.entry_mut(Period::day(day(16))).rating = Some(5);
		let report = build(&state, week, &BTreeMap::new());
		assert_eq!(report.ratings.len(), 2);
		assert_eq!(report.average_rating(), Some(3.0));
		assert_eq!(report.highlights.len(), 1);
		assert_eq!(report.planned_days, 1);
		assert_eq!(report.days.len(), 7);
	}

	#[test]
	fn merging_joins_overlapping_and_touching_spans() {
		let at = |h| calendar::at_clock(day(12), h, 0);
		let spans = merged(vec![(at(13), at(14)), (at(9), at(10)), (at(10), at(11))]);
		assert_eq!(spans, vec![(at(9), at(11)), (at(13), at(14))]);
	}
}
