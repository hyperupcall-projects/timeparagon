//! Window-sized layers holding a strip that can be dragged to resize a box.
//!
//! The pointer stays with the strip for the whole drag, even once it is past
//! the window's edge, because Wayland keeps a pressed pointer with the surface
//! it was pressed on. Positions are reported from the screen edge the window
//! is anchored to, which stays put while the window itself changes size.
use iced::advanced::widget::{Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, renderer};
use iced::{Element, Length, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::app::{Message, ResizeEdge, SettingsMessage};
use crate::panel::Side;

/// How wide the grabbable strip is.
pub(super) const WIDTH: f32 = 6.0;

/// Where the handle sits, as a distance from one side of the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Place {
	/// From the edge the window is anchored to.
	FromAnchor(f32),
	/// From the far edge.
	FromFar(f32),
}

/// A layer with a drag handle at `place`, leaving the rest of the window to
/// the layers below. The pointer turns into a resize arrow over it.
pub(super) fn handle<'a>(edge: ResizeEdge, side: Side, place: Place) -> Element<'a, Message> {
	Element::new(EdgeLayer { edge, side, place })
}

struct EdgeLayer {
	edge: ResizeEdge,
	side: Side,
	place: Place,
}

#[derive(Default)]
struct Grab {
	dragging: bool,
}

impl EdgeLayer {
	/// How far `point` is from the window edge on the screen-edge side.
	fn distance_from_anchor(&self, point: Point, window: Rectangle) -> f32 {
		match self.side {
			Side::Left => point.x - window.x,
			Side::Right => window.x + window.width - point.x,
		}
	}

	fn grab_bounds(&self, window: Rectangle) -> Rectangle {
		let (inset, from_far) = match self.place {
			Place::FromAnchor(inset) => (inset, false),
			Place::FromFar(inset) => (inset, true),
		};
		let x = if (self.side == Side::Left) != from_far {
			window.x + inset
		} else {
			window.x + window.width - inset - WIDTH
		};
		Rectangle {
			x,
			width: WIDTH,
			..window
		}
	}
}

impl Widget<Message, Theme, Renderer> for EdgeLayer {
	fn tag(&self) -> tree::Tag {
		tree::Tag::of::<Grab>()
	}

	fn state(&self) -> tree::State {
		tree::State::new(Grab::default())
	}

	fn size(&self) -> Size<Length> {
		Size::new(Length::Fill, Length::Fill)
	}

	fn layout(&mut self, _: &mut Tree, _: &Renderer, limits: &layout::Limits) -> layout::Node {
		layout::Node::new(limits.resolve(Length::Fill, Length::Fill, Size::ZERO))
	}

	fn update(
		&mut self,
		tree: &mut Tree,
		event: &iced::Event,
		layout: Layout<'_>,
		cursor: mouse::Cursor,
		_: &Renderer,
		_: &mut dyn Clipboard,
		shell: &mut Shell<'_, Message>,
		_: &Rectangle,
	) {
		let edge = self.edge;
		let window = layout.bounds();
		let grab = tree.state.downcast_mut::<Grab>();
		match event {
			iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
				if let Some(point) = cursor.position_over(self.grab_bounds(window)) {
					grab.dragging = true;
					shell.publish(Message::Settings(SettingsMessage::PaneResizeStarted(
						edge,
						self.distance_from_anchor(point, window),
					)));
					shell.capture_event();
					shell.request_redraw();
				}
			}
			iced::Event::Mouse(mouse::Event::CursorMoved { position }) if grab.dragging => {
				shell.publish(Message::Settings(SettingsMessage::PaneResizeMoved(
					self.distance_from_anchor(*position, window),
				)));
				shell.capture_event();
			}
			iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
				if grab.dragging =>
			{
				grab.dragging = false;
				shell.publish(Message::Settings(SettingsMessage::PaneResizeEnded));
				shell.capture_event();
			}
			_ => {}
		}
	}

	fn draw(
		&self,
		_: &Tree,
		_: &mut Renderer,
		_: &Theme,
		_: &renderer::Style,
		_: Layout<'_>,
		_: mouse::Cursor,
		_: &Rectangle,
	) {
	}

	fn mouse_interaction(
		&self,
		tree: &Tree,
		layout: Layout<'_>,
		cursor: mouse::Cursor,
		_: &Rectangle,
		_: &Renderer,
	) -> mouse::Interaction {
		if tree.state.downcast_ref::<Grab>().dragging
			|| cursor.is_over(self.grab_bounds(layout.bounds()))
		{
			mouse::Interaction::ResizingHorizontally
		} else {
			mouse::Interaction::None
		}
	}
}
