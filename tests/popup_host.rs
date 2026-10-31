//! Shared dialogs belong to the floating editor, with a main-window fallback.
use iced::{Size, window};
use iced_test::{Simulator, core::Settings};
use timeslay::{
    app::{self, Message, State},
    calendar,
    config::Config,
    inbox,
    theme::Catalog,
    ui::controls::Picker,
};

fn popup(kind: &str) -> (State, &'static str) {
    let mut state = State::new(Config::default(), Catalog::built_in());
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    let label = match kind {
        "description" => {
            state.description_popup = Some("standup".into());
            "Save description"
        }
        "task" => {
            let _ = app::update(&mut state, Message::Inbox(inbox::Action::NewTask));
            "Task, or paste a link"
        }
        "focus" => {
            state.timeblocking.focus = Some("standup".into());
            "FOCUS · TIME BLOCK"
        }
        "session" => {
            state.config.task_work.start(
                "task".into(),
                "Test task".into(),
                None,
                chrono::Utc::now(),
            );
            state.timeblocking.session_edit = Some(0);
            "WORK SESSION"
        }
        "estimate" => {
            state.timeblocking.estimate_task = Some("standup".into());
            "Estimated completion time"
        }
        "delete" => {
            state.calendar.pending_delete = Some("standup".into());
            "Delete this event?"
        }
        "picker" => {
            let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
            state.picker = Some(Picker {
                value: String::new(),
                month: day,
                focus: day,
                change: std::sync::Arc::new(|_| Message::Ignored),
                optional: true,
                with_time: true,
                time: None,
            });
            "Choose date"
        }
        _ => unreachable!(),
    };
    (state, label)
}

#[test]
fn shared_popups_render_in_exactly_one_window() {
    for kind in [
        "description",
        "task",
        "focus",
        "session",
        "estimate",
        "delete",
        "picker",
    ] {
        let (mut state, label) = popup(kind);
        let main = window::Id::unique();
        let editor = window::Id::unique();
        state.main_window = Some(main);
        state.editor_window = Some(editor);
        for (window, expected) in [(main, false), (editor, true)] {
            let mut ui = Simulator::with_size(
                Settings::default(),
                Size::new(480.0, 900.0),
                app::view(&state, window),
            );
            assert_eq!(ui.find(label).is_ok(), expected, "{kind}: {window:?}");
        }
        state.editor_window = None;
        let mut ui = Simulator::with_size(
            Settings::default(),
            Size::new(480.0, 900.0),
            app::view(&state, main),
        );
        assert!(
            ui.find(label).is_ok(),
            "{kind}: missing main-window fallback"
        );
    }
}
