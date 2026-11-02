//! The People screen: the contacts of every signed-in Google account, and the
//! Workspace directories those accounts belong to.
//!
//! Each account is its own [`Book`], so a person saved in two accounts stays two
//! contacts and it is always clear which account a change goes to. Contacts are
//! read in full and searched here. A directory can run to thousands of people,
//! so it is only ever asked about what was typed.
//!
//! A contact keeps the JSON Google sent. An edit changes the fields it touched
//! within that JSON and sends only those, so whatever this screen has no control
//! for (a phonetic name, a structured address) comes through unharmed.

pub mod api;

use iced::Task;
use iced::widget::text_editor;
use serde_json::{Value, json};

use crate::app::{Message, State};
use crate::blocking::{self, WorkerFailed};
use crate::config::CalendarSettings;
use crate::integrations::accounts::{self, Login};

pub use api::{Group, Problem};

/// How many of a section's rows are drawn at first: laying out thousands of
/// them would be felt on every redraw, and search is the way through that many.
pub const PAGE: usize = 200;
/// Waits for a pause in typing before asking Google.
const SEARCH_DELAY: std::time::Duration = std::time::Duration::from_millis(350);
/// A single letter would match a good part of any directory.
const MIN_SEARCH: usize = 2;

const EMAIL_KINDS: &[&str] = &["home", "work", "other"];
const PHONE_KINDS: &[&str] = &["mobile", "work", "home", "other"];
const ADDRESS_KINDS: &[&str] = &["home", "work", "other"];
const URL_KINDS: &[&str] = &["homePage", "work", "blog", "profile", "other"];

/// The parts of a stored address that say the same thing as its one-line form.
/// Editing that line has to drop them or Google would keep the old address.
const ADDRESS_PARTS: &[&str] = &[
    "poBox",
    "streetAddress",
    "extendedAddress",
    "city",
    "region",
    "postalCode",
    "country",
    "countryCode",
];

impl From<WorkerFailed> for Problem {
    fn from(failed: WorkerFailed) -> Self {
        Problem {
            message: failed.to_string(),
            reconnect: false,
        }
    }
}

/// A multi-valued field of a contact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    Emails,
    Phones,
    Addresses,
    Urls,
}

impl List {
    const ALL: [List; 4] = [List::Emails, List::Phones, List::Addresses, List::Urls];

    /// The field's name in the People API.
    fn field(self) -> &'static str {
        match self {
            List::Emails => "emailAddresses",
            List::Phones => "phoneNumbers",
            List::Addresses => "addresses",
            List::Urls => "urls",
        }
    }

    /// Which key of an entry holds the text that is edited.
    fn value_key(self) -> &'static str {
        match self {
            List::Addresses => "formattedValue",
            _ => "value",
        }
    }

    pub fn kinds(self) -> &'static [&'static str] {
        match self {
            List::Emails => EMAIL_KINDS,
            List::Phones => PHONE_KINDS,
            List::Addresses => ADDRESS_KINDS,
            List::Urls => URL_KINDS,
        }
    }
}

/// A contact, or a person in a directory.
#[derive(Debug, Clone, PartialEq)]
pub struct Contact {
    pub resource: String,
    /// As Google sent it.
    person: Value,
    pub name: String,
    pub emails: Vec<String>,
    pub phones: Vec<String>,
    pub company: String,
    pub title: String,
    pub notes: String,
    pub labels: Vec<String>,
    /// Everything searchable, in lower case.
    search: String,
}

impl Contact {
    pub fn parse(person: Value, groups: &[Group]) -> Option<Self> {
        let resource = person["resourceName"].as_str()?.to_owned();
        let emails = values(&person, List::Emails);
        let phones = values(&person, List::Phones);
        let company = text_at(&person["organizations"][0]["name"]);
        let title = text_at(&person["organizations"][0]["title"]);
        let notes = text_at(&person["biographies"][0]["value"]);
        let labels: Vec<String> = member_groups(&person)
            .iter()
            .filter_map(|resource| groups.iter().find(|group| group.resource == *resource))
            .map(|group| group.name.clone())
            .collect();
        let name = [
            text_at(&person["names"][0]["displayName"]),
            emails.first().cloned().unwrap_or_default(),
            phones.first().cloned().unwrap_or_default(),
            company.clone(),
        ]
        .into_iter()
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| "Unnamed contact".to_owned());
        let search = [
            name.as_str(),
            &emails.join(" "),
            &phones.join(" "),
            &company,
            &title,
            &notes,
            &labels.join(" "),
        ]
        .join("\n")
        .to_lowercase();
        Some(Self {
            resource,
            person,
            name,
            emails,
            phones,
            company,
            title,
            notes,
            labels,
            search,
        })
    }

    /// Whether every word of `query` appears somewhere in the contact.
    pub fn matches(&self, query: &str) -> bool {
        query
            .to_lowercase()
            .split_whitespace()
            .all(|word| self.search.contains(word))
    }

    /// The line of the notes that explains why the contact matched, since the
    /// notes are not otherwise on show in the list.
    pub fn note_hint(&self, query: &str) -> Option<&str> {
        let query = query.to_lowercase();
        let words: Vec<&str> = query.split_whitespace().collect();
        let name = self.name.to_lowercase();
        if words.is_empty() || words.iter().all(|word| name.contains(word)) {
            return None;
        }
        self.notes.lines().find(|line| {
            let line = line.to_lowercase();
            words.iter().any(|word| line.contains(word))
        })
    }

    /// The one line under the name: how to reach them, and where they work.
    pub fn summary(&self) -> String {
        let reach = self.emails.first().or(self.phones.first());
        [reach.map(String::as_str), Some(self.company.as_str())]
            .into_iter()
            .flatten()
            .filter(|part| !part.is_empty() && *part != self.name)
            .collect::<Vec<_>>()
            .join(" · ")
    }

    fn sort_key(&self) -> String {
        self.name.to_lowercase()
    }
}

fn text_at(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

fn values(person: &Value, list: List) -> Vec<String> {
    person[list.field()]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| text_at(&entry[list.value_key()]))
        .filter(|value| !value.is_empty())
        .collect()
}

fn member_groups(person: &Value) -> Vec<String> {
    person["memberships"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|member| member["contactGroupMembership"]["contactGroupResourceName"].as_str())
        .map(str::to_owned)
        .collect()
}

fn parse_all(people: Vec<Value>, groups: &[Group]) -> Vec<Contact> {
    people
        .into_iter()
        .filter_map(|person| Contact::parse(person, groups))
        .collect()
}

/// One signed-in account and what it holds.
#[derive(Debug, Clone)]
pub struct Book {
    pub login: Login,
    pub contacts: Vec<Contact>,
    pub groups: Vec<Group>,
    pub loading: bool,
    pub problem: Option<Problem>,
    /// The latest directory results, in Google's order.
    pub directory: Vec<Contact>,
    pub searching: bool,
    pub directory_problem: Option<Problem>,
    /// Google said this account is not part of a Workspace domain.
    no_directory: bool,
}

impl Book {
    fn new(login: Login) -> Self {
        Self {
            login,
            contacts: Vec::new(),
            groups: Vec::new(),
            loading: true,
            problem: None,
            directory: Vec::new(),
            searching: false,
            directory_problem: None,
            no_directory: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_tests(login: Login) -> Self {
        Self {
            loading: false,
            ..Self::new(login)
        }
    }

    pub fn account(&self) -> &str {
        &self.login.label
    }

    /// The Workspace domain to search, as far as can be told without asking.
    pub fn directory_domain(&self) -> Option<&str> {
        if self.no_directory || !api::may_have_directory(self.account()) {
            return None;
        }
        self.account().rsplit_once('@').map(|(_, domain)| domain)
    }

    fn may_search(&self) -> bool {
        !self.no_directory && api::may_have_directory(self.account())
    }

    /// The contact already saved under one of `emails`, so a directory entry
    /// leads to it instead of to a duplicate.
    pub fn saved_with_email(&self, emails: &[String]) -> Option<&Contact> {
        self.contacts.iter().find(|contact| {
            contact
                .emails
                .iter()
                .any(|mine| emails.iter().any(|email| email.eq_ignore_ascii_case(mine)))
        })
    }
}

/// One line of a multi-valued field in the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub value: String,
    pub kind: String,
    /// Its position in the JSON Google sent. `None` for a new line.
    source: Option<usize>,
}

fn entries_of(person: &Value, list: List) -> Vec<Entry> {
    person[list.field()]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, entry)| Entry {
            value: text_at(&entry[list.value_key()]),
            kind: text_at(&entry["type"]),
            source: Some(index),
        })
        .collect()
}

/// Lines worth keeping, tidied, so a blank line or stray space is not a change.
fn normalized(entries: &[Entry]) -> Vec<Entry> {
    entries
        .iter()
        .map(|entry| Entry {
            value: entry.value.trim().to_owned(),
            kind: entry.kind.clone(),
            source: entry.source,
        })
        .filter(|entry| !entry.value.is_empty())
        .collect()
}

/// A copy of an entry with what only Google may write taken out.
fn clean(entry: &Value) -> Value {
    let mut entry = entry.clone();
    if let Some(fields) = entry.as_object_mut() {
        for key in ["metadata", "formattedType", "canonicalForm"] {
            fields.remove(key);
        }
    }
    entry
}

/// What a text box on the panel edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    GivenName,
    FamilyName,
    Company,
    Title,
    NewLabel,
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
/// being read.
pub struct Draft {
    pub account: String,
    login: Login,
    pub contact: Contact,
    /// A directory entry: its domain owns it, so it cannot be changed from here.
    pub read_only: bool,
    pub given: String,
    pub family: String,
    pub company: String,
    pub title: String,
    pub emails: Vec<Entry>,
    pub phones: Vec<Entry>,
    pub addresses: Vec<Entry>,
    pub urls: Vec<Entry>,
    /// The resource names of the account's labels this contact has.
    pub labels: Vec<String>,
    pub new_label: String,
    pub notes: text_editor::Content,
    pub saving: bool,
    pub saved: bool,
    pub creating_label: bool,
    pub problem: Option<Problem>,
    /// Asked to go elsewhere with unsaved changes: the panel asks first.
    pub pending: Option<Next>,
}

impl Draft {
    fn new(login: &Login, contact: &Contact, read_only: bool) -> Self {
        let person = &contact.person;
        Self {
            account: login.label.clone(),
            login: login.clone(),
            contact: contact.clone(),
            read_only,
            given: text_at(&person["names"][0]["givenName"]),
            family: text_at(&person["names"][0]["familyName"]),
            company: contact.company.clone(),
            title: contact.title.clone(),
            emails: entries_of(person, List::Emails),
            phones: entries_of(person, List::Phones),
            addresses: entries_of(person, List::Addresses),
            urls: entries_of(person, List::Urls),
            labels: member_groups(person),
            new_label: String::new(),
            notes: text_editor::Content::with_text(&contact.notes),
            saving: false,
            saved: false,
            creating_label: false,
            problem: None,
            pending: None,
        }
    }

    pub fn entries(&self, list: List) -> &[Entry] {
        match list {
            List::Emails => &self.emails,
            List::Phones => &self.phones,
            List::Addresses => &self.addresses,
            List::Urls => &self.urls,
        }
    }

    fn entries_mut(&mut self, list: List) -> &mut Vec<Entry> {
        match list {
            List::Emails => &mut self.emails,
            List::Phones => &mut self.phones,
            List::Addresses => &mut self.addresses,
            List::Urls => &mut self.urls,
        }
    }

    pub fn dirty(&self) -> bool {
        !self.read_only && !self.changed_fields().is_empty()
    }

    /// The People API fields that differ from what Google sent.
    fn changed_fields(&self) -> Vec<&'static str> {
        let person = &self.contact.person;
        let mut fields = Vec::new();
        if self.given.trim() != text_at(&person["names"][0]["givenName"]).trim()
            || self.family.trim() != text_at(&person["names"][0]["familyName"]).trim()
        {
            fields.push("names");
        }
        if self.company.trim() != self.contact.company.trim()
            || self.title.trim() != self.contact.title.trim()
        {
            fields.push("organizations");
        }
        for list in List::ALL {
            if normalized(self.entries(list)) != normalized(&entries_of(person, list)) {
                fields.push(list.field());
            }
        }
        if self.notes.text().trim_end() != self.contact.notes.trim_end() {
            fields.push("biographies");
        }
        let (mut mine, mut theirs) = (self.labels.clone(), member_groups(person));
        mine.sort();
        theirs.sort();
        if mine != theirs {
            fields.push("memberships");
        }
        fields
    }

    /// The update to send: the changed fields, and the sources Google checks to
    /// be sure nobody else has changed the contact since it was read.
    fn patch(&self) -> (Vec<&'static str>, Value) {
        let person = &self.contact.person;
        let fields = self.changed_fields();
        let sources: Vec<Value> = person["metadata"]["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|source| source["type"].as_str() == Some("CONTACT"))
            .cloned()
            .collect();
        let mut body = json!({
            "resourceName": self.contact.resource,
            "etag": person["etag"],
            "metadata": { "sources": sources },
        });
        for field in &fields {
            body[*field] = match *field {
                "names" => self.names_value(),
                "organizations" => self.organizations_value(),
                "biographies" => self.notes_value(),
                "memberships" => self.memberships_value(),
                _ => {
                    let list = List::ALL
                        .into_iter()
                        .find(|list| list.field() == *field)
                        .expect("every other field is a list");
                    self.list_value(list)
                }
            };
        }
        (fields, body)
    }

    fn names_value(&self) -> Value {
        let mut name = clean(&self.contact.person["names"][0]);
        if !name.is_object() {
            name = json!({});
        }
        // Google composes these from the parts, and would rather believe them.
        for key in ["displayName", "displayNameLastFirst", "unstructuredName"] {
            name.as_object_mut().map(|name| name.remove(key));
        }
        name["givenName"] = json!(self.given.trim());
        name["familyName"] = json!(self.family.trim());
        if self.given.trim().is_empty() && self.family.trim().is_empty() {
            return json!([]);
        }
        json!([name])
    }

    /// The panel edits the first organization; any others are passed along.
    fn organizations_value(&self) -> Value {
        let all = self.contact.person["organizations"].as_array();
        let mut first = all
            .and_then(|all| all.first())
            .map(clean)
            .unwrap_or_else(|| json!({}));
        first["name"] = json!(self.company.trim());
        first["title"] = json!(self.title.trim());
        let keep_first = !self.company.trim().is_empty() || !self.title.trim().is_empty();
        let rest = all.into_iter().flatten().skip(1).map(clean);
        Value::Array(
            keep_first
                .then_some(first)
                .into_iter()
                .chain(rest)
                .collect(),
        )
    }

    fn list_value(&self, list: List) -> Value {
        let originals = self.contact.person[list.field()].as_array();
        let entries = normalized(self.entries(list))
            .into_iter()
            .map(|entry| {
                let original = entry
                    .source
                    .and_then(|index| originals.and_then(|all| all.get(index)));
                let mut value = original.map(clean).unwrap_or_else(|| json!({}));
                let same_text =
                    original.is_some_and(|o| text_at(&o[list.value_key()]).trim() == entry.value);
                if list == List::Addresses && !same_text {
                    for part in ADDRESS_PARTS {
                        value.as_object_mut().map(|value| value.remove(*part));
                    }
                }
                value[list.value_key()] = json!(entry.value);
                match (entry.kind.is_empty(), value.as_object_mut()) {
                    (false, Some(fields)) => {
                        fields.insert("type".to_owned(), json!(entry.kind));
                    }
                    (true, Some(fields)) => {
                        fields.remove("type");
                    }
                    _ => {}
                }
                value
            })
            .collect();
        Value::Array(entries)
    }

    fn notes_value(&self) -> Value {
        let text = self.notes.text();
        let text = text.trim_end();
        if text.is_empty() {
            return json!([]);
        }
        let mut notes = clean(&self.contact.person["biographies"][0]);
        if !notes.is_object() {
            notes = json!({ "contentType": "TEXT_PLAIN" });
        }
        notes["value"] = json!(text);
        json!([notes])
    }

    /// Memberships the panel does not show, such as "My Contacts", stay. Google
    /// refuses an update that would leave a contact in no group at all.
    fn memberships_value(&self) -> Value {
        let memberships = self.contact.person["memberships"].as_array();
        let mine = member_groups(&self.contact.person);
        let keep = memberships.into_iter().flatten().filter(|member| {
            member["contactGroupMembership"]["contactGroupResourceName"]
                .as_str()
                .is_none_or(|resource| self.labels.iter().any(|label| label == resource))
        });
        let added = self
            .labels
            .iter()
            .filter(|label| !mine.contains(label))
            .map(|label| membership(label));
        let mut all: Vec<Value> = keep.map(clean).chain(added).collect();
        if all.is_empty() {
            all.push(membership("contactGroups/myContacts"));
        }
        Value::Array(all)
    }

    /// A new contact holding this directory entry's details.
    fn as_new_contact(&self) -> Value {
        let person = &self.contact.person;
        let mut body = json!({});
        if let Some(name) = person["names"].as_array().and_then(|names| names.first()) {
            let mut name = clean(name);
            for key in ["displayName", "displayNameLastFirst", "unstructuredName"] {
                name.as_object_mut().map(|name| name.remove(key));
            }
            body["names"] = json!([name]);
        }
        for field in [
            "emailAddresses",
            "phoneNumbers",
            "organizations",
            "addresses",
            "urls",
        ] {
            if let Some(entries) = person[field].as_array() {
                body[field] = Value::Array(entries.iter().map(clean).collect());
            }
        }
        body
    }
}

fn membership(group: &str) -> Value {
    json!({ "contactGroupMembership": { "contactGroupResourceName": group } })
}

/// Everything the People screen holds.
#[derive(Default)]
pub struct Store {
    /// Only real app startup reaches Google; test stores stay isolated.
    live: bool,
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
}

impl Store {
    pub fn go_live(&mut self) {
        self.live = true;
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

#[derive(Debug, Clone)]
pub enum Action {
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
        Action::Refresh => refresh(state),
        Action::Listed(generation, logins) => {
            if generation != state.people.generation {
                return Task::none();
            }
            state.people.listing = false;
            let logins = logins.unwrap_or_default();
            let mut tasks = Vec::new();
            let mut books = Vec::new();
            for login in logins {
                let mut book = state
                    .people
                    .books
                    .iter()
                    .position(|book| book.login == login)
                    .map(|at| state.people.books.remove(at))
                    .unwrap_or_else(|| Book::new(login.clone()));
                book.loading = true;
                let settings = state.config.calendar.clone();
                let account = login.label.clone();
                tasks.push(
                    blocking::run(move || load(settings, &login)).map(move |result| {
                        Message::People(Action::Loaded(generation, account.clone(), result))
                    }),
                );
                books.push(book);
            }
            state.people.books = books;
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
            match result {
                Ok((people, groups)) => {
                    book.contacts = parse_all(people, &groups);
                    book.contacts.sort_by_key(Contact::sort_key);
                    book.groups = groups;
                    book.problem = None;
                }
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
                    (contact.person != draft.contact.person)
                        .then(|| Draft::new(&book.login, contact, false))
                })?
            });
            if fresh.is_some() {
                state.people.draft = fresh;
            }
            Task::none()
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
                Ok(api::Directory::Found(people)) => book.directory = parse_all(people, &[]),
                Ok(api::Directory::None) => {
                    book.no_directory = true;
                    book.directory.clear();
                }
                Err(problem) => book.directory_problem = Some(problem),
            }
            Task::none()
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
                    let contact = book.and_then(|book| {
                        let contact = Contact::parse(person, &book.groups)?;
                        match book
                            .contacts
                            .iter_mut()
                            .find(|c| c.resource == contact.resource)
                        {
                            Some(slot) => *slot = contact.clone(),
                            None => book.contacts.push(contact.clone()),
                        }
                        book.contacts.sort_by_key(Contact::sort_key);
                        Some(contact)
                    });
                    if let (Some(contact), Some(draft), true) =
                        (contact, &state.people.draft, same_account)
                    {
                        let mut next = Draft::new(&draft.login, &contact, false);
                        next.saved = true;
                        state.people.draft = Some(next);
                    }
                    // A reload begun before this save would bring the old
                    // version back, so start it over.
                    if was_loading {
                        return refresh(state);
                    }
                }
                Err(problem) => draft.problem = Some(problem),
            }
            Task::none()
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
                    if let Some(contact) = Contact::parse(person, &book.groups) {
                        book.contacts.push(contact.clone());
                        book.contacts.sort_by_key(Contact::sort_key);
                        let mut next = Draft::new(&book.login, &contact, false);
                        next.saved = true;
                        state.people.draft = Some(next);
                    }
                }
                Err(problem) => draft.problem = Some(problem),
            }
            Task::none()
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
                    book.groups.push(group.clone());
                    book.groups.sort_by_key(|group| group.name.to_lowercase());
                }
                if let Some(draft) = &mut state.people.draft
                    && draft.account == account
                {
                    draft.creating_label = false;
                    draft.new_label.clear();
                    draft.labels.push(group.resource);
                }
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
        .and_then(|book| {
            book.groups
                .iter()
                .find(|group| group.name.eq_ignore_ascii_case(&name))
        })
        .map(|group| group.resource.clone());
    let login = draft.login.clone();
    let draft = state.people.draft.as_mut().expect("checked above");
    if let Some(resource) = existing {
        if !draft.labels.contains(&resource) {
            draft.labels.push(resource);
        }
        draft.new_label.clear();
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
        Action::FieldEdited(field, value) => match field {
            Field::GivenName => draft.given = value,
            Field::FamilyName => draft.family = value,
            Field::Company => draft.company = value,
            Field::Title => draft.title = value,
            Field::NewLabel => draft.new_label = value,
        },
        Action::EntryEdited(list, index, value) => {
            if let Some(entry) = draft.entries_mut(list).get_mut(index) {
                entry.value = value;
            }
        }
        Action::EntryKindChosen(list, index, kind) => {
            if let Some(entry) = draft.entries_mut(list).get_mut(index) {
                entry.kind = kind;
            }
        }
        Action::EntryAdded(list) => draft.entries_mut(list).push(Entry {
            value: String::new(),
            kind: String::new(),
            source: None,
        }),
        Action::EntryRemoved(list, index) => {
            if index < draft.entries(list).len() {
                draft.entries_mut(list).remove(index);
            }
        }
        Action::LabelToggled(resource) => {
            match draft.labels.iter().position(|label| *label == resource) {
                Some(at) => {
                    draft.labels.remove(at);
                }
                None => draft.labels.push(resource),
            }
        }
        Action::NotesEdited(action) => draft.notes.perform(action),
        _ => {}
    }
}

#[cfg(test)]
mod tests;
