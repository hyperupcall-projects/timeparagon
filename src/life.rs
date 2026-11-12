//! The Life tab: life areas, each with a vision, goals, and the projects and
//! habits that belong to it.
//!
//! The areas themselves live in the config beside the projects
//! (`config::ProjectsSettings::areas`), so they are saved and synced with
//! them. This holds what the tab is in the middle of: a name being typed, the
//! area open in the side panel, and its vision being edited.

use std::collections::HashMap;

use iced::widget::text_editor;
use iced::{Color, Task};

use crate::app::{Message, State, queue_save};
use crate::config::PERSONAL_AREA;

#[derive(Default)]
pub struct Workspace {
	/// The name typed into the add box.
	pub new_name: String,
	pub add_error: Option<String>,
	/// The area open in the side panel.
	pub open: Option<Open>,
}

impl std::fmt::Debug for Workspace {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Workspace")
			.field("new_name", &self.new_name)
			.field("add_error", &self.add_error)
			.field("open", &self.open.as_ref().map(|open| open.area))
			.finish()
	}
}

/// The two tabs of an open life area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
	#[default]
	Vision,
	Implementation,
}

/// Whether the vision is being written or read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisionMode {
	Edit,
	#[default]
	View,
}

/// A life area open in the side panel.
pub struct Open {
	pub area: u64,
	pub tab: Tab,
	/// The name as typed. It is saved whenever it is a valid name, so a name
	/// that is half-typed or taken is kept here without being lost.
	pub name: String,
	pub name_error: Option<String>,
	pub vision: text_editor::Content,
	pub vision_mode: VisionMode,
	/// The name typed into the add-goal box.
	pub new_goal: String,
	/// The names typed into the add-objective boxes, by the goal they add
	/// under (`None` for unassigned objectives).
	pub new_objective: HashMap<Option<u64>, String>,
	/// The name typed into the add-project box.
	pub new_project: String,
	pub new_project_error: Option<String>,
	/// The project whose details are being edited.
	pub project: Option<ProjectEdit>,
	/// Whether removing the area is waiting for a yes.
	pub confirm_remove: bool,
}

/// A project being edited in the Implementation tab.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectEdit {
	pub id: u64,
	/// The name as typed; saved whenever it is a valid name.
	pub name: String,
	pub name_error: Option<String>,
	/// The directory as typed; blank means none.
	pub directory: String,
	/// Whether removing the project is waiting for a yes.
	pub confirm_remove: bool,
}

#[derive(Debug, Clone)]
pub enum Action {
	/// The add box changed.
	NewName(String),
	Add,
	/// Open this area in the side panel.
	Open(u64),
	Close,
	ShowTab(Tab),
	Rename(String),
	Color(Option<Color>),
	VisionMode(VisionMode),
	VisionEdited(text_editor::Action),
	NewGoal(String),
	AddGoal,
	GoalName(u64, String),
	GoalDescription(u64, String),
	RemoveGoal(u64),
	/// The add-objective box under a goal (or unassigned) changed.
	NewObjective(Option<u64>, String),
	AddObjective(Option<u64>),
	ObjectiveName(u64, String),
	ObjectiveDescription(u64, String),
	/// Put an objective under another goal, or none.
	ObjectiveGoal(u64, Option<u64>),
	RemoveObjective(u64),
	/// Have an objective carried out by an existing habit.
	AddHabit(u64, String),
	RemoveHabit(u64, String),
	NewProject(String),
	/// Add a project to the open area.
	AddProject,
	/// Open or close a project's details.
	EditProject(u64),
	ProjectName(String),
	ProjectColor(Option<Color>),
	ProjectDirectory(String),
	RemoveProject,
	ConfirmRemoveProject,
	CancelRemoveProject,
	/// Move a project into an area.
	MoveProject(u64, u64),
	/// Move a habit into an area.
	MoveHabit(String, u64),
	Remove,
	ConfirmRemove,
	CancelRemove,
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
	state.config.projects.ensure_personal_area();

	match action {
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
					state.config.projects.add_area(name);
					state.life.new_name.clear();
					state.life.add_error = None;
					queue_save(state)
				}
				Err(error) => {
					state.life.add_error = Some(error);
					Task::none()
				}
			}
		}

		Action::Open(id) => {
			if state.life.open.as_ref().is_some_and(|open| open.area == id) {
				return Task::none();
			}
			let Some(area) = state.config.projects.area(id) else {
				return Task::none();
			};
			state.life.open = Some(Open {
				area: id,
				tab: Tab::Vision,
				name: area.name.clone(),
				name_error: None,
				vision: text_editor::Content::with_text(&area.vision),
				// An empty vision opens ready to be written.
				vision_mode: if area.vision.trim().is_empty() {
					VisionMode::Edit
				} else {
					VisionMode::View
				},
				new_goal: String::new(),
				new_objective: HashMap::new(),
				new_project: String::new(),
				new_project_error: None,
				project: None,
				confirm_remove: false,
			});
			Task::none()
		}

		Action::Close => {
			state.life.open = None;
			Task::none()
		}

		Action::ShowTab(tab) => {
			if let Some(open) = &mut state.life.open {
				open.tab = tab;
			}
			Task::none()
		}

		Action::Rename(name) => {
			let Some(open) = &mut state.life.open else {
				return Task::none();
			};
			open.name = name;
			let id = open.area;
			match state.config.projects.clean_area_name(&open.name, Some(id)) {
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
			}
		}

		Action::Color(color) => edit_area(state, |area| area.color = color.map(Into::into)),

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
			let is_edit = action.is_edit();
			open.vision.perform(action);
			if !is_edit {
				return Task::none();
			}
			let vision = open.vision.text();
			edit_area(state, |area| area.vision = vision)
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
			let name = open.new_goal.trim().to_owned();
			if name.is_empty() {
				return Task::none();
			}
			open.new_goal.clear();
			let id = open.area;
			state.config.projects.add_goal(id, name);
			queue_save(state)
		}

		Action::GoalName(goal, name) => edit_goal(state, goal, |goal| goal.name = name),

		Action::GoalDescription(goal, description) => {
			edit_goal(state, goal, |goal| goal.description = description)
		}

		Action::RemoveGoal(goal) => edit_area(state, |area| {
			area.goals.retain(|g| g.id != goal);
			// Its objectives stay, unassigned.
			for objective in &mut area.objectives {
				if objective.goal == Some(goal) {
					objective.goal = None;
				}
			}
		}),

		Action::NewObjective(goal, name) => {
			if let Some(open) = &mut state.life.open {
				open.new_objective.insert(goal, name);
			}
			Task::none()
		}

		Action::AddObjective(goal) => {
			let Some(open) = &mut state.life.open else {
				return Task::none();
			};
			let name = open
				.new_objective
				.get(&goal)
				.map(|name| name.trim().to_owned())
				.unwrap_or_default();
			if name.is_empty() {
				return Task::none();
			}
			open.new_objective.remove(&goal);
			let id = open.area;
			state.config.projects.add_objective(id, goal, name);
			queue_save(state)
		}

		Action::ObjectiveName(objective, name) => {
			edit_objective(state, objective, |objective| objective.name = name)
		}

		Action::ObjectiveDescription(objective, description) => {
			edit_objective(state, objective, |objective| {
				objective.description = description
			})
		}

		Action::ObjectiveGoal(objective, goal) => {
			edit_objective(state, objective, |objective| objective.goal = goal)
		}

		Action::RemoveObjective(objective) => {
			edit_area(state, |area| area.objectives.retain(|o| o.id != objective))
		}

		Action::AddHabit(objective, habit) => {
			let Some(area) = state.life.open.as_ref().map(|open| open.area) else {
				return Task::none();
			};
			if state.habits.library.habit(&habit).is_none() {
				return Task::none();
			}
			// A habit carries out one objective at a time.
			for other in &mut state.config.projects.areas {
				for o in &mut other.objectives {
					o.habits.retain(|h| *h != habit);
				}
			}
			let save = edit_objective(state, objective, |objective| {
				objective.habits.push(habit.clone())
			});
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

		Action::RemoveHabit(objective, habit) => edit_objective(state, objective, |objective| {
			objective.habits.retain(|h| *h != habit)
		}),

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
			match state.config.projects.clean_name(&open.new_project, None) {
				Ok(name) => {
					let id = state.config.projects.add(name);
					let area = open.area;
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

		Action::EditProject(id) => {
			let Some(open) = &mut state.life.open else {
				return Task::none();
			};
			if open.project.as_ref().is_some_and(|edit| edit.id == id) {
				open.project = None;
				return Task::none();
			}
			let Some(project) = state.config.projects.get(id) else {
				return Task::none();
			};
			open.project = Some(ProjectEdit {
				id,
				name: project.name.clone(),
				name_error: None,
				directory: project
					.directory
					.as_ref()
					.map(|directory| directory.display().to_string())
					.unwrap_or_default(),
				confirm_remove: false,
			});
			Task::none()
		}

		Action::ProjectName(name) => {
			let Some(edit) = state
				.life
				.open
				.as_mut()
				.and_then(|open| open.project.as_mut())
			else {
				return Task::none();
			};
			edit.name = name;
			let id = edit.id;
			match state.config.projects.clean_name(&edit.name, Some(id)) {
				Ok(name) => {
					edit.name_error = None;
					edit_project(state, |project| project.name = name)
				}
				Err(error) => {
					edit.name_error = Some(error);
					Task::none()
				}
			}
		}

		Action::ProjectColor(color) => {
			edit_project(state, |project| project.color = color.map(Into::into))
		}

		Action::ProjectDirectory(directory) => {
			let Some(edit) = state
				.life
				.open
				.as_mut()
				.and_then(|open| open.project.as_mut())
			else {
				return Task::none();
			};
			edit.directory = directory;
			let trimmed = edit.directory.trim();
			let directory = (!trimmed.is_empty()).then(|| std::path::PathBuf::from(trimmed));
			edit_project(state, |project| project.directory = directory)
		}

		Action::RemoveProject => {
			if let Some(edit) = state
				.life
				.open
				.as_mut()
				.and_then(|open| open.project.as_mut())
			{
				edit.confirm_remove = true;
			}
			Task::none()
		}

		Action::ConfirmRemoveProject => {
			let Some(edit) = state
				.life
				.open
				.as_mut()
				.and_then(|open| open.project.take())
			else {
				return Task::none();
			};
			state.config.projects.remove(edit.id);
			queue_save(state)
		}

		Action::CancelRemoveProject => {
			if let Some(edit) = state
				.life
				.open
				.as_mut()
				.and_then(|open| open.project.as_mut())
			{
				edit.confirm_remove = false;
			}
			Task::none()
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

		Action::Remove => {
			if let Some(open) = &mut state.life.open
				&& open.area != PERSONAL_AREA
			{
				open.confirm_remove = true;
			}
			Task::none()
		}

		Action::ConfirmRemove => {
			let Some(open) = state.life.open.take() else {
				return Task::none();
			};
			// What was in it falls back to Personal on its own, since an area
			// that is gone resolves to Personal.
			state.config.projects.remove_area(open.area);
			queue_save(state)
		}

		Action::CancelRemove => {
			if let Some(open) = &mut state.life.open {
				open.confirm_remove = false;
			}
			Task::none()
		}
	}
}

/// Changes the open area and saves.
fn edit_area(
	state: &mut State,
	change: impl FnOnce(&mut crate::config::LifeArea),
) -> Task<Message> {
	let Some(id) = state.life.open.as_ref().map(|open| open.area) else {
		return Task::none();
	};
	let Some(area) = state.config.projects.area_mut(id) else {
		return Task::none();
	};
	change(area);
	queue_save(state)
}

fn edit_goal(
	state: &mut State,
	goal: u64,
	change: impl FnOnce(&mut crate::config::Goal),
) -> Task<Message> {
	edit_area(state, |area| {
		if let Some(goal) = area.goals.iter_mut().find(|g| g.id == goal) {
			change(goal);
		}
	})
}

fn edit_objective(
	state: &mut State,
	objective: u64,
	change: impl FnOnce(&mut crate::config::Objective),
) -> Task<Message> {
	edit_area(state, |area| {
		if let Some(objective) = area.objectives.iter_mut().find(|o| o.id == objective) {
			change(objective);
		}
	})
}

/// Changes the project being edited and saves, keeping the list in order.
fn edit_project(
	state: &mut State,
	change: impl FnOnce(&mut crate::config::Project),
) -> Task<Message> {
	let Some(id) = state
		.life
		.open
		.as_ref()
		.and_then(|open| open.project.as_ref())
		.map(|edit| edit.id)
	else {
		return Task::none();
	};
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
	fn adding_an_area_refuses_blank_and_repeated_names() {
		let mut state = state();
		add(&mut state, "Health");
		assert!(state.life.new_name.is_empty());

		let _ = update(&mut state, Action::NewName("   ".into()));
		let _ = update(&mut state, Action::Add);
		assert!(state.life.add_error.is_some());

		let _ = update(&mut state, Action::NewName("health".into()));
		let _ = update(&mut state, Action::Add);
		assert!(state.life.add_error.is_some());
		assert_eq!(state.config.projects.areas.len(), 2);
	}

	#[test]
	fn an_open_area_can_be_renamed_and_recolored() {
		let mut state = state();
		let id = add(&mut state, "Health");
		let red = Color::from_rgb8(0xDC, 0x21, 0x27);

		let _ = update(&mut state, Action::Open(id));
		let _ = update(&mut state, Action::Rename("Personal".into()));
		assert!(state.life.open.as_ref().unwrap().name_error.is_some());
		assert_eq!(state.config.projects.area(id).unwrap().name, "Health");

		let _ = update(&mut state, Action::Rename(" Fitness ".into()));
		let _ = update(&mut state, Action::Color(Some(red)));
		let area = state.config.projects.area(id).unwrap();
		assert_eq!(area.name, "Fitness");
		assert_eq!(area.color(), Some(red));
	}

	#[test]
	fn the_vision_is_written_into_the_area() {
		let mut state = state();
		let _ = update(&mut state, Action::Open(PERSONAL_AREA));
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
	fn goals_are_added_renamed_described_and_removed() {
		let mut state = state();
		let _ = update(&mut state, Action::Open(PERSONAL_AREA));
		let _ = update(&mut state, Action::NewGoal("  Run a marathon ".into()));
		let _ = update(&mut state, Action::AddGoal);
		let goal = state.config.projects.area(PERSONAL_AREA).unwrap().goals[0].id;
		assert!(state.life.open.as_ref().unwrap().new_goal.is_empty());

		let _ = update(&mut state, Action::GoalName(goal, "Run a half".into()));
		let _ = update(&mut state, Action::GoalDescription(goal, "By June".into()));
		let saved = &state.config.projects.area(PERSONAL_AREA).unwrap().goals[0];
		assert_eq!(
			(saved.name.as_str(), saved.description.as_str()),
			("Run a half", "By June")
		);

		let _ = update(&mut state, Action::RemoveGoal(goal));
		assert!(
			state
				.config
				.projects
				.area(PERSONAL_AREA)
				.unwrap()
				.goals
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
		assert_eq!(
			state.config.projects.items[0].area,
			Some(health),
			"moved into Health"
		);

		// Removing the area puts what was in it back in Personal.
		let _ = update(&mut state, Action::Open(health));
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
	fn objectives_sit_under_goals_and_fall_back_to_unassigned() {
		let mut state = state();
		let _ = update(&mut state, Action::Open(PERSONAL_AREA));
		let _ = update(&mut state, Action::NewGoal("Fit".into()));
		let _ = update(&mut state, Action::AddGoal);
		let goal = state.config.projects.area(PERSONAL_AREA).unwrap().goals[0].id;

		let _ = update(
			&mut state,
			Action::NewObjective(Some(goal), " 5k run ".into()),
		);
		let _ = update(&mut state, Action::AddObjective(Some(goal)));
		let _ = update(&mut state, Action::NewObjective(None, "Read more".into()));
		let _ = update(&mut state, Action::AddObjective(None));
		let area = state.config.projects.area(PERSONAL_AREA).unwrap();
		assert_eq!(area.objectives.len(), 2);
		assert_eq!(area.objectives[0].name, "5k run");
		assert_eq!(area.objective_goal(&area.objectives[0]), Some(goal));
		assert_eq!(area.objective_goal(&area.objectives[1]), None);
		assert!(state.life.open.as_ref().unwrap().new_objective.is_empty());

		// Removing the goal leaves its objectives, unassigned.
		let _ = update(&mut state, Action::RemoveGoal(goal));
		let area = state.config.projects.area(PERSONAL_AREA).unwrap();
		assert_eq!(area.objectives.len(), 2);
		assert!(area.objectives.iter().all(|o| o.goal.is_none()));

		let objective = area.objectives[1].id;
		let _ = update(&mut state, Action::RemoveObjective(objective));
		assert_eq!(
			state
				.config
				.projects
				.area(PERSONAL_AREA)
				.unwrap()
				.objectives
				.len(),
			1
		);
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
		let _ = update(&mut state, Action::Open(health));
		for name in ["A", "B"] {
			let _ = update(&mut state, Action::NewObjective(None, name.into()));
			let _ = update(&mut state, Action::AddObjective(None));
		}
		let ids: Vec<u64> = state
			.config
			.projects
			.area(health)
			.unwrap()
			.objectives
			.iter()
			.map(|o| o.id)
			.collect();

		let _ = update(&mut state, Action::AddHabit(ids[0], "h1".into()));
		assert_eq!(state.habits.library.habit("h1").unwrap().area, Some(health));
		let _ = update(&mut state, Action::AddHabit(ids[1], "h1".into()));
		let area = state.config.projects.area(health).unwrap();
		assert!(area.objectives[0].habits.is_empty());
		assert_eq!(area.objectives[1].habits, ["h1"]);
		assert!(area.has_habit("h1"));

		let _ = update(&mut state, Action::RemoveHabit(ids[1], "h1".into()));
		assert!(!state.config.projects.area(health).unwrap().has_habit("h1"));
	}

	#[test]
	fn projects_are_added_edited_and_removed_from_an_area() {
		let mut state = state();
		let health = add(&mut state, "Health");
		let _ = update(&mut state, Action::Open(health));
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

		let red = Color::from_rgb8(0xDC, 0x21, 0x27);
		let _ = update(&mut state, Action::EditProject(id));
		let _ = update(&mut state, Action::ProjectName("Work".into()));
		assert!(
			state
				.life
				.open
				.as_ref()
				.unwrap()
				.project
				.as_ref()
				.unwrap()
				.name_error
				.is_some()
		);
		let _ = update(&mut state, Action::ProjectName("Gym".into()));
		let _ = update(&mut state, Action::ProjectColor(Some(red)));
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

		let _ = update(&mut state, Action::RemoveProject);
		assert!(state.config.projects.get(id).is_some());
		let _ = update(&mut state, Action::ConfirmRemoveProject);
		assert!(state.config.projects.get(id).is_none());
		assert!(state.life.open.as_ref().unwrap().project.is_none());
	}

	#[test]
	fn the_personal_area_cannot_be_removed() {
		let mut state = state();
		let _ = update(&mut state, Action::Open(PERSONAL_AREA));
		let _ = update(&mut state, Action::Remove);
		assert!(!state.life.open.as_ref().unwrap().confirm_remove);
		state.config.projects.remove_area(PERSONAL_AREA);
		assert!(state.config.projects.area(PERSONAL_AREA).is_some());
	}

	#[test]
	fn areas_and_goals_survive_the_config_file() {
		let mut config = Config::default();
		let id = config.projects.add_area("Health".into());
		config.projects.add_goal(id, "Sleep well".into());
		config.projects.area_mut(id).unwrap().vision = "# Strong".into();
		config.projects.items[0].area = Some(id);

		let text = toml::to_string_pretty(&config).unwrap();
		let read: Config = toml::from_str(&text).unwrap();
		assert_eq!(read.projects, config.projects);

		// A file from before life areas still has Personal.
		let read: Config = toml::from_str("[projects]\nitems = []\n").unwrap();
		assert_eq!(read.projects.areas.len(), 1);
		assert_eq!(read.projects.areas[0].id, PERSONAL_AREA);
	}
}
