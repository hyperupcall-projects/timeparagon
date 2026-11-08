//! What can go wrong talking to Google, decided once where the response is
//! still at hand.
//!
//! Text for the user is made from a [`GoogleError`] at the edge. Nothing past
//! this module should need to search an error message to find out what
//! happened.

use std::fmt;
use std::time::Duration;

use serde_json::Value;

/// The kinds of failure the app can tell apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// The token was rejected: sign-in has expired or been revoked.
    Auth,
    /// The token is good, but was not granted the scope this call needs.
    MissingScope,
    /// The API has not been turned on for the user's Cloud project.
    ApiDisabled,
    RateLimited {
        retry_after: Option<Duration>,
    },
    NotFound,
    /// Any other status Google answered with.
    Http,
    /// Google could not be reached, or its answer could not be read.
    Transport,
    /// Not from Google at all: a local check on the way to making the call.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleError {
    pub kind: Kind,
    /// The HTTP status, when Google answered.
    pub status: Option<u16>,
    /// Names the API, as in "Google Calendar". Empty for a bare message.
    pub context: String,
    /// Google's own message, or what went wrong getting it.
    pub detail: String,
}

impl GoogleError {
    /// The error Google's `status` and `detail` amount to.
    pub fn from_status(context: &str, status: u16, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        Self {
            kind: classify(status, &detail, None, None),
            status: Some(status),
            context: context.to_owned(),
            detail,
        }
    }

    /// The call never got an answer that could be read.
    /// No usable sign-in: not connected, or the stored one was refused. Fixed
    /// by connecting again.
    pub fn auth(detail: impl Into<String>) -> Self {
        Self {
            kind: Kind::Auth,
            status: None,
            context: String::new(),
            detail: detail.into(),
        }
    }

    pub fn transport(context: &str, error: impl fmt::Display) -> Self {
        Self {
            kind: Kind::Transport,
            status: None,
            context: context.to_owned(),
            detail: error.to_string(),
        }
    }
}

impl fmt::Display for GoogleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.context.is_empty() {
            f.write_str(&self.detail)
        } else {
            write!(f, "{}: {}", self.context, self.detail)
        }
    }
}

impl std::error::Error for GoogleError {}

impl From<GoogleError> for String {
    fn from(error: GoogleError) -> Self {
        error.to_string()
    }
}

impl From<String> for GoogleError {
    fn from(detail: String) -> Self {
        Self {
            kind: Kind::Other,
            status: None,
            context: String::new(),
            detail,
        }
    }
}

impl From<&str> for GoogleError {
    fn from(detail: &str) -> Self {
        detail.to_owned().into()
    }
}

/// A refusal from Google, read into its parts.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub status: u16,
    /// `error.message` from the JSON body, when there is one.
    pub message: Option<String>,
    /// The machine-readable `error.errors[0].reason`, when there is one.
    pub reason: Option<String>,
    pub retry_after: Option<Duration>,
    pub body: String,
}

impl Refusal {
    pub fn read(status: u16, response: ureq::Response) -> Self {
        let retry_after = response
            .header("Retry-After")
            .and_then(|value| value.trim().parse().ok())
            .map(Duration::from_secs);
        let body = response.into_string().unwrap_or_default();
        let json = serde_json::from_str::<Value>(&body).ok();
        let error = json.as_ref().map(|json| &json["error"]);
        let text = |value: &Value| value.as_str().map(str::to_owned);
        Self {
            status,
            message: error
                .and_then(|error| text(&error["message"]))
                .filter(|message| !message.is_empty()),
            reason: error.and_then(|error| text(&error["errors"][0]["reason"])),
            retry_after,
            body,
        }
    }

    /// Google's message, or the bare status when it gave none.
    pub fn detail(&self) -> String {
        self.message
            .clone()
            .unwrap_or_else(|| format!("HTTP {}", self.status))
    }

    pub fn into_error(self, context: &str) -> GoogleError {
        let detail = self.detail();
        GoogleError {
            kind: classify(
                self.status,
                &detail,
                self.reason.as_deref(),
                self.retry_after,
            ),
            status: Some(self.status),
            context: context.to_owned(),
            detail,
        }
    }
}

/// Turns the outcome of a request into the response or a typed error.
pub fn send(
    context: &str,
    result: Result<ureq::Response, ureq::Error>,
) -> Result<ureq::Response, GoogleError> {
    match result {
        Ok(response) => Ok(response),
        Err(ureq::Error::Status(status, response)) => {
            Err(Refusal::read(status, response).into_error(context))
        }
        Err(error) => Err(GoogleError::transport(context, error)),
    }
}

/// Google signals several of these only in the wording of its message.
fn classify(
    status: u16,
    detail: &str,
    reason: Option<&str>,
    retry_after: Option<Duration>,
) -> Kind {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("insufficient") || lower.contains("scope") {
        Kind::MissingScope
    } else if lower.contains("has not been used")
        || lower.contains("not been used")
        || lower.contains("tasks api")
        || lower.contains("accessnotconfigured")
    {
        Kind::ApiDisabled
    } else if status == 429 || matches!(reason, Some("rateLimitExceeded" | "userRateLimitExceeded"))
    {
        Kind::RateLimited { retry_after }
    } else if status == 401 {
        Kind::Auth
    } else if status == 404 {
        Kind::NotFound
    } else {
        Kind::Http
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(status: u16, detail: &str) -> Kind {
        GoogleError::from_status("Google", status, detail).kind
    }

    #[test]
    fn google_wording_decides_the_kinds_it_has_no_status_for() {
        assert_eq!(
            kind(403, "Request had insufficient authentication scopes."),
            Kind::MissingScope
        );
        assert_eq!(
            kind(
                403,
                "Google Tasks API has not been used in project 1 before or it is disabled."
            ),
            Kind::ApiDisabled
        );
        assert_eq!(kind(403, "accessNotConfigured"), Kind::ApiDisabled);
    }

    #[test]
    fn statuses_decide_the_rest() {
        assert_eq!(kind(401, "Invalid Credentials"), Kind::Auth);
        assert_eq!(kind(404, "Not Found"), Kind::NotFound);
        assert_eq!(kind(500, "Backend Error"), Kind::Http);
        assert_eq!(
            kind(429, "Slow down"),
            Kind::RateLimited { retry_after: None }
        );
    }

    #[test]
    fn a_rate_limit_is_recognized_from_the_reason_on_a_403() {
        let refusal = Refusal {
            status: 403,
            message: Some("Rate Limit Exceeded".into()),
            reason: Some("rateLimitExceeded".into()),
            retry_after: Some(Duration::from_secs(7)),
            body: String::new(),
        };
        assert_eq!(
            refusal.into_error("Google").kind,
            Kind::RateLimited {
                retry_after: Some(Duration::from_secs(7))
            }
        );
    }

    #[test]
    fn the_text_is_the_context_and_the_detail() {
        let error = GoogleError::from_status("Google Calendar", 500, "Backend Error");
        assert_eq!(error.to_string(), "Google Calendar: Backend Error");
        assert_eq!(
            GoogleError::from("Invalid task list").to_string(),
            "Invalid task list"
        );
        let refusal = Refusal {
            status: 503,
            message: None,
            reason: None,
            retry_after: None,
            body: String::new(),
        };
        assert_eq!(refusal.into_error("Tasks").to_string(), "Tasks: HTTP 503");
    }
}
