//! Dragging the panel and overlay edges changes the widths they are drawn at.

use iced::{Size, Task, futures::StreamExt, window};
use iced_test::runtime::{self, Action, window::Action as WindowAction};
use timeparagon::app::{
	self, EventMessage, Message, Pane, ResizeEdge, SettingsMessage, Sidebar, State,
};
use timeparagon::{calendar, panel::Side};

const SCREEN: f32 = 1800.0;

fn state() -> State {
	let mut state = State::blank();
	state.main_window = Some(window::Id::unique());
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	state.sidebar = Some(Sidebar {
		width: 300.0,
		side: Side::Left,
	});
	state.config.views.calendar_width = Some(300.0);
	state.monitor_size = Some(Size::new(SCREEN, 1000.0));
	state
}

/// Runs a task, answering window opens, and returns the panel resizes it asked for.
fn run(state: &State, task: Task<Message>) -> Vec<f32> {
	let main = state.main_window;
	iced::futures::executor::block_on(async {
		let mut sizes = Vec::new();
		if let Some(mut stream) = runtime::task::into_stream(task) {
			while let Some(action) = stream.next().await {
				match action {
					Action::Window(WindowAction::Open(id, _, sender)) => {
						let _ = sender.send(id);
					}
					Action::Window(WindowAction::Resize(id, size)) if Some(id) == main => {
						sizes.push(size.width)
					}
					_ => {}
				}
			}
		}
		sizes
	})
}

fn send(state: &mut State, message: SettingsMessage) -> Vec<f32> {
	let task = app::update(state, Message::Settings(message));
	run(state, task)
}

fn open_details(state: &mut State) {
	let task = app::update(
		state,
		Message::Event(EventMessage::EventOpened("all-day".into())),
	);
	run(state, task);
	assert_eq!(state.overlay_panes(), [Pane::Detail]);
}

#[test]
fn the_panel_edge_alone_stops_at_a_third_of_the_screen() {
	let mut state = state();
	assert_eq!(state.panel_edge(), ResizeEdge::Panel);
	send(
		&mut state,
		SettingsMessage::PaneResizeStarted(ResizeEdge::Panel, 308.0),
	);
	let margins = state.screen_margin_width();
	assert_eq!(
		send(&mut state, SettingsMessage::PaneResizeMoved(358.0)),
		[350.0 + margins]
	);
	assert_eq!(state.calendar_width(), 350.0);
	send(&mut state, SettingsMessage::PaneResizeMoved(5000.0));
	assert_eq!(state.calendar_width(), SCREEN / 3.0);
	send(&mut state, SettingsMessage::PaneResizeEnded);
	assert_eq!(state.config.views.calendar_width, Some(SCREEN / 3.0));
}

#[test]
fn the_boundary_moves_both_the_panel_and_the_overlay() {
	let mut state = state();
	open_details(&mut state);
	assert_eq!(state.panel_edge(), ResizeEdge::Boundary);
	let detail = state.config.views.detail_width();
	send(
		&mut state,
		SettingsMessage::PaneResizeStarted(ResizeEdge::Boundary, 308.0),
	);
	send(&mut state, SettingsMessage::PaneResizeMoved(358.0));
	assert_eq!(state.calendar_width(), 350.0);
	assert_eq!(state.config.views.detail_width(), detail - 50.0);
	assert_eq!(state.overlay_width(), detail - 50.0);
}

#[test]
fn the_overlay_edge_stops_at_the_screen_edge() {
	let mut state = state();
	open_details(&mut state);
	send(
		&mut state,
		SettingsMessage::PaneResizeStarted(ResizeEdge::Overlay, 360.0),
	);
	assert!(send(&mut state, SettingsMessage::PaneResizeMoved(400.0)).is_empty());
	assert_eq!(state.config.views.detail_width(), 400.0);
	send(&mut state, SettingsMessage::PaneResizeMoved(5000.0));
	let margin = state.screen_margin();
	assert_eq!(
		state.config.views.detail_width(),
		SCREEN - 300.0 - 3.0 * margin
	);
	assert_eq!(state.calendar_width(), 300.0);
}
