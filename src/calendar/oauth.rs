//! Google OAuth for a desktop app.
//!
//! Installed-app clients talk to Google with PKCE: a localhost redirect
//! catches the authorization code, then we exchange it for a refresh token
//! that lives next to the rest of the config. The refresh token is the only
//! secret we keep; the access token is derived from it as needed.

use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
// Calendar is read-only for the calendar list, and read/write for events so
// the day grid can drag an event's time. Tasks is read/write so the details
// card can tick a task off. A token minted before events were writable lacks
// the `calendar.events` scope, so a write is refused until the user reconnects.
const SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/calendar.calendars https://www.googleapis.com/auth/tasks";

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

pub fn token_path() -> Option<PathBuf> {
    Some(config::project_dirs()?.config_dir().join("google-token.toml"))
}

pub fn load_tokens() -> Option<Tokens> {
    let path = token_path()?;
    let text = fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

pub fn save_tokens(tokens: &Tokens) -> io::Result<()> {
    let path = token_path().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "no config directory available")
    })?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let contents = toml::to_string_pretty(tokens)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let temporary = path.with_extension("toml.tmp");
    fs::write(&temporary, contents.as_bytes())?;
    fs::rename(&temporary, &path)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

pub fn clear_tokens() {
    if let Some(path) = token_path() {
        let _ = fs::remove_file(path);
    }
}

/// Runs the browser-based sign-in and returns tokens on success.
///
/// Blocking: the local socket waits for Google to redirect back. Call this
/// from [`crate::blocking::run`], not from the UI thread.
pub fn authorize(client_id: &str, client_secret: &str) -> Result<Tokens, String> {
    let client_id = client_id.trim();
    if client_id.is_empty() {
        return Err(
            "add a Google OAuth client ID in Settings before connecting".to_owned(),
        );
    }

    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(false)
        .map_err(|error| error.to_string())?;

    let port = listener.local_addr().map_err(|error| error.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}");
    let verifier = code_verifier();
    let challenge = code_challenge(&verifier);

    let url = format!(
        "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent",
        urlencoding::encode(client_id),
        urlencoding::encode(&redirect),
        urlencoding::encode(SCOPE),
        urlencoding::encode(&challenge),
    );

    open_in_browser(&url)?;

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(listener.accept());
    });
    let (mut stream, _) = rx
        .recv_timeout(AUTH_TIMEOUT)
        .map_err(|_| "timed out waiting for Google sign-in".to_owned())?
        .map_err(|error| format!("waiting for Google sign-in: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;

    let mut buffer = [0u8; 4096];
    let read = stream.read(&mut buffer).map_err(|error| error.to_string())?;
    let request = String::from_utf8_lossy(&buffer[..read]);
    let code = parse_redirect(&request)?;

    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n\
         <!doctype html><title>TimeTracker</title><p>Signed in. You can close this tab and return to TimeTracker.</p>",
    );

    exchange_code(client_id, client_secret, &redirect, &code, &verifier)
}

pub fn refresh(client_id: &str, client_secret: &str, refresh_token: &str) -> Result<Tokens, String> {
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
pub fn ensure_access(client_id: &str, client_secret: &str, tokens: Tokens) -> Result<Tokens, String> {
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
    // xdg-open is the right tool on this desktop; we do not wait for the
    // browser, only for it to be launched.
    match std::process::Command::new("xdg-open").arg(url).spawn() {
        Ok(_) => Ok(()),
        Err(error) => Err(format!("could not open a browser: {error}")),
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

fn parse_redirect(request: &str) -> Result<String, String> {
    let line = request.lines().next().unwrap_or("");
    let path = line.split_whitespace().nth(1).unwrap_or("");
    let query = path.split_once('?').map(|(_, query)| query).unwrap_or("");

    let mut code = None;
    let mut error = None;

    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = urlencoding::decode(value)
            .unwrap_or_else(|_| value.into())
            .into_owned();

        match key {
            "code" => code = Some(value),
            "error" => error = Some(value),
            _ => {}
        }
    }

    if let Some(error) = error {
        return Err(format!("Google sign-in was cancelled ({error})"));
    }

    code.ok_or_else(|| "Google sign-in did not return an authorization code".to_owned())
}

fn code_verifier() -> String {
    let mut bytes = [0u8; 32];
    if getrandom::fill(&mut bytes).is_err() {
        // A weak verifier is still better than aborting sign-in; Google
        // rejects reused verifiers, not entropy quality, at this layer.
        let now = now_unix().to_le_bytes();
        for (index, slot) in bytes.iter_mut().enumerate() {
            *slot = now[index % now.len()];
        }
    }
    URL_SAFE_NO_PAD.encode(bytes)
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

    #[test]
    fn a_successful_redirect_yields_the_code() {
        let request = "GET /?code=abc%2Fde&scope=calendar HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(parse_redirect(request).unwrap(), "abc/de");
    }

    #[test]
    fn a_denied_redirect_is_an_error() {
        let request = "GET /?error=access_denied HTTP/1.1\r\n\r\n";
        assert!(parse_redirect(request).unwrap_err().contains("access_denied"));
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
    fn an_unexpired_token_is_not_refreshed() {
        let tokens = Tokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            expires_at: now_unix() + 3_600,
        };
        assert!(tokens.access_valid());
    }
}
