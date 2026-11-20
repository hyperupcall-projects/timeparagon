//! Navbar line icons, drawn independently of the installed fonts.

use iced::theme::palette::mix;
use iced::widget::canvas::{
	self, Fill, Frame, Geometry, LineCap, LineJoin, Path, Stroke, fill, gradient,
};
use iced::{Color, Element, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::app::Message;

#[derive(Clone, Copy)]
pub(super) enum Icon {
	Menu,
	Settings,
	Info,
	/// Life: a tree.
	Tree,
	/// Do and Plan, when the bar is too narrow to spell them out.
	Calendar,
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
		// A color for light themes and a brighter one for dark themes.
		let hue = |light: u32, night: u32| {
			let rgb = if palette.is_dark { night } else { light };
			Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
		};
		// Lit from above: lighter at the top, darker at the bottom.
		let lit = |top: Color, bottom: Color, from: f32, to: f32| Fill {
			style: canvas::Style::Gradient(
				gradient::Linear::new(point(12.0, from), point(12.0, to))
					.add_stop(0.0, top)
					.add_stop(1.0, bottom)
					.into(),
			),
			rule: fill::Rule::EvenOdd,
		};
		match self.icon {
			Icon::Menu => {
				// Three raised bars, each with a soft shadow below it.
				let shadow = Color::BLACK.scale_alpha(if palette.is_dark { 0.55 } else { 0.22 });
				let top = mix(color, Color::WHITE, 0.45);
				let bottom = mix(color, Color::BLACK, 0.2);
				let bar = |y: f32| {
					Path::rounded_rectangle(
						point(3.5, y - 1.6),
						Size::new(17.0 * scale, 3.2 * scale),
						(1.6 * scale).into(),
					)
				};
				for y in [6.0, 12.0, 18.0] {
					frame.fill(&bar(y + 1.0), shadow);
					frame.fill(&bar(y), lit(top, bottom, y - 1.6, y + 1.6));
				}
			}
			Icon::Settings => {
				// A brass gear with a hole through its hub, outlined in the
				// icon's ink so that it stands out on a lit tab too.
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
					builder.circle(point(12.0, 12.0), 3.2 * scale);
				});
				let top = hue(0xFFD77A, 0xFFDD8C);
				let bottom = hue(0xE08A1E, 0xE89A32);
				frame.fill(&outline, lit(top, bottom, 2.0, 22.0));
				frame.stroke(
					&outline,
					Stroke {
						width: 1.5 * scale,
						..stroke
					},
				);
			}
			Icon::Info => {
				frame.stroke(&Path::circle(point(12.0, 12.0), 10.0 * scale), stroke);
				frame.stroke(&Path::line(point(12.0, 11.0), point(12.0, 17.0)), stroke);
				frame.fill(&Path::circle(point(12.0, 7.5), 1.4 * scale), color);
			}
			Icon::Tree => {
				// A tall, thin tree whose branches end in round clusters of
				// leaves, in three greens so that neighbouring clusters stand
				// apart. The trunk is drawn in the icon's ink.
				let pale = hue(0x7CC242, 0xA6DA6C);
				let mid = hue(0x3FA04F, 0x6CC47A);
				let deep = hue(0x1D763A, 0x3BA05E);
				let trunk = Path::new(|builder| {
					builder.move_to(point(11.0, 23.0));
					builder.line_to(point(12.9, 23.0));
					builder.line_to(point(12.4, 9.0));
					builder.line_to(point(11.6, 9.0));
					builder.close();
				});
				frame.fill(&trunk, color);
				let branches = Path::new(|builder| {
					builder.move_to(point(11.9, 18.2));
					builder.quadratic_curve_to(point(9.0, 17.6), point(7.6, 13.6));
					builder.move_to(point(12.3, 17.2));
					builder.quadratic_curve_to(point(15.0, 16.8), point(16.6, 13.4));
				});
				frame.stroke(
					&branches,
					Stroke {
						width: 1.1 * scale,
						..stroke
					},
				);
				// Back to front: the bumpy crown, then the clusters the
				// branches end in.
				for (x, y, radius, shade) in [
					(11.6, 5.6, 3.3, pale),
					(14.8, 4.4, 3.2, pale),
					(16.0, 7.6, 3.0, pale),
					(13.0, 8.2, 3.0, pale),
					(7.4, 8.4, 2.8, mid),
					(7.2, 12.0, 2.7, pale),
					(11.0, 12.6, 3.3, deep),
					(16.6, 12.8, 3.0, mid),
				] {
					frame.fill(&Path::circle(point(x, y), radius * scale), shade);
				}
			}
			Icon::Calendar => {
				let page = Path::rounded_rectangle(
					point(3.5, 5.0),
					Size::new(17.0 * scale, 15.5 * scale),
					(2.5 * scale).into(),
				);
				frame.stroke(&page, stroke);
				frame.stroke(&Path::line(point(3.5, 10.0), point(20.5, 10.0)), stroke);
				for x in [8.0, 16.0] {
					frame.stroke(&Path::line(point(x, 3.0), point(x, 7.0)), stroke);
				}
			}
		}
		vec![frame.into_geometry()]
	}
}
