//! Independent visibility controls for the kinds of events on TimeParagon.

use super::{Event, sleep};
use crate::config::CalendarSettings;
use iced_core::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeParagonSource {
	Tasks,
	TimeBlocks,
	Habits,
	Sleep,
}

impl TimeParagonSource {
	pub const ALL: [Self; 4] = [Self::Tasks, Self::TimeBlocks, Self::Habits, Self::Sleep];

	pub const fn id(self) -> &'static str {
		match self {
			Self::Tasks => "timeparagon:tasks",
			Self::TimeBlocks => "timeparagon:time-blocks",
			Self::Habits => "timeparagon:habits",
			Self::Sleep => sleep::SOURCE_ID,
		}
	}

	pub fn label(self) -> &'static str {
		match self {
			Self::Tasks => "Tasks",
			Self::TimeBlocks => "Time Blocks",
			Self::Habits => "Habits",
			Self::Sleep => "Sleep",
		}
	}

	pub fn from_id(id: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|source| source.id() == id)
	}

	pub fn color(self, settings: &CalendarSettings) -> Color {
		match self {
			Self::Tasks => settings.task_event_color(),
			Self::TimeBlocks => settings.time_block_color(),
			Self::Habits => settings.habit_color(),
			Self::Sleep => settings.sleep_color(),
		}
	}

	/// The custom color set for this kind, if the user chose one.
	pub fn custom_color(self, settings: &CalendarSettings) -> Option<Color> {
		match self {
			Self::Tasks => settings.task_event_color,
			Self::TimeBlocks => settings.time_block_color,
			Self::Habits => settings.habit_color,
			Self::Sleep => settings.sleep_color,
		}
		.map(|color| color.0)
	}

	/// Sets this kind's color, or clears it back to the built-in default.
	pub fn set_color(self, settings: &mut CalendarSettings, color: Option<Color>) {
		match self {
			Self::Tasks => settings.set_task_event_color(color),
			Self::TimeBlocks => settings.set_time_block_color(color),
			Self::Habits => settings.set_habit_color(color),
			Self::Sleep => settings.set_sleep_color(color),
		}
	}

	pub fn of(event: &Event, app_calendar: &str) -> Option<Self> {
		if event.details.sleep.is_some() {
			return Some(Self::Sleep);
		}
		let on_calendar = event.calendar_id == app_calendar;
		if !on_calendar {
			return None;
		}
		Some(if event.is_task() {
			Self::Tasks
		} else if event.details.habit {
			Self::Habits
		} else if !event.all_day {
			// Plain timed entries on the app calendar are time blocks.
			Self::TimeBlocks
		} else {
			// Plain all-day entries remain regular Google events, including
			// any saved on the app calendar by an older version.
			return None;
		})
	}
}
