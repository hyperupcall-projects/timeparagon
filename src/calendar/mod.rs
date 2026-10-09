//! Google Calendar: calendars, today's events, and the bits of OAuth the UI
//! should not have to know about.

pub mod api;
pub mod bridge;
pub mod colors;
mod day_cache;
pub mod layout;
pub mod oauth;

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Datelike, Local, Months, NaiveDate, Timelike};
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
    /// A finished Google Task. Calendar events are never completed.
    pub completed: bool,
    pub details: Details,
}

impl Event {
    /// Whether this came from Google Tasks rather than Google Calendar. Task
    /// events carry a `task:` id and a `tasks:` calendar id.
    pub fn is_task(&self) -> bool {
        self.id.starts_with("task:") || self.calendar_id.starts_with("tasks:")
    }
}

/// The parts of a Google event that are worth reading, beyond the chip.
///
/// It carries serde derives so a day can be written out to the cache whole.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Details {
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

/// A task list's color: the user's choice if there is one, otherwise Google's
/// blue.
fn task_color(settings: &CalendarSettings, calendar_id: &str) -> Color {
    settings.task_color(calendar_id).unwrap_or(colors::TASK)
}

/// Gives a Google Tasks event its list's color, leaving calendar events alone.
fn recolor_task(event: &mut Event, settings: &CalendarSettings) {
    if event.calendar_id.starts_with("tasks:") {
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
    /// The first day of the month the picker is showing.
    pub month: NaiveDate,
    /// Whether the day in view should follow the real today. Cleared once the
    /// user navigates to another day, so a boundary rollover does not yank them
    /// back to today.
    pub follow_today: bool,
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
            day,
            last_sync: None,
            error: None,
            hour_height: DEFAULT_HOUR_HEIGHT,
            open_event: None,
            details_below: false,
            color_editor: None,
            month_open: false,
            month: first_of_month(day),
            follow_today: true,
        }
    }

    /// A store with events already in hand, for tests and screenshots.
    pub fn with_snapshot(snapshot: Snapshot) -> Self {
        Self {
            connection: Connection::SignedIn,
            sync: SyncState::Idle,
            menu_open: false,
            calendars: snapshot.calendars,
            events: snapshot.events,
            cache: HashMap::new(),
            day_cache: None,
            day: snapshot.day,
            last_sync: Some(snapshot.fetched_at),
            error: None,
            hour_height: DEFAULT_HOUR_HEIGHT,
            open_event: None,
            details_below: false,
            color_editor: None,
            month_open: false,
            month: first_of_month(snapshot.day),
            // A snapshot is pinned to the day it was fetched for, so it does
            // not follow the clock until the user asks it to.
            follow_today: false,
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
    pub fn forget_cached_days(&mut self) {
        self.cache.clear();
        self.events.clear();
        self.open_event = None;

        // The days on disk were filed under the old boundary too.
        if let Some(directory) = self.day_cache.as_ref() {
            day_cache::clear(directory);
        }
    }

    /// Keeps the caches from growing without bound, dropping the days furthest
    /// from the one in view first. The day in view, the day before it and the
    /// day after it are kept, along with [`EXTRA_CACHED_DAYS`] further days.
    ///
    /// The days on disk are trimmed by the same rule, so both caches end up
    /// holding the same days.
    fn trim_cache(&mut self) {
        let current = self.day;

        for day in days_to_drop(self.cache.keys().copied(), current) {
            self.cache.remove(&day);
        }

        let Some(directory) = self.day_cache.as_ref() else {
            return;
        };
        for day in days_to_drop(day_cache::days(directory), current) {
            day_cache::remove(directory, day);
        }
    }

    /// The day's events as an earlier run left them, if it left them.
    fn read_day(&self, day: NaiveDate) -> Option<Vec<Event>> {
        day_cache::load(self.day_cache.as_ref()?, day)
    }

    /// Writes one day out, so the run after this one paints it at once.
    fn write_day(&self, day: NaiveDate, events: &[Event]) {
        let Some(directory) = self.day_cache.as_ref() else {
            return;
        };

        if let Err(error) = day_cache::save(directory, day, events) {
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

    pub fn calendar_enabled(&self, id: &str, settings: &CalendarSettings) -> bool {
        !settings.disabled.iter().any(|hidden| hidden == id)
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

        // The day in view has fresh events now, so the run after this one
        // opens on them rather than on an empty grid.
        self.write_day(self.day, &self.events);
    }

    pub fn visible_events<'a>(
        &'a self,
        settings: &'a CalendarSettings,
    ) -> impl Iterator<Item = &'a Event> {
        self.events
            .iter()
            .filter(move |event| settings.show_tasks || !event.is_task())
            .filter(|event| self.calendar_enabled(&event.calendar_id, settings))
    }

    /// The event with this id, if it is still loaded.
    pub fn event(&self, id: &str) -> Option<&Event> {
        self.events.iter().find(|event| event.id == id)
    }

    /// A mutable handle to the event with this id.
    pub fn event_mut(&mut self, id: &str) -> Option<&mut Event> {
        self.events.iter_mut().find(|event| event.id == id)
    }

    pub fn all_day_events<'a>(
        &'a self,
        settings: &'a CalendarSettings,
    ) -> impl Iterator<Item = &'a Event> {
        self.visible_events(settings).filter(|event| event.all_day)
    }

    pub fn timed_events<'a>(
        &'a self,
        settings: &'a CalendarSettings,
    ) -> impl Iterator<Item = &'a Event> {
        self.visible_events(settings).filter(|event| !event.all_day)
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
) -> Result<Snapshot, String> {
    let tokens = oauth::load_tokens().ok_or_else(|| "Not connected to Google Calendar".to_owned())?;
    let tokens = oauth::ensure_access(client_id, client_secret, tokens)?;
    let mut snapshot = api::fetch_day(&tokens.access_token, disabled, day, day_end_hour)?;
    // Tasks come back as all-day chips because the API has no time for them.
    // If the browser helper is installed and signed in, the scheduled ones get
    // their real time here.
    bridge::promote_task_times(&mut snapshot);
    Ok(snapshot)
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
    let tokens = oauth::load_tokens().ok_or_else(|| "Not connected to Google Calendar".to_owned())?;
    let tokens = oauth::ensure_access(client_id, client_secret, tokens)?;

    let list_id = calendar_id.strip_prefix("tasks:").unwrap_or(calendar_id);
    let task_id = event_id.strip_prefix("task:").unwrap_or(event_id);

    api::set_task_completed(&tokens.access_token, list_id, task_id, completed)
        .map_err(describe_task_write_error)
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
    fn disabling_tasks_hides_them_from_the_day_view() {
        let mut snapshot = sample_snapshot();
        let day = snapshot.day;
        snapshot.events.push(Event {
            id: "task:abc".into(),
            calendar_id: "tasks:list".into(),
            title: "File taxes".into(),
            start: day.and_hms_opt(0, 0, 0).unwrap().and_local_timezone(Local).unwrap(),
            end: day.and_hms_opt(0, 0, 0).unwrap().and_local_timezone(Local).unwrap(),
            all_day: true,
            color: colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        });

        let store = Store::with_snapshot(snapshot);
        let mut settings = CalendarSettings::default();
        assert_eq!(store.visible_events(&settings).filter(|e| e.is_task()).count(), 1);

        settings.show_tasks = false;
        assert_eq!(store.visible_events(&settings).filter(|e| e.is_task()).count(), 0);
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

        let list = store.calendars.iter().find(|c| c.id == "tasks:list").unwrap();
        assert_eq!(list.color, red);
        for event in store.events.iter().filter(|e| e.calendar_id == "tasks:list") {
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

        let list = store.calendars.iter().find(|c| c.id == "tasks:list").unwrap();
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
        assert!(describe_task_write_error("Request had insufficient authentication scopes.".to_owned())
            .contains("Reconnect"));
        assert_eq!(describe_task_write_error("Network down".to_owned()), "Network down");
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
        assert_eq!(grid.last().unwrap(), &NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());

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
        assert!(store.events.is_empty(), "the old day's events must not linger");
        assert!(!store.follow_today, "a moved day no longer follows today");

        store.set_day(today, today);
        assert!(store.follow_today, "back on today, the view follows it again");
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
        assert_eq!(store.events.len(), parked, "the cached day should come back");
    }

    #[test]
    fn forgetting_the_cache_drops_the_days_it_held() {
        let mut store = Store::with_snapshot(sample_snapshot());
        let today = store.day;

        store.set_day(today.succ_opt().unwrap(), today);
        store.forget_cached_days();
        store.set_day(today, today);

        assert!(store.events.is_empty(), "a forgotten day has to be fetched again");
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

        assert_eq!(kept.len(), 13, "the day in view, its neighbours and ten more");
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

    /// The days on disk are trimmed by the same rule as the memory cache.
    #[test]
    fn days_far_from_the_one_in_view_leave_the_cache_directory() {
        let scratch = day_cache::Scratch::new("pruning");
        let directory = scratch.path().to_path_buf();
        let today = NaiveDate::from_ymd_opt(2026, 10, 8).unwrap();

        for offset in -20..=20 {
            let day = today + chrono::Duration::days(i64::from(offset));
            day_cache::save(&directory, day, &[]).unwrap();
        }

        let mut store = Store::with_snapshot(empty_snapshot(today));
        store.cache_days_in(Some(directory.clone()));

        let left = day_cache::days(&directory);
        assert_eq!(left.len(), 13, "the day in view, its neighbours and ten more");
        for offset in [-6, -1, 0, 1, 6] {
            let day = today + chrono::Duration::days(i64::from(offset));
            assert!(left.contains(&day), "day {offset} should stay on disk");
        }
        for offset in [-20, -7, 7, 20] {
            let day = today + chrono::Duration::days(i64::from(offset));
            assert!(!left.contains(&day), "day {offset} should leave the disk");
        }
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
}
