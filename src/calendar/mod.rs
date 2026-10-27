//! Google Calendar: calendars, today's events, and the bits of OAuth the UI
//! should not have to know about.

pub mod api;
pub mod colors;
mod day_cache;
pub mod deadline;
pub mod layout;
pub mod oauth;
pub mod scheduled;

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Datelike, Local, Months, NaiveDate, TimeZone, Timelike};
use iced::Color;
use serde::{Deserialize, Serialize};

use crate::config::CalendarSettings;

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
        self.id.starts_with("tasks:")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: String,
    pub calendar_id: String,
    pub title: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub all_day: bool,
    pub color: Color,
    /// A finished Google Task, including its retained calendar representation.
    pub completed: bool,
    pub details: Details,
}

impl Event {
    /// Source used for calendar visibility and overlap ordering.
    pub fn source_id(&self) -> &str {
        if self.is_task() {
            CalendarSettings::GOOGLE_TASKS_ID
        } else {
            &self.calendar_id
        }
    }

    /// Whether this came from Google Tasks rather than Google Calendar. Task
    /// events carry a `task:` id and a `tasks:` calendar id.
    pub fn is_task(&self) -> bool {
        self.id.starts_with("task:") || self.calendar_id.starts_with("tasks:")
    }
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

/// A new event being filled in before it is written to Google.
///
/// It is created by a click or a drag on empty grid space and lives in the
/// store until it is saved or cancelled. The times are kept both as real
/// instants and as the text the user is editing, so a half-typed `9:3` does not
/// throw away the last valid time.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub title: String,
    pub description: String,
    /// The calendar the event will be created on.
    pub calendar_id: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    /// The `HH:MM` text shown in the start field.
    pub start_text: String,
    /// The `HH:MM` text shown in the end field.
    pub end_text: String,
    /// Whether the calendar dropdown is open.
    pub calendar_dropdown_open: bool,
}

impl Draft {
    /// A draft on `calendar_id` spanning `start` to `end`.
    pub fn new(calendar_id: String, start: DateTime<Local>, end: DateTime<Local>) -> Self {
        Self {
            title: String::new(),
            description: String::new(),
            calendar_id,
            start,
            end,
            start_text: clock_label(start),
            end_text: clock_label(end),
            calendar_dropdown_open: false,
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

/// An existing event being edited before the changes are written to Google.
///
/// It carries the event's id so the update knows which event to patch, and
/// starts from the event's current values so the user only changes what they
/// want to.
#[derive(Debug, Clone, PartialEq)]
pub struct EditDraft {
    pub id: String,
    pub title: String,
    pub description: String,
    /// The calendar the event is on.
    pub calendar_id: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    /// The `HH:MM` text shown in the start field.
    pub start_text: String,
    /// The `HH:MM` text shown in the end field.
    pub end_text: String,
}

impl EditDraft {
    /// Build an edit draft from an existing event.
    pub fn from_event(event: &Event) -> Self {
        Self {
            id: event.id.clone(),
            title: event.title.clone(),
            description: event.details.description.clone(),
            calendar_id: event.calendar_id.clone(),
            start: event.start,
            end: event.end,
            start_text: clock_label(event.start),
            end_text: clock_label(event.end),
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

/// A local instant at a clock time on a given date, resolving the daylight
/// saving gaps the same way the rest of the calendar does.
pub(crate) fn at_clock(day: NaiveDate, hour: u32, minute: u32) -> DateTime<Local> {
    let naive = day.and_hms_opt(hour, minute, 0).unwrap_or_default();
    match Local.from_local_datetime(&naive) {
        chrono::offset::LocalResult::Single(when)
        | chrono::offset::LocalResult::Ambiguous(when, _) => when,
        chrono::offset::LocalResult::None => naive.and_utc().with_timezone(&Local),
    }
}

/// The parts of a Google event that are worth reading, beyond the chip.
///
/// It carries serde derives so a day can be written out to the cache whole.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Details {
    pub scheduled: Option<scheduled::Link>,
    pub task_data: Option<crate::task_db::TaskData>,
    pub location: String,
    pub description: String,
    pub status: String,
    pub link: String,
    pub meeting: String,
    pub organizer: String,
    pub attendees: Vec<String>,
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

/// The months that should leave the on-disk cache while `current` is in view.
///
/// The current month, the month before it and the month after it are never
/// dropped, and the [`EXTRA_CACHED_DAYS`] closest months beyond that
/// window survive too. Everything further out goes, furthest first.
fn months_to_drop(
    months: impl IntoIterator<Item = NaiveDate>,
    current: NaiveDate,
) -> Vec<NaiveDate> {
    let current_month = first_of_month(current);
    let current_index = current_month.year() as i32 * 12 + current_month.month() as i32;

    let mut far: Vec<NaiveDate> = months
        .into_iter()
        .filter(|month| {
            let m = first_of_month(*month);
            let index = m.year() as i32 * 12 + m.month() as i32;
            i64::from((index - current_index).abs()) > ALWAYS_KEPT_DISTANCE
        })
        .collect();

    far.sort_by_key(|month| {
        let m = first_of_month(*month);
        let index = m.year() as i32 * 12 + m.month() as i32;
        (index - current_index).abs()
    });
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

/// Gives a Google Tasks event its list's color, leaving calendar events alone.
fn recolor_task(event: &mut Event, settings: &CalendarSettings) {
    if event.calendar_id.starts_with("tasks:") && event.details.scheduled.is_none() {
        event.color = task_color(settings, &event.calendar_id);
    }
}

/// Everything the calendar view and its settings pane need.
pub struct Store {
    pub connection: Connection,
    pub sync: SyncState,
    pub menu_open: bool,
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
    /// Events for the days already visited, so stepping back to one paints it
    /// at once while the fresh sync runs. The day in view lives in `events`.
    cache: HashMap<NaiveDate, Vec<Event>>,
    /// The directory those days are also written to, so they survive a
    /// restart. `None` until [`Store::cache_days_in`] is called, which only
    /// `boot` and a reconnect do: a store built by hand must not read or
    /// rewrite the cache the user's own days live in.
    day_cache: Option<PathBuf>,
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
    /// The details card sits at the bottom when the click was in the top half.
    pub details_below: bool,
    /// The task list whose color picker is open, by calendar id.
    pub color_editor: Option<String>,
    /// Whether the month picker is showing over the day view.
    pub month_open: bool,
    /// The day whose pending Google Tasks are listed in a popup.
    pub pending_open: Option<NaiveDate>,
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
}

impl Store {
    pub fn new(day_end_hour: u32) -> Self {
        let connection = if oauth::load_tokens().is_some() {
            Connection::SignedIn
        } else {
            Connection::SignedOut
        };
        let day = day_at(Local::now(), day_end_hour);

        Self {
            connection,
            sync: SyncState::Idle,
            menu_open: false,
            calendars: Vec::new(),
            events: Vec::new(),
            cache: HashMap::new(),
            day_cache: None,
            day_end_hour,
            day,
            last_sync: None,
            error: None,
            hour_height: DEFAULT_HOUR_HEIGHT,
            open_event: None,
            details_below: false,
            color_editor: None,
            month_open: false,
            pending_open: None,
            month: first_of_month(day),
            follow_today: true,
            pending: HashMap::new(),
            draft: None,
            edit_draft: None,
            pending_delete: None,
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
            calendars: snapshot.calendars,
            events: snapshot.events,
            cache: HashMap::new(),
            day_cache: None,
            day_end_hour: 0,
            day: snapshot.day,
            last_sync: Some(snapshot.fetched_at),
            error: None,
            hour_height: DEFAULT_HOUR_HEIGHT,
            open_event: None,
            details_below: false,
            color_editor: None,
            month_open: false,
            pending_open: None,
            month: first_of_month(snapshot.day),
            // A snapshot is pinned to the day it was fetched for, so it does
            // not follow the clock until the user asks it to.
            follow_today: false,
            pending: HashMap::new(),
            draft: None,
            edit_draft: None,
            pending_delete: None,
        }
    }

    /// Points the store at the directory cached days are written to, so they
    /// outlive the process.
    ///
    /// The day in view is read straight back when a previous run left it
    /// there, which is what gives the window something to show before the
    /// first sync lands, and days that fell out of the cache while the app
    /// was closed are dropped. Called at boot and when a reconnect replaces
    /// the store: a store built by hand keeps its days in memory only.
    pub fn cache_days_in(&mut self, directory: Option<PathBuf>) {
        self.day_cache = directory;

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
    /// yet comes back from the cache directory if an earlier run left it there,
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
        if let Some(directory) = self.day_cache.as_ref() {
            day_cache::clear(directory);
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
        if let Some(directory) = self.day_cache.as_ref() {
            day_cache::clear(directory);
        }
    }

    /// Keeps the caches from growing without bound, dropping the days furthest
    /// from the one in view first. The day in view, the day before it and the
    /// day after it are kept, along with [`EXTRA_CACHED_DAYS`] further days.
    ///
    /// The days in memory are trimmed day by day. The months on disk are
    /// trimmed month by month: the current month, its neighbours, and
    /// [`EXTRA_CACHED_DAYS`] further months survive; everything further out
    /// goes.
    fn trim_cache(&mut self) {
        let current = self.day;

        for day in days_to_drop(self.cache.keys().copied(), current) {
            self.cache.remove(&day);
        }

        let Some(directory) = self.day_cache.as_ref() else {
            return;
        };
        for month in months_to_drop(day_cache::days(directory), current) {
            day_cache::remove(directory, month);
        }
    }

    /// The day's events as an earlier run left them, if it left them.
    fn read_day(&self, day: NaiveDate) -> Option<Vec<Event>> {
        day_cache::load(self.day_cache.as_ref()?, day, self.day_end_hour)
    }

    /// Writes one day out, so the run after this one paints it at once.
    fn write_day(&self, day: NaiveDate, events: &[Event]) {
        let Some(directory) = self.day_cache.as_ref() else {
            return;
        };

        if let Err(error) = day_cache::save(directory, day, events, self.day_end_hour) {
            log::warn!("could not cache {}: {error}", day);
        }
    }

    /// Grows or shrinks the hour rows. `factor` above 1 spreads the day out.
    pub fn zoom(&mut self, factor: f32) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }

        self.hour_height = (self.hour_height * factor).clamp(MIN_HOUR_HEIGHT, MAX_HOUR_HEIGHT);
    }

    pub fn is_signed_in(&self) -> bool {
        self.connection == Connection::SignedIn
    }

    pub fn is_busy(&self) -> bool {
        matches!(self.sync, SyncState::Connecting | SyncState::Syncing)
    }

    /// All real calendars plus one shared Google Tasks entry, including hidden sources.
    pub fn display_sources(&self, settings: &CalendarSettings) -> Vec<Calendar> {
        let mut sources: Vec<_> = self
            .calendars
            .iter()
            .filter(|calendar| !calendar.is_task_list())
            .cloned()
            .collect();
        sources.push(Calendar {
            id: CalendarSettings::GOOGLE_TASKS_ID.into(),
            name: "Google Tasks".into(),
            color: task_color(settings, CalendarSettings::GOOGLE_TASKS_ID),
            primary: false,
            selected_by_google: true,
        });
        // Stable sorting appends newly discovered sources in Google's order.
        sources.sort_by_key(|source| {
            settings
                .source_order
                .iter()
                .position(|id| id == &source.id)
                .unwrap_or(usize::MAX)
        });
        sources
    }

    pub fn calendar_enabled(&self, id: &str, settings: &CalendarSettings) -> bool {
        if id.starts_with("tasks:") {
            settings.show_tasks
        } else {
            !settings.disabled.iter().any(|hidden| hidden == id)
        }
    }

    /// Gives every Google Tasks list its saved color, or Google's blue when it
    /// has none. Google hands all lists back with the same blue, so this is
    /// what keeps a customized list its own color across syncs, and what puts a
    /// list back to blue after its custom color is cleared. Real calendars are
    /// left exactly as Google returned them.
    pub fn apply_task_colors(&mut self, settings: &CalendarSettings) {
        for calendar in &mut self.calendars {
            if calendar.is_task_list() {
                calendar.color = task_color(settings, &calendar.id);
            }
        }

        for event in &mut self.events {
            recolor_task(event, settings);
        }

        // The cached days are recolored too, so a day painted from the cache
        // does not flash the old color before the sync lands.
        for events in self.cache.values_mut() {
            for event in events {
                recolor_task(event, settings);
            }
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: Snapshot) {
        self.calendars = snapshot.calendars;
        self.events = snapshot.events;
        self.day = snapshot.day;
        self.last_sync = Some(snapshot.fetched_at);
        self.connection = Connection::SignedIn;
        self.sync = SyncState::Idle;
        self.error = snapshot.notice;
        if self
            .open_event
            .as_ref()
            .is_some_and(|id| self.events.iter().all(|event| &event.id != id))
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
        let current = self.day;
        for event in &mut snapshot.events {
            if let Some(pending) = self.pending.get(&event.id) {
                event.start = pending.start;
                event.end = pending.end;
            }
        }
        let mut current_snapshot = snapshot.clone();
        for &day in days {
            let start = at_clock(day, self.day_end_hour, 0);
            let end = at_clock(day.succ_opt().unwrap_or(day), self.day_end_hour, 0);
            let events: Vec<_> = snapshot
                .events
                .iter()
                .filter(|event| {
                    event.is_task()
                        || if event.all_day {
                            event.start.date_naive() <= day && event.end.date_naive() > day
                        } else {
                            event.start < end && event.end > start
                        }
                })
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

    pub fn events_on(&self, day: NaiveDate) -> &[Event] {
        if day == self.day {
            &self.events
        } else {
            self.cache.get(&day).map(Vec::as_slice).unwrap_or(&[])
        }
    }

    pub fn replace_cached_task(&mut self, previous_id: &str, task: &Event) {
        let key = crate::timeblocking::task_key(task);
        for events in self.cache.values_mut() {
            events.retain(|event| {
                event.id != previous_id
                    && (!event.is_task() || crate::timeblocking::task_key(event) != key)
            });
            events.push(task.clone());
        }
        self.clear_disk_cache();
    }

    pub fn clear_disk_cache(&self) {
        if let Some(directory) = self.day_cache.as_ref() {
            day_cache::clear(directory);
        }
    }

    pub fn forget_event_everywhere(&mut self, id: &str) {
        self.events.retain(|event| event.id != id);
        for events in self.cache.values_mut() {
            events.retain(|event| event.id != id);
        }
        self.clear_disk_cache();
    }

    pub fn load_cached_days(&mut self, days: &[NaiveDate]) {
        for &day in days {
            if day != self.day && !self.cache.contains_key(&day) {
                if let Some(events) = self.read_day(day) {
                    self.cache.insert(day, events);
                }
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
            .filter(|event| self.calendar_enabled(event.source_id(), settings))
            .filter(|event| {
                event
                    .details
                    .scheduled
                    .as_ref()
                    .is_none_or(|link| self.calendar_enabled(&link.calendar_id, settings))
            })
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
            .filter(|event| self.calendar_enabled(event.source_id(), settings))
            .filter(|event| {
                event
                    .details
                    .scheduled
                    .as_ref()
                    .is_none_or(|link| self.calendar_enabled(&link.calendar_id, settings))
            })
    }

    /// The event with this id, if it is still loaded.
    pub fn event(&self, id: &str) -> Option<&Event> {
        self.events.iter().find(|event| event.id == id)
    }

    /// A mutable handle to the event with this id.
    pub fn event_mut(&mut self, id: &str) -> Option<&mut Event> {
        self.events.iter_mut().find(|event| event.id == id)
    }

    /// Sets the location text of an event wherever it is held, so the details
    /// pane shows a place at once rather than after the next sync.
    pub fn set_location(&mut self, id: &str, location: &str) {
        if let Some(event) = self.event_mut_anywhere(id) {
            event.details.location = location.to_owned();
        }
    }

    /// A mutable handle to an event, whether it is on the day in view or parked
    /// in the cache. A write can land while the user is looking at another day.
    fn event_mut_anywhere(&mut self, id: &str) -> Option<&mut Event> {
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

    /// The calendar a new event should be created on.
    ///
    /// The configured default wins when it still exists; otherwise the primary
    /// calendar, and failing that the first writable one. Task lists are never
    /// offered, since they hold Google Tasks rather than events. The calendar
    /// that stores assigned task times is managed by the app, so a new event
    /// is never filed there either.
    pub fn default_calendar_id(&self, settings: &CalendarSettings) -> Option<String> {
        if let Some(id) = settings
            .time_block_calendar
            .as_ref()
            .or(settings.default_calendar.as_ref())
            && !is_managed_tasks_calendar(settings, id)
            && self
                .calendars
                .iter()
                .any(|calendar| &calendar.id == id && !calendar.is_task_list())
        {
            return Some(id.clone());
        }

        self.calendars
            .iter()
            .find(|calendar| {
                calendar.primary
                    && !calendar.is_task_list()
                    && !is_managed_tasks_calendar(settings, &calendar.id)
            })
            .or_else(|| {
                self.calendars.iter().find(|calendar| {
                    !calendar.is_task_list() && !is_managed_tasks_calendar(settings, &calendar.id)
                })
            })
            .map(|calendar| calendar.id.clone())
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
        self.month_open = false;
    }

    /// Drops a deleted task, every cached copy of it, and the calendar blocks
    /// that were standing in for it.
    pub fn forget_task(&mut self, id: &str) {
        let task_id = id.strip_prefix("task:").unwrap_or(id).to_owned();
        let drop_copies = |events: &mut Vec<Event>| {
            events.retain(|event| {
                event.id != id
                    && event
                        .details
                        .scheduled
                        .as_ref()
                        .map(|link| link.task_id.as_str())
                        != Some(task_id.as_str())
            });
        };
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
                .filter(|event| self.calendar_enabled(event.source_id(), settings))
                .filter(|event| {
                    event
                        .details
                        .scheduled
                        .as_ref()
                        .is_none_or(|link| self.calendar_enabled(&link.calendar_id, settings))
                }),
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

    pub fn status_line(&self) -> String {
        match (self.sync, self.error.as_deref(), self.last_sync) {
            (SyncState::Connecting, _, _) => "Signing in…".to_owned(),
            (SyncState::Syncing, _, _) => "Syncing…".to_owned(),
            (_, Some(error), _) => error.to_owned(),
            (_, _, Some(when)) => format!("Synced {}", clock_label(when)),
            _ if self.is_signed_in() => "Not synced yet".to_owned(),
            _ => "Not connected".to_owned(),
        }
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

/// The calendar that holds a task's assigned time. The app writes those
/// events itself, so they are not a calendar the user files new events on.
fn is_managed_tasks_calendar(settings: &CalendarSettings, id: &str) -> bool {
    settings.tasks_calendar.as_deref() == Some(id)
}

/// Whether this event is the managed tasks calendar showing up as a normal chip.
///
/// A task that already sits beside its time block is drawn there, so the grid
/// does not draw it again. A plain event stored on that calendar is managed by
/// the app and stays off the day view. A task given a time of its own, with no
/// time block, still occupies the grid: that chip is the only place its time
/// is visible.
fn omit_managed_tasks_event(
    event: &Event,
    time_blocks: &[String],
    settings: &CalendarSettings,
) -> bool {
    let Some(managed) = settings.tasks_calendar.as_deref() else {
        return false;
    };
    if event.calendar_id == managed {
        return true;
    }
    event
        .details
        .scheduled
        .as_ref()
        .is_some_and(|link| link.calendar_id == managed)
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
    client_id: &str,
    client_secret: &str,
    disabled: &[String],
    day: NaiveDate,
    day_end_hour: u32,
    tasks_calendar: Option<&str>,
) -> Result<Snapshot, String> {
    let tokens =
        oauth::load_tokens().ok_or_else(|| "Not connected to Google Calendar".to_owned())?;
    let tokens = oauth::ensure_access(client_id, client_secret, tokens)?;
    api::fetch_day(
        &tokens.access_token,
        disabled,
        day,
        day_end_hour,
        tasks_calendar,
    )
}

pub fn sync_week(
    client_id: &str,
    client_secret: &str,
    disabled: &[String],
    anchor: NaiveDate,
    days: u32,
    day_end_hour: u32,
    tasks_calendar: Option<&str>,
) -> Result<Snapshot, String> {
    let tokens =
        oauth::load_tokens().ok_or_else(|| "Not connected to Google Calendar".to_owned())?;
    let tokens = oauth::ensure_access(client_id, client_secret, tokens)?;
    let dates = week_dates(anchor, days);
    api::fetch_range(
        &tokens.access_token,
        disabled,
        anchor,
        dates[0],
        *dates.last().unwrap(),
        day_end_hour,
        tasks_calendar,
    )
}

/// The selected day and the following days, with one preceding day for context.
pub fn week_dates(anchor: NaiveDate, days: u32) -> Vec<NaiveDate> {
    (-1..i64::from(days.clamp(7, 8)))
        .filter_map(|offset| anchor.checked_add_signed(chrono::Duration::days(offset)))
        .collect()
}

pub fn disconnect() {
    oauth::clear_tokens();
}

/// Ticks a Google Task off, or puts it back. Blocking: run it on a worker
/// thread through [`crate::blocking::run`].
pub fn set_task_completed(
    client_id: &str,
    client_secret: &str,
    calendar_id: &str,
    event_id: &str,
    completed: bool,
) -> Result<(), String> {
    let tokens =
        oauth::load_tokens().ok_or_else(|| "Not connected to Google Calendar".to_owned())?;
    let tokens = oauth::ensure_access(client_id, client_secret, tokens)?;

    let list_id = calendar_id.strip_prefix("tasks:").unwrap_or(calendar_id);
    let task_id = event_id.strip_prefix("task:").unwrap_or(event_id);

    api::set_task_completed(&tokens.access_token, list_id, task_id, completed)
        .map_err(describe_task_write_error)
}

/// Completion retains the event and updates its visual completion marker.
/// If Calendar refuses the update, restore Tasks so the optimistic UI can
/// truthfully roll back as well.
pub fn set_linked_task_completed(
    client_id: &str,
    client_secret: &str,
    task: &Event,
    completed: bool,
) -> Result<(), String> {
    set_linked_task_status(
        client_id,
        client_secret,
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
    client_id: &str,
    client_secret: &str,
    task: &Event,
    status: crate::timeblocking::TaskStatus,
) -> Result<Event, String> {
    let tokens = oauth::load_tokens().ok_or("Not connected to Google Calendar")?;
    let tokens = oauth::ensure_access(client_id, client_secret, tokens)?;
    let mut database = crate::task_db::open()?;
    save_task_status(&mut database, task, status, |updated| {
        write_task_status(&tokens.access_token, updated)
    })
}

/// Persist the distinction between ignored and completed only after Google
/// accepts it. A failed remote or local write restores the previous state.
fn save_task_status(
    database: &mut crate::task_db::Database,
    task: &Event,
    status: crate::timeblocking::TaskStatus,
    mut write: impl FnMut(&Event) -> Result<(), String>,
) -> Result<Event, String> {
    let mut updated = task.clone();
    updated.details.task_data = Some(database.import(task)?);
    crate::timeblocking::apply_status(&mut updated, status);
    if let Err(error) = write(&updated)
        .and_then(|()| database.save(&updated, updated.details.task_data.as_ref().unwrap()))
    {
        return match write(task) {
            Ok(()) => Err(format!(
                "Task status could not be saved; the task was restored: {error}"
            )),
            Err(rollback) => Err(format!(
                "Task status failed: {error}. Restoring the task also failed: {rollback}. Sync before retrying."
            )),
        };
    }
    Ok(updated)
}

fn write_task_status(token: &str, task: &Event) -> Result<(), String> {
    let list = task
        .calendar_id
        .strip_prefix("tasks:")
        .ok_or("Invalid task list")?;
    let id = task.id.strip_prefix("task:").ok_or("Invalid task")?;
    api::set_task_completed(token, list, id, task.completed).map_err(describe_task_write_error)?;
    if let Some(link) = &task.details.scheduled {
        scheduled::request(
            "PATCH",
            &link.event_url(),
            token,
            Some(&scheduled::content(task, link)),
        )
        .map_err(|error| error.message)?;
    }
    Ok(())
}

/// Deletes a task from Google Tasks, and the calendar blocks that point at it.
///
/// The chip's list can be stale after the task is moved onto the scheduling
/// list. The Tasks API is asked for the task wherever it still lives. Every
/// calendar event that records this task is removed too, including a second
/// block created for the same task.
pub fn delete_task(
    client_id: &str,
    client_secret: &str,
    task: &Event,
    tasks_calendar: Option<&str>,
) -> Result<(), String> {
    let tokens =
        oauth::load_tokens().ok_or_else(|| "Not connected to Google Calendar".to_owned())?;
    let tokens = oauth::ensure_access(client_id, client_secret, tokens)?;

    let chip_list = task
        .calendar_id
        .strip_prefix("tasks:")
        .unwrap_or(task.calendar_id.as_str());
    let chip_task = task.id.strip_prefix("task:").unwrap_or(task.id.as_str());
    // The schedule records the list the task was moved to. A cached chip can
    // still name the list it left.
    let (list_id, task_id) = match &task.details.scheduled {
        Some(link) if !link.list_id.is_empty() && !link.task_id.is_empty() => {
            (link.list_id.as_str(), link.task_id.as_str())
        }
        _ => (chip_list, chip_task),
    };

    api::delete_task(&tokens.access_token, list_id, task_id).map_err(describe_task_write_error)?;

    let mut calendars = Vec::new();
    if let Some(link) = &task.details.scheduled {
        calendars.push(link.calendar_id.as_str());
    }
    if let Some(calendar) = tasks_calendar.filter(|calendar| !calendars.contains(calendar)) {
        calendars.push(calendar);
    }
    for calendar in calendars {
        scheduled::remove_task_copies(&tokens.access_token, calendar, task_id)?;
    }
    Ok(())
}

/// Sets where an event happens on Google. Only the location is sent, so
/// nothing else about the event changes. Blocking: run it on a worker thread
/// through [`crate::blocking::run`].
pub fn set_event_location(
    client_id: &str,
    client_secret: &str,
    calendar_id: &str,
    event_id: &str,
    location: &str,
) -> Result<(), String> {
    let tokens =
        oauth::load_tokens().ok_or_else(|| "Not connected to Google Calendar".to_owned())?;
    let tokens = oauth::ensure_access(client_id, client_secret, tokens)?;

    api::patch_event_location(&tokens.access_token, calendar_id, event_id, location)
        .map_err(describe_event_write_error)
}

/// Turns a read-only-token rejection into something a user can act on.
fn describe_task_write_error(error: String) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("insufficient") || lower.contains("scope") {
        "Reconnect Google in Settings so Tasks can be ticked off".to_owned()
    } else {
        error
    }
}

/// Moves an event's start and end on Google. Blocking: run it on a worker
/// thread through [`crate::blocking::run`].
pub fn set_event_time(
    client_id: &str,
    client_secret: &str,
    calendar_id: &str,
    event_id: &str,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<(), String> {
    let tokens =
        oauth::load_tokens().ok_or_else(|| "Not connected to Google Calendar".to_owned())?;
    let tokens = oauth::ensure_access(client_id, client_secret, tokens)?;

    api::patch_event_time(&tokens.access_token, calendar_id, event_id, start, end)
        .map_err(describe_event_write_error)
}

/// Turns a read-only-token rejection into something a user can act on.
fn describe_event_write_error(error: String) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("insufficient") || lower.contains("scope") {
        "Reconnect Google in Settings so events can be moved".to_owned()
    } else {
        error
    }
}

/// Creates a new event on Google and returns its id. Blocking: run it on a
/// worker thread through [`crate::blocking::run`].
pub fn create_event(
    client_id: &str,
    client_secret: &str,
    calendar_id: &str,
    title: &str,
    description: &str,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<String, String> {
    let tokens =
        oauth::load_tokens().ok_or_else(|| "Not connected to Google Calendar".to_owned())?;
    let tokens = oauth::ensure_access(client_id, client_secret, tokens)?;

    api::insert_event(
        &tokens.access_token,
        calendar_id,
        title,
        description,
        start,
        end,
    )
    .map_err(describe_event_write_error)
}

/// Updates an existing event on Google: title, description, start and end.
/// Blocking: run it on a worker thread through [`crate::blocking::run`].
pub fn update_event(
    client_id: &str,
    client_secret: &str,
    calendar_id: &str,
    event_id: &str,
    title: &str,
    description: &str,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<(), String> {
    let tokens =
        oauth::load_tokens().ok_or_else(|| "Not connected to Google Calendar".to_owned())?;
    let tokens = oauth::ensure_access(client_id, client_secret, tokens)?;

    api::patch_event(
        &tokens.access_token,
        calendar_id,
        event_id,
        title,
        description,
        start,
        end,
    )
    .map_err(describe_event_write_error)
}

/// Deletes a single event on Google. Blocking: run it on a worker thread
/// through [`crate::blocking::run`]. Tasks are Google Tasks, not calendar
/// events, so callers should guard with [`Event::is_task`] before reaching
/// here.
pub fn delete_event(
    client_id: &str,
    client_secret: &str,
    calendar_id: &str,
    event_id: &str,
) -> Result<(), String> {
    let tokens =
        oauth::load_tokens().ok_or_else(|| "Not connected to Google Calendar".to_owned())?;
    let tokens = oauth::ensure_access(client_id, client_secret, tokens)?;

    api::delete_event(&tokens.access_token, calendar_id, event_id)
        .map_err(describe_event_write_error)
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

/// The single-letter weekday labels for the month grid, Monday first.
pub const WEEKDAY_INITIALS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];

/// The days laid out in a month grid, Monday first and padded to whole weeks.
///
/// The leading and trailing days belong to the neighbouring months so every
/// row is full; the caller dims them.
pub fn month_grid(month: NaiveDate) -> Vec<NaiveDate> {
    let first = first_of_month(month);
    let lead = first.weekday().num_days_from_monday() as i64;
    let start = first - chrono::Duration::days(lead);
    let cells = lead + days_in_month(first) as i64;
    let weeks = (cells + 6) / 7;

    (0..weeks * 7)
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
            .unwrap_or_else(|| Local::now())
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
    use super::*;

    #[test]
    fn task_status_round_trips_and_failed_writes_restore_the_previous_status() {
        use crate::timeblocking::{self as tb, TaskStatus};
        let mut database =
            crate::task_db::Database::open(std::path::Path::new(":memory:")).unwrap();
        let mut task = sample_snapshot().events.remove(0);
        task.id = "task:status".into();
        task.calendar_id = "tasks:list".into();
        let mut data = database.import(&task).unwrap();
        let mut link = scheduled::Link::new("calendar".into(), &task).unwrap();
        link.pending = false;
        link.block = Some("work/block".into());
        data.metadata.block = link.block.clone();
        data.metadata.minutes = 45;
        data.metadata.schedule = Some(link.clone());
        database.save(&task, &data).unwrap();
        task.details.task_data = Some(data);
        task.details.scheduled = Some(link.clone());

        for status in [
            TaskStatus::Ignored,
            TaskStatus::Active,
            TaskStatus::Completed,
            TaskStatus::Active,
            TaskStatus::Ignored,
            TaskStatus::Completed,
            TaskStatus::Ignored,
            TaskStatus::Active,
        ] {
            let previous = task.clone();
            let mut calls = Vec::new();
            let failed = save_task_status(&mut database, &task, status, |updated| {
                calls.push(tb::task_status(updated));
                if calls.len() == 1 {
                    Err("Calendar is offline".into())
                } else {
                    Ok(())
                }
            });
            assert!(failed.unwrap_err().contains("the task was restored"));
            assert_eq!(calls, [status, tb::task_status(&previous)]);
            assert_eq!(database.lookup(&task).unwrap(), previous.details.task_data);

            task = save_task_status(&mut database, &task, status, |updated| {
                assert_eq!(updated.completed, status != TaskStatus::Active);
                assert_eq!(tb::task_status(updated), status);
                Ok(())
            })
            .unwrap();
            let mut reloaded = task.clone();
            reloaded.details.task_data = database.lookup(&task).unwrap();
            assert_eq!(tb::task_status(&reloaded), status);
            assert_eq!(tb::metadata(&reloaded).block, link.block);
            assert_eq!(tb::metadata(&reloaded).minutes, 45);
            assert_eq!(reloaded.details.scheduled.as_ref(), Some(&link));
            assert_eq!((task.start, task.end), (previous.start, previous.end));
        }
    }

    #[test]
    fn forgetting_a_task_drops_the_stale_list_copy_and_its_calendar_block() {
        let mut store = Store::with_snapshot(sample_snapshot());
        let day = store.day;
        let at = store.events[0].start;
        let mirror = Event {
            id: "ttblock".into(),
            calendar_id: "tasks-calendar".into(),
            title: "Shopping2".into(),
            start: at,
            end: at,
            all_day: false,
            color: colors::DEFAULT_EVENT,
            completed: false,
            details: Details {
                scheduled: Some(scheduled::Link {
                    calendar_id: "tasks-calendar".into(),
                    event_id: "ttblock".into(),
                    list_id: "time-blocked".into(),
                    task_id: "abc".into(),
                    pending: false,
                    block: None,
                    order: 0,
                }),
                ..Details::default()
            },
        };
        let ghost = Event {
            id: "task:abc".into(),
            calendar_id: "tasks:low-priority".into(),
            title: "Shopping2".into(),
            start: at,
            end: at,
            all_day: true,
            color: colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        };
        let kept = Event {
            id: "task:other".into(),
            calendar_id: "tasks:low-priority".into(),
            title: "Keep".into(),
            start: at,
            end: at,
            all_day: true,
            color: colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        };
        store.events = vec![mirror, ghost.clone(), kept];
        store.cache.insert(day.pred_opt().unwrap(), vec![ghost]);
        store.open_event = Some("ttblock".into());

        store.forget_task("task:abc");

        let titles: Vec<_> = store
            .events
            .iter()
            .map(|event| event.title.as_str())
            .collect();
        assert_eq!(titles, vec!["Keep"]);
        assert!(store.cache.values().all(|events| events.is_empty()));
        assert!(store.open_event.is_none());
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

        let now = Local::now();
        let at = |minutes: i64| now + chrono::Duration::minutes(minutes);

        let events = vec![
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

        let now = Local::now();
        let at = |minutes: i64| now + chrono::Duration::minutes(minutes);
        let events = vec![make("a", at(10)), make("b", at(20)), make("c", at(30))];

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
    fn the_managed_tasks_calendar_is_not_drawn_like_a_normal_calendar() {
        let mut snapshot = sample_snapshot();
        let day = snapshot.day;
        let at = |hour, minute| {
            day.and_hms_opt(hour, minute, 0)
                .unwrap()
                .and_local_timezone(Local)
                .unwrap()
        };
        snapshot.events.push(Event {
            id: "focus".into(),
            calendar_id: "work".into(),
            title: "Focus".into(),
            start: at(10, 0),
            end: at(12, 0),
            all_day: false,
            color: colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        });
        snapshot.events.push(Event {
            id: "tasks-all-day".into(),
            calendar_id: "tasks".into(),
            title: "Managed all day".into(),
            start: at(0, 0),
            end: at(0, 0),
            all_day: true,
            color: colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        });

        let mut assigned = Event {
            id: "task:inside".into(),
            calendar_id: "tasks:list".into(),
            title: "Inside the block".into(),
            start: at(10, 0),
            end: at(10, 30),
            all_day: false,
            color: colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        };
        let mut link = scheduled::Link::new("tasks".into(), &assigned).unwrap();
        link.pending = false;
        link.block = Some("work/focus".into());
        assigned.details.task_data = Some(crate::task_db::TaskData {
            id: 100,
            metadata: crate::timeblocking::Metadata {
                block: Some("work/focus".into()),
                minutes: 30,
                schedule: Some(link.clone()),
                ..Default::default()
            },
            legacy_key: None,
        });
        assigned.details.scheduled = Some(link);
        snapshot.events.push(assigned);

        let mut alone = Event {
            id: "task:alone".into(),
            calendar_id: "tasks:list".into(),
            title: "On my own".into(),
            start: at(15, 0),
            end: at(15, 30),
            all_day: false,
            color: colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        };
        let mut alone_link = scheduled::Link::new("tasks".into(), &alone).unwrap();
        alone_link.pending = false;
        alone.details.task_data = Some(crate::task_db::TaskData {
            id: 101,
            metadata: crate::timeblocking::Metadata {
                minutes: 30,
                schedule: Some(alone_link.clone()),
                ..Default::default()
            },
            legacy_key: None,
        });
        alone.details.scheduled = Some(alone_link);
        snapshot.events.push(alone);

        let store = Store::with_snapshot(snapshot);
        let mut settings = CalendarSettings::default();
        settings.tasks_calendar = Some("tasks".into());
        settings.time_block_calendar = Some("work".into());

        let titles: Vec<_> = store
            .timed_events(&settings)
            .map(|event| event.title.as_str())
            .collect();
        assert!(titles.contains(&"Standup"));
        assert!(titles.contains(&"Focus"));
        assert!(titles.contains(&"On my own"));
        assert!(!titles.contains(&"Inside the block"));
        assert!(!titles.contains(&"Write the Cal tab"));
        assert!(
            !store
                .all_day_events(&settings)
                .any(|event| event.title == "Managed all day")
        );
        assert!(
            store
                .all_day_events(&settings)
                .any(|event| event.title == "Team offsite")
        );

        settings.time_block_calendar = None;
        settings.default_calendar = Some("tasks".into());
        assert_eq!(
            store.default_calendar_id(&settings).as_deref(),
            Some("personal")
        );
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
    fn real_calendars_keep_the_color_google_gave_them() {
        let mut store = Store::with_snapshot(sample_snapshot());
        let before = store.calendars[0].color;

        store.apply_task_colors(&CalendarSettings::default());

        assert_eq!(store.calendars[0].color, before);
    }

    #[test]
    fn a_read_only_token_is_explained_as_a_reconnect() {
        assert!(
            describe_task_write_error("Request had insufficient authentication scopes.".to_owned())
                .contains("Reconnect")
        );
        assert_eq!(
            describe_task_write_error("Network down".to_owned()),
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
    fn the_month_grid_starts_on_a_monday_and_covers_the_whole_month() {
        // October 2026 starts on a Thursday and has 31 days.
        let month = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let grid = month_grid(month);

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
        assert_eq!(month_grid(month).len(), 28);

        // August 2026 starts on a Saturday and has 31 days, so it needs six.
        let month = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        assert_eq!(month_grid(month).len(), 42);
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
            fetched_at: Local::now(),
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

    /// The months on disk are trimmed by the same rule as the memory cache.
    #[test]
    fn months_far_from_the_one_in_view_leave_the_cache_directory() {
        let scratch = day_cache::Scratch::new("pruning");
        let directory = scratch.path().to_path_buf();
        let today = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();

        for offset in -20..=20 {
            let day = today + chrono::Duration::days(i64::from(offset));
            day_cache::save(&directory, day, &[], 0).unwrap();
        }

        let mut store = Store::with_snapshot(empty_snapshot(today));
        store.cache_days_in(Some(directory.clone()));

        let left = day_cache::days(&directory);
        // September and October are the current month and its neighbour;
        // nothing reaches November.
        assert_eq!(left.len(), 2, "the current month and its neighbour");
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
}
