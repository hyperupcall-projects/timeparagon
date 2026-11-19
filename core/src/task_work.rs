//! Persistent work sessions keyed by local task IDs.
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
/// The lengths a distraction can be recorded with, in minutes.
pub const DISTRACTION_MINUTES: [u32; 5] = [1, 5, 10, 15, 30];
/// Time lost to something else while a time block was in focus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Distraction {
	pub block_key: String,
	pub minutes: u32,
	pub recorded_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkLog {
	pub sessions: Vec<Session>,
	pub distractions: Vec<Distraction>,
}
impl WorkLog {
	pub fn is_empty(&self) -> bool {
		self.sessions.is_empty() && self.distractions.is_empty()
	}
	/// How many distractions a block has, and the minutes they cost in total.
	pub fn distracted(&self, block: &str) -> (usize, u32) {
		self.distractions
			.iter()
			.filter(|d| d.block_key == block)
			.fold((0, 0), |(count, minutes), d| {
				(count + 1, minutes.saturating_add(d.minutes))
			})
	}
	/// Take back the block's latest distraction, for a mistaken press.
	pub fn undo_distraction(&mut self, block: &str) {
		if let Some(index) = self.distractions.iter().rposition(|d| d.block_key == block) {
			self.distractions.remove(index);
		}
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
	fn distractions_add_up_per_block_and_undo_takes_back_only_that_blocks_latest() {
		let at = DateTime::from_timestamp(1000, 0).unwrap();
		let mut log = WorkLog::default();
		assert!(log.is_empty());
		for (block, minutes) in [("a", 5), ("b", 30), ("a", 10)] {
			log.distractions.push(Distraction {
				block_key: block.into(),
				minutes,
				recorded_at: at,
			});
		}
		assert!(!log.is_empty());
		assert_eq!(log.distracted("a"), (2, 15));
		assert_eq!(log.distracted("c"), (0, 0));
		log.undo_distraction("a");
		assert_eq!(log.distracted("a"), (1, 5));
		assert_eq!(log.distracted("b"), (1, 30));
		log.undo_distraction("c");
		assert_eq!(log.distractions.len(), 2);
	}
	#[test]
	fn a_log_saved_before_distractions_existed_still_loads_and_they_round_trip() {
		let old: WorkLog = toml::from_str("sessions = []").unwrap();
		assert!(old.distractions.is_empty());
		let mut log = WorkLog::default();
		log.distractions.push(Distraction {
			block_key: "block".into(),
			minutes: 15,
			recorded_at: DateTime::from_timestamp(1000, 0).unwrap(),
		});
		let restored: WorkLog = toml::from_str(&toml::to_string(&log).unwrap()).unwrap();
		assert_eq!(restored, log);
	}
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
