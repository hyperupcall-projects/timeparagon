//! The Google account the calendar talks to.
//!
//! Every call to Google needs an access token, and getting one may mean
//! refreshing it with the OAuth client's ID and secret. Those used to be passed
//! by hand through every layer, and each call loaded and refreshed the token on
//! its own, so two threads finding it expired at once both refreshed it.
//! [`GoogleClient`] carries the credentials, and refreshes one at a time.

use std::sync::Mutex;

use super::google::GoogleError;
use super::oauth;
use crate::config::CalendarSettings;

/// Held while a token is read and, if it has expired, refreshed and saved.
/// Process-wide: the token is shared by every client in the process.
static REFRESH: Mutex<()> = Mutex::new(());

/// The OAuth client this app signs in to Google with.
#[derive(Clone, PartialEq, Eq)]
pub struct GoogleClient {
    client_id: String,
    client_secret: String,
}

impl std::fmt::Debug for GoogleClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The secret stays out of logs.
        f.debug_struct("GoogleClient")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

impl GoogleClient {
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
        }
    }

    /// The client configured in the `[calendar]` settings.
    pub fn from_settings(settings: &CalendarSettings) -> Self {
        Self::new(&settings.client_id, &settings.client_secret)
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn client_secret(&self) -> &str {
        &self.client_secret
    }

    /// A usable access token for the connected account, refreshed first when
    /// it is about to expire. Blocking.
    pub fn access_token(&self) -> Result<String, GoogleError> {
        let _refreshing = REFRESH
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tokens = oauth::load_tokens()
            .ok_or_else(|| GoogleError::auth("Not connected to Google Calendar"))?;
        oauth::ensure_access(&self.client_id, &self.client_secret, tokens)
            .map(|tokens| tokens.access_token)
    }
}
