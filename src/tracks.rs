//! Time spent with the timer running, kept where it survives a restart.
//!
//! The clock counts from a monotonic source and knows nothing about the
//! calendar date, but the record has to outlive the process: closing the
//! window mid-task must not lose the morning. Every stretch with the timer
//! running is one track, opened when the timer starts and closed when it
//! pauses. The file is rewritten whenever a track opens or closes, and again
//! every [`FLUSH_EVERY`] while one is open, so a crash costs at most that.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Local};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

/// How long an open track may go unwritten while the timer runs.
pub const FLUSH_EVERY: Duration = Duration::from_secs(5 * 60);

/// One stretch with the timer running, in wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub start: DateTime<Local>,
    pub stop: DateTime<Local>,
}

impl Track {
    /// How long the stretch lasted, ignoring a clock that went backwards.
    pub fn span(&self) -> Duration {
        (self.stop - self.start).to_std().unwrap_or(Duration::ZERO)
    }
}

/// The record as the file holds it: `{"tracks": [...]}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tracks {
    pub tracks: Vec<Track>,
}

/// The record as the app holds it: the tracks, plus which end of the last
/// one the running timer is still writing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recorder {
    tracks: Tracks,
    /// Whether `tracks.last()` belongs to a timer that is still running.
    open: bool,
    /// Where the record is written. `None` for a recorder built by hand in
    /// a test, which keeps its tracks in memory and leaves the disk alone.
    path: Option<PathBuf>,
}

impl Recorder {
    /// Points the recorder at a file and takes in whatever an earlier run
    /// left there.
    ///
    /// Called at boot, in the same spirit as the day cache: a recorder built
    /// by hand must not read or rewrite the user's own record.
    pub fn keep_in(&mut self, path: Option<PathBuf>) {
        self.path = path.clone();

        if let Some(tracks) = path.as_deref().and_then(read) {
            self.tracks = tracks;
        }
    }

    /// Every stretch recorded so far, including one still open.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks.tracks
    }

    /// The time the tracks add up to.
    pub fn total(&self) -> Duration {
        self.tracks.tracks.iter().map(Track::span).sum()
    }

    /// Whether the running timer's stretch is being written right now.
    pub fn is_recording(&self) -> bool {
        self.open
    }

    /// Whether there is a file to write to.
    pub fn is_kept(&self) -> bool {
        self.path.is_some()
    }

    /// Brings the record in line with the timer: a running timer has an open
    /// track, a paused or idle one does not.
    ///
    /// Returns whether the file wants rewriting, which is exactly when a
    /// track has just opened or closed.
    pub fn follow(&mut self, running: bool, now: DateTime<Local>) -> bool {
        match (running, self.open) {
            (true, false) => {
                self.tracks.tracks.push(Track { start: now, stop: now });
                self.open = true;
                true
            }
            (false, true) => {
                self.open = false;
                self.extend(now);
                true
            }
            _ => false,
        }
    }

    /// Moves the open track's end to `now`, the way a periodic flush does.
    ///
    /// Returns whether there was anything to flush.
    pub fn flush(&mut self, now: DateTime<Local>) -> bool {
        if !self.open {
            return false;
        }

        self.extend(now);
        true
    }

    fn extend(&mut self, now: DateTime<Local>) {
        if let Some(track) = self.tracks.tracks.last_mut() {
            track.stop = now;
        }
    }

    /// Writes the record out. A recorder with nowhere to write is content to
    /// keep everything in memory.
    pub fn save(&self) -> io::Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };

        write(path, &self.tracks)
    }
}

/// Where the record lives: `$XDG_STATE_HOME/timekeeper`, which is
/// `~/.local/state/timekeeper` unless the environment says otherwise.
pub fn path() -> Option<PathBuf> {
    Some(BaseDirs::new()?.state_dir()?.join("timekeeper"))
}

/// The record as an earlier run left it, or nothing when there is none.
fn read(path: &Path) -> Option<Tracks> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            log::warn!("{} could not be read: {error}", path.display());
            return None;
        }
    };

    match serde_json::from_slice(&bytes) {
        Ok(tracks) => Some(tracks),
        Err(error) => {
            log::warn!("{} could not be parsed, ignoring it: {error}", path.display());
            None
        }
    }
}

/// Replaces the record, going through a temporary file so a crash mid-write
/// cannot leave a truncated one behind.
fn write(path: &Path, tracks: &Tracks) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let bytes = serde_json::to_vec_pretty(tracks)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)
}

/// A throwaway file under the system's temp dir, removed when the test that
/// made it ends either way.
#[cfg(test)]
struct Scratch(PathBuf);

#[cfg(test)]
impl Scratch {
    fn new(name: &str) -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or_default();

        let path = std::env::temp_dir().join(format!(
            "timetracker-tracks-{}-{unique}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);

        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[cfg(test)]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    fn minutes(n: i64) -> chrono::TimeDelta {
        chrono::TimeDelta::minutes(n)
    }

    #[test]
    fn the_record_lives_under_the_xdg_state_directory() {
        let path = path().expect("a state directory");

        assert!(
            path.ends_with("timekeeper"),
            "{} should be the timekeeper file in the XDG state directory",
            path.display()
        );
    }

    #[test]
    fn starting_opens_a_track_and_pausing_closes_it() {
        let mut recorder = Recorder::default();
        let started = Local::now();

        assert!(recorder.follow(true, started));
        assert!(recorder.is_recording());
        assert_eq!(recorder.tracks().len(), 1);
        assert_eq!(recorder.tracks()[0].span(), Duration::ZERO);

        // A timer that keeps running has nothing new to say.
        assert!(!recorder.follow(true, started + minutes(1)));
        assert_eq!(recorder.tracks().len(), 1);

        let paused = started + minutes(1);
        assert!(recorder.follow(false, paused));
        assert!(!recorder.is_recording());
        assert_eq!(recorder.tracks()[0].span(), MINUTE);
    }

    #[test]
    fn starting_again_opens_the_next_track() {
        let mut recorder = Recorder::default();
        let started = Local::now();

        let _ = recorder.follow(true, started);
        let _ = recorder.follow(false, started + minutes(1));
        let _ = recorder.follow(true, started + minutes(2));

        assert_eq!(recorder.tracks().len(), 2);
        assert!(recorder.is_recording());
    }

    #[test]
    fn a_flush_moves_the_end_of_the_open_track() {
        let mut recorder = Recorder::default();
        let started = Local::now();

        let _ = recorder.follow(true, started);

        assert!(
            recorder.flush(started + minutes(5)),
            "an open track has something to flush"
        );
        assert_eq!(recorder.tracks()[0].span(), MINUTE * 5);

        let _ = recorder.follow(false, started + minutes(10));

        assert!(
            !recorder.flush(started + minutes(20)),
            "a closed track has nothing to flush"
        );
        assert_eq!(recorder.tracks()[0].span(), MINUTE * 10);
    }

    #[test]
    fn the_file_is_a_list_of_stretches_with_a_start_and_a_stop() {
        let scratch = Scratch::new("shape");
        let mut recorder = Recorder::default();
        recorder.keep_in(Some(scratch.path().to_owned()));

        let started = Local::now();
        let _ = recorder.follow(true, started);
        let _ = recorder.follow(false, started + minutes(1));
        recorder.save().unwrap();

        let text = fs::read_to_string(scratch.path()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();

        let tracks = parsed
            .get("tracks")
            .and_then(|tracks| tracks.as_array())
            .unwrap_or_else(|| panic!("{text} should hold a list of tracks"));

        assert_eq!(tracks.len(), 1);
        assert!(
            tracks[0].get("start").and_then(|start| start.as_str()).is_some(),
            "{text}"
        );
        assert!(
            tracks[0].get("stop").and_then(|stop| stop.as_str()).is_some(),
            "{text}"
        );
    }

    #[test]
    fn a_saved_record_reads_back() {
        let scratch = Scratch::new("round-trip");
        let mut recorder = Recorder::default();
        recorder.keep_in(Some(scratch.path().to_owned()));

        let started = Local::now();
        let _ = recorder.follow(true, started);
        let _ = recorder.follow(false, started + minutes(1));
        recorder.save().unwrap();

        let mut reopened = Recorder::default();
        reopened.keep_in(Some(scratch.path().to_owned()));

        assert_eq!(reopened.tracks(), recorder.tracks());
        assert!(!reopened.is_recording(), "an open stretch does not reopen");
        assert_eq!(reopened.total(), MINUTE);
    }

    #[test]
    fn a_file_that_was_never_written_reads_as_nothing() {
        let scratch = Scratch::new("missing");
        let mut recorder = Recorder::default();

        recorder.keep_in(Some(scratch.path().to_owned()));

        assert!(recorder.tracks().is_empty());
    }

    #[test]
    fn an_unreadable_record_is_ignored_rather_than_raised() {
        let scratch = Scratch::new("broken");
        fs::write(scratch.path(), b"not json").unwrap();

        let mut recorder = Recorder::default();
        recorder.keep_in(Some(scratch.path().to_owned()));

        assert!(recorder.tracks().is_empty());
    }

    #[test]
    fn a_recorder_with_nowhere_to_write_stays_in_memory() {
        let mut recorder = Recorder::default();
        assert!(!recorder.is_kept());

        let _ = recorder.follow(true, Local::now());
        recorder.save().unwrap();

        assert!(recorder.is_recording());
    }
}
