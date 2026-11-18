//! Navbar line icons, drawn independently of the installed fonts.

use iced::widget::canvas::{self, Frame, Geometry, LineCap, LineJoin, Path, Stroke};
use iced::{Element, Point, Rectangle, Renderer, Theme, mouse};

use crate::app::Message;

#[derive(Clone, Copy)]
pub(super) enum Icon {
	Menu,
	Settings,
	Info,
}

pub(super) fn view<'a>(icon: Icon, size: f32, active: bool) -> Element<'a, Message> {
	iced::widget::canvas(NavIcon { icon, active })
		.width(size)
		.height(size)
		.into()
}

struct NavIcon {
	icon: Icon,
	active: bool,
}

impl canvas::Program<Message> for NavIcon {
	type State = ();

	fn draw(
		&self,
		_state: &(),
		renderer: &Renderer,
		theme: &Theme,
		bounds: Rectangle,
		_cursor: mouse::Cursor,
	) -> Vec<Geometry> {
		let mut frame = Frame::new(renderer, bounds.size());
		let scale = bounds.width.min(bounds.height) / 24.0;
		let point = |x, y| Point::new(x * scale, y * scale);
		let palette = theme.extended_palette();
		let color = if self.active {
			palette.primary.weak.text
		} else {
			palette.background.base.text
		};
		let stroke = Stroke {
			style: canvas::Style::Solid(color),
			width: 1.8 * scale,
			line_cap: LineCap::Round,
			line_join: LineJoin::Round,
			..Default::default()
		};
		match self.icon {
			Icon::Menu => {
				for y in [6.0, 12.0, 18.0] {
					frame.stroke(&Path::line(point(4.0, y), point(20.0, y)), stroke);
				}
			}
			Icon::Settings => {
				let outline = Path::new(|builder| {
					for i in 0..32 {
						let angle = (i as f32 - 0.5) * std::f32::consts::TAU / 32.0;
						let radius = if i % 4 < 2 { 10.0 } else { 8.0 };
						let p = point(12.0 + radius * angle.cos(), 12.0 + radius * angle.sin());
						if i == 0 {
							builder.move_to(p);
						} else {
							builder.line_to(p);
						}
					}
					builder.close();
				});
				frame.stroke(&outline, stroke);
				frame.stroke(&Path::circle(point(12.0, 12.0), 3.0 * scale), stroke);
			}
			Icon::Info => {
				frame.stroke(&Path::circle(point(12.0, 12.0), 10.0 * scale), stroke);
				frame.stroke(&Path::line(point(12.0, 11.0), point(12.0, 17.0)), stroke);
				frame.fill(&Path::circle(point(12.0, 7.5), 1.4 * scale), color);
			}
		}
		vec![frame.into_geometry()]
	}
}
