//! The People sheet: the contacts of the signed-in Google account and, for a
//! Workspace account, its directory. The desktop's People screen, laid out for
//! a phone: the list, and the open contact in its place. On a wide window the
//! two sit side by side.
//!
//! What a contact is, how it is searched and how an edit becomes an update are
//! `timeparagon_core::people`, shared with the desktop. So is the cache, which
//! opens the sheet straight away and keeps it useful offline.
//!
//! Only the calendar's own sign-in is used here: the desktop's extra accounts
//! have no secure store on the phone yet. Cached books of such accounts (the
//! Dioxus desktop build shares the iced app's database) are kept as they are.

use std::path::PathBuf;

use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use serde_json::Value;
use timeparagon_core::calendar::GoogleClient;
use timeparagon_core::calendar::oauth;
use timeparagon_core::config::CalendarSettings;
use timeparagon_core::people::{
	self as shared, Book, Contact, Directory, Edit, Field, Group, List, Login, PAGE, Problem, api,
	cache,
};

/// A row of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
	pub account: String,
	pub resource: String,
	pub directory: bool,
}

/// Where to go once unsaved changes have been dealt with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
	Open(Target),
	Close,
}

/// The contact on show: being edited, or a directory entry being read.
#[derive(Debug, Clone, PartialEq)]
pub struct Opened {
	pub login: Login,
	pub edit: Edit,
	pub saving: bool,
	pub saved: bool,
	pub creating_label: bool,
	pub problem: Option<Problem>,
	/// Asked to go elsewhere with unsaved changes: the sheet asks first.
	pub pending: Option<Next>,
}

impl Opened {
	fn new(login: &Login, contact: &Contact, read_only: bool) -> Self {
		Self {
			login: login.clone(),
			edit: Edit::new(contact, read_only),
			saving: false,
			saved: false,
			creating_label: false,
			problem: None,
			pending: None,
		}
	}

	pub fn account(&self) -> &str {
		&self.login.label
	}

	fn is(&self, account: &str, resource: &str) -> bool {
		self.account() == account && self.edit.contact.resource == resource
	}
}

/// An edit to the open contact.
#[derive(Debug, Clone)]
pub enum Change {
	Field(Field, String),
	Entry(List, usize, String),
	Kind(List, usize, String),
	Add(List),
	Remove(List, usize),
	Label(String),
	Notes(String),
}

/// A call to Google the store has asked for.
pub enum Call {
	Save {
		login: Login,
		resource: String,
		fields: Vec<&'static str>,
		body: Value,
	},
	Add {
		login: Login,
		body: Value,
	},
	Label {
		login: Login,
		name: String,
	},
}

/// Everything the People sheet holds. Lives as long as the app, so reopening
/// the sheet shows what was there.
#[derive(Default)]
pub struct Store {
	/// Only the real app reaches Google or the disk; test stores stay isolated.
	live: bool,
	/// The cache has been read.
	started: bool,
	cache_path: Option<PathBuf>,
	pub books: Vec<Book>,
	/// Cached accounts this app cannot sign in to. Kept for the cache's sake.
	others: Vec<Book>,
	/// Finding out who is signed in.
	pub listing: bool,
	pub query: String,
	/// Show only this account.
	pub account: Option<String>,
	/// Further pages shown of every section.
	pub more: usize,
	pub open: Option<Opened>,
	generation: u64,
	search_generation: u64,
}

impl Store {
	/// A store that talks to Google and keeps the cache in the app's database.
	pub fn live() -> Self {
		Self {
			live: true,
			cache_path: timeparagon_core::task_db::path(),
			..Self::default()
		}
	}

	/// Reads the cache the first time the sheet opens, rather than at startup.
	fn start(&mut self) {
		if self.started || !self.live {
			return;
		}
		self.started = true;
		if let Some(path) = &self.cache_path {
			let (mine, others) = cache::load(path)
				.into_iter()
				.partition(|book| book.login.is_primary());
			self.books = mine;
			self.others = others;
		}
	}

	pub fn shown_books(&self) -> impl Iterator<Item = &Book> {
		self.books.iter().filter(|book| {
			self.account
				.as_deref()
				.is_none_or(|account| account == book.account())
		})
	}

	pub fn limit(&self) -> usize {
		PAGE * (self.more + 1)
	}

	pub fn searches_directory(&self) -> bool {
		shared::searches_directory(&self.query)
	}

	pub fn busy(&self) -> bool {
		self.listing || self.books.iter().any(|book| book.loading)
	}

	pub fn is_open(&self, account: &str, resource: &str) -> bool {
		self.open
			.as_ref()
			.is_some_and(|open| open.is(account, resource))
	}

	pub fn book(&self, account: &str) -> Option<&Book> {
		self.books.iter().find(|book| book.account() == account)
	}

	fn book_mut(&mut self, account: &str) -> Option<&mut Book> {
		self.books.iter_mut().find(|book| book.account() == account)
	}

	/// How many rows the search finds.
	pub fn total(&self) -> usize {
		self.shown_books()
			.map(|book| {
				let directory = if self.searches_directory() {
					book.directory.len()
				} else {
					0
				};
				book.matching(&self.query).len() + directory
			})
			.sum()
	}

	/// The domains a search would reach, as a phrase.
	pub fn directory_domains(&self) -> String {
		let mut domains: Vec<&str> = self
			.shown_books()
			.filter_map(Book::directory_domain)
			.collect();
		domains.dedup();
		domains.join(" and ")
	}

	// ------------------------------------------------------------ loading

	fn begin_refresh(&mut self) -> u64 {
		self.generation += 1;
		self.listing = true;
		self.generation
	}

	/// Who is signed in. Returns the accounts to load, or nothing if this
	/// answer is out of date.
	fn listed(&mut self, generation: u64, logins: Vec<Login>) -> Vec<Login> {
		if generation != self.generation {
			return Vec::new();
		}
		self.listing = false;
		let mut books = Vec::new();
		for login in logins {
			// Offline, the calendar's account can't say what its address
			// is. It is still the one that was cached under that address.
			let mut book = self
				.books
				.iter()
				.position(|book| {
					book.login == login || (login.is_unidentified() && book.login.is_primary())
				})
				.map(|at| self.books.remove(at))
				.unwrap_or_else(|| Book::new(login));
			book.loading = true;
			books.push(book);
		}
		// Whatever is left over is no longer signed in.
		self.books = books;
		if self
			.open
			.as_ref()
			.is_some_and(|open| self.book(open.account()).is_none())
		{
			self.open = None;
		}
		if let Some(account) = &self.account
			&& self.book(account).is_none()
		{
			self.account = None;
		}
		self.books.iter().map(|book| book.login.clone()).collect()
	}

	/// An account's contacts arrived. Returns whether to write the cache.
	fn loaded(
		&mut self,
		generation: u64,
		account: &str,
		result: Result<(Vec<Value>, Vec<Group>), Problem>,
	) -> bool {
		if generation != self.generation {
			return false;
		}
		let Some(book) = self.book_mut(account) else {
			return false;
		};
		book.loading = false;
		// A failure leaves the cached contacts on show, with the reason.
		let fetched = result.is_ok();
		match result {
			Ok((people, groups)) => book.loaded(people, groups),
			Err(problem) => book.problem = Some(problem),
		}
		// An open contact with nothing to lose follows what was loaded, so the
		// next save is against the latest version.
		let fresh = self.open.as_ref().and_then(|open| {
			if open.account() != account || open.edit.read_only || open.edit.dirty() {
				return None;
			}
			let book = self.book(account)?;
			let contact = book
				.contacts
				.iter()
				.find(|c| c.resource == open.edit.contact.resource)?;
			(contact.person() != open.edit.contact.person())
				.then(|| Opened::new(&book.login, contact, false))
		});
		if fresh.is_some() {
			self.open = fresh;
		}
		fetched
	}

	// ------------------------------------------------------------ search

	/// Returns the search to send once typing pauses, if the query reaches
	/// the directory.
	pub fn set_query(&mut self, query: String) -> Option<u64> {
		self.query = query;
		self.more = 0;
		self.search_generation += 1;
		if !self.searches_directory() {
			for book in &mut self.books {
				book.directory.clear();
				book.searching = false;
				book.directory_problem = None;
			}
			return None;
		}
		let query = self.query.trim().to_owned();
		for book in self.books.iter_mut().filter(|book| book.may_search()) {
			book.show_seen(&query);
		}
		Some(self.search_generation)
	}

	/// The accounts to ask about the query, marked as being asked.
	fn begin_search(&mut self, generation: u64) -> Vec<Login> {
		if generation != self.search_generation || !self.searches_directory() {
			return Vec::new();
		}
		self.books
			.iter_mut()
			.filter(|book| book.may_search())
			.map(|book| {
				book.searching = true;
				book.login.clone()
			})
			.collect()
	}

	/// A directory answered. Returns whether to write the cache.
	fn found(
		&mut self,
		generation: u64,
		account: &str,
		result: Result<Directory, Problem>,
	) -> bool {
		if generation != self.search_generation {
			return false;
		}
		let Some(book) = self.book_mut(account) else {
			return false;
		};
		book.searching = false;
		book.directory_problem = None;
		match result {
			Ok(Directory::Found(people)) => book.show_found(shared::parse_all(people, &[])),
			Ok(Directory::None) => book.has_no_directory(),
			Err(problem) => {
				book.directory_problem = Some(problem);
				return false;
			}
		}
		true
	}

	// ------------------------------------------------------------ the open contact

	/// Goes straight to `next`, unless that would lose unsaved changes.
	pub fn go(&mut self, next: Next) {
		if let Some(open) = &mut self.open {
			let same = matches!(&next, Next::Open(target)
                if target.directory == open.edit.read_only
                    && open.is(&target.account, &target.resource));
			if open.saving || same {
				return;
			}
			if open.edit.dirty() {
				open.pending = Some(next);
				return;
			}
		}
		self.follow(next);
	}

	fn follow(&mut self, next: Next) {
		match next {
			Next::Close => self.open = None,
			Next::Open(target) => {
				let Some(book) = self.book(&target.account) else {
					return;
				};
				let source = if target.directory {
					&book.directory
				} else {
					&book.contacts
				};
				if let Some(contact) = source.iter().find(|c| c.resource == target.resource) {
					self.open = Some(Opened::new(&book.login, contact, target.directory));
				}
			}
		}
	}

	/// Throws away the unsaved changes and carries on to wherever was asked for.
	pub fn discard(&mut self) {
		if let Some(next) = self.open.as_mut().and_then(|open| open.pending.take()) {
			self.follow(next);
		}
	}

	pub fn keep_editing(&mut self) {
		if let Some(open) = &mut self.open {
			open.pending = None;
		}
	}

	pub fn revert(&mut self) {
		if let Some(open) = &mut self.open {
			*open = Opened::new(&open.login, &open.edit.contact, open.edit.read_only);
		}
	}

	pub fn change(&mut self, change: Change) {
		let Some(open) = &mut self.open else {
			return;
		};
		if open.edit.read_only || open.saving {
			return;
		}
		open.saved = false;
		open.pending = None;
		let edit = &mut open.edit;
		match change {
			Change::Field(field, value) => edit.set_field(field, value),
			Change::Entry(list, index, value) => edit.set_entry(list, index, value),
			Change::Kind(list, index, kind) => edit.set_kind(list, index, kind),
			Change::Add(list) => edit.add_entry(list),
			Change::Remove(list, index) => edit.remove_entry(list, index),
			Change::Label(resource) => edit.toggle_label(resource),
			Change::Notes(text) => edit.notes = text,
		}
	}

	fn begin_save(&mut self) -> Option<Call> {
		let open = self.open.as_mut()?;
		if open.saving || !open.edit.dirty() {
			return None;
		}
		open.saving = true;
		open.saved = false;
		open.problem = None;
		open.pending = None;
		let (fields, body) = open.edit.patch();
		Some(Call::Save {
			login: open.login.clone(),
			resource: open.edit.contact.resource.clone(),
			fields,
			body,
		})
	}

	fn begin_add(&mut self) -> Option<Call> {
		let open = self.open.as_mut()?;
		if open.saving || !open.edit.read_only {
			return None;
		}
		open.saving = true;
		open.problem = None;
		Some(Call::Add {
			login: open.login.clone(),
			body: open.edit.as_new_contact(),
		})
	}

	/// Selects the label named in the box, creating it if the account has
	/// none by that name. Created straight away, as Google Contacts does, so
	/// that a failed save has no half-made label to account for.
	fn begin_label(&mut self) -> Option<Call> {
		let open = self.open.as_ref()?;
		let name = open.edit.new_label.trim().to_owned();
		if name.is_empty() || open.creating_label || open.edit.read_only {
			return None;
		}
		let existing = self
			.book(open.account())
			.and_then(|book| book.group_named(&name))
			.map(|group| group.resource.clone());
		let open = self.open.as_mut()?;
		if let Some(resource) = existing {
			open.saved = false;
			open.edit.choose_label(resource);
			return None;
		}
		open.creating_label = true;
		open.problem = None;
		Some(Call::Label {
			login: open.login.clone(),
			name,
		})
	}

	/// Google answered a save or an add with the contact as it now holds it.
	/// Returns whether to write the cache.
	fn stored(&mut self, account: &str, result: Result<Value, Problem>) -> bool {
		let Some(open) = &mut self.open else {
			return false;
		};
		let same_account = open.account() == account;
		if same_account {
			open.saving = false;
		}
		let person = match result {
			Ok(person) => person,
			Err(problem) => {
				if same_account {
					open.problem = Some(problem);
				}
				return false;
			}
		};
		let Some(book) = self.book_mut(account) else {
			return false;
		};
		let Some(contact) = book.store(person) else {
			return false;
		};
		if same_account {
			let login = book.login.clone();
			let mut next = Opened::new(&login, &contact, false);
			next.saved = true;
			self.open = Some(next);
		}
		true
	}

	/// Returns whether to write the cache.
	fn label_created(&mut self, account: &str, result: Result<Group, Problem>) -> bool {
		let group = match result {
			Ok(group) => group,
			Err(problem) => {
				if let Some(open) = &mut self.open {
					open.creating_label = false;
					open.problem = Some(problem);
				}
				return false;
			}
		};
		if let Some(book) = self.book_mut(account) {
			book.add_group(group.clone());
		}
		if let Some(open) = &mut self.open
			&& open.account() == account
		{
			open.creating_label = false;
			open.saved = false;
			open.edit.choose_label(group.resource);
		}
		true
	}

	fn snapshot(&self) -> Option<(PathBuf, cache::Snapshot)> {
		let path = self.cache_path.clone()?;
		let books: Vec<Book> = self.books.iter().chain(&self.others).cloned().collect();
		match cache::snapshot(&books) {
			Ok(snapshot) => Some((path, snapshot)),
			Err(error) => {
				eprintln!("could not prepare the contacts cache: {error}");
				None
			}
		}
	}
}

// ---------------------------------------------------------------- Google

/// The calendar account's access token, refreshed if needed. Blocking.
fn token(settings: &CalendarSettings) -> Result<String, Problem> {
	GoogleClient::from_settings(settings)
		.access_token()
		.map_err(api::describe)
}

/// The calendar's account, by its address if that can be found. Blocking.
fn identify(settings: &CalendarSettings) -> Vec<Login> {
	if oauth::load_tokens().is_none() {
		return Vec::new();
	}
	let email = token(settings)
		.ok()
		.and_then(|token| api::profile_email(&token).ok());
	vec![match email {
		Some(email) => Login::primary(&email),
		None => Login::unidentified(),
	}]
}

fn load(settings: &CalendarSettings) -> Result<(Vec<Value>, Vec<Group>), Problem> {
	let token = token(settings)?;
	let groups = api::groups(&token)?;
	Ok((api::contacts(&token)?, groups))
}

/// Runs blocking work on a worker thread.
async fn worker<T: Send + 'static>(
	work: impl FnOnce() -> Result<T, Problem> + Send + 'static,
) -> Result<T, Problem> {
	tokio::task::spawn_blocking(work)
		.await
		.map_err(|error| Problem::other(format!("The contacts worker stopped: {error}")))
		.and_then(|result| result)
}

/// Writes the cache on a worker thread.
fn persist(store: Signal<Store>) {
	let Some((path, snapshot)) = store.peek().snapshot() else {
		return;
	};
	std::thread::spawn(move || {
		if let Err(error) = snapshot.write(&path) {
			eprintln!("could not save the contacts cache: {error}");
		}
	});
}

/// Reloads everything. Results of an earlier refresh still on their way are
/// ignored when they arrive. Work outlives the sheet: the store is the app's.
pub fn refresh(mut store: Signal<Store>, settings: CalendarSettings) {
	if !store.peek().live {
		return;
	}
	let generation = store.write().begin_refresh();
	spawn_forever(async move {
		let listing = settings.clone();
		let logins = worker(move || Ok(identify(&listing)))
			.await
			.unwrap_or_default();
		let had_books = !store.peek().books.is_empty();
		let to_load = store.write().listed(generation, logins);
		if had_books && store.peek().books.is_empty() {
			persist(store);
		}
		for login in to_load {
			let settings = settings.clone();
			spawn_forever(async move {
				let result = worker(move || load(&settings)).await;
				if store.write().loaded(generation, &login.label, result) {
					persist(store);
				}
			});
		}
		let search = store.peek().search_generation;
		search_directories(store, settings, search);
	});
}

/// Asks each account's directory about the current search.
fn search_directories(mut store: Signal<Store>, settings: CalendarSettings, generation: u64) {
	if !store.peek().live {
		return;
	}
	let query = store.peek().query.trim().to_owned();
	let asking = store.write().begin_search(generation);
	for login in asking {
		let (settings, query) = (settings.clone(), query.clone());
		spawn_forever(async move {
			let result = worker(move || {
				let token = token(&settings)?;
				api::search_directory(&token, &query)
			})
			.await;
			if store.write().found(generation, &login.label, result) {
				persist(store);
			}
		});
	}
}

/// Takes in what was typed in the search box, and asks the directories once
/// typing pauses.
pub fn search(mut store: Signal<Store>, settings: CalendarSettings, query: String) {
	let Some(generation) = store.write().set_query(query) else {
		return;
	};
	if !store.peek().live {
		return;
	}
	spawn_forever(async move {
		tokio::time::sleep(shared::SEARCH_DELAY).await;
		search_directories(store, settings, generation);
	});
}

/// Sends what the store asked for, and gives it Google's answer.
fn send(mut store: Signal<Store>, settings: CalendarSettings, call: Option<Call>) {
	let Some(call) = call else {
		return;
	};
	if !store.peek().live {
		return;
	}
	spawn_forever(async move {
		let persist_after = match call {
			Call::Save {
				login,
				resource,
				fields,
				body,
			} => {
				let reach = settings.clone();
				let result =
					worker(move || api::update(&token(&reach)?, &resource, &fields, &body)).await;
				let was_loading = store.peek().book(&login.label).is_some_and(|b| b.loading);
				let stored = store.write().stored(&login.label, result);
				// A reload begun before this save would bring the old version
				// back, so start it over.
				if stored && was_loading {
					refresh(store, settings);
				}
				stored
			}
			Call::Add { login, body } => {
				let result = worker(move || api::create(&token(&settings)?, &body)).await;
				store.write().stored(&login.label, result)
			}
			Call::Label { login, name } => {
				let result = worker(move || api::create_group(&token(&settings)?, &name)).await;
				store.write().label_created(&login.label, result)
			}
		};
		if persist_after {
			persist(store);
		}
	});
}

// ---------------------------------------------------------------- the sheet

#[component]
pub fn PeopleSheet(
	store: Signal<Store>,
	settings: CalendarSettings,
	/// The Google sign-in under way, which [`PeopleSheet::on_reconnect`] starts.
	connecting: Signal<bool>,
	on_reconnect: EventHandler<()>,
	on_close: EventHandler<()>,
) -> Element {
	let refresh_settings = settings.clone();
	use_hook(move || {
		store.write().start();
		refresh(store, refresh_settings);
	});
	// Signing in again from a problem line loads everything again once done.
	let mut awaiting_sign_in = use_signal(|| false);
	let reconnect_settings = settings.clone();
	use_effect(move || {
		if !connecting() && *awaiting_sign_in.peek() {
			awaiting_sign_in.set(false);
			refresh(store, reconnect_settings.clone());
		}
	});
	let reconnect = move |()| {
		awaiting_sign_in.set(true);
		on_reconnect.call(());
	};

	let shown = store.read();
	let busy = shown.busy();
	let total = shown.total();
	let has_open = shown.open.is_some();
	let saving = shown.open.as_ref().is_some_and(|open| open.saving);
	drop(shown);
	let refresh_now = settings.clone();
	rsx! {
		div { class: "modal-backdrop editor-backdrop",
			div { class: "modal editor people-sheet", role: "dialog", aria_label: "People",
				div { class: "modal-head",
					div { class: "modal-titles",
						div { class: "modal-title-row",
							h2 { class: "modal-title", "People" }
							span { class: "count-pill", "{total}" }
						}
						p { class: "modal-sub", "Contacts from your Google account" }
					}
					button {
						class: "btn small",
						disabled: busy,
						onclick: move |_| refresh(store, refresh_now.clone()),
						if busy { "Refreshing…" } else { "Refresh" }
					}
					button {
						class: "modal-close",
						title: "Close",
						disabled: saving,
						onclick: move |_| on_close.call(()),
						"×"
					}
				}
				div { class: "modal-body editor-body people-body", "data-open": "{has_open}",
					div { class: "people-list",
						ContactList { store, settings: settings.clone(), on_reconnect: reconnect }
					}
					if has_open {
						div { class: "people-detail",
							ContactPanel { store, settings: settings.clone(), on_reconnect: reconnect }
						}
					}
				}
			}
		}
	}
}

#[component]
fn ContactList(
	store: Signal<Store>,
	settings: CalendarSettings,
	on_reconnect: EventHandler<()>,
) -> Element {
	let shown = store.read();
	let query = shown.query.clone();
	let accounts: Vec<String> = shown.books.iter().map(|b| b.account().to_owned()).collect();
	let chosen = shown.account.clone().unwrap_or_default();
	let domains = shown.directory_domains();
	let searches = shown.searches_directory();
	let no_accounts = shown.books.is_empty();
	let listing = shown.listing;
	let books: Vec<Book> = shown.shown_books().cloned().collect();
	drop(shown);
	rsx! {
		input {
			class: "client-input",
			r#type: "search",
			placeholder: "Search names, notes and labels…",
			aria_label: "Search people",
			value: "{query}",
			oninput: move |event| search(store, settings.clone(), event.value()),
		}
		if accounts.len() > 1 {
			select {
				class: "client-input",
				aria_label: "Account",
				value: "{chosen}",
				onchange: move |event: FormEvent| {
					let value = event.value();
					let mut store = store.write();
					store.account = (!value.is_empty()).then_some(value);
					store.more = 0;
				},
				option { value: "", selected: chosen.is_empty(), "All accounts" }
				for account in accounts {
					option { key: "{account}", value: "{account}", selected: chosen == account, "{account}" }
				}
			}
		}
		if no_accounts {
			if listing {
				p { class: "empty-note", "Looking for your Google account…" }
			} else {
				p { class: "empty-note",
					"Not connected to Google. Connect your account in Settings to see its contacts."
				}
			}
		}
		if !searches && !domains.is_empty() {
			p { class: "hint", "Type to also search the {domains} directory" }
		}
		for book in books {
			Section { key: "{book.account()}", store, book: book.clone(), on_reconnect }
			if searches
				&& (book.searching || book.directory_problem.is_some() || !book.directory.is_empty())
			{
				DirectorySection { key: "{book.account()}-directory", store, book, on_reconnect }
			}
		}
	}
}

/// One account's contacts under its address.
#[component]
fn Section(store: Signal<Store>, book: Book, on_reconnect: EventHandler<()>) -> Element {
	let query = store.read().query.clone();
	let contacts: Vec<Contact> = book.matching(&query).into_iter().cloned().collect();
	let empty = if book.contacts.is_empty() {
		"No contacts in this account"
	} else {
		"No matching contacts"
	};
	rsx! {
		section { class: "panel-section people-section",
			div { class: "section-head",
				span { class: "section-title", "{book.account()}" }
				span { class: "count-pill", "{contacts.len()}" }
			}
			if let Some(problem) = book.problem.clone() {
				ProblemLine { problem, on_reconnect }
			}
			if book.loading && book.contacts.is_empty() {
				p { class: "empty-note", "Loading contacts…" }
			} else if contacts.is_empty() && book.problem.is_none() {
				p { class: "empty-note", "{empty}" }
			}
			Rows { store, account: book.account().to_owned(), contacts, directory: false }
		}
	}
}

/// The people from the account's directory who match the search.
#[component]
fn DirectorySection(store: Signal<Store>, book: Book, on_reconnect: EventHandler<()>) -> Element {
	rsx! {
		section { class: "panel-section people-section",
			div { class: "section-head",
				span { class: "section-title", "{book.directory_title()}" }
				span { class: "count-pill", "{book.directory.len()}" }
			}
			if let Some(problem) = book.directory_problem.clone() {
				ProblemLine { problem, on_reconnect }
			} else if book.searching && book.directory.is_empty() {
				p { class: "empty-note", "Searching…" }
			}
			Rows {
				store,
				account: book.account().to_owned(),
				contacts: book.directory.clone(),
				directory: true,
			}
		}
	}
}

#[component]
fn Rows(store: Signal<Store>, account: String, contacts: Vec<Contact>, directory: bool) -> Element {
	let shown = store.read();
	let limit = shown.limit();
	let query = shown.query.clone();
	let rows: Vec<(Contact, bool)> = contacts
		.iter()
		.take(limit)
		.map(|contact| (contact.clone(), shown.is_open(&account, &contact.resource)))
		.collect();
	drop(shown);
	let left = contacts.len().saturating_sub(limit);
	rsx! {
		div { class: "people-rows",
			for (contact, open) in rows {
				{
					let target = Target {
						account: account.clone(),
						resource: contact.resource.clone(),
						directory,
					};
					let summary = contact.summary();
					let hint = contact.note_hint(&query).map(str::to_owned);
					rsx! {
						button {
							key: "{contact.resource}",
							class: "people-row",
							aria_pressed: "{open}",
							onclick: move |_| store.write().go(Next::Open(target.clone())),
							span { class: "people-name", "{contact.name}" }
							if !summary.is_empty() {
								span { class: "people-sub", "{summary}" }
							}
							if let Some(hint) = hint {
								span { class: "people-sub", "Notes: {hint}" }
							}
							if !contact.labels.is_empty() {
								span { class: "people-labels",
									for label in contact.labels.iter() {
										span { key: "{label}", class: "meta-pill", "{label}" }
									}
								}
							}
						}
					}
				}
			}
			if left > 0 {
				button {
					class: "btn",
					onclick: move |_| store.write().more += 1,
					"Show more ({left} left)"
				}
			}
		}
	}
}

/// What went wrong with the account, and the sign-in that would fix it.
#[component]
fn ProblemLine(problem: Problem, on_reconnect: EventHandler<()>) -> Element {
	rsx! {
		p { class: "editor-error", role: "alert", "{problem.message}" }
		if problem.reconnect {
			button { class: "btn primary", onclick: move |_| on_reconnect.call(()), "Sign in to Google again" }
		}
	}
}

/// The open contact: an editor, or a directory entry to read.
#[component]
fn ContactPanel(
	store: Signal<Store>,
	settings: CalendarSettings,
	on_reconnect: EventHandler<()>,
) -> Element {
	let Some(open) = store.read().open.clone() else {
		return rsx! {};
	};
	let edit = &open.edit;
	let saving = open.saving;
	let place = if edit.read_only {
		format!("In the directory of {}", open.account())
	} else {
		format!("Saved in {}", open.account())
	};
	rsx! {
		div { class: "people-detail-head",
			button {
				class: "btn small people-back",
				disabled: saving,
				onclick: move |_| store.write().go(Next::Close),
				"‹ People"
			}
			div { class: "modal-titles",
				h3 { class: "modal-title", "{edit.contact.name}" }
				p { class: "modal-sub", "{place}" }
			}
		}
		if let Some(problem) = open.problem.clone() {
			ProblemLine { problem, on_reconnect }
		}
		if edit.read_only {
			DirectoryEntry { store, settings: settings.clone(), open: open.clone() }
		} else {
			ContactEditor { store, settings: settings.clone(), open: open.clone() }
		}
		Footer { store, settings, open: open.clone() }
	}
}

fn text_line(
	mut store: Signal<Store>,
	label: &'static str,
	field: Field,
	value: String,
) -> Element {
	rsx! {
		label { class: "editor-field people-half",
			span { class: "editor-label", "{label}" }
			input {
				class: "client-input",
				r#type: "text",
				placeholder: "{label}",
				value: "{value}",
				oninput: move |event| store.write().change(Change::Field(field, event.value())),
			}
		}
	}
}

const LISTS: [(List, &str, &str, &str, &str); 4] = [
	(
		List::Emails,
		"Email",
		"Add email",
		"name@example.org",
		"email",
	),
	(List::Phones, "Phone", "Add phone", "Phone number", "tel"),
	(List::Addresses, "Address", "Add address", "Address", "text"),
	(List::Urls, "Website", "Add website", "https://", "url"),
];

#[component]
fn ContactEditor(store: Signal<Store>, settings: CalendarSettings, open: Opened) -> Element {
	let edit = open.edit.clone();
	let groups: Vec<Group> = store
		.read()
		.book(open.account())
		.map(|book| book.groups.clone())
		.unwrap_or_default();
	let new_label = edit.new_label.clone();
	let can_add_label = !new_label.trim().is_empty() && !open.creating_label;
	rsx! {
		div { class: "people-pair",
			{text_line(store, "First name", Field::GivenName, edit.given.clone())}
			{text_line(store, "Last name", Field::FamilyName, edit.family.clone())}
		}
		div { class: "people-pair",
			{text_line(store, "Company", Field::Company, edit.company.clone())}
			{text_line(store, "Job title", Field::Title, edit.title.clone())}
		}
		for (list, label, add, placeholder, kind) in LISTS {
			div { key: "{label}", class: "editor-field",
				span { class: "editor-label", "{label}" }
				for (index, entry) in edit.entries(list).iter().enumerate() {
					div { key: "{index}", class: "editor-row",
						input {
							class: "client-input",
							r#type: "{kind}",
							placeholder: "{placeholder}",
							aria_label: "{label} {index + 1}",
							value: "{entry.value}",
							oninput: move |event| store.write().change(Change::Entry(list, index, event.value())),
						}
						select {
							class: "client-input people-kind",
							aria_label: "{label} {index + 1} type",
							value: "{entry.kind}",
							onchange: move |event: FormEvent| store.write().change(Change::Kind(list, index, event.value())),
							option { value: "", selected: entry.kind.is_empty(), "Type" }
							for choice in list.kinds_with(&entry.kind) {
								option { key: "{choice}", value: "{choice}", selected: choice == entry.kind, "{choice}" }
							}
						}
						button {
							class: "btn icon",
							title: "Remove",
							aria_label: "Remove {label} {index + 1}",
							onclick: move |_| store.write().change(Change::Remove(list, index)),
							"×"
						}
					}
				}
				button {
					class: "btn small people-add",
					onclick: move |_| store.write().change(Change::Add(list)),
					"{add}"
				}
			}
		}
		div { class: "editor-field",
			span { class: "editor-label", "Labels" }
			if !groups.is_empty() {
				div { class: "editor-wrap",
					for group in groups {
						{
							let on = edit.labels.contains(&group.resource);
							let resource = group.resource.clone();
							rsx! {
								button {
									key: "{group.resource}",
									class: "nav-tab people-label",
									aria_pressed: "{on}",
									onclick: move |_| store.write().change(Change::Label(resource.clone())),
									"{group.name}"
								}
							}
						}
					}
				}
			}
			form {
				class: "editor-row",
				onsubmit: move |event| {
					event.prevent_default();
					let call = store.write().begin_label();
					send(store, settings.clone(), call);
				},
				input {
					class: "client-input",
					r#type: "text",
					placeholder: "New label",
					aria_label: "New label",
					value: "{new_label}",
					oninput: move |event| store.write().change(Change::Field(Field::NewLabel, event.value())),
				}
				button {
					class: "btn small",
					r#type: "submit",
					disabled: !can_add_label,
					if open.creating_label { "Adding…" } else { "Add label" }
				}
			}
		}
		label { class: "editor-field",
			span { class: "editor-label", "Notes" }
			textarea {
				class: "client-input editor-notes",
				placeholder: "Notes",
				value: "{edit.notes}",
				oninput: move |event| store.write().change(Change::Notes(event.value())),
			}
		}
	}
}

/// A directory entry: what the domain says about them, and a way to keep a
/// copy that can be edited.
#[component]
fn DirectoryEntry(store: Signal<Store>, settings: CalendarSettings, open: Opened) -> Element {
	let contact = open.edit.contact.clone();
	let saved = store
		.read()
		.book(open.account())
		.and_then(|book| book.saved_with_email(&contact.emails))
		.map(|saved| Target {
			account: open.account().to_owned(),
			resource: saved.resource.clone(),
			directory: false,
		});
	let facts = [
		("Company", contact.company.clone()),
		("Job title", contact.title.clone()),
	];
	rsx! {
		for (label, value) in facts {
			if !value.is_empty() {
				div { key: "{label}", class: "editor-field",
					span { class: "editor-label", "{label}" }
					p { class: "editor-value", "{value}" }
				}
			}
		}
		for (list, label, _, _, _) in LISTS {
			if !open.edit.entries(list).is_empty() {
				div { key: "{label}", class: "editor-field",
					span { class: "editor-label", "{label}" }
					for (index, entry) in open.edit.entries(list).iter().enumerate() {
						p { key: "{index}", class: "editor-value", "{entry.value}" }
					}
				}
			}
		}
		p { class: "hint",
			"Your organization manages this entry, so it can't be edited here. Add it to your contacts to keep your own copy."
		}
		match saved {
			Some(target) => rsx! {
				button {
					class: "btn primary",
					onclick: move |_| store.write().go(Next::Open(target.clone())),
					"Open in your contacts"
				}
			},
			None => rsx! {
				button {
					class: "btn primary",
					disabled: open.saving,
					onclick: move |_| {
						let call = store.write().begin_add();
						send(store, settings.clone(), call);
					},
					if open.saving { "Adding…" } else { "Add to your contacts" }
				}
			},
		}
	}
}

#[component]
fn Footer(store: Signal<Store>, settings: CalendarSettings, open: Opened) -> Element {
	if let Some(next) = &open.pending {
		let leaving = match next {
			Next::Close => "Close without saving your changes?",
			Next::Open(_) => "Leave this contact without saving your changes?",
		};
		return rsx! {
			div { class: "editor-confirm people-footer", role: "alert",
				p { "{leaving}" }
				div { class: "editor-row",
					button {
						class: "btn primary editor-grow",
						onclick: move |_| store.write().keep_editing(),
						"Keep editing"
					}
					button {
						class: "btn danger editor-grow",
						onclick: move |_| store.write().discard(),
						"Discard"
					}
				}
			}
		};
	}
	if open.edit.read_only {
		return rsx! {};
	}
	let dirty = open.edit.dirty();
	let ready = dirty && !open.saving;
	let label = if open.saving {
		"Saving…"
	} else if open.saved && !dirty {
		"Saved"
	} else {
		"Save"
	};
	rsx! {
		div { class: "editor-row people-footer",
			button {
				class: "btn primary editor-grow",
				disabled: !ready,
				onclick: move |_| {
					let call = store.write().begin_save();
					send(store, settings.clone(), call);
				},
				"{label}"
			}
			button {
				class: "btn editor-grow",
				disabled: !ready,
				onclick: move |_| store.write().revert(),
				"Undo changes"
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use serde_json::json;

	fn group() -> Group {
		Group {
			resource: "contactGroups/1".to_owned(),
			name: "Family".to_owned(),
		}
	}

	fn contact(resource: &str, name: &str, notes: &str, groups: &[Group]) -> Contact {
		Contact::parse(
			json!({
				"resourceName": resource,
				"etag": "e1",
				"names": [{ "displayName": name, "givenName": name }],
				"emailAddresses": [{ "value": format!("{name}@example.org"), "type": "work" }],
				"biographies": [{ "value": notes }],
				"memberships": groups.iter().map(|group| json!({
					"contactGroupMembership": { "contactGroupResourceName": group.resource }
				})).collect::<Vec<_>>(),
			}),
			groups,
		)
		.unwrap()
	}

	fn store() -> Store {
		let mut book = Book::idle(Login::primary("me@csumb.edu"));
		book.groups = vec![group()];
		book.contacts = vec![
			contact("people/1", "Ada", "Likes tea", &[group()]),
			contact("people/2", "Grace", "", &[]),
		];
		book.directory = vec![contact("people/3", "Hedy", "", &[])];
		Store {
			books: vec![book],
			..Store::default()
		}
	}

	fn target(resource: &str, directory: bool) -> Target {
		Target {
			account: "me@csumb.edu".to_owned(),
			resource: resource.to_owned(),
			directory,
		}
	}

	#[derive(Props, Clone)]
	struct HarnessProps {
		setup: fn(&mut Store),
	}

	impl PartialEq for HarnessProps {
		fn eq(&self, _: &Self) -> bool {
			false
		}
	}

	#[allow(non_snake_case)]
	fn Harness(props: HarnessProps) -> Element {
		let store = use_signal(|| {
			let mut store = store();
			(props.setup)(&mut store);
			store
		});
		let connecting = use_signal(|| false);
		rsx! {
			PeopleSheet {
				store,
				settings: CalendarSettings::default(),
				connecting,
				on_reconnect: |_| {},
				on_close: |_| {},
			}
		}
	}

	fn render(setup: fn(&mut Store)) -> String {
		let mut dom = VirtualDom::new_with_props(Harness, HarnessProps { setup });
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[track_caller]
	fn assert_shows(html: &str, wanted: &[&str]) {
		for text in wanted {
			assert!(html.contains(text), "missing {text:?} in {html}");
		}
	}

	#[test]
	fn every_contact_sits_under_its_account() {
		let html = render(|_| {});
		assert_shows(
			&html,
			&[
				"People",
				"me@csumb.edu",
				"Ada",
				"Family",
				"Grace",
				"Search names, notes and labels",
			],
		);
		assert_shows(&html, &["Type to also search the csumb.edu directory"]);
		assert!(!html.contains("Hedy"), "the directory waits for a search");
	}

	#[test]
	fn a_search_narrows_to_notes_and_shows_the_directory() {
		let html = render(|store| {
			store.set_query("tea".to_owned());
			// The directory answer, as Google would send it.
			store.books[0].directory = vec![contact("people/3", "Hedy", "", &[])];
		});
		assert_shows(
			&html,
			&["Ada", "Notes: Likes tea", "csumb.edu directory", "Hedy"],
		);
		assert!(!html.contains("Grace"));
	}

	#[test]
	fn an_open_contact_offers_every_field() {
		let html = render(|store| store.go(Next::Open(target("people/1", false))));
		assert_shows(
			&html,
			&[
				"Saved in me@csumb.edu",
				"First name",
				"Last name",
				"Company",
				"Job title",
				"Add email",
				"Add phone",
				"Add address",
				"Add website",
				"Labels",
				"New label",
				"Notes",
				"Likes tea",
				"Save",
				"Undo changes",
				"‹ People",
			],
		);
		assert_shows(&html, &[r#"data-open="true""#]);
	}

	#[test]
	fn a_directory_entry_offers_a_copy_rather_than_an_editor() {
		let html = render(|store| store.go(Next::Open(target("people/3", true))));
		assert_shows(
			&html,
			&["In the directory of me@csumb.edu", "Add to your contacts"],
		);
		assert!(!html.contains("First name"));
		assert!(!html.contains("Undo changes"));
	}

	#[test]
	fn a_problem_offers_signing_in_again() {
		let html = render(|store| {
			store.books[0].problem = Some(Problem::sign_in("Google sign-in expired"));
		});
		assert_shows(
			&html,
			&["Google sign-in expired", "Sign in to Google again"],
		);
	}

	#[test]
	fn with_no_account_the_way_forward_is_settings() {
		let html = render(|store| store.books.clear());
		assert_shows(&html, &["Connect your account in Settings"]);
	}

	#[test]
	fn leaving_with_changes_asks_first() {
		let mut store = store();
		store.go(Next::Open(target("people/1", false)));
		store.change(Change::Field(Field::GivenName, "Augusta".to_owned()));
		store.go(Next::Open(target("people/2", false)));
		let open = store.open.as_ref().unwrap();
		assert_eq!(open.edit.contact.resource, "people/1");
		assert!(open.pending.is_some());
		store.discard();
		assert_eq!(
			store.open.as_ref().unwrap().edit.contact.resource,
			"people/2"
		);

		store.change(Change::Notes("New".to_owned()));
		store.go(Next::Close);
		store.keep_editing();
		assert!(store.open.as_ref().unwrap().pending.is_none());
		store.revert();
		assert!(!store.open.as_ref().unwrap().edit.dirty());
		store.go(Next::Close);
		assert!(store.open.is_none());
	}

	#[test]
	fn the_leaving_question_is_shown() {
		let html = render(|store| {
			store.go(Next::Open(target("people/1", false)));
			store.change(Change::Field(Field::Title, "Countess".to_owned()));
			store.go(Next::Close);
		});
		assert_shows(
			&html,
			&[
				"Close without saving your changes?",
				"Keep editing",
				"Discard",
			],
		);
	}

	#[test]
	fn a_save_sends_only_what_changed_and_takes_googles_answer() {
		let mut store = store();
		store.go(Next::Open(target("people/1", false)));
		store.change(Change::Notes("Likes coffee".to_owned()));
		let Some(Call::Save { fields, body, .. }) = store.begin_save() else {
			panic!("a changed contact is saved");
		};
		assert_eq!(fields, ["biographies"]);
		assert_eq!(body["biographies"][0]["value"], "Likes coffee");
		assert!(store.begin_save().is_none(), "one save at a time");

		let mut returned = store.open.as_ref().unwrap().edit.contact.person().clone();
		returned["biographies"] = json!([{ "value": "Likes coffee" }]);
		returned["etag"] = json!("e2");
		assert!(store.stored("me@csumb.edu", Ok(returned)));
		let open = store.open.as_ref().unwrap();
		assert!(open.saved && !open.saving && !open.edit.dirty());
		assert_eq!(store.books[0].contacts[0].notes, "Likes coffee");
	}

	#[test]
	fn a_failed_save_keeps_the_edit() {
		let mut store = store();
		store.go(Next::Open(target("people/1", false)));
		store.change(Change::Field(Field::Company, "Analytical".to_owned()));
		assert!(store.begin_save().is_some());
		assert!(!store.stored("me@csumb.edu", Err(Problem::other("offline"))));
		let open = store.open.as_ref().unwrap();
		assert!(open.edit.dirty() && !open.saving);
		assert_eq!(open.problem.as_ref().unwrap().message, "offline");
	}

	#[test]
	fn a_typed_label_is_chosen_if_it_exists_and_created_if_not() {
		let mut store = store();
		store.go(Next::Open(target("people/2", false)));
		store.change(Change::Field(Field::NewLabel, "family".to_owned()));
		assert!(store.begin_label().is_none());
		assert_eq!(
			store.open.as_ref().unwrap().edit.labels,
			["contactGroups/1"]
		);

		store.change(Change::Field(Field::NewLabel, "Work".to_owned()));
		assert!(matches!(store.begin_label(), Some(Call::Label { name, .. }) if name == "Work"));
		let work = Group {
			resource: "contactGroups/2".to_owned(),
			name: "Work".to_owned(),
		};
		assert!(store.label_created("me@csumb.edu", Ok(work)));
		let open = store.open.as_ref().unwrap();
		assert_eq!(open.edit.labels, ["contactGroups/1", "contactGroups/2"]);
		assert!(open.edit.new_label.is_empty() && !open.creating_label);
		assert_eq!(store.books[0].groups.len(), 2);
	}

	#[test]
	fn a_directory_answer_from_an_old_search_is_ignored() {
		let mut store = store();
		let first = store.set_query("he".to_owned()).unwrap();
		let second = store.set_query("hed".to_owned()).unwrap();
		assert!(store.begin_search(first).is_empty());
		assert_eq!(store.begin_search(second).len(), 1);
		let hedy = json!({ "resourceName": "people/3", "names": [{ "displayName": "Hedy" }] });
		assert!(!store.found(first, "me@csumb.edu", Ok(Directory::Found(vec![]))));
		assert!(store.found(second, "me@csumb.edu", Ok(Directory::Found(vec![hedy]))));
		assert_eq!(store.books[0].directory[0].name, "Hedy");
		assert!(!store.books[0].searching);
		// Clearing the search empties the directory section.
		assert!(store.set_query("h".to_owned()).is_none());
		assert!(store.books[0].directory.is_empty());
	}

	#[test]
	fn signing_in_keeps_the_cached_contacts_of_the_same_account() {
		let mut store = store();
		let generation = store.begin_refresh();
		// Offline, the account can't be named, but it is still the same one.
		let to_load = store.listed(generation, vec![Login::unidentified()]);
		assert_eq!(to_load.len(), 1);
		assert_eq!(store.books[0].contacts.len(), 2);
		assert!(store.books[0].loading);
		assert!(!store.loaded(generation, "me@csumb.edu", Err(Problem::other("offline"))));
		assert_eq!(store.books[0].contacts.len(), 2);
		assert!(!store.books[0].loading);

		let generation = store.begin_refresh();
		assert!(store.listed(generation, Vec::new()).is_empty());
		assert!(store.books.is_empty());
	}
}
