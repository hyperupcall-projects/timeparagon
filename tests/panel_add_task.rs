use iced::{Size, widget::Id};
use iced_test::{Simulator, core::Settings};
use time_slay::{
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
    state.main_size = Size::new(width + state.screen_margin(), 700.0);
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
                time_slay::ui::view(&state),
            );
            let original = ui.find(Id::new("panel-add-task")).unwrap().bounds();
            let body = ui.find(Id::new("sidebar-body")).unwrap().bounds();
            assert!((original.y + original.height - (body.y + body.height)).abs() <= 2.0);
            drop(ui);

            // Task details, the block editor, then details beside the editor.
            for event in ["all-day", "standup", "lunch"] {
                let _ = app::update(&mut state, Message::EventOpened(event.into()));
                let base = state
                    .timeblocking
                    .original_width
                    .unwrap_or(width + state.screen_margin());
                let editor = if state.timeblocking.block.is_some() {
                    400.0
                } else {
                    0.0
                };
                let detail = if state.calendar.open_event.is_some() {
                    state.config.views.detail_width()
                } else {
                    0.0
                };
                let mut ui = Simulator::with_size(
                    Settings::default(),
                    Size::new(base + editor + detail, 700.0),
                    time_slay::ui::view(&state),
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
        time_slay::ui::view(&state),
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
        time_slay::ui::view(&state),
    );
    for label in [
        "Task, or paste a link",
        "Deadline (optional)",
        "Due date (optional)",
        "Add task",
        "Cancel",
    ] {
        assert!(ui.find(label).is_ok(), "missing {label}");
    }
}
