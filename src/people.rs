//! The People screen: the contacts of every signed-in Google account, and the
//! Workspace directories those accounts belong to.
//!
//! What a contact is, how it is searched and how an edit becomes an update is
//! shared with the Dioxus app in [`timeslay_core::people`]. This is the iced
//! screen's state: which accounts are loaded, what is being searched, and the
//! contact open in the side panel.

use std::ops::{Deref, DerefMut};

use iced::Task;
use iced::widget::text_editor;
use serde_json::Value;

use crate::app::{Message, State};
use crate::blocking::{self, WorkerFailed};
use crate::config::CalendarSettings;
use crate::integrations::accounts::{self, Login};

pub use timeslay_core::people::{
    Book, Contact, Edit, Entry, Field, Group, List, MIN_SEARCH, PAGE, Problem, SEARCH_DELAY, api,
    cache, parse_all,
};

impl From<WorkerFailed> for Problem {
    fn from(failed: WorkerFailed) -> Self {
        Problem::other(failed.to_string())
    }
}

/// Where to go once unsaved changes have been dealt with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    Open(Target),
    Close,
}

/// A row of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub account: String,
    pub resource: String,
    pub directory: bool,
}

/// What the side panel is showing: a contact being edited, or a directory entry
/// being read. The fields being edited are its [`Edit`].
pub struct Draft {
    pub account: String,
    login: Login,
    edit: Edit,
    /// The editor for [`Edit::notes`], which it keeps up to date.
    pub notes: text_editor::Content,
    pub saving: bool,
    pub saved: bool,
    pub creating_label: bool,
    pub problem: Option<Problem>,
    /// Asked to go elsewhere with unsaved changes: the panel asks first.
    pub pending: Option<Next>,
}

impl Deref for Draft {
    type Target = Edit;

    fn deref(&self) -> &Edit {
        &self.edit
    }
}

impl DerefMut for Draft {
    fn deref_mut(&mut self) -> &mut Edit {
        &mut self.edit
    }
}

impl Draft {
    fn new(login: &Login, contact: &Contact, read_only: bool) -> Self {
        Self {
            account: login.label.clone(),
            login: login.clone(),
            edit: Edit::new(contact, read_only),
            notes: text_editor::Content::with_text(&contact.notes),
            saving: false,
            saved: false,
            creating_label: false,
            problem: None,
            pending: None,
        }
    }

    /// Replaces the notes, in the editor and in what will be saved.
    #[cfg(test)]
    fn set_notes(&mut self, text: &str) {
        self.notes = text_editor::Content::with_text(text);
        self.edit.notes = self.notes.text();
    }
}

/// Everything the People screen holds.
#[derive(Default)]
pub struct Store {
    /// Only real app startup reaches Google or the disk; test stores stay isolated.
    live: bool,
    cache_path: Option<std::path::PathBuf>,
    pub books: Vec<Book>,
    /// Finding out which accounts are signed in.
    pub listing: bool,
    pub query: String,
    /// Show only this account.
    pub account: Option<String>,
    /// Further pages shown of every section.
    pub more: usize,
    pub draft: Option<Draft>,
    generation: u64,
    search_generation: u64,
    guest_generation: u64,
    /// The guest picker has asked for up-to-date contacts this session.
    guests_refreshed: bool,
}

impl Store {
    /// Starts talking to Google, and picks up the contacts from the last run.
    pub fn go_live(&mut self, cache_path: Option<std::path::PathBuf>) {
        self.live = true;
        if let Some(path) = &cache_path {
            self.books = cache::load(path);
        }
        self.cache_path = cache_path;
    }

    pub fn shown_books(&self) -> impl Iterator<Item = &Book> {
        self.books.iter().filter(|book| {
            self.account
                .as_deref()
                .is_none_or(|account| account == book.account())
        })
    }

    /// `book`'s contacts that match the search.
    pub fn visible<'a>(&self, book: &'a Book) -> Vec<&'a Contact> {
        book.contacts
            .iter()
            .filter(|contact| contact.matches(&self.query))
            .collect()
    }

    /// How many rows each section draws.
    pub fn limit(&self) -> usize {
        PAGE * (self.more + 1)
    }

    pub fn searches_directory(&self) -> bool {
        self.query.trim().chars().count() >= MIN_SEARCH
    }

    pub fn is_open(&self, account: &str, resource: &str) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|d| d.account == account && d.contact.resource == resource)
    }

    /// Whether this contact's change is still on its way to Google.
    pub fn is_saving(&self, account: &str, resource: &str) -> bool {
        self.is_open(account, resource) && self.draft.as_ref().is_some_and(|d| d.saving)
    }

    fn book_mut(&mut self, account: &str) -> Option<&mut Book> {
        self.books.iter_mut().find(|book| book.account() == account)
    }

    fn open(&mut self, target: &Target) {
        let Some(book) = self.books.iter().find(|b| b.account() == target.account) else {
            return;
        };
        let source = if target.directory {
            &book.directory
        } else {
            &book.contacts
        };
        if let Some(contact) = source.iter().find(|c| c.resource == target.resource) {
            self.draft = Some(Draft::new(&book.login, contact, target.directory));
        }
    }
}

/// How many people the guest picker suggests at a time.
const GUEST_SUGGESTIONS: usize = 8;

/// Someone who can be invited to an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitee {
    pub name: String,
    pub email: String,
}

impl Store {
    /// Everyone matching `query` from every account's contacts and directory,
    /// once per address, leaving out `taken`. Contacts come first.
    pub fn invitees(&self, query: &str, taken: &[String]) -> Vec<Invitee> {
        if query.trim().chars().count() < MIN_SEARCH {
            return Vec::new();
        }
        let saved = self.books.iter().flat_map(|book| &book.contacts);
        let directory = self.books.iter().flat_map(|book| &book.directory_seen);
        let mut found: Vec<Invitee> = Vec::new();
        for contact in saved.chain(directory).filter(|c| c.matches(query)) {
            for email in &contact.emails {
                let known = |other: &str| other.eq_ignore_ascii_case(email);
                if !taken.iter().any(|t| known(t)) && !found.iter().any(|f| known(&f.email)) {
                    found.push(Invitee {
                        name: contact.name.clone(),
                        email: email.clone(),
                    });
                }
            }
        }
        found.truncate(GUEST_SUGGESTIONS);
        found
    }
}

/// What the guest picker's search needs from Google: contacts that may never
/// have been loaded, as the People screen is what normally asks for them, and
/// the directory, which is only ever searched.
pub fn search_guests(state: &mut State, query: &str) -> Task<Message> {
    let people = &mut state.people;
    people.guest_generation += 1;
    if !people.live || query.trim().chars().count() < MIN_SEARCH {
        return Task::none();
    }
    let generation = people.guest_generation;
    let query = query.trim().to_owned();
    let refresh = if people.guests_refreshed || people.listing {
        Task::none()
    } else {
        people.guests_refreshed = true;
        refresh(state)
    };
    Task::batch([
        refresh,
        blocking::run(move || std::thread::sleep(SEARCH_DELAY))
            .map(move |()| Message::People(Action::GuestSearchDue(generation, query.clone()))),
    ])
}

#[derive(Debug, Clone)]
pub enum Action {
    GuestSearchDue(u64, String),
    GuestsFound(u64, String, Result<api::Directory, Problem>),
    Refresh,
    Listed(u64, Result<Vec<Login>, String>),
    Loaded(u64, String, Result<(Vec<Value>, Vec<Group>), Problem>),
    QueryChanged(String),
    SearchDue(u64),
    DirectoryFound(u64, String, Result<api::Directory, Problem>),
    AccountChosen(Option<String>),
    ShowMore,
    Open(Target),
    Close,
    /// Throw away the unsaved changes and carry on to wherever was asked for.
    Discard,
    KeepEditing,
    Revert,
    FieldEdited(Field, String),
    EntryEdited(List, usize, String),
    EntryKindChosen(List, usize, String),
    EntryAdded(List),
    EntryRemoved(List, usize),
    LabelToggled(String),
    NewLabelSubmitted,
    LabelCreated(String, Result<Group, Problem>),
    NotesEdited(text_editor::Action),
    Save,
    Saved(String, Result<Value, Problem>),
    /// Save a directory entry to the account's own contacts.
    AddToContacts,
    Added(String, Result<Value, Problem>),
}

/// Writes the books to the cache on a scratch thread.
fn persist(state: &State) -> Task<Message> {
    let Some(path) = state.people.cache_path.clone() else {
        return Task::none();
    };
    let snapshot = match cache::snapshot(&state.people.books) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            log::warn!("could not prepare the contacts cache: {error}");
            return Task::none();
        }
    };
    blocking::run(move || {
        if let Err(error) = snapshot.write(&path) {
            log::warn!("could not save the contacts cache: {error}");
        }
    })
    .map(|()| Message::Ignored)
}

/// Removes the accounts `signed_out` picks, from the screen and from the disk.
fn forget(state: &mut State, signed_out: impl Fn(&Login) -> bool) -> Task<Message> {
    let people = &mut state.people;
    let before = people.books.len();
    people.books.retain(|book| !signed_out(&book.login));
    if people.draft.as_ref().is_some_and(|d| signed_out(&d.login)) {
        people.draft = None;
    }
    if people.books.len() == before {
        return Task::none();
    }
    persist(state)
}

/// The calendar's sign-in has ended.
pub fn forget_primary(state: &mut State) -> Task<Message> {
    forget(state, Login::is_primary)
}

/// A connected account was removed.
pub fn forget_account(state: &mut State, email: &str) -> Task<Message> {
    forget(state, |login| {
        !login.is_primary() && login.label.eq_ignore_ascii_case(email)
    })
}

/// Reloads everything. Results of an earlier refresh still on their way are
/// ignored when they arrive.
pub fn refresh(state: &mut State) -> Task<Message> {
    let people = &mut state.people;
    if !people.live {
        return Task::none();
    }
    people.generation += 1;
    people.listing = true;
    let generation = people.generation;
    let settings = state.config.calendar.clone();
    blocking::run(move || Ok(accounts::identified(&settings)))
        .map(move |logins| Message::People(Action::Listed(generation, logins)))
}

fn load(settings: CalendarSettings, login: &Login) -> Result<(Vec<Value>, Vec<Group>), Problem> {
    let token = accounts::token(&settings, login).map_err(Problem::sign_in)?;
    let groups = api::groups(&token)?;
    Ok((api::contacts(&token)?, groups))
}

/// Asks each account's directory about the current search.
fn search(state: &mut State) -> Task<Message> {
    if !state.people.live || !state.people.searches_directory() {
        return Task::none();
    }
    let generation = state.people.search_generation;
    let query = state.people.query.trim().to_owned();
    let settings = state.config.calendar.clone();
    let mut tasks = Vec::new();
    for book in state.people.books.iter_mut().filter(|b| b.may_search()) {
        book.searching = true;
        let (login, query, settings) = (book.login.clone(), query.clone(), settings.clone());
        let account = login.label.clone();
        tasks.push(
            blocking::run(move || {
                let token = accounts::token(&settings, &login).map_err(Problem::sign_in)?;
                api::search_directory(&token, &query)
            })
            .map(move |found| {
                Message::People(Action::DirectoryFound(generation, account.clone(), found))
            }),
        );
    }
    Task::batch(tasks)
}

/// Runs `work` as the panel's account, turning the result into a message.
fn as_account<T: Send + 'static>(
    state: &State,
    login: Login,
    work: impl FnOnce(&str) -> Result<T, Problem> + Send + 'static,
    done: impl Fn(String, Result<T, Problem>) -> Action + Send + 'static,
) -> Task<Message> {
    let settings = state.config.calendar.clone();
    let account = login.label.clone();
    blocking::run(move || {
        let token = accounts::token(&settings, &login).map_err(Problem::sign_in)?;
        work(&token)
    })
    .map(move |result| Message::People(done(account.clone(), result)))
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
    match action {
        Action::GuestSearchDue(generation, query) => {
            if generation != state.people.guest_generation {
                return Task::none();
            }
            let settings = state.config.calendar.clone();
            let mut tasks = Vec::new();
            for book in state.people.books.iter().filter(|b| b.may_search()) {
                let (login, query, settings) =
                    (book.login.clone(), query.clone(), settings.clone());
                let account = login.label.clone();
                tasks.push(
                    blocking::run(move || {
                        let token = accounts::token(&settings, &login).map_err(Problem::sign_in)?;
                        api::search_directory(&token, &query)
                    })
                    .map(move |found| {
                        Message::People(Action::GuestsFound(generation, account.clone(), found))
                    }),
                );
            }
            Task::batch(tasks)
        }
        // A late answer still teaches us who exists, so it isn't thrown away.
        Action::GuestsFound(_, account, result) => {
            let Some(book) = state.people.book_mut(&account) else {
                return Task::none();
            };
            match result {
                Ok(api::Directory::Found(people)) => book.remember(&parse_all(people, &[])),
                Ok(api::Directory::None) => {
                    book.no_directory = true;
                    book.directory_seen.clear();
                }
                // The picker still has contacts, and takes any typed address.
                Err(problem) => {
                    log::warn!("could not search the directory for guests: {problem:?}");
                    return Task::none();
                }
            }
            persist(state)
        }
        Action::Refresh => refresh(state),
        Action::Listed(generation, logins) => {
            if generation != state.people.generation {
                return Task::none();
            }
            state.people.listing = false;
            // Not knowing who is signed in is no reason to drop anyone.
            let Ok(logins) = logins else {
                return Task::none();
            };
            let mut tasks = Vec::new();
            let mut books = Vec::new();
            for login in logins {
                // Offline, the calendar's account can't say what its address
                // is. It is still the one that was cached under that address.
                let mut book = state
                    .people
                    .books
                    .iter()
                    .position(|book| {
                        book.login == login || (login.is_unidentified() && book.login.is_primary())
                    })
                    .map(|at| state.people.books.remove(at))
                    .unwrap_or_else(|| Book::new(login));
                book.loading = true;
                let login = book.login.clone();
                let settings = state.config.calendar.clone();
                let account = login.label.clone();
                tasks.push(
                    blocking::run(move || load(settings, &login)).map(move |result| {
                        Message::People(Action::Loaded(generation, account.clone(), result))
                    }),
                );
                books.push(book);
            }
            // Whatever is left over is no longer signed in.
            if !state.people.books.is_empty() {
                state.people.books = books;
                tasks.push(persist(state));
            } else {
                state.people.books = books;
            }
            if let Some(account) = &state.people.account
                && !state.people.books.iter().any(|b| b.account() == account)
            {
                state.people.account = None;
            }
            tasks.push(search(state));
            Task::batch(tasks)
        }
        Action::Loaded(generation, account, result) => {
            if generation != state.people.generation {
                return Task::none();
            }
            let Some(book) = state.people.book_mut(&account) else {
                return Task::none();
            };
            book.loading = false;
            // A failure leaves the cached contacts on show, with the reason.
            let fetched = result.is_ok();
            match result {
                Ok((people, groups)) => book.loaded(people, groups),
                Err(problem) => book.problem = Some(problem),
            }
            // An open contact with nothing to lose follows what was loaded, so
            // the next save is against the latest version.
            let fresh = state.people.draft.as_ref().and_then(|draft| {
                let same = draft.account == account && !draft.read_only && !draft.dirty();
                same.then(|| {
                    let book = state.people.books.iter().find(|b| b.account() == account)?;
                    let contact = book
                        .contacts
                        .iter()
                        .find(|c| c.resource == draft.contact.resource)?;
                    (contact.person() != draft.contact.person())
                        .then(|| Draft::new(&book.login, contact, false))
                })?
            });
            if fresh.is_some() {
                state.people.draft = fresh;
            }
            if fetched {
                persist(state)
            } else {
                Task::none()
            }
        }
        Action::QueryChanged(query) => {
            let people = &mut state.people;
            people.query = query;
            people.more = 0;
            people.search_generation += 1;
            if !people.searches_directory() {
                for book in &mut people.books {
                    book.directory.clear();
                    book.searching = false;
                    book.directory_problem = None;
                }
                return Task::none();
            }
            let query = people.query.trim().to_owned();
            for book in people.books.iter_mut().filter(|book| book.may_search()) {
                book.show_seen(&query);
            }
            if !people.live {
                return Task::none();
            }
            let generation = people.search_generation;
            blocking::run(move || std::thread::sleep(SEARCH_DELAY))
                .map(move |()| Message::People(Action::SearchDue(generation)))
        }
        Action::SearchDue(generation) => {
            if generation == state.people.search_generation {
                search(state)
            } else {
                Task::none()
            }
        }
        Action::DirectoryFound(generation, account, result) => {
            if generation != state.people.search_generation {
                return Task::none();
            }
            let Some(book) = state.people.book_mut(&account) else {
                return Task::none();
            };
            book.searching = false;
            book.directory_problem = None;
            match result {
                Ok(api::Directory::Found(people)) => book.show_found(parse_all(people, &[])),
                Ok(api::Directory::None) => {
                    book.no_directory = true;
                    book.directory.clear();
                    book.directory_seen.clear();
                }
                Err(problem) => {
                    book.directory_problem = Some(problem);
                    return Task::none();
                }
            }
            persist(state)
        }
        Action::AccountChosen(account) => {
            state.people.account = account;
            state.people.more = 0;
            Task::none()
        }
        Action::ShowMore => {
            state.people.more += 1;
            Task::none()
        }
        Action::Open(target) => {
            go(state, Next::Open(target));
            Task::none()
        }
        Action::Close => {
            go(state, Next::Close);
            Task::none()
        }
        Action::Discard => {
            if let Some(next) = state.people.draft.as_mut().and_then(|d| d.pending.take()) {
                follow(state, next);
            }
            Task::none()
        }
        Action::KeepEditing => {
            if let Some(draft) = &mut state.people.draft {
                draft.pending = None;
            }
            Task::none()
        }
        Action::Revert => {
            if let Some(draft) = &state.people.draft {
                let reverted = Draft::new(&draft.login, &draft.contact, draft.read_only);
                state.people.draft = Some(reverted);
            }
            Task::none()
        }
        Action::Save => save(state),
        Action::Saved(account, result) => {
            let Some(draft) = &mut state.people.draft else {
                return Task::none();
            };
            draft.saving = false;
            let same_account = draft.account == account;
            match result {
                Ok(person) => {
                    let book = state.people.book_mut(&account);
                    let was_loading = book.as_ref().is_some_and(|book| book.loading);
                    let contact = book.and_then(|book| book.store(person));
                    if let (Some(contact), Some(draft), true) =
                        (contact, &state.people.draft, same_account)
                    {
                        let mut next = Draft::new(&draft.login, &contact, false);
                        next.saved = true;
                        state.people.draft = Some(next);
                    }
                    let saved = persist(state);
                    // A reload begun before this save would bring the old
                    // version back, so start it over.
                    if was_loading {
                        return Task::batch([saved, refresh(state)]);
                    }
                    saved
                }
                Err(problem) => {
                    draft.problem = Some(problem);
                    Task::none()
                }
            }
        }
        Action::AddToContacts => {
            let Some(draft) = &mut state.people.draft else {
                return Task::none();
            };
            if draft.saving {
                return Task::none();
            }
            draft.saving = true;
            draft.problem = None;
            let body = draft.as_new_contact();
            let login = draft.login.clone();
            as_account(
                state,
                login,
                move |token| api::create(token, &body),
                Action::Added,
            )
        }
        Action::Added(account, result) => {
            let Some(draft) = &mut state.people.draft else {
                return Task::none();
            };
            draft.saving = false;
            match result {
                Ok(person) => {
                    let Some(book) = state.people.book_mut(&account) else {
                        return Task::none();
                    };
                    if let Some(contact) = book.store(person) {
                        let mut next = Draft::new(&book.login, &contact, false);
                        next.saved = true;
                        state.people.draft = Some(next);
                    }
                    persist(state)
                }
                Err(problem) => {
                    draft.problem = Some(problem);
                    Task::none()
                }
            }
        }
        Action::NewLabelSubmitted => new_label(state),
        Action::LabelCreated(account, result) => {
            let group = match result {
                Ok(group) => Some(group),
                Err(problem) => {
                    if let Some(draft) = &mut state.people.draft {
                        draft.creating_label = false;
                        draft.problem = Some(problem);
                    }
                    None
                }
            };
            if let Some(group) = group {
                if let Some(book) = state.people.book_mut(&account) {
                    book.add_group(group.clone());
                }
                if let Some(draft) = &mut state.people.draft
                    && draft.account == account
                {
                    draft.creating_label = false;
                    draft.new_label.clear();
                    draft.labels.push(group.resource);
                }
                return persist(state);
            }
            Task::none()
        }
        edit => {
            if let Some(draft) = &mut state.people.draft {
                apply_edit(draft, edit);
            }
            Task::none()
        }
    }
}

/// Goes straight to `next`, unless that would lose unsaved changes.
fn go(state: &mut State, next: Next) {
    if let Some(draft) = &mut state.people.draft {
        let same = matches!(&next, Next::Open(target)
            if target.directory == draft.read_only
                && target.account == draft.account
                && target.resource == draft.contact.resource);
        if draft.saving || same {
            return;
        }
        if draft.dirty() {
            draft.pending = Some(next);
            return;
        }
    }
    follow(state, next);
}

fn follow(state: &mut State, next: Next) {
    match next {
        Next::Open(target) => state.people.open(&target),
        Next::Close => state.people.draft = None,
    }
}

fn save(state: &mut State) -> Task<Message> {
    let Some(draft) = &mut state.people.draft else {
        return Task::none();
    };
    if draft.saving || !draft.dirty() {
        return Task::none();
    }
    draft.saving = true;
    draft.saved = false;
    draft.problem = None;
    draft.pending = None;
    let (fields, body) = draft.patch();
    let resource = draft.contact.resource.clone();
    let login = draft.login.clone();
    as_account(
        state,
        login,
        move |token| api::update(token, &resource, &fields, &body),
        Action::Saved,
    )
}

/// Selects the label named in the box, creating it if the account has none by
/// that name. Created straight away, as Google Contacts does, so that a failed
/// save has no half-made label to account for.
fn new_label(state: &mut State) -> Task<Message> {
    let Some(draft) = &state.people.draft else {
        return Task::none();
    };
    let name = draft.new_label.trim().to_owned();
    if name.is_empty() || draft.creating_label {
        return Task::none();
    }
    let existing = state
        .people
        .books
        .iter()
        .find(|book| book.account() == draft.account)
        .and_then(|book| book.group_named(&name))
        .map(|group| group.resource.clone());
    let login = draft.login.clone();
    let draft = state.people.draft.as_mut().expect("checked above");
    if let Some(resource) = existing {
        draft.choose_label(resource);
        return Task::none();
    }
    draft.creating_label = true;
    draft.problem = None;
    as_account(
        state,
        login,
        move |token| api::create_group(token, &name),
        Action::LabelCreated,
    )
}

fn apply_edit(draft: &mut Draft, edit: Action) {
    if draft.read_only {
        return;
    }
    draft.saved = false;
    draft.pending = None;
    match edit {
        Action::FieldEdited(field, value) => draft.set_field(field, value),
        Action::EntryEdited(list, index, value) => draft.set_entry(list, index, value),
        Action::EntryKindChosen(list, index, kind) => draft.set_kind(list, index, kind),
        Action::EntryAdded(list) => draft.add_entry(list),
        Action::EntryRemoved(list, index) => draft.remove_entry(list, index),
        Action::LabelToggled(resource) => draft.toggle_label(resource),
        Action::NotesEdited(action) => {
            let edits = action.is_edit();
            draft.notes.perform(action);
            if edits {
                draft.edit.notes = draft.notes.text();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
