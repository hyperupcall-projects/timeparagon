//! Independent visibility controls for the kinds of events on TimeSlay.

use super::{Event, sleep};
use crate::config::CalendarSettings;
use iced_core::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeSlaySource {
    Events,
    Tasks,
    TimeBlocks,
    Habits,
    Sleep,
}

impl TimeSlaySource {
    pub const ALL: [Self; 5] = [
        Self::Events,
        Self::Tasks,
        Self::TimeBlocks,
        Self::Habits,
        Self::Sleep,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Events => "timeslay:events",
            Self::Tasks => "timeslay:tasks",
            Self::TimeBlocks => "timeslay:time-blocks",
            Self::Habits => "timeslay:habits",
            Self::Sleep => sleep::SOURCE_ID,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Events => "Events",
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
            Self::Events => settings.event_color(),
            Self::Tasks => settings.task_event_color(),
            Self::TimeBlocks => settings.time_block_color(),
            Self::Habits => settings.habit_color(),
            Self::Sleep => settings.sleep_color(),
        }
    }

    /// The custom color set for this kind, if the user chose one.
    pub fn custom_color(self, settings: &CalendarSettings) -> Option<Color> {
        match self {
            Self::Events => settings.event_color,
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
            Self::Events => settings.set_event_color(color),
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
        let scheduled_on_calendar = event
            .details
            .scheduled
            .as_ref()
            .is_some_and(|link| link.calendar_id == app_calendar);
        if !on_calendar && !scheduled_on_calendar {
            return None;
        }
        Some(if event.is_task() || event.details.scheduled.is_some() {
            Self::Tasks
        } else if event.details.habit {
            Self::Habits
        } else if !event.all_day {
            // Plain timed entries on the app calendar are time blocks.
            Self::TimeBlocks
        } else {
            Self::Events
        })
    }
}
