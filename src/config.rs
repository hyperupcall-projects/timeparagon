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

/// How long to wait after the last change before writing to disk.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub appearance: Appearance,
    pub window: Window,
    pub timer: TimerSettings,
    pub alarm: AlarmProfile,
    pub calendar: CalendarSettings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            appearance: Appearance::default(),
            window: Window::default(),
            timer: TimerSettings::default(),
            alarm: AlarmProfile::default(),
            calendar: CalendarSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Window {
    pub width: f32,
    pub height: f32,
    pub always_on_top: bool,
}

impl Default for Window {
    fn default() -> Self {
        Self {
            width: 260.0,
            height: 340.0,
            always_on_top: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
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

/// Google Calendar connection and which calendars are shown on the Cal tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
    /// Custom colors for Google Tasks lists, keyed by the calendar id
    /// (`tasks:<list>`). Google hands every list back with the same blue, so
    /// this is where a list gets a color of its own. A missing key means the
    /// shared task blue.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub task_colors: BTreeMap<String, HexColor>,
}

impl Default for CalendarSettings {
    fn default() -> Self {
        Self {
            sync_minutes: 15,
            client_id: String::new(),
            client_secret: String::new(),
            disabled: Vec::new(),
            show_tasks: true,
            task_colors: BTreeMap::new(),
        }
    }
}

impl CalendarSettings {
    pub const MIN_SYNC_MINUTES: u32 = 1;
    pub const MAX_SYNC_MINUTES: u32 = 120;

    pub fn sync_every(&self) -> Duration {
        let minutes = self.sync_minutes.clamp(Self::MIN_SYNC_MINUTES, Self::MAX_SYNC_MINUTES);
        Duration::from_secs(u64::from(minutes) * 60)
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
#[serde(default, deny_unknown_fields)]
pub struct AlarmProfile {
    pub notification: Notification,
    pub sound: Sound,
    pub flash: Flash,
    /// Bring the window to the front when the timer fires.
    pub raise_window: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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
#[serde(default, deny_unknown_fields)]
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

    pub fn save(&self) -> io::Result<()> {
        let path = config_path().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no config directory available")
        })?;

        let contents = toml::to_string_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

        write_atomically(&path, contents.as_bytes())
    }
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
