//! Markdown notes stored in SQLite and replicated through Turso.
//! Local saves enter the database edit journal; the sync worker handles delivery.

pub mod markdown;

use std::path::Path;
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
    /// The stable ID used by the editor and note connections.
    pub id: String,
    /// Retained for existing Turso item identities; never used to access Drive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drive_id: Option<String>,
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
        self.drive_id.is_none()
            && self.title.trim().is_empty()
            && self.body.trim().is_empty()
            && self.links.is_empty()
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

/// The notes on this device. Deletions are recorded by the database journal.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub notes: Vec<Note>,
}

impl Library {
    /// Loads the local database copy.
    pub fn load(path: &Path) -> Self {
        crate::task_db::read_document(path, crate::task_db::keys::NOTES)
            .unwrap_or_else(|error| {
                log::warn!("the saved notes could not be read: {error}");
                None
            })
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        crate::task_db::write_document(path, crate::task_db::keys::NOTES, self)
    }

    pub fn note(&self, id: &str) -> Option<&Note> {
        self.notes.iter().find(|note| note.id == id)
    }

    /// Starts a note and returns its id.
    pub fn create(&mut self, title: &str, links: Vec<Link>) -> String {
        let id = local_id();
        self.notes.push(Note {
            id: id.clone(),
            drive_id: None,
            title: title.to_owned(),
            body: String::new(),
            color: Pastel::default(),
            links,
            modified_ms: crate::clock::now().timestamp_millis(),
            revision: 1,
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

    /// Removes a note. Saving the library journals the deletion.
    pub fn delete(&mut self, id: &str) {
        let Some(index) = self.notes.iter().position(|note| note.id == id) else {
            return;
        };
        self.notes.remove(index);
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
    fn local_saves_and_deletions_enter_the_turso_journal() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.sqlite3");
        crate::task_db::Database::open(&path).unwrap();
        let mut library = Library::default();
        let id = library.create("A", vec![habit()]);
        library.save(&path).unwrap();
        assert_eq!(Library::load(&path), library);
        assert_eq!(crate::database_sync::history(&path).unwrap().len(), 1);
        library.delete(&id);
        library.save(&path).unwrap();
        assert!(Library::load(&path).notes.is_empty());
        assert_eq!(crate::database_sync::history(&path).unwrap().len(), 2);
    }

    #[test]
    fn existing_notes_keep_their_replication_identity_after_editing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.sqlite3");
        crate::task_db::write_document_json(&path, crate::task_db::keys::NOTES,
            r#"{"notes":[{"id":"local-1","drive_id":"imported-1","title":"A","body":"keep","dirty":true,"synced_ms":123}],"deleted":["gone"]}"#).unwrap();
        let mut library = Library::load(&path);
        assert_eq!(library.notes[0].body, "keep");
        library.edit("local-1", |note| note.title = "Edited".into());
        library.save(&path).unwrap();
        let edits = crate::database_sync::history(&path).unwrap();
        let projected = crate::database_sync::model::project(&edits).unwrap();
        let notes: serde_json::Value =
            serde_json::from_str(projected[crate::task_db::keys::NOTES].as_ref().unwrap()).unwrap();
        assert_eq!(notes["notes"].as_array().unwrap().len(), 1);
        assert_eq!(notes["notes"][0]["drive_id"], "imported-1");
        assert_eq!(notes["notes"][0]["title"], "Edited");
        assert_eq!(notes["notes"][0]["body"], "keep");
        assert!(notes["notes"][0].get("dirty").is_none());
    }
}
