//! Reminders for what is coming up on the calendar.
//!
//! Both apps share the reminder settings and the planning here. Each one
//! delivers the result its platform's way: the desktop posts a freedesktop
//! notification and plays the sound itself; Android hands the plan to
//! `AlarmManager` and posts on a notification channel whose sound is the
//! one chosen here.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use chrono::{DateTime, Duration, Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::calendar::event_options::Method;
use crate::calendar::sources::TimeSlaySource;
use crate::calendar::time_format::{self, TimeFormat};
use crate::calendar::{self, Event};
use crate::config::CalendarSettings;
use crate::places::{self, PlaceSettings};
use crate::sounds::Tone;

/// How far ahead a plan looks. Android re-plans at least this often through
/// the widget's refresh job, and the desktop re-plans every poll.
pub const HORIZON_HOURS: i64 = 36;

/// A reminder may still be delivered this long after its time, for a desktop
/// that was asleep or a poll that ran late. Older ones are dropped quietly.
pub const LATE_GRACE_MINUTES: i64 = 5;

/// The kinds of things a reminder can be about, matching the TimeSlay rows of
/// the calendar list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Events,
    Tasks,
    TimeBlocks,
    Habits,
    Sleep,
    Travel,
}

impl Category {
    pub const ALL: [Self; 6] = [
        Self::Events,
        Self::Tasks,
        Self::TimeBlocks,
        Self::Habits,
        Self::Sleep,
        Self::Travel,
    ];

    /// The stable name used in config, notification channel ids and files.
    pub fn id(self) -> &'static str {
        match self {
            Self::Events => "events",
            Self::Tasks => "tasks",
            Self::TimeBlocks => "time_blocks",
            Self::Habits => "habits",
            Self::Sleep => "sleep",
            Self::Travel => "travel",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|category| category.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Events => "Events",
            Self::Tasks => "Tasks",
            Self::TimeBlocks => "Time Blocks",
            Self::Habits => "Habits",
            Self::Sleep => "Sleep",
            Self::Travel => "Travel",
        }
    }

    /// What the lead time is measured from, for the settings label.
    pub fn lead_label(self) -> &'static str {
        match self {
            Self::Travel => "Before leaving",
            Self::Sleep => "Before bedtime",
            _ => "Before it starts",
        }
    }

    /// The color of the matching calendar row.
    pub fn color(self, settings: &CalendarSettings) -> crate::Color {
        match self {
            Self::Events => settings.event_color(),
            Self::Tasks => TimeSlaySource::Tasks.color(settings),
            Self::TimeBlocks => TimeSlaySource::TimeBlocks.color(settings),
            Self::Habits => TimeSlaySource::Habits.color(settings),
            Self::Sleep => TimeSlaySource::Sleep.color(settings),
            Self::Travel => calendar::colors::TRAVEL,
        }
    }

    /// Which kind of reminder an event gets. Anything not made by TimeSlay is
    /// an event.
    pub fn of(event: &Event, app_calendar: &str) -> Self {
        match TimeSlaySource::of(event, app_calendar) {
            Some(TimeSlaySource::Tasks) => Self::Tasks,
            Some(TimeSlaySource::TimeBlocks) => Self::TimeBlocks,
            Some(TimeSlaySource::Habits) => Self::Habits,
            Some(TimeSlaySource::Sleep) => Self::Sleep,
            None => Self::Events,
        }
    }
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// What a reminder sounds like.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Sound {
    /// Whatever the device plays for a notification by default.
    #[default]
    Default,
    /// One of the device's own notification sounds: a file in the sound
    /// theme on the desktop, a ringtone URI on Android.
    Device { name: String, uri: String },
    /// One of the tones TimeSlay generates.
    Tone { tone: Tone },
    /// The notification is posted without any sound.
    Silent,
}

impl Sound {
    pub fn label(&self) -> String {
        match self {
            Self::Default => "Device default".to_owned(),
            Self::Device { name, .. } => name.clone(),
            Self::Tone { tone } => tone.to_string(),
            Self::Silent => "Silent".to_owned(),
        }
    }

    /// A short, filesystem- and id-safe name that changes whenever the sound
    /// does. Android needs a fresh channel for every sound, because a channel's
    /// sound cannot be changed once it exists.
    pub fn key(&self) -> String {
        match self {
            Self::Default => "default".to_owned(),
            Self::Silent => "silent".to_owned(),
            Self::Tone { tone } => format!("tone_{}", tone.name()),
            Self::Device { uri, .. } => format!("device_{:016x}", fnv1a(uri.as_bytes())),
        }
    }

    /// The choices offered everywhere, before the device's own sounds.
    pub fn builtin_choices() -> Vec<Self> {
        let mut choices = vec![Self::Default, Self::Silent];
        choices.extend(Tone::ALL.into_iter().map(|tone| Self::Tone { tone }));
        choices
    }
}

impl std::fmt::Display for Sound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label())
    }
}

/// A stable hash, so a key survives restarts and builds.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The settings for one kind of reminder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CategorySettings {
    pub enabled: bool,
    /// How long before the start, or before leaving for travel, to remind.
    pub minutes_before: u32,
    pub sound: Sound,
}

impl CategorySettings {
    fn with_lead(minutes_before: u32) -> Self {
        Self {
            enabled: true,
            minutes_before,
            sound: Sound::Default,
        }
    }
}

impl Default for CategorySettings {
    fn default() -> Self {
        Self::with_lead(0)
    }
}

/// The reminder settings: `reminders.*` and the `reminders` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReminderSettings {
    /// The master switch. Off until the user turns reminders on, so nobody
    /// is surprised by notifications after an upgrade.
    pub enabled: bool,
    /// Whether an event's own notification reminders from Google Calendar
    /// decide when it is reminded, instead of `events.minutes_before`.
    pub use_calendar_reminders: bool,
    pub events: CategorySettings,
    pub tasks: CategorySettings,
    pub time_blocks: CategorySettings,
    pub habits: CategorySettings,
    pub sleep: CategorySettings,
    pub travel: CategorySettings,
}

impl Default for ReminderSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            use_calendar_reminders: true,
            events: CategorySettings::with_lead(10),
            tasks: CategorySettings::with_lead(0),
            time_blocks: CategorySettings::with_lead(0),
            habits: CategorySettings::with_lead(0),
            sleep: CategorySettings::with_lead(30),
            travel: CategorySettings::with_lead(5),
        }
    }
}

impl ReminderSettings {
    pub fn get(&self, category: Category) -> &CategorySettings {
        match category {
            Category::Events => &self.events,
            Category::Tasks => &self.tasks,
            Category::TimeBlocks => &self.time_blocks,
            Category::Habits => &self.habits,
            Category::Sleep => &self.sleep,
            Category::Travel => &self.travel,
        }
    }

    pub fn get_mut(&mut self, category: Category) -> &mut CategorySettings {
        match category {
            Category::Events => &mut self.events,
            Category::Tasks => &mut self.tasks,
            Category::TimeBlocks => &mut self.time_blocks,
            Category::Habits => &mut self.habits,
            Category::Sleep => &mut self.sleep,
            Category::Travel => &mut self.travel,
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Whether anything at all will be reminded.
    pub fn any_enabled(&self) -> bool {
        self.enabled
            && Category::ALL
                .into_iter()
                .any(|category| self.get(category).enabled)
    }
}

/// The lead times offered in the settings, in minutes.
pub const LEAD_CHOICES: [u32; 12] = [0, 1, 2, 5, 10, 15, 20, 30, 45, 60, 90, 120];

/// "At start", "5 min", "1 h 30 min".
pub fn lead_label(minutes: u32) -> String {
    match minutes {
        0 => "At the time".to_owned(),
        m if m < 60 => format!("{m} min"),
        m if m % 60 == 0 => format!("{} h", m / 60),
        m => format!("{} h {} min", m / 60, m % 60),
    }
}

/// One reminder to deliver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Due {
    /// Unique per event, occurrence and lead time, so a reminder is never
    /// delivered twice and a moved event gets a fresh one.
    pub key: String,
    pub category: Category,
    pub event_id: String,
    pub title: String,
    pub body: String,
    pub at: DateTime<Local>,
}

/// Everything [`plan`] needs to know besides the events.
pub struct Context<'a> {
    pub settings: &'a ReminderSettings,
    pub calendar: &'a CalendarSettings,
    pub places: &'a PlaceSettings,
    /// The id of TimeSlay's own calendar, which tells time blocks and habits
    /// apart from ordinary events.
    pub app_calendar: &'a str,
    pub time_format: TimeFormat,
}

/// The reminders due in `(from, until]`, soonest first.
///
/// `events` should be what the user can see: events from hidden calendars
/// or kinds are not reminded. Duplicates (the same occurrence loaded for two
/// days) are tolerated.
pub fn plan(
    events: &[Event],
    context: &Context<'_>,
    from: DateTime<Local>,
    until: DateTime<Local>,
) -> Vec<Due> {
    let settings = context.settings;
    if !settings.enabled {
        return Vec::new();
    }

    let mut seen = HashSet::new();
    let timed: Vec<&Event> = events
        .iter()
        .filter(|event| reminds(event))
        .filter(|event| seen.insert((event.id.as_str(), event.start)))
        .collect();

    let mut due = Vec::new();
    let mut push = |due_item: Due| {
        if due_item.at > from && due_item.at <= until {
            due.push(due_item);
        }
    };

    for event in &timed {
        let category = Category::of(event, context.app_calendar);
        let category_settings = settings.get(category);
        if !category_settings.enabled || event.start <= from {
            continue;
        }
        for minutes in leads(event, category, settings) {
            push(Due {
                key: format!(
                    "{}:{}:{}:{minutes}",
                    category.id(),
                    event.id,
                    event.start.timestamp()
                ),
                category,
                event_id: event.id.clone(),
                title: event.title.clone(),
                body: event_body(event, category, minutes, context.time_format),
                at: event.start - Duration::minutes(i64::from(minutes)),
            });
        }
    }

    if settings.travel.enabled {
        for trip in trips(&timed, context.places) {
            if trip.arrive <= from {
                continue;
            }
            let minutes = settings.travel.minutes_before;
            let depart = trip.arrive - Duration::seconds(i64::from(trip.seconds));
            push(Due {
                key: format!(
                    "travel:{}:{}:{}:{minutes}",
                    trip.event_id,
                    trip.arrive.timestamp(),
                    trip.seconds
                ),
                category: Category::Travel,
                event_id: trip.event_id.clone(),
                title: format!("Leave for {}", trip.to_title),
                body: travel_body(&trip, minutes, context.time_format),
                at: depart - Duration::minutes(i64::from(minutes)),
            });
        }
    }

    due.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.key.cmp(&b.key)));
    due
}

/// Only timed, unfinished things with a real date get a reminder.
fn reminds(event: &Event) -> bool {
    !event.all_day
        && !event.completed
        && event.start.date_naive() != crate::timeblocking::UNDATED
        && !event.title.trim().is_empty()
}

/// The lead times, in minutes, an event is reminded at.
fn leads(event: &Event, category: Category, settings: &ReminderSettings) -> Vec<u32> {
    if category == Category::Events && settings.use_calendar_reminders {
        let options = &event.details.options;
        let reminders = if options.reminders.use_default {
            &options.default_reminders
        } else {
            &options.reminders.overrides
        };
        let mut minutes: Vec<u32> = reminders
            .iter()
            .filter(|reminder| reminder.method == Method::Notification)
            .map(|reminder| reminder.minutes)
            .collect();
        // An event the calendar set no reminders for, or explicitly none.
        if !options.reminders.use_default || !minutes.is_empty() {
            minutes.sort_unstable();
            minutes.dedup();
            return minutes;
        }
    }
    vec![settings.get(category).minutes_before]
}

fn when(minutes: u32) -> String {
    match minutes {
        0 => "Now".to_owned(),
        m => format!("In {}", lead_label(m)),
    }
}

fn event_body(event: &Event, category: Category, minutes: u32, format: TimeFormat) -> String {
    let span = time_format::event(event, format);
    match category {
        Category::Sleep => format!("{} · bedtime {span}", when(minutes)),
        _ => format!("{} · {span}", when(minutes)),
    }
}

fn travel_body(trip: &Trip, minutes: u32, format: TimeFormat) -> String {
    let leave = if minutes == 0 {
        "Leave now".to_owned()
    } else {
        format!("Leave in {}", lead_label(minutes))
    };
    format!(
        "{leave} · {} · {} from {} · arrive by {}",
        trip.mode,
        places::duration_label(trip.seconds),
        trip.from_title,
        time_format::clock(trip.arrive, format)
    )
}

/// A trip the travel settings have a time for.
struct Trip {
    event_id: String,
    from_title: String,
    to_title: String,
    mode: places::TravelMode,
    arrive: DateTime<Local>,
    seconds: u32,
}

/// The trips between the day's events, found the same way the calendar
/// draws them. Trips without a worked-out duration have no departure time,
/// so they get no reminder.
fn trips(timed: &[&Event], travel: &PlaceSettings) -> Vec<Trip> {
    if travel.places.is_empty() {
        return Vec::new();
    }
    let mut days: BTreeMap<NaiveDate, Vec<&Event>> = BTreeMap::new();
    for event in timed {
        days.entry(event.start.date_naive())
            .or_default()
            .push(event);
    }

    let mut found = Vec::new();
    for events in days.values_mut() {
        events.sort_by_key(|event| (event.start, event.end));
        let stops: Vec<places::Stop> = events
            .iter()
            .map(|event| places::Stop {
                id: event.id.clone(),
                title: event.title.clone(),
                place: places::place_of(&travel.places, &travel.event_places, event),
            })
            .collect();
        for hop in places::hops(&travel.places, travel.home, &stops) {
            let Some(route) = travel.route(hop.leg) else {
                continue;
            };
            let Some(event) = events.iter().find(|event| event.id == hop.before) else {
                continue;
            };
            found.push(Trip {
                event_id: hop.before.clone(),
                from_title: hop.from_title,
                to_title: hop.to_title,
                mode: travel.mode_for(hop.leg.from, hop.leg.to),
                arrive: event.start,
                seconds: route.seconds,
            });
        }
    }
    found
}

/// Reads the calendar the way the apps last cached it and plans from it.
///
/// Both apps write every day they sync into the day cache, so this sees the
/// same events whether the desktop is showing the week or the phone's widget
/// job is running with the app closed.
pub fn plan_from_cache(
    database: &Path,
    settings: &ReminderSettings,
    calendar_settings: &CalendarSettings,
    places: &PlaceSettings,
    time_format: TimeFormat,
    from: DateTime<Local>,
    until: DateTime<Local>,
) -> Vec<Due> {
    if !settings.any_enabled() {
        return Vec::new();
    }
    let day_end_hour = calendar_settings.day_start_hour();
    let calendars = calendar::day_cache::load_sync_state(database)
        .map(|(_, calendars)| calendars)
        .unwrap_or_default();
    let first = calendar::day_at(from, day_end_hour);
    let last = calendar::day_at(until, day_end_hour);

    let mut events = Vec::new();
    let mut day = first;
    while day <= last {
        if let Some(loaded) = calendar::day_cache::load(database, day, day_end_hour) {
            events.extend(
                loaded
                    .into_iter()
                    .filter(|event| calendar::event_enabled(&calendars, event, calendar_settings)),
            );
        }
        let Some(next) = day.succ_opt() else { break };
        day = next;
    }

    let app_calendar = calendar_settings
        .app_calendar
        .clone()
        .or_else(|| {
            calendars
                .iter()
                .find(|calendar| calendar.is_app_calendar(calendar_settings))
                .map(|calendar| calendar.id.clone())
        })
        .unwrap_or_default();

    plan(
        &events,
        &Context {
            settings,
            calendar: calendar_settings,
            places,
            app_calendar: &app_calendar,
            time_format,
        },
        from,
        until,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::Details;
    use crate::calendar::event_options::{Reminder, Reminders, Unit};
    use chrono::TimeZone;

    const APP: &str = "app-calendar";

    fn at(hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2026, 11, 5)
                    .unwrap()
                    .and_hms_opt(hour, minute, 0)
                    .unwrap(),
            )
            .single()
            .unwrap()
    }

    fn event(id: &str, calendar_id: &str, start: DateTime<Local>) -> Event {
        Event {
            id: id.to_owned(),
            calendar_id: calendar_id.to_owned(),
            title: id.to_owned(),
            start,
            end: start + Duration::minutes(30),
            all_day: false,
            color: calendar::colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        }
    }

    fn enabled() -> ReminderSettings {
        ReminderSettings {
            enabled: true,
            ..ReminderSettings::default()
        }
    }

    fn run(events: &[Event], settings: &ReminderSettings, places: &PlaceSettings) -> Vec<Due> {
        plan(
            events,
            &Context {
                settings,
                calendar: &CalendarSettings::default(),
                places,
                app_calendar: APP,
                time_format: TimeFormat::TwentyFourHour,
            },
            at(8, 0),
            at(23, 59),
        )
    }

    #[test]
    fn nothing_is_planned_while_reminders_are_off() {
        let events = [event("standup", "work", at(9, 0))];
        assert!(
            run(
                &events,
                &ReminderSettings::default(),
                &PlaceSettings::default()
            )
            .is_empty()
        );
    }

    #[test]
    fn an_event_is_reminded_its_lead_time_ahead() {
        let events = [event("standup", "work", at(9, 0))];
        let due = run(&events, &enabled(), &PlaceSettings::default());

        assert_eq!(due.len(), 1);
        assert_eq!(due[0].category, Category::Events);
        assert_eq!(due[0].at, at(8, 50));
        assert_eq!(due[0].body, "In 10 min · 09:00 – 09:30");
    }

    #[test]
    fn google_reminders_replace_the_lead_time_for_events() {
        let mut standup = event("standup", "work", at(9, 0));
        standup.details.options.reminders = Reminders {
            use_default: false,
            overrides: vec![
                Reminder {
                    method: Method::Notification,
                    minutes: 30,
                    unit: Unit::Minutes,
                    input: None,
                },
                Reminder {
                    method: Method::Email,
                    minutes: 5,
                    unit: Unit::Minutes,
                    input: None,
                },
                Reminder {
                    method: Method::Notification,
                    minutes: 0,
                    unit: Unit::Minutes,
                    input: None,
                },
            ],
        };
        let due = run(&[standup.clone()], &enabled(), &PlaceSettings::default());
        let times: Vec<_> = due.iter().map(|due| due.at).collect();
        assert_eq!(times, [at(8, 30), at(9, 0)]);

        let mut settings = enabled();
        settings.use_calendar_reminders = false;
        let due = run(&[standup], &settings, &PlaceSettings::default());
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].at, at(8, 50));
    }

    #[test]
    fn categories_follow_what_the_event_is_and_can_be_switched_off() {
        let block = event("deep work", APP, at(10, 0));
        let mut habit = event("stretch", APP, at(11, 0));
        habit.details.habit = true;
        let mut settings = enabled();
        settings.habits.enabled = false;

        let due = run(&[block, habit], &settings, &PlaceSettings::default());

        assert_eq!(due.len(), 1);
        assert_eq!(due[0].category, Category::TimeBlocks);
        assert_eq!(due[0].at, at(10, 0));
        assert_eq!(due[0].body, "Now · 10:00 – 10:30");
    }

    #[test]
    fn past_all_day_finished_and_duplicate_events_are_skipped() {
        let past = event("past", "work", at(7, 0));
        let mut all_day = event("holiday", "work", at(0, 0));
        all_day.all_day = true;
        let mut done = event("done", "work", at(12, 0));
        done.completed = true;
        let lunch = event("lunch", "work", at(12, 0));

        let due = run(
            &[past, all_day, done, lunch.clone(), lunch],
            &enabled(),
            &PlaceSettings::default(),
        );

        assert_eq!(due.len(), 1);
        assert_eq!(due[0].event_id, "lunch");
    }

    #[test]
    fn travel_is_reminded_before_leaving() {
        use crate::places::{Place, SavedRoute, TravelMode};

        let place = |id: u64, name: &str| Place {
            id,
            name: name.to_owned(),
            address: format!("{name} street"),
            place_id: String::new(),
            lat: 0.0,
            lng: 0.0,
            favorite: false,
        };
        let mut places = PlaceSettings {
            home: Some(1),
            places: vec![place(1, "Home"), place(2, "Gym")],
            ..PlaceSettings::default()
        };
        places.routes.push(SavedRoute {
            from: 1,
            to: 2,
            mode: TravelMode::default(),
            seconds: 20 * 60,
            meters: 5000,
        });
        places.event_places.insert("gym".to_owned(), 2);

        let due = run(&[event("gym", "work", at(18, 0))], &enabled(), &places);
        let travel: Vec<_> = due
            .iter()
            .filter(|due| due.category == Category::Travel)
            .collect();

        assert_eq!(travel.len(), 1);
        assert_eq!(travel[0].title, "Leave for gym");
        // 20 minutes on the road and five more to get going.
        assert_eq!(travel[0].at, at(17, 35));
    }

    #[test]
    fn sound_keys_change_with_the_sound_and_are_safe_ids() {
        let a = Sound::Device {
            name: "A".into(),
            uri: "content://media/1".into(),
        };
        let b = Sound::Device {
            name: "A".into(),
            uri: "content://media/2".into(),
        };
        assert_ne!(a.key(), b.key());
        for sound in Sound::builtin_choices().into_iter().chain([a, b]) {
            assert!(
                sound
                    .key()
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            );
        }
    }

    #[test]
    fn settings_round_trip_through_toml() {
        let mut settings = enabled();
        settings.events.sound = Sound::Tone {
            tone: Tone::Kalimba,
        };
        settings.sleep.sound = Sound::Device {
            name: "Bell".into(),
            uri: "/usr/share/sounds/bell.oga".into(),
        };
        settings.habits.sound = Sound::Silent;

        let text = toml::to_string(&settings).unwrap();
        let parsed: ReminderSettings = toml::from_str(&text).unwrap();
        assert_eq!(parsed, settings);

        let partial: ReminderSettings = toml::from_str(
            "enabled = true\n[events]\nsound = { kind = \"tone\", tone = \"rise\" }",
        )
        .unwrap();
        assert!(partial.enabled);
        assert_eq!(partial.events.sound, Sound::Tone { tone: Tone::Rise });
        assert!(partial.events.enabled);
        assert_eq!(partial.sleep.minutes_before, 30);
    }
}
