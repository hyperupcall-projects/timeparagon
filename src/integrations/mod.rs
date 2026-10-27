//! Services a task can come from.
//!
//! Paste a link into the new-task box and the matching [`Integration`] reads
//! the page behind it through that service's API, so the task arrives with a
//! real title instead of a bare URL. The link itself is kept in the task's
//! notes, which is what lets the task show the service's icon later, on any
//! device, without a separate store.
//!
//! To add a service, implement [`Integration`] in its own module and list it
//! in [`REGISTRY`]. Nothing else needs to know about it.
pub mod accounts;
pub mod gmail;

use chrono::NaiveDate;

use crate::calendar::deadline::Deadline;
use crate::config::CalendarSettings;

/// Which service a task came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Gmail,
}

impl Kind {
    /// The name shown to people, as in "Open in Gmail".
    pub fn name(self) -> &'static str {
        integration(self).name()
    }
}

/// A task's origin: the service, and the link that opens it there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub kind: Kind,
    pub url: String,
}

/// What an integration learned about a link. Everything except the title is
/// optional, and fills the matching form field only when it was left empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Details {
    pub title: String,
    pub deadline: Option<Deadline>,
    pub due: Option<NaiveDate>,
}

/// What an integration may use to reach its service.
#[derive(Debug, Clone)]
pub struct Auth {
    pub settings: CalendarSettings,
}

impl Auth {
    /// Every Google account a link might belong to: the calendar's, if it is
    /// signed in, and each one connected for Gmail in Settings.
    pub fn google_accounts(&self) -> Vec<accounts::Login> {
        accounts::logins()
    }

    /// A fresh access token for one of them, refreshed if the stored one has
    /// expired.
    pub fn google_token(&self, account: &accounts::Login) -> Result<String, String> {
        accounts::token(&self.settings, account)
    }
}

pub trait Integration: Sync {
    fn kind(&self) -> Kind;
    fn name(&self) -> &'static str;
    /// Whether `url` belongs to this service. Offline and cheap: it runs as
    /// the user types.
    fn recognizes(&self, url: &str) -> bool;
    /// Reads the page behind `url` from the service's API. Blocking: call it
    /// from [`crate::blocking::run`].
    fn fetch(&self, url: &str, auth: &Auth) -> Result<Details, String>;
}

static REGISTRY: &[&dyn Integration] = &[&gmail::Gmail];

pub fn integration(kind: Kind) -> &'static dyn Integration {
    REGISTRY
        .iter()
        .copied()
        .find(|integration| integration.kind() == kind)
        .expect("every kind is registered")
}

/// Whether `text` is nothing but one web link.
pub fn is_url(text: &str) -> bool {
    let text = text.trim();
    let Some(rest) = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
    else {
        return false;
    };
    !rest.is_empty() && !text.contains(char::is_whitespace)
}

/// The source of `text`, when the whole text is a link some integration owns.
pub fn recognize(text: &str) -> Option<Source> {
    let url = text.trim();
    if !is_url(url) {
        return None;
    }
    REGISTRY
        .iter()
        .find(|integration| integration.recognizes(url))
        .map(|integration| Source {
            kind: integration.kind(),
            url: url.to_owned(),
        })
}

/// The first link in `notes` that an integration owns.
pub fn source_in(notes: &str) -> Option<Source> {
    notes
        .split_whitespace()
        .find_map(|word| recognize(word.trim_matches(|c| matches!(c, '<' | '>' | '(' | ')' | ','))))
}

pub fn fetch(source: &Source, auth: &Auth) -> Result<Details, String> {
    integration(source.kind).fetch(&source.url, auth)
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD: &str = "https://mail.google.com/mail/u/0/#inbox/FMfcgzQgLjNPlfJCVRfnNkPGkLhWClCW";

    #[test]
    fn only_a_lone_link_counts_as_pasted() {
        assert!(is_url(" https://example.org/a?b=c#d "));
        assert!(!is_url("https://"));
        assert!(!is_url("see https://example.org"));
        assert!(!is_url("example.org"));
        assert!(!is_url("mailto:me@example.org"));
    }

    #[test]
    fn a_gmail_link_is_recognized_and_other_text_is_not() {
        let source = recognize(&format!("  {THREAD}\n")).unwrap();
        assert_eq!(source.kind, Kind::Gmail);
        assert_eq!(source.url, THREAD);
        assert_eq!(Kind::Gmail.name(), "Gmail");
        assert_eq!(recognize("https://example.org/mail"), None);
        assert_eq!(recognize("Buy milk"), None);
        assert_eq!(recognize(&format!("Reply {THREAD}")), None);
    }

    #[test]
    fn notes_keep_their_source_among_other_lines() {
        let notes = format!("DUE: 2026-11-02\nRead this first\n<{THREAD}>\nthanks");
        assert_eq!(source_in(&notes).unwrap().url, THREAD);
        assert_eq!(source_in("DUE: 2026-11-02\nNothing here"), None);
    }
}
