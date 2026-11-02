//! Google Calendar day view, drawn to resemble the web UI.

mod block_text;
mod day;
mod day_canvas;
mod details;
mod draft;
mod editor;
mod header;
mod menu;
mod month;
mod pointer;
#[cfg(test)]
mod sleep_tests;
#[cfg(test)]
mod tests;
mod two_week;
mod week;

use block_text::*;
pub use day::*;
use day_canvas::*;
pub use details::*;
pub(crate) use draft::*;
pub use editor::*;
pub(crate) use header::*;
use menu::*;
pub use month::*;
pub use pointer::*;
pub use two_week::*;
pub use week::*;

use crate::calendar::time_format::TimeFormat;
use chrono::{DateTime, Datelike, Local, NaiveDate};
use iced::font::Weight;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Style, Text};
use iced::widget::{
    Space, button, canvas as canvas_widget, column, container, mouse_area, responsive, row,
    scrollable, stack, text,
};
use iced::{
    Alignment, Color, Element, Fill, Font, Length, Pixels, Point, Rectangle, Renderer, Size, Theme,
    mouse,
};

use crate::alarms;
use crate::app::{
    CalendarMessage, EventDraftMessage, EventEditMessage, EventMessage, SettingsMessage,
    SystemMessage, TaskMessage,
};
use crate::app::{Message, State};
use crate::calendar::colors::{self, on_color};
use crate::calendar::layout::{self, Span};
use crate::calendar::{self, Event};

use super::layout::Metrics;

const GUTTER: f32 = 52.0;
const COMPACT_GUTTER: f32 = 40.0;
const EVENT_GAP: f32 = 2.0;
const EVENT_RADIUS: f32 = 6.0;
const NEEDLE_RADIUS: f32 = 5.5;

/// The gap between a block's left edge and the text inside it.
const BLOCK_PADDING: f32 = 6.0;
/// The gap between a block's right edge and the text inside it.
const BLOCK_PADDING_RIGHT: f32 = 4.0;
/// The gap above a block's first line of text.
const BLOCK_TOP: f32 = 3.0;
/// The gap between a block's title and the time label under it.
const BLOCK_GAP: f32 = 2.0;
/// The mark that says a title was cut short to fit its block.
const ELLIPSIS: &str = "…";
/// The width a paragraph is measured at, wide enough that nothing wraps.
const MEASURE_WIDTH: f32 = 100_000.0;

/// How close to a block's top or bottom edge a press has to land to resize it
/// rather than move it. The middle of the block is always the move handle.
const EDGE_GRAB: f32 = 8.0;
/// Dragged times snap to this many minutes, the way Google Calendar's own day
/// view does. The event keeps its offset, so a 9:07 meeting moves in 15-minute
/// steps rather than jumping to the quarter hour.
const SNAP_MINUTES: i32 = 15;
/// The shortest an event can be dragged to.
const MIN_DURATION: i32 = SNAP_MINUTES;
/// The length a new event gets from a plain click, matching Google Calendar's
/// one-hour default.
const DEFAULT_DURATION: i32 = 60;

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let grid = day_grid(state, metrics, crate::clock::now());

    // The pointer watch records where the click landed without publishing a
    // message, so scrolling stays on the widget's own path.
    let mut body = column![].spacing(metrics.spacing).height(Fill).width(Fill);
    if let Some(all_day) = all_day_row(state, metrics) {
        body = body.push(all_day);
    }
    let body = body.push(grid);

    let mut content = column![heading_with_alarm_hint(state, metrics, false), body]
        .spacing(metrics.spacing)
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill);

    // The bar comes last, so it appearing and going leaves the grid alone.
    if let Some(dismiss) = dismiss_bar(state, metrics) {
        content = content.push(dismiss);
    }

    // The root stays a stack whether or not a card is open. Swapping the root
    // widget throws away the day grid's scroll state, which jumps the bar.
    // Event details are no longer an overlay: they are a pane beside the view.
    let overlay = if state.range_view_active() {
        Space::new().width(0).height(0).into()
    } else if state
        .calendar
        .draft
        .as_ref()
        .is_some_and(|d| !d.editor_open)
    {
        draft_overlay(state, metrics)
    } else if state.alarms.editor.is_some() {
        super::alarms::editor_overlay(state, metrics)
    } else if state.calendar.month_open {
        month_overlay(state, metrics)
    } else if state.calendar.menu_open {
        menu_overlay(state, metrics)
    } else {
        iced::widget::Space::new().width(0.0).height(0.0).into()
    };

    pointer_watch(stack![content, overlay])
}

fn habit_occurrence<'a>(
    state: &'a State,
    event: &'a Event,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let format = state.config.appearance.time_format;
    let completed = if event.completed {
        match event.details.habit_completed_at {
            Some(when) => format!(
                "Completed at {}{}",
                calendar::time_format::clock(when, format),
                if when.date_naive() != event.start.date_naive() {
                    format!(" ({})", when.format("%b %-d"))
                } else {
                    String::new()
                }
            ),
            None => "Completed · time unknown".to_owned(),
        }
    } else {
        "Not completed".to_owned()
    };
    let mut toggle = button(
        text(if event.completed {
            "Mark incomplete"
        } else {
            "Mark complete"
        })
        .size(metrics.label() - 1.0),
    )
    .padding([2, 4])
    .style(crate::ui::controls::secondary);
    if !state.habits.completing.contains(&event.id) {
        toggle = toggle.on_press(Message::Habits(crate::habits::Action::ToggleCompleted(
            event.id.clone(),
        )));
    }
    container(
        column![
            text(format!(
                "Scheduled {} – {}",
                calendar::time_format::clock(event.start, format),
                calendar::time_format::clock(event.end, format)
            ))
            .size(metrics.label() - 1.0),
            text(completed).size(metrics.label() - 1.0),
            toggle,
        ]
        .spacing(2),
    )
    .padding(3)
    .width(Fill)
    .style(crate::ui::controls::card_surface)
    .into()
}
