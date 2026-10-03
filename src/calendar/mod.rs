//! Google Calendar: calendars, today's events, and the bits of OAuth the UI
//! should not have to know about.

pub mod api;
pub mod colors;
pub mod layout;
pub mod oauth;

use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike};
use iced::Color;

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
#[derive(Debug, Clone, PartialEq, Default)]
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

/// A task list's color: the user's choice if there is one, otherwise Google's
/// blue.
fn task_color(settings: &CalendarSettings, calendar_id: &str) -> Color {
    settings.task_color(calendar_id).unwrap_or(colors::TASK)
}

/// Everything the Calendar tab and its settings pane need.
pub struct Store {
    pub connection: Connection,
    pub sync: SyncState,
    pub menu_open: bool,
    pub calendars: Vec<Calendar>,
    pub events: Vec<Event>,
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
}

impl Store {
    pub fn new() -> Self {
        let connection = if oauth::load_tokens().is_some() {
            Connection::SignedIn
        } else {
            Connection::SignedOut
        };

        Self {
            connection,
            sync: SyncState::Idle,
            menu_open: false,
            calendars: Vec::new(),
            events: Vec::new(),
            day: Local::now().date_naive(),
            last_sync: None,
            error: None,
            hour_height: DEFAULT_HOUR_HEIGHT,
            open_event: None,
            details_below: false,
            color_editor: None,
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
            day: snapshot.day,
            last_sync: Some(snapshot.fetched_at),
            error: None,
            hour_height: DEFAULT_HOUR_HEIGHT,
            open_event: None,
            details_below: false,
            color_editor: None,
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
            if event.calendar_id.starts_with("tasks:") {
                event.color = task_color(settings, &event.calendar_id);
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
        Self::new()
    }
}

/// Minutes from local midnight, used to place events and the needle.
pub fn minutes_from_midnight(when: DateTime<Local>) -> i32 {
    when.hour() as i32 * 60 + when.minute() as i32
}

pub fn clock_label(when: DateTime<Local>) -> String {
    format!("{:02}:{:02}", when.hour(), when.minute())
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

/// Pulls today's calendars and events, refreshing the access token if needed.
pub fn sync_today(client_id: &str, client_secret: &str, disabled: &[String]) -> Result<Snapshot, String> {
    let tokens = oauth::load_tokens().ok_or_else(|| "Not connected to Google Calendar".to_owned())?;
    let tokens = oauth::ensure_access(client_id, client_secret, tokens)?;
    api::fetch_today(&tokens.access_token, disabled)
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

fn month_short(month: u32) -> &'static str {
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

/// Sample day used by tests and screenshot generation, so the Cal tab can be
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
    fn zoom_stays_inside_the_readable_range() {
        let mut store = Store::new();
        store.zoom(100.0);
        assert_eq!(store.hour_height, MAX_HOUR_HEIGHT);
        store.zoom(0.001);
        assert_eq!(store.hour_height, MIN_HOUR_HEIGHT);
        store.zoom(0.0);
        assert_eq!(store.hour_height, MIN_HOUR_HEIGHT);
    }
}
