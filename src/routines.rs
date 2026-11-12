//! Routines: guided rituals to plan, refresh and reflect on the day, the week
//! and the month.
//!
//! A routine is a short series of steps ([`Routine::steps`]) shown one at a
//! time. What it writes down (the day's highlight, the week's objectives, a
//! reflection's answers) is kept per period in
//! [`timeparagon_core::routines::Routines`], which syncs between devices.
//!
//! The tasks a routine moves are changed through the same actions as
//! everywhere else. The app sends one task change at a time (see
//! `timeblocking::Busy`), and a person deciding about a pile of tasks is
//! faster than that, so decisions wait their turn in [`Workspace::queue`] and
//! [`pump`] sends the next one whenever nothing else is being saved.

pub mod report;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use chrono::{DateTime, Duration, Local, NaiveDate};
use iced::Task;
use iced::widget::text_editor;

use crate::app::{CalendarMessage, Message, State, TaskMessage};
use crate::calendar::deadline::Deadline;
use crate::calendar::{self, Event};
use crate::timeblocking::{self as tb, BacklogSection, Priority, TaskStatus, UNDATED};

pub use report::{countable, task_day};
pub use timeparagon_core::routines::{Entry, Highlight, Objective, Period, Routines, Scale};

fn msg(action: Action) -> Message {
	Message::Routines(action)
}

/// One of the guided rituals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Routine {
	DailyPlan,
	DailyRefresh,
	DailyReflection,
	WeeklyPlan,
	WeeklyReflection,
	MonthlyPlan,
	MonthlyReflection,
}

impl Routine {
	pub const ALL: [Self; 7] = [
		Self::DailyPlan,
		Self::DailyRefresh,
		Self::DailyReflection,
		Self::WeeklyPlan,
		Self::WeeklyReflection,
		Self::MonthlyPlan,
		Self::MonthlyReflection,
	];

	pub fn scale(self) -> Scale {
		match self {
			Self::DailyPlan | Self::DailyRefresh | Self::DailyReflection => Scale::Day,
			Self::WeeklyPlan | Self::WeeklyReflection => Scale::Week,
			Self::MonthlyPlan | Self::MonthlyReflection => Scale::Month,
		}
	}

	pub fn title(self) -> &'static str {
		match self {
			Self::DailyPlan => "Daily plan",
			Self::DailyRefresh => "Daily refresh",
			Self::DailyReflection => "Daily reflection",
			Self::WeeklyPlan => "Weekly plan",
			Self::WeeklyReflection => "Weekly reflection",
			Self::MonthlyPlan => "Monthly plan",
			Self::MonthlyReflection => "Monthly reflection",
		}
	}

	/// What it is for, in a line.
	pub fn blurb(self) -> &'static str {
		match self {
			Self::DailyPlan => "Choose today's tasks, size them up and set a highlight",
			Self::DailyRefresh => {
				"Go through overdue, unfinished, inbox and today's tasks one by one"
			}
			Self::DailyReflection => "See what you did, deal with what's left and reflect",
			Self::WeeklyPlan => "Set the week's objectives and spread the work over its days",
			Self::WeeklyReflection => "Review the week's highlights, numbers and objectives",
			Self::MonthlyPlan => "Set the month's objectives and groom the backlog",
			Self::MonthlyReflection => "Review the month and how it moved your goals",
		}
	}

	pub fn steps(self) -> &'static [Step] {
		match self {
			Self::DailyPlan => &[
				Step::Review,
				Step::ChooseTasks,
				Step::Estimate,
				Step::Timebox,
				Step::Highlight,
			],
			Self::DailyRefresh => &[Step::RefreshStart, Step::Triage, Step::RefreshDone],
			Self::DailyReflection => &[Step::Report, Step::Unfinished, Step::Reflect, Step::Wrapup],
			Self::WeeklyPlan | Self::MonthlyPlan => &[
				Step::LookBack,
				Step::Objectives,
				Step::Schedule,
				Step::Focus,
			],
			Self::WeeklyReflection | Self::MonthlyReflection => &[
				Step::Report,
				Step::ObjectivesReview,
				Step::Reflect,
				Step::Wrapup,
			],
		}
	}

	pub fn plans(self) -> bool {
		matches!(self, Self::DailyPlan | Self::WeeklyPlan | Self::MonthlyPlan)
	}

	pub fn reflects(self) -> bool {
		matches!(
			self,
			Self::DailyReflection | Self::WeeklyReflection | Self::MonthlyReflection
		)
	}

	/// Whether it has been done for the period `entry` is for.
	pub fn done(self, entry: Option<&Entry>) -> bool {
		entry.is_some_and(|entry| {
			if self.plans() {
				entry.planned_at.is_some()
			} else if self.reflects() {
				entry.reflected_at.is_some()
			} else {
				!entry.refreshes.is_empty()
			}
		})
	}

	/// The reflection's questions, by the id their answers are kept under.
	pub fn prompts(self) -> &'static [(&'static str, &'static str)] {
		match self.scale() {
			Scale::Day => &[
				("went_well", "What went well today?"),
				("got_in_the_way", "What got in the way?"),
				("learned", "What did you learn?"),
				("tomorrow", "What's the first thing to do tomorrow?"),
			],
			Scale::Week => &[
				("wins", "What were this week's wins?"),
				("challenges", "What was hard, and why?"),
				("learned", "What did you learn?"),
				("change", "What will you do differently next week?"),
			],
			Scale::Month => &[
				("wins", "What are you proud of this month?"),
				("challenges", "What held you back?"),
				("goals", "How did this month move your goals forward?"),
				("change", "What will you change next month?"),
			],
		}
	}

	/// The prompt whose answer the next plan opens with.
	pub fn carried_prompt(scale: Scale) -> &'static str {
		match scale {
			Scale::Day => "tomorrow",
			Scale::Week | Scale::Month => "change",
		}
	}

	pub fn plan_of(scale: Scale) -> Self {
		match scale {
			Scale::Day => Self::DailyPlan,
			Scale::Week => Self::WeeklyPlan,
			Scale::Month => Self::MonthlyPlan,
		}
	}

	pub fn reflection_of(scale: Scale) -> Self {
		match scale {
			Scale::Day => Self::DailyReflection,
			Scale::Week => Self::WeeklyReflection,
			Scale::Month => Self::MonthlyReflection,
		}
	}
}

/// One screen of a routine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
	/// Daily plan: yesterday, the week's objectives, and what needs triage.
	Review,
	ChooseTasks,
	Estimate,
	Timebox,
	/// The day's one most important thing.
	Highlight,
	RefreshStart,
	/// The refresh's cards, one task at a time.
	Triage,
	RefreshDone,
	Report,
	/// Daily reflection: today's tasks still open.
	Unfinished,
	ObjectivesReview,
	Reflect,
	Wrapup,
	/// Week and month plans: how the last one went.
	LookBack,
	Objectives,
	Schedule,
	Focus,
}

impl Step {
	/// The short name shown above the step.
	pub fn title(self, scale: Scale) -> &'static str {
		match (self, scale) {
			(Self::Review, _) => "Start fresh",
			(Self::ChooseTasks, _) => "Choose tasks",
			(Self::Estimate, _) => "Size it up",
			(Self::Timebox, _) => "Timebox",
			(Self::Highlight, _) => "Highlight",
			(Self::RefreshStart, _) => "Overview",
			(Self::Triage, _) => "Decide",
			(Self::RefreshDone, _) => "Done",
			(Self::Report, _) => "Review",
			(Self::Unfinished, _) => "What's left",
			(Self::ObjectivesReview, _) => "Objectives",
			(Self::Reflect, _) => "Reflect",
			(Self::Wrapup, Scale::Day) => "Shut down",
			(Self::Wrapup, _) => "Wrap up",
			(Self::LookBack, _) => "Look back",
			(Self::Objectives, _) => "Objectives",
			(Self::Schedule, Scale::Month) => "Backlog",
			(Self::Schedule, _) => "Spread it out",
			(Self::Focus, _) => "Focus",
		}
	}

	/// The question the step asks.
	pub fn question(self, scale: Scale) -> &'static str {
		match (self, scale) {
			(Self::Review, _) => {
				"Before you plan: how did yesterday end, and is anything waiting to be sorted?"
			}
			(Self::ChooseTasks, _) => "What will you work on today?",
			(Self::Estimate, _) => "How long will each take, and does it fit in the day?",
			(Self::Timebox, _) => "When will you do them? Put tasks into today's time blocks.",
			(Self::Highlight, _) => "What's the one thing that would make today a good day?",
			(Self::RefreshStart, _) => {
				"Clear the decks: decide what happens to each task that needs attention."
			}
			(Self::Triage, _) => "Where does this task belong?",
			(Self::RefreshDone, _) => "Here's what changed.",
			(Self::Report, Scale::Day) => "Here's what you did today.",
			(Self::Report, Scale::Week) => "Here's what you did this week.",
			(Self::Report, Scale::Month) => "Here's what you did this month.",
			(Self::Unfinished, _) => "What happens to what's left?",
			(Self::ObjectivesReview, Scale::Week) => "Did you meet this week's objectives?",
			(Self::ObjectivesReview, _) => "Did you meet this month's objectives?",
			(Self::Reflect, Scale::Day) => "How did today go?",
			(Self::Reflect, Scale::Week) => "How did this week go?",
			(Self::Reflect, Scale::Month) => "How did this month go?",
			(Self::Wrapup, Scale::Day) => "You're done for today.",
			(Self::Wrapup, _) => "All wrapped up.",
			(Self::LookBack, Scale::Week) => "How did last week go?",
			(Self::LookBack, _) => "How did last month go?",
			(Self::Objectives, Scale::Week) => "What do you want to get done this week?",
			(Self::Objectives, _) => "What do you want to get done this month?",
			(Self::Schedule, Scale::Month) => "Which parked tasks belong in this month?",
			(Self::Schedule, _) => "Which days will the work go on?",
			(Self::Focus, Scale::Week) => "What's your focus this week?",
			(Self::Focus, _) => "What's your focus this month?",
		}
	}
}

/// The task lists the daily refresh goes through, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
	/// Past their deadline.
	Overdue,
	/// Planned for a day that has gone by, and not done.
	Unfinished,
	Inbox,
	/// Planned for today without a time slot.
	Today,
}

impl Section {
	pub const ALL: [Self; 4] = [Self::Overdue, Self::Unfinished, Self::Inbox, Self::Today];

	pub fn label(self) -> &'static str {
		match self {
			Self::Overdue => "Overdue",
			Self::Unfinished => "Unfinished",
			Self::Inbox => "Inbox",
			Self::Today => "Today",
		}
	}

	pub fn about(self) -> &'static str {
		match self {
			Self::Overdue => "Past their deadline",
			Self::Unfinished => "Planned for an earlier day and not done",
			Self::Inbox => "Waiting to be triaged",
			Self::Today => "Planned for today without a time",
		}
	}
}

/// What the refresh decided for a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
	Day(NaiveDate),
	Backlog(BacklogSection),
	/// Off the Inbox and onto the triaged list.
	Triage,
	/// Leave it where it is: in the inbox, or on today.
	Keep,
	Done,
	Ignore,
	Delete,
	/// Not now; it stays as it is.
	Skip,
}

/// What happens to a passed deadline when an overdue task is moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeadlinePolicy {
	/// Becomes the new day, or is cleared when the task is parked.
	#[default]
	Follow,
	Keep,
	Clear,
}

impl DeadlinePolicy {
	pub const ALL: [Self; 3] = [Self::Follow, Self::Keep, Self::Clear];

	pub fn label(self) -> &'static str {
		match self {
			Self::Follow => "Move it with the task",
			Self::Keep => "Keep it",
			Self::Clear => "Clear it",
		}
	}
}

/// One task in the refresh.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
	/// The task's key, which outlives a move between Tasks and Calendar.
	pub key: String,
	pub section: Section,
	/// The title when the refresh began, for a task that has since gone.
	pub title: String,
}

#[derive(Debug, Clone, Default)]
pub struct Refresh {
	pub cards: Vec<Card>,
	pub index: usize,
	pub decisions: HashMap<String, Decision>,
	pub deadlines: DeadlinePolicy,
	/// Whether this pass has been written down.
	pub recorded: bool,
}

impl Refresh {
	pub fn card(&self) -> Option<&Card> {
		self.cards.get(self.index)
	}

	/// How many tasks got each kind of decision.
	pub fn tally(&self) -> Vec<(&'static str, usize)> {
		let mut counts: Vec<(&'static str, usize)> = Vec::new();
		for decision in self.decisions.values() {
			let label = match decision {
				Decision::Day(_) => "Scheduled",
				Decision::Backlog(_) => "Parked in the backlog",
				Decision::Triage => "Triaged",
				Decision::Keep => "Kept",
				Decision::Done => "Completed",
				Decision::Ignore => "Ignored",
				Decision::Delete => "Deleted",
				Decision::Skip => "Skipped",
			};
			match counts.iter_mut().find(|(l, _)| *l == label) {
				Some((_, count)) => *count += 1,
				None => counts.push((label, 1)),
			}
		}
		counts.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
		counts
	}

	/// Decisions that changed something.
	pub fn decided(&self) -> usize {
		self.decisions
			.values()
			.filter(|d| !matches!(d, Decision::Skip))
			.count()
	}
}

/// A routine in progress.
#[derive(Debug, Clone)]
pub struct Flow {
	pub routine: Routine,
	pub period: Period,
	pub step: usize,
	pub refresh: Refresh,
	/// Where to go back to when this one is done: a refresh started from
	/// the daily plan returns to it.
	pub then: Option<(Routine, Period, usize)>,
}

impl Flow {
	fn new(routine: Routine, period: Period) -> Self {
		Self {
			routine,
			period,
			step: 0,
			refresh: Refresh::default(),
			then: None,
		}
	}

	pub fn current(&self) -> Step {
		let steps = self.routine.steps();
		steps[self.step.min(steps.len() - 1)]
	}

	pub fn last_step(&self) -> bool {
		self.step + 1 >= self.routine.steps().len()
	}
}

/// A change to a task, waiting its turn to be sent.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
	Day(NaiveDate),
	Backlog(BacklogSection),
	/// Move between backlog horizons, staying parked.
	Horizon(BacklogSection),
	Triage,
	Complete,
	Ignore,
	/// Back to active, from completed or ignored.
	Reopen,
	Delete,
	Estimate(u32),
	Priority(Option<Priority>),
	Deadline(Option<Deadline>),
	/// Into the time block with this event id.
	Block(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Op {
	/// The task's key: its id can change when it moves between Google Tasks
	/// and Calendar, which an earlier change in the queue may do.
	pub key: String,
	pub change: Change,
}

/// Lists of tasks offered for picking, which open in full on request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
	Unfinished,
	DueSoon,
	LaterThisWeek,
	Triaged,
	NextWeeks,
	Inbox,
	NextMonth,
	NextQuarter,
	Done,
}

/// Stretches of time, as start and end.
pub type Spans = Vec<(DateTime<Local>, DateTime<Local>)>;

/// Numbers that need the period's days read from the cache, worked out when
/// a step opens rather than on every redraw.
#[derive(Debug, Clone, Default)]
pub struct Figures {
	pub period: Option<Period>,
	pub report: Option<report::Report>,
	/// The time taken by meetings and other timed events, by day, merged.
	pub busy: BTreeMap<NaiveDate, Spans>,
}

#[derive(Default)]
pub struct Workspace {
	/// The day the overview is about. `None` follows today.
	pub anchor: Option<NaiveDate>,
	pub flow: Option<Flow>,
	/// A routine left for the overview, to be picked up again.
	pub paused: Option<Flow>,
	/// Task changes waiting for the one before them to be saved.
	pub queue: VecDeque<Op>,
	/// A task being typed into the daily plan.
	pub quick_add: String,
	pub adding: bool,
	pub objective_draft: String,
	pub error: Option<String>,
	pub figures: Figures,
	/// The period's events are being fetched.
	pub loading: bool,
	requests: u64,
	/// The period last fetched, and when.
	fetched: Option<(Period, std::time::Instant)>,
	/// The summary was just copied.
	pub copied: bool,
	/// The task whose deletion waits for a second press, by key.
	pub confirm_delete: Option<String>,
	pub expanded: HashSet<Group>,
	/// One editor per reflection prompt.
	pub editors: Vec<(&'static str, text_editor::Content)>,
}

impl Workspace {
	/// Changes still to be sent, including the one being sent.
	pub fn saving(&self, state_busy: bool) -> usize {
		self.queue.len() + usize::from(state_busy)
	}

	/// Whether a change to this task is waiting its turn.
	pub fn queued(&self, key: &str) -> bool {
		self.queue.iter().any(|op| op.key == key)
	}

	pub fn editor(&self, prompt: &str) -> Option<&text_editor::Content> {
		self.editors
			.iter()
			.find(|(id, _)| *id == prompt)
			.map(|(_, content)| content)
	}
}

// Messages only live until the next update, so a large variant costs nothing lasting.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Action {
	/// Move the overview by this many days; zero goes back to today.
	Shift(i64),
	Start(Routine),
	/// Leave the routine for the overview. Its place is kept.
	Close,
	/// Pick a routine up where it was left.
	Resume,
	/// Throw away the routine in progress.
	Discard,
	Next,
	Back,
	Step(usize),
	/// Run the daily refresh, then come back to the next step.
	Detour,
	Decide(Decision),
	/// Show the refresh card at this index.
	Card(usize),
	Deadlines(DeadlinePolicy),
	CancelDelete,
	/// Change a task, by key.
	Task(String, Change),
	/// Make the same change to several tasks.
	Tasks(Vec<String>, Change),
	QuickAdd(String),
	QuickAddSubmit,
	QuickAdded(Result<Event, String>),
	Intention(String),
	IntentionMet(Option<bool>),
	Rating(u8),
	Prompt(&'static str, text_editor::Action),
	ObjectiveDraft(String),
	AddObjective,
	ObjectiveText(u64, String),
	ObjectiveDone(u64, bool),
	RemoveObjective(u64),
	/// Copy an objective of the previous period into this one.
	CarryOver(u64),
	/// Copy an objective of this period into the next one.
	CarryForward(u64),
	/// Pick a finished task out as a highlight, or put it back: key, title.
	Highlight(String, String),
	Expand(Group),
	Copy(String),
	/// Open the planner the step points at.
	OpenPlanner,
	RangeLoaded {
		request: u64,
		/// The task revision the request began at.
		revision: u64,
		period: Period,
		dates: Vec<NaiveDate>,
		result: Result<calendar::Snapshot, String>,
	},
}

/// The planning day it is now.
pub fn today(state: &State) -> NaiveDate {
	calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour())
}

/// The day the overview is about.
pub fn anchor(state: &State) -> NaiveDate {
	state.routines.anchor.unwrap_or_else(|| today(state))
}

pub fn period(state: &State, scale: Scale) -> Period {
	Period::of(
		scale,
		anchor(state),
		state.config.calendar.first_day_of_week,
	)
}

pub fn entry(state: &State, period: Period) -> Option<&Entry> {
	state.config.routines.get(period)
}

/// The loaded task with this key.
pub fn task<'a>(state: &'a State, key: &str) -> Option<&'a Event> {
	state
		.calendar
		.events
		.iter()
		.find(|event| event.is_task() && tb::task_key(event) == key)
}

fn hour(state: &State) -> u32 {
	state.config.calendar.day_start_hour()
}

/// Open tasks planned for `day`: all-day ones first, then by time.
pub fn day_tasks(state: &State, day: NaiveDate) -> Vec<&Event> {
	let hour = hour(state);
	let mut tasks: Vec<&Event> = state
		.calendar
		.events
		.iter()
		.filter(|event| {
			countable(event)
				&& tb::task_status(event) == TaskStatus::Active
				&& task_day(event, hour) == Some(day)
		})
		.collect();
	tasks.sort_by_cached_key(|task| {
		(
			!task.all_day,
			task.start,
			tb::metadata(task).order,
			task.title.to_lowercase(),
		)
	});
	tasks
}

/// Tasks planned for `day` and finished.
pub fn done_on(state: &State, day: NaiveDate) -> Vec<&Event> {
	let hour = hour(state);
	state
		.calendar
		.events
		.iter()
		.filter(|event| {
			countable(event)
				&& tb::task_status(event) == TaskStatus::Completed
				&& report::done_day(event, hour) == Some(day)
		})
		.collect()
}

/// Open tasks planned for a day before `today` whose deadline, if any, has
/// not passed. Those past it are overdue instead.
pub fn unfinished(state: &State, today: NaiveDate) -> Vec<&Event> {
	let hour = hour(state);
	let now = crate::clock::now();
	let mut tasks: Vec<&Event> = state
		.calendar
		.events
		.iter()
		.filter(|event| {
			countable(event)
				&& tb::task_status(event) == TaskStatus::Active
				&& task_day(event, hour).is_some_and(|day| day < today)
				&& !tb::past_due(event, now)
		})
		.collect();
	tasks.sort_by_cached_key(|task| (task.start, task.title.to_lowercase()));
	tasks
}

/// Open tasks whose deadline is between now and `until`, soonest first.
pub fn due_by(state: &State, until: NaiveDate) -> Vec<&Event> {
	let now = crate::clock::now();
	let mut tasks: Vec<(&Event, Deadline)> = state
		.calendar
		.events
		.iter()
		.filter(|event| countable(event) && tb::task_status(event) == TaskStatus::Active)
		.filter_map(|event| Some((event, tb::metadata(event).deadline?)))
		.filter(|(_, deadline)| !deadline.is_past(now.naive_local()) && deadline.date <= until)
		.collect();
	tasks.sort_by_key(|(task, deadline)| (deadline.date, deadline.time, task.title.to_lowercase()));
	tasks.into_iter().map(|(task, _)| task).collect()
}

/// Parked tasks in a backlog horizon, in backlog order.
pub fn backlog(state: &State, section: BacklogSection) -> Vec<&Event> {
	let mut tasks: Vec<&Event> = state
		.calendar
		.events
		.iter()
		.filter(|event| {
			countable(event)
				&& tb::task_status(event) == TaskStatus::Active
				&& event.start.date_naive() == UNDATED
				&& tb::is_backlog_list(state, &event.calendar_id)
				&& tb::metadata(event).backlog_section == section
		})
		.collect();
	tasks.sort_by_cached_key(|task| tb::backlog_order(task));
	tasks
}

/// Open tasks in the daily refresh's lists, each once, in its first list.
pub fn refresh_lists(state: &State) -> Vec<(Section, Vec<&Event>)> {
	let today = today(state);
	let mut seen = HashSet::new();
	let lists = [
		(Section::Overdue, crate::inbox::overdue_tasks(state)),
		(Section::Unfinished, unfinished(state, today)),
		(Section::Inbox, crate::inbox::tasks(state)),
		(Section::Today, crate::inbox::today_tasks(state)),
	];
	lists
		.into_iter()
		.map(|(section, tasks)| {
			let tasks = tasks
				.into_iter()
				.filter(|task| countable(task) && seen.insert(tb::task_key(task)))
				.collect();
			(section, tasks)
		})
		.collect()
}

/// When the waking day ends: bedtime on the night after `day`.
fn bedtime(state: &State, day: NaiveDate) -> DateTime<Local> {
	let next = day + Duration::days(1);
	let times = calendar::sleep::times_for(&state.config.calendar, next);
	// A bedtime in the small hours belongs to the next date.
	let date = if times.bedtime < chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap() {
		next
	} else {
		day
	};
	calendar::at_clock(
		date,
		chrono::Timelike::hour(&times.bedtime),
		chrono::Timelike::minute(&times.bedtime),
	)
}

/// When the waking day starts: the wake-up time of the night ending `day`.
fn wakeup(state: &State, day: NaiveDate) -> DateTime<Local> {
	let times = calendar::sleep::times_for(&state.config.calendar, day);
	calendar::at_clock(
		day,
		chrono::Timelike::hour(&times.wakeup),
		chrono::Timelike::minute(&times.wakeup),
	)
}

/// The waking minutes left in `day`, less meetings and other timed events.
/// Today counts from now. Time blocks are where tasks go, so they count as
/// free.
pub fn free_minutes(state: &State, day: NaiveDate) -> u32 {
	let start = wakeup(state, day).max(crate::clock::now());
	let end = bedtime(state, day);
	if end <= start {
		return 0;
	}
	let busy: i64 = state
		.routines
		.figures
		.busy
		.get(&day)
		.into_iter()
		.flatten()
		.map(|&(from, to)| (to.min(end) - from.max(start)).num_minutes().max(0))
		.sum();
	((end - start).num_minutes() - busy).max(0) as u32
}

/// The estimates of `tasks` added up, and how many have none.
pub fn estimated(tasks: &[&Event]) -> (u32, usize) {
	tasks.iter().fold((0, 0), |(minutes, missing), task| {
		let estimate = tb::metadata(task).minutes;
		(minutes + estimate, missing + usize::from(estimate == 0))
	})
}

/// Today's time blocks, in time order.
pub fn blocks_on(state: &State, day: NaiveDate) -> Vec<&Event> {
	let hour = hour(state);
	let mut blocks: Vec<&Event> = state
		.calendar
		.events
		.iter()
		.filter(|event| {
			tb::is_block(event, &state.config.calendar)
				&& calendar::day_at(event.start, hour) == day
		})
		.collect();
	blocks.sort_by_key(|block| block.start);
	blocks
}

/// The events of each of the period's days that are known, from memory or
/// from the copy on disk.
pub fn load_days(state: &State, period: Period) -> BTreeMap<NaiveDate, Vec<Event>> {
	period
		.days()
		.into_iter()
		.filter_map(|day| Some((day, state.calendar.events_for_day(day)?)))
		.collect()
}

fn save(state: &mut State, period: Period, edit: impl FnOnce(&mut Entry)) -> Task<Message> {
	edit(state.config.routines.entry_mut(period));
	state.config.routines.tidy();
	crate::app::queue_save(state)
}

/// Works out the numbers the open step needs.
fn recompute(state: &mut State) {
	let Some(flow) = &state.routines.flow else {
		return;
	};
	let period = flow.period;
	let step = flow.current();
	let days = load_days(state, period);
	let settings = &state.config.calendar;
	let busy = days
		.iter()
		.map(|(&day, events)| {
			let spans = events
				.iter()
				.filter(|event| report::is_meeting(event, settings))
				.map(|event| (event.start, event.end))
				.collect();
			(day, report::merged(spans))
		})
		.collect();
	let report =
		(step == Step::Report || step == Step::Wrapup).then(|| report::build(state, period, &days));
	state.routines.figures = Figures {
		period: Some(period),
		report,
		busy,
	};
}

/// How long the numbers of a period that includes today are trusted before
/// opening another of its steps fetches them again.
const REFETCH: std::time::Duration = std::time::Duration::from_secs(120);

/// Fetches the period's events from Google, so the numbers cover days never
/// opened on this device. Nothing is written to Google.
fn load_range(state: &mut State) -> Task<Message> {
	let Some(flow) = &state.routines.flow else {
		return Task::none();
	};
	let period = flow.period;
	if !state.calendar.is_signed_in() || state.routines.loading {
		return Task::none();
	}
	let dates = period.days();
	let missing = dates
		.iter()
		.any(|day| !state.routines.figures.busy.contains_key(day));
	let fresh = state
		.routines
		.fetched
		.is_some_and(|(fetched, at)| fetched == period && at.elapsed() < REFETCH);
	// Earlier periods do not change; today's figures go stale as it goes on.
	if fresh || (!missing && !period.contains(today(state))) {
		return Task::none();
	}
	state.routines.loading = true;
	state.routines.requests += 1;
	let request = state.routines.requests;
	let revision = state.timeblocking.revision;
	let settings = state.config.calendar.clone();
	let fetched = dates.clone();
	crate::blocking::run(move || {
		let token = calendar::GoogleClient::from_settings(&settings).access_token()?;
		let first = *dates.first().ok_or("No days to fetch")?;
		let last = *dates.last().ok_or("No days to fetch")?;
		calendar::api::fetch_range(
			&token,
			&settings.disabled,
			period.first,
			first.pred_opt().unwrap_or(first),
			last.succ_opt().unwrap_or(last),
			0,
			settings.app_calendar.as_deref(),
		)
	})
	.map(move |result| {
		msg(Action::RangeLoaded {
			request,
			revision,
			period,
			dates: fetched.clone(),
			result,
		})
	})
}

/// What a step needs as it opens.
fn enter(state: &mut State) -> Task<Message> {
	state.routines.confirm_delete = None;
	state.routines.copied = false;
	state.routines.error = None;
	let Some(flow) = &state.routines.flow else {
		return Task::none();
	};
	let routine = flow.routine;
	let period = flow.period;
	match flow.current() {
		Step::Triage => {
			if flow.refresh.cards.is_empty() {
				let cards = refresh_lists(state)
					.into_iter()
					.flat_map(|(section, tasks)| {
						tasks.into_iter().map(move |task| Card {
							key: tb::task_key(task),
							section,
							title: task.title.clone(),
						})
					})
					.collect::<Vec<_>>();
				if let Some(flow) = &mut state.routines.flow {
					flow.refresh.cards = cards;
					flow.refresh.index = 0;
				}
			}
			if state
				.routines
				.flow
				.as_ref()
				.is_some_and(|flow| flow.refresh.card().is_none())
			{
				return go(state, Step::RefreshDone);
			}
			Task::none()
		}
		Step::RefreshDone => {
			record_refresh(state);
			Task::none()
		}
		Step::Reflect => {
			let entry = state.config.routines.get(period);
			state.routines.editors = routine
				.prompts()
				.iter()
				.map(|(id, _)| {
					(
						*id,
						text_editor::Content::with_text(entry.map_or("", |e| e.answer(id))),
					)
				})
				.collect();
			Task::none()
		}
		Step::Report | Step::Wrapup | Step::Estimate | Step::Schedule => {
			recompute(state);
			load_range(state)
		}
		_ => Task::none(),
	}
}

/// Opens `step` of the routine in progress.
fn go(state: &mut State, step: Step) -> Task<Message> {
	let Some(flow) = &mut state.routines.flow else {
		return Task::none();
	};
	if let Some(index) = flow.routine.steps().iter().position(|s| *s == step) {
		flow.step = index;
	}
	enter(state)
}

fn start(state: &mut State, routine: Routine, period: Period) -> Task<Message> {
	state.routines.flow = Some(Flow::new(routine, period));
	state.routines.expanded.clear();
	state.routines.quick_add.clear();
	state.routines.objective_draft.clear();
	let shown = show_day(state);
	Task::batch([shown, enter(state)])
}

/// Puts the day of a daily routine in the panel's day view. A day's time
/// blocks and repeating tasks are only loaded for the day on show, and the
/// timebox step shows that view beside it.
fn show_day(state: &mut State) -> Task<Message> {
	let Some(period) = state.routines.flow.as_ref().map(|flow| flow.period) else {
		return Task::none();
	};
	if period.scale != Scale::Day || state.calendar.day == period.first {
		return Task::none();
	}
	crate::app::dispatch(
		state,
		Message::Calendar(CalendarMessage::CalendarMonthDayPicked(
			crate::app::MonthPicker::Panel,
			period.first,
		)),
	)
}

/// Writes the refresh down in today's entry, once per pass.
fn record_refresh(state: &mut State) {
	let Some(flow) = &mut state.routines.flow else {
		return;
	};
	if flow.routine != Routine::DailyRefresh || flow.refresh.recorded {
		return;
	}
	flow.refresh.recorded = true;
	let period = flow.period;
	let decided = flow.refresh.decided() as u32;
	let pending: HashSet<String> = state
		.routines
		.queue
		.iter()
		.map(|op| op.key.clone())
		.collect();
	let remaining = refresh_lists(state)
		.into_iter()
		.flat_map(|(_, tasks)| tasks)
		.filter(|task| !pending.contains(&tb::task_key(task)))
		.count() as u32;
	let _ = save(state, period, |entry| {
		entry.refreshes.push(timeparagon_core::routines::Refresh {
			recorded_at: crate::clock::utc_now(),
			decided,
			remaining,
		});
	});
}

/// Finishes the routine in progress: writes down that it was done, then
/// goes back to the routine that sent it, or to the overview.
fn finish(state: &mut State) -> Task<Message> {
	record_refresh(state);
	let Some(flow) = state.routines.flow.take() else {
		return Task::none();
	};
	let now = crate::clock::utc_now();
	let saved = if flow.routine.plans() {
		let committed = (flow.routine == Routine::DailyPlan).then(|| {
			let tasks = day_tasks(state, flow.period.first);
			let keys: Vec<String> = tasks.iter().map(|task| tb::task_key(task)).collect();
			(keys, estimated(&tasks).0)
		});
		save(state, flow.period, |entry| {
			entry.planned_at = Some(now);
			if let Some((keys, minutes)) = committed {
				entry.planned_tasks = keys;
				entry.planned_minutes = minutes;
			}
		})
	} else if flow.routine.reflects() {
		save(state, flow.period, |entry| entry.reflected_at = Some(now))
	} else {
		Task::none()
	};
	let next = match flow.then {
		Some((routine, period, step)) => {
			state.routines.flow = Some(Flow {
				step,
				..Flow::new(routine, period)
			});
			let shown = show_day(state);
			Task::batch([shown, enter(state)])
		}
		None => Task::none(),
	};
	Task::batch([saved, next])
}

/// The changes that carry out `decision` for the task on `card`.
fn changes_for(
	state: &State,
	card: &Card,
	decision: Decision,
	policy: DeadlinePolicy,
) -> Vec<Change> {
	let Some(task) = task(state, &card.key) else {
		return Vec::new();
	};
	let mut changes = Vec::new();
	let active = tb::task_status(task) == TaskStatus::Active;
	if !active
		&& !matches!(
			decision,
			Decision::Done | Decision::Ignore | Decision::Delete | Decision::Skip
		) {
		changes.push(Change::Reopen);
	}
	let overdue = tb::metadata(task)
		.deadline
		.is_some_and(|deadline| deadline.is_past(crate::clock::now().naive_local()));
	match decision {
		Decision::Day(day) => {
			if task_day(task, hour(state)) != Some(day) {
				changes.push(Change::Day(day));
			}
			if overdue {
				match policy {
					DeadlinePolicy::Follow => changes.push(Change::Deadline(Some(Deadline {
						date: day,
						time: None,
					}))),
					DeadlinePolicy::Clear => changes.push(Change::Deadline(None)),
					DeadlinePolicy::Keep => {}
				}
			}
		}
		Decision::Backlog(section) => {
			changes.push(Change::Backlog(section));
			if overdue && policy != DeadlinePolicy::Keep {
				changes.push(Change::Deadline(None));
			}
		}
		Decision::Triage => changes.push(Change::Triage),
		Decision::Keep | Decision::Skip => {}
		Decision::Done if active => changes.push(Change::Complete),
		Decision::Ignore if active => changes.push(Change::Ignore),
		Decision::Done | Decision::Ignore => {}
		Decision::Delete => changes.push(Change::Delete),
	}
	changes
}

fn enqueue(state: &mut State, key: &str, changes: Vec<Change>) {
	// A newer decision about the same task replaces what was still waiting.
	state.routines.queue.retain(|op| op.key != key);
	state
		.routines
		.queue
		.extend(changes.into_iter().map(|change| Op {
			key: key.to_owned(),
			change,
		}));
}

/// Sends queued task changes while nothing else is being saved.
pub fn pump(state: &mut State) -> Task<Message> {
	let mut tasks = Vec::new();
	// While the add-task popup is open the time-blocking workspace is set
	// aside with its saves, so this one cannot tell what is in flight.
	while state.timeblocking.busy.is_empty() && state.inbox.draft.is_none() {
		let Some(op) = state.routines.queue.pop_front() else {
			break;
		};
		let Some(task) = task(state, &op.key) else {
			continue;
		};
		let id = task.id.clone();
		let status = tb::task_status(task);
		let message = match op.change {
			Change::Day(day) => Message::TimeBlocking(tb::Action::ScheduleDay(id, day)),
			Change::Backlog(section) => {
				Message::TimeBlocking(tb::Action::MoveToBacklog(id, section))
			}
			Change::Horizon(section) => {
				Message::TimeBlocking(tb::Action::SetBacklogSection(id, section))
			}
			Change::Triage => Message::TimeBlocking(tb::Action::Triage(id)),
			Change::Complete if status == TaskStatus::Active => {
				Message::Task(TaskMessage::TaskToggled(id))
			}
			Change::Ignore if status == TaskStatus::Active => {
				Message::Task(TaskMessage::TaskIgnoreToggled(id))
			}
			Change::Reopen if status == TaskStatus::Completed => {
				Message::Task(TaskMessage::TaskToggled(id))
			}
			Change::Reopen if status == TaskStatus::Ignored => {
				Message::Task(TaskMessage::TaskIgnoreToggled(id))
			}
			Change::Complete | Change::Ignore | Change::Reopen => continue,
			Change::Delete => {
				tasks.push(delete(state, id));
				continue;
			}
			Change::Estimate(minutes) => {
				Message::TimeBlocking(tb::Action::SetEstimate(id, minutes))
			}
			Change::Priority(priority) => Message::TimeBlocking(tb::Action::Priority(id, priority)),
			Change::Deadline(deadline) => {
				Message::TimeBlocking(tb::Action::SetDeadline(id, deadline))
			}
			Change::Block(block) => Message::TimeBlocking(tb::Action::Assign(id, block)),
		};
		tasks.push(crate::app::dispatch(state, message));
		// A change that was refused, such as triaging a task missing a
		// required field, leaves nothing in flight; say why.
		if state.timeblocking.busy.is_empty()
			&& let Some(error) = state.inbox.error.take()
		{
			state.routines.error = Some(error);
		}
	}
	Task::batch(tasks)
}

/// Deletes a task without the confirmation dialog: the routine asked already.
fn delete(state: &mut State, id: String) -> Task<Message> {
	let Some(task) = state.calendar.event(&id).filter(|e| e.is_task()).cloned() else {
		return Task::none();
	};
	state.timeblocking.revision += 1;
	state.timeblocking.busy.insert(id.clone());
	state.calendar.error = None;
	let app_calendar = state.config.calendar.app_calendar.clone();
	crate::blocking::run(move || calendar::delete_task(&task, app_calendar.as_deref()))
		.map(move |result| Message::Task(TaskMessage::TaskDeleted(id.clone(), result)))
}

/// Keeps the reflection's editors in step with answers that changed
/// elsewhere, such as on another device.
pub fn sync_editors(state: &mut State) {
	let Some(flow) = &state.routines.flow else {
		return;
	};
	if flow.current() != Step::Reflect {
		return;
	}
	let entry = state.config.routines.get(flow.period);
	for (id, editor) in &mut state.routines.editors {
		let answer = entry.map_or("", |entry| entry.answer(id));
		if editor.text() != answer {
			*editor = text_editor::Content::with_text(answer);
		}
	}
}

/// The plan or reflection as text, to paste somewhere.
pub fn summary(state: &State, flow: &Flow) -> String {
	let entry = entry(state, flow.period);
	let mut out = format!("{} · {}\n", flow.routine.title(), flow.period.label());
	if let Some(entry) = entry
		&& !entry.intention.trim().is_empty()
	{
		let label = match flow.period.scale {
			Scale::Day => "Highlight",
			_ => "Focus",
		};
		out.push_str(&format!("{label}: {}\n", entry.intention.trim()));
	}
	if let Some(entry) = entry
		&& !entry.objectives.is_empty()
	{
		out.push_str("\nObjectives\n");
		for objective in &entry.objectives {
			let mark = if flow.routine.reflects() {
				if objective.done { "[x] " } else { "[ ] " }
			} else {
				"- "
			};
			out.push_str(&format!("{mark}{}\n", objective.text));
		}
	}
	if flow.routine == Routine::DailyPlan {
		let tasks = day_tasks(state, flow.period.first);
		let (minutes, _) = estimated(&tasks);
		out.push_str(&format!("\nTasks ({})\n", duration(minutes)));
		for task in tasks {
			let estimate = tb::metadata(task).minutes;
			if estimate > 0 {
				out.push_str(&format!("- {} ({})\n", task.title, duration(estimate)));
			} else {
				out.push_str(&format!("- {}\n", task.title));
			}
		}
	}
	if flow.routine.reflects()
		&& let Some(report) = &state.routines.figures.report
	{
		out.push_str(&format!(
			"\nDone: {} tasks · Focus: {} · Meetings: {}\n",
			report.done.len(),
			duration((report.tracked / 60) as u32),
			duration(report.meetings.minutes),
		));
		let highlights: Vec<&str> = entry
			.into_iter()
			.flat_map(|entry| entry.highlights.iter().map(|h| h.title.as_str()))
			.chain(report.highlights.iter().map(|(_, h)| h.title.as_str()))
			.collect::<Vec<_>>();
		if !highlights.is_empty() {
			out.push_str("\nHighlights\n");
			let mut seen = HashSet::new();
			for title in highlights.into_iter().filter(|t| seen.insert(*t)) {
				out.push_str(&format!("★ {title}\n"));
			}
		}
		if let Some(entry) = entry {
			for (id, question) in flow.routine.prompts() {
				let answer = entry.answer(id).trim();
				if !answer.is_empty() {
					out.push_str(&format!("\n{question}\n{answer}\n"));
				}
			}
		}
	}
	out
}

/// "1h 30m", "45m", "2h".
pub fn duration(minutes: u32) -> String {
	match (minutes / 60, minutes % 60) {
		(0, minutes) => format!("{minutes}m"),
		(hours, 0) => format!("{hours}h"),
		(hours, minutes) => format!("{hours}h {minutes}m"),
	}
}

/// The routine that suits the time of day, week and month, for the overview
/// to point at. Nothing when it has been done already.
pub fn suggested(state: &State) -> Option<Routine> {
	use chrono::Timelike;
	let now = crate::clock::now();
	let today = today(state);
	let first_day = state.config.calendar.first_day_of_week;
	let week = Period::of(Scale::Week, today, first_day);
	let month = Period::of(Scale::Month, today, first_day);
	let done = |routine: Routine, period: Period| routine.done(entry(state, period));
	let evening = now.hour() >= 17 || now.date_naive() > today;
	let candidates = [
		(
			today == month.first && !evening,
			Routine::MonthlyPlan,
			month,
		),
		(
			today == month.last() && evening,
			Routine::MonthlyReflection,
			month,
		),
		(today == week.first && !evening, Routine::WeeklyPlan, week),
		(
			today == week.last() && evening,
			Routine::WeeklyReflection,
			week,
		),
		(!evening, Routine::DailyPlan, Period::day(today)),
		(
			now.hour() >= 11 && !evening,
			Routine::DailyRefresh,
			Period::day(today),
		),
		(evening, Routine::DailyReflection, Period::day(today)),
	];
	candidates
		.into_iter()
		.find(|(fits, routine, period)| *fits && !done(*routine, *period))
		.map(|(_, routine, _)| routine)
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
	match action {
		Action::Shift(days) => {
			let today = today(state);
			let day = if days == 0 {
				today
			} else {
				anchor(state) + Duration::days(days)
			};
			state.routines.anchor = (day != today).then_some(day);
			Task::none()
		}
		Action::Start(routine) => {
			// From inside a routine, another is about the same days.
			let day = state
				.routines
				.flow
				.as_ref()
				.map_or_else(|| anchor(state), |flow| flow.period.first);
			let period = Period::of(
				routine.scale(),
				day,
				state.config.calendar.first_day_of_week,
			);
			// The same routine left halfway is picked up where it was.
			if state
				.routines
				.paused
				.as_ref()
				.is_some_and(|flow| flow.routine == routine && flow.period == period)
			{
				return update(state, Action::Resume);
			}
			// One started from inside another sets that one aside, to be
			// continued from the overview, unless there is nothing left of it.
			state.routines.paused = state
				.routines
				.flow
				.take()
				.filter(|flow| flow.current() != Step::RefreshDone);
			start(state, routine, period)
		}
		Action::Close => {
			state.routines.error = None;
			state.routines.confirm_delete = None;
			// Kept so it can be picked up again, unless there is nothing
			// left of it: a refresh whose summary has been seen is over.
			state.routines.paused = state
				.routines
				.flow
				.take()
				.filter(|flow| flow.current() != Step::RefreshDone);
			Task::none()
		}
		Action::Resume => {
			if let Some(flow) = state.routines.paused.take() {
				state.routines.flow = Some(flow);
				let shown = show_day(state);
				return Task::batch([shown, enter(state)]);
			}
			Task::none()
		}
		Action::Discard => {
			state.routines.paused = None;
			state.routines.flow = None;
			Task::none()
		}
		Action::Next => {
			let Some(flow) = &mut state.routines.flow else {
				return Task::none();
			};
			if flow.last_step() {
				return finish(state);
			}
			// Moving past the cards skips the rest of them.
			flow.step += 1;
			enter(state)
		}
		Action::Back => {
			let Some(flow) = &mut state.routines.flow else {
				return Task::none();
			};
			if flow.current() == Step::Triage && flow.refresh.index > 0 {
				flow.refresh.index -= 1;
				state.routines.confirm_delete = None;
				return Task::none();
			}
			flow.step = flow.step.saturating_sub(1);
			enter(state)
		}
		Action::Step(index) => {
			let Some(flow) = &mut state.routines.flow else {
				return Task::none();
			};
			flow.step = index.min(flow.routine.steps().len() - 1);
			enter(state)
		}
		Action::Detour => {
			let Some(flow) = state.routines.flow.take() else {
				return Task::none();
			};
			let today = today(state);
			state.routines.flow = Some(Flow {
				then: Some((flow.routine, flow.period, flow.step + 1)),
				..Flow::new(Routine::DailyRefresh, Period::day(today))
			});
			enter(state)
		}
		Action::Decide(decision) => decide(state, decision),
		Action::Card(index) => {
			if let Some(flow) = &mut state.routines.flow
				&& index < flow.refresh.cards.len()
			{
				flow.refresh.index = index;
				state.routines.confirm_delete = None;
				state.routines.error = None;
			}
			Task::none()
		}
		Action::Deadlines(policy) => {
			if let Some(flow) = &mut state.routines.flow {
				flow.refresh.deadlines = policy;
			}
			Task::none()
		}
		Action::CancelDelete => {
			state.routines.confirm_delete = None;
			Task::none()
		}
		Action::Task(key, change) => {
			state.routines.error = None;
			if change == Change::Triage
				&& let Some(task) = task(state, &key)
				&& let Some(error) = tb::triage_error(state, &task.id)
			{
				state.routines.error = Some(error);
				return Task::none();
			}
			if change == Change::Delete && state.routines.confirm_delete.as_deref() != Some(&key) {
				state.routines.confirm_delete = Some(key);
				return Task::none();
			}
			state.routines.confirm_delete = None;
			// An estimate or priority stacks with other waiting changes.
			if matches!(change, Change::Estimate(_) | Change::Priority(_)) {
				state.routines.queue.retain(|op| {
					op.key != key
						|| std::mem::discriminant(&op.change) != std::mem::discriminant(&change)
				});
				state.routines.queue.push_back(Op { key, change });
			} else {
				enqueue(state, &key, vec![change]);
			}
			pump(state)
		}
		Action::Tasks(keys, change) => {
			state.routines.error = None;
			for key in keys {
				enqueue(state, &key, vec![change.clone()]);
			}
			pump(state)
		}
		Action::QuickAdd(text) => {
			state.routines.quick_add = text;
			Task::none()
		}
		Action::QuickAddSubmit => {
			let title = state.routines.quick_add.trim().to_owned();
			if title.is_empty() || state.routines.adding {
				return Task::none();
			}
			let day = state
				.routines
				.flow
				.as_ref()
				.map_or_else(|| today(state), |flow| flow.period.first);
			state.routines.adding = true;
			state.routines.error = None;
			let settings = state.config.calendar.clone();
			crate::blocking::run(move || crate::inbox::add(&settings, &title, "", Some(day), None))
				.map(|result| msg(Action::QuickAdded(result)))
		}
		Action::QuickAdded(result) => {
			state.routines.adding = false;
			match result {
				Ok(event) => {
					state.routines.quick_add.clear();
					state.timeblocking.revision += 1;
					state.calendar.events.retain(|other| other.id != event.id);
					state.calendar.replace_cached_task(&event.id, &event);
					state.calendar.add_event(event);
					Task::done(Message::Calendar(CalendarMessage::CalendarSyncRequested))
				}
				Err(error) => {
					state.routines.error = Some(error);
					Task::none()
				}
			}
		}
		Action::Intention(text) => edit_entry(state, |entry| entry.intention = text),
		Action::IntentionMet(met) => edit_entry(state, |entry| entry.intention_met = met),
		Action::Rating(rating) => edit_entry(state, |entry| {
			entry.rating = (entry.rating != Some(rating)).then_some(rating);
		}),
		Action::Prompt(prompt, action) => {
			let edit = action.is_edit();
			let Some((_, editor)) = state
				.routines
				.editors
				.iter_mut()
				.find(|(id, _)| *id == prompt)
			else {
				return Task::none();
			};
			editor.perform(action);
			if !edit {
				return Task::none();
			}
			let text = editor.text();
			edit_entry(state, |entry| {
				if text.trim().is_empty() {
					entry.answers.remove(prompt);
				} else {
					entry.answers.insert(prompt.to_owned(), text);
				}
			})
		}
		Action::ObjectiveDraft(text) => {
			state.routines.objective_draft = text;
			Task::none()
		}
		Action::AddObjective => {
			let text = state.routines.objective_draft.trim().to_owned();
			if text.is_empty() {
				return Task::none();
			}
			state.routines.objective_draft.clear();
			edit_entry(state, |entry| {
				entry.objectives.push(Objective {
					id: timeparagon_core::routines::new_id(),
					text,
					done: false,
				});
			})
		}
		Action::ObjectiveText(id, text) => edit_entry(state, |entry| {
			if let Some(objective) = entry.objectives.iter_mut().find(|o| o.id == id) {
				objective.text = text;
			}
		}),
		Action::ObjectiveDone(id, done) => edit_entry(state, |entry| {
			if let Some(objective) = entry.objectives.iter_mut().find(|o| o.id == id) {
				objective.done = done;
			}
		}),
		Action::RemoveObjective(id) => {
			edit_entry(state, |entry| entry.objectives.retain(|o| o.id != id))
		}
		Action::CarryOver(id) => {
			let Some(flow) = &state.routines.flow else {
				return Task::none();
			};
			let previous = flow.period.previous();
			let Some(objective) = entry(state, previous)
				.and_then(|entry| entry.objectives.iter().find(|o| o.id == id))
				.cloned()
			else {
				return Task::none();
			};
			edit_entry(state, |entry| carry(entry, &objective))
		}
		Action::CarryForward(id) => {
			let Some(flow) = &state.routines.flow else {
				return Task::none();
			};
			let (period, next) = (flow.period, flow.period.next());
			let Some(objective) = entry(state, period)
				.and_then(|entry| entry.objectives.iter().find(|o| o.id == id))
				.cloned()
			else {
				return Task::none();
			};
			save(state, next, |entry| carry(entry, &objective))
		}
		Action::Highlight(key, title) => edit_entry(state, |entry| {
			if entry.is_highlight(&key) {
				entry.highlights.retain(|h| h.id != key);
			} else {
				entry.highlights.push(Highlight { id: key, title });
			}
		}),
		Action::Expand(group) => {
			if !state.routines.expanded.remove(&group) {
				state.routines.expanded.insert(group);
			}
			Task::none()
		}
		Action::Copy(text) => {
			state.routines.copied = true;
			iced::clipboard::write(text)
		}
		Action::OpenPlanner => {
			let step = state
				.routines
				.flow
				.as_ref()
				.map(|flow| (flow.current(), flow.period.scale));
			match step {
				Some((Step::Schedule, Scale::Week)) => Task::done(Message::Calendar(
					CalendarMessage::CalendarModeSelected(true),
				)),
				Some((Step::Schedule, Scale::Month)) => Task::done(Message::Navigation(
					crate::app::NavigationMessage::ScreenSelected(crate::app::Screen::Backlog),
				)),
				_ => Task::done(Message::Navigation(
					crate::app::NavigationMessage::PlanDaySelected,
				)),
			}
		}
		Action::RangeLoaded {
			request,
			revision,
			period,
			dates,
			result,
		} => {
			if request != state.routines.requests {
				return Task::none();
			}
			state.routines.loading = false;
			match result {
				// A task saved since the request began may be stale in it, so
				// such an answer is dropped rather than shown.
				Ok(snapshot) if revision == state.timeblocking.revision => {
					state.calendar.cache_fetched_snapshot(snapshot, &dates);
					state.calendar.apply_task_colors(&state.config.calendar);
					state.routines.fetched = Some((period, std::time::Instant::now()));
				}
				Ok(_) => {}
				Err(error) => log::warn!("could not fetch the routine's days: {error}"),
			}
			recompute(state);
			Task::none()
		}
	}
}

/// Adds a copy of `objective` unless one with its text is there already.
fn carry(entry: &mut Entry, objective: &Objective) {
	if entry
		.objectives
		.iter()
		.any(|o| o.text.trim() == objective.text.trim())
	{
		return;
	}
	entry.objectives.push(Objective {
		id: timeparagon_core::routines::new_id(),
		text: objective.text.clone(),
		done: false,
	});
}

/// Edits the entry of the routine in progress.
fn edit_entry(state: &mut State, edit: impl FnOnce(&mut Entry)) -> Task<Message> {
	let Some(period) = state.routines.flow.as_ref().map(|flow| flow.period) else {
		return Task::none();
	};
	save(state, period, edit)
}

fn decide(state: &mut State, decision: Decision) -> Task<Message> {
	let Some(flow) = &state.routines.flow else {
		return Task::none();
	};
	if flow.current() != Step::Triage {
		return Task::none();
	}
	let Some(card) = flow.refresh.card().cloned() else {
		return Task::none();
	};
	let policy = flow.refresh.deadlines;
	state.routines.error = None;
	if decision == Decision::Triage
		&& let Some(task) = task(state, &card.key)
		&& let Some(error) = tb::triage_error(state, &task.id)
	{
		state.routines.error = Some(error);
		return Task::none();
	}
	if decision == Decision::Delete && state.routines.confirm_delete.as_deref() != Some(&card.key) {
		state.routines.confirm_delete = Some(card.key);
		return Task::none();
	}
	state.routines.confirm_delete = None;
	let changes = changes_for(state, &card, decision, policy);
	enqueue(state, &card.key, changes);
	let Some(flow) = &mut state.routines.flow else {
		return Task::none();
	};
	flow.refresh.decisions.insert(card.key.clone(), decision);
	flow.refresh.index += 1;
	let finished = flow.refresh.card().is_none();
	let send = pump(state);
	if finished {
		return Task::batch([send, go(state, Step::RefreshDone)]);
	}
	send
}

#[cfg(test)]
mod tests;
