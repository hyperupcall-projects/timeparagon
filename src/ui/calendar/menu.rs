//! The calendars menu and its color pickers.

use super::*;
use iced::widget::column;

/// The palette shown under a source whose color is being changed. Six
/// swatches per row keeps it inside the sheet even at 200px.
pub(super) fn source_color_picker<'a>(
	id: &str,
	current: Option<Color>,
	is_tasks: bool,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let mut rows: Vec<Element<'a, Message>> = Vec::new();
	let palette = if is_tasks {
		colors::palette()
	} else {
		colors::calendar_palette()
	};
	let pick = |color| {
		if is_tasks {
			Message::Settings(SettingsMessage::TaskColorPicked(id.to_owned(), color))
		} else {
			Message::Settings(SettingsMessage::CalendarColorPicked(id.to_owned(), color))
		}
	};
	for chunk in palette.chunks(6) {
		let swatches: Vec<Element<'a, Message>> = chunk
			.iter()
			.map(|&color| swatch_button(color, current == Some(color), pick(Some(color))))
			.collect();
		rows.push(row(swatches).spacing(6.0).into());
	}

	let reset = button(
		text("Reset to default")
			.size(metrics.label() - 1.0)
			.center()
			.width(Fill),
	)
	.width(Fill)
	.on_press(pick(None))
	.style(crate::ui::controls::secondary);

	column![
		text(if is_tasks {
			"Task color"
		} else {
			"Calendar color in TimeParagon"
		})
		.size(metrics.label() - 1.0)
		.style(text::secondary),
		column(rows).spacing(6.0),
		reset,
	]
	.spacing(6.0)
	.width(Fill)
	.into()
}

/// The palette shown under a TimeParagon event kind whose color is being changed.
pub(super) fn time_paragon_color_picker<'a>(
	source: calendar::sources::TimeParagonSource,
	current: Option<Color>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let mut rows: Vec<Element<'a, Message>> = Vec::new();
	for chunk in colors::palette().chunks(6) {
		let swatches: Vec<Element<'a, Message>> = chunk
			.iter()
			.map(|&color| {
				swatch_button(
					color,
					current == Some(color),
					Message::Settings(SettingsMessage::TimeParagonColorPicked(
						source.id().to_owned(),
						Some(color),
					)),
				)
			})
			.collect();
		rows.push(row(swatches).spacing(6.0).into());
	}

	let reset = button(
		text("Reset to default")
			.size(metrics.label() - 1.0)
			.center()
			.width(Fill),
	)
	.width(Fill)
	.on_press(Message::Settings(SettingsMessage::TimeParagonColorPicked(
		source.id().to_owned(),
		None,
	)))
	.style(crate::ui::controls::secondary);

	column![
		text(format!("Color for {}", source.label()))
			.size(metrics.label() - 1.0)
			.style(text::secondary),
		column(rows).spacing(6.0),
		reset,
	]
	.spacing(6.0)
	.width(Fill)
	.into()
}

/// The swatch beside Travel: the gray its trips are drawn on the grid, with no
/// picker of its own.
fn travel_swatch<'a>() -> Element<'a, Message> {
	container(Space::new())
		.width(Length::Fixed(14.0))
		.height(Length::Fixed(14.0))
		.style(|_theme: &Theme| container::Style {
			background: Some(colors::TRAVEL.into()),
			border: iced::Border {
				color: Color::BLACK.scale_alpha(0.45),
				width: 1.0,
				radius: crate::ui::controls::RADIUS_SMALL.into(),
			},
			..container::Style::default()
		})
		.into()
}

/// A row of the Zoom section. `message` is `None` when there is no further to go.
fn zoom_button<'a>(
	label: &'a str,
	key: &'a str,
	message: Option<CalendarMessage>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	// Cmd on a Mac and Ctrl elsewhere, the same as the shortcut itself.
	let modifier = if cfg!(target_os = "macos") {
		"Cmd"
	} else {
		"Ctrl"
	};
	button(
		row![
			text(label).size(metrics.label()).width(Fill),
			text(format!("{modifier} {key}"))
				.size(metrics.label() - 1.0)
				.style(text::secondary),
		]
		.spacing(8.0),
	)
	.width(Fill)
	.on_press_maybe(message.map(Message::Calendar))
	.style(crate::ui::controls::secondary)
	.into()
}

fn zoom_section<'a>(state: &State, metrics: &Metrics) -> Element<'a, Message> {
	let height = state.calendar.hour_height;
	let zoom = |factor| CalendarMessage::CalendarZoomed {
		factor,
		anchor: None,
	};
	column![
		text("Zoom").size(metrics.label()).style(text::secondary),
		zoom_button(
			"Zoom in",
			"+",
			(height < calendar::MAX_HOUR_HEIGHT).then(|| zoom(calendar::ZOOM_STEP)),
			metrics,
		),
		zoom_button(
			"Reset zoom",
			"0",
			(height != calendar::DEFAULT_HOUR_HEIGHT).then_some(CalendarMessage::CalendarZoomReset),
			metrics,
		),
		zoom_button(
			"Zoom out",
			"-",
			(height > calendar::MIN_HOUR_HEIGHT).then(|| zoom(1.0 / calendar::ZOOM_STEP)),
			metrics,
		),
	]
	.spacing(metrics.spacing)
	.width(Fill)
	.into()
}

pub(super) fn menu_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let dismiss = Message::Calendar(CalendarMessage::CalendarMenuToggled);

	let mut panel = column![].spacing(metrics.spacing).width(Fill);

	let sources = state.calendar.display_sources(&state.config.calendar);
	for (heading, is_tasks) in [("Google Calendars", false), ("Google Task Lists", true)] {
		let mut section = column![text(heading).size(metrics.label()).style(text::secondary)]
			.spacing(metrics.spacing)
			.width(Fill);
		for source in sources.iter().filter(|source| {
			source.id != calendar::sleep::SOURCE_ID
				&& source.is_task_list() == is_tasks
				&& !source.is_app_calendar(&state.config.calendar)
		}) {
			let id = source.id.clone();
			let enabled = state.calendar.calendar_enabled(&id, &state.config.calendar);
			let editing = state.calendar.color_editor.as_deref() == Some(id.as_str());
			let toggle_id = id.clone();
			section = section.push(
				row![
					swatch_button(
						source.color,
						editing,
						Message::Settings(SettingsMessage::SourceColorEditorToggled(id.clone())),
					),
					text(source.name.clone()).size(metrics.label()).width(Fill),
					crate::ui::controls::toggler(enabled).on_toggle(move |value| {
						Message::Calendar(CalendarMessage::CalendarToggled(
							toggle_id.clone(),
							value,
						))
					}),
				]
				.spacing(8.0)
				.align_y(Alignment::Center),
			);
			if editing {
				section = section.push(source_color_picker(
					&id,
					if is_tasks {
						state.config.calendar.task_color(&id)
					} else {
						state
							.config
							.calendar
							.calendar_colors
							.get(&id)
							.map(|color| color.0)
					},
					is_tasks,
					metrics,
				));
			}
		}
		panel = panel.push(section);
	}

	let mut section = column![
		text("TimeParagon")
			.size(metrics.label())
			.style(text::secondary)
	]
	.spacing(metrics.spacing)
	.width(Fill);
	for source in calendar::sources::TimeParagonSource::ALL {
		let id = source.id().to_owned();
		let editing = state.calendar.color_editor.as_deref() == Some(id.as_str());
		section =
			section.push(
				row![
					swatch_button(
						source.color(&state.config.calendar),
						editing,
						Message::Settings(SettingsMessage::SourceColorEditorToggled(id.clone())),
					),
					text(source.label()).size(metrics.label()).width(Fill),
					crate::ui::controls::toggler(
						state
							.calendar
							.calendar_enabled(source.id(), &state.config.calendar)
					)
					.on_toggle(move |value| Message::Calendar(
						CalendarMessage::CalendarToggled(source.id().into(), value)
					)),
				]
				.spacing(8.0)
				.align_y(Alignment::Center),
			);
		if editing {
			section = section.push(time_paragon_color_picker(
				source,
				source.custom_color(&state.config.calendar),
				metrics,
			));
		}
	}
	// Travel is not an event kind, so its row has no color picker: the swatch
	// only shows the gray its trips are drawn in. It is toggled here with the
	// other TimeParagon sources, and off means no route is ever worked out.
	section = section.push(
		row![
			travel_swatch(),
			text("Travel").size(metrics.label()).width(Fill),
			crate::ui::controls::toggler(state.travel.show)
				.on_toggle(|_| Message::Travel(crate::travel::Action::Toggle)),
		]
		.spacing(8.0)
		.align_y(Alignment::Center),
	);
	panel = panel.push(section);
	panel = panel.push(zoom_section(state, metrics));
	if state.calendar.calendars.is_empty() {
		panel = panel.push(
			text(if state.calendar.is_signed_in() {
				"No calendars yet. Sync to load them."
			} else {
				"Connect Google Calendar in Settings."
			})
			.size(metrics.label())
			.style(text::secondary),
		);
	}

	view_sheet(state, metrics, ViewTab::Calendars, panel, dismiss)
}

/// The two tabs of the View sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ViewTab {
	Calendars,
	Filter,
}

/// Wide enough for the filter's time control, and narrow enough on a large
/// window to leave the view beside it in sight, updating as it changes.
const VIEW_SHEET_WIDTH: f32 = 380.0;

/// The View button: the way into the Calendars and Filter tabs. It stays lit
/// while a filter is on, because the filter stays on after the sheet closes.
pub(super) fn view_button<'a>(state: &State, metrics: &Metrics) -> Element<'a, Message> {
	let search = &state.calendar.search;
	let open = state.calendar.menu_open || search.open;
	// Each tab's own toggle closes it; with neither open, the sheet opens on
	// the Calendars tab.
	let message = if search.open {
		Message::Calendar(CalendarMessage::CalendarSearch(
			calendar::search::Action::Toggle,
		))
	} else {
		Message::Calendar(CalendarMessage::CalendarMenuToggled)
	};
	button(text("View").size(metrics.label()).center())
		.on_press(message)
		.style(if open || search.is_modified() {
			crate::ui::controls::primary
		} else {
			crate::ui::controls::secondary
		})
		.into()
}

/// The Actions button, the way into Sync now and Set Alarm.
pub(super) fn actions_button<'a>(state: &State, metrics: &Metrics) -> Element<'a, Message> {
	button(text("Actions").size(metrics.label()).center())
		.on_press(Message::Calendar(CalendarMessage::CalendarActionsToggled))
		.style(if state.calendar.actions_open || state.alarms.mode {
			crate::ui::controls::primary
		} else {
			crate::ui::controls::secondary
		})
		.into()
}

fn tab_button<'a>(
	label: &'a str,
	selected: bool,
	message: Message,
	metrics: &Metrics,
) -> Element<'a, Message> {
	button(text(label).size(metrics.label()).center().width(Fill))
		.width(Fill)
		.on_press_maybe((!selected).then_some(message))
		.style(if selected {
			crate::ui::controls::primary
		} else {
			crate::ui::controls::secondary
		})
		.into()
}

/// The View sheet: the Calendars and Filter tabs over `panel`, the content
/// of the tab in view, in a card that hangs below the header at the left.
pub(super) fn view_sheet<'a>(
	state: &'a State,
	metrics: &Metrics,
	tab: ViewTab,
	panel: iced::widget::Column<'a, Message>,
	dismiss: Message,
) -> Element<'a, Message> {
	let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill)).on_press(dismiss);

	let tabs = row![
		tab_button(
			"Calendars",
			tab == ViewTab::Calendars,
			Message::Calendar(CalendarMessage::CalendarMenuToggled),
			metrics,
		),
		tab_button(
			"Filter",
			tab == ViewTab::Filter,
			Message::Calendar(CalendarMessage::CalendarSearch(
				calendar::search::Action::Toggle,
			)),
			metrics,
		),
	]
	.spacing(metrics.spacing)
	.width(Fill);

	let body = crate::ui::controls::scrollable(container(panel).padding(iced::Padding {
		right: 14.0,
		..iced::Padding::ZERO
	}))
	.height(Length::Shrink);

	let card = container(column![tabs, body].spacing(metrics.spacing))
		.max_height(
			(state.main_size.height - header_offset(state, metrics) - metrics.padding * 2.0)
				.max(120.0),
		)
		.max_width(VIEW_SHEET_WIDTH)
		.padding(metrics.padding)
		.width(Fill)
		.style(crate::ui::controls::surface);

	let aligned = column![
		// Keep the header row tappable; the sheet starts below it.
		Space::new().height(header_offset(state, metrics)),
		container(iced::widget::opaque(card))
			.padding(metrics.padding)
			.width(Fill),
	]
	.width(Fill);

	stack![dismiss, aligned].into()
}

/// How wide the Actions popup is: room for its labels and no more.
const ACTIONS_WIDTH: f32 = 220.0;

/// The Actions popup: Sync now above Set Alarm, as alike as two buttons.
/// `alarm` is false where there is no grid to set an alarm on.
pub(super) fn actions_overlay<'a>(
	state: &'a State,
	metrics: &Metrics,
	alarm: bool,
) -> Element<'a, Message> {
	let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill))
		.on_press(Message::Calendar(CalendarMessage::CalendarActionsToggled));

	let sync_label = if state.calendar.is_busy() {
		"Syncing…"
	} else {
		"Sync now"
	};
	let sync = button(text(sync_label).size(metrics.label()).center().width(Fill))
		.width(Fill)
		.on_press_maybe(
			(state.calendar.is_signed_in() && !state.calendar.is_busy())
				.then_some(Message::Calendar(CalendarMessage::CalendarSyncRequested)),
		)
		.style(crate::ui::controls::secondary);

	let mut panel = column![sync].spacing(metrics.spacing).width(Fill);
	if alarm {
		// Alarm setting mode turns the day view into a place to click a time,
		// so the button that enters it is lit for as long as it lasts.
		panel = panel.push(
			button(text("Set Alarm").size(metrics.label()).center().width(Fill))
				.width(Fill)
				.on_press(Message::Alarms(crate::alarms::Action::ModeToggled))
				.style(if state.alarms.mode {
					crate::ui::controls::primary
				} else {
					crate::ui::controls::secondary
				}),
		);
	}

	let card = container(panel)
		.max_width(ACTIONS_WIDTH)
		.padding(metrics.padding)
		.width(Fill)
		.style(crate::ui::controls::surface);

	let aligned = column![
		Space::new().height(header_offset(state, metrics)),
		container(iced::widget::opaque(card))
			.padding(metrics.padding)
			.width(Fill),
	]
	.width(Fill);

	stack![dismiss, aligned].into()
}
