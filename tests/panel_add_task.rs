use iced::{Size, widget::Id};
use iced_test::{Simulator, core::Settings};
use timeslay::{
    app::{self, Message, Sidebar, State},
    calendar,
    config::Config,
    inbox, panel,
    theme::Catalog,
};

fn state(side: panel::Side, width: f32) -> State {
    let mut state = State::new(Config::default(), Catalog::built_in());
    state.sidebar = Some(Sidebar { width, side });
    state.main_window = Some(iced::window::Id::unique());
    state.main_size = Size::new(width + state.screen_margin_width(), 700.0);
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    state
}

#[test]
fn panel_add_task_stays_at_the_bottom_and_keeps_its_size_when_panes_open() {
    for side in [panel::Side::Left, panel::Side::Right] {
        for width in [180.0, 200.0, 260.0] {
            let mut state = state(side, width);
            let mut ui = Simulator::with_size(
                Settings::default(),
                state.main_size,
                timeslay::ui::view(&state),
            );
            let original = ui.find(Id::new("panel-add-task")).unwrap().bounds();
            let body = ui.find(Id::new("sidebar-body")).unwrap().bounds();
            assert!((original.y + original.height - (body.y + body.height)).abs() <= 2.0);
            drop(ui);

            // Task details, the block editor, then details beside the editor.
            for event in ["all-day", "standup", "lunch"] {
                let _ = app::update(&mut state, Message::EventOpened(event.into()));
                let mut ui = Simulator::with_size(
                    Settings::default(),
                    state.main_size,
                    timeslay::ui::view(&state),
                );
                assert_eq!(
                    ui.find(Id::new("panel-add-task")).unwrap().bounds(),
                    original
                );
            }
        }
    }
}

#[test]
fn panel_plus_opens_the_inbox_task_form() {
    let mut state = state(panel::Side::Left, 260.0);
    let mut ui = Simulator::with_size(
        Settings::default(),
        state.main_size,
        timeslay::ui::view(&state),
    );
    ui.click(Id::new("panel-add-task")).unwrap();
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::Inbox(inbox::Action::NewTask)))
    );
    for message in messages {
        let _ = app::update(&mut state, message);
    }
    assert!(state.inbox.draft.is_some());
    let mut ui = Simulator::with_size(
        Settings::default(),
        state.main_size,
        timeslay::ui::view(&state),
    );
    for label in [
        "Task, or paste a link",
        "Deadline",
        "Due Date",
        "Projects",
        "+ Add project",
        "Estimate",
        "Labels",
        "Priority",
        "Location",
        "Description",
        "Add task",
        "Cancel",
    ] {
        assert!(ui.find(label).is_ok(), "missing {label}");
    }
}

#[test]
fn dock_editor_window_uses_the_full_form_and_can_open_the_estimate_dialog() {
    let mut state = State::new(Config::default(), Catalog::built_in());
    state.task_creation_window = true;
    let _ = app::update(&mut state, Message::Inbox(inbox::Action::NewTask));
    for width in [240.0, 480.0] {
        let mut ui = Simulator::with_size(
            Settings::default(),
            Size::new(width, 700.0),
            timeslay::ui::view(&state),
        );
        assert!(ui.find(Id::new("application-body")).is_err());
        for label in [
            "Title",
            "Projects",
            "Estimate",
            "Due Date",
            "Deadline",
            "Labels",
            "Priority",
            "Location",
            "Description",
            "Add task",
            "Cancel",
        ] {
            assert!(ui.find(label).is_ok(), "missing {label} at {width}");
        }
    }
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(timeslay::timeblocking::Action::EditEstimate(
            inbox::DRAFT_ID.into(),
        )),
    );
    let mut ui = Simulator::with_size(
        Settings::default(),
        Size::new(480.0, 700.0),
        timeslay::ui::view(&state),
    );
    assert!(ui.find("Estimated completion time").is_ok());
    assert!(ui.find("Save").is_ok());
}

#[test]
fn dock_task_card_bottom_matches_the_dock_surface_at_short_and_tall_heights() {
    let mut state = State::new(Config::default(), Catalog::built_in());
    state.task_creation_window = true;
    let _ = app::update(&mut state, Message::Inbox(inbox::Action::NewTask));
    for height in [700.0, 1600.0] {
        let mut ui = Simulator::with_size(
            Settings::default(),
            Size::new(440.0, height),
            timeslay::ui::view(&state),
        );
        let card = ui.find(Id::new("dock-task-card")).unwrap().bounds();
        assert!((card.y + card.height - height).abs() < 1.0, "{card:?}");
        if height == 1600.0 {
            assert!(card.y > 0.0);
        }
    }
}

#[test]
fn dock_map_input_region_covers_the_surface_and_returns_to_the_task_card() {
    let mut state = State::new(Config::default(), Catalog::built_in());
    state.task_creation_window = true;
    let _ = app::update(&mut state, Message::Inbox(inbox::Action::NewTask));
    for height in [700.0, 1600.0] {
        let size = Size::new(440.0, height);
        for map_open in [false, true, false] {
            // No browser is needed: this tests the Wayland input region
            // reported by the actual popover view, before events reach CEF.
            state.travel.map.active = map_open;
            let mut ui =
                Simulator::with_size(Settings::default(), size, timeslay::ui::view(&state));
            let expected = if map_open {
                iced::Rectangle::with_size(size)
            } else {
                ui.find(Id::new("dock-task-card")).unwrap().bounds()
            };
            ui.simulate([iced::Event::Window(iced::window::Event::RedrawRequested(
                iced::time::Instant::now(),
            ))]);
            assert!(
                ui.into_messages().any(|message| matches!(
                    message, Message::TaskPopoverBounds(bounds) if bounds == expected
                )),
                "missing input bounds {expected:?} (map open: {map_open})"
            );
        }
    }
}
