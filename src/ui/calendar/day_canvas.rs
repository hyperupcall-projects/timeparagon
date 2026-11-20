//! The canvas that draws the day grid and handles dragging on it.

use super::*;

#[derive(Debug, Clone)]
pub(super) struct Block {
	pub(super) id: String,
	pub(super) title: String,
	pub(super) when: String,
	pub(super) start_min: i32,
	pub(super) end_min: i32,
	pub(super) column: usize,
	pub(super) columns: usize,
	pub(super) column_span: usize,
	pub(super) color: Color,
	/// Timed task blocks and calendar events can both be dragged.
	pub(super) editable: bool,
	/// Whether a change to this event is still being written to Google.
	/// The block is drawn lighter, and cannot be opened, dragged or ticked
	/// off, until the write is confirmed.
	pub(super) pending: bool,
	pub(super) tasks: Vec<(String, String, u32)>,
	pub(super) time_block: bool,
	/// The habit this event counts towards and the day it would be recorded
	/// on, for its Record button.
	pub(super) habit: Option<(String, NaiveDate)>,
	/// Completion of a visible task or habit occurrence.
	pub(super) completion: Option<Completion>,
	/// The Start sleep or Finish sleep button a night offers.
	pub(super) sleep_timer: Option<calendar::sleep::Timer>,
}

/// An alarm's needle on the day in view.
#[derive(Debug, Clone)]
pub(super) struct AlarmMark {
	pub(super) id: u64,
	/// Minutes from the top of the day, 0 to 24h.
	pub(super) minutes: i32,
	pub(super) label: String,
}

/// A trip drawn beside the events it joins: in a gray box just before the
/// event it leads to, or just after the day's last event for the trip home.
#[derive(Debug, Clone)]
pub(super) struct TravelBox {
	pub(super) leg: crate::travel::Leg,
	/// What the trip leaves and what it reaches, for its details card.
	pub(super) between: (String, String),
	pub(super) label: String,
	/// The minute of the day the box is fixed to: the start of the event it
	/// comes before, or the end of the event it follows.
	pub(super) anchor_min: i32,
	/// Whether the box hangs below its anchor rather than above it.
	pub(super) below: bool,
	/// How long the trip takes, once known, which sets the box's height.
	pub(super) minutes: u32,
	/// Where the event it belongs to sits, so the box lines up with it.
	pub(super) column: usize,
	pub(super) columns: usize,
	pub(super) column_span: usize,
}

pub(super) use timeparagon_core::theme::TRAVEL_MIN_HEIGHT;
use timeparagon_core::theme::{
	BLOCK_ACTION, BLOCK_FILLED, EVENT_DROPS, HOUR_LINE, TIME_BLOCK_LABEL_SHARE, TRAVEL_EDGE,
};

/// The gray boxes for the day's trips, found from the blocks already laid out.
pub(super) fn travel_boxes(state: &State, blocks: &[Block], day: NaiveDate) -> Vec<TravelBox> {
	crate::travel::day_hops(state, day)
		.into_iter()
		.filter_map(|hop| {
			let block = blocks.iter().find(|block| block.id == hop.before)?;
			let minutes = match crate::travel::summary(state, hop.leg) {
				crate::travel::Summary::Ready(route) => route.seconds.div_ceil(60),
				_ => 0,
			};

			Some(TravelBox {
				leg: hop.leg,
				label: crate::travel::short_label(state, hop.leg),
				between: (hop.from_title, hop.to_title),
				anchor_min: block.start_min,
				below: false,
				minutes,
				column: block.column,
				columns: block.columns,
				column_span: block.column_span,
			})
		})
		.collect()
}

pub(super) struct DayCanvas {
	pub(super) time_format: TimeFormat,
	pub(super) blocks: Vec<Block>,
	pub(super) travel: Vec<TravelBox>,
	pub(super) needle: Option<i32>,
	/// The alarms that fall on this day.
	pub(super) alarms: Vec<AlarmMark>,
	/// Whether the grid is in alarm setting mode: a purple needle follows the
	/// pointer, and a click sets an alarm where it is.
	pub(super) alarm_mode: bool,
	pub(super) hour_height: f32,
	pub(super) gutter: f32,
	pub(super) compact: bool,
	/// The hour drawn at the top of the grid. The day's window opens here, so
	/// with a 3am end of day the top row reads 3 AM and the small hours sit at
	/// the foot.
	pub(super) day_start_hour: u32,
	/// The date the grid is showing, so a dragged block can be turned back into
	/// a real time when it is dropped.
	pub(super) day: NaiveDate,
	/// Whether empty grid space draws out a new event. False while a card is
	/// open, so a click there still dismisses it.
	pub(super) can_create: bool,
	/// The span, in minutes into this day, selected by the open new-event
	/// draft. It stays highlighted after the pointer is released.
	pub(super) selection: Option<(i32, i32)>,
	pub(super) external_drag: bool,
	/// The task in the air as it would be placed on this day if let go now.
	pub(super) drop_preview: Option<Block>,
	/// The time block the task in the air would join if let go now.
	pub(super) drop_block: Option<String>,
}

/// A drag on empty grid space that is drawing out a new event.
///
/// Both ends are snapped to the quarter hour as the pointer moves, so the
/// preview lands on the same times the event will.
#[derive(Debug, Clone, Copy)]
pub(super) struct CreateDrag {
	/// The minute the press landed on.
	pub(super) anchor_min: i32,
	/// The minute under the pointer now.
	pub(super) head_min: i32,
}

/// What a drag on a block is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DragMode {
	/// Slide the whole event, keeping its length.
	Move,
	/// Pull the top edge up or down, keeping the end fixed.
	ResizeStart,
	/// Pull the bottom edge up or down, keeping the start fixed.
	ResizeEnd,
}

/// A drag in progress on the day grid.
///
/// The preview is kept here rather than in the store: the event is only written
/// once, when the pointer is released, so a drag does not rebuild the day on
/// every pixel of travel. The event is named by id rather than by its place in
/// the block list, so a sync landing mid-drag cannot move the wrong one.
#[derive(Debug, Clone)]
pub(super) struct Drag {
	/// The event being dragged.
	pub(super) id: String,
	pub(super) mode: DragMode,
	/// Where the pointer was when the drag started, in grid pixels.
	pub(super) origin_y: f32,
	/// The block's start and end when the drag started, in minutes.
	pub(super) start_min: i32,
	pub(super) end_min: i32,
	/// The times the block is drawn at while the drag is in progress.
	pub(super) preview_start: i32,
	pub(super) preview_end: i32,
}

/// The control under the pointer, tracked separately from its event so moving
/// between controls redraws the overlay without rebuilding the cached grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum HoverPart {
	Body,
	Action(Rectangle),
	Checkbox(Rectangle),
}

/// Geometry for the grid, the events and their labels, reused while scrolling.
///
/// The labels are traced into glyph outlines by [`fill_glyphs`], and that
/// tracing is the expensive part: it shapes every string and builds a path for
/// every glyph. The cache keeps that work for when the events or the zoom
/// actually change. The needle is drawn outside the cache so it can move
/// without rebuilding the titles.
pub(super) struct DrawState {
	pub(super) grid: canvas::Cache,
	pub(super) key: std::cell::Cell<u64>,
	/// Whether Ctrl is held, which turns the wheel into a zoom.
	pub(super) zoom_held: std::cell::Cell<bool>,
	/// Event and control under the pointer. Kept out of the grid cache.
	pub(super) hovered: std::cell::Cell<Option<(usize, HoverPart)>>,
	/// The drag in progress, if any. Kept out of the grid cache so the preview
	/// can follow the pointer without retracing the day.
	pub(super) drag: std::cell::RefCell<Option<Drag>>,
	/// The new-event drag in progress, if any. Kept out of the grid cache for
	/// the same reason as `drag`.
	pub(super) create: std::cell::RefCell<Option<CreateDrag>>,
}

impl DrawState {
	/// A copy of the drag in progress, if there is one.
	pub(super) fn drag(&self) -> Option<Drag> {
		self.drag.borrow().as_ref().cloned()
	}

	/// Replaces the drag in progress, or clears it with `None`.
	pub(super) fn set_drag(&self, drag: Option<Drag>) {
		*self.drag.borrow_mut() = drag;
	}

	/// A copy of the new-event drag in progress, if there is one.
	pub(super) fn create(&self) -> Option<CreateDrag> {
		*self.create.borrow()
	}

	/// Replaces the new-event drag in progress, or clears it with `None`.
	pub(super) fn set_create(&self, create: Option<CreateDrag>) {
		*self.create.borrow_mut() = create;
	}
}

impl Default for DrawState {
	fn default() -> Self {
		Self {
			grid: canvas::Cache::new(),
			key: std::cell::Cell::new(0),
			zoom_held: std::cell::Cell::new(false),
			hovered: std::cell::Cell::new(None),
			drag: std::cell::RefCell::new(None),
			create: std::cell::RefCell::new(None),
		}
	}
}

impl canvas::Program<Message> for DayCanvas {
	type State = DrawState;

	fn update(
		&self,
		state: &mut Self::State,
		event: &iced::Event,
		bounds: Rectangle,
		cursor: mouse::Cursor,
	) -> Option<canvas::Action<Message>> {
		// Alarm mode takes the pointer for itself: the grid is not a place to
		// drag events or draw new ones until the mode is left. Zooming still
		// works, so the time can be picked more finely.
		if self.alarm_mode
			&& !matches!(
				event,
				iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(_))
					| iced::Event::Mouse(mouse::Event::WheelScrolled { .. })
			) {
			return self.alarm_mode_update(event, bounds, cursor);
		}

		if matches!(
			event,
			iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
		) && self.external_drag
		{
			let point = cursor.position_in(bounds)?;
			let block = self.blocks.iter().find(|b| {
				b.time_block
					&& self
						.rect_for(b, bounds.width, b.start_min, b.end_min)
						.contains(point)
			});
			if let Some(block) = block {
				return Some(
					canvas::Action::publish(Message::TimeBlocking(
						crate::timeblocking::Action::DropBlock(block.id.clone()),
					))
					.and_capture(),
				);
			}
			return Some(
				canvas::Action::publish(Message::TimeBlocking(
					crate::timeblocking::Action::DropTime(self.time_at(self.snap_minute(point.y))),
				))
				.and_capture(),
			);
		}
		match event {
			iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => {
				state.zoom_held.set(modifiers.command());
				None
			}
			iced::Event::Mouse(mouse::Event::WheelScrolled { delta })
				if state.zoom_held.get() && cursor.position_in(bounds).is_some() =>
			{
				let steps = zoom_steps(*delta);
				if steps == 0.0 {
					return None;
				}

				// The row under the pointer, in grid pixels, so the zoom can
				// keep it anchored instead of letting the grid slide away.
				let anchor = cursor.position_in(bounds)?.y;

				// Scroll up spreads the hours apart; scroll down packs them.
				Some(
					canvas::Action::publish(Message::Calendar(CalendarMessage::CalendarZoomed {
						factor: WHEEL_ZOOM.powf(steps),
						anchor: Some(anchor),
					}))
					.and_capture(),
				)
			}
			iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
				// A drag owns the pointer until it is released, so the hover
				// highlight stays out of the way while the block follows it.
				if let Some(drag) = state.drag() {
					if let Some(point) = cursor.position_in(bounds) {
						let moved = self.preview(&drag, point.y);
						if moved.preview_start != drag.preview_start
							|| moved.preview_end != drag.preview_end
						{
							state.set_drag(Some(moved));
							return Some(canvas::Action::request_redraw().and_capture());
						}
					}
					return Some(canvas::Action::capture());
				}

				// A new-event drag owns the pointer too, so the hover highlight
				// stays out of the way while the preview follows it.
				if let Some(create) = state.create() {
					if let Some(point) = cursor.position_in(bounds) {
						let head = self.snap_minute(point.y);
						if head != create.head_min {
							state.set_create(Some(CreateDrag {
								head_min: head,
								..create
							}));
							return Some(canvas::Action::request_redraw().and_capture());
						}
					}
					return Some(canvas::Action::capture());
				}

				self.track_hover(state, bounds, cursor)
			}
			iced::Event::Mouse(mouse::Event::CursorLeft) => self.track_hover(state, bounds, cursor),
			iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
				let point = cursor.position_in(bounds)?;

				// An alarm's needle opens the alarm for editing.
				if let Some(id) = self.alarm_at(point, bounds.width) {
					return Some(
						canvas::Action::publish(Message::Alarms(alarms::Action::Open(id)))
							.and_capture(),
					);
				}

				// A travel box opens the trip's details. It is drawn over the
				// events, so it is asked before them.
				if let Some(travel) = self.travel_at(point, bounds.width) {
					return Some(
						canvas::Action::publish(Message::Travel(crate::travel::Action::OpenLeg(
							travel.leg,
							Some(travel.between.clone()),
						)))
						.and_capture(),
					);
				}

				// A block whose change is still on its way to Google is locked.
				if let Some(index) = self.hit(Some(point), bounds.width)
					&& self.blocks.get(index).is_some_and(|block| block.pending)
				{
					return Some(canvas::Action::capture());
				}

				for block in &self.blocks {
					if !block.time_block {
						continue;
					}
					let rect = self.block_rect(block, bounds.width);
					let (_, focus, plan) = block_actions(rect);
					if focus.contains(point) {
						return Some(
							canvas::Action::publish(Message::TimeBlocking(
								crate::timeblocking::Action::View(block.id.clone()),
							))
							.and_capture(),
						);
					}
					if plan.contains(point) {
						return Some(
							canvas::Action::publish(Message::TimeBlocking(
								crate::timeblocking::Action::Plan(block.id.clone()),
							))
							.and_capture(),
						);
					}
				}

				if let Some((id, completion)) = self.completion_at(point, bounds.width) {
					return Some(canvas::Action::publish(completion.toggle(id)).and_capture());
				}

				if let Some(id) = self.sleep_timer_at(point, bounds.width) {
					return Some(
						canvas::Action::publish(Message::Settings(
							crate::app::SettingsMessage::SleepTimerPressed(id.to_owned()),
						))
						.and_capture(),
					);
				}

				if let Some((id, day)) = self.record_at(point, bounds.width) {
					return Some(
						canvas::Action::publish(Message::Habits(crate::habits::Action::OpenDay(
							id.to_owned(),
							day,
						)))
						.and_capture(),
					);
				}

				// A press on a draggable block starts a drag. The click that
				// opens the details card is decided on release, so a press that
				// never moves still opens it.
				if let Some((index, mode)) = self.grab(point, bounds.width)
					&& self.draggable(index)
				{
					let block = self.blocks.get(index)?;
					state.set_drag(Some(Drag {
						id: block.id.clone(),
						mode,
						origin_y: point.y,
						start_min: block.start_min,
						end_min: block.end_min,
						preview_start: block.start_min,
						preview_end: block.end_min,
					}));
					return Some(canvas::Action::request_redraw().and_capture());
				}

				// Empty time either draws out a new event or closes any open
				// card, which is how clicking away still works now that there
				// is no dismiss layer.
				match self.hit(Some(point), bounds.width) {
					Some(index) => {
						let id = self.blocks.get(index)?.id.clone();
						Some(
							canvas::Action::publish(Message::Event(EventMessage::EventOpened(id)))
								.and_capture(),
						)
					}
					None if self.can_create => {
						let anchor = self.snap_minute(point.y);
						state.set_create(Some(CreateDrag {
							anchor_min: anchor,
							head_min: anchor,
						}));
						Some(canvas::Action::request_redraw().and_capture())
					}
					None => Some(
						canvas::Action::publish(Message::Event(EventMessage::EventClosed))
							.and_capture(),
					),
				}
			}
			iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
				// A new-event drag is finished first: it never shares the
				// pointer with a move or resize.
				if let Some(create) = state.create.take() {
					let (start, end) = self.create_range(&create);
					CREATE_ANCHOR.with(|anchor| {
						anchor.set(Rectangle {
							x: bounds.x + self.gutter,
							y: bounds.y + start as f32 * self.hour_height / 60.0,
							width: (bounds.width - self.gutter).max(1.0),
							height: (end - start) as f32 * self.hour_height / 60.0,
						})
					});
					return Some(
						canvas::Action::publish(Message::EventDraft(
							EventDraftMessage::EventDraftStarted {
								start: self.time_at(start),
								end: self.time_at(end),
							},
						))
						.and_capture(),
					);
				}

				let drag = state.drag.take()?;

				// A press that never moved is a click, not a drag.
				let action =
					if drag.preview_start == drag.start_min && drag.preview_end == drag.end_min {
						canvas::Action::publish(Message::Event(EventMessage::EventOpened(drag.id)))
					} else {
						canvas::Action::publish(Message::Event(EventMessage::EventTimeChanged {
							id: drag.id,
							start: self.time_at(drag.preview_start),
							end: self.time_at(drag.preview_end),
						}))
					};

				Some(action.and_capture())
			}
			_ => None,
		}
	}

	fn mouse_interaction(
		&self,
		state: &Self::State,
		bounds: Rectangle,
		cursor: mouse::Cursor,
	) -> mouse::Interaction {
		if self.alarm_mode {
			return match cursor.position_in(bounds) {
				Some(point) if self.alarm_can_be_set(point.y) => mouse::Interaction::Crosshair,
				Some(_) => mouse::Interaction::NotAllowed,
				None => mouse::Interaction::default(),
			};
		}

		if self.external_drag {
			return mouse::Interaction::Grabbing;
		}

		if cursor
			.position_in(bounds)
			.is_some_and(|point| self.alarm_at(point, bounds.width).is_some())
		{
			return mouse::Interaction::Pointer;
		}
		if cursor
			.position_in(bounds)
			.is_some_and(|point| self.travel_at(point, bounds.width).is_some())
		{
			return mouse::Interaction::Pointer;
		}
		if state.create().is_some() {
			return mouse::Interaction::Crosshair;
		}

		if let Some(drag) = state.drag() {
			return match drag.mode {
				DragMode::Move => mouse::Interaction::Grabbing,
				DragMode::ResizeStart | DragMode::ResizeEnd => {
					mouse::Interaction::ResizingVertically
				}
			};
		}

		if let Some(point) = cursor.position_in(bounds) {
			if let Some(index) = self.hit(Some(point), bounds.width)
				&& self.blocks[index].pending
			{
				return mouse::Interaction::NotAllowed;
			}
			for block in self.blocks.iter().filter(|block| block.time_block) {
				let (_, focus, plan) = block_actions(self.block_rect(block, bounds.width));
				if focus.contains(point) || plan.contains(point) {
					return mouse::Interaction::Pointer;
				}
			}
			if self.record_at(point, bounds.width).is_some()
				|| self.completion_at(point, bounds.width).is_some()
				|| self.sleep_timer_at(point, bounds.width).is_some()
			{
				return mouse::Interaction::Pointer;
			}
		}

		match cursor
			.position_in(bounds)
			.and_then(|point| self.grab(point, bounds.width))
		{
			Some((index, mode)) if self.draggable(index) => match mode {
				DragMode::Move => mouse::Interaction::Grab,
				DragMode::ResizeStart | DragMode::ResizeEnd => {
					mouse::Interaction::ResizingVertically
				}
			},
			Some(_) => mouse::Interaction::Pointer,
			None => mouse::Interaction::default(),
		}
	}

	fn draw(
		&self,
		state: &Self::State,
		renderer: &Renderer,
		theme: &Theme,
		bounds: Rectangle,
		cursor: mouse::Cursor,
	) -> Vec<Geometry> {
		// The dragged block is left out of the cached grid and drawn on top
		// instead, so the preview can follow the pointer without retracing the
		// day. The cache key carries which block that is, so the grid rebuilds
		// once when the drag starts and once when it ends.
		let drag = state.drag();
		let dragging = drag
			.as_ref()
			.and_then(|drag| self.blocks.iter().position(|block| block.id == drag.id));
		let key = cache_key(
			&self.blocks,
			self.hour_height,
			self.gutter,
			self.compact,
			self.day_start_hour,
			dragging,
		)
		.wrapping_mul(31)
		.wrapping_add(travel_key(&self.travel))
		.wrapping_mul(31)
		.wrapping_add(self.time_format as u64);
		if state.key.get() != key {
			state.grid.clear();
			state.key.set(key);
		}

		let grid = state.grid.draw(renderer, bounds.size(), |frame| {
			self.draw_grid(frame, dragging);
			self.draw_travel(frame);
		});

		let mut overlay = Frame::new(renderer, bounds.size());
		// A task in the air can land anywhere on the grid, so tinting the whole
		// day in the theme's primary color reads as one big drop target.
		if self.external_drag && !self.alarm_mode {
			overlay.fill_rectangle(
				Point::new(self.gutter, 0.0),
				Size::new((bounds.width - self.gutter).max(0.0), bounds.height),
				theme
					.extended_palette()
					.primary
					.base
					.color
					.scale_alpha(0.10),
			);
			if let Some(id) = &self.drop_block
				&& let Some(block) = self.blocks.iter().find(|block| &block.id == id)
			{
				self.draw_block_drop(&mut overlay, theme, block);
			}
			if let Some(preview) = &self.drop_preview {
				self.draw_block(&mut overlay, preview);
				let rect = self.block_rect(preview, bounds.width);
				overlay.stroke(
					&Path::rounded_rectangle(
						Point::new(rect.x, rect.y),
						Size::new(rect.width, rect.height),
						EVENT_RADIUS.into(),
					),
					Stroke {
						style: Style::Solid(theme.extended_palette().primary.strong.color),
						width: 2.0,
						..Stroke::default()
					},
				);
			}
		}
		self.draw_needle(&mut overlay);
		self.draw_alarms(&mut overlay, cursor.position_in(bounds));
		if self.alarm_mode {
			if let Some(point) = cursor.position_in(bounds) {
				self.draw_alarm_cursor(&mut overlay, point);
			}
		} else if let Some(create) = state.create() {
			let (start, end) = self.create_range(&create);
			self.draw_create(&mut overlay, theme, start, end);
		} else if let Some((start, end)) = self.selection {
			self.draw_create(&mut overlay, theme, start, end);
		} else if let Some(drag) = drag {
			self.draw_drag(&mut overlay, &drag);
		} else if !self.external_drag
			&& let Some((index, part)) = self.hover_at(cursor.position_in(bounds), bounds.width)
		{
			self.draw_hover(&mut overlay, index, part);
		}

		vec![grid, overlay.into_geometry()]
	}
}

impl DayCanvas {
	pub(super) fn draw_grid(&self, frame: &mut Frame, dragging: Option<usize>) {
		let grid_left = self.gutter;
		let label_size = if self.compact { 10.0 } else { 11.0 };

		for hour in 0..24 {
			let y = hour as f32 * self.hour_height;
			frame.stroke(
				&Path::line(Point::new(grid_left, y), Point::new(frame.width(), y)),
				Stroke {
					style: Style::Solid(HOUR_LINE),
					width: 1.0,
					..Stroke::default()
				},
			);

			fill_glyphs(
				frame,
				Text {
					content: calendar::time_format::hour(
						(self.day_start_hour + hour) % 24,
						self.time_format,
					),
					position: Point::new(self.gutter - 6.0, y + 2.0),
					color: colors::GUTTER_TEXT,
					size: Pixels(label_size),
					font: Font::DEFAULT,
					align_x: iced::alignment::Horizontal::Right.into(),
					align_y: iced::alignment::Vertical::Top,
					line_height: text::LineHeight::default(),
					shaping: text::Shaping::Basic,
					max_width: self.gutter - 8.0,
				},
			);
		}

		for (index, block) in self.blocks.iter().enumerate() {
			// The dragged block is drawn on top of the cache instead.
			if Some(index) == dragging {
				continue;
			}
			self.draw_block(frame, block);
		}
	}

	/// Outlines the time block a task in the air would join.
	fn draw_block_drop(&self, frame: &mut Frame, theme: &Theme, block: &Block) {
		let color = theme.extended_palette().primary.strong.color;
		let bounds = self.block_rect(block, frame.width());
		frame.stroke(
			&Path::rounded_rectangle(
				Point::new(bounds.x, bounds.y),
				Size::new(bounds.width, bounds.height),
				EVENT_RADIUS.into(),
			),
			Stroke {
				style: Style::Solid(color),
				width: 2.0,
				..Stroke::default()
			},
		);
	}

	/// Draws one event block with its label, actions and planned capacity.
	pub(super) fn draw_block(&self, frame: &mut Frame, block: &Block) {
		{
			let block_bounds = self.block_rect(block, frame.width());
			let border = event_border(false);
			let shape = Path::rounded_rectangle(
				Point::new(block_bounds.x, block_bounds.y),
				Size::new(block_bounds.width, block_bounds.height),
				border.radius,
			);

			let fill = block.color;
			// Canvas fills have no shadow of their own, so two faint offset
			// copies of the block stand in for one.
			for (drop, alpha) in EVENT_DROPS {
				frame.fill(
					&Path::rounded_rectangle(
						Point::new(block_bounds.x, block_bounds.y + drop),
						Size::new(block_bounds.width, block_bounds.height),
						EVENT_RADIUS.into(),
					),
					Color::BLACK.scale_alpha(alpha),
				);
			}
			frame.fill(&shape, fill);
			if block.time_block {
				let total: u32 = block.tasks.iter().map(|t| t.2).sum();
				let visible_start = block.start_min.max(0) as f32;
				let visible_end = block.end_min.min(DAY_MINUTES) as f32;
				let occupied_end = (block.start_min as f32 + total as f32).min(visible_end);
				let occupied = ((occupied_end - visible_start).max(0.0)
					/ (visible_end - visible_start).max(1.0))
				.min(1.0);
				frame.fill_rectangle(
					Point::new(block_bounds.x, block_bounds.y),
					Size::new(block_bounds.width, block_bounds.height * occupied),
					BLOCK_FILLED,
				);
			}
			// An unconfirmed write is outlined in grey with an italic label,
			// so the block reads as not yet final until Google answers.
			frame.stroke(
				&shape,
				if block.pending {
					Stroke {
						style: Style::Solid(crate::ui::controls::PENDING_BORDER),
						width: 2.0,
						..Stroke::default()
					}
				} else {
					Stroke {
						style: Style::Solid(border.color),
						width: border.width,
						..Stroke::default()
					}
				},
			);

			let ink = on_color(fill);
			let title_size = if self.compact { 11.0 } else { 12.0 };
			let mut label_bounds = block_bounds;
			if block.time_block {
				label_bounds = block_actions(block_bounds).0;
			} else if block.habit.is_some() {
				label_bounds = habit_actions(block_bounds).0;
			} else if block.sleep_timer.is_some() {
				label_bounds = sleep_actions(block_bounds).0;
			}
			if let Some(completion) = block.completion {
				let (label, checkbox) = completion_bounds(label_bounds);
				label_bounds = label;
				draw_completion_checkbox(frame, checkbox, completion.completed(), ink, false);
			}
			if label_bounds.height >= 14.0 {
				draw_block_label(
					frame,
					label_bounds,
					&block.title,
					&block.when,
					ink,
					title_size,
					block.pending,
				);
			}
			if block.time_block {
				let (_, focus, plan) = block_actions(block_bounds);
				draw_action(frame, focus, "Focus", ink);
				draw_action(frame, plan, "Plan", ink);
			} else if block.habit.is_some() {
				draw_action(frame, habit_actions(block_bounds).1, "Record", ink);
			} else if let Some(timer) = block.sleep_timer {
				draw_action(frame, sleep_actions(block_bounds).1, timer.label(), ink);
			}
		}
	}

	/// Draws the gray boxes for the trips, over the events they lead to.
	pub(super) fn draw_travel(&self, frame: &mut Frame) {
		let size = if self.compact { 10.0 } else { 11.0 };

		for trip in &self.travel {
			let bounds = self.travel_rect(trip, frame.width());
			let shape = Path::rounded_rectangle(
				Point::new(bounds.x, bounds.y),
				Size::new(bounds.width, bounds.height),
				EVENT_RADIUS.into(),
			);

			frame.fill(&shape, colors::TRAVEL);
			frame.stroke(
				&shape,
				Stroke {
					style: Style::Solid(TRAVEL_EDGE),
					width: 1.0,
					..Stroke::default()
				},
			);

			let room = (bounds.width - BLOCK_PADDING - BLOCK_PADDING_RIGHT).max(8.0);
			fill_glyphs(
				frame,
				Text {
					content: fit_line(&trip.label, size, room),
					position: Point::new(bounds.x + BLOCK_PADDING, bounds.y + 2.0),
					color: on_color(colors::TRAVEL),
					size: Pixels(size),
					font: Font::DEFAULT,
					align_x: iced::alignment::Horizontal::Left.into(),
					align_y: iced::alignment::Vertical::Top,
					line_height: text::LineHeight::default(),
					shaping: text::Shaping::Basic,
					max_width: room,
				},
			);
		}
	}

	/// Where a trip's box sits: on the event it belongs to, hanging off its
	/// start (or its end, for the trip home) by as long as the trip takes.
	pub(super) fn travel_rect(&self, trip: &TravelBox, width: f32) -> Rectangle {
		let minute_height = self.hour_height / 60.0;
		let grid_width = (width - self.gutter).max(40.0);
		let columns = trip.columns.max(1) as f32;
		let height = (trip.minutes as f32 * minute_height).max(TRAVEL_MIN_HEIGHT);
		let anchor = trip.anchor_min as f32 * minute_height;
		let span = trip.column_span.max(1) as f32;
		let box_width = (span * grid_width / columns - EVENT_GAP).max(8.0);

		Rectangle {
			x: self.gutter + trip.column as f32 * (grid_width / columns) + EVENT_GAP / 2.0,
			y: if trip.below {
				anchor
			} else {
				(anchor - height).max(0.0)
			},
			width: box_width,
			height,
		}
	}

	/// The trip box under a grid point, if any. The last drawn is on top.
	pub(super) fn travel_at(&self, point: Point, width: f32) -> Option<&TravelBox> {
		self.travel
			.iter()
			.rev()
			.find(|trip| self.travel_rect(trip, width).contains(point))
	}

	pub(super) fn draw_needle(&self, frame: &mut Frame) {
		let Some(minutes) = self.needle else {
			return;
		};

		let y = (minutes as f32 * (self.hour_height / 60.0)).clamp(0.0, frame.height());
		frame.stroke(
			&Path::line(Point::new(self.gutter, y), Point::new(frame.width(), y)),
			Stroke {
				style: Style::Solid(colors::NEEDLE),
				width: 2.0,
				..Stroke::default()
			},
		);
		frame.fill(
			&Path::circle(Point::new(self.gutter, y), NEEDLE_RADIUS),
			colors::NEEDLE,
		);
	}

	/// The pixel row a minute of the day sits on.
	pub(super) fn y_of(&self, minutes: i32) -> f32 {
		minutes as f32 * (self.hour_height / 60.0)
	}

	/// The minute of the day under a grid pixel, to the whole minute.
	///
	/// Alarms are exact where events snap to the quarter hour, so the needle
	/// follows the pointer minute by minute.
	pub(super) fn alarm_minute(&self, y: f32) -> i32 {
		let minute_height = self.hour_height / 60.0;

		((y / minute_height).round() as i32).clamp(0, DAY_MINUTES)
	}

	/// Whether an alarm can be set at the minute under a grid pixel.
	pub(super) fn alarm_can_be_set(&self, y: f32) -> bool {
		alarms::can_set(crate::clock::now(), self.time_at(self.alarm_minute(y)))
	}

	/// What alarm setting mode does with the pointer: the needle follows it,
	/// and a click sets the alarm if its time is still to come.
	pub(super) fn alarm_mode_update(
		&self,
		event: &iced::Event,
		bounds: Rectangle,
		cursor: mouse::Cursor,
	) -> Option<canvas::Action<Message>> {
		match event {
			iced::Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
				Some(canvas::Action::request_redraw())
			}
			iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
				let point = cursor.position_in(bounds)?;
				let at = self.time_at(self.alarm_minute(point.y));

				// A time that has passed is refused here as well as where the
				// alarm is set, and the click is still taken so it cannot fall
				// through to an event underneath.
				if alarms::can_set(crate::clock::now(), at) {
					Some(
						canvas::Action::publish(Message::Alarms(alarms::Action::Place(at)))
							.and_capture(),
					)
				} else {
					Some(canvas::Action::capture())
				}
			}
			_ => None,
		}
	}

	/// The pill an alarm's label sits in, above the right end of its needle.
	pub(super) fn alarm_pill(&self, mark: &AlarmMark, width: f32) -> Rectangle {
		let size = Size::new(self.alarm_pill_width(&mark.label, width), ALARM_PILL_HEIGHT);
		let y = self.y_of(mark.minutes);

		Rectangle {
			x: (width - size.width - ALARM_PILL_MARGIN).max(self.gutter),
			// Below the needle instead when there is no room above it.
			y: if y >= size.height + 2.0 {
				y - size.height - 1.0
			} else {
				y + 2.0
			},
			width: size.width,
			height: size.height,
		}
	}

	pub(super) fn alarm_pill_width(&self, label: &str, width: f32) -> f32 {
		let room = (width - self.gutter - ALARM_PILL_MARGIN * 2.0).max(24.0);

		(text_width_of(label, ALARM_PILL_TEXT) + 12.0).min(room)
	}

	/// The alarm under a grid point, if any.
	///
	/// The pill and the dot on the gutter always count. The needle's own line
	/// only counts where no event is, so an alarm never makes the event it
	/// crosses impossible to click.
	pub(super) fn alarm_at(&self, point: Point, width: f32) -> Option<u64> {
		let free = self.hit(Some(point), width).is_none();

		self.alarms.iter().rev().find_map(|mark| {
			let y = self.y_of(mark.minutes);
			let on_pill = self.alarm_pill(mark, width).contains(point);
			let on_dot = point.distance(Point::new(self.gutter, y)) <= NEEDLE_RADIUS + 3.0;
			let on_line = free && point.x >= self.gutter && (point.y - y).abs() <= 4.0;

			(on_pill || on_dot || on_line).then_some(mark.id)
		})
	}

	/// Draws every alarm's needle, with the one under the pointer thicker.
	pub(super) fn draw_alarms(&self, frame: &mut Frame, pointer: Option<Point>) {
		let hovered = if self.alarm_mode {
			None
		} else {
			pointer.and_then(|point| self.alarm_at(point, frame.width()))
		};

		for mark in &self.alarms {
			let y = self.y_of(mark.minutes).clamp(0.0, frame.height());
			let lit = hovered == Some(mark.id);

			frame.stroke(
				&Path::line(Point::new(self.gutter, y), Point::new(frame.width(), y)),
				Stroke {
					style: Style::Solid(colors::ALARM),
					width: if lit { 3.5 } else { 2.0 },
					..Stroke::default()
				},
			);
			frame.fill(
				&Path::circle(Point::new(self.gutter, y), NEEDLE_RADIUS),
				colors::ALARM,
			);

			let pill = self.alarm_pill(mark, frame.width());
			let label = fit_line(&mark.label, ALARM_PILL_TEXT, pill.width - 8.0);
			draw_pill(frame, pill, &label, colors::ALARM);
		}
	}

	/// Draws the needle that follows the pointer in alarm setting mode, with
	/// its offset from now beside it.
	///
	/// The needle goes grey and the offset turns negative once the pointer is
	/// over a time that has passed, which is the one place an alarm cannot go.
	pub(super) fn draw_alarm_cursor(&self, frame: &mut Frame, pointer: Point) {
		let minutes = self.alarm_minute(pointer.y);
		let at = self.time_at(minutes);
		let now = crate::clock::now();

		let color = if alarms::can_set(now, at) {
			colors::ALARM
		} else {
			colors::ALARM_REFUSED
		};
		let y = self.y_of(minutes);

		frame.stroke(
			&Path::line(Point::new(self.gutter, y), Point::new(frame.width(), y)),
			Stroke {
				style: Style::Solid(color),
				width: 2.0,
				..Stroke::default()
			},
		);
		frame.fill(
			&Path::circle(Point::new(self.gutter, y), NEEDLE_RADIUS),
			color,
		);

		let label = alarms::offset_label(alarms::minutes_between(now, at));
		let width = text_width_of(&label, ALARM_PILL_TEXT) + 12.0;
		let x = (pointer.x + 14.0).clamp(
			self.gutter + 4.0,
			(frame.width() - width - 4.0).max(self.gutter),
		);
		let top = if y >= ALARM_PILL_HEIGHT + 4.0 {
			y - ALARM_PILL_HEIGHT - 3.0
		} else {
			y + 5.0
		};

		draw_pill(
			frame,
			Rectangle {
				x,
				y: top,
				width,
				height: ALARM_PILL_HEIGHT,
			},
			&label,
			color,
		);
	}

	pub(super) fn draw_hover(&self, frame: &mut Frame, index: usize, part: HoverPart) {
		let Some(block) = self.blocks.get(index) else {
			return;
		};
		let ink = on_color(block.color);
		let border = event_border(true);
		let bounds = self.block_rect(block, frame.width());
		frame.stroke(
			&Path::rounded_rectangle(bounds.position(), bounds.size(), border.radius),
			Stroke {
				style: Style::Solid(border.color),
				width: border.width,
				..Stroke::default()
			},
		);
		match part {
			HoverPart::Body => {}
			HoverPart::Action(bounds) => {
				let shape = Path::rounded_rectangle(
					bounds.position(),
					bounds.size(),
					BLOCK_ACTION_RADIUS.into(),
				);
				frame.fill(&shape, ink.scale_alpha(0.09));
				frame.stroke(
					&shape,
					Stroke {
						style: Style::Solid(border.color),
						width: 1.0,
						..Stroke::default()
					},
				);
			}
			HoverPart::Checkbox(bounds) => {
				if let Some(completion) = block.completion {
					draw_completion_checkbox(frame, bounds, completion.completed(), ink, true);
				}
			}
		}
	}

	/// Draws the block being dragged at its preview position, in its own
	/// color with a black outline.
	pub(super) fn draw_drag(&self, frame: &mut Frame, drag: &Drag) {
		let Some(block) = self.blocks.iter().find(|block| block.id == drag.id) else {
			return;
		};
		let bounds = self.rect_for(block, frame.width(), drag.preview_start, drag.preview_end);
		let shape = Path::rounded_rectangle(
			Point::new(bounds.x, bounds.y),
			Size::new(bounds.width, bounds.height),
			EVENT_RADIUS.into(),
		);

		let fill = block.color;
		frame.fill(&shape, fill);
		frame.stroke(
			&shape,
			Stroke {
				style: Style::Solid(Color::BLACK),
				width: 2.0,
				..Stroke::default()
			},
		);

		let ink = on_color(fill);
		let title_size = if self.compact { 11.0 } else { 12.0 };

		// The label shows the time the event will land on, not the one it had.
		let when = self.range_label(drag.preview_start, drag.preview_end);
		draw_block_label(frame, bounds, &block.title, &when, ink, title_size, false);
	}

	/// Which block is under the pointer, and which part of it was grabbed.
	///
	/// The top and bottom edges resize; the middle moves. The edge zone is
	/// capped at a third of the block so a short event still has a middle.
	pub(super) fn grab(&self, point: Point, width: f32) -> Option<(usize, DragMode)> {
		let index = self.hit(Some(point), width)?;
		let block = self.blocks.get(index)?;
		let bounds = self.block_rect(block, width);
		let edge = EDGE_GRAB.min(bounds.height / 3.0);

		let mode = if block.start_min >= 0 && point.y - bounds.y <= edge {
			DragMode::ResizeStart
		} else if block.end_min <= DAY_MINUTES && bounds.y + bounds.height - point.y <= edge {
			DragMode::ResizeEnd
		} else {
			DragMode::Move
		};

		Some((index, mode))
	}

	pub(super) fn completion_at(&self, point: Point, width: f32) -> Option<(&str, Completion)> {
		let block = self.blocks.get(self.hit(Some(point), width)?)?;
		let completion = block.completion?;
		let bounds = self.block_rect(block, width);
		let label = if block.habit.is_some() {
			habit_actions(bounds).0
		} else {
			bounds
		};
		completion_bounds(label)
			.1
			.contains(point)
			.then_some((block.id.as_str(), completion))
	}

	/// The night whose Start/Finish sleep button is under a grid point.
	pub(super) fn sleep_timer_at(&self, point: Point, width: f32) -> Option<&str> {
		self.blocks.iter().rev().find_map(|block| {
			block.sleep_timer?;
			sleep_actions(self.block_rect(block, width))
				.1
				.contains(point)
				.then_some(block.id.as_str())
		})
	}

	/// The habit and day of the Record button under a grid point, if any.
	pub(super) fn record_at(&self, point: Point, width: f32) -> Option<(&str, NaiveDate)> {
		self.blocks.iter().rev().find_map(|block| {
			let (id, day) = block.habit.as_ref()?;
			habit_actions(self.block_rect(block, width))
				.1
				.contains(point)
				.then_some((id.as_str(), *day))
		})
	}

	/// Whether the block at this index can be dragged at all.
	pub(super) fn draggable(&self, index: usize) -> bool {
		self.blocks
			.get(index)
			.is_some_and(|block| block.editable && !block.pending)
	}

	/// The drag as it stands with the pointer at `cursor_y`.
	///
	/// The travel is snapped to [`SNAP_MINUTES`] and added to the times the
	/// drag started from, so the event keeps its offset and a press that does
	/// not move leaves the times untouched.
	pub(super) fn preview(&self, drag: &Drag, cursor_y: f32) -> Drag {
		let minute_height = self.hour_height / 60.0;
		let travel = (cursor_y - drag.origin_y) / minute_height;
		let delta = (travel / SNAP_MINUTES as f32).round() as i32 * SNAP_MINUTES;
		let duration = drag.end_min - drag.start_min;
		// Keep any part outside this day. Clamping the stored interval to
		// the visible slice would shorten an overnight event on its first edit.
		let lower = drag.start_min.min(0);
		let upper = drag.end_min.max(DAY_MINUTES);

		let (start, end) = match drag.mode {
			DragMode::Move => {
				let start = (drag.start_min + delta).clamp(lower, upper - duration);
				(start, start + duration)
			}
			DragMode::ResizeStart => {
				let start = (drag.start_min + delta).clamp(lower, drag.end_min - MIN_DURATION);
				(start, drag.end_min)
			}
			DragMode::ResizeEnd => {
				let end = (drag.end_min + delta).clamp(drag.start_min + MIN_DURATION, upper);
				(drag.start_min, end)
			}
		};

		Drag {
			id: drag.id.clone(),
			mode: drag.mode,
			origin_y: drag.origin_y,
			start_min: drag.start_min,
			end_min: drag.end_min,
			preview_start: start,
			preview_end: end,
		}
	}

	/// The minute-of-day a grid pixel lands on, snapped to the quarter hour
	/// and clamped to the day.
	pub(super) fn snap_minute(&self, y: f32) -> i32 {
		let minute_height = self.hour_height / 60.0;
		let minutes = (y / minute_height).round() as i32;
		let snapped = (minutes as f32 / SNAP_MINUTES as f32).round() as i32 * SNAP_MINUTES;
		snapped.clamp(0, DAY_MINUTES)
	}

	/// The span a new-event drag covers.
	///
	/// A click, or a drag too short to be a real span, gets
	/// [`DEFAULT_DURATION`] from where it started, the way Google Calendar
	/// does. The span is then clamped so it stays inside the day and never
	/// inverts.
	pub(super) fn create_range(&self, create: &CreateDrag) -> (i32, i32) {
		let (mut start, mut end) = if create.anchor_min <= create.head_min {
			(create.anchor_min, create.head_min)
		} else {
			(create.head_min, create.anchor_min)
		};

		if end - start < MIN_DURATION {
			end = start + DEFAULT_DURATION;
		}

		start = start.clamp(0, DAY_MINUTES - MIN_DURATION);
		end = end.clamp(start + MIN_DURATION, DAY_MINUTES);
		(start, end)
	}

	/// Draws the new-event preview: a translucent block in the target
	/// calendar's color, labelled with the span being drawn.
	pub(super) fn draw_create(&self, frame: &mut Frame, theme: &Theme, start: i32, end: i32) {
		let minute_height = self.hour_height / 60.0;
		let grid_width = (frame.width() - self.gutter).max(40.0);
		let bounds = Rectangle {
			x: self.gutter + EVENT_GAP / 2.0,
			y: start as f32 * minute_height,
			width: (grid_width - EVENT_GAP).max(8.0),
			height: ((end - start) as f32 * minute_height).max(16.0),
		};
		let shape = Path::rounded_rectangle(
			Point::new(bounds.x, bounds.y),
			Size::new(bounds.width, bounds.height),
			EVENT_RADIUS.into(),
		);

		let palette = theme.extended_palette();
		let (fill, outline) = if palette.is_dark {
			(
				Color::WHITE.scale_alpha(0.14),
				Color::WHITE.scale_alpha(0.35),
			)
		} else {
			(
				Color::from_rgb8(0xEE, 0xEE, 0xF0),
				Color::BLACK.scale_alpha(0.22),
			)
		};
		frame.fill(&shape, fill);
		frame.stroke(
			&shape,
			Stroke {
				style: Style::Solid(outline),
				width: 1.0,
				..Stroke::default()
			},
		);

		let ink = palette.background.base.text;
		let title_size = if self.compact { 11.0 } else { 12.0 };
		let text_width = (bounds.width - BLOCK_PADDING - BLOCK_PADDING_RIGHT).max(8.0);
		let label = self.range_label(start, end);
		fill_glyphs(
			frame,
			Text {
				content: fit_line(&label, title_size, text_width),
				position: Point::new(bounds.x + BLOCK_PADDING, bounds.y + 3.0),
				color: ink,
				size: Pixels(title_size),
				font: Font::DEFAULT,
				align_x: iced::alignment::Horizontal::Left.into(),
				align_y: iced::alignment::Vertical::Top,
				line_height: text::LineHeight::default(),
				shaping: text::Shaping::Basic,
				max_width: text_width,
			},
		);
	}

	/// The real time a previewed minute-of-day lands on.
	pub(super) fn time_at(&self, minutes: i32) -> DateTime<Local> {
		let offset = self.day_start_hour as i32 * 60;
		let naive = self.day.and_hms_opt(0, 0, 0).unwrap_or_default()
			+ chrono::Duration::minutes(i64::from(offset + minutes));

		calendar::local_time(naive)
	}

	/// The wall-clock time a previewed minute-of-day lands on.
	fn naive_time(&self, minutes: i32) -> chrono::NaiveTime {
		let total = (self.day_start_hour as i32 * 60 + minutes).rem_euclid(DAY_MINUTES);
		chrono::NaiveTime::from_hms_opt((total / 60) as u32, (total % 60) as u32, 0).unwrap()
	}

	/// A previewed range such as "8 – 10p", sharing the period when both ends do.
	pub(super) fn range_label(&self, start: i32, end: i32) -> String {
		calendar::time_format::range(
			self.naive_time(start),
			self.naive_time(end),
			self.time_format,
		)
	}

	pub(super) fn track_hover(
		&self,
		state: &mut DrawState,
		bounds: Rectangle,
		cursor: mouse::Cursor,
	) -> Option<canvas::Action<Message>> {
		let hit = self.hover_at(cursor.position_in(bounds), bounds.width);
		if state.hovered.get() == hit {
			return None;
		}
		state.hovered.set(hit);
		Some(canvas::Action::request_redraw())
	}

	fn hover_at(&self, local: Option<Point>, width: f32) -> Option<(usize, HoverPart)> {
		let point = local?;
		if self.alarm_at(point, width).is_some() || self.travel_at(point, width).is_some() {
			return None;
		}
		let index = self.hit(Some(point), width)?;
		let block = &self.blocks[index];
		if block.pending {
			return None;
		}
		let bounds = self.block_rect(block, width);
		let (label, actions) = if block.time_block {
			let (label, focus, plan) = block_actions(bounds);
			(label, [Some(focus), Some(plan)])
		} else if block.habit.is_some() {
			let (label, record) = habit_actions(bounds);
			(label, [Some(record), None])
		} else if block.sleep_timer.is_some() {
			let (label, timer) = sleep_actions(bounds);
			(label, [Some(timer), None])
		} else {
			(bounds, [None, None])
		};
		for action in actions.into_iter().flatten() {
			if action.contains(point) {
				return Some((index, HoverPart::Action(action)));
			}
		}
		if block.completion.is_some() {
			let checkbox = completion_bounds(label).1;
			if checkbox.contains(point) {
				return Some((index, HoverPart::Checkbox(checkbox)));
			}
		}
		Some((index, HoverPart::Body))
	}

	pub(super) fn hit(&self, local: Option<Point>, width: f32) -> Option<usize> {
		let point = local?;
		self.blocks
			.iter()
			.enumerate()
			.rev()
			.find_map(|(index, block)| {
				self.block_rect(block, width)
					.contains(point)
					.then_some(index)
			})
	}

	pub(super) fn block_rect(&self, block: &Block, width: f32) -> Rectangle {
		self.rect_for(block, width, block.start_min, block.end_min)
	}

	/// The rectangle a block occupies for a given pair of times, so the drag
	/// preview can be drawn at times the block does not hold yet.
	pub(super) fn rect_for(
		&self,
		block: &Block,
		width: f32,
		start_min: i32,
		end_min: i32,
	) -> Rectangle {
		let start_min = start_min.clamp(0, DAY_MINUTES);
		let end_min = end_min.clamp(0, DAY_MINUTES);
		let minute_height = self.hour_height / 60.0;
		let grid_width = (width - self.gutter).max(40.0);
		let columns = block.columns.max(1) as f32;
		let column = block.column as f32;
		let span = block.column_span.max(1) as f32;
		let block_width = span * (grid_width / columns) - EVENT_GAP;
		Rectangle {
			x: self.gutter + column * (grid_width / columns) + EVENT_GAP / 2.0,
			y: start_min as f32 * minute_height,
			width: block_width.max(8.0),
			height: ((end_min - start_min) as f32 * minute_height).max(16.0),
		}
	}
}

/// The text size inside an alarm's pill.
pub(super) const ALARM_PILL_TEXT: f32 = 10.0;
pub(super) const ALARM_PILL_HEIGHT: f32 = 16.0;
/// The gap between an alarm's pill and the right edge of the grid, which keeps
/// it clear of the scrollbar.
pub(super) const ALARM_PILL_MARGIN: f32 = 10.0;

/// A rounded label with white text on a solid color.
pub(super) fn draw_pill(frame: &mut Frame, bounds: Rectangle, label: &str, color: Color) {
	frame.fill(
		&Path::rounded_rectangle(bounds.position(), bounds.size(), 4.0.into()),
		color,
	);
	fill_glyphs(
		frame,
		Text {
			content: label.to_owned(),
			position: Point::new(bounds.center_x(), bounds.center_y()),
			color: Color::WHITE,
			size: Pixels(ALARM_PILL_TEXT),
			align_x: iced::alignment::Horizontal::Center.into(),
			align_y: iced::alignment::Vertical::Center,
			..Text::default()
		},
	);
}

/// A time block is split into the label on the left, with Plan stacked over
/// Focus on the right, inset from the edges and each other. Anything outside
/// the two buttons opens the details card through the usual click/drag
/// handling. Returns (label, focus, plan).
pub(super) fn block_actions(bounds: Rectangle) -> (Rectangle, Rectangle, Rectangle) {
	let label = Rectangle {
		width: bounds.width * TIME_BLOCK_LABEL_SHARE,
		..bounds
	};
	let plan = Rectangle {
		x: bounds.x + label.width,
		width: bounds.width - label.width,
		height: bounds.height / 2.0,
		..bounds
	};
	let focus = Rectangle {
		y: bounds.y + plan.height,
		..plan
	};
	(label, inset_action(focus), inset_action(plan))
}

/// The widest a habit event's Record button gets. Habit events span their whole
/// column, where a share of the width would make it far larger than its label.
const HABIT_RECORD_WIDTH: f32 = 56.0;

/// A habit event is split into the label on the left and an inset Record
/// button on the right. Returns (label, record).
pub(super) fn habit_actions(bounds: Rectangle) -> (Rectangle, Rectangle) {
	let width = (bounds.width * (1.0 - TIME_BLOCK_LABEL_SHARE)).min(HABIT_RECORD_WIDTH);
	let label = Rectangle {
		width: bounds.width - width,
		..bounds
	};
	let record = Rectangle {
		x: bounds.x + label.width,
		width,
		..bounds
	};
	(label, inset_action(record))
}

/// The widest a night's Start/Finish sleep button gets.
const SLEEP_TIMER_WIDTH: f32 = 72.0;

/// A night is split like a habit event, with a slightly wider button for the
/// longer label. Returns (label, timer).
pub(super) fn sleep_actions(bounds: Rectangle) -> (Rectangle, Rectangle) {
	let width = (bounds.width * (1.0 - TIME_BLOCK_LABEL_SHARE)).min(SLEEP_TIMER_WIDTH);
	let label = Rectangle {
		width: bounds.width - width,
		..bounds
	};
	let timer = Rectangle {
		x: bounds.x + label.width,
		width,
		..bounds
	};
	(label, inset_action(timer))
}

/// Leave a small margin around every event action, reducing it in tight spaces.
fn inset_action(bounds: Rectangle) -> Rectangle {
	let margin = 3.0_f32.min(bounds.width / 4.0).min(bounds.height / 4.0);
	bounds.shrink(margin)
}

/// The corner radius of a button drawn on a block.
const BLOCK_ACTION_RADIUS: f32 = 4.0;

/// One of the buttons drawn on a block.
fn draw_action(frame: &mut Frame, rect: Rectangle, name: &str, ink: Color) {
	let shape = Path::rounded_rectangle(rect.position(), rect.size(), BLOCK_ACTION_RADIUS.into());
	frame.fill(&shape, BLOCK_ACTION);
	frame.stroke(
		&shape,
		Stroke {
			style: Style::Solid(event_border(false).color),
			width: 1.0,
			..Stroke::default()
		},
	);
	// Keep the label inside its box even in overlapping columns.
	let size = (10.0 * ((rect.width - 2.0).max(0.0) / text_width_of(name, 10.0)).min(1.0))
		.min(rect.height.max(0.0));
	if size > 0.0 {
		fill_glyphs(
			frame,
			Text {
				content: name.into(),
				position: Point::new(rect.center_x(), rect.center_y()),
				color: ink,
				size: Pixels(size),
				align_x: iced::alignment::Horizontal::Center.into(),
				align_y: iced::alignment::Vertical::Center,
				..Text::default()
			},
		);
	}
}

/// Blends a color toward white, for the lighter shade an unconfirmed time
/// change is drawn in.
pub(super) fn lighten(color: Color) -> Color {
	const AMOUNT: f32 = 0.3;
	Color {
		r: color.r + (1.0 - color.r) * AMOUNT,
		g: color.g + (1.0 - color.g) * AMOUNT,
		b: color.b + (1.0 - color.b) * AMOUNT,
		a: color.a,
	}
}

/// Reserve a compact lane next to the title, before the optional Record action.
pub(super) fn completion_bounds(bounds: Rectangle) -> (Rectangle, Rectangle) {
	let width = 24.0_f32.min(bounds.width * 0.4);
	let label = Rectangle {
		width: bounds.width - width,
		..bounds
	};
	let checkbox = Rectangle {
		x: bounds.x + label.width,
		width,
		..bounds
	};
	(label, checkbox)
}

fn draw_completion_checkbox(
	frame: &mut Frame,
	bounds: Rectangle,
	completed: bool,
	ink: Color,
	hovered: bool,
) {
	let size = 14.0_f32
		.min((bounds.width - 4.0).max(0.0))
		.min((bounds.height - 4.0).max(0.0));
	if size < 3.0 {
		return;
	}
	let x = bounds.center_x() - size / 2.0;
	let y = bounds.center_y() - size / 2.0;
	let shape = Path::rounded_rectangle(Point::new(x, y), Size::new(size, size), 2.0.into());
	if hovered {
		frame.fill(&shape, ink.scale_alpha(0.09));
	}
	frame.stroke(
		&shape,
		Stroke {
			style: Style::Solid(ink),
			width: if hovered { 1.8 } else { 1.2 },
			..Stroke::default()
		},
	);
	if completed || hovered {
		let tick = Path::new(|path| {
			path.move_to(Point::new(x + size * 0.2, y + size * 0.5));
			path.line_to(Point::new(x + size * 0.43, y + size * 0.75));
			path.line_to(Point::new(x + size * 0.82, y + size * 0.25));
		});
		frame.stroke(
			&tick,
			Stroke {
				style: Style::Solid(ink),
				width: 1.8,
				..Stroke::default()
			},
		);
	}
}
