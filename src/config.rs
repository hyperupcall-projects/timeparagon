//! Persisted settings.
//!
//! A malformed or half-written config must never stop the app from starting,
//! so every read falls back to defaults and every write goes through a
//! temporary file that is renamed into place.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use directories::ProjectDirs;
use iced::Color;
use serde::{Deserialize, Serialize};

use crate::audio::Tone;
use crate::hex::HexColor;
use crate::todos::TodoList;

/// How long to wait after the last change before writing to disk.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub appearance: Appearance,
    pub window: Window,
    pub timer: TimerSettings,
    pub alarm: AlarmProfile,
    pub calendar: CalendarSettings,
    pub projects: ProjectsSettings,
    #[serde(default, skip_serializing_if = "crate::task_work::WorkLog::is_empty")]
    pub task_work: crate::task_work::WorkLog,
    /// The temporary todo scratchpad. It is left out of the file while it is
    /// empty, so a config that never used it stays as it was.
    #[serde(default, skip_serializing_if = "TodoList::is_empty")]
    pub todos: TodoList,
    /// The notepad's page. It is left out of the file while it is blank, so a
    /// config that never used it stays as it was.
    #[serde(default, skip_serializing_if = "Notepad::is_empty")]
    pub notepad: Notepad,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            appearance: Appearance::default(),
            window: Window::default(),
            timer: TimerSettings::default(),
            alarm: AlarmProfile::default(),
            calendar: CalendarSettings::default(),
            projects: ProjectsSettings::default(),
            task_work: crate::task_work::WorkLog::default(),
            todos: TodoList::default(),
            notepad: Notepad::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// The name of the selected theme, matching what the picker shows.
    pub theme: String,
    /// When set, the theme follows the system light/dark preference and
    /// `theme` is only remembered for when it is switched back off.
    pub follow_system: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: "Sakura".to_owned(),
            follow_system: false,
        }
    }
}

/// The notepad: a page of free text that is kept between sessions.
///
/// It lives in the config file rather than in a document of its own, so there
/// is exactly one place the app reads and writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notepad {
    /// What is written on the page.
    #[serde(default)]
    pub text: String,
    /// How large the text on the page is, as a percentage of the size it is
    /// drawn at by default.
    #[serde(default = "default_font_size")]
    pub font_size: u8,
}

impl Default for Notepad {
    fn default() -> Self {
        Self {
            text: String::new(),
            font_size: default_font_size(),
        }
    }
}

impl Notepad {
    /// The smallest the text on the page may be.
    pub const MIN_FONT_SIZE: u8 = 60;

    /// The largest the text on the page may be.
    pub const MAX_FONT_SIZE: u8 = 250;

    /// How much one press of the size buttons changes it.
    pub const FONT_SIZE_STEP: u8 = 10;

    /// Whether the page is untouched, which is what keeps it out of the file.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.font_size == default_font_size()
    }

    /// The size of the text on the page, as a multiplier.
    ///
    /// It is clamped, so that a hand-edited config cannot ask for text that
    /// will not fit on the screen.
    pub fn font_scale(&self) -> f32 {
        let size = self
            .font_size
            .clamp(Self::MIN_FONT_SIZE, Self::MAX_FONT_SIZE);

        f32::from(size) / 100.0
    }
}

/// The size the notepad's text is drawn at unless it is changed.
fn default_font_size() -> u8 {
    100
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Window {
    pub width: f32,
    pub height: f32,
    pub always_on_top: bool,
    /// Width with the time-block editor open, independent of the normal window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_block_width: Option<f32>,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            width: 260.0,
            height: 340.0,
            always_on_top: false,
            time_block_width: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TimerSettings {
    /// The target time the timer is armed with on startup, in seconds.
    /// 0 means no target, i.e. a plain count-up.
    pub last_duration_secs: u64,
    /// Preset chips, in seconds.
    pub presets_secs: Vec<u64>,
}

impl Default for TimerSettings {
    fn default() -> Self {
        Self {
            last_duration_secs: 300,
            presets_secs: vec![60, 300, 600, 1500, 3000],
        }
    }
}

impl TimerSettings {
    /// The armed target, or `None` when 0 means a plain count-up.
    pub fn last_duration(&self) -> Option<Duration> {
        (self.last_duration_secs > 0).then(|| Duration::from_secs(self.last_duration_secs))
    }

    pub fn presets(&self) -> impl Iterator<Item = Duration> + '_ {
        self.presets_secs.iter().copied().map(Duration::from_secs)
    }
}

/// The fold/// The optional folder whose immediate subdirectories appear in the Projects tab.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectsSettings {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<PathBuf>,
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
    /// When false, Google Tasks are hidden and only calendar events show.
    pub show_tasks: bool,
    pub hide_overdue_tasks: bool,
    pub time_block_calendar: Option<String>,
    pub time_block_list: Option<String>,
    /// Google Tasks list for tasks moved to the backlog. None uses Not Time Blocked.
    pub backlog_list: Option<String>,
    /// Calendar for individual scheduled task events; independent of container blocks.
    pub tasks_calendar: Option<String>,
    /// The hour the day ends, 0-5. 0 is midnight, the usual boundary. Set it
    /// to 3 and the day runs 3am to 3am, so the small hours stay with the
    /// evening they belong to. This only moves the view: an event keeps its
    /// real date and time, it is just drawn at the foot of the day it belongs
    /// to.
    pub day_end_hour: u32,
    /// Custom colors for Google Tasks lists, keyed by the calendar id
    /// (`tasks:<list>`). Google hands every list back with the same blue, so
    /// this is where a list gets a color of its own. A missing key means the
    /// shared task blue.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub task_colors: BTreeMap<String, HexColor>,
    /// The calendar new events are created on, by id. `None` means the primary
    /// calendar, falling back to the first writable one. Set from the calendar
    /// settings pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_calendar: Option<String>,
}

impl Default for CalendarSettings {
    fn default() -> Self {
        Self {
            sync_minutes: 15,
            client_id: String::new(),
            client_secret: String::new(),
            disabled: Vec::new(),
            show_tasks: true,
            hide_overdue_tasks: false,
            time_block_calendar: None,
            time_block_list: None,
            backlog_list: None,
            tasks_calendar: None,
            day_end_hour: 0,
            task_colors: BTreeMap::new(),
            default_calendar: None,
        }
    }
}

impl CalendarSettings {
    pub const MIN_SYNC_MINUTES: u32 = 1;
    pub const MAX_SYNC_MINUTES: u32 = 120;
    /// The latest hour a day can end: 5am, which is as far into the morning
    /// as a late night is worth pushing the boundary. Midnight (0) is the
    /// default.
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
        self.task_colors.get(calendar_id).map(|color| color.0)
    }

    /// Sets a task list's custom color, or clears it back to Google's blue.
    pub fn set_task_color(&mut self, calendar_id: &str, color: Option<Color>) {
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
}

/// Everything that happens when the timer reaches zero.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AlarmProfile {
    pub notification: Notification,
    pub sound: Sound,
    pub flash: Flash,
    /// Bring the window to the front when the timer fires.
    pub raise_window: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notification {
    pub enabled: bool,
    pub summary: String,
    pub body: String,
    pub urgency: Urgency,
    /// Seconds to keep the notification up; 0 leaves it to the daemon.
    pub timeout_secs: u32,
}

impl Default for Notification {
    fn default() -> Self {
        Self {
            enabled: true,
            summary: "Time's up".to_owned(),
            body: String::new(),
            urgency: Urgency::Critical,
            timeout_secs: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

impl Urgency {
    pub const ALL: [Self; 3] = [Self::Low, Self::Normal, Self::Critical];
}

impl std::fmt::Display for Urgency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Low => "Low",
            Self::Normal => "Normal",
            Self::Critical => "Critical",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sound {
    pub enabled: bool,
    pub choice: SoundChoice,
    /// 0-100, as shown on the slider.
    pub volume: u8,
    /// How many times to play the sound; 0 loops until dismissed.
    pub repeats: u8,
    pub fade_in_ms: u64,
}

impl Default for Sound {
    fn default() -> Self {
        Self {
            enabled: true,
            choice: SoundChoice::default(),
            volume: 70,
            repeats: 2,
            fade_in_ms: 150,
        }
    }
}

impl Sound {
    /// The slider position as a linear amplitude multiplier.
    ///
    /// Squared because loudness is perceived roughly logarithmically: a linear
    /// mapping makes the bottom half of the slider do almost nothing.
    pub fn gain(&self) -> f32 {
        let fraction = f32::from(self.volume.min(100)) / 100.0;
        fraction * fraction
    }

    pub fn fade_in(&self) -> Duration {
        Duration::from_millis(self.fade_in_ms)
    }
}

/// Which sound to play.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SoundChoice {
    /// One of the tones we synthesize ourselves.
    Builtin(Tone),
    /// Any audio file the user picked.
    Custom { file: PathBuf },
}

impl Default for SoundChoice {
    fn default() -> Self {
        Self::Builtin(Tone::default())
    }
}

impl std::fmt::Display for SoundChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Builtin(tone) => tone.fmt(f),
            Self::Custom { file } => {
                let name = file
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| file.display().to_string());

                write!(f, "{name}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Flash {
    pub enabled: bool,
    pub color: HexColor,
    /// The most opaque the overlay ever gets, 0.0 to 1.0.
    pub peak_opacity: f32,
    /// How many times to pulse.
    pub pulses: u8,
    /// Length of a single fade in and out.
    pub pulse_ms: u64,
    /// Let clicks through to whatever is underneath the flash.
    pub passthrough: bool,
    /// Pulse the app window during the final seconds. 0 disables it.
    pub final_countdown_secs: u8,
}

impl Default for Flash {
    fn default() -> Self {
        Self {
            enabled: true,
            color: HexColor(Color::from_rgb8(0xFF, 0x20, 0x20)),
            peak_opacity: 0.35,
            pulses: 2,
            pulse_ms: 700,
            passthrough: false,
            final_countdown_secs: 0,
        }
    }
}

impl Flash {
    /// The shortest pulse we are willing to draw.
    ///
    /// Fast full-screen flashing is a seizure risk, so the rate is capped here
    /// rather than trusted from the config file. Three per second is the
    /// commonly cited ceiling for general-audience content.
    pub const MIN_PULSE: Duration = Duration::from_millis(334);

    pub fn pulse_period(&self) -> Duration {
        Duration::from_millis(self.pulse_ms).max(Self::MIN_PULSE)
    }

    /// Total length of the flash, after clamping.
    pub fn total_duration(&self) -> Duration {
        self.pulse_period() * u32::from(self.pulses.max(1))
    }

    pub fn peak(&self) -> f32 {
        self.peak_opacity.clamp(0.0, 1.0)
    }

    pub fn final_countdown(&self) -> Option<Duration> {
        (self.final_countdown_secs > 0)
            .then(|| Duration::from_secs(u64::from(self.final_countdown_secs)))
    }
}

/// Where the config and user themes live.
pub fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("dev", "TimeTracker", "timetracker")
}

pub fn config_path() -> Option<PathBuf> {
    Some(project_dirs()?.config_dir().join("config.toml"))
}

/// The directory scanned for user-written theme files.
pub fn themes_dir() -> Option<PathBuf> {
    Some(project_dirs()?.config_dir().join("themes"))
}

/// Where cached days live: `$XDG_CACHE_HOME/timetracker/days`, which is
/// `~/.cache/timetracker/days` unless the environment says otherwise. It is
/// a cache rather than state: losing it only costs a re-sync.
pub fn cache_days_dir() -> Option<PathBuf> {
    Some(project_dirs()?.cache_dir().join("days"))
}

/// Last successful Keep snapshot, including original card previews.
pub fn cache_keep_path() -> Option<PathBuf> {
    Some(project_dirs()?.cache_dir().join("keep").join("notes.json"))
}

impl Config {
    /// Reads the config, falling back to defaults for anything unreadable.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            log::warn!("no config directory available; using defaults");
            return Self::default();
        };

        match fs::read_to_string(&path) {
            Ok(contents) => match Self::parse(&contents) {
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

    /// Accept fields written by other app versions, reporting their paths.
    fn parse(contents: &str) -> Result<Self, toml::de::Error> {
        Self::parse_with_ignored(contents, |path| {
            log::warn!("ignoring unknown config field: {path}");
        })
    }

    fn parse_with_ignored(
        contents: &str,
        mut ignored: impl FnMut(String),
    ) -> Result<Self, toml::de::Error> {
        serde_ignored::deserialize(toml::de::Deserializer::parse(contents)?, |path| {
            ignored(path.to_string());
        })
    }

    /// Unlike startup, a failed live read must keep the current settings.
    fn read_from(path: &Path) -> io::Result<Self> {
        let contents = fs::read_to_string(path)?;
        Self::parse(&contents).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    pub fn save(&self) -> io::Result<()> {
        let path = config_path().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no config directory available")
        })?;

        let contents = toml::to_string_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

        write_atomically(&path, contents.as_bytes())
    }
}

/// Watch the directory, not the inode: editors and our own saves replace the
/// file by renaming a temporary file. The native watcher does no idle polling.
pub fn watch() -> iced::Subscription<Config> {
    iced::Subscription::run(config_changes)
}

fn config_changes() -> impl iced::futures::Stream<Item = Config> {
    config_changes_at(config_path())
}

fn config_changes_at(path: Option<PathBuf>) -> impl iced::futures::Stream<Item = Config> {
    iced::stream::channel(16, async move |mut output| {
        use iced::futures::{SinkExt, StreamExt};
        let Some(path) = path else { return };
        let Some(parent) = path.parent() else { return };
        let (sender, mut receiver) = iced::futures::channel::mpsc::unbounded();
        let (events, event_rx) = std::sync::mpsc::channel();
        let watched_path = path.clone();
        let mut watcher =
            match notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                match event {
                    Ok(event)
                        if event.paths.iter().any(|path| path == &watched_path)
                            && matches!(
                                event.kind,
                                notify::EventKind::Create(_)
                                    | notify::EventKind::Modify(_)
                                    | notify::EventKind::Remove(_)
                            ) =>
                    {
                        let _ = events.send(());
                    }
                    Err(error) => log::warn!("config watcher error: {error}"),
                    _ => {}
                }
            }) {
                Ok(watcher) => watcher,
                Err(error) => {
                    log::warn!("could not create config watcher: {error}");
                    return;
                }
            };
        if let Err(error) = fs::create_dir_all(parent).and_then(|()| {
            notify::Watcher::watch(&mut watcher, parent, notify::RecursiveMode::NonRecursive)
                .map_err(io::Error::other)
        }) {
            log::warn!("could not watch {}: {error}", parent.display());
            return;
        }
        let reload_path = path.clone();
        let worker = std::thread::Builder::new()
            .name("timetracker-config-watch".into())
            .spawn(move || {
                while event_rx.recv().is_ok() {
                    // Wait for the write burst to settle: a truncate-and-write
                    // editor can otherwise briefly look like a valid empty config.
                    loop {
                        match event_rx.recv_timeout(Duration::from_millis(150)) {
                            Ok(()) => continue,
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    match Config::read_from(&reload_path) {
                        Ok(config) => {
                            if sender.unbounded_send(config).is_err() {
                                return;
                            }
                        }
                        Err(error) => log::warn!(
                            "could not reload {}; keeping current settings: {error}",
                            reload_path.display()
                        ),
                    }
                }
            });
        if let Err(error) = worker {
            log::warn!("could not start config reload worker: {error}");
            return;
        }
        // Close the gap between startup's read and installing the watcher.
        if let Ok(config) = Config::read_from(&path) {
            let _ = output.send(config).await;
        }
        while let Some(config) = receiver.next().await {
            if output.send(config).await.is_err() {
                break;
            }
        }
        drop(watcher);
    })
}

/// Writes through a temporary file so a crash mid-write cannot truncate the
/// existing config.
fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, contents)?;
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_fields_warn_without_discarding_known_settings() {
        let mut ignored = Vec::new();
        let config = Config::parse_with_ignored(
            r#"
            future_setting = true
            [appearance]
            theme = "Ocean"
            future_theme_setting = 42
            [alarm.sound]
            volume = 25
            future_sound_setting = "anything"
            [[todos.items]]
            text = "keep me"
            future_item_setting = true
            "#,
            |path| ignored.push(path),
        )
        .unwrap();
        assert_eq!(config.appearance.theme, "Ocean");
        assert_eq!(config.alarm.sound.volume, 25);
        assert_eq!(config.todos.items()[0].text, "keep me");
        assert_eq!(ignored.len(), 4, "{ignored:?}");
        assert!(ignored.contains(&"future_setting".to_owned()));
        assert!(ignored.contains(&"appearance.future_theme_setting".to_owned()));
        assert!(ignored.contains(&"alarm.sound.future_sound_setting".to_owned()));
    }

    #[test]
    fn watcher_survives_atomic_replacements_and_invalid_edits() {
        use iced::futures::StreamExt;
        let directory = std::env::temp_dir().join(format!(
            "timetracker-config-watch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.toml");
        fs::write(&path, "[alarm.sound]\nvolume = 25").unwrap();
        let watched_path = path.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            iced::futures::executor::block_on(async move {
                let changes = config_changes_at(Some(watched_path));
                iced::futures::pin_mut!(changes);
                // Repeated native events are allowed; forward all snapshots.
                while let Some(config) = changes.next().await {
                    let done = config.alarm.sound.volume == 60;
                    if sender.send(config).is_err() || done {
                        break;
                    }
                }
            });
        });
        let wait_for_volume = |volume| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let config = receiver
                    .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .unwrap();
                if config.alarm.sound.volume == volume {
                    break;
                }
            }
        };
        wait_for_volume(25); // Initial read means the watcher is installed.
        write_atomically(&path, b"[alarm.sound]\nvolume = 40").unwrap();
        wait_for_volume(40);
        fs::write(&path, "[alarm.sound]\nvolume = ").unwrap();
        assert!(Config::read_from(&path).is_err());
        // Allow the native callback to read the invalid edit before replacing it.
        let deadline = std::time::Instant::now() + Duration::from_millis(300);
        while let Ok(config) =
            receiver.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
        {
            assert_eq!(
                config.alarm.sound.volume, 40,
                "invalid edits must not emit defaults"
            );
        }
        write_atomically(&path, b"[alarm.sound]\nvolume = 60").unwrap();
        wait_for_volume(60);
        worker.join().unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn defaults_round_trip_through_toml() {
        let original = Config::default();
        let text = toml::to_string_pretty(&original).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();

        assert_eq!(original, parsed);
    }

    #[test]
    fn a_partial_config_keeps_defaults_for_everything_else() {
        let parsed: Config = toml::from_str(
            r#"
            [alarm.sound]
            volume = 25
            "#,
        )
        .unwrap();

        assert_eq!(parsed.alarm.sound.volume, 25);
        assert_eq!(parsed.alarm.sound.repeats, Sound::default().repeats);
        assert_eq!(parsed.appearance, Appearance::default());
    }

    #[test]
    fn builtin_and_custom_sounds_both_parse() {
        let builtin: Config = toml::from_str("[alarm.sound]\nchoice = \"bell\"").unwrap();
        assert_eq!(builtin.alarm.sound.choice, SoundChoice::Builtin(Tone::Bell));

        let custom: Config =
            toml::from_str("[alarm.sound.choice]\nfile = \"/tmp/alarm.ogg\"").unwrap();
        assert_eq!(
            custom.alarm.sound.choice,
            SoundChoice::Custom {
                file: PathBuf::from("/tmp/alarm.ogg")
            }
        );
    }

    #[test]
    fn flash_rate_is_capped_regardless_of_config() {
        let flash = Flash {
            pulse_ms: 10,
            ..Flash::default()
        };

        assert_eq!(flash.pulse_period(), Flash::MIN_PULSE);
    }

    #[test]
    fn calendar_sync_defaults_to_fifteen_minutes() {
        let parsed: Config = toml::from_str("").unwrap();
        assert_eq!(parsed.calendar.sync_minutes, 15);
        assert_eq!(parsed.calendar.sync_every(), Duration::from_secs(15 * 60));
    }

    #[test]
    fn cached_days_live_under_the_xdg_cache_directory() {
        let days = cache_days_dir().expect("a cache directory");
        assert!(
            days.ends_with(Path::new("timetracker").join("days")),
            "{} should sit under the XDG cache directory",
            days.display()
        );
    }

    #[test]
    fn the_day_ends_at_midnight_by_default() {
        let parsed: Config = toml::from_str("").unwrap();
        assert_eq!(parsed.calendar.day_end_hour, 0);
        assert_eq!(parsed.calendar.day_start_hour(), 0);
        assert_eq!(parsed.calendar.day_end_minutes(), 0);
    }

    #[test]
    fn an_out_of_range_day_end_hour_is_clamped() {
        let settings = CalendarSettings {
            day_end_hour: 99,
            ..CalendarSettings::default()
        };

        assert_eq!(
            settings.day_start_hour(),
            CalendarSettings::MAX_DAY_END_HOUR
        );
        assert_eq!(settings.day_end_minutes(), 5 * 60);
    }

    #[test]
    fn a_custom_task_color_survives_a_round_trip() {
        let mut settings = CalendarSettings::default();
        let red = Color::from_rgb8(0xDC, 0x21, 0x27);
        settings.set_task_color("tasks:list", Some(red));

        let text = toml::to_string_pretty(&settings).unwrap();
        let parsed: CalendarSettings = toml::from_str(&text).unwrap();

        assert_eq!(parsed.task_color("tasks:list"), Some(red));
        assert_eq!(parsed.task_color("tasks:other"), None);
    }

    #[test]
    fn clearing_a_task_color_forgets_it() {
        let mut settings = CalendarSettings::default();
        settings.set_task_color("tasks:list", Some(Color::WHITE));
        settings.set_task_color("tasks:list", None);

        assert_eq!(settings.task_color("tasks:list"), None);
        assert!(settings.task_colors.is_empty());
    }

    #[test]
    fn an_empty_task_color_map_is_left_out_of_the_config_file() {
        let text = toml::to_string_pretty(&CalendarSettings::default()).unwrap();
        assert!(!text.contains("task_colors"), "got {text:?}");
    }

    #[test]
    fn a_todo_scratchpad_survives_a_round_trip() {
        let mut config = Config::default();
        config.todos.add("write the report");
        config.todos.add("send the email");
        config.todos.set_current(1);
        config.todos.indent(1, 1);

        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();

        assert_eq!(parsed.todos, config.todos);
        assert_eq!(parsed.todos.current().unwrap().text, "send the email");
        assert_eq!(parsed.todos.items()[1].indent, 1);
    }

    #[test]
    fn an_empty_scratchpad_is_left_out_of_the_config_file() {
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        assert!(!text.contains("todos"), "got {text:?}");
    }

    #[test]
    fn a_notepad_survives_a_round_trip() {
        let mut config = Config::default();
        config.notepad.text = "remember the milk\nand the bread".to_owned();
        config.notepad.font_size = 150;

        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();

        assert_eq!(parsed.notepad.text, config.notepad.text);
        assert_eq!(parsed.notepad.font_size, 150);
    }

    #[test]
    fn a_blank_notepad_is_left_out_of_the_config_file() {
        let text = toml::to_string_pretty(&Config::default()).unwrap();
        assert!(!text.contains("notepad"), "got {text:?}");
    }

    #[test]
    fn a_notepad_with_a_size_of_its_own_is_kept() {
        let mut config = Config::default();
        config.notepad.font_size = 150;

        let text = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();

        assert_eq!(parsed.notepad.font_size, 150);
    }

    #[test]
    fn the_notepad_text_size_is_clamped_to_what_fits() {
        let tiny = Notepad {
            font_size: 10,
            ..Notepad::default()
        };
        let huge = Notepad {
            font_size: 250,
            ..Notepad::default()
        };

        assert_eq!(tiny.font_scale(), f32::from(Notepad::MIN_FONT_SIZE) / 100.0);
        assert_eq!(huge.font_scale(), f32::from(Notepad::MAX_FONT_SIZE) / 100.0);
    }

    #[test]
    fn toggling_a_calendar_off_and_on_round_trips() {
        let mut settings = CalendarSettings::default();
        settings.set_calendar_enabled("work", false);
        assert_eq!(settings.disabled, vec!["work".to_owned()]);
        settings.set_calendar_enabled("work", false);
        assert_eq!(settings.disabled.len(), 1);
        settings.set_calendar_enabled("work", true);
        assert!(settings.disabled.is_empty());
    }

    #[test]
    fn volume_curve_spans_silence_to_unity() {
        let silent = Sound {
            volume: 0,
            ..Sound::default()
        };
        let loud = Sound {
            volume: 100,
            ..Sound::default()
        };
        let over = Sound {
            volume: 250,
            ..Sound::default()
        };

        assert_eq!(silent.gain(), 0.0);
        assert_eq!(loud.gain(), 1.0);
        assert_eq!(over.gain(), 1.0);
    }
}
