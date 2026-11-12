//! The Google People API: contacts, their labels, and Workspace directories.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::calendar::api;
use crate::calendar::google::{self, GoogleError, Kind};

const API: &str = "https://people.googleapis.com/v1";
const CONTEXT: &str = "Google Contacts";

/// What is read of a contact. `metadata` is not optional: an update has to send
/// its sources back.
const PERSON_FIELDS: &str = "names,emailAddresses,phoneNumbers,organizations,addresses,urls,biographies,memberships,metadata";
/// Directory entries have no notes or labels of the reader's to show.
const DIRECTORY_FIELDS: &str =
	"names,emailAddresses,phoneNumbers,organizations,addresses,urls,metadata";
/// The contact's own data only. Merging in the person's public profile would
/// give `names` a second value, and an update allows just one.
const SOURCES: &str = "sources=READ_SOURCE_TYPE_CONTACT";
const DIRECTORY_SOURCES: &str =
	"sources=DIRECTORY_SOURCE_TYPE_DOMAIN_PROFILE&sources=DIRECTORY_SOURCE_TYPE_DOMAIN_CONTACT";
/// Enough to find someone by typing more; a campus directory runs to thousands.
const DIRECTORY_RESULTS: u32 = 50;

/// A label on an account's contacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
	pub resource: String,
	pub name: String,
}

/// Why a call failed, worded for the person using the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
	pub message: String,
	/// Signing in to the account again would fix it.
	pub reconnect: bool,
}

impl Problem {
	pub fn sign_in(message: impl Into<String>) -> Self {
		Self {
			message: message.into(),
			reconnect: true,
		}
	}

	pub fn other(message: impl Into<String>) -> Self {
		Self {
			message: message.into(),
			reconnect: false,
		}
	}
}

/// What came of searching an account's directory.
#[derive(Debug, Clone, PartialEq)]
pub enum Directory {
	Found(Vec<Value>),
	/// A personal account: there is no directory to search.
	None,
}

/// Words a failed call for the person using the app.
pub fn describe(error: GoogleError) -> Problem {
	let lower = error.detail.to_lowercase();
	match error.kind {
		Kind::MissingScope => {
			Problem::sign_in("Sign in again to allow access to contacts and the directory")
		}
		Kind::Auth => Problem::sign_in("Google sign-in expired. Sign in again"),
		_ if lower.contains("disabled") || error.kind == Kind::ApiDisabled => Problem::other(
			"Turn on the People API for your Google Cloud project, then press Refresh",
		),
		Kind::RateLimited { .. } => Problem::other("Google is rate limiting. Try again shortly"),
		_ if lower.contains("etag") => Problem::other(
			"This contact changed in Google since it was loaded. Press Refresh, then edit it again",
		),
		_ => Problem::other(error.to_string()),
	}
}

fn get(url: &str, token: &str) -> Result<Value, GoogleError> {
	let result = api::agent()
		.get(url)
		.set("Authorization", &format!("Bearer {token}"))
		.set("Accept", "application/json")
		.call();
	read(google::send(CONTEXT, result)?)
}

fn send(method: &str, url: &str, token: &str, body: &Value) -> Result<Value, Problem> {
	let result = api::agent()
		.request(method, url)
		.set("Authorization", &format!("Bearer {token}"))
		.set("Content-Type", "application/json")
		.send_string(&body.to_string());
	google::send(CONTEXT, result)
		.and_then(read)
		.map_err(describe)
}

fn read(response: ureq::Response) -> Result<Value, GoogleError> {
	response
		.into_json()
		.map_err(|error| GoogleError::transport(CONTEXT, error))
}

/// Follows `nextPageToken` until Google has no more, collecting `key`.
fn all_pages(url: &str, key: &str, token: &str) -> Result<Vec<Value>, Problem> {
	let mut items = Vec::new();
	let mut page: Option<String> = None;
	loop {
		let address = match &page {
			Some(page) => format!("{url}&pageToken={}", urlencoding::encode(page)),
			None => url.to_owned(),
		};
		let mut body = get(&address, token).map_err(describe)?;
		if let Value::Array(found) = body[key].take() {
			items.extend(found);
		}
		match body["nextPageToken"].as_str() {
			Some(next) if !next.is_empty() => page = Some(next.to_owned()),
			_ => return Ok(items),
		}
	}
}

/// Every contact of the account.
pub fn contacts(token: &str) -> Result<Vec<Value>, Problem> {
	all_pages(
		&format!(
			"{API}/people/me/connections?personFields={PERSON_FIELDS}&{SOURCES}&pageSize=1000"
		),
		"connections",
		token,
	)
}

/// The account's own labels. System groups such as "My Contacts" hold nearly
/// everyone and cannot be chosen in Google's own interface, so they are left out.
pub fn groups(token: &str) -> Result<Vec<Group>, Problem> {
	let groups = all_pages(
		&format!("{API}/contactGroups?pageSize=1000"),
		"contactGroups",
		token,
	)?;
	Ok(groups.iter().filter_map(parse_group).collect())
}

fn parse_group(group: &Value) -> Option<Group> {
	(group["groupType"].as_str() == Some("USER_CONTACT_GROUP")).then_some(())?;
	Some(Group {
		resource: group["resourceName"].as_str()?.to_owned(),
		name: group["formattedName"]
			.as_str()
			.or(group["name"].as_str())?
			.to_owned(),
	})
}

pub fn create_group(token: &str, name: &str) -> Result<Group, Problem> {
	let created = send(
		"POST",
		&format!("{API}/contactGroups"),
		token,
		&json!({ "contactGroup": { "name": name } }),
	)?;
	Ok(Group {
		resource: created["resourceName"]
			.as_str()
			.ok_or_else(|| Problem::other("Google did not return the new label"))?
			.to_owned(),
		name: name.to_owned(),
	})
}

/// Sends the fields named in `fields`. Returns the contact as Google now has it.
pub fn update(
	token: &str,
	resource: &str,
	fields: &[&str],
	body: &Value,
) -> Result<Value, Problem> {
	send(
		"PATCH",
		&format!(
			"{API}/{resource}:updateContact?updatePersonFields={}&personFields={PERSON_FIELDS}&{SOURCES}",
			fields.join(",")
		),
		token,
		body,
	)
}

pub fn create(token: &str, body: &Value) -> Result<Value, Problem> {
	send(
		"POST",
		&format!("{API}/people:createContact?personFields={PERSON_FIELDS}&{SOURCES}"),
		token,
		body,
	)
}

const PROFILE: &str = "https://gmail.googleapis.com/gmail/v1/users/me/profile";

/// The address a token belongs to, from Gmail's own profile call: every account
/// here has been granted read-only Gmail. Blocking.
pub fn profile_email(access_token: &str) -> Result<String, String> {
	let result = api::agent()
		.get(PROFILE)
		.set("Authorization", &format!("Bearer {access_token}"))
		.set("Accept", "application/json")
		.call();
	match result {
		Ok(response) => {
			let body: Value = response
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

/// The people in the account's Workspace domain whose fields start with `query`.
pub fn search_directory(token: &str, query: &str) -> Result<Directory, Problem> {
	let url = format!(
		"{API}/people:searchDirectoryPeople?query={}&readMask={DIRECTORY_FIELDS}&{DIRECTORY_SOURCES}&pageSize={DIRECTORY_RESULTS}",
		urlencoding::encode(query)
	);
	match get(&url, token) {
		Ok(mut body) => Ok(match body["people"].take() {
			Value::Array(people) => Directory::Found(people),
			_ => Directory::Found(Vec::new()),
		}),
		Err(error) if not_in_a_domain(&error) => Ok(Directory::None),
		Err(error) => Err(describe(error)),
	}
}

/// How Google answers a directory search from outside any Workspace domain.
fn not_in_a_domain(error: &GoogleError) -> bool {
	let lower = error.detail.to_lowercase();
	error.status == Some(400)
		&& (lower.contains("domain") || lower.contains("g suite") || lower.contains("workspace"))
}

/// Personal addresses are known to have no directory, so they are not asked.
pub fn may_have_directory(email: &str) -> bool {
	match email.rsplit_once('@') {
		Some((_, domain)) => {
			!domain.eq_ignore_ascii_case("gmail.com")
				&& !domain.eq_ignore_ascii_case("googlemail.com")
		}
		// The calendar's account, when its address could not be read.
		None => true,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_a_users_own_groups_count_as_labels() {
		let system = json!({"resourceName": "contactGroups/myContacts", "groupType": "SYSTEM_CONTACT_GROUP", "formattedName": "My Contacts"});
		let mine = json!({"resourceName": "contactGroups/1", "groupType": "USER_CONTACT_GROUP", "name": "Family"});
		assert_eq!(parse_group(&system), None);
		assert_eq!(parse_group(&mine).unwrap().name, "Family");
	}

	#[test]
	fn personal_accounts_are_not_asked_for_a_directory() {
		assert!(!may_have_directory("me@Gmail.com"));
		assert!(!may_have_directory("me@googlemail.com"));
		assert!(may_have_directory("me@csumb.edu"));
	}

	#[test]
	fn refusals_read_as_what_to_do_next() {
		let error = GoogleError::from_status(
			CONTEXT,
			403,
			"Request had insufficient authentication scopes.",
		);
		assert!(describe(error).reconnect);
		let error =
			GoogleError::from_status(CONTEXT, 403, "People API has not been used in project 1");
		let problem = describe(error);
		assert!(!problem.reconnect);
		assert!(problem.message.contains("People API"));
		let error = GoogleError::from_status(
			CONTEXT,
			400,
			"Request person.etag is different than the current person.etag.",
		);
		assert!(describe(error).message.contains("changed in Google"));
		assert!(not_in_a_domain(&GoogleError::from_status(
			CONTEXT,
			400,
			"Must be a G Suite domain user."
		)));
		assert!(!not_in_a_domain(&GoogleError::from_status(
			CONTEXT,
			400,
			"Invalid readMask"
		)));
	}
}
