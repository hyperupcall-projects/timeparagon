use iced::{Size, Task, futures::StreamExt, window};
use iced_test::runtime::{self, Action, window::Action as WindowAction};
use timetracker::{
    app::{self, LayoutField, Message, Sidebar, State},
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

#[test]
fn configured_pane_sizes_apply_live_and_survive_restart() {
    for sidebar in [false, true] {
        let mut state = state(Config::default(), sidebar);
        let height = state.main_size.height;
        let margin = if sidebar { 8.0 } else { 0.0 };
        let base = 300.0 + margin;
        assert_resize(
            app::update(&mut state, Message::CalendarWidthChanged(300.0)),
            Size::new(base, height),
        );
        resize(&mut state, Size::new(base, height));
        let _ = app::update(&mut state, Message::TimeBlockWidthChanged(240.0));
        assert_resize(
            app::update(&mut state, Message::EventOpened("standup".into())),
            Size::new(base + 240.0, height),
        );
        resize(&mut state, Size::new(base + 240.0, height));

        // Choosing a narrower default must override the remembered total.
        assert_resize(
            app::update(&mut state, Message::TimeBlockWidthChanged(200.0)),
            Size::new(base + 200.0, height),
        );
        resize(&mut state, Size::new(base + 200.0, height));
        assert_resize(
            app::update(
                &mut state,
                Message::TimeBlocking(timeblocking::Action::Close),
            ),
            Size::new(base, height),
        );
        resize(&mut state, Size::new(base, height));

        let config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
        let mut restarted = self::state(config, sidebar);
        if sidebar {
            restarted.main_size = Size::new(base, height);
        } else {
            assert_eq!(restarted.main_size, Size::new(base, height));
        }
        assert_resize(
            app::update(&mut restarted, Message::EventOpened("standup".into())),
            Size::new(base + 200.0, height),
        );
    }
}

#[test]
fn external_pane_settings_resize_an_open_editor_in_both_windows() {
    for sidebar in [false, true] {
        let mut state = state(Config::default(), sidebar);
        let height = state.main_size.height;
        let _ = app::update(&mut state, Message::EventOpened("standup".into()));
        resize(&mut state, Size::new(1000.0, height));
        let mut config = state.config.clone();
        config.views.calendar_width = Some(280.0);
        config.views.time_block_width = 220.0;
        config.views.margin = 12.0;
        let base = 280.0 + if sidebar { 12.0 } else { 0.0 };
        assert_resize(
            app::update(&mut state, Message::ConfigReloaded(config)),
            Size::new(base + 220.0, height),
        );
        assert_eq!(state.timeblocking.original_width, Some(base));
    }
}

#[test]
fn side_panel_margin_preserves_pane_widths_and_leaves_three_clear_edges() {
    use iced_test::{Simulator, core::Settings};

    let mut state = state(Config::default(), true);
    state.main_size = Size::new(208.0, 600.0);
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    resize(&mut state, Size::new(608.0, 600.0));
    assert_resize(
        app::update(&mut state, Message::ScreenMarginChanged(20.0)),
        Size::new(620.0, 600.0),
    );
    assert_eq!(state.timeblocking.original_width, Some(220.0));
    assert_eq!(state.config.window.time_block_width, Some(620.0));

    for (side, x) in [(panel::Side::Left, 20.0), (panel::Side::Right, 0.0)] {
        state.sidebar.as_mut().unwrap().side = side;
        let mut ui = Simulator::with_size(
            Settings::default(),
            Size::new(620.0, 600.0),
            timetracker::ui::view(&state),
        );
        let body = ui
            .find(iced::widget::Id::new("sidebar-body"))
            .unwrap()
            .bounds();
        assert_eq!(
            body,
            iced::Rectangle::new(iced::Point::new(x, 20.0), Size::new(600.0, 560.0))
        );
    }
    resize(&mut state, Size::new(620.0, 600.0));
    let mut config = state.config.clone();
    config.views.margin = 30.0;
    assert_resize(
        app::update(&mut state, Message::ConfigReloaded(config)),
        Size::new(630.0, 600.0),
    );
    assert_resize(
        app::update(
            &mut state,
            Message::TimeBlocking(timeblocking::Action::Close),
        ),
        Size::new(230.0, 600.0),
    );
}

#[test]
fn layout_numbers_can_be_cleared_retyped_and_submitted_with_bounds() {
    let mut state = state(Config::default(), false);
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
