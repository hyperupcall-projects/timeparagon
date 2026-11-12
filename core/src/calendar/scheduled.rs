//! Shared Google resource discovery and retry-safe Calendar writes.
use super::{api, google};
use crate::calendar::ids;
use serde_json::{Value, json};
/// Status remains available to distinguish an absent event from permission or
/// network failures. In particular, a retry must never create a duplicate.
#[derive(Debug)]
pub struct Error {
	pub status: Option<u16>,
	pub message: String,
	/// The request never got an answer, so trying again may well work.
	pub transport: bool,
}

/// Keeps what a retry needs to know: Google's status, or that there was none
/// because the network failed.
impl From<Error> for google::GoogleError {
	fn from(error: Error) -> Self {
		match error.status {
			Some(status) => google::GoogleError::from_status("Google", status, error.message),
			None if error.transport => google::GoogleError::transport("Google", error.message),
			None => error.message.into(),
		}
	}
}

pub fn request(method: &str, url: &str, token: &str, body: Option<&Value>) -> Result<Value, Error> {
	if let Some(properties) = body.and_then(|b| b["extendedProperties"]["private"].as_object())
		&& properties
			.values()
			.any(|value| value.as_str().is_some_and(|s| s.chars().count() > 1024))
	{
		return Err(Error {
			status: None,
			message: "Calendar metadata exceeds the 1024 character property limit".into(),
			transport: false,
		});
	}
	#[cfg(test)]
	if let Some(answer) = api::faked(method, url, body.map(|b| b.to_string()).as_deref()) {
		return match answer {
			Ok(text) if text.is_empty() => Ok(Value::Null),
			Ok(text) => serde_json::from_str(&text).map_err(|e| Error {
				status: None,
				message: format!("Google response: {e}"),
				transport: true,
			}),
			Err((status, message)) => Err(Error {
				status: Some(status),
				message,
				transport: false,
			}),
		};
	}
	let request = api::agent()
		.request(method, url)
		.set("Authorization", &format!("Bearer {token}"))
		.set("Accept", "application/json");
	let result = match (method, body) {
		(_, Some(body)) => request.send_json(body),
		// Tasks.move is a POST with an empty body. Google answers 411 unless
		// that request carries Content-Length, and ureq's call() omits it.
		("POST" | "PUT" | "PATCH", None) => request.send_string(""),
		(_, None) => request.call(),
	};
	match result {
		Ok(response) if response.status() == 204 => Ok(Value::Null),
		Ok(response) => response.into_json().map_err(|e| Error {
			status: None,
			message: format!("Google response: {e}"),
			transport: true,
		}),
		Err(ureq::Error::Status(status, response)) => {
			let detail = google::Refusal::read(status, response).detail();
			let message = if status == 403 && detail.to_lowercase().contains("scope") {
				"Reconnect Google in Settings to allow task scheduling and calendar creation"
					.to_owned()
			} else {
				format!("Google: {detail}")
			};
			Err(Error {
				status: Some(status),
				message,
				transport: false,
			})
		}
		Err(error) => Err(Error {
			status: None,
			message: format!("Google: {error}"),
			transport: true,
		}),
	}
}

pub fn upsert_at(
	token: &str,
	url: &str,
	collection: &str,
	id: &str,
	body: Value,
) -> Result<(), String> {
	upsert_with(id, body, |method, body| {
		request(
			method,
			if method == "POST" { collection } else { url },
			token,
			Some(body),
		)
	})
}

fn upsert_with(
	id: &str,
	mut body: Value,
	mut request: impl FnMut(&str, &Value) -> Result<Value, Error>,
) -> Result<(), String> {
	match request("PATCH", &body) {
		Ok(_) => Ok(()),
		Err(error) if error.status == Some(404) => {
			body["id"] = json!(id);
			match request("POST", &body) {
				Ok(_) => Ok(()),
				Err(error) if error.status == Some(409) => {
					body.as_object_mut().unwrap().remove("id");
					request("PATCH", &body).map(|_| ()).map_err(|e| e.message)
				}
				Err(error) => Err(error.message),
			}
		}
		Err(error) => Err(error.message),
	}
}

/// The name of the list new tasks go to, unless another is chosen in Settings.
pub const DEFAULT_LIST: &str = "Inbox";

/// The Google Tasks list new tasks land in until they are triaged. An explicit
/// choice wins; otherwise the list called "Inbox" is reused, or created the
/// first time a task is added.
pub fn inbox_destination(token: &str, list: Option<&str>) -> Result<String, String> {
	match list {
		Some(id) => Ok(ids::bare_list(id).to_owned()),
		None => find_or_create(
			token,
			api::TASK_LISTS,
			api::TASK_LISTS,
			"title",
			DEFAULT_LIST,
		),
	}
}

/// The id of the app's single TimeSlay calendar: the one already there under
/// that name or a former name, or a freshly created one. Matching by name and
/// ignoring the access role means a calendar the app just made is reused rather
/// than duplicated on the next sync.
pub fn app_calendar(token: &str) -> Result<String, String> {
	if let Some(id) = find_app_calendar(token)? {
		return Ok(id);
	}
	let mut body = json!({});
	body["summary"] = json!(crate::config::CalendarSettings::APP_CALENDAR_NAME);
	request("POST", api::EVENTS, token, Some(&body)).map_err(|e| e.message)?["id"]
		.as_str()
		.map(str::to_owned)
		.ok_or_else(|| "Google did not return an ID for the TimeSlay calendar".into())
}

/// The id of the first calendar whose name is the app calendar or a former
/// name of it, if there is one.
pub fn find_app_calendar(token: &str) -> Result<Option<String>, String> {
	let mut page: Option<String> = None;
	loop {
		let url = format!(
			"https://www.googleapis.com/calendar/v3/users/me/calendarList?maxResults=250{}",
			page.as_ref()
				.map(|p| format!("&pageToken={}", urlencoding::encode(p)))
				.unwrap_or_default()
		);
		let response = request("GET", &url, token, None).map_err(|e| e.message)?;
		if let Some(id) = response["items"]
			.as_array()
			.into_iter()
			.flatten()
			.find(|item| {
				// A local rename is reported as `summaryOverride`, so match
				// both or the app would create a second calendar.
				["summary", "summaryOverride"].iter().any(|key| {
					item[*key]
						.as_str()
						.is_some_and(crate::config::CalendarSettings::is_app_calendar_name)
				})
			})
			.and_then(|item| item["id"].as_str())
		{
			return Ok(Some(id.to_owned()));
		}
		page = response["nextPageToken"].as_str().map(str::to_owned);
		if page.is_none() {
			return Ok(None);
		}
	}
}

pub fn find_or_create(
	token: &str,
	collection: &str,
	create_url: &str,
	name_field: &str,
	name: &str,
) -> Result<String, String> {
	let mut page = None;
	loop {
		let url = format!(
			"{collection}?maxResults=100{}",
			page.as_ref()
				.map(|p: &String| format!("&pageToken={}", urlencoding::encode(p)))
				.unwrap_or_default()
		);
		let response = request("GET", &url, token, None).map_err(|e| e.message)?;
		if let Some(id) = response["items"]
			.as_array()
			.into_iter()
			.flatten()
			.find(|item| {
				item[name_field].as_str() == Some(name)
					&& (name_field != "summary"
						|| matches!(item["accessRole"].as_str(), Some("owner" | "writer")))
			})
			.and_then(|item| item["id"].as_str())
		{
			return Ok(id.to_owned());
		}
		page = response["nextPageToken"].as_str().map(str::to_owned);
		if page.is_none() {
			break;
		}
	}
	let mut body = json!({});
	body[name_field] = json!(name);
	request("POST", create_url, token, Some(&body)).map_err(|e| e.message)?["id"]
		.as_str()
		.map(str::to_owned)
		.ok_or_else(|| "Google did not return an ID for the scheduling destination".into())
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn an_insert_conflict_reuses_the_reserved_id_instead_of_duplicating_the_event() {
		let mut methods = Vec::new();
		upsert_with("reserved-id", json!({"summary":"Task"}), |method, body| {
			methods.push(method.to_owned());
			match methods.len() {
				1 => Err(Error {
					status: Some(404),
					message: "missing".into(),
					transport: false,
				}),
				2 => {
					assert_eq!(body["id"], "reserved-id");
					Err(Error {
						status: Some(409),
						message: "already inserted".into(),
						transport: false,
					})
				}
				_ => {
					assert!(body.get("id").is_none());
					Ok(json!({}))
				}
			}
		})
		.unwrap();
		assert_eq!(methods, ["PATCH", "POST", "PATCH"]);
	}
	#[test]
	fn permission_and_network_errors_never_trigger_an_insert() {
		for status in [Some(403), None] {
			let mut count = 0;
			let error = upsert_with("id", json!({}), |method, _| {
				count += 1;
				assert_eq!(method, "PATCH");
				Err(Error {
					status,
					message: "refused".into(),
					transport: false,
				})
			})
			.unwrap_err();
			assert_eq!(error, "refused");
			assert_eq!(count, 1);
		}
	}
}
