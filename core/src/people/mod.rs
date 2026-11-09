//! People: the contacts of every signed-in Google account, and the Workspace
//! directories those accounts belong to. The renderer-agnostic half, shared by
//! the iced People screen and the Dioxus People sheet.
//!
//! Each account is its own [`Book`], so a person saved in two accounts stays two
//! contacts and it is always clear which account a change goes to. Contacts are
//! read in full and searched here. A directory can run to thousands of people,
//! so it is only ever asked about what was typed.
//!
//! A contact keeps the JSON Google sent. An [`Edit`] changes the fields it
//! touched within that JSON and sends only those, so whatever the screen has no
//! control for (a phonetic name, a structured address) comes through unharmed.
//!
//! Everything Google sends goes into [`cache`]. That includes its answer to an
//! edit, which is the contact as Google now holds it: the list, the editor and
//! the cache all take that rather than what was typed.

pub mod api;
pub mod cache;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use api::{Directory, Group, Problem};

/// How many of a section's rows are drawn at first: laying out thousands of
/// them would be felt on every redraw, and search is the way through that many.
pub const PAGE: usize = 200;
/// Waits for a pause in typing before asking Google.
pub const SEARCH_DELAY: std::time::Duration = std::time::Duration::from_millis(350);
/// A single letter would match a good part of any directory.
pub const MIN_SEARCH: usize = 2;

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

/// What the account is called when the calendar's own sign-in is used and its
/// address could not be looked up.
pub const PRIMARY_LABEL: &str = "Google Calendar account";

/// A signed-in Google account. Holds no credentials, only which account to find
/// them for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Login {
    /// What to call it in a message: an address, or the calendar account.
    pub label: String,
    which: Which,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Which {
    /// The account the calendar is signed in with.
    Primary,
    /// An account connected in the desktop's Settings, by address.
    Extra(String),
}

impl Login {
    /// The calendar's sign-in, called `label`.
    pub fn primary(label: &str) -> Self {
        Self {
            label: label.to_owned(),
            which: Which::Primary,
        }
    }

    /// The calendar's sign-in, before its address is known.
    pub fn unidentified() -> Self {
        Self::primary(PRIMARY_LABEL)
    }

    /// An account connected besides the calendar's.
    pub fn extra(email: &str) -> Self {
        Self {
            label: email.to_owned(),
            which: Which::Extra(email.to_owned()),
        }
    }

    /// Whether this is the calendar's sign-in, which is renewed from a
    /// different place than a connected account.
    pub fn is_primary(&self) -> bool {
        self.which == Which::Primary
    }

    /// The calendar's sign-in, when its address could not be looked up.
    pub fn is_unidentified(&self) -> bool {
        self.is_primary() && self.label == PRIMARY_LABEL
    }

    /// The address of a connected account; `None` for the calendar's.
    pub fn extra_email(&self) -> Option<&str> {
        match &self.which {
            Which::Primary => None,
            Which::Extra(email) => Some(email),
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
    pub const ALL: [List; 4] = [List::Emails, List::Phones, List::Addresses, List::Urls];

    /// The field's name in the People API.
    pub fn field(self) -> &'static str {
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

    /// The kinds to offer for `current`: the usual ones, and whatever Google
    /// already had if it is none of them.
    pub fn kinds_with(self, current: &str) -> Vec<String> {
        let mut kinds: Vec<String> = self.kinds().iter().map(|kind| (*kind).to_owned()).collect();
        if !current.is_empty() && !kinds.iter().any(|kind| kind == current) {
            kinds.push(current.to_owned());
        }
        kinds
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

    /// The JSON Google sent.
    pub fn person(&self) -> &Value {
        &self.person
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

    pub fn sort_key(&self) -> String {
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

pub fn parse_all(people: Vec<Value>, groups: &[Group]) -> Vec<Contact> {
    people
        .into_iter()
        .filter_map(|person| Contact::parse(person, groups))
        .collect()
}

/// Whether `query` is long enough to ask a directory about.
pub fn searches_directory(query: &str) -> bool {
    query.trim().chars().count() >= MIN_SEARCH
}

/// One signed-in account and what it holds.
#[derive(Debug, Clone, PartialEq)]
pub struct Book {
    pub login: Login,
    pub contacts: Vec<Contact>,
    pub groups: Vec<Group>,
    pub loading: bool,
    pub problem: Option<Problem>,
    /// What the directory section shows, in Google's order once it has answered.
    pub directory: Vec<Contact>,
    /// Everyone a directory search has ever returned. A new search looks here
    /// first, so results appear at once and without a connection.
    pub directory_seen: Vec<Contact>,
    pub searching: bool,
    pub directory_problem: Option<Problem>,
    /// Google said this account is not part of a Workspace domain.
    pub no_directory: bool,
}

impl Book {
    /// An account whose contacts are on their way.
    pub fn new(login: Login) -> Self {
        Self {
            login,
            contacts: Vec::new(),
            groups: Vec::new(),
            loading: true,
            problem: None,
            directory: Vec::new(),
            directory_seen: Vec::new(),
            searching: false,
            directory_problem: None,
            no_directory: false,
        }
    }

    /// An account with nothing on its way.
    pub fn idle(login: Login) -> Self {
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
        if !self.may_search() {
            return None;
        }
        self.account().rsplit_once('@').map(|(_, domain)| domain)
    }

    /// The directory section's heading.
    pub fn directory_title(&self) -> String {
        match self.directory_domain() {
            Some(domain) => format!("{domain} directory"),
            None => format!("{} directory", self.account()),
        }
    }

    pub fn may_search(&self) -> bool {
        !self.no_directory && api::may_have_directory(self.account())
    }

    /// `contacts` that match `query`.
    pub fn matching(&self, query: &str) -> Vec<&Contact> {
        self.contacts
            .iter()
            .filter(|contact| contact.matches(query))
            .collect()
    }

    /// Shows what is already known of `query`, until Google answers.
    pub fn show_seen(&mut self, query: &str) {
        self.directory = self
            .directory_seen
            .iter()
            .filter(|person| person.matches(query))
            .cloned()
            .collect();
    }

    /// Google's answer replaces what is shown, and adds to what is known.
    pub fn show_found(&mut self, found: Vec<Contact>) {
        self.remember(&found);
        self.directory = found;
    }

    pub fn remember(&mut self, found: &[Contact]) {
        for person in found {
            match self
                .directory_seen
                .iter_mut()
                .find(|seen| seen.resource == person.resource)
            {
                Some(seen) => *seen = person.clone(),
                None => self.directory_seen.push(person.clone()),
            }
        }
    }

    /// Google said there is no directory to search.
    pub fn has_no_directory(&mut self) {
        self.no_directory = true;
        self.directory.clear();
        self.directory_seen.clear();
    }

    /// Takes in a fresh read of the account.
    pub fn loaded(&mut self, people: Vec<Value>, groups: Vec<Group>) {
        self.contacts = parse_all(people, &groups);
        self.contacts.sort_by_key(Contact::sort_key);
        self.groups = groups;
        self.problem = None;
    }

    /// Puts `person`, as Google returned it, in place of the contact it was,
    /// or adds it. `None` if Google's answer could not be read.
    pub fn store(&mut self, person: Value) -> Option<Contact> {
        let contact = Contact::parse(person, &self.groups)?;
        match self
            .contacts
            .iter_mut()
            .find(|c| c.resource == contact.resource)
        {
            Some(slot) => *slot = contact.clone(),
            None => self.contacts.push(contact.clone()),
        }
        self.contacts.sort_by_key(Contact::sort_key);
        Some(contact)
    }

    /// Takes in a label just created.
    pub fn add_group(&mut self, group: Group) {
        self.groups.push(group);
        self.groups.sort_by_key(|group| group.name.to_lowercase());
    }

    /// The label called `name`, in any case.
    pub fn group_named(&self, name: &str) -> Option<&Group> {
        self.groups
            .iter()
            .find(|group| group.name.eq_ignore_ascii_case(name.trim()))
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

impl Entry {
    /// A line that is not yet in Google.
    pub fn new(value: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            kind: kind.into(),
            source: None,
        }
    }
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

/// What a single-line text box of the editor edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    GivenName,
    FamilyName,
    Company,
    Title,
    NewLabel,
}

/// A contact being edited, or a directory entry being read.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
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
    pub notes: String,
}

impl Edit {
    pub fn new(contact: &Contact, read_only: bool) -> Self {
        let person = &contact.person;
        Self {
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
            notes: contact.notes.clone(),
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

    pub fn entries_mut(&mut self, list: List) -> &mut Vec<Entry> {
        match list {
            List::Emails => &mut self.emails,
            List::Phones => &mut self.phones,
            List::Addresses => &mut self.addresses,
            List::Urls => &mut self.urls,
        }
    }

    pub fn field(&self, field: Field) -> &str {
        match field {
            Field::GivenName => &self.given,
            Field::FamilyName => &self.family,
            Field::Company => &self.company,
            Field::Title => &self.title,
            Field::NewLabel => &self.new_label,
        }
    }

    pub fn set_field(&mut self, field: Field, value: String) {
        match field {
            Field::GivenName => self.given = value,
            Field::FamilyName => self.family = value,
            Field::Company => self.company = value,
            Field::Title => self.title = value,
            Field::NewLabel => self.new_label = value,
        }
    }

    pub fn set_entry(&mut self, list: List, index: usize, value: String) {
        if let Some(entry) = self.entries_mut(list).get_mut(index) {
            entry.value = value;
        }
    }

    pub fn set_kind(&mut self, list: List, index: usize, kind: String) {
        if let Some(entry) = self.entries_mut(list).get_mut(index) {
            entry.kind = kind;
        }
    }

    pub fn add_entry(&mut self, list: List) {
        self.entries_mut(list).push(Entry::new("", ""));
    }

    pub fn remove_entry(&mut self, list: List, index: usize) {
        if index < self.entries(list).len() {
            self.entries_mut(list).remove(index);
        }
    }

    pub fn toggle_label(&mut self, resource: String) {
        match self.labels.iter().position(|label| *label == resource) {
            Some(at) => {
                self.labels.remove(at);
            }
            None => self.labels.push(resource),
        }
    }

    /// Selects an existing label and empties the box it was typed in.
    pub fn choose_label(&mut self, resource: String) {
        if !self.labels.contains(&resource) {
            self.labels.push(resource);
        }
        self.new_label.clear();
    }

    pub fn dirty(&self) -> bool {
        !self.read_only && !self.changed_fields().is_empty()
    }

    /// The People API fields that differ from what Google sent.
    pub fn changed_fields(&self) -> Vec<&'static str> {
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
        if self.notes.trim_end() != self.contact.notes.trim_end() {
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
    pub fn patch(&self) -> (Vec<&'static str>, Value) {
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

    /// The editor edits the first organization; any others are passed along.
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
        let text = self.notes.trim_end();
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

    /// Memberships the editor does not show, such as "My Contacts", stay. Google
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
    pub fn as_new_contact(&self) -> Value {
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
