//! Renders the interface at each breakpoint and theme, headlessly.
//!
//! Run with `cargo run --example screenshots`. The PNGs land in
//! `screenshots/` and are how the narrow layouts get eyeballed without
//! resizing a window by hand forty times.

use iced::{Element, Size};
use iced_test::Simulator;
use iced_test::core::Settings;

use timeparagon::app::{self, Message, Screen, State};
use timeparagon::config::Config;
use timeparagon::routines::Routine;
use timeparagon::theme::Catalog;

/// The widths the layout is designed around.
const WIDTHS: [f32; 4] = [180.0, 200.0, 260.0, 420.0];

/// Every selectable theme.
const THEMES: [&str; 4] = ["Light", "Light high contrast", "Dark", "Dark high contrast"];

fn main() {
	let directory = std::path::Path::new("screenshots");
	std::fs::create_dir_all(directory).expect("create the screenshots directory");

	// Start from a clean slate so old renders are actually replaced.
	for entry in std::fs::read_dir(directory).expect("read the screenshots directory") {
		let path = entry.expect("read a directory entry").path();

		if path.extension().is_some_and(|extension| extension == "png") {
			std::fs::remove_file(path).expect("remove a stale screenshot");
		}
	}

	let mut count = 0;

	for theme in THEMES {
		for width in WIDTHS {
			let size = Size::new(width, 360.0);

			count += shoot(directory, theme, size, "finished", |state| {
				state.alarm_active = true;
			});

			count += shoot(directory, theme, size, "alarm-mode", |state| {
				state.alarms.mode = true;
			});

			count += shoot(directory, theme, size, "alarm-popup", |state| {
				let at = timeparagon::alarms::minute_of(chrono::Local::now())
					+ chrono::Duration::minutes(45);
				let id = state.alarms.add(at, state.config.alarm.clone());
				let _ = app::update(
					state,
					Message::Alarms(timeparagon::alarms::Action::Open(id)),
				);
			});

			count += shoot(directory, theme, size, "settings", |state| {
				state.screen = Screen::Settings;
			});

			count += shoot(
				directory,
				theme,
				Size::new(width, 720.0),
				"calendar",
				|state| {
					state.screen = Screen::Calendar;
					state.calendar = timeparagon::calendar::Store::with_snapshot(
						timeparagon::calendar::sample_snapshot(),
					);
				},
			);

			// A Google summary is free text, and it is often far longer than
			// the event it names. This one brings its own line breaks and a
			// word too wide for the block, so the labels can be seen staying
			// inside their boxes.
			count += shoot(
				directory,
				theme,
				Size::new(width, 720.0),
				"calendar-long-title",
				|state| {
					state.screen = Screen::Calendar;

					let mut snapshot = timeparagon::calendar::sample_snapshot();
					let day = snapshot.day;
					let at = |hour, minute| {
						day.and_hms_opt(hour, minute, 0)
							.and_then(|naive| naive.and_local_timezone(chrono::Local).single())
							.expect("a local time")
					};
					let work = snapshot
						.calendars
						.iter()
						.find(|calendar| calendar.id == "work")
						.expect("the work calendar");

					// A quarter of an hour is barely a line of text, which is
					// the case that used to spill over the events below it.
					snapshot.events.push(timeparagon::calendar::Event {
                        id: "long-title".into(),
                        calendar_id: work.id.clone(),
                        title: "17-4 HW10/22 (Thu)2pm: L18\nReading.Inaka vs Tokai-->Changed\nfrom the L18 department store\nSupercalifragilisticexpialidocious".into(),
                        start: at(7, 0),
                        end: at(7, 15),
                        all_day: false,
                        color: work.color,
                        completed: false,
                        details: Default::default(),
                    });

					state.calendar = timeparagon::calendar::Store::with_snapshot(snapshot);
				},
			);

			count += shoot(
				directory,
				theme,
				Size::new(width, 720.0),
				"calendar-month",
				|state| {
					state.screen = Screen::Calendar;
					state.calendar = timeparagon::calendar::Store::with_snapshot(
						timeparagon::calendar::sample_snapshot(),
					);
					state.calendar.month_open = true;
				},
			);

			count += shoot(
				directory,
				theme,
				Size::new(width, 720.0),
				"notepad",
				|state| {
					state.screen = Screen::Notepad;
					state.set_notepad(
                    "Things to sort out before Friday:\n\n- the draft\n- the numbers\n- who is picking up the room",
                );
				},
			);

			// The text at its largest, which is the size the narrow layout has
			// to survive.
			count += shoot(
				directory,
				theme,
				Size::new(width, 720.0),
				"notepad-large",
				|state| {
					state.screen = Screen::Notepad;
					state.config.notepad.font_size = timeparagon::config::Notepad::MAX_FONT_SIZE;
					state.set_notepad(
						"Things to sort out before Friday:\n\n- the draft\n- the numbers"
							.to_owned(),
					);
				},
			);

			count += shoot(
				directory,
				theme,
				Size::new(width, 720.0),
				"notepad-small",
				|state| {
					state.screen = Screen::Notepad;
					state.config.notepad.font_size = timeparagon::config::Notepad::MIN_FONT_SIZE;
					state.set_notepad(
						"Things to sort out before Friday:\n\n- the draft\n- the numbers"
							.to_owned(),
					);
				},
			);
		}
	}

	// And the widest layout, where the settings sit beside the calendar.
	for theme in THEMES {
		count += shoot(
			directory,
			theme,
			Size::new(900.0, 560.0),
			"wide-settings",
			|state| {
				state.screen = Screen::Settings;
			},
		);
	}

	// The routines, with a day's worth of tasks, meetings and work to show.
	for theme in ["Light", "Dark"] {
		for width in [260.0, 420.0] {
			let size = Size::new(width, 1100.0);
			let steps: [(&str, Routine, usize); 8] = [
				("routines-overview", Routine::DailyPlan, usize::MAX),
				("routines-plan-review", Routine::DailyPlan, 0),
				("routines-plan-choose", Routine::DailyPlan, 1),
				("routines-plan-estimate", Routine::DailyPlan, 2),
				("routines-refresh-card", Routine::DailyRefresh, 1),
				("routines-reflection", Routine::DailyReflection, 0),
				("routines-week-report", Routine::WeeklyReflection, 0),
				("routines-week-schedule", Routine::WeeklyPlan, 2),
			];
			for (name, routine, step) in steps {
				count += shoot(directory, theme, size, name, |state| {
					routine_sample(state);
					if step == usize::MAX {
						return;
					}
					let _ = app::update(
						state,
						Message::Routines(timeparagon::routines::Action::Start(routine)),
					);
					let action = if routine == Routine::DailyRefresh {
						timeparagon::routines::Action::Next
					} else {
						timeparagon::routines::Action::Step(step)
					};
					let _ = app::update(state, Message::Routines(action));
				});
			}
		}
	}

	println!("wrote {count} screenshots to {}", directory.display());
}

/// Today's tasks, meetings, a time block, some work done and what the
/// routines wrote down, all relative to today so the figures fill in.
fn routine_sample(state: &mut State) {
	use chrono::Duration;
	use timeparagon::calendar::{self, Event};
	use timeparagon::routines::{Objective, Period, Scale};
	use timeparagon::timeblocking as tb;

	state.screen = Screen::Routines;
	state.config.calendar.inbox_list = Some("tasks:inbox".into());
	state.config.calendar.app_calendar = Some("app".into());
	let today = timeparagon::routines::today(state);
	let yesterday = today - Duration::days(1);
	let work = state
		.config
		.projects
		.items
		.iter()
		.find(|p| p.name == "Work")
		.map(|p| p.id);
	let task = |id: i64, title: &str, list: &str, day: Option<chrono::NaiveDate>, minutes: u32| {
		let start = calendar::at_clock(day.unwrap_or(tb::UNDATED), 0, 0);
		Event {
			id: format!("task:{id}"),
			calendar_id: format!("tasks:{list}"),
			title: title.into(),
			start,
			end: start + Duration::days(1),
			all_day: true,
			color: calendar::colors::TASK,
			completed: false,
			details: calendar::Details {
				task_data: Some(timeparagon::task_db::TaskData {
					id,
					metadata: tb::Metadata {
						minutes,
						projects: work.filter(|_| list == "work").into_iter().collect(),
						..Default::default()
					},
				}),
				..Default::default()
			},
		}
	};
	let event = |id: &str, calendar: &str, title: &str, from: u32, to: u32| Event {
		id: id.into(),
		calendar_id: calendar.into(),
		title: title.into(),
		start: calendar::at_clock(today, from, 0),
		end: calendar::at_clock(today, to, 0),
		all_day: false,
		color: calendar::colors::TASK,
		completed: false,
		details: Default::default(),
	};
	let mut overdue = task(110, "Submit the expense report", "work", None, 20);
	overdue
		.details
		.task_data
		.as_mut()
		.unwrap()
		.metadata
		.deadline = Some(calendar::deadline::Deadline {
		date: yesterday,
		time: None,
	});
	let mut finished = task(111, "Review the launch checklist", "work", Some(today), 45);
	finished.completed = true;
	let mut parked = task(112, "Plan the team offsite", "backlog", None, 60);
	parked
		.details
		.task_data
		.as_mut()
		.unwrap()
		.metadata
		.backlog_section = tb::BacklogSection::NextWeekOrTwo;
	state.calendar.events = vec![
		event("standup", "work", "Standup", 9, 10),
		event("block", "app", "Deep work", 10, 12),
		event("review", "work", "Design review", 14, 15),
		task(100, "Write the quarterly update", "work", Some(today), 90),
		task(
			101,
			"Reply to Sam about the contract",
			"work",
			Some(today),
			15,
		),
		task(102, "Book the dentist", "personal", Some(today), 0),
		task(103, "Fix the flaky sync test", "work", Some(yesterday), 60),
		task(104, "Read the onboarding doc", "inbox", None, 0),
		task(105, "Look into the billing alert", "inbox", None, 30),
		overdue,
		finished,
		parked,
	];
	let at = |hour: u32| calendar::at_clock(today, hour, 0).with_timezone(&chrono::Utc);
	state.config.task_work.sessions = vec![timeparagon::task_work::Session {
		task_key: "111".into(),
		task_title: "Review the launch checklist".into(),
		block_key: None,
		started_at: at(10),
		stopped_at: Some(at(11)),
	}];
	let first_day = state.config.calendar.first_day_of_week;
	let week = Period::of(Scale::Week, today, first_day);
	let entry = state.config.routines.entry_mut(week);
	entry.intention = "Get the launch over the line".into();
	for (id, text) in [
		(1, "Launch the beta"),
		(2, "Hire a designer"),
		(3, "Run three times"),
	] {
		entry.objectives.push(Objective {
			id,
			text: text.into(),
			done: id == 3,
		});
	}
	let entry = state.config.routines.entry_mut(Period::day(yesterday));
	entry.intention = "Finish the slides".into();
	entry.intention_met = Some(true);
	entry.rating = Some(4);
	entry.answers.insert(
		"tomorrow".into(),
		"The quarterly update, first thing".into(),
	);
	state
		.config
		.routines
		.entry_mut(Period::day(today))
		.intention = "Write the quarterly update".into();
}

/// Renders one state and writes it out.
fn shoot(
	directory: &std::path::Path,
	theme: &str,
	size: Size,
	name: &str,
	setup: impl FnOnce(&mut State),
) -> usize {
	let mut config = Config::default();
	config.appearance.theme = theme.to_owned();

	let mut state = State::new(config, Catalog::built_in());
	setup(&mut state);

	let palette = state
		.catalog
		.resolve(&state.config.appearance.theme)
		.theme
		.clone();

	let element: Element<'_, Message> = app::view(&state, iced::window::Id::unique());
	let mut ui = Simulator::with_size(Settings::default(), size, element);

	let snapshot = ui.snapshot(&palette).expect("render the interface");

	let slug = format!(
		"{}-{}-{name}",
		theme.to_lowercase().replace(' ', "-"),
		size.width as u32
	);

	snapshot
		.matches_image(directory.join(slug))
		.expect("write the screenshot");

	1
}
