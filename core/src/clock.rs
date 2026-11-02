//! The wall clock, with a seam for tests.
//!
//! Dates decide what the app shows, so code reads the time from here rather
//! than from chrono directly. A test can then pin the clock to the moment it
//! is about, such as a daylight saving change, and stop depending on the day
//! it happens to run.
//!
//! The pin is per thread so tests running in parallel cannot disturb each
//! other. Work on another thread, like a blocking worker, sees the real clock.

use std::cell::Cell;
use std::marker::PhantomData;

use chrono::{DateTime, Local, Utc};

thread_local! {
    static FROZEN: Cell<Option<DateTime<Utc>>> = const { Cell::new(None) };
}

/// The current local time, or the frozen one in a test.
pub fn now() -> DateTime<Local> {
    utc_now().with_timezone(&Local)
}

/// The current UTC time, or the frozen one in a test.
pub fn utc_now() -> DateTime<Utc> {
    FROZEN.get().unwrap_or_else(Utc::now)
}

/// Pins this thread's clock until the returned guard is dropped.
pub fn freeze(at: DateTime<Utc>) -> Frozen {
    Frozen {
        previous: FROZEN.replace(Some(at)),
        _same_thread: PhantomData,
    }
}

/// Restores the previous clock when dropped. It has to be dropped on the
/// thread it was frozen on, so it is deliberately not `Send`.
#[must_use = "the clock unfreezes when this is dropped"]
pub struct Frozen {
    previous: Option<DateTime<Utc>>,
    _same_thread: PhantomData<*const ()>,
}

impl Drop for Frozen {
    fn drop(&mut self) {
        FROZEN.set(self.previous);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frozen_clock_reads_back_and_thaws_in_order() {
        let outer = DateTime::parse_from_rfc3339("2026-11-01T08:30:00Z")
            .unwrap()
            .to_utc();
        let inner = DateTime::parse_from_rfc3339("2027-03-14T09:30:00Z")
            .unwrap()
            .to_utc();

        let first = freeze(outer);
        assert_eq!(utc_now(), outer);
        assert_eq!(now().to_utc(), outer);
        {
            let _second = freeze(inner);
            assert_eq!(utc_now(), inner);
        }
        assert_eq!(utc_now(), outer);
        drop(first);
        assert_ne!(utc_now(), outer);
    }

    #[test]
    fn the_pin_does_not_reach_other_threads() {
        let pinned = DateTime::parse_from_rfc3339("2001-01-01T00:00:00Z")
            .unwrap()
            .to_utc();
        let _frozen = freeze(pinned);
        let elsewhere = std::thread::spawn(utc_now).join().unwrap();
        assert_ne!(elsewhere, pinned);
    }
}
