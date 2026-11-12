//! A thin strip over a window edge that can be dragged to resize it.
//!
//! The pointer stays with the strip for the whole drag, even once it is past
//! the window's edge, because Wayland keeps a pressed pointer with the surface
//! it was pressed on. Positions are reported from the screen edge the window
//! is anchored to, which stays put while the window itself changes size.
use iced::advanced::widget::{Tree, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, renderer};
use iced::widget::{Space, row};
use iced::{Element, Length, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::app::{Message, ResizeEdge, SettingsMessage};
use crate::panel::Side;

/// How wide the grabbable strip is.
pub(super) const WIDTH: f32 = 6.0;

/// A layer for a window's stack, holding a handle `from_anchor` pixels from
/// the window's anchored edge and leaving everything else to the layers below.
pub(super) fn layer<'a>(edge: ResizeEdge, side: Side, from_anchor: f32) -> Element<'a, Message> {
	let handle = Element::new(Handle { edge, side });
	let gap = Space::new().width(Length::Fixed(from_anchor.max(0.0)));
	let rest = Space::new().width(Length::Fill);
	match side {
		Side::Left => row![gap, handle, rest],
		Side::Right => row![rest, handle, gap],
	}
	.height(Length::Fill)
	.into()
}

/// A layer with the handle against the window's far edge.
pub(super) fn far_edge_layer<'a>(edge: ResizeEdge, side: Side) -> Element<'a, Message> {
	let handle = Element::new(Handle { edge, side });
	let rest = Space::new().width(Length::Fill);
	match side {
		Side::Left => row![rest, handle],
		Side::Right => row![handle, rest],
	}
	.height(Length::Fill)
	.into()
}

struct Handle {
	edge: ResizeEdge,
	side: Side,
}

#[derive(Default)]
struct Grab {
	dragging: bool,
}

impl Handle {
	/// How far `point` is from the window edge on the screen-edge side.
	fn distance_from_anchor(&self, point: Point, window: &Rectangle) -> f32 {
		match self.side {
			Side::Left => point.x - window.x,
			Side::Right => window.x + window.width - point.x,
		}
	}
}

impl Widget<Message, Theme, Renderer> for Handle {
	fn tag(&self) -> tree::Tag {
		tree::Tag::of::<Grab>()
	}

	fn state(&self) -> tree::State {
		tree::State::new(Grab::default())
	}

	fn size(&self) -> Size<Length> {
		Size::new(Length::Fixed(WIDTH), Length::Fill)
	}

	fn layout(&mut self, _: &mut Tree, _: &Renderer, limits: &layout::Limits) -> layout::Node {
		layout::Node::new(limits.resolve(Length::Fixed(WIDTH), Length::Fill, Size::ZERO))
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
		viewport: &Rectangle,
	) {
		let grab = tree.state.downcast_mut::<Grab>();
		match event {
			iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
				if let Some(point) = cursor.position_over(layout.bounds()) {
					grab.dragging = true;
					shell.publish(Message::Settings(SettingsMessage::PaneResizeStarted(
						self.edge,
						self.distance_from_anchor(point, viewport),
					)));
					shell.capture_event();
					shell.request_redraw();
				}
			}
			iced::Event::Mouse(mouse::Event::CursorMoved { position }) if grab.dragging => {
				shell.publish(Message::Settings(SettingsMessage::PaneResizeMoved(
					self.distance_from_anchor(*position, viewport),
				)));
				shell.capture_event();
			}
			iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
				if grab.dragging =>
			{
				grab.dragging = false;
				shell.publish(Message::Settings(SettingsMessage::PaneResizeEnded));
				shell.capture_event();
				shell.request_redraw();
			}
			iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
				// Redraw so the highlight follows the pointer in and out.
				shell.request_redraw();
			}
			_ => {}
		}
	}

	fn draw(
		&self,
		tree: &Tree,
		renderer: &mut Renderer,
		theme: &Theme,
		_: &renderer::Style,
		layout: Layout<'_>,
		cursor: mouse::Cursor,
		_: &Rectangle,
	) {
		let bounds = layout.bounds();
		if !tree.state.downcast_ref::<Grab>().dragging && !cursor.is_over(bounds) {
			return;
		}
		let line = 2.0;
		<Renderer as renderer::Renderer>::fill_quad(
			renderer,
			renderer::Quad {
				bounds: Rectangle {
					x: bounds.center_x() - line / 2.0,
					width: line,
					..bounds
				},
				..Default::default()
			},
			theme.palette().primary,
		);
	}

	fn mouse_interaction(
		&self,
		tree: &Tree,
		layout: Layout<'_>,
		cursor: mouse::Cursor,
		_: &Rectangle,
		_: &Renderer,
	) -> mouse::Interaction {
		if tree.state.downcast_ref::<Grab>().dragging || cursor.is_over(layout.bounds()) {
			mouse::Interaction::ResizingHorizontally
		} else {
			mouse::Interaction::None
		}
	}
}
