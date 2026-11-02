//! Persisted calendar settings for the data layer.
//!
//! [`CalendarSettings`] is the one definition of the `[calendar]` table, used by
//! both apps. [`Config`] is deliberately a subset of the iced app's config: the
//! Dioxus day view only needs that table. Unknown settings in a full desktop
//! config are ignored, so the same `config.toml` can be dropped onto a phone
//! without rewriting it.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::calendar::colors;
use crate::hex::HexColor;
use iced_core::Color;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub calendar: CalendarSettings,
    pub projects: ProjectsSettings,
    pub travel: crate::places::PlaceSettings,
}

/// A project that tasks can be filed under.
///
/// Tasks refer to a project by `id`, never by name, so a project can be
/// renamed without losing track of what belongs to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: u64,
    /// Slash-separated, so `School/Calculus` reads as a part of `School`.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<HexColor>,
    /// Where the project's files are kept on disk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<PathBuf>,
}

impl Project {
    pub fn color(&self) -> Option<Color> {
        self.color.map(Color::from)
    }
}

/// The projects shown in the Projects tab and offered when editing a task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectsSettings {
    pub items: Vec<Project>,
}

impl Default for ProjectsSettings {
    fn default() -> Self {
        // Fixed ids, so the defaults mean the same project on every machine.
        let items = [
            "Work",
            "School/Homework",
            "School/Calculus",
            "Personal",
            "Personal/Chores",
        ]
        .into_iter()
        .zip(1..)
        .map(|(name, id)| Project {
            id,
            name: name.to_owned(),
            color: None,
            directory: None,
        })
        .collect();

        let mut settings = Self { items };
        settings.sort();
        settings
    }
}

impl ProjectsSettings {
    /// Keeps the projects in alphabetical order, ignoring case.
    pub fn sort(&mut self) {
        self.items
            .sort_by_cached_key(|project| (project.name.to_lowercase(), project.id));
    }

    /// The projects in alphabetical order, whatever order the file lists them in.
    pub fn sorted(&self) -> Vec<&Project> {
        let mut projects: Vec<&Project> = self.items.iter().collect();
        projects.sort_by_cached_key(|project| (project.name.to_lowercase(), project.id));
        projects
    }

    pub fn get(&self, id: u64) -> Option<&Project> {
        self.items.iter().find(|project| project.id == id)
    }

    /// Tidies `input` into a project name and checks that no other project
    /// (`except` is the one being renamed) already has it.
    ///
    /// Spaces around a `/` are dropped and empty segments removed, so
    /// `School / Physics/` is the same project as `School/Physics`.
    pub fn clean_name(&self, input: &str, except: Option<u64>) -> Result<String, String> {
        let name = input
            .split('/')
            .map(str::trim)
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join("/");

        if name.is_empty() {
            return Err("A project needs a name".to_owned());
        }

        let taken = self.items.iter().any(|project| {
            Some(project.id) != except && project.name.to_lowercase() == name.to_lowercase()
        });

        if taken {
            return Err(format!("There is already a project named {name}"));
        }

        Ok(name)
    }

    /// Adds a project and returns its id.
    pub fn add(&mut self, name: String) -> u64 {
        let id = self.next_id();
        self.items.push(Project {
            id,
            name,
            color: None,
            directory: None,
        });
        self.sort();
        id
    }

    pub fn remove(&mut self, id: u64) {
        self.items.retain(|project| project.id != id);
    }

    /// An id that no project has had before, as far as the file can tell.
    ///
    /// Ids are never handed out again after a removal that took the highest
    /// one, so a task left pointing at a removed project does not suddenly
    /// belong to a new one. The clock keeps them unique between runs.
    fn next_id(&self) -> u64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64);
        let highest = self
            .items
            .iter()
            .map(|project| project.id)
            .max()
            .unwrap_or(0);

        now.max(highest + 1)
    }
}

/// Google Calendar connection and which calendars are shown on the calendar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarSettings {
    /// How often to pull events, in minutes. 15 is Google's own default-ish
    /// cadence and is what we ship with.
    pub sync_minutes: u32,
    /// OAuth client ID for a Desktop app created in Google Cloud. Empty until
    /// the user pastes one in Settings.
    pub client_id: String,
    /// Client secret from the same Desktop OAuth client. Google still issues
    /// one for installed apps; it is not treated as a real secret, but the
    /// token endpoint often wants it next to PKCE.
    pub client_secret: String,
    /// Calendar IDs the hamburger menu has toggled off.
    pub disabled: Vec<String>,
    /// Calendar-view source IDs, ordered from left to right for overlapping events.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_order: Vec<String>,
    /// Whether standalone Google Tasks are visible; tasks inside time blocks stay visible.
    pub show_tasks: bool,
    pub hide_overdue_tasks: bool,
    /// The discovered id of the app-owned TimeSlay calendar. Every event the
    /// app writes (time blocks and scheduled tasks) lives here. Managed by the
    /// app from the calendar list; not exposed as a user setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_calendar: Option<String>,
    pub time_block_list: Option<String>,
    /// Google Tasks list for tasks moved to the backlog. None uses Backlog.
    pub backlog_list: Option<String>,
    /// Google Tasks list that collects triaged tasks until they are scheduled.
    /// None uses Triaged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triaged_list: Option<String>,
    /// Google Tasks list that collects new tasks until they are triaged. None
    /// uses Inbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbox_list: Option<String>,
    #[serde(default)]
    pub inbox_sort: crate::calendar::task_sort::InboxSort,
    #[serde(default)]
    pub overdue_sort: crate::calendar::task_sort::InboxSort,
    /// Which metadata a task needs before it can leave the Inbox.
    #[serde(default)]
    pub triage: TriageRequirements,
    /// The hour the day ends, 0-5. 0 is midnight, the usual boundary. Set it
    /// to 3 and the day runs 3am to 3am, so the small hours stay with the
    /// evening they belong to. This only moves the view: an event keeps its
    /// real date and time, it is just drawn at the foot of the day it belongs
    /// to.
    pub day_end_hour: u32,
    /// Nightly sleep target in hours, clamped to 6–12.
    pub sleep_goal_hours: u32,
    /// Hour (0-6) at which a day starts for sleep calculations. Events before
    /// it are ignored when looking for the first event after waking up.
    pub sleep_day_start_hour: u32,
    /// Latest wake-up hour (4-14). With no earlier event, sleep ends here.
    pub latest_wakeup_hour: u32,
    /// Total visible days: 7 from the selected day, or 8 from the previous day.
    pub weekly_days: u32,
    /// Width of each day in the horizontally scrollable weekly view.
    pub weekly_day_width: u32,
    /// Show task cards instead of the weekly time grid.
    pub weekly_task_list: bool,
    /// Card placement in each day of the week board. Unlisted cards follow in time order.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub weekly_task_order: BTreeMap<String, Vec<String>>,
    /// Custom colors for Google Tasks lists, keyed by the calendar id
    /// (`tasks:<list>`). Google hands every list back with the same blue, so
    /// this is where a list gets a color of its own. A missing key means the
    /// shared task blue.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub task_colors: BTreeMap<String, HexColor>,
    /// Local calendar color overrides, keyed by Google Calendar ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub calendar_colors: BTreeMap<String, HexColor>,
    /// Color of all-day events on the TimeSlay calendar. None uses the
    /// built-in event color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_color: Option<HexColor>,
    /// Color of sleep events on the TimeSlay calendar. None uses the built-in
    /// sleep gray.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sleep_color: Option<HexColor>,
    /// Color of time-block events on the TimeSlay calendar. None uses the
    /// built-in block color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_block_color: Option<HexColor>,
    /// Color of scheduled task events on the TimeSlay calendar. None uses
    /// Google's task blue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_event_color: Option<HexColor>,
    /// Color of habit events on the TimeSlay calendar. None uses the built-in
    /// grape. A habit with its own Google `colorId` keeps that color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub habit_color: Option<HexColor>,
    /// The calendar new events are created on, by id. `None` means the primary
    /// calendar, falling back to the first writable one. Set from the calendar
    /// settings pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_calendar: Option<String>,
}

impl Default for CalendarSettings {
    fn default() -> Self {
        Self {
            sync_minutes: 60,
            client_id: String::new(),
            client_secret: String::new(),
            disabled: Vec::new(),
            source_order: Vec::new(),
            show_tasks: true,
            hide_overdue_tasks: false,
            app_calendar: None,
            time_block_list: None,
            backlog_list: None,
            triaged_list: None,
            inbox_list: None,
            inbox_sort: Default::default(),
            overdue_sort: Default::default(),
            triage: TriageRequirements::default(),
            day_end_hour: 0,
            sleep_goal_hours: 8,
            sleep_day_start_hour: 0,
            latest_wakeup_hour: 8,
            weekly_days: 8,
            weekly_day_width: 150,
            weekly_task_list: false,
            weekly_task_order: BTreeMap::new(),
            task_colors: BTreeMap::new(),
            calendar_colors: BTreeMap::new(),
            event_color: None,
            sleep_color: None,
            time_block_color: None,
            task_event_color: None,
            habit_color: None,
            default_calendar: None,
        }
    }
}

/// Which metadata a task must have before it can be triaged out of the Inbox.
/// Everything is required by default: the Inbox is where each task is given a
/// project, a priority and an estimate before it goes anywhere else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TriageRequirements {
    pub estimate: bool,
    pub project: bool,
    pub priority: bool,
}

impl Default for TriageRequirements {
    fn default() -> Self {
        Self {
            estimate: true,
            project: true,
            priority: true,
        }
    }
}

impl CalendarSettings {
    pub fn sleep_day_start_hour(&self) -> u32 {
        self.sleep_day_start_hour.min(6)
    }

    pub fn latest_wakeup_hour(&self) -> u32 {
        self.latest_wakeup_hour.clamp(4, 14)
    }

    pub fn sleep_goal_hours(&self) -> u32 {
        self.sleep_goal_hours.clamp(6, 12)
    }

    /// Synthetic source shared by every Google Tasks list.
    pub const GOOGLE_TASKS_ID: &str = "tasks:";

    /// The Google Tasks list time-blocked and scheduled tasks are kept on,
    /// unless another is chosen in Settings.
    pub const DEFAULT_TIME_BLOCK_LIST: &str = "Time Blocked";

    /// The Google Tasks list tasks are moved to for the backlog, unless
    /// another is chosen in Settings.
    pub const DEFAULT_BACKLOG_LIST: &str = "Backlog";

    /// The Google Tasks list triaged tasks are kept on, unless another is
    /// chosen in Settings.
    pub const DEFAULT_TRIAGED_LIST: &str = "Triaged";

    /// The name of the single calendar every event the app writes lives on.
    /// Created on first use; not customizable from Settings.
    pub const APP_CALENDAR_NAME: &str = "TimeSlay";

    /// Names the app calendar has had before. They are still recognized so a
    /// half-renamed calendar is reused rather than duplicated.
    pub const APP_CALENDAR_ALIASES: [&str; 2] = ["TimeSlayer", "TimeSlay"];

    /// Whether `name` is the app calendar or one of its former names.
    pub fn is_app_calendar_name(name: &str) -> bool {
        let name = name.trim();
        name == Self::APP_CALENDAR_NAME || Self::APP_CALENDAR_ALIASES.contains(&name)
    }

    pub const MIN_SYNC_MINUTES: u32 = 1;
    pub const MAX_SYNC_MINUTES: u32 = 1440;
    /// The latest hour a day can end: 5am, which is as far into the morning
    /// as a late night is worth pushing the boundary. Midnight (0) is the
    /// default.
    pub const MAX_DAY_END_HOUR: u32 = 5;

    pub fn weekly_days(&self) -> u32 {
        self.weekly_days.clamp(7, 8)
    }

    pub fn weekly_day_width(&self) -> u32 {
        self.weekly_day_width.clamp(100, 400)
    }

    pub fn sync_every(&self) -> Duration {
        let minutes = self
            .sync_minutes
            .clamp(Self::MIN_SYNC_MINUTES, Self::MAX_SYNC_MINUTES);
        Duration::from_secs(u64::from(minutes) * 60)
    }

    /// Minutes past midnight at which the day ends, clamped to a real hour.
    pub fn day_end_minutes(&self) -> i32 {
        self.day_start_hour() as i32 * 60
    }

    /// The hour the day's window opens, which is the hour the previous day
    /// ended. The grid's top row is labelled with this hour.
    pub fn day_start_hour(&self) -> u32 {
        self.day_end_hour.min(Self::MAX_DAY_END_HOUR)
    }

    pub fn set_calendar_enabled(&mut self, id: &str, enabled: bool) {
        if enabled {
            self.disabled.retain(|hidden| hidden != id);
        } else if !self.disabled.iter().any(|hidden| hidden == id) {
            self.disabled.push(id.to_owned());
        }
    }

    /// The custom color set for a Google Tasks list, if it has one.
    pub fn task_color(&self, calendar_id: &str) -> Option<Color> {
        self.task_colors
            .get(calendar_id)
            .or_else(|| self.task_colors.get(Self::GOOGLE_TASKS_ID))
            .map(|color| color.0)
    }

    /// Sets a task list's custom color, or clears it back to Google's blue.
    pub fn set_task_color(&mut self, calendar_id: &str, color: Option<Color>) {
        // Picking the aggregate swatch applies to every list.
        if calendar_id == Self::GOOGLE_TASKS_ID {
            self.task_colors.clear();
        }
        match color {
            Some(color) => {
                self.task_colors
                    .insert(calendar_id.to_owned(), HexColor(color));
            }
            None => {
                self.task_colors.remove(calendar_id);
            }
        }
    }

    /// The color all-day events are drawn in, or the built-in event color.
    pub fn event_color(&self) -> Color {
        self.event_color
            .map_or(colors::DEFAULT_EVENT, |color| color.0)
    }

    /// The color sleep events are drawn in, or the built-in sleep gray.
    pub fn sleep_color(&self) -> Color {
        self.sleep_color
            .map_or(crate::calendar::sleep::COLOR, |color| color.0)
    }

    /// The color time-block events are drawn in, or the built-in block color.
    pub fn time_block_color(&self) -> Color {
        self.time_block_color
            .map_or(colors::TIME_BLOCK, |color| color.0)
    }

    /// The color scheduled task events are drawn in, or Google's task blue.
    pub fn task_event_color(&self) -> Color {
        self.task_event_color.map_or(colors::TASK, |color| color.0)
    }

    /// The color habit events are drawn in, unless the habit has a Google
    /// color of its own.
    pub fn habit_color(&self) -> Color {
        self.habit_color.map_or(colors::HABIT, |color| color.0)
    }

    /// Sets the all-day event color, or clears it back to the built-in default.
    pub fn set_event_color(&mut self, color: Option<Color>) {
        self.event_color = color.map(HexColor);
    }

    /// Sets the sleep color, or clears it back to the built-in gray.
    pub fn set_sleep_color(&mut self, color: Option<Color>) {
        self.sleep_color = color.map(HexColor);
    }

    /// Sets the time-block color, or clears it back to the built-in default.
    pub fn set_time_block_color(&mut self, color: Option<Color>) {
        self.time_block_color = color.map(HexColor);
    }

    /// Sets the scheduled task color, or clears it back to Google's blue.
    pub fn set_task_event_color(&mut self, color: Option<Color>) {
        self.task_event_color = color.map(HexColor);
    }

    /// Sets the habit color, or clears it back to the built-in grape.
    pub fn set_habit_color(&mut self, color: Option<Color>) {
        self.habit_color = color.map(HexColor);
    }
}

pub fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("dev", "TimeSlay", "timeslay")
}

pub fn config_path() -> Option<PathBuf> {
    Some(project_dirs()?.config_dir().join("config.toml"))
}

/// Where cached days live, matching the iced app's XDG cache directory.
pub fn cache_days_dir() -> Option<PathBuf> {
    Some(project_dirs()?.cache_dir().join("days"))
}

/// Stores the Google OAuth client in `config.toml`, keeping every other
/// setting (including ones this crate does not know about) as it was.
///
/// The file is created readable by the app only, since it holds the secret.
pub fn save_google_client(client_id: &str, client_secret: &str) -> io::Result<()> {
    let path = config_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory available"))?;
    save_google_client_at(&path, client_id, client_secret)
}

fn save_google_client_at(
    path: &std::path::Path,
    client_id: &str,
    client_secret: &str,
) -> io::Result<()> {
    edit_calendar_table(path, |calendar| {
        calendar.insert("client_id".into(), client_id.trim().into());
        calendar.insert("client_secret".into(), client_secret.trim().into());
    })
}

/// Saves which sources are switched off, leaving the rest of the file alone.
pub fn save_calendar_visibility(settings: &CalendarSettings) -> io::Result<()> {
    let path = config_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory available"))?;
    save_calendar_visibility_at(&path, settings)
}

fn save_calendar_visibility_at(
    path: &std::path::Path,
    settings: &CalendarSettings,
) -> io::Result<()> {
    edit_calendar_table(path, |calendar| {
        calendar.insert("disabled".into(), settings.disabled.clone().into());
        calendar.insert("show_tasks".into(), settings.show_tasks.into());
    })
}

/// Saves whether travel is shown on the calendar.
pub fn save_travel_show(show: bool) -> io::Result<()> {
    let path = config_path().ok_or_else(no_config_directory)?;
    save_travel_show_at(&path, show)
}

fn save_travel_show_at(path: &std::path::Path, show: bool) -> io::Result<()> {
    edit_table(path, "travel", |travel| {
        travel.insert("show".into(), show.into());
        Ok(())
    })
}

/// Saves a place Mapbox found and returns its id, reusing one already saved.
pub fn save_found_place(found: &crate::places::Found) -> io::Result<u64> {
    let path = config_path().ok_or_else(no_config_directory)?;
    save_found_place_at(&path, found)
}

fn save_found_place_at(path: &std::path::Path, found: &crate::places::Found) -> io::Result<u64> {
    let mut id = 0;
    edit_table(path, "travel", |travel| {
        let invalid = |error: String| io::Error::new(io::ErrorKind::InvalidData, error);
        let mut places: Vec<crate::places::Place> = match travel.get("places") {
            Some(places) => places
                .clone()
                .try_into()
                .map_err(|error: toml::de::Error| invalid(error.to_string()))?,
            None => Vec::new(),
        };
        id = crate::places::add_place(
            &mut places,
            &found.name,
            &found.address,
            &found.place_id,
            found.lat,
            found.lng,
        );
        let places = toml::Value::try_from(places)
            .map_err(|error: toml::ser::Error| invalid(error.to_string()))?;
        travel.insert("places".into(), places);
        Ok(())
    })?;
    Ok(id)
}

/// Records the saved place a calendar event was given, or forgets it. The iced
/// app prefers this over the event's location text, so a change has to reach it.
pub fn save_event_place(event_id: &str, place: Option<u64>) -> io::Result<()> {
    let path = config_path().ok_or_else(no_config_directory)?;
    save_event_place_at(&path, event_id, place)
}

fn save_event_place_at(
    path: &std::path::Path,
    event_id: &str,
    place: Option<u64>,
) -> io::Result<()> {
    edit_table(path, "travel", |travel| {
        let places = travel
            .entry("event_places")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "event_places is not a table")
            })?;
        match place {
            Some(place) => {
                let place = i64::try_from(place)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                places.insert(event_id.to_owned(), place.into());
            }
            None => {
                places.remove(event_id);
            }
        }
        Ok(())
    })
}

fn no_config_directory() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "no config directory available")
}

/// Rewrites only the given keys of `[calendar]`. The Dioxus app's [`Config`] is
/// a subset of the file, so saving that struct would drop the desktop settings.
fn edit_calendar_table(
    path: &std::path::Path,
    edit: impl FnOnce(&mut toml::Table),
) -> io::Result<()> {
    edit_table(path, "calendar", |calendar| {
        edit(calendar);
        Ok(())
    })
}

/// Rewrites one table of the config, leaving the rest of the file alone.
fn edit_table(
    path: &std::path::Path,
    name: &str,
    edit: impl FnOnce(&mut toml::Table) -> io::Result<()>,
) -> io::Result<()> {
    crate::fsutil::locked(path, || {
        let mut document = match fs::read_to_string(path) {
            Ok(text) => toml::from_str::<toml::Table>(&text)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => toml::Table::new(),
            Err(error) => return Err(error),
        };
        let table = document
            .entry(name)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("[{name}] is not a table"),
                )
            })?;
        edit(table)?;

        let text = toml::to_string_pretty(&document)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        crate::fsutil::write_atomic(path, text.as_bytes(), crate::fsutil::Kind::Private)
    })
}

impl Config {
    /// Reads the config, falling back to defaults for anything unreadable.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            log::warn!("no config directory available; using defaults");
            return Self::default();
        };
        if let Err(error) = crate::fsutil::make_private(&path)
            && error.kind() != io::ErrorKind::NotFound
        {
            log::warn!("could not make {} private: {error}", path.display());
        }

        match fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str(&contents) {
                Ok(config) => config,
                Err(error) => {
                    log::warn!(
                        "{} could not be parsed, using defaults instead: {error}",
                        path.display()
                    );
                    Self::default()
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                log::warn!("{} could not be read: {error}", path.display());
                Self::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_the_client_keeps_other_settings_and_is_private() {
        let dir = std::env::temp_dir().join(format!("tt-config-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("config.toml");

        save_google_client_at(&path, " id ", " secret ").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("client_id = \"id\""));
        assert!(text.contains("client_secret = \"secret\""));

        fs::write(&path, "[other]\nkeep = 1\n\n[calendar]\nsync_minutes = 5\n").unwrap();
        save_google_client_at(&path, "id2", "secret2").unwrap();
        let config: toml::Table = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(config["other"]["keep"].as_integer(), Some(1));
        assert_eq!(config["calendar"]["sync_minutes"].as_integer(), Some(5));
        assert_eq!(config["calendar"]["client_id"].as_str(), Some("id2"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::remove_file(&path);
            save_google_client_at(&path, "a", "b").unwrap();
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_travel_switch_keeps_the_rest_of_travel() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            "[travel]\napi_key = \"pk\"\n\n[[travel.routes]]\nfrom = 1\nto = 2\nmode = \"car\"\nseconds = 600\nmeters = 5000\n",
        )
        .unwrap();

        save_travel_show_at(&path, true).unwrap();

        let saved: Config = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(saved.travel.show);
        assert_eq!(saved.travel.api_key, "pk");
        assert_eq!(saved.travel.routes.len(), 1);
    }

    #[test]
    fn saving_places_keeps_the_rest_of_travel_and_reuses_a_saved_place() {
        let dir = std::env::temp_dir().join(format!("tt-places-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(
            &path,
            "[window]\nwidth = 900\n\n[travel]\napi_key = \"pk\"\n",
        )
        .unwrap();
        let found = crate::places::Found {
            place_id: "mapbox-1".into(),
            name: "Library".into(),
            address: "1 Main St".into(),
            lat: 1.5,
            lng: 2.5,
        };

        let id = save_found_place_at(&path, &found).unwrap();
        assert_eq!(save_found_place_at(&path, &found).unwrap(), id);
        save_event_place_at(&path, "standup", Some(id)).unwrap();
        save_event_place_at(&path, "other", Some(id)).unwrap();
        save_event_place_at(&path, "other", None).unwrap();

        let saved: Config = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved.travel.places.len(), 1);
        assert_eq!(saved.travel.api_key, "pk");
        assert_eq!(saved.travel.event_places.len(), 1);
        assert_eq!(saved.travel.event_places["standup"], id);
        let table: toml::Table = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(table["window"]["width"].as_integer(), Some(900));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_visibility_keeps_other_settings_and_refuses_a_broken_file() {
        let dir = std::env::temp_dir().join(format!("tt-visibility-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(
            &path,
            "[window]\nwidth = 900\n\n[calendar]\nclient_id = \"id\"\ndisabled = [\"old\"]\n",
        )
        .unwrap();

        let settings = CalendarSettings {
            disabled: vec!["work".into(), "timeslay:habits".into()],
            show_tasks: false,
            ..CalendarSettings::default()
        };
        save_calendar_visibility_at(&path, &settings).unwrap();

        let saved: Config = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved.calendar.disabled, settings.disabled);
        assert!(!saved.calendar.show_tasks);
        assert_eq!(saved.calendar.client_id, "id");
        let table: toml::Table = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(table["window"]["width"].as_integer(), Some(900));

        fs::write(&path, "not [ toml").unwrap();
        assert!(save_calendar_visibility_at(&path, &settings).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "not [ toml");
        let _ = fs::remove_dir_all(&dir);
    }
}
