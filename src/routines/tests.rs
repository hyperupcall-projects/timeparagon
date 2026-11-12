use super::*;
use crate::calendar::deadline::Deadline;
use crate::task_db::TaskData;
use crate::timeblocking::BusyKey;

fn task(id: i64, title: &str, list: &str, day: Option<NaiveDate>) -> Event {
	let start = calendar::at_clock(day.unwrap_or(UNDATED), 0, 0);
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

fn state() -> State {
	let mut state = State::blank();
	state.config.calendar.inbox_list = Some("tasks:inbox".into());
	state
}

/// One task in each of the refresh's lists, plus one in two lists and one
/// already done.
fn refresh_state() -> State {
	let mut state = state();
	let today = today(&state);
	let yesterday = today - Duration::days(1);
	let mut overdue = task(100, "Overdue in the inbox", "inbox", None);
	overdue
		.details
		.task_data
		.as_mut()
		.unwrap()
		.metadata
		.deadline = Some(Deadline {
		date: yesterday,
		time: None,
	});
	let mut done = task(104, "Done already", "inbox", None);
	done.completed = true;
	state.calendar.events = vec![
		task(103, "For today", "work", Some(today)),
		task(102, "In the inbox", "inbox", None),
		task(101, "Left from yesterday", "work", Some(yesterday)),
		overdue,
		done,
	];
	state
}

fn run(state: &mut State, action: Action) {
	let _ = crate::app::update(state, Message::Routines(action));
}

fn step(state: &State) -> Step {
	state.routines.flow.as_ref().unwrap().current()
}

#[test]
fn the_refresh_goes_through_each_list_once_in_order() {
	let mut state = refresh_state();
	run(&mut state, Action::Start(Routine::DailyRefresh));
	assert_eq!(step(&state), Step::RefreshStart);
	run(&mut state, Action::Next);
	assert_eq!(step(&state), Step::Triage);
	let cards: Vec<(&str, Section)> = state
		.routines
		.flow
		.as_ref()
		.unwrap()
		.refresh
		.cards
		.iter()
		.map(|card| (card.key.as_str(), card.section))
		.collect();
	// Past a deadline or left from an earlier day, each is asked about once,
	// as unfinished.
	assert_eq!(
		cards,
		[
			("100", Section::Unfinished),
			("101", Section::Unfinished),
			("102", Section::Inbox),
			("103", Section::Today),
		]
	);
}

#[test]
fn decisions_wait_their_turn_and_go_out_as_each_save_finishes() {
	let mut state = refresh_state();
	let today = today(&state);
	run(&mut state, Action::Start(Routine::DailyRefresh));
	run(&mut state, Action::Next);

	// Today, for an overdue task: the deadline moves with it.
	run(&mut state, Action::Decide(Decision::Day(today)));
	assert!(state.timeblocking.busy.contains_task("task:100"));
	assert_eq!(
		state.routines.queue.front(),
		Some(&Op {
			key: "100".into(),
			change: Change::Deadline(Some(Deadline {
				date: today,
				time: None
			})),
		})
	);
	// The next card is up at once, and its decision waits.
	run(
		&mut state,
		Action::Decide(Decision::Backlog(BacklogSection::Someday)),
	);
	assert_eq!(state.routines.queue.len(), 2);
	assert_eq!(state.routines.flow.as_ref().unwrap().refresh.index, 2);

	// The first save comes back, so the next change goes out.
	let mut saved = r_task(&state, "100");
	saved.start = calendar::at_clock(today, 0, 0);
	let _ = crate::app::update(
		&mut state,
		Message::TimeBlocking(tb::Action::SavedMany(
			BusyKey::Task("task:100".into()),
			Ok(vec![saved]),
		)),
	);
	assert!(state.timeblocking.busy.contains_task("task:100"));
	assert_eq!(
		state.routines.queue.front().map(|op| op.key.as_str()),
		Some("101")
	);
}

fn r_task(state: &State, key: &str) -> Event {
	task_by(state, key).clone()
}

fn task_by<'a>(state: &'a State, key: &str) -> &'a Event {
	super::task(state, key).unwrap()
}

#[test]
fn a_changed_mind_replaces_the_waiting_change() {
	let mut state = refresh_state();
	state.timeblocking.busy.insert("something-else");
	run(&mut state, Action::Start(Routine::DailyRefresh));
	run(&mut state, Action::Next);
	run(&mut state, Action::Decide(Decision::Skip));
	run(&mut state, Action::Decide(Decision::Done));
	assert_eq!(state.routines.queue.len(), 1);
	run(&mut state, Action::Back);
	run(
		&mut state,
		Action::Decide(Decision::Backlog(BacklogSection::NextMonth)),
	);
	assert_eq!(
		state.routines.queue.iter().collect::<Vec<_>>(),
		[&Op {
			key: "101".into(),
			change: Change::Backlog(BacklogSection::NextMonth),
		}]
	);
}

#[test]
fn deleting_takes_a_second_press() {
	let mut state = refresh_state();
	run(&mut state, Action::Start(Routine::DailyRefresh));
	run(&mut state, Action::Next);
	run(&mut state, Action::Decide(Decision::Delete));
	assert_eq!(state.routines.confirm_delete.as_deref(), Some("100"));
	assert_eq!(state.routines.flow.as_ref().unwrap().refresh.index, 0);
	assert!(state.timeblocking.busy.is_empty());
	run(&mut state, Action::Decide(Decision::Delete));
	assert!(state.routines.confirm_delete.is_none());
	assert!(state.timeblocking.busy.contains_task("task:100"));
	assert_eq!(state.routines.flow.as_ref().unwrap().refresh.index, 1);
}

#[test]
fn triaging_an_incomplete_task_says_why_and_stays_on_it() {
	let mut state = refresh_state();
	run(&mut state, Action::Start(Routine::DailyRefresh));
	run(&mut state, Action::Next);
	run(&mut state, Action::Card(2));
	run(&mut state, Action::Decide(Decision::Triage));
	let error = state.routines.error.as_deref().unwrap();
	assert!(error.contains("project"), "{error}");
	assert_eq!(state.routines.flow.as_ref().unwrap().refresh.index, 2);
	assert!(state.routines.queue.is_empty());
}

#[test]
fn the_refresh_is_written_down_once_when_it_ends() {
	let mut state = refresh_state();
	run(&mut state, Action::Start(Routine::DailyRefresh));
	run(&mut state, Action::Next);
	for _ in 0..4 {
		run(&mut state, Action::Decide(Decision::Skip));
	}
	assert_eq!(step(&state), Step::RefreshDone);
	let period = Period::day(today(&state));
	let refreshes = || {
		state
			.config
			.routines
			.get(period)
			.map_or(0, |e| e.refreshes.len())
	};
	assert_eq!(refreshes(), 1);
	let mut state = state;
	run(&mut state, Action::Next);
	assert!(state.routines.flow.is_none());
	assert_eq!(
		state.config.routines.get(period).unwrap().refreshes.len(),
		1
	);
	assert_eq!(
		state.config.routines.get(period).unwrap().refreshes[0].remaining,
		4
	);
}

#[test]
fn a_reflection_keeps_its_answers_and_rating() {
	let mut state = state();
	run(&mut state, Action::Start(Routine::DailyReflection));
	run(&mut state, Action::Step(2));
	assert_eq!(step(&state), Step::Reflect);
	assert_eq!(state.routines.editors.len(), 4);
	for c in "Shipped".chars() {
		run(
			&mut state,
			Action::Prompt(
				"went_well",
				text_editor::Action::Edit(text_editor::Edit::Insert(c)),
			),
		);
	}
	run(&mut state, Action::Rating(4));
	let period = Period::day(today(&state));
	let entry = state.config.routines.get(period).unwrap();
	assert_eq!(entry.answer("went_well"), "Shipped");
	assert_eq!(entry.rating, Some(4));
	// Pressing the same rating again takes it back.
	run(&mut state, Action::Rating(4));
	assert_eq!(state.config.routines.get(period).unwrap().rating, None);

	run(&mut state, Action::Next);
	assert_eq!(step(&state), Step::Wrapup);
	run(&mut state, Action::Next);
	assert!(state.routines.flow.is_none());
	assert!(
		state
			.config
			.routines
			.get(period)
			.unwrap()
			.reflected_at
			.is_some()
	);
}

#[test]
fn an_answer_changed_elsewhere_reaches_the_open_editor() {
	let mut state = state();
	run(&mut state, Action::Start(Routine::DailyReflection));
	run(&mut state, Action::Step(2));
	let period = Period::day(today(&state));
	state
		.config
		.routines
		.entry_mut(period)
		.answers
		.insert("learned".into(), "From the phone".into());
	run(&mut state, Action::Rating(3));
	assert_eq!(
		state.routines.editor("learned").unwrap().text(),
		"From the phone"
	);
}

#[test]
fn finishing_the_daily_plan_commits_to_the_days_tasks() {
	let mut state = state();
	let today = today(&state);
	let mut long = task(105, "Write", "work", Some(today));
	long.details.task_data.as_mut().unwrap().metadata.minutes = 45;
	state.calendar.events = vec![task(103, "Call", "work", Some(today)), long];
	run(&mut state, Action::Start(Routine::DailyPlan));
	run(&mut state, Action::Intention("Write".into()));
	run(&mut state, Action::Step(4));
	run(&mut state, Action::Next);
	assert!(state.routines.flow.is_none());
	let entry = state.config.routines.get(Period::day(today)).unwrap();
	assert!(entry.planned_at.is_some());
	assert_eq!(entry.planned_tasks, ["103", "105"]);
	assert_eq!(entry.planned_minutes, 75);
	assert_eq!(entry.intention, "Write");
	assert!(Routine::DailyPlan.done(Some(entry)));
}

#[test]
fn closing_keeps_the_routine_to_pick_up_again() {
	let mut state = state();
	run(&mut state, Action::Start(Routine::WeeklyPlan));
	run(&mut state, Action::Step(2));
	run(&mut state, Action::Close);
	assert!(state.routines.flow.is_none());
	assert!(state.routines.paused.is_some());
	run(&mut state, Action::Start(Routine::WeeklyPlan));
	assert_eq!(step(&state), Step::Schedule);
	assert!(state.routines.paused.is_none());
	// Starting another routine lets the paused one go.
	run(&mut state, Action::Close);
	run(&mut state, Action::Start(Routine::DailyReflection));
	assert!(state.routines.paused.is_none());
	assert_eq!(step(&state), Step::Report);
}

#[test]
fn objectives_carry_between_weeks_without_doubling() {
	let mut state = state();
	run(&mut state, Action::Start(Routine::WeeklyReflection));
	let week = state.routines.flow.as_ref().unwrap().period;
	for text in ["Ship the launch", "Run three times"] {
		run(&mut state, Action::ObjectiveDraft(text.into()));
		run(&mut state, Action::AddObjective);
	}
	let objectives = state.config.routines.get(week).unwrap().objectives.clone();
	assert_eq!(objectives.len(), 2);
	run(&mut state, Action::ObjectiveDone(objectives[1].id, true));
	run(&mut state, Action::CarryForward(objectives[0].id));
	run(&mut state, Action::CarryForward(objectives[0].id));
	let next = state.config.routines.get(week.next()).unwrap();
	assert_eq!(next.objectives.len(), 1);
	assert_eq!(next.objectives[0].text, "Ship the launch");
	assert!(!next.objectives[0].done);
	assert_ne!(next.objectives[0].id, objectives[0].id);

	// And from the other side: next week's plan carries one over.
	run(&mut state, Action::Discard);
	state.routines.flow = Some(Flow::new(Routine::WeeklyPlan, week.next()));
	run(&mut state, Action::CarryOver(objectives[1].id));
	assert_eq!(
		state
			.config
			.routines
			.get(week.next())
			.unwrap()
			.objectives
			.len(),
		2
	);
}

#[test]
fn a_refresh_started_from_the_plan_goes_back_to_it() {
	let mut state = refresh_state();
	run(&mut state, Action::Start(Routine::DailyPlan));
	assert_eq!(step(&state), Step::Review);
	run(&mut state, Action::Detour);
	assert_eq!(
		state.routines.flow.as_ref().unwrap().routine,
		Routine::DailyRefresh
	);
	run(&mut state, Action::Next);
	run(&mut state, Action::Next);
	assert_eq!(step(&state), Step::RefreshDone);
	run(&mut state, Action::Next);
	let flow = state.routines.flow.as_ref().unwrap();
	assert_eq!(flow.routine, Routine::DailyPlan);
	assert_eq!(flow.current(), Step::ChooseTasks);
}

#[test]
fn moving_everything_left_to_tomorrow_queues_each_task() {
	let mut state = state();
	let today = today(&state);
	state.calendar.events = vec![
		task(103, "Call", "work", Some(today)),
		task(105, "Write", "work", Some(today)),
	];
	run(&mut state, Action::Start(Routine::DailyReflection));
	run(
		&mut state,
		Action::Tasks(
			vec!["103".into(), "105".into()],
			Change::Day(today + Duration::days(1)),
		),
	);
	assert!(state.timeblocking.busy.contains_task("task:103"));
	assert_eq!(
		state.routines.queue.iter().collect::<Vec<_>>(),
		[&Op {
			key: "105".into(),
			change: Change::Day(today + Duration::days(1)),
		}]
	);
}

#[test]
fn the_timebox_step_shows_the_day_beside_the_panel() {
	let mut state = state();
	state.screen = crate::app::Screen::Routines;
	run(&mut state, Action::Start(Routine::DailyPlan));
	assert!(!state.day_plan_active());
	run(&mut state, Action::Step(3));
	assert_eq!(step(&state), Step::Timebox);
	assert!(state.day_plan_active());
}

#[test]
fn the_summary_lists_the_plan() {
	let mut state = state();
	let today = today(&state);
	state.calendar.events = vec![task(103, "Call the bank", "work", Some(today))];
	run(&mut state, Action::Start(Routine::DailyPlan));
	run(&mut state, Action::Intention("Call the bank".into()));
	let summary = summary(&state, state.routines.flow.as_ref().unwrap());
	assert!(summary.contains("Highlight: Call the bank"), "{summary}");
	assert!(summary.contains("- Call the bank (30m)"), "{summary}");
}

#[test]
fn a_daily_routine_shows_its_day_in_the_panel() {
	let mut state = state();
	let today = today(&state);
	let other = today - Duration::days(3);
	state.calendar.set_day(other, today);
	run(&mut state, Action::Start(Routine::DailyPlan));
	assert_eq!(state.calendar.day, today);
	// A week's routine leaves the day view alone.
	run(&mut state, Action::Discard);
	state.calendar.set_day(other, today);
	run(&mut state, Action::Start(Routine::WeeklyPlan));
	assert_eq!(state.calendar.day, other);
}

#[test]
fn a_routine_started_from_another_sets_it_aside() {
	let mut state = state();
	run(&mut state, Action::Start(Routine::DailyPlan));
	run(&mut state, Action::Step(1));
	run(&mut state, Action::Start(Routine::WeeklyPlan));
	assert_eq!(
		state.routines.flow.as_ref().unwrap().routine,
		Routine::WeeklyPlan
	);
	let paused = state.routines.paused.as_ref().unwrap();
	assert_eq!(paused.routine, Routine::DailyPlan);
	assert_eq!(paused.current(), Step::ChooseTasks);
	// Finishing the week's plan leaves the day's to continue.
	for _ in 0..4 {
		run(&mut state, Action::Next);
	}
	assert!(state.routines.flow.is_none());
	run(&mut state, Action::Resume);
	assert_eq!(step(&state), Step::ChooseTasks);
}
