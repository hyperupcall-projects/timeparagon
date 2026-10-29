//! An independent, persistent work/break clock. Task work remains in WorkLog.
use crate::task_work::{Session, WorkLog};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    #[default]
    Work,
    Break,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pomodoro {
    pub work_minutes: u32,
    pub break_minutes: u32,
    pub phase: Phase,
    pub remaining: u64,
    pub work_seconds: u64,
    pub break_seconds: u64,
    pub running_since: Option<DateTime<Utc>>,
    /// The task to resume after a break; never itself an active work session.
    pub target: Option<Session>,
}
impl Default for Pomodoro {
    fn default() -> Self {
        Self {
            work_minutes: 25,
            break_minutes: 5,
            phase: Phase::Work,
            remaining: 25 * 60,
            work_seconds: 0,
            break_seconds: 0,
            running_since: None,
            target: None,
        }
    }
}
impl Pomodoro {
    pub fn running(&self) -> bool {
        self.running_since.is_some()
    }
    pub fn fresh(&self) -> bool {
        !self.running() && self.work_seconds == 0 && self.break_seconds == 0
    }
    pub fn duration(&self) -> u64 {
        u64::from(
            match self.phase {
                Phase::Work => self.work_minutes,
                Phase::Break => self.break_minutes,
            }
            .clamp(1, 180),
        ) * 60
    }
    /// Consume actual elapsed time, splitting the task log at exact boundaries
    /// even when ticks are late or the application was closed for a while.
    pub fn advance(
        &mut self,
        log: &mut WorkLog,
        now: DateTime<Utc>,
        eligible: impl Fn(&str) -> bool,
    ) -> bool {
        let Some(mut cursor) = self.running_since else {
            return false;
        };
        let mut elapsed = (now - cursor).num_seconds().max(0) as u64;
        let mut transitioned = false;
        self.remaining = self.remaining.clamp(1, self.duration());
        while elapsed > 0 {
            let seconds = elapsed.min(self.remaining);
            match self.phase {
                Phase::Work => self.work_seconds = self.work_seconds.saturating_add(seconds),
                Phase::Break => self.break_seconds = self.break_seconds.saturating_add(seconds),
            }
            self.remaining -= seconds;
            elapsed -= seconds;
            cursor += Duration::seconds(seconds as i64);
            if self.remaining == 0 {
                transitioned = true;
                match self.phase {
                    Phase::Work => {
                        self.target = log.active().cloned();
                        log.stop_all(cursor);
                        self.phase = Phase::Break;
                    }
                    Phase::Break => {
                        self.phase = Phase::Work;
                        self.resume_task(log, cursor, &eligible);
                    }
                }
                self.remaining = self.duration();
            }
        }
        self.running_since = Some(cursor);
        transitioned
    }
    pub fn resume_task(
        &mut self,
        log: &mut WorkLog,
        now: DateTime<Utc>,
        eligible: impl Fn(&str) -> bool,
    ) {
        if let Some(target) = &self.target {
            if eligible(&target.task_key) {
                log.start(
                    target.task_key.clone(),
                    target.task_title.clone(),
                    target.block_key.clone(),
                    now,
                );
            } else {
                self.target = None;
            }
        }
    }
    pub fn reset(&mut self) {
        *self = Self {
            work_minutes: self.work_minutes,
            break_minutes: self.break_minutes,
            ..Self::default()
        };
        self.remaining = self.duration();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn start() -> DateTime<Utc> {
        DateTime::from_timestamp(1000, 0).unwrap()
    }
    #[test]
    fn late_tick_splits_work_at_boundaries_and_counts_both_totals() {
        let now = start();
        let mut timer = Pomodoro {
            work_minutes: 1,
            break_minutes: 1,
            remaining: 60,
            running_since: Some(now),
            ..Pomodoro::default()
        };
        let mut log = WorkLog::default();
        log.start("a".into(), "Task".into(), None, now);
        assert!(timer.advance(&mut log, now + Duration::seconds(150), |_| true));
        assert_eq!(
            (timer.work_seconds, timer.break_seconds, timer.remaining),
            (90, 60, 30)
        );
        assert_eq!(log.seconds("a", now + Duration::seconds(150)), 90);
        assert_eq!(
            log.sessions[0].stopped_at,
            Some(now + Duration::seconds(60))
        );
        assert_eq!(
            log.active().unwrap().started_at,
            now + Duration::seconds(120)
        );
    }
    #[test]
    fn completed_task_is_not_resumed_and_paused_time_is_excluded() {
        let now = start();
        let mut timer = Pomodoro {
            work_minutes: 1,
            break_minutes: 1,
            remaining: 60,
            running_since: Some(now),
            ..Pomodoro::default()
        };
        let mut log = WorkLog::default();
        log.start("a".into(), "Task".into(), None, now);
        timer.advance(&mut log, now + Duration::seconds(60), |_| true);
        assert!(log.active().is_none());
        timer.running_since = None;
        timer.advance(&mut log, now + Duration::seconds(600), |_| true);
        assert_eq!(timer.break_seconds, 0);
        timer.running_since = Some(now + Duration::seconds(600));
        timer.advance(&mut log, now + Duration::seconds(660), |_| false);
        assert!(log.active().is_none());
        assert!(timer.target.is_none());
    }
    #[test]
    fn persistence_and_reset_preserve_settings() {
        let mut timer = Pomodoro {
            work_minutes: 10,
            break_minutes: 2,
            running_since: Some(start()),
            ..Pomodoro::default()
        };
        timer.remaining = timer.duration();
        let restored: Pomodoro = toml::from_str(&toml::to_string(&timer).unwrap()).unwrap();
        assert_eq!(restored, timer);
        timer.reset();
        assert_eq!(
            (timer.remaining, timer.work_seconds, timer.break_seconds),
            (600, 0, 0)
        );
        assert!(!timer.running());
    }
}
