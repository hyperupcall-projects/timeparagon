//! Project selection opens an editable overview beside the list.
use iced::Size;
use iced_test::{Simulator, core::Settings};
use timeslay::{
    app::{Message, State},
    config::Config,
    projects::{self, Action},
    theme::Catalog,
    ui::{self, layout::Metrics},
};

fn state() -> State {
    State::new(Config::default(), Catalog::built_in())
}

fn click(state: &mut State, width: f32, label: &str) {
    let size = Size::new(width, 900.0);
    let mut view: Simulator<'_, Message> = Simulator::with_size(
        Settings::default(),
        size,
        ui::projects::view(state, &Metrics::of(size)),
    );
    view.click(label).unwrap();
    let messages: Vec<_> = view.into_messages().collect();
    assert!(!messages.is_empty(), "{label} must emit an action");
    for message in messages {
        if let Message::Projects(action) = message {
            let _ = projects::update(state, action);
        }
    }
}

#[test]
fn selecting_a_project_opens_details_to_the_right_and_save_keeps_them_open() {
    let mut state = state();
    click(&mut state, 1000.0, "Work");
    let id = state.projects.editing.as_ref().unwrap().id;
    let size = Size::new(1000.0, 900.0);
    let mut view: Simulator<'_, Message> = Simulator::with_size(
        Settings::default(),
        size,
        ui::projects::view(&state, &Metrics::of(size)),
    );
    let project = view.find("Work").unwrap().bounds();
    let overview = view.find("Project overview").unwrap().bounds();
    assert!(overview.x > project.x + project.width);
    for label in ["Name", "Color", "Folder path", "Browse…", "Save"] {
        assert!(view.find(label).is_ok(), "missing field: {label}");
    }
    drop(view);

    let _ = projects::update(&mut state, Action::Name(" Studio / Work ".into()));
    let _ = projects::update(&mut state, Action::Color(Some(iced::Color::BLACK)));
    let _ = projects::update(
        &mut state,
        Action::Directory(std::env::temp_dir().display().to_string()),
    );
    // Re-selecting the active project preserves the draft.
    click(&mut state, 1000.0, "Work");
    assert_eq!(
        state.projects.editing.as_ref().unwrap().name,
        " Studio / Work "
    );
    click(&mut state, 1000.0, "Save");
    let project = state.config.projects.get(id).unwrap();
    assert_eq!(project.name, "Studio/Work");
    assert_eq!(project.color(), Some(iced::Color::BLACK));
    assert_eq!(project.directory, Some(std::env::temp_dir()));
    let draft = state.projects.editing.as_ref().unwrap();
    assert_eq!(draft.id, id);
    assert_eq!(draft.name, "Studio/Work");
}

#[test]
fn narrow_overview_can_cancel_changes_and_return_to_the_list() {
    for width in [200.0, 420.0] {
        let mut state = state();
        click(&mut state, width, "Work");
        let id = state.projects.editing.as_ref().unwrap().id;
        let _ = projects::update(&mut state, Action::Name("Unsaved".into()));
        click(&mut state, width, "Close");
        assert!(state.projects.editing.is_none());
        assert_eq!(state.config.projects.get(id).unwrap().name, "Work");
        click(&mut state, width, "Work");
        assert_eq!(state.projects.editing.as_ref().unwrap().name, "Work");
    }
}

#[test]
fn overview_removal_requires_confirmation_and_closes_after_removing() {
    let mut state = state();
    click(&mut state, 1000.0, "Work");
    let id = state.projects.editing.as_ref().unwrap().id;
    click(&mut state, 1000.0, "Remove project");
    assert!(state.config.projects.get(id).is_some());
    click(&mut state, 1000.0, "Keep");
    assert!(state.projects.confirm_remove.is_none());
    click(&mut state, 1000.0, "Remove project");
    click(&mut state, 1000.0, "Remove");
    assert!(state.config.projects.get(id).is_none());
    assert!(state.projects.editing.is_none());
}
