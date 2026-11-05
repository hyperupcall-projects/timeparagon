//! Google accounts connected for the Gmail integration and the People screen.
//!
//! A link pasted into a new task can belong to any mailbox the person reads,
//! so any number of Google accounts can be connected here. The Gmail
//! integration asks every one of them about the link and keeps the one that
//! knows it (see [`super::gmail`]).
//!
//! These accounts are separate from the Google sign-in that feeds the
//! calendar: they are asked for read-only Gmail access and for contacts, and
//! their tokens live in their own file next to the config. The calendar's own
//! account is still tried as well, since it was already granted Gmail access.
//! They share the OAuth client from Settings.
//!
//! Tokens are stored on desktop only; the Android build has no encrypted
//! store for them yet, so there it refuses to hold any rather than write
//! them in the clear.

use std::io;
use std::sync::Mutex;

use iced::Task;
use serde::{Deserialize, Serialize};

use crate::app::{Message, State};
use crate::blocking;
use crate::calendar::api;
use crate::calendar::oauth::{self, Tokens};
use crate::config::CalendarSettings;

const PROFILE: &str = "https://gmail.googleapis.com/gmail/v1/users/me/profile";

/// What the account is called when the calendar's own sign-in is used.
const PRIMARY_LABEL: &str = "Google Calendar account";

/// Serializes reads and rewrites of the token file. Two lookups may refresh
/// different accounts at once; without this one would overwrite the other.
static STORE: Mutex<()> = Mutex::new(());

/// One connected account and the means to act as it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub email: String,
    pub tokens: Tokens,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    accounts: Vec<Account>,
}

/// An account a link can be tried against. Holds no credentials, only which
/// account to find them for.
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
    /// A connected Gmail account, by address.
    Extra(String),
}

impl Login {
    /// Whether this is the calendar's sign-in, which is renewed from a
    /// different place than a connected account.
    pub fn is_primary(&self) -> bool {
        self.which == Which::Primary
    }

    /// The calendar's sign-in, when its address could not be looked up.
    pub fn is_unidentified(&self) -> bool {
        self.is_primary() && self.label == PRIMARY_LABEL
    }

    #[cfg(test)]
    pub fn primary(label: &str) -> Self {
        Self {
            label: label.to_owned(),
            which: Which::Primary,
        }
    }

    #[cfg(test)]
    pub fn extra(email: &str) -> Self {
        Self {
            label: email.to_owned(),
            which: Which::Extra(email.to_owned()),
        }
    }
}

/// Every account a Gmail link may belong to, in the order they are tried: the
/// calendar's account first, if it is signed in, then the connected ones.
pub fn logins() -> Vec<Login> {
    let primary = oauth::load_tokens().map(|_| Login {
        label: PRIMARY_LABEL.to_owned(),
        which: Which::Primary,
    });
    primary
        .into_iter()
        .chain(emails().into_iter().map(|email| Login {
            label: email.clone(),
            which: Which::Extra(email),
        }))
        .collect()
}

/// Every signed-in account once, each called by its address. The calendar's
/// account is looked up, and a connected account that turns out to be the same
/// one is dropped. Blocking.
pub fn identified(settings: &CalendarSettings) -> Vec<Login> {
    let mut logins = logins();
    if let Some(primary) = logins.iter_mut().find(|login| login.is_primary())
        && let Ok(email) = token(settings, primary).and_then(|token| profile_email(&token))
    {
        primary.label = email;
    }
    let primary = logins
        .iter()
        .find(|login| login.is_primary())
        .map(|login| login.label.clone());
    logins.retain(|login| {
        login.is_primary()
            || !primary
                .as_ref()
                .is_some_and(|email| same(email, &login.label))
    });
    logins
}

/// A fresh access token for `login`, refreshed if the stored one has expired.
/// Blocking.
pub fn token(settings: &CalendarSettings, login: &Login) -> Result<String, String> {
    let email = match &login.which {
        Which::Primary => return crate::timeblocking::access(settings),
        Which::Extra(email) => email,
    };
    let stored = read()
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|account| same(&account.email, email))
        .ok_or("This account was removed in Settings")?;
    if stored.tokens.access_valid() {
        return Ok(stored.tokens.access_token);
    }
    let refreshed = oauth::refresh(
        &settings.client_id,
        &settings.client_secret,
        &stored.tokens.refresh_token,
    )
    .map_err(|error| format!("{error}. Remove the account in Settings and add it again"))?;
    let account = Account {
        email: stored.email,
        tokens: refreshed,
    };
    let access = account.tokens.access_token.clone();
    let _guard = lock();
    let mut accounts = read().map_err(|error| error.to_string())?;
    // Removed while the refresh was under way: the answer is still good for
    // this one request, but it must not bring the account back.
    if let Some(slot) = accounts
        .iter_mut()
        .find(|saved| same(&saved.email, &account.email))
    {
        *slot = account;
        if let Err(error) = write(&accounts) {
            log::warn!("could not save refreshed Gmail tokens: {error}");
        }
    }
    Ok(access)
}

/// Signs in to another Google account and remembers it. Blocking: the browser
/// has to come back. Returns the address that was connected.
pub fn connect(client_id: &str, client_secret: &str) -> Result<String, String> {
    let tokens = oauth::authorize_for(
        client_id,
        client_secret,
        oauth::EXTRA_ACCOUNT_SCOPE,
        "select_account consent",
    )?;
    let email = profile_email(&tokens.access_token)?;
    let _guard = lock();
    let mut accounts = read().map_err(|error| error.to_string())?;
    upsert(
        &mut accounts,
        Account {
            email: email.clone(),
            tokens,
        },
    );
    write(&accounts).map_err(|error| format!("could not save the account: {error}"))?;
    Ok(email)
}

/// Forgets a connected account.
pub fn remove(email: &str) -> io::Result<()> {
    let _guard = lock();
    let mut accounts = read()?;
    accounts.retain(|account| !same(&account.email, email));
    write(&accounts)
}

/// The addresses of the connected accounts.
pub fn emails() -> Vec<String> {
    let _guard = lock();
    match read() {
        Ok(accounts) => accounts.into_iter().map(|account| account.email).collect(),
        Err(error) => {
            log::warn!("could not read the connected Gmail accounts: {error}");
            Vec::new()
        }
    }
}

/// Adds `account`, or replaces the one with the same address so connecting an
/// account again renews it instead of listing it twice.
fn upsert(accounts: &mut Vec<Account>, account: Account) {
    match accounts
        .iter_mut()
        .find(|saved| same(&saved.email, &account.email))
    {
        Some(saved) => *saved = account,
        None => accounts.push(account),
    }
}

/// Addresses are not case sensitive.
fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    STORE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The address a token belongs to, from Gmail's own profile call: every account
/// here has been granted read-only Gmail.
fn profile_email(access_token: &str) -> Result<String, String> {
    let result = api::agent()
        .get(PROFILE)
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("Accept", "application/json")
        .call();
    match result {
        Ok(response) => {
            let body: serde_json::Value = response
                .into_json()
                .map_err(|error| format!("Gmail response: {error}"))?;
            body["emailAddress"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "Gmail did not say which account this is".to_owned())
        }
        Err(ureq::Error::Status(403, _)) => {
            Err("Turn on the Gmail API for your Google Cloud project, then try again".to_owned())
        }
        Err(ureq::Error::Status(status, _)) => Err(format!("Gmail: HTTP {status}")),
        Err(error) => Err(format!("Gmail: {error}")),
    }
}

#[cfg(not(target_os = "android"))]
fn read() -> io::Result<Vec<Account>> {
    let Some(path) = crate::secrets::path() else {
        return Ok(Vec::new());
    };
    let file: Option<File> = crate::secrets::read(&path, crate::secrets::keys::GMAIL_ACCOUNTS)?;
    Ok(file.map(|file| file.accounts).unwrap_or_default())
}

#[cfg(not(target_os = "android"))]
fn write(accounts: &[Account]) -> io::Result<()> {
    let path = crate::secrets::path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no state directory available"))?;
    let key = crate::secrets::keys::GMAIL_ACCOUNTS;
    if accounts.is_empty() {
        return crate::secrets::remove(&path, key);
    }
    // Long-lived credentials for a mailbox, so readable by this user only.
    crate::secrets::write(
        &path,
        key,
        &File {
            accounts: accounts.to_vec(),
        },
    )
}

#[cfg(target_os = "android")]
fn read() -> io::Result<Vec<Account>> {
    Ok(Vec::new())
}

#[cfg(target_os = "android")]
fn write(_: &[Account]) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "extra Google accounts cannot be stored securely on Android yet",
    ))
}

/// What Settings shows about the connected accounts.
#[derive(Debug, Default)]
pub struct Accounts {
    pub emails: Vec<String>,
    /// The browser sign-in is under way.
    pub connecting: bool,
    pub error: Option<String>,
}

impl Accounts {
    pub fn load() -> Self {
        Self {
            emails: emails(),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
pub enum Action {
    /// Sign in to another Google account.
    Add,
    /// The sign-in finished: the address that was connected, or why not.
    Added(Result<String, String>),
    Remove(String),
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
    match action {
        Action::Add => {
            if state.accounts.connecting {
                return Task::none();
            }
            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            if client_id.trim().is_empty() {
                state.accounts.error =
                    Some("Add a Google OAuth client ID above before connecting".to_owned());
                return Task::none();
            }
            state.accounts.connecting = true;
            state.accounts.error = None;
            blocking::run(move || connect(&client_id, &client_secret))
                .map(|result| Message::Accounts(Action::Added(result)))
        }
        Action::Added(result) => {
            state.accounts.connecting = false;
            match result {
                Ok(_) => {
                    state.accounts.emails = emails();
                    state.accounts.error = None;
                    if state.screen == crate::app::Screen::People {
                        return crate::people::refresh(state);
                    }
                }
                Err(error) => {
                    log::warn!("Gmail account sign-in failed: {error}");
                    state.accounts.error = Some(error);
                }
            }
            Task::none()
        }
        Action::Remove(email) => {
            state.accounts.error = remove(&email)
                .err()
                .map(|error| format!("could not remove {email}: {error}"));
            state.accounts.emails = emails();
            if state.accounts.error.is_some() {
                return Task::none();
            }
            crate::people::forget_account(state, &email)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(email: &str, token: &str) -> Account {
        Account {
            email: email.to_owned(),
            tokens: Tokens {
                access_token: token.to_owned(),
                refresh_token: "refresh".to_owned(),
                expires_at: 1,
            },
        }
    }

    #[test]
    fn connecting_the_same_address_again_renews_it() {
        let mut accounts = vec![account("a@example.org", "old")];
        upsert(&mut accounts, account("b@example.org", "b"));
        upsert(&mut accounts, account("A@Example.org", "new"));
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].tokens.access_token, "new");
    }

    #[test]
    fn addresses_compare_without_regard_to_case() {
        assert!(same("Me@Example.org", " me@example.org"));
        assert!(!same("me@example.org", "you@example.org"));
    }

    #[test]
    fn accounts_survive_a_round_trip_through_the_file() {
        let file = File {
            accounts: vec![account("a@example.org", "x"), account("b@example.org", "y")],
        };
        let text = toml::to_string_pretty(&file).unwrap();
        let back: File = toml::from_str(&text).unwrap();
        assert_eq!(back.accounts, file.accounts);
        let empty: File = toml::from_str("").unwrap();
        assert!(empty.accounts.is_empty());
    }

    #[test]
    fn adding_needs_an_oauth_client_and_goes_no_further_without_one() {
        let mut state = State::blank();
        let _ = update(&mut state, Action::Add);
        assert!(!state.accounts.connecting);
        assert!(state.accounts.error.is_some());
    }
}
