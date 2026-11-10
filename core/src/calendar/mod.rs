//! Google Calendar: calendars, today's events, and the bits of OAuth the UI
//! should not have to know about.

pub mod api;
pub mod calendar_tasks;
pub mod client;
pub mod colors;
pub(crate) mod day_cache;
pub mod deadline;
pub mod event_options;
pub mod first_day;
pub mod google;
pub mod layout;
pub mod meeting;
pub mod oauth;
pub mod operations;
pub mod projects;
pub mod recurring_tasks;
pub mod schedule;
pub mod scheduled;
pub mod search;
pub mod sleep;
pub mod sources;
pub mod task_metadata;
pub mod task_sort;
pub mod time_format;

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Datelike, Local, Months, NaiveDate, Offset, TimeZone, Timelike, Utc};
use iced_core::Color;
use serde::{Deserialize, Serialize};

use crate::config::CalendarSettings;
pub use client::GoogleClient;
use google::GoogleError;

/// Google Calendar in the browser, opened on the day view.
pub const WEB_URL: &str = "https://calendar.google.com/calendar/r/day";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Connection {
    #[default]
    SignedOut,
    SignedIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncState {
    #[default]
    Idle,
    Connecting,
    Syncing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Calendar {
    pub id: String,
    pub name: String,
    pub color: Color,
    pub primary: bool,
    pub selected_by_google: bool,
}

impl Calendar {
    /// Whether this is a Google Tasks list rather than a calendar. Task lists
    /// are synthetic calendars carrying a `tasks:` id.
    pub fn is_task_list(&self) -> bool {
        self.id.starts_with(ids::TASK_LIST)
    }

    pub fn is_app_calendar(&self, settings: &CalendarSettings) -> bool {
        !self.is_task_list()
            && (settings.app_calendar.as_deref() == Some(self.id.as_str())
                || CalendarSettings::is_app_calendar_name(&self.name))
    }

    /// Lists reserved for the app's task workflow are not calendar sources.
    pub fn is_managed_task_list(&self, settings: &CalendarSettings) -> bool {
        self.is_task_list()
            && ([
                scheduled::DEFAULT_LIST,
                CalendarSettings::DEFAULT_TRIAGED_LIST,
                CalendarSettings::DEFAULT_BACKLOG_LIST,
            ]
            .iter()
            .any(|name| self.name.trim().eq_ignore_ascii_case(name))
                || [
                    &settings.inbox_list,
                    &settings.triaged_list,
                    &settings.backlog_list,
                ]
                .into_iter()
                .flatten()
                .any(|id| ids::bare_list(id) == ids::bare_list(&self.id)))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub calendar_id: String,
    pub title: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub all_day: bool,
    #[serde(with = "crate::hex::serde_color")]
    pub color: Color,
    /// A finished Google Task, including its retained calendar representation.
    pub completed: bool,
    pub details: Details,
}

impl Event {
    /// Apply a local calendar color while preserving explicit event colors and
    /// the separate TimeSlay task, habit and time-block color settings.
    pub fn display_color(&self, settings: &CalendarSettings) -> Color {
        if self.details.sleep.is_some() {
            return settings.sleep_color();
        }
        if self.details.habit {
            return self
                .details
                .color_id
                .as_deref()
                .and_then(colors::event)
                .unwrap_or_else(|| settings.habit_color());
        }
        if self.details.color_id.is_none()
            && !self.is_task()
            && !self.is_calendar_task()
            && !self.all_day
            && settings.app_calendar.as_deref() == Some(self.calendar_id.as_str())
        {
            return settings.time_block_color();
        }
        if !self.is_task()
            && !self.is_calendar_task()
            && !self.details.habit
            && self.details.color_id.is_none()
        {
            if self.all_day && settings.app_calendar.as_deref() == Some(self.calendar_id.as_str()) {
                return settings.event_color();
            }
            if self.all_day || settings.app_calendar.as_deref() != Some(self.calendar_id.as_str()) {
                return settings
                    .calendar_colors
                    .get(&self.calendar_id)
                    .map_or(self.color, |color| color.0);
            }
        }
        self.color
    }

    /// Source used for calendar visibility and overlap ordering.
    pub fn source_id(&self) -> &str {
        if self.details.sleep.is_some() {
            sleep::SOURCE_ID
        } else {
            &self.calendar_id
        }
    }

    /// A task, either undated work in Google Tasks or a dated Calendar event.
    pub fn is_task(&self) -> bool {
        self.details.task_data.is_some()
            || self.id.starts_with(ids::TASK)
            || self.calendar_id.starts_with(ids::TASK_LIST)
    }

    pub fn is_calendar_task(&self) -> bool {
        self.is_task()
            && !self.id.starts_with(ids::TASK)
            && !self.calendar_id.starts_with(ids::TASK_LIST)
    }

    /// Whether changing this item needs a recurrence scope.
    pub fn is_recurring(&self) -> bool {
        self.details.recurring_event_id.is_some() || !self.details.recurrence.is_empty()
    }

    /// The Google Task this is, when it is one.
    pub fn task_ref(&self) -> Option<ids::TaskRef<'_>> {
        ids::TaskRef::of(self)
    }

    /// What this item is to the app.
    ///
    /// Google stores everything as events or tasks; the app tells its own
    /// kinds apart by their metadata. Sleep, habits and dated tasks all live
    /// on the app's calendar. `calendars` lets the app's calendar
    /// be recognized by name before its ID has been recorded in `settings`.
    pub fn kind(&self, settings: &CalendarSettings, calendars: &[Calendar]) -> EventKind {
        if self.details.sleep.is_some() {
            EventKind::Sleep
        } else if self.details.habit {
            EventKind::Habit
        } else if self.is_calendar_task() {
            EventKind::ScheduledTask
        } else if self.is_task() {
            EventKind::Task
        } else if settings.app_calendar.as_deref() == Some(self.calendar_id.as_str())
            || calendars.iter().any(|calendar| {
                calendar.id == self.calendar_id
                    && CalendarSettings::is_app_calendar_name(&calendar.name)
            })
        {
            if self.all_day {
                EventKind::AppEvent
            } else {
                EventKind::TimeBlock
            }
        } else {
            EventKind::External
        }
    }
}

/// What an item on the calendar is to the app. See [`Event::kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    /// A night, written by the sleep planner.
    Sleep,
    /// An occurrence of a habit's event.
    Habit,
    /// A task's time on the calendar: an event linked to a Google Task.
    ScheduledTask,
    /// A Google Task.
    Task,
    /// A timed event on the app's calendar, holding tasks.
    TimeBlock,
    /// An all-day event on the app's calendar.
    AppEvent,
    /// Anything on another calendar.
    External,
}

impl EventKind {
    /// The name the interface shows on the kind's chip.
    pub fn label(self) -> Option<&'static str> {
        Some(match self {
            Self::Sleep => "Sleep",
            Self::Habit => "Habit",
            Self::ScheduledTask | Self::Task => "Task",
            Self::TimeBlock => "Time Block",
            Self::AppEvent | Self::External => "Event",
        })
    }

    /// The kind's chip: its name, in the color configured for it. Plain
    /// events on other calendars keep their event color.
    pub fn chip(
        self,
        event: &Event,
        settings: &CalendarSettings,
        _calendars: &[Calendar],
    ) -> (String, Color) {
        let color = match self {
            Self::Sleep => settings.sleep_color(),
            Self::Habit => settings.habit_color(),
            Self::ScheduledTask => settings.task_event_color(),
            Self::Task => settings
                .task_color(&event.calendar_id)
                .unwrap_or(colors::TASK),
            Self::TimeBlock => settings.time_block_color(),
            Self::AppEvent => settings.event_color(),
            Self::External => event.display_color(settings),
        };
        (self.label().unwrap_or_default().to_owned(), color)
    }
}

/// How Google Tasks appear among calendar events.
///
/// A task becomes an [`Event`] whose ID is `task:` followed by the task's ID
/// and whose calendar ID is `tasks:` followed by its list's ID. These are the
/// only places that know that encoding.
pub mod ids {
    use super::Event;

    /// The prefix of a task's event ID.
    pub const TASK: &str = "task:";
    /// The prefix of a task list's calendar ID.
    pub const TASK_LIST: &str = "tasks:";

    /// A Google Task: its list and its own ID, without the prefixes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct TaskRef<'a> {
        pub list: &'a str,
        pub task: &'a str,
    }

    impl<'a> TaskRef<'a> {
        /// The task an event stands for, when both of its IDs say it is one.
        pub fn of(event: &'a Event) -> Option<Self> {
            Some(Self {
                list: event.calendar_id.strip_prefix(TASK_LIST)?,
                task: event.id.strip_prefix(TASK)?,
            })
        }

        pub fn event_id(self) -> String {
            event_id(self.task)
        }

        pub fn calendar_id(self) -> String {
            calendar_id(self.list)
        }
    }

    /// The event ID of the task with Google ID `task`.
    pub fn event_id(task: &str) -> String {
        format!("{TASK}{}", bare_task(task))
    }

    /// The calendar ID of the task list with Google ID `list`.
    pub fn calendar_id(list: &str) -> String {
        format!("{TASK_LIST}{}", bare_list(list))
    }

    /// A task's Google ID from its event ID, or the ID itself if it has no
    /// prefix.
    pub fn bare_task(id: &str) -> &str {
        id.trim_start_matches(TASK)
    }

    /// A list's Google ID from its calendar ID, or the ID itself if it has no
    /// prefix.
    pub fn bare_list(id: &str) -> &str {
        id.trim_start_matches(TASK_LIST)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn task_ids_round_trip_and_tolerate_either_form() {
            let mut event = crate::calendar::sample_snapshot().events.remove(0);
            event.id = event_id("abc");
            event.calendar_id = calendar_id("tasks:list");
            assert_eq!(event.id, "task:abc");
            assert_eq!(event.calendar_id, "tasks:list");
            let task = TaskRef::of(&event).unwrap();
            assert_eq!((task.list, task.task), ("list", "abc"));
            assert_eq!(task.event_id(), event.id);
            assert_eq!(task.calendar_id(), event.calendar_id);
            assert_eq!(bare_task("task:abc"), "abc");
            assert_eq!(bare_task("abc"), "abc");
            event.calendar_id = "primary".into();
            assert!(TaskRef::of(&event).is_none());
        }
    }
}

/// Normalize the human notes written to a Calendar event.
pub fn event_description(notes: &str) -> String {
    notes.trim_end().to_owned()
}

/// A time change that has been applied to the view but not yet confirmed by
/// Google.
///
/// The event is drawn lighter while the write is in flight, and the times here
/// are what the chip goes back to if Google refuses it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PendingTime {
    /// The times shown while the write is in flight.
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    /// The times to put back if the write fails.
    pub previous_start: DateTime<Local>,
    pub previous_end: DateTime<Local>,
}

/// What a new event from the add-event popup is.
///
/// An `Event` is an ordinary calendar entry on the chosen calendar. A
/// `TimeBlock` is written to the app's single TimeSlay calendar and is
/// labelled and colored as a block, the same as one created anywhere else.
/// A `Habit` is a recurring event; choosing it hands the popup's fields to
/// the Habits editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DraftKind {
    #[default]
    Event,
    TimeBlock,
    Habit,
    Sleep,
    /// A Google Task planned for the selected range; choosing it hands the
    /// popup's title and times to the new-task popup.
    Task,
}

/// A new event being filled in before it is written to Google.
///
/// It is created by a click or a drag on empty grid space and lives in the
/// store until it is saved or cancelled. The times are kept both as real
/// instants and as the text the user is editing, so a half-typed `9:3` does not
/// throw away the last valid time.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    /// Video call, guests and their permissions.
    pub meeting: meeting::Form,
    /// The type chooser has handed this draft to the details pane.
    pub editor_open: bool,
    pub options: event_options::EventOptions,
    pub projects: projects::Selection,
    /// Whether this is an ordinary event or a time block.
    pub kind: DraftKind,
    pub title: String,
    pub description: String,
    /// The calendar the event will be created on.
    pub calendar_id: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    /// The `YYYY-MM-DD` text shown in the date field.
    pub date_text: String,
    /// The `HH:MM` text shown in the start field.
    pub start_text: String,
    /// The `HH:MM` text shown in the end field.
    pub end_text: String,
    /// The event covers whole days rather than clock times. `start` and `end`
    /// are then midnights, with Google's exclusive end: a one-day event on the
    /// 6th runs from the 6th to the 7th. The clock texts keep the last times
    /// typed so switching back restores them.
    pub all_day: bool,
    /// The `YYYY-MM-DD` text shown in the end date field. For an all-day event
    /// this is the last day covered, not Google's exclusive end.
    pub end_date_text: String,
    /// The shared repeat controls. Events default to not repeating.
    pub rule: crate::recurrence::Rule,
    /// Whether the calendar dropdown is open.
    pub calendar_dropdown_open: bool,
    /// A write to Google is in flight, so the draft must not be submitted
    /// again or edited underneath it.
    pub saving: bool,
}

impl Draft {
    /// A draft on `calendar_id` spanning `start` to `end`.
    pub fn new(calendar_id: String, start: DateTime<Local>, end: DateTime<Local>) -> Self {
        Self {
            meeting: meeting::Form::default(),
            options: Default::default(),
            editor_open: false,
            kind: DraftKind::Event,
            projects: projects::Selection::default(),
            title: String::new(),
            description: String::new(),
            calendar_id,
            start,
            end,
            date_text: start.date_naive().to_string(),
            start_text: clock_label(start),
            end_text: clock_label(end),
            all_day: false,
            end_date_text: end.date_naive().to_string(),
            rule: crate::recurrence::Rule::never(start.date_naive()),
            calendar_dropdown_open: false,
            saving: false,
        }
    }

    /// Applies the date field's text. A value that does not parse keeps the
    /// real dates alone, the same forgiving rule the clock fields use. The
    /// end moves by the same number of days, so a span keeps its length.
    pub fn set_date_text(&mut self, text: String) {
        if let Ok(day) = crate::calendar::deadline::parse_date(text.trim()) {
            (self.start, self.end) = move_start_day(self.start, self.end, day, self.all_day);
            self.end_date_text = last_day(self.start, self.end, self.all_day).to_string();
        }
        self.date_text = text;
    }

    /// Applies the end date field's text: the last day an all-day event
    /// covers, or the day a timed event ends on.
    pub fn set_end_date_text(&mut self, text: String) {
        if let Ok(day) = crate::calendar::deadline::parse_date(text.trim()) {
            self.end = end_on_day(self.end, day, self.all_day);
        }
        self.end_date_text = text;
    }

    /// Switches between whole days and clock times, keeping the days covered.
    pub fn set_all_day(&mut self, all_day: bool) {
        if self.all_day == all_day {
            return;
        }
        (self.start, self.end) = switch_all_day(
            self.start,
            self.end,
            all_day,
            &self.start_text,
            &self.end_text,
        );
        self.all_day = all_day;
        if !all_day {
            self.start_text = clock_label(self.start);
            self.end_text = clock_label(self.end);
        }
        self.date_text = self.start.date_naive().to_string();
        self.end_date_text = last_day(self.start, self.end, all_day).to_string();
    }

    /// The recurrence lines the editor describes, or the reason it cannot be
    /// written yet.
    pub fn recurrence_lines(&self) -> Result<Vec<String>, String> {
        if self.all_day {
            self.rule.all_day_lines_at(self.start.date_naive())
        } else {
            self.rule
                .lines_at(self.start.date_naive(), self.start.time())
        }
    }

    /// Replaces the repeat controls, so the habits editor can take them over
    /// unchanged when the new-event dialog switches to a habit.
    pub fn rule(&self) -> &crate::recurrence::Rule {
        &self.rule
    }

    /// Applies the start field's text. A value that does not parse as a clock
    /// time is kept as text but leaves the real start alone.
    pub fn set_start_text(&mut self, text: String) {
        if let Some((hour, minute)) = parse_clock(&text) {
            self.start = at_clock(self.start.date_naive(), hour, minute);
        }
        self.start_text = text;
    }

    /// Applies the end field's text, with the same forgiving parse as the start.
    pub fn set_end_text(&mut self, text: String) {
        if let Some((hour, minute)) = parse_clock(&text) {
            self.end = at_clock(self.end.date_naive(), hour, minute);
        }
        self.end_text = text;
    }
}

/// An existing event being edited before the changes are written to Google.
///
/// It carries the event's id so the update knows which event to patch, and
/// starts from the event's current values so the user only changes what they
/// want to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SaveScope {
    Current,
    CurrentAndFuture,
    All,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditDraft {
    #[serde(default)]
    pub deadline_text: String,
    /// Video call, guests and their permissions.
    pub meeting: meeting::Form,
    pub sleep: Option<sleep::Record>,
    pub options: event_options::EventOptions,
    pub save_menu_open: bool,
    pub delete_menu_open: bool,
    pub saving: bool,
    pub recurrence_changed: bool,
    pub loading: bool,
    pub series_id: Option<String>,
    pub editing_title: bool,
    pub labels: Vec<String>,
    pub label_draft: String,
    pub end_date_text: String,
    pub projects: projects::Selection,
    pub all_day: bool,
    pub id: String,
    pub title: String,
    pub description: String,
    /// The calendar the event should be on once saved.
    pub calendar_id: String,
    /// Whether the calendar picker's dropdown is showing.
    pub calendar_dropdown_open: bool,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    /// The `YYYY-MM-DD` text shown in the date field.
    pub date_text: String,
    /// The `HH:MM` text shown in the start field.
    pub start_text: String,
    /// The `HH:MM` text shown in the end field.
    pub end_text: String,
    /// The shared repeat controls, seeded from the event's own recurrence.
    pub rule: crate::recurrence::Rule,
}

impl EditDraft {
    pub fn deadline(&self) -> Result<Option<deadline::Deadline>, String> {
        if self.deadline_text.trim().is_empty() {
            Ok(None)
        } else {
            deadline::parse_input(&self.deadline_text).map(Some)
        }
    }

    /// Build an edit draft from an existing event.
    pub fn from_event(event: &Event) -> Self {
        Self {
            deadline_text: deadline::input_text(event),
            meeting: meeting::Form::from_event(&event.details),
            sleep: event.details.sleep.clone(),
            options: event.details.options.clone(),
            save_menu_open: false,
            delete_menu_open: false,
            saving: false,
            recurrence_changed: false,
            loading: false,
            series_id: None,
            editing_title: false,
            labels: event.details.labels.clone(),
            label_draft: String::new(),
            end_date_text: last_day(event.start, event.end, event.all_day).to_string(),
            projects: projects::Selection {
                ids: event.details.projects.clone(),
                ..Default::default()
            },
            all_day: event.all_day,
            id: event.id.clone(),
            title: event.title.clone(),
            description: event.details.description.clone(),
            calendar_id: event.calendar_id.clone(),
            calendar_dropdown_open: false,
            start: event.start,
            end: event.end,
            date_text: event.start.date_naive().to_string(),
            // An all-day event has no clock times of its own; these are what
            // switching it to a timed event starts from.
            start_text: if event.all_day {
                "09:00".to_owned()
            } else {
                clock_label(event.start)
            },
            end_text: if event.all_day {
                "10:00".to_owned()
            } else {
                clock_label(event.end)
            },
            rule: crate::recurrence::Rule::from_recurrence(
                event.start.date_naive(),
                &crate::recurrence::Recurrence::from_lines(&event.details.recurrence),
            ),
        }
    }

    /// Keep the selected habit occurrence's values and date while loading its
    /// master's repeat schedule. A scoped save needs the occurrence as its anchor.
    pub fn from_habit_occurrence(event: &Event, master: &Event) -> Self {
        let mut draft = Self::from_event(event);
        draft.series_id = Some(master.id.clone());
        draft.rule = crate::recurrence::Rule::from_recurrence(
            master.start.date_naive(),
            &crate::recurrence::Recurrence::from_lines(&master.details.recurrence),
        );
        draft
    }

    /// Applies the date field's text, keeping the clock times.
    pub fn set_date_text(&mut self, text: String) {
        if let Ok(day) = crate::calendar::deadline::parse_date(text.trim()) {
            self.start = at_clock(day, self.start.hour(), self.start.minute());
        }
        self.date_text = text;
    }

    /// Applies the end date field's text: the last day an all-day event
    /// covers, or the day a timed event ends on.
    pub fn set_end_date_text(&mut self, text: String) {
        if let Ok(day) = crate::calendar::deadline::parse_date(text.trim()) {
            self.end = end_on_day(self.end, day, self.all_day);
        }
        self.end_date_text = text;
    }

    /// Switches between whole days and clock times, keeping the days covered.
    pub fn set_all_day(&mut self, all_day: bool) {
        if self.all_day == all_day {
            return;
        }
        (self.start, self.end) = switch_all_day(
            self.start,
            self.end,
            all_day,
            &self.start_text,
            &self.end_text,
        );
        self.all_day = all_day;
        if !all_day {
            self.start_text = clock_label(self.start);
            self.end_text = clock_label(self.end);
        }
        self.date_text = self.start.date_naive().to_string();
        self.end_date_text = last_day(self.start, self.end, all_day).to_string();
    }

    /// The recurrence lines the editor describes.
    pub fn recurrence_lines(&self) -> Result<Vec<String>, String> {
        if self.all_day {
            self.rule.all_day_lines_at(self.start.date_naive())
        } else {
            self.rule
                .lines_at(self.start.date_naive(), self.start.time())
        }
    }

    /// Applies the start field's text. A value that does not parse as a clock
    /// time is kept as text but leaves the real start alone.
    pub fn set_start_text(&mut self, text: String) {
        if let Some((hour, minute)) = parse_clock(&text) {
            self.start = at_clock(self.start.date_naive(), hour, minute);
        }
        self.start_text = text;
    }

    /// Applies the end field's text, with the same forgiving parse as the start.
    pub fn set_end_text(&mut self, text: String) {
        if let Some((hour, minute)) = parse_clock(&text) {
            self.end = at_clock(self.end.date_naive(), hour, minute);
        }
        self.end_text = text;
    }
}

/// The day an editor's end date field shows. An all-day end is Google's
/// exclusive midnight, so the field shows the day before it: the last day the
/// event covers. A timed end shows the day it falls on.
pub fn last_day(start: DateTime<Local>, end: DateTime<Local>, all_day: bool) -> NaiveDate {
    if !all_day {
        return end.date_naive();
    }
    let day = end.date_naive();
    day.pred_opt().unwrap_or(day).max(start.date_naive())
}

/// Moves a span to start on `day`, keeping its clock times and its length in
/// days.
pub fn move_start_day(
    start: DateTime<Local>,
    end: DateTime<Local>,
    day: NaiveDate,
    all_day: bool,
) -> (DateTime<Local>, DateTime<Local>) {
    let shift = day - start.date_naive();
    let end_day = end.date_naive() + shift;
    if all_day {
        (at_clock(day, 0, 0), at_clock(end_day, 0, 0))
    } else {
        (
            at_clock(day, start.hour(), start.minute()),
            at_clock(end_day, end.hour(), end.minute()),
        )
    }
}

/// The end for an end date field reading `day`: the midnight after it for an
/// all-day event, or the same clock time on that day for a timed one.
pub fn end_on_day(end: DateTime<Local>, day: NaiveDate, all_day: bool) -> DateTime<Local> {
    if all_day {
        at_clock(day.succ_opt().unwrap_or(day), 0, 0)
    } else {
        at_clock(day, end.hour(), end.minute())
    }
}

/// The span after switching between all-day and timed, covering the same
/// days.
///
/// Going all-day, a timed end exactly at midnight does not count its day.
/// Going back, the clock texts the user last typed are put back on the first
/// and last days, falling back to an hour from the start when they do not
/// make a valid span.
pub fn switch_all_day(
    start: DateTime<Local>,
    end: DateTime<Local>,
    all_day: bool,
    start_text: &str,
    end_text: &str,
) -> (DateTime<Local>, DateTime<Local>) {
    let first = start.date_naive();
    if all_day {
        let mut last = end.date_naive();
        if end > start && end.time() == chrono::NaiveTime::MIN {
            last = last.pred_opt().unwrap_or(last);
        }
        let last = last.max(first);
        return (
            at_clock(first, 0, 0),
            at_clock(last.succ_opt().unwrap_or(last), 0, 0),
        );
    }
    let last = last_day(start, end, true);
    let (start_hour, start_minute) = parse_clock(start_text).unwrap_or((9, 0));
    let start = at_clock(first, start_hour, start_minute);
    let end = parse_clock(end_text)
        .map(|(hour, minute)| at_clock(last, hour, minute))
        .filter(|end| *end > start)
        .unwrap_or(start + chrono::Duration::hours(1));
    (start, end)
}

/// Parses an `HH:MM` (or `H:MM`) clock time. Anything else is rejected.
pub fn parse_clock(text: &str) -> Option<(u32, u32)> {
    let (hour, minute) = text.trim().split_once(':')?;
    let hour: u32 = hour.trim().parse().ok()?;
    let minute: u32 = minute.trim().parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

/// Whether the source with this id is switched on.
pub fn calendar_enabled(id: &str, settings: &CalendarSettings) -> bool {
    if id.starts_with(ids::TASK_LIST) {
        settings.show_tasks && !settings.disabled.iter().any(|hidden| hidden == id)
    } else {
        !settings.disabled.iter().any(|hidden| hidden == id)
    }
}

/// Switches one source on or off.
pub fn set_source_enabled(
    settings: &mut CalendarSettings,
    calendars: &[Calendar],
    id: &str,
    enabled: bool,
) {
    // Preserve a previous global hide when switching to per-list controls.
    if enabled && id.starts_with(ids::TASK_LIST) && !settings.show_tasks {
        for calendar in calendars.iter().filter(|calendar| calendar.is_task_list()) {
            settings.set_calendar_enabled(&calendar.id, false);
        }
        settings.show_tasks = true;
    }
    settings.set_calendar_enabled(id, enabled);
}

/// Source and per-kind visibility apply to every calendar view, including
/// cached days and task-backed copies of scheduled calendar events.
pub fn event_enabled(calendars: &[Calendar], event: &Event, settings: &CalendarSettings) -> bool {
    let app_calendar = settings.app_calendar.as_deref().or_else(|| {
        calendars
            .iter()
            .find(|calendar| calendar.is_app_calendar(settings))
            .map(|calendar| calendar.id.as_str())
    });
    let kind = sources::TimeSlaySource::of(event, app_calendar.unwrap_or(""));
    calendar_enabled(event.source_id(), settings)
        && kind.is_none_or(|kind| calendar_enabled(kind.id(), settings))
}

/// The instant a local clock reading means.
///
/// A reading that happens twice, when the clocks go back, is the first of the
/// two. One that never happens, when they go forward, moves ahead by the size
/// of the jump: 2:30am becomes 3:30am. That is how calendars generally treat
/// it, and it keeps the event on the day it was meant for.
pub fn local_time(naive: chrono::NaiveDateTime) -> DateTime<Local> {
    match Local.from_local_datetime(&naive) {
        chrono::offset::LocalResult::Single(when)
        | chrono::offset::LocalResult::Ambiguous(when, _) => when,
        chrono::offset::LocalResult::None => {
            // Read it with the offset in force before the jump.
            let before = naive - chrono::Duration::days(1);
            let offset = Local.offset_from_local_datetime(&before).earliest();
            let utc = offset.map_or(naive, |offset| naive - offset.fix());
            utc.and_utc().with_timezone(&Local)
        }
    }
}

/// A local instant at a clock time on a given date, resolving daylight saving
/// through [`local_time`].
pub fn at_clock(day: NaiveDate, hour: u32, minute: u32) -> DateTime<Local> {
    local_time(day.and_hms_opt(hour, minute, 0).unwrap_or_default())
}

/// The parts of a Google event that are worth reading, beyond the chip.
///
/// It carries serde derives so a day can be written out to the cache whole.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Details {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep: Option<sleep::Record>,
    pub options: event_options::EventOptions,
    /// Last modification instant reported by Google Tasks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_updated_at: Option<DateTime<Utc>>,
    /// Project IDs assigned to this calendar event (tasks use their task metadata).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    pub task_data: Option<crate::task_db::TaskData>,
    pub location: String,
    pub description: String,
    pub status: String,
    pub link: String,
    pub meeting: String,
    pub organizer: String,
    pub attendees: Vec<String>,
    /// The guests as data: their replies, permissions and the user's part.
    pub attendance: meeting::Attendance,
    /// This is a habit event on the app's TimeSlay calendar.
    pub habit: bool,
    /// The habit this event counts towards, from its `ttHabit` property.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub habit_id: Option<String>,
    /// Completion instant for this occurrence; older records may not have one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub habit_completed_at: Option<DateTime<Local>>,
    /// The recurring event master, for an expanded habit or other recurrence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurring_event_id: Option<String>,
    /// The raw RFC 5545 recurrence lines from Google (`RRULE:`, `RDATE:`,
    /// `EXDATE:`). Empty for a one-off event.
    pub recurrence: Vec<String>,
    /// Google's event `colorId`, or `None` when the event uses its calendar's
    /// color.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_id: Option<String>,
    /// A change to this item is on its way to Google. The interface shows it
    /// greyed out and does not let it be changed again until Google answers.
    /// Set by [`operations::Overlay::apply`]; never stored.
    #[serde(skip)]
    pub sending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
    pub day: NaiveDate,
    pub fetched_at: DateTime<Local>,
    /// Set when calendars loaded but Tasks did not, so the day view still appears.
    pub notice: Option<String>,
}

/// Default height of one hour, close to Google Calendar's day view.
pub const DEFAULT_HOUR_HEIGHT: f32 = 48.0;
/// Shortest hour row. Below this the titles stop being readable.
pub const MIN_HOUR_HEIGHT: f32 = 24.0;
/// Tallest hour row, so a zoomed-in day stays a grid rather than a poster.
pub const MAX_HOUR_HEIGHT: f32 = 180.0;
/// How far one press of a zoom key or menu button goes. Bigger than a wheel
/// notch, which comes in runs.
pub const ZOOM_STEP: f32 = 1.25;

/// Shared membership rule for both memory and disk snapshots.
fn event_on_day(event: &Event, day: NaiveDate, day_end_hour: u32) -> bool {
    if event.is_task() {
        return true;
    }
    if event.all_day {
        return event.start.date_naive() == day
            || (event.start.date_naive() < day && event.end.date_naive() > day);
    }
    let start = at_clock(day, day_end_hour, 0);
    let end = at_clock(day.succ_opt().unwrap_or(day), day_end_hour, 0);
    event.start < end && event.end > start
}

/// How far either side of the day in view a cached day can be and still be
/// kept whatever else is cached: the previous day and the next one, once they
/// have been viewed.
const ALWAYS_KEPT_DISTANCE: i64 = 1;

/// How many days beyond that window stay cached, counted from the day in view
/// outwards. The memory cache and the days on disk are trimmed by the same
/// rule, so both end up holding the same days.
const EXTRA_CACHED_DAYS: usize = 10;

/// The days that should leave a cache while `current` is in view.
///
/// The day in view, the day before it and the day after it are never dropped,
/// and the [`EXTRA_CACHED_DAYS`] closest days beyond that window survive too.
/// Everything further out goes, furthest first.
fn days_to_drop(days: impl IntoIterator<Item = NaiveDate>, current: NaiveDate) -> Vec<NaiveDate> {
    let mut far: Vec<NaiveDate> = days
        .into_iter()
        .filter(|day| (*day - current).num_days().abs() > ALWAYS_KEPT_DISTANCE)
        .collect();

    far.sort_by_key(|day| (*day - current).num_days().abs());
    if far.len() <= EXTRA_CACHED_DAYS {
        Vec::new()
    } else {
        far.split_off(EXTRA_CACHED_DAYS)
    }
}

/// A task list's color: the user's choice if there is one, otherwise Google's
/// blue.
fn task_color(settings: &CalendarSettings, calendar_id: &str) -> Color {
    settings.task_color(calendar_id).unwrap_or(colors::TASK)
}

/// Gives a Google Tasks event its list's color and an app-calendar event the
/// per-kind color from settings, leaving every other calendar's events alone.
fn recolor_event(event: &mut Event, settings: &CalendarSettings) {
    if event.details.sleep.is_some() {
        event.color = settings.sleep_color();
    } else if event.calendar_id.starts_with(ids::TASK_LIST) && !event.is_calendar_task() {
        event.color = task_color(settings, &event.calendar_id);
    } else if event.is_calendar_task() {
        // A task's calendar block, on the TimeSlay calendar.
        event.color = settings.task_event_color();
    } else if event.details.habit {
        // A habit's own Google color wins; otherwise use the habit color.
        event.color = event
            .details
            .color_id
            .as_deref()
            .and_then(colors::event)
            .unwrap_or_else(|| settings.habit_color());
    } else if settings.app_calendar.as_deref() == Some(event.calendar_id.as_str())
        && event.details.color_id.is_none()
    {
        if event.all_day {
            // A plain all-day event on the TimeSlay calendar.
            event.color = settings.event_color();
        } else {
            // A plain timed event on the TimeSlay calendar is a time block.
            event.color = settings.time_block_color();
        }
    }
}

/// Everything the calendar view and its settings pane need.
pub struct Store {
    pub connection: Connection,
    pub sync: SyncState,
    pub menu_open: bool,
    /// The Actions popup in the calendar header: Sync now and Set Alarm.
    pub actions_open: bool,
    pub search: search::Search,
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
    /// Events for the days already visited, so stepping back to one paints it
    /// at once while the fresh sync runs. The day in view lives in `events`.
    cache: HashMap<NaiveDate, Vec<Event>>,
    /// Dates currently displayed by a range view must survive cache pruning.
    visible_days: Vec<NaiveDate>,
    /// The database those days are also written to, so they survive a
    /// restart. `None` until [`Store::cache_days_in`] is called, which only
    /// `boot` and a reconnect do: a store built by hand must not read or
    /// rewrite the cache the user's own days live in.
    day_cache: Option<PathBuf>,
    /// Preferences used to recolor days restored from disk before display.
    color_settings: Option<CalendarSettings>,
    /// The hour the calendar day ends, used to decide which day an event
    /// belongs to when reading from or writing to the month cache on disk.
    day_end_hour: u32,
    pub day: NaiveDate,
    pub last_sync: Option<DateTime<Local>>,
    pub error: Option<String>,
    /// Pixel height of one hour on the day grid. Shift+scroll changes it.
    pub hour_height: f32,
    /// The event whose details are open, by id.
    pub open_event: Option<String>,
    /// An off-day habit master or unsaved habit shown only in the details pane.
    pub detail_event: Option<Event>,
    /// The details card sits at the bottom when the click was in the top half.
    pub details_below: bool,
    /// The task list whose color picker is open, by calendar id.
    pub color_editor: Option<String>,
    /// Whether the month picker is showing over the day view.
    pub month_open: bool,
    /// The first day of the month the picker is showing.
    pub month: NaiveDate,
    /// Whether the day in view should follow the real today. Cleared once the
    /// user navigates to another day, so a boundary rollover does not yank them
    /// back to today.
    pub follow_today: bool,
    /// Time changes still being written to Google, by event id. The event is
    /// drawn lighter until the write is confirmed, and it goes back to the
    /// times here if the write fails. Read through [`Store::pending_time`].
    pending: HashMap<String, PendingTime>,
    /// The new event being filled in, if the add-event popup is open.
    pub draft: Option<Draft>,
    /// The existing event being edited, if the edit-event popup is open.
    pub edit_draft: Option<EditDraft>,
    /// The event a delete is being confirmed for, by id. While set, the details
    /// card is replaced by an "are you sure" prompt so a stray click cannot
    /// wipe an event.
    pub pending_delete: Option<String>,
    pub pending_delete_scope: SaveScope,
    /// Whether confirming that delete emails the event's guests.
    pub pending_delete_notify: bool,
    /// A dragged item awaiting recurrence scope or guest notification choices.
    pub pending_move: Option<PendingMove>,
}

/// A time change waiting for recurrence scope or guest notification choices.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingMove {
    pub original: Event,
    pub scope: Option<SaveScope>,
    pub id: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
}

impl Store {
    pub fn new(day_end_hour: u32) -> Self {
        let connection = if oauth::load_tokens().is_some() {
            Connection::SignedIn
        } else {
            Connection::SignedOut
        };
        let day = day_at(crate::clock::now(), day_end_hour);

        Self {
            connection,
            sync: SyncState::Idle,
            menu_open: false,
            actions_open: false,
            search: search::Search::default(),
            calendars: Vec::new(),
            events: Vec::new(),
            cache: HashMap::new(),
            visible_days: Vec::new(),
            day_cache: None,
            color_settings: None,
            day_end_hour,
            day,
            last_sync: None,
            error: None,
            hour_height: DEFAULT_HOUR_HEIGHT,
            open_event: None,
            detail_event: None,
            details_below: false,
            color_editor: None,
            month_open: false,
            month: first_of_month(day),
            follow_today: true,
            pending: HashMap::new(),
            draft: None,
            edit_draft: None,
            pending_delete: None,
            pending_delete_scope: SaveScope::Current,
            pending_delete_notify: true,
            pending_move: None,
        }
    }

    /// A store with events already in hand, for tests and screenshots.
    ///
    /// The day boundary defaults to midnight (0), which is also the config
    /// default, so cached days are filed under the same days they were fetched
    /// for.
    pub fn with_snapshot(snapshot: Snapshot) -> Self {
        Self {
            connection: Connection::SignedIn,
            sync: SyncState::Idle,
            menu_open: false,
            actions_open: false,
            search: search::Search::default(),
            calendars: snapshot.calendars,
            events: snapshot.events,
            cache: HashMap::new(),
            visible_days: Vec::new(),
            day_cache: None,
            color_settings: None,
            day_end_hour: 0,
            day: snapshot.day,
            last_sync: Some(snapshot.fetched_at),
            error: None,
            hour_height: DEFAULT_HOUR_HEIGHT,
            open_event: None,
            detail_event: None,
            details_below: false,
            color_editor: None,
            month_open: false,
            month: first_of_month(snapshot.day),
            // A snapshot is pinned to the day it was fetched for, so it does
            // not follow the clock until the user asks it to.
            follow_today: false,
            pending: HashMap::new(),
            draft: None,
            edit_draft: None,
            pending_delete: None,
            pending_delete_scope: SaveScope::Current,
            pending_delete_notify: true,
            pending_move: None,
        }
    }

    /// Points the store at the database cached days are written to, so they
    /// outlive the process.
    ///
    /// The day in view is read straight back when a previous run left it
    /// there, which is what gives the window something to show before the
    /// first sync lands, and days that fell out of the cache while the app
    /// was closed are dropped. Called at boot and when a reconnect replaces
    /// the store: a store built by hand keeps its days in memory only.
    pub fn cache_days_in(&mut self, database: Option<PathBuf>) {
        self.day_cache = database;
        if self.last_sync.is_none()
            && let Some((fetched_at, calendars)) = self
                .day_cache
                .as_ref()
                .and_then(|database| day_cache::load_sync_state(database))
        {
            self.last_sync = Some(fetched_at);
            self.calendars = calendars;
        }

        if self.events.is_empty()
            && let Some(events) = self.read_day(self.day)
        {
            self.events = events;
        }

        self.trim_cache();
    }

    /// The hour the calendar day ends, used to assign events to days when
    /// reading from or writing to the month cache on disk.
    pub fn day_end_hour(&self) -> u32 {
        self.day_end_hour
    }

    /// Moves the day in view, restoring whatever was cached for it.
    ///
    /// The day being left is parked in the cache, so stepping back to it paints
    /// at once rather than waiting for the sync. A day that has not been seen
    /// yet comes back from the database if an earlier run left it there,
    /// and empty otherwise, filling in when the sync lands. `today` is the
    /// real day the clock is on, used to decide whether the view keeps
    /// following it.
    pub fn set_day(&mut self, day: NaiveDate, today: NaiveDate) {
        if day != self.day {
            let left = self.day;
            self.cache.insert(left, std::mem::take(&mut self.events));

            let restored = match self.cache.remove(&day) {
                Some(events) => events,
                None => self.read_day(day).unwrap_or_default(),
            };
            self.events = restored;
            self.open_event = None;
            self.day = day;
            self.trim_cache();

            // The day we left only stays on disk if the cache still wants it.
            if let Some(events) = self.cache.get(&left) {
                self.write_day(left, events);
            }
        }

        self.day = day;
        self.follow_today = day == today;
    }

    /// Drops every cached day, so the next visit to each fetches it afresh.
    ///
    /// The day boundary moving changes which events a day holds, so the cache
    /// no longer describes the days filed under it.
    pub fn invalidate_other_days(&mut self) {
        self.cache.clear();
        if let Some(database) = self.day_cache.as_ref() {
            day_cache::clear(database);
        }
    }
    pub fn forget_cached_days(&mut self) {
        self.cache.clear();
        self.events.clear();
        self.open_event = None;
        // The events the pending writes belonged to are gone; the answers will
        // arrive to find nothing to confirm or put back.
        self.pending.clear();

        // The days on disk were filed under the old boundary too.
        if let Some(database) = self.day_cache.as_ref() {
            day_cache::clear(database);
        }
    }

    /// Bound memory use while retaining every visited month on disk.
    fn trim_cache(&mut self) {
        let current = self.day;

        for day in days_to_drop(
            self.cache
                .keys()
                .copied()
                .filter(|day| !self.visible_days.contains(day)),
            current,
        ) {
            self.cache.remove(&day);
        }
    }

    /// The day's events as an earlier run left them, if it left them.
    fn read_day(&self, day: NaiveDate) -> Option<Vec<Event>> {
        let mut events = day_cache::load(self.day_cache.as_ref()?, day, self.day_end_hour)?;
        if let Some(settings) = &self.color_settings {
            for event in &mut events {
                recolor_event(event, settings);
            }
        }
        Some(events)
    }

    /// Writes one day out, so the run after this one paints it at once.
    fn write_day(&self, day: NaiveDate, events: &[Event]) {
        let Some(database) = self.day_cache.as_ref() else {
            return;
        };

        if let Err(error) = day_cache::save(database, day, events, self.day_end_hour) {
            log::warn!("could not cache {}: {error}", day);
        }
    }

    /// Grows or shrinks the hour rows. `factor` above 1 spreads the day out.
    pub fn zoom(&mut self, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }

        let height = (self.hour_height * factor).clamp(MIN_HOUR_HEIGHT, MAX_HOUR_HEIGHT);

        // Rounding must not leave stepping in and back out a hair off the
        // default, with Reset zoom still lit for no visible difference.
        self.hour_height = if (height - DEFAULT_HOUR_HEIGHT).abs() < 0.5 {
            DEFAULT_HOUR_HEIGHT
        } else {
            height
        };
    }

    pub fn reset_zoom(&mut self) {
        self.hour_height = DEFAULT_HOUR_HEIGHT;
    }

    /// The zoom as a multiple of the default, for the views without hour rows.
    pub fn zoom_ratio(&self) -> f32 {
        self.hour_height / DEFAULT_HOUR_HEIGHT
    }

    pub fn is_signed_in(&self) -> bool {
        self.connection == Connection::SignedIn
    }

    pub fn is_busy(&self) -> bool {
        matches!(self.sync, SyncState::Connecting | SyncState::Syncing)
    }

    /// Calendars and individual task lists, excluding the app's managed lists.
    pub fn display_sources(&self, settings: &CalendarSettings) -> Vec<Calendar> {
        let mut sources: Vec<_> = self
            .calendars
            .iter()
            .filter(|calendar| !calendar.is_managed_task_list(settings))
            .cloned()
            .map(|mut calendar| {
                if calendar.is_task_list() {
                    calendar.color = task_color(settings, &calendar.id);
                } else if let Some(color) = settings.calendar_colors.get(&calendar.id) {
                    calendar.color = color.0;
                }
                calendar
            })
            .collect();
        sources.push(Calendar {
            id: sleep::SOURCE_ID.to_owned(),
            name: "Sleep".to_owned(),
            color: settings.sleep_color(),
            primary: false,
            selected_by_google: true,
        });
        // Stable sorting appends newly discovered sources in Google's order.
        sources.sort_by_key(|source| {
            settings
                .source_order
                .iter()
                .position(|id| id == &source.id)
                .or_else(|| {
                    // Expand the old shared source in place for existing settings.
                    source
                        .is_task_list()
                        .then(|| {
                            settings
                                .source_order
                                .iter()
                                .position(|id| id == CalendarSettings::GOOGLE_TASKS_ID)
                        })
                        .flatten()
                })
                .unwrap_or(usize::MAX)
        });
        sources
    }

    pub fn calendar_enabled(&self, id: &str, settings: &CalendarSettings) -> bool {
        calendar_enabled(id, settings)
    }

    /// Source and per-kind visibility apply to every calendar view, including
    /// cached days and task-backed copies of scheduled calendar events.
    pub fn event_enabled(&self, event: &Event, settings: &CalendarSettings) -> bool {
        event_enabled(&self.calendars, event, settings)
    }

    /// Gives every Google Tasks list its saved color, and every event on the
    /// app's TimeSlay calendar the color for its kind: a time block or a
    /// scheduled task. Google hands all task lists back with the same blue, so
    /// this keeps a customized list its own color across syncs. Every other
    /// calendar's events are left exactly as Google returned them.
    pub fn apply_task_colors(&mut self, settings: &CalendarSettings) {
        // Snapshots are cached with Google's colors, and old cache files may
        // predate a preference change. Future disk reads need these settings
        // too, even when no sync is available to correct their colors later.
        self.color_settings = Some(settings.clone());
        for calendar in &mut self.calendars {
            if calendar.is_task_list() {
                calendar.color = task_color(settings, &calendar.id);
            }
        }

        for event in &mut self.events {
            recolor_event(event, settings);
        }

        // The cached days are recolored too, so a day painted from the cache
        // does not flash the old color before the sync lands.
        for events in self.cache.values_mut() {
            for event in events {
                recolor_event(event, settings);
            }
        }
    }

    /// The database the day cache and the local edits live in, when this
    /// store is backed by one.
    pub fn local_database(&self) -> Option<&std::path::Path> {
        self.day_cache.as_deref()
    }

    /// Reads the local edits and puts them over every day in memory.
    /// Blocking: the poll reads the overlay on a worker thread with
    /// [`operations::load_overlay`] and hands it to [`Store::apply_overlay`].
    pub fn apply_local_operations(&mut self) -> Result<(), String> {
        let Some(path) = &self.day_cache else {
            return Ok(());
        };
        let overlay = operations::load_overlay(path)?;
        self.apply_overlay(&overlay);
        Ok(())
    }

    /// Puts already-read local edits over every day in memory. Touches no
    /// files, so it is safe on the UI thread.
    pub fn apply_overlay(&mut self, overlay: &operations::Overlay) {
        if self.day_cache.is_none() {
            return;
        }
        overlay.apply(&mut self.events, None);
        self.events
            .retain(|e| event_on_day(e, self.day, self.day_end_hour));
        for (day, events) in &mut self.cache {
            overlay.apply(events, None);
            events.retain(|e| event_on_day(e, *day, self.day_end_hour));
        }
        if let Some(event) = &mut self.detail_event {
            let id = event.id.clone();
            let mut detail = vec![event.clone()];
            overlay.apply(&mut detail, None);
            if let Some(updated) = detail.into_iter().find(|e| e.id == id) {
                *event = updated;
            }
        }
    }

    pub fn apply_snapshot(&mut self, mut snapshot: Snapshot) {
        if let Some(path) = &self.day_cache
            && let Err(e) =
                operations::overlay(path, &mut snapshot.events, Some(snapshot.fetched_at))
        {
            log::warn!("{e}");
        }
        snapshot
            .events
            .retain(|e| event_on_day(e, snapshot.day, self.day_end_hour));
        self.calendars = snapshot.calendars;
        self.events = snapshot.events;
        self.day = snapshot.day;
        self.last_sync = Some(snapshot.fetched_at);
        if let Some(database) = &self.day_cache
            && let Err(error) =
                day_cache::save_sync_state(database, snapshot.fetched_at, &self.calendars)
        {
            log::warn!("could not cache calendar sync state: {error}");
        }
        self.connection = Connection::SignedIn;
        self.sync = SyncState::Idle;
        self.error = snapshot.notice;
        if self
            .open_event
            .as_ref()
            .is_some_and(|id| self.event(id).is_none())
        {
            self.open_event = None;
        }

        // A write that is still in flight wins over the snapshot: Google may
        // not have processed it yet, and the chip must not snap back to the
        // old time while the user is watching.
        for (id, pending) in &self.pending {
            if let Some(event) = self.events.iter_mut().find(|event| &event.id == id) {
                event.start = pending.start;
                event.end = pending.end;
            }
        }

        // The day in view has fresh events now, so the run after this one
        // opens on them rather than on an empty grid.
        self.write_day(self.day, &self.events);
    }

    /// Install a range fetch as ordinary per-day snapshots, retaining all tasks
    /// on each day just as a day fetch does.
    pub fn apply_week_snapshot(&mut self, mut snapshot: Snapshot, days: &[NaiveDate]) {
        if let Some(path) = &self.day_cache
            && let Err(e) =
                operations::overlay(path, &mut snapshot.events, Some(snapshot.fetched_at))
        {
            log::warn!("{e}");
        }
        self.visible_days.extend_from_slice(days);
        self.visible_days.sort_unstable();
        self.visible_days.dedup();
        let current = self.day;
        for event in &mut snapshot.events {
            if let Some(pending) = self.pending.get(&event.id) {
                event.start = pending.start;
                event.end = pending.end;
            }
        }
        let mut current_snapshot = snapshot.clone();
        // A background fetch can complete after navigation. Cache the fetched
        // dates while preserving the current day's events and selection.
        current_snapshot.day = current;
        current_snapshot.events = self.events.clone();
        for &day in days {
            let events: Vec<_> = snapshot
                .events
                .iter()
                .filter(|event| event_on_day(event, day, self.day_end_hour))
                .cloned()
                .collect();
            if day == current {
                current_snapshot.events = events;
                current_snapshot.day = day;
            } else {
                self.write_day(day, &events);
                self.cache.insert(day, events);
            }
        }
        self.apply_snapshot(current_snapshot);
        self.trim_cache();
    }

    /// Accept results for any previously requested view without navigating or
    /// scheduling a follow-up fetch. Keep the active view pinned in the cache.
    pub fn cache_fetched_snapshot(&mut self, snapshot: Snapshot, days: &[NaiveDate]) {
        let visible = self.visible_days.clone();
        self.apply_week_snapshot(snapshot, days);
        self.visible_days = visible;
        self.trim_cache();
    }

    /// The current view's cached data, without making a network request.
    pub fn cached_snapshot(&self) -> Option<Snapshot> {
        Some(Snapshot {
            calendars: self.calendars.clone(),
            events: self.events.clone(),
            day: self.day,
            fetched_at: self.last_sync?,
            notice: self.error.clone(),
        })
    }

    /// View navigation does not call this; startup and the periodic timer do.
    pub fn sync_due(&self, interval: std::time::Duration, now: DateTime<Local>) -> bool {
        self.last_sync.is_none_or(|last| {
            now.signed_duration_since(last)
                .to_std()
                .is_ok_and(|elapsed| elapsed >= interval)
        })
    }

    pub fn deadline_markers_on(&self, day: NaiveDate, settings: &CalendarSettings) -> Vec<Event> {
        let mut seen = std::collections::HashSet::new();
        self.events
            .iter()
            .chain(self.cache.values().flatten())
            .filter(|event| seen.insert(event.id.clone()))
            .filter(|event| {
                self.event_enabled(event, settings) && self.search_shows(event, day, settings)
            })
            .filter_map(deadline::marker)
            .filter(|event| deadline::on_day(event, day, settings.day_start_hour()))
            .collect()
    }

    pub fn events_on(&self, day: NaiveDate) -> &[Event] {
        if day == self.day {
            &self.events
        } else {
            self.cache.get(&day).map(Vec::as_slice).unwrap_or(&[])
        }
    }

    /// Shared backlog tasks and off-day linked blocks do not establish that
    /// this day's calendar data has been loaded.
    pub fn has_calendar_events_on(&self, day: NaiveDate) -> bool {
        self.events_on(day)
            .iter()
            .any(|event| !event.is_task() && event_on_day(event, day, self.day_end_hour))
    }

    /// Update only existing copies of this occurrence, including disk history.
    pub fn replace_habit_occurrence(&mut self, updated: &Event) {
        let replace = |events: &mut Vec<Event>| {
            for event in events {
                if event.id == updated.id && event.calendar_id == updated.calendar_id {
                    *event = updated.clone();
                }
            }
        };
        replace(&mut self.events);
        for events in self.cache.values_mut() {
            replace(events);
        }
        if let Some(event) = self.detail_event.as_mut()
            && event.id == updated.id
            && event.calendar_id == updated.calendar_id
        {
            *event = updated.clone();
        }
        if let Some(database) = &self.day_cache {
            day_cache::update_events(database, replace);
        }
    }

    pub fn replace_cached_task(&mut self, previous_id: &str, task: &Event) {
        let key = crate::timeblocking::task_key(task);
        let replace = |events: &mut Vec<Event>| {
            events.retain(|event| {
                event.id != previous_id
                    && (!event.is_task() || crate::timeblocking::task_key(event) != key)
            });
            events.push(task.clone());
        };
        for events in self.cache.values_mut() {
            replace(events);
        }
        if let Some(database) = &self.day_cache {
            day_cache::update_events(database, replace);
        }
    }

    pub fn clear_disk_cache(&self) {
        if let Some(database) = self.day_cache.as_ref() {
            day_cache::clear(database);
        }
    }

    pub fn forget_event_everywhere(&mut self, id: &str) {
        self.events.retain(|event| event.id != id);
        for events in self.cache.values_mut() {
            events.retain(|event| event.id != id);
        }
        if let Some(database) = &self.day_cache {
            day_cache::update_events(database, |events| events.retain(|event| event.id != id));
        }
    }

    pub fn load_cached_days(&mut self, days: &[NaiveDate]) {
        self.visible_days = days.to_vec();
        for &day in days {
            if day != self.day
                && !self.cache.contains_key(&day)
                && let Some(events) = self.read_day(day)
            {
                self.cache.insert(day, events);
            }
        }
        self.trim_cache();
    }

    pub fn all_day_events_on<'a>(
        &'a self,
        day: NaiveDate,
        settings: &'a CalendarSettings,
    ) -> impl Iterator<Item = &'a Event> {
        let events = self.events_on(day);
        let blocks: Vec<_> = events
            .iter()
            .filter(|event| crate::timeblocking::is_block(event, settings))
            .map(crate::timeblocking::block_key)
            .collect();
        events
            .iter()
            .filter(|event| self.event_enabled(event, settings))
            .filter(move |event| {
                event.all_day
                    && (!event.is_task()
                        || (event.start.date_naive() != crate::timeblocking::UNDATED
                            && event.start.date_naive() == day)
                        || (!settings.hide_overdue_tasks && crate::timeblocking::overdue(event)))
                    && !omit_managed_tasks_event(event, &blocks, settings)
            })
    }

    pub fn visible_events<'a>(
        &'a self,
        settings: &'a CalendarSettings,
    ) -> impl Iterator<Item = &'a Event> {
        self.events
            .iter()
            .filter(|event| self.event_enabled(event, settings))
    }

    /// The event with this id, if it is still loaded.
    pub fn event(&self, id: &str) -> Option<&Event> {
        self.detail_event
            .as_ref()
            .filter(|event| event.id == id)
            .or_else(|| self.events.iter().find(|event| event.id == id))
    }

    /// A mutable handle to the event with this id.
    pub fn event_mut(&mut self, id: &str) -> Option<&mut Event> {
        if self
            .detail_event
            .as_ref()
            .is_some_and(|event| event.id == id)
        {
            return self.detail_event.as_mut();
        }
        self.events.iter_mut().find(|event| event.id == id)
    }

    /// Sets the location text of every copy of an event, so the details pane
    /// and the trips, which read different copies, both change at once rather
    /// than after the next sync.
    pub fn set_location(&mut self, id: &str, location: &str) {
        let set = |events: &mut Vec<Event>| {
            for event in events.iter_mut().filter(|event| event.id == id) {
                event.details.location = location.to_owned();
            }
        };
        set(&mut self.events);
        for events in self.cache.values_mut() {
            set(events);
        }
        if let Some(event) = self.detail_event.as_mut().filter(|event| event.id == id) {
            event.details.location = location.to_owned();
        }
        if let Some(database) = &self.day_cache {
            day_cache::update_events(database, set);
        }
    }

    /// A mutable handle to an event, whether it is on the day in view or parked
    /// in the cache. A write can land while the user is looking at another day.
    pub fn event_mut_anywhere(&mut self, id: &str) -> Option<&mut Event> {
        if self
            .detail_event
            .as_ref()
            .is_some_and(|event| event.id == id)
        {
            return self.detail_event.as_mut();
        }
        if let Some(event) = self.events.iter_mut().find(|event| event.id == id) {
            return Some(event);
        }
        self.cache
            .values_mut()
            .flatten()
            .find(|event| event.id == id)
    }

    /// Applies a dragged time to the event and remembers it as unconfirmed.
    ///
    /// The chip moves at once; the lighter shade and the write to Google
    /// follow. Returns false when the event is unknown or already has a write
    /// in flight, so a second drag cannot race the first.
    pub fn begin_time_change(
        &mut self,
        id: &str,
        start: DateTime<Local>,
        end: DateTime<Local>,
    ) -> bool {
        if self.pending.contains_key(id) {
            return false;
        }
        let Some(event) = self.events.iter_mut().find(|event| event.id == id) else {
            return false;
        };

        let previous_start = event.start;
        let previous_end = event.end;
        event.start = start;
        event.end = end;
        self.pending.insert(
            id.to_owned(),
            PendingTime {
                start,
                end,
                previous_start,
                previous_end,
            },
        );
        true
    }

    /// The unconfirmed time change for an event, if there is one.
    pub fn pending_time(&self, id: &str) -> Option<PendingTime> {
        self.pending.get(id).copied()
    }

    /// Google accepted the write, so the chip is final again.
    pub fn confirm_time_change(&mut self, id: &str) {
        self.pending.remove(id);
    }

    /// Plain events belong on regular Google calendars, not task lists or
    /// the app's calendar, which is reserved for TimeSlay's own kinds.
    pub fn event_calendars<'a>(
        &'a self,
        settings: &'a CalendarSettings,
    ) -> impl Iterator<Item = &'a Calendar> {
        self.calendars
            .iter()
            .filter(|calendar| !calendar.is_task_list() && !calendar.is_app_calendar(settings))
    }

    /// The configured regular calendar, otherwise the primary or first one.
    pub fn default_calendar_id(&self, settings: &CalendarSettings) -> Option<String> {
        let calendars: Vec<_> = self.event_calendars(settings).collect();
        settings
            .default_calendar
            .as_ref()
            .and_then(|id| calendars.iter().find(|calendar| &calendar.id == id))
            .or_else(|| calendars.iter().find(|calendar| calendar.primary))
            .or_else(|| calendars.first())
            .map(|calendar| calendar.id.clone())
    }

    /// The calendars `event` can be on, including its current one.
    ///
    /// Empty when it has to stay where it is: tasks, habits, sleep and anything
    /// else living on the TimeSlay calendar belong to the app, and the TimeSlay
    /// calendar is never a destination. Also empty with nowhere else to go.
    pub fn move_targets(&self, event: &Event, settings: &CalendarSettings) -> Vec<&Calendar> {
        if event.is_task() || event.details.habit || event.details.sleep.is_some() {
            return Vec::new();
        }
        let targets: Vec<&Calendar> = self
            .calendars
            .iter()
            .filter(|calendar| !calendar.is_task_list() && !calendar.is_app_calendar(settings))
            .collect();
        if targets.len() < 2
            || !targets
                .iter()
                .any(|calendar| calendar.id == event.calendar_id)
        {
            return Vec::new();
        }
        targets
    }

    /// Records that Google moved an event, so a further save reaches the right
    /// calendar and its chip is right before the next sync.
    pub fn event_moved(&mut self, id: &str, calendar_id: &str) {
        let color = self.calendar_color(calendar_id);
        let apply = |events: &mut Vec<Event>| {
            for event in events.iter_mut().filter(|event| event.id == id) {
                event.calendar_id = calendar_id.to_owned();
                if event.details.color_id.is_none() {
                    event.color = color;
                }
            }
        };
        apply(&mut self.events);
        for events in self.cache.values_mut() {
            apply(events);
        }
        if let Some(event) = self.detail_event.as_mut().filter(|event| event.id == id) {
            event.calendar_id = calendar_id.to_owned();
            if event.details.color_id.is_none() {
                event.color = color;
            }
        }
        if let Some(database) = &self.day_cache {
            day_cache::update_events(database, apply);
        }
    }

    /// The color a calendar's events are drawn in, or the default event color
    /// when the calendar is unknown.
    pub fn calendar_color(&self, id: &str) -> Color {
        self.calendars
            .iter()
            .find(|calendar| calendar.id == id)
            .map_or(colors::DEFAULT_EVENT, |calendar| calendar.color)
    }

    /// Opens the add-event popup on `calendar_id`, spanning `start` to `end`.
    pub fn begin_draft(
        &mut self,
        calendar_id: String,
        start: DateTime<Local>,
        end: DateTime<Local>,
    ) {
        self.draft = Some(Draft::new(calendar_id, start, end));
        self.open_event = None;
        self.edit_draft = None;
        self.menu_open = false;
        self.search.open = false;
        self.month_open = false;
    }

    /// Opens the edit-event popup for the event with `id`, pre-filled from its
    /// current values.
    pub fn begin_edit(&mut self, id: &str) {
        let Some(event) = self.event(id) else {
            return;
        };
        self.edit_draft = Some(EditDraft::from_event(event));
        self.open_event = None;
        self.draft = None;
        self.menu_open = false;
        self.search.open = false;
        self.month_open = false;
    }

    /// Drops a deleted task, every cached copy of it, and the calendar blocks
    /// that were standing in for it.
    pub fn forget_task(&mut self, id: &str) {
        let drop_copies = |events: &mut Vec<Event>| {
            events.retain(|event| event.id != id);
        };
        if let Some(database) = &self.day_cache {
            day_cache::update_events(database, drop_copies);
        }
        drop_copies(&mut self.events);
        for events in self.cache.values_mut() {
            drop_copies(events);
        }
        if self.open_event.as_ref().is_some_and(|open| {
            !self.events.iter().any(|event| &event.id == open)
                && !self
                    .cache
                    .values()
                    .any(|events| events.iter().any(|event| &event.id == open))
        }) {
            self.open_event = None;
        }
        self.write_day(self.day, &self.events);
    }

    /// Puts a freshly created event into the day in view, keeping the order the
    /// grid expects.
    pub fn add_event(&mut self, event: Event) {
        self.events.push(event);
        self.events
            .sort_by(|left, right| left.start.cmp(&right.start).then(right.end.cmp(&left.end)));
        self.write_day(self.day, &self.events);
    }

    /// Google refused the write, so the event goes back to the time it had.
    pub fn fail_time_change(&mut self, id: &str) {
        let Some(pending) = self.pending.remove(id) else {
            return;
        };
        if let Some(event) = self.event_mut_anywhere(id) {
            event.start = pending.previous_start;
            event.end = pending.previous_end;
        }
    }

    pub fn all_day_events<'a>(
        &'a self,
        settings: &'a CalendarSettings,
    ) -> impl Iterator<Item = &'a Event> {
        let blocks = self.time_block_keys(settings);
        self.visible_events(settings).filter(move |event| {
            event.all_day
                && (!event.is_task()
                    || (event.start.date_naive() != crate::timeblocking::UNDATED
                        && event.start.date_naive() == self.day)
                    || (!settings.hide_overdue_tasks && crate::timeblocking::overdue(event)))
                && !omit_managed_tasks_event(event, &blocks, settings)
        })
    }

    pub fn timed_events<'a>(
        &'a self,
        settings: &'a CalendarSettings,
    ) -> impl Iterator<Item = &'a Event> {
        timed_in(
            self.visible_events(settings),
            self.time_block_keys(settings),
            self.day,
            settings,
        )
    }

    /// Agenda entries include tasks normally tucked into their time blocks.
    pub fn task_list_events_on<'a>(
        &'a self,
        day: NaiveDate,
        settings: &'a CalendarSettings,
    ) -> Vec<&'a Event> {
        let start = at_clock(day, settings.day_start_hour(), 0);
        let end = at_clock(day.succ_opt().unwrap_or(day), settings.day_start_hour(), 0);
        let mut events: Vec<_> = self
            .events_on(day)
            .iter()
            .filter(|event| event.details.sleep.is_none())
            .filter(|event| self.event_enabled(event, settings))
            .filter(|event| {
                if event.all_day && event.is_task() {
                    event.start.date_naive() == day
                } else if event.all_day {
                    event_on_day(event, day, settings.day_start_hour())
                } else {
                    event.start < end && event.end > start
                }
            })
            .collect();
        events.sort_by(|a, b| (a.start, a.end, &a.id).cmp(&(b.start, b.end, &b.id)));
        events
    }

    /// The timed events of `day` that the grid would draw, in start order,
    /// for any day that has been loaded, not only the one in view. Empty for
    /// a day that was never visited.
    pub fn timed_events_on<'a>(
        &'a self,
        day: NaiveDate,
        settings: &'a CalendarSettings,
    ) -> Vec<&'a Event> {
        let events: &[Event] = if day == self.day {
            &self.events
        } else {
            match self.cache.get(&day) {
                Some(events) => events,
                None => return Vec::new(),
            }
        };

        let blocks = events
            .iter()
            .filter(|event| crate::timeblocking::is_block(event, settings))
            .map(crate::timeblocking::block_key)
            .collect();
        let mut found: Vec<&Event> = timed_in(
            events
                .iter()
                .filter(|event| self.event_enabled(event, settings)),
            blocks,
            day,
            settings,
        )
        .collect();
        found.sort_by_key(|event| (event.start, event.end));
        found
    }

    /// Every day whose events are in hand: the day in view and the days
    /// visited before it, oldest first.
    pub fn loaded_days(&self) -> Vec<NaiveDate> {
        let mut days: Vec<NaiveDate> = self
            .cache
            .keys()
            .copied()
            .filter(|day| *day != self.day)
            .collect();
        days.push(self.day);
        days.sort();
        days
    }

    /// Keys of the time blocks currently loaded, so a scheduled task can be
    /// recognized as already drawn beside its block.
    fn time_block_keys(&self, settings: &CalendarSettings) -> Vec<String> {
        self.events
            .iter()
            .filter(|event| crate::timeblocking::is_block(event, settings))
            .map(crate::timeblocking::block_key)
            .collect()
    }
}

/// The events of `events` that sit on the grid for `day`.
fn timed_in<'a>(
    events: impl Iterator<Item = &'a Event> + 'a,
    blocks: Vec<String>,
    day: NaiveDate,
    settings: &'a CalendarSettings,
) -> impl Iterator<Item = &'a Event> {
    let start = at_clock(day, settings.day_start_hour(), 0);
    let end = at_clock(day.succ_opt().unwrap_or(day), settings.day_start_hour(), 0);
    events.filter(move |event| {
        !event.all_day
            && event.start < end
            && event.end > start
            && !omit_managed_tasks_event(event, &blocks, settings)
    })
}

/// Whether this event is a task's calendar copy that is already drawn as part
/// of its time block.
///
/// A task that already sits beside its time block is drawn there, so the grid
/// does not draw it again. A plain event on the app calendar is a time block and
/// is always drawn; a task given a time of its own, with no block, still
/// occupies the grid because that chip is the only place its time is visible.
fn omit_managed_tasks_event(
    event: &Event,
    time_blocks: &[String],
    settings: &CalendarSettings,
) -> bool {
    let Some(managed) = settings.app_calendar.as_deref() else {
        return false;
    };
    event.is_calendar_task()
        && event.calendar_id == managed
        && crate::timeblocking::metadata(event)
            .block
            .as_ref()
            .is_some_and(|key| time_blocks.iter().any(|block| block == key))
}

impl Default for Store {
    fn default() -> Self {
        Self::new(0)
    }
}

/// The calendar day `now` belongs to, given the hour the day ends.
///
/// With the default midnight boundary this is simply today's date. With a later
/// boundary the small hours still belong to the day that is ending, so a 3am
/// end of day keeps 1am on the previous date. The date an event is stored under
/// never changes; this only decides which day's view it lands in.
pub fn day_at(now: DateTime<Local>, day_end_hour: u32) -> NaiveDate {
    let date = now.date_naive();
    if now.hour() < day_end_hour.min(24) {
        date.pred_opt().unwrap_or(date)
    } else {
        date
    }
}

/// Minutes from local midnight, used to place events and the needle.
pub fn minutes_from_midnight(when: DateTime<Local>) -> i32 {
    when.hour() as i32 * 60 + when.minute() as i32
}

pub fn clock_label(when: DateTime<Local>) -> String {
    format!("{:02}:{:02}", when.hour(), when.minute())
}

/// An hour of the day as the grid's gutter spells it, matching Google
/// Calendar: "12 AM", "9 AM", "12 PM", "1 PM".
pub fn hour_label(hour: u32) -> String {
    match hour % 24 {
        0 => "12 AM".to_owned(),
        12 => "12 PM".to_owned(),
        1..=11 => format!("{} AM", hour % 24),
        hour => format!("{} PM", hour - 12),
    }
}

pub fn event_time_label(event: &Event) -> String {
    if deadline::is_marker(event) && !event.all_day {
        return clock_label(event.start);
    }
    if event.all_day {
        "All day".to_owned()
    } else {
        format!("{} – {}", clock_label(event.start), clock_label(event.end))
    }
}

/// The next few things coming up, in the order they will happen.
///
/// A timed event counts until it has finished, so the one in progress is still
/// offered; a task counts until it is ticked off, since an all-day task has no
/// hour to sit next to. The list is capped at `limit` so the timer view stays a
/// glance rather than a second calendar.
pub fn upcoming<'a>(
    events: impl Iterator<Item = &'a Event>,
    now: DateTime<Local>,
    limit: usize,
) -> Vec<&'a Event> {
    let mut upcoming: Vec<&Event> = events
        .filter(|event| {
            if event.is_task() {
                !event.completed
            } else {
                event.end > now
            }
        })
        .collect();

    upcoming.sort_by_key(|event| event.start);
    upcoming.truncate(limit);
    upcoming
}

/// Connects the Google account, blocking until the browser comes back.
pub fn connect(client_id: String, client_secret: String) -> Result<oauth::Tokens, String> {
    let tokens = oauth::authorize(&client_id, &client_secret)?;
    oauth::save_tokens(&tokens).map_err(|error| error.to_string())?;
    Ok(tokens)
}

/// Pulls the calendars and the events of `day`, refreshing the access token if
/// needed. The day's window ends at `day_end_hour`, so a day that ends at 3am
/// still includes the small hours of the next morning.
pub fn sync_day(
    client: &GoogleClient,
    disabled: &[String],
    day: NaiveDate,
    day_end_hour: u32,
    app_calendar: Option<&str>,
) -> Result<Snapshot, String> {
    let token = client.access_token()?;
    api::fetch_day(&token, disabled, day, day_end_hour, app_calendar)
}

pub fn sync_week(
    client: &GoogleClient,
    disabled: &[String],
    anchor: NaiveDate,
    days: u32,
    day_end_hour: u32,
    app_calendar: Option<&str>,
) -> Result<Snapshot, String> {
    sync_range(
        client,
        disabled,
        anchor,
        &week_dates(anchor, days),
        day_end_hour,
        app_calendar,
    )
}

/// Fetch the complete visible range for a week or month in a single request.
pub fn sync_range(
    client: &GoogleClient,
    disabled: &[String],
    anchor: NaiveDate,
    dates: &[NaiveDate],
    day_end_hour: u32,
    app_calendar: Option<&str>,
) -> Result<Snapshot, String> {
    let first = *dates.first().ok_or("Calendar range is empty")?;
    let last = *dates.last().ok_or("Calendar range is empty")?;
    let token = client.access_token()?;
    api::fetch_range(
        &token,
        disabled,
        anchor,
        first,
        last,
        day_end_hour,
        app_calendar,
    )
}

/// Seven or eight days starting at the selected day, or for nine, eight
/// starting at the selected day with the day before it in front.
pub fn week_dates(anchor: NaiveDate, days: u32) -> Vec<NaiveDate> {
    let (start, end) = match days.clamp(7, 9) {
        7 => (0, 7),
        8 => (0, 8),
        _ => (-1, 8),
    };
    (start..end)
        .filter_map(|offset| anchor.checked_add_signed(chrono::Duration::days(offset)))
        .collect()
}

/// Connects a habit event to a habit, or lets it go when `habit_id` is `None`.
pub fn link_habit_event(
    client: &GoogleClient,
    calendar_id: &str,
    event_id: &str,
    habit_id: Option<&str>,
) -> Result<(), String> {
    let token = client.access_token()?;
    api::link_habit_event(&token, calendar_id, event_id, habit_id)
        .map_err(|error| describe_event_write_error(error).to_string())
}

/// Pulls every habit event series and past completion from the app's calendar.
pub fn sync_habit_listing(
    client: &GoogleClient,
    app_calendar: Option<&str>,
) -> Result<api::HabitListing, String> {
    let token = client.access_token()?;
    api::fetch_habit_listing(&token, app_calendar)
}

/// Pulls every habit master from the app's TimeSlay calendar.
pub fn sync_habits(
    client: &GoogleClient,
    app_calendar: Option<&str>,
) -> Result<Vec<api::HabitEvent>, String> {
    let token = client.access_token()?;
    api::fetch_habits(&token, app_calendar)
}

/// Load a calendar series repeat schedule without losing the selected occurrence.
/// The Habits list starts at the first occurrence of its representative series.
pub fn habit_edit_draft(
    client: &GoogleClient,
    event: &Event,
) -> Result<(Event, EditDraft), String> {
    let token = client.access_token()?;
    let master = api::fetch_event(
        &token,
        event,
        event
            .details
            .recurring_event_id
            .as_deref()
            .unwrap_or(&event.id),
    )?;
    let occurrence = if event.id == master.id && !master.details.recurrence.is_empty() {
        api::fetch_first_occurrence(&token, &master)?
    } else if event.id == master.id {
        master.clone()
    } else {
        event.clone()
    };
    let draft = EditDraft::from_habit_occurrence(&occurrence, &master);
    Ok((occurrence, draft))
}

// Writes. Each of these saves the change locally and hands it to
// `operations`, which sends it to Google in the background through the
// matching function in `send`; none of them talks to Google itself, so none
// needs a client.

/// Creates a habit event using the shared details editor's fields, counting
/// towards the habit `habit_id`.
pub fn create_habit(
    draft: &EditDraft,
    color_id: Option<&str>,
    habit_id: &str,
) -> Result<api::HabitEvent, String> {
    let id = operations::new_id()?;
    let recurrence = draft.recurrence_lines()?;
    let calendar = if draft.calendar_id.is_empty() {
        crate::config::Config::load()
            .calendar
            .app_calendar
            .ok_or("Sync Google once to discover the TimeSlay calendar")?
    } else {
        draft.calendar_id.clone()
    };
    let event = Event {
        id: id.clone(),
        calendar_id: calendar.clone(),
        title: draft.title.clone(),
        start: draft.start,
        end: draft.end,
        all_day: draft.all_day,
        color: colors::DEFAULT_EVENT,
        completed: false,
        details: Details {
            description: draft.description.clone(),
            habit: true,
            recurrence: recurrence.clone(),
            projects: draft.projects.ids.clone(),
            labels: draft.labels.clone(),
            options: draft.options.clone(),
            ..Default::default()
        },
    };
    operations::submit(
        operations::Job::CreateHabit {
            id: id.clone(),
            draft: draft.clone(),
            color: color_id.map(str::to_owned),
            habit: habit_id.into(),
        },
        Some(&event),
        &calendar,
        &id,
    )?;
    Ok(api::HabitEvent {
        id,
        calendar_id: calendar,
        title: draft.title.clone(),
        notes: draft.description.clone(),
        start: draft.start,
        end: draft.end,
        recurrence,
        color_id: color_id.map(str::to_owned),
        habit_id: Some(habit_id.into()),
    })
}

/// Deletes a habit master and every occurrence in its series.
pub fn delete_habit(calendar_id: &str, event_id: &str) -> Result<(), String> {
    delete_event_scope(calendar_id, event_id, SaveScope::All, false)
}

pub fn disconnect() {
    oauth::clear_tokens();
}

/// Ticks a Google Task off, or puts it back.
pub fn set_task_completed(
    calendar_id: &str,
    event_id: &str,
    completed: bool,
) -> Result<(), String> {
    let task = operations::cached_event(calendar_id, event_id)?.ok_or("Task is not cached")?;
    set_linked_task_completed(&task, completed)
}

/// Completion retains the event and updates its visual completion marker.
pub fn set_linked_task_completed(task: &Event, completed: bool) -> Result<(), String> {
    set_linked_task_status(
        task,
        if completed {
            crate::timeblocking::TaskStatus::Completed
        } else {
            crate::timeblocking::TaskStatus::Active
        },
    )
    .map(|_| ())
}

pub fn set_linked_task_status(
    task: &Event,
    status: crate::timeblocking::TaskStatus,
) -> Result<Event, String> {
    let mut event = task.clone();
    let mut db = crate::task_db::open()?;
    event.details.task_data = Some(db.import(task)?);
    crate::timeblocking::apply_status(&mut event, status);
    operations::submit(
        operations::Job::Status {
            task: task.clone(),
            status,
        },
        Some(&event),
        &task.calendar_id,
        &task.id,
    )?;
    Ok(event)
}

/// Deletes a task from Google Tasks, and the calendar blocks that point at it.
pub fn delete_task(task: &Event, app_calendar: Option<&str>) -> Result<(), String> {
    operations::submit(
        operations::Job::DeleteTask {
            task: task.clone(),
            calendar: app_calendar.map(str::to_owned),
        },
        None,
        &task.calendar_id,
        &task.id,
    )
}

/// Deletes one task instance by list and ID, straight away. Used by the habits
/// migration, which has no cached event to hand to [`delete_task`].
pub fn delete_task_instance(
    client: &GoogleClient,
    list_id: &str,
    task_id: &str,
) -> Result<(), String> {
    let token = client.access_token()?;
    Ok(api::delete_task(&token, list_id, task_id)?)
}

/// Store completion on one habit occurrence.
pub fn set_habit_completed(event: &Event) -> Result<(), String> {
    operations::submit(
        operations::Job::HabitCompleted {
            event: event.clone(),
        },
        Some(event),
        &event.calendar_id,
        &event.id,
    )
}

/// Sets only an event's location on Google.
pub fn set_event_location(calendar_id: &str, event_id: &str, location: &str) -> Result<(), String> {
    let mut event =
        operations::cached_event(calendar_id, event_id)?.ok_or("Event is not cached")?;
    event.details.location = location.into();
    operations::submit(
        operations::Job::Location {
            calendar: calendar_id.into(),
            id: event_id.into(),
            location: location.into(),
        },
        Some(&event),
        calendar_id,
        event_id,
    )
}

/// Moves an event's start and end on Google.
pub fn set_event_time(
    calendar_id: &str,
    event_id: &str,
    start: DateTime<Local>,
    end: DateTime<Local>,
    notify: bool,
) -> Result<(), String> {
    let mut event =
        operations::cached_event(calendar_id, event_id)?.ok_or("Event is not cached")?;
    event.start = start;
    event.end = end;
    operations::submit(
        operations::Job::Time {
            calendar: calendar_id.into(),
            id: event_id.into(),
            start,
            end,
            notify,
        },
        Some(&event),
        calendar_id,
        event_id,
    )
}

/// Creates a new event on Google and returns the ID it has until Google
/// assigns one (see `operations`).
#[allow(clippy::too_many_arguments)]
pub fn create_event(
    calendar_id: &str,
    title: &str,
    description: &str,
    color_id: Option<&str>,
    start: DateTime<Local>,
    end: DateTime<Local>,
    recurrence: &[String],
    projects: &[u64],
    all_day: bool,
    options: Option<&event_options::EventOptions>,
    meeting: Option<&meeting::Form>,
) -> Result<api::Inserted, String> {
    let id = operations::new_id()?;
    let event = Event {
        id: id.clone(),
        calendar_id: calendar_id.into(),
        title: title.into(),
        start,
        end,
        all_day,
        color: colors::DEFAULT_EVENT,
        completed: false,
        details: Details {
            description: description.into(),
            recurrence: recurrence.to_vec(),
            projects: projects.to_vec(),
            options: options.cloned().unwrap_or_default(),
            ..Default::default()
        },
    };
    operations::submit(
        operations::Job::Create {
            id: id.clone(),
            calendar: calendar_id.into(),
            title: title.into(),
            description: description.into(),
            color: color_id.map(str::to_owned),
            start,
            end,
            recurrence: recurrence.to_vec(),
            projects: projects.to_vec(),
            all_day,
            options: options.cloned(),
            meeting: meeting.cloned(),
        },
        Some(&event),
        calendar_id,
        &id,
    )?;
    Ok(api::Inserted {
        id,
        ..Default::default()
    })
}

/// Creates a time block on the app's single TimeSlay calendar and returns its
/// id together with the calendar it landed on. Blocks do not repeat, so there
/// are no recurrence lines.
#[allow(clippy::too_many_arguments)]
pub fn create_time_block(
    known_calendar: Option<&str>,
    title: &str,
    description: &str,
    color_id: Option<&str>,
    start: DateTime<Local>,
    end: DateTime<Local>,
    projects: &[u64],
) -> Result<(String, String), String> {
    let calendar = known_calendar.ok_or("Sync Google once to discover the TimeSlay calendar")?;
    let inserted = create_event(
        calendar,
        title,
        description,
        color_id,
        start,
        end,
        &[],
        projects,
        false,
        None,
        None,
    )?;
    Ok((inserted.id, calendar.into()))
}

/// Updates an existing event on Google: title, description, start and end.
#[allow(clippy::too_many_arguments)]
pub fn update_event(
    calendar_id: &str,
    event_id: &str,
    title: &str,
    description: &str,
    color_id: Option<&str>,
    start: DateTime<Local>,
    end: DateTime<Local>,
    recurrence: Option<&[String]>,
    projects: &[u64],
    all_day: bool,
    labels: &[String],
    options: Option<&event_options::EventOptions>,
) -> Result<(), String> {
    let original = operations::cached_event(calendar_id, event_id)?.ok_or("Event is not cached")?;
    let mut draft = EditDraft::from_event(&original);
    draft.title = title.into();
    draft.description = description.into();
    draft.start = start;
    draft.end = end;
    draft.projects.ids = projects.to_vec();
    draft.all_day = all_day;
    draft.labels = labels.to_vec();
    if let Some(options) = options {
        draft.options = options.clone();
    }
    if let Some(recurrence) = recurrence {
        draft.rule = crate::recurrence::Rule::from_recurrence(
            start.date_naive(),
            &crate::recurrence::Recurrence::from_lines(recurrence),
        );
        draft.recurrence_changed = true;
    }
    save_event_scope(&original, &draft, SaveScope::Current, description, color_id).map(|_| ())
}

/// Save an occurrence or a selected portion of its recurring series.
pub fn save_event_scope(
    original: &Event,
    draft: &EditDraft,
    scope: SaveScope,
    description: &str,
    color_id: Option<&str>,
) -> Result<api::Saved, String> {
    let mut event = original.clone();
    event.title = draft.title.clone();
    event.calendar_id = draft.calendar_id.clone();
    event.start = draft.start;
    event.end = draft.end;
    event.all_day = draft.all_day;
    event.details.description = description.into();
    event.details.options = draft.options.clone();
    event.details.projects = draft.projects.ids.clone();
    event.details.labels = draft.labels.clone();
    if let Some(data) = &mut event.details.task_data {
        data.metadata.deadline = draft.deadline()?;
        data.metadata.projects = draft.projects.ids.clone();
        data.metadata.labels = draft.labels.clone();
        if !draft.all_day {
            data.metadata.minutes = (draft.end - draft.start).num_minutes().clamp(0, 10080) as u32;
        }
    }
    if draft.recurrence_changed {
        event.details.recurrence = draft.recurrence_lines()?;
    }
    operations::submit(
        operations::Job::Edit {
            original: Box::new(original.clone()),
            draft: draft.clone(),
            scope,
            description: description.into(),
            color: color_id.map(str::to_owned),
        },
        Some(&event),
        &original.calendar_id,
        &original.id,
    )?;
    if event.details.options.child_metadata.force || original.details.options.child_metadata.force {
        let tasks: Vec<Event> = {
            let mut db = crate::task_db::open()?;
            let mut q = db
                .connection()
                .prepare("SELECT event FROM google_task_records")
                .map_err(|e| e.to_string())?;
            let rows = q
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            rows.map(|row| {
                serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())
            })
            .collect::<Result<_, _>>()?
        };
        let key = crate::timeblocking::block_key(original);
        let inherited = crate::timeblocking::inherited_metadata(&event);
        for task in tasks
            .into_iter()
            .filter(|t| crate::timeblocking::metadata(t).block.as_deref() == Some(&key))
        {
            crate::task_edit::save(
                task,
                &CalendarSettings::default(),
                if event.details.options.child_metadata.force {
                    crate::task_edit::Change::Inherit(inherited.clone())
                } else {
                    crate::task_edit::Change::ReleaseInheritance
                },
                0,
                crate::task_edit::google_access,
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(api::Saved {
        link: original.details.link.clone(),
        zoom_needed: false,
    })
}

/// The invitations on `calendar` waiting for an answer. Blocking: run on a
/// worker thread.
pub fn sync_invitations(client: &GoogleClient, calendar: &Calendar) -> Result<Vec<Event>, String> {
    let token = client.access_token()?;
    api::fetch_invitations(&token, calendar)
}

/// Sends the user's answer to an invitation.
pub fn reply_to_event(
    calendar_id: &str,
    event_id: &str,
    email: &str,
    response: meeting::Response,
) -> Result<(), String> {
    let mut event =
        operations::cached_event(calendar_id, event_id)?.ok_or("Invitation is not cached")?;
    event.details.attendance.set_own_response(response);
    operations::submit(
        operations::Job::Reply {
            event: event.clone(),
            email: email.into(),
            response,
        },
        Some(&event),
        calendar_id,
        event_id,
    )
}

/// Deletes a single event on Google. Tasks are Google Tasks, not calendar
/// events, so callers should guard with [`Event::is_task`] before reaching
/// here.
pub fn delete_event(calendar_id: &str, event_id: &str) -> Result<(), String> {
    delete_event_scope(calendar_id, event_id, SaveScope::Current, false)
}

/// Delete an occurrence or a selected portion of its series.
pub fn delete_event_scope(
    calendar_id: &str,
    event_id: &str,
    scope: SaveScope,
    notify: bool,
) -> Result<(), String> {
    operations::submit(
        operations::Job::Delete {
            calendar: calendar_id.into(),
            id: event_id.into(),
            scope,
            notify,
        },
        None,
        calendar_id,
        event_id,
    )
}

/// Turns a read-only-token rejection into something a user can act on,
/// keeping the error's kind.
fn describe_task_write_error(error: GoogleError) -> GoogleError {
    match error.kind {
        google::Kind::MissingScope => GoogleError {
            detail: "Reconnect Google in Settings so Tasks can be ticked off".to_owned(),
            context: String::new(),
            ..error
        },
        _ => error,
    }
}

/// Turns a read-only-token rejection into something a user can act on.
fn describe_event_write_error(error: GoogleError) -> GoogleError {
    match error.kind {
        google::Kind::MissingScope => GoogleError {
            detail: "Reconnect Google in Settings so events can be moved".to_owned(),
            context: String::new(),
            ..error
        },
        _ => error,
    }
}

/// Sending writes to Google. Only `operations` calls these, with the thread
/// marked as replaying so the layers below talk to Google rather than record
/// another write.
pub(crate) mod send {
    use super::*;

    pub fn create_habit(
        client: &GoogleClient,
        draft: &EditDraft,
        color_id: Option<&str>,
        habit_id: &str,
    ) -> Result<api::HabitEvent, GoogleError> {
        let token = client.access_token()?;
        let mut body = api::habit_body(
            &draft.title,
            &draft.description,
            draft.start,
            draft.end,
            &draft.recurrence_lines()?,
            color_id,
        );
        draft.options.write(&mut body)?;
        projects::write(&mut body, &draft.projects.ids)?;
        body["extendedProperties"]["private"][crate::habits::LINK_PROPERTY] = habit_id.into();
        body["extendedProperties"]["private"]["ttLabels"] = serde_json::to_string(&draft.labels)
            .map_err(|error| error.to_string())?
            .into();
        Ok(api::insert_habit(
            &token,
            (!draft.calendar_id.is_empty()).then_some(draft.calendar_id.as_str()),
            &body,
        )?)
    }

    pub fn task_status(
        client: &GoogleClient,
        task: &Event,
        status: crate::timeblocking::TaskStatus,
    ) -> Result<Event, GoogleError> {
        let token = client.access_token()?;
        let mut database = crate::task_db::open()?;
        save_task_status(&mut database, task, status, |updated| {
            write_task_status(&token, updated)
        })
    }

    /// Persist the distinction between ignored and completed only after Google
    /// accepts it. A failed remote or local write restores the previous state.
    pub(super) fn save_task_status(
        database: &mut crate::task_db::Database,
        task: &Event,
        status: crate::timeblocking::TaskStatus,
        mut write: impl FnMut(&Event) -> Result<(), GoogleError>,
    ) -> Result<Event, GoogleError> {
        let mut previous = task.clone();
        previous.details.task_data = Some(database.import(task)?);
        let mut updated = previous.clone();
        crate::timeblocking::apply_status(&mut updated, status);
        if let Err(error) = write(&updated).and_then(|()| {
            database
                .save(&updated, updated.details.task_data.as_ref().unwrap())
                .map_err(GoogleError::from)
        }) {
            // The message changes, but the failure keeps the original kind.
            let detail = match write(&previous) {
                Ok(()) => format!("Task status could not be saved; the task was restored: {error}"),
                Err(rollback) => format!(
                    "Task status failed: {error}. Restoring the task also failed: {rollback}. Sync before retrying."
                ),
            };
            return Err(GoogleError {
                detail,
                context: String::new(),
                ..error
            });
        }
        Ok(updated)
    }

    fn write_task_status(token: &str, task: &Event) -> Result<(), GoogleError> {
        if task.is_calendar_task() {
            return api::patch(
                &calendar_tasks::event_url(task),
                token,
                &calendar_tasks::body(task)?.to_string(),
                "Save task completion",
            );
        }
        let native = task.task_ref().ok_or("Invalid Google Task")?;
        if let Some(data) = &task.details.task_data {
            task_metadata::save(token, task, data)?;
        }
        api::set_task_completed(token, native.list, native.task, task.completed)
            .map_err(describe_task_write_error)?;
        Ok(())
    }

    /// The chip's list can be stale after the task is moved onto the
    /// scheduling list. The Tasks API is asked for the task wherever it still
    /// lives. Every calendar event that records this task is removed too,
    /// including a second block created for the same task.
    pub fn delete_task(
        client: &GoogleClient,
        task: &Event,
        app_calendar: Option<&str>,
    ) -> Result<(), GoogleError> {
        let token = client.access_token()?;
        if task.is_calendar_task() {
            calendar_tasks::remove(&token, &calendar_tasks::event_url(task))?;
        } else {
            let native = task.task_ref().ok_or("Invalid Google Task")?;
            match api::delete_task(&token, native.list, native.task) {
                Ok(()) => {}
                Err(e) if matches!(e.status, Some(404 | 410)) => {}
                Err(e) => return Err(e),
            }
        }
        let _ = app_calendar;
        Ok(())
    }

    pub fn habit_completed(client: &GoogleClient, event: &Event) -> Result<(), GoogleError> {
        let token = client.access_token()?;
        api::patch_habit_completed(&token, event).map_err(describe_event_write_error)
    }

    pub fn event_location(
        client: &GoogleClient,
        calendar_id: &str,
        event_id: &str,
        location: &str,
    ) -> Result<(), GoogleError> {
        let token = client.access_token()?;
        api::patch_event_location(&token, calendar_id, event_id, location)
            .map_err(describe_event_write_error)
    }

    pub fn event_time(
        client: &GoogleClient,
        calendar_id: &str,
        event_id: &str,
        start: DateTime<Local>,
        end: DateTime<Local>,
        notify: bool,
    ) -> Result<(), GoogleError> {
        let token = client.access_token()?;
        api::patch_event_time(&token, calendar_id, event_id, start, end, notify)
            .map_err(describe_event_write_error)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_event(
        client: &GoogleClient,
        calendar_id: &str,
        title: &str,
        description: &str,
        color_id: Option<&str>,
        start: DateTime<Local>,
        end: DateTime<Local>,
        recurrence: &[String],
        projects: &[u64],
        all_day: bool,
        options: Option<&event_options::EventOptions>,
        meeting: Option<&meeting::Form>,
    ) -> Result<api::Inserted, GoogleError> {
        let token = client.access_token()?;
        api::insert_event(
            &token,
            calendar_id,
            title,
            description,
            color_id,
            start,
            end,
            recurrence,
            projects,
            all_day,
            options,
            meeting,
        )
        .map_err(describe_event_write_error)
    }

    pub fn event_scope(
        client: &GoogleClient,
        original: &Event,
        draft: &EditDraft,
        scope: SaveScope,
        description: &str,
        color_id: Option<&str>,
    ) -> Result<api::Saved, GoogleError> {
        let token = client.access_token()?;
        api::save_event_scope(&token, original, draft, scope, description, color_id)
            .map_err(describe_event_write_error)
    }

    pub fn reply(
        client: &GoogleClient,
        calendar_id: &str,
        event_id: &str,
        email: &str,
        response: meeting::Response,
    ) -> Result<(), GoogleError> {
        let token = client.access_token()?;
        api::reply_to_event(&token, calendar_id, event_id, email, response)
            .map_err(describe_event_write_error)
    }

    pub fn delete_event_scope(
        client: &GoogleClient,
        calendar_id: &str,
        event_id: &str,
        scope: SaveScope,
        notify: bool,
    ) -> Result<(), GoogleError> {
        let token = client.access_token()?;
        match api::delete_event_scope(&token, calendar_id, event_id, scope, notify) {
            Ok(()) => Ok(()),
            // Already gone is what a delete wants.
            Err(error) if error.status == Some(404) => Ok(()),
            Err(error) => Err(describe_event_write_error(error)),
        }
    }
}

pub fn day_heading(day: NaiveDate, today: NaiveDate) -> String {
    let weekday = weekday_short(day);
    let month = month_short(day.month());
    if day == today {
        format!("Today · {weekday} {month} {}", day.day())
    } else {
        format!("{weekday} {month} {}", day.day())
    }
}

fn weekday_short(day: NaiveDate) -> &'static str {
    match day.weekday().number_from_monday() {
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        6 => "Sat",
        _ => "Sun",
    }
}

pub fn month_short(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        _ => "Dec",
    }
}

/// A compact date for calendar navigation, e.g. "Nov. 5" or "May 5".
pub fn month_day_label(day: NaiveDate) -> String {
    let month = month_short(day.month());
    let period = if day.month() == 5 { "" } else { "." };
    format!("{month}{period} {}", day.day())
}

fn month_long(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        _ => "December",
    }
}

/// The month a date belongs to, as its first day.
pub fn first_of_month(day: NaiveDate) -> NaiveDate {
    day.with_day(1).unwrap_or(day)
}

/// The month picker's title, e.g. "October 2026".
pub fn month_heading(month: NaiveDate) -> String {
    format!("{} {}", month_long(month.month()), month.year())
}

/// The days laid out in a month grid, padded to whole weeks.
///
/// The leading and trailing days belong to the neighbouring months so every
/// row is full; the caller dims them.
pub fn month_grid(month: NaiveDate, first_day: first_day::FirstDayOfWeek) -> Vec<NaiveDate> {
    let first = first_of_month(month);
    let lead = first_day.days_into_week(first.weekday());
    let start = first - chrono::Duration::days(lead);
    let cells = lead + days_in_month(first) as i64;
    let weeks = (cells + 6) / 7;

    (0..weeks * 7)
        .map(|offset| start + chrono::Duration::days(offset))
        .collect()
}

/// Two whole weeks, starting with the week that holds `anchor`.
pub fn two_week_dates(anchor: NaiveDate, first_day: first_day::FirstDayOfWeek) -> Vec<NaiveDate> {
    let start = first_day.week_start(anchor);
    (0..14)
        .map(|offset| start + chrono::Duration::days(offset))
        .collect()
}

/// How many days the month containing `month` has.
fn days_in_month(month: NaiveDate) -> u32 {
    let first = first_of_month(month);
    let next = first.checked_add_months(Months::new(1)).unwrap_or(first);
    (next - first).num_days() as u32
}

/// Sample day used by tests and screenshot generation, so the day view can be
/// exercised without talking to Google.
pub fn sample_snapshot() -> Snapshot {
    let day = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
    let at = |hour, minute| {
        day.and_hms_opt(hour, minute, 0)
            .and_then(|naive| naive.and_local_timezone(Local).single())
            .unwrap_or_else(crate::clock::now)
    };

    let personal = Calendar {
        id: "personal".into(),
        name: "Personal".into(),
        color: colors::calendar("16").unwrap_or(colors::DEFAULT_EVENT),
        primary: true,
        selected_by_google: true,
    };
    let work = Calendar {
        id: "work".into(),
        name: "Work".into(),
        color: colors::event("6").unwrap_or(colors::DEFAULT_EVENT),
        primary: false,
        selected_by_google: true,
    };
    let tasks = Calendar {
        id: "tasks".into(),
        name: "Tasks".into(),
        color: colors::event("2").unwrap_or(colors::DEFAULT_EVENT),
        primary: false,
        selected_by_google: true,
    };

    Snapshot {
        calendars: vec![personal.clone(), work.clone(), tasks.clone()],
        events: vec![
            Event {
                id: "all-day".into(),
                calendar_id: personal.id.clone(),
                title: "Team offsite".into(),
                start: at(0, 0),
                end: at(0, 0),
                all_day: true,
                color: personal.color,
                completed: false,
                details: Details::default(),
            },
            Event {
                id: "standup".into(),
                calendar_id: work.id.clone(),
                title: "Standup".into(),
                start: at(9, 0),
                end: at(9, 30),
                all_day: false,
                color: work.color,
                completed: false,
                details: Details::default(),
            },
            Event {
                id: "deep".into(),
                calendar_id: tasks.id.clone(),
                title: "Write the Cal tab".into(),
                start: at(9, 15),
                end: at(11, 0),
                all_day: false,
                color: tasks.color,
                completed: false,
                details: Details::default(),
            },
            Event {
                id: "lunch".into(),
                calendar_id: personal.id.clone(),
                title: "Lunch".into(),
                start: at(12, 0),
                end: at(13, 0),
                all_day: false,
                color: personal.color,
                completed: false,
                details: Details::default(),
            },
            Event {
                id: "review".into(),
                calendar_id: work.id.clone(),
                title: "Design review".into(),
                start: at(14, 0),
                end: at(15, 0),
                all_day: false,
                color: work.color,
                completed: false,
                details: Details::default(),
            },
        ],
        day,
        fetched_at: at(10, 5),
        notice: None,
    }
}

#[cfg(test)]
mod tests {
    use super::first_day::FirstDayOfWeek;
    #[test]
    fn only_plain_events_can_move_and_never_onto_the_app_calendar() {
        let mut snapshot = sample_snapshot();
        snapshot.calendars.push(Calendar {
            id: "app".into(),
            name: "TimeSlay".into(),
            color: colors::DEFAULT_EVENT,
            primary: false,
            selected_by_google: true,
        });
        snapshot.calendars.push(Calendar {
            id: "tasks:list".into(),
            name: "My Tasks".into(),
            color: colors::DEFAULT_EVENT,
            primary: false,
            selected_by_google: true,
        });
        let plain = snapshot
            .events
            .iter()
            .find(|event| !event.is_task() && !event.details.habit)
            .unwrap()
            .clone();
        let store = Store::with_snapshot(snapshot);
        let settings = CalendarSettings::default();

        let targets = store.move_targets(&plain, &settings);
        assert!(targets.iter().any(|c| c.id == plain.calendar_id));
        assert!(targets.iter().all(|c| c.id != "app" && !c.is_task_list()));

        let mut on_app = plain.clone();
        on_app.calendar_id = "app".into();
        assert!(store.move_targets(&on_app, &settings).is_empty());

        let mut habit = plain.clone();
        habit.details.habit = true;
        assert!(store.move_targets(&habit, &settings).is_empty());

        let mut unknown = plain;
        unknown.calendar_id = "gone".into();
        assert!(store.move_targets(&unknown, &settings).is_empty());
    }

    /// Only says anything where these dates are clock changes, which CI makes
    /// sure of by running with `TZ=America/Los_Angeles`.
    #[test]
    fn a_clock_reading_resolves_across_daylight_saving() {
        let reading = |year, month, day, hour, minute| {
            NaiveDate::from_ymd_opt(year, month, day)
                .unwrap()
                .and_hms_opt(hour, minute, 0)
                .unwrap()
        };
        let gap_reading = reading(2027, 3, 14, 2, 30);
        if Local.from_local_datetime(&gap_reading).single().is_none()
            && Local.from_local_datetime(&gap_reading).earliest().is_none()
        {
            let gap = local_time(gap_reading);
            assert_eq!(gap.naive_local(), reading(2027, 3, 14, 3, 30));
            assert_eq!(gap.date_naive(), gap_reading.date());
        } else {
            return;
        }

        let twice = local_time(reading(2026, 11, 1, 1, 30));
        assert_eq!(twice.naive_local(), reading(2026, 11, 1, 1, 30));
        assert_eq!(
            Some(twice),
            Local
                .from_local_datetime(&reading(2026, 11, 1, 1, 30))
                .earliest()
        );
    }

    use super::*;

    #[test]
    fn a_new_draft_can_cover_several_whole_days_and_go_back_to_its_times() {
        let day = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        let mut draft = Draft::new(
            "cal".into(),
            at_clock(day(6), 9, 30),
            at_clock(day(6), 11, 0),
        );
        assert_eq!(draft.end_date_text, "2026-11-06");

        draft.set_all_day(true);
        // One whole day ends at the next midnight, as Google stores it, but
        // the field shows the last day covered.
        assert_eq!(
            (draft.start, draft.end),
            (at_clock(day(6), 0, 0), at_clock(day(7), 0, 0))
        );
        assert_eq!(draft.end_date_text, "2026-11-06");

        draft.set_end_date_text("2026-11-08".into());
        assert_eq!(draft.end, at_clock(day(9), 0, 0));
        // Moving the start keeps the three days.
        draft.set_date_text("2026-11-10".into());
        assert_eq!(
            (draft.start, draft.end),
            (at_clock(day(10), 0, 0), at_clock(day(13), 0, 0))
        );
        assert_eq!(draft.end_date_text, "2026-11-12");

        // Back to timed: the typed clock times return on the first and last days.
        draft.set_all_day(false);
        assert_eq!(
            (draft.start, draft.end),
            (at_clock(day(10), 9, 30), at_clock(day(12), 11, 0))
        );
        assert_eq!(draft.end_date_text, "2026-11-12");
    }

    #[test]
    fn a_timed_draft_ending_at_midnight_becomes_one_whole_day() {
        let day = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        let mut draft = Draft::new(
            "cal".into(),
            at_clock(day(6), 22, 0),
            at_clock(day(7), 0, 0),
        );
        draft.set_all_day(true);
        assert_eq!(
            (draft.start, draft.end),
            (at_clock(day(6), 0, 0), at_clock(day(7), 0, 0))
        );
    }

    #[test]
    fn all_day_drafts_repeat_with_dates_rather_than_instants() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        let mut draft = Draft::new("cal".into(), at_clock(day, 9, 0), at_clock(day, 10, 0));
        draft.set_all_day(true);
        draft.rule.repeat = crate::recurrence::RepeatMode::Daily;
        draft.rule.ends = crate::recurrence::EndKind::OnDate;
        draft.rule.until = "2026-11-20".into();
        draft.rule.skips = vec![NaiveDate::from_ymd_opt(2026, 11, 8).unwrap()];
        let lines = draft.recurrence_lines().unwrap();
        assert!(lines[0].ends_with(";UNTIL=20261120"), "{lines:?}");
        assert_eq!(lines[1], "EXDATE;VALUE=DATE:20261108");
    }

    #[test]
    fn editing_an_all_day_event_shows_its_last_day_as_the_end() {
        let day = |d: u32| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        let mut event = sample_snapshot().events[0].clone();
        event.all_day = true;
        event.start = at_clock(day(6), 0, 0);
        event.end = at_clock(day(9), 0, 0);
        let mut draft = EditDraft::from_event(&event);
        assert_eq!(draft.end_date_text, "2026-11-08");
        draft.set_end_date_text("2026-11-10".into());
        assert_eq!(draft.end, at_clock(day(11), 0, 0));
        draft.set_all_day(false);
        assert!(!draft.all_day);
        assert_eq!(
            (draft.start, draft.end),
            (at_clock(day(6), 9, 0), at_clock(day(10), 10, 0))
        );
    }

    #[test]
    fn habit_drafts_keep_the_selected_occurrence_and_load_only_the_series_schedule() {
        let mut master = sample_snapshot().events[1].clone();
        master.id = "habit-series".into();
        master.details.habit = true;
        master.details.recurrence = vec!["RRULE:FREQ=DAILY;COUNT=10".into()];
        let mut occurrence = master.clone();
        occurrence.id = "habit-instance".into();
        occurrence.details.recurring_event_id = Some(master.id.clone());
        occurrence.details.recurrence.clear();
        occurrence.start += chrono::Duration::days(4);
        occurrence.end += chrono::Duration::days(4);
        // An individual occurrence can have its own title, notes and options.
        occurrence.title = "Stretch gently".into();
        occurrence.details.description = "After the run".into();
        occurrence.details.projects = vec![7];
        occurrence.details.options.transparency = event_options::Availability::Free;
        let draft = EditDraft::from_habit_occurrence(&occurrence, &master);
        assert_eq!(draft.id, occurrence.id);
        assert_eq!(draft.series_id.as_deref(), Some("habit-series"));
        assert_eq!(draft.start, occurrence.start);
        assert_eq!(draft.end, occurrence.end);
        assert_eq!(draft.date_text, occurrence.start.date_naive().to_string());
        assert_eq!(draft.title, occurrence.title);
        assert_eq!(draft.description, occurrence.details.description);
        assert_eq!(draft.projects.ids, [7]);
        assert_eq!(draft.options, occurrence.details.options);
        assert_eq!(draft.rule.repeat, crate::recurrence::RepeatMode::Daily);
        assert_eq!(draft.rule.count, "10");
        assert_eq!(draft.rule.ends, crate::recurrence::EndKind::After);
        assert!(!draft.recurrence_changed);
        assert!(!draft.loading);
    }

    #[test]
    fn task_edits_and_deletions_preserve_unrelated_cached_dates_on_disk() {
        let scratch = day_cache::Scratch::new("task-edits");
        let snapshot = sample_snapshot();
        let day = snapshot.day;
        let next = day.succ_opt().unwrap();
        let mut neighbor = snapshot.events[1].clone();
        neighbor.id = "neighbor-event".into();
        neighbor.start += chrono::Duration::days(1);
        neighbor.end += chrono::Duration::days(1);
        let mut task = snapshot.events[1].clone();
        task.id = "task:one".into();
        task.calendar_id = "tasks:list".into();
        day_cache::save(scratch.path(), day, &[task.clone()], 0).unwrap();
        day_cache::save(scratch.path(), next, &[neighbor.clone(), task.clone()], 0).unwrap();
        let mut store = Store::with_snapshot(snapshot);
        store.cache_days_in(Some(scratch.path().to_path_buf()));
        task.title = "Updated task".into();
        store.replace_cached_task("task:one", &task);
        let cached = day_cache::load(scratch.path(), next, 0).unwrap();
        assert!(cached.contains(&neighbor));
        assert!(cached.contains(&task));
        store.forget_task("task:one");
        let cached = day_cache::load(scratch.path(), next, 0).unwrap();
        assert!(cached.contains(&neighbor));
        assert!(!cached.iter().any(Event::is_task));
    }

    #[test]
    fn recent_sync_time_and_calendar_sources_survive_restart() {
        let scratch = day_cache::Scratch::new("sync-state");
        let mut snapshot = sample_snapshot();
        snapshot.fetched_at = crate::clock::now();
        let mut previous = Store::new(0);
        previous.cache_days_in(Some(scratch.path().to_path_buf()));
        previous.apply_snapshot(snapshot.clone());
        let mut restarted = Store::new(0);
        restarted.cache_days_in(Some(scratch.path().to_path_buf()));
        assert_eq!(restarted.last_sync, Some(snapshot.fetched_at));
        assert_eq!(restarted.calendars, snapshot.calendars);
        assert!(!restarted.sync_due(
            std::time::Duration::from_secs(3600),
            snapshot.fetched_at + chrono::Duration::minutes(59)
        ));
        assert!(restarted.sync_due(
            std::time::Duration::from_secs(3600),
            snapshot.fetched_at + chrono::Duration::minutes(60)
        ));
    }

    #[test]
    fn calendar_cache_coverage_ignores_shared_tasks_and_off_day_events() {
        let snapshot = sample_snapshot();
        let day = snapshot.day;
        let mut store = Store::with_snapshot(snapshot);
        assert!(store.has_calendar_events_on(day));
        for event in &mut store.events {
            event.id = format!("task:{}", event.id);
        }
        assert!(!store.has_calendar_events_on(day));
        for event in &mut store.events {
            event.id = ids::bare_task(&event.id).to_owned();
            event.start += chrono::Duration::days(5);
            event.end += chrono::Duration::days(5);
        }
        assert!(!store.has_calendar_events_on(day));
        store.events.clear();
        assert!(!store.has_calendar_events_on(day));
    }

    #[test]
    fn a_range_result_after_navigation_is_cached_without_changing_the_active_view() {
        let snapshot = sample_snapshot();
        let fetched_day = snapshot.day;
        let mut store = Store::with_snapshot(snapshot.clone());
        let current = fetched_day + chrono::Duration::days(10);
        store.set_day(current, fetched_day);
        let visible = week_dates(current, 7);
        store.load_cached_days(&visible);
        store.sync = SyncState::Syncing;
        store.cache_fetched_snapshot(snapshot.clone(), &[fetched_day]);
        assert_eq!(store.day, current);
        assert!(store.events.is_empty());
        assert_eq!(store.visible_days, visible);
        assert_eq!(store.sync, SyncState::Idle);
        assert_eq!(store.events_on(fetched_day), snapshot.events.as_slice());
    }

    #[test]
    fn upcoming_keeps_unfinished_things_in_time_order() {
        fn make(
            id: &str,
            calendar_id: &str,
            start: DateTime<Local>,
            end: DateTime<Local>,
            all_day: bool,
            completed: bool,
        ) -> Event {
            Event {
                id: id.into(),
                calendar_id: calendar_id.into(),
                title: id.into(),
                start,
                end,
                all_day,
                color: colors::DEFAULT_EVENT,
                completed,
                details: Details::default(),
            }
        }

        let now = crate::clock::now();
        let at = |minutes: i64| now + chrono::Duration::minutes(minutes);

        let events = [
            make("later", "work", at(60), at(90), false, false),
            make("task:done", "tasks:list", at(10), at(20), false, true),
            make("past", "work", at(-60), at(-30), false, false),
            make("soon", "work", at(5), at(15), false, false),
            make("task:today", "tasks:list", at(0), at(0), true, false),
        ];

        let ids: Vec<&str> = upcoming(events.iter(), now, 5)
            .iter()
            .map(|event| event.id.as_str())
            .collect();

        // The finished task and the event that is already over are dropped; an
        // all-day task is kept even though it has no hour of its own.
        assert_eq!(ids, vec!["task:today", "soon", "later"]);
    }

    #[test]
    fn upcoming_is_capped_at_the_limit() {
        fn make(id: &str, start: DateTime<Local>) -> Event {
            Event {
                id: id.into(),
                calendar_id: "work".into(),
                title: id.into(),
                start,
                end: start + chrono::Duration::minutes(30),
                all_day: false,
                color: colors::DEFAULT_EVENT,
                completed: false,
                details: Details::default(),
            }
        }

        let now = crate::clock::now();
        let at = |minutes: i64| now + chrono::Duration::minutes(minutes);
        let events = [make("a", at(10)), make("b", at(20)), make("c", at(30))];

        assert_eq!(upcoming(events.iter(), now, 2).len(), 2);
    }

    #[test]
    fn disabling_a_calendar_hides_its_events() {
        let mut settings = CalendarSettings::default();
        let store = Store::with_snapshot(sample_snapshot());

        assert_eq!(store.visible_events(&settings).count(), 5);

        settings.disabled.push("work".into());
        let remaining: Vec<_> = store
            .visible_events(&settings)
            .map(|event| event.title.as_str())
            .collect();

        assert!(!remaining.contains(&"Standup"));
        assert!(remaining.contains(&"Lunch"));
    }

    #[test]
    fn disabling_tasks_hides_both_all_day_and_timed_tasks() {
        let mut snapshot = sample_snapshot();
        let day = snapshot.day;
        snapshot.events.push(Event {
            id: "task:abc".into(),
            calendar_id: "tasks:list".into(),
            title: "File taxes".into(),
            start: day
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap(),
            end: day
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap(),
            all_day: true,
            color: colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        });
        // A task with a time follows the same aggregate toggle.
        snapshot.events.push(Event {
            id: "task:def".into(),
            calendar_id: "tasks:list".into(),
            title: "Stand-up".into(),
            start: day
                .and_hms_opt(9, 30, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap(),
            end: day
                .and_hms_opt(10, 0, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap(),
            all_day: false,
            color: colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        });

        let store = Store::with_snapshot(snapshot);
        let mut settings = CalendarSettings::default();
        assert_eq!(
            store
                .visible_events(&settings)
                .filter(|e| e.is_task())
                .count(),
            2
        );

        settings.show_tasks = false;
        let tasks: Vec<_> = store
            .visible_events(&settings)
            .filter(|e| e.is_task())
            .map(|e| e.title.as_str())
            .collect();
        assert!(tasks.is_empty());
    }

    #[test]
    fn a_saved_task_color_is_put_back_on_the_list_and_its_events() {
        let mut snapshot = sample_snapshot();
        snapshot.calendars[2].id = "tasks:list".into();
        snapshot.calendars[2].color = colors::TASK;
        for event in &mut snapshot.events {
            if event.calendar_id == "tasks" {
                event.calendar_id = "tasks:list".into();
            }
        }

        let mut store = Store::with_snapshot(snapshot);
        let mut settings = CalendarSettings::default();
        let red = crate::hex::parse("#DC2127").unwrap();
        settings.set_task_color("tasks:list", Some(red));

        store.apply_task_colors(&settings);

        let list = store
            .calendars
            .iter()
            .find(|c| c.id == "tasks:list")
            .unwrap();
        assert_eq!(list.color, red);
        for event in store
            .events
            .iter()
            .filter(|e| e.calendar_id == "tasks:list")
        {
            assert_eq!(event.color, red);
        }
    }

    #[test]
    fn a_task_list_without_a_saved_color_goes_back_to_google_blue() {
        let mut snapshot = sample_snapshot();
        snapshot.calendars[2].id = "tasks:list".into();
        snapshot.calendars[2].color = crate::hex::parse("#123456").unwrap();

        let mut store = Store::with_snapshot(snapshot);
        store.apply_task_colors(&CalendarSettings::default());

        let list = store
            .calendars
            .iter()
            .find(|c| c.id == "tasks:list")
            .unwrap();
        assert_eq!(list.color, colors::TASK);
    }

    #[test]
    fn habit_colors_stay_the_same_across_disk_cache_and_sync() {
        let scratch = day_cache::Scratch::new("habit-colors");
        let mut snapshot = sample_snapshot();
        let day = snapshot.day;
        let settings = CalendarSettings::default();
        snapshot.events.truncate(1);
        let habit = &mut snapshot.events[0];
        habit.details.habit = true;
        habit.details.color_id = None;
        // Google supplies the inherited calendar red; an older cache may
        // instead contain the purple already applied by the desktop app.
        habit.color = colors::HABIT;
        let id = habit.id.clone();
        let mut previous = Store::with_snapshot(empty_snapshot(day));
        previous.cache_days_in(Some(scratch.path().to_path_buf()));
        previous.apply_snapshot(snapshot.clone());
        let mut store = Store::with_snapshot(empty_snapshot(day));
        store.apply_task_colors(&settings);
        store.cache_days_in(Some(scratch.path().to_path_buf()));
        assert_eq!(
            store.event(&id).unwrap().display_color(&settings),
            colors::HABIT
        );
        snapshot.events[0].color = colors::event("4").unwrap();
        store.apply_snapshot(snapshot);
        // Render correctly even before any caller reapplies display settings.
        assert_eq!(
            store.event(&id).unwrap().display_color(&settings),
            colors::HABIT
        );
    }

    #[test]
    fn habits_use_local_defaults_unless_google_has_an_explicit_color() {
        let mut event = sample_snapshot().events.remove(0);
        event.details.habit = true;
        let mut settings = CalendarSettings::default();
        event.color = colors::event("4").unwrap();
        event.details.color_id = None;
        assert_eq!(event.display_color(&settings), colors::HABIT);
        recolor_event(&mut event, &settings);
        assert_eq!(event.color, colors::HABIT);
        settings.set_habit_color(Some(colors::TASK));
        assert_eq!(event.display_color(&settings), colors::TASK);
        event.details.color_id = Some("4".into());
        // Even a stale cached rendered color cannot hide the Google override.
        assert_eq!(event.display_color(&settings), colors::event("4").unwrap());
        recolor_event(&mut event, &settings);
        assert_eq!(event.color, colors::event("4").unwrap());
    }

    #[test]
    fn real_calendars_keep_the_color_google_gave_them() {
        let mut store = Store::with_snapshot(sample_snapshot());
        let before = store.calendars[0].color;

        store.apply_task_colors(&CalendarSettings::default());

        assert_eq!(store.calendars[0].color, before);
    }

    #[test]
    fn a_read_only_token_is_explained_as_a_reconnect() {
        let scope = describe_task_write_error(google::GoogleError::from_status(
            "Google Tasks",
            403,
            "Request had insufficient authentication scopes.",
        ));
        assert!(scope.to_string().contains("Reconnect"));
        // The friendlier message keeps the original kind.
        assert_eq!(scope.kind, google::Kind::MissingScope);
        assert_eq!(
            describe_task_write_error(google::GoogleError::transport("", "Network down"))
                .to_string(),
            "Network down"
        );
    }

    #[test]
    fn today_heading_says_today() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        assert!(day_heading(day, day).starts_with("Today"));
        assert!(!day_heading(day, day.succ_opt().unwrap()).contains("Today"));
    }

    #[test]
    fn the_day_boundary_follows_the_end_of_day_setting() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        let at = |hour, minute| {
            day.and_hms_opt(hour, minute, 0)
                .unwrap()
                .and_local_timezone(Local)
                .single()
                .unwrap()
        };

        // Midnight: the date is simply today's, all day long.
        assert_eq!(day_at(at(0, 30), 0), day);
        assert_eq!(day_at(at(23, 30), 0), day);

        // A 3am boundary keeps the small hours on the day that is ending.
        assert_eq!(day_at(at(1, 0), 3), day.pred_opt().unwrap());
        assert_eq!(day_at(at(2, 59), 3), day.pred_opt().unwrap());
        assert_eq!(day_at(at(3, 0), 3), day);
        assert_eq!(day_at(at(23, 0), 3), day);
    }

    #[test]
    fn zoom_stays_inside_the_readable_range() {
        let mut store = Store::new(0);
        store.zoom(100.0);
        assert_eq!(store.hour_height, MAX_HOUR_HEIGHT);
        store.zoom(0.001);
        assert_eq!(store.hour_height, MIN_HOUR_HEIGHT);
        store.zoom(0.0);
        assert_eq!(store.hour_height, MIN_HOUR_HEIGHT);
    }

    #[test]
    fn stepping_in_and_back_out_lands_exactly_on_the_default() {
        let mut store = Store::new(0);
        for _ in 0..3 {
            store.zoom(ZOOM_STEP);
        }
        for _ in 0..3 {
            store.zoom(1.0 / ZOOM_STEP);
        }
        assert_eq!(store.hour_height, DEFAULT_HOUR_HEIGHT);
    }

    #[test]
    fn reset_zoom_returns_to_the_default() {
        let mut store = Store::new(0);
        store.zoom(2.0);
        assert_eq!(store.zoom_ratio(), 2.0);
        store.reset_zoom();
        assert_eq!(store.hour_height, DEFAULT_HOUR_HEIGHT);
        assert_eq!(store.zoom_ratio(), 1.0);
    }

    #[test]
    fn the_month_grid_starts_on_a_monday_and_covers_the_whole_month() {
        // October 2026 starts on a Thursday and has 31 days.
        let month = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let grid = month_grid(month, FirstDayOfWeek::Monday);

        assert_eq!(grid.len() % 7, 0, "the grid has to be whole weeks");
        assert_eq!(grid[0].weekday(), chrono::Weekday::Mon);
        assert_eq!(grid[0], NaiveDate::from_ymd_opt(2026, 9, 28).unwrap());
        assert_eq!(
            grid.last().unwrap(),
            &NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
        );

        // Every day of the month is present exactly once.
        for day in 1..=31 {
            let date = NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
            assert_eq!(grid.iter().filter(|d| **d == date).count(), 1);
        }
    }

    #[test]
    fn the_month_grid_is_only_as_tall_as_it_has_to_be() {
        // February 2027 starts on a Monday and has 28 days: exactly four weeks.
        let month = NaiveDate::from_ymd_opt(2027, 2, 1).unwrap();
        assert_eq!(month_grid(month, FirstDayOfWeek::Monday).len(), 28);

        // August 2026 starts on a Saturday and has 31 days, so it needs six.
        let month = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        assert_eq!(month_grid(month, FirstDayOfWeek::Monday).len(), 42);
        // With Saturday first that same month opens on its first cell.
        assert_eq!(month_grid(month, FirstDayOfWeek::Saturday).len(), 35);
    }

    #[test]
    fn the_month_grid_and_labels_follow_the_first_day_of_the_week() {
        // October 2026 starts on a Thursday.
        let month = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        for (first_day, weekday, start, labels) in [
            (
                FirstDayOfWeek::Sunday,
                chrono::Weekday::Sun,
                (9, 27),
                ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
            ),
            (
                FirstDayOfWeek::Monday,
                chrono::Weekday::Mon,
                (9, 28),
                ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
            ),
            (
                FirstDayOfWeek::Saturday,
                chrono::Weekday::Sat,
                (9, 26),
                ["Sat", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri"],
            ),
        ] {
            let grid = month_grid(month, first_day);
            assert_eq!(grid.len() % 7, 0);
            assert_eq!(grid[0].weekday(), weekday);
            assert_eq!(
                grid[0],
                NaiveDate::from_ymd_opt(2026, start.0, start.1).unwrap()
            );
            assert_eq!(first_day.short_labels(), labels);
            assert_eq!(first_day.initials()[0], &labels[0][..1]);
            for day in 1..=31 {
                let date = NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
                assert_eq!(grid.iter().filter(|d| **d == date).count(), 1);
            }
        }
    }

    #[test]
    fn two_weeks_start_with_the_week_holding_the_anchor() {
        let date = |month, day| NaiveDate::from_ymd_opt(2026, month, day).unwrap();
        for (anchor, first_day, start) in [
            // A Thursday.
            (date(10, 29), FirstDayOfWeek::Sunday, date(10, 25)),
            (date(10, 29), FirstDayOfWeek::Monday, date(10, 26)),
            (date(10, 29), FirstDayOfWeek::Saturday, date(10, 24)),
            // The first day itself stays put.
            (date(10, 25), FirstDayOfWeek::Sunday, date(10, 25)),
            (date(10, 26), FirstDayOfWeek::Monday, date(10, 26)),
            (date(10, 24), FirstDayOfWeek::Saturday, date(10, 24)),
            // The last day of the week still belongs to it.
            (date(10, 31), FirstDayOfWeek::Sunday, date(10, 25)),
            (date(11, 1), FirstDayOfWeek::Monday, date(10, 26)),
            (date(10, 30), FirstDayOfWeek::Saturday, date(10, 24)),
        ] {
            let days = two_week_dates(anchor, first_day);
            assert_eq!(days.len(), 14);
            assert_eq!(days[0], start, "{anchor} with {first_day}");
            assert_eq!(days[13], start + chrono::Duration::days(13));
        }
    }

    #[test]
    fn the_first_day_of_the_week_defaults_to_sunday_and_round_trips() {
        let settings: crate::config::Config = toml::from_str("[calendar]\n").unwrap();
        assert_eq!(settings.calendar.first_day_of_week, FirstDayOfWeek::Sunday);
        for first_day in FirstDayOfWeek::ALL {
            let parsed: crate::config::Config = toml::from_str(&format!(
                "[calendar]\nfirst_day_of_week = '{}'\n",
                first_day.name()
            ))
            .unwrap();
            assert_eq!(parsed.calendar.first_day_of_week, first_day);
            assert_eq!(FirstDayOfWeek::parse(first_day.name()), Some(first_day));
        }
    }

    #[test]
    fn the_month_heading_spells_the_month_and_year() {
        let month = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        assert_eq!(month_heading(month), "October 2026");
        assert_eq!(
            first_of_month(NaiveDate::from_ymd_opt(2026, 10, 17).unwrap()),
            month
        );
    }

    #[test]
    fn moving_the_day_drops_the_events_that_belonged_to_the_old_one() {
        let mut store = Store::with_snapshot(sample_snapshot());
        let today = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        assert!(!store.events.is_empty());

        let next = today.succ_opt().unwrap();
        store.set_day(next, today);

        assert_eq!(store.day, next);
        assert!(
            store.events.is_empty(),
            "the old day's events must not linger"
        );
        assert!(!store.follow_today, "a moved day no longer follows today");

        store.set_day(today, today);
        assert!(
            store.follow_today,
            "back on today, the view follows it again"
        );
    }

    #[test]
    fn stepping_back_to_a_day_brings_its_events_back() {
        let mut store = Store::with_snapshot(sample_snapshot());
        let today = store.day;
        let parked = store.events.len();
        assert!(parked > 0);

        store.set_day(today.succ_opt().unwrap(), today);
        assert!(store.events.is_empty());

        store.set_day(today, today);
        assert_eq!(
            store.events.len(),
            parked,
            "the cached day should come back"
        );
    }

    #[test]
    fn forgetting_the_cache_drops_the_days_it_held() {
        let mut store = Store::with_snapshot(sample_snapshot());
        let today = store.day;

        store.set_day(today.succ_opt().unwrap(), today);
        store.forget_cached_days();
        store.set_day(today, today);

        assert!(
            store.events.is_empty(),
            "a forgotten day has to be fetched again"
        );
    }

    /// A snapshot with nothing in it, standing in for a run that has not
    /// synced yet.
    fn empty_snapshot(day: NaiveDate) -> Snapshot {
        Snapshot {
            calendars: Vec::new(),
            events: Vec::new(),
            day,
            fetched_at: crate::clock::now(),
            notice: None,
        }
    }

    /// The day in view, the day before it and the day after it are never
    /// dropped, and ten more days beyond that window survive; everything
    /// further out goes.
    #[test]
    fn the_cache_keeps_the_day_in_view_its_neighbours_and_ten_more() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();
        let all: Vec<NaiveDate> = (-40..=40)
            .map(|offset| today + chrono::Duration::days(i64::from(offset)))
            .collect();

        let dropped = days_to_drop(all.clone(), today);
        let kept: Vec<NaiveDate> = all
            .into_iter()
            .filter(|day| !dropped.contains(day))
            .collect();

        assert_eq!(
            kept.len(),
            13,
            "the day in view, its neighbours and ten more"
        );
        for offset in [-6, -1, 0, 1, 6] {
            let day = today + chrono::Duration::days(i64::from(offset));
            assert!(kept.contains(&day), "day {offset} should stay");
        }
        for offset in [-7, 7, 40] {
            let day = today + chrono::Duration::days(i64::from(offset));
            assert!(!kept.contains(&day), "day {offset} should go");
        }
    }

    /// A day parked by one run is painted by the next one, straight from the
    /// cache directory, without a sync having run.
    #[test]
    fn a_day_parked_by_one_run_is_painted_by_the_next() {
        let scratch = day_cache::Scratch::new("across-runs");
        let directory = scratch.path().to_path_buf();

        let mut first = Store::with_snapshot(sample_snapshot());
        first.cache_days_in(Some(directory.clone()));
        let today = first.day;
        let parked = first.events.clone();
        assert!(!parked.is_empty());

        let tomorrow = today.succ_opt().unwrap();
        first.set_day(tomorrow, today);
        drop(first);

        // The next run starts with nothing in memory, and tomorrow was never
        // fetched by anyone.
        let mut second = Store::with_snapshot(empty_snapshot(tomorrow));
        second.cache_days_in(Some(directory));
        assert!(second.events.is_empty(), "tomorrow has never been fetched");

        second.set_day(today, today);
        assert_eq!(second.events, parked, "the day should come back from disk");
    }

    /// Startup and navigation recolor stale disk entries without a network fetch.

    /// The months on disk are trimmed by the same rule as the memory cache.
    #[test]
    fn visited_months_remain_on_disk_after_distant_navigation() {
        let scratch = day_cache::Scratch::new("pruning");
        let directory = scratch.path().to_path_buf();
        let today = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();

        for offset in -600..=600 {
            let day = today + chrono::Duration::days(i64::from(offset));
            day_cache::save(&directory, day, &[], 0).unwrap();
        }

        let mut store = Store::with_snapshot(empty_snapshot(today));
        store.cache_days_in(Some(directory.clone()));

        let left = day_cache::days(&directory);
        // September and October are the current month and its neighbour;
        // nothing reaches November.
        assert!(
            left.len() > 30,
            "all visited months remain available offline"
        );
        let september = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        let october = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        assert!(left.contains(&september), "september should stay on disk");
        assert!(left.contains(&october), "october should stay on disk");
    }

    #[test]
    fn forgetting_the_cache_drops_the_days_on_disk_too() {
        let scratch = day_cache::Scratch::new("forgetting");
        let directory = scratch.path().to_path_buf();

        let mut store = Store::with_snapshot(sample_snapshot());
        store.cache_days_in(Some(directory.clone()));
        let today = store.day;

        store.set_day(today.succ_opt().unwrap(), today);
        assert!(
            day_cache::days(&directory).contains(&today),
            "the day we left is on disk"
        );

        store.forget_cached_days();
        assert!(
            day_cache::days(&directory).is_empty(),
            "the boundary moved, so the days filed under it go too"
        );
        assert!(store.events.is_empty());
    }

    /// `State::new` builds its store by hand, and a state built by hand must
    /// not read or rewrite the days the real app has cached.
    #[test]
    fn a_store_built_by_hand_keeps_its_days_in_memory() {
        assert!(Store::new(0).day_cache.is_none());
        assert!(Store::with_snapshot(sample_snapshot()).day_cache.is_none());
    }

    /// The standup in the sample day, and a time an hour later.
    fn standup_and_later() -> (Store, DateTime<Local>, DateTime<Local>) {
        let store = Store::with_snapshot(sample_snapshot());
        let event = store.event("standup").unwrap();
        let start = event.start + chrono::Duration::hours(1);
        let end = event.end + chrono::Duration::hours(1);
        (store, start, end)
    }

    #[test]
    fn a_dragged_time_moves_the_chip_and_marks_it_pending() {
        let (mut store, start, end) = standup_and_later();

        assert!(store.begin_time_change("standup", start, end));

        let event = store.event("standup").unwrap();
        assert_eq!(event.start, start);
        assert_eq!(event.end, end);
        assert_eq!(store.pending_time("standup").map(|p| p.start), Some(start));
    }

    #[test]
    fn confirming_a_drag_makes_the_chip_final() {
        let (mut store, start, end) = standup_and_later();
        store.begin_time_change("standup", start, end);

        store.confirm_time_change("standup");

        assert!(store.pending_time("standup").is_none());
        assert_eq!(store.event("standup").unwrap().start, start);
    }

    #[test]
    fn a_refused_drag_puts_the_old_time_back() {
        let (mut store, start, end) = standup_and_later();
        let before = store.event("standup").unwrap().start;
        store.begin_time_change("standup", start, end);

        store.fail_time_change("standup");

        assert!(store.pending_time("standup").is_none());
        assert_eq!(store.event("standup").unwrap().start, before);
    }

    #[test]
    fn a_second_drag_on_a_pending_event_is_refused() {
        let (mut store, start, end) = standup_and_later();
        assert!(store.begin_time_change("standup", start, end));

        // A second drag would race the first write, so it is turned away.
        assert!(!store.begin_time_change("standup", start, end));
        assert!(store.begin_time_change("lunch", start, end));
    }

    #[test]
    fn a_sync_does_not_snap_a_pending_chip_back() {
        let (mut store, start, end) = standup_and_later();
        store.begin_time_change("standup", start, end);

        // Google has not processed the write yet, so the snapshot still carries
        // the old time. The pending time has to win.
        store.apply_snapshot(sample_snapshot());

        assert_eq!(store.event("standup").unwrap().start, start);
        assert_eq!(store.event("standup").unwrap().end, end);
    }

    #[test]
    fn a_new_draft_starts_as_an_event() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let start = day
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .unwrap();
        let draft = Draft::new("personal".into(), start, start + chrono::Duration::hours(1));
        assert_eq!(draft.kind, DraftKind::Event);
    }

    #[test]
    fn a_drag_on_an_unknown_event_is_refused() {
        let (mut store, start, end) = standup_and_later();
        assert!(!store.begin_time_change("nobody", start, end));
    }

    #[test]
    fn a_refused_drag_reaches_an_event_parked_in_the_cache() {
        let (mut store, start, end) = standup_and_later();
        let day = store.day;
        store.begin_time_change("standup", start, end);

        // The user steps to another day while the write is in flight.
        store.set_day(day.succ_opt().unwrap(), day);
        store.fail_time_change("standup");

        // Coming back paints the old time, not the one Google refused.
        store.set_day(day, day);
        assert_ne!(store.event("standup").unwrap().start, start);
    }
    #[test]
    fn editing_event_dates_keeps_the_other_endpoint_and_clock_times() {
        let mut event = sample_snapshot().events.remove(1);
        event.start = at_clock(NaiveDate::from_ymd_opt(2026, 10, 29).unwrap(), 22, 0);
        event.end = at_clock(NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(), 7, 30);
        let mut draft = EditDraft::from_event(&event);
        assert_eq!(draft.end_date_text, "2026-10-30");
        draft.set_date_text("2026-10-28".into());
        assert_eq!(draft.end, event.end);
        assert_eq!(draft.start.time(), event.start.time());
        let start = draft.start;
        draft.set_end_date_text("2026-11-01".into());
        assert_eq!(draft.start, start);
        assert_eq!(draft.end.date_naive().to_string(), "2026-11-01");
        assert_eq!(draft.end.time(), event.end.time());
    }

    #[test]
    fn an_events_recurrence_is_loaded_into_the_shared_editor_and_written_back() {
        let mut event = sample_snapshot().events.remove(1);
        event.details.recurrence = vec!["RRULE:FREQ=WEEKLY;BYDAY=MO,WE".into()];
        let draft = EditDraft::from_event(&event);
        assert_eq!(
            draft.rule.summary(event.start.date_naive()),
            "Weekly on Mon, Wed"
        );
        assert_eq!(
            draft.recurrence_lines().unwrap(),
            vec!["RRULE:FREQ=WEEKLY;INTERVAL=1;BYDAY=MO,WE".to_owned()]
        );
    }

    #[test]
    fn a_new_event_defaults_to_not_repeating_and_can_turn_repeating_on() {
        let start = at_clock(NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(), 9, 0);
        let mut draft = Draft::new("work".into(), start, start + chrono::Duration::minutes(30));
        assert!(draft.recurrence_lines().unwrap().is_empty());
        draft.rule.apply_action(
            crate::recurrence::Action::Repeat(crate::recurrence::RepeatMode::Daily),
            start.date_naive(),
        );
        assert_eq!(
            draft.recurrence_lines().unwrap(),
            vec!["RRULE:FREQ=DAILY;INTERVAL=1".to_owned()]
        );
    }

    /// Every copy of an event has to agree on its location: trips are worked
    /// out from the days, the details pane reads the open copy, and the disk is
    /// what the next run paints first.
    #[test]
    fn a_location_reaches_every_copy_of_an_event() {
        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("timeslay.sqlite3");
        let snapshot = sample_snapshot();
        let day = snapshot.day;
        let other = day.succ_opt().unwrap();

        let mut store = Store::with_snapshot(snapshot.clone());
        store.cache_days_in(Some(directory.clone()));
        store.apply_snapshot(snapshot);
        let copy = store.event("standup").unwrap().clone();
        store.cache.insert(other, vec![copy.clone()]);
        store.detail_event = Some(copy);

        store.set_location("standup", "Campus");

        let location = |events: &[Event]| {
            events
                .iter()
                .find(|event| event.id == "standup")
                .map(|event| event.details.location.clone())
        };
        let campus = Some("Campus".to_owned());
        assert_eq!(location(&store.events), campus, "the day in view");
        assert_eq!(location(&store.cache[&other]), campus, "a parked day");
        assert_eq!(
            store.detail_event.as_ref().unwrap().details.location,
            "Campus",
            "the open copy"
        );
        assert_eq!(
            location(&store.read_day(day).unwrap()),
            campus,
            "the day on disk"
        );

        let _ = std::fs::remove_dir_all(directory);
    }
}
