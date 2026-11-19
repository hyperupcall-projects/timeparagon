//! The settings tab and the timer-hits-zero menu.

use crate::calendar::first_day::FirstDayOfWeek;
use crate::calendar::time_format::TimeFormat;
use iced::widget::{button, column, container, row, text};
use iced::{Center, Element, Fill, Length};

use crate::app::{BorderLayer, LayoutField, Message, State};
use crate::app::{CalendarMessage, SettingsMessage};
use crate::calendar;
use crate::config;
use crate::integrations::accounts;

use super::controls::with_info;
use super::layout::{Breakpoint, Metrics};
pub(super) use timeparagon_core::alarms::repeat_label;

/// Room left for the scrollbar, which is drawn over the content rather than
/// beside it. Without this the right edge of every control sits underneath
/// it, which at 200px means visibly clipped labels.
const SCROLLBAR_GUTTER: f32 = 14.0;

const SHORTCUTS_INFO: &str = "Global shortcuts. Each action can have several, and any of them triggers it. Combine Ctrl, Alt, Shift or Meta with a letter, number or named key (e.g. Space or F2).";
const SYNC_URL_INFO: &str = "Leave blank to disable sync.";
const NOTES_INFO: &str = "To see the same notes on mobile, sync this folder and choose its copy there. Colors and connections use database sync.";
const DOCK_RESERVATION_INFO: &str =
	"Enabling prevents maximized windows from overlapping the dock.";
const PANEL_RESERVATION_INFO: &str =
	"Enabling prevents maximized windows from overlapping the panel.";
const TRANSITION_INFO: &str = "Customize the app's transition effects for KDE.";
const CALENDAR_ORDER_INFO: &str =
	"Top to bottom sets left to right when events overlap. App-managed task lists are omitted.";
const SLEEP_INFO: &str = "Each row is the night ending on that weekday. Bedtimes later than wake-up are on the preceding evening. Changes apply from today through four weeks ahead.";
const GOOGLE_OAUTH_INFO: &str = "Create a Desktop OAuth client in Google Cloud; enable the Calendar API, Tasks API, and the Gmail API. Allow http://127.0.0.1 as a redirect.";
const TASKS_INFO: &str = "A task cannot leave the Inbox until every required field is filled in.";
const DISTRACTION_INFO: &str = "\"I got distracted\" in a time block asks which of these it was, then records it on that habit.";
const GMAIL_INFO: &str = "Use to fetch the title and description of Gmail emails from their URL, and to list contacts under People.";

/// A calendar offered in the default-calendar picker. The name is shown to
/// the user; the id is what gets saved as the default.
#[derive(Clone, PartialEq)]
struct CalendarOption {
	id: String,
	name: String,
}

impl std::fmt::Display for CalendarOption {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.name)
	}
}

#[derive(Clone, PartialEq)]
struct NumberOption {
	value: u32,
	label: String,
}

impl std::fmt::Display for NumberOption {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.label)
	}
}

/// Show every value in a dropdown, including in narrow panels.
fn number_dropdown<'a>(
	range: std::ops::RangeInclusive<u32>,
	selected: u32,
	label: impl Fn(u32) -> String,
	on_select: impl Fn(u32) -> Message + 'a,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let options: Vec<_> = range
		.map(|value| NumberOption {
			value,
			label: label(value),
		})
		.collect();
	let selected = options
		.iter()
		.find(|option| option.value == selected)
		.cloned();
	super::controls::pick_list(options, selected, move |option| on_select(option.value))
		.text_size(metrics.label())
		.width(Fill)
		.into()
}

fn days_shown<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	choice(
		"Days shown in week view",
		config::WeekRange::ALL.to_vec(),
		Some(state.config.calendar.week_range()),
		|range| Message::Settings(SettingsMessage::CalendarWeekRangeChanged(range)),
		metrics,
	)
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	// There is one settings page rather than one per tab: everything is set
	// from the same place.
	let content = column![
		section("Preferences", preferences(state, metrics), metrics),
		section(
			"Desktop appearance",
			desktop_appearance(state, metrics),
			metrics
		),
		section(
			"Reminders",
			super::reminders::settings_section(state, metrics),
			metrics
		),
		section(
			"Database sync",
			column![
				with_info(
					text("Sync server URL").size(metrics.label()).width(Fill),
					SYNC_URL_INFO,
					metrics.label(),
				),
				crate::ui::controls::text_input(
					"https://sync.example.com",
					&state.config.database_sync.url
				)
				.on_input(|url| Message::Settings(SettingsMessage::DatabaseSyncUrlChanged(url)))
				.size(metrics.label()),
				text(crate::database_sync::status()).size(metrics.label()),
			]
			.spacing(metrics.spacing)
			.into(),
			metrics
		),
		section_with_info(
			"Notes",
			NOTES_INFO,
			column![
				text("Notes folder").size(metrics.label()),
				super::controls::text_input("~/Documents/Notes", &state.notes.directory_input)
					.on_input(
						|value| Message::Settings(SettingsMessage::NotesDirectoryChanged(value))
					)
					.on_submit(Message::Settings(SettingsMessage::NotesDirectoryApply))
					.size(metrics.label()),
				row![
					button(text("Choose folder").size(metrics.label()))
						.on_press(Message::Settings(SettingsMessage::NotesDirectoryBrowse)),
					button(text("Apply").size(metrics.label()))
						.on_press(Message::Settings(SettingsMessage::NotesDirectoryApply)),
				]
				.spacing(metrics.spacing),
			]
			.spacing(metrics.spacing)
			.into(),
			metrics
		),
		section_with_info(
			"Keyboard shortcuts",
			SHORTCUTS_INFO,
			shortcuts(state, metrics),
			metrics
		),
		section_with_info(
			"Calendar view order",
			CALENDAR_ORDER_INFO,
			calendar_order(state, metrics),
			metrics
		),
		section("Google Calendar", calendar(state, metrics), metrics),
		section_with_info("Tasks", TASKS_INFO, tasks(state, metrics), metrics),
		section_with_info(
			"Distraction habits",
			DISTRACTION_INFO,
			distraction_habits(state, metrics),
			metrics
		),
		section_with_info("Sleep", SLEEP_INFO, sleep(state, metrics), metrics),
		section(
			"Travel",
			super::travel::settings_section(state, metrics),
			metrics
		),
		section("AI", crate::voice::settings(state, metrics), metrics),
		section(
			"Account integrations",
			integrations(state, metrics),
			metrics
		),
		section("Application", application(metrics), metrics),
	]
	.spacing(metrics.spacing)
	.padding(iced::Padding {
		top: 2.0,
		left: 3.0,
		right: SCROLLBAR_GUTTER,
		bottom: 8.0,
	})
	.width(Fill);

	column![
		super::controls::section_heading(
			"Settings",
			None,
			if metrics.breakpoint.is_compact() {
				16.0
			} else {
				21.0
			}
		),
		crate::ui::controls::scrollable(content)
			.height(Fill)
			.width(Fill),
	]
	.spacing(metrics.spacing)
	.padding(metrics.padding)
	.width(Fill)
	.height(Fill)
	.into()
}

fn heading<'a>(label: &'a str, metrics: &Metrics) -> Element<'a, Message> {
	super::controls::section_heading(label, None, metrics.label() + 1.0).into()
}

fn section<'a>(
	label: &'a str,
	body: Element<'a, Message>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	card(heading(label, metrics), body, metrics)
}

fn section_with_info<'a>(
	label: &'a str,
	info: &'a str,
	body: Element<'a, Message>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	card(
		super::controls::section_heading(label, None, metrics.label() + 1.0)
			.push(super::controls::info_tip(info, metrics.label()))
			.into(),
		body,
		metrics,
	)
}

fn card<'a>(
	heading: Element<'a, Message>,
	body: Element<'a, Message>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	container(column![heading, body].spacing(metrics.spacing))
		.padding(metrics.padding)
		.width(Fill)
		.style(super::controls::card_surface)
		.into()
}

/// A labelled row that stacks vertically once there is no room for two
/// columns, which is what keeps every control reachable at 200px.
///
/// The label is owned rather than borrowed because most callers build it
/// from the value they are showing.
pub(super) fn field<'a>(
	label: impl Into<String>,
	control: Element<'a, Message>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	labelled(text(label.into()).size(metrics.label()), control, metrics)
}

/// A [`field`] whose label carries an info symbol.
pub(super) fn field_with_info<'a>(
	label: impl Into<String>,
	info: &'a str,
	control: Element<'a, Message>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let caption = text(label.into()).size(metrics.label()).width(Fill);
	labelled(with_info(caption, info, metrics.label()), control, metrics)
}

fn labelled<'a>(
	caption: impl Into<Element<'a, Message>>,
	control: Element<'a, Message>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let caption = container(caption);

	if metrics.breakpoint.is_compact() {
		column![caption, control].spacing(2.0).width(Fill).into()
	} else {
		row![caption.width(Length::FillPortion(2)), control]
			.spacing(metrics.spacing)
			.align_y(Center)
			.into()
	}
}

/// A chooser that survives a narrow window.
///
/// Dropdown overlays need more width than they have in compact mode, so there
/// the control becomes a button that cycles to the next option on each tap.
pub(super) fn choice<'a, T>(
	label: &'a str,
	options: Vec<T>,
	selected: Option<T>,
	on_select: impl Fn(T) -> Message + 'a,
	metrics: &Metrics,
) -> Element<'a, Message>
where
	T: ToString + PartialEq + Clone + 'a,
{
	if metrics.breakpoint.is_compact() {
		let shown = selected
			.as_ref()
			.map(ToString::to_string)
			.unwrap_or_else(|| "-".to_owned());

		let next = next_option(&options, selected.as_ref());

		let control = button(text(shown).size(metrics.label()).center().width(Fill)).width(Fill);

		let control = match next {
			Some(value) => control.on_press(on_select(value)),
			None => control,
		};

		field(
			label,
			control.style(crate::ui::controls::secondary).into(),
			metrics,
		)
	} else {
		field(
			label,
			crate::ui::controls::pick_list(options, selected, on_select)
				.text_size(metrics.label())
				.width(Fill)
				.into(),
			metrics,
		)
	}
}

/// The option after `selected`, wrapping around at the end.
fn next_option<T: PartialEq + Clone>(options: &[T], selected: Option<&T>) -> Option<T> {
	if options.is_empty() {
		return None;
	}

	let index = selected
		.and_then(|current| options.iter().position(|option| option == current))
		.map_or(0, |index| (index + 1) % options.len());

	options.get(index).cloned()
}

fn preferences<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let settings = &state.config.calendar;
	container(
		column![
			choice(
				"Time format",
				TimeFormat::ALL.to_vec(),
				Some(state.config.appearance.time_format),
				|v0| Message::Settings(SettingsMessage::TimeFormatSelected(v0)),
				metrics,
			),
			choice(
				"First day of the week",
				FirstDayOfWeek::ALL.to_vec(),
				Some(settings.first_day_of_week),
				|v0| Message::Settings(SettingsMessage::FirstDayOfWeekSelected(v0)),
				metrics,
			),
			days_shown(state, metrics),
			field(
				day_end_label(
					settings.day_start_hour(),
					state.config.appearance.time_format,
				),
				number_dropdown(
					0..=config::CalendarSettings::MAX_DAY_END_HOUR,
					settings.day_start_hour(),
					|hour| {
						if hour == 0 {
							"Midnight".to_owned()
						} else {
							calendar::time_format::hour(hour, state.config.appearance.time_format)
						}
					},
					|v0| Message::Settings(SettingsMessage::CalendarDayEndChanged(v0)),
					metrics,
				),
				metrics,
			),
			choice(
				"Distance unit",
				config::DistanceUnit::ALL.to_vec(),
				Some(state.config.appearance.distance_unit),
				|unit| Message::Settings(SettingsMessage::DistanceUnitSelected(unit)),
				metrics,
			),
			choice(
				"Theme",
				crate::theme::picker_names(&state.catalog),
				Some(crate::theme::selected_name(
					&state.catalog,
					&state.config.appearance.theme,
				)),
				|v0| Message::Settings(SettingsMessage::ThemeSelected(v0)),
				metrics,
			),
			container(switch(
				"Dock hour lines",
				state.config.views.dock_hour_lines,
				|v0| Message::Settings(SettingsMessage::DockHourLinesToggled(v0)),
				metrics,
			))
			.id("dock-hour-lines")
			.width(Fill),
		]
		.spacing(metrics.spacing),
	)
	.id("time-format")
	.width(Fill)
	.into()
}

fn desktop_appearance<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	column![
		subsection("Reserve space", reserve_space(state, metrics), metrics),
		subsection_with_info(
			"Desktop transition",
			TRANSITION_INFO,
			windowing(state, metrics),
			metrics
		),
		subsection("Borders", borders(state, metrics), metrics),
		subsection("Dock", dock(state, metrics), metrics),
		subsection("Side panel", panel(state, metrics), metrics),
	]
	.spacing(metrics.spacing * 2.0)
	.into()
}

fn subsection<'a>(
	label: &'a str,
	body: Element<'a, Message>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	column![subsection_title(label, metrics), body]
		.spacing(metrics.spacing)
		.width(Fill)
		.into()
}

fn subsection_with_info<'a>(
	label: &'a str,
	info: &'a str,
	body: Element<'a, Message>,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let title = subsection_title(label, metrics).width(Fill);
	column![with_info(title, info, metrics.label()), body]
		.spacing(metrics.spacing)
		.width(Fill)
		.into()
}

fn subsection_title<'a>(label: &'a str, metrics: &Metrics) -> iced::widget::Text<'a> {
	text(label).size(metrics.label() + 1.0).font(iced::Font {
		weight: iced::font::Weight::Bold,
		..iced::Font::DEFAULT
	})
}

fn reserve_space<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let views = &state.config.views;

	column![
		with_info(
			container(switch(
				"Reserve screen space for the dock",
				views.dock_reserve_space,
				|v0| Message::Settings(SettingsMessage::DockReserveSpaceToggled(v0)),
				metrics,
			))
			.id("dock-reservation")
			.width(Fill),
			DOCK_RESERVATION_INFO,
			metrics.label(),
		),
		with_info(
			container(switch(
				"Reserve screen space for the side panel",
				views.panel_reserve_space,
				|v0| Message::Settings(SettingsMessage::PanelReserveSpaceToggled(v0)),
				metrics,
			))
			.id("panel-reservation")
			.width(Fill),
			PANEL_RESERVATION_INFO,
			metrics.label(),
		),
	]
	.spacing(metrics.spacing)
	.into()
}

fn borders<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let views = &state.config.views;
	column![
		layout_number(
			"Outer border depth (px, 0–5)",
			LayoutField::PanelBorderOuterWidth,
			state,
			metrics
		),
		panel_color_input(
			"Outer border color",
			BorderLayer::Outer,
			views.panel_border_outer_color,
			state,
			metrics
		),
		layout_number(
			"Inner border depth (px, 0–5)",
			LayoutField::PanelBorderInnerWidth,
			state,
			metrics
		),
		panel_color_input(
			"Inner border color",
			BorderLayer::Inner,
			views.panel_border_inner_color,
			state,
			metrics
		),
	]
	.spacing(metrics.spacing)
	.into()
}

fn dock<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	column![
		choice(
			"Screen edge",
			crate::config::DockSide::ALL.to_vec(),
			Some(state.config.views.dock_side),
			|v0| Message::Settings(SettingsMessage::DockSideSelected(v0)),
			metrics,
		),
		layout_number(
			"Corner radius (px, 0–30)",
			LayoutField::DockBorderRadius,
			state,
			metrics
		),
		layout_number(
			"Dock width (px, 2–40)",
			LayoutField::DockWidth,
			state,
			metrics
		),
		layout_number(
			"Left margin (px, 0–100)",
			LayoutField::DockLeftMargin,
			state,
			metrics,
		),
		layout_number(
			"Right margin (px, 0–100)",
			LayoutField::DockRightMargin,
			state,
			metrics,
		),
	]
	.spacing(metrics.spacing)
	.into()
}

fn panel<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	column![
		layout_number(
			"Screen margin (px, 0–30)",
			LayoutField::ScreenMargin,
			state,
			metrics,
		),
		layout_number(
			"Corner radius (px, 0–30)",
			LayoutField::PanelBorderRadius,
			state,
			metrics
		),
	]
	.spacing(metrics.spacing)
	.into()
}

fn panel_color_input<'a>(
	label: &str,
	layer: BorderLayer,
	current: crate::hex::HexColor,
	state: &'a State,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let input = state.panel_border_input.as_ref();
	border_color_input(
		label,
		"panel-border-color",
		layer,
		current,
		input,
		|v0, v1| Message::Settings(SettingsMessage::PanelBorderColorInput(v0, v1)),
		|v0| Message::Settings(SettingsMessage::PanelBorderColorSubmitted(v0)),
		metrics,
	)
}

#[allow(clippy::too_many_arguments)]
fn border_color_input<'a>(
	label: &str,
	id_prefix: &str,
	layer: BorderLayer,
	current: crate::hex::HexColor,
	edited: Option<&(BorderLayer, String)>,
	on_input: impl Fn(BorderLayer, String) -> Message + 'a,
	on_submit: impl Fn(BorderLayer) -> Message + 'a,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let shown = edited
		.filter(|(edited, _)| *edited == layer)
		.map_or_else(|| current.to_string(), |(_, input)| input.clone());
	let submitted = on_submit(layer);
	let input = crate::ui::controls::text_input("#RRGGBB", &shown)
		.id(format!(
			"{id_prefix}-{}",
			match layer {
				BorderLayer::Outer => 0,
				BorderLayer::Inner => 1,
			}
		))
		.size(metrics.label())
		.width(Fill)
		.on_input(move |input| on_input(layer, input))
		.on_submit(submitted);

	field(label, input.into(), metrics)
}

fn application<'a>(metrics: &Metrics) -> Element<'a, Message> {
	column![
		crate::ui::controls::action_button(
			text("Full sync").size(metrics.label()).center().width(Fill),
		)
		.width(Fill)
		.on_press(Message::Settings(SettingsMessage::FullSyncRequested)),
		restart(metrics),
	]
	.spacing(metrics.spacing)
	.into()
}

fn restart<'a>(metrics: &Metrics) -> Element<'a, Message> {
	column![
		crate::ui::controls::action_button(
			text("Restart and load the latest version")
				.size(metrics.label())
				.center()
				.width(Fill),
		)
		.width(Fill)
		.on_press(Message::Settings(SettingsMessage::RestartRequested))
	]
	.spacing(metrics.spacing)
	.into()
}

fn windowing<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let views = &state.config.views;
	let mut section = column![choice(
		"When the window appears",
		crate::config::WindowAnimation::ALL.to_vec(),
		Some(views.window_animation),
		|v0| Message::Settings(SettingsMessage::WindowAnimationSelected(v0)),
		metrics,
	)]
	.spacing(metrics.spacing);
	if views.window_animation == crate::config::WindowAnimation::FadeIn {
		section = section.push(layout_number(
			"Fade duration (ms, 0–5000)",
			LayoutField::FadeDuration,
			state,
			metrics,
		));
	}
	section.into()
}

fn layout_number<'a>(
	label: &str,
	kind: LayoutField,
	state: &'a State,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let value = kind.value(state);
	let step = match kind {
		LayoutField::CalendarWidth
		| LayoutField::CalendarWeekWidth
		| LayoutField::CalendarMonthWidth
		| LayoutField::TimeBlockWidth
		| LayoutField::DetailWidth => 10.0,
		LayoutField::ScreenMargin
		| LayoutField::BorderRadius
		| LayoutField::DockBorderRadius
		| LayoutField::PanelBorderRadius
		| LayoutField::DockWidth
		| LayoutField::DockLeftMargin
		| LayoutField::DockRightMargin
		| LayoutField::DockBorderOuterWidth
		| LayoutField::DockBorderInnerWidth
		| LayoutField::PanelBorderOuterWidth
		| LayoutField::PanelBorderInnerWidth => 1.0,
		LayoutField::FadeDuration => 10.0,
	};
	let (min, max) = kind.bounds();
	let shown = state
		.view_layout_input
		.as_ref()
		.filter(|(field, _)| *field == kind)
		.map_or_else(|| format!("{value:.0}"), |(_, input)| input.clone());
	let input = crate::ui::controls::text_input("", &shown)
		.id(format!("layout-{kind:?}"))
		.size(metrics.label())
		.on_input(move |input| {
			Message::Settings(SettingsMessage::ViewLayoutInputChanged(kind, input))
		})
		.on_submit(Message::Settings(
			SettingsMessage::ViewLayoutInputSubmitted(kind),
		));
	field(
		label,
		crate::ui::controls::stepper(
			input,
			metrics.label(),
			(value < max).then(|| kind.changed((value + step).min(max))),
			(value > min).then(|| kind.changed((value - step).max(min))),
		),
		metrics,
	)
}

fn shortcuts<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let mut section = column![].spacing(metrics.spacing);
	for action in crate::panel::Action::ALL {
		section = section.push(shortcut_action(state, action, metrics));
	}
	indent(section.into(), metrics)
}

fn shortcut_action<'a>(
	state: &'a State,
	action: crate::panel::Action,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let editor = &state.shortcut_editors[action.index()];
	let bound = state.config.panel.shortcuts(action);

	let name = row![
		text(action.label()).size(metrics.label()),
		super::controls::info_tip(action.description(), metrics.label()),
	]
	.spacing(4)
	.align_y(Center);
	let mut current = row![name].spacing(6).align_y(Center);

	if bound.is_empty() {
		current = current.push(
			text("No shortcut")
				.size(metrics.label())
				.style(text::secondary),
		);
	}
	for (index, shortcut) in bound.iter().enumerate() {
		current = current.push(shortcut_chip(action, index, shortcut, editor.busy, metrics));
	}

	let mut input =
		crate::ui::controls::text_input("e.g. Meta+F", &editor.input).size(metrics.label());
	let mut add = crate::ui::controls::action_button(
		text(if editor.busy { "Applying…" } else { "Add" }).size(metrics.label()),
	);
	if !editor.busy {
		input = input
			.on_input(move |value| {
				Message::Settings(SettingsMessage::ShortcutInputChanged(action, value))
			})
			.on_submit(Message::Settings(SettingsMessage::ShortcutAdded(action)));
		if !editor.input.trim().is_empty() {
			add = add.on_press(Message::Settings(SettingsMessage::ShortcutAdded(action)));
		}
	}
	let adder = row![input, add].spacing(4).align_y(Center);

	// Side by side once there is room, so each action takes a single line.
	let entry: Element<'a, Message> = if metrics.breakpoint == Breakpoint::Wide {
		row![
			current
				.width(Length::FillPortion(3))
				.wrap()
				.vertical_spacing(4),
			adder.width(Length::FillPortion(2)),
		]
		.spacing(metrics.spacing)
		.align_y(Center)
		.into()
	} else {
		column![current.wrap().vertical_spacing(4), adder]
			.spacing(4)
			.into()
	};

	let mut section = column![entry].spacing(4);
	if let Some(result) = &editor.result {
		section = section.push(match result {
			Ok(message) => text(message).size(metrics.label()).style(text::success),
			Err(error) => text(error).size(metrics.label()).style(text::danger),
		});
	}
	section.into()
}

/// A bound shortcut as a pill, with its remove button inside.
fn shortcut_chip<'a>(
	action: crate::panel::Action,
	index: usize,
	shortcut: &'a str,
	busy: bool,
	metrics: &Metrics,
) -> Element<'a, Message> {
	let remove = iced::widget::tooltip(
		button(text("×").size(metrics.label()).center())
			.padding([0, 4])
			.style(crate::ui::controls::ghost)
			.on_press_maybe((!busy).then_some(Message::Settings(
				SettingsMessage::ShortcutRemoved(action, index),
			))),
		text("Remove").size(12),
		iced::widget::tooltip::Position::Bottom,
	)
	.style(crate::ui::controls::surface);

	container(
		row![text(shortcut).size(metrics.label()), remove]
			.spacing(2)
			.align_y(Center),
	)
	.padding(iced::Padding {
		top: 1.0,
		right: 2.0,
		bottom: 1.0,
		left: 8.0,
	})
	.style(crate::ui::controls::tag)
	.into()
}

fn calendar_order<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let sources = state.calendar.display_sources(&state.config.calendar);
	let mut section = column![].spacing(metrics.spacing).width(Fill);
	for (index, source) in sources.iter().enumerate() {
		let mut up = button(text("Up").size(metrics.label())).style(crate::ui::controls::secondary);
		let mut down =
			button(text("Down").size(metrics.label())).style(crate::ui::controls::secondary);
		if index > 0 {
			up = up.on_press(Message::Calendar(CalendarMessage::CalendarSourceMoved(
				source.id.clone(),
				true,
			)));
		}
		if index + 1 < sources.len() {
			down = down.on_press(Message::Calendar(CalendarMessage::CalendarSourceMoved(
				source.id.clone(),
				false,
			)));
		}
		let controls = row![up, down].spacing(4.0);
		let entry: Element<'a, Message> = if metrics.breakpoint.is_compact() {
			column![text(source.name.clone()).size(metrics.label()), controls]
				.spacing(2.0)
				.width(Fill)
				.into()
		} else {
			row![
				text(source.name.clone()).size(metrics.label()).width(Fill),
				controls
			]
			.spacing(metrics.spacing)
			.align_y(Center)
			.into()
		};
		section = section.push(entry);
	}
	indent(section.into(), metrics)
}

fn sleep<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let settings = &state.config.calendar;
	let time_format = state.config.appearance.time_format;

	let mut section = column![].spacing(metrics.spacing);
	for day in settings.first_day_of_week.weekdays() {
		let times = settings.sleep_schedule[day.num_days_from_monday() as usize];
		let time_field = |label: &'static str, bedtime: bool, value: chrono::NaiveTime| {
			column![
				crate::ui::controls::field_label(label, metrics.label()),
				crate::ui::controls::time(
					time_format,
					&value.format("%H:%M").to_string(),
					true,
					metrics.label(),
					move |value| Message::Settings(SettingsMessage::SleepTimeChanged(
						day, bedtime, value
					)),
				),
			]
			.spacing(4)
			.width(Fill)
		};
		section = section.push(
			column![
				text(day.to_string()).size(metrics.label()),
				row![
					time_field("Bedtime", true, times.bedtime),
					time_field("Wake-up", false, times.wakeup)
				]
				.spacing(metrics.spacing),
			]
			.spacing(4),
		);
	}
	section.into()
}

fn calendar<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let settings = &state.config.calendar;
	let connected = state.calendar.is_signed_in();

	let mut section = column![
		field_with_info(
			"OAuth client ID",
			GOOGLE_OAUTH_INFO,
			crate::ui::controls::text_input("Desktop client ID", &settings.client_id)
				.on_input(|v0| Message::Settings(SettingsMessage::GoogleClientIdChanged(v0)))
				.size(metrics.label())
				.into(),
			metrics,
		),
		field(
			"Client secret",
			crate::ui::controls::text_input("optional", &settings.client_secret)
				.on_input(|v0| Message::Settings(SettingsMessage::GoogleClientSecretChanged(v0)))
				.secure(true)
				.size(metrics.label())
				.into(),
			metrics,
		)
	]
	.spacing(metrics.spacing);

	let action = if connected {
		let mut disconnect = button(
			text("Disconnect")
				.size(metrics.label())
				.center()
				.width(Fill),
		)
		.width(Fill)
		.style(crate::ui::controls::secondary);
		if !state.calendar.is_busy() {
			disconnect = disconnect.on_press(Message::Settings(
				SettingsMessage::GoogleDisconnectRequested,
			));
		}
		disconnect
	} else {
		let mut connect = button(
			text("Connect Google Calendar")
				.size(metrics.label())
				.center()
				.width(Fill),
		)
		.width(Fill)
		.style(crate::ui::controls::primary);
		if !state.calendar.is_busy() {
			connect = connect.on_press(Message::Settings(SettingsMessage::GoogleConnectRequested));
		}
		connect
	};

	section = section.push(action);

	let resync_label = if state.calendar.is_busy() {
		"Syncing…"
	} else {
		"Resync calendar and tasks data"
	};
	let mut resync = button(
		text(resync_label)
			.size(metrics.label())
			.center()
			.width(Fill),
	)
	.width(Fill)
	.style(crate::ui::controls::secondary);
	if connected && !state.calendar.is_busy() {
		resync = resync.on_press(Message::Calendar(CalendarMessage::CalendarResyncRequested));
	}
	section = section.push(resync);

	// New plain events use regular Google calendars; task lists and the
	// calendar reserved for TimeParagon's own kinds are excluded.
	let options: Vec<calendar::Calendar> = state
		.calendar
		.calendars
		.iter()
		.filter(|calendar| !calendar.is_task_list() && !calendar.is_app_calendar(settings))
		.cloned()
		.collect();

	let selected = options
		.iter()
		.find(|calendar| settings.default_calendar.as_deref() == Some(calendar.id.as_str()))
		.cloned();

	let option_map: Vec<CalendarOption> = options
		.iter()
		.map(|calendar| CalendarOption {
			id: calendar.id.clone(),
			name: calendar.name.clone(),
		})
		.collect();

	let selected_option = selected.map(|calendar| CalendarOption {
		id: calendar.id.clone(),
		name: calendar.name.clone(),
	});

	section = section.push(field(
		"Default calendar for new events",
		crate::ui::controls::pick_list(option_map, selected_option, |option: CalendarOption| {
			Message::Settings(SettingsMessage::DefaultCalendarSelected(option.id))
		})
		.text_size(metrics.label())
		.width(Fill)
		.into(),
		metrics,
	));

	indent(section.into(), metrics)
}

/// Keep a single named default choice, with an empty ID so destinations still
/// find or create the default list when tasks are filed.
fn task_list_options(
	task_lists: &[CalendarOption],
	default_name: &str,
	configured: Option<&str>,
) -> (Vec<CalendarOption>, Option<CalendarOption>) {
	let default = CalendarOption {
		id: String::new(),
		name: default_name.to_owned(),
	};
	let uses_default = configured.is_none_or(str::is_empty)
		|| task_lists
			.iter()
			.any(|list| Some(list.id.as_str()) == configured && list.name == default_name);
	let mut options = vec![default.clone()];
	options.extend(
		task_lists
			.iter()
			.filter(|list| list.name != default_name)
			.cloned(),
	);
	let selected = if uses_default {
		Some(default)
	} else {
		options
			.iter()
			.find(|list| Some(list.id.as_str()) == configured)
			.cloned()
	};
	(options, selected)
}

/// The Google Tasks lists the app files tasks into. Each task list is chosen
/// here: where scheduled tasks, backlog, triaged tasks and new tasks land.
fn google_tasks<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let settings = &state.config.calendar;
	let task_lists: Vec<CalendarOption> = state
		.calendar
		.calendars
		.iter()
		.filter(|calendar| calendar.is_task_list())
		.map(|calendar| CalendarOption {
			id: calendar.id.clone(),
			name: calendar.name.clone(),
		})
		.collect();

	let mut section = column![].spacing(metrics.spacing);

	let (options, selected) = task_list_options(
		&task_lists,
		crate::config::CalendarSettings::DEFAULT_BACKLOG_LIST,
		settings.backlog_list.as_deref(),
	);
	section = section.push(field(
		"Google Tasks list for backlog tasks",
		crate::ui::controls::pick_list(options, selected, |option: CalendarOption| {
			Message::TimeBlocking(crate::timeblocking::Action::BacklogDestination(
				(!option.id.is_empty()).then_some(option.id),
			))
		})
		.text_size(metrics.label())
		.width(Fill)
		.into(),
		metrics,
	));

	let (options, selected) = task_list_options(
		&task_lists,
		crate::config::CalendarSettings::DEFAULT_TRIAGED_LIST,
		settings.triaged_list.as_deref(),
	);
	section = section.push(field(
		"Google Tasks list for triaged tasks",
		crate::ui::controls::pick_list(options, selected, |option: CalendarOption| {
			Message::TimeBlocking(crate::timeblocking::Action::TriagedDestination(
				(!option.id.is_empty()).then_some(option.id),
			))
		})
		.text_size(metrics.label())
		.width(Fill)
		.into(),
		metrics,
	));

	let (options, selected) = task_list_options(
		&task_lists,
		crate::inbox::DEFAULT_LIST,
		settings.inbox_list.as_deref(),
	);
	section = section.push(field(
		"Google Tasks list for new tasks (the Inbox)",
		crate::ui::controls::pick_list(options, selected, |option: CalendarOption| {
			Message::Inbox(crate::inbox::Action::List(
				(!option.id.is_empty()).then_some(option.id),
			))
		})
		.text_size(metrics.label())
		.width(Fill)
		.into(),
		metrics,
	));

	section.into()
}

/// What a task needs before it can leave the Inbox, and the Google Tasks lists
/// tasks are filed into. A task cannot be triaged until every field switched
/// on here has been filled in.
fn tasks<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let settings = &state.config.calendar;
	let mut section = column![].spacing(metrics.spacing);
	section = section.push(switch(
		"Require an estimated time",
		settings.triage.estimate,
		|required| Message::TimeBlocking(crate::timeblocking::Action::RequireEstimate(required)),
		metrics,
	));
	section = section.push(switch(
		"Require a project",
		settings.triage.project,
		|required| Message::TimeBlocking(crate::timeblocking::Action::RequireProject(required)),
		metrics,
	));
	section = section.push(switch(
		"Require a priority",
		settings.triage.priority,
		|required| Message::TimeBlocking(crate::timeblocking::Action::RequirePriority(required)),
		metrics,
	));
	indent(
		column![
			section,
			subsection("Google Tasks", google_tasks(state, metrics), metrics),
		]
		.spacing(metrics.spacing * 2.0)
		.into(),
		metrics,
	)
}

/// The habits a time block's "I got distracted" button asks about.
fn distraction_habits<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	use crate::habits::Action;
	let library = &state.habits.library;
	let chosen = library.distraction_habits();
	let mut section = column![].spacing(metrics.spacing);
	for habit in &chosen {
		let remove = button(text("Remove").size(metrics.label()))
			.style(crate::ui::controls::secondary)
			.on_press(Message::Habits(Action::SetDistraction(
				habit.id.clone(),
				false,
			)));
		let name = text(habit.name.as_str()).size(metrics.label());
		let entry: Element<'a, Message> = if metrics.breakpoint.is_compact() {
			column![name, remove].spacing(2.0).width(Fill).into()
		} else {
			row![name.width(Fill), remove]
				.spacing(metrics.spacing)
				.align_y(Center)
				.into()
		};
		section = section.push(entry);
	}
	let available: Vec<CalendarOption> = library
		.habits
		.iter()
		.filter(|habit| !habit.distraction && !habit.archived)
		.map(|habit| CalendarOption {
			id: habit.id.clone(),
			name: habit.name.clone(),
		})
		.collect();
	if !available.is_empty() {
		section = section.push(
			crate::ui::controls::pick_list(available, None::<CalendarOption>, |option| {
				Message::Habits(Action::SetDistraction(option.id, true))
			})
			.placeholder("Add a habit")
			.text_size(metrics.label())
			.width(Fill),
		);
	} else if chosen.is_empty() {
		section = section.push(
			text("Create a habit on the Habits page to add it here.")
				.size(metrics.label())
				.style(text::secondary),
		);
	}
	indent(section.into(), metrics)
}

/// Services a pasted link can come from, and the accounts each one reads as.
fn integrations<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let accounts = &state.accounts;

	let mut section = column![with_info(
		text("Gmail").size(metrics.label()).width(Fill),
		GMAIL_INFO,
		metrics.label(),
	)]
	.spacing(metrics.spacing);

	if accounts.emails.is_empty() {
		section = section.push(
			text("No extra accounts connected")
				.size(metrics.label())
				.style(text::secondary),
		);
	}
	for email in &accounts.emails {
		let remove = button(text("Remove").size(metrics.label()))
			.style(crate::ui::controls::secondary)
			.on_press(Message::Accounts(accounts::Action::Remove(email.clone())));
		let entry: Element<'a, Message> = if metrics.breakpoint.is_compact() {
			column![text(email.as_str()).size(metrics.label()), remove]
				.spacing(2.0)
				.width(Fill)
				.into()
		} else {
			row![
				text(email.as_str()).size(metrics.label()).width(Fill),
				remove
			]
			.spacing(metrics.spacing)
			.align_y(Center)
			.into()
		};
		section = section.push(entry);
	}

	let mut add = button(
		text(if accounts.connecting {
			"Waiting for Google…"
		} else {
			"Add Google account"
		})
		.size(metrics.label())
		.center()
		.width(Fill),
	)
	.width(Fill)
	.style(crate::ui::controls::primary);
	if !accounts.connecting {
		add = add.on_press(Message::Accounts(accounts::Action::Add));
	}
	section = section.push(add);

	indent(section.into(), metrics)
}

fn day_end_label(hour: u32, time_format: TimeFormat) -> String {
	if hour == 0 {
		"Day ends at midnight".to_owned()
	} else {
		format!(
			"Day ends at {}",
			calendar::time_format::hour(hour, time_format)
		)
	}
}

pub(super) fn switch<'a>(
	label: &'a str,
	value: bool,
	on_toggle: impl Fn(bool) -> Message + 'a,
	metrics: &Metrics,
) -> Element<'a, Message> {
	crate::ui::controls::toggler(value)
		.label(label)
		.text_size(metrics.label())
		.on_toggle(on_toggle)
		.width(Fill)
		.into()
}

/// Indents a subsection so the sliders visibly belong to their toggle.
pub(super) fn indent<'a>(content: Element<'a, Message>, metrics: &Metrics) -> Element<'a, Message> {
	container(content)
		.padding(iced::Padding {
			left: if metrics.breakpoint.is_compact() {
				0.0
			} else {
				6.0
			},
			..iced::Padding::ZERO
		})
		.width(Fill)
		.into()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn task_list_defaults_are_selected_once_and_keep_automatic_creation() {
		for default_name in [
			config::CalendarSettings::DEFAULT_BACKLOG_LIST,
			config::CalendarSettings::DEFAULT_TRIAGED_LIST,
			crate::inbox::DEFAULT_LIST,
		] {
			let custom = CalendarOption {
				id: "tasks:custom".into(),
				name: "Personal".into(),
			};
			// The named default remains selectable before Google has created it.
			let (options, selected) =
				task_list_options(std::slice::from_ref(&custom), default_name, None);
			assert_eq!(options.len(), 2);
			let selected = selected.unwrap();
			assert_eq!(selected.name, default_name);
			assert!(selected.id.is_empty(), "default must use find-or-create");

			let lists = vec![
				CalendarOption {
					id: "tasks:default".into(),
					name: default_name.into(),
				},
				custom,
			];
			for configured in [None, Some(""), Some("tasks:default")] {
				let (options, selected) = task_list_options(&lists, default_name, configured);
				assert_eq!(options.len(), 2);
				assert_eq!(
					options
						.iter()
						.filter(|list| list.name == default_name)
						.count(),
					1
				);
				let selected = selected.unwrap();
				assert_eq!(selected.name, default_name);
				assert!(selected.id.is_empty());
			}
			let (_, selected) = task_list_options(&lists, default_name, Some("tasks:custom"));
			assert_eq!(selected.unwrap().id, "tasks:custom");
		}
	}

	#[test]
	fn sleep_settings_live_in_their_own_section() {
		use iced_test::{Simulator, core::Settings};

		let state = State::blank();
		let size = iced::Size::new(420.0, 1200.0);
		let metrics = Metrics::of(size);
		for label in ["Bedtime", "Wake-up", "Sun", "Mon", "Sat"] {
			let mut ui = Simulator::with_size(Settings::default(), size, sleep(&state, &metrics));
			assert!(ui.find(label).is_ok(), "missing {label}");
		}

		let mut ui = Simulator::with_size(Settings::default(), size, calendar(&state, &metrics));
		assert!(ui.find("Bedtime").is_err());
	}

	#[test]
	fn sleep_weekdays_follow_the_preferred_week_start() {
		use iced_test::{Simulator, core::Settings};
		let mut state = State::blank();
		let size = iced::Size::new(420.0, 1400.0);
		let metrics = Metrics::of(size);
		for first in FirstDayOfWeek::ALL {
			state.config.calendar.first_day_of_week = first;
			let mut ui = Simulator::with_size(Settings::default(), size, sleep(&state, &metrics));
			let positions: Vec<_> = first
				.short_labels()
				.into_iter()
				.map(|label| ui.find(label).unwrap().bounds().y)
				.collect();
			assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
		}
	}

	#[test]
	fn task_controls_live_in_their_sections() {
		use iced_test::{Simulator, core::Settings};

		let state = State::blank();
		let size = iced::Size::new(420.0, 2400.0);
		let metrics = Metrics::of(size);

		// The default Google Tasks lists are a subsection of Tasks, below the
		// triage requirements.
		let mut ui = Simulator::with_size(Settings::default(), size, tasks(&state, &metrics));
		let positions: Vec<_> = ["Google Tasks", "Google Tasks list for backlog tasks"]
			.into_iter()
			.map(|label| ui.find(label).unwrap().bounds().y)
			.collect();
		assert!(positions[0] > 0.0, "the triage switches come first");
		assert!(positions[0] < positions[1]);
		for label in [
			"Google Tasks list for backlog tasks",
			"Google Tasks list for triaged tasks",
			"Google Tasks list for new tasks (the Inbox)",
		] {
			assert!(ui.find(label).is_ok(), "missing {label}");
		}

		assert!(
			ui.find("Google Tasks list for time-blocked and scheduled tasks")
				.is_err()
		);

		// Neither is in Google Calendar, and the day end control moved out.
		let mut ui = Simulator::with_size(Settings::default(), size, calendar(&state, &metrics));
		assert!(ui.find("Google Tasks list for backlog tasks").is_err());
		assert!(ui.find("Day ends at midnight").is_err());

		let mut ui = Simulator::with_size(Settings::default(), size, preferences(&state, &metrics));
		assert!(ui.find("Day ends at midnight").is_ok());
	}

	#[test]
	fn distraction_habits_are_listed_added_and_removed_at_every_width() {
		use crate::habits::{Action, Habit};
		use iced_test::{Simulator, core::Settings};

		let mut state = State::blank();
		let mut phone = Habit::new("phone", "Check phone");
		phone.distraction = true;
		let mut old = Habit::new("old", "Archived distraction");
		old.distraction = true;
		old.archived = true;
		state.habits.library.habits = vec![phone, old, Habit::new("read", "Read a book")];

		for width in [200.0, 420.0, 900.0] {
			let size = iced::Size::new(width, 900.0);
			let metrics = Metrics::of(size);
			let mut ui = Simulator::with_size(
				Settings::default(),
				size,
				distraction_habits(&state, &metrics),
			);
			assert!(ui.find("Check phone").is_ok(), "{width}");
			assert!(ui.find("Archived distraction").is_err(), "{width}");
			let remove = ui.find("Remove").unwrap().bounds();
			assert!(remove.x + remove.width <= width, "cut off at {width}");
			let _ = ui.click("Remove").unwrap();
			assert!(ui.into_messages().any(|message| matches!(
				message,
				Message::Habits(Action::SetDistraction(id, false)) if id == "phone"
			)));
		}

		let _ = crate::habits::update(&mut state, Action::SetDistraction("read".into(), true));
		let _ = crate::habits::update(&mut state, Action::SetDistraction("phone".into(), false));
		let names: Vec<_> = state
			.habits
			.library
			.distraction_habits()
			.iter()
			.map(|habit| habit.name.as_str())
			.collect();
		assert_eq!(names, ["Read a book"]);
	}

	#[test]
	fn week_day_choices_show_the_start_day_and_switch_in_a_narrow_panel() {
		use crate::config::WeekRange;
		use iced_test::{Simulator, core::Settings};

		let size = iced::Size::new(200.0, 400.0);
		let metrics = Metrics::of(size);
		for (range, next) in [
			(WeekRange::Seven, WeekRange::Eight),
			(WeekRange::Eight, WeekRange::EightAndYesterday),
			(WeekRange::EightAndYesterday, WeekRange::Seven),
		] {
			let mut state = State::blank();
			state.config.calendar.set_week_range(range);
			let label = range.to_string();
			let mut ui =
				Simulator::with_size(Settings::default(), size, days_shown(&state, &metrics));
			ui.click(label.as_str()).unwrap();
			assert!(ui.into_messages().any(|message| matches!(
				message, Message::Settings(SettingsMessage::CalendarWeekRangeChanged(value)) if value == next
			)));
		}
	}

	#[test]
	fn reservation_switches_are_reachable_and_independent_at_narrow_widths() {
		use crate::app;
		use iced_test::{Simulator, core::Settings};

		for width in [180.0, 420.0] {
			let mut state = State::blank();
			let size = iced::Size::new(width, 400.0);
			let metrics = Metrics::of(size);
			let mut ui =
				Simulator::with_size(Settings::default(), size, reserve_space(&state, &metrics));
			ui.click(iced::widget::Id::from("dock-reservation"))
				.unwrap();
			for message in ui.into_messages() {
				let _ = app::update(&mut state, message);
			}
			assert!(state.config.views.dock_reserve_space);
			assert!(!state.config.views.panel_reserve_space);

			let mut ui =
				Simulator::with_size(Settings::default(), size, reserve_space(&state, &metrics));
			ui.click(iced::widget::Id::from("panel-reservation"))
				.unwrap();
			for message in ui.into_messages() {
				let _ = app::update(&mut state, message);
			}
			assert!(state.config.views.dock_reserve_space);
			assert!(state.config.views.panel_reserve_space);
		}
	}

	#[test]
	fn margin_controls_live_in_their_respective_sections() {
		use iced_test::{Simulator, core::Settings};
		let state = State::blank();
		let size = iced::Size::new(420.0, 1800.0);
		let metrics = Metrics::of(size);
		let mut ui = Simulator::with_size(Settings::default(), size, borders(&state, &metrics));
		assert!(ui.find("Screen margin (px, 0–30)").is_err());
		assert!(ui.find("Border radius (px, 0–30)").is_err());
		let mut ui = Simulator::with_size(Settings::default(), size, panel(&state, &metrics));
		assert!(ui.find("Screen margin (px, 0–30)").is_ok());
		let mut ui = Simulator::with_size(Settings::default(), size, dock(&state, &metrics));
		assert!(ui.find("Left margin (px, 0–100)").is_ok());
		assert!(ui.find("Right margin (px, 0–100)").is_ok());
	}

	#[test]
	fn dock_hour_lines_live_in_preferences() {
		use crate::app;
		use iced_test::{Simulator, core::Settings};

		let mut state = State::blank();
		let size = iced::Size::new(420.0, 800.0);
		let metrics = Metrics::of(size);

		let mut ui = Simulator::with_size(Settings::default(), size, preferences(&state, &metrics));
		ui.click(iced::widget::Id::from("dock-hour-lines")).unwrap();
		for message in ui.into_messages() {
			let _ = app::update(&mut state, message);
		}
		assert!(!state.config.views.dock_hour_lines);

		let mut ui = Simulator::with_size(
			Settings::default(),
			size,
			desktop_appearance(&state, &metrics),
		);
		assert!(ui.find(iced::widget::Id::from("dock-hour-lines")).is_err());
	}

	#[test]
	fn shortcut_descriptions_are_tucked_behind_info_symbols() {
		use crate::panel::Action;
		use iced_test::{Simulator, core::Settings};

		let state = State::blank();
		for width in [180.0, 300.0, 420.0, 800.0] {
			let size = iced::Size::new(width, 800.0);
			let metrics = Metrics::of(size);
			let mut ui =
				Simulator::with_size(Settings::default(), size, shortcuts(&state, &metrics));
			for action in Action::ALL {
				assert!(
					ui.find(action.label()).is_ok(),
					"missing {}",
					action.label()
				);
				assert!(ui.find(action.description()).is_err());
			}
			assert!(ui.find(SHORTCUTS_INFO).is_err());
		}
	}

	#[test]
	fn explanations_are_tucked_behind_info_symbols() {
		use iced_test::{Simulator, core::Settings};

		let state = State::blank();
		for width in [180.0, 420.0, 900.0] {
			let size = iced::Size::new(width, 6000.0);
			let metrics = Metrics::of(size);
			let mut ui = Simulator::with_size(Settings::default(), size, view(&state, &metrics));
			for info in [
				SHORTCUTS_INFO,
				SYNC_URL_INFO,
				NOTES_INFO,
				DOCK_RESERVATION_INFO,
				PANEL_RESERVATION_INFO,
				TRANSITION_INFO,
				CALENDAR_ORDER_INFO,
				SLEEP_INFO,
				GOOGLE_OAUTH_INFO,
				TASKS_INFO,
				DISTRACTION_INFO,
				GMAIL_INFO,
			] {
				assert!(
					ui.find(info).is_err(),
					"{info:?} is spelled out at {width}px"
				);
			}
			for label in ["Sync server URL", "OAuth client ID", "Gmail"] {
				assert!(ui.find(label).is_ok(), "missing {label} at {width}px");
			}
		}
	}

	#[test]
	fn a_shortcut_is_removed_from_its_pill() {
		use crate::panel::Action;
		use iced_test::{Simulator, core::Settings};

		let state = State::blank();
		for width in [180.0, 420.0] {
			let size = iced::Size::new(width, 300.0);
			let metrics = Metrics::of(size);
			let mut ui = Simulator::with_size(
				Settings::default(),
				size,
				shortcut_action(&state, Action::ToggleVisibility, &metrics),
			);
			assert!(ui.find("Meta+F").is_ok());
			ui.click("×").unwrap();
			assert!(ui.into_messages().any(|message| matches!(
				message,
				Message::Settings(SettingsMessage::ShortcutRemoved(
					Action::ToggleVisibility,
					0
				))
			)));
		}
	}

	#[test]
	fn theme_and_days_shown_live_in_preferences() {
		use iced_test::{Simulator, core::Settings};

		let state = State::blank();
		let size = iced::Size::new(420.0, 800.0);
		let metrics = Metrics::of(size);

		let mut ui = Simulator::with_size(Settings::default(), size, preferences(&state, &metrics));
		for label in ["Theme", "Days shown in week view"] {
			assert!(ui.find(label).is_ok(), "missing {label}");
		}

		let mut ui = Simulator::with_size(
			Settings::default(),
			size,
			desktop_appearance(&state, &metrics),
		);
		for label in ["Theme", "Days shown in week view"] {
			assert!(ui.find(label).is_err(), "{label} should have moved");
		}
	}

	#[test]
	fn desktop_appearance_keeps_every_control_in_subsection_order() {
		use iced_test::{Simulator, core::Settings};

		let state = State::blank();
		let size = iced::Size::new(420.0, 2400.0);
		let metrics = Metrics::of(size);
		let mut ui = Simulator::with_size(
			Settings::default(),
			size,
			desktop_appearance(&state, &metrics),
		);

		let positions: Vec<_> = [
			"Reserve space",
			"Desktop transition",
			"Borders",
			"Dock",
			"Side panel",
		]
		.into_iter()
		.map(|label| ui.find(label).unwrap().bounds().y)
		.collect();
		assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));

		for id in ["dock-reservation", "panel-reservation"] {
			assert!(ui.find(iced::widget::Id::from(id)).is_ok(), "missing {id}");
		}
		for label in [
			"When the window appears",
			"Outer border depth (px, 0–5)",
			"Outer border color",
			"Inner border depth (px, 0–5)",
			"Inner border color",
			"Screen edge",
			"Corner radius (px, 0–30)",
			"Dock width (px, 2–40)",
			"Left margin (px, 0–100)",
			"Right margin (px, 0–100)",
			"Screen margin (px, 0–30)",
		] {
			assert!(ui.find(label).is_ok(), "missing {label}");
		}
	}

	#[test]
	fn layout_number_buttons_step_and_stop_at_the_limits() {
		use crate::app;
		use iced_test::{Simulator, core::Settings};

		let mut state = State::blank();
		state.config.views.dock_left_margin = Some(10.0);
		state.config.views.dock_right_margin = Some(20.0);
		for (kind, step) in [
			(LayoutField::CalendarWidth, 10.0),
			(LayoutField::CalendarWeekWidth, 10.0),
			(LayoutField::CalendarMonthWidth, 10.0),
			(LayoutField::TimeBlockWidth, 10.0),
			(LayoutField::DetailWidth, 10.0),
			(LayoutField::ScreenMargin, 1.0),
			(LayoutField::DockWidth, 1.0),
			(LayoutField::DockLeftMargin, 1.0),
			(LayoutField::DockRightMargin, 1.0),
			(LayoutField::DockBorderOuterWidth, 1.0),
			(LayoutField::DockBorderInnerWidth, 1.0),
			(LayoutField::PanelBorderOuterWidth, 1.0),
			(LayoutField::PanelBorderInnerWidth, 1.0),
		] {
			for width in [180.0, 420.0] {
				let size = iced::Size::new(width, 100.0);
				let metrics = Metrics::of(size);
				for delta in [-step, step] {
					let before = kind.value(&state);
					let mut ui = Simulator::with_size(
						Settings::default(),
						size,
						layout_number("Size (px)", kind, &state, &metrics),
					);
					let input = ui
						.find(iced::widget::Id::from(format!("layout-{kind:?}")))
						.unwrap()
						.bounds();
					assert!(
						input.width >= 30.0,
						"number must remain editable at {width}px"
					);
					assert!(input.x >= 0.0 && input.x + input.width <= width);
					ui.click(if delta > 0.0 { "+" } else { "−" }).unwrap();
					for message in ui.into_messages() {
						let _ = app::update(&mut state, message);
					}
					assert_eq!(kind.value(&state), before + delta);
				}
			}
		}
		for (value, label) in [(0.0, "−"), (30.0, "+")] {
			state.config.views.margin = value;
			let size = iced::Size::new(180.0, 100.0);
			let mut ui = Simulator::with_size(
				Settings::default(),
				size,
				layout_number(
					"Margin",
					LayoutField::ScreenMargin,
					&state,
					&Metrics::of(size),
				),
			);
			ui.click(label).unwrap();
			assert_eq!(ui.into_messages().count(), 0);
		}
	}

	#[test]
	fn the_first_day_of_the_week_defaults_to_sunday_and_persists() {
		use crate::{app, config::Config, theme::Catalog};
		use iced_test::{Simulator, core::Settings};

		let old: Config = toml::from_str("[calendar]\nweekly_day_width = 150\n").unwrap();
		assert_eq!(old.calendar.first_day_of_week, FirstDayOfWeek::Sunday);
		let mut state = State::new(old, Catalog::built_in());
		for expected in FirstDayOfWeek::ALL {
			let _ = app::update(
				&mut state,
				Message::Settings(SettingsMessage::FirstDayOfWeekSelected(expected)),
			);
			let saved = toml::to_string(&state.config).unwrap();
			let restored: Config = toml::from_str(&saved).unwrap();
			assert_eq!(restored.calendar.first_day_of_week, expected);
		}

		// In a narrow panel the choice is a button that moves to the next day.
		let size = iced::Size::new(180.0, 400.0);
		state.config.calendar.first_day_of_week = FirstDayOfWeek::Sunday;
		let mut ui = Simulator::with_size(
			Settings::default(),
			size,
			preferences(&state, &Metrics::of(size)),
		);
		ui.click("Sunday").unwrap();
		assert!(ui.into_messages().any(|message| matches!(
			message,
			Message::Settings(SettingsMessage::FirstDayOfWeekSelected(
				FirstDayOfWeek::Monday
			))
		)));
	}

	#[test]
	fn distance_unit_choices_persist_without_changing_cached_routes() {
		use crate::{
			app,
			config::{Config, DistanceUnit, SavedRoute, TravelMode},
		};
		use iced_test::{Simulator, core::Settings};

		let old: Config = toml::from_str("[appearance]\ntheme = 'Sakura'\n").unwrap();
		assert_eq!(old.appearance.distance_unit, DistanceUnit::Kilometers);
		let mut state = State::blank();
		state.config.travel.routes.push(SavedRoute {
			from: 1,
			to: 2,
			mode: TravelMode::Walk,
			seconds: 600,
			meters: 1609,
		});
		let cached = state.config.travel.routes.clone();
		for unit in [DistanceUnit::Miles, DistanceUnit::Kilometers] {
			let _ = app::update(
				&mut state,
				Message::Settings(SettingsMessage::DistanceUnitSelected(unit)),
			);
			let saved = toml::to_string(&state.config).unwrap();
			let restored: Config = toml::from_str(&saved).unwrap();
			assert_eq!(restored.appearance.distance_unit, unit);
			assert_eq!(restored.travel.routes, cached);
			let shared: timeparagon_core::config::Config = toml::from_str(&saved).unwrap();
			assert_eq!(shared.appearance.distance_unit, unit);
			assert_eq!(shared.travel.routes, cached);
		}
		let size = iced::Size::new(180.0, 600.0);
		let mut ui = Simulator::with_size(
			Settings::default(),
			size,
			preferences(&state, &Metrics::of(size)),
		);
		ui.click("Kilometers").unwrap();
		assert!(ui.into_messages().any(|message| matches!(
			message,
			Message::Settings(SettingsMessage::DistanceUnitSelected(DistanceUnit::Miles))
		)));
	}

	#[test]
	fn time_format_choices_update_and_persist_the_preference() {
		use crate::{app, calendar::time_format::TimeFormat, config::Config, theme::Catalog};
		use iced_test::{Simulator, core::Settings};

		let old: Config = toml::from_str("[appearance]\ntheme = 'Sakura'\n").unwrap();
		assert_eq!(old.appearance.time_format, TimeFormat::TwentyFourHour);
		let mut state = State::new(old, Catalog::built_in());
		for expected in TimeFormat::ALL {
			let _ = app::update(
				&mut state,
				Message::Settings(SettingsMessage::TimeFormatSelected(expected)),
			);
			assert_eq!(state.config.appearance.time_format, expected);
			let saved = toml::to_string(&state.config).unwrap();
			let restored: Config = toml::from_str(&saved).unwrap();
			assert_eq!(restored.appearance.time_format, expected);
		}
		for width in [180.0, 420.0] {
			let size = iced::Size::new(width, 400.0);
			let mut ui = Simulator::with_size(
				Settings::default(),
				size,
				preferences(&state, &Metrics::of(size)),
			);
			if width < 200.0 {
				ui.click("24-hour").unwrap();
				assert!(ui.into_messages().any(|message| matches!(
					message,
					Message::Settings(SettingsMessage::TimeFormatSelected(TimeFormat::TwelveHour))
				)));
			} else {
				// Pick lists draw their labels directly, so operate the control by position.
				ui.point_at(iced::Point::new(width - 30.0, 15.0));
				ui.simulate([
					iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(
						iced::keyboard::Modifiers::CTRL,
					)),
					iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
						delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
					}),
				]);
				assert!(ui.into_messages().any(|message| matches!(
					message,
					Message::Settings(SettingsMessage::TimeFormatSelected(TimeFormat::Compact))
				)));
			}
		}
	}

	#[test]
	fn calendar_widths_are_independent_and_persist() {
		use crate::{app, config::Config};
		let mut state = State::blank();
		for (field, value) in [
			(LayoutField::CalendarWidth, 300.0),
			(LayoutField::CalendarWeekWidth, 900.0),
			(LayoutField::CalendarMonthWidth, 1100.0),
		] {
			let _ = app::update(&mut state, field.changed(value));
		}
		let restored: Config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
		assert_eq!(restored.views.calendar_width(200.0), 300.0);
		assert_eq!(restored.views.calendar_week_width(200.0), 900.0);
		assert_eq!(restored.views.calendar_month_width(), 1100.0);
	}

	#[test]
	fn appearance_border_edits_are_shared_with_the_dock() {
		use crate::app;
		let mut state = State::blank();
		let _ = app::update(
			&mut state,
			Message::Settings(SettingsMessage::PanelBorderOuterWidthChanged(3.0)),
		);
		let _ = app::update(
			&mut state,
			Message::Settings(SettingsMessage::PanelBorderInnerWidthChanged(2.0)),
		);
		let _ = app::update(
			&mut state,
			Message::Settings(SettingsMessage::PanelBorderColorInput(
				BorderLayer::Outer,
				"#123456".into(),
			)),
		);
		let _ = app::update(
			&mut state,
			Message::Settings(SettingsMessage::PanelBorderColorInput(
				BorderLayer::Inner,
				"#ABCDEF".into(),
			)),
		);
		let dock = crate::dock::config::DockConfig::load(&state.config.views);
		assert_eq!(
			dock.border_outer_width,
			state.config.views.panel_border_outer_width()
		);
		assert_eq!(
			dock.border_inner_width,
			state.config.views.panel_border_inner_width()
		);
		assert_eq!(
			dock.border_outer_color,
			state.config.views.panel_border_outer_color
		);
		assert_eq!(
			dock.border_inner_color,
			state.config.views.panel_border_inner_color
		);
	}

	#[test]
	fn cycling_wraps_around_the_options() {
		let options = vec!["a", "b", "c"];

		assert_eq!(next_option(&options, Some(&"a")), Some("b"));
		assert_eq!(next_option(&options, Some(&"c")), Some("a"));
	}

	#[test]
	fn cycling_from_nothing_selects_the_first_option() {
		let options = vec!["a", "b"];

		assert_eq!(next_option(&options, None), Some("a"));
	}

	#[test]
	fn cycling_an_empty_list_selects_nothing() {
		assert_eq!(next_option::<&str>(&[], None), None);
	}

	#[test]
	fn repeat_labels_read_naturally() {
		assert_eq!(repeat_label(0), "Repeat until dismissed");
		assert_eq!(repeat_label(1), "Play once");
		assert_eq!(repeat_label(3), "Play 3 times");
	}

	#[test]
	fn day_end_labels_read_naturally() {
		assert_eq!(
			day_end_label(0, TimeFormat::TwelveHour),
			"Day ends at midnight"
		);
		assert_eq!(day_end_label(3, TimeFormat::TwelveHour), "Day ends at 3 AM");
		assert_eq!(day_end_label(5, TimeFormat::TwelveHour), "Day ends at 5 AM");
	}
}
