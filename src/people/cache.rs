//! Every contact Google has sent, kept in the database exactly as it was sent.
//!
//! The document opens the People screen straight away and keeps it useful when
//! Google can't be reached. It holds personal details, and the database is
//! readable by this user only. An account's entries leave it when the account is
//! signed out.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Book, Contact, Group, parse_all};
use crate::integrations::accounts::Login;

/// Hands out each snapshot's place in line. Snapshots are written on scratch
/// threads that may finish in any order, and the newest one has to win.
static NEXT: AtomicU64 = AtomicU64::new(1);
/// The newest snapshot written to each database.
static WRITTEN: Mutex<Option<HashMap<PathBuf, u64>>> = Mutex::new(None);

#[derive(Serialize, Deserialize)]
struct File {
    accounts: Vec<Account>,
}

#[derive(Serialize, Deserialize)]
struct Account {
    login: Login,
    groups: Vec<Group>,
    contacts: Vec<Value>,
    /// Everyone a directory search has returned.
    #[serde(default)]
    directory: Vec<Value>,
    #[serde(default)]
    no_directory: bool,
}

/// The books as they stood when this was made, waiting to be written.
pub(super) struct Snapshot {
    order: u64,
    json: String,
}

pub(super) fn snapshot(books: &[Book]) -> Result<Snapshot, serde_json::Error> {
    let people = |contacts: &[Contact]| contacts.iter().map(|c| c.person.clone()).collect();
    let file = File {
        accounts: books
            .iter()
            .map(|book| Account {
                login: book.login.clone(),
                groups: book.groups.clone(),
                contacts: people(&book.contacts),
                directory: people(&book.directory_seen),
                no_directory: book.no_directory,
            })
            .collect(),
    };
    Ok(Snapshot {
        order: NEXT.fetch_add(1, Ordering::Relaxed),
        json: serde_json::to_string(&file)?,
    })
}

impl Snapshot {
    /// Writes the document, unless a newer snapshot has already been written to it.
    pub(super) fn write(self, path: &Path) -> io::Result<()> {
        let mut written = WRITTEN.lock().unwrap_or_else(|p| p.into_inner());
        let written = written.get_or_insert_with(HashMap::new);
        if written.get(path).is_some_and(|newest| *newest > self.order) {
            return Ok(());
        }
        crate::task_db::write_document_json(path, crate::task_db::keys::CONTACTS, &self.json)?;
        written.insert(path.to_owned(), self.order);
        Ok(())
    }
}

/// The books from the last run. Nothing saved, or something unreadable, means
/// starting empty: everything in it can be fetched again.
pub(super) fn load(path: &Path) -> Vec<Book> {
    match crate::task_db::read_document::<File>(path, crate::task_db::keys::CONTACTS) {
        Ok(file) => file
            .map(|file| file.accounts.into_iter().map(book_from).collect())
            .unwrap_or_default(),
        Err(error) => {
            log::warn!("could not read the contacts cache: {error}");
            Vec::new()
        }
    }
}

fn book_from(account: Account) -> Book {
    let mut book = Book::new(account.login);
    book.loading = false;
    book.contacts = parse_all(account.contacts, &account.groups);
    book.contacts.sort_by_key(Contact::sort_key);
    book.directory_seen = parse_all(account.directory, &[]);
    book.groups = account.groups;
    book.no_directory = account.no_directory;
    book
}
