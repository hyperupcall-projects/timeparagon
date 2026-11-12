//! The settings both apps share, and the minimal config the Dioxus app reads.
//!
//! [`CalendarSettings`] is the one definition of the calendar settings, used
//! by both apps. [`Config`] is deliberately a subset of the iced app's config:
//! the Dioxus app only needs these parts. Both are kept in the database by
//! [`crate::settings`], which reads and writes only the parts each one has,
//! so the phone never disturbs the desktop's settings and the other way
//! around.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::calendar::colors;
use crate::hex::HexColor;
pub use crate::life::{CheckIn, Measure, Milestone, Pace, Rating, Review, Status};
use iced_core::Color;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
	pub alarm: crate::alarms::AlarmProfile,
	pub database_sync: DatabaseSyncSettings,
	pub notes: NotesSettings,
	pub calendar: CalendarSettings,
	pub projects: ProjectsSettings,
	pub travel: crate::places::PlaceSettings,
	pub appearance: AppearanceSettings,
	pub notepad: NotepadText,
	pub brain_dump: crate::brain_dump::Dumps,
	pub reminders: crate::reminders::ReminderSettings,
}

/// A setting of this device read on its own, or empty when it is not set or
/// cannot be read.
fn device_text(key: &str) -> String {
	let value = database().and_then(|path| crate::settings::device_value(&path, key));
	match value {
		Ok(value) => value
			.as_ref()
			.and_then(serde_json::Value::as_str)
			.unwrap_or_default()
			.to_owned(),
		Err(error) => {
			log::warn!("could not read the setting {key}: {error}");
			String::new()
		}
	}
}

/// Device-local server settings. An empty URL disables network sync.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DatabaseSyncSettings {
	pub url: String,
}

impl DatabaseSyncSettings {
	/// Reads only the sync URL, without the rest of the settings.
	pub fn load() -> Self {
		Self {
			url: device_text("database_sync.url"),
		}
	}
}

/// Where the notes are kept, one Markdown file each. Empty, the
/// default, means nowhere: notes cannot be saved until it is set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotesSettings {
	/// A directory; a leading `~/` is the home directory.
	pub directory: String,
}

impl NotesSettings {
	/// Reads only the notes directory, without the rest of the settings.
	pub fn load() -> Self {
		Self {
			directory: device_text("notes.directory"),
		}
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

/// Changes only this device's notes folder.
pub fn save_notes_directory(directory: &str) -> io::Result<()> {
	save_notes_directory_at(&database()?, directory)
}

fn save_notes_directory_at(database: &Path, directory: &str) -> io::Result<()> {
	update_at(database, |config| {
		config.notes.directory = directory.trim().to_owned();
	})
}

pub fn save_database_sync_url(url: &str) -> io::Result<()> {
	save_database_sync_url_at(&database()?, url)
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

fn save_database_sync_url_at(database: &Path, url: &str) -> io::Result<()> {
	validate_database_sync_url(url).map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
	update_at(database, |config| {
		config.database_sync.url = url.trim().to_owned();
	})
}

/// The part of `notepad` the Dioxus app edits. Its font size is the desktop's.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotepadText {
	pub text: String,
}

/// The part of `appearance` the Dioxus app follows. The theme is left out:
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
	/// How satisfied one has been with this area, week by week, oldest first.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub ratings: Vec<Rating>,
}

impl LifeArea {
	pub fn new(id: u64, name: String) -> Self {
		Self {
			id,
			name,
			color: None,
			vision: String::new(),
			goals: Vec::new(),
			objectives: Vec::new(),
			ratings: Vec::new(),
		}
	}

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

	pub fn goal(&self, id: u64) -> Option<&Goal> {
		self.goals.iter().find(|goal| goal.id == id)
	}

	pub fn objective(&self, id: u64) -> Option<&Objective> {
		self.objectives.iter().find(|objective| objective.id == id)
	}

	/// The objectives listed under `goal`, or with none.
	pub fn objectives_of(&self, goal: Option<u64>) -> impl Iterator<Item = &Objective> {
		self.objectives
			.iter()
			.filter(move |objective| self.objective_goal(objective) == goal)
	}

	/// The latest satisfaction rating.
	pub fn satisfaction(&self) -> Option<&Rating> {
		self.ratings.last()
	}

	/// The rating given for the week starting `week`.
	pub fn rating_for(&self, week: NaiveDate) -> Option<u8> {
		self.ratings
			.iter()
			.find(|rating| rating.date == week)
			.map(|rating| rating.score)
	}

	/// Rates the week starting `week`, replacing any rating already given for it.
	pub fn rate(&mut self, week: NaiveDate, score: u8) {
		let score = score.clamp(1, 10);
		crate::life::upsert_by_date(
			&mut self.ratings,
			week,
			Rating { date: week, score },
			|rating| rating.date,
		);
	}

	/// How far a goal has come: the mean progress of its objectives that are
	/// measured and not dropped. A goal marked done is all the way there.
	pub fn goal_progress(
		&self,
		goal: &Goal,
		habit_score: &dyn Fn(&str) -> Option<f64>,
	) -> Option<f64> {
		if goal.status == Status::Done {
			return Some(1.0);
		}
		crate::life::mean(
			self.objectives_of(Some(goal.id))
				.filter(|objective| objective.status != Status::Dropped)
				.filter_map(|objective| objective.progress(habit_score)),
		)
	}

	/// The mean progress of every measured objective that is not dropped.
	pub fn progress(&self, habit_score: &dyn Fn(&str) -> Option<f64>) -> Option<f64> {
		crate::life::mean(
			self.objectives
				.iter()
				.filter(|objective| objective.status != Status::Dropped)
				.filter_map(|objective| objective.progress(habit_score)),
		)
	}
}

/// One goal of a life area: something aspired to, which can be reached in
/// more than one way. How it will be reached is up to its objectives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Goal {
	pub id: u64,
	pub name: String,
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub description: String,
	/// Why it matters, to come back to when motivation runs low.
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub why: String,
	#[serde(default, skip_serializing_if = "Status::is_default")]
	pub status: Status,
}

impl Goal {
	pub fn new(id: u64, name: String) -> Self {
		Self {
			id,
			name,
			description: String::new(),
			why: String::new(),
			status: Status::Active,
		}
	}
}

/// Something concrete that a life area works towards, usually in service of
/// one of its goals: specific, measurable and bound to a time frame, like a
/// SMART goal or a key result. Carried out by habits and projects.
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
	/// Projects that carry it out, by project id.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub projects: Vec<u64>,
	#[serde(default, skip_serializing_if = "Status::is_default")]
	pub status: Status,
	/// Pinned to the Focus page: what this week is about.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub focus: bool,
	/// When work on it began, which pace is measured from.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub start: Option<NaiveDate>,
	/// When it should be reached by.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub due: Option<NaiveDate>,
	#[serde(default, skip_serializing_if = "Measure::is_default")]
	pub measure: Measure,
	/// For [`Measure::Number`]: where it started.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub baseline: Option<f64>,
	/// For [`Measure::Number`]: where it should end up.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub target: Option<f64>,
	/// For [`Measure::Number`]: what is counted, such as `kg` or `km/week`.
	#[serde(default, skip_serializing_if = "String::is_empty")]
	pub unit: String,
	/// For [`Measure::Number`]: the values recorded, oldest first.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub check_ins: Vec<CheckIn>,
	/// For [`Measure::Milestones`], in order.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub milestones: Vec<Milestone>,
}

impl Objective {
	pub fn new(id: u64, name: String, goal: Option<u64>) -> Self {
		Self {
			id,
			name,
			description: String::new(),
			goal,
			habits: Vec::new(),
			projects: Vec::new(),
			status: Status::Active,
			focus: false,
			start: None,
			due: None,
			measure: Measure::None,
			baseline: None,
			target: None,
			unit: String::new(),
			check_ins: Vec::new(),
			milestones: Vec::new(),
		}
	}

	/// The latest value of a [`Measure::Number`] objective: the last
	/// check-in, or where it started.
	pub fn current(&self) -> Option<f64> {
		self.check_ins
			.last()
			.map(|check_in| check_in.value)
			.or(self.baseline)
	}

	/// How far along it is, from 0 to 1, when that can be told. A done
	/// objective is all the way there whatever its measure says.
	pub fn progress(&self, habit_score: &dyn Fn(&str) -> Option<f64>) -> Option<f64> {
		if self.status == Status::Done {
			return Some(1.0);
		}
		match self.measure {
			Measure::None => None,
			Measure::Number => {
				crate::life::number_progress(self.baseline?, self.target?, self.current()?)
			}
			Measure::Milestones => {
				let done = self.milestones.iter().filter(|m| m.done).count();
				(!self.milestones.is_empty()).then(|| done as f64 / self.milestones.len() as f64)
			}
			Measure::Habits => {
				crate::life::mean(self.habits.iter().filter_map(|habit| habit_score(habit)))
			}
		}
	}

	pub fn pace(&self, today: NaiveDate, habit_score: &dyn Fn(&str) -> Option<f64>) -> Pace {
		crate::life::pace(
			self.status,
			self.start,
			self.due,
			self.progress(habit_score),
			today,
		)
	}

	/// Records `value` for `date`, replacing what was recorded that day.
	pub fn check_in(&mut self, date: NaiveDate, value: f64, note: String) {
		crate::life::upsert_by_date(
			&mut self.check_ins,
			date,
			CheckIn { date, value, note },
			|check_in| check_in.date,
		);
	}

	/// The last day anything was recorded for it.
	pub fn last_check_in(&self) -> Option<NaiveDate> {
		self.check_ins.last().map(|check_in| check_in.date)
	}

	/// Adds a milestone at the end and returns its id.
	pub fn add_milestone(&mut self, name: String) -> u64 {
		let id = unique_id(self.milestones.iter().map(|m| m.id));
		self.milestones.push(Milestone {
			id,
			name,
			done: false,
		});
		id
	}
}

fn personal_area() -> LifeArea {
	LifeArea::new(PERSONAL_AREA, "Personal".to_owned())
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
	/// Weekly reviews, oldest first.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub reviews: Vec<Review>,
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
			reviews: Vec::new(),
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
		self.ensure_unique_life_ids();
	}

	/// Gives a new id to any goal or objective that shares its id with one in
	/// an earlier area. Ids used to be unique only within their area, but
	/// the Life tab finds goals and objectives by id alone. An objective
	/// keeps pointing at its goal, since both are in the same area.
	pub fn ensure_unique_life_ids(&mut self) -> bool {
		let mut changed = false;
		let mut goals = std::collections::HashSet::new();
		let mut objectives = std::collections::HashSet::new();
		let mut next = unique_id(self.areas.iter().flat_map(|a| {
			a.goals
				.iter()
				.map(|g| g.id)
				.chain(a.objectives.iter().map(|o| o.id))
		}));
		for area in &mut self.areas {
			let mut renamed = BTreeMap::new();
			for goal in &mut area.goals {
				if !goals.insert(goal.id) {
					renamed.insert(goal.id, next);
					goal.id = next;
					goals.insert(next);
					next += 1;
					changed = true;
				}
			}
			for objective in &mut area.objectives {
				if let Some(goal) = objective.goal.and_then(|g| renamed.get(&g)) {
					objective.goal = Some(*goal);
				}
				if !objectives.insert(objective.id) {
					objective.id = next;
					objectives.insert(next);
					next += 1;
					changed = true;
				}
			}
		}
		changed
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
		self.areas.push(LifeArea::new(id, name));
		id
	}

	/// Removes a life area. What was in it falls back to Personal, which
	/// itself cannot be removed.
	pub fn remove_area(&mut self, id: u64) {
		if id != PERSONAL_AREA {
			self.areas.retain(|area| area.id != id);
		}
	}

	/// Moves an area one place up or down the list.
	pub fn shift_area(&mut self, id: u64, up: bool) {
		let Some(index) = self.areas.iter().position(|area| area.id == id) else {
			return;
		};
		let other = if up {
			index.checked_sub(1)
		} else {
			Some(index + 1).filter(|i| *i < self.areas.len())
		};
		if let Some(other) = other {
			self.areas.swap(index, other);
		}
	}

	/// The area holding the goal with this id, with the goal.
	pub fn find_goal(&self, id: u64) -> Option<(&LifeArea, &Goal)> {
		self.areas
			.iter()
			.find_map(|area| area.goal(id).map(|goal| (area, goal)))
	}

	/// The area holding the objective with this id, with the objective.
	pub fn find_objective(&self, id: u64) -> Option<(&LifeArea, &Objective)> {
		self.areas
			.iter()
			.find_map(|area| area.objective(id).map(|objective| (area, objective)))
	}

	pub fn goal_mut(&mut self, id: u64) -> Option<&mut Goal> {
		self.areas
			.iter_mut()
			.find_map(|area| area.goals.iter_mut().find(|goal| goal.id == id))
	}

	pub fn objective_mut(&mut self, id: u64) -> Option<&mut Objective> {
		self.areas.iter_mut().find_map(|area| {
			area.objectives
				.iter_mut()
				.find(|objective| objective.id == id)
		})
	}

	/// Every objective, with the area it is in.
	pub fn all_objectives(&self) -> impl Iterator<Item = (&LifeArea, &Objective)> {
		self.areas
			.iter()
			.flat_map(|area| area.objectives.iter().map(move |o| (area, o)))
	}

	/// The objectives a project carries out.
	pub fn objectives_of_project(
		&self,
		project: u64,
	) -> impl Iterator<Item = (&LifeArea, &Objective)> {
		self.all_objectives()
			.filter(move |(_, objective)| objective.projects.contains(&project))
	}

	pub fn review(&self, week: NaiveDate) -> Option<&Review> {
		self.reviews.iter().find(|review| review.date == week)
	}

	/// The review of the week starting `week`, made if there is none yet.
	pub fn review_mut(&mut self, week: NaiveDate) -> &mut Review {
		let index = match self
			.reviews
			.binary_search_by_key(&week, |review| review.date)
		{
			Ok(index) => index,
			Err(index) => {
				self.reviews.insert(
					index,
					Review {
						date: week,
						reflection: String::new(),
						done: false,
					},
				);
				index
			}
		};
		&mut self.reviews[index]
	}

	/// Adds a goal to an area and returns its id.
	pub fn add_goal(&mut self, area: u64, name: String) -> Option<u64> {
		// Unique across every area, so a goal can be found by id alone.
		let id = unique_id(self.areas.iter().flat_map(|a| a.goals.iter().map(|g| g.id)));
		let area = self.area_mut(area)?;
		area.goals.push(Goal::new(id, name));
		Some(id)
	}

	/// Adds an objective to an area, under `goal` if given, and returns its id.
	pub fn add_objective(&mut self, area: u64, goal: Option<u64>, name: String) -> Option<u64> {
		// Unique across every area, so an objective can be found by id alone.
		let id = unique_id(
			self.areas
				.iter()
				.flat_map(|a| a.objectives.iter().map(|o| o.id)),
		);
		let area = self.area_mut(area)?;
		area.objectives.push(Objective::new(id, name, goal));
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
	/// cadence and is what we ship with. Only used when [`Self::incremental_sync`]
	/// is off: incremental sync checks every [`Self::INCREMENTAL_SYNC_EVERY`].
	pub sync_minutes: u32,
	/// Sync with Google incrementally: Calendar sync tokens and Tasks'
	/// `updatedMin`, so a sync only fetches what changed. Off fetches the
	/// whole range in view on every sync, as before.
	pub incremental_sync: bool,
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
	/// The discovered id of the app-owned TimeParagon calendar. Every event the
	/// app writes (time blocks and scheduled tasks) lives here. Managed by the
	/// app from the calendar list; not exposed as a user setting.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub app_calendar: Option<String>,
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
	/// Bedtime and wake-up time by wake-up weekday, Monday first.
	pub sleep_schedule: [crate::calendar::sleep::Times; 7],
	/// Dated weekday changes keep earlier nights on their previous schedule.
	pub sleep_schedule_changes: Vec<crate::calendar::sleep::ScheduleChange>,
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
	/// Color of all-day events on the TimeParagon calendar. None uses the
	/// built-in event color.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub event_color: Option<HexColor>,
	/// Color of sleep events on the TimeParagon calendar. None uses the built-in
	/// sleep gray.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub sleep_color: Option<HexColor>,
	/// Color of time-block events on the TimeParagon calendar. None uses the
	/// built-in block color.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub time_block_color: Option<HexColor>,
	/// Color of scheduled task events on the TimeParagon calendar. None uses
	/// Google's task blue.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub task_event_color: Option<HexColor>,
	/// Color of habit events on the TimeParagon calendar. None uses the built-in
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
			incremental_sync: true,
			client_id: String::new(),
			client_secret: String::new(),
			disabled: Vec::new(),
			source_order: Vec::new(),
			show_tasks: true,
			hide_overdue_tasks: false,
			app_calendar: None,
			backlog_list: None,
			triaged_list: None,
			inbox_list: None,
			inbox_sort: Default::default(),
			overdue_sort: Default::default(),
			triage: TriageRequirements::default(),
			day_end_hour: 0,
			sleep_schedule: [Default::default(); 7],
			sleep_schedule_changes: Vec::new(),
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
	/// Synthetic source shared by every Google Tasks list.
	pub const GOOGLE_TASKS_ID: &str = "tasks:";

	/// The Google Tasks list tasks are moved to for the backlog, unless
	/// another is chosen in Settings.
	pub const DEFAULT_BACKLOG_LIST: &str = "Backlog";

	/// The Google Tasks list triaged tasks are kept on, unless another is
	/// chosen in Settings.
	pub const DEFAULT_TRIAGED_LIST: &str = "Triaged";

	/// The name of the single calendar every event the app writes lives on.
	/// Created on first use; not customizable from Settings.
	pub const APP_CALENDAR_NAME: &str = "TimeParagon";

	/// Whether `name` is the app calendar.
	pub fn is_app_calendar_name(name: &str) -> bool {
		name.trim() == Self::APP_CALENDAR_NAME
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

	/// How often incremental sync asks Google for changes in the background.
	pub const INCREMENTAL_SYNC_EVERY: Duration = Duration::from_secs(2 * 60);

	/// How often the apps sync in the background: every two minutes with
	/// incremental sync, since each sync only fetches what changed, otherwise
	/// every [`Self::sync_every`].
	pub fn refresh_every(&self) -> Duration {
		if self.incremental_sync {
			Self::INCREMENTAL_SYNC_EVERY
		} else {
			self.sync_every()
		}
	}

	/// [`Self::incremental_sync`] as saved in the settings of the database
	/// `connection` is open on. Read on its own so that every process (the
	/// dock, the phone's widget) follows the same switch.
	pub(crate) fn stored_incremental_sync(connection: &rusqlite::Connection) -> bool {
		use rusqlite::OptionalExtension;
		let saved: Option<String> = connection
			.query_row(
				"SELECT value FROM settings WHERE key='calendar.incremental_sync'",
				[],
				|row| row.get(0),
			)
			.optional()
			.ok()
			.flatten();
		saved
			.and_then(|text| serde_json::from_str::<bool>(&text).ok())
			.unwrap_or(Self::default().incremental_sync)
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

	/// [`Self::day_start_hour`] as saved in the settings of the database
	/// `connection` is open on, read on its own by code already inside a
	/// transaction there.
	pub(crate) fn stored_day_start_hour(connection: &rusqlite::Connection) -> u32 {
		use rusqlite::OptionalExtension;
		let saved: Option<String> = connection
			.query_row(
				"SELECT value FROM settings WHERE key='calendar.day_end_hour'",
				[],
				|row| row.get(0),
			)
			.optional()
			.ok()
			.flatten();
		let hour = saved
			.and_then(|text| serde_json::from_str::<u32>(&text).ok())
			.unwrap_or(Self::default().day_end_hour);
		hour.min(Self::MAX_DAY_END_HOUR)
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
	let dirs = ProjectDirs::from(directories::ProjectDirs::from(
		"dev",
		"TimeParagon",
		"timeparagon",
	)?);
	static MIGRATED: OnceLock<()> = OnceLock::new();
	MIGRATED.get_or_init(|| {
		if let Some(legacy) = directories::ProjectDirs::from("dev", "TimeSlay", "timeslay") {
			migrate_legacy_dirs(&ProjectDirs::from(legacy), &dirs);
		}
	});
	Some(dirs)
}

impl ProjectDirs {
	fn from(dirs: directories::ProjectDirs) -> Self {
		Self {
			config: dirs.config_dir().to_owned(),
			data: dirs.data_dir().to_owned(),
			data_local: dirs.data_local_dir().to_owned(),
			cache: dirs.cache_dir().to_owned(),
			state: dirs.state_dir().map(Path::to_owned),
		}
	}
}

/// The database's name before the project was renamed from TimeSlay.
const LEGACY_DATABASE: &str = "timeslay.sqlite3";

/// Moves what the app kept under its old name, TimeSlay, to the TimeParagon
/// directories. A directory is only moved when its new one does not exist
/// yet, so nothing is ever overwritten; `packaging/install.sh` does the same.
/// Inside the data directory the database is renamed too, its WAL first so
/// that an interruption never separates committed pages from the database.
fn migrate_legacy_dirs(legacy: &ProjectDirs, current: &ProjectDirs) {
	let pairs = [
		(Some(&legacy.config), Some(&current.config)),
		(Some(&legacy.data), Some(&current.data)),
		(Some(&legacy.data_local), Some(&current.data_local)),
		(Some(&legacy.cache), Some(&current.cache)),
		(legacy.state.as_ref(), current.state.as_ref()),
	];
	for (old, new) in pairs {
		let (Some(old), Some(new)) = (old, new) else {
			continue;
		};
		// On Linux the data and local data directories are the same.
		if !old.is_dir() || new.exists() {
			continue;
		}
		let moved = new
			.parent()
			.map_or(Ok(()), std::fs::create_dir_all)
			.and_then(|()| std::fs::rename(old, new));
		match moved {
			Ok(()) => log::info!("moved {} to {}", old.display(), new.display()),
			Err(error) => log::error!(
				"could not move {} to {}: {error}",
				old.display(),
				new.display()
			),
		}
	}
	let directory = &current.data_local;
	let database = directory.join("timeparagon.sqlite3");
	if database.exists() || !directory.join(LEGACY_DATABASE).exists() {
		return;
	}
	for suffix in ["-wal", "-shm", "-journal", ""] {
		let old = directory.join(format!("{LEGACY_DATABASE}{suffix}"));
		if !old.exists() {
			continue;
		}
		let new = directory.join(format!("timeparagon.sqlite3{suffix}"));
		if let Err(error) = std::fs::rename(&old, &new) {
			log::error!(
				"could not rename {} to {}: {error}",
				old.display(),
				new.display()
			);
			return;
		}
	}
	// Only a lock; the sync worker makes a new one under the new name.
	let _ = std::fs::remove_file(directory.join("timeslay-sync.owner"));
	log::info!("renamed the database to {}", database.display());
}

/// Whether this process is a test run that must not touch the user's files or
/// accounts. Every path then lives in a fresh scratch directory, so tests never
/// see the real database, config or Google tokens, and background workers that
/// talk to Google or the sync server stay off.
///
/// Core's own tests and Cargo test executables are always isolated, including
/// when this crate is compiled as an ordinary dependency without `cfg(test)`.
/// Detection must not depend on features enabled by the calling test crate.
/// Normal app binaries live outside Cargo's `deps` directory.
pub fn isolated() -> bool {
	isolated_root().is_some()
}

fn isolated_root() -> Option<&'static Path> {
	static ROOT: OnceLock<Option<tempfile::TempDir>> = OnceLock::new();
	ROOT.get_or_init(|| {
		let executable = std::env::current_exe().expect("cannot determine storage isolation");
		let test_harness = executable
			.parent()
			.and_then(Path::file_name)
			.is_some_and(|name| name == "deps");
		if !cfg!(test) && !test_harness {
			return None;
		}
		// Fail closed: never fall back to the user's files if allocation fails.
		Some(
			tempfile::Builder::new()
				.prefix("timeparagon-test-")
				.tempdir()
				.expect("tests need a scratch directory instead of the real data"),
		)
	})
	.as_ref()
	.map(tempfile::TempDir::path)
}

/// The settings file older versions kept. The first start of this version
/// imports it into the database and renames it (see [`crate::settings`]).
pub fn config_path() -> Option<PathBuf> {
	Some(project_dirs()?.config_dir().join("config.toml"))
}

/// The local database, which holds the settings.
fn database() -> io::Result<PathBuf> {
	crate::task_db::path()
		.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data directory available"))
}

/// Changes the saved settings: reads them, lets `change` change them and saves
/// what it changed, with other writers held off in between. Settings this
/// type does not have (the desktop's) are left exactly as they are.
pub fn update<R>(change: impl FnOnce(&mut Config) -> R) -> io::Result<R> {
	update_at(&database()?, change)
}

fn update_at<R>(database: &Path, change: impl FnOnce(&mut Config) -> R) -> io::Result<R> {
	crate::settings::update(database, change)
}

/// Stores the Google OAuth client.
pub fn save_google_client(client_id: &str, client_secret: &str) -> io::Result<()> {
	save_google_client_at(&database()?, client_id, client_secret)
}

fn save_google_client_at(database: &Path, client_id: &str, client_secret: &str) -> io::Result<()> {
	update_at(database, |config| {
		config.calendar.client_id = client_id.trim().to_owned();
		config.calendar.client_secret = client_secret.trim().to_owned();
	})
}

/// Saves which sources are switched off.
pub fn save_calendar_visibility(settings: &CalendarSettings) -> io::Result<()> {
	save_calendar_visibility_at(&database()?, settings)
}

fn save_calendar_visibility_at(database: &Path, settings: &CalendarSettings) -> io::Result<()> {
	update_at(database, |config| {
		config.calendar.disabled = settings.disabled.clone();
		config.calendar.show_tasks = settings.show_tasks;
	})
}

/// Saves the orders of the Inbox and Overdue lists.
pub fn save_task_sorts(
	inbox: crate::calendar::task_sort::InboxSort,
	overdue: crate::calendar::task_sort::InboxSort,
) -> io::Result<()> {
	save_task_sorts_at(&database()?, inbox, overdue)
}

fn save_task_sorts_at(
	database: &Path,
	inbox: crate::calendar::task_sort::InboxSort,
	overdue: crate::calendar::task_sort::InboxSort,
) -> io::Result<()> {
	update_at(database, |config| {
		config.calendar.inbox_sort = inbox;
		config.calendar.overdue_sort = overdue;
	})
}

/// Saves which day weeks start on.
pub fn save_first_day_of_week(
	first_day: crate::calendar::first_day::FirstDayOfWeek,
) -> io::Result<()> {
	save_first_day_of_week_at(&database()?, first_day)
}

fn save_first_day_of_week_at(
	database: &Path,
	first_day: crate::calendar::first_day::FirstDayOfWeek,
) -> io::Result<()> {
	update_at(database, |config| {
		config.calendar.first_day_of_week = first_day;
	})
}

/// Turns incremental sync with Google on or off.
pub fn save_incremental_sync(on: bool) -> io::Result<()> {
	update_at(&database()?, |config| {
		config.calendar.incremental_sync = on;
	})
}

/// Saves the weekly sleep schedule and its dated changes.
pub fn save_sleep_schedule(settings: &CalendarSettings) -> io::Result<()> {
	save_sleep_schedule_at(&database()?, settings)
}

fn save_sleep_schedule_at(database: &Path, settings: &CalendarSettings) -> io::Result<()> {
	update_at(database, |config| {
		config.calendar.sleep_schedule = settings.sleep_schedule;
		config.calendar.sleep_schedule_changes = settings.sleep_schedule_changes.clone();
	})
}

/// Saves whether travel is shown on the calendar.
pub fn save_travel_show(show: bool) -> io::Result<()> {
	save_travel_show_at(&database()?, show)
}

fn save_travel_show_at(database: &Path, show: bool) -> io::Result<()> {
	update_at(database, |config| config.travel.show = show)
}

/// Replaces the reminder settings.
pub fn save_reminders(settings: &crate::reminders::ReminderSettings) -> io::Result<()> {
	save_reminders_at(&database()?, settings)
}

fn save_reminders_at(
	database: &Path,
	settings: &crate::reminders::ReminderSettings,
) -> io::Result<()> {
	update_at(database, |config| config.reminders = settings.clone())
}

/// Saves a place Mapbox found and returns its id, reusing one already saved.
pub fn save_found_place(found: &crate::places::Found) -> io::Result<u64> {
	save_found_place_at(&database()?, found)
}

fn save_found_place_at(database: &Path, found: &crate::places::Found) -> io::Result<u64> {
	update_at(database, |config| {
		crate::places::add_place(
			&mut config.travel.places,
			&found.name,
			&found.address,
			&found.place_id,
			found.lat,
			found.lng,
		)
	})
}

/// Records the saved place a calendar event was given, or forgets it. The iced
/// app prefers this over the event's location text, so a change has to reach it.
pub fn save_event_place(event_id: &str, place: Option<u64>) -> io::Result<()> {
	save_event_place_at(&database()?, event_id, place)
}

fn save_event_place_at(database: &Path, event_id: &str, place: Option<u64>) -> io::Result<()> {
	update_at(database, |config| match place {
		Some(place) => {
			config
				.travel
				.event_places
				.insert(event_id.to_owned(), place);
		}
		None => {
			config.travel.event_places.remove(event_id);
		}
	})
}

/// Replaces the notepad's text. Its font size is a setting of each device.
pub fn save_notepad_text(text: &str) -> io::Result<()> {
	let notepad = NotepadText {
		text: text.to_owned(),
	};
	crate::shared_data::save_section(&database()?, "notepad", &notepad)
}

/// Replaces the brain dump pages.
pub fn save_brain_dump(dumps: &crate::brain_dump::Dumps) -> io::Result<()> {
	crate::shared_data::save_section(&database()?, "brain_dump", dumps)
}

/// Replaces the list of projects.
pub fn save_projects(projects: &ProjectsSettings) -> io::Result<()> {
	crate::shared_data::save_section(&database()?, "projects", projects)
}

impl Config {
	/// Reads the settings, falling back to defaults for anything unreadable.
	pub fn load() -> Self {
		match database() {
			Ok(path) => Self::load_from(&path),
			Err(error) => {
				log::warn!("{error}; using default settings");
				Self::default()
			}
		}
	}

	fn load_from(database: &Path) -> Self {
		// The desktop's settings are not this type's, so they are not reported.
		match crate::settings::load::<Self>(database, |_| {}) {
			Ok(loaded) => {
				for problem in &loaded.problems {
					log::warn!("ignoring a setting that cannot be read: {problem}");
				}
				loaded.value
			}
			Err(error) => {
				log::warn!("could not read the settings, using defaults instead: {error}");
				Self::default()
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn legacy_directories_and_database_are_moved_once() {
		use std::fs;
		let root = tempfile::tempdir().unwrap();
		let legacy = ProjectDirs::under(&root.path().join("old"));
		let current = ProjectDirs::under(&root.path().join("new"));
		fs::create_dir_all(&legacy.data_local).unwrap();
		fs::write(legacy.data_local.join(LEGACY_DATABASE), "pages").unwrap();
		fs::write(legacy.data_local.join("timeslay.sqlite3-wal"), "wal").unwrap();
		fs::write(legacy.data_local.join("timeslay-sync.owner"), "").unwrap();
		fs::create_dir_all(&legacy.config).unwrap();
		fs::write(legacy.config.join("config.toml.imported"), "old").unwrap();
		// A directory the new name already has is left alone, both copies intact.
		fs::create_dir_all(legacy.state.as_ref().unwrap()).unwrap();
		fs::write(legacy.state.as_ref().unwrap().join("secrets.json"), "old").unwrap();
		fs::create_dir_all(current.state.as_ref().unwrap()).unwrap();

		migrate_legacy_dirs(&legacy, &current);

		let local = &current.data_local;
		assert_eq!(
			fs::read_to_string(local.join("timeparagon.sqlite3")).unwrap(),
			"pages"
		);
		assert_eq!(
			fs::read_to_string(local.join("timeparagon.sqlite3-wal")).unwrap(),
			"wal"
		);
		assert!(!local.join(LEGACY_DATABASE).exists());
		assert!(!local.join("timeslay-sync.owner").exists());
		assert!(!legacy.data_local.exists());
		assert!(current.config.join("config.toml.imported").exists());
		assert!(legacy.state.as_ref().unwrap().join("secrets.json").exists());
		assert!(
			!current
				.state
				.as_ref()
				.unwrap()
				.join("secrets.json")
				.exists()
		);

		// Running again, as every process does, changes nothing.
		fs::write(local.join("timeparagon.sqlite3"), "newer").unwrap();
		migrate_legacy_dirs(&legacy, &current);
		assert_eq!(
			fs::read_to_string(local.join("timeparagon.sqlite3")).unwrap(),
			"newer"
		);
	}

	fn database() -> (tempfile::TempDir, PathBuf) {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("timeparagon.sqlite3");
		(directory, path)
	}

	/// A desktop setting this type knows nothing about, to check it survives.
	fn desktop_setting(path: &Path) -> Option<String> {
		let mut db = crate::task_db::Database::open(path).unwrap();
		let connection = db.connection();
		connection
			.execute(
				"INSERT INTO device_settings VALUES('window.width','321') ON CONFLICT DO NOTHING",
				[],
			)
			.unwrap();
		connection
			.query_row(
				"SELECT value FROM device_settings WHERE key='window.width'",
				[],
				|row| row.get(0),
			)
			.ok()
	}

	#[test]
	fn notes_folder_setting_preserves_desktop_settings() {
		let (_directory, path) = database();
		desktop_setting(&path);
		save_notes_directory_at(&path, "  ~/Documents/Shared Notes  ").unwrap();
		assert_eq!(
			Config::load_from(&path).notes.directory,
			"~/Documents/Shared Notes"
		);
		assert_eq!(desktop_setting(&path).as_deref(), Some("321"));
		save_notes_directory_at(&path, " ").unwrap();
		assert!(Config::load_from(&path).notes.path().is_none());
	}

	#[test]
	fn sync_url_defaults_to_disabled_and_preserves_other_settings() {
		assert!(Config::default().database_sync.url.is_empty());
		let (_directory, path) = database();
		desktop_setting(&path);
		save_database_sync_url_at(&path, "  https://sync.example.com  ").unwrap();
		assert_eq!(
			Config::load_from(&path).database_sync.url,
			"https://sync.example.com"
		);
		assert_eq!(desktop_setting(&path).as_deref(), Some("321"));
		save_database_sync_url_at(&path, "  ").unwrap();
		assert!(Config::load_from(&path).database_sync.url.is_empty());
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
		let (_directory, path) = database();
		save_database_sync_url_at(&path, "https://sync.example.com").unwrap();
		let error = save_database_sync_url_at(&path, "sync.example.com").unwrap_err();
		assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
		assert_eq!(
			Config::load_from(&path).database_sync.url,
			"https://sync.example.com"
		);
	}

	#[test]
	fn shared_documents_come_back_with_the_settings() {
		let (_directory, path) = database();
		let mut dumps = crate::brain_dump::Dumps::default();
		dumps.set(
			chrono::NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(),
			"idea".into(),
		);
		crate::shared_data::save_section(&path, "brain_dump", &dumps).unwrap();
		let notepad = NotepadText { text: "new".into() };
		crate::shared_data::save_section(&path, "notepad", &notepad).unwrap();

		let read = Config::load_from(&path);
		assert_eq!(read.brain_dump, dumps);
		assert_eq!(read.notepad.text, "new");
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
	fn saving_the_client_keeps_other_settings() {
		let (_directory, path) = database();
		save_task_sorts_at(
			&path,
			crate::calendar::task_sort::InboxSort::ALL[1],
			crate::calendar::task_sort::InboxSort::ALL[1],
		)
		.unwrap();
		save_google_client_at(&path, " id ", " secret ").unwrap();
		save_google_client_at(&path, "id2", "secret2").unwrap();
		let config = Config::load_from(&path);
		assert_eq!(config.calendar.client_id, "id2");
		assert_eq!(config.calendar.client_secret, "secret2");
		assert_eq!(
			config.calendar.inbox_sort,
			crate::calendar::task_sort::InboxSort::ALL[1]
		);
	}

	#[test]
	fn saving_the_first_day_of_the_week_keeps_other_settings() {
		use crate::calendar::first_day::FirstDayOfWeek;

		let (_directory, path) = database();
		save_travel_show_at(&path, true).unwrap();
		save_first_day_of_week_at(&path, FirstDayOfWeek::Saturday).unwrap();

		let config = Config::load_from(&path);
		assert!(config.travel.show);
		assert_eq!(config.calendar.first_day_of_week, FirstDayOfWeek::Saturday);
	}

	#[test]
	fn saving_the_sleep_schedule_reads_back_and_keeps_other_settings() {
		use crate::calendar::sleep;

		let (_directory, path) = database();
		save_travel_show_at(&path, true).unwrap();
		let mut settings = CalendarSettings::default();
		let from = chrono::NaiveDate::from_ymd_opt(2030, 11, 4).unwrap();
		sleep::set_weekday_times(
			&mut settings,
			from,
			sleep::Times {
				bedtime: chrono::NaiveTime::from_hms_opt(23, 15, 0).unwrap(),
				wakeup: chrono::NaiveTime::from_hms_opt(7, 30, 0).unwrap(),
			},
		);

		save_sleep_schedule_at(&path, &settings).unwrap();

		let config = Config::load_from(&path);
		assert!(config.travel.show);
		assert_eq!(config.calendar.sleep_schedule, settings.sleep_schedule);
		assert_eq!(
			config.calendar.sleep_schedule_changes,
			settings.sleep_schedule_changes
		);
	}

	#[test]
	fn saving_the_task_sorts_reads_back() {
		use crate::calendar::task_sort::InboxSort;

		let (_directory, path) = database();
		for (inbox, overdue) in InboxSort::ALL
			.into_iter()
			.zip(InboxSort::ALL.into_iter().rev())
		{
			save_task_sorts_at(&path, inbox, overdue).unwrap();
			let config = Config::load_from(&path);
			assert_eq!(config.calendar.inbox_sort, inbox);
			assert_eq!(config.calendar.overdue_sort, overdue);
			assert_eq!(InboxSort::parse(inbox.name()), Some(inbox));
		}
	}

	#[test]
	fn saving_reminders_reads_back_and_keeps_other_settings() {
		use crate::reminders::{ReminderSettings, Sound};
		use crate::sounds::Tone;

		let (_directory, path) = database();
		save_travel_show_at(&path, true).unwrap();
		let mut settings = ReminderSettings {
			enabled: true,
			..ReminderSettings::default()
		};
		settings.travel.sound = Sound::Tone { tone: Tone::Bell };
		settings.events.minutes_before = 25;

		save_reminders_at(&path, &settings).unwrap();

		let config = Config::load_from(&path);
		assert_eq!(config.reminders, settings);
		assert!(config.travel.show);
		save_reminders_at(&path, &ReminderSettings::default()).unwrap();
		assert_eq!(
			Config::load_from(&path).reminders,
			ReminderSettings::default()
		);
	}

	#[test]
	fn saving_places_reuses_a_saved_place_and_links_events() {
		let (_directory, path) = database();
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

		let saved = Config::load_from(&path);
		assert_eq!(saved.travel.places.len(), 1);
		assert_eq!(saved.travel.places[0].address, "1 Main St");
		assert_eq!(saved.travel.event_places.len(), 1);
		assert_eq!(saved.travel.event_places["standup"], id);
	}

	#[test]
	fn saving_visibility_keeps_other_settings() {
		let (_directory, path) = database();
		save_google_client_at(&path, "id", "").unwrap();
		let settings = CalendarSettings {
			disabled: vec!["timeparagon:habits".into(), "work".into()],
			show_tasks: false,
			..CalendarSettings::default()
		};
		save_calendar_visibility_at(&path, &settings).unwrap();

		let saved = Config::load_from(&path);
		assert_eq!(saved.calendar.disabled, settings.disabled);
		assert!(!saved.calendar.show_tasks);
		assert_eq!(saved.calendar.client_id, "id");
	}
}
