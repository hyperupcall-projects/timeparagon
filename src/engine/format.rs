//! Turning durations into the strings on the readout.

use std::time::Duration;

/// How much of the duration to spell out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    /// `1:23`, or `1:02:03` once there are hours to show.
    Seconds,
    /// `1:23.45`, for a clock that shows hundredths.
    Centis,
}

/// Formats a duration for the big readout.
///
/// `allow_hours` is what the layout uses to keep the readout inside a narrow
/// window: when it is `false` the hours are folded into the minutes, so a
/// 90 minute timer reads `90:00` instead of `1:30:00`.
pub fn readout(duration: Duration, precision: Precision, allow_hours: bool) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);

    match precision {
        Precision::Seconds => {
            if hours > 0 && allow_hours {
                format!("{hours}:{minutes:02}:{seconds:02}")
            } else if hours > 0 {
                format!("{}:{seconds:02}", hours * 60 + minutes)
            } else {
                format!("{minutes}:{seconds:02}")
            }
        }
        Precision::Centis => {
            let centis = duration.subsec_millis() / 10;

            if hours > 0 && allow_hours {
                format!("{hours}:{minutes:02}:{seconds:02}.{centis:02}")
            } else if hours > 0 {
                format!("{}:{seconds:02}.{centis:02}", hours * 60 + minutes)
            } else {
                format!("{minutes}:{seconds:02}.{centis:02}")
            }
        }
    }
}

/// Formats a duration as a short label, the way a preset chip reads.
pub fn compact(duration: Duration) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);

    match (hours, minutes, seconds) {
        (0, 0, s) => format!("{s}s"),
        (0, m, 0) => format!("{m}m"),
        (0, m, s) => format!("{m}m{s:02}"),
        (h, 0, 0) => format!("{h}h"),
        (h, m, _) => format!("{h}h{m:02}"),
    }
}

/// Rounds a remaining duration up to whole seconds.
///
/// A countdown should read `0:01` for the whole of the final second and only
/// reach `0:00` when the time is actually up; truncating instead would show
/// `0:00` for a full second while the timer is still running.
pub fn ceil_secs(duration: Duration) -> Duration {
    if duration.subsec_nanos() == 0 {
        duration
    } else {
        Duration::from_secs(duration.as_secs() + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_precision_omits_empty_hours() {
        let d = Duration::from_secs(83);
        assert_eq!(readout(d, Precision::Seconds, true), "1:23");
    }

    #[test]
    fn hours_are_folded_into_minutes_when_not_allowed() {
        let d = Duration::from_secs(3600 + 30 * 60 + 5);
        assert_eq!(readout(d, Precision::Seconds, true), "1:30:05");
        assert_eq!(readout(d, Precision::Seconds, false), "90:05");
    }

    #[test]
    fn centis_are_truncated_not_rounded() {
        let d = Duration::from_millis(83_999);
        assert_eq!(readout(d, Precision::Centis, true), "1:23.99");
    }

    #[test]
    fn zero_reads_as_zero() {
        assert_eq!(readout(Duration::ZERO, Precision::Seconds, true), "0:00");
        assert_eq!(readout(Duration::ZERO, Precision::Centis, true), "0:00.00");
    }

    #[test]
    fn compact_labels() {
        assert_eq!(compact(Duration::from_secs(30)), "30s");
        assert_eq!(compact(Duration::from_secs(300)), "5m");
        assert_eq!(compact(Duration::from_secs(330)), "5m30");
        assert_eq!(compact(Duration::from_secs(3600)), "1h");
        assert_eq!(compact(Duration::from_secs(5400)), "1h30");
    }

    #[test]
    fn ceil_keeps_whole_seconds_untouched() {
        assert_eq!(ceil_secs(Duration::from_secs(5)), Duration::from_secs(5));
        assert_eq!(
            ceil_secs(Duration::from_millis(4_001)),
            Duration::from_secs(5)
        );
        assert_eq!(ceil_secs(Duration::ZERO), Duration::ZERO);
    }
}
