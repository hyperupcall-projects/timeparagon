//! Markdown notes, one `.md` file per note in the directory set by
//! `[notes] directory` in `config.toml`.
//!
//! A note's file name is its title. Its color and connections, when it has
//! any, are kept in front matter at the top of the file (see [`files`]), so
//! the directory reads well in any other Markdown editor. Nothing about notes
//! is kept in the database.

#[cfg(target_os = "android")]
pub mod android;
mod files;
pub mod markdown;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

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
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day: Option<NaiveDate>,
}

impl Link {
    pub fn same_target(&self, other: &Self) -> bool {
        self.kind == other.kind && self.id == other.id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// The stable ID used by the editor while the app runs. It is not saved:
    /// a note read from disk is identified by its file name.
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
    /// Front matter lines this app does not use, such as tags added in
    /// another editor. They are written back as they were.
    #[serde(skip)]
    pub extra: String,
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
        self.links.iter().any(|own| own.same_target(link))
    }

    /// Nothing the user would miss: an abandoned "New note".
    pub fn is_blank(&self) -> bool {
        self.title.trim().is_empty()
            && self.body.trim().is_empty()
            && self.links.is_empty()
            && self.extra.trim().is_empty()
    }

    /// Whether what is saved of the two is the same.
    fn same_content(&self, other: &Self) -> bool {
        self.title == other.title
            && self.body == other.body
            && self.color == other.color
            && self.links == other.links
            && self.extra == other.extra
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
        #[cfg(target_os = "android")]
        if android::is_tree(directory) {
            return android::read(directory);
        }
        files::read(directory)
    }

    /// Writes every note to its file, renaming the files of retitled notes and
    /// removing those of deleted ones. Files this library does not know about,
    /// such as ones made in another editor since it was read, are left alone.
    pub fn save(&self, directory: &Path) -> std::io::Result<Saved> {
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
    }

    /// Carries what is only known in memory (ids, revisions, and the title as
    /// typed rather than as the file name has it) from `previous` over to this
    /// freshly read copy, matching notes by file.
    pub fn adopt_identities(&mut self, previous: &Self) {
        for note in &mut self.notes {
            let Some(old) = previous
                .notes
                .iter()
                .find(|old| old.file.is_some() && old.file == note.file)
            else {
                continue;
            };
            note.id = old.id.clone();
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
            extra: String::new(),
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
        if let Some(file) = self.notes.remove(index).file {
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
/// modification times), so a poll only rereads the notes when it changes.
pub fn fingerprint(directory: &Path) -> u64 {
    #[cfg(target_os = "android")]
    if android::is_tree(directory) {
        return android::fingerprint(directory);
    }
    files::fingerprint(directory)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn habit() -> Link {
        Link {
            kind: LinkKind::Habit,
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
        assert_eq!(plain_text, "# Hello\n\nworld\n");
        let rich_text = std::fs::read_to_string(directory.path().join("Rich.md")).unwrap();
        assert_eq!(
            rich_text,
            "---\ncolor: rose\nlinks:\n  - {\"kind\":\"habit\",\"id\":\"h1\",\"title\":\"Read\"}\n---\ntext"
        );

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
    fn front_matter_from_another_editor_is_kept() {
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
        assert!(text.contains("tags:\n  - life\n"), "{text}");
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
        assert!(read.notes[0].extra.is_empty());
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
