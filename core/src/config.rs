//! Persisted calendar settings for the data layer.
//!
//! This is deliberately a subset of the iced app's config: the Dioxus day view
//! only needs the `[calendar]` table. Unknown settings in a full desktop config
//! are ignored, so the same `config.toml` can be dropped onto a phone without
//! rewriting it.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::hex::HexColor;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub calendar: CalendarSettings,
}

/// Google Calendar connection and which calendars are shown on the calendar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarSettings {
    pub sync_minutes: u32,
    pub client_id: String,
    pub client_secret: String,
    pub disabled: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_order: Vec<String>,
    pub show_tasks: bool,
    pub hide_overdue_tasks: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_calendar: Option<String>,
    pub time_block_list: Option<String>,
    pub backlog_list: Option<String>,
    pub inbox_list: Option<String>,
    pub triaged_list: Option<String>,
    pub habit_color: Option<HexColor>,
    /// The hour the day ends, 0-5. 0 is midnight.
    pub day_end_hour: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub task_colors: BTreeMap<String, HexColor>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub calendar_colors: BTreeMap<String, HexColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_block_color: Option<HexColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_event_color: Option<HexColor>,
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
            inbox_list: None,
            triaged_list: None,
            habit_color: None,
            day_end_hour: 0,
            task_colors: BTreeMap::new(),
            calendar_colors: BTreeMap::new(),
            time_block_color: None,
            task_event_color: None,
            default_calendar: None,
        }
    }
}

impl CalendarSettings {
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
    /// The latest hour a day can end.
    pub const MAX_DAY_END_HOUR: u32 = 5;

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

    /// The hour the day's window opens.
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
    pub fn task_color(&self, calendar_id: &str) -> Option<crate::Color> {
        self.task_colors
            .get(calendar_id)
            .or_else(|| self.task_colors.get(Self::GOOGLE_TASKS_ID))
            .map(|color| color.0)
    }

    /// Sets a task list's custom color, or clears it back to Google's blue.
    pub fn set_task_color(&mut self, calendar_id: &str, color: Option<crate::Color>) {
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

    /// The color time-block events are drawn in, or the built-in block color.
    pub fn time_block_color(&self) -> crate::Color {
        self.time_block_color
            .map_or(crate::calendar::colors::TIME_BLOCK, |color| color.0)
    }

    /// The color habit events are drawn in.
    pub fn habit_color(&self) -> crate::Color {
        self.habit_color
            .map_or(crate::calendar::colors::HABIT, |color| color.0)
    }

    /// The color scheduled task events are drawn in, or Google's task blue.
    pub fn task_event_color(&self) -> crate::Color {
        self.task_event_color
            .map_or(crate::calendar::colors::TASK, |color| color.0)
    }

    /// Sets the time-block color, or clears it back to the built-in default.
    pub fn set_time_block_color(&mut self, color: Option<crate::Color>) {
        self.time_block_color = color.map(HexColor);
    }

    /// Sets the scheduled task color, or clears it back to Google's blue.
    pub fn set_task_event_color(&mut self, color: Option<crate::Color>) {
        self.task_event_color = color.map(HexColor);
    }
}

pub fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("dev", "TimeSlay", "time-slay")
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
    use std::io::Write;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut document = match fs::read_to_string(path) {
        Ok(text) => toml::from_str::<toml::Table>(&text)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => toml::Table::new(),
        Err(error) => return Err(error),
    };
    let calendar = document
        .entry("calendar")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "[calendar] is not a table"))?;
    calendar.insert("client_id".into(), client_id.trim().into());
    calendar.insert("client_secret".into(), client_secret.trim().into());

    let text = toml::to_string_pretty(&document)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let temporary = path.with_extension("toml.tmp");
    let _ = fs::remove_file(&temporary);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(text.as_bytes())?;
    drop(file);
    fs::rename(&temporary, path)
}

impl Config {
    /// Reads the config, falling back to defaults for anything unreadable.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            log::warn!("no config directory available; using defaults");
            return Self::default();
        };

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
}
