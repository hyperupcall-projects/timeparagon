//! Dragging the edges of the side panel and of the floating overlay beside it.
//!
//! The panel is resized from its inner edge only. With the overlay open, that
//! edge is shared, so dragging it moves the boundary: the panel and the
//! overlay pane next to it change by the same amount in opposite directions.
//! The overlay's own far edge resizes the overlay pane on that side.
//!
//! The panel never grows past a third of the screen, and the overlay never
//! grows past the screen edge, less the margins on either side.

use super::*;

/// One of the panes the floating overlay is made of, nearest the panel first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
	/// The day calendar shown while planning.
	PlanCalendar,
	TimeBlock,
	/// The shared event/task details, also used for contacts and the map.
	Detail,
	/// The week or schedule range view.
	Week,
	/// The month or two-week range view.
	Month,
	Habit,
	Life,
}

/// Which edge a drag handle sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
	/// The panel's inner edge, with no overlay beside it.
	Panel,
	/// Between the panel and the overlay. Moves both.
	Boundary,
	/// The overlay's far edge.
	Overlay,
}

/// A drag under way, with the widths it started from.
///
/// Positions are measured from the screen edge each window is anchored to,
/// which does not move while that window grows or shrinks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneResize {
	edge: ResizeEdge,
	origin: f32,
	panel: f32,
	/// The overlay pane the drag changes, its width, and how narrow it may go.
	pane: Option<(Pane, f32, f32)>,
	/// The rest of the overlay, which the drag leaves alone.
	others: f32,
}

/// Room left for the panel and overlay once the screen margins are taken:
/// one between the screen edge and the panel, one between the panel and the
/// overlay, and one beyond the overlay.
const MARGINS: f32 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Limits {
	screen: Option<f32>,
	margin: f32,
}

impl Limits {
	/// The widest the panel may be: a third of the screen.
	fn panel(self) -> f32 {
		self.screen
			.map_or(config::ViewSettings::MAX_WIDTH, |screen| screen / 3.0)
			.clamp(
				config::ViewSettings::MIN_WIDTH,
				config::ViewSettings::MAX_WIDTH,
			)
	}

	/// The widest the whole overlay may be beside a panel this wide.
	fn overlay(self, panel: f32) -> f32 {
		self.screen.map_or(f32::INFINITY, |screen| {
			screen - panel - self.margin * MARGINS
		})
	}
}

/// The panel width and overlay pane width a drag of `delta` asks for.
fn resized(drag: &PaneResize, delta: f32, limits: Limits) -> (f32, Option<f32>) {
	let min = config::ViewSettings::MIN_WIDTH;
	let max = config::ViewSettings::MAX_WIDTH;
	match (drag.edge, drag.pane) {
		(ResizeEdge::Boundary, Some((_, pane, pane_min))) => {
			// The total stays put, so the screen limit cannot be crossed.
			let total = drag.panel + pane;
			let panel = (drag.panel + delta)
				.min(limits.panel())
				.min(total - pane_min)
				.max(min);
			(panel, Some((total - panel).clamp(pane_min, max)))
		}
		(ResizeEdge::Overlay, Some((_, pane, pane_min))) => {
			let room = limits.overlay(drag.panel) - drag.others;
			(
				drag.panel,
				Some((pane + delta).min(room).min(max).max(pane_min)),
			)
		}
		_ => (
			(drag.panel + delta).min(limits.panel()).max(min),
			drag.pane.map(|(_, pane, _)| pane),
		),
	}
}

impl State {
	/// The panes of the floating overlay, nearest the panel first. Empty when
	/// the overlay is closed.
	pub fn overlay_panes(&self) -> Vec<Pane> {
		let visible =
			self.floating_editors() && self.main_window.is_some() && !self.panel_expected_close;
		let mut panes = Vec::new();
		if visible && self.people_panel_open() {
			panes.push(Pane::Detail);
		} else if visible && self.life_panel_open() {
			panes.push(Pane::Life);
		} else if visible && self.habit_panel_open() {
			panes.push(Pane::Habit);
			if self.calendar.open_event.is_some() {
				panes.push(Pane::Detail);
			}
		} else if self.range_view_active()
			&& self.main_window.is_some()
			&& !self.panel_expected_close
		{
			panes.push(if self.monthly() || self.two_weekly() {
				Pane::Month
			} else {
				Pane::Week
			});
		} else if visible {
			if self.day_plan_active() {
				panes.push(Pane::PlanCalendar);
			}
			if self.timeblocking.block.is_some() {
				panes.push(Pane::TimeBlock);
			}
			if self.details_pane_open {
				panes.push(Pane::Detail);
			}
		}
		// The map opens in the overlay even when nothing else would have it
		// open, as when Home is chosen from the settings.
		if panes.is_empty() && self.map_in_editor_window() {
			panes.push(Pane::Detail);
		}
		panes
	}

	pub fn pane_width(&self, pane: Pane) -> f32 {
		let views = &self.config.views;
		match pane {
			Pane::PlanCalendar => views.plan_calendar_width(self.calendar_width()),
			Pane::TimeBlock => views.time_block_width(),
			Pane::Detail => views.detail_width(),
			Pane::Week | Pane::Month => self.weekly_window_width(),
			Pane::Habit => views.habit_width(ui::habits::PANEL_WIDTH),
			Pane::Life => views.life_width(ui::life::PANEL_WIDTH),
		}
	}

	/// The width of the whole overlay.
	pub fn overlay_width(&self) -> f32 {
		self.overlay_panes()
			.into_iter()
			.map(|pane| self.pane_width(pane))
			.sum()
	}

	/// The narrowest a pane may be dragged. The range views carry the screen
	/// margins in their width as well.
	fn pane_min(&self, pane: Pane) -> f32 {
		match pane {
			Pane::Week | Pane::Month => {
				config::ViewSettings::MIN_WIDTH + self.screen_margin_width()
			}
			_ => config::ViewSettings::MIN_WIDTH,
		}
	}

	fn set_pane_width(&mut self, pane: Pane, width: f32) {
		let width = width.clamp(
			config::ViewSettings::MIN_WIDTH,
			config::ViewSettings::MAX_WIDTH,
		);
		let margins = self.screen_margin_width();
		let views = &mut self.config.views;
		match pane {
			Pane::PlanCalendar => views.plan_calendar_width = Some(width),
			Pane::TimeBlock => views.time_block_width = width,
			Pane::Detail => views.detail_width = width,
			Pane::Week => {
				views.calendar_week_width =
					Some((width - margins).max(config::ViewSettings::MIN_WIDTH));
			}
			Pane::Month => {
				views.calendar_month_width =
					Some((width - margins).max(config::ViewSettings::MIN_WIDTH));
			}
			Pane::Habit => views.habit_width = Some(width),
			Pane::Life => views.life_width = Some(width),
		}
	}

	/// The handle the panel's inner edge offers: it moves the boundary when
	/// the overlay is beside it.
	pub fn panel_edge(&self) -> ResizeEdge {
		if self.editor_window.is_some() && !self.overlay_panes().is_empty() {
			ResizeEdge::Boundary
		} else {
			ResizeEdge::Panel
		}
	}

	fn resize_limits(&self) -> Limits {
		Limits {
			screen: self
				.monitor_size
				.map(|size| size.width)
				.filter(|width| width.is_finite() && *width > 0.0),
			margin: self.screen_margin(),
		}
	}
}

pub(super) fn started(state: &mut State, edge: ResizeEdge, origin: f32) -> Task<Message> {
	let panes = state.overlay_panes();
	let pane = match edge {
		ResizeEdge::Panel => None,
		ResizeEdge::Boundary => panes.first().copied(),
		ResizeEdge::Overlay => panes.last().copied(),
	}
	.map(|pane| (pane, state.pane_width(pane), state.pane_min(pane)));
	state.pane_resize = Some(PaneResize {
		edge,
		origin,
		panel: state.calendar_width(),
		pane,
		others: state.overlay_width() - pane.map_or(0.0, |(_, width, _)| width),
	});
	Task::none()
}

pub(super) fn moved(state: &mut State, position: f32) -> Task<Message> {
	let Some(drag) = state.pane_resize else {
		return Task::none();
	};
	let (panel, pane) = resized(&drag, position - drag.origin, state.resize_limits());
	if let (Some((which, _, _)), Some(width)) = (drag.pane, pane) {
		state.set_pane_width(which, width);
	}
	// The overlay follows in `update`; only the panel needs resizing here.
	let resize = if (panel - state.calendar_width()).abs() >= 0.5 {
		state.config.views.calendar_width = Some(panel);
		apply_view_sizes(state)
	} else {
		Task::none()
	};
	Task::batch([resize, queue_save(state)])
}

pub(super) fn ended(state: &mut State) -> Task<Message> {
	if state.pane_resize.take().is_some() {
		queue_save(state)
	} else {
		Task::none()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const LIMITS: Limits = Limits {
		screen: Some(1800.0),
		margin: 10.0,
	};

	fn drag(edge: ResizeEdge, panel: f32, pane: Option<f32>, others: f32) -> PaneResize {
		PaneResize {
			edge,
			origin: 0.0,
			panel,
			pane: pane.map(|width| (Pane::Detail, width, config::ViewSettings::MIN_WIDTH)),
			others,
		}
	}

	#[test]
	fn the_panel_stops_at_a_third_of_the_screen() {
		let drag = drag(ResizeEdge::Panel, 300.0, None, 0.0);
		assert_eq!(resized(&drag, 100.0, LIMITS).0, 400.0);
		assert_eq!(resized(&drag, 1000.0, LIMITS).0, 600.0);
		assert_eq!(
			resized(&drag, -1000.0, LIMITS).0,
			config::ViewSettings::MIN_WIDTH
		);
	}

	#[test]
	fn the_boundary_trades_width_between_panel_and_overlay() {
		let drag = drag(ResizeEdge::Boundary, 300.0, Some(400.0), 0.0);
		assert_eq!(resized(&drag, 50.0, LIMITS), (350.0, Some(350.0)));
		assert_eq!(resized(&drag, -50.0, LIMITS), (250.0, Some(450.0)));
	}

	#[test]
	fn the_boundary_still_stops_the_panel_at_a_third_of_the_screen() {
		let drag = drag(ResizeEdge::Boundary, 300.0, Some(800.0), 0.0);
		assert_eq!(resized(&drag, 1000.0, LIMITS), (600.0, Some(500.0)));
	}

	#[test]
	fn the_boundary_keeps_the_overlay_pane_usable() {
		let drag = drag(ResizeEdge::Boundary, 400.0, Some(250.0), 0.0);
		let (panel, pane) = resized(&drag, 500.0, LIMITS);
		assert_eq!(pane, Some(config::ViewSettings::MIN_WIDTH));
		assert_eq!(panel, 650.0 - config::ViewSettings::MIN_WIDTH);
	}

	#[test]
	fn the_overlay_stops_at_the_screen_edge_less_the_margins() {
		let drag = drag(ResizeEdge::Overlay, 400.0, Some(360.0), 300.0);
		assert_eq!(resized(&drag, 40.0, LIMITS), (400.0, Some(400.0)));
		// 1800 - 400 - 3 * 10 - 300 leaves 1070 for the pane.
		assert_eq!(resized(&drag, 5000.0, LIMITS), (400.0, Some(1070.0)));
		assert_eq!(
			resized(&drag, -5000.0, LIMITS),
			(400.0, Some(config::ViewSettings::MIN_WIDTH))
		);
	}

	#[test]
	fn without_a_screen_size_only_the_usual_bounds_apply() {
		let limits = Limits {
			screen: None,
			margin: 10.0,
		};
		let drag = drag(ResizeEdge::Overlay, 400.0, Some(360.0), 0.0);
		assert_eq!(resized(&drag, 640.0, limits).1, Some(1000.0));
	}
}
