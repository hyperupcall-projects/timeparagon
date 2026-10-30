//! The event details pane is a column of its own to the right of the view.
//!
//! These tests drive the real wire: opening an event widens the window by the
//! configured pane width, closing it narrows back, and a live width change
//! resizes an open pane. They mirror the time-block editor's own suite.

use iced::{Size, Task, futures::StreamExt, window};
use iced_test::runtime::{self, Action, window::Action as WindowAction};
use timeslay::{
    app::{self, LayoutField, Message, State},
    calendar,
    config::Config,
    theme::Catalog,
};

/// The pane width the default settings ship with.
const DETAIL: f32 = 360.0;

fn state() -> State {
    let mut state = State::new(Config::default(), Catalog::built_in());
    state.main_window = Some(window::Id::unique());
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
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
fn opening_an_event_widens_by_the_details_width_and_closing_restores_it() {
    let mut state = state();
    let original = state.main_size;
    let base = state.calendar_width();
    assert_eq!(base, 260.0);

    let opened = assert_resize(
        app::update(&mut state, Message::EventOpened("all-day".into())),
        Size::new(base + DETAIL, original.height),
    );
    // The pane is not an editor: nothing marks the window as oversized beyond
    // the resize itself.
    assert!(
        opened
            .iter()
            .all(|action| !matches!(action, Action::Window(WindowAction::SetMaxSize(..))))
    );
    resize(&mut state, Size::new(base + DETAIL, original.height));

    // A width the user drags out belongs to the pane, not the window, so it is
    // not saved as the normal window size.
    resize(
        &mut state,
        Size::new(base + DETAIL + 120.0, original.height),
    );
    assert_eq!(state.config.window.width, original.width);
    assert!(state.calendar.open_event.is_some());

    // Switching to another event keeps the pane and does not resize again.
    assert!(
        actions(app::update(
            &mut state,
            Message::EventOpened("lunch".into())
        ))
        .is_empty()
    );

    assert_resize(app::update(&mut state, Message::EventClosed), original);
    assert!(state.calendar.open_event.is_none());
}

#[test]
fn the_details_width_setting_resizes_an_open_pane_live() {
    let mut state = state();
    let height = state.main_size.height;
    let base = state.calendar_width();

    let _ = app::update(&mut state, Message::EventOpened("all-day".into()));
    resize(&mut state, Size::new(base + DETAIL, height));

    assert_resize(
        app::update(&mut state, Message::DetailWidthChanged(240.0)),
        Size::new(base + 240.0, height),
    );
    assert_eq!(state.config.views.detail_width(), 240.0);

    // The new default survives a round trip through the config file.
    let config: Config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
    assert_eq!(config.views.detail_width(), 240.0);
}

#[test]
fn layout_number_steps_the_details_width_through_the_message() {
    let mut state = state();
    let before = LayoutField::DetailWidth.value(&state);
    let _ = app::update(
        &mut state,
        Message::ViewLayoutInputSubmitted(LayoutField::DetailWidth),
    );
    assert_eq!(state.config.views.detail_width(), before);

    let _ = app::update(&mut state, LayoutField::DetailWidth.changed(before + 10.0));
    assert_eq!(state.config.views.detail_width(), before + 10.0);
}

#[test]
fn navigating_away_closes_the_pane_and_narrows_the_window() {
    let mut state = state();
    let original = state.main_size;

    let _ = app::update(&mut state, Message::EventOpened("all-day".into()));
    assert!(state.details_pane_open);
    resize(&mut state, Size::new(260.0 + DETAIL, original.height));

    assert_resize(
        app::update(&mut state, Message::ScreenSelected(app::Screen::Notepad)),
        original,
    );
    assert!(!state.details_pane_open);
    // The event stays open, so returning to the calendar brings the pane back.
    assert!(state.calendar.open_event.is_some());
}
