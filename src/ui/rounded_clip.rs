//! Rounded clipping for window content. Iced containers only clip a rectangle.
use iced::advanced::{
	Clipboard, Layout, Shell, Widget, layout, overlay, renderer,
	widget::{Operation, Tree},
};
use iced::{Element, Length, Rectangle, Renderer, Size, Theme, Vector, mouse};

pub fn rounded_clip<'a, Message: 'a>(
	content: impl Into<Element<'a, Message>>,
	radius: f32,
) -> Element<'a, Message> {
	Element::new(RoundedClip {
		content: content.into(),
		radius,
	})
}

struct RoundedClip<'a, Message> {
	content: Element<'a, Message>,
	radius: f32,
}

impl<Message> Widget<Message, Theme, Renderer> for RoundedClip<'_, Message> {
	fn children(&self) -> Vec<Tree> {
		vec![Tree::new(&self.content)]
	}
	fn diff(&self, tree: &mut Tree) {
		tree.diff_children(&[&self.content]);
	}
	fn size(&self) -> Size<Length> {
		self.content.as_widget().size()
	}
	fn layout(
		&mut self,
		tree: &mut Tree,
		renderer: &Renderer,
		limits: &layout::Limits,
	) -> layout::Node {
		self.content
			.as_widget_mut()
			.layout(&mut tree.children[0], renderer, limits)
	}
	fn operate(
		&mut self,
		tree: &mut Tree,
		layout: Layout<'_>,
		renderer: &Renderer,
		operation: &mut dyn Operation,
	) {
		self.content
			.as_widget_mut()
			.operate(&mut tree.children[0], layout, renderer, operation);
	}
	fn update(
		&mut self,
		tree: &mut Tree,
		event: &iced::Event,
		layout: Layout<'_>,
		cursor: mouse::Cursor,
		renderer: &Renderer,
		clipboard: &mut dyn Clipboard,
		shell: &mut Shell<'_, Message>,
		viewport: &Rectangle,
	) {
		self.content.as_widget_mut().update(
			&mut tree.children[0],
			event,
			layout,
			cursor,
			renderer,
			clipboard,
			shell,
			viewport,
		);
	}
	fn draw(
		&self,
		tree: &Tree,
		renderer: &mut Renderer,
		theme: &Theme,
		style: &renderer::Style,
		layout: Layout<'_>,
		cursor: mouse::Cursor,
		viewport: &Rectangle,
	) {
		use iced::advanced::Renderer as _;
		let bounds = layout.bounds();
		// The renderer exposes rectangular scissor layers. Only the corner
		// bands need separate draws; the entire middle is drawn in one pass.
		let radius = self
			.radius
			.min(bounds.width / 2.0)
			.min(bounds.height / 2.0)
			.max(0.0);
		let mut draw = |clip: Rectangle| {
			if let Some(clip) = clip.intersection(viewport) {
				renderer.with_layer(clip, |renderer| {
					self.content.as_widget().draw(
						&tree.children[0],
						renderer,
						theme,
						style,
						layout,
						cursor,
						&clip,
					);
				});
			}
		};
		draw(Rectangle {
			y: bounds.y + radius,
			height: bounds.height - 2.0 * radius,
			..bounds
		});
		let mut offset = 0.0;
		while offset < radius {
			let height = (radius - offset).min(1.0);
			let dy = radius - offset;
			let inset = radius - (radius * radius - dy * dy).max(0.0).sqrt();
			for y in [
				bounds.y + offset,
				bounds.y + bounds.height - offset - height,
			] {
				draw(Rectangle {
					x: bounds.x + inset,
					y,
					width: bounds.width - 2.0 * inset,
					height,
				});
			}
			offset += height;
		}
	}
	fn mouse_interaction(
		&self,
		tree: &Tree,
		layout: Layout<'_>,
		cursor: mouse::Cursor,
		viewport: &Rectangle,
		renderer: &Renderer,
	) -> mouse::Interaction {
		self.content.as_widget().mouse_interaction(
			&tree.children[0],
			layout,
			cursor,
			viewport,
			renderer,
		)
	}
	fn overlay<'b>(
		&'b mut self,
		tree: &'b mut Tree,
		layout: Layout<'b>,
		renderer: &Renderer,
		viewport: &Rectangle,
		translation: Vector,
	) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
		self.content.as_widget_mut().overlay(
			&mut tree.children[0],
			layout,
			renderer,
			viewport,
			translation,
		)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use iced::{
		Color, Fill,
		widget::{Space, container},
	};
	use iced_test::{Simulator, core::Settings};

	#[test]
	fn square_children_cannot_paint_into_rounded_corners() {
		for radius in [0.0, 8.0, 30.0] {
			let child = container(Space::new().width(Fill).height(Fill))
				.style(|_| container::Style::default().background(Color::from_rgb(1.0, 0.0, 0.0)));
			let element: Element<'_, ()> = rounded_clip(child, radius);
			let mut ui = Simulator::with_size(Settings::default(), Size::new(64.0, 64.0), element);
			let directory = tempfile::tempdir().unwrap();
			ui.snapshot(&Theme::Light)
				.unwrap()
				.matches_image(directory.path().join("clip"))
				.unwrap();
			let path = std::fs::read_dir(directory.path())
				.unwrap()
				.next()
				.unwrap()
				.unwrap()
				.path();
			let image = iced::advanced::graphics::image::load(
				&iced::advanced::image::Handle::from_path(path),
			)
			.unwrap();
			let red = [255, 0, 0, 255];
			assert_eq!(image.get_pixel(64, 64).0, red);
			for (x, y) in [(0, 0), (127, 0), (0, 127), (127, 127)] {
				if radius > 0.0 {
					assert_ne!(image.get_pixel(x, y).0, red);
				} else {
					assert_eq!(image.get_pixel(x, y).0, red);
				}
			}
		}
	}
}
