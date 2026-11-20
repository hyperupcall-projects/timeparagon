//! The type chooser over a range chosen on the calendar, and what the event
//! forms share.

use super::*;
use iced::widget::column;

/// What a new item of `kind` is called at the top of its form.
pub(crate) fn draft_heading(kind: calendar::DraftKind) -> &'static str {
	match kind {
		calendar::DraftKind::Task => "New task",
		calendar::DraftKind::TimeBlock => "New time block",
		calendar::DraftKind::Habit => "New habit",
		calendar::DraftKind::Sleep => "New sleep",
		calendar::DraftKind::Event => "New event",
	}
}

/// A title laid out as the creation form lays out every type's, so switching
/// types keeps it in place.
pub(crate) fn creation_title<'a>(
	state: &'a State,
	input: iced::widget::TextInput<'a, Message>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	column![
		crate::ui::controls::field_label("Title", metrics.label()),
		row![
			input.id(iced::widget::Id::new(crate::inbox::TITLE_FIELD)),
			crate::image_input::text_button(state),
			crate::image_input::input_button(state)
		]
		.spacing(6)
		.align_y(Alignment::Center),
		crate::image_input::sources(state)
	]
	.spacing(6)
	.into()
}

/// Only the title and type chooser float over the selected calendar range.
pub(super) fn draft_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let Some(draft) = state.calendar.draft.as_ref() else {
		return Space::new().into();
	};
	let weekly = state.weekly();
	let metrics = *metrics;
	let focused_kind = state.draft_kind_focus;
	let overlay = responsive(move |size| {
		let control_height = metrics.label() * 1.3 + 12.0;
		let title = crate::ui::controls::text_input("Add title", &draft.title)
			.id("calendar-draft-title")
			.padding(6.0)
			.on_input(|v0| Message::EventDraft(EventDraftMessage::EventDraftTitleChanged(v0)))
			.size(metrics.label() + 2.0)
			.width(Fill);
		let kind_button = |label: &'static str, kind: calendar::DraftKind| {
			button(text(label).size(metrics.label()).center())
				.height(control_height)
				.padding(6.0)
				.width(Fill)
				.on_press(Message::EventDraft(
					EventDraftMessage::EventDraftKindSelected(kind),
				))
				.style(move |theme, status| {
					if focused_kind == Some(kind) {
						crate::ui::controls::primary(theme, status)
					} else {
						crate::ui::controls::secondary(theme, status)
					}
				})
		};
		let kinds: Element<'a, Message> = column![
			row![
				kind_button("Event", calendar::DraftKind::Event),
				kind_button("Time Block", calendar::DraftKind::TimeBlock),
			]
			.spacing(6.0)
			.width(Fill),
			row![
				kind_button("Task", calendar::DraftKind::Task),
				kind_button("Habit", calendar::DraftKind::Habit),
			]
			.spacing(6.0)
			.width(Fill),
			button(text("Cancel").size(metrics.label()).center().width(Fill))
				.height(control_height)
				.padding(6.0)
				.width(Fill)
				.on_press(Message::EventDraft(EventDraftMessage::EventDraftCancelled))
				.style(crate::ui::controls::secondary),
		]
		.spacing(6.0)
		.width(Fill)
		.into();

		let body = column![title, kinds].spacing(metrics.spacing);
		let card = mouse_area(
			container(crate::ui::controls::scrollable(body).height(Fill))
				.padding(metrics.padding)
				.width(Fill)
				.height(Fill)
				.style(crate::ui::controls::surface),
		)
		.on_press(Message::Ignored);

		let width = 340.0_f32.min((size.width - 16.0).max(1.0));
		// Match the input, three button rows, gaps and both padding edges.
		let height = (2.0 * metrics.padding
			+ (metrics.label() + 2.0) * 1.3
			+ 12.0 + 3.0 * control_height
			+ metrics.spacing
			+ 12.0)
			.min((size.height - 16.0).max(1.0));
		let anchor = CREATE_ANCHOR.with(|a| a.get());
		let root = ROOT_BOUNDS.with(|r| r.get());
		let anchor = Rectangle {
			x: anchor.x - root.x,
			y: anchor.y - root.y,
			..anchor
		};
		let position = draft_position(anchor, size, Size::new(width, height), weekly);
		container(container(card).width(width).height(height))
			.padding(iced::Padding {
				top: position.y,
				left: position.x,
				right: 0.0,
				bottom: 0.0,
			})
			.width(Fill)
			.height(Fill)
			.into()
	});
	let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill))
		.on_press(Message::EventDraft(EventDraftMessage::EventDraftCancelled));
	stack![dismiss, overlay].into()
}

pub(super) fn draft_position(anchor: Rectangle, viewport: Size, card: Size, weekly: bool) -> Point {
	let gap = 8.0;
	let max_x = (viewport.width - card.width - gap).max(0.0);
	let max_y = (viewport.height - card.height - gap).max(0.0);
	let x = anchor.x.clamp(gap.min(max_x), max_x);
	if anchor.y - gap - card.height >= gap {
		Point::new(x, (anchor.y - gap - card.height).min(max_y))
	} else if weekly && anchor.x - gap - card.width >= gap {
		Point::new(anchor.x - gap - card.width, anchor.y.clamp(0.0, max_y))
	} else {
		Point::new(x, (anchor.y + anchor.height + gap).clamp(0.0, max_y))
	}
}

/// A calendar offered in the add-event popup's calendar picker.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CalendarChoice {
	pub(super) id: String,
	pub(super) name: String,
	pub(super) color: Color,
}

impl std::fmt::Display for CalendarChoice {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(formatter, "{}", self.name)
	}
}

/// A small colored circle used as a calendar swatch in the dropdown.
pub(crate) fn color_circle<'a>(color: Color) -> Element<'a, Message> {
	container(iced::widget::Space::new().width(10.0).height(10.0))
		.style(move |_theme| container::Style {
			background: Some(color.into()),
			border: iced::Border {
				radius: crate::ui::controls::RADIUS_ROW.into(),
				..iced::Border::default()
			},
			..container::Style::default()
		})
		.into()
}

/// A calendar choice under a "Calendar" caption: a dropdown when there is room,
/// and a button that cycles the options when there is not.
pub(super) fn calendar_select<'a>(
	options: Vec<CalendarChoice>,
	selected: Option<CalendarChoice>,
	dropdown_open: bool,
	metrics: &Metrics,
	on_select: fn(String) -> Message,
	on_toggle: Message,
) -> Element<'a, Message> {
	let caption = crate::ui::controls::field_label("Calendar", metrics.label() - 1.0);

	if metrics.breakpoint.is_compact() {
		let shown = selected
			.as_ref()
			.map(|option| option.name.clone())
			.unwrap_or_else(|| "Calendar".to_owned());
		let next = next_calendar(&options, selected.as_ref());

		let control = button(text(shown).size(metrics.label()).center().width(Fill)).width(Fill);
		let control = match next {
			Some(option) => control.on_press(on_select(option.id)),
			None => control,
		};

		column![caption, control.style(crate::ui::controls::secondary)]
			.spacing(2.0)
			.width(Fill)
			.into()
	} else if dropdown_open {
		calendar_dropdown_overlay(options, selected, metrics, on_select, on_toggle)
	} else {
		let shown = selected
			.as_ref()
			.map(|option| option.name.clone())
			.unwrap_or_else(|| "Calendar".to_owned());
		let color = selected
			.as_ref()
			.map_or(Color::from_rgb8(0xCC, 0xCC, 0xCC), |c| c.color);

		let control = button(
			row![
				color_circle(color),
				text(shown).size(metrics.label()).width(Fill),
			]
			.spacing(8.0)
			.align_y(Alignment::Center)
			.padding([2.0, 8.0]),
		)
		.width(Fill)
		.on_press(on_toggle)
		.style(crate::ui::controls::secondary);

		column![caption, control].spacing(2.0).width(Fill).into()
	}
}

/// The open calendar dropdown overlay: a scrollable list of calendars,
/// each shown with its color circle to the left of the name.
pub(super) fn calendar_dropdown_overlay<'a>(
	options: Vec<CalendarChoice>,
	selected: Option<CalendarChoice>,
	metrics: &Metrics,
	on_select: fn(String) -> Message,
	on_toggle: Message,
) -> Element<'a, Message> {
	let dismiss = mouse_area(
		container(iced::widget::Space::new())
			.width(Fill)
			.height(Fill),
	)
	.on_press(on_toggle);

	let mut list = column![];
	for option in &options {
		let is_selected = selected.as_ref().is_some_and(|s| s.id == option.id);
		let id = option.id.clone();
		let color = option.color;
		let name = option.name.clone();

		let item = button(
			row![
				color_circle(color),
				text(name).size(metrics.label()).width(Fill),
			]
			.spacing(8.0)
			.align_y(Alignment::Center)
			.padding([6.0, 10.0]),
		)
		.width(Fill)
		.on_press(on_select(id))
		.style(move |theme, status| {
			if is_selected {
				crate::ui::controls::primary(theme, status)
			} else {
				crate::ui::controls::ghost(theme, status)
			}
		});

		list = list.push(item);
		list = list.push(Space::new().height(4.0));
	}

	let scrollable = crate::ui::controls::scrollable(list)
		.width(Fill)
		.height(Length::Fixed(320.0));

	let card = container(scrollable)
		.padding(4.0)
		.width(Fill)
		.height(Length::Fixed(328.0))
		.style(|theme: &Theme| {
			let palette = theme.extended_palette();
			container::Style {
				background: Some(palette.background.weakest.color.into()),
				border: iced::Border {
					color: palette.background.strong.color,
					width: 1.0,
					radius: crate::ui::controls::RADIUS_CARD.into(),
				},
				..crate::ui::controls::surface(theme)
			}
		});

	container(stack![dismiss, iced::widget::opaque(card)])
		.width(Fill)
		.height(Length::Fixed(328.0))
		.into()
}

/// The calendar after `selected`, wrapping around at the end.
pub(super) fn next_calendar(
	options: &[CalendarChoice],
	selected: Option<&CalendarChoice>,
) -> Option<CalendarChoice> {
	if options.is_empty() {
		return None;
	}

	let index = selected
		.and_then(|current| options.iter().position(|option| option == current))
		.map_or(0, |index| (index + 1) % options.len());

	options.get(index).cloned()
}
