//! Desktop notifications.
//!
//! `notify_rust::Notification::show` makes a blocking D-Bus call, so this is
//! only ever called from a background task, never from the update loop.

use notify_rust::{Notification, Timeout, Urgency};

use crate::config;

/// The icon name notification daemons look up in the icon theme.
const ICON: &str = "time-slay";

/// Shows the alarm notification described by `settings`.
pub fn show(settings: &config::Notification) -> Result<(), String> {
    Notification::new()
        .appname("TimeSlay")
        .summary(&settings.summary)
        .body(&settings.body)
        .icon(ICON)
        .urgency(urgency(settings.urgency))
        .timeout(timeout(settings))
        .show()
        .map(|_handle| ())
        .map_err(|error| error.to_string())
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
