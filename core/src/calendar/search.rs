//! What the calendar's Search popup narrows the views down to.
//!
//! The filter is only ever applied where events are drawn. The store keeps every
//! event, so the dock, travel routes and task bookkeeping are left alone.

use chrono::{NaiveDate, NaiveTime};
use iced_core::Color;

use super::event_options::{Availability, Visibility};
use super::{Event, Store, projects, task_metadata};
use crate::config::CalendarSettings;
use crate::timeblocking;

/// All an event says about itself while its details are hidden.
pub const PRIVATE_TITLE: &str = "Private";
/// A neutral fill for the same, since an event's color gives away its calendar
/// or project.
pub const PRIVATE_COLOR: Color = Color::from_rgb(0.45, 0.45, 0.48);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AvailabilityFilter {
    #[default]
    Any,
    Busy,
    Free,
}

impl AvailabilityFilter {
    pub const ALL: [Self; 3] = [Self::Any, Self::Busy, Self::Free];
}

impl std::fmt::Display for AvailabilityFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Any => "Busy or free",
            Self::Busy => "Busy",
            Self::Free => "Free",
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Search {
    /// Whether the popup is showing. The filter stays on after it closes.
    pub open: bool,
    pub query: String,
    /// Look in descriptions as well as titles.
    pub in_description: bool,
    pub time: Option<NaiveTime>,
    pub projects: projects::Selection,
    pub labels: Vec<String>,
    pub availability: AvailabilityFilter,
    pub public_only: bool,
    pub location: Option<String>,
    /// Draw only explicitly public events in full, and blank out the rest.
    pub details_by_visibility: bool,
}

#[derive(Debug, Clone)]
pub enum Action {
    Toggle,
    Query(String),
    InDescription(bool),
    /// A time as `HH:MM`, the form the shared time control reports.
    Time(String),
    ClearTime,
    Projects(projects::Action),
    Label(String, bool),
    Availability(AvailabilityFilter),
    PublicOnly(bool),
    Location(Option<String>),
    DetailsByVisibility(bool),
    Clear,
}

impl Search {
    pub fn update(&mut self, action: Action) {
        match action {
            Action::Toggle => self.open = !self.open,
            Action::Query(query) => self.query = query,
            Action::InDescription(value) => self.in_description = value,
            Action::Time(value) => {
                if let Ok(time) = NaiveTime::parse_from_str(&value, "%H:%M") {
                    self.time = Some(time);
                }
            }
            Action::ClearTime => self.time = None,
            Action::Projects(action) => self.projects.update(action),
            Action::Label(label, true) => {
                if !self.labels.contains(&label) {
                    self.labels.push(label);
                }
            }
            Action::Label(label, false) => self.labels.retain(|value| *value != label),
            Action::Availability(value) => self.availability = value,
            Action::PublicOnly(value) => self.public_only = value,
            Action::Location(value) => self.location = value,
            Action::DetailsByVisibility(value) => self.details_by_visibility = value,
            Action::Clear => {
                *self = Self {
                    open: self.open,
                    ..Self::default()
                }
            }
        }
    }

    /// Whether anything is being filtered out.
    pub fn is_active(&self) -> bool {
        !self.query.trim().is_empty()
            || self.time.is_some()
            || !self.projects.ids.is_empty()
            || !self.labels.is_empty()
            || self.availability != AvailabilityFilter::Any
            || self.public_only
            || self.location.is_some()
    }

    /// Whether anything in the popup differs from how it started, including
    /// the detail filter, which changes what is drawn but not which events.
    pub fn is_modified(&self) -> bool {
        self.is_active() || self.details_by_visibility
    }

    /// Whether this event is drawn blank. Only an event explicitly marked public
    /// is drawn in full: tasks have no visibility, and an event on default
    /// visibility may well be private, so both are treated as private.
    pub fn hides_details(&self, event: &Event) -> bool {
        self.details_by_visibility && event.details.options.visibility != Visibility::Public
    }

    /// The part of the filter about what an event is: its text, projects and
    /// labels. A task in a time block can satisfy this on the block's behalf.
    fn matches_content(&self, event: &Event) -> bool {
        self.matches_text(event) && self.matches_tags(event)
    }

    fn matches_text(&self, event: &Event) -> bool {
        let query = self.query.trim().to_lowercase();
        event.title.to_lowercase().contains(&query)
            || (self.in_description
                && task_metadata::prose(&event.details.description)
                    .to_lowercase()
                    .contains(&query))
    }

    fn matches_tags(&self, event: &Event) -> bool {
        if self.projects.ids.is_empty() && self.labels.is_empty() {
            return true;
        }
        // Tasks keep these in their metadata, and calendar events in their details.
        let metadata = event.is_task().then(|| timeblocking::metadata(event));
        let (projects, labels) = match &metadata {
            Some(metadata) => (&metadata.projects, &metadata.labels),
            None => (&event.details.projects, &event.details.labels),
        };
        (self.projects.ids.is_empty() || self.projects.ids.iter().any(|id| projects.contains(id)))
            && (self.labels.is_empty() || self.labels.iter().any(|label| labels.contains(label)))
    }

    /// The part of the filter about the calendar entry itself: when it is, and
    /// the fields only a Google Calendar event has.
    fn matches_entry(&self, event: &Event) -> bool {
        // Tasks and sleep records have no availability of their own, though
        // their defaulted options would read as Busy.
        let has_availability = !event.is_task() && event.details.sleep.is_none();
        self.time.is_none_or(|time| covers_time(event, time))
            && match self.availability {
                AvailabilityFilter::Any => true,
                AvailabilityFilter::Busy => {
                    has_availability && event.details.options.transparency == Availability::Busy
                }
                AvailabilityFilter::Free => {
                    has_availability && event.details.options.transparency == Availability::Free
                }
            }
            && (!self.public_only || event.details.options.visibility == Visibility::Public)
            && self
                .location
                .as_deref()
                .is_none_or(|location| event.details.location.trim() == location)
    }

    pub fn matches(&self, event: &Event) -> bool {
        self.matches_entry(event) && self.matches_content(event)
    }

    /// Whether a view should draw this event, given the rest of its day.
    ///
    /// A time block is also kept when one of its tasks matches, because those
    /// tasks are drawn inside it and would otherwise have nowhere to appear.
    pub fn shows(&self, event: &Event, day_events: &[Event], settings: &CalendarSettings) -> bool {
        if !self.is_active() {
            return true;
        }
        if !self.matches_entry(event) {
            return false;
        }
        if self.matches_content(event) {
            return true;
        }
        if !timeblocking::is_block(event, settings) {
            return false;
        }
        let hour = settings.day_start_hour();
        day_events
            .iter()
            .any(|task| timeblocking::in_block(task, event, hour) && self.matches_content(task))
    }
}

/// Whether an event is under way at this time of day, counting the minute it
/// starts and the minute it ends. An all-day event covers the whole day.
fn covers_time(event: &Event, time: NaiveTime) -> bool {
    if event.all_day {
        return true;
    }
    let (start, end) = (event.start.naive_local(), event.end.naive_local());
    match (end.date() - start.date()).num_days() {
        ..0 => false,
        0 => start.time() <= time && time <= end.time(),
        // Past midnight, the event holds both ends of the clock.
        1 => start.time() <= time || time <= end.time(),
        _ => true,
    }
}

impl Store {
    fn loaded_events(&self) -> impl Iterator<Item = &Event> {
        self.events.iter().chain(self.cache.values().flatten())
    }

    /// Whether a view should draw this event, given the search in effect.
    pub fn search_shows(&self, event: &Event, day: NaiveDate, settings: &CalendarSettings) -> bool {
        self.search.shows(event, self.events_on(day), settings)
    }

    /// Whether a task inside `block` should be listed there: all of them when
    /// the block itself matches, otherwise only the ones that do.
    pub fn search_shows_in_block(&self, task: &Event, block: &Event) -> bool {
        let search = &self.search;
        !search.is_active() || search.matches_content(block) || search.matches_content(task)
    }

    /// Every label on the events loaded so far, plus any already chosen, so a
    /// choice stays on the list after moving to days without it. Events with
    /// hidden details add nothing, or the list would give their labels away.
    pub fn search_labels(&self) -> Vec<String> {
        let mut labels = self.search.labels.clone();
        for event in self.loaded_events() {
            if self.search.hides_details(event) {
                continue;
            }
            if event.is_task() {
                labels.extend(timeblocking::metadata(event).labels);
            } else {
                labels.extend(event.details.labels.iter().cloned());
            }
        }
        labels.sort();
        labels.dedup();
        labels
    }

    /// Every location on the events loaded so far, plus the one already chosen,
    /// leaving out events with hidden details for the same reason as the labels.
    pub fn search_locations(&self) -> Vec<String> {
        let mut locations: Vec<String> = self
            .loaded_events()
            .filter(|event| !self.search.hides_details(event))
            .map(|event| event.details.location.trim())
            .filter(|location| !location.is_empty())
            .map(str::to_owned)
            .chain(self.search.location.clone())
            .collect();
        locations.sort();
        locations.dedup();
        locations
    }
}

#[cfg(test)]
mod tests {
    use super::super::event_options::{Availability, Visibility};
    use super::*;
    use chrono::{DateTime, Local, TimeZone};

    fn at(day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 10, day, hour, minute, 0)
            .unwrap()
    }

    fn event(title: &str, start: DateTime<Local>, end: DateTime<Local>) -> Event {
        Event {
            id: title.to_owned(),
            calendar_id: "work".into(),
            title: title.to_owned(),
            start,
            end,
            all_day: false,
            color: super::super::colors::DEFAULT_EVENT,
            completed: false,
            details: Default::default(),
        }
    }

    fn morning(title: &str) -> Event {
        event(title, at(1, 9, 0), at(1, 10, 0))
    }

    fn task(title: &str, block: Option<&str>) -> Event {
        let mut task = morning(title);
        task.id = crate::calendar::ids::event_id(title);
        task.calendar_id = "tasks:list".into();
        task.details.task_data = Some(crate::task_db::TaskData {
            id: 100,
            metadata: timeblocking::Metadata {
                block: block.map(str::to_owned),
                ..Default::default()
            },
        });
        task
    }

    fn clock(hour: u32, minute: u32) -> Option<NaiveTime> {
        NaiveTime::from_hms_opt(hour, minute, 0)
    }

    #[test]
    fn an_untouched_search_is_inactive_and_clearing_keeps_the_popup_open() {
        let mut search = Search::default();
        assert!(!search.is_active());
        search.update(Action::Query("   ".into()));
        search.update(Action::InDescription(true));
        assert!(!search.is_active(), "neither narrows anything by itself");

        search.update(Action::Toggle);
        search.update(Action::Query("standup".into()));
        search.update(Action::PublicOnly(true));
        assert!(search.is_active());
        search.update(Action::Clear);
        assert_eq!(
            search,
            Search {
                open: true,
                ..Search::default()
            }
        );
    }

    #[test]
    fn the_query_is_a_case_insensitive_substring_of_the_title_by_default() {
        let mut planning = morning("Sprint Planning");
        planning.details.description = "quarterly budget".into();
        let mut search = Search::default();

        search.update(Action::Query("PLAN".into()));
        assert!(search.matches(&planning));

        search.update(Action::Query("budget".into()));
        assert!(!search.matches(&planning));
        search.update(Action::InDescription(true));
        assert!(search.matches(&planning));
    }

    #[test]
    fn a_description_search_skips_the_metadata_stored_in_task_notes() {
        let mut task = task("Laundry", None);
        task.details.description = "Whites only\n---\ntt-version: 1\ntt-minutes: 30".into();
        let mut search = Search {
            in_description: true,
            ..Search::default()
        };
        search.update(Action::Query("whites".into()));
        assert!(search.matches(&task));
        search.update(Action::Query("tt-minutes".into()));
        assert!(!search.matches(&task));
    }

    #[test]
    fn a_time_of_day_includes_both_ends_of_an_event() {
        let meeting = morning("Meeting");
        let matches = |time| {
            Search {
                time,
                ..Search::default()
            }
            .matches(&meeting)
        };
        assert!(!matches(clock(8, 59)));
        assert!(matches(clock(9, 0)));
        assert!(matches(clock(9, 30)));
        assert!(matches(clock(10, 0)));
        assert!(!matches(clock(10, 1)));
    }

    #[test]
    fn a_time_of_day_follows_an_event_across_midnight() {
        let late = event("Late", at(1, 23, 0), at(2, 1, 0));
        let long = event("Long", at(1, 23, 0), at(3, 1, 0));
        let mut all_day = morning("Holiday");
        all_day.all_day = true;
        let matches = |event: &Event, time| {
            Search {
                time,
                ..Search::default()
            }
            .matches(event)
        };
        assert!(matches(&late, clock(23, 30)));
        assert!(matches(&late, clock(0, 30)));
        assert!(!matches(&late, clock(12, 0)));
        assert!(matches(&long, clock(12, 0)));
        assert!(matches(&all_day, clock(12, 0)));
    }

    #[test]
    fn projects_and_labels_match_any_of_those_chosen() {
        let mut filed = morning("Filed");
        filed.details.projects = vec![1];
        filed.details.labels = vec!["red".into()];
        let bare = morning("Bare");
        let mut task = task("Task", None);
        if let Some(data) = &mut task.details.task_data {
            data.metadata.projects = vec![2];
            data.metadata.labels = vec!["blue".into()];
        }

        let mut search = Search::default();
        search.update(Action::Projects(projects::Action::Add(1)));
        search.update(Action::Projects(projects::Action::Add(2)));
        assert!(search.matches(&filed));
        assert!(search.matches(&task), "tasks keep theirs in metadata");
        assert!(!search.matches(&bare));

        search.update(Action::Label("red".into(), true));
        search.update(Action::Label("green".into(), true));
        assert!(search.matches(&filed));
        assert!(!search.matches(&task), "both kinds of filter must hold");

        search.update(Action::Label("red".into(), false));
        assert!(!search.matches(&filed));
    }

    #[test]
    fn availability_only_ever_matches_real_calendar_events() {
        let busy = morning("Busy");
        let mut free = morning("Free");
        free.details.options.transparency = Availability::Free;
        let task = task("Task", None);
        let mut sleep = morning("Sleep");
        sleep.details.sleep = Some(super::super::sleep::Record::planned(
            at(1, 9, 0).date_naive(),
        ));

        let filter = |availability| Search {
            availability,
            ..Search::default()
        };
        let any = filter(AvailabilityFilter::Any);
        assert!(any.matches(&busy) && any.matches(&free) && any.matches(&task));
        let only_busy = filter(AvailabilityFilter::Busy);
        assert!(only_busy.matches(&busy) && !only_busy.matches(&free));
        assert!(!only_busy.matches(&task) && !only_busy.matches(&sleep));
        let only_free = filter(AvailabilityFilter::Free);
        assert!(only_free.matches(&free) && !only_free.matches(&busy));
        assert!(!only_free.matches(&task));
    }

    #[test]
    fn public_only_and_location_narrow_to_the_exact_field() {
        let mut public = morning("Public");
        public.details.options.visibility = Visibility::Public;
        public.details.location = " Room 1 ".into();
        let default = morning("Default");

        let mut search = Search {
            public_only: true,
            ..Search::default()
        };
        assert!(search.matches(&public));
        assert!(!search.matches(&default));

        search.public_only = false;
        search.update(Action::Location(Some("Room 1".into())));
        assert!(search.matches(&public));
        assert!(!search.matches(&default));
        search.update(Action::Location(Some("Room".into())));
        assert!(!search.matches(&public));
    }

    #[test]
    fn only_explicitly_public_events_keep_their_details() {
        let mut search = Search::default();
        let mut public = morning("Public");
        public.details.options.visibility = Visibility::Public;
        let mut private = morning("Private");
        private.details.options.visibility = Visibility::Private;
        let mut confidential = morning("Confidential");
        confidential.details.options.visibility = Visibility::Confidential;
        let default = morning("Default");
        let task = task("Task", None);

        for event in [&public, &private, &confidential, &default, &task] {
            assert!(!search.hides_details(event), "off by default");
        }
        assert!(!search.is_modified());

        search.update(Action::DetailsByVisibility(true));
        assert!(!search.hides_details(&public));
        for event in [&private, &confidential, &default, &task] {
            assert!(search.hides_details(event), "{}", event.title);
        }
        assert!(search.is_modified());
        assert!(!search.is_active(), "every event stays on screen");

        search.update(Action::Clear);
        assert!(!search.details_by_visibility);
    }

    #[test]
    fn hidden_events_do_not_add_to_the_label_and_location_lists() {
        let mut snapshot = super::super::sample_snapshot();
        let mut public = morning("Public");
        public.details.options.visibility = Visibility::Public;
        public.details.labels = vec!["open".into()];
        public.details.location = "Lobby".into();
        let mut private = morning("Private");
        private.details.labels = vec!["secret".into()];
        private.details.location = "Clinic".into();
        snapshot.events = vec![public, private];
        let mut store = Store::with_snapshot(snapshot);
        assert_eq!(store.search_labels(), ["open", "secret"]);

        store.search.update(Action::DetailsByVisibility(true));
        assert_eq!(store.search_labels(), ["open"]);
        assert_eq!(store.search_locations(), ["Lobby"]);
    }

    fn block_store() -> (Store, CalendarSettings, Event, Event, Event, NaiveDate) {
        let settings = CalendarSettings {
            app_calendar: Some("work".into()),
            ..Default::default()
        };
        let block = morning("Deep work");
        let key = timeblocking::block_key(&block);
        let inside = task("Write report", Some(&key));
        let other = task("Read mail", Some(&key));
        let mut snapshot = super::super::sample_snapshot();
        let day = snapshot.day;
        snapshot.events = vec![block.clone(), inside.clone(), other.clone()];
        (
            Store::with_snapshot(snapshot),
            settings,
            block,
            inside,
            other,
            day,
        )
    }

    #[test]
    fn a_time_block_stays_for_a_matching_task_and_lists_only_that_task() {
        let (mut store, settings, block, inside, other, day) = block_store();
        store.search.update(Action::Query("report".into()));
        assert!(store.search_shows(&block, day, &settings));
        assert!(store.search_shows_in_block(&inside, &block));
        assert!(!store.search_shows_in_block(&other, &block));

        store.search.update(Action::Query("deep".into()));
        assert!(store.search_shows(&block, day, &settings));
        assert!(store.search_shows_in_block(&inside, &block), "all of them");
        assert!(store.search_shows_in_block(&other, &block));

        store.search.update(Action::Query("nothing".into()));
        assert!(!store.search_shows(&block, day, &settings));
    }

    #[test]
    fn a_task_cannot_keep_a_time_block_that_is_outside_the_time_of_day() {
        let (mut store, settings, block, _, _, day) = block_store();
        store.search.update(Action::Query("report".into()));
        store.search.time = clock(15, 0);
        assert!(!store.search_shows(&block, day, &settings));
    }

    #[test]
    fn the_choices_come_from_loaded_events_and_keep_what_is_chosen() {
        let mut snapshot = super::super::sample_snapshot();
        let mut first = morning("First");
        first.details.labels = vec!["b".into(), "a".into()];
        first.details.location = "Room 1".into();
        let mut second = morning("Second");
        second.details.labels = vec!["a".into()];
        second.details.location = "  ".into();
        let mut task = task("Task", None);
        if let Some(data) = &mut task.details.task_data {
            data.metadata.labels = vec!["c".into()];
        }
        snapshot.events = vec![first, second, task];
        let mut store = Store::with_snapshot(snapshot);

        assert_eq!(store.search_labels(), ["a", "b", "c"]);
        assert_eq!(store.search_locations(), ["Room 1"]);

        store.search.update(Action::Label("gone".into(), true));
        store.search.update(Action::Location(Some("Away".into())));
        assert_eq!(store.search_labels(), ["a", "b", "c", "gone"]);
        assert_eq!(store.search_locations(), ["Away", "Room 1"]);
    }
}
