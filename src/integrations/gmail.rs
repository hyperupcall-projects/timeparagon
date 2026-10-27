//! Gmail links.
//!
//! Gmail's address bar does not show the ID the API wants. Recent links end in
//! a token such as `FMfcgzQgLjNPlfJCVRfnNkPGkLhWClCW`; older ones carry the hex
//! ID directly, or `permthid=thread-f:<decimal>` in the query. This module
//! turns each into the hex ID, then asks the Gmail API for the subject.
//!
//! The token is the decimal ID, written as `f:<decimal>` (`thread-f:` or
//! `msg-f:` in some places), base64-encoded, and then respelled with a
//! consonant-only alphabet so it never spells a word. Undoing that is plain
//! base conversion.
//!
//! The link does not say which mailbox it belongs to (the `/u/0/` in it is a
//! position in the browser's account switcher, not an address), so every
//! connected Google account is asked and the one that has the conversation
//! answers. See [`super::accounts`].
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;

use super::accounts::Login;
use super::{Auth, Details, Integration, Kind};
use crate::calendar::api;

const API: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

/// The alphabet Gmail's tokens are written in: no vowels.
const TOKEN_ALPHABET: &str = "BCDFGHJKLMNPQRSTVWXZbcdfghjklmnpqrstvwxz";
const BASE64_ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
/// Shorter than any real token, so ordinary words in a path are not mistaken
/// for one.
const MIN_TOKEN: usize = 16;

pub struct Gmail;

impl Integration for Gmail {
    fn kind(&self) -> Kind {
        Kind::Gmail
    }

    fn name(&self) -> &'static str {
        "Gmail"
    }

    fn recognizes(&self, url: &str) -> bool {
        let parts = Parts::of(url);
        parts.host.eq_ignore_ascii_case("mail.google.com") && parts.path.starts_with("/mail")
    }

    fn fetch(&self, url: &str, auth: &Auth) -> Result<Details, String> {
        let reference = reference(url).ok_or(
            "This Gmail link does not point at a conversation. Open the message, then copy the address bar.",
        )?;
        let accounts = auth.google_accounts();
        let response = first_match(&accounts, |account| {
            let token = auth.google_token(account).map_err(Failure::Other)?;
            get(&reference.url(), &token)
        })?;
        let title = subject(&response, &reference).unwrap_or_default();
        Ok(Details {
            title: if title.is_empty() {
                "(no subject)".to_owned()
            } else {
                title
            },
            deadline: None,
            due: None,
        })
    }
}

/// Why one account could not answer.
#[derive(Debug, PartialEq, Eq)]
enum Failure {
    /// The mailbox has no such conversation: the link belongs to another one.
    NotFound,
    Other(String),
}

/// Asks each account in turn until one has the conversation.
///
/// An account that simply does not have it is not an error, since that is the
/// expected answer from every account but one. Only when none does is the
/// reason reported: either no account has it, or some could not be asked.
fn first_match<T>(
    accounts: &[Login],
    mut ask: impl FnMut(&Login) -> Result<T, Failure>,
) -> Result<T, String> {
    if accounts.is_empty() {
        return Err("Connect a Google account in Settings to read Gmail links".to_owned());
    }
    let mut problems = Vec::new();
    let mut missing = 0;
    for account in accounts {
        match ask(account) {
            Ok(found) => {
                log::debug!("Gmail link found in {}", account.label);
                return Ok(found);
            }
            Err(Failure::NotFound) => missing += 1,
            Err(Failure::Other(problem)) if accounts.len() == 1 => return Err(problem),
            Err(Failure::Other(problem)) => problems.push(format!("{}: {problem}", account.label)),
        }
    }
    if problems.is_empty() {
        return Err(if accounts.len() == 1 {
            "Gmail cannot find this message. It may belong to a different Google account; add that account in Settings".to_owned()
        } else {
            format!(
                "None of the {} connected Google accounts can see this message",
                accounts.len()
            )
        });
    }
    if missing > 0 {
        problems.push(format!("not found in the other {missing}"));
    }
    Err(problems.join("; "))
}

/// A thread or a single message, by the hex ID the API uses.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Id {
    message: bool,
    hex: String,
}

fn reference(url: &str) -> Option<Id> {
    let parts = Parts::of(url);
    for (key, value) in query_pairs(parts.query) {
        let found = match key.as_str() {
            // A thread opened from "print", or the old basic view.
            "th" | "permthid" | "permmsgid" => from_text(&value),
            _ => None,
        };
        if found.is_some() {
            return found;
        }
    }
    // `#inbox/<thread>`, `#label/Work/<thread>`, `#search/text/<thread>`: the
    // conversation is the last segment that reads as an ID.
    let fragment = parts.fragment.split('?').next().unwrap_or_default();
    fragment
        .rsplit('/')
        .find_map(|segment| from_text(&urlencoding::decode(segment).ok()?))
}

impl Id {
    fn url(&self) -> String {
        let kind = if self.message { "messages" } else { "threads" };
        format!(
            "{API}/{kind}/{}?format=metadata&metadataHeaders=Subject",
            self.hex
        )
    }
}

/// An ID as it appears in a URL: hex, `thread-f:<decimal>`, or a token.
fn from_text(text: &str) -> Option<Id> {
    let text = text.trim();
    if (15..=16).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(Id {
            message: false,
            hex: text.to_ascii_lowercase(),
        });
    }
    if text.len() >= MIN_TOKEN && text.chars().all(|c| TOKEN_ALPHABET.contains(c)) {
        return from_decoded(&decode_token(text)?);
    }
    from_decoded(text)
}

/// `f:<decimal>`, optionally prefixed with `thread-` or `msg-`.
fn from_decoded(text: &str) -> Option<Id> {
    let (message, rest) = match text.strip_prefix("msg-") {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix("thread-").unwrap_or(text)),
    };
    // `a:` marks ids Gmail makes for mail you sent to yourself; they have no
    // API equivalent that can be derived from the link.
    let decimal = rest.strip_prefix("f:")?;
    let number: u64 = decimal.parse().ok()?;
    Some(Id {
        message,
        hex: format!("{number:x}"),
    })
}

/// Reads a token written in [`TOKEN_ALPHABET`] as a base-40 number, and writes
/// that number out in base 64 to get the base64 text it stands for.
fn decode_token(token: &str) -> Option<String> {
    let mut digits: Vec<u32> = Vec::new(); // base 64, least significant first
    for c in token.chars() {
        let mut carry = TOKEN_ALPHABET.find(c)? as u32;
        for digit in &mut digits {
            let value = *digit * TOKEN_ALPHABET.len() as u32 + carry;
            *digit = value % 64;
            carry = value / 64;
        }
        while carry > 0 {
            digits.push(carry % 64);
            carry /= 64;
        }
    }
    let mut encoded: String = digits
        .iter()
        .rev()
        .map(|&digit| BASE64_ALPHABET.as_bytes()[digit as usize] as char)
        .collect();
    if encoded.len() % 4 == 1 {
        return None;
    }
    while !encoded.len().is_multiple_of(4) {
        encoded.push('=');
    }
    String::from_utf8(STANDARD.decode(encoded).ok()?).ok()
}

fn subject(response: &Value, id: &Id) -> Option<String> {
    let headers = if id.message {
        &response["payload"]["headers"]
    } else {
        // A thread starts with its first message, whose subject names it.
        &response["messages"][0]["payload"]["headers"]
    };
    let value = headers.as_array()?.iter().find(|header| {
        header["name"]
            .as_str()
            .is_some_and(|name| name.eq_ignore_ascii_case("subject"))
    })?["value"]
        .as_str()?;
    // Long subjects arrive folded across lines.
    Some(value.split_whitespace().collect::<Vec<_>>().join(" "))
}

fn get(url: &str, token: &str) -> Result<Value, Failure> {
    let result = api::agent()
        .get(url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Accept", "application/json")
        .call();
    match result {
        Ok(response) => response
            .into_json()
            .map_err(|error| Failure::Other(format!("Gmail response: {error}"))),
        Err(ureq::Error::Status(status, response)) => {
            let detail = response
                .into_json::<Value>()
                .ok()
                .and_then(|body| body["error"]["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("HTTP {status}"));
            Err(describe_error(status, &detail))
        }
        Err(error) => Err(Failure::Other(format!("Gmail: {error}"))),
    }
}

fn describe_error(status: u16, detail: &str) -> Failure {
    let lower = detail.to_lowercase();
    Failure::Other(match status {
        401 | 403 if lower.contains("scope") || lower.contains("insufficient") => {
            "Reconnect Google in Settings to allow read access to Gmail".to_owned()
        }
        403 if lower.contains("not been used") || lower.contains("disabled") => {
            "Turn on the Gmail API for your Google Cloud project, then try again".to_owned()
        }
        401 => "Google sign-in expired. Reconnect Google in Settings".to_owned(),
        // 400 is how Gmail answers an ID that is not in this mailbox's format.
        404 | 400 => return Failure::NotFound,
        _ => format!("Gmail: {detail}"),
    })
}

/// The pieces of a URL this module cares about. Good enough for the addresses
/// a browser produces; it does not validate.
struct Parts<'a> {
    host: &'a str,
    path: &'a str,
    query: &'a str,
    fragment: &'a str,
}

impl<'a> Parts<'a> {
    fn of(url: &'a str) -> Self {
        let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
        let (rest, fragment) = rest.split_once('#').unwrap_or((rest, ""));
        let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
        let (host, path) = match rest.find('/') {
            Some(at) => rest.split_at(at),
            None => (rest, ""),
        };
        Self {
            host: host.rsplit('@').next().unwrap_or(host),
            path,
            query,
            fragment,
        }
    }
}

fn query_pairs(query: &str) -> impl Iterator<Item = (String, String)> + '_ {
    query.split('&').filter_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        Some((
            urlencoding::decode(key).ok()?.into_owned(),
            urlencoding::decode(value).ok()?.into_owned(),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(url: &str) -> Option<(bool, String)> {
        reference(url).map(|id| (id.message, id.hex))
    }

    #[test]
    fn a_modern_token_decodes_to_the_hex_thread_id() {
        // `f:1864099454408550099` in base64, respelled without vowels.
        assert_eq!(
            decode_token("FMfcgzQgLjNPlfJCVRfnNkPGkLhWClCW").as_deref(),
            Some("f:1864099454408550099")
        );
        assert_eq!(
            id("https://mail.google.com/mail/u/0/#inbox/FMfcgzQgLjNPlfJCVRfnNkPGkLhWClCW"),
            Some((false, "19de9c93ce0566d3".to_owned()))
        );
    }

    #[test]
    fn the_conversation_is_found_after_any_folder_label_or_search() {
        let token = "FMfcgzQgLjNPlfJCVRfnNkPGkLhWClCW";
        for fragment in [
            format!("inbox/{token}"),
            format!("label/Work%2FClients/{token}"),
            format!("search/invoice+march/{token}"),
            format!("all/{token}?compose=new"),
            format!("inbox/{token}/"),
        ] {
            assert_eq!(
                id(&format!("https://mail.google.com/mail/u/1/#{fragment}")),
                Some((false, "19de9c93ce0566d3".to_owned())),
                "{fragment}"
            );
        }
    }

    #[test]
    fn older_link_styles_still_resolve() {
        assert_eq!(
            id("https://mail.google.com/mail/u/0/#inbox/18c3a1b2c3d4e5f6"),
            Some((false, "18c3a1b2c3d4e5f6".to_owned()))
        );
        assert_eq!(
            id("https://mail.google.com/mail/?th=18C3A1B2C3D4E5F6&view=pt"),
            Some((false, "18c3a1b2c3d4e5f6".to_owned()))
        );
        assert_eq!(
            id(
                "https://mail.google.com/mail/u/0/?view=pt&search=all&permthid=thread-f%3A1864099454408550099"
            ),
            Some((false, "19de9c93ce0566d3".to_owned()))
        );
        assert_eq!(
            id("https://mail.google.com/mail/u/0/?view=pt&permmsgid=msg-f%3A1864099454408550099"),
            Some((true, "19de9c93ce0566d3".to_owned()))
        );
    }

    #[test]
    fn links_without_a_conversation_have_no_id() {
        for url in [
            "https://mail.google.com/mail/u/0/#inbox",
            "https://mail.google.com/mail/u/0/",
            "https://mail.google.com/mail/u/0/#label/Receipts",
            "https://mail.google.com/mail/u/0/#search/from%3Aboss",
            // Mail sent to yourself uses a different scheme with no hex form.
            "https://mail.google.com/mail/u/0/#inbox/KtbxLwHDkcrRDgFknhGFpwWXVSwWHDmHRL",
        ] {
            assert_eq!(id(url), None, "{url}");
        }
    }

    #[test]
    fn only_gmail_addresses_are_recognized() {
        let gmail = Gmail;
        assert!(gmail.recognizes("https://mail.google.com/mail/u/0/#inbox/x"));
        assert!(gmail.recognizes("http://MAIL.google.com/mail"));
        assert!(!gmail.recognizes("https://mail.google.com.evil.example/mail/u/0/"));
        assert!(!gmail.recognizes("https://evil.example/@mail.google.com/mail"));
        assert!(!gmail.recognizes("https://calendar.google.com/mail"));
        assert!(!gmail.recognizes("https://mail.google.com/chat"));
    }

    #[test]
    fn the_subject_comes_from_the_first_message_of_a_thread() {
        let thread = json!({"messages": [
            {"payload": {"headers": [
                {"name": "From", "value": "a@example.org"},
                {"name": "Subject", "value": "Quarterly   report\r\n due"}
            ]}},
            {"payload": {"headers": [{"name": "Subject", "value": "Re: Quarterly report"}]}}
        ]});
        let reference = from_text("19de9c93ce0566d3").unwrap();
        assert_eq!(
            subject(&thread, &reference).as_deref(),
            Some("Quarterly report due")
        );
        let message = json!({"payload": {"headers": [{"name": "subject", "value": "Hello"}]}});
        let reference = from_decoded("msg-f:1864099454408550099").unwrap();
        assert_eq!(subject(&message, &reference).as_deref(), Some("Hello"));
        assert_eq!(
            subject(&json!({}), &from_text("19de9c93ce0566d3").unwrap()),
            None
        );
    }

    #[test]
    fn api_requests_target_the_right_resource() {
        assert_eq!(
            from_text("19de9c93ce0566d3").unwrap().url(),
            "https://gmail.googleapis.com/gmail/v1/users/me/threads/19de9c93ce0566d3?format=metadata&metadataHeaders=Subject"
        );
        assert!(
            from_decoded("msg-f:1864099454408550099")
                .unwrap()
                .url()
                .contains("/messages/19de9c93ce0566d3?")
        );
    }

    #[test]
    fn failures_say_what_to_do() {
        let text = |failure| match failure {
            Failure::Other(text) => text,
            Failure::NotFound => panic!("expected a message"),
        };
        assert!(
            text(describe_error(
                403,
                "Request had insufficient authentication scopes."
            ))
            .contains("Reconnect")
        );
        assert!(
            text(describe_error(
                403,
                "Gmail API has not been used in project 1 before"
            ))
            .contains("Gmail API")
        );
        assert_eq!(
            text(describe_error(500, "Backend Error")),
            "Gmail: Backend Error"
        );
    }

    #[test]
    fn a_missing_conversation_is_not_a_failure_of_the_account() {
        assert_eq!(
            describe_error(404, "Requested entity was not found."),
            Failure::NotFound
        );
        assert_eq!(describe_error(400, "Invalid id value"), Failure::NotFound);
    }

    fn logins(emails: &[&str]) -> Vec<Login> {
        emails.iter().map(|email| Login::extra(email)).collect()
    }

    #[test]
    fn the_account_that_has_the_conversation_answers() {
        let accounts = logins(&["a@x.org", "b@x.org", "c@x.org"]);
        let mut asked = Vec::new();
        let found = first_match(&accounts, |account| {
            asked.push(account.label.clone());
            if account.label == "b@x.org" {
                Ok("subject")
            } else {
                Err(Failure::NotFound)
            }
        });
        assert_eq!(found, Ok("subject"));
        // Nothing is asked once an account has answered.
        assert_eq!(asked, ["a@x.org", "b@x.org"]);
    }

    #[test]
    fn an_account_that_cannot_be_asked_does_not_hide_one_that_can() {
        let accounts = logins(&["a@x.org", "b@x.org"]);
        let found = first_match(&accounts, |account| {
            if account.label == "a@x.org" {
                Err(Failure::Other("expired".to_owned()))
            } else {
                Ok(7)
            }
        });
        assert_eq!(found, Ok(7));
    }

    #[test]
    fn when_no_account_has_it_the_error_says_so() {
        let one = first_match::<()>(&logins(&["a@x.org"]), |_| Err(Failure::NotFound));
        assert!(one.unwrap_err().contains("different Google account"));
        let many = first_match::<()>(&logins(&["a@x.org", "b@x.org"]), |_| Err(Failure::NotFound));
        assert_eq!(
            many.unwrap_err(),
            "None of the 2 connected Google accounts can see this message"
        );
        let none = first_match::<()>(&[], |_| Err(Failure::NotFound));
        assert!(none.unwrap_err().contains("Connect a Google account"));
    }

    #[test]
    fn errors_name_the_account_they_came_from() {
        let accounts = logins(&["a@x.org", "b@x.org"]);
        let error = first_match::<()>(&accounts, |account| {
            if account.label == "a@x.org" {
                Err(Failure::Other("expired".to_owned()))
            } else {
                Err(Failure::NotFound)
            }
        })
        .unwrap_err();
        assert_eq!(error, "a@x.org: expired; not found in the other 1");
        // With a single account there is nothing to tell apart.
        let error = first_match::<()>(&accounts[..1], |_| Err(Failure::Other("expired".into())))
            .unwrap_err();
        assert_eq!(error, "expired");
    }
}
