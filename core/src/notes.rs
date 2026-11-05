//! Notes: Markdown documents kept in Google Drive's hidden app folder.
//!
//! Each note is one `.md` file, so its text stays an ordinary Markdown
//! document. Its title is the file's name, and what Markdown has no place for
//! (the background color and what it is connected to) is JSON in the file's
//! `description`. That lets one listing show every note's details, and devices
//! never fight over a shared index file.
//!
//! This is unrelated to the notepad, which is one page in the config file.
//!
//! Google Drive is the source of truth, with a cached copy so that notes open
//! at once and can be written offline. A sync never loses text: when this
//! device and another both changed a note, Drive's copy keeps the note and the
//! edits made here become a separate "(conflict)" note.
//!
//! [`sync`] runs on a worker thread against a copy of the [`Library`], so the
//! user can keep typing. It reports [`Change`]s, which [`Library::apply`] then
//! folds into whatever the library has become by the time they arrive.

pub mod markdown;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::calendar::oauth;
use crate::config_sync::{Failure, agent, send};

const FILES: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";
const MIME: &str = "text/markdown";
const EXTENSION: &str = ".md";
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
    /// Stable on this device, including across a note's first upload.
    pub id: String,
    /// The file in Google Drive. `None` until the first upload.
    #[serde(default)]
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
    /// Drive's `modifiedTime` for the copy both sides last agreed on.
    #[serde(default)]
    pub synced_ms: Option<i64>,
    /// Holds changes Google Drive has not seen.
    #[serde(default)]
    pub dirty: bool,
    /// Counts local edits, to notice one made while a sync was running.
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

/// An id for a note that exists only here so far.
pub fn local_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "local-{:x}-{:x}",
        crate::clock::now().timestamp_micros(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Every note on this device, and the deletions Drive has yet to hear about.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub notes: Vec<Note>,
    /// Drive files to delete at the next sync.
    #[serde(default)]
    pub deleted: Vec<String>,
}

impl Library {
    /// The cached copy. Unreadable or missing means starting over from Drive.
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
            synced_ms: None,
            dirty: true,
            revision: 1,
        });
        id
    }

    /// Changes a note, marking it for upload. Returns whether it exists.
    pub fn edit(&mut self, id: &str, change: impl FnOnce(&mut Note)) -> bool {
        let Some(note) = self.notes.iter_mut().find(|note| note.id == id) else {
            return false;
        };
        change(note);
        note.dirty = true;
        note.revision += 1;
        note.modified_ms = crate::clock::now().timestamp_millis();
        true
    }

    /// Removes a note, queueing the Drive file for deletion if it has one.
    pub fn delete(&mut self, id: &str) {
        let Some(index) = self.notes.iter().position(|note| note.id == id) else {
            return;
        };
        if let Some(drive_id) = self.notes.remove(index).drive_id {
            self.deleted.push(drive_id);
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

    pub fn has_unsynced(&self) -> bool {
        !self.deleted.is_empty() || self.notes.iter().any(|note| note.dirty && !note.is_blank())
    }

    /// Folds a sync's results into the library as it is now. A note edited
    /// while the sync ran keeps those edits and stays marked for upload.
    pub fn apply(&mut self, changes: Vec<Change>) {
        for change in changes {
            match change {
                Change::Uploaded {
                    id,
                    revision,
                    drive_id,
                    synced_ms,
                } => match self.notes.iter_mut().find(|note| note.id == id) {
                    Some(note) => {
                        note.drive_id = Some(drive_id);
                        note.synced_ms = Some(synced_ms);
                        if note.revision == revision {
                            note.dirty = false;
                        }
                    }
                    // Deleted while its first upload was on the way.
                    None => {
                        if !self.deleted.contains(&drive_id) {
                            self.deleted.push(drive_id);
                        }
                    }
                },
                Change::Incoming(incoming) => {
                    let drive_id = incoming.drive_id.as_deref().unwrap_or_default();
                    if self.deleted.iter().any(|deleted| deleted == drive_id) {
                        continue;
                    }
                    match self.notes.iter().position(|note| note.id == incoming.id) {
                        Some(index) => {
                            if self.notes[index].revision != incoming.revision {
                                let copy = conflict_copy(&self.notes[index]);
                                self.notes.push(copy);
                            }
                            self.notes[index] = incoming;
                        }
                        None => self.notes.push(incoming),
                    }
                }
                Change::Gone { id, revision } => {
                    if let Some(index) = self.notes.iter().position(|note| note.id == id) {
                        if self.notes[index].revision == revision {
                            self.notes.remove(index);
                        } else {
                            // Edited here while it was deleted elsewhere: the
                            // edits become a new file at the next sync.
                            self.notes[index].drive_id = None;
                            self.notes[index].synced_ms = None;
                        }
                    }
                }
                Change::Purged { drive_id } => self.deleted.retain(|id| *id != drive_id),
                Change::Forked(note) => self.notes.push(note),
            }
        }
    }
}

/// The text of `note` as a new note, for when Drive's copy has to win.
fn conflict_copy(note: &Note) -> Note {
    Note {
        id: local_id(),
        drive_id: None,
        title: format!("{} (conflict)", note.display_title()),
        synced_ms: None,
        dirty: true,
        revision: 1,
        ..note.clone()
    }
}

/// One thing a sync found or did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Drive now holds this revision of a note.
    Uploaded {
        id: String,
        revision: u64,
        drive_id: String,
        synced_ms: i64,
    },
    /// A note that is new here or changed on another device. `revision` is the
    /// revision it replaces.
    Incoming(Note),
    /// Deleted on another device.
    Gone { id: String, revision: u64 },
    /// A queued deletion reached Drive.
    Purged { drive_id: String },
    /// Local edits split off because Drive's copy changed as well.
    Forked(Note),
}

/// What a sync achieved, which may be partial: what succeeded before a failure
/// must still be recorded, or a retry would upload the same new note twice.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    pub changes: Vec<Change>,
    pub error: Option<String>,
}

/// A note as Google Drive lists it, without its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteNote {
    pub id: String,
    pub title: String,
    pub color: Pastel,
    pub links: Vec<Link>,
    pub modified_ms: i64,
}

/// The slice of Google Drive that notes need. A trait so that syncing can be
/// tested against an in-memory fake.
pub(crate) trait Drive {
    fn list(&self) -> Result<Vec<RemoteNote>, Failure>;
    fn download(&self, id: &str) -> Result<String, Failure>;
    /// Creates a file when `note.drive_id` is `None`. Returns the file's id and
    /// Drive's `modifiedTime`.
    fn upload(&self, note: &Note, replace: bool) -> Result<(String, i64), Failure>;
    fn delete(&self, id: &str) -> Result<(), Failure>;
}

fn sync_with(
    library: &Library,
    drive: &impl Drive,
    changes: &mut Vec<Change>,
) -> Result<(), Failure> {
    for drive_id in &library.deleted {
        drive.delete(drive_id)?;
        changes.push(Change::Purged {
            drive_id: drive_id.clone(),
        });
    }

    let remote: HashMap<String, RemoteNote> = drive
        .list()?
        .into_iter()
        .map(|note| (note.id.clone(), note))
        .collect();
    let mut known: HashSet<&str> = library.deleted.iter().map(String::as_str).collect();

    let upload = |note: &Note, replace: bool, changes: &mut Vec<Change>| {
        let (drive_id, synced_ms) = drive.upload(note, replace)?;
        changes.push(Change::Uploaded {
            id: note.id.clone(),
            revision: note.revision,
            drive_id,
            synced_ms,
        });
        Ok::<(), Failure>(())
    };

    for note in &library.notes {
        // A "New note" nothing has been written in yet may still be dropped.
        if note.is_blank() {
            continue;
        }
        let found = note.drive_id.as_deref().and_then(|id| remote.get(id));
        let Some(file) = found else {
            if note.drive_id.is_none() || note.dirty {
                // Never uploaded, or deleted elsewhere with edits to save.
                upload(note, false, changes)?;
            } else {
                changes.push(Change::Gone {
                    id: note.id.clone(),
                    revision: note.revision,
                });
            }
            continue;
        };
        known.insert(&file.id);

        if note.synced_ms == Some(file.modified_ms) {
            if note.dirty {
                upload(note, true, changes)?;
            }
            continue;
        }

        let body = drive.download(&file.id)?;
        if note.dirty
            && (note.body != body
                || note.display_title() != file.title
                || note.color != file.color
                || note.links != file.links)
        {
            changes.push(Change::Forked(conflict_copy(note)));
        }
        changes.push(Change::Incoming(incoming(
            file,
            body,
            note.id.clone(),
            note.revision,
        )));
    }

    for file in remote.values() {
        if !known.contains(file.id.as_str()) {
            let body = drive.download(&file.id)?;
            changes.push(Change::Incoming(incoming(file, body, file.id.clone(), 0)));
        }
    }
    Ok(())
}

fn incoming(file: &RemoteNote, body: String, id: String, revision: u64) -> Note {
    Note {
        id,
        drive_id: Some(file.id.clone()),
        title: file.title.clone(),
        body,
        color: file.color,
        links: file.links.clone(),
        modified_ms: file.modified_ms,
        synced_ms: Some(file.modified_ms),
        dirty: false,
        revision,
    }
}

fn describe(failure: Failure) -> String {
    match failure {
        Failure::SignedOut => "Sign in to Google to sync notes".to_owned(),
        Failure::Reconnect => "Reconnect Google in Settings to sync notes".to_owned(),
        Failure::Other(error) => error,
    }
}

pub(crate) fn sync_drive(library: &Library, drive: &impl Drive) -> Outcome {
    let mut changes = Vec::new();
    let error = sync_with(library, drive, &mut changes).err().map(describe);
    Outcome { changes, error }
}

/// Brings a copy of the library and Google Drive into step.
pub fn sync(library: &Library, client_id: &str, client_secret: &str) -> Outcome {
    let tokens = oauth::load_tokens()
        .ok_or(Failure::SignedOut)
        .and_then(|tokens| {
            oauth::ensure_access(client_id, client_secret, tokens).map_err(Failure::Other)
        });
    match tokens {
        Ok(tokens) => sync_drive(
            library,
            &GoogleDrive {
                token: tokens.access_token,
            },
        ),
        Err(failure) => Outcome {
            changes: Vec::new(),
            error: Some(describe(failure)),
        },
    }
}

// --- Google Drive --------------------------------------------------------

/// What a file's `description` holds.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Details {
    /// A string so that a color this version does not know reads as the default.
    #[serde(default)]
    color: String,
    #[serde(default)]
    links: Vec<Link>,
}

fn color_key(color: Pastel) -> String {
    serde_json::to_value(color)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn write_details(note: &Note) -> String {
    serde_json::to_string(&Details {
        color: color_key(note.color),
        links: note.links.clone(),
    })
    .unwrap_or_default()
}

fn read_details(description: &str) -> (Pastel, Vec<Link>) {
    let details: Details = serde_json::from_str(description).unwrap_or_default();
    let color = Pastel::ALL
        .into_iter()
        .find(|color| color_key(*color) == details.color)
        .unwrap_or_default();
    (color, details.links)
}

fn file_name(note: &Note) -> String {
    format!("{}{EXTENSION}", note.display_title())
}

fn title_of(name: &str) -> String {
    name.strip_suffix(EXTENSION).unwrap_or(name).to_owned()
}

struct GoogleDrive {
    token: String,
}

#[derive(Deserialize)]
struct FileList {
    #[serde(default)]
    files: Vec<ApiFile>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct ApiFile {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "modifiedTime")]
    modified_time: String,
    #[serde(default)]
    description: String,
}

fn modified_ms(time: &str) -> Result<i64, Failure> {
    DateTime::parse_from_rfc3339(time)
        .map(|time| time.timestamp_millis())
        .map_err(|_| Failure::Other("Google Drive returned an unreadable time".to_owned()))
}

impl GoogleDrive {
    fn request(&self, method: &str, url: &str) -> ureq::Request {
        agent()
            .request(method, url)
            .set("Authorization", &format!("Bearer {}", self.token))
    }
}

impl Drive for GoogleDrive {
    fn list(&self) -> Result<Vec<RemoteNote>, Failure> {
        let mut notes = Vec::new();
        let mut page: Option<String> = None;
        loop {
            let mut request = self
                .request("GET", FILES)
                .query("spaces", "appDataFolder")
                .query("q", &format!("mimeType = '{MIME}' and trashed = false"))
                .query("pageSize", "1000")
                .query(
                    "fields",
                    "nextPageToken,files(id,name,modifiedTime,description)",
                );
            if let Some(token) = &page {
                request = request.query("pageToken", token);
            }
            let list: FileList = send(request.call())?.into_json().map_err(Failure::other)?;
            for file in list.files {
                let (color, links) = read_details(&file.description);
                notes.push(RemoteNote {
                    modified_ms: modified_ms(&file.modified_time)?,
                    title: title_of(&file.name),
                    id: file.id,
                    color,
                    links,
                });
            }
            page = list.next_page_token;
            if page.is_none() {
                return Ok(notes);
            }
        }
    }

    fn download(&self, id: &str) -> Result<String, Failure> {
        send(
            self.request("GET", &format!("{FILES}/{id}"))
                .query("alt", "media")
                .call(),
        )?
        .into_string()
        .map_err(Failure::other)
    }

    fn upload(&self, note: &Note, replace: bool) -> Result<(String, i64), Failure> {
        let mut metadata = serde_json::Map::new();
        metadata.insert("name".into(), file_name(note).into());
        metadata.insert("mimeType".into(), MIME.into());
        metadata.insert("description".into(), write_details(note).into());
        let existing = note.drive_id.as_deref().filter(|_| replace);
        if existing.is_none() {
            metadata.insert("parents".into(), serde_json::json!(["appDataFolder"]));
        }

        let mut boundary = String::from("timeslay-note-boundary");
        while note.body.contains(&boundary) {
            boundary.push('x');
        }
        let body = format!(
            "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{}\r\n--{boundary}\r\nContent-Type: {MIME}; charset=UTF-8\r\n\r\n{}\r\n--{boundary}--",
            serde_json::Value::Object(metadata),
            note.body,
        );
        let request = match existing {
            Some(id) => self.request("PATCH", &format!("{UPLOAD}/{id}")),
            None => self.request("POST", UPLOAD),
        };
        let file: ApiFile = send(
            request
                .query("uploadType", "multipart")
                .query("fields", "id,modifiedTime")
                .set(
                    "Content-Type",
                    &format!("multipart/related; boundary={boundary}"),
                )
                .send_string(&body),
        )?
        .into_json()
        .map_err(Failure::other)?;
        Ok((file.id.clone(), modified_ms(&file.modified_time)?))
    }

    fn delete(&self, id: &str) -> Result<(), Failure> {
        match self.request("DELETE", &format!("{FILES}/{id}")).call() {
            // Already gone is what was wanted.
            Err(ureq::Error::Status(404, _)) => Ok(()),
            result => send(result).map(|_| ()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeDrive {
        files: RefCell<HashMap<String, (RemoteNote, String)>>,
        next: RefCell<i64>,
        fail_uploads_after: RefCell<Option<u32>>,
    }

    impl FakeDrive {
        fn put(&self, id: &str, title: &str, body: &str, modified_ms: i64) {
            self.files.borrow_mut().insert(
                id.to_owned(),
                (
                    RemoteNote {
                        id: id.to_owned(),
                        title: title.to_owned(),
                        color: Pastel::default(),
                        links: Vec::new(),
                        modified_ms,
                    },
                    body.to_owned(),
                ),
            );
        }

        fn body(&self, id: &str) -> Option<String> {
            self.files.borrow().get(id).map(|(_, body)| body.clone())
        }
    }

    impl Drive for &FakeDrive {
        fn list(&self) -> Result<Vec<RemoteNote>, Failure> {
            Ok(self
                .files
                .borrow()
                .values()
                .map(|(note, _)| note.clone())
                .collect())
        }

        fn download(&self, id: &str) -> Result<String, Failure> {
            self.body(id)
                .ok_or_else(|| Failure::Other("missing".to_owned()))
        }

        fn upload(&self, note: &Note, replace: bool) -> Result<(String, i64), Failure> {
            if let Some(left) = self.fail_uploads_after.borrow_mut().as_mut() {
                if *left == 0 {
                    return Err(Failure::Other("offline".to_owned()));
                }
                *left -= 1;
            }
            *self.next.borrow_mut() += 1;
            let stamp = 1000 + *self.next.borrow();
            let id = match (&note.drive_id, replace) {
                (Some(id), true) => id.clone(),
                _ => format!("file-{stamp}"),
            };
            self.files.borrow_mut().insert(
                id.clone(),
                (
                    RemoteNote {
                        id: id.clone(),
                        title: note.display_title().to_owned(),
                        color: note.color,
                        links: note.links.clone(),
                        modified_ms: stamp,
                    },
                    note.body.clone(),
                ),
            );
            Ok((id, stamp))
        }

        fn delete(&self, id: &str) -> Result<(), Failure> {
            self.files.borrow_mut().remove(id);
            Ok(())
        }
    }

    fn pass(library: &mut Library, drive: &FakeDrive) {
        let outcome = sync_drive(library, &drive);
        assert_eq!(outcome.error, None);
        library.apply(outcome.changes);
    }

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
    fn details_round_trip_and_unknown_colors_fall_back() {
        let note = Note {
            color: Pastel::Mint,
            links: vec![habit()],
            ..incoming_note("n")
        };
        let (color, links) = read_details(&write_details(&note));
        assert_eq!(color, Pastel::Mint);
        assert_eq!(links, [habit()]);
        assert_eq!(
            read_details(r##"{"color":"#000000"}"##).0,
            Pastel::default()
        );
        assert_eq!(read_details("not json"), (Pastel::default(), Vec::new()));
    }

    fn incoming_note(id: &str) -> Note {
        Note {
            id: id.to_owned(),
            drive_id: None,
            title: "T".into(),
            body: String::new(),
            color: Pastel::default(),
            links: Vec::new(),
            modified_ms: 0,
            synced_ms: None,
            dirty: false,
            revision: 0,
        }
    }

    #[test]
    fn titles_are_file_names_without_the_extension() {
        assert_eq!(title_of("Plans.md"), "Plans");
        assert_eq!(title_of("v1.md.md"), "v1.md");
        assert_eq!(file_name(&incoming_note("n")), "T.md");
        let untitled = Note {
            title: "  ".into(),
            ..incoming_note("n")
        };
        assert_eq!(file_name(&untitled), "Untitled.md");
    }

    #[test]
    fn a_new_note_uploads_once_and_then_keeps_its_id() {
        let drive = FakeDrive::default();
        let mut library = Library::default();
        let id = library.create("Plans", vec![habit()]);
        library.edit(&id, |note| note.body = "# Hi".into());
        pass(&mut library, &drive);

        let note = library.note(&id).unwrap();
        assert!(!note.dirty);
        let drive_id = note.drive_id.clone().unwrap();
        assert_eq!(drive.body(&drive_id).as_deref(), Some("# Hi"));

        pass(&mut library, &drive);
        assert_eq!(drive.files.borrow().len(), 1);
        assert_eq!(library.notes.len(), 1);
    }

    #[test]
    fn a_blank_new_note_stays_off_drive_until_it_has_something_in_it() {
        let drive = FakeDrive::default();
        let mut library = Library::default();
        let id = library.create("", Vec::new());
        assert!(!library.has_unsynced());
        pass(&mut library, &drive);
        assert!(drive.files.borrow().is_empty());
        assert!(library.note(&id).unwrap().is_blank());

        library.edit(&id, |note| note.body = "text".into());
        assert!(library.has_unsynced());
        pass(&mut library, &drive);
        assert_eq!(drive.files.borrow().len(), 1);
    }

    #[test]
    fn another_devices_note_and_links_arrive() {
        let drive = FakeDrive::default();
        let mut other = Library::default();
        let id = other.create("Shared", vec![habit()]);
        other.edit(&id, |note| {
            note.body = "text".into();
            note.color = Pastel::Sky;
        });
        pass(&mut other, &drive);

        let mut here = Library::default();
        pass(&mut here, &drive);
        assert_eq!(here.notes.len(), 1);
        assert_eq!(here.notes[0].body, "text");
        assert_eq!(here.notes[0].color, Pastel::Sky);
        assert_eq!(here.linked(&habit()).len(), 1);
    }

    #[test]
    fn a_remote_change_replaces_a_clean_note() {
        let drive = FakeDrive::default();
        drive.put("f", "A", "one", 10);
        let mut library = Library::default();
        pass(&mut library, &drive);
        drive.put("f", "A", "two", 20);
        pass(&mut library, &drive);
        assert_eq!(library.notes.len(), 1);
        assert_eq!(library.notes[0].body, "two");
    }

    #[test]
    fn edits_on_both_sides_keep_both_texts() {
        let drive = FakeDrive::default();
        drive.put("f", "A", "base", 10);
        let mut library = Library::default();
        pass(&mut library, &drive);

        let id = library.notes[0].id.clone();
        library.edit(&id, |note| note.body = "mine".into());
        drive.put("f", "A", "theirs", 20);
        pass(&mut library, &drive);

        assert_eq!(library.notes.len(), 2);
        assert_eq!(library.note(&id).unwrap().body, "theirs");
        let copy = library.notes.iter().find(|note| note.id != id).unwrap();
        assert_eq!(copy.body, "mine");
        assert_eq!(copy.title, "A (conflict)");

        // The copy reaches Drive like any other note.
        pass(&mut library, &drive);
        assert_eq!(drive.files.borrow().len(), 2);
        assert!(!library.has_unsynced());
    }

    #[test]
    fn the_same_edit_on_both_sides_is_not_a_conflict() {
        let drive = FakeDrive::default();
        drive.put("f", "A", "base", 10);
        let mut library = Library::default();
        pass(&mut library, &drive);
        let id = library.notes[0].id.clone();
        library.edit(&id, |note| note.body = "same".into());
        drive.put("f", "A", "same", 20);
        pass(&mut library, &drive);
        assert_eq!(library.notes.len(), 1);
        assert!(!library.notes[0].dirty);
    }

    #[test]
    fn typing_during_a_sync_survives_and_stays_unsynced() {
        let drive = FakeDrive::default();
        let mut library = Library::default();
        let id = library.create("A", Vec::new());
        let outcome = sync_drive(&library, &&drive);
        library.edit(&id, |note| note.body = "typed meanwhile".into());
        library.apply(outcome.changes);

        let note = library.note(&id).unwrap();
        assert_eq!(note.body, "typed meanwhile");
        assert!(note.drive_id.is_some());
        assert!(note.dirty);

        pass(&mut library, &drive);
        let drive_id = library.note(&id).unwrap().drive_id.clone().unwrap();
        assert_eq!(drive.body(&drive_id).as_deref(), Some("typed meanwhile"));
    }

    #[test]
    fn typing_during_a_download_is_split_off_rather_than_lost() {
        let drive = FakeDrive::default();
        drive.put("f", "A", "base", 10);
        let mut library = Library::default();
        pass(&mut library, &drive);
        let id = library.notes[0].id.clone();
        drive.put("f", "A", "theirs", 20);

        let outcome = sync_drive(&library, &&drive);
        library.edit(&id, |note| note.body = "typed meanwhile".into());
        library.apply(outcome.changes);

        assert_eq!(library.note(&id).unwrap().body, "theirs");
        assert!(
            library
                .notes
                .iter()
                .any(|note| note.body == "typed meanwhile")
        );
    }

    #[test]
    fn deleting_reaches_drive_and_does_not_come_back() {
        let drive = FakeDrive::default();
        drive.put("f", "A", "x", 10);
        let mut library = Library::default();
        pass(&mut library, &drive);
        let id = library.notes[0].id.clone();
        library.delete(&id);
        assert_eq!(library.deleted, ["f"]);
        pass(&mut library, &drive);
        assert!(library.notes.is_empty());
        assert!(library.deleted.is_empty());
        assert!(drive.files.borrow().is_empty());
    }

    #[test]
    fn a_note_deleted_during_a_sync_does_not_return() {
        let drive = FakeDrive::default();
        drive.put("f", "A", "x", 10);
        let mut library = Library::default();
        pass(&mut library, &drive);
        let id = library.notes[0].id.clone();
        drive.put("f", "A", "newer", 20);
        let outcome = sync_drive(&library, &&drive);
        library.delete(&id);
        library.apply(outcome.changes);
        assert!(library.notes.is_empty());
        assert_eq!(library.deleted, ["f"]);
    }

    #[test]
    fn deleted_elsewhere_removes_a_clean_note_and_keeps_edits() {
        let drive = FakeDrive::default();
        drive.put("a", "A", "x", 10);
        drive.put("b", "B", "y", 10);
        let mut library = Library::default();
        pass(&mut library, &drive);
        let edited = library
            .notes
            .iter()
            .find(|note| note.title == "B")
            .unwrap()
            .id
            .clone();
        library.edit(&edited, |note| note.body = "kept".into());
        drive.files.borrow_mut().clear();
        pass(&mut library, &drive);
        assert_eq!(library.notes.len(), 1);
        assert_eq!(library.notes[0].body, "kept");
        assert_eq!(drive.files.borrow().len(), 1);
    }

    #[test]
    fn a_failed_sync_records_what_got_through() {
        let drive = FakeDrive::default();
        *drive.fail_uploads_after.borrow_mut() = Some(1);
        let mut library = Library::default();
        library.create("One", Vec::new());
        library.create("Two", Vec::new());

        let outcome = sync_drive(&library, &&drive);
        assert_eq!(outcome.error.as_deref(), Some("offline"));
        library.apply(outcome.changes);
        assert_eq!(drive.files.borrow().len(), 1);

        *drive.fail_uploads_after.borrow_mut() = None;
        pass(&mut library, &drive);
        assert_eq!(drive.files.borrow().len(), 2);
    }

    #[test]
    fn connecting_is_idempotent_and_marks_the_note_for_upload() {
        let mut library = Library::default();
        let id = library.create("A", Vec::new());
        library.notes[0].dirty = false;
        library.connect(&id, habit());
        library.connect(&id, habit());
        assert_eq!(library.note(&id).unwrap().links.len(), 1);
        assert!(library.note(&id).unwrap().dirty);
        library.disconnect(&id, &habit());
        assert!(library.linked(&habit()).is_empty());
    }

    #[test]
    fn the_cache_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested").join("test.sqlite3");
        let mut library = Library::default();
        library.create("A", vec![habit()]);
        library.deleted.push("gone".into());
        library.save(&path).unwrap();
        assert_eq!(Library::load(&path), library);
        crate::task_db::write_document_json(&path, crate::task_db::keys::NOTES, "not json")
            .unwrap();
        assert_eq!(Library::load(&path), Library::default());
    }
}
