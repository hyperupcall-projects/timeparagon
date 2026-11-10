//! Desktop notifications.
//!
//! `notify_rust::Notification::show` makes a blocking D-Bus call, so this is
//! only ever called from a background task, never from the update loop.

use notify_rust::{Notification, Timeout, Urgency};

use crate::config;

/// The icon name notification daemons look up in the icon theme.
const ICON: &str = "timeslay";

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

/// A reminder about something coming up.
#[derive(Debug, Clone)]
pub struct Reminder {
    pub summary: String,
    pub body: String,
    /// A sound from the desktop's sound theme for the daemon to play.
    pub sound_name: Option<&'static str>,
    /// Asks the daemon not to play anything, because the app plays the
    /// sound itself or the reminder is meant to be silent.
    pub silent: bool,
}

/// Shows a reminder notification.
pub fn show_reminder(reminder: &Reminder) -> Result<(), String> {
    use notify_rust::Hint;

    let mut notification = Notification::new();
    notification
        .appname("TimeSlay")
        .summary(&reminder.summary)
        .body(&reminder.body)
        .icon(ICON)
        .urgency(Urgency::Normal)
        .timeout(Timeout::Default);
    if reminder.silent {
        notification.hint(Hint::SuppressSound(true));
    } else if let Some(name) = reminder.sound_name {
        notification.hint(Hint::SoundName(name.to_owned()));
    }
    notification
        .show()
        .map(|_handle| ())
        .map_err(|error| error.to_string())
}

/// Shows the running-sleep notification, or replaces the one with `replaces`.
/// `ongoing` keeps it up until it is replaced; the summary expires normally.
/// Returns the notification's id so it can be replaced later.
pub fn show_sleep(
    summary: &str,
    body: &str,
    ongoing: bool,
    replaces: Option<u32>,
) -> Result<u32, String> {
    use notify_rust::Hint;

    let mut notification = Notification::new();
    notification
        .appname("TimeSlay")
        .summary(summary)
        .body(body)
        .icon(ICON)
        .urgency(Urgency::Low)
        .hint(Hint::SuppressSound(true))
        .timeout(if ongoing {
            Timeout::Never
        } else {
            Timeout::Default
        });
    if ongoing {
        notification.hint(Hint::Resident(true));
    }
    if let Some(id) = replaces {
        notification.id(id);
    }
    notification
        .show()
        .map(|handle| handle.id())
        .map_err(|error| error.to_string())
}
