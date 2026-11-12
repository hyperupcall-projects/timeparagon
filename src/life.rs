//! The Life tab: where life areas, their goals and objectives, and the
//! projects and habits that carry them out come together.
//!
//! The tab has five pages:
//!
//! - **Focus**: the objectives pinned for this week, what needs attention, and
//!   what is due soon.
//! - **Areas**: each life area with its satisfaction and progress.
//! - **Goals**: every goal with its objectives, area by area.
//! - **Timeline**: objectives by when they are due.
//! - **Review**: the weekly review: rate each area, check in on objectives,
//!   choose next week's focus, and reflect.
//!
//! Anything picked from them opens in the side panel, which steps between an
//! area, a goal, an objective and a project like pages in a browser.
//!
//! What is shown lives in the config beside the projects
//! (`config::ProjectsSettings`), so it is saved and synced with them. This
//! holds what the tab is in the middle of: the page shown, text being typed,
//! and what is open in the side panel.

use std::collections::HashMap;

use chrono::{Datelike, Months, NaiveDate};
use iced::widget::text_editor;
use iced::{Color, Task};

use crate::app::{Message, State, queue_save};
use crate::config::{Measure, PERSONAL_AREA, ProjectsSettings, Status};

/// The pages of the Life tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
	#[default]
	Focus,
	Areas,
	Goals,
	Timeline,
	Review,
}

impl Page {
	pub const ALL: [Self; 5] = [
		Self::Focus,
		Self::Areas,
		Self::Goals,
		Self::Timeline,
		Self::Review,
	];

	pub fn label(self) -> &'static str {
		match self {
			Self::Focus => "Focus",
			Self::Areas => "Areas",
			Self::Goals => "Goals",
			Self::Timeline => "Timeline",
			Self::Review => "Review",
		}
	}
}

/// Which goals the Goals page lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GoalFilter {
	#[default]
	Active,
	Done,
	All,
}

impl GoalFilter {
	pub const ALL: [Self; 3] = [Self::Active, Self::Done, Self::All];

	pub fn label(self) -> &'static str {
		match self {
			Self::Active => "In progress",
			Self::Done => "Achieved",
			Self::All => "All",
		}
	}

	pub fn shows(self, status: Status) -> bool {
		match self {
			Self::Active => matches!(status, Status::Active | Status::Paused),
			Self::Done => status == Status::Done,
			Self::All => true,
		}
	}
}

/// Something open in the side panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selection {
	Area(u64),
	Goal(u64),
	Objective(u64),
	Project(u64),
}

impl Selection {
	/// Whether it is still there to be shown.
	pub fn exists(self, projects: &ProjectsSettings) -> bool {
		match self {
			Self::Area(id) => projects.area(id).is_some(),
			Self::Goal(id) => projects.find_goal(id).is_some(),
			Self::Objective(id) => projects.find_objective(id).is_some(),
			Self::Project(id) => projects.get(id).is_some(),
		}
	}
}

/// A quick due date for an objective.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Horizon {
	Week,
	Month,
	Quarter,
	Year,
}

impl Horizon {
	pub const ALL: [Self; 4] = [Self::Week, Self::Month, Self::Quarter, Self::Year];

	pub fn label(self) -> &'static str {
		match self {
			Self::Week => "This week",
			Self::Month => "This month",
			Self::Quarter => "This quarter",
			Self::Year => "This year",
		}
	}

	/// The last day of this horizon, counted from `today`.
	pub fn end(self, today: NaiveDate, week_start: NaiveDate) -> NaiveDate {
		let first_of_month = today.with_day(1).expect("every month has a first day");
		let end_of = |first: NaiveDate, months: u32| {
			first
				.checked_add_months(Months::new(months))
				.and_then(|next| next.pred_opt())
				.unwrap_or(today)
		};
		match self {
			Self::Week => week_start + chrono::Days::new(6),
			Self::Month => end_of(first_of_month, 1),
			Self::Quarter => {
				let month0 = (today.month0() / 3) * 3;
				let first = NaiveDate::from_ymd_opt(today.year(), month0 + 1, 1)
					.expect("a quarter starts on a real day");
				end_of(first, 3)
			}
			Self::Year => NaiveDate::from_ymd_opt(today.year(), 12, 31).expect("Dec 31 exists"),
		}
	}
}

#[derive(Default)]
pub struct Workspace {
	pub page: Page,
	/// The life-area name typed into the Areas page.
	pub new_name: String,
	pub add_error: Option<String>,
	pub goal_filter: GoalFilter,
	/// The goal typed into the Goals page, and the area it goes in.
	pub new_goal: String,
	pub new_goal_area: Option<u64>,
	/// What is open in the side panel.
	pub open: Option<Open>,
	pub review: ReviewDraft,
}

impl std::fmt::Debug for Workspace {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Workspace")
			.field("page", &self.page)
			.field("new_name", &self.new_name)
			.field("add_error", &self.add_error)
			.field("open", &self.open.as_ref().map(|open| open.selection))
			.finish()
	}
}

/// The weekly review being written.
#[derive(Default)]
pub struct ReviewDraft {
	/// The first day of the week being reviewed, once the page has been shown.
	pub week: Option<NaiveDate>,
	pub reflection: text_editor::Content,
	/// Check-in values typed in, by objective.
	pub check_ins: HashMap<u64, String>,
}

/// Whether the vision is being written or read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisionMode {
	Edit,
	#[default]
	View,
}

/// What the side panel shows, and what is being typed into it. Everything
/// but the selection and the way back is reset when another page opens.
pub struct Open {
	pub selection: Selection,
	/// Pages to go back to, most recent last.
	pub back: Vec<Selection>,
	/// The area or project name as typed. It is saved whenever it is a valid
	/// name, so a name that is half-typed or taken is kept here without
	/// being lost.
	pub name: String,
	pub name_error: Option<String>,
	pub vision: text_editor::Content,
	pub vision_mode: VisionMode,
	/// The goal typed into an area's page.
	pub new_goal: String,
	/// The objective typed into an area's or goal's page.
	pub new_objective: String,
	/// The project typed into an area's page.
	pub new_project: String,
	pub new_project_error: Option<String>,
	/// The milestone typed into an objective's page.
	pub new_milestone: String,
	/// An objective's numbers as typed; saved whenever they parse.
	pub baseline: String,
	pub target: String,
	pub check_in: String,
	pub check_in_note: String,
	/// A project's directory as typed; blank means none.
	pub directory: String,
	/// Whether the area's name, color and removal are shown.
	pub settings_open: bool,
	/// Whether removing what is open is waiting for a yes.
	pub confirm_remove: bool,
}

impl Open {
	fn new(state: &State, selection: Selection, back: Vec<Selection>) -> Self {
		let projects = &state.config.projects;
		let mut open = Self {
			selection,
			back,
			name: String::new(),
			name_error: None,
			vision: text_editor::Content::new(),
			vision_mode: VisionMode::View,
			new_goal: String::new(),
			new_objective: String::new(),
			new_project: String::new(),
			new_project_error: None,
			new_milestone: String::new(),
			baseline: String::new(),
			target: String::new(),
			check_in: String::new(),
			check_in_note: String::new(),
			directory: String::new(),
			settings_open: false,
			confirm_remove: false,
		};
		match selection {
			Selection::Area(id) => {
				if let Some(area) = projects.area(id) {
					open.name = area.name.clone();
					open.vision = text_editor::Content::with_text(&area.vision);
					// An empty vision opens ready to be written.
					if area.vision.trim().is_empty() {
						open.vision_mode = VisionMode::Edit;
					}
				}
			}
			Selection::Objective(id) => {
				if let Some((_, objective)) = projects.find_objective(id) {
					open.baseline = objective.baseline.map(format_number).unwrap_or_default();
					open.target = objective.target.map(format_number).unwrap_or_default();
				}
			}
			Selection::Project(id) => {
				if let Some(project) = projects.get(id) {
					open.name = project.name.clone();
					open.directory = project
						.directory
						.as_ref()
						.map(|directory| directory.display().to_string())
						.unwrap_or_default();
				}
			}
			Selection::Goal(_) => {}
		}
		open
	}
}

/// A number as typed back into a box: no needless `.0`.
pub fn format_number(value: f64) -> String {
	timeparagon_core::habits::format_amount(value)
}

fn parse_number(text: &str) -> Option<f64> {
	text.trim()
		.replace(',', ".")
		.parse::<f64>()
		.ok()
		.filter(|v| v.is_finite())
}

#[derive(Debug, Clone)]
pub enum Action {
	ShowPage(Page),
	/// The life-area box on the Areas page changed.
	NewName(String),
	Add,
	/// Open something in the side panel, remembering what was open before.
	Open(Selection),
	/// Go back to what was open before.
	Back,
	Close,
	ShiftArea(u64, bool),
	/// Rate an area's satisfaction for this week.
	Rate(u64, u8),

	// What is open: an area or a project.
	Rename(String),
	Color(Option<Color>),
	ToggleSettings,
	Remove,
	ConfirmRemove,
	CancelRemove,

	// An area.
	VisionMode(VisionMode),
	VisionEdited(text_editor::Action),
	NewGoal(String),
	AddGoal,
	NewProject(String),
	AddProject,
	MoveProject(u64, u64),
	MoveHabit(String, u64),

	// The Goals page.
	GoalFilter(GoalFilter),
	QuickGoal(String),
	QuickGoalArea(u64),
	AddQuickGoal,

	// A goal.
	GoalName(u64, String),
	GoalDescription(u64, String),
	GoalWhy(u64, String),
	GoalStatus(u64, Status),
	/// The objective box on an area's or goal's page changed.
	NewObjective(String),
	AddObjective,

	// An objective.
	ObjectiveName(u64, String),
	ObjectiveDescription(u64, String),
	/// Put an objective under another goal of its area, or none.
	ObjectiveGoal(u64, Option<u64>),
	ObjectiveStatus(u64, Status),
	Focus(u64, bool),
	/// A start date as `YYYY-MM-DD`, or blank for none.
	Start(u64, String),
	Due(u64, String),
	DueBy(u64, Horizon),
	Measure(u64, Measure),
	Baseline(String),
	Target(String),
	Unit(u64, String),
	CheckInValue(String),
	CheckInNote(String),
	/// Record what is typed as today's check-in on the open objective.
	CheckIn,
	RemoveCheckIn(u64, NaiveDate),
	NewMilestone(String),
	AddMilestone,
	ToggleMilestone(u64, u64),
	RemoveMilestone(u64, u64),
	/// Have an objective carried out by an existing habit.
	AddHabit(u64, String),
	RemoveHabit(u64, String),
	LinkProject(u64, u64),
	UnlinkProject(u64, u64),

	// A project.
	ProjectDirectory(String),

	// The weekly review.
	ReviewEdited(text_editor::Action),
	ReviewCheckInValue(u64, String),
	ReviewCheckIn(u64),
	FinishReview,
}

pub fn today() -> NaiveDate {
	crate::clock::now().date_naive()
}

/// The first day of the week `today` is in.
pub fn week_start(state: &State, today: NaiveDate) -> NaiveDate {
	state.config.calendar.first_day_of_week.week_start(today)
}

/// The week the Review page is about: this one.
pub fn review_week(state: &State) -> NaiveDate {
	week_start(state, today())
}

/// Opens the review of this week in the editor, unless it already is.
fn prepare_review(state: &mut State) {
	let week = review_week(state);
	if state.life.review.week == Some(week) {
		return;
	}
	let reflection = state
		.config
		.projects
		.review(week)
		.map(|review| review.reflection.clone())
		.unwrap_or_default();
	state.life.review = ReviewDraft {
		week: Some(week),
		reflection: text_editor::Content::with_text(&reflection),
		check_ins: HashMap::new(),
	};
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
	state.config.projects.ensure_personal_area();

	match action {
		Action::ShowPage(page) => {
			state.life.page = page;
			if page == Page::Review {
				prepare_review(state);
			}
			Task::none()
		}

		Action::NewName(name) => {
			state.life.new_name = name;
			state.life.add_error = None;
			Task::none()
		}

		Action::Add => {
			match state
				.config
				.projects
				.clean_area_name(&state.life.new_name, None)
			{
				Ok(name) => {
					let id = state.config.projects.add_area(name);
					state.life.new_name.clear();
					state.life.add_error = None;
					open(state, Selection::Area(id), false);
					queue_save(state)
				}
				Err(error) => {
					state.life.add_error = Some(error);
					Task::none()
				}
			}
		}

		Action::Open(selection) => {
			open(state, selection, true);
			Task::none()
		}

		Action::Back => {
			let Some(open_now) = state.life.open.take() else {
				return Task::none();
			};
			let mut back = open_now.back;
			while let Some(previous) = back.pop() {
				if previous.exists(&state.config.projects) {
					state.life.open = Some(Open::new(state, previous, back));
					return Task::none();
				}
			}
			Task::none()
		}

		Action::Close => {
			state.life.open = None;
			Task::none()
		}

		Action::ShiftArea(id, up) => {
			state.config.projects.shift_area(id, up);
			queue_save(state)
		}

		Action::Rate(area, score) => {
			let week = review_week(state);
			let Some(area) = state.config.projects.area_mut(area) else {
				return Task::none();
			};
			area.rate(week, score);
			queue_save(state)
		}

		Action::Rename(name) => rename(state, name),

		Action::Color(color) => match selected(state) {
			Some(Selection::Area(id)) => {
				let Some(area) = state.config.projects.area_mut(id) else {
					return Task::none();
				};
				area.color = color.map(Into::into);
				queue_save(state)
			}
			Some(Selection::Project(id)) => {
				edit_project(state, id, |project| project.color = color.map(Into::into))
			}
			_ => Task::none(),
		},

		Action::ToggleSettings => {
			if let Some(open) = &mut state.life.open {
				open.settings_open = !open.settings_open;
				open.confirm_remove = false;
			}
			Task::none()
		}

		Action::Remove => {
			if let Some(open) = &mut state.life.open
				&& open.selection != Selection::Area(PERSONAL_AREA)
			{
				open.confirm_remove = true;
			}
			Task::none()
		}

		Action::CancelRemove => {
			if let Some(open) = &mut state.life.open {
				open.confirm_remove = false;
			}
			Task::none()
		}

		Action::ConfirmRemove => {
			let Some(open_now) = state.life.open.take() else {
				return Task::none();
			};
			if !open_now.confirm_remove {
				state.life.open = Some(open_now);
				return Task::none();
			}
			let projects = &mut state.config.projects;
			match open_now.selection {
				// What was in it falls back to Personal on its own, since an
				// area that is gone resolves to Personal.
				Selection::Area(id) => projects.remove_area(id),
				Selection::Goal(id) => {
					for area in &mut projects.areas {
						area.goals.retain(|goal| goal.id != id);
						// Its objectives stay, unassigned.
						for objective in &mut area.objectives {
							if objective.goal == Some(id) {
								objective.goal = None;
							}
						}
					}
				}
				Selection::Objective(id) => {
					for area in &mut projects.areas {
						area.objectives.retain(|objective| objective.id != id);
					}
				}
				Selection::Project(id) => {
					projects.remove(id);
					for area in &mut projects.areas {
						for objective in &mut area.objectives {
							objective.projects.retain(|p| *p != id);
						}
					}
				}
			}
			// Step back to whatever is left of where it was opened from.
			let mut back = open_now.back;
			while let Some(previous) = back.pop() {
				if previous.exists(&state.config.projects) {
					state.life.open = Some(Open::new(state, previous, back));
					break;
				}
			}
			queue_save(state)
		}

		Action::VisionMode(mode) => {
			if let Some(open) = &mut state.life.open {
				open.vision_mode = mode;
			}
			Task::none()
		}

		Action::VisionEdited(action) => {
			let Some(open) = &mut state.life.open else {
				return Task::none();
			};
			let Selection::Area(id) = open.selection else {
				return Task::none();
			};
			let is_edit = action.is_edit();
			open.vision.perform(action);
			if !is_edit {
				return Task::none();
			}
			let vision = open.vision.text();
			let Some(area) = state.config.projects.area_mut(id) else {
				return Task::none();
			};
			area.vision = vision;
			queue_save(state)
		}

		Action::NewGoal(name) => {
			if let Some(open) = &mut state.life.open {
				open.new_goal = name;
			}
			Task::none()
		}

		Action::AddGoal => {
			let Some(open) = &mut state.life.open else {
				return Task::none();
			};
			let Selection::Area(area) = open.selection else {
				return Task::none();
			};
			let name = open.new_goal.trim().to_owned();
			if name.is_empty() {
				return Task::none();
			}
			open.new_goal.clear();
			state.config.projects.add_goal(area, name);
			queue_save(state)
		}

		Action::NewProject(name) => {
			if let Some(open) = &mut state.life.open {
				open.new_project = name;
				open.new_project_error = None;
			}
			Task::none()
		}

		Action::AddProject => {
			let Some(open) = &mut state.life.open else {
				return Task::none();
			};
			let Selection::Area(area) = open.selection else {
				return Task::none();
			};
			match state.config.projects.clean_name(&open.new_project, None) {
				Ok(name) => {
					let id = state.config.projects.add(name);
					if let Some(project) =
						state.config.projects.items.iter_mut().find(|p| p.id == id)
					{
						project.area = (area != PERSONAL_AREA).then_some(area);
					}
					open.new_project.clear();
					open.new_project_error = None;
					queue_save(state)
				}
				Err(error) => {
					open.new_project_error = Some(error);
					Task::none()
				}
			}
		}

		Action::MoveProject(project, area) => {
			let area = (area != PERSONAL_AREA).then_some(area);
			let Some(project) = state
				.config
				.projects
				.items
				.iter_mut()
				.find(|item| item.id == project)
			else {
				return Task::none();
			};
			project.area = area;
			queue_save(state)
		}

		Action::MoveHabit(habit, area) => {
			let area = (area != PERSONAL_AREA).then_some(area);
			let Some(habit) = state.habits.library.habit_mut(&habit) else {
				return Task::none();
			};
			habit.area = area;
			crate::habits::persist(state)
		}

		Action::GoalFilter(filter) => {
			state.life.goal_filter = filter;
			Task::none()
		}

		Action::QuickGoal(name) => {
			state.life.new_goal = name;
			Task::none()
		}

		Action::QuickGoalArea(area) => {
			state.life.new_goal_area = Some(area);
			Task::none()
		}

		Action::AddQuickGoal => {
			let name = state.life.new_goal.trim().to_owned();
			if name.is_empty() {
				return Task::none();
			}
			let area = state.config.projects.resolve_area(state.life.new_goal_area);
			let Some(goal) = state.config.projects.add_goal(area, name) else {
				return Task::none();
			};
			state.life.new_goal.clear();
			open(state, Selection::Goal(goal), false);
			queue_save(state)
		}

		Action::GoalName(goal, name) => edit_goal(state, goal, |goal| goal.name = name),
		Action::GoalDescription(goal, description) => {
			edit_goal(state, goal, |goal| goal.description = description)
		}
		Action::GoalWhy(goal, why) => edit_goal(state, goal, |goal| goal.why = why),
		Action::GoalStatus(goal, status) => edit_goal(state, goal, |goal| goal.status = status),

		Action::NewObjective(name) => {
			if let Some(open) = &mut state.life.open {
				open.new_objective = name;
			}
			Task::none()
		}

		Action::AddObjective => {
			let Some(open) = &mut state.life.open else {
				return Task::none();
			};
			let name = open.new_objective.trim().to_owned();
			if name.is_empty() {
				return Task::none();
			}
			let (area, goal) = match open.selection {
				Selection::Area(area) => (area, None),
				Selection::Goal(goal) => {
					let Some((area, _)) = state.config.projects.find_goal(goal) else {
						return Task::none();
					};
					(area.id, Some(goal))
				}
				_ => return Task::none(),
			};
			open.new_objective.clear();
			let today = today();
			if let Some(id) = state.config.projects.add_objective(area, goal, name)
				&& let Some(objective) = state.config.projects.objective_mut(id)
			{
				// Pace is measured from the day it was set.
				objective.start = Some(today);
			}
			queue_save(state)
		}

		Action::ObjectiveName(id, name) => edit_objective(state, id, |o| o.name = name),
		Action::ObjectiveDescription(id, description) => {
			edit_objective(state, id, |o| o.description = description)
		}
		Action::ObjectiveGoal(id, goal) => edit_objective(state, id, |o| o.goal = goal),
		Action::ObjectiveStatus(id, status) => edit_objective(state, id, |o| {
			o.status = status;
			// Done or set aside is not something to focus on this week.
			if status != Status::Active {
				o.focus = false;
			}
		}),
		Action::Focus(id, focus) => edit_objective(state, id, |o| o.focus = focus),

		Action::Start(id, date) => {
			let date = NaiveDate::parse_from_str(date.get(..10).unwrap_or(""), "%Y-%m-%d").ok();
			edit_objective(state, id, |o| o.start = date)
		}

		Action::Due(id, date) => {
			let date = NaiveDate::parse_from_str(date.get(..10).unwrap_or(""), "%Y-%m-%d").ok();
			edit_objective(state, id, |o| o.due = date)
		}

		Action::DueBy(id, horizon) => {
			let today = today();
			let due = horizon.end(today, week_start(state, today));
			edit_objective(state, id, |o| {
				o.due = Some(due);
				o.start.get_or_insert(today);
			})
		}

		Action::Measure(id, measure) => edit_objective(state, id, |o| o.measure = measure),

		Action::Baseline(text) => {
			let Some((id, value)) = typed_number(state, text, |open| &mut open.baseline) else {
				return Task::none();
			};
			edit_objective(state, id, |o| o.baseline = value)
		}

		Action::Target(text) => {
			let Some((id, value)) = typed_number(state, text, |open| &mut open.target) else {
				return Task::none();
			};
			edit_objective(state, id, |o| o.target = value)
		}

		Action::Unit(id, unit) => edit_objective(state, id, |o| o.unit = unit),

		Action::CheckInValue(value) => {
			if let Some(open) = &mut state.life.open {
				open.check_in = value;
			}
			Task::none()
		}

		Action::CheckInNote(note) => {
			if let Some(open) = &mut state.life.open {
				open.check_in_note = note;
			}
			Task::none()
		}

		Action::CheckIn => {
			let Some(open) = &mut state.life.open else {
				return Task::none();
			};
			let Selection::Objective(id) = open.selection else {
				return Task::none();
			};
			let Some(value) = parse_number(&open.check_in) else {
				return Task::none();
			};
			let note = open.check_in_note.trim().to_owned();
			open.check_in.clear();
			open.check_in_note.clear();
			let today = today();
			edit_objective(state, id, |o| o.check_in(today, value, note))
		}

		Action::RemoveCheckIn(id, date) => edit_objective(state, id, |o| {
			o.check_ins.retain(|check_in| check_in.date != date)
		}),

		Action::NewMilestone(name) => {
			if let Some(open) = &mut state.life.open {
				open.new_milestone = name;
			}
			Task::none()
		}

		Action::AddMilestone => {
			let Some(open) = &mut state.life.open else {
				return Task::none();
			};
			let Selection::Objective(id) = open.selection else {
				return Task::none();
			};
			let name = open.new_milestone.trim().to_owned();
			if name.is_empty() {
				return Task::none();
			}
			open.new_milestone.clear();
			edit_objective(state, id, |o| {
				o.add_milestone(name);
			})
		}

		Action::ToggleMilestone(id, milestone) => edit_objective(state, id, |o| {
			if let Some(m) = o.milestones.iter_mut().find(|m| m.id == milestone) {
				m.done = !m.done;
			}
		}),

		Action::RemoveMilestone(id, milestone) => {
			edit_objective(state, id, |o| o.milestones.retain(|m| m.id != milestone))
		}

		Action::AddHabit(objective, habit) => {
			let Some((area, _)) = state.config.projects.find_objective(objective) else {
				return Task::none();
			};
			let area = area.id;
			if state.habits.library.habit(&habit).is_none() {
				return Task::none();
			}
			// A habit carries out one objective at a time.
			for other in &mut state.config.projects.areas {
				for o in &mut other.objectives {
					o.habits.retain(|h| *h != habit);
				}
			}
			let save = edit_objective(state, objective, |o| o.habits.push(habit.clone()));
			// And it is listed in the area of that objective.
			let projects = &state.config.projects;
			let habit = state
				.habits
				.library
				.habit_mut(&habit)
				.expect("checked above");
			if projects.resolve_area(habit.area) == area {
				return save;
			}
			habit.area = (area != PERSONAL_AREA).then_some(area);
			Task::batch([save, crate::habits::persist(state)])
		}

		Action::RemoveHabit(objective, habit) => {
			edit_objective(state, objective, |o| o.habits.retain(|h| *h != habit))
		}

		Action::LinkProject(objective, project) => {
			if state.config.projects.get(project).is_none() {
				return Task::none();
			}
			edit_objective(state, objective, |o| {
				if !o.projects.contains(&project) {
					o.projects.push(project);
				}
			})
		}

		Action::UnlinkProject(objective, project) => {
			edit_objective(state, objective, |o| o.projects.retain(|p| *p != project))
		}

		Action::ProjectDirectory(directory) => {
			let Some(open) = &mut state.life.open else {
				return Task::none();
			};
			let Selection::Project(id) = open.selection else {
				return Task::none();
			};
			open.directory = directory;
			let trimmed = open.directory.trim();
			let directory = (!trimmed.is_empty()).then(|| std::path::PathBuf::from(trimmed));
			edit_project(state, id, |project| project.directory = directory)
		}

		Action::ReviewEdited(action) => {
			prepare_review(state);
			let is_edit = action.is_edit();
			state.life.review.reflection.perform(action);
			if !is_edit {
				return Task::none();
			}
			let week = review_week(state);
			let reflection = state.life.review.reflection.text();
			state.config.projects.review_mut(week).reflection = reflection;
			queue_save(state)
		}

		Action::ReviewCheckInValue(id, value) => {
			state.life.review.check_ins.insert(id, value);
			Task::none()
		}

		Action::ReviewCheckIn(id) => {
			let Some(value) = state
				.life
				.review
				.check_ins
				.get(&id)
				.and_then(|text| parse_number(text))
			else {
				return Task::none();
			};
			state.life.review.check_ins.remove(&id);
			let today = today();
			edit_objective(state, id, |o| o.check_in(today, value, String::new()))
		}

		Action::FinishReview => {
			let week = review_week(state);
			let review = state.config.projects.review_mut(week);
			review.done = !review.done;
			queue_save(state)
		}
	}
}

fn selected(state: &State) -> Option<Selection> {
	state.life.open.as_ref().map(|open| open.selection)
}

/// Opens `selection` in the side panel. With `remember`, what was open
/// before can be gone back to.
fn open(state: &mut State, selection: Selection, remember: bool) {
	if !selection.exists(&state.config.projects) {
		return;
	}
	let previous = state.life.open.take();
	if previous
		.as_ref()
		.is_some_and(|open| open.selection == selection)
	{
		state.life.open = previous;
		return;
	}
	let back = match previous {
		Some(previous) if remember => {
			let mut back = previous.back;
			back.push(previous.selection);
			back
		}
		_ => Vec::new(),
	};
	state.life.open = Some(Open::new(state, selection, back));
}

/// Keeps what is typed into a number box, and returns the objective it is for
/// with the value to save, when it is blank or a number.
fn typed_number(
	state: &mut State,
	text: String,
	field: impl FnOnce(&mut Open) -> &mut String,
) -> Option<(u64, Option<f64>)> {
	let open = state.life.open.as_mut()?;
	let Selection::Objective(id) = open.selection else {
		return None;
	};
	let blank = text.trim().is_empty();
	let value = parse_number(&text);
	*field(open) = text;
	if blank {
		Some((id, None))
	} else {
		value.map(|value| (id, Some(value)))
	}
}

fn rename(state: &mut State, name: String) -> Task<Message> {
	let Some(open) = &mut state.life.open else {
		return Task::none();
	};
	open.name = name;
	match open.selection {
		Selection::Area(id) => match state.config.projects.clean_area_name(&open.name, Some(id)) {
			Ok(name) => {
				open.name_error = None;
				if let Some(area) = state.config.projects.area_mut(id)
					&& area.name != name
				{
					area.name = name;
					return queue_save(state);
				}
				Task::none()
			}
			Err(error) => {
				open.name_error = Some(error);
				Task::none()
			}
		},
		Selection::Project(id) => match state.config.projects.clean_name(&open.name, Some(id)) {
			Ok(name) => {
				open.name_error = None;
				edit_project(state, id, |project| project.name = name)
			}
			Err(error) => {
				open.name_error = Some(error);
				Task::none()
			}
		},
		_ => Task::none(),
	}
}

fn edit_goal(
	state: &mut State,
	goal: u64,
	change: impl FnOnce(&mut crate::config::Goal),
) -> Task<Message> {
	let Some(goal) = state.config.projects.goal_mut(goal) else {
		return Task::none();
	};
	let before = goal.clone();
	change(goal);
	if *goal == before {
		return Task::none();
	}
	queue_save(state)
}

fn edit_objective(
	state: &mut State,
	objective: u64,
	change: impl FnOnce(&mut crate::config::Objective),
) -> Task<Message> {
	let Some(objective) = state.config.projects.objective_mut(objective) else {
		return Task::none();
	};
	let before = objective.clone();
	change(objective);
	if *objective == before {
		return Task::none();
	}
	queue_save(state)
}

/// Changes a project and saves, keeping the list in order.
fn edit_project(
	state: &mut State,
	id: u64,
	change: impl FnOnce(&mut crate::config::Project),
) -> Task<Message> {
	let Some(project) = state.config.projects.items.iter_mut().find(|p| p.id == id) else {
		return Task::none();
	};
	let before = project.clone();
	change(project);
	if *project == before {
		return Task::none();
	}
	state.config.projects.sort();
	queue_save(state)
}

/// A habit's strength, from 0 to 1, as of today.
pub fn habit_score(state: &State, habit: &str) -> Option<f64> {
	let today = state.habits.today();
	state
		.habits
		.computed(habit)
		.map(|computed| computed.score(today))
}

/// The mean strength of some habits.
pub fn mean_habit_score<'a>(state: &State, habits: impl Iterator<Item = &'a str>) -> Option<f64> {
	timeparagon_core::life::mean(habits.filter_map(|habit| habit_score(state, habit)))
}

/// The open tasks filed under a project, each once.
pub fn project_tasks(state: &State, project: u64) -> Vec<&crate::calendar::Event> {
	use timeparagon_core::timeblocking::{TaskStatus, metadata, task_key, task_status};
	let mut seen = std::collections::HashSet::new();
	state
		.calendar
		.events
		.iter()
		.filter(|event| event.details.task_data.is_some())
		.filter(|event| task_status(event) == TaskStatus::Active)
		.filter(|event| metadata(event).projects.contains(&project))
		.filter(|event| seen.insert(task_key(event)))
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::config::Config;

	fn state() -> State {
		State::blank()
	}

	fn add(state: &mut State, name: &str) -> u64 {
		let _ = update(state, Action::NewName(name.into()));
		let _ = update(state, Action::Add);
		state
			.config
			.projects
			.areas
			.iter()
			.find(|area| area.name == name)
			.unwrap()
			.id
	}

	fn open_selection(state: &State) -> Option<Selection> {
		state.life.open.as_ref().map(|open| open.selection)
	}

	fn add_goal(state: &mut State, area: u64, name: &str) -> u64 {
		let _ = update(state, Action::Open(Selection::Area(area)));
		let _ = update(state, Action::NewGoal(name.into()));
		let _ = update(state, Action::AddGoal);
		state
			.config
			.projects
			.area(area)
			.unwrap()
			.goals
			.iter()
			.find(|goal| goal.name == name)
			.unwrap()
			.id
	}

	fn add_objective(state: &mut State, goal: u64, name: &str) -> u64 {
		let _ = update(state, Action::Open(Selection::Goal(goal)));
		let _ = update(state, Action::NewObjective(name.into()));
		let _ = update(state, Action::AddObjective);
		state
			.config
			.projects
			.all_objectives()
			.find(|(_, o)| o.name == name)
			.unwrap()
			.1
			.id
	}

	#[test]
	fn there_is_a_personal_area_to_start_with() {
		let state = state();
		let names: Vec<_> = state
			.config
			.projects
			.areas
			.iter()
			.map(|a| &a.name)
			.collect();
		assert_eq!(names, ["Personal"]);
		assert_eq!(state.config.projects.areas[0].id, PERSONAL_AREA);
	}

	#[test]
	fn adding_an_area_refuses_blank_and_repeated_names_and_opens_it() {
		let mut state = state();
		let id = add(&mut state, "Health");
		assert!(state.life.new_name.is_empty());
		assert_eq!(open_selection(&state), Some(Selection::Area(id)));

		let _ = update(&mut state, Action::NewName("   ".into()));
		let _ = update(&mut state, Action::Add);
		assert!(state.life.add_error.is_some());

		let _ = update(&mut state, Action::NewName("health".into()));
		let _ = update(&mut state, Action::Add);
		assert!(state.life.add_error.is_some());
		assert_eq!(state.config.projects.areas.len(), 2);
	}

	#[test]
	fn an_open_area_can_be_renamed_recolored_and_reordered() {
		let mut state = state();
		let id = add(&mut state, "Health");
		let red = Color::from_rgb8(0xDC, 0x21, 0x27);

		let _ = update(&mut state, Action::Rename("Personal".into()));
		assert!(state.life.open.as_ref().unwrap().name_error.is_some());
		assert_eq!(state.config.projects.area(id).unwrap().name, "Health");

		let _ = update(&mut state, Action::Rename(" Fitness ".into()));
		let _ = update(&mut state, Action::Color(Some(red)));
		let area = state.config.projects.area(id).unwrap();
		assert_eq!(area.name, "Fitness");
		assert_eq!(area.color(), Some(red));

		let _ = update(&mut state, Action::ShiftArea(id, true));
		assert_eq!(state.config.projects.areas[0].id, id);
	}

	#[test]
	fn the_vision_is_written_into_the_area() {
		let mut state = state();
		let _ = update(&mut state, Action::Open(Selection::Area(PERSONAL_AREA)));
		assert_eq!(
			state.life.open.as_ref().unwrap().vision_mode,
			VisionMode::Edit
		);
		for c in "# Calm".chars() {
			let _ = update(
				&mut state,
				Action::VisionEdited(text_editor::Action::Edit(text_editor::Edit::Insert(c))),
			);
		}
		assert_eq!(
			state.config.projects.area(PERSONAL_AREA).unwrap().vision,
			"# Calm"
		);
	}

	#[test]
	fn panel_pages_can_be_stepped_into_and_back_out_of() {
		let mut state = state();
		let goal = add_goal(&mut state, PERSONAL_AREA, "Fit");
		let objective = add_objective(&mut state, goal, "5k");
		let _ = update(&mut state, Action::Open(Selection::Objective(objective)));
		assert_eq!(
			state.life.open.as_ref().unwrap().back,
			[Selection::Area(PERSONAL_AREA), Selection::Goal(goal)]
		);
		let _ = update(&mut state, Action::Back);
		assert_eq!(open_selection(&state), Some(Selection::Goal(goal)));
		let _ = update(&mut state, Action::Back);
		assert_eq!(open_selection(&state), Some(Selection::Area(PERSONAL_AREA)));
		let _ = update(&mut state, Action::Back);
		assert_eq!(open_selection(&state), None);
	}

	#[test]
	fn goals_are_added_described_given_a_status_and_removed() {
		let mut state = state();
		let goal = add_goal(&mut state, PERSONAL_AREA, "Run a marathon");
		assert!(state.life.open.as_ref().unwrap().new_goal.is_empty());

		let _ = update(&mut state, Action::GoalName(goal, "Run a half".into()));
		let _ = update(&mut state, Action::GoalDescription(goal, "By June".into()));
		let _ = update(&mut state, Action::GoalWhy(goal, "Feel strong".into()));
		let _ = update(&mut state, Action::GoalStatus(goal, Status::Done));
		let saved = &state.config.projects.area(PERSONAL_AREA).unwrap().goals[0];
		assert_eq!(
			(
				saved.name.as_str(),
				saved.description.as_str(),
				saved.why.as_str(),
				saved.status
			),
			("Run a half", "By June", "Feel strong", Status::Done)
		);

		let _ = update(&mut state, Action::Open(Selection::Goal(goal)));
		let _ = update(&mut state, Action::ConfirmRemove);
		assert!(
			state.config.projects.find_goal(goal).is_some(),
			"needs a yes"
		);
		let _ = update(&mut state, Action::Remove);
		let _ = update(&mut state, Action::ConfirmRemove);
		assert!(state.config.projects.find_goal(goal).is_none());
		assert_eq!(
			open_selection(&state),
			Some(Selection::Area(PERSONAL_AREA)),
			"goes back to where it was opened from"
		);
	}

	#[test]
	fn a_goal_can_be_added_from_the_goals_page_into_any_area() {
		let mut state = state();
		let health = add(&mut state, "Health");
		let _ = update(&mut state, Action::QuickGoal("Sleep well".into()));
		let _ = update(&mut state, Action::QuickGoalArea(health));
		let _ = update(&mut state, Action::AddQuickGoal);
		let (area, goal) = state
			.config
			.projects
			.areas
			.iter()
			.find_map(|a| a.goals.first().map(|g| (a.id, g.id)))
			.unwrap();
		assert_eq!(area, health);
		assert_eq!(open_selection(&state), Some(Selection::Goal(goal)));
	}

	#[test]
	fn objectives_sit_under_goals_and_fall_back_to_unassigned() {
		let mut state = state();
		let goal = add_goal(&mut state, PERSONAL_AREA, "Fit");
		let first = add_objective(&mut state, goal, " 5k run ".trim());
		let _ = update(&mut state, Action::Open(Selection::Area(PERSONAL_AREA)));
		let _ = update(&mut state, Action::NewObjective("Read more".into()));
		let _ = update(&mut state, Action::AddObjective);

		let area = state.config.projects.area(PERSONAL_AREA).unwrap();
		assert_eq!(area.objectives.len(), 2);
		assert_eq!(
			area.objective_goal(area.objective(first).unwrap()),
			Some(goal)
		);
		assert_eq!(area.objective_goal(&area.objectives[1]), None);
		assert_eq!(area.objectives[0].start, Some(today()), "pace starts today");

		// Removing the goal leaves its objectives, unassigned.
		let _ = update(&mut state, Action::Open(Selection::Goal(goal)));
		let _ = update(&mut state, Action::Remove);
		let _ = update(&mut state, Action::ConfirmRemove);
		let area = state.config.projects.area(PERSONAL_AREA).unwrap();
		assert_eq!(area.objectives.len(), 2);
		assert!(area.objectives.iter().all(|o| o.goal.is_none()));
	}

	#[test]
	fn a_number_objective_is_measured_by_its_check_ins() {
		let mut state = state();
		let goal = add_goal(&mut state, PERSONAL_AREA, "Lighter");
		let id = add_objective(&mut state, goal, "Weigh 80 kg");
		let _ = update(&mut state, Action::Open(Selection::Objective(id)));
		let _ = update(&mut state, Action::Measure(id, Measure::Number));
		let _ = update(&mut state, Action::Baseline("90".into()));
		let _ = update(&mut state, Action::Target("8".into()));
		let _ = update(&mut state, Action::Target("80,0".into()));
		let _ = update(&mut state, Action::Unit(id, "kg".into()));
		let _ = update(&mut state, Action::CheckInValue("85".into()));
		let _ = update(&mut state, Action::CheckIn);
		let (_, objective) = state.config.projects.find_objective(id).unwrap();
		assert_eq!(
			(objective.baseline, objective.target),
			(Some(90.0), Some(80.0))
		);
		assert_eq!(objective.progress(&|_| None), Some(0.5));
		assert!(state.life.open.as_ref().unwrap().check_in.is_empty());

		// A second check-in the same day replaces the first.
		let _ = update(&mut state, Action::CheckInValue("82".into()));
		let _ = update(&mut state, Action::CheckIn);
		let (_, objective) = state.config.projects.find_objective(id).unwrap();
		assert_eq!(objective.check_ins.len(), 1);
		assert_eq!(objective.current(), Some(82.0));

		// Typing nonsense does not clear what was saved.
		let _ = update(&mut state, Action::Baseline("abc".into()));
		let (_, objective) = state.config.projects.find_objective(id).unwrap();
		assert_eq!(objective.baseline, Some(90.0));
	}

	#[test]
	fn milestones_measure_an_objective_and_done_finishes_it() {
		let mut state = state();
		let goal = add_goal(&mut state, PERSONAL_AREA, "Write");
		let id = add_objective(&mut state, goal, "Finish draft");
		let _ = update(&mut state, Action::Open(Selection::Objective(id)));
		let _ = update(&mut state, Action::Measure(id, Measure::Milestones));
		for name in ["Outline", "Chapter 1"] {
			let _ = update(&mut state, Action::NewMilestone(name.into()));
			let _ = update(&mut state, Action::AddMilestone);
		}
		let first = state
			.config
			.projects
			.find_objective(id)
			.unwrap()
			.1
			.milestones[0]
			.id;
		let _ = update(&mut state, Action::ToggleMilestone(id, first));
		let projects = &state.config.projects;
		let (area, objective) = projects.find_objective(id).unwrap();
		assert_eq!(objective.progress(&|_| None), Some(0.5));
		assert_eq!(
			area.goal_progress(area.goal(goal).unwrap(), &|_| None),
			Some(0.5)
		);

		let _ = update(&mut state, Action::Focus(id, true));
		let _ = update(&mut state, Action::ObjectiveStatus(id, Status::Done));
		let (_, objective) = state.config.projects.find_objective(id).unwrap();
		assert_eq!(objective.progress(&|_| None), Some(1.0));
		assert!(!objective.focus, "done is no longer in focus");
	}

	#[test]
	fn a_due_date_can_be_picked_by_horizon() {
		let monday = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
		let today = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
		let end = |h: Horizon| h.end(today, monday);
		assert_eq!(
			end(Horizon::Week),
			NaiveDate::from_ymd_opt(2026, 11, 15).unwrap()
		);
		assert_eq!(
			end(Horizon::Month),
			NaiveDate::from_ymd_opt(2026, 11, 30).unwrap()
		);
		assert_eq!(
			end(Horizon::Quarter),
			NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()
		);
		assert_eq!(
			end(Horizon::Year),
			NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()
		);

		let mut state = state();
		let goal = add_goal(&mut state, PERSONAL_AREA, "Fit");
		let id = add_objective(&mut state, goal, "5k");
		let _ = update(&mut state, Action::Due(id, "2027-03-01".into()));
		assert_eq!(
			state.config.projects.find_objective(id).unwrap().1.due,
			NaiveDate::from_ymd_opt(2027, 3, 1)
		);
		let _ = update(&mut state, Action::Due(id, String::new()));
		assert_eq!(
			state.config.projects.find_objective(id).unwrap().1.due,
			None
		);
	}

	#[test]
	fn projects_are_added_linked_edited_and_removed() {
		let mut state = state();
		let health = add(&mut state, "Health");
		let _ = update(&mut state, Action::NewProject(" Gym / Plan ".into()));
		let _ = update(&mut state, Action::AddProject);
		let id = state
			.config
			.projects
			.items
			.iter()
			.find(|p| p.name == "Gym/Plan")
			.unwrap()
			.id;
		assert_eq!(state.config.projects.get(id).unwrap().area, Some(health));

		let _ = update(&mut state, Action::NewProject("work".into()));
		let _ = update(&mut state, Action::AddProject);
		assert!(
			state
				.life
				.open
				.as_ref()
				.unwrap()
				.new_project_error
				.is_some()
		);

		let goal = add_goal(&mut state, health, "Strong");
		let objective = add_objective(&mut state, goal, "Squat 100 kg");
		let _ = update(&mut state, Action::LinkProject(objective, id));
		let _ = update(&mut state, Action::LinkProject(objective, id));
		assert_eq!(
			state
				.config
				.projects
				.find_objective(objective)
				.unwrap()
				.1
				.projects,
			[id]
		);

		let red = Color::from_rgb8(0xDC, 0x21, 0x27);
		let _ = update(&mut state, Action::Open(Selection::Project(id)));
		let _ = update(&mut state, Action::Rename("Work".into()));
		assert!(state.life.open.as_ref().unwrap().name_error.is_some());
		let _ = update(&mut state, Action::Rename("Gym".into()));
		let _ = update(&mut state, Action::Color(Some(red)));
		let _ = update(&mut state, Action::ProjectDirectory(" /tmp/gym ".into()));
		let project = state.config.projects.get(id).unwrap();
		assert_eq!(project.name, "Gym");
		assert_eq!(project.color(), Some(red));
		assert_eq!(
			project.directory.as_deref(),
			Some(std::path::Path::new("/tmp/gym"))
		);
		let _ = update(&mut state, Action::ProjectDirectory("".into()));
		assert_eq!(state.config.projects.get(id).unwrap().directory, None);

		let _ = update(&mut state, Action::Remove);
		let _ = update(&mut state, Action::ConfirmRemove);
		assert!(state.config.projects.get(id).is_none());
		assert!(
			state
				.config
				.projects
				.find_objective(objective)
				.unwrap()
				.1
				.projects
				.is_empty()
		);
	}

	#[test]
	fn projects_without_an_area_are_personal_and_can_be_moved() {
		let mut state = state();
		let health = add(&mut state, "Health");
		let project = state.config.projects.items[0].id;
		let projects = &state.config.projects;
		assert!(
			projects
				.items
				.iter()
				.all(|p| projects.resolve_area(p.area) == PERSONAL_AREA)
		);

		let _ = update(&mut state, Action::MoveProject(project, health));
		assert_eq!(state.config.projects.items[0].area, Some(health));

		// Removing the area puts what was in it back in Personal.
		let _ = update(&mut state, Action::Open(Selection::Area(health)));
		let _ = update(&mut state, Action::Remove);
		let _ = update(&mut state, Action::ConfirmRemove);
		assert!(state.config.projects.area(health).is_none());
		assert_eq!(
			state
				.config
				.projects
				.resolve_area(state.config.projects.items[0].area),
			PERSONAL_AREA
		);
	}

	#[test]
	fn habits_can_be_moved_between_areas() {
		let mut state = state();
		let health = add(&mut state, "Health");
		state
			.habits
			.library
			.habits
			.push(crate::habits::Habit::new("h1", "Stretch"));

		let _ = update(&mut state, Action::MoveHabit("h1".into(), health));
		assert_eq!(state.habits.library.habit("h1").unwrap().area, Some(health));
		let _ = update(&mut state, Action::MoveHabit("h1".into(), PERSONAL_AREA));
		assert_eq!(state.habits.library.habit("h1").unwrap().area, None);
	}

	#[test]
	fn a_habit_carries_out_one_objective_and_moves_into_its_area() {
		let mut state = state();
		let health = add(&mut state, "Health");
		state
			.habits
			.library
			.habits
			.push(crate::habits::Habit::new("h1", "Stretch"));
		let goal = add_goal(&mut state, health, "Supple");
		let a = add_objective(&mut state, goal, "A");
		let b = add_objective(&mut state, goal, "B");

		let _ = update(&mut state, Action::AddHabit(a, "h1".into()));
		assert_eq!(state.habits.library.habit("h1").unwrap().area, Some(health));
		let _ = update(&mut state, Action::AddHabit(b, "h1".into()));
		let area = state.config.projects.area(health).unwrap();
		assert!(area.objective(a).unwrap().habits.is_empty());
		assert_eq!(area.objective(b).unwrap().habits, ["h1"]);
		assert!(area.has_habit("h1"));

		let _ = update(&mut state, Action::RemoveHabit(b, "h1".into()));
		assert!(!state.config.projects.area(health).unwrap().has_habit("h1"));
	}

	#[test]
	fn goals_and_objectives_are_told_apart_across_areas() {
		let mut state = state();
		let health = add(&mut state, "Health");
		let work = add(&mut state, "Work");
		let a = add_goal(&mut state, health, "Fit");
		let b = add_goal(&mut state, work, "Promoted");
		assert_ne!(a, b);
		let o = add_objective(&mut state, b, "Ship it");
		assert_eq!(state.config.projects.find_objective(o).unwrap().0.id, work);

		// Data from before ids were unique across areas is repaired.
		let projects = &mut state.config.projects;
		projects.area_mut(health).unwrap().goals[0].id = b;
		projects
			.area_mut(health)
			.unwrap()
			.objectives
			.push(crate::config::Objective::new(o, "Run".into(), Some(b)));
		assert!(projects.ensure_unique_life_ids());
		assert!(!projects.ensure_unique_life_ids(), "once is enough");
		// The earlier area keeps its ids; the later one's are renewed.
		let (first, second) = (projects.area(health).unwrap(), projects.area(work).unwrap());
		assert_eq!(first.goals[0].id, b);
		assert_ne!(second.goals[0].id, b);
		assert_ne!(first.objectives[0].id, second.objectives[0].id);
		// Each objective still serves the goal of its own area.
		assert_eq!(
			first.objective_goal(&first.objectives[0]),
			Some(first.goals[0].id)
		);
		assert_eq!(
			second.objective_goal(&second.objectives[0]),
			Some(second.goals[0].id)
		);
	}

	#[test]
	fn the_personal_area_cannot_be_removed() {
		let mut state = state();
		let _ = update(&mut state, Action::Open(Selection::Area(PERSONAL_AREA)));
		let _ = update(&mut state, Action::Remove);
		assert!(!state.life.open.as_ref().unwrap().confirm_remove);
		state.config.projects.remove_area(PERSONAL_AREA);
		assert!(state.config.projects.area(PERSONAL_AREA).is_some());
	}

	#[test]
	fn the_weekly_review_rates_areas_checks_in_and_reflects() {
		let mut state = state();
		let goal = add_goal(&mut state, PERSONAL_AREA, "Lighter");
		let id = add_objective(&mut state, goal, "80 kg");
		let _ = update(&mut state, Action::ShowPage(Page::Review));
		let week = review_week(&state);

		let _ = update(&mut state, Action::Rate(PERSONAL_AREA, 7));
		let _ = update(&mut state, Action::Rate(PERSONAL_AREA, 8));
		let area = state.config.projects.area(PERSONAL_AREA).unwrap();
		assert_eq!(area.ratings.len(), 1, "one rating a week");
		assert_eq!(area.rating_for(week), Some(8));

		let _ = update(&mut state, Action::ReviewCheckInValue(id, "84".into()));
		let _ = update(&mut state, Action::ReviewCheckIn(id));
		assert_eq!(
			state
				.config
				.projects
				.find_objective(id)
				.unwrap()
				.1
				.current(),
			Some(84.0)
		);

		for c in "Good".chars() {
			let _ = update(
				&mut state,
				Action::ReviewEdited(text_editor::Action::Edit(text_editor::Edit::Insert(c))),
			);
		}
		let _ = update(&mut state, Action::FinishReview);
		let review = state.config.projects.review(week).unwrap();
		assert_eq!(review.reflection, "Good");
		assert!(review.done);
	}

	#[test]
	fn areas_goals_and_objectives_survive_the_config_file() {
		let mut config = Config::default();
		let id = config.projects.add_area("Health".into());
		let goal = config.projects.add_goal(id, "Sleep well".into()).unwrap();
		let objective = config
			.projects
			.add_objective(id, Some(goal), "8 hours".into())
			.unwrap();
		{
			let o = config.projects.objective_mut(objective).unwrap();
			o.measure = Measure::Number;
			o.baseline = Some(6.0);
			o.target = Some(8.0);
			o.check_in(today(), 7.0, "ok".into());
			o.add_milestone("Blackout curtains".into());
			o.focus = true;
		}
		config.projects.area_mut(id).unwrap().vision = "# Strong".into();
		config.projects.area_mut(id).unwrap().rate(today(), 6);
		config.projects.review_mut(today()).reflection = "Fine".into();
		config.projects.items[0].area = Some(id);

		let text = toml::to_string_pretty(&config).unwrap();
		let read: Config = toml::from_str(&text).unwrap();
		assert_eq!(read.projects, config.projects);

		let json = serde_json::to_string(&config.projects).unwrap();
		let read: crate::config::ProjectsSettings = serde_json::from_str(&json).unwrap();
		assert_eq!(read, config.projects);

		// A file from before life areas still has Personal.
		let read: Config = toml::from_str("[projects]\nitems = []\n").unwrap();
		assert_eq!(read.projects.areas.len(), 1);
		assert_eq!(read.projects.areas[0].id, PERSONAL_AREA);
	}
}
