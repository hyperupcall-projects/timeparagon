//! The Life tab: life areas, each with a vision, goals, and the projects and
//! habits that belong to it.
//!
//! The areas themselves live in the config beside the projects
//! (`config::ProjectsSettings::areas`), so they are saved and synced with
//! them. This holds what the tab is in the middle of: a name being typed, the
//! area open in the side panel, and its vision being edited.

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
    /// Whether removing the area is waiting for a yes.
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

        Action::RemoveGoal(goal) => edit_area(state, |area| area.goals.retain(|g| g.id != goal)),

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
