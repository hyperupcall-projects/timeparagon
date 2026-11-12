//! Shows where the task in the air would land if it were let go now.
//!
//! The drop under the pointer is found by [`super::task_drop::probe`], which
//! runs the plan view's own release handling, and kept in
//! `state.timeblocking.drop_hover`. These helpers turn it into the task as it
//! would be after the drop, for each view to draw in place.
use crate::app::{Message, State};
use crate::calendar::{self, Event};
use crate::timeblocking::{self as tb, Action, DropHover};
use chrono::NaiveDate;
use iced::widget::{container, text};
use iced::{Element, Fill, Theme};

fn hover(state: &State) -> Option<(&DropHover, &Event)> {
	let id = state.timeblocking.dragging.as_deref()?;
	let hover = state.timeblocking.drop_hover.as_ref()?;
	Some((hover, state.calendar.event(id)?))
}

/// The day a hover stands for: its column's, or the day being planned.
fn day_of(state: &State, hover: &DropHover) -> NaiveDate {
	hover.day.unwrap_or(state.calendar.day)
}

/// The dragged task as it would be timed after a drop on the time grid,
/// matching [`Action::DropTime`].
pub(crate) fn timed(state: &State) -> Option<Event> {
	let (hover, task) = hover(state)?;
	let Action::DropTime(start) = hover.action else {
		return None;
	};
	let mut task = task.clone();
	task.start = start;
	task.end = start + duration(&task);
	task.all_day = false;
	Some(task)
}

/// Whether letting go now drops the task on `day` as a whole, matching
/// [`Action::DropDay`], which makes it an all-day task on that day.
pub(crate) fn on_day(state: &State, day: NaiveDate) -> Option<Event> {
	let (hover, task) = hover(state)?;
	if !matches!(hover.action, Action::DropDay) || day_of(state, hover) != day {
		return None;
	}
	let mut task = task.clone();
	task.start = calendar::at_clock(day, 0, 0);
	task.end = task.start + chrono::Duration::days(1);
	task.all_day = true;
	Some(task)
}

/// Where a drop on the week board's `day` column would put the task: the card
/// it goes next to (none for the end of the column), whether it goes after
/// that card, and the task as it would then be timed.
pub(crate) fn week_list(state: &State, day: NaiveDate) -> Option<(Option<String>, bool, Event)> {
	let (hover, task) = hover(state)?;
	let Action::DropWeekList { target, after } = &hover.action else {
		return None;
	};
	if day_of(state, hover) != day || target.as_deref() == Some(task.id.as_str()) {
		return None;
	}
	let boundary = state.config.calendar.day_start_hour();
	let mut preview = task.clone();
	// Matches `drop_week_list`: a task already timed on the day keeps its
	// time and only moves in the list.
	if task.all_day || calendar::day_at(task.start, boundary) != day {
		let (start, end) = tb::week_drop_times(task, day, boundary)?;
		preview.start = start;
		preview.end = end;
		preview.all_day = false;
	}
	Some((target.clone(), *after, preview))
}

/// The time block the task would go into, or the task row in a block it
/// would be dropped on (and so join that row's block).
pub(crate) fn block_or_row(state: &State) -> Option<&str> {
	let (hover, _) = hover(state)?;
	match &hover.action {
		Action::DropBlock(block) => Some(block),
		// On the Backlog screen a row drop sorts into a backlog section instead.
		Action::DropTask(row) if state.screen != crate::app::Screen::Backlog => Some(row),
		_ => None,
	}
}

/// Whether the floating card should step aside: on the time grid the preview
/// itself sits under the pointer and shows the task better.
pub(crate) fn hides_ghost(state: &State) -> bool {
	state
		.timeblocking
		.drop_hover
		.as_ref()
		.is_some_and(|hover| matches!(hover.action, Action::DropTime(_)))
}

/// The outline drawn around a preview, so it reads as where the task would go
/// rather than as something already there.
pub(crate) fn outline(theme: &Theme) -> iced::Border {
	iced::Border {
		color: theme.extended_palette().primary.strong.color,
		width: 2.0,
		radius: super::controls::RADIUS_ROW.into(),
	}
}

/// A one-line chip for the dragged task in its own color, for the day cells
/// and all-day rows.
pub(crate) fn chip<'a>(state: &State, task: &Event, size: f32) -> Element<'a, Message> {
	let color = task.display_color(&state.config.calendar, &state.calendar.calendars);
	let ink = calendar::colors::on_color(color);
	container(
		text(tb::task_label(task))
			.size(size)
			.color(ink)
			.wrapping(text::Wrapping::None),
	)
	.width(Fill)
	.padding([3, 6])
	.clip(true)
	.style(move |theme: &Theme| container::Style {
		background: Some(color.into()),
		border: outline(theme),
		shadow: iced::Shadow {
			color: iced::Color::BLACK.scale_alpha(0.25),
			offset: iced::Vector::new(0.0, 2.0),
			blur_radius: 6.0,
		},
		..container::Style::default()
	})
	.into()
}

/// What a row in a day's list stands for, so a drop between rows can be timed
/// against its neighbours.
#[derive(Debug, Clone)]
pub(crate) struct Slot {
	id: String,
	all_day: bool,
	start: chrono::DateTime<chrono::Local>,
	end: chrono::DateTime<chrono::Local>,
}

impl Slot {
	pub(crate) fn of(event: &Event) -> Self {
		Self {
			id: event.id.clone(),
			all_day: event.all_day,
			start: event.start,
			end: event.end,
		}
	}
}

/// How long the task runs once timed, matching [`Action::DropTime`].
fn duration(task: &Event) -> chrono::Duration {
	chrono::Duration::minutes(i64::from(match tb::metadata(task).minutes {
		0 => 30,
		minutes => minutes,
	}))
}

/// When a task dropped between `above` and `below` would start: right as the
/// timed row above ends, or so it ends as the timed row below starts. With no
/// timed row on either side it goes on the day as a whole.
fn gap_start(
	task: &Event,
	above: Option<&Slot>,
	below: Option<&Slot>,
) -> Option<chrono::DateTime<chrono::Local>> {
	match (above.filter(|s| !s.all_day), below.filter(|s| !s.all_day)) {
		(Some(above), _) => Some(above.end),
		(None, Some(below)) => Some(below.start - duration(task)),
		(None, None) => None,
	}
}

/// The dragged task as a drop into a gap of `day`'s list would leave it, and
/// the row the gap follows.
fn between(state: &State, day: NaiveDate) -> Option<(Option<String>, Event)> {
	let (hover, task) = hover(state)?;
	let Action::DropAt { after, start } = &hover.action else {
		return None;
	};
	if day_of(state, hover) != day {
		return None;
	}
	let mut task = task.clone();
	match start {
		Some(start) => {
			task.end = *start + duration(&task);
			task.start = *start;
			task.all_day = false;
		}
		None => {
			task.start = calendar::at_clock(day, 0, 0);
			task.end = task.start + chrono::Duration::days(1);
			task.all_day = true;
		}
	}
	Some((after.clone(), task))
}

/// Lets a task in the air drop between the rows of `day`'s list: over a
/// row's top half it goes just before that row, over its bottom half just
/// after. Rows without a slot (notes, counts) are left alone. The task, as it
/// would be, is shown in the gap under the pointer by `preview`.
///
/// `wrap` gives a drop message the day it belongs to, where the list is not
/// already inside an element that does so.
pub(crate) fn day_list<'a>(
	state: &State,
	day: NaiveDate,
	rows: Vec<(Option<Slot>, Element<'a, Message>)>,
	wrap: impl Fn(Message) -> Message,
	preview: impl FnOnce(&Event) -> Element<'a, Message>,
) -> Vec<Element<'a, Message>> {
	let Some(task) = state
		.timeblocking
		.dragging
		.as_deref()
		.and_then(|id| state.calendar.event(id))
	else {
		return rows.into_iter().map(|(_, row)| row).collect();
	};
	// The task itself is moving, so it is no one's neighbour.
	let neighbours: Vec<Slot> = rows
		.iter()
		.filter_map(|(slot, _)| slot.clone())
		.filter(|slot| slot.id != task.id)
		.collect();
	// Gap `k` is just before neighbour `k`; the last one ends the list.
	let gap = |k: usize| {
		let above = k.checked_sub(1).map(|i| &neighbours[i]);
		let below = neighbours.get(k);
		wrap(Message::TimeBlocking(Action::DropAt {
			after: above.map(|slot| slot.id.clone()),
			start: gap_start(task, above, below),
		}))
	};
	let shown = between(state, day).and_then(|(after, moved)| {
		let k = match after {
			Some(id) => neighbours.iter().position(|n| n.id == id)? + 1,
			None => 0,
		};
		Some((k, moved))
	});
	let preview_gap = shown.as_ref().map(|(k, _)| *k);
	// It answers to the same drop as the gap it shows, so moving over it
	// keeps it put rather than sending it back and forth as rows shift.
	let mut preview = shown.map(|(k, moved)| gap_target(preview(&moved), gap(k), gap(k)));
	let mut list = Vec::with_capacity(rows.len() + 1);
	let mut passed = 0;
	for (slot, row) in rows {
		let Some(slot) = slot else {
			list.push(row);
			continue;
		};
		if preview_gap == Some(passed)
			&& let Some(preview) = preview.take()
		{
			list.push(preview);
		}
		if slot.id == task.id {
			// The task's own row keeps it about where it is.
			list.push(gap_target(row, gap(passed), gap(passed)));
		} else {
			list.push(gap_target(row, gap(passed), gap(passed + 1)));
			passed += 1;
		}
	}
	list.extend(preview);
	list
}

/// A row that takes a dropped task: `before` over its top half, `after` over
/// its bottom half.
fn gap_target<'a>(
	content: Element<'a, Message>,
	before: Message,
	after: Message,
) -> Element<'a, Message> {
	Element::new(GapTarget {
		content,
		before,
		after,
	})
}

struct GapTarget<'a> {
	content: Element<'a, Message>,
	before: Message,
	after: Message,
}

use iced::advanced::{Layout, Shell, Widget, layout as node, overlay, renderer, widget};
use iced::{Event as UiEvent, Length, Rectangle, Renderer, Size, Vector, mouse};

impl Widget<Message, Theme, Renderer> for GapTarget<'_> {
	fn children(&self) -> Vec<widget::Tree> {
		vec![widget::Tree::new(&self.content)]
	}
	fn diff(&self, tree: &mut widget::Tree) {
		tree.diff_children(std::slice::from_ref(&self.content));
	}
	fn size(&self) -> Size<Length> {
		self.content.as_widget().size()
	}
	fn layout(
		&mut self,
		tree: &mut widget::Tree,
		renderer: &Renderer,
		limits: &node::Limits,
	) -> node::Node {
		self.content
			.as_widget_mut()
			.layout(&mut tree.children[0], renderer, limits)
	}
	fn operate(
		&mut self,
		tree: &mut widget::Tree,
		layout: Layout<'_>,
		renderer: &Renderer,
		operation: &mut dyn widget::Operation,
	) {
		operation.traverse(&mut |operation| {
			self.content
				.as_widget_mut()
				.operate(&mut tree.children[0], layout, renderer, operation)
		});
	}
	fn update(
		&mut self,
		tree: &mut widget::Tree,
		event: &UiEvent,
		layout: Layout<'_>,
		cursor: mouse::Cursor,
		renderer: &Renderer,
		clipboard: &mut dyn iced::advanced::Clipboard,
		shell: &mut Shell<'_, Message>,
		viewport: &Rectangle,
	) {
		let bounds = layout.bounds();
		if matches!(
			event,
			UiEvent::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
		) && cursor.is_over(bounds)
			&& cursor.is_over(*viewport)
			&& let Some(point) = cursor.position()
		{
			shell.publish(if point.y < bounds.center_y() {
				self.before.clone()
			} else {
				self.after.clone()
			});
			shell.capture_event();
			return;
		}
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
		tree: &widget::Tree,
		renderer: &mut Renderer,
		theme: &Theme,
		style: &renderer::Style,
		layout: Layout<'_>,
		cursor: mouse::Cursor,
		viewport: &Rectangle,
	) {
		self.content.as_widget().draw(
			&tree.children[0],
			renderer,
			theme,
			style,
			layout,
			cursor,
			viewport,
		);
	}
	fn mouse_interaction(
		&self,
		_tree: &widget::Tree,
		layout: Layout<'_>,
		cursor: mouse::Cursor,
		_viewport: &Rectangle,
		_renderer: &Renderer,
	) -> mouse::Interaction {
		if cursor.is_over(layout.bounds()) {
			mouse::Interaction::Grabbing
		} else {
			mouse::Interaction::None
		}
	}
	fn overlay<'b>(
		&'b mut self,
		tree: &'b mut widget::Tree,
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

/// Outlines a row standing in for the task in the air.
pub(crate) fn outlined<'a>(row: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
	iced::widget::stack![
		row.into(),
		container(iced::widget::Space::new())
			.width(Fill)
			.height(Fill)
			.style(|theme: &Theme| container::Style {
				border: outline(theme),
				..container::Style::default()
			}),
	]
	.into()
}
