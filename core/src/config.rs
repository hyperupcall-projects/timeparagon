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
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::calendar::colors;
use crate::hex::HexColor;
use iced_core::Color;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub database_sync: DatabaseSyncSettings,
    pub notes: NotesSettings,
    pub calendar: CalendarSettings,
    pub projects: ProjectsSettings,
    pub travel: crate::places::PlaceSettings,
    pub appearance: AppearanceSettings,
    pub notepad: NotepadText,
    pub todos: crate::todos::TodoList,
    pub brain_dump: crate::brain_dump::Dumps,
    pub reminders: crate::reminders::ReminderSettings,
}

/// Device-local server settings. An empty URL disables network sync.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DatabaseSyncSettings {
    pub url: String,
}

impl DatabaseSyncSettings {
    /// Read only sync settings, without opening the local database.
    pub fn load() -> Self {
        #[derive(Default, Deserialize)]
        #[serde(default)]
        struct Settings {
            database_sync: DatabaseSyncSettings,
        }
        config_path()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str::<Settings>(&text).ok())
            .unwrap_or_default()
            .database_sync
    }
}

/// `[notes]`: where the notes are kept, one Markdown file each. Empty, the
/// default, means nowhere: notes cannot be saved until it is set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotesSettings {
    /// A directory; a leading `~/` is the home directory.
    pub directory: String,
}

impl NotesSettings {
    /// Reads only `[notes]`, without opening the local database.
    pub fn load() -> Self {
        #[derive(Default, Deserialize)]
        #[serde(default)]
        struct Settings {
            notes: NotesSettings,
        }
        config_path()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str::<Settings>(&text).ok())
            .unwrap_or_default()
            .notes
    }

    pub fn is_empty(&self) -> bool {
        self.directory.trim().is_empty()
    }

    /// The directory, or `None` while none is set.
    pub fn path(&self) -> Option<PathBuf> {
        let directory = self.directory.trim();
        if directory.is_empty() {
            return None;
        }
        if let Some(rest) = directory.strip_prefix("~/")
            && let Some(home) = directories::BaseDirs::new()
        {
            return Some(home.home_dir().join(rest));
        }
        Some(PathBuf::from(directory))
    }
}

pub fn save_database_sync_url(url: &str) -> io::Result<()> {
    let path = config_path().ok_or_else(no_config_directory)?;
    save_database_sync_url_at(&path, url)
}

/// Checks a sync server URL the way the sync client will read it. A blank URL
/// is valid: it disables sync.
pub fn validate_database_sync_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.is_empty() {
        return Ok(());
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err("The sync URL must start with https://, http://, libsql:// or turso://".into());
    };
    if !["https", "http", "libsql", "turso"].contains(&scheme.to_ascii_lowercase().as_str()) {
        return Err(format!(
            "Unsupported sync URL scheme \"{scheme}://\"; use https://, http://, libsql:// or turso://"
        ));
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.is_empty() {
        return Err("The sync URL is missing a server name".into());
    }
    if url.chars().any(char::is_whitespace) {
        return Err("The sync URL cannot contain spaces".into());
    }
    Ok(())
}

fn save_database_sync_url_at(path: &std::path::Path, url: &str) -> io::Result<()> {
    validate_database_sync_url(url).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    edit_table(path, "database_sync", |table| {
        table.insert("url".into(), url.trim().into());
        Ok(())
    })
}

/// The part of `[notepad]` the Dioxus app edits. Its font size is the desktop's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotepadText {
    pub text: String,
}

/// The part of `[appearance]` the Dioxus app follows. The theme is left out:
/// it is chosen per device there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    pub distance_unit: crate::places::DistanceUnit,
    pub time_format: crate::calendar::time_format::TimeFormat,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            time_format: crate::calendar::time_format::TimeFormat::TwentyFourHour,
            distance_unit: crate::places::DistanceUnit::default(),
        }
    }
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
    /// The [`LifeArea`] it belongs to, by id. Unset, or pointing at an area
    /// that is gone, means [`PERSONAL_AREA`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<u64>,
}

impl Project {
    pub fn color(&self) -> Option<Color> {
        self.color.map(Color::from)
    }
}

/// The life area that projects and habits without one belong to. It always
/// exists and cannot be removed, though it can be renamed.
pub const PERSONAL_AREA: u64 = 1;

/// A part of life, such as health or work, with a vision of where it is
/// going, the goals that get it there, and the projects and habits in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LifeArea {
    pub id: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<HexColor>,
    /// Markdown describing where this area of life is meant to go.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub vision: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub goals: Vec<Goal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objectives: Vec<Objective>,
}

impl LifeArea {
    pub fn color(&self) -> Option<Color> {
        self.color.map(Color::from)
    }

    /// The goal `objective` is listed under: its own when that goal still
    /// exists in this area, otherwise none.
    pub fn objective_goal(&self, objective: &Objective) -> Option<u64> {
        objective
            .goal
            .filter(|id| self.goals.iter().any(|goal| goal.id == *id))
    }

    /// Whether any objective of this area is carried out by `habit`.
    pub fn has_habit(&self, habit: &str) -> bool {
        self.objectives
            .iter()
            .any(|objective| objective.habits.iter().any(|h| h == habit))
    }
}

/// One goal of a life area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    pub id: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// Something concrete that a life area works towards, usually in service of
/// one of its goals, and carried out by habits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Objective {
    pub id: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The [`Goal`] of the same area it serves. Unset, or pointing at a goal
    /// that is gone, means it is unassigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<u64>,
    /// Habits that carry it out, by habit id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub habits: Vec<String>,
}

fn personal_area() -> LifeArea {
    LifeArea {
        id: PERSONAL_AREA,
        name: "Personal".to_owned(),
        color: None,
        vision: String::new(),
        goals: Vec::new(),
        objectives: Vec::new(),
    }
}

fn default_areas() -> Vec<LifeArea> {
    vec![personal_area()]
}

/// The projects offered when editing a task, and the life areas they are
/// grouped under in the Life tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectsSettings {
    pub items: Vec<Project>,
    /// In the order they are listed.
    #[serde(default = "default_areas")]
    pub areas: Vec<LifeArea>,
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
            area: None,
        })
        .collect();

        let mut settings = Self {
            items,
            areas: default_areas(),
        };
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
            area: None,
        });
        self.sort();
        id
    }

    pub fn remove(&mut self, id: u64) {
        self.items.retain(|project| project.id != id);
    }

    pub fn area(&self, id: u64) -> Option<&LifeArea> {
        self.areas.iter().find(|area| area.id == id)
    }

    pub fn area_mut(&mut self, id: u64) -> Option<&mut LifeArea> {
        self.areas.iter_mut().find(|area| area.id == id)
    }

    /// The area something filed under `area` is listed in: that area when it
    /// still exists, otherwise [`PERSONAL_AREA`].
    pub fn resolve_area(&self, area: Option<u64>) -> u64 {
        area.filter(|id| self.area(*id).is_some())
            .unwrap_or(PERSONAL_AREA)
    }

    /// Puts back the Personal area if the data somehow lost it, so that what
    /// falls back to it is never left without a place to be listed.
    pub fn ensure_personal_area(&mut self) {
        if self.area(PERSONAL_AREA).is_none() {
            self.areas.insert(0, personal_area());
        }
    }

    /// Tidies an area name and checks that no other area (`except` is the
    /// one being renamed) already has it.
    pub fn clean_area_name(&self, input: &str, except: Option<u64>) -> Result<String, String> {
        let name = input.trim();
        if name.is_empty() {
            return Err("A life area needs a name".to_owned());
        }
        let taken = self
            .areas
            .iter()
            .any(|area| Some(area.id) != except && area.name.to_lowercase() == name.to_lowercase());
        if taken {
            return Err(format!("There is already a life area named {name}"));
        }
        Ok(name.to_owned())
    }

    /// Adds a life area at the end of the list and returns its id.
    pub fn add_area(&mut self, name: String) -> u64 {
        let id = unique_id(self.areas.iter().map(|area| area.id));
        self.areas.push(LifeArea {
            id,
            name,
            color: None,
            vision: String::new(),
            goals: Vec::new(),
            objectives: Vec::new(),
        });
        id
    }

    /// Removes a life area. What was in it falls back to Personal, which
    /// itself cannot be removed.
    pub fn remove_area(&mut self, id: u64) {
        if id != PERSONAL_AREA {
            self.areas.retain(|area| area.id != id);
        }
    }

    /// Adds a goal to an area and returns its id.
    pub fn add_goal(&mut self, area: u64, name: String) -> Option<u64> {
        let area = self.area_mut(area)?;
        let id = unique_id(area.goals.iter().map(|goal| goal.id));
        area.goals.push(Goal {
            id,
            name,
            description: String::new(),
        });
        Some(id)
    }

    /// Adds an objective to an area, under `goal` if given, and returns its id.
    pub fn add_objective(&mut self, area: u64, goal: Option<u64>, name: String) -> Option<u64> {
        let area = self.area_mut(area)?;
        let id = unique_id(area.objectives.iter().map(|objective| objective.id));
        area.objectives.push(Objective {
            id,
            name,
            description: String::new(),
            goal,
            habits: Vec::new(),
        });
        Some(id)
    }

    /// An id that no project has had before, as far as the file can tell.
    ///
    /// Ids are never handed out again after a removal that took the highest
    /// one, so a task left pointing at a removed project does not suddenly
    /// belong to a new one. The clock keeps them unique between runs.
    fn next_id(&self) -> u64 {
        unique_id(self.items.iter().map(|project| project.id))
    }
}

/// An id above every one in `taken`, from the clock so that ids stay unique
/// between runs and devices.
fn unique_id(taken: impl Iterator<Item = u64>) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64);
    let highest = taken.max().unwrap_or(0);

    now.max(highest + 1)
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
    /// The first column of the month and two-week views, and the date pickers.
    pub first_day_of_week: crate::calendar::first_day::FirstDayOfWeek,
    /// Total visible days: 7 or 8 from the selected day, or 9 for 8 from the
    /// selected day plus the day before it.
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
            first_day_of_week: Default::default(),
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

/// Which days the week view shows, relative to the selected day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeekRange {
    /// The selected day and the six after it: Wed to next Tue.
    Seven,
    /// The selected day and the seven after it: Wed to next Wed.
    Eight,
    /// `Eight`, with the day before the selected day in front: Tue to next Wed.
    EightAndYesterday,
}

impl WeekRange {
    pub const ALL: [Self; 3] = [Self::Seven, Self::Eight, Self::EightAndYesterday];

    /// The number of columns the range shows.
    pub fn days(self) -> u32 {
        match self {
            Self::Seven => 7,
            Self::Eight => 8,
            Self::EightAndYesterday => 9,
        }
    }
}

impl std::fmt::Display for WeekRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Seven => "7 days",
            Self::Eight => "8 days",
            Self::EightAndYesterday => "8 days + ystd.",
        })
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

    /// How many days the week view shows, yesterday included.
    pub fn weekly_days(&self) -> u32 {
        self.weekly_days.clamp(7, 9)
    }

    /// How far the week view's arrows move: the days shown from the selected
    /// day on. Yesterday is context, so it repeats as the next page's first
    /// column rather than being skipped.
    pub fn weekly_step(&self) -> u32 {
        self.weekly_days().min(8)
    }

    pub fn week_range(&self) -> WeekRange {
        match self.weekly_days() {
            7 => WeekRange::Seven,
            8 => WeekRange::Eight,
            _ => WeekRange::EightAndYesterday,
        }
    }

    pub fn set_week_range(&mut self, range: WeekRange) {
        self.weekly_days = range.days();
    }

    /// The days the week view shows for `anchor`.
    pub fn week_dates(&self, anchor: chrono::NaiveDate) -> Vec<chrono::NaiveDate> {
        crate::calendar::week_dates(anchor, self.weekly_days())
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

/// Where the app keeps its config, database, tokens and caches.
///
/// Mirrors the accessors of [`directories::ProjectDirs`], so callers read the
/// same way, but can also point at a scratch directory (see [`isolated`]).
#[derive(Debug, Clone)]
pub struct ProjectDirs {
    config: PathBuf,
    data: PathBuf,
    data_local: PathBuf,
    cache: PathBuf,
    state: Option<PathBuf>,
}

impl ProjectDirs {
    fn under(root: &Path) -> Self {
        Self {
            config: root.join("config"),
            data: root.join("data"),
            data_local: root.join("data-local"),
            cache: root.join("cache"),
            state: Some(root.join("state")),
        }
    }

    pub fn config_dir(&self) -> &Path {
        &self.config
    }

    pub fn data_dir(&self) -> &Path {
        &self.data
    }

    pub fn data_local_dir(&self) -> &Path {
        &self.data_local
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache
    }

    pub fn state_dir(&self) -> Option<&Path> {
        self.state.as_deref()
    }
}

pub fn project_dirs() -> Option<ProjectDirs> {
    if let Some(root) = isolated_root() {
        return Some(ProjectDirs::under(root));
    }
    let dirs = directories::ProjectDirs::from("dev", "TimeSlay", "timeslay")?;
    Some(ProjectDirs {
        config: dirs.config_dir().to_owned(),
        data: dirs.data_dir().to_owned(),
        data_local: dirs.data_local_dir().to_owned(),
        cache: dirs.cache_dir().to_owned(),
        state: dirs.state_dir().map(Path::to_owned),
    })
}

/// Whether this process is a test run that must not touch the user's files or
/// accounts. Every path then lives in a fresh scratch directory, so tests never
/// see the real database, config or Google tokens, and background workers that
/// talk to Google or the sync server stay off.
///
/// Core's own tests are always isolated. Other crates' tests opt in through the
/// `test-isolation` feature from their `[dev-dependencies]`. Cargo can also
/// enable that feature for a normal binary when one invocation builds tests and
/// binaries together (`--all-targets`), so it only applies to executables that
/// Cargo placed in `deps/`, which is where test harnesses live.
pub fn isolated() -> bool {
    isolated_root().is_some()
}

fn isolated_root() -> Option<&'static Path> {
    static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
    ROOT.get_or_init(|| {
        let test_harness = || {
            std::env::current_exe().is_ok_and(|exe| {
                exe.parent()
                    .and_then(Path::file_name)
                    .is_some_and(|name| name == "deps")
            })
        };
        if !(cfg!(test) || (cfg!(feature = "test-isolation") && test_harness())) {
            return None;
        }
        // A scratch directory that is never handed out twice, so a test run
        // cannot fall back to the real directories if creation fails.
        let root = tempfile::Builder::new()
            .prefix("timeslay-test-")
            .tempdir()
            .expect("tests need a scratch directory instead of the real data")
            .keep();
        Some(root)
    })
    .as_deref()
}

pub fn config_path() -> Option<PathBuf> {
    Some(project_dirs()?.config_dir().join("config.toml"))
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

/// Saves which day weeks start on, leaving the rest of the file alone.
pub fn save_first_day_of_week(
    first_day: crate::calendar::first_day::FirstDayOfWeek,
) -> io::Result<()> {
    let path = config_path().ok_or_else(no_config_directory)?;
    save_first_day_of_week_at(&path, first_day)
}

fn save_first_day_of_week_at(
    path: &std::path::Path,
    first_day: crate::calendar::first_day::FirstDayOfWeek,
) -> io::Result<()> {
    edit_calendar_table(path, |calendar| {
        calendar.insert("first_day_of_week".into(), first_day.name().into());
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

/// Replaces the `[reminders]` table, leaving the rest of the file alone.
pub fn save_reminders(settings: &crate::reminders::ReminderSettings) -> io::Result<()> {
    let path = config_path().ok_or_else(no_config_directory)?;
    save_reminders_at(&path, settings)
}

fn save_reminders_at(
    path: &std::path::Path,
    settings: &crate::reminders::ReminderSettings,
) -> io::Result<()> {
    let value = toml::Value::try_from(settings)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let toml::Value::Table(replacement) = value else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "reminder settings are not a table",
        ));
    };
    edit_table(path, "reminders", |table| {
        *table = replacement;
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

/// The local database, which holds the scratch data below.
fn database() -> io::Result<PathBuf> {
    crate::task_db::path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data directory available"))
}

/// Replaces the notepad's text. Its font size is a setting, left in the config.
pub fn save_notepad_text(text: &str) -> io::Result<()> {
    let notepad = NotepadText {
        text: text.to_owned(),
    };
    crate::shared_data::save_section(&database()?, "notepad", &notepad)
}

/// Replaces the temporary todo list.
pub fn save_todos(todos: &crate::todos::TodoList) -> io::Result<()> {
    crate::shared_data::save_section(&database()?, "todos", todos)
}

/// Replaces the brain dump pages.
pub fn save_brain_dump(dumps: &crate::brain_dump::Dumps) -> io::Result<()> {
    crate::shared_data::save_section(&database()?, "brain_dump", dumps)
}

/// Replaces the list of projects.
pub fn save_projects(projects: &ProjectsSettings) -> io::Result<()> {
    crate::shared_data::save_section(&database()?, "projects", projects)
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
    /// Reads the config, with the scratch data from the database, falling back
    /// to defaults for anything unreadable.
    pub fn load() -> Self {
        // First, so data an older build left in the file has moved before it is read.
        let database = crate::task_db::path();
        let Some(path) = config_path() else {
            log::warn!("no config directory available; using defaults");
            return Self::default();
        };
        if let Err(error) = crate::fsutil::make_private(&path)
            && error.kind() != io::ErrorKind::NotFound
        {
            log::warn!("could not make {} private: {error}", path.display());
        }
        Self::load_from(&path, database.as_deref())
    }

    fn load_from(path: &std::path::Path, database: Option<&std::path::Path>) -> Self {
        let mut table = match fs::read_to_string(path) {
            Ok(contents) => match toml::from_str::<toml::Table>(&contents) {
                Ok(table) => table,
                Err(error) => {
                    log::warn!(
                        "{} could not be parsed, using defaults instead: {error}",
                        path.display()
                    );
                    toml::Table::new()
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => toml::Table::new(),
            Err(error) => {
                log::warn!("{} could not be read: {error}", path.display());
                toml::Table::new()
            }
        };
        if let Some(database) = database
            && let Err(error) = crate::shared_data::overlay(database, &mut table)
        {
            log::warn!("could not read the scratch data: {error}");
        }
        toml::Value::Table(table)
            .try_into()
            .unwrap_or_else(|error| {
                log::warn!(
                    "{} could not be parsed, using defaults instead: {error}",
                    path.display()
                );
                Self::default()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_url_defaults_to_disabled_and_preserves_other_settings() {
        assert!(Config::default().database_sync.url.is_empty());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(&path, "[window]\nwidth = 321\n").unwrap();
        save_database_sync_url_at(&path, "  https://sync.example.com  ").unwrap();
        let loaded = Config::load_from(&path, None);
        assert_eq!(loaded.database_sync.url, "https://sync.example.com");
        let table: toml::Table = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(table["window"]["width"].as_integer(), Some(321));
        save_database_sync_url_at(&path, "  ").unwrap();
        assert!(Config::load_from(&path, None).database_sync.url.is_empty());
    }

    #[test]
    fn invalid_sync_urls_are_rejected_and_not_saved() {
        for url in [
            "",
            "https://sync.example.com",
            "libsql://db.turso.io",
            "http://10.0.0.2:8080/",
        ] {
            assert_eq!(validate_database_sync_url(url), Ok(()), "{url}");
        }
        for url in [
            "sync.example.com",
            "ftp://example.com",
            "https://",
            "https://a b.com",
        ] {
            assert!(validate_database_sync_url(url).is_err(), "{url}");
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        save_database_sync_url_at(&path, "https://sync.example.com").unwrap();
        let error = save_database_sync_url_at(&path, "sync.example.com").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(
            Config::load_from(&path, None).database_sync.url,
            "https://sync.example.com"
        );
    }

    #[test]
    fn scratch_data_comes_from_the_database_and_leaves_the_config_alone() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let database = directory.path().join("test.sqlite3");
        let original = "[pomodoro]\nwork_minutes = 25\n[notepad]\nfont_size = 120\n";
        std::fs::write(&path, original).unwrap();

        let mut todos = crate::todos::TodoList::default();
        todos.add("write it");
        crate::shared_data::save_section(&database, "todos", &todos).unwrap();
        let mut dumps = crate::brain_dump::Dumps::default();
        dumps.set(
            chrono::NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(),
            "idea".into(),
        );
        crate::shared_data::save_section(&database, "brain_dump", &dumps).unwrap();
        let notepad = NotepadText { text: "new".into() };
        crate::shared_data::save_section(&database, "notepad", &notepad).unwrap();

        let read = Config::load_from(&path, Some(&database));
        assert_eq!(read.todos, todos);
        assert_eq!(read.brain_dump, dumps);
        assert_eq!(read.notepad.text, "new");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn the_time_format_is_read_from_the_shared_appearance_table() {
        let config: Config =
            toml::from_str("[appearance]\ntheme = \"Dark\"\ntime_format = \"compact\"").unwrap();
        assert_eq!(
            config.appearance.time_format,
            crate::calendar::time_format::TimeFormat::Compact
        );
        assert_eq!(
            Config::default().appearance.time_format,
            crate::calendar::time_format::TimeFormat::TwentyFourHour
        );
    }

    #[test]
    fn distance_unit_defaults_and_round_trips_in_shared_config() {
        use crate::places::DistanceUnit;
        let old: Config = toml::from_str("[appearance]\ntime_format = 'compact'").unwrap();
        assert_eq!(old.appearance.distance_unit, DistanceUnit::Kilometers);
        for unit in DistanceUnit::ALL {
            let mut config = old.clone();
            config.appearance.distance_unit = unit;
            let saved = toml::to_string(&config).unwrap();
            let restored: Config = toml::from_str(&saved).unwrap();
            assert_eq!(restored.appearance.distance_unit, unit);
        }
    }

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
    fn saving_the_first_day_of_the_week_keeps_other_settings() {
        use crate::calendar::first_day::FirstDayOfWeek;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            "[calendar]\nsync_minutes = 30\n\n[window]\nalways_on_top = true\n",
        )
        .unwrap();

        save_first_day_of_week_at(&path, FirstDayOfWeek::Saturday).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let document: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(document["window"]["always_on_top"].as_bool(), Some(true));
        let config: Config = toml::from_str(&text).unwrap();
        assert_eq!(config.calendar.sync_minutes, 30);
        assert_eq!(config.calendar.first_day_of_week, FirstDayOfWeek::Saturday);
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
    fn saving_reminders_replaces_only_their_table() {
        use crate::reminders::{ReminderSettings, Sound};
        use crate::sounds::Tone;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        fs::write(
            &path,
            "[window]\nwidth = 800\n\n[reminders]\nenabled = false\nstale = 1\n",
        )
        .unwrap();
        let mut settings = ReminderSettings {
            enabled: true,
            ..ReminderSettings::default()
        };
        settings.travel.sound = Sound::Tone { tone: Tone::Bell };

        save_reminders_at(&path, &settings).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let table: toml::Table = toml::from_str(&text).unwrap();
        assert_eq!(table["window"]["width"].as_integer(), Some(800));
        assert!(table["reminders"].get("stale").is_none());
        let saved: Config = toml::from_str(&text).unwrap();
        assert_eq!(saved.reminders, settings);
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
