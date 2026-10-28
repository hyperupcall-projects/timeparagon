//! One free-text brain dump per day, kept in the config file.
//!
//! The entries live here rather than in the calendar because they are private
//! scratch notes: they never travel to Google and a day has at most one page.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::app::{Message, State};

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

/// What the Brain dump screen and its editor are in the middle of.
pub struct Workspace {
    /// The day whose page is open in the editor.
    pub open: Option<NaiveDate>,
    /// The editor widget's own copy, so the cursor survives a redraw.
    pub editor: iced::widget::text_editor::Content<iced::Renderer>,
    /// The substring that narrows the list of pages.
    pub query: String,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            open: None,
            editor: iced::widget::text_editor::Content::with_text(""),
            query: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Action {
    /// Open the page for a day, creating it if it is new.
    Open(NaiveDate),
    Close,
    Edited(iced::widget::text_editor::Action),
    Query(String),
    Delete(NaiveDate),
}

pub fn update(state: &mut State, action: Action) -> iced::Task<Message> {
    match action {
        Action::Open(date) => {
            let text = state
                .config
                .brain_dump
                .get(date)
                .unwrap_or_default()
                .to_owned();
            state.brain_dump.open = Some(date);
            state.brain_dump.editor = iced::widget::text_editor::Content::with_text(&text);
            iced::widget::operation::focus(iced::widget::Id::new("brain-dump-editor"))
        }
        Action::Close => {
            state.brain_dump.open = None;
            iced::Task::none()
        }
        Action::Edited(action) => {
            let Some(date) = state.brain_dump.open else {
                return iced::Task::none();
            };
            state.brain_dump.editor.perform(action);
            let text = state.brain_dump.editor.text();
            state.config.brain_dump.set(date, text);
            crate::app::queue_save(state)
        }
        Action::Query(query) => {
            state.brain_dump.query = query;
            iced::Task::none()
        }
        Action::Delete(date) => {
            state.config.brain_dump.remove(date);
            if state.brain_dump.open == Some(date) {
                state.brain_dump.open = None;
            }
            crate::app::queue_save(state)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_day_has_its_own_page() {
        let mut dumps = Dumps::default();
        let first = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        let second = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        dumps.set(first, "yesterday".into());
        dumps.set(second, "today".into());
        dumps.set(first, "yesterday, edited".into());
        assert_eq!(dumps.get(first), Some("yesterday, edited"));
        assert_eq!(dumps.get(second), Some("today"));
        assert_eq!(dumps.entries.len(), 2);
    }

    #[test]
    fn search_is_a_case_insensitive_substring_over_the_text() {
        let mut dumps = Dumps::default();
        dumps.set(
            NaiveDate::from_ymd_opt(2026, 10, 27).unwrap(),
            "Book the Dentist".into(),
        );
        dumps.set(
            NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(),
            "Buy milk".into(),
        );
        let found: Vec<_> = dumps
            .search("dent")
            .into_iter()
            .map(|entry| entry.date)
            .collect();
        assert_eq!(found, vec![NaiveDate::from_ymd_opt(2026, 10, 27).unwrap()]);
        assert_eq!(dumps.search("").len(), 2);
        // Newest first, so yesterday's page appears above the day before.
        assert_eq!(dumps.search("")[0].text, "Buy milk");
    }

    #[test]
    fn opening_a_page_loads_it_and_typing_saves_it() {
        let mut state = crate::app::State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        );
        let day = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        state.config.brain_dump.set(day, "start".into());
        let _ = update(&mut state, Action::Open(day));
        assert_eq!(state.brain_dump.editor.text(), "start");

        let _ = update(
            &mut state,
            Action::Edited(iced::widget::text_editor::Action::Edit(
                iced::widget::text_editor::Edit::Insert('!'),
            )),
        );
        let text = state.config.brain_dump.get(day).unwrap();
        assert!(text.contains('!'), "got {text:?}");
        assert!(text.contains("start"), "got {text:?}");
    }
}
