//! Markdown notes, one `.md` file per note in the directory set by
//! `notes.directory` setting, which each device has its own of.
//!
//! A note's file name is its title. Front matter holds its stable ID; color
//! and connections are stored in the database and replicated by database sync.

#[cfg(target_os = "android")]
pub mod android;
mod files;
pub mod markdown;
mod store;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

pub use crate::calendar::identity::{Occurrence, Reach};

/// The name a note without a title is stored and shown under.
pub const UNTITLED: &str = "Untitled";

/// The light pastels a note's background can be. It is deliberately a closed
/// set: text on a note is always dark, so every color has to stay light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pastel {
	#[default]
	Butter,
	Peach,
	Rose,
	Lilac,
	Lavender,
	Sky,
	Aqua,
	Mint,
}

impl Pastel {
	pub const ALL: [Self; 8] = [
		Self::Butter,
		Self::Peach,
		Self::Rose,
		Self::Lilac,
		Self::Lavender,
		Self::Sky,
		Self::Aqua,
		Self::Mint,
	];

	pub fn name(self) -> &'static str {
		match self {
			Self::Butter => "Butter",
			Self::Peach => "Peach",
			Self::Rose => "Rose",
			Self::Lilac => "Lilac",
			Self::Lavender => "Lavender",
			Self::Sky => "Sky",
			Self::Aqua => "Aqua",
			Self::Mint => "Mint",
		}
	}

	pub fn rgb(self) -> (u8, u8, u8) {
		match self {
			Self::Butter => (0xFB, 0xF3, 0xC1),
			Self::Peach => (0xFD, 0xDF, 0xC8),
			Self::Rose => (0xFB, 0xD5, 0xDC),
			Self::Lilac => (0xF4, 0xD9, 0xF2),
			Self::Lavender => (0xE3, 0xDA, 0xF7),
			Self::Sky => (0xD3, 0xE4, 0xFA),
			Self::Aqua => (0xCD, 0xEE, 0xF0),
			Self::Mint => (0xCF, 0xF0, 0xDC),
		}
	}
}

/// What kind of thing a note is connected to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
	Event,
	TimeBlock,
	Task,
	/// A habit in the Habits list, never one of its calendar events, so the
	/// note shows on every event series connected to the habit.
	Habit,
}

impl LinkKind {
	pub fn label(self) -> &'static str {
		match self {
			Self::Event => "Event",
			Self::TimeBlock => "Time block",
			Self::Task => "Task",
			Self::Habit => "Habit",
		}
	}
}

/// One thing a note is connected to. The title and day are remembered so the
/// link still reads properly when that day's events are not loaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
	pub kind: LinkKind,
	pub id: String,
	/// Retain the occurrence's lineage and original day across series edits.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub occurrence: Option<Occurrence>,
	#[serde(default)]
	pub title: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub day: Option<NaiveDate>,
}

/// Events and time blocks are one kind of thing to connect to: an event can
/// become a time block and back.
fn calendar_item(kind: LinkKind) -> bool {
	matches!(kind, LinkKind::Event | LinkKind::TimeBlock)
}

fn comparable(a: LinkKind, b: LinkKind) -> bool {
	a == b || (calendar_item(a) && calendar_item(b))
}

impl Link {
	/// How far along its series this connection reaches.
	pub fn reach(&self) -> Reach {
		self.occurrence
			.as_ref()
			.map_or(Reach::One, |occurrence| occurrence.reach)
	}

	/// How far along its series a connection reaches, to read after its
	/// title, with days written in `format`. `None` for one occurrence.
	pub fn reach_text(&self, format: &str) -> Option<String> {
		let occurrence = self.occurrence.as_ref()?;
		let mut text = match occurrence.reach {
			Reach::One => return None,
			Reach::Following => format!("from {}", occurrence.day.format(format)),
			Reach::All => "every occurrence".to_owned(),
		};
		if let Some(until) = occurrence.until {
			text.push_str(&format!(" until {}", until.format(format)));
		}
		Some(text)
	}

	/// Whether this connection takes in `target`: the same item, or an
	/// occurrence this reaches along its series.
	pub fn covers(&self, target: &Self) -> bool {
		match &self.occurrence {
			Some(own) if own.reach != Reach::One && calendar_item(self.kind) => {
				calendar_item(target.kind)
					&& match &target.occurrence {
						Some(other) => own.covers(other),
						None => own.matches_id(&target.id, target.day),
					}
			}
			_ => self.same_target(target),
		}
	}

	/// Whether this connection takes in everything `other` does.
	fn subsumes(&self, other: &Self) -> bool {
		if !comparable(self.kind, other.kind) {
			return false;
		}
		match (&self.occurrence, &other.occurrence) {
			(Some(own), Some(theirs)) if calendar_item(self.kind) => own.subsumes(theirs),
			(Some(own), None) if calendar_item(self.kind) => {
				own.matches_id(&other.id, other.day) || self.id == other.id
			}
			_ => other.reach() == Reach::One && self.same_target(other),
		}
	}

	/// What is left of this connection once `cut`, an occurrence of
	/// `target` reaching as the save's scope says, is disconnected.
	fn without(self, target: &Self, cut: &Occurrence) -> Option<Self> {
		if !calendar_item(self.kind) || !calendar_item(target.kind) {
			return (!self.same_target(target)).then_some(self);
		}
		match self.occurrence.clone() {
			Some(own) => own.without(cut).map(|occurrence| Self {
				occurrence: Some(occurrence),
				..self
			}),
			None => {
				let gone = self.same_target(target) || cut.matches_id(&self.id, self.day);
				(!gone).then_some(self)
			}
		}
	}

	pub fn same_target(&self, other: &Self) -> bool {
		if !comparable(self.kind, other.kind) {
			return false;
		}
		if self.id == other.id {
			return true;
		}
		if !calendar_item(self.kind) {
			return false;
		}
		match (&self.occurrence, &other.occurrence) {
			(Some(a), Some(b)) => a.same(b),
			(Some(a), None) => a.matches_id(&other.id, other.day),
			(None, Some(b)) => b.matches_id(&self.id, self.day),
			(None, None) => false,
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
	/// The stable ID saved in Markdown front matter and used as the database key.
	pub id: String,
	pub title: String,
	#[serde(default)]
	pub body: String,
	#[serde(default)]
	pub color: Pastel,
	#[serde(default)]
	pub links: Vec<Link>,
	/// When it last changed, for sorting.
	#[serde(default)]
	pub modified_ms: i64,
	/// Counts local edits.
	#[serde(default)]
	pub revision: u64,
	/// The file it was last read from or saved to, within the directory.
	#[serde(skip)]
	pub file: Option<String>,
	/// Front matter has unused fields to remove when the file is rewritten.
	#[serde(skip)]
	pub(crate) cleanup: bool,
	/// The Markdown reader found legacy metadata that needs importing.
	#[serde(skip)]
	pub(crate) migrate: bool,
}

impl Note {
	/// The title to show, which is never blank.
	pub fn display_title(&self) -> &str {
		match self.title.trim() {
			"" => UNTITLED,
			title => title,
		}
	}

	pub fn is_linked(&self, link: &Link) -> bool {
		self.links.iter().any(|own| own.covers(link))
	}

	/// Nothing the user would miss: an abandoned "New note".
	pub fn is_blank(&self) -> bool {
		self.title.trim().is_empty() && self.body.trim().is_empty() && self.links.is_empty()
	}

	/// Whether what is saved of the two is the same.
	fn same_content(&self, other: &Self) -> bool {
		self.title == other.title
			&& self.body == other.body
			&& self.color == other.color
			&& self.links == other.links
	}
}

/// A unique ID for a new note.
pub fn local_id() -> String {
	static COUNTER: AtomicU64 = AtomicU64::new(0);
	format!(
		"local-{:x}-{:x}",
		crate::clock::now().timestamp_micros(),
		COUNTER.fetch_add(1, Ordering::Relaxed)
	)
}

/// The configured notes directory, or `None` while none is set.
pub fn directory() -> Option<PathBuf> {
	crate::config::NotesSettings::load().path()
}

/// Opens the saved Markdown file using the device's associated app.
pub fn open_file(directory: &Path, file: &str) -> std::io::Result<()> {
	if Path::new(file).file_name().and_then(|name| name.to_str()) != Some(file) {
		return Err(std::io::Error::other("Invalid note filename"));
	}
	#[cfg(target_os = "android")]
	return android::open(directory, file);
	#[cfg(not(target_os = "android"))]
	{
		let path = directory.join(file).canonicalize()?;
		#[cfg(target_os = "macos")]
		let mut command = std::process::Command::new("open");
		#[cfg(target_os = "windows")]
		let mut command = std::process::Command::new("explorer");
		#[cfg(not(any(target_os = "macos", target_os = "windows")))]
		let mut command = std::process::Command::new("xdg-open");
		if command.arg(path).status()?.success() {
			Ok(())
		} else {
			Err(std::io::Error::other(
				"Could not open the note in its default app",
			))
		}
	}
}

/// What a save did, for [`Library::saved`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Saved {
	/// Each note's file after the save, by note id.
	pub files: Vec<(String, String)>,
	/// The files of deleted notes that are now gone.
	pub removed: Vec<String>,
}

/// The notes in the directory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Library {
	#[serde(default)]
	pub notes: Vec<Note>,
	/// Files of deleted notes, removed by the next save.
	#[serde(skip)]
	pub removed: Vec<String>,
	/// IDs whose metadata is removed after their files are successfully deleted.
	#[serde(skip)]
	pub removed_ids: Vec<String>,
}

impl Library {
	/// Reads the notes in `directory`. A directory that does not exist yet
	/// has no notes.
	pub fn load(directory: &Path) -> Self {
		Self::read(directory).unwrap_or_else(|error| {
			log::warn!(
				"the notes in {} could not be read: {error}",
				directory.display()
			);
			Self::default()
		})
	}

	/// [`Self::load`], with the error.
	pub fn read(directory: &Path) -> std::io::Result<Self> {
		let mut database = crate::task_db::open().map_err(std::io::Error::other)?;
		store::read(directory, database.connection())
	}

	/// Saves metadata before writing files, so a failed database write never
	/// strips the only copy of legacy front matter.
	pub fn save(&self, directory: &Path) -> std::io::Result<Saved> {
		let mut database = crate::task_db::open().map_err(std::io::Error::other)?;
		store::save(self, directory, database.connection())
	}

	fn read_files(directory: &Path) -> std::io::Result<Self> {
		#[cfg(target_os = "android")]
		if android::is_tree(directory) {
			return android::read(directory);
		}
		files::read(directory)
	}

	fn write_files(&self, directory: &Path) -> std::io::Result<Saved> {
		#[cfg(target_os = "android")]
		if android::is_tree(directory) {
			return android::save(self, directory);
		}
		files::write(self, directory)
	}

	/// Records what a save of a copy of this library did.
	pub fn saved(&mut self, saved: &Saved) {
		for (id, file) in &saved.files {
			if let Some(note) = self.notes.iter_mut().find(|note| &note.id == id) {
				note.file = Some(file.clone());
			}
		}
		self.removed.retain(|file| !saved.removed.contains(file));
		if self.removed.is_empty() {
			self.removed_ids.clear();
		}
	}

	/// Carries what is only known in memory (revisions and the title as
	/// typed rather than as the file name has it) from `previous` over to this
	/// freshly read copy, matching notes by stable ID.
	pub fn adopt_identities(&mut self, previous: &Self) {
		for note in &mut self.notes {
			let Some(old) = previous.notes.iter().find(|old| old.id == note.id) else {
				continue;
			};
			note.revision = old.revision;
			if files::file_stem(&old.title) == note.title {
				note.title = old.title.clone();
			}
			if note.same_content(old) {
				note.modified_ms = old.modified_ms;
			}
		}
	}

	pub fn note(&self, id: &str) -> Option<&Note> {
		self.notes.iter().find(|note| note.id == id)
	}

	/// Starts a note and returns its id.
	pub fn create(&mut self, title: &str, links: Vec<Link>) -> String {
		let id = local_id();
		self.notes.push(Note {
			id: id.clone(),
			title: title.to_owned(),
			body: String::new(),
			color: Pastel::default(),
			links,
			modified_ms: crate::clock::now().timestamp_millis(),
			revision: 1,
			file: None,
			cleanup: false,
			migrate: false,
		});
		id
	}

	/// Changes a note. Returns whether it exists.
	pub fn edit(&mut self, id: &str, change: impl FnOnce(&mut Note)) -> bool {
		let Some(note) = self.notes.iter_mut().find(|note| note.id == id) else {
			return false;
		};
		change(note);
		note.revision += 1;
		note.modified_ms = crate::clock::now().timestamp_millis();
		true
	}

	/// Removes a note. The next save removes its file.
	pub fn delete(&mut self, id: &str) {
		let Some(index) = self.notes.iter().position(|note| note.id == id) else {
			return;
		};
		let note = self.notes.remove(index);
		self.removed_ids.push(note.id);
		if let Some(file) = note.file {
			self.removed.push(file);
		}
	}

	/// Connects a note to something, once.
	pub fn connect(&mut self, id: &str, link: Link) {
		if self.note(id).is_some_and(|note| !note.is_linked(&link)) {
			self.edit(id, |note| note.links.push(link));
		}
	}

	pub fn disconnect(&mut self, id: &str, link: &Link) {
		if self.note(id).is_some_and(|note| note.is_linked(link)) {
			self.edit(id, |note| note.links.retain(|own| !own.same_target(link)));
		}
	}

	/// Connects a note to `target` and, when it is an occurrence of a series,
	/// to as much more of the series as `reach` says.
	pub fn connect_reaching(&mut self, id: &str, target: Link, reach: Reach) {
		let Some(occurrence) = target.occurrence.as_ref().filter(|_| reach != Reach::One) else {
			return self.connect(id, target);
		};
		let wide = Link {
			occurrence: Some(occurrence.reaching(reach)),
			..target
		};
		let Some(note) = self.note(id) else {
			return;
		};
		if note.links.iter().any(|own| own.subsumes(&wide)) {
			return;
		}
		self.edit(id, |note| {
			note.links.retain(|own| !wide.subsumes(own));
			note.links.push(wide);
		});
	}

	/// Disconnects a note from `target` and, when it is an occurrence of a
	/// series, from as much more of the series as `reach` says. A connection
	/// reaching further along the series keeps the rest.
	pub fn disconnect_reaching(&mut self, id: &str, target: &Link, reach: Reach) {
		let Some(occurrence) = &target.occurrence else {
			return self.disconnect(id, target);
		};
		let cut = occurrence.reaching(reach);
		let Some(note) = self.note(id) else {
			return;
		};
		let links: Vec<Link> = note
			.links
			.iter()
			.cloned()
			.filter_map(|link| link.without(target, &cut))
			.collect();
		if links != note.links {
			self.edit(id, |note| note.links = links);
		}
	}

	/// Applies the connections an editor held until it was saved, by note ID:
	/// `true` to connect, `false` to disconnect.
	pub fn apply_staged(&mut self, staged: &[(String, bool)], target: &Link, reach: Reach) {
		for (id, connect) in staged {
			if *connect {
				self.connect_reaching(id, target.clone(), reach);
			} else {
				self.disconnect_reaching(id, target, reach);
			}
		}
	}

	/// The notes connected to something, newest first.
	pub fn linked(&self, link: &Link) -> Vec<&Note> {
		let mut notes: Vec<&Note> = self
			.notes
			.iter()
			.filter(|note| note.is_linked(link))
			.collect();
		notes.sort_by_key(|note| std::cmp::Reverse(note.modified_ms));
		notes
	}

	/// The notes matching a search of their titles and text, newest first.
	pub fn search(&self, query: &str) -> Vec<&Note> {
		let query = query.trim().to_lowercase();
		let mut notes: Vec<&Note> = self
			.notes
			.iter()
			.filter(|note| {
				query.is_empty()
					|| note.display_title().to_lowercase().contains(&query)
					|| note.body.to_lowercase().contains(&query)
			})
			.collect();
		notes.sort_by_key(|note| std::cmp::Reverse(note.modified_ms));
		notes
	}
}

/// A cheap summary of the directory's Markdown files (their names, sizes and
/// modification times) and database metadata counter, for polling changes.
pub fn fingerprint(directory: &Path) -> u64 {
	use std::hash::{Hash, Hasher};
	let mut hash = std::collections::hash_map::DefaultHasher::new();
	let files = {
		#[cfg(target_os = "android")]
		if android::is_tree(directory) {
			android::fingerprint(directory)
		} else {
			files::fingerprint(directory)
		}
		#[cfg(not(target_os = "android"))]
		files::fingerprint(directory)
	};
	files.hash(&mut hash);
	crate::task_db::path()
		.and_then(|path| crate::local_changes::read(&path).ok())
		.map(|counters| counters.notes)
		.hash(&mut hash);
	hash.finish()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn habit() -> Link {
		Link {
			kind: LinkKind::Habit,
			occurrence: None,
			id: "h1".into(),
			title: "Read".into(),
			day: None,
		}
	}

	fn names(directory: &Path) -> Vec<String> {
		let mut names: Vec<String> = std::fs::read_dir(directory)
			.unwrap()
			.map(|entry| entry.unwrap().file_name().into_string().unwrap())
			.collect();
		names.sort();
		names
	}

	/// Saves and records the save, as the apps do.
	fn save(library: &mut Library, directory: &Path) {
		let saved = library.save(directory).unwrap();
		library.saved(&saved);
	}

	#[test]
	fn opening_rejects_paths_outside_the_notes_folder() {
		assert!(open_file(Path::new("/tmp"), "../private.md").is_err());
		assert!(open_file(Path::new("/tmp"), "/private.md").is_err());
	}

	#[test]
	fn every_pastel_is_light_and_soft() {
		for color in Pastel::ALL {
			let (r, g, b) = color.rgb();
			let (low, high) = (r.min(g).min(b), r.max(g).max(b));
			assert!(low >= 0xC0, "{} is too dark", color.name());
			assert!(high - low <= 0x40, "{} is too saturated", color.name());
		}
	}

	#[test]
	fn connections_are_unique_and_search_includes_note_text() {
		let mut library = Library::default();
		let id = library.create("Reading", Vec::new());
		library.connect(&id, habit());
		library.connect(&id, habit());
		assert_eq!(library.linked(&habit()).len(), 1);
		assert_eq!(library.note(&id).unwrap().links.len(), 1);
		library.edit(&id, |note| note.body = "A paper about SQLite".into());
		assert_eq!(library.search("sqlite")[0].id, id);
		library.disconnect(&id, &habit());
		assert!(library.linked(&habit()).is_empty());
	}

	#[test]
	fn occurrence_notes_survive_retiming_splitting_and_restart() {
		use crate::calendar::{at_clock, identity::Occurrence};
		use chrono::Duration;
		let day = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
		let mut block = crate::calendar::sample_snapshot().events.remove(0);
		block.calendar_id = "app".into();
		block.all_day = false;
		block.start = at_clock(day, 9, 0);
		block.end = at_clock(day, 10, 0);
		block.details = Default::default();
		block.details.recurring_event_id = Some("series".into());
		block.details.original_start = Some(block.start);
		block.id = format!("series_{}", block.start.to_utc().format("%Y%m%dT%H%M%SZ"));
		let target = |block: &crate::calendar::Event| Link {
			kind: LinkKind::TimeBlock,
			id: block.id.clone(),
			occurrence: Occurrence::of(block),
			title: block.title.clone(),
			day: Some(block.start.date_naive()),
		};
		// Notes saved before occurrence identity was added must recover too.
		let mut legacy = target(&block);
		legacy.occurrence = None;
		let mut library = Library::default();
		let id = library.create("Plan", vec![legacy]);
		library.edit(&id, |note| {
			note.body = "Keep this attached".into();
			note.color = Pastel::Mint;
		});
		let directory = tempfile::tempdir().unwrap();
		save(&mut library, directory.path());
		let mut library = Library::read(directory.path()).unwrap();

		for series in ["series", "new-series"] {
			block.start = at_clock(day, 11, 0);
			block.end = at_clock(day, 12, 30);
			block.details.original_start = Some(block.start);
			block.details.recurring_event_id = Some(series.into());
			block.details.split_from = vec!["app/series".into()];
			block.id = format!("{series}_{}", block.start.to_utc().format("%Y%m%dT%H%M%SZ"));
			let link = target(&block);
			assert_eq!(library.linked(&link).len(), 1, "{series}");
			library.connect(&id, link.clone());
			assert_eq!(library.note(&id).unwrap().links.len(), 1);
			// Moving just this occurrence to another day keeps its original slot.
			block.start += Duration::days(2);
			block.end += Duration::days(2);
			assert_eq!(library.linked(&target(&block)).len(), 1);
			// The following occurrence must never inherit this one's note.
			let mut next = block.clone();
			next.details.original_start = Some(at_clock(day.succ_opt().unwrap(), 11, 0));
			next.id = format!(
				"{series}_{}",
				next.details
					.original_start
					.unwrap()
					.to_utc()
					.format("%Y%m%dT%H%M%SZ")
			);
			assert!(library.linked(&target(&next)).is_empty());
		}
		let note = library.note(&id).unwrap();
		assert_eq!(note.body, "Keep this attached");
		assert_eq!(note.color, Pastel::Mint);
		library.disconnect(&id, &target(&block));
		assert!(library.note(&id).unwrap().links.is_empty());
	}

	#[test]
	fn event_notes_survive_a_change_between_event_and_time_block() {
		let mut link = habit();
		link.kind = LinkKind::TimeBlock;
		let mut library = Library::default();
		let id = library.create("Plan", vec![link.clone()]);
		link.kind = LinkKind::Event;
		assert_eq!(library.linked(&link)[0].id, id);
		link.kind = LinkKind::Task;
		assert!(library.linked(&link).is_empty());
	}

	/// The connection target of a block series' occurrence on day `n`.
	fn block_on(series: &[&str], n: u32) -> Link {
		let day = NaiveDate::from_ymd_opt(2026, 11, n).unwrap();
		Link {
			kind: LinkKind::TimeBlock,
			id: format!("{}_202611{n:02}T090000Z", series.last().unwrap()),
			occurrence: Some(Occurrence {
				series: series.iter().map(|s| format!("app/{s}")).collect(),
				day,
				reach: Reach::One,
				until: None,
				except: Vec::new(),
			}),
			title: "Deep work".into(),
			day: Some(day),
		}
	}

	fn shown_on(library: &Library, days: &[u32]) -> Vec<u32> {
		days.iter()
			.copied()
			.filter(|n| !library.linked(&block_on(&["s"], *n)).is_empty())
			.collect()
	}

	#[test]
	fn a_note_follows_the_scope_it_was_saved_with() {
		let days = [3, 10, 17, 24];
		for (reach, expected) in [
			(Reach::One, vec![10]),
			(Reach::Following, vec![10, 17, 24]),
			(Reach::All, vec![3, 10, 17, 24]),
		] {
			let mut library = Library::default();
			let id = library.create("Plan", Vec::new());
			library.apply_staged(&[(id.clone(), true)], &block_on(&["s"], 10), reach);
			assert_eq!(shown_on(&library, &days), expected, "{reach:?}");
			// A series split off later by "this and following" keeps it.
			if reach != Reach::One {
				assert!(!library.linked(&block_on(&["s", "t"], 30)).is_empty());
			}
			assert!(library.linked(&block_on(&["other"], 10)).is_empty());
		}
	}

	#[test]
	fn a_connection_says_how_far_along_its_series_it_reaches() {
		let mut library = Library::default();
		let id = library.create("Plan", Vec::new());
		library.connect_reaching(&id, block_on(&["s"], 10), Reach::Following);
		library.disconnect_reaching(&id, &block_on(&["s"], 24), Reach::Following);
		let link = &library.note(&id).unwrap().links[0];
		assert_eq!(
			link.reach_text("%b %-d").as_deref(),
			Some("from Nov 10 until Nov 23")
		);
		assert_eq!(block_on(&["s"], 10).reach_text("%b %-d"), None);
		library.connect_reaching(&id, block_on(&["s"], 10), Reach::All);
		let link = &library.note(&id).unwrap().links[0];
		assert_eq!(
			link.reach_text("%b %-d").as_deref(),
			Some("every occurrence")
		);
	}

	#[test]
	fn a_wider_connection_replaces_what_it_takes_in() {
		let mut library = Library::default();
		// A new note starts on its one occurrence; saving widens it.
		let id = library.create("Plan", vec![block_on(&["s"], 10)]);
		library.connect_reaching(&id, block_on(&["s"], 17), Reach::Following);
		library.connect_reaching(&id, block_on(&["s"], 10), Reach::Following);
		assert_eq!(library.note(&id).unwrap().links.len(), 1);
		library.connect_reaching(&id, block_on(&["s"], 24), Reach::Following);
		library.connect_reaching(&id, block_on(&["s"], 24), Reach::One);
		assert_eq!(library.note(&id).unwrap().links.len(), 1);
		library.connect_reaching(&id, block_on(&["s"], 3), Reach::All);
		let links = &library.note(&id).unwrap().links;
		assert_eq!(links.len(), 1);
		assert_eq!(links[0].reach(), Reach::All);
	}

	#[test]
	fn disconnecting_with_a_scope_keeps_the_rest_of_the_series() {
		let days = [3, 10, 17, 24, 30];
		let mut library = Library::default();
		let id = library.create("Plan", Vec::new());
		library.connect_reaching(&id, block_on(&["s"], 3), Reach::All);
		library.disconnect_reaching(&id, &block_on(&["s"], 17), Reach::One);
		assert_eq!(shown_on(&library, &days), [3, 10, 24, 30]);
		library.disconnect_reaching(&id, &block_on(&["s"], 24), Reach::Following);
		assert_eq!(shown_on(&library, &days), [3, 10]);
		// Connecting the one occurrence again puts back just that one.
		library.connect_reaching(&id, block_on(&["s"], 17), Reach::One);
		assert_eq!(shown_on(&library, &days), [3, 10, 17]);
		library.disconnect_reaching(&id, &block_on(&["s"], 10), Reach::All);
		assert_eq!(shown_on(&library, &days), Vec::<u32>::new());
		assert!(library.note(&id).unwrap().links.is_empty());
		// Plain links, and links to other things, are left as they were.
		library.connect(&id, habit());
		library.connect_reaching(&id, block_on(&["other"], 10), Reach::All);
		library.disconnect_reaching(&id, &block_on(&["s"], 10), Reach::All);
		assert_eq!(library.note(&id).unwrap().links.len(), 2);
	}

	#[test]
	fn the_last_staged_choice_for_a_note_wins() {
		let mut draft = crate::calendar::EditDraft::from_event(
			&crate::calendar::sample_snapshot().events.remove(0),
		);
		draft.stage_note("a", true);
		draft.stage_note("b", true);
		draft.stage_note("a", false);
		assert_eq!(draft.staged_note("a"), Some(false));
		assert_eq!(draft.staged_note("b"), Some(true));
		assert_eq!(draft.staged_note("c"), None);
		let mut library = Library::default();
		let a = library.create("A", vec![block_on(&["s"], 10)]);
		let b = library.create("B", Vec::new());
		library.apply_staged(
			&[(a.clone(), false), (b.clone(), true)],
			&block_on(&["s"], 17),
			Reach::Following,
		);
		assert_eq!(library.note(&a).unwrap().links.len(), 1, "before the cut");
		assert!(library.linked(&block_on(&["s"], 17))[0].id == b);
	}

	#[test]
	fn each_note_is_a_markdown_file_named_after_its_title() {
		let directory = tempfile::tempdir().unwrap();
		let mut library = Library::default();
		let plain = library.create("Plain", Vec::new());
		library.edit(&plain, |note| note.body = "# Hello\n\nworld\n".into());
		let rich = library.create("Rich", vec![habit()]);
		library.edit(&rich, |note| {
			note.color = Pastel::Rose;
			note.body = "text".into();
		});
		save(&mut library, directory.path());

		assert_eq!(names(directory.path()), ["Plain.md", "Rich.md"]);
		let plain_text = std::fs::read_to_string(directory.path().join("Plain.md")).unwrap();
		assert_eq!(
			plain_text,
			format!("---\nid: \"{plain}\"\n---\n# Hello\n\nworld\n")
		);
		let rich_text = std::fs::read_to_string(directory.path().join("Rich.md")).unwrap();
		assert_eq!(rich_text, format!("---\nid: \"{rich}\"\n---\ntext"));

		let mut read = Library::load(directory.path());
		read.adopt_identities(&library);
		assert_eq!(read.notes.len(), 2);
		let again = read.note(&rich).unwrap();
		assert_eq!(again.color, Pastel::Rose);
		assert_eq!(again.links, vec![habit()]);
		assert_eq!(again.body, "text");
		assert_eq!(read.note(&plain).unwrap().body, "# Hello\n\nworld\n");
	}

	#[test]
	fn renaming_moves_the_file_and_deleting_removes_it() {
		let directory = tempfile::tempdir().unwrap();
		let mut library = Library::default();
		let id = library.create("", Vec::new());
		save(&mut library, directory.path());
		assert_eq!(names(directory.path()), ["Untitled.md"]);

		library.edit(&id, |note| note.title = "Trip: plans/ideas".into());
		save(&mut library, directory.path());
		assert_eq!(names(directory.path()), ["Trip- plans-ideas.md"]);

		// Reading it back keeps the title as typed.
		let mut read = Library::load(directory.path());
		read.adopt_identities(&library);
		assert_eq!(read.note(&id).unwrap().title, "Trip: plans/ideas");

		library.delete(&id);
		save(&mut library, directory.path());
		assert!(names(directory.path()).is_empty());
		assert!(library.removed.is_empty());
	}

	#[test]
	fn notes_with_the_same_title_and_other_files_are_kept_apart() {
		let directory = tempfile::tempdir().unwrap();
		std::fs::write(directory.path().join("Idea.md"), "made elsewhere").unwrap();
		std::fs::write(directory.path().join("picture.png"), "not a note").unwrap();
		let mut library = Library::default();
		library.create("Idea", Vec::new());
		library.create("Idea", Vec::new());
		save(&mut library, directory.path());
		assert_eq!(
			names(directory.path()),
			["Idea 2.md", "Idea 3.md", "Idea.md", "picture.png"]
		);
		let text = std::fs::read_to_string(directory.path().join("Idea.md")).unwrap();
		assert_eq!(text, "made elsewhere");
		assert_eq!(Library::load(directory.path()).notes.len(), 3);
	}

	#[test]
	fn unused_front_matter_is_removed() {
		let directory = tempfile::tempdir().unwrap();
		std::fs::write(
            directory.path().join("Mixed.md"),
            "---\ntags:\n  - life\ncolor: Mint\nlinks:\n  - kind: habit\n    id: h1\n    title: \"Read\"\n---\nbody\n",
        )
        .unwrap();
		let mut library = Library::load(directory.path());
		let note = &library.notes[0];
		assert_eq!(note.color, Pastel::Mint);
		assert_eq!(note.links, vec![habit()]);
		assert_eq!(note.body, "body\n");
		let id = note.id.clone();
		library.edit(&id, |note| note.body = "changed\n".into());
		save(&mut library, directory.path());
		let text = std::fs::read_to_string(directory.path().join("Mixed.md")).unwrap();
		assert_eq!(text, format!("---\nid: \"{id}\"\n---\nchanged\n"));
		assert!(text.ends_with("---\nchanged\n"), "{text}");
	}

	#[test]
	fn a_note_starting_with_a_rule_is_not_read_as_front_matter() {
		let directory = tempfile::tempdir().unwrap();
		let mut library = Library::default();
		let id = library.create("Rules", Vec::new());
		library.edit(&id, |note| {
			note.body = "---\nnot: front matter\n---\nend".into()
		});
		save(&mut library, directory.path());
		let read = Library::load(directory.path());
		assert_eq!(read.notes[0].body, "---\nnot: front matter\n---\nend");
	}

	#[test]
	fn reading_again_matches_what_is_in_memory() {
		let directory = tempfile::tempdir().unwrap();
		let mut library = Library::default();
		let id = library.create("Same", vec![habit()]);
		library.edit(&id, |note| note.body = "x".into());
		save(&mut library, directory.path());
		let mut read = Library::load(directory.path());
		read.adopt_identities(&library);
		assert_eq!(read, library);
		assert_ne!(fingerprint(directory.path()), 0);
	}
}
