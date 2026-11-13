//! Translate custom RGB choices into native Google Calendar labels.
use super::{api, colors, google::GoogleError, scheduled};
use serde_json::{Value, json};

/// Prepare a Calendar write. A private RGB value preserves the exact choice
/// when reading with the legacy palette API used elsewhere by the app.
pub(crate) fn prepare(
	url: &str,
	token: &str,
	body: &Value,
) -> Result<(String, Value), GoogleError> {
	let mut body = body.clone();
	if !url.starts_with(api::EVENTS) || !url.contains("/events") {
		return Ok((url.to_owned(), body));
	}
	let Some(choice) = body
		.get("colorId")
		.and_then(Value::as_str)
		.map(str::to_owned)
	else {
		return Ok((url.to_owned(), body));
	};
	let choice = colors::selection(&choice)?;
	if let Some(rgb) = choice.as_deref().filter(|s| s.starts_with('#')) {
		let calendar_url = url.split("/events").next().ok_or("Missing calendar URL")?;
		let label = ensure_label(calendar_url, token, rgb)?;
		body.as_object_mut().unwrap().remove("colorId");
		body["eventLabelId"] = label.into();
		body["extendedProperties"]["private"]["ttColor"] = rgb.into();
		let separator = if url.contains('?') { '&' } else { '?' };
		Ok((format!("{url}{separator}eventLabelVersion=1"), body))
	} else {
		body["colorId"] = choice.map(Value::String).unwrap_or(Value::Null);
		body["extendedProperties"]["private"]["ttColor"] = Value::Null;
		// Legacy palette writes are handled by Google's compatibility mapping.
		Ok((url.to_owned(), body))
	}
}

fn ensure_label(url: &str, token: &str, rgb: &str) -> Result<String, GoogleError> {
	// Serialize read/modify/write in this process and use ETag preconditions
	// so another client cannot have its labels silently overwritten.
	static LABELS: std::sync::Mutex<()> = std::sync::Mutex::new(());
	let _guard = LABELS.lock().unwrap_or_else(|e| e.into_inner());
	let mut calendar = scheduled::request("GET", url, token, None).map_err(GoogleError::from)?;
	let mut labels = calendar["labelProperties"]["eventLabels"]
		.as_array()
		.cloned()
		.unwrap_or_default();
	if let Some(id) = labels
		.iter()
		.find(|l| {
			l["backgroundColor"]
				.as_str()
				.is_some_and(|c| c.eq_ignore_ascii_case(rgb))
		})
		.and_then(|l| l["id"].as_str())
	{
		return Ok(id.to_owned());
	}
	if labels.len() >= 200 {
		return Err(
			"This Google calendar has reached its limit of 200 color labels"
				.to_owned()
				.into(),
		);
	}
	let mut bytes = [0u8; 16];
	getrandom::fill(&mut bytes).map_err(|e| e.to_string())?;
	bytes[6] = (bytes[6] & 0x0f) | 0x40;
	bytes[8] = (bytes[8] & 0x3f) | 0x80;
	let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
	let id = format!(
		"{}-{}-{}-{}-{}",
		&hex[..8],
		&hex[8..12],
		&hex[12..16],
		&hex[16..20],
		&hex[20..]
	);
	labels.push(json!({"id":id,"backgroundColor":rgb}));
	calendar["labelProperties"]["eventLabels"] = labels.into();
	#[cfg(test)]
	if let Some(answer) = api::faked("PUT", url, Some(&calendar.to_string())) {
		answer.map_err(|(status, detail)| {
			GoogleError::from_status("Create Google calendar color", status, detail)
		})?;
		return Ok(id);
	}

	let mut request = api::agent()
		.request("PUT", url)
		.set("Authorization", &format!("Bearer {token}"));
	if let Some(etag) = calendar["etag"].as_str() {
		request = request.set("If-Match", etag);
	}
	super::google::send(
		"Create Google calendar color (calendar owner access required)",
		request.send_json(calendar),
	)?;
	Ok(id)
}

pub(crate) fn read_url(url: &str) -> String {
	if url.starts_with(api::EVENTS)
		&& url.contains("/events")
		&& !url.contains("eventLabelVersion=")
	{
		format!(
			"{url}{}eventLabelVersion=1",
			if url.contains('?') { '&' } else { '?' }
		)
	} else {
		url.to_owned()
	}
}

/// Resolve native Google labels on each fetch, including changes made in Google.
pub(crate) fn read(url: &str, token: &str, value: &mut Value) -> Result<(), GoogleError> {
	if !url.starts_with(api::EVENTS) || !url.contains("/events") {
		return Ok(());
	}
	let items: Vec<&Value> = match value.get("items").and_then(Value::as_array) {
		Some(items) => items.iter().collect(),
		None => vec![value],
	};

	let has_labels = items
		.iter()
		.any(|item| item["eventLabelId"].as_str().is_some_and(|s| !s.is_empty()));
	let calendar = if has_labels {
		let calendar_url = url.split("/events").next().ok_or("Missing calendar URL")?;
		scheduled::request("GET", calendar_url, token, None).map_err(GoogleError::from)?
	} else {
		Value::Null
	};

	let labels = calendar["labelProperties"]["eventLabels"].as_array();
	let resolve = |item: &mut Value| {
		let rgb = item["eventLabelId"]
			.as_str()
			.and_then(|id| labels?.iter().find(|l| l["id"].as_str() == Some(id)))
			.and_then(|l| l["backgroundColor"].as_str())
			.map(str::to_owned);
		// Native labels are authoritative; don't keep an outdated private RGB.
		if let Some(private) = item
			.get_mut("extendedProperties")
			.and_then(|properties| properties.get_mut("private"))
			.and_then(Value::as_object_mut)
		{
			private.remove("ttColor");
		}
		if item.get("eventLabelId").is_some() {
			if let Some(rgb) = rgb {
				item["colorId"] = colors::resolve(&rgb)
					.and_then(colors::event_id)
					.map(str::to_owned)
					.unwrap_or(rgb)
					.into();
			} else {
				item.as_object_mut().unwrap().remove("colorId");
			}
		}
	};
	if let Some(items) = value.get_mut("items").and_then(Value::as_array_mut) {
		for item in items {
			resolve(item);
		}
	} else {
		resolve(value);
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::{Arc, Mutex};

	#[test]
	fn reading_colors_does_not_add_null_fields() {
		for original in [
			json!({"kind":"calendar#events", "nextSyncToken":"next"}),
			json!({"id":"event", "summary":"No custom color"}),
			json!({"items":[{"id":"event"}]}),
		] {
			let mut value = original.clone();
			read(&format!("{}/work/events", api::EVENTS), "token", &mut value).unwrap();
			assert_eq!(value, original);
		}
	}

	#[test]
	fn custom_color_reuses_a_native_label_and_preserves_event_metadata() {
		api::with_transport(
			Box::new(|method, url, _| {
				assert_eq!(method, "GET");
				assert!(url.ends_with("/work"));
				Ok(json!({"labelProperties":{"eventLabels":[{"id":"existing","backgroundColor":"#123ABC"}]}}).to_string())
			}),
			|| {
				let (url, body) = prepare(
					&format!("{}/work/events/event?sendUpdates=none", api::EVENTS),
					"token",
					&json!({"colorId":"#123abc","extendedProperties":{"private":{"ttKind":"habit"}}}),
				)
				.unwrap();
				assert!(url.ends_with("&eventLabelVersion=1"));
				assert_eq!(body["eventLabelId"], "existing");
				assert!(body.get("colorId").is_none());
				assert_eq!(body["extendedProperties"]["private"]["ttKind"], "habit");
			},
		);
	}

	#[test]
	fn adding_a_label_preserves_existing_labels() {
		let written = Arc::new(Mutex::new(Value::Null));
		let capture = written.clone();
		api::with_transport(
			Box::new(move |method, _, body| {
				match method {
            "GET" => Ok(json!({"summary":"Work","etag":"v1","labelProperties":{"eventLabels":[{"id":"keep","backgroundColor":"#FFFFFF","name":"Keep me"}]}}).to_string()),
            "PUT" => { *capture.lock().unwrap() = serde_json::from_str(body.unwrap()).unwrap(); Ok("{}".into()) },
            _ => panic!("Unexpected request"),
        }
			}),
			|| {
				prepare(
					&format!("{}/work/events", api::EVENTS),
					"token",
					&json!({"colorId":"#123456"}),
				)
				.unwrap();
			},
		);
		let body = written.lock().unwrap();
		assert_eq!(body["summary"], "Work");
		let labels = body["labelProperties"]["eventLabels"].as_array().unwrap();
		assert_eq!(labels.len(), 2);
		assert_eq!(labels[0]["name"], "Keep me");
		assert_eq!(labels[1]["backgroundColor"], "#123456");
		assert_eq!(labels[1]["id"].as_str().unwrap().len(), 36);
	}

	#[test]
	fn google_label_changes_override_stale_private_rgb() {
		api::with_transport(
			Box::new(|_, _, _| {
				Ok(json!({"labelProperties":{"eventLabels":[{"id":"changed","backgroundColor":"#123456"}]}}).to_string())
			}),
			|| {
				let mut value = json!({"items":[{"eventLabelId":"changed","extendedProperties":{"private":{"ttColor":"#654321"}}}]});
				read(&format!("{}/work/events", api::EVENTS), "token", &mut value).unwrap();
				assert_eq!(value["items"][0]["colorId"], "#123456");
				assert!(
					value["items"][0]["extendedProperties"]["private"]
						.get("ttColor")
						.is_none()
				);
			},
		);
	}

	#[test]
	fn resetting_a_label_in_google_discards_the_old_private_color() {
		let mut value =
			json!({"id":"event","extendedProperties":{"private":{"ttColor":"#123456"}}});
		read(
			&format!("{}/work/events/event", api::EVENTS),
			"token",
			&mut value,
		)
		.unwrap();
		assert!(
			value["extendedProperties"]["private"]
				.get("ttColor")
				.is_none()
		);
		assert!(value.get("colorId").is_none());
	}

	#[test]
	fn palette_default_and_invalid_choices_are_distinct() {
		assert_eq!(
			colors::selection("#abc").unwrap().as_deref(),
			Some("#AABBCC")
		);
		assert_eq!(colors::selection("11").unwrap().as_deref(), Some("11"));
		assert_eq!(colors::selection("").unwrap(), None);
		for invalid in ["#abcd", "#12345678", "12", "red"] {
			assert!(colors::selection(invalid).is_err());
		}
		let url = format!("{}/work/events/id", api::EVENTS);
		let (_, body) = prepare(&url, "token", &json!({"colorId":""})).unwrap();
		assert!(body["colorId"].is_null());
		let (_, body) = prepare(&url, "token", &json!({"summary":"unchanged color"})).unwrap();
		assert!(body.get("colorId").is_none());
	}
}
