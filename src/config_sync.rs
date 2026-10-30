//! Keeps `config.toml` in step with the app's hidden Google Drive folder.
//!
//! Google Drive's `appDataFolder` is a per-app space that does not show up in
//! the user's Drive, so the config can follow the user from desktop to phone
//! without cluttering anything. Both the iced app and the Dioxus app start
//! the same engine, which means they share one file there.
//!
//! The rules, in order of importance:
//!
//! * An empty config never replaces another one. "Empty" means missing, blank
//!   or comments only. A config that does not parse is left alone entirely.
//!   It is probably half-edited, and it must not be uploaded either.
//! * If only one side changed since the last sync, that side wins.
//! * If both changed (or this is the first sync after launch), the newest
//!   timestamp wins: the local file's modification time against Drive's
//!   `modifiedTime`.
//!
//! To keep those timestamps comparable, an upload stamps the Drive file with
//! the local modification time, and a download stamps the local file with
//! Drive's. Syncing therefore does not make either copy look newer than the
//! other.
//!
//! The local file is watched for edits, and Drive is polled for changes made
//! on other devices. Both just wake the same worker thread.

use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local, SecondsFormat, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::calendar::oauth;
use crate::config;

const FILE_NAME: &str = "config.toml";
const FILES: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files";

/// How long the file must stay quiet before a local edit is synced. Editors
/// and the app's own saves can write several times in a burst.
const SETTLE: Duration = Duration::from_secs(2);
/// How often Drive is asked whether another device changed the config.
const POLL_EVERY: Duration = Duration::from_secs(30);
/// Files older than this are placeholders (stamped with
/// the epoch so any real config beats it), not something to tell Drive about.
const PLAUSIBLE_AFTER_MS: i64 = 946_684_800_000; // 2000-01-01

/// What the sync worker is up to, for a status line.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// The worker has not finished its first pass.
    Starting,
    /// No Google account is connected yet.
    SignedOut,
    /// The Google token predates Drive sync, so it lacks the permission.
    NeedsReconnect,
    /// Both copies agree as of this time.
    Synced(SystemTime),
    Failed(String),
}

impl Status {
    /// A short, user-facing description.
    pub fn summary(&self) -> String {
        match self {
            Self::Starting => "Config sync starting…".to_owned(),
            Self::SignedOut => "Config sync waits for Google sign-in".to_owned(),
            Self::NeedsReconnect => "Reconnect Google to sync the config".to_owned(),
            Self::Synced(at) => format!(
                "Config synced {}",
                DateTime::<Local>::from(*at).format("%-I:%M %p")
            ),
            Self::Failed(_) => "Config sync failed".to_owned(),
        }
    }
}

static STATUS: Mutex<Status> = Mutex::new(Status::Starting);
static GENERATION: AtomicU64 = AtomicU64::new(0);
static WORKER: OnceLock<Sender<Wake>> = OnceLock::new();

/// Starts the worker once per process. Further calls do nothing.
pub fn start() {
    WORKER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel();
        let watcher_sender = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("timeslay-config-sync".to_owned())
            .spawn(move || run(receiver, watcher_sender));
        if let Err(error) = spawned {
            log::warn!("could not start the config sync worker: {error}");
        }
        sender
    });
}

/// Syncs right away, rather than at the next poll. Call it after signing in.
pub fn poke() {
    if let Some(sender) = WORKER.get() {
        let _ = sender.send(Wake::Now);
    }
}

pub fn status() -> Status {
    STATUS
        .lock()
        .map(|status| status.clone())
        .unwrap_or(Status::Starting)
}

/// Counts how many times `config.toml` changed contents under the running app,
/// whether by an edit, a save, or a download from Drive. A UI that holds the
/// config in memory re-reads it when this moves.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

fn set_status(next: Status) {
    if let Ok(mut status) = STATUS.lock()
        && *status != next
    {
        match &next {
            Status::Failed(error) => log::warn!("config sync failed: {error}"),
            Status::NeedsReconnect => log::warn!(
                "config sync needs the Google Drive permission; disconnect and connect Google again"
            ),
            _ => {}
        }
        *status = next;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    /// The file on disk changed.
    File,
    /// Go to Drive now.
    Now,
}

fn run(receiver: mpsc::Receiver<Wake>, sender: Sender<Wake>) {
    let Some(path) = config::config_path() else {
        set_status(Status::Failed("no config directory available".to_owned()));
        return;
    };
    // Held for as long as the worker lives; dropping it stops the watch.
    let _watcher = watch(&path, sender);

    let mut engine = Engine::default();
    let mut poll = true;
    loop {
        engine.cycle(&path, poll);

        poll = match receiver.recv_timeout(POLL_EVERY) {
            Ok(Wake::Now) => true,
            Ok(Wake::File) => {
                let mut poll = false;
                loop {
                    match receiver.recv_timeout(SETTLE) {
                        Ok(wake) => poll |= wake == Wake::Now,
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                poll
            }
            Err(RecvTimeoutError::Timeout) => true,
            Err(RecvTimeoutError::Disconnected) => return,
        };
    }
}

/// Watches the directory rather than the file: saves replace the file by
/// renaming a temporary one over it, which a watch on the old inode misses.
fn watch(path: &Path, sender: Sender<Wake>) -> Option<notify::RecommendedWatcher> {
    use notify::{EventKind, Watcher};

    let parent = path.parent()?;
    let watched = path.to_owned();
    let mut watcher =
        match notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            Ok(event)
                if event.paths.iter().any(|path| path == &watched)
                    && matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                    ) =>
            {
                let _ = sender.send(Wake::File);
            }
            Err(error) => log::warn!("config sync watcher error: {error}"),
            _ => {}
        }) {
            Ok(watcher) => watcher,
            Err(error) => {
                log::warn!("could not watch the config for changes: {error}");
                return None;
            }
        };
    if let Err(error) = fs::create_dir_all(parent).and_then(|()| {
        watcher
            .watch(parent, notify::RecursiveMode::NonRecursive)
            .map_err(io::Error::other)
    }) {
        log::warn!("could not watch {}: {error}", parent.display());
        return None;
    }
    Some(watcher)
}

// --- The decision --------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Missing, blank, or nothing but comments.
    Empty,
    /// Does not parse as TOML.
    Invalid,
    Ready,
}

fn classify(text: &str) -> Kind {
    match toml::from_str::<toml::Table>(text) {
        Ok(table) if table.is_empty() => Kind::Empty,
        Ok(_) => Kind::Ready,
        Err(_) => Kind::Invalid,
    }
}

type Hash = [u8; 32];

fn hash(text: &str) -> Hash {
    Sha256::digest(text.as_bytes()).into()
}

#[derive(Debug, Clone)]
struct LocalFile {
    text: String,
    modified_ms: i64,
    hash: Hash,
    kind: Kind,
}

impl LocalFile {
    fn ready(&self) -> bool {
        self.kind == Kind::Ready
    }
}

/// A missing file reads as an empty one: neither may replace anything.
fn read_local(path: &Path) -> io::Result<LocalFile> {
    let (text, modified_ms) = match fs::read_to_string(path) {
        Ok(text) => {
            let modified = fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .map(millis)
                .unwrap_or(0);
            (text, modified)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => (String::new(), 0),
        Err(error) => return Err(error),
    };
    Ok(LocalFile {
        hash: hash(&text),
        kind: classify(&text),
        text,
        modified_ms,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Nothing,
    Upload,
    Download,
    /// Both sides already hold the same text; only the bookkeeping moves.
    Adopt,
}

/// What to do when Drive's copy has changed and has been fetched.
///
/// `local_changed` is whether the local file differs from what was last
/// synced; it is true when nothing has been synced yet this run.
fn resolve(
    local: &LocalFile,
    remote_text: &str,
    remote_modified_ms: i64,
    local_changed: bool,
) -> Action {
    let remote_ready = classify(remote_text) == Kind::Ready;
    match (local.ready(), remote_ready) {
        (false, false) => Action::Nothing,
        // An empty config never replaces a real one, whichever side it is on.
        (true, false) => Action::Upload,
        (false, true) => Action::Download,
        (true, true) if local.text == remote_text => Action::Adopt,
        // Only the other device touched it: take its version, whatever the
        // clocks say.
        (true, true) if !local_changed => Action::Download,
        (true, true) if local.modified_ms > remote_modified_ms => Action::Upload,
        (true, true) => Action::Download,
    }
}

// --- The engine ----------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Synced {
    /// The local text both sides agreed on.
    hash: Hash,
    /// Drive's `modifiedTime` for that agreement.
    remote_modified_ms: i64,
}

#[derive(Default)]
struct Engine {
    last: Option<Synced>,
    /// The last file contents this run saw, to notice outside changes.
    seen: Option<Hash>,
}

#[derive(Debug, Clone, PartialEq)]
enum Failure {
    SignedOut,
    Reconnect,
    Other(String),
}

impl Failure {
    fn other(error: impl std::fmt::Display) -> Self {
        Self::Other(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RemoteFile {
    id: String,
    modified_ms: i64,
}

/// The slice of Google Drive the engine needs. A trait so the decisions can be
/// tested against an in-memory fake.
trait Drive {
    fn find(&self) -> Result<Option<RemoteFile>, Failure>;
    fn download(&self, file: &RemoteFile) -> Result<String, Failure>;
    /// Creates the file when `existing` is `None`. `modified_ms` stamps the
    /// Drive copy, unless it is a placeholder time.
    fn upload(
        &self,
        existing: Option<&RemoteFile>,
        text: &str,
        modified_ms: i64,
    ) -> Result<RemoteFile, Failure>;
}

impl Engine {
    /// One pass, with the outcome folded into [`status`].
    fn cycle(&mut self, path: &Path, poll: bool) {
        let result = self.step(path, poll, |local| {
            let tokens = oauth::load_tokens().ok_or(Failure::SignedOut)?;
            let (client_id, client_secret) = credentials(&local.text);
            let tokens =
                oauth::ensure_access(&client_id, &client_secret, tokens).map_err(Failure::Other)?;
            Ok(GoogleDrive {
                token: tokens.access_token,
            })
        });
        match result {
            Ok(Pass::Synced) => set_status(Status::Synced(SystemTime::now())),
            Ok(Pass::Skipped) => {}
            Err(Failure::SignedOut) => set_status(Status::SignedOut),
            Err(Failure::Reconnect) => set_status(Status::NeedsReconnect),
            Err(Failure::Other(error)) => set_status(Status::Failed(error)),
        }
    }

    /// `poll` is whether to ask Drive even when the local file looks settled.
    fn step<D: Drive>(
        &mut self,
        path: &Path,
        poll: bool,
        connect: impl FnOnce(&LocalFile) -> Result<D, Failure>,
    ) -> Result<Pass, Failure> {
        let local = read_local(path).map_err(Failure::other)?;

        if self.seen != Some(local.hash) {
            if self.seen.is_some() {
                GENERATION.fetch_add(1, Ordering::Relaxed);
            }
            self.seen = Some(local.hash);
        }

        if local.kind == Kind::Invalid {
            log::warn!(
                "{} does not parse; leaving it alone until it does",
                path.display()
            );
            return Ok(Pass::Skipped);
        }
        // Our own writes and no-op saves land here: nothing new to tell Drive.
        if !poll && self.last.is_some_and(|last| last.hash == local.hash) {
            return Ok(Pass::Skipped);
        }

        let drive = connect(&local)?;
        let local_changed = self.last.is_none_or(|last| last.hash != local.hash);

        let Some(remote) = drive.find()? else {
            if local.ready() {
                let uploaded = drive.upload(None, &local.text, local.modified_ms)?;
                log::info!("uploaded the config to Google Drive");
                self.record(local.hash, uploaded.modified_ms);
            }
            return Ok(Pass::Synced);
        };

        let remote_changed = self
            .last
            .is_none_or(|last| last.remote_modified_ms != remote.modified_ms);

        if !local_changed && !remote_changed {
            return Ok(Pass::Synced);
        }
        // Stamped by an earlier sync, so there is nothing to transfer.
        if self.last.is_none() && local.ready() && local.modified_ms == remote.modified_ms {
            self.record(local.hash, remote.modified_ms);
            return Ok(Pass::Synced);
        }

        if !remote_changed {
            if local.ready() {
                let uploaded = drive.upload(Some(&remote), &local.text, local.modified_ms)?;
                log::info!("uploaded the config to Google Drive");
                self.record(local.hash, uploaded.modified_ms);
            }
            return Ok(Pass::Synced);
        }

        let remote_text = drive.download(&remote)?;
        match resolve(&local, &remote_text, remote.modified_ms, local_changed) {
            Action::Nothing => {}
            Action::Adopt => self.record(local.hash, remote.modified_ms),
            Action::Upload => {
                let uploaded = drive.upload(Some(&remote), &local.text, local.modified_ms)?;
                log::info!("uploaded the config to Google Drive");
                self.record(local.hash, uploaded.modified_ms);
            }
            Action::Download => {
                write_local(path, &remote_text, remote.modified_ms).map_err(Failure::other)?;
                log::info!("updated the config from Google Drive");
                let downloaded = hash(&remote_text);
                self.seen = Some(downloaded);
                GENERATION.fetch_add(1, Ordering::Relaxed);
                self.record(downloaded, remote.modified_ms);
            }
        }
        Ok(Pass::Synced)
    }

    fn record(&mut self, hash: Hash, remote_modified_ms: i64) {
        self.last = Some(Synced {
            hash,
            remote_modified_ms,
        });
    }
}

enum Pass {
    /// The two sides are as agreed as they can be.
    Synced,
    /// Nothing was asked of Drive.
    Skipped,
}

/// The OAuth client, read from the config text, for refreshing a token.
fn credentials(text: &str) -> (String, String) {
    let table = toml::from_str::<toml::Table>(text).unwrap_or_default();
    let field = |name: &str| {
        table
            .get("calendar")
            .and_then(|calendar| calendar.get(name))
            .and_then(toml::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    (field("client_id"), field("client_secret"))
}

/// Replaces the config atomically, stamping it with Drive's modified time.
fn write_local(path: &Path, text: &str, modified_ms: i64) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    // Not `config.toml.tmp`, which the apps' own saves use.
    let temporary = path.with_extension("toml.sync-tmp");
    let _ = fs::remove_file(&temporary);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(text.as_bytes())?;
    if let Err(error) = file.set_modified(from_millis(modified_ms)) {
        log::warn!("could not stamp the config's modification time: {error}");
    }
    drop(file);
    fs::rename(&temporary, path)
}

fn millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

fn from_millis(ms: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms.max(0) as u64)
}

// --- Google Drive --------------------------------------------------------

struct GoogleDrive {
    token: String,
}

#[derive(Deserialize)]
struct FileList {
    #[serde(default)]
    files: Vec<ApiFile>,
}

#[derive(Deserialize)]
struct ApiFile {
    id: String,
    #[serde(rename = "modifiedTime")]
    modified_time: String,
}

impl ApiFile {
    fn into_remote(self) -> Option<RemoteFile> {
        let modified = DateTime::parse_from_rfc3339(&self.modified_time).ok()?;
        Some(RemoteFile {
            id: self.id,
            modified_ms: modified.timestamp_millis(),
        })
    }
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
    })
}

impl GoogleDrive {
    fn request(&self, method: &str, url: &str) -> ureq::Request {
        agent()
            .request(method, url)
            .set("Authorization", &format!("Bearer {}", self.token))
    }
}

impl Drive for GoogleDrive {
    fn find(&self) -> Result<Option<RemoteFile>, Failure> {
        let response = send(
            self.request("GET", FILES)
                .query("spaces", "appDataFolder")
                .query("q", &format!("name = '{FILE_NAME}' and trashed = false"))
                .query("orderBy", "modifiedTime desc")
                .query("pageSize", "10")
                .query("fields", "files(id,modifiedTime)")
                .call(),
        )?;
        let list: FileList = response.into_json().map_err(Failure::other)?;
        // Two devices creating the file at once leave duplicates; the newest
        // is the one in use.
        Ok(list.files.into_iter().find_map(ApiFile::into_remote))
    }

    fn download(&self, file: &RemoteFile) -> Result<String, Failure> {
        let response = send(
            self.request("GET", &format!("{FILES}/{}", file.id))
                .query("alt", "media")
                .call(),
        )?;
        response.into_string().map_err(Failure::other)
    }

    fn upload(
        &self,
        existing: Option<&RemoteFile>,
        text: &str,
        modified_ms: i64,
    ) -> Result<RemoteFile, Failure> {
        let mut metadata = serde_json::Map::new();
        if existing.is_none() {
            metadata.insert("name".into(), FILE_NAME.into());
            metadata.insert("parents".into(), serde_json::json!(["appDataFolder"]));
        }
        if modified_ms >= PLAUSIBLE_AFTER_MS
            && let Some(time) = DateTime::<Utc>::from_timestamp_millis(modified_ms)
        {
            metadata.insert(
                "modifiedTime".into(),
                time.to_rfc3339_opts(SecondsFormat::Millis, true).into(),
            );
        }

        let mut boundary = String::from("timeslay-config-boundary");
        while text.contains(&boundary) {
            boundary.push('x');
        }
        let body = format!(
            "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{}\r\n--{boundary}\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n{text}\r\n--{boundary}--",
            serde_json::Value::Object(metadata),
        );

        let request = match existing {
            Some(file) => self.request("PATCH", &format!("{UPLOAD}/{}", file.id)),
            None => self.request("POST", UPLOAD),
        };
        let response = send(
            request
                .query("uploadType", "multipart")
                .query("fields", "id,modifiedTime")
                .set(
                    "Content-Type",
                    &format!("multipart/related; boundary={boundary}"),
                )
                .send_string(&body),
        )?;
        let file: ApiFile = response.into_json().map_err(Failure::other)?;
        file.into_remote()
            .ok_or_else(|| Failure::Other("Google Drive returned an unreadable time".to_owned()))
    }
}

fn send(result: Result<ureq::Response, ureq::Error>) -> Result<ureq::Response, Failure> {
    match result {
        Ok(response) => Ok(response),
        Err(ureq::Error::Status(code, response)) => {
            let body = response.into_string().unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|json| json["error"]["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| body.chars().take(200).collect());
            // A token from before Drive sync existed has no Drive scope.
            if code == 403 && body.to_lowercase().contains("insufficient") {
                Err(Failure::Reconnect)
            } else {
                Err(Failure::Other(format!(
                    "Google Drive returned HTTP {code}: {message}"
                )))
            }
        }
        Err(error) => Err(Failure::Other(format!("Google Drive: {error}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;

    const NOW: i64 = 1_800_000_000_000;

    #[derive(Default)]
    struct FakeDrive {
        file: RefCell<Option<(String, i64)>>,
        downloads: RefCell<u32>,
        uploads: RefCell<u32>,
    }

    impl FakeDrive {
        fn holding(text: &str, modified_ms: i64) -> Self {
            Self {
                file: RefCell::new(Some((text.to_owned(), modified_ms))),
                ..Self::default()
            }
        }

        fn text(&self) -> Option<String> {
            self.file.borrow().as_ref().map(|(text, _)| text.clone())
        }
    }

    impl Drive for &FakeDrive {
        fn find(&self) -> Result<Option<RemoteFile>, Failure> {
            Ok(self
                .file
                .borrow()
                .as_ref()
                .map(|(_, modified_ms)| RemoteFile {
                    id: "id".to_owned(),
                    modified_ms: *modified_ms,
                }))
        }

        fn download(&self, _: &RemoteFile) -> Result<String, Failure> {
            *self.downloads.borrow_mut() += 1;
            Ok(self.file.borrow().as_ref().unwrap().0.clone())
        }

        fn upload(
            &self,
            _: Option<&RemoteFile>,
            text: &str,
            modified_ms: i64,
        ) -> Result<RemoteFile, Failure> {
            *self.uploads.borrow_mut() += 1;
            let stamped = if modified_ms >= PLAUSIBLE_AFTER_MS {
                modified_ms
            } else {
                NOW
            };
            *self.file.borrow_mut() = Some((text.to_owned(), stamped));
            Ok(RemoteFile {
                id: "id".to_owned(),
                modified_ms: stamped,
            })
        }
    }

    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "timeslay-config-sync-{name}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn config(&self) -> PathBuf {
            self.0.join("config.toml")
        }

        fn write(&self, text: &str, modified_ms: i64) {
            write_local(&self.config(), text, modified_ms).unwrap();
        }

        fn read(&self) -> String {
            fs::read_to_string(self.config()).unwrap_or_default()
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sync(engine: &mut Engine, dir: &Dir, drive: &FakeDrive, poll: bool) {
        engine
            .step(&dir.config(), poll, |_| Ok(drive))
            .map(|_| ())
            .unwrap();
    }

    #[test]
    fn blank_and_comment_only_text_is_empty() {
        assert_eq!(classify(""), Kind::Empty);
        assert_eq!(classify("  \n"), Kind::Empty);
        assert_eq!(classify("# nothing yet\n"), Kind::Empty);
        assert_eq!(classify("[timer]\nlast_duration_secs = 5"), Kind::Ready);
        assert_eq!(classify("[timer\n"), Kind::Invalid);
    }

    #[test]
    fn the_newer_side_wins_a_conflict() {
        let dir = Dir::new("newest");
        dir.write("a = 1", NOW);
        let drive = FakeDrive::holding("a = 2", NOW - 60_000);
        sync(&mut Engine::default(), &dir, &drive, true);
        assert_eq!(drive.text().unwrap(), "a = 1");

        let dir = Dir::new("newest-remote");
        dir.write("a = 1", NOW - 60_000);
        let drive = FakeDrive::holding("a = 2", NOW);
        sync(&mut Engine::default(), &dir, &drive, true);
        assert_eq!(dir.read(), "a = 2");
    }

    #[test]
    fn a_download_is_stamped_with_drives_time() {
        let dir = Dir::new("stamp");
        let drive = FakeDrive::holding("a = 2", NOW);
        sync(&mut Engine::default(), &dir, &drive, true);

        assert_eq!(read_local(&dir.config()).unwrap().modified_ms, NOW);
        // And a second launch sees the two as already in step.
        sync(&mut Engine::default(), &dir, &drive, true);
        assert_eq!(*drive.downloads.borrow(), 1);
        assert_eq!(*drive.uploads.borrow(), 0);
    }

    #[test]
    fn an_empty_local_config_never_replaces_drives() {
        for text in ["", "# just a comment\n"] {
            let dir = Dir::new("empty-local");
            dir.write(text, NOW + 60_000);
            let drive = FakeDrive::holding("a = 2", NOW);
            sync(&mut Engine::default(), &dir, &drive, true);
            assert_eq!(drive.text().unwrap(), "a = 2");
            // Rather, Drive fills it in.
            assert_eq!(dir.read(), "a = 2");
        }
    }

    #[test]
    fn an_empty_drive_config_never_replaces_the_local_one() {
        let dir = Dir::new("empty-remote");
        dir.write("a = 1", NOW - 60_000);
        let drive = FakeDrive::holding("   \n", NOW);
        sync(&mut Engine::default(), &dir, &drive, true);
        assert_eq!(dir.read(), "a = 1");
        assert_eq!(drive.text().unwrap(), "a = 1");
    }

    #[test]
    fn a_missing_local_file_is_filled_from_drive() {
        let dir = Dir::new("missing");
        let drive = FakeDrive::holding("a = 2", NOW);
        sync(&mut Engine::default(), &dir, &drive, true);
        assert_eq!(dir.read(), "a = 2");
    }

    #[test]
    fn nothing_is_uploaded_from_an_empty_local_file_to_an_empty_drive() {
        let dir = Dir::new("both-empty");
        let drive = FakeDrive::default();
        sync(&mut Engine::default(), &dir, &drive, true);
        assert_eq!(drive.text(), None);
    }

    #[test]
    fn a_first_config_is_uploaded() {
        let dir = Dir::new("first");
        dir.write("a = 1", NOW);
        let drive = FakeDrive::default();
        sync(&mut Engine::default(), &dir, &drive, true);
        assert_eq!(drive.text().unwrap(), "a = 1");
    }

    #[test]
    fn an_invalid_local_file_is_neither_uploaded_nor_replaced() {
        let dir = Dir::new("invalid");
        dir.write("[timer\n", NOW + 60_000);
        let drive = FakeDrive::holding("a = 2", NOW);
        sync(&mut Engine::default(), &dir, &drive, true);
        assert_eq!(dir.read(), "[timer\n");
        assert_eq!(drive.text().unwrap(), "a = 2");
        assert_eq!(*drive.uploads.borrow(), 0);
    }

    #[test]
    fn a_placeholder_stamped_with_the_epoch_loses_to_drive() {
        let dir = Dir::new("bootstrap");
        dir.write("[calendar]\nclient_id = \"x\"\n", 0);
        let drive = FakeDrive::holding(
            "[calendar]\nclient_id = \"x\"\n[timer]\nlast_duration_secs = 5\n",
            NOW,
        );
        sync(&mut Engine::default(), &dir, &drive, true);
        assert!(dir.read().contains("last_duration_secs"));
    }

    #[test]
    fn a_local_edit_is_uploaded_and_the_other_side_is_then_left_alone() {
        let dir = Dir::new("edit");
        dir.write("a = 1", NOW);
        let drive = FakeDrive::holding("a = 1", NOW - 1_000);
        let mut engine = Engine::default();
        sync(&mut engine, &dir, &drive, true);
        assert_eq!(*drive.uploads.borrow(), 0, "identical text is not re-sent");

        dir.write("a = 3", NOW + 5_000);
        sync(&mut engine, &dir, &drive, false);
        assert_eq!(drive.text().unwrap(), "a = 3");
        assert_eq!(*drive.uploads.borrow(), 1);

        // The echo of our own upload changes nothing.
        sync(&mut engine, &dir, &drive, false);
        sync(&mut engine, &dir, &drive, true);
        assert_eq!(*drive.uploads.borrow(), 1);
        assert_eq!(*drive.downloads.borrow(), 1);
    }

    #[test]
    fn a_change_made_elsewhere_wins_over_an_untouched_local_file() {
        let dir = Dir::new("remote-edit");
        dir.write("a = 1", NOW);
        let drive = FakeDrive::holding("a = 1", NOW);
        let mut engine = Engine::default();
        sync(&mut engine, &dir, &drive, true);

        // Another device saves earlier by this device's clock, and this device
        // rewrites the same text later. Neither should win on timestamps.
        *drive.file.borrow_mut() = Some(("a = 9".to_owned(), NOW + 1_000));
        dir.write("a = 1", NOW + 50_000);
        sync(&mut engine, &dir, &drive, true);
        assert_eq!(dir.read(), "a = 9");
        assert_eq!(*drive.uploads.borrow(), 0);
    }

    #[test]
    fn when_both_changed_the_newest_timestamp_wins() {
        let dir = Dir::new("both");
        dir.write("a = 1", NOW);
        let drive = FakeDrive::holding("a = 1", NOW);
        let mut engine = Engine::default();
        sync(&mut engine, &dir, &drive, true);

        *drive.file.borrow_mut() = Some(("a = 9".to_owned(), NOW + 10_000));
        dir.write("a = 5", NOW + 20_000);
        sync(&mut engine, &dir, &drive, true);
        assert_eq!(drive.text().unwrap(), "a = 5");

        *drive.file.borrow_mut() = Some(("a = 8".to_owned(), NOW + 40_000));
        dir.write("a = 6", NOW + 30_000);
        sync(&mut engine, &dir, &drive, true);
        assert_eq!(dir.read(), "a = 8");
    }

    #[test]
    fn credentials_come_from_the_calendar_table() {
        let (id, secret) = credentials("[calendar]\nclient_id = \"abc\"\nclient_secret = \"s\"");
        assert_eq!((id.as_str(), secret.as_str()), ("abc", "s"));
        assert_eq!(credentials("not toml ["), (String::new(), String::new()));
    }
}
