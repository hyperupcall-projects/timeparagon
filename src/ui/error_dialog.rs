//! A final window layer. The background keeps its layout and drawing, but
//! cannot receive input, focus operations, or expose menus above the error.
use iced::advanced::{Layout, Widget, layout, renderer, widget::Tree};
use iced::widget::{button, column, container, responsive, scrollable, stack, text};
use iced::{Color, Element, Fill, Length, Rectangle, Renderer, Size, Theme, mouse};

use crate::app::{Message, State};

pub(crate) fn host<'a>(state: &'a State, content: Element<'a, Message>) -> Element<'a, Message> {
	if !state.errors.is_open() {
		return content;
	}
	let background = Element::new(Inert(content));
	let dialog = responsive(move |size| {
		let details = state.errors.messages.join("\n\n");
		let card = container(
			column![
				text("Error").size(18),
				container(scrollable(
					text(details).size(14).wrapping(text::Wrapping::WordOrGlyph)
				))
				.max_height((size.height - 150.0).max(40.0)),
				button(text("Close").center().width(Fill))
					.width(Fill)
					.on_press(Message::CloseError),
			]
			.spacing(12),
		)
		.id("error-dialog")
		.padding(16)
		.width((size.width - 24.0).clamp(1.0, 560.0))
		.style(super::controls::card_surface);
		container(card)
			.center(Fill)
			.style(|_| {
				container::Style::default().background(Color::from_rgba(0.2, 0.2, 0.2, 0.65))
			})
			.into()
	});
	let modal = stack![background, iced::widget::opaque(dialog)];
	if state.task_creation_window {
		// The dock's task editor usually accepts input only over its card.
		// While an error is open the entire dimmed surface must take input.
		let bounds = |size: Size| {
			Message::System(crate::app::SystemMessage::TaskPopoverBounds(
				Rectangle::with_size(size),
			))
		};
		iced::widget::sensor(modal)
			.on_show(bounds)
			.on_resize(bounds)
			.into()
	} else {
		modal.into()
	}
}

struct Inert<'a>(Element<'a, Message>);

impl Widget<Message, Theme, Renderer> for Inert<'_> {
	fn children(&self) -> Vec<Tree> {
		vec![Tree::new(&self.0)]
	}

	fn diff(&self, tree: &mut Tree) {
		tree.diff_children(&[&self.0]);
	}

	fn size(&self) -> Size<Length> {
		self.0.as_widget().size()
	}

	fn layout(
		&mut self,
		tree: &mut Tree,
		renderer: &Renderer,
		limits: &layout::Limits,
	) -> layout::Node {
		self.0
			.as_widget_mut()
			.layout(&mut tree.children[0], renderer, limits)
	}

	fn draw(
		&self,
		tree: &Tree,
		renderer: &mut Renderer,
		theme: &Theme,
		style: &renderer::Style,
		layout: Layout<'_>,
		_cursor: mouse::Cursor,
		viewport: &Rectangle,
	) {
		self.0.as_widget().draw(
			&tree.children[0],
			renderer,
			theme,
			style,
			layout,
			mouse::Cursor::Unavailable,
			viewport,
		);
	}
}
