//! The More menu: a card of picture tiles below the menu bar.

use iced::widget::canvas::{self, Frame, Geometry, LineCap, LineJoin, Path, Stroke};
use iced::widget::{Space, button, column, container, mouse_area, row, stack, text};
use iced::{
	Alignment, Border, Color, Degrees, Element, Fill, Font, Length, Padding, Point, Radians,
	Rectangle, Renderer, Size, Theme, mouse,
};
use timeparagon_core::calendar::colors::on_color;
use timeparagon_core::more_menu::{Item, ROWS, Shape};
use timeparagon_core::theme as look;

use super::controls;
use super::layout::Metrics;
use crate::app::{Message, NavigationMessage, Screen, State};

const CARD_WIDTH: f32 = 320.0;
const CARD_PADDING: f32 = 6.0;
const TILE_GAP: f32 = 6.0;
/// Tiles share a short window's height, down to what a label needs.
const TILE_MIN_HEIGHT: f32 = 30.0;
const TILE_MAX_HEIGHT: f32 = 64.0;
/// From this height a half-width tile has room for its picture above its label.
const STACKED_HEIGHT: f32 = 56.0;
/// How strongly a tile's color shows through its background picture.
const BACKDROP_ALPHA: f32 = 0.2;
/// Below this window width the background picture would run into the label.
const BACKDROP_MIN_WIDTH: f32 = 240.0;

pub(super) fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill))
		.on_press(Message::Navigation(NavigationMessage::MoreMenuToggled));

	let top = super::tabs_height(state, metrics) + metrics.spacing;
	let height = tile_height(top, metrics);
	let mut panel = column![].spacing(TILE_GAP).width(Fill);
	for items in ROWS {
		let wide = items.len() == 1;
		let tiles = items
			.iter()
			.map(|&item| tile(state, item, wide, height, metrics));
		panel = panel.push(iced::widget::Row::with_children(tiles).spacing(TILE_GAP));
	}

	let width = (metrics.size.width - metrics.padding * 2.0).clamp(96.0, CARD_WIDTH);
	let card = container(controls::scrollable(panel))
		.width(Length::Fixed(width))
		.padding(CARD_PADDING)
		.style(controls::surface);

	let aligned = column![
		Space::new().height(top),
		container(row![Space::new().width(Fill), iced::widget::opaque(card)])
			.padding(Padding {
				right: metrics.padding,
				bottom: metrics.padding,
				..Default::default()
			})
			.width(Fill),
	]
	.width(Fill)
	.align_x(Alignment::Start);

	stack![dismiss, aligned].into()
}

fn tile_height(top: f32, metrics: &Metrics) -> f32 {
	let rows = ROWS.len() as f32;
	let chrome = top + metrics.padding + CARD_PADDING * 2.0 + TILE_GAP * (rows - 1.0);
	((metrics.size.height - chrome) / rows).clamp(TILE_MIN_HEIGHT, TILE_MAX_HEIGHT)
}

fn screen(item: Item) -> Option<Screen> {
	match item {
		Item::Notepad => Some(Screen::Notepad),
		Item::Habits => Some(Screen::Habits),
		Item::Notes => Some(Screen::Notes),
		Item::People => Some(Screen::People),
		Item::Travel => Some(Screen::Travel),
		Item::BrainDump => Some(Screen::BrainDump),
		Item::Routines => Some(Screen::Routines),
		Item::Alarms => Some(Screen::Alarms),
		Item::Logs => None,
	}
}

fn tile<'a>(
	state: &State,
	item: Item,
	wide: bool,
	height: f32,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let stacked = !wide && height >= STACKED_HEIGHT;
	let badge_size = if stacked {
		(height * 0.46).clamp(20.0, 30.0)
	} else {
		(height - 10.0).clamp(20.0, 36.0)
	};
	let label = text(item.label())
		.size(metrics.label())
		.font(Font {
			weight: iced::font::Weight::Semibold,
			..Font::DEFAULT
		})
		.line_height(1.1);

	let content: Element<'a, Message> = if stacked {
		column![badge(item, badge_size), label.center()]
			.spacing(3)
			.align_x(Alignment::Center)
			.into()
	} else {
		row![badge(item, badge_size), label]
			.spacing(if wide { 10 } else { 5 })
			.align_y(Alignment::Center)
			.into()
	};
	let content = container(content)
		.height(Fill)
		.padding([0, 6])
		.center_y(Fill);
	let content = if stacked {
		content.center_x(Fill)
	} else {
		content.width(Fill)
	};

	let face: Element<'a, Message> = if wide && metrics.size.width >= BACKDROP_MIN_WIDTH {
		let backdrop = container(picture(item, height - 8.0, Ink::Backdrop))
			.height(Fill)
			.width(Fill)
			.padding([0, 10])
			.align_right(Fill)
			.center_y(Fill);
		stack![backdrop, content].width(Fill).height(Fill).into()
	} else {
		content.into()
	};

	let active = screen(item).is_some_and(|screen| state.screen == screen);
	let mut tile = button(face)
		.width(Fill)
		.height(height)
		.padding(0)
		.style(move |theme, status| tile_style(theme, status, item, active));
	if let Some(screen) = screen(item) {
		tile = tile.on_press(Message::Navigation(NavigationMessage::ScreenSelected(
			screen,
		)));
	}
	tile.into()
}

fn tile_style(theme: &Theme, status: button::Status, item: Item, active: bool) -> button::Style {
	let surface = controls::card_color(theme);
	let color = item.color(theme.extended_palette().is_dark);
	let strength = match status {
		button::Status::Hovered => look::TODAY_HOVER_TINT,
		button::Status::Pressed => look::TODAY_PRESS_TINT,
		_ => look::TODAY_TINT,
	};
	button::Style {
		background: Some(look::tint(surface.color, color, strength).into()),
		text_color: surface.text,
		border: Border {
			color: if active {
				color
			} else {
				look::tint(surface.color, color, 0.4)
			},
			width: if active { 2.0 } else { 1.0 },
			radius: controls::RADIUS_ROW.into(),
		},
		..Default::default()
	}
}

/// The item's picture on a square of its color.
fn badge<'a>(item: Item, size: f32) -> Element<'a, Message> {
	container(picture(item, size * 0.7, Ink::OnBadge))
		.center(size)
		.style(move |theme: &Theme| container::Style {
			background: Some(item.color(theme.extended_palette().is_dark).into()),
			border: Border {
				radius: controls::RADIUS_CONTROL.into(),
				..Default::default()
			},
			..Default::default()
		})
		.into()
}

fn picture<'a>(item: Item, size: f32, ink: Ink) -> Element<'a, Message> {
	iced::widget::canvas(Picture { item, ink })
		.width(size)
		.height(size)
		.into()
}

#[derive(Clone, Copy)]
enum Ink {
	OnBadge,
	Backdrop,
}

struct Picture {
	item: Item,
	ink: Ink,
}

impl canvas::Program<Message> for Picture {
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
		let color = self.item.color(theme.extended_palette().is_dark);
		let color: Color = match self.ink {
			Ink::OnBadge => on_color(color),
			Ink::Backdrop => color.scale_alpha(BACKDROP_ALPHA),
		};
		let stroke = Stroke {
			style: canvas::Style::Solid(color),
			width: 2.0 * scale,
			line_cap: LineCap::Round,
			line_join: LineJoin::Round,
			..Default::default()
		};
		for shape in self.item.picture() {
			frame.stroke(&path(*shape, scale), stroke);
		}
		vec![frame.into_geometry()]
	}
}

fn path(shape: Shape, scale: f32) -> Path {
	let point = |x: f32, y: f32| Point::new(x * scale, y * scale);
	let through = |points: &[(f32, f32)], close: bool| {
		Path::new(|builder| {
			for (index, &(x, y)) in points.iter().enumerate() {
				if index == 0 {
					builder.move_to(point(x, y));
				} else {
					builder.line_to(point(x, y));
				}
			}
			if close {
				builder.close();
			}
		})
	};
	match shape {
		Shape::Lines(points) => through(points, false),
		Shape::Polygon(points) => through(points, true),
		Shape::Circle { x, y, radius } => Path::circle(point(x, y), radius * scale),
		Shape::Arc {
			x,
			y,
			radius,
			from,
			to,
		} => Path::new(|builder| {
			builder.arc(canvas::path::Arc {
				center: point(x, y),
				radius: radius * scale,
				start_angle: Radians::from(Degrees(from)),
				end_angle: Radians::from(Degrees(to)),
			});
		}),
		Shape::RoundedRect {
			x,
			y,
			width,
			height,
			radius,
		} => Path::rounded_rectangle(
			point(x, y),
			Size::new(width * scale, height * scale),
			(radius * scale).into(),
		),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_public_labels_follow_the_tiles() {
		let labels: Vec<&str> = ROWS
			.iter()
			.flat_map(|items| items.iter().map(|item| item.label()))
			.collect();
		assert_eq!(labels, super::super::MORE_LABELS);
	}
}
