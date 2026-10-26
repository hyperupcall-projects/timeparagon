use iced::{Size, Task, futures::StreamExt, window};
use iced_test::runtime::{self, Action, window::Action as WindowAction};
use timetracker::{
    app::{self, Message, Sidebar, State},
    calendar,
    config::Config,
    panel,
    theme::Catalog,
    timeblocking,
};

fn state(config: Config, sidebar: bool) -> State {
    let mut state = State::new(config, Catalog::built_in());
    state.main_window = Some(window::Id::unique());
    if sidebar {
        state.sidebar = Some(Sidebar {
            width: 200.0,
            side: panel::Side::Left,
        });
        state.main_size.width = 200.0;
    }
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    state.config.calendar.time_block_calendar =
        Some(state.calendar.event("standup").unwrap().calendar_id.clone());
    state
}

fn actions(task: Task<Message>) -> Vec<Action<Message>> {
    runtime::task::into_stream(task)
        .map(|stream| iced::futures::executor::block_on(stream.collect()))
        .unwrap_or_default()
}

fn resize(state: &mut State, size: Size) {
    let _ = app::update(
        state,
        Message::WindowResized(state.main_window.unwrap(), size),
    );
}

fn assert_resize(task: Task<Message>, expected: Size) -> Vec<Action<Message>> {
    let actions = actions(task);
    let sizes: Vec<_> = actions
        .iter()
        .filter_map(|action| match action {
            Action::Window(WindowAction::Resize(_, size)) => Some(*size),
            _ => None,
        })
        .collect();
    assert_eq!(sizes, [expected]);
    actions
}

#[test]
fn time_block_editor_remembers_its_width_after_closing_and_restarting() {
    for sidebar in [false, true] {
        let mut state = state(Config::default(), sidebar);
        let original = state.main_size;
        let original_config = state.config.window.clone();
        let initial = Size::new(original.width + timeblocking::PANEL_WIDTH, original.height);
        let opened = assert_resize(
            app::update(&mut state, Message::EventOpened("standup".into())),
            initial,
        );
        if sidebar {
            assert!(
                opened.iter().any(|action| matches!(
                    action,
                    Action::Window(WindowAction::SetMaxSize(_, None))
                )),
                "the editor must allow widening past its default size"
            );
        }
        resize(&mut state, initial);

        let wider = Size::new(initial.width + 300.0, original.height);
        resize(&mut state, wider);
        assert_eq!(state.config.window.time_block_width, Some(wider.width));
        assert_eq!(state.config.window.width, original_config.width);
        assert_eq!(state.timeblocking.original_width, Some(original.width));

        assert_resize(
            app::update(
                &mut state,
                Message::TimeBlocking(timeblocking::Action::Close),
            ),
            original,
        );
        resize(&mut state, original);
        assert_eq!(state.config.window.time_block_width, Some(wider.width));

        assert_resize(
            app::update(&mut state, Message::EventOpened("standup".into())),
            wider,
        );
        resize(&mut state, wider);
        assert!(
            actions(app::update(
                &mut state,
                Message::EventOpened("lunch".into())
            ))
            .is_empty()
        );
        assert_eq!(state.timeblocking.original_width, Some(original.width));

        let saved = toml::to_string(&state.config).unwrap();
        let config = toml::from_str(&saved).unwrap();
        let mut restarted = self::state(config, sidebar);
        assert_resize(
            app::update(&mut restarted, Message::EventOpened("standup".into())),
            wider,
        );
    }
}

#[test]
fn reloading_window_settings_keeps_the_saved_time_block_width() {
    let mut state = state(Config::default(), false);
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    resize(&mut state, Size::new(1000.0, 600.0));
    let mut config = state.config.clone();
    config.window.height = 700.0;
    assert_resize(
        app::update(&mut state, Message::ConfigReloaded(config)),
        Size::new(1000.0, 700.0),
    );
    assert_eq!(state.timeblocking.original_width, Some(260.0));
}
