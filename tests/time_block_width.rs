//! All task/event/habit/block editors share one independent window lifecycle.
use iced::{Size, Task, futures::StreamExt, window};
use iced_test::runtime::{self, Action, window::Action as WindowAction};
use timeslay::{
    app::{self, LayoutField, Message, Screen, Sidebar, State},
    calendar,
    config::Config,
    panel,
    theme::Catalog,
    timeblocking,
};

fn state(sidebar: bool, screen: Screen) -> State {
    let mut state = State::new(Config::default(), Catalog::built_in());
    state.main_window = Some(window::Id::unique());
    state.screen = screen;
    if sidebar {
        state.sidebar = Some(Sidebar {
            width: 200.0,
            side: panel::Side::Left,
        });
        state.main_size = Size::new(216.0, 700.0);
    }
    let mut snapshot = calendar::sample_snapshot();
    let mut task = snapshot.events[1].clone();
    task.id = "task:example".into();
    task.calendar_id = "tasks:list".into();
    snapshot.events.push(task);
    state.calendar = calendar::Store::with_snapshot(snapshot);
    state.config.calendar.app_calendar =
        Some(state.calendar.event("standup").unwrap().calendar_id.clone());
    state
}

#[derive(Debug)]
enum Geometry {
    Open(window::Id, window::Settings),
    Resize(window::Id, Size),
    Close(window::Id),
}

fn actions(task: Task<Message>) -> Vec<Geometry> {
    iced::futures::executor::block_on(async {
        let mut result = Vec::new();
        if let Some(mut stream) = runtime::task::into_stream(task) {
            while let Some(action) = stream.next().await {
                match action {
                    Action::Window(WindowAction::Open(id, settings, sender)) => {
                        result.push(Geometry::Open(id, settings));
                        let _ = sender.send(id);
                    }
                    Action::Window(WindowAction::Resize(id, size)) => {
                        result.push(Geometry::Resize(id, size))
                    }
                    Action::Window(WindowAction::Close(id)) => result.push(Geometry::Close(id)),
                    _ => {}
                }
            }
        }
        result
    })
}

#[test]
fn every_details_destination_uses_an_independent_editor() {
    for sidebar in [false, true] {
        for screen in [
            Screen::Calendar,
            Screen::Inbox,
            Screen::Overdue,
            Screen::Triaged,
            Screen::Backlog,
            Screen::Habits,
        ] {
            for event in ["lunch", "task:example", "standup"] {
                let mut state = state(sidebar, screen);
                let original = state.main_size;
                let main = state.main_window.unwrap();
                let result = actions(app::update(&mut state, Message::EventOpened(event.into())));
                let editor = state.editor_window.unwrap();
                let width = if event == "standup" {
                    state.config.views.time_block_width()
                } else {
                    state.config.views.detail_width()
                };
                assert!(matches!(result.as_slice(), [Geometry::Open(id, settings)]
                    if *id == editor && settings.size.width == width && !settings.decorations));
                assert_ne!(main, editor);
                assert_eq!(state.main_size, original);
                assert_eq!(state.main_window, Some(main));
                // Switching among same-kind items reuses the editor surface.
                assert!(
                    actions(app::update(&mut state, Message::EventOpened(event.into()))).is_empty()
                );
                assert_eq!(state.editor_window, Some(editor));
            }
        }
    }
}

#[test]
fn closing_a_block_preserves_details_and_resizes_only_the_editor() {
    for sidebar in [false, true] {
        let mut state = state(sidebar, Screen::Calendar);
        let _ = actions(app::update(
            &mut state,
            Message::EventOpened("standup".into()),
        ));
        let editor = state.editor_window.unwrap();
        let _ = actions(app::update(
            &mut state,
            Message::TimeBlocking(timeblocking::Action::Details("lunch".into())),
        ));
        let result = actions(app::update(
            &mut state,
            Message::TimeBlocking(timeblocking::Action::Close),
        ));
        assert!(
            matches!(result.as_slice(), [Geometry::Resize(id, size)] if *id == editor && size.width == state.config.views.detail_width())
        );
        assert!(state.details_pane_open);
        assert!(state.timeblocking.block.is_none());
        let result = actions(app::update(&mut state, Message::EventClosed));
        assert!(matches!(result.as_slice(), [Geometry::Close(id)] if *id == editor));
        assert!(state.editor_window.is_none());
    }
}

#[test]
fn deleting_an_open_block_closes_its_editor_only_on_success() {
    for sidebar in [false, true] {
        let mut state = state(sidebar, Screen::Calendar);
        let _ = actions(app::update(
            &mut state,
            Message::EventOpened("standup".into()),
        ));
        let editor = state.editor_window.unwrap();
        assert!(
            actions(app::update(
                &mut state,
                Message::EventDeleted("standup".into(), Err("Offline".into()))
            ))
            .is_empty()
        );
        assert_eq!(state.editor_window, Some(editor));
        let result = actions(app::update(
            &mut state,
            Message::EventDeleted("standup".into(), Ok(())),
        ));
        assert!(matches!(result.as_slice(), [Geometry::Close(id)] if *id == editor));
        assert!(state.calendar.event("standup").is_none());
        assert!(state.timeblocking.block.is_none());
        assert!(state.editor_window.is_none());
    }
}

#[test]
fn configured_editor_widths_apply_live_and_survive_restart() {
    for sidebar in [false, true] {
        let mut state = state(sidebar, Screen::Inbox);
        let _ = actions(app::update(
            &mut state,
            Message::EventOpened("standup".into()),
        ));
        let editor = state.editor_window.unwrap();
        let original = state.main_size;
        let result = actions(app::update(
            &mut state,
            Message::TimeBlockWidthChanged(240.0),
        ));
        assert!(
            matches!(result.as_slice(), [Geometry::Resize(id, size)] if *id == editor && size.width == 240.0)
        );
        let mut config = state.config.clone();
        config.views.time_block_width = 220.0;
        let result = actions(app::update(&mut state, Message::ConfigReloaded(config)));
        assert!(
            matches!(result.as_slice(), [Geometry::Resize(id, size)] if *id == editor && size.width == 220.0)
        );
        assert_eq!(state.main_size, original);
        let config: Config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
        assert_eq!(config.views.time_block_width(), 220.0);
    }
}

#[test]
fn sidebar_margin_and_width_changes_do_not_add_editor_width_to_the_panel() {
    for side in [panel::Side::Left, panel::Side::Right] {
        let mut state = state(true, Screen::Inbox);
        state.sidebar.as_mut().unwrap().side = side;
        let _ = actions(app::update(
            &mut state,
            Message::EventOpened("task:example".into()),
        ));
        let main = state.main_window.unwrap();
        let result = actions(app::update(&mut state, Message::ScreenMarginChanged(20.0)));
        assert!(result.iter().any(
            |a| matches!(a, Geometry::Resize(id, size) if *id == main && size.width == 240.0)
        ));
        let result = actions(app::update(
            &mut state,
            Message::CalendarWidthChanged(300.0),
        ));
        assert!(result.iter().any(
            |a| matches!(a, Geometry::Resize(id, size) if *id == main && size.width == 340.0)
        ));
    }
}

#[test]
fn switching_tabs_closes_and_reopens_the_editor_surface() {
    for sidebar in [false, true] {
        let mut state = state(sidebar, Screen::Calendar);
        let _ = actions(app::update(
            &mut state,
            Message::EventOpened("lunch".into()),
        ));
        let editor = state.editor_window.unwrap();
        let result = actions(app::update(
            &mut state,
            Message::ScreenSelected(Screen::Settings),
        ));
        assert!(
            result
                .iter()
                .any(|a| matches!(a, Geometry::Close(id) if *id == editor))
        );
        assert!(state.editor_window.is_none());
        let result = actions(app::update(
            &mut state,
            Message::ScreenSelected(Screen::Calendar),
        ));
        assert!(result.iter().any(|a| matches!(a, Geometry::Open(..))));
        assert_ne!(state.editor_window, Some(editor));
    }
}

#[test]
fn hiding_and_reopening_a_panel_preserves_only_its_original_reservation() {
    use std::sync::mpsc;
    use timeslay::app::PanelCommand;
    let mut state = state(true, Screen::Inbox);
    state.panel_config = Some(panel::PanelConfig::default());
    let (tx, rx) = mpsc::channel();
    state.panel_command_rx = Some(rx);
    let main = state.main_window.unwrap();
    let _ = actions(app::update(
        &mut state,
        Message::EventOpened("task:example".into()),
    ));
    let editor = state.editor_window.unwrap();
    tx.send(PanelCommand::Hide).unwrap();
    let result = actions(app::update(&mut state, Message::PanelCommandPolled));
    for target in [main, editor] {
        assert!(
            result
                .iter()
                .any(|a| matches!(a, Geometry::Close(id) if *id == target))
        );
    }
    let _ = actions(app::update(&mut state, Message::WindowClosed(main)));
    assert!(state.editor_window.is_none());
    tx.send(PanelCommand::Show).unwrap();
    let result = actions(app::update(&mut state, Message::PanelCommandPolled));
    assert!(
        matches!(result.as_slice(), [Geometry::Open(_, settings)] if settings.size.width == 216.0)
    );
}

#[test]
fn layout_numbers_can_be_cleared_retyped_and_submitted_with_bounds() {
    let mut state = state(false, Screen::Calendar);
    for input in ["", "2", "28", "280"] {
        let _ = app::update(
            &mut state,
            Message::ViewLayoutInputChanged(LayoutField::CalendarWidth, input.into()),
        );
    }
    assert_eq!(state.config.views.calendar_width, Some(280.0));
    let _ = app::update(
        &mut state,
        Message::ViewLayoutInputChanged(LayoutField::ScreenMargin, "31".into()),
    );
    assert_eq!(state.config.views.margin(), 8.0);
    let _ = app::update(
        &mut state,
        Message::ViewLayoutInputSubmitted(LayoutField::ScreenMargin),
    );
    assert_eq!(state.config.views.margin(), 30.0);
    let _ = app::update(&mut state, Message::ScreenMarginChanged(-1.0));
    assert_eq!(state.config.views.margin(), 0.0);
}
