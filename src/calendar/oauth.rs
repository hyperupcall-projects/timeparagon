//! Google OAuth for a desktop app.
//!
//! Installed-app clients talk to Google with PKCE: a localhost redirect
//! catches the authorization code, then we exchange it for a refresh token
//! stored next to the config on desktop. Android stores encrypted tokens in
//! its no-backup directory using a key held by Android Keystore.

#[cfg(not(target_os = "android"))]
use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config;

#[cfg(target_os = "android")]
mod android_tokens;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
// Calendar is read-only for the calendar list, and read/write for events so
// the day grid can drag an event's time. Tasks is read/write so the details
// card can tick a task off. A token minted before events were writable lacks
// the `calendar.events` scope, so a write is refused until the user reconnects.
// Drive's `drive.appdata` is limited to the app's own hidden folder; it is how
// the config follows the user between devices (see `config_sync`), and it
// cannot see any of the user's other Drive files.
// Gmail is read-only: a link pasted into a new task is read for its subject, and
// the mail is never changed. Tokens minted before that scope was asked for get
// a "Reconnect Google" message from the Gmail integration.
const SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/calendar.calendars https://www.googleapis.com/auth/tasks https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/drive.appdata";

/// The one scope an extra Gmail account is asked for. Calendar and Tasks stay
/// with the main Google account; these accounts are only ever read for a
/// message's subject.
pub const GMAIL_SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";

/// How long we wait for the browser to come back with a code.
const AUTH_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds since the Unix epoch, when `access_token` stops working.
    pub expires_at: u64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct TokenError {
    #[serde(default)]
    error: String,
    #[serde(default)]
    error_description: String,
}

impl Tokens {
    pub fn access_valid(&self) -> bool {
        let now = now_unix();
        // Refresh a minute early so a request never races the expiry.
        self.expires_at > now + 60
    }
}

/// Desktop token file, also used to find legacy plaintext tokens on Android.
/// Android's current encrypted file is resolved through getNoBackupFilesDir().
pub fn token_path() -> Option<PathBuf> {
    Some(
        config::project_dirs()?
            .config_dir()
            .join("google-token.toml"),
    )
}

#[cfg(not(target_os = "android"))]
pub fn load_tokens() -> Option<Tokens> {
    let path = token_path()?;
    let text = fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

#[cfg(target_os = "android")]
pub fn load_tokens() -> Option<Tokens> {
    match android_tokens::load() {
        Ok(tokens) => tokens,
        Err(error) => {
            log::warn!("could not open protected Google tokens: {error}");
            None
        }
    }
}

#[cfg(target_os = "android")]
pub fn save_tokens(tokens: &Tokens) -> io::Result<()> {
    android_tokens::save(tokens)
}

#[cfg(not(target_os = "android"))]
pub fn save_tokens(tokens: &Tokens) -> io::Result<()> {
    let path = token_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no config directory available"))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = toml::to_string_pretty(tokens)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let temporary = path.with_extension("toml.tmp");
    let _ = fs::remove_file(&temporary);
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(contents.as_bytes())?;
    drop(file);
    fs::rename(&temporary, &path)
}

pub fn clear_tokens() {
    #[cfg(target_os = "android")]
    {
        if let Err(error) = android_tokens::clear() {
            log::warn!("could not remove protected Google tokens: {error}");
        }
    }
    #[cfg(not(target_os = "android"))]
    if let Some(path) = token_path() {
        let _ = fs::remove_file(path);
    }
}

/// Runs the browser-based sign-in and returns tokens on success.
///
/// Blocking: the local socket waits for Google to redirect back. Call this
/// from [`crate::blocking::run`], not from the UI thread.
pub fn authorize(client_id: &str, client_secret: &str) -> Result<Tokens, String> {
    authorize_for(client_id, client_secret, SCOPE, "consent")
}

/// Like [`authorize`], for any scope. `prompt` is Google's `prompt` parameter:
/// `select_account consent` makes the browser ask which account to use, which
/// is what lets a second account be added while the first is signed in.
pub fn authorize_for(
    client_id: &str,
    client_secret: &str,
    scope: &str,
    prompt: &str,
) -> Result<Tokens, String> {
    let client_id = client_id.trim();
    if client_id.is_empty() {
        return Err("add a Google OAuth client ID in Settings before connecting".to_owned());
    }

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(false)
        .map_err(|error| error.to_string())?;

    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let redirect = format!("http://127.0.0.1:{port}");
    let verifier = random_token(32)?;
    let challenge = code_challenge(&verifier);
    let state = random_token(16)?;

    let url = format!(
        "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&state={}&access_type=offline&prompt={}",
        urlencoding::encode(client_id),
        urlencoding::encode(&redirect),
        urlencoding::encode(scope),
        urlencoding::encode(&challenge),
        urlencoding::encode(&state),
        urlencoding::encode(prompt),
    );

    open_in_browser(&url)?;

    // Any app on the device can reach a loopback port, so keep listening until
    // a request carrying our `state` arrives instead of trusting the first one.
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + AUTH_TIMEOUT;
    let code = loop {
        let mut stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("timed out waiting for Google sign-in".to_owned());
                }
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(error) => return Err(format!("waiting for Google sign-in: {error}")),
        };
        if stream.set_nonblocking(false).is_err()
            || stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .is_err()
        {
            continue;
        }

        let mut buffer = [0u8; 4096];
        let Ok(read) = stream.read(&mut buffer) else {
            continue;
        };
        let request = String::from_utf8_lossy(&buffer[..read]);
        match parse_redirect(&request, &state) {
            Redirect::Unrelated => {
                let _ = stream.write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
            Redirect::Done(result) => {
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n\
                     <!doctype html><title>TimeTracker</title><p>You can close this tab and return to TimeTracker.</p>",
                );
                break result?;
            }
        }
    };

    exchange_code(client_id, client_secret, &redirect, &code, &verifier)
}

pub fn refresh(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<Tokens, String> {
    let client_id = client_id.trim();
    if client_id.is_empty() {
        return Err("Google OAuth client ID is missing".to_owned());
    }

    let body = form(&[
        ("client_id", client_id),
        ("client_secret", client_secret.trim()),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ]);

    let response = post_token(&body)?;
    Ok(Tokens {
        access_token: response.access_token,
        refresh_token: response
            .refresh_token
            .unwrap_or_else(|| refresh_token.to_owned()),
        expires_at: now_unix().saturating_add(response.expires_in.max(60)),
    })
}

/// Makes sure `tokens` has a usable access token, refreshing if needed.
pub fn ensure_access(
    client_id: &str,
    client_secret: &str,
    tokens: Tokens,
) -> Result<Tokens, String> {
    if tokens.access_valid() {
        return Ok(tokens);
    }

    let refreshed = refresh(client_id, client_secret, &tokens.refresh_token)?;
    if let Err(error) = save_tokens(&refreshed) {
        log::warn!("could not save refreshed Google tokens: {error}");
    }
    Ok(refreshed)
}

pub fn open_in_browser(url: &str) -> Result<(), String> {
    // Android has no xdg-open; use the platform Intent API instead.
    #[cfg(target_os = "android")]
    {
        webbrowser::open(url)
            .map(|_| ())
            .map_err(|error| format!("could not open a browser: {error}"))
    }
    // xdg-open is the right tool on this desktop; we do not wait for the
    // browser, only for it to be launched.
    #[cfg(not(target_os = "android"))]
    {
        match std::process::Command::new("xdg-open").arg(url).spawn() {
            Ok(_) => Ok(()),
            Err(error) => Err(format!("could not open a browser: {error}")),
        }
    }
}

fn exchange_code(
    client_id: &str,
    client_secret: &str,
    redirect: &str,
    code: &str,
    verifier: &str,
) -> Result<Tokens, String> {
    let body = form(&[
        ("client_id", client_id),
        ("client_secret", client_secret.trim()),
        ("code", code),
        ("code_verifier", verifier),
        ("redirect_uri", redirect),
        ("grant_type", "authorization_code"),
    ]);

    let response = post_token(&body)?;
    let refresh_token = response.refresh_token.ok_or_else(|| {
        "Google did not return a refresh token; try disconnecting and connecting again".to_owned()
    })?;

    Ok(Tokens {
        access_token: response.access_token,
        refresh_token,
        expires_at: now_unix().saturating_add(response.expires_in.max(60)),
    })
}

fn post_token(body: &str) -> Result<TokenResponse, String> {
    let result = ureq::post(TOKEN_URL)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(body);

    match result {
        Ok(response) => response
            .into_json()
            .map_err(|error| format!("Google token response: {error}")),
        Err(ureq::Error::Status(code, response)) => {
            let detail = response
                .into_json::<TokenError>()
                .map(|parsed| {
                    if parsed.error_description.is_empty() {
                        parsed.error
                    } else {
                        parsed.error_description
                    }
                })
                .unwrap_or_else(|_| format!("HTTP {code}"));
            Err(format!("Google sign-in failed: {detail}"))
        }
        Err(error) => Err(format!("Google sign-in failed: {error}")),
    }
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut body = String::new();
    for (key, value) in pairs {
        if value.is_empty() {
            continue;
        }
        if !body.is_empty() {
            body.push('&');
        }
        body.push_str(&urlencoding::encode(key));
        body.push('=');
        body.push_str(&urlencoding::encode(value));
    }
    body
}

/// What a request to the loopback listener turned out to be.
enum Redirect {
    /// Not Google's redirect for this sign-in: wrong or missing `state`.
    Unrelated,
    /// The redirect for this sign-in: the code, or why there is none.
    Done(Result<String, String>),
}

fn parse_redirect(request: &str, expected_state: &str) -> Redirect {
    let line = request.lines().next().unwrap_or("");
    let path = line.split_whitespace().nth(1).unwrap_or("");
    let query = path.split_once('?').map(|(_, query)| query).unwrap_or("");

    let mut code = None;
    let mut error = None;
    let mut state = None;

    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = urlencoding::decode(value)
            .unwrap_or_else(|_| value.into())
            .into_owned();

        match key {
            "code" => code = Some(value),
            "error" => error = Some(value),
            "state" => state = Some(value),
            _ => {}
        }
    }

    if state.as_deref() != Some(expected_state) {
        return Redirect::Unrelated;
    }

    if let Some(error) = error {
        return Redirect::Done(Err(format!("Google sign-in was cancelled ({error})")));
    }

    Redirect::Done(
        code.ok_or_else(|| "Google sign-in did not return an authorization code".to_owned()),
    )
}

/// Random URL-safe text. Fails rather than falling back to anything
/// predictable, since PKCE and `state` are only worth having if unguessable.
fn random_token(bytes: usize) -> Result<String, String> {
    random_token_with(bytes, getrandom::fill)
}

fn random_token_with(
    bytes: usize,
    fill: impl FnOnce(&mut [u8]) -> Result<(), getrandom::Error>,
) -> Result<String, String> {
    let mut buffer = vec![0u8; bytes];
    fill(&mut buffer)
        .map_err(|error| format!("could not gather randomness for sign-in: {error}"))?;
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

fn code_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn done(redirect: Redirect) -> Result<String, String> {
        match redirect {
            Redirect::Done(result) => result,
            Redirect::Unrelated => panic!("expected the redirect to be accepted"),
        }
    }

    #[test]
    fn a_successful_redirect_yields_the_code() {
        let request =
            "GET /?code=abc%2Fde&state=s1&scope=calendar HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(done(parse_redirect(request, "s1")).unwrap(), "abc/de");
    }

    #[test]
    fn a_denied_redirect_is_an_error() {
        let request = "GET /?error=access_denied&state=s1 HTTP/1.1\r\n\r\n";
        assert!(
            done(parse_redirect(request, "s1"))
                .unwrap_err()
                .contains("access_denied")
        );
    }

    #[test]
    fn a_request_without_the_expected_state_is_ignored() {
        for request in [
            "GET /?code=abc HTTP/1.1\r\n\r\n",
            "GET /?code=abc&state=other HTTP/1.1\r\n\r\n",
            "GET /?error=x&state=other HTTP/1.1\r\n\r\n",
            "GET / HTTP/1.1\r\n\r\n",
        ] {
            assert!(matches!(parse_redirect(request, "s1"), Redirect::Unrelated));
        }
    }

    #[test]
    fn the_pkce_challenge_is_s256() {
        // Vector from RFC 7636 appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            code_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn randomness_failure_aborts_instead_of_producing_a_verifier() {
        assert!(random_token_with(32, |_| Err(getrandom::Error::UNSUPPORTED)).is_err());
    }

    #[test]
    fn pkce_verifiers_have_32_random_bytes_and_url_safe_encoding() {
        let verifier = random_token(32).unwrap();
        assert_eq!(verifier.len(), 43);
        assert_eq!(URL_SAFE_NO_PAD.decode(verifier).unwrap().len(), 32);
    }

    #[test]
    fn an_unexpired_token_is_not_refreshed() {
        let tokens = Tokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: now_unix() + 3_600,
        };
        assert!(tokens.access_valid());
    }
}
