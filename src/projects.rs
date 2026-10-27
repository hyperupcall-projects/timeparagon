//! Adding, renaming, recoloring and removing projects.
//!
//! The projects themselves live in the config (`config::ProjectsSettings`);
//! this holds what the Projects tab is in the middle of: a name being typed,
//! a project being edited, a removal waiting to be confirmed.

use std::path::PathBuf;

use iced::{Color, Task};

use crate::app::{Message, State, queue_save};
use crate::blocking;

#[derive(Debug, Default)]
pub struct Workspace {
    /// The name typed into the add box.
    pub new_name: String,
    pub add_error: Option<String>,
    /// The project being edited, with its changes so far.
    pub editing: Option<Draft>,
    /// The project whose removal is waiting for a yes.
    pub confirm_remove: Option<u64>,
}

/// A project's fields as they are being edited, before they are saved.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub id: u64,
    pub name: String,
    pub color: Option<Color>,
    /// Text rather than a path, so a half-typed one is kept as it is.
    pub directory: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Action {
    /// The add box changed.
    NewName(String),
    Add,
    /// Start editing this project.
    Edit(u64),
    CancelEdit,
    Name(String),
    Color(Option<Color>),
    Directory(String),
    /// Open the system folder chooser for the project's directory.
    Browse,
    Browsed(Option<PathBuf>),
    Save,
    /// Ask whether to remove this project.
    Remove(u64),
    ConfirmRemove(u64),
    CancelRemove,
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
    match action {
        Action::NewName(name) => {
            state.projects.new_name = name;
            state.projects.add_error = None;
            Task::none()
        }

        Action::Add => {
            match state
                .config
                .projects
                .clean_name(&state.projects.new_name, None)
            {
                Ok(name) => {
                    state.config.projects.add(name);
                    state.projects.new_name.clear();
                    state.projects.add_error = None;
                    queue_save(state)
                }
                Err(error) => {
                    state.projects.add_error = Some(error);
                    Task::none()
                }
            }
        }

        Action::Edit(id) => {
            let Some(project) = state.config.projects.get(id) else {
                return Task::none();
            };
            state.projects.editing = Some(Draft {
                id,
                name: project.name.clone(),
                color: project.color(),
                directory: project
                    .directory
                    .as_ref()
                    .map(|directory| directory.display().to_string())
                    .unwrap_or_default(),
                error: None,
            });
            state.projects.confirm_remove = None;
            Task::none()
        }

        Action::CancelEdit => {
            state.projects.editing = None;
            Task::none()
        }

        Action::Name(name) => {
            edit(state, |draft| draft.name = name);
            Task::none()
        }

        Action::Color(color) => {
            edit(state, |draft| draft.color = color);
            Task::none()
        }

        Action::Directory(directory) => {
            edit(state, |draft| draft.directory = directory);
            Task::none()
        }

        Action::Browse => {
            let start = state
                .projects
                .editing
                .as_ref()
                .map(|draft| expand_home(&draft.directory))
                .filter(|path| path.is_dir());

            blocking::run(move || {
                let mut dialog = rfd::FileDialog::new().set_title("Choose the project's folder");
                if let Some(start) = start {
                    dialog = dialog.set_directory(start);
                }
                dialog.pick_folder()
            })
            .map(|folder| Message::Projects(Action::Browsed(folder)))
        }

        Action::Browsed(Some(folder)) => {
            edit(state, |draft| {
                draft.directory = folder.display().to_string();
                draft.error = None;
            });
            Task::none()
        }

        Action::Browsed(None) => Task::none(),

        Action::Save => save(state),

        Action::Remove(id) => {
            state.projects.confirm_remove = Some(id);
            Task::none()
        }

        Action::ConfirmRemove(id) => {
            state.projects.confirm_remove = None;
            if state
                .projects
                .editing
                .as_ref()
                .is_some_and(|draft| draft.id == id)
            {
                state.projects.editing = None;
            }
            state.config.projects.remove(id);
            queue_save(state)
        }

        Action::CancelRemove => {
            state.projects.confirm_remove = None;
            Task::none()
        }
    }
}

fn edit(state: &mut State, change: impl FnOnce(&mut Draft)) {
    if let Some(draft) = &mut state.projects.editing {
        change(draft);
        draft.error = None;
    }
}

fn save(state: &mut State) -> Task<Message> {
    let Some(draft) = state.projects.editing.clone() else {
        return Task::none();
    };
    let Some(current) = state.config.projects.get(draft.id) else {
        state.projects.editing = None;
        return Task::none();
    };

    let name = match state
        .config
        .projects
        .clean_name(&draft.name, Some(draft.id))
    {
        Ok(name) => name,
        Err(error) => return fail(state, error),
    };

    let directory = draft.directory.trim();
    let directory = (!directory.is_empty()).then(|| expand_home(directory));

    // Only a folder that is being chosen has to be there. A project whose
    // folder is on a drive that is not plugged in can still be renamed.
    if directory != current.directory
        && let Some(directory) = &directory
        && !directory.is_dir()
    {
        return fail(
            state,
            format!("{} is not a folder that exists", directory.display()),
        );
    }

    if let Some(project) = state
        .config
        .projects
        .items
        .iter_mut()
        .find(|project| project.id == draft.id)
    {
        project.name = name;
        project.color = draft.color.map(Into::into);
        project.directory = directory;
    }
    state.config.projects.sort();
    state.projects.editing = None;

    queue_save(state)
}

fn fail(state: &mut State, error: String) -> Task<Message> {
    if let Some(draft) = &mut state.projects.editing {
        draft.error = Some(error);
    }
    Task::none()
}

/// `~` and `~/…` mean the home directory, which is how folders are usually typed.
fn expand_home(text: &str) -> PathBuf {
    let text = text.trim();
    let home = || directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf());

    if text == "~" {
        return home().unwrap_or_else(|| PathBuf::from(text));
    }

    match text.strip_prefix("~/").zip(home()) {
        Some((rest, home)) => home.join(rest),
        None => PathBuf::from(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::theme::Catalog;

    fn state() -> State {
        State::new(Config::default(), Catalog::built_in())
    }

    fn names(state: &State) -> Vec<String> {
        state
            .config
            .projects
            .sorted()
            .into_iter()
            .map(|project| project.name.clone())
            .collect()
    }

    fn id_of(state: &State, name: &str) -> u64 {
        state
            .config
            .projects
            .items
            .iter()
            .find(|project| project.name == name)
            .unwrap()
            .id
    }

    #[test]
    fn the_defaults_are_listed_alphabetically() {
        assert_eq!(
            names(&state()),
            [
                "Personal",
                "Personal/Chores",
                "School/Calculus",
                "School/Homework",
                "Work"
            ]
        );
    }

    #[test]
    fn adding_a_project_tidies_the_name_and_keeps_the_order() {
        let mut state = state();
        let _ = update(&mut state, Action::NewName(" School / Physics/ ".into()));
        let _ = update(&mut state, Action::Add);

        assert!(state.projects.new_name.is_empty());
        assert!(names(&state).contains(&"School/Physics".to_owned()));
        let stored: Vec<_> = state
            .config
            .projects
            .items
            .iter()
            .map(|p| &p.name)
            .collect();
        let mut sorted = stored.clone();
        sorted.sort_by_key(|name| name.to_lowercase());
        assert_eq!(stored, sorted);
    }

    #[test]
    fn a_blank_or_repeated_name_is_refused() {
        let mut state = state();

        let _ = update(&mut state, Action::NewName("  / ".into()));
        let _ = update(&mut state, Action::Add);
        assert!(state.projects.add_error.is_some());

        let _ = update(&mut state, Action::NewName("work".into()));
        let _ = update(&mut state, Action::Add);
        assert!(state.projects.add_error.is_some());
        assert_eq!(state.config.projects.items.len(), 5);
    }

    #[test]
    fn editing_renames_recolors_and_sets_the_directory() {
        let mut state = state();
        let id = id_of(&state, "Work");
        let red = Color::from_rgb8(0xDC, 0x21, 0x27);

        let _ = update(&mut state, Action::Edit(id));
        let _ = update(&mut state, Action::Name("Zebra Work".into()));
        let _ = update(&mut state, Action::Color(Some(red)));
        let _ = update(
            &mut state,
            Action::Directory(std::env::temp_dir().display().to_string()),
        );
        let _ = update(&mut state, Action::Save);

        assert!(state.projects.editing.is_none());
        let project = state.config.projects.get(id).unwrap();
        assert_eq!(project.name, "Zebra Work");
        assert_eq!(project.color(), Some(red));
        assert_eq!(project.directory, Some(std::env::temp_dir()));
        assert_eq!(names(&state).last().unwrap(), "Zebra Work");
    }

    #[test]
    fn a_directory_that_does_not_exist_is_refused_and_keeps_the_edit_open() {
        let mut state = state();
        let id = id_of(&state, "Work");

        let _ = update(&mut state, Action::Edit(id));
        let _ = update(
            &mut state,
            Action::Directory("/definitely/not/a/real/folder".into()),
        );
        let _ = update(&mut state, Action::Save);

        let draft = state.projects.editing.as_ref().unwrap();
        assert!(draft.error.is_some());
        assert_eq!(state.config.projects.get(id).unwrap().directory, None);
    }

    #[test]
    fn removing_asks_first_and_never_reuses_the_id() {
        let mut state = state();
        let id = id_of(&state, "Personal/Chores");

        let _ = update(&mut state, Action::Remove(id));
        assert_eq!(state.config.projects.items.len(), 5);
        let _ = update(&mut state, Action::CancelRemove);
        assert_eq!(state.projects.confirm_remove, None);

        let _ = update(&mut state, Action::Remove(id));
        let _ = update(&mut state, Action::ConfirmRemove(id));
        assert!(state.config.projects.get(id).is_none());

        let _ = update(&mut state, Action::NewName("Personal/Chores".into()));
        let _ = update(&mut state, Action::Add);
        assert_ne!(id_of(&state, "Personal/Chores"), id);
    }

    #[test]
    fn projects_survive_the_config_file() {
        let mut config = Config::default();
        config.projects.items[0].color = Some(Color::from_rgb8(0x10, 0x20, 0x30).into());
        config.projects.items[0].directory = Some("/tmp/work".into());

        let text = toml::to_string_pretty(&config).unwrap();
        let read: Config = toml::from_str(&text).unwrap();
        assert_eq!(read.projects, config.projects);

        let mut empty = Config::default();
        empty.projects.items.clear();
        let text = toml::to_string_pretty(&empty).unwrap();
        let read: Config = toml::from_str(&text).unwrap();
        assert!(read.projects.items.is_empty(), "removing them all sticks");
    }
}
