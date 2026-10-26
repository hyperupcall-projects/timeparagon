//! The count-up clock.
//!
//! Everything is derived from the wall clock rather than accumulated from
//! ticks, so the readout cannot drift no matter how irregular the tick rate
//! is. There is a single clock: it counts up from zero, and if a target time
//! is set it fires the alarm on the way past that target, then keeps counting.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Running {
        since: Instant,
        accumulated: Duration,
    },
    Paused {
        accumulated: Duration,
    },
}

/// One clock that counts up, with an optional target that fires the alarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timer {
    state: State,
    /// The time at which the alarm fires. `None` is a plain count-up.
    target: Option<Duration>,
    /// Whether the alarm has already fired during the current run, so it
    /// cannot fire again on every tick once the target has been passed.
    fired: bool,
}

impl Timer {
    pub fn new(target: Option<Duration>) -> Self {
        Self {
            state: State::Idle,
            target,
            fired: false,
        }
    }

    /// The time elapsed on the clock.
    pub fn elapsed(&self, now: Instant) -> Duration {
        match self.state {
            State::Idle => Duration::ZERO,
            State::Running { since, accumulated } => {
                accumulated + now.saturating_duration_since(since)
            }
            State::Paused { accumulated } => accumulated,
        }
    }

    /// The target time, if one has been set.
    pub fn target(&self) -> Option<Duration> {
        self.target
    }

    pub fn has_target(&self) -> bool {
        self.target.is_some()
    }

    /// Whether the alarm has already gone off for the current run.
    pub fn has_fired(&self) -> bool {
        self.fired
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, State::Running { .. })
    }

    pub fn is_idle(&self) -> bool {
        matches!(self.state, State::Idle)
    }

    /// How far through the target we are, from 0.0 to 1.0. Zero with no target.
    pub fn progress(&self, now: Instant) -> f32 {
        match self.target {
            Some(target) if !target.is_zero() => {
                (self.elapsed(now).as_secs_f32() / target.as_secs_f32()).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }

    /// Starts from zero, or resumes from wherever it was paused.
    pub fn start(&mut self, now: Instant) {
        match self.state {
            State::Idle => {
                self.state = State::Running {
                    since: now,
                    accumulated: Duration::ZERO,
                };
            }
            State::Paused { accumulated } => {
                self.state = State::Running {
                    since: now,
                    accumulated,
                };
            }
            State::Running { .. } => {}
        }
    }

    pub fn pause(&mut self, now: Instant) {
        if let State::Running { since, accumulated } = self.state {
            self.state = State::Paused {
                accumulated: accumulated + now.saturating_duration_since(since),
            };
        }
    }

    pub fn toggle(&mut self, now: Instant) {
        if self.is_running() {
            self.pause(now);
        } else {
            self.start(now);
        }
    }

    /// Returns the elapsed time to zero without touching the target.
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.fired = false;
    }

    /// Replaces the target and re-arms the clock from zero.
    pub fn set_target(&mut self, target: Option<Duration>) {
        self.target = target;
        self.state = State::Idle;
        self.fired = false;
    }

    /// Advances the clock to `now`, reporting whether the alarm should fire.
    ///
    /// The transition is detected here rather than in the view so that it
    /// fires exactly once per run, regardless of how many times we redraw.
    /// Passing the target never stops the clock: it just keeps counting up.
    pub fn poll(&mut self, now: Instant) -> bool {
        if self.fired || !self.is_running() {
            return false;
        }

        let reached = self
            .target
            .is_some_and(|target| self.elapsed(now) >= target);

        if reached {
            self.fired = true;
        }

        reached
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn a_new_timer_is_at_zero() {
        let timer = Timer::new(Some(MINUTE));
        assert_eq!(timer.elapsed(Instant::now()), Duration::ZERO);
        assert!(!timer.is_running());
        assert!(!timer.has_fired());
    }

    #[test]
    fn a_running_timer_counts_up_against_the_clock() {
        let start = Instant::now();
        let mut timer = Timer::new(None);
        timer.start(start);

        assert_eq!(
            timer.elapsed(start + Duration::from_secs(20)),
            Duration::from_secs(20)
        );
    }

    #[test]
    fn pausing_freezes_the_elapsed_and_resuming_does_not_count_the_gap() {
        let start = Instant::now();
        let mut timer = Timer::new(None);
        timer.start(start);
        timer.pause(start + Duration::from_secs(20));

        let paused_at = start + Duration::from_secs(20);
        assert_eq!(timer.elapsed(paused_at), Duration::from_secs(20));
        assert_eq!(
            timer.elapsed(paused_at + Duration::from_secs(600)),
            Duration::from_secs(20)
        );

        let resumed_at = paused_at + Duration::from_secs(600);
        timer.start(resumed_at);
        assert_eq!(
            timer.elapsed(resumed_at + Duration::from_secs(10)),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn poll_fires_the_alarm_exactly_once_at_the_target() {
        let start = Instant::now();
        let mut timer = Timer::new(Some(Duration::from_secs(5)));
        timer.start(start);

        assert!(!timer.poll(start + Duration::from_secs(4)));
        assert!(timer.poll(start + Duration::from_secs(5)));
        assert!(!timer.poll(start + Duration::from_secs(6)));
        assert!(timer.has_fired());
    }

    #[test]
    fn a_timer_with_no_target_never_fires() {
        let start = Instant::now();
        let mut timer = Timer::new(None);
        timer.start(start);

        assert!(!timer.poll(start + Duration::from_secs(3600)));
    }

    #[test]
    fn passing_the_target_keeps_counting() {
        let start = Instant::now();
        let mut timer = Timer::new(Some(Duration::from_secs(5)));
        timer.start(start);

        let _ = timer.poll(start + Duration::from_secs(5));
        assert_eq!(
            timer.elapsed(start + Duration::from_secs(12)),
            Duration::from_secs(12)
        );
    }

    #[test]
    fn reset_zeroes_the_clock_but_keeps_the_target() {
        let start = Instant::now();
        let mut timer = Timer::new(Some(MINUTE));
        timer.start(start);
        let _ = timer.poll(start + MINUTE);
        timer.reset();

        assert_eq!(timer.elapsed(start), Duration::ZERO);
        assert_eq!(timer.target(), Some(MINUTE));
        assert!(!timer.has_fired());
    }

    #[test]
    fn set_target_replaces_the_target_and_restarts_from_zero() {
        let start = Instant::now();
        let mut timer = Timer::new(Some(MINUTE));
        timer.start(start);
        timer.set_target(Some(Duration::from_secs(90)));

        assert_eq!(timer.target(), Some(Duration::from_secs(90)));
        assert_eq!(timer.elapsed(start), Duration::ZERO);
        assert!(!timer.is_running());
    }

    #[test]
    fn clearing_the_target_turns_off_the_alarm() {
        let mut timer = Timer::new(Some(MINUTE));
        timer.set_target(None);

        assert!(!timer.has_target());
        assert!(!timer.poll(Instant::now() + Duration::from_secs(3600)));
    }

    #[test]
    fn progress_tracks_elapsed_fraction() {
        let start = Instant::now();
        let mut timer = Timer::new(Some(MINUTE));
        timer.start(start);

        assert_eq!(timer.progress(start), 0.0);
        assert_eq!(timer.progress(start + Duration::from_secs(30)), 0.5);
        assert_eq!(timer.progress(start + Duration::from_secs(60)), 1.0);
        assert_eq!(timer.progress(start + Duration::from_secs(90)), 1.0);
    }

    #[test]
    fn progress_is_zero_without_a_target() {
        let start = Instant::now();
        let mut timer = Timer::new(None);
        timer.start(start);

        assert_eq!(timer.progress(start + Duration::from_secs(30)), 0.0);
    }
}
