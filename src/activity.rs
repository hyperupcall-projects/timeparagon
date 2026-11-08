//! What the app is doing in the background right now, and a log of what it
//! has done.
//!
//! The two are kept apart on purpose. What is under way (saving to Google,
//! loading the calendar, database sync) is worked out afresh from the app's
//! state after every update, and shown at the foot of the panel. The log is
//! written when something finishes, in words a person cares about ("Synced 12
//! events and 24 tasks from Google") rather than the queue of single
//! operations that the Operations screen lists.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};

/// How long each of several things under way stays on show before the next.
pub const ROTATE_EVERY: Duration = Duration::from_secs(3);
/// The most log entries kept.
const LOG_LIMIT: usize = 200;
/// What the foot of the panel says when nothing is under way.
pub const IDLE_LABEL: &str = "Logs";

/// A kind of background work. At most one of each is under way at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    SavingToCloud,
    LoadingFromCloud,
    DatabaseSync,
}

/// Something under way, and how to say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Current {
    pub kind: Kind,
    pub label: String,
}

/// Something that happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// When it last happened.
    pub at: DateTime<Local>,
    pub text: String,
    pub failed: bool,
    /// How many times in a row it happened; the same sync every few minutes
    /// is one line, not a screenful.
    pub repeats: u32,
}

#[derive(Debug, Default)]
pub struct Activity {
    current: Vec<Current>,
    shown: Option<Kind>,
    shown_since: Option<Instant>,
    log: VecDeque<LogEntry>,
    /// The last database sync log entry already copied into [`Self::log`].
    database_sequence: u64,
    /// Google operations last seen waiting, with their errors, to tell when
    /// they are saved or fail.
    operations: Vec<(i64, Option<String>)>,
    /// The popup listing the log is open.
    pub open: bool,
}

impl Activity {
    /// Takes in what is under way now. Something that has just started is
    /// shown at once; otherwise what is on show stays until it ends or it is
    /// time to rotate.
    pub fn observe(&mut self, current: Vec<Current>, now: Instant) {
        let started = current
            .iter()
            .rev()
            .find(|new| !self.current.iter().any(|old| old.kind == new.kind))
            .map(|new| new.kind);
        let shown_still_running = self
            .shown
            .is_some_and(|shown| current.iter().any(|item| item.kind == shown));
        if let Some(kind) = started {
            self.show(Some(kind), now);
        } else if !shown_still_running {
            self.show(current.first().map(|item| item.kind), now);
        }
        self.current = current;
    }

    fn show(&mut self, kind: Option<Kind>, now: Instant) {
        self.shown = kind;
        self.shown_since = kind.map(|_| now);
    }

    /// Moves on to the next thing under way once the current one has been
    /// on show for [`ROTATE_EVERY`].
    pub fn tick(&mut self, now: Instant) {
        if self.current.len() < 2 {
            return;
        }
        let due = self
            .shown_since
            .is_none_or(|since| now.duration_since(since) >= ROTATE_EVERY);
        if !due {
            return;
        }
        let index = self
            .shown
            .and_then(|shown| self.current.iter().position(|item| item.kind == shown))
            .map_or(0, |index| (index + 1) % self.current.len());
        self.show(Some(self.current[index].kind), now);
    }

    /// More than one thing is under way, so the label needs to rotate.
    pub fn rotating(&self) -> bool {
        self.current.len() > 1
    }

    /// Something is under way.
    pub fn busy(&self) -> bool {
        !self.current.is_empty()
    }

    /// What the foot of the panel says.
    pub fn label(&self) -> &str {
        self.shown
            .and_then(|shown| self.current.iter().find(|item| item.kind == shown))
            .map_or(IDLE_LABEL, |item| item.label.as_str())
    }

    /// Notes that something happened.
    pub fn record(&mut self, text: impl Into<String>, failed: bool) {
        self.record_at(crate::clock::now(), text.into(), failed);
    }

    fn record_at(&mut self, at: DateTime<Local>, text: String, failed: bool) {
        if let Some(last) = self.log.back_mut()
            && last.text == text
            && last.failed == failed
        {
            last.at = at;
            last.repeats += 1;
            return;
        }
        self.log.push_back(LogEntry {
            at,
            text,
            failed,
            repeats: 1,
        });
        while self.log.len() > LOG_LIMIT {
            self.log.pop_front();
        }
    }

    /// The log, newest first.
    pub fn log(&self) -> impl Iterator<Item = &LogEntry> {
        self.log.iter().rev()
    }

    /// Copies in what database sync has done since it was last asked.
    pub fn pull_database_log(&mut self) {
        for entry in timeslay_core::database_sync::log_since(self.database_sequence) {
            self.database_sequence = self.database_sequence.max(entry.sequence);
            let failed = !entry.text.starts_with("Sent")
                && !entry.text.starts_with("Received")
                && !entry.text.starts_with("Connected");
            self.record_at(entry.at, entry.text, failed);
        }
    }

    /// Compares a fresh read of the Google queue with the last one, and logs
    /// what was saved and what newly failed.
    pub fn observe_operations(&mut self, rows: &[timeslay_core::calendar::operations::Operation]) {
        let saved = self
            .operations
            .iter()
            .filter(|(id, _)| !rows.iter().any(|row| row.id == *id))
            .count();
        if saved > 0 {
            let plural = if saved == 1 { "" } else { "s" };
            self.record(format!("Saved {saved} change{plural} to Google"), false);
        }
        for row in rows {
            let Some(error) = &row.error else {
                continue;
            };
            let known = self
                .operations
                .iter()
                .any(|(id, seen)| *id == row.id && seen.as_ref() == Some(error));
            if !known {
                self.record(format!("Could not save to Google: {error}"), true);
            }
        }
        self.operations = rows.iter().map(|row| (row.id, row.error.clone())).collect();
    }
}

/// "Synced 12 events and 24 tasks from Google".
pub fn synced_text(events: &[timeslay_core::calendar::Event]) -> String {
    let tasks = events.iter().filter(|event| event.is_task()).count();
    let timed = events.len() - tasks;
    let count = |count: usize, noun: &str| {
        let plural = if count == 1 { "" } else { "s" };
        format!("{count} {noun}{plural}")
    };
    format!(
        "Synced {} and {} from Google",
        count(timed, "event"),
        count(tasks, "task")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: Kind, label: &str) -> Current {
        Current {
            kind,
            label: label.into(),
        }
    }

    #[test]
    fn idle_says_logs() {
        let mut activity = Activity::default();
        assert_eq!(activity.label(), IDLE_LABEL);
        activity.observe(
            vec![item(Kind::LoadingFromCloud, "Loading")],
            Instant::now(),
        );
        assert_eq!(activity.label(), "Loading");
        activity.observe(Vec::new(), Instant::now());
        assert_eq!(activity.label(), IDLE_LABEL);
    }

    #[test]
    fn what_just_started_shows_at_once_then_everything_rotates() {
        let start = Instant::now();
        let mut activity = Activity::default();
        activity.observe(vec![item(Kind::SavingToCloud, "Saving")], start);
        activity.observe(
            vec![
                item(Kind::SavingToCloud, "Saving"),
                item(Kind::LoadingFromCloud, "Loading"),
            ],
            start,
        );
        assert_eq!(activity.label(), "Loading");
        assert!(activity.rotating());

        activity.tick(start + Duration::from_secs(1));
        assert_eq!(activity.label(), "Loading");
        activity.tick(start + ROTATE_EVERY);
        assert_eq!(activity.label(), "Saving");
        activity.tick(start + ROTATE_EVERY * 2);
        assert_eq!(activity.label(), "Loading");

        // A third starting jumps the queue.
        activity.observe(
            vec![
                item(Kind::SavingToCloud, "Saving"),
                item(Kind::LoadingFromCloud, "Loading"),
                item(Kind::DatabaseSync, "Database"),
            ],
            start + ROTATE_EVERY * 2,
        );
        assert_eq!(activity.label(), "Database");

        // The one on show ending moves straight to another.
        activity.observe(
            vec![
                item(Kind::SavingToCloud, "Saving"),
                item(Kind::LoadingFromCloud, "Loading"),
            ],
            start + ROTATE_EVERY * 2,
        );
        assert_eq!(activity.label(), "Saving");
    }

    #[test]
    fn a_changing_count_keeps_the_same_item_on_show() {
        let start = Instant::now();
        let mut activity = Activity::default();
        activity.observe(
            vec![
                item(Kind::SavingToCloud, "Saving (2)"),
                item(Kind::LoadingFromCloud, "Loading"),
            ],
            start,
        );
        activity.observe(
            vec![
                item(Kind::SavingToCloud, "Saving (1)"),
                item(Kind::LoadingFromCloud, "Loading"),
            ],
            start,
        );
        assert_eq!(activity.label(), "Loading");
    }

    #[test]
    fn repeats_collapse_into_one_entry() {
        let mut activity = Activity::default();
        activity.record("Synced 1 event and 0 tasks from Google", false);
        activity.record("Synced 1 event and 0 tasks from Google", false);
        activity.record("Saved 1 change to Google", false);
        let log: Vec<_> = activity.log().collect();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].text, "Saved 1 change to Google");
        assert_eq!(log[1].repeats, 2);
    }

    #[test]
    fn saved_and_failed_operations_are_logged_once() {
        use timeslay_core::calendar::operations::Operation;
        let operation = |id, error: Option<&str>| Operation {
            id,
            kind: "time".into(),
            entity: "event".into(),
            created_at: String::new(),
            attempts: 1,
            error: error.map(Into::into),
            payload: String::new(),
            failed_at: None,
            retry_at: 0,
        };
        let mut activity = Activity::default();
        activity.observe_operations(&[operation(1, None), operation(2, None)]);
        assert_eq!(activity.log().count(), 0);
        activity.observe_operations(&[operation(2, Some("offline"))]);
        activity.observe_operations(&[operation(2, Some("offline"))]);
        let log: Vec<_> = activity.log().map(|entry| entry.text.as_str()).collect();
        assert_eq!(
            log,
            [
                "Could not save to Google: offline",
                "Saved 1 change to Google"
            ]
        );
        activity.observe_operations(&[]);
        assert_eq!(
            activity.log().next().unwrap().text,
            "Saved 1 change to Google"
        );
    }
}
