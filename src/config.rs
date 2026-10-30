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
use crate::calendar::colors;
use crate::hex::HexColor;
use crate::todos::TodoList;

/// How long to wait after the last change before writing to disk.
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
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

impl Default for Config {
    fn default() -> Self {
        Self {
            appearance: Appearance::default(),
            window: Window::default(),
            views: ViewSettings::default(),
            panel: crate::panel::PanelConfig::default(),
            alarm: AlarmProfile::default(),
            calendar: CalendarSettings::default(),
            projects: ProjectsSettings::default(),
            travel: TravelSettings::default(),
            task_work: crate::task_work::WorkLog::default(),
            pomodoro: crate::pomodoro::Pomodoro::default(),
            todos: TodoList::default(),
            notepad: Notepad::default(),
            brain_dump: crate::brain_dump::Dumps::default(),
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

/// Default pane sizes, in logical pixels, shared by the Iced windows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewSettings {
    /// None preserves the saved main-window width and the `[panel]` width.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_width: Option<f32>,
    /// Width added alongside the calendar when editing a time block.
    pub time_block_width: f32,
    /// Width of the event/task details pane shown to the right of the view.
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
    /// Thickness of the side panel's outer border, in logical pixels. Zero
    /// hides it.
    pub panel_border_outer_width: f32,
    /// Colour of the side panel's outer border.
    pub panel_border_outer_color: HexColor,
    /// Thickness of the side panel's inner border, in logical pixels. Zero
    /// hides it.
    pub panel_border_inner_width: f32,
    /// Colour of the side panel's inner border.
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
            time_block_width: 400.0,
            detail_width: 360.0,
            margin: 8.0,
            border_radius: 4.0,
            dock_border_radius: 0.0,
            panel_border_radius: None,
            dock_width: 10.0,
            dock_margin: 0.0,
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

/// A place that tasks and events can happen at, found with Mapbox search.
///
/// Everything needed to find the place again is kept apart from the title,
/// so the title can be renamed to whatever reads best ("Home", "Campus")
/// without the place losing its address or its Mapbox ID.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    /// What tasks refer to. Unlike the title, it never changes.
    pub id: u64,
    /// The title shown in the app. Free to edit.
    pub name: String,
    /// The address as Mapbox spelled it when the place was chosen.
    #[serde(default)]
    pub address: String,
    /// Mapbox's ID for the place, which keeps one place from being saved
    /// twice. (Places saved before Mapbox hold a Google ID here; routes use
    /// the coordinates, so they keep working.)
    #[serde(default)]
    pub place_id: String,
    pub lat: f64,
    pub lng: f64,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TravelSettings {
    /// A Mapbox access token (public, `pk.`) that may use the Search Box and
    /// Directions APIs.
    pub api_key: String,
    /// Where a day's travel starts: the first event is reached from here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<u64>,
    /// Where a day's travel ends: the last event is left for here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<u64>,
    /// How a trip is made until it is set otherwise.
    pub default_mode: TravelMode,
    /// The map "Open in Maps" opens a trip on.
    pub map_app: MapApp,
    pub places: Vec<Place>,
    /// Trips whose method was chosen by hand.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<RouteMode>,
    /// The place each calendar event, by id, was given.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub event_places: std::collections::BTreeMap<String, u64>,
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
        if !place_id.is_empty()
            && let Some(existing) = self.places.iter().find(|place| place.place_id == place_id)
        {
            return existing.id;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis() as u64);
        let highest = self.places.iter().map(|place| place.id).max().unwrap_or(0);
        let id = now.max(highest + 1);

        self.places.push(Place {
            id,
            name: name.trim().to_owned(),
            address: address.trim().to_owned(),
            place_id: place_id.to_owned(),
            lat,
            lng,
        });
        id
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
    /// Which metadata a task needs before it can leave the Inbox.
    #[serde(default)]
    pub triage: TriageRequirements,
    /// The hour the day ends, 0-5. 0 is midnight, the usual boundary. Set it
    /// to 3 and the day runs 3am to 3am, so the small hours stay with the
    /// evening they belong to. This only moves the view: an event keeps its
    /// real date and time, it is just drawn at the foot of the day it belongs
    /// to.
    pub day_end_hour: u32,
    /// Days beginning with the selected day; the previous day is also shown.
    pub weekly_days: u32,
    /// Width of each day in the horizontally scrollable weekly view.
    pub weekly_day_width: u32,
    /// Custom colors for Google Tasks lists, keyed by the calendar id
    /// (`tasks:<list>`). Google hands every list back with the same blue, so
    /// this is where a list gets a color of its own. A missing key means the
    /// shared task blue.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub task_colors: BTreeMap<String, HexColor>,
    /// Local calendar color overrides, keyed by Google Calendar ID.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub calendar_colors: BTreeMap<String, HexColor>,
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
            triage: TriageRequirements::default(),
            day_end_hour: 0,
            weekly_days: 8,
            weekly_day_width: 150,
            task_colors: BTreeMap::new(),
            calendar_colors: BTreeMap::new(),
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

/// Where the config and user themes live.
pub fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("dev", "TimeSlay", "timeslay")
}

pub fn config_path() -> Option<PathBuf> {
    Some(project_dirs()?.config_dir().join("config.toml"))
}

/// The directory scanned for user-written theme files.
pub fn themes_dir() -> Option<PathBuf> {
    Some(project_dirs()?.config_dir().join("themes"))
}

/// Where cached days live: `$XDG_CACHE_HOME/timeslay/days`, which is
/// `~/.cache/timeslay/days` unless the environment says otherwise. It is
/// a cache rather than state: losing it only costs a re-sync.
pub fn cache_days_dir() -> Option<PathBuf> {
    Some(project_dirs()?.cache_dir().join("days"))
}

/// Last successful Keep snapshot, including original card previews.
pub fn cache_keep_path() -> Option<PathBuf> {
    Some(project_dirs()?.cache_dir().join("keep").join("notes.json"))
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
    fn explicit_sync_intervals_are_preserved_and_invalid_values_are_clamped() {
        let parsed: Config = toml::from_str("[calendar]\nsync_minutes = 120").unwrap();
        assert_eq!(parsed.calendar.sync_every(), Duration::from_secs(7200));
        let mut settings = CalendarSettings::default();
        settings.sync_minutes = 0;
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
