//! The desktop's settings, kept in the database by `timeparagon_core::settings`.
//!
//! A setting that cannot be read must never stop the app from starting, so it
//! is left at its default and reported. It is not lost to that default either:
//! a save writes only what was changed in the app, so the stored value stays
//! until it is changed.

use crate::calendar::time_format::TimeFormat;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use iced::Color;
use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::audio::Tone;
use crate::hex::HexColor;

pub use timeparagon_core::config::{
	CalendarSettings, DatabaseSyncSettings, Goal, LifeArea, Measure, Objective, PERSONAL_AREA,
	Pace, Project, ProjectsSettings, Status, TriageRequirements, WeekRange, config_path,
	project_dirs,
};
pub use timeparagon_core::places::{DistanceUnit, Place, RouteMode, SavedRoute, TravelMode};

/// How long to wait after the last change before saving.
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
	/// Notifications for what is coming up. Left out of the file until
	/// something is changed, so an untouched config stays as it was.
	#[serde(
		default,
		skip_serializing_if = "timeparagon_core::reminders::ReminderSettings::is_default"
	)]
	pub reminders: timeparagon_core::reminders::ReminderSettings,
	pub database_sync: DatabaseSyncSettings,
	/// Where the notes are kept. Left out of the file while unset.
	#[serde(
		default,
		skip_serializing_if = "timeparagon_core::config::NotesSettings::is_empty"
	)]
	pub notes: timeparagon_core::config::NotesSettings,
	pub calendar: CalendarSettings,
	pub projects: ProjectsSettings,
	/// Places and travel. It is left out of the file until something is set,
	/// so a config that never used it stays as it was.
	#[serde(default, skip_serializing_if = "TravelSettings::is_empty")]
	pub travel: TravelSettings,
	#[serde(default, skip_serializing_if = "crate::task_work::WorkLog::is_empty")]
	pub task_work: crate::task_work::WorkLog,
	pub pomodoro: crate::pomodoro::Pomodoro,
	/// The notepad's page. It is left out of the file while it is blank, so a
	/// config that never used it stays as it was.
	#[serde(default, skip_serializing_if = "Notepad::is_empty")]
	pub notepad: Notepad,
	/// One brain dump page per day. It is left out of the file until the first
	/// page is written.
	#[serde(default, skip_serializing_if = "crate::brain_dump::Dumps::is_empty")]
	pub brain_dump: crate::brain_dump::Dumps,
	/// What the routines wrote down for each day, week and month. Left out of
	/// the file until the first one is written.
	#[serde(
		default,
		skip_serializing_if = "timeparagon_core::routines::Routines::is_empty"
	)]
	pub routines: timeparagon_core::routines::Routines,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
	/// Display units for distances; cached routes remain in meters.
	pub distance_unit: DistanceUnit,
	/// The name of the selected theme, matching what the picker shows. It may
	/// be one of the catalog's names or one of the follow-the-system choices.
	pub theme: String,
	/// Display format for clock times throughout the application.
	pub time_format: TimeFormat,
}

impl Default for Appearance {
	fn default() -> Self {
		Self {
			theme: "Light".to_owned(),
			distance_unit: DistanceUnit::default(),
			time_format: TimeFormat::TwentyFourHour,
		}
	}
}

/// The notepad: a page of free text that is kept between sessions. Its text
/// is shared between devices; its size is this screen's.
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

	/// Whether the page is untouched, so there is nothing to store.
	pub fn is_empty(&self) -> bool {
		self.text.is_empty() && self.font_size == default_font_size()
	}

	/// The size of the text on the page, as a multiplier.
	///
	/// It is clamped, so that a stored size cannot ask for text that will not
	/// fit on the screen.
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
	/// Independent horizontal gaps between the dock and each screen edge.
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

	pub fn dock_left_margin(&self) -> f32 {
		self.dock_left_margin
			.filter(|v| v.is_finite())
			.map_or_else(|| 0.0, |v| v.clamp(0.0, Self::MAX_DOCK_MARGIN))
	}

	pub fn dock_right_margin(&self) -> f32 {
		self.dock_right_margin
			.filter(|v| v.is_finite())
			.map_or_else(|| 0.0, |v| v.clamp(0.0, Self::MAX_DOCK_MARGIN))
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

/// Places, the Mapbox access token, and how trips between places are made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
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
	/// calendar menu beside the TimeParagon sources, and off until then.
	pub show: bool,
	pub places: Vec<Place>,
	/// Trips whose method was chosen by hand.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub modes: Vec<RouteMode>,
	/// The place each calendar event, by id, was given.
	#[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
	pub event_places: std::collections::BTreeMap<String, u64>,
	/// The trips worked out so far, for the devices that do not work them out.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub routes: Vec<SavedRoute>,
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
		timeparagon_core::places::add_place(&mut self.places, name, address, place_id, lat, lng)
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

pub use timeparagon_core::alarms::{
	AlarmProfile, Flash, Notification, Sound, SoundChoice, Urgency,
};

impl Config {
	/// Reads the settings, falling back to defaults for anything unreadable.
	pub fn load() -> Self {
		Self::load_checked().0
	}

	/// Why some settings could not be used, if any could not.
	pub fn load_problem() -> Option<String> {
		Self::load_checked().1
	}

	/// The settings, and why some could not be used, if any could not.
	pub fn load_checked() -> (Self, Option<String>) {
		let Some(database) = crate::task_db::path() else {
			log::warn!("no data directory available; using default settings");
			return (Self::default(), None);
		};
		match Self::read(&database) {
			Ok(loaded) if loaded.problems.is_empty() => (loaded.value, None),
			Ok(loaded) => {
				let problem = format!(
					"Some settings could not be read and are at their defaults until changed: {}",
					loaded.problems.join("; ")
				);
				(loaded.value, Some(problem))
			}
			Err(error) => {
				log::warn!("could not read the settings, using defaults instead: {error}");
				let problem = format!("The settings could not be read: {error}");
				(Self::default(), Some(problem))
			}
		}
	}

	/// Reads the settings in `database`. Settings this build does not know
	/// (a newer version's) are reported and left alone.
	fn read(database: &Path) -> io::Result<timeparagon_core::settings::Loaded<Self>> {
		let loaded = timeparagon_core::settings::load::<Self>(database, |path| {
			log::warn!("ignoring unknown setting: {path}");
		})?;
		for problem in &loaded.problems {
			log::warn!("ignoring a setting that cannot be read: {problem}");
		}
		Ok(loaded)
	}

	/// Saves what changed since `baseline`, the settings as they were last
	/// loaded or saved. Whatever else changed meanwhile, in another process
	/// or on another device, is left as it is.
	pub fn save(&self, baseline: &Config) -> io::Result<()> {
		let database = crate::task_db::path().ok_or_else(|| {
			io::Error::new(io::ErrorKind::NotFound, "no data directory available")
		})?;
		self.save_at(&database, baseline)
	}

	fn save_at(&self, database: &Path, baseline: &Config) -> io::Result<()> {
		timeparagon_core::settings::save(database, baseline, self)
	}

	/// Changes the stored settings rather than this process's copy of them,
	/// with other writers held off between the read and the write.
	pub fn update_stored(change: impl FnOnce(&mut Self)) -> io::Result<()> {
		let database = crate::task_db::path().ok_or_else(|| {
			io::Error::new(io::ErrorKind::NotFound, "no data directory available")
		})?;
		timeparagon_core::settings::update(&database, change)
	}

	/// Whether `other` would be stored the same as this. A reload of this
	/// process's own save can differ in the order of lists kept as sets.
	pub fn stored_same(&self, other: &Config) -> bool {
		timeparagon_core::settings::same_stored(self, other)
	}
}

/// How often the settings are checked for changes made elsewhere: by another
/// TimeParagon process, or by database sync bringing another device's.
const WATCH_INTERVAL: Duration = Duration::from_millis(300);

/// The settings, each time they change. The first is sent at once, which
/// closes any gap since they were loaded.
pub fn watch() -> iced::Subscription<Config> {
	iced::Subscription::run(config_changes)
}

fn config_changes() -> impl iced::futures::Stream<Item = Config> {
	config_changes_at(crate::task_db::path())
}

fn config_changes_at(database: Option<PathBuf>) -> impl iced::futures::Stream<Item = Config> {
	iced::stream::channel(16, async move |mut output| {
		use iced::futures::{SinkExt, StreamExt};
		let Some(database) = database else { return };
		let (sender, mut receiver) = iced::futures::channel::mpsc::unbounded();
		if let Err(error) = spawn_watcher_at(database, move |config| {
			sender.unbounded_send(config).is_ok()
		}) {
			log::warn!("could not watch the settings: {error}");
			return;
		}
		while let Some(config) = receiver.next().await {
			if output.send(config).await.is_err() {
				break;
			}
		}
	})
}

/// Calls `changed` on a thread of its own with the settings, at once and then
/// each time they change, until it returns `false`. A cheap counter is all
/// that is read until something changes (see `local_changes`).
pub fn spawn_watcher(changed: impl FnMut(Config) -> bool + Send + 'static) -> io::Result<()> {
	let database = crate::task_db::path()
		.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data directory available"))?;
	spawn_watcher_at(database, changed)
}

fn spawn_watcher_at(
	database: PathBuf,
	mut changed: impl FnMut(Config) -> bool + Send + 'static,
) -> io::Result<()> {
	std::thread::Builder::new()
		.name("timeparagon-config-watch".into())
		.spawn(move || {
			let mut reader = None;
			let mut seen = None;
			loop {
				if reader.is_none() {
					reader = timeparagon_core::local_changes::Reader::open(&database)
						.map_err(|error| log::warn!("could not watch the settings: {error}"))
						.ok();
				}
				if let Some(counters) = reader.as_mut().and_then(|reader| {
					reader
						.read()
						.map_err(|error| log::warn!("could not watch the settings: {error}"))
						.ok()
				}) && seen != Some(counters.config)
				{
					// A failed read keeps the current settings, and is tried
					// again on the next change.
					match Config::read(&database) {
						Ok(loaded) => {
							seen = Some(counters.config);
							if !changed(loaded.value) {
								return;
							}
						}
						Err(error) => {
							log::warn!("could not reload the settings; keeping these: {error}");
						}
					}
				}
				std::thread::sleep(WATCH_INTERVAL);
			}
		})?;
	Ok(())
}

#[cfg(test)]
mod tests {
	fn database() -> (tempfile::TempDir, PathBuf) {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("timeparagon.sqlite3");
		(directory, path)
	}

	fn read(path: &Path) -> Config {
		Config::read(path).unwrap().value
	}

	#[test]
	fn a_save_writes_only_what_changed_since_its_baseline() {
		let (_directory, path) = database();
		let baseline = read(&path);
		// Another process changes something meanwhile.
		timeparagon_core::settings::update(&path, |config: &mut Config| {
			config.views.dock_width = 20.0;
		})
		.unwrap();

		let mut config = baseline.clone();
		config.appearance.theme = "Ocean".into();
		config.save_at(&path, &baseline).unwrap();

		let saved = read(&path);
		assert_eq!(saved.appearance.theme, "Ocean");
		assert_eq!(
			saved.views.dock_width, 20.0,
			"not put back to what this one saw"
		);
	}

	#[test]
	fn concurrent_updates_do_not_lose_each_others_changes() {
		let (_directory, path) = database();
		std::thread::scope(|scope| {
			for _ in 0..4 {
				scope.spawn(|| {
					for _ in 0..5 {
						timeparagon_core::settings::update(&path, |config: &mut Config| {
							config.notepad.text.push('x');
						})
						.unwrap();
					}
				});
			}
		});
		assert_eq!(read(&path).notepad.text.len(), 20);
	}

	#[test]
	fn the_desktops_settings_survive_the_phones_saves() {
		let (_directory, path) = database();
		let baseline = read(&path);
		let mut config = baseline.clone();
		config.views.dock_side = DockSide::Bottom;
		config.panel.width = 240;
		config.voice.binary = "/opt/whisper".into();
		config.travel.map_app = MapApp::GoogleMaps;
		config.save_at(&path, &baseline).unwrap();

		// The phone's smaller config knows none of these.
		timeparagon_core::settings::update(
			&path,
			|phone: &mut timeparagon_core::config::Config| {
				phone.calendar.sync_minutes = 5;
				phone.travel.show = true;
			},
		)
		.unwrap();

		let saved = read(&path);
		assert_eq!(saved.views.dock_side, DockSide::Bottom);
		assert_eq!(saved.panel.width, 240);
		assert_eq!(saved.voice.binary, "/opt/whisper");
		assert_eq!(saved.travel.map_app, MapApp::GoogleMaps);
		assert!(saved.travel.show);
		assert_eq!(saved.calendar.sync_minutes, 5);
	}

	#[test]
	fn shared_data_is_saved_to_the_database_and_read_back_with_the_config() {
		let (_directory, path) = database();
		let baseline = read(&path);
		let mut config = baseline.clone();
		config.notepad.text = "milk".into();
		config.notepad.font_size = 120;
		config.brain_dump.set(
			chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap(),
			"idea".into(),
		);
		config.save_at(&path, &baseline).unwrap();
		assert_eq!(read(&path), config);
		assert!(read(&path).stored_same(&config));
	}

	#[test]
	fn unknown_settings_warn_without_discarding_known_settings() {
		let (_directory, path) = database();
		timeparagon_core::settings::update(&path, |config: &mut Config| {
			config.appearance.theme = "Ocean".into();
			config.alarm.sound.volume = 25;
		})
		.unwrap();
		// As a newer version would store them.
		let mut phone = timeparagon_core::settings::load::<serde_json::Value>(&path, |_| {})
			.unwrap()
			.value;
		phone["future_section"] = serde_json::json!({"setting": true});
		phone["appearance"]["future_theme_setting"] = 42.into();
		timeparagon_core::settings::save(&path, &serde_json::json!({}), &phone).unwrap();

		let mut ignored = Vec::new();
		let config = timeparagon_core::settings::load::<Config>(&path, |path| ignored.push(path))
			.unwrap()
			.value;
		assert_eq!(config.appearance.theme, "Ocean");
		assert_eq!(config.alarm.sound.volume, 25);
		assert_eq!(ignored.len(), 2, "{ignored:?}");
		assert!(ignored.contains(&"future_section".to_owned()));
		assert!(ignored.contains(&"appearance.future_theme_setting".to_owned()));

		// A save of something else leaves them stored.
		let baseline = read(&path);
		let mut changed = baseline.clone();
		changed.window.width = 500.0;
		changed.save_at(&path, &baseline).unwrap();
		let mut ignored = Vec::new();
		timeparagon_core::settings::load::<Config>(&path, |path| ignored.push(path)).unwrap();
		assert_eq!(ignored.len(), 2);
	}

	#[test]
	fn the_watcher_sends_the_settings_and_then_each_change() {
		use iced::futures::StreamExt;
		let (_directory, path) = database();
		timeparagon_core::settings::update(&path, |config: &mut Config| {
			config.alarm.sound.volume = 25
		})
		.unwrap();
		let (sender, receiver) = std::sync::mpsc::channel();
		let watched = path.clone();
		let worker = std::thread::spawn(move || {
			iced::futures::executor::block_on(async move {
				let changes = config_changes_at(Some(watched));
				iced::futures::pin_mut!(changes);
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
		wait_for_volume(25);
		for volume in [40, 60] {
			timeparagon_core::settings::update(&path, |config: &mut Config| {
				config.alarm.sound.volume = volume;
			})
			.unwrap();
			wait_for_volume(volume);
		}
		worker.join().unwrap();
	}

	use super::*;

	#[test]
	fn the_managed_task_lists_are_inbox_triaged_and_backlog() {
		assert_eq!(crate::inbox::DEFAULT_LIST, "Inbox");
		assert_eq!(CalendarSettings::DEFAULT_TRIAGED_LIST, "Triaged");
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
	fn view_settings_round_trip_custom_sizes() {
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
		assert_eq!(config.panel.toggle_visibility, ["Meta+F"]);

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
		assert_eq!(views.dock_border_outer_width(), 1.0);
		assert_eq!(views.dock_border_inner_width(), 1.0);

		let views: ViewSettings = toml::from_str(
            "dock_side = \"bottom\"\ndock_left_margin = 18.0\ndock_right_margin = 18.0\ndock_border_outer_width = 2.0\ndock_border_outer_color = \"#FFFFFF\"\ndock_border_inner_width = 4.0\ndock_border_inner_color = \"#000000\"",
        )
        .unwrap();
		assert_eq!(views.dock_side, DockSide::Bottom);
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
	fn dock_margins_can_be_saved_independently() {
		let mut views: ViewSettings =
			toml::from_str("dock_left_margin = 18.0\ndock_right_margin = 18.0").unwrap();
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
