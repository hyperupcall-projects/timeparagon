//! Desktop notifications.
//!
//! `notify_rust::Notification::show` makes a blocking D-Bus call, so this is
//! only ever called from a background task, never from the update loop.

use std::time::Duration;

use notify_rust::{Notification, Timeout, Urgency};

use crate::config;
use crate::engine::format;

/// The icon name notification daemons look up in the icon theme.
const ICON: &str = "alarm-symbolic";

/// Shows the alarm notification described by `settings`.
pub fn show(settings: &config::Notification, duration: Duration) -> Result<(), String> {
    Notification::new()
        .appname("TimeTracker")
        .summary(&settings.summary)
        .body(&body(settings, duration))
        .icon(ICON)
        .urgency(urgency(settings.urgency))
        .timeout(timeout(settings))
        .show()
        .map(|_handle| ())
        .map_err(|error| error.to_string())
}

/// Falls back to describing the timer when no body text is configured, since
/// a notification with only "Time's up" does not say which timer.
fn body(settings: &config::Notification, duration: Duration) -> String {
    if settings.body.trim().is_empty() {
        format!("Your {} timer has finished.", format::compact(duration))
    } else {
        settings.body.clone()
    }
}

fn urgency(urgency: config::Urgency) -> Urgency {
    match urgency {
        config::Urgency::Low => Urgency::Low,
        config::Urgency::Normal => Urgency::Normal,
        config::Urgency::Critical => Urgency::Critical,
    }
}

/// A `timeout_secs` of 0 hands the decision to the notification daemon, which
/// is what most desktops expect: critical notifications stay up until they are
/// dismissed, everything else expires on the daemon's own schedule.
fn timeout(settings: &config::Notification) -> Timeout {
    if settings.timeout_secs == 0 {
        Timeout::Default
    } else {
        Timeout::Milliseconds(settings.timeout_secs.saturating_mul(1_000))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_body_describes_the_timer_that_fired() {
        let settings = config::Notification {
            body: String::new(),
            ..config::Notification::default()
        };

        assert_eq!(
            body(&settings, Duration::from_secs(300)),
            "Your 5m timer has finished."
        );
    }

    #[test]
    fn a_configured_body_is_used_verbatim() {
        let settings = config::Notification {
            body: "Tea is ready".to_owned(),
            ..config::Notification::default()
        };

        assert_eq!(body(&settings, Duration::from_secs(300)), "Tea is ready");
    }

    #[test]
    fn whitespace_counts_as_empty() {
        let settings = config::Notification {
            body: "   ".to_owned(),
            ..config::Notification::default()
        };

        assert!(body(&settings, Duration::from_secs(60)).starts_with("Your 1m"));
    }

    #[test]
    fn zero_timeout_defers_to_the_daemon() {
        let settings = config::Notification {
            timeout_secs: 0,
            ..config::Notification::default()
        };

        assert!(matches!(timeout(&settings), Timeout::Default));
    }

    #[test]
    fn a_configured_timeout_is_converted_to_milliseconds() {
        let settings = config::Notification {
            timeout_secs: 5,
            ..config::Notification::default()
        };

        assert!(matches!(timeout(&settings), Timeout::Milliseconds(5_000)));
    }
}
