//! The interface.
//!
//! The root is wrapped in `responsive`, so the layout is chosen from the
//! space the window actually has rather than from a guess made at startup.
//! That is what lets the same code look deliberate at 180px and at 900px.

pub mod alarms;
mod anchored_menu;
pub mod brain_dump;
pub mod calendar;
pub mod controls;
mod description;
mod drop_preview;
pub mod flash;
pub mod habits;
mod habits_charts;
pub mod inbox;
pub mod layout;
pub mod life;
pub mod links;
mod meetings;
pub mod notepad;
pub mod notes;
pub mod people;
pub mod recurrence;
pub mod reminders;
mod rounded_clip;
pub mod routines;
pub mod settings;
pub mod source_icon;
mod task_drag;
pub(crate) mod task_drop;
mod task_input;
pub mod timeblocking;
mod timeblocking_focus;
pub mod travel;
mod weekly_task_list;

use iced::widget::{Space, button, column, container, mouse_area, responsive, row, text, tooltip};
use iced::{Alignment, Center, Element, Fill, Length, Padding, Size};

use crate::app::{CalendarMessage, CalendarView, NavigationMessage, PlanTab, SystemMessage};
use crate::app::{Message, Screen, State};

use layout::Metrics;

/// Align the floating editor with the panel's visible top edge.
pub fn editor_window_top(state: &State) -> f32 {
	state.screen_margin()
}

pub fn editor_window(state: &State) -> Element<'_, Message> {
	let content = responsive(move |size| {
		if state.travel.map.active {
			return crate::travel::map::view(state);
		}
		// Checked before the range views: a contact opened while the week is
		// on show takes this window over until it is closed.
		if state.people_panel_open() {
			let metrics = Metrics::of(size);
			return container(people::panel(state, &metrics))
				.width(Fill)
				.height(Fill)
				.style(controls::surface)
				.into();
		}
		if state.life_panel_open() {
			let metrics = Metrics::of(size);
			// Date pickers opened from the panel belong to this window.
			let mut layers = vec![life::panel(state, &metrics)];
			push_editor_dialogs(&mut layers, state, &metrics);
			return container(iced::widget::stack(layers))
				.width(Fill)
				.height(Fill)
				.style(controls::surface)
				.into();
		}
		// Checked before the range views for the same reason. An event opened
		// from the habit's page goes beside it.
		if state.habit_panel_open() {
			let width = habits::PANEL_WIDTH.min(size.width);
			let metrics = Metrics::of(Size::new(width, size.height));
			let mut panes = row![container(habits::panel(state, &metrics)).width(width)];
			if state.calendar.open_event.is_some() {
				let width = state
					.config
					.views
					.detail_width()
					.min((size.width - width).max(1.0));
				let metrics = Metrics::of(Size::new(width, size.height));
				panes = panes.push(container(calendar::details_pane(state, &metrics)).width(width));
			}
			let metrics = Metrics::of(size);
			let mut layers = vec![panes.height(Fill).into()];
			push_editor_dialogs(&mut layers, state, &metrics);
			return container(iced::widget::stack(layers))
				.width(Fill)
				.height(Fill)
				.style(controls::surface)
				.into();
		}
		if state.range_view_active() {
			let metrics = Metrics::of(size);
			let (content, name) = if state.view == CalendarView::Schedule {
				(calendar::schedule_view(state, &metrics), "Schedule")
			} else if state.monthly() {
				(calendar::monthly_view(state, &metrics), "Month")
			} else if state.two_weekly() {
				(calendar::two_week_view(state, &metrics), "2 Weeks")
			} else {
				(calendar::weekly_view(state, &metrics), "Week")
			};
			let mut view_header = row![text(name).size(metrics.label()),]
				.spacing(12)
				.align_y(Alignment::Center);
			if state.weekly() && !state.monthly() {
				view_header = view_header.push(
					crate::ui::controls::toggler(state.config.calendar.weekly_task_list)
						.label("Show task list")
						.text_size(metrics.label())
						.on_toggle(|v0| {
							Message::Calendar(CalendarMessage::CalendarTaskListToggled(v0))
						}),
				);
			}
			let body: Element<'_, Message> =
				if state.weekly() && state.config.calendar.weekly_task_list {
					content
				} else {
					column![view_header.padding([4, 8]), content]
						.height(Fill)
						.into()
				};
			let mut layers = vec![body];
			push_editor_dialogs(&mut layers, state, &metrics);
			if let Some((task, pointer)) = range_drag_ghost(state) {
				layers.push(drag_ghost(state, task, &metrics, pointer, size));
			}
			return container(iced::widget::stack(layers))
				.width(Fill)
				.height(Fill)
				.style(controls::surface)
				.into();
		}
		let mut panes = row![];
		if state.day_plan_active() {
			let width = state.calendar_width().min(size.width);
			let metrics = Metrics::of(Size::new(width, size.height));
			panes = panes.push(container(calendar::view(state, &metrics)).width(width));
		}
		if state.timeblocking.block.is_some() {
			let width = state.config.views.time_block_width().min(size.width);
			let metrics = Metrics::of(Size::new(width, size.height));
			panes = panes.push(container(timeblocking::panel(state, &metrics)).width(width));
		}
		if state.calendar.open_event.is_some()
			|| (state.inbox.draft.is_none()
				&& state.calendar.draft.as_ref().is_some_and(|d| d.editor_open))
		{
			let block = if state.timeblocking.block.is_some() {
				state.config.views.time_block_width()
			} else {
				0.0
			};
			let width = state.config.views.detail_width().min(
				(size.width
					- block - if state.day_plan_active() {
					state.calendar_width()
				} else {
					0.0
				})
				.max(1.0),
			);
			let metrics = Metrics::of(Size::new(width, size.height));
			panes = panes.push(container(calendar::details_pane(state, &metrics)).width(width));
		}
		let metrics = Metrics::of(size);
		let mut layers = vec![panes.height(Fill).into()];
		push_editor_dialogs(&mut layers, state, &metrics);
		if let Some((task, pointer)) = range_drag_ghost(state) {
			layers.push(drag_ghost(state, task, &metrics, pointer, size));
		}
		container(iced::widget::stack(layers))
			.width(Fill)
			.height(Fill)
			.style(controls::surface)
			.into()
	});
	task_drop::host(window_frame(state, content.into(), "editor-body"))
}

/// Add task dialogs belong to the main sidebar even while a planning window
/// is open. Keep the form's auxiliary dialogs (pickers, estimates, etc.) with it.
fn push_editor_dialogs<'a>(
	layers: &mut Vec<Element<'a, Message>>,
	state: &'a State,
	metrics: &Metrics,
) {
	if state.inbox.draft.is_none() {
		push_dialogs(layers, state, metrics);
	}
}

/// Keep the dialog list shared so each host renders the same supporting dialogs.
fn push_dialogs<'a>(layers: &mut Vec<Element<'a, Message>>, state: &'a State, metrics: &Metrics) {
	// A task opened from the focus view must not end up underneath it.
	if state.timeblocking.focus.is_some()
		&& !(state.editor_window.is_some() && state.calendar.open_event.is_some())
	{
		layers.push(timeblocking::focus_overlay(state, metrics));
	}
	if state.timeblocking.pending_recurring.is_some() {
		layers.push(timeblocking::recurring_confirmation(state, metrics));
	}
	if state.timeblocking.pending_join.is_some() {
		layers.push(timeblocking::join_confirmation(state, metrics));
	}
	if state.timeblocking.session_edit.is_some() {
		layers.push(timeblocking::session_editor(state, metrics));
	}
	if state.timeblocking.distraction_open {
		layers.push(timeblocking::distraction_overlay(metrics));
	}
	if state.brain_dump.open.is_some() {
		layers.push(brain_dump::editor_overlay(state, metrics));
	}
	if state.inbox.draft.is_some() {
		layers.push(inbox::add_something_overlay(state, metrics));
	}
	if state.timeblocking.estimate_task.is_some() {
		layers.push(timeblocking::estimate_overlay(state, metrics));
	}
	if state.calendar.pending_delete.is_some() {
		layers.push(calendar::delete_confirmation(state, metrics));
	}
	if state.calendar.pending_move.is_some() {
		layers.push(calendar::move_confirmation(state, metrics));
	}
	if state.travel.open.is_some() {
		layers.push(travel::leg_overlay(state, metrics));
	}
	if state.description_popup.is_some() {
		layers.push(description::overlay(state, metrics));
	}
	if state.habits.confirm_delete.is_some() {
		layers.push(habits::delete_confirmation(state, metrics));
	}
	if state.habits.day_dialog.is_some() {
		layers.push(habits::day_dialog(state, metrics));
	}
	if let Some(picker) = &state.picker {
		layers.push(controls::overlay(
			picker,
			metrics,
			state.config.appearance.time_format,
			state.config.calendar.first_day_of_week,
		));
	}
	if state.inbox.voice.debug_open {
		layers.push(crate::voice::debug_overlay(state, metrics));
	}
}

pub fn view(state: &State) -> Element<'_, Message> {
	let content = responsive(move |size| {
		// When the map belongs to the floating window the panel keeps showing
		// what it was: drawing the map in both would make the windows fight
		// over the browser's size and show each other's frames.
		if state.travel.map.active && !state.map_in_editor_window() {
			let map = crate::travel::map::view(state);
			if state.task_creation_window {
				// The dock normally accepts input only over its bottom-aligned
				// task card. The map replaces it and fills the entire surface.
				let bounds = |size: Size| {
					Message::System(SystemMessage::TaskPopoverBounds(
						iced::Rectangle::with_size(size),
					))
				};
				return iced::widget::sensor(map)
					.on_show(bounds)
					.on_resize(bounds)
					.into();
			}
			return map;
		}
		let metrics = Metrics::of(size);

		if state.task_creation_window {
			let mut layers = vec![inbox::add_something_overlay(state, &metrics)];
			if state.timeblocking.estimate_task.is_some() {
				layers.push(timeblocking::estimate_overlay(state, &metrics));
			}
			if let Some(picker) = &state.picker {
				layers.push(controls::overlay(
					picker,
					&metrics,
					state.config.appearance.time_format,
					state.config.calendar.first_day_of_week,
				));
			}
			if state.inbox.voice.debug_open {
				layers.push(crate::voice::debug_overlay(state, &metrics));
			}
			return iced::widget::stack(layers).into();
		}

		if state.sidebar.is_some() && state.inbox.draft.is_some() {
			// Task creation replaces the whole sidebar, including navigation
			// and the add-task footer. Supporting dialogs stay above the form.
			let mut layers = Vec::new();
			push_dialogs(&mut layers, state, &metrics);
			return iced::widget::stack(layers).into();
		}

		let main_metrics = metrics;
		let nav_width = size.width;
		let nav_metrics = Metrics::of(size);

		let content: Element<'_, Message> = match (state.screen, metrics.breakpoint) {
			(Screen::Settings, _) => settings::view(state, &metrics),
			// The notepad takes the whole window for the same reason: it is
			// somewhere to write, not somewhere to watch the clock from.
			(Screen::Notes, _) => notes::view(state, &metrics),
			(Screen::People, _) => people::view(state, &metrics),
			(Screen::Notepad, _) => notepad::view(state, &metrics),
			(Screen::BrainDump, _) => brain_dump::view(state, &metrics),
			(Screen::Overdue, _) => inbox::overdue(state, &main_metrics),
			(Screen::Inbox, _) => inbox::view(state, &main_metrics),
			(Screen::Today, _) => inbox::today(state, &main_metrics),
			(Screen::Triaged, _) => inbox::triaged(state, &main_metrics),
			(Screen::Backlog, _) => timeblocking::backlog(state, &main_metrics),
			(Screen::Routines, _) => routines::view(state, &main_metrics),
			(Screen::Life, _) => life::view(state, &metrics),
			(Screen::Travel, _) => travel::view(state, &main_metrics),
			(Screen::Habits, _) => habits::view(state, &main_metrics),
			(Screen::Calendar, _) => {
				row![container(calendar::view(state, &main_metrics)).width(Fill)].into()
			}
		};

		let mut base = column![
			container(tabs(state, &nav_metrics)).width(Length::Fixed(nav_width)),
			content
		]
		.width(Fill)
		.height(Fill);
		if let Some(strip) = sleep_strip(state) {
			base = base.push(strip);
		}
		if state.sidebar.is_some() {
			// The add-task bar belongs to the main panel; floating editors
			// never participate in its layout.
			let borders = 2.0
				* (state.config.views.panel_border_outer_width()
					+ state.config.views.panel_border_inner_width());
			let footer_width = (state.calendar_width() - borders).max(1.0).min(size.width);
			let add = container(
				tooltip(
					button(text("+").size(14).center().width(Fill))
						.width(Fill)
						.height(PANEL_ACTION_HEIGHT)
						.padding(0)
						.style(crate::ui::controls::square)
						.on_press(Message::Inbox(crate::inbox::Action::NewTask)),
					text("Create new item").size(12),
					tooltip::Position::Top,
				)
				.style(crate::ui::controls::surface),
			)
			.id("panel-add-task")
			.width(Length::Fixed(
				(footer_width * PANEL_ADD_SHARE).max(PANEL_ACTION_HEIGHT * 2.0),
			))
			.height(PANEL_ACTION_HEIGHT);
			base = base.push(
				container(
					row![activity_button(state), add]
						.spacing(0)
						.align_y(Alignment::Center),
				)
				.id("panel-footer")
				.width(Length::Fixed(footer_width))
				.height(PANEL_ACTION_HEIGHT),
			);
		} else {
			base = base.push(
				container(activity_button(state))
					.id("panel-footer")
					.width(Fill)
					.height(PANEL_ACTION_HEIGHT),
			);
		}
		// Adding/removing a wrapper resets the state of all its descendants.
		// Keep one stack with the calendar first, and append active dialogs.
		let mut layers = vec![base.into()];
		if state.view_menu_open {
			layers.push(view_menu(state, &nav_metrics));
		}
		if state.more_menu_open {
			layers.push(more_menu(state, &nav_metrics));
		}
		if state.activity.open {
			layers.push(activity_log(state, &nav_metrics));
		}
		// Shared state is visible to both windows, but dialogs have one owner.
		// Add task creation stays in the sidebar, including its supporting dialogs.
		if state.editor_window.is_none() || state.inbox.draft.is_some() {
			push_dialogs(&mut layers, state, &metrics);
		} else if state.timeblocking.focus.is_some() && state.details_pane_open {
			// The floating editor is showing a task beside the focus view.
			layers.push(timeblocking::focus_overlay(state, &metrics));
		}
		// The floating task card is drawn last so it is never hidden, and it is
		// only added while a task is actually in the air.
		if let Some(task) = state
			.timeblocking
			.dragging
			.as_deref()
			.and_then(|id| state.calendar.event(id))
			.filter(|_| {
				state.timeblocking.drag_pointer.is_some_and(|point| {
					state.timeblocking.drag_window.or(state.main_window) == state.main_window
						&& iced::Rectangle::with_size(state.main_size).contains(point)
				})
			}) {
			let pointer = state.timeblocking.drag_pointer.unwrap_or_default();
			// The sidebar body is inset from the window, and the pointer is in
			// window coordinates, so shift it into the view's own space.
			let inset = if state.sidebar.is_some() {
				let margin = state.screen_margin();
				iced::Point::new(margin, margin)
			} else {
				iced::Point::ORIGIN
			};
			layers.push(drag_ghost(
				state,
				task,
				&metrics,
				iced::Point::new(pointer.x - inset.x, pointer.y - inset.y),
				size,
			));
		}
		iced::widget::stack(layers).into()
	});
	if state.task_creation_window {
		return content.into();
	}
	let content: Element<'_, Message> = match &state.config_error {
		Some(error) => column![
			container(text(error).size(13).style(text::danger))
				.width(Fill)
				.padding([4, 8]),
			content,
		]
		.into(),
		None => content.into(),
	};
	let body = window_frame(
		state,
		content,
		if state.sidebar.is_some() {
			"sidebar-body"
		} else {
			"application-body"
		},
	);
	if state.sidebar.is_some() {
		let margin = state.screen_margin();
		container(body)
			.padding(margin)
			.width(Fill)
			.height(Fill)
			.into()
	} else {
		body
	}
}

/// Shared nested borders keep the panel and every floating editor in sync.
fn window_frame<'a>(
	state: &State,
	content: Element<'a, Message>,
	id: &'static str,
) -> Element<'a, Message> {
	let radius = if state.sidebar.is_some() {
		state.config.views.panel_border_radius()
	} else {
		state.config.views.border_radius()
	};
	let content = container(content).width(Fill).height(Fill).clip(true);
	let body: Element<'_, Message> = {
		let outer_width = state.config.views.panel_border_outer_width();
		let inner_width = state.config.views.panel_border_inner_width();
		let outer_color = state.config.views.panel_border_outer_color.0;
		let inner_color = state.config.views.panel_border_inner_color.0;
		let body_radius = (radius - outer_width - inner_width).max(0.0);
		let inner_radius = (radius - outer_width).max(0.0);

		let body = container(rounded_clip::rounded_clip(content, body_radius))
			.width(Fill)
			.height(Fill)
			.clip(true)
			.style(move |theme: &iced::Theme| container::Style {
				background: Some(theme.palette().background.into()),
				border: iced::Border {
					radius: body_radius.into(),
					..Default::default()
				},
				..Default::default()
			});
		let inner = container(body)
			.width(Fill)
			.height(Fill)
			.clip(true)
			.padding(inner_width)
			.style(move |_: &iced::Theme| container::Style {
				background: Some(inner_color.into()),
				border: iced::Border {
					radius: inner_radius.into(),
					..Default::default()
				},
				..Default::default()
			});

		container(inner)
			.id(id)
			.width(Fill)
			.height(Fill)
			.clip(true)
			.padding(outer_width)
			.style(move |_: &iced::Theme| container::Style {
				background: Some(outer_color.into()),
				border: iced::Border {
					radius: radius.into(),
					..Default::default()
				},
				..Default::default()
			})
			.into()
	};
	body
}

fn range_drag_ghost(state: &State) -> Option<(&crate::calendar::Event, iced::Point)> {
	if drop_preview::hides_ghost(state) {
		return None;
	}
	let task = state
		.calendar
		.event(state.timeblocking.dragging.as_deref()?)?;
	let point = task_drop::range_position(
		state,
		state.timeblocking.drag_window.or(state.main_window)?,
		state.timeblocking.drag_pointer?,
	)?;
	Some((task, point))
}

/// Places the floating task card just under the pointer, clamped to the window
/// so it stays fully on screen.
fn drag_ghost<'a>(
	state: &'a State,
	task: &'a crate::calendar::Event,
	metrics: &Metrics,
	pointer: iced::Point,
	size: Size,
) -> Element<'a, Message> {
	/// How far the card trails the pointer, so the pointer reads as holding it.
	const OFFSET: f32 = 12.0;
	let width = (size.width - 24.0).clamp(160.0, 320.0);
	let left = (pointer.x + OFFSET)
		.min((size.width - width - 4.0).max(0.0))
		.max(0.0);
	let top = (pointer.y + OFFSET)
		.min((size.height - 80.0).max(0.0))
		.max(0.0);
	container(timeblocking::drag_ghost(state, task, metrics, width))
		.width(Fill)
		.height(Fill)
		.padding(Padding {
			top,
			left,
			..Padding::default()
		})
		.into()
}

/// The seven main navigation items.
const TAB_COUNT: usize = 7;
const TAB_LABELS: [&str; TAB_COUNT] = [
	"Do", "Plan Day", "Routines", "Life", "Note", "More", "Settings",
];
const TAB_SHORT_LABELS: [&str; TAB_COUNT] =
	["Do", "Plan Day", "Rtn", "Life", "Note", "More", "Set"];
const TAB_WIDEST_LABELS: [&str; TAB_COUNT] = [
	"Do",
	"Plan 2 Weeks",
	"Routines",
	"Life",
	"Note",
	"More",
	"Settings",
];
const TAB_WIDEST_SHORT_LABELS: [&str; TAB_COUNT] =
	["Do", "Plan 2 Weeks", "Rtn", "Life", "Note", "More", "Set"];

/// Height of the bottom add-task button.
const PANEL_ACTION_HEIGHT: f32 = 22.0;
/// The part of the footer the add-task button takes; what the app is doing
/// takes the rest.
const PANEL_ADD_SHARE: f32 = 0.3;

/// The Do destination opens the day panel.
pub fn calendar_view_label(_state: &State) -> &'static str {
	"Do"
}

/// The Plan button names the overview currently selected.
pub fn plan_view_label(state: &State) -> &'static str {
	match state.plan_view {
		CalendarView::Day => "Plan Day",
		CalendarView::Week => "Plan Week",
		CalendarView::TwoWeek => "Plan 2 Weeks",
		CalendarView::Month => "Plan Month",
		CalendarView::Schedule => "Plan Schedule",
	}
}

/// Task lists available beneath each planning period.
pub const TASKS_LABELS: [&str; 4] = ["Unfinished", "Inbox", "Today", "Backlog"];

/// The destinations kept inside the More menu.
pub const MORE_LABELS: [&str; 5] = ["Travel", "Habits", "Notes", "People", "Brain dump"];

/// A little breathing room between the menu and the top of the window.
const TAB_TOP_MARGIN: f32 = 6.0;
/// The gap between the top menu and the planning submenus.
const TASKS_ROW_GAP: f32 = 2.0;

/// Average glyph width as a fraction of the font size, with some slack.
const TAB_GLYPH_WIDTH: f32 = 0.6;
/// The smallest tab text that is still worth showing in full.
const TAB_FULL_MIN_SIZE: f32 = 9.0;
const TAB_MIN_SIZE: f32 = 7.0;

/// How the tab row is laid out at a given width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabFit {
	pub short: bool,
	pub size: f32,
	pad: f32,
	gap: f32,
}

/// The labels shown at `width`, so tests and callers agree with `tabs`.
pub fn tab_labels(width: f32) -> [&'static str; TAB_COUNT] {
	if tab_fit(Metrics::of(Size::new(width, 0.0))).short {
		TAB_SHORT_LABELS
	} else {
		TAB_LABELS
	}
}

/// Choose the longest labels and largest text that fit on a single row.
fn tab_fit(metrics: Metrics) -> TabFit {
	let available = (metrics.size.width - metrics.padding * 2.0).max(1.0);
	let max_size = metrics.label();
	let (pad, gap) = if metrics.size.width < 300.0 {
		(1.0, 1.0)
	} else {
		(5.0, metrics.spacing.min(6.0))
	};
	let size_for = |labels: &[&str; TAB_COUNT]| {
		let chars: usize = labels
			.iter()
			.map(|label| label.chars().count())
			.sum::<usize>();
		// Seven tabs, four within-group gaps, and gaps on both sides of each spacer.
		let fixed = 16.0 * pad + 4.0 * gap + 4.0 * (gap * 2.0).max(4.0) + 7.0;
		((available - fixed) / ((chars + 1) as f32 * TAB_GLYPH_WIDTH)).min(max_size)
	};
	let full = size_for(&TAB_WIDEST_LABELS);
	if full >= TAB_FULL_MIN_SIZE.min(max_size) {
		TabFit {
			short: false,
			size: full,
			pad,
			gap,
		}
	} else {
		TabFit {
			short: true,
			size: size_for(&TAB_WIDEST_SHORT_LABELS).max(TAB_MIN_SIZE),
			pad,
			gap,
		}
	}
}

/// A single row of navigation in three groups with space between them:
/// Do/Plan/Routines, then Life, then Note/More/Settings.
/// Labels shrink (and shorten, when very narrow) rather than wrapping.
///
/// Planning destinations show their submenus beneath the main row.
pub(crate) fn tabs<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let fit = tab_fit(*metrics);
	let mut labels = if fit.short {
		TAB_SHORT_LABELS
	} else {
		TAB_LABELS
	};
	labels[1] = plan_view_label(state);
	let planning = state.range_view_active();
	let tasks_open = !planning
		&& matches!(
			state.screen,
			Screen::Overdue | Screen::Inbox | Screen::Today | Screen::Triaged | Screen::Backlog
		);
	let more_active = state.more_menu_open
		|| matches!(
			state.screen,
			Screen::Travel | Screen::Habits | Screen::Notes | Screen::People | Screen::BrainDump
		);

	let view = tab(
		labels[0],
		!planning && state.screen == Screen::Calendar,
		Message::Navigation(NavigationMessage::ScreenSelected(Screen::Calendar)),
		metrics,
		fit,
	);
	let plan_active = state.view_menu_open || planning || tasks_open;
	let plan = container(
		row![
			button(
				text(labels[1])
					.size(fit.size)
					.wrapping(text::Wrapping::None)
					.center()
			)
			.height(tab_height(metrics))
			.padding(Padding::from([0.0, fit.pad]))
			.on_press(plan_view_message(state.plan_view))
			.style(view_half_style(plan_active)),
			iced::widget::rule::vertical(1),
			container(
				button(text("▾").size(fit.size).center())
					.height(tab_height(metrics))
					.padding(Padding::from([0.0, fit.pad + 2.0]))
					.on_press(Message::Navigation(NavigationMessage::ViewMenuToggled))
					.style(view_half_style(plan_active)),
			)
			.id("plan-menu-toggle"),
		]
		.height(tab_height(metrics))
		.align_y(Center),
	)
	.style(view_button_style(plan_active));
	let routines = tab(
		labels[2],
		state.screen == Screen::Routines,
		Message::Navigation(NavigationMessage::ScreenSelected(Screen::Routines)),
		metrics,
		fit,
	);
	let life = tab(
		labels[3],
		state.screen == Screen::Life,
		Message::Navigation(NavigationMessage::ScreenSelected(Screen::Life)),
		metrics,
		fit,
	);
	let note = tab(
		labels[4],
		state.screen == Screen::Notepad,
		Message::Navigation(NavigationMessage::ScreenSelected(Screen::Notepad)),
		metrics,
		fit,
	);
	let more = tab(
		labels[5],
		more_active,
		Message::Navigation(NavigationMessage::MoreMenuToggled),
		metrics,
		fit,
	);
	let settings = tab(
		labels[6],
		state.screen == Screen::Settings,
		Message::Navigation(NavigationMessage::ScreenSelected(Screen::Settings)),
		metrics,
		fit,
	);

	let group_gap = (fit.gap * 2.0).max(4.0);
	let bar = container(
		row![
			row![view, plan, routines].spacing(fit.gap),
			Space::new().width(Fill),
			life,
			Space::new().width(Fill),
			row![note, more, settings].spacing(fit.gap),
		]
		.spacing(group_gap)
		.align_y(Center),
	)
	.padding(Padding {
		left: metrics.padding,
		right: metrics.padding,
		top: TAB_TOP_MARGIN,
		bottom: 0.0,
	})
	.width(Fill);

	// The whole navigation is one card pinned to the top of the window.
	let bar_card = |content: Element<'a, Message>| -> Element<'a, Message> {
		container(content)
			.padding(Padding {
				bottom: TAB_TOP_MARGIN,
				..Padding::ZERO
			})
			.width(Fill)
			.style(|theme: &iced::Theme| container::Style {
				border: iced::Border {
					radius: 0.0.into(),
					..controls::card_surface(theme).border
				},
				..controls::card_surface(theme)
			})
			.into()
	};
	if !tasks_open && !planning {
		return bar_card(bar.into());
	}

	let subtabs = row([
		("Schedule", PlanTab::Schedule, Screen::Calendar),
		("Unfinished", PlanTab::Overdue, Screen::Overdue),
		("Inbox", PlanTab::Inbox, Screen::Inbox),
		("Today", PlanTab::Today, Screen::Today),
		("Backlog", PlanTab::Backlog, Screen::Backlog),
	]
	.into_iter()
	.filter(|(_, target, _)| planning || *target != PlanTab::Schedule)
	.map(|(label, target, screen)| {
		tab(
			label,
			state.screen == screen,
			Message::Navigation(NavigationMessage::PlanTabSelected(target)),
			metrics,
			fit,
		)
	}))
	.spacing(fit.gap);

	bar_card(
		column![
			bar,
			container(subtabs).id("plan-tabs").padding(Padding {
				left: metrics.padding,
				right: metrics.padding,
				top: TASKS_ROW_GAP,
				bottom: 0.0,
			}),
		]
		.width(Fill)
		.into(),
	)
}

/// The height `tabs` takes, including planning submenus when shown.
fn tabs_height(state: &State, metrics: &Metrics) -> f32 {
	let tasks_row = if state.range_view_active()
		|| matches!(
			state.screen,
			Screen::Overdue | Screen::Inbox | Screen::Today | Screen::Triaged | Screen::Backlog
		) {
		TASKS_ROW_GAP + tab_height(metrics)
	} else {
		0.0
	};
	TAB_TOP_MARGIN + tab_height(metrics) + tasks_row + TAB_TOP_MARGIN
}

fn tab_height(metrics: &Metrics) -> f32 {
	if metrics.breakpoint.is_compact() {
		16.0
	} else {
		metrics.icon_button()
	}
}

/// One tab, lit when it is the page on show.
fn tab<'a>(
	label: &'a str,
	active: bool,
	message: Message,
	metrics: &Metrics,
	fit: TabFit,
) -> Element<'a, Message> {
	button(
		text(label)
			.size(fit.size)
			.wrapping(text::Wrapping::None)
			.center(),
	)
	.height(tab_height(metrics))
	.padding(Padding::from([0.0, fit.pad]))
	.on_press(message)
	.style(controls::nav(active))
	.into()
}

/// The shared background, border and shadow behind both halves of the switcher.
fn view_button_style(active: bool) -> impl Fn(&iced::Theme) -> container::Style {
	move |theme: &iced::Theme| {
		let palette = theme.extended_palette();
		let base = if active {
			palette.primary.base
		} else {
			palette.background.weakest
		};

		container::Style {
			background: Some(base.color.into()),
			text_color: Some(base.text),
			border: iced::Border {
				color: palette.background.strong.color.scale_alpha(0.65),
				width: 1.0,
				radius: controls::RADIUS_CONTROL.into(),
			},
			shadow: controls::shadow(theme, 0.1, 1.0, 3.0),
			..Default::default()
		}
	}
}

/// A transparent button half that tints when hovered or pressed, so each side
/// of the switcher responds on its own without breaking the shared shape.
fn view_half_style(active: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
	move |theme: &iced::Theme, status: button::Status| {
		let palette = theme.extended_palette();
		let base = if active {
			palette.primary.base
		} else {
			palette.background.weakest
		};
		let background = match status {
			button::Status::Hovered if active => Some(iced::Color::WHITE.scale_alpha(0.08).into()),
			button::Status::Hovered => Some(iced::Color::BLACK.scale_alpha(0.05).into()),
			button::Status::Pressed => Some(iced::Color::BLACK.scale_alpha(0.10).into()),
			_ => None,
		};

		button::Style {
			background,
			text_color: base.text,
			..Default::default()
		}
	}
}

/// Both halves use the same action for the remembered planning period.
fn plan_view_message(view: CalendarView) -> Message {
	match view {
		CalendarView::Day => Message::Navigation(NavigationMessage::PlanDaySelected),
		CalendarView::Week => Message::Calendar(CalendarMessage::CalendarModeSelected(true)),
		CalendarView::TwoWeek => Message::Calendar(CalendarMessage::CalendarTwoWeekViewSelected),
		CalendarView::Month => Message::Calendar(CalendarMessage::CalendarMonthViewSelected),
		CalendarView::Schedule => Message::Calendar(CalendarMessage::CalendarScheduleViewSelected),
	}
}

/// The Plan menu selects a planning range without toggling it off.
fn view_menu<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill))
		.on_press(Message::Navigation(NavigationMessage::ViewMenuToggled));

	let mut panel = column![].spacing(2.0).width(Fill);
	for (label, view) in [
		("Day", CalendarView::Day),
		("Week", CalendarView::Week),
		("2 Weeks", CalendarView::TwoWeek),
		("Month", CalendarView::Month),
		("Schedule", CalendarView::Schedule),
	] {
		let selected = state.plan_view == view;
		let message = plan_view_message(view);
		panel = panel.push(
			button(text(label).size(metrics.label()).center().width(Fill))
				.width(Fill)
				.on_press(message)
				.style(crate::ui::controls::nav(selected)),
		);
	}

	let width = (metrics.size.width - metrics.padding * 2.0).clamp(96.0, 180.0);
	let card = container(panel)
		.id("plan-menu")
		.width(Length::Fixed(width))
		.padding(6.0)
		.style(crate::ui::controls::surface);

	let aligned = column![
		Space::new().height(tabs_height(state, metrics) + metrics.spacing),
		container(row![iced::widget::opaque(card), Space::new().width(Fill)])
			.padding(Padding {
				left: metrics.padding,
				..Default::default()
			})
			.width(Fill),
	]
	.width(Fill)
	.align_x(Alignment::Start);

	let layers: Vec<Element<'a, Message>> = vec![dismiss.into(), aligned.into()];
	iced::widget::stack(layers).into()
}

/// The More menu: Travel, Habits, Notes, People and Brain dump, in a card below the menu bar.
fn more_menu<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill))
		.on_press(Message::Navigation(NavigationMessage::MoreMenuToggled));

	let mut panel = column![].spacing(2.0).width(Fill);
	for (label, screen) in MORE_LABELS.iter().zip([
		Screen::Travel,
		Screen::Habits,
		Screen::Notes,
		Screen::People,
		Screen::BrainDump,
	]) {
		panel = panel.push(
			button(text(*label).size(metrics.label()).center().width(Fill))
				.width(Fill)
				.on_press(Message::Navigation(NavigationMessage::ScreenSelected(
					screen,
				)))
				.style(crate::ui::controls::nav(state.screen == screen)),
		);
	}

	let width = (metrics.size.width - metrics.padding * 2.0).clamp(96.0, 180.0);
	let card = container(panel)
		.width(Length::Fixed(width))
		.padding(6.0)
		.style(crate::ui::controls::surface);

	let aligned = column![
		Space::new().height(tabs_height(state, metrics) + metrics.spacing),
		container(row![Space::new().width(Fill), iced::widget::opaque(card)])
			.padding(Padding {
				right: metrics.padding,
				..Default::default()
			})
			.width(Fill),
	]
	.width(Fill)
	.align_x(Alignment::Start);

	let layers: Vec<Element<'a, Message>> = vec![dismiss.into(), aligned.into()];
	iced::widget::stack(layers).into()
}

/// The foot of the panel's left side: what is happening in the background,
/// or "Logs" when nothing is. It opens the log either way.
/// While sleep is being timed, a strip above the footer with how long it has
/// run, the wake-up times for 6 to 9 hours and the target, and Finish sleep.
fn sleep_strip<'a>(state: &State) -> Option<Element<'a, Message>> {
	let event = state.calendar.active_sleep()?;
	let record = event.details.sleep.as_ref()?;
	let start = record.actual_start?;
	let format = state.config.appearance.time_format;
	let elapsed = (crate::clock::now() - start).num_minutes();
	let finish = button(text("Finish sleep").size(12))
		.padding([4, 8])
		.style(crate::ui::controls::secondary)
		.on_press(Message::Settings(
			crate::app::SettingsMessage::SleepTimerPressed(event.id.clone()),
		));
	let details = column![
		text(format!(
			"Sleeping since {} · {}",
			crate::calendar::time_format::clock(start, format),
			crate::calendar::sleep::minutes_label(elapsed)
		))
		.size(12),
		text(format!(
			"Wake up: {}",
			crate::calendar::sleep::predictions_line(event, format)
		))
		.size(11)
		.style(text::secondary),
	]
	.spacing(2)
	.width(Fill);
	Some(
		container(row![details, finish].spacing(8).align_y(Alignment::Center))
			.id("sleep-status")
			.width(Fill)
			.padding([4, 8])
			.style(crate::ui::controls::surface)
			.into(),
	)
}

fn activity_button<'a>(state: &State) -> Element<'a, Message> {
	let label = text(state.activity.label().to_owned())
		.size(12)
		.wrapping(text::Wrapping::None);
	let label = if state.activity.busy() {
		label
	} else {
		label.style(text::secondary)
	};
	container(
		button(
			container(label)
				.clip(true)
				.width(Fill)
				.height(Fill)
				.align_y(Alignment::Center),
		)
		.width(Fill)
		.height(PANEL_ACTION_HEIGHT)
		.padding([0, 6])
		.style(crate::ui::controls::square)
		.on_press(Message::Navigation(NavigationMessage::ActivityLogToggled)),
	)
	.id("activity-status")
	.width(Fill)
	.into()
}

/// The log of background work, in a card rising from the foot of the panel.
fn activity_log<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
	let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill))
		.on_press(Message::Navigation(NavigationMessage::ActivityLogToggled));
	let time_format = state.config.appearance.time_format;

	let mut entries = column![].spacing(6.0).width(Fill);
	let mut empty = true;
	for entry in state.activity.log() {
		empty = false;
		let time = crate::calendar::time_format::clock(entry.at, time_format);
		let what = if entry.repeats > 1 {
			format!("{} (×{})", entry.text, entry.repeats)
		} else {
			entry.text.clone()
		};
		let what = text(what).size(metrics.label()).width(Fill);
		let what = if entry.failed {
			what.style(text::danger)
		} else {
			what
		};
		entries = entries.push(
			row![
				text(time)
					.size(metrics.label() - 1.0)
					.style(text::secondary)
					.wrapping(text::Wrapping::None),
				what,
			]
			.spacing(8.0),
		);
	}
	if empty {
		entries = entries.push(
			text("Nothing has happened yet.")
				.size(metrics.label())
				.style(text::secondary),
		);
	}

	let panel = column![
		text("Logs").size(metrics.label() + 2.0).font(iced::Font {
			weight: iced::font::Weight::Bold,
			..iced::Font::DEFAULT
		}),
		container(crate::ui::controls::scrollable(entries).width(Fill))
			.max_height((metrics.size.height * 0.6).max(120.0)),
	]
	.spacing(8.0)
	.width(Fill);

	let card = container(panel)
		.id("activity-log")
		.width(Fill)
		.padding(10.0)
		.style(crate::ui::controls::surface);

	let aligned = column![
		Space::new().height(Fill),
		container(iced::widget::opaque(card))
			.padding(Padding {
				bottom: PANEL_ACTION_HEIGHT,
				..Default::default()
			})
			.width(Fill),
	]
	.width(Fill)
	.height(Fill);

	let layers: Vec<Element<'a, Message>> = vec![dismiss.into(), aligned.into()];
	iced::widget::stack(layers).into()
}
