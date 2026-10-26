//! Persistent work sessions keyed by local task IDs, with aliases for migrated UUIDs.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub task_key: String,
    pub task_title: String,
    pub block_key: Option<String>,
    pub started_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
}
impl Session {
    pub fn seconds(&self, now: DateTime<Utc>) -> u64 {
        (self.stopped_at.unwrap_or(now) - self.started_at)
            .num_seconds()
            .max(0) as u64
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkLog {
    pub sessions: Vec<Session>,
}
impl WorkLog {
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
    pub fn active(&self) -> Option<&Session> {
        self.sessions.iter().rev().find(|s| s.stopped_at.is_none())
    }
    pub fn running(&self, key: &str) -> bool {
        self.active().is_some_and(|s| s.task_key == key)
    }
    pub fn seconds(&self, key: &str, now: DateTime<Utc>) -> u64 {
        self.sessions
            .iter()
            .filter(|s| s.task_key == key)
            .fold(0u64, |sum, s| sum.saturating_add(s.seconds(now)))
    }
    pub fn start(&mut self, key: String, title: String, block: Option<String>, now: DateTime<Utc>) {
        if self.running(&key) {
            return;
        }
        self.stop_all(now);
        self.sessions.push(Session {
            task_key: key,
            task_title: title,
            block_key: block,
            started_at: now,
            stopped_at: None,
        });
    }
    pub fn stop(&mut self, key: &str, now: DateTime<Utc>) {
        for session in self
            .sessions
            .iter_mut()
            .filter(|s| s.task_key == key && s.stopped_at.is_none())
        {
            session.stopped_at = Some(now.max(session.started_at));
        }
    }
    pub fn stop_all(&mut self, now: DateTime<Utc>) {
        for session in self.sessions.iter_mut().filter(|s| s.stopped_at.is_none()) {
            session.stopped_at = Some(now.max(session.started_at));
        }
    }
}
pub fn clock(seconds: u64) -> String {
    format!(
        "{}:{:02}:{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn switching_stops_previous_and_resuming_accumulates() {
        let start = DateTime::from_timestamp(1000, 0).unwrap();
        let mut log = WorkLog::default();
        log.start("a".into(), "First".into(), None, start);
        log.start(
            "a".into(),
            "First".into(),
            None,
            start + chrono::Duration::seconds(5),
        );
        assert_eq!(log.sessions.len(), 1);
        log.start(
            "b".into(),
            "Second".into(),
            None,
            start + chrono::Duration::seconds(20),
        );
        assert_eq!(log.seconds("a", start + chrono::Duration::seconds(30)), 20);
        assert!(log.running("b"));
        log.start(
            "a".into(),
            "First".into(),
            None,
            start + chrono::Duration::seconds(35),
        );
        log.stop("a", start + chrono::Duration::seconds(45));
        assert_eq!(log.seconds("a", start + chrono::Duration::days(1)), 30);
        assert_eq!(log.seconds("b", start + chrono::Duration::days(1)), 15);
        assert!(log.active().is_none());
    }
    #[test]
    fn active_sessions_survive_serialization_and_stopped_sessions_stay_frozen() {
        let start = DateTime::from_timestamp(1000, 0).unwrap();
        let mut log = WorkLog::default();
        log.start(
            "ttid".into(),
            "Task title".into(),
            Some("block".into()),
            start,
        );
        let encoded = toml::to_string(&log).unwrap();
        let mut restored: WorkLog = toml::from_str(&encoded).unwrap();
        assert_eq!(
            restored.seconds("ttid", start + chrono::Duration::seconds(42)),
            42
        );
        restored.stop("ttid", start + chrono::Duration::seconds(42));
        assert_eq!(
            restored.seconds("ttid", start + chrono::Duration::days(1)),
            42
        );
    }
}
