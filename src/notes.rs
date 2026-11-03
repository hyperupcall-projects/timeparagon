//! The Notes screen: Markdown documents kept in Google Drive's app folder.
//!
//! The documents themselves and how they sync are in `timeslay_core::notes`.
//! This is what the app is in the middle of doing with them. It is unrelated
//! to the notepad, which is the single page behind the "Note" tab.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::Task;
use iced::widget::text_editor;
use timeslay_core::notes::{self, Library, Link, LinkKind, Outcome, Pastel};

use crate::app::{EventMessage, Message, NavigationMessage, Screen, State};
use crate::blocking;
use crate::calendar::Event;

/// How long typing must pause before the note goes to Google Drive. Each save
/// is a whole file, so this is much longer than the local save's debounce.
const UPLOAD_AFTER: Duration = Duration::from_secs(3);
/// How far either side of the day in view to look for things to connect.
const SEARCH_DAYS: i64 = 30;
/// How many matches to offer when connecting a note.
const MAX_CANDIDATES: usize = 8;

/// The two faces of an open note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Edit,
    View,
}

pub struct Store {
    pub library: Library,
    /// Set once the app is really running, so a state built by hand in a test
    /// neither touches the network nor the user's cache.
    live: bool,
    cache_path: Option<PathBuf>,
    /// The note open in the editor.
    pub open: Option<String>,
    pub tab: Tab,
    /// The editor widget's own copy of the open note's text.
    pub editor: text_editor::Content<iced::Renderer>,
    /// Narrows the list of notes.
    pub query: String,
    /// The note whose color swatches are showing in the list.
    pub color_menu: Option<String>,
    /// Finds something to connect the open note to.
    pub connect_query: String,
    /// Deleting the open note is waiting for confirmation.
    pub confirm_delete: bool,
    /// Where the back button goes when a note was opened from elsewhere.
    return_to: Option<Screen>,
    syncing: bool,
    /// Something asked for a sync while one was running.
    sync_again: bool,
    /// The cached copy on disk is behind.
    cache_stale: bool,
    /// The last edit Google Drive has not been asked to take.
    edited_at: Option<Instant>,
    pub error: Option<String>,
}

impl Default for Store {
    fn default() -> Self {
        Self {
            library: Library::default(),
            live: false,
            cache_path: None,
            open: None,
            tab: Tab::default(),
            editor: text_editor::Content::new(),
            query: String::new(),
            color_menu: None,
            connect_query: String::new(),
            confirm_delete: false,
            return_to: None,
            syncing: false,
            sync_again: false,
            cache_stale: false,
            edited_at: None,
            error: None,
        }
    }
}

impl Store {
    /// Loads the cached notes and allows syncing.
    pub fn go_live(&mut self, cache_path: Option<PathBuf>) {
        self.live = true;
        if let Some(path) = &cache_path {
            self.library = Library::load(path);
        }
        self.cache_path = cache_path;
    }

    pub fn syncing(&self) -> bool {
        self.syncing
    }

    /// Whether the debounce timer has anything to do.
    pub fn wants_tick(&self) -> bool {
        self.live && (self.cache_stale || self.edited_at.is_some())
    }

    pub fn open_note(&self) -> Option<&notes::Note> {
        self.library.note(self.open.as_deref()?)
    }

    fn touched(&mut self) {
        self.cache_stale = true;
        self.edited_at = Some(Instant::now());
    }

    fn load_editor(&mut self) {
        let body = self
            .open_note()
            .map(|note| note.body.clone())
            .unwrap_or_default();
        self.editor = text_editor::Content::with_text(&body);
    }

    /// Puts the open note away. An untouched new note is not worth keeping.
    fn close(&mut self) {
        if let Some(id) = self.open.take()
            && self.library.note(&id).is_some_and(notes::Note::is_blank)
        {
            self.library.delete(&id);
            self.cache_stale = true;
        }
        self.connect_query.clear();
        self.confirm_delete = false;
    }
}

#[derive(Debug, Clone)]
pub enum Action {
    /// Sync now.
    Refresh,
    Synced(Result<Outcome, String>),
    Saved(Result<(), String>),
    /// The debounce timer for saving and uploading.
    Tick,
    New,
    /// A new note connected to something, from that thing's details.
    NewFor(Link),
    Open(String),
    /// Leaves the open note.
    Back,
    ShowTab(Tab),
    TitleChanged(String),
    Edited(text_editor::Action),
    QueryChanged(String),
    ColorMenuToggled(String),
    ColorChosen(String, Pastel),
    ConnectQueryChanged(String),
    Connect(String, Link),
    Disconnect(String, Link),
    /// Goes to what a note is connected to.
    Follow(Link),
    /// The second half of following a link, once the day has been selected.
    OpenLoaded(String),
    DeleteRequested,
    DeleteCancelled,
    DeleteConfirmed,
}

/// Syncs with Google Drive in the background, unless that is already happening.
pub fn refresh(state: &mut State) -> Task<Message> {
    let store = &mut state.notes;
    if !store.live {
        return Task::none();
    }
    store.edited_at = None;
    if store.syncing {
        store.sync_again = true;
        return Task::none();
    }
    store.syncing = true;
    let library = store.library.clone();
    let settings = &state.config.calendar;
    let (client_id, client_secret) = (settings.client_id.clone(), settings.client_secret.clone());
    blocking::run(move || Ok(notes::sync(&library, &client_id, &client_secret)))
        .map(|outcome| Message::Notes(Action::Synced(outcome)))
}

/// Called when the app moves to another screen.
pub fn leave(state: &mut State) -> Task<Message> {
    state.notes.return_to = None;
    state.notes.color_menu = None;
    state.notes.close();
    if state.notes.library.has_unsynced() {
        refresh(state)
    } else {
        Task::none()
    }
}

fn save_cache(store: &mut Store) -> Task<Message> {
    store.cache_stale = false;
    let Some(path) = store.cache_path.clone() else {
        return Task::none();
    };
    let library = store.library.clone();
    blocking::run(move || library.save(&path).map_err(|error| error.to_string()))
        .map(|result| Message::Notes(Action::Saved(result)))
}

fn go_to(screen: Screen) -> Task<Message> {
    Task::done(Message::Navigation(NavigationMessage::ScreenSelected(
        screen,
    )))
}

/// Opens a note, going to the Notes screen if it is somewhere else.
fn open(state: &mut State, id: String) -> Task<Message> {
    let Some(note) = state.notes.library.note(&id) else {
        return Task::none();
    };
    state.notes.tab = if note.body.trim().is_empty() {
        Tab::Edit
    } else {
        Tab::View
    };
    state.notes.close();
    state.notes.open = Some(id);
    state.notes.load_editor();
    state.notes.color_menu = None;
    if state.screen == Screen::Notes {
        Task::none()
    } else {
        state.notes.return_to = Some(state.screen);
        go_to(Screen::Notes)
    }
}

fn start_note(state: &mut State, links: Vec<Link>) -> Task<Message> {
    let id = state.notes.library.create("", links);
    state.notes.touched();
    let task = open(state, id);
    Task::batch([task, iced::widget::operation::focus(title_input_id())])
}

/// The id of the title field, so a new note starts there.
pub fn title_input_id() -> iced::widget::Id {
    iced::widget::Id::new("note-title")
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
    match action {
        Action::Refresh => refresh(state),
        Action::Synced(result) => {
            let store = &mut state.notes;
            store.syncing = false;
            let outcome = result.unwrap_or_else(|error| Outcome {
                changes: Vec::new(),
                error: Some(error),
            });
            let failed = outcome.error.is_some();
            store.error = outcome.error;
            store.library.apply(outcome.changes);
            store.cache_stale = true;

            // Another device may have changed or removed the open note.
            if let Some(id) = store.open.clone() {
                match store.library.note(&id) {
                    None => store.close(),
                    Some(note)
                        if !note.dirty
                            && note.body.trim_end() != store.editor.text().trim_end() =>
                    {
                        store.load_editor();
                    }
                    Some(_) => {}
                }
            }
            let again = std::mem::take(&mut store.sync_again);
            if !failed && again {
                return refresh(state);
            }
            if !failed && store.library.has_unsynced() && store.edited_at.is_none() {
                store.edited_at = Some(Instant::now());
            }
            Task::none()
        }
        Action::Saved(result) => {
            if let Err(error) = result {
                log::warn!("could not cache notes: {error}");
            }
            Task::none()
        }
        Action::Tick => {
            let save = if state.notes.cache_stale {
                save_cache(&mut state.notes)
            } else {
                Task::none()
            };
            let due = state
                .notes
                .edited_at
                .is_some_and(|edited| edited.elapsed() >= UPLOAD_AFTER);
            if due {
                Task::batch([save, refresh(state)])
            } else {
                save
            }
        }
        Action::New => start_note(state, Vec::new()),
        Action::NewFor(link) => start_note(state, vec![link]),
        Action::Open(id) => open(state, id),
        Action::Back => {
            state.notes.close();
            let upload = if state.notes.library.has_unsynced() {
                refresh(state)
            } else {
                Task::none()
            };
            match state.notes.return_to.take() {
                Some(screen) => Task::batch([upload, go_to(screen)]),
                None => upload,
            }
        }
        Action::ShowTab(tab) => {
            state.notes.tab = tab;
            Task::none()
        }
        Action::TitleChanged(title) => {
            if let Some(id) = state.notes.open.clone() {
                state.notes.library.edit(&id, |note| note.title = title);
                state.notes.touched();
            }
            Task::none()
        }
        Action::Edited(action) => {
            let Some(id) = state.notes.open.clone() else {
                return Task::none();
            };
            let is_edit = action.is_edit();
            state.notes.editor.perform(action);
            if is_edit {
                let body = state.notes.editor.text();
                state.notes.library.edit(&id, |note| note.body = body);
                state.notes.touched();
            }
            Task::none()
        }
        Action::QueryChanged(query) => {
            state.notes.query = query;
            Task::none()
        }
        Action::ColorMenuToggled(id) => {
            let store = &mut state.notes;
            store.color_menu = (store.color_menu.as_deref() != Some(&id)).then_some(id);
            Task::none()
        }
        Action::ColorChosen(id, color) => {
            if state
                .notes
                .library
                .note(&id)
                .is_some_and(|note| note.color != color)
            {
                state.notes.library.edit(&id, |note| note.color = color);
                state.notes.touched();
            }
            state.notes.color_menu = None;
            Task::none()
        }
        Action::ConnectQueryChanged(query) => {
            state.notes.connect_query = query;
            Task::none()
        }
        Action::Connect(id, link) => {
            state.notes.library.connect(&id, link);
            state.notes.connect_query.clear();
            state.notes.touched();
            Task::none()
        }
        Action::Disconnect(id, link) => {
            state.notes.library.disconnect(&id, &link);
            state.notes.touched();
            Task::none()
        }
        Action::Follow(link) => follow(state, link),
        Action::OpenLoaded(id) => {
            if state.calendar.event(&id).is_some() {
                Task::done(Message::Event(EventMessage::EventOpened(id)))
            } else {
                Task::none()
            }
        }
        Action::DeleteRequested => {
            state.notes.confirm_delete = true;
            Task::none()
        }
        Action::DeleteCancelled => {
            state.notes.confirm_delete = false;
            Task::none()
        }
        Action::DeleteConfirmed => {
            if let Some(id) = state.notes.open.take() {
                state.notes.library.delete(&id);
            }
            state.notes.confirm_delete = false;
            state.notes.cache_stale = true;
            let upload = refresh(state);
            match state.notes.return_to.take() {
                Some(screen) => Task::batch([upload, go_to(screen)]),
                None => upload,
            }
        }
    }
}

fn follow(state: &mut State, link: Link) -> Task<Message> {
    if link.kind == LinkKind::Habit {
        state.habits.form = None;
        state.habits.selected = Some(link.id);
        return go_to(Screen::Habits);
    }
    let mut task = go_to(Screen::Calendar);
    if state.calendar.event(&link.id).is_none()
        && let Some(day) = link.day
    {
        task = task.chain(Task::done(Message::Calendar(
            crate::app::CalendarMessage::CalendarDaySelected(day),
        )));
    }
    task.chain(Task::done(Message::Notes(Action::OpenLoaded(link.id))))
}

/// What a note connected to this calendar item is really connected to. Every
/// occurrence of a habit's events leads to the habit itself. `None` for what
/// cannot have notes: sleep, and a habit event not connected to any habit.
pub fn target_of(state: &State, event: &Event) -> Option<Link> {
    if event.details.sleep.is_some() {
        return None;
    }
    if event.details.habit {
        let id = state.habits.habit_of(event)?;
        return Some(habit_link(id, state.habits.library.habit(id)?.name.clone()));
    }
    let kind = if event.is_task() {
        LinkKind::Task
    } else if crate::timeblocking::is_block(event, &state.config.calendar) {
        LinkKind::TimeBlock
    } else {
        LinkKind::Event
    };
    let day = event.start.date_naive();
    Some(Link {
        kind,
        id: event.id.clone(),
        title: event.title.clone(),
        day: (day != crate::timeblocking::UNDATED).then_some(day),
    })
}

pub fn habit_link(id: &str, name: String) -> Link {
    Link {
        kind: LinkKind::Habit,
        id: id.to_owned(),
        title: name,
        day: None,
    }
}

/// A link's title as it is now, when that can be known. The remembered one is
/// for what is not loaded.
pub fn current_title<'a>(state: &'a State, link: &'a Link) -> &'a str {
    let live = match link.kind {
        LinkKind::Habit => state
            .habits
            .library
            .habit(&link.id)
            .map(|habit| habit.name.as_str()),
        _ => state
            .calendar
            .event(&link.id)
            .map(|event| event.title.as_str()),
    };
    match live.unwrap_or(&link.title) {
        "" => "(No title)",
        title => title,
    }
}

/// What the open note could be connected to, given a search: habits, plus the
/// tasks, events and time blocks loaded around the day in view.
pub fn candidates(state: &State) -> Vec<Link> {
    let query = state.notes.connect_query.trim().to_lowercase();
    let Some(note) = state.notes.open_note().filter(|_| !query.is_empty()) else {
        return Vec::new();
    };
    let mut found: Vec<Link> = state
        .habits
        .library
        .habits
        .iter()
        .filter(|habit| !habit.archived)
        .map(|habit| habit_link(&habit.id, habit.name.clone()))
        .collect();
    let center = state.calendar.day;
    for offset in -SEARCH_DAYS..=SEARCH_DAYS {
        let day = center + chrono::Duration::days(offset);
        for event in state.calendar.events_on(day) {
            // A habit's events are offered as the habit, above.
            if !event.details.habit
                && let Some(link) = target_of(state, event)
            {
                found.push(link);
            }
        }
    }
    let mut seen = Vec::<(LinkKind, String)>::new();
    found.retain(|link| {
        let key = (link.kind, link.id.clone());
        if !link.title.to_lowercase().contains(&query)
            || note.is_linked(link)
            || seen.contains(&key)
        {
            return false;
        }
        seen.push(key);
        true
    });
    found.truncate(MAX_CANDIDATES);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::update as app_update;
    use timeslay_core::habits::Habit;

    fn state() -> State {
        State::blank()
    }

    fn act(state: &mut State, action: Action) {
        let _ = update(state, action);
    }

    fn event(id: &str, title: &str) -> Event {
        let start = crate::clock::now();
        Event {
            id: id.into(),
            calendar_id: "work".into(),
            title: title.into(),
            start,
            end: start + chrono::Duration::hours(1),
            all_day: false,
            color: iced::Color::BLACK,
            completed: false,
            details: Default::default(),
        }
    }

    #[test]
    fn a_new_note_starts_in_the_editor_and_an_empty_one_is_dropped() {
        let mut state = state();
        state.screen = Screen::Notes;
        act(&mut state, Action::New);
        assert_eq!(state.notes.tab, Tab::Edit);
        assert_eq!(state.notes.library.notes.len(), 1);
        act(&mut state, Action::Back);
        assert!(state.notes.open.is_none());
        assert!(state.notes.library.notes.is_empty());
    }

    #[test]
    fn typing_changes_the_note_and_opening_it_again_shows_the_view_tab() {
        let mut state = state();
        state.screen = Screen::Notes;
        act(&mut state, Action::New);
        act(&mut state, Action::TitleChanged("Plans".into()));
        act(
            &mut state,
            Action::Edited(text_editor::Action::Edit(text_editor::Edit::Insert('x'))),
        );
        let id = state.notes.open.clone().unwrap();
        let note = state.notes.library.note(&id).unwrap();
        assert_eq!(note.title, "Plans");
        assert!(note.body.starts_with('x'));
        assert!(note.dirty);

        act(&mut state, Action::Back);
        act(&mut state, Action::Open(id));
        assert_eq!(state.notes.tab, Tab::View);
        assert!(state.notes.editor.text().starts_with('x'));
    }

    #[test]
    fn moving_the_cursor_is_not_an_edit() {
        let mut state = state();
        state.screen = Screen::Notes;
        act(&mut state, Action::New);
        let id = state.notes.open.clone().unwrap();
        let revision = state.notes.library.note(&id).unwrap().revision;
        act(
            &mut state,
            Action::Edited(text_editor::Action::Move(text_editor::Motion::Left)),
        );
        assert_eq!(state.notes.library.note(&id).unwrap().revision, revision);
    }

    #[test]
    fn a_note_opened_from_elsewhere_returns_there() {
        let mut state = state();
        state.screen = Screen::Calendar;
        let link = habit_link("h", "Read".into());
        let task = update(&mut state, Action::NewFor(link.clone()));
        drop(task);
        // The test runtime does not run tasks, so take the screen change by hand.
        let _ = app_update(
            &mut state,
            Message::Navigation(NavigationMessage::ScreenSelected(Screen::Notes)),
        );
        assert_eq!(state.notes.library.linked(&link).len(), 1);
        assert!(state.notes.open.is_some());
        assert_eq!(state.notes.return_to, Some(Screen::Calendar));

        let id = state.notes.open.clone().unwrap();
        let _ = app_update(
            &mut state,
            Message::Navigation(NavigationMessage::ScreenSelected(Screen::Projects)),
        );
        assert!(state.notes.open.is_none());
        assert!(state.notes.return_to.is_none());
        assert!(state.notes.library.note(&id).is_some(), "it has a link");
    }

    #[test]
    fn every_event_of_a_habit_leads_to_the_habit_itself() {
        let mut state = state();
        state.habits.library.habits.push(Habit::new("h1", "Read"));
        let mut morning = event("morning_1", "Morning read");
        morning.details.habit = true;
        morning.details.habit_id = Some("h1".into());
        morning.details.recurring_event_id = Some("morning".into());
        let mut evening = event("evening_1", "Evening read");
        evening.details.habit = true;
        evening.details.habit_id = Some("h1".into());
        evening.details.recurring_event_id = Some("evening".into());

        let one = target_of(&state, &morning).unwrap();
        let two = target_of(&state, &evening).unwrap();
        assert_eq!(one.kind, LinkKind::Habit);
        assert_eq!(one.id, "h1");
        assert!(one.same_target(&two));

        let id = state.notes.library.create("Reading list", vec![one]);
        let shown = state.notes.library.linked(&two);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].id, id);
    }

    #[test]
    fn a_habit_event_without_a_habit_has_no_target() {
        let state = state();
        let mut orphan = event("x", "Orphan");
        orphan.details.habit = true;
        assert_eq!(target_of(&state, &orphan), None);
    }

    #[test]
    fn events_tasks_and_blocks_are_told_apart() {
        let mut state = state();
        state.config.calendar.app_calendar = Some("app".into());
        assert_eq!(
            target_of(&state, &event("e", "Event")).unwrap().kind,
            LinkKind::Event
        );
        let mut block = event("b", "Block");
        block.calendar_id = "app".into();
        assert_eq!(target_of(&state, &block).unwrap().kind, LinkKind::TimeBlock);
        assert_eq!(
            target_of(&state, &event("task:t", "Task")).unwrap().kind,
            LinkKind::Task
        );
    }

    #[test]
    fn connecting_offers_each_thing_once_and_not_what_is_already_connected() {
        let mut state = state();
        state.screen = Screen::Notes;
        state.habits.library.habits.push(Habit::new("h1", "Read"));
        state.calendar.events.push(event("e1", "Read a paper"));
        state.calendar.events.push(event("e2", "Standup"));
        let mut habit_event = event("h1_1", "Read");
        habit_event.details.habit = true;
        habit_event.details.habit_id = Some("h1".into());
        state.calendar.events.push(habit_event);
        act(&mut state, Action::New);
        assert!(candidates(&state).is_empty(), "nothing before a search");

        act(&mut state, Action::ConnectQueryChanged("read".into()));
        let found = candidates(&state);
        let ids: Vec<&str> = found.iter().map(|link| link.id.as_str()).collect();
        assert_eq!(ids, ["h1", "e1"]);

        let id = state.notes.open.clone().unwrap();
        act(&mut state, Action::Connect(id.clone(), found[0].clone()));
        assert!(state.notes.connect_query.is_empty());
        act(&mut state, Action::ConnectQueryChanged("read".into()));
        let ids: Vec<String> = candidates(&state).into_iter().map(|link| link.id).collect();
        assert_eq!(ids, ["e1"]);

        act(&mut state, Action::Disconnect(id.clone(), found[0].clone()));
        assert!(state.notes.library.note(&id).unwrap().links.is_empty());
    }

    #[test]
    fn a_color_can_only_be_a_pastel_and_marks_the_note_for_upload() {
        let mut state = state();
        let id = state.notes.library.create("A", Vec::new());
        state.notes.library.notes[0].dirty = false;
        act(&mut state, Action::ColorMenuToggled(id.clone()));
        assert_eq!(state.notes.color_menu.as_deref(), Some(id.as_str()));
        act(&mut state, Action::ColorChosen(id.clone(), Pastel::Mint));
        let note = state.notes.library.note(&id).unwrap();
        assert_eq!(note.color, Pastel::Mint);
        assert!(note.dirty);
        assert!(state.notes.color_menu.is_none());
    }

    #[test]
    fn deleting_asks_first() {
        let mut state = state();
        state.screen = Screen::Notes;
        act(&mut state, Action::New);
        act(&mut state, Action::TitleChanged("A".into()));
        act(&mut state, Action::DeleteRequested);
        assert!(state.notes.confirm_delete);
        act(&mut state, Action::DeleteCancelled);
        assert_eq!(state.notes.library.notes.len(), 1);
        act(&mut state, Action::DeleteRequested);
        act(&mut state, Action::DeleteConfirmed);
        assert!(state.notes.library.notes.is_empty());
        assert!(state.notes.open.is_none());
    }

    #[test]
    fn a_change_from_another_device_reaches_an_unedited_open_note() {
        let mut state = state();
        state.screen = Screen::Notes;
        let id = state.notes.library.create("A", Vec::new());
        state
            .notes
            .library
            .edit(&id, |note| note.body = "old".into());
        state.notes.library.notes[0].dirty = false;
        act(&mut state, Action::Open(id.clone()));

        let mut newer = state.notes.library.note(&id).unwrap().clone();
        newer.body = "new".into();
        act(
            &mut state,
            Action::Synced(Ok(Outcome {
                changes: vec![notes::Change::Incoming(newer)],
                error: None,
            })),
        );
        assert_eq!(state.notes.editor.text().trim_end(), "new");

        let revision = state.notes.library.note(&id).unwrap().revision;
        act(
            &mut state,
            Action::Synced(Ok(Outcome {
                changes: vec![notes::Change::Gone {
                    id: id.clone(),
                    revision,
                }],
                error: None,
            })),
        );
        assert!(state.notes.open.is_none());
    }

    #[test]
    fn a_failed_sync_is_reported_and_the_notes_stay() {
        let mut state = state();
        state.notes.library.create("A", Vec::new());
        act(
            &mut state,
            Action::Synced(Ok(Outcome {
                changes: Vec::new(),
                error: Some("offline".into()),
            })),
        );
        assert_eq!(state.notes.error.as_deref(), Some("offline"));
        assert_eq!(state.notes.library.notes.len(), 1);
    }
}
