//! One free-text brain dump per day, kept in the config file.
//!
//! The entries live here rather than in the calendar because they are private
//! scratch notes: they never travel to Google and a day has at most one page.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// The page written for one day.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub date: NaiveDate,
    pub text: String,
}

/// Every page, in no particular order. The screen sorts them by date.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Dumps {
    #[serde(default)]
    pub entries: Vec<Entry>,
}

impl Dumps {
    /// Whether there is nothing to write to the config file.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The page for a day, unless that day has never been started.
    pub fn get(&self, date: NaiveDate) -> Option<&str> {
        self.entries
            .iter()
            .find(|entry| entry.date == date)
            .map(|entry| entry.text.as_str())
    }

    /// Creates or replaces the page for a day.
    pub fn set(&mut self, date: NaiveDate, text: String) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.date == date) {
            entry.text = text;
        } else {
            self.entries.push(Entry { date, text });
        }
    }

    /// Removes the page for a day, if it exists.
    pub fn remove(&mut self, date: NaiveDate) {
        self.entries.retain(|entry| entry.date != date);
    }

    /// The pages newest first, which is how the screen lists them.
    pub fn newest_first(&self) -> Vec<&Entry> {
        let mut entries: Vec<_> = self.entries.iter().collect();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.date));
        entries
    }

    /// The pages whose text contains the query, ignoring case. An empty query
    /// matches every page, and the date is searchable too.
    pub fn search(&self, query: &str) -> Vec<&Entry> {
        let query = query.trim().to_lowercase();
        self.newest_first()
            .into_iter()
            .filter(|entry| {
                query.is_empty()
                    || entry.text.to_lowercase().contains(&query)
                    || entry
                        .date
                        .format("%A, %B %-d, %Y")
                        .to_string()
                        .to_lowercase()
                        .contains(&query)
            })
            .collect()
    }
}
