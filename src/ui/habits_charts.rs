//! The charts on a habit's page, drawn the way Loop Habit Tracker draws them.
//!
//! Each chart keeps its own horizontal scroll position. Loop pages through the
//! past by swiping, so here a chart pans with a drag or a sideways scroll.

use chrono::{Datelike, Duration, NaiveDate};
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text, path};
use iced::{Color, Element, Length, Point, Radians, Rectangle, Renderer, Size, Theme, mouse};

use crate::app::Message;
use crate::calendar::colors::on_color;
use crate::calendar::month_short;
use crate::habits::{Action, Bucket, Computed, Icons, Kind, Status, Streak};
use timeparagon_core::habits::{self, palette_color};

const TEXT_SIZE: f32 = 11.0;
/// Loop's grid colors, which are not in the app's themes.
fn low_contrast(dark: bool) -> Color {
	if dark {
		Color::from_rgb8(0x42, 0x42, 0x42)
	} else {
		Color::from_rgb8(0xE0, 0xE0, 0xE0)
	}
}
fn medium_contrast() -> Color {
	Color::from_rgb8(0x9E, 0x9E, 0x9E)
}

fn is_dark(theme: &Theme) -> bool {
	theme.extended_palette().is_dark
}

fn card_background(theme: &Theme) -> Color {
	crate::ui::controls::card_color(theme).color
}

fn blend(top: Color, bottom: Color, amount: f32) -> Color {
	Color {
		r: top.r * amount + bottom.r * (1.0 - amount),
		g: top.g * amount + bottom.g * (1.0 - amount),
		b: top.b * amount + bottom.b * (1.0 - amount),
		a: 1.0,
	}
}

fn label(
	frame: &mut Frame,
	content: impl Into<String>,
	position: Point,
	color: Color,
	align: iced::alignment::Horizontal,
) {
	frame.fill_text(Text {
		content: content.into(),
		position,
		color,
		size: iced::Pixels(TEXT_SIZE),
		align_x: align.into(),
		align_y: iced::alignment::Vertical::Center,
		..Text::default()
	});
}

fn line(frame: &mut Frame, from: Point, to: Point, color: Color, width: f32) {
	frame.stroke(
		&Path::line(from, to),
		Stroke {
			style: canvas::Style::Solid(color),
			width,
			..Default::default()
		},
	);
}

/// Dragging or sideways scrolling, in pixels from the newest data.
#[derive(Debug, Default)]
pub struct Pan {
	offset: f32,
	press: Option<(f32, f32)>,
	dragged: bool,
}

/// How far a press may wander and still count as a click.
const CLICK_SLOP: f32 = 4.0;

impl Pan {
	fn columns(&self, width: f32) -> usize {
		(self.offset / width).floor().max(0.0) as usize
	}

	/// Applies a mouse event. Returns whether anything visible changed, and
	/// where a click landed.
	fn update(
		&mut self,
		event: &iced::Event,
		bounds: Rectangle,
		cursor: mouse::Cursor,
		max_offset: f32,
	) -> (bool, Option<Point>) {
		let iced::Event::Mouse(event) = event else {
			return (false, None);
		};
		match event {
			mouse::Event::ButtonPressed(mouse::Button::Left) => {
				if let Some(point) = cursor.position_in(bounds) {
					self.press = Some((point.x, self.offset));
					self.dragged = false;
				}
				(false, None)
			}
			mouse::Event::CursorMoved { .. } => {
				let (Some((origin, start)), Some(position)) = (self.press, cursor.position())
				else {
					return (false, None);
				};
				let distance = position.x - bounds.x - origin;
				if distance.abs() > CLICK_SLOP {
					self.dragged = true;
				}
				if self.dragged {
					self.offset = (start + distance).clamp(0.0, max_offset.max(0.0));
				}
				(self.dragged, None)
			}
			mouse::Event::ButtonReleased(mouse::Button::Left) => {
				let pressed = self.press.take().is_some();
				let click = (pressed && !self.dragged)
					.then(|| cursor.position_in(bounds))
					.flatten();
				(false, click)
			}
			mouse::Event::WheelScrolled { delta } if cursor.position_in(bounds).is_some() => {
				let x = match delta {
					mouse::ScrollDelta::Lines { x, .. } => x * 24.0,
					mouse::ScrollDelta::Pixels { x, .. } => *x,
				};
				if x == 0.0 {
					return (false, None);
				}
				self.offset = (self.offset - x).clamp(0.0, max_offset.max(0.0));
				(true, None)
			}
			_ => (false, None),
		}
	}
}

fn redraw_if(changed: bool) -> Option<canvas::Action<Message>> {
	changed.then(canvas::Action::request_redraw)
}

// --------------------------------------------------------------------- ring

/// Loop's score ring: a track with the score swept clockwise from the top.
#[derive(Debug, Clone, Copy)]
pub struct Ring {
	pub score: f32,
	pub color: u8,
	pub thickness: f32,
}

pub fn ring<'a>(score: f64, color: u8, size: f32) -> Element<'a, Message> {
	canvas::Canvas::new(Ring {
		score: score.clamp(0.0, 1.0) as f32,
		color,
		thickness: (size * 0.22).max(2.5),
	})
	.width(size)
	.height(size)
	.into()
}

impl canvas::Program<Message> for Ring {
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
		let dark = is_dark(theme);
		let center = frame.center();
		let radius = bounds.width.min(bounds.height) / 2.0 - self.thickness / 2.0;
		let stroke = |color| Stroke {
			style: canvas::Style::Solid(color),
			width: self.thickness,
			..Default::default()
		};
		frame.stroke(&Path::circle(center, radius), stroke(low_contrast(dark)));
		if self.score > 0.0 {
			let start = -std::f32::consts::FRAC_PI_2;
			let arc = Path::new(|builder| {
				builder.arc(path::Arc {
					center,
					radius,
					start_angle: Radians(start),
					end_angle: Radians(start + std::f32::consts::TAU * self.score),
				});
			});
			frame.stroke(&arc, stroke(palette_color(self.color, dark)));
		}
		vec![frame.into_geometry()]
	}
}

// ---------------------------------------------------------------- day marks

/// The mark in a day's button on the list.
#[derive(Debug, Clone)]
pub struct DayMark {
	pub status: Status,
	pub color: u8,
	pub goal: u32,
	pub question_marks: bool,
	pub icons: Icons,
	pub kind: Kind,
	/// How many times it was done, shown when more than once.
	pub times: u32,
	/// On a "No" habit: how many times it was resisted, shown on a clean day.
	pub resisted: u32,
	/// The number recorded for the day.
	pub value: Option<f64>,
}

impl canvas::Program<Message> for DayMark {
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
		let dark = is_dark(theme);
		let color = palette_color(self.color, dark);
		let quiet = if dark {
			Color::from_rgb8(0x75, 0x75, 0x75)
		} else {
			Color::from_rgb8(0xBD, 0xBD, 0xBD)
		};
		let center = frame.center();
		let glyph = |frame: &mut Frame, points: &[(f32, f32)], color: Color, width: f32| {
			let shape = Path::new(|builder| {
				for (index, (x, y)) in points.iter().enumerate() {
					let point = Point::new(center.x + x, center.y + y);
					if index == 0 {
						builder.move_to(point);
					} else {
						builder.line_to(point);
					}
				}
			});
			frame.stroke(
				&shape,
				Stroke {
					style: canvas::Style::Solid(color),
					width,
					line_cap: canvas::LineCap::Round,
					line_join: canvas::LineJoin::Round,
					..Default::default()
				},
			);
		};
		let cross = |frame: &mut Frame| {
			glyph(frame, &[(-4.0, -4.0), (4.0, 4.0)], quiet, 2.0);
			glyph(frame, &[(4.0, -4.0), (-4.0, 4.0)], quiet, 2.0);
		};
		let small = |frame: &mut Frame, content: String, y: f32, color: Color, size: f32| {
			frame.fill_text(Text {
				content,
				position: Point::new(center.x, center.y + y),
				color,
				size: iced::Pixels(size),
				align_x: iced::alignment::Horizontal::Center.into(),
				align_y: iced::alignment::Vertical::Center,
				..Text::default()
			});
		};
		// Doing it again is worth noting, however little it changes the score.
		if self.times > 1 && self.status == Status::Yes {
			small(&mut frame, format!("×{}", self.times), 13.0, color, 9.0);
		}
		if let Some(value) = self.value {
			small(&mut frame, habits::format_amount(value), -16.0, color, 9.0);
		}
		// As is holding back, however little it changes the score. A day that
		// slipped shows its slips beside however many times it held back.
		match self.status {
			Status::Clean if self.resisted > 0 => {
				small(&mut frame, format!("+{}", self.resisted), 13.0, color, 9.0);
			}
			Status::Slipped(slips) => {
				let slips = slips.max(self.times);
				let slip = slip_color(dark);
				if self.resisted > 0 {
					let good = Point::new(center.x - 8.0, center.y + 13.0);
					let bad = Point::new(center.x + 8.0, center.y + 13.0);
					for (content, position, tint) in [
						(format!("+{}", self.resisted), good, color),
						(format!("+{slips}"), bad, slip),
					] {
						frame.fill_text(Text {
							content,
							position,
							color: tint,
							size: iced::Pixels(9.0),
							align_x: iced::alignment::Horizontal::Center.into(),
							align_y: iced::alignment::Vertical::Center,
							..Text::default()
						});
					}
				} else {
					small(&mut frame, format!("+{slips}"), 13.0, slip, 9.0);
				}
			}
			_ => {}
		}
		if self.icons == Icons::Faces
			&& let Some((face, tint)) = self.face(color, quiet, dark)
		{
			draw_face(&mut frame, center, face, tint);
			return vec![frame.into_geometry()];
		}
		match self.status {
			Status::Clean => glyph(frame_ref(&mut frame), CHECK, color, 2.6),
			// Doing what was meant to be avoided is the one mark that stands out.
			Status::Slipped(_) => {
				let slip = slip_color(dark);
				glyph(
					frame_ref(&mut frame),
					&[(-4.5, -4.5), (4.5, 4.5)],
					slip,
					2.6,
				);
				glyph(
					frame_ref(&mut frame),
					&[(4.5, -4.5), (-4.5, 4.5)],
					slip,
					2.6,
				);
			}
			// Before a "No" habit began there is nothing to judge.
			Status::Unknown if self.kind == Kind::No && !self.question_marks => {
				glyph(
					frame_ref(&mut frame),
					&[(-3.0, 0.0), (3.0, 0.0)],
					quiet,
					2.0,
				);
			}
			Status::Yes => glyph(frame_ref(&mut frame), CHECK, color, 2.6),
			// Covered by an earlier repetition: Loop's quieter check.
			Status::Auto => glyph(frame_ref(&mut frame), CHECK, quiet, 2.2),
			Status::Partial(reps) => {
				frame.fill_text(Text {
					content: format!("{reps}/{}", self.goal),
					position: center,
					color,
					size: iced::Pixels(12.0),
					align_x: iced::alignment::Horizontal::Center.into(),
					align_y: iced::alignment::Vertical::Center,
					..Text::default()
				});
			}
			Status::Skip => {
				glyph(
					frame_ref(&mut frame),
					&[(-5.5, -4.5), (-0.5, 0.0), (-5.5, 4.5)],
					color,
					2.2,
				);
				glyph(
					frame_ref(&mut frame),
					&[(0.5, -4.5), (5.5, 0.0), (0.5, 4.5)],
					color,
					2.2,
				);
			}
			Status::Unknown if self.question_marks => {
				frame.fill_text(Text {
					content: "?".to_owned(),
					position: center,
					color: quiet,
					size: iced::Pixels(15.0),
					align_x: iced::alignment::Horizontal::Center.into(),
					align_y: iced::alignment::Vertical::Center,
					..Text::default()
				});
			}
			Status::No | Status::Unknown => cross(&mut frame),
		}
		vec![frame.into_geometry()]
	}
}

impl DayMark {
	/// The face a day shows, and its color. A partial count says something a face
	/// cannot, so it keeps what it always shows.
	fn face(&self, color: Color, quiet: Color, dark: bool) -> Option<(Face, Color)> {
		match self.status {
			Status::Yes | Status::Clean => Some((Face::Happy, color)),
			Status::Auto => Some((Face::Happy, quiet)),
			Status::Slipped(_) => Some((Face::Disappointed, slip_color(dark))),
			Status::Skip => Some((Face::Question, color)),
			Status::No => Some((Face::Disappointed, quiet)),
			Status::Unknown if self.question_marks => Some((Face::Question, quiet)),
			Status::Unknown if self.kind == Kind::No => None,
			Status::Unknown => Some((Face::Disappointed, quiet)),
			Status::Partial(_) => None,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Face {
	Happy,
	Disappointed,
	Question,
}

/// An emoji-style face, drawn so it does not depend on the fonts installed.
fn draw_face(frame: &mut Frame, center: Point, face: Face, tint: Color) {
	const RADIUS: f32 = 9.5;
	let ink = on_color(tint);
	frame.fill(&Path::circle(center, RADIUS), tint);
	if face == Face::Question {
		frame.fill_text(Text {
			content: "?".to_owned(),
			position: center,
			color: ink,
			size: iced::Pixels(14.0),
			align_x: iced::alignment::Horizontal::Center.into(),
			align_y: iced::alignment::Vertical::Center,
			..Text::default()
		});
		return;
	}
	for x in [-3.3, 3.3] {
		frame.fill(
			&Path::circle(Point::new(center.x + x, center.y - 2.4), 1.4),
			ink,
		);
	}
	// The mouth curves up for a smile and down for disappointment.
	let (ends, bend) = if face == Face::Happy {
		(1.8, 7.8)
	} else {
		(5.2, 1.0)
	};
	let mouth = Path::new(|builder| {
		builder.move_to(Point::new(center.x - 4.2, center.y + ends));
		builder.quadratic_curve_to(
			Point::new(center.x, center.y + bend),
			Point::new(center.x + 4.2, center.y + ends),
		);
	});
	frame.stroke(
		&mouth,
		Stroke {
			style: canvas::Style::Solid(ink),
			width: 1.6,
			line_cap: canvas::LineCap::Round,
			..Default::default()
		},
	);
}

fn slip_color(dark: bool) -> Color {
	palette_color(0, dark)
}

fn frame_ref(frame: &mut Frame) -> &mut Frame {
	frame
}

const CHECK: &[(f32, f32)] = &[(-5.5, 0.5), (-1.5, 4.5), (5.5, -4.0)];

// -------------------------------------------------------------- axis labels

/// Two lines under a column. Loop names the period, and adds the year whenever
/// it changes.
fn axis_label(
	date: NaiveDate,
	previous: Option<NaiveDate>,
	bucket: Bucket,
) -> (String, Option<String>) {
	let new_year = previous.is_none_or(|previous| previous.year() != date.year());
	let year = new_year.then(|| date.year().to_string());
	match bucket {
		Bucket::Day | Bucket::Week => {
			if previous.is_none_or(|previous| previous.month() != date.month()) {
				(month_short(date.month()).to_owned(), year)
			} else {
				(date.day().to_string(), None)
			}
		}
		Bucket::Month => (month_short(date.month()).to_owned(), year),
		Bucket::Quarter => (format!("Q{}", date.month0() / 3 + 1), year),
		Bucket::Year => (date.year().to_string(), None),
	}
}

fn draw_axis(
	frame: &mut Frame,
	columns: &[(f32, NaiveDate)],
	bucket: Bucket,
	baseline: f32,
	color: Color,
) {
	let mut previous = None;
	for (x, date) in columns {
		let (first, second) = axis_label(*date, previous, bucket);
		label(
			frame,
			first,
			Point::new(*x, baseline + 11.0),
			color,
			iced::alignment::Horizontal::Center,
		);
		if let Some(second) = second {
			label(
				frame,
				second,
				Point::new(*x, baseline + 24.0),
				color,
				iced::alignment::Horizontal::Center,
			);
		}
		previous = Some(*date);
	}
}

const AXIS_HEIGHT: f32 = 32.0;
const COLUMN_WIDTH: f32 = 34.0;

// -------------------------------------------------------------------- score

/// Loop's score card: a line with a dot on each period, newest on the right.
pub struct ScoreChart {
	/// Newest first.
	pub series: Vec<(NaiveDate, f64)>,
	pub bucket: Bucket,
	pub color: u8,
}

impl canvas::Program<Message> for ScoreChart {
	type State = Pan;

	fn update(
		&self,
		pan: &mut Pan,
		event: &iced::Event,
		bounds: Rectangle,
		cursor: mouse::Cursor,
	) -> Option<canvas::Action<Message>> {
		let hidden = self.series.len() as f32 - (bounds.width / COLUMN_WIDTH).floor();
		let (changed, _) = pan.update(event, bounds, cursor, hidden * COLUMN_WIDTH);
		redraw_if(changed)
	}

	fn draw(
		&self,
		pan: &Pan,
		renderer: &Renderer,
		theme: &Theme,
		bounds: Rectangle,
		_cursor: mouse::Cursor,
	) -> Vec<Geometry> {
		let mut frame = Frame::new(renderer, bounds.size());
		let dark = is_dark(theme);
		let color = palette_color(self.color, dark);
		let plot = bounds.height - AXIS_HEIGHT;
		let y = |score: f64| 4.0 + (plot - 8.0) * (1.0 - score.clamp(0.0, 1.0) as f32);

		for step in 0..=5 {
			let value = f64::from(step) / 5.0;
			let height = y(value);
			line(
				&mut frame,
				Point::new(0.0, height),
				Point::new(bounds.width, height),
				low_contrast(dark),
				1.0,
			);
			if step > 0 {
				label(
					&mut frame,
					format!("{}%", step * 20),
					Point::new(4.0, height + 9.0),
					medium_contrast(),
					iced::alignment::Horizontal::Left,
				);
			}
		}

		let count = (bounds.width / COLUMN_WIDTH).floor().max(1.0) as usize;
		let skip = pan.columns(COLUMN_WIDTH);
		let margin = (bounds.width - count as f32 * COLUMN_WIDTH) / 2.0;
		// Oldest first, so the axis can read left to right.
		let columns: Vec<(f32, NaiveDate, f64)> = (0..count)
			.rev()
			.filter_map(|index| self.series.get(index + skip).map(|point| (index, point)))
			.map(|(index, (date, score))| {
				let column = (count - 1 - index) as f32;
				(
					margin + column * COLUMN_WIDTH + COLUMN_WIDTH / 2.0,
					*date,
					*score,
				)
			})
			.collect();
		for pair in columns.windows(2) {
			line(
				&mut frame,
				Point::new(pair[0].0, y(pair[0].2)),
				Point::new(pair[1].0, y(pair[1].2)),
				color,
				2.0,
			);
		}
		for (x, _, score) in &columns {
			frame.fill(&Path::circle(Point::new(*x, y(*score)), 3.5), color);
		}
		let axis: Vec<(f32, NaiveDate)> = columns.iter().map(|(x, date, _)| (*x, *date)).collect();
		draw_axis(&mut frame, &axis, self.bucket, plot, medium_contrast());
		vec![frame.into_geometry()]
	}
}

// ------------------------------------------------------------------ history

/// Loop's history card: repetitions per period as labelled bars.
pub struct HistoryChart {
	/// Newest first.
	pub series: Vec<(NaiveDate, f64)>,
	pub bucket: Bucket,
	pub color: u8,
	/// A line to measure the bars against, for the Value chart.
	pub target: Option<f64>,
}

impl canvas::Program<Message> for HistoryChart {
	type State = Pan;

	fn update(
		&self,
		pan: &mut Pan,
		event: &iced::Event,
		bounds: Rectangle,
		cursor: mouse::Cursor,
	) -> Option<canvas::Action<Message>> {
		let hidden = self.series.len() as f32 - (bounds.width / COLUMN_WIDTH).floor();
		let (changed, _) = pan.update(event, bounds, cursor, hidden * COLUMN_WIDTH);
		redraw_if(changed)
	}

	fn draw(
		&self,
		pan: &Pan,
		renderer: &Renderer,
		theme: &Theme,
		bounds: Rectangle,
		_cursor: mouse::Cursor,
	) -> Vec<Geometry> {
		let mut frame = Frame::new(renderer, bounds.size());
		let dark = is_dark(theme);
		let color = palette_color(self.color, dark);
		const TOP: f32 = 16.0;
		let plot = bounds.height - AXIS_HEIGHT;
		let tallest = (plot - TOP).max(1.0);
		for step in 0..5 {
			let height = TOP + tallest * step as f32 / 4.0;
			line(
				&mut frame,
				Point::new(0.0, height),
				Point::new(bounds.width, height),
				low_contrast(dark),
				1.0,
			);
		}
		let count = (bounds.width / COLUMN_WIDTH).floor().max(1.0) as usize;
		let skip = pan.columns(COLUMN_WIDTH);
		let margin = (bounds.width - count as f32 * COLUMN_WIDTH) / 2.0;
		let most = self
			.series
			.iter()
			.map(|(_, value)| *value)
			.chain(self.target)
			.fold(0.0_f64, f64::max);
		let most = if most > 0.0 { most } else { 1.0 };
		let mut axis = Vec::new();
		for index in (0..count).rev() {
			let Some((date, value)) = self.series.get(index + skip) else {
				continue;
			};
			let x = margin + (count - 1 - index) as f32 * COLUMN_WIDTH + COLUMN_WIDTH / 2.0;
			axis.push((x, *date));
			if *value <= 0.0 {
				continue;
			}
			let height = (tallest * (*value / most) as f32).max(1.0);
			let top = plot - height;
			frame.fill(
				&Path::rounded_rectangle(
					Point::new(x - 7.0, top),
					Size::new(14.0, height),
					iced::border::radius(2.0).bottom(0.0),
				),
				color,
			);
			label(
				&mut frame,
				habits::format_amount(*value),
				Point::new(x, top - 8.0),
				color,
				iced::alignment::Horizontal::Center,
			);
		}
		if let Some(target) = self.target {
			let y = plot - tallest * (target / most) as f32;
			frame.stroke(
				&Path::line(Point::new(0.0, y), Point::new(bounds.width, y)),
				Stroke {
					style: canvas::Style::Solid(medium_contrast()),
					width: 1.5,
					line_dash: canvas::LineDash {
						segments: &[5.0, 4.0],
						offset: 0,
					},
					..Default::default()
				},
			);
			// On a patch of the card, so that it stays readable over a bar.
			let words = format!("Target {}", habits::format_amount(target));
			frame.fill_rectangle(
				Point::new(0.0, y - 15.0),
				Size::new(words.chars().count() as f32 * 6.5 + 4.0, 14.0),
				card_background(theme),
			);
			label(
				&mut frame,
				words,
				Point::new(2.0, y - 8.0),
				medium_contrast(),
				iced::alignment::Horizontal::Left,
			);
		}
		draw_axis(&mut frame, &axis, self.bucket, plot, medium_contrast());
		vec![frame.into_geometry()]
	}
}

// ----------------------------------------------------------------- calendar

/// Loop's calendar card: one square per day, in a column per week, ending at
/// today. Clicking a square opens that day's dialog, as on the list.
pub struct CalendarChart {
	pub habit_id: String,
	pub statuses: Computed,
	pub color: u8,
	pub today: NaiveDate,
	pub first_weekday: chrono::Weekday,
	pub weekday_names: [&'static str; 7],
}

const SQUARE: f32 = 26.0;
const SQUARE_GAP: f32 = 2.0;
const WEEKDAY_COLUMN: f32 = 34.0;
pub const CALENDAR_HEIGHT: f32 = SQUARE * 8.0;

impl CalendarChart {
	fn week_start(&self, day: NaiveDate) -> NaiveDate {
		Bucket::Week.start(day, self.first_weekday)
	}

	fn columns(&self, width: f32) -> usize {
		((width - WEEKDAY_COLUMN) / SQUARE).floor().max(1.0) as usize
	}

	/// The first day of the leftmost column on show.
	fn first_shown(&self, width: f32, pan: &Pan) -> NaiveDate {
		let back = self.columns(width) as i64 - 1 + pan.columns(SQUARE) as i64;
		self.week_start(self.today) - Duration::days(7 * back)
	}

	fn day_at(&self, point: Point, width: f32, pan: &Pan) -> Option<NaiveDate> {
		if point.y < SQUARE || point.x >= self.columns(width) as f32 * SQUARE {
			return None;
		}
		let column = (point.x / SQUARE).floor() as i64;
		let row = ((point.y - SQUARE) / SQUARE).floor() as i64;
		let day = self.first_shown(width, pan) + Duration::days(column * 7 + row);
		((0..7).contains(&row) && day <= self.today).then_some(day)
	}
}

impl canvas::Program<Message> for CalendarChart {
	type State = Pan;

	fn update(
		&self,
		pan: &mut Pan,
		event: &iced::Event,
		bounds: Rectangle,
		cursor: mouse::Cursor,
	) -> Option<canvas::Action<Message>> {
		// Weeks back to the first record, and no further.
		let weeks = (self.today - self.statuses.first_day()).num_days() / 7 + 1;
		let hidden = weeks as f32 - self.columns(bounds.width) as f32;
		if let iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) = event
			&& let Some(point) = cursor.position_in(bounds)
			&& let Some(day) = self.day_at(point, bounds.width, pan)
		{
			return Some(
				canvas::Action::publish(Message::Habits(Action::OpenDay(
					self.habit_id.clone(),
					day,
				)))
				.and_capture(),
			);
		}
		let (changed, click) = pan.update(event, bounds, cursor, hidden * SQUARE);
		if let Some(point) = click
			&& let Some(day) = self.day_at(point, bounds.width, pan)
		{
			return Some(
				canvas::Action::publish(Message::Habits(Action::OpenDay(
					self.habit_id.clone(),
					day,
				)))
				.and_capture(),
			);
		}
		redraw_if(changed)
	}

	fn mouse_interaction(
		&self,
		pan: &Pan,
		bounds: Rectangle,
		cursor: mouse::Cursor,
	) -> mouse::Interaction {
		match cursor.position_in(bounds) {
			Some(point) if self.day_at(point, bounds.width, pan).is_some() => {
				mouse::Interaction::Pointer
			}
			_ => mouse::Interaction::default(),
		}
	}

	fn draw(
		&self,
		pan: &Pan,
		renderer: &Renderer,
		theme: &Theme,
		bounds: Rectangle,
		_cursor: mouse::Cursor,
	) -> Vec<Geometry> {
		let mut frame = Frame::new(renderer, bounds.size());
		let dark = is_dark(theme);
		let color = palette_color(self.color, dark);
		let card = card_background(theme);
		let off = low_contrast(dark);
		let columns = self.columns(bounds.width);
		let first = self.first_shown(bounds.width, pan);
		let size = SQUARE - SQUARE_GAP;
		let dimmed = blend(color, card, 0.5);

		let mut last_month = None;
		let mut last_year = None;
		// Text may run past its own column, so the next one starts after it.
		let mut overflow: f32 = 0.0;
		for column in 0..columns {
			let week = first + Duration::days(column as i64 * 7);
			let x = column as f32 * SQUARE;
			let header = if last_month != Some(week.month()) {
				last_month = Some(week.month());
				month_short(week.month()).to_owned()
			} else if last_year != Some(week.year()) {
				last_year = Some(week.year());
				week.year().to_string()
			} else {
				String::new()
			};
			if !header.is_empty() {
				label(
					&mut frame,
					header.clone(),
					Point::new(x + overflow, SQUARE / 2.0),
					medium_contrast(),
					iced::alignment::Horizontal::Left,
				);
			}
			overflow += header.len() as f32 * TEXT_SIZE * 0.55 + SQUARE * 0.1;
			overflow = (overflow - SQUARE).max(0.0);

			for row in 0..7 {
				let day = week + Duration::days(row);
				if day > self.today {
					continue;
				}
				let status = self.statuses.status(day);
				// Loop's squares: on, and dimmed when covered.
				let fill = match status {
					Status::Yes | Status::Clean => color,
					Status::Auto | Status::Skip | Status::Partial(_) => dimmed,
					Status::No | Status::Unknown | Status::Slipped(_) => off,
				};
				let origin = Point::new(x, SQUARE + row as f32 * SQUARE);
				frame.fill(
					&Path::rounded_rectangle(origin, Size::new(size, size), (size * 0.15).into()),
					fill,
				);
				if status == Status::Skip {
					// Loop's hatching, in the card's color.
					let mut k = size / 10.0;
					for _ in 0..5 {
						line(
							&mut frame,
							Point::new(origin.x + k, origin.y),
							Point::new(origin.x, origin.y + k),
							card,
							0.75,
						);
						line(
							&mut frame,
							Point::new(origin.x + size - k, origin.y + size),
							Point::new(origin.x + size, origin.y + size - k),
							card,
							0.75,
						);
						k += size / 5.0;
					}
				}
				let text = if fill == color {
					card
				} else if dark {
					Color::from_rgb8(0xBD, 0xBD, 0xBD)
				} else {
					Color::from_rgb8(0x75, 0x75, 0x75)
				};
				label(
					&mut frame,
					day.day().to_string(),
					Point::new(origin.x + size / 2.0, origin.y + size / 2.0),
					text,
					iced::alignment::Horizontal::Center,
				);
			}
		}
		for (row, name) in self.weekday_names.iter().enumerate() {
			label(
				&mut frame,
				*name,
				Point::new(
					columns as f32 * SQUARE + 2.0,
					SQUARE + row as f32 * SQUARE + size / 2.0,
				),
				medium_contrast(),
				iced::alignment::Horizontal::Left,
			);
		}
		vec![frame.into_geometry()]
	}
}

// ------------------------------------------------------------------ streaks

/// Loop's best streaks card: centred bars with the dates on either side.
pub struct StreakChart {
	pub streaks: Vec<Streak>,
	pub color: u8,
}

pub const STREAK_ROW: f32 = 24.0;

fn short_date(day: NaiveDate) -> String {
	format!("{} {}, {}", month_short(day.month()), day.day(), day.year())
}

impl canvas::Program<Message> for StreakChart {
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
		let dark = is_dark(theme);
		let color = palette_color(self.color, dark);
		let card = card_background(theme);
		// Dates make way for the bars in a narrow pane, as they do in Loop.
		let label_width = TEXT_SIZE * 0.55 * 12.0 + 6.0;
		let labels = bounds.width >= label_width * 2.0 + 60.0;
		let available = if labels {
			bounds.width - label_width * 2.0
		} else {
			bounds.width
		};
		let longest = self
			.streaks
			.iter()
			.map(|streak| streak.length())
			.max()
			.unwrap_or(1)
			.max(1);
		for (index, streak) in self.streaks.iter().enumerate() {
			let share = streak.length() as f32 / longest as f32;
			let width = (share * available).max(28.0);
			let x = (bounds.width - width) / 2.0;
			let y = index as f32 * STREAK_ROW;
			let fill = if share >= 1.0 {
				color
			} else if share >= 0.8 {
				blend(color, card, 0.75)
			} else if share >= 0.5 {
				blend(color, card, 0.38)
			} else {
				low_contrast(dark)
			};
			frame.fill(
				&Path::rounded_rectangle(
					Point::new(x, y + 1.0),
					Size::new(width, STREAK_ROW - 2.0),
					3.0.into(),
				),
				fill,
			);
			let middle = y + STREAK_ROW / 2.0;
			label(
				&mut frame,
				streak.length().to_string(),
				Point::new(bounds.width / 2.0, middle),
				if share >= 0.5 {
					card
				} else {
					medium_contrast()
				},
				iced::alignment::Horizontal::Center,
			);
			if labels {
				label(
					&mut frame,
					short_date(streak.start),
					Point::new(x - 6.0, middle),
					medium_contrast(),
					iced::alignment::Horizontal::Right,
				);
				label(
					&mut frame,
					short_date(streak.end),
					Point::new(x + width + 6.0, middle),
					medium_contrast(),
					iced::alignment::Horizontal::Left,
				);
			}
		}
		vec![frame.into_geometry()]
	}
}

// ---------------------------------------------------------------- frequency

/// Loop's frequency card: a dot per weekday per month, sized by how often the
/// habit was done on that weekday.
pub struct FrequencyChart {
	pub computed: Computed,
	pub color: u8,
	pub today: NaiveDate,
	pub weekday_names: [&'static str; 7],
	/// Monday-based offset of the first row.
	pub first_weekday: usize,
}

const FREQUENCY_CELL: f32 = 30.0;
pub const FREQUENCY_HEIGHT: f32 = FREQUENCY_CELL * 7.0 + AXIS_HEIGHT;

impl canvas::Program<Message> for FrequencyChart {
	type State = Pan;

	fn update(
		&self,
		pan: &mut Pan,
		event: &iced::Event,
		bounds: Rectangle,
		cursor: mouse::Cursor,
	) -> Option<canvas::Action<Message>> {
		let months = habits::weekday_frequency(&self.computed).len() as f32;
		let hidden = months - ((bounds.width - WEEKDAY_COLUMN) / FREQUENCY_CELL).floor();
		let (changed, _) = pan.update(event, bounds, cursor, hidden * FREQUENCY_CELL);
		redraw_if(changed)
	}

	fn draw(
		&self,
		pan: &Pan,
		renderer: &Renderer,
		theme: &Theme,
		bounds: Rectangle,
		_cursor: mouse::Cursor,
	) -> Vec<Geometry> {
		let mut frame = Frame::new(renderer, bounds.size());
		let dark = is_dark(theme);
		let color = palette_color(self.color, dark);
		let grid = low_contrast(dark);
		let shades = [
			grid,
			blend(color, grid, 0.33),
			blend(color, grid, 0.66),
			color,
		];
		let data = habits::weekday_frequency(&self.computed);
		let columns = ((bounds.width - WEEKDAY_COLUMN) / FREQUENCY_CELL)
			.floor()
			.max(1.0) as usize;
		let back = columns as u32 - 1 + pan.columns(FREQUENCY_CELL) as u32;
		let this_month = self.today.with_day(1).unwrap_or(self.today);
		let first = this_month
			.checked_sub_months(chrono::Months::new(back))
			.unwrap_or(this_month);

		for row in 0..=7 {
			let y = row as f32 * FREQUENCY_CELL;
			line(
				&mut frame,
				Point::new(0.0, y),
				Point::new(bounds.width, y),
				grid,
				1.0,
			);
			if row < 7 {
				label(
					&mut frame,
					self.weekday_names[row],
					Point::new(
						columns as f32 * FREQUENCY_CELL + 2.0,
						y + FREQUENCY_CELL / 2.0,
					),
					medium_contrast(),
					iced::alignment::Horizontal::Left,
				);
			}
		}

		let mut axis = Vec::new();
		for column in 0..columns {
			let month = first
				.checked_add_months(chrono::Months::new(column as u32))
				.unwrap_or(first);
			let x = column as f32 * FREQUENCY_CELL + FREQUENCY_CELL / 2.0;
			axis.push((x, month));
			let Some(values) = data.get(&month) else {
				continue;
			};
			let available = habits::weekdays_in_month(month);
			for row in 0..7 {
				let weekday = (row + self.first_weekday) % 7;
				let out_of = f64::from(available[weekday].max(1));
				let scale = (values[weekday] / out_of).min(1.0) as f32;
				if scale <= 0.0 {
					continue;
				}
				let radius = (FREQUENCY_CELL - 2.0 * FREQUENCY_CELL * 0.2) / 2.0 * scale;
				let shade = shades[((shades.len() - 1) as f32 * scale).round() as usize];
				frame.fill(
					&Path::circle(
						Point::new(x, row as f32 * FREQUENCY_CELL + FREQUENCY_CELL / 2.0),
						radius,
					),
					shade,
				);
			}
		}
		draw_axis(
			&mut frame,
			&axis,
			Bucket::Month,
			FREQUENCY_CELL * 7.0,
			medium_contrast(),
		);
		vec![frame.into_geometry()]
	}
}

/// A chart filling the card's width.
pub fn chart<'a, P>(program: P, height: f32) -> Element<'a, Message>
where
	P: canvas::Program<Message> + 'a,
{
	canvas::Canvas::new(program)
		.width(Length::Fill)
		.height(height)
		.into()
}
