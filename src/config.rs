//! Persisted settings.
//!
//! A malformed config must never stop the app from starting, so every read
//! falls back to defaults. It must not be lost to those defaults either, so a
//! save refuses to replace a file it cannot parse. Every write goes through a
//! temporary file that is renamed into place.

use crate::calendar::time_format::TimeFormat;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use iced::Color;
use serde::{Deserialize, Serialize};

use crate::audio::Tone;
use crate::hex::HexColor;
use crate::todos::TodoList;

pub use timeslay_core::config::{
    CalendarSettings, Project, ProjectsSettings, TriageRequirements, cache_days_dir, config_path,
    project_dirs,
};
pub use timeslay_core::places::Place;

/// How long to wait after the last change before writing to disk.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct Config {
    pub voice: crate::voice::Settings,
    pub appearance: Appearance,
    pub window: Window,
    pub views: ViewSettings,
    /// The side panel: its edge, width and global shortcut.
    pub panel: crate::panel::PanelConfig,
    pub alarm: AlarmProfile,
    pub calendar: CalendarSettings,
    pub projects: ProjectsSettings,
    /// Places and travel. It is left out of the file until something is set,
    /// so a config that never used it stays as it was.
    #[serde(default, skip_serializing_if = "TravelSettings::is_empty")]
    pub travel: TravelSettings,
    #[serde(default, skip_serializing_if = "crate::task_work::WorkLog::is_empty")]
    pub task_work: crate::task_work::WorkLog,
    pub pomodoro: crate::pomodoro::Pomodoro,
    /// The temporary todo scratchpad. It is left out of the file while it is
    /// empty, so a config that never used it stays as it was.
    #[serde(default, skip_serializing_if = "TodoList::is_empty")]
    pub todos: TodoList,
    /// The notepad's page. It is left out of the file while it is blank, so a
    /// config that never used it stays as it was.
    #[serde(default, skip_serializing_if = "Notepad::is_empty")]
    pub notepad: Notepad,
    /// One brain dump page per day. It is left out of the file until the first
    /// page is written.
    #[serde(default, skip_serializing_if = "crate::brain_dump::Dumps::is_empty")]
    pub brain_dump: crate::brain_dump::Dumps,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    /// The name of the selected theme, matching what the picker shows. It may
    /// be one of the catalog's names or one of the follow-the-system choices.
    pub theme: String,
    /// Legacy setting from when following the system was a toggle. Old configs
    /// still carry it, so it is read and migrated to [`crate::theme::SYSTEM`]
    /// when the config is parsed, then dropped on the next save.
    #[serde(default, skip_serializing)]
    pub follow_system: bool,
    /// Display format for clock times throughout the application.
    #[serde(alias = "use_24_hour")]
    pub time_format: TimeFormat,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: "Light".to_owned(),
            follow_system: false,
            time_format: TimeFormat::TwentyFourHour,
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
    /// Legacy combined window width. Retained for config compatibility; floating
    /// editors use `views.time_block_width` instead.
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

/// Default pane sizes, in logical pixels, shared by the Iced windows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewSettings {
    /// None preserves the saved main-window width and the `[panel]` width.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_width: Option<f32>,
    /// Independent total widths for the week and month calendars.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_week_width: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_month_width: Option<f32>,
    /// Width added alongside the calendar when editing a time block.
    pub time_block_width: f32,
    /// Width of the shared event/habit/task details pane.
    pub detail_width: f32,
    /// Visible inset from the side panel's screen edge, top and bottom.
    pub margin: f32,
    /// Radius of the application's visible outer corners, in logical pixels.
    pub border_radius: f32,
    pub dock_border_radius: f32,
    pub panel_border_radius: Option<f32>,
    /// Width of the dock strip, in logical pixels.
    pub dock_width: f32,
    /// Reserve the visible dock strip in the compositor work area.
    pub dock_reserve_space: bool,
    /// Reserve the side panel independently of the dock.
    pub panel_reserve_space: bool,
    /// Space between the dock and the left and right screen edges, in logical
    /// pixels. A top or bottom dock uses it at both ends; a side dock uses it
    /// as the gap from the edge it hugs.
    pub dock_margin: f32,
    /// Independent horizontal gaps; absent values inherit the legacy dock margin.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dock_left_margin: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dock_right_margin: Option<f32>,
    /// Which screen edge the dock hugs.
    pub dock_side: DockSide,
    /// Thickness of the dock's outer border, in logical pixels. Zero hides it.
    pub dock_border_outer_width: f32,
    /// Colour of the dock's outer border.
    pub dock_border_outer_color: HexColor,
    /// Thickness of the dock's inner border, in logical pixels. Zero hides it.
    pub dock_border_inner_width: f32,
    /// Colour of the dock's inner border.
    pub dock_border_inner_color: HexColor,
    /// Shared outer border depth for the dock and application windows.
    /// The panel-prefixed keys are retained for saved-config compatibility.
    pub panel_border_outer_width: f32,
    /// Shared outer border color.
    pub panel_border_outer_color: HexColor,
    /// Shared inner border depth. Zero hides it.
    pub panel_border_inner_width: f32,
    /// Shared inner border color.
    pub panel_border_inner_color: HexColor,
    /// How the dock appears on screen.
    pub window_animation: WindowAnimation,
    /// Length of the fade-in, in milliseconds.
    pub fade_duration_ms: u32,
    /// Whether the dock draws a faint gray line at each hour on its strip.
    pub dock_hour_lines: bool,
}

/// Which screen edge the dock strip hugs.
///
/// This is separate from the side panel's [`crate::panel::Side`], which only
/// has a left and a right edge: the dock can also run along the top or bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DockSide {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

impl DockSide {
    pub const ALL: [Self; 4] = [Self::Left, Self::Right, Self::Top, Self::Bottom];

    /// Whether the strip runs up and down the screen, with the day laid out
    /// from top to bottom.
    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }
}

impl std::fmt::Display for DockSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Top => "Top",
            Self::Bottom => "Bottom",
        })
    }
}

/// How a window appears. Shared by the dock's layer surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowAnimation {
    /// Whatever the desktop does for a new window, such as KWin's pop-in.
    #[default]
    DesktopDefault,
    /// Fades from transparent to opaque over the configured duration.
    FadeIn,
    /// Appears at once, with no desktop effect.
    Instant,
}

impl WindowAnimation {
    pub const ALL: [Self; 3] = [Self::DesktopDefault, Self::FadeIn, Self::Instant];

    /// The layer-shell namespace to ask for. KWin maps "dock" to its Dock
    /// window type, which its Scale, Fade and Fading Popups effects skip, so
    /// the desktop's own pop-in can't fight ours.
    pub fn namespace(self, desktop_default: &'static str) -> &'static str {
        match self {
            Self::DesktopDefault => desktop_default,
            Self::FadeIn | Self::Instant => "dock",
        }
    }
}

impl std::fmt::Display for WindowAnimation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DesktopDefault => "Desktop default",
            Self::FadeIn => "Fade in",
            Self::Instant => "Instant",
        })
    }
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            calendar_width: None,
            calendar_week_width: None,
            calendar_month_width: None,
            time_block_width: 400.0,
            detail_width: 360.0,
            margin: 8.0,
            border_radius: 4.0,
            dock_border_radius: 0.0,
            panel_border_radius: None,
            dock_width: 10.0,
            dock_margin: 0.0,
            dock_left_margin: None,
            dock_right_margin: None,
            dock_side: DockSide::Left,
            dock_border_outer_width: 1.0,
            dock_border_outer_color: HexColor(Color::WHITE),
            dock_border_inner_width: 1.0,
            dock_border_inner_color: HexColor(Color::BLACK),
            panel_border_outer_width: 1.0,
            panel_border_outer_color: HexColor(Color::WHITE),
            panel_border_inner_width: 1.0,
            panel_border_inner_color: HexColor(Color::BLACK),
            window_animation: WindowAnimation::DesktopDefault,
            fade_duration_ms: 150,
            dock_hour_lines: true,
            dock_reserve_space: false,
            panel_reserve_space: false,
        }
    }
}

impl ViewSettings {
    pub const MIN_WIDTH: f32 = 180.0;
    pub const MAX_WIDTH: f32 = 3840.0;
    pub const MAX_MARGIN: f32 = 30.0;
    pub const MAX_BORDER_RADIUS: f32 = 30.0;
    pub const MIN_DOCK_WIDTH: f32 = 2.0;
    pub const MAX_DOCK_WIDTH: f32 = 40.0;
    pub const MAX_DOCK_MARGIN: f32 = 100.0;
    pub const MAX_BORDER_WIDTH: f32 = 5.0;
    pub const MAX_FADE_DURATION_MS: u32 = 5000;

    pub fn fade_duration_ms(&self) -> u32 {
        self.fade_duration_ms.min(Self::MAX_FADE_DURATION_MS)
    }

    pub fn calendar_week_width(&self, fallback: f32) -> f32 {
        Self::width(self.calendar_week_width.unwrap_or(fallback), fallback)
    }

    pub fn calendar_month_width(&self) -> f32 {
        Self::width(self.calendar_month_width.unwrap_or(1050.0), 1050.0)
    }

    pub fn calendar_width(&self, fallback: f32) -> f32 {
        Self::width(self.calendar_width.unwrap_or(fallback), fallback)
    }

    pub fn time_block_width(&self) -> f32 {
        Self::width(self.time_block_width, 400.0)
    }

    pub fn detail_width(&self) -> f32 {
        Self::width(self.detail_width, 360.0)
    }

    pub fn margin(&self) -> f32 {
        if self.margin.is_finite() {
            self.margin.clamp(0.0, Self::MAX_MARGIN)
        } else {
            Self::default().margin
        }
    }

    pub fn dock_border_radius(&self) -> f32 {
        if self.dock_border_radius.is_finite() {
            self.dock_border_radius.clamp(0.0, Self::MAX_BORDER_RADIUS)
        } else {
            0.0
        }
    }
    pub fn panel_border_radius(&self) -> f32 {
        self.panel_border_radius
            .filter(|r| r.is_finite())
            .unwrap_or(self.border_radius())
            .clamp(0.0, Self::MAX_BORDER_RADIUS)
    }

    pub fn border_radius(&self) -> f32 {
        if self.border_radius.is_finite() {
            self.border_radius.clamp(0.0, Self::MAX_BORDER_RADIUS)
        } else {
            Self::default().border_radius
        }
    }

    pub fn dock_width(&self) -> f32 {
        if self.dock_width.is_finite() {
            self.dock_width
                .clamp(Self::MIN_DOCK_WIDTH, Self::MAX_DOCK_WIDTH)
        } else {
            Self::default().dock_width
        }
    }

    /// The gap between the dock and the left and right screen edges.
    pub fn dock_margin(&self) -> f32 {
        if self.dock_margin.is_finite() {
            self.dock_margin.clamp(0.0, Self::MAX_DOCK_MARGIN)
        } else {
            Self::default().dock_margin
        }
    }

    pub fn dock_left_margin(&self) -> f32 {
        self.dock_left_margin.filter(|v| v.is_finite()).map_or_else(
            || self.dock_margin(),
            |v| v.clamp(0.0, Self::MAX_DOCK_MARGIN),
        )
    }

    pub fn dock_right_margin(&self) -> f32 {
        self.dock_right_margin
            .filter(|v| v.is_finite())
            .map_or_else(
                || self.dock_margin(),
                |v| v.clamp(0.0, Self::MAX_DOCK_MARGIN),
            )
    }

    /// A border thickness, clamped to the range the settings offer.
    pub fn dock_border_outer_width(&self) -> f32 {
        Self::border_width(self.dock_border_outer_width)
    }

    pub fn dock_border_inner_width(&self) -> f32 {
        Self::border_width(self.dock_border_inner_width)
    }

    pub fn panel_border_outer_width(&self) -> f32 {
        Self::border_width(self.panel_border_outer_width)
    }

    pub fn panel_border_inner_width(&self) -> f32 {
        Self::border_width(self.panel_border_inner_width)
    }

    fn border_width(value: f32) -> f32 {
        if value.is_finite() {
            value.clamp(0.0, Self::MAX_BORDER_WIDTH)
        } else {
            Self::default().dock_border_outer_width
        }
    }

    fn width(value: f32, fallback: f32) -> f32 {
        if value.is_finite() {
            value.clamp(Self::MIN_WIDTH, Self::MAX_WIDTH)
        } else {
            fallback.clamp(Self::MIN_WIDTH, Self::MAX_WIDTH)
        }
    }
}

/// How a trip between two places is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TravelMode {
    #[default]
    Walk,
    Bike,
    /// Configs written when public transport was offered read as `Car`:
    /// Mapbox has no transit routing.
    #[serde(alias = "multimodal")]
    Car,
}

impl TravelMode {
    pub const ALL: [TravelMode; 3] = [Self::Walk, Self::Bike, Self::Car];

    pub fn label(self) -> &'static str {
        match self {
            Self::Walk => "Walk",
            Self::Bike => "Bike",
            Self::Car => "Car",
        }
    }

    /// The profile the Mapbox Directions API expects.
    pub fn profile(self) -> &'static str {
        match self {
            Self::Walk => "walking",
            Self::Bike => "cycling",
            Self::Car => "driving",
        }
    }

    /// The `travelmode` of a Google Maps directions link.
    pub fn google_travelmode(self) -> &'static str {
        match self {
            Self::Walk => "walking",
            Self::Bike => "bicycling",
            Self::Car => "driving",
        }
    }

    /// The routing engine of an OpenStreetMap directions link.
    pub fn osm_engine(self) -> &'static str {
        match self {
            Self::Walk => "fossgis_osrm_foot",
            Self::Bike => "fossgis_osrm_bike",
            Self::Car => "fossgis_osrm_car",
        }
    }

    /// The mode after this one, wrapping around.
    pub fn next(self) -> Self {
        let index = Self::ALL.iter().position(|mode| *mode == self).unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }
}

impl std::fmt::Display for TravelMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// The map that "Open in Maps" shows a trip on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MapApp {
    #[default]
    OpenStreetMap,
    GoogleMaps,
}

impl MapApp {
    pub const ALL: [MapApp; 2] = [Self::OpenStreetMap, Self::GoogleMaps];

    pub fn label(self) -> &'static str {
        match self {
            Self::OpenStreetMap => "OpenStreetMap",
            Self::GoogleMaps => "Google Maps",
        }
    }
}

impl std::fmt::Display for MapApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// How the trip from one place to another is made, when that is not the default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteMode {
    pub from: u64,
    pub to: u64,
    pub mode: TravelMode,
}

/// Places, the Mapbox access token, and how trips between places are made.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct TravelSettings {
    /// A Mapbox access token (public, `pk.`) that may use the Search Box and
    /// Directions APIs.
    pub api_key: String,
    /// Home: the day's first and last trips, and the center for place search.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home: Option<u64>,
    /// How a trip is made until it is set otherwise.
    pub default_mode: TravelMode,
    /// The map "Open in Maps" opens a trip on.
    pub map_app: MapApp,
    /// Whether travel times are shown on the calendar. Toggled from the
    /// calendar menu beside the TimeSlay sources, and off until then.
    pub show: bool,
    pub places: Vec<Place>,
    /// Trips whose method was chosen by hand.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<RouteMode>,
    /// The place each calendar event, by id, was given.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub event_places: std::collections::BTreeMap<String, u64>,
}

// Read the old start/end settings once, then save only Home. Keep all saved
// places, including the former end location, available to tasks and events.
impl<'de> Deserialize<'de> for TravelSettings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Default, Deserialize)]
        #[serde(default)]
        struct Stored {
            api_key: String,
            home: Option<u64>,
            start: Option<u64>,
            end: Option<u64>,
            default_mode: TravelMode,
            map_app: MapApp,
            show: bool,
            places: Vec<Place>,
            modes: Vec<RouteMode>,
            event_places: std::collections::BTreeMap<String, u64>,
        }
        let stored = Stored::deserialize(deserializer)?;
        Ok(Self {
            api_key: stored.api_key,
            home: stored.home.or(stored.start).or(stored.end),
            default_mode: stored.default_mode,
            map_app: stored.map_app,
            show: stored.show,
            places: stored.places,
            modes: stored.modes,
            event_places: stored.event_places,
        })
    }
}

impl TravelSettings {
    /// Whether nothing has been set, which keeps the table out of the file.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    pub fn place(&self, id: u64) -> Option<&Place> {
        self.places.iter().find(|place| place.id == id)
    }

    /// The places in alphabetical order, ignoring case.
    pub fn sorted(&self) -> Vec<&Place> {
        let mut places: Vec<&Place> = self.places.iter().collect();
        places.sort_by_cached_key(|place| (place.name.to_lowercase(), place.id));
        places
    }

    /// The token to send to Mapbox: the one in settings, or else the
    /// `MAPBOX_ACCESS_TOKEN` environment variable.
    pub fn key(&self) -> Option<String> {
        let key = self.api_key.trim();
        if !key.is_empty() {
            return Some(key.to_owned());
        }
        std::env::var("MAPBOX_ACCESS_TOKEN")
            .ok()
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty())
    }

    /// Saves a place Mapbox found, unless it is saved already, and returns
    /// its id. Choosing the same place twice gives the same place back, with
    /// the title it has been renamed to.
    pub fn add_place(
        &mut self,
        name: &str,
        address: &str,
        place_id: &str,
        lat: f64,
        lng: f64,
    ) -> u64 {
        timeslay_core::places::add_place(&mut self.places, name, address, place_id, lat, lng)
    }

    /// How the trip from `from` to `to` is made.
    pub fn mode_for(&self, from: u64, to: u64) -> TravelMode {
        self.modes
            .iter()
            .find(|route| route.from == from && route.to == to)
            .map_or(self.default_mode, |route| route.mode)
    }

    pub fn set_mode(&mut self, from: u64, to: u64, mode: TravelMode) {
        match self
            .modes
            .iter_mut()
            .find(|route| route.from == from && route.to == to)
        {
            Some(route) => route.mode = mode,
            None => self.modes.push(RouteMode { from, to, mode }),
        }
    }
}

/// Everything that happens when an alarm goes off.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AlarmProfile {
    pub notification: Notification,
    pub sound: Sound,
    pub flash: Flash,
    /// Bring the window to the front when the alarm fires.
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
}

/// Last successful Keep snapshot, including original card previews.
pub fn cache_keep_path() -> Option<PathBuf> {
    Some(project_dirs()?.cache_dir().join("keep").join("notes.json"))
}

/// Every contact fetched from Google, and the directory entries seen in searches.
pub fn cache_people_path() -> Option<PathBuf> {
    Some(
        project_dirs()?
            .cache_dir()
            .join("people")
            .join("contacts.json"),
    )
}

/// Travel times already worked out, so a restart does not ask Mapbox again.
pub fn cache_travel_path() -> Option<PathBuf> {
    Some(
        project_dirs()?
            .cache_dir()
            .join("travel")
            .join("routes.json"),
    )
}

impl Config {
    /// Reads the config, falling back to defaults for anything unreadable.
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            log::warn!("no config directory available; using defaults");
            return Self::default();
        };
        // It holds the OAuth client secret and API keys, and older versions
        // wrote it readable by everyone.
        if let Err(error) = crate::fsutil::make_private(&path)
            && error.kind() != io::ErrorKind::NotFound
        {
            log::warn!("could not make {} private: {error}", path.display());
        }

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
        let mut config: Self =
            serde_ignored::deserialize(toml::de::Deserializer::parse(contents)?, |path| {
                ignored(path.to_string());
            })?;
        if config.appearance.follow_system {
            config.appearance.theme = crate::theme::SYSTEM.to_owned();
            config.appearance.follow_system = false;
        }
        Ok(config)
    }

    /// Unlike startup, a failed live read must keep the current settings.
    fn read_from(path: &Path) -> io::Result<Self> {
        let contents = fs::read_to_string(path)?;
        Self::parse(&contents).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// Why the config on disk cannot be used, if it cannot.
    pub fn load_problem() -> Option<String> {
        check_replaceable(&config_path()?)
            .err()
            .map(|error| error.to_string())
    }

    pub fn save(&self) -> io::Result<()> {
        let path = config_path().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no config directory available")
        })?;
        self.save_to(&path)
    }

    fn save_to(&self, path: &Path) -> io::Result<()> {
        let contents = toml::to_string_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

        crate::fsutil::locked(path, || {
            check_replaceable(path)?;
            write_private(path, contents.as_bytes())
        })
    }

    /// Changes the config on disk rather than this process's copy of it, with
    /// other writers held off between the read and the write.
    pub fn update_on_disk(change: impl FnOnce(&mut Self)) -> io::Result<()> {
        let path = config_path().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no config directory available")
        })?;
        Self::update_at(&path, change)
    }

    fn update_at(path: &Path, change: impl FnOnce(&mut Self)) -> io::Result<()> {
        crate::fsutil::locked(path, || {
            let mut config = match Self::read_from(path) {
                Ok(config) => config,
                Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
                Err(error) => return Err(error),
            };
            change(&mut config);
            let contents = toml::to_string_pretty(&config)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            write_private(path, contents.as_bytes())
        })
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
            .name("timeslay-config-watch".into())
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

/// A config that does not parse may be one typo away from all of the user's
/// settings and notes, while what is in memory is only defaults. Keeping the
/// file also keeps it out of the Drive sync, which leaves unparseable files alone
/// but would treat a fresh save of the defaults as the newest config.
fn check_replaceable(path: &Path) -> io::Result<()> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(io::Error::new(
                error.kind(),
                format!("{} could not be read: {error}", path.display()),
            ));
        }
    };
    Config::parse_with_ignored(&contents, |_| {}).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{} has an error, so changes are not being saved. Fix or remove the file: {error}",
                path.display()
            ),
        )
    })?;
    Ok(())
}

fn write_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    crate::fsutil::write_atomic(path, contents, crate::fsutil::Kind::Private)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_save_never_replaces_a_config_it_cannot_parse() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[notepad]\ntext = \"precious\"\n[broken\n").unwrap();

        let error = Config::default().save_to(&path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("not being saved"), "{error}");
        assert!(fs::read_to_string(&path).unwrap().contains("precious"));
        assert!(check_replaceable(&path).is_err());

        fs::write(&path, "[notepad]\ntext = \"fixed\"\n").unwrap();
        Config::default().save_to(&path).unwrap();
        assert!(check_replaceable(&path).is_ok());
    }

    #[test]
    fn saving_makes_the_config_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o664)).unwrap();

        Config::default().save_to(&path).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn concurrent_updates_on_disk_do_not_lose_each_others_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..5 {
                        Config::update_at(&path, |config| {
                            config.notepad.text.push('x');
                        })
                        .unwrap();
                    }
                });
            }
        });
        assert_eq!(Config::read_from(&path).unwrap().notepad.text.len(), 20);
    }

    #[test]
    fn an_update_on_disk_refuses_a_broken_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[broken\n").unwrap();

        assert!(Config::update_at(&path, |_| {}).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "[broken\n");
    }

    use super::*;

    #[test]
    fn the_managed_task_lists_are_inbox_triaged_time_blocked_and_backlog() {
        assert_eq!(crate::inbox::DEFAULT_LIST, "Inbox");
        assert_eq!(CalendarSettings::DEFAULT_TRIAGED_LIST, "Triaged");
        assert_eq!(CalendarSettings::DEFAULT_TIME_BLOCK_LIST, "Time Blocked");
        assert_eq!(CalendarSettings::DEFAULT_BACKLOG_LIST, "Backlog");
        assert_eq!(
            TriageRequirements::default(),
            TriageRequirements {
                estimate: true,
                project: true,
                priority: true
            }
        );
    }

    #[test]
    fn the_legacy_follow_system_setting_becomes_the_system_theme() {
        let config = Config::parse("[appearance]\ntheme = \"Dark\"\nfollow_system = true").unwrap();

        assert_eq!(config.appearance.theme, crate::theme::SYSTEM);
        assert!(!config.appearance.follow_system);
        assert!(!toml::to_string(&config).unwrap().contains("follow_system"));
    }

    #[test]
    fn view_settings_keep_legacy_widths_and_round_trip_custom_sizes() {
        let config: Config = toml::from_str("[window]\nwidth = 320.0").unwrap();
        assert_eq!(config.views.calendar_width(config.window.width), 320.0);
        assert_eq!(config.views.time_block_width(), 400.0);
        assert_eq!(config.views.detail_width(), 360.0);
        assert_eq!(config.views.margin(), 8.0);

        let config: Config = toml::from_str(
            "[views]\ncalendar_width = 280.0\ntime_block_width = 360.0\ndetail_width = 320.0\nmargin = 12.0",
        )
        .unwrap();
        let restored: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(restored, config);
        assert_eq!(restored.views.calendar_width(200.0), 280.0);
        assert_eq!(restored.views.time_block_width(), 360.0);
        assert_eq!(restored.views.detail_width(), 320.0);
        assert_eq!(restored.views.margin(), 12.0);
    }

    #[test]
    fn hand_edited_view_sizes_cannot_break_the_layout() {
        let views: ViewSettings = toml::from_str(
            "calendar_width = nan\ntime_block_width = -1.0\ndetail_width = inf\nmargin = inf",
        )
        .unwrap();
        assert_eq!(views.calendar_width(260.0), 260.0);
        assert_eq!(views.time_block_width(), ViewSettings::MIN_WIDTH);
        assert_eq!(views.detail_width(), 360.0);
        assert_eq!(views.margin(), 8.0);
    }

    #[test]
    fn the_panel_settings_live_in_the_main_config() {
        let config: Config = toml::from_str("[panel]\nside = \"right\"\nwidth = 240").unwrap();
        assert_eq!(config.panel.side, crate::panel::Side::Right);
        assert_eq!(config.panel.width, 240);
        assert_eq!(config.panel.toggle_visibility, ["Meta+U"]);

        let restored: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(restored.panel, config.panel);

        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.panel, crate::panel::PanelConfig::default());
    }

    #[test]
    fn window_options_default_to_the_desktop_and_round_trip() {
        let views = ViewSettings::default();
        assert_eq!(views.window_animation, WindowAnimation::DesktopDefault);
        assert_eq!(views.fade_duration_ms(), 150);
        assert!(views.dock_hour_lines);

        let views: ViewSettings =
            toml::from_str("window_animation = \"fade_in\"\nfade_duration_ms = 99999").unwrap();
        assert_eq!(views.window_animation, WindowAnimation::FadeIn);
        assert_eq!(views.fade_duration_ms(), ViewSettings::MAX_FADE_DURATION_MS);
        let back: ViewSettings = toml::from_str(&toml::to_string(&views).unwrap()).unwrap();
        assert_eq!(back, views);
    }

    #[test]
    fn the_dock_edge_and_borders_round_trip() {
        let views = ViewSettings::default();
        assert_eq!(views.dock_side, DockSide::Left);
        assert_eq!(views.dock_margin(), 0.0);
        assert_eq!(views.dock_border_outer_width(), 1.0);
        assert_eq!(views.dock_border_inner_width(), 1.0);

        let views: ViewSettings = toml::from_str(
            "dock_side = \"bottom\"\ndock_margin = 18.0\ndock_border_outer_width = 2.0\ndock_border_outer_color = \"#FFFFFF\"\ndock_border_inner_width = 4.0\ndock_border_inner_color = \"#000000\"",
        )
        .unwrap();
        assert_eq!(views.dock_side, DockSide::Bottom);
        assert_eq!(views.dock_margin(), 18.0);
        assert_eq!(views.dock_border_outer_width(), 2.0);
        assert_eq!(views.dock_border_inner_width(), 4.0);
        assert_eq!(views.dock_border_outer_color.to_string(), "#FFFFFF");
        assert_eq!(views.dock_border_inner_color.to_string(), "#000000");

        let back: ViewSettings = toml::from_str(&toml::to_string(&views).unwrap()).unwrap();
        assert_eq!(back, views);

        let views: ViewSettings =
            toml::from_str("dock_border_outer_width = 99.0\ndock_border_inner_width = nan")
                .unwrap();
        assert_eq!(
            views.dock_border_outer_width(),
            ViewSettings::MAX_BORDER_WIDTH
        );
        assert_eq!(views.dock_border_inner_width(), 1.0);
    }

    #[test]
    fn the_panel_borders_round_trip() {
        let views = ViewSettings::default();
        assert_eq!(views.panel_border_outer_width(), 1.0);
        assert_eq!(views.panel_border_inner_width(), 1.0);

        let views: ViewSettings = toml::from_str(
            "panel_border_outer_width = 2.0\npanel_border_outer_color = \"#112233\"\npanel_border_inner_width = 4.0\npanel_border_inner_color = \"#445566\"",
        )
        .unwrap();
        assert_eq!(views.panel_border_outer_width(), 2.0);
        assert_eq!(views.panel_border_inner_width(), 4.0);
        assert_eq!(views.panel_border_outer_color.to_string(), "#112233");
        assert_eq!(views.panel_border_inner_color.to_string(), "#445566");

        let back: ViewSettings = toml::from_str(&toml::to_string(&views).unwrap()).unwrap();
        assert_eq!(back, views);

        let views: ViewSettings =
            toml::from_str("panel_border_outer_width = 99.0\npanel_border_inner_width = nan")
                .unwrap();
        assert_eq!(
            views.panel_border_outer_width(),
            ViewSettings::MAX_BORDER_WIDTH
        );
        assert_eq!(views.panel_border_inner_width(), 1.0);
    }

    #[test]
    fn the_dock_width_defaults_and_stays_within_bounds() {
        let views: ViewSettings = toml::from_str("").unwrap();
        assert_eq!(views.dock_width(), 10.0);

        let views: ViewSettings = toml::from_str("dock_width = 400.0").unwrap();
        assert_eq!(views.dock_width(), ViewSettings::MAX_DOCK_WIDTH);

        let views: ViewSettings = toml::from_str("dock_width = nan").unwrap();
        assert_eq!(views.dock_width(), 10.0);
    }

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
            "timeslay-config-watch-{}-{}",
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
        write_private(&path, b"[alarm.sound]\nvolume = 40").unwrap();
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
        write_private(&path, b"[alarm.sound]\nvolume = 60").unwrap();
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
    fn dock_margins_inherit_legacy_values_and_can_be_saved_independently() {
        let mut views: ViewSettings = toml::from_str("dock_margin = 18.0").unwrap();
        assert_eq!(
            (views.dock_left_margin(), views.dock_right_margin()),
            (18.0, 18.0)
        );
        views.dock_left_margin = Some(0.0);
        views.dock_right_margin = Some(32.0);
        let restored: ViewSettings = toml::from_str(&toml::to_string(&views).unwrap()).unwrap();
        assert_eq!(
            (restored.dock_left_margin(), restored.dock_right_margin()),
            (0.0, 32.0)
        );
    }

    #[test]
    fn explicit_sync_intervals_are_preserved_and_invalid_values_are_clamped() {
        let parsed: Config = toml::from_str("[calendar]\nsync_minutes = 120").unwrap();
        assert_eq!(parsed.calendar.sync_every(), Duration::from_secs(7200));
        let mut settings = CalendarSettings {
            sync_minutes: 0,
            ..Default::default()
        };
        assert_eq!(settings.sync_every(), Duration::from_secs(60));
        settings.sync_minutes = u32::MAX;
        assert_eq!(settings.sync_every(), Duration::from_secs(24 * 3600));
    }

    #[test]
    fn calendar_sync_defaults_to_one_hour() {
        let parsed: Config = toml::from_str("").unwrap();
        assert_eq!(parsed.calendar.sync_minutes, 60);
        assert_eq!(parsed.calendar.sync_every(), Duration::from_secs(60 * 60));
    }

    #[test]
    fn cached_days_live_under_the_xdg_cache_directory() {
        let days = cache_days_dir().expect("a cache directory");
        assert!(
            days.ends_with(Path::new("timeslay").join("days")),
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
