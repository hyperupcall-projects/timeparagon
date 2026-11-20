//! Calendar events a model reads out of a picture or text, for review before
//! anything is saved. The request, schema and validation are shared by both.
use serde::Deserialize;
use serde_json::{Value, json};
use std::{io::Read, time::Duration};

pub const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
/// The most events accepted from one picture or text.
pub const MAX_EVENTS: usize = 100;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Found {
	pub title: String,
	pub date: Option<String>,
	pub end_date: Option<String>,
	pub start_time: Option<String>,
	pub end_time: Option<String>,
	pub description: Option<String>,
	pub location: Option<String>,
}

pub fn schema() -> Value {
	let nullable = json!({"type":["string","null"]});
	json!({"type":"object","additionalProperties":false,"required":["events"],"properties":{"events":{"type":"array","maxItems":MAX_EVENTS,"items":{"type":"object","additionalProperties":false,"required":["title","date","end_date","start_time","end_time","description","location"],"properties":{"title":{"type":"string"},"date":nullable,"end_date":nullable,"start_time":nullable,"end_time":nullable,"description":nullable,"location":nullable}}}}})
}

/// OpenRouter's form of [`schema`].
pub fn response_format() -> Value {
	json!({"type":"json_schema","json_schema":{"name":"calendar_events","strict":true,"schema":schema()}})
}

/// The system prompt: `intro` says what to read, the rest how to answer.
pub fn prompt(intro: &str) -> String {
	format!(
		"{intro} Return only JSON matching the schema. Today is {} ({}). Use YYYY-MM-DD dates and 24-hour HH:MM local times. Resolve relative dates from today. Use null for missing fields; never invent dates, times or locations. Include useful notes in description. Return an empty events array if none are found.",
		crate::clock::now().format("%A %Y-%m-%d"),
		iana_time_zone::get_timezone().unwrap_or_default()
	)
}

/// Posts a chat request `body` built for OpenRouter or Ollama and returns the
/// events in the answer. `what` names the model in errors ("Image") and
/// `check` lists what to check in settings when the request fails.
pub fn ask(
	openrouter: bool,
	url: &str,
	body: Value,
	what: &str,
	check: &str,
) -> anyhow::Result<Vec<Found>> {
	let agent = ureq::AgentBuilder::new()
		.timeout_connect(Duration::from_secs(5))
		.timeout(Duration::from_secs(180))
		.build();
	let mut request = agent.post(url);
	if openrouter {
		let token = crate::secrets::path()
			.and_then(|p| {
				crate::secrets::read(&p, crate::secrets::keys::OPENROUTER_TOKEN)
					.ok()
					.flatten()
			})
			.filter(|s: &String| !s.trim().is_empty())
			.ok_or_else(|| anyhow::anyhow!("Save an OpenRouter token in Settings → AI"))?;
		request = request.set("Authorization", &format!("Bearer {}", token.trim()));
	}
	let response = request.send_json(body).map_err(|error| {
		let status = match error {
			ureq::Error::Status(code, _) => format!(" (HTTP {code})"),
			ureq::Error::Transport(_) => String::new(),
		};
		anyhow::anyhow!("{what} model request failed{status}. Check {check} in Settings → AI.")
	})?;
	let mut raw = String::new();
	response
		.into_reader()
		.take(2 * 1024 * 1024)
		.read_to_string(&mut raw)?;
	let value: Value = serde_json::from_str(&raw)?;
	let content = if openrouter {
		value["choices"][0]["message"]["content"].as_str()
	} else {
		value["message"]["content"].as_str()
	}
	.ok_or_else(|| anyhow::anyhow!("{what} model returned no event JSON"))?;
	parse(content, what)
}

/// The events in a model's JSON answer, which must match [`schema`] exactly.
pub fn parse(content: &str, what: &str) -> anyhow::Result<Vec<Found>> {
	#[derive(Deserialize)]
	#[serde(deny_unknown_fields)]
	struct Response {
		events: Vec<Found>,
	}
	let response: Response = serde_json::from_str(content).map_err(|_| {
		anyhow::anyhow!(
			"{what} model returned invalid event JSON. Try again or choose another model."
		)
	})?;
	anyhow::ensure!(
		response.events.len() <= MAX_EVENTS,
		"Too many events at once (maximum {MAX_EVENTS})"
	);
	Ok(response.events)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn answers_must_match_the_schema() {
		let events =
			parse(r#"{"events":[{"title":"Talk","date":"2026-11-17","end_date":null,"start_time":"18:00","end_time":null,"description":null,"location":"Library"}]}"#, "Text")
				.unwrap();
		assert_eq!(events[0].title, "Talk");
		assert_eq!(events[0].start_time.as_deref(), Some("18:00"));
		assert!(parse(r#"{"events":[]}"#, "Text").unwrap().is_empty());
		for raw in [
			"",
			"not json",
			r#"{"events":[],"unexpected":true}"#,
			r#"{"events":[{"date":"2026-11-17"}]}"#,
			r#"{"events":[{"title":"Talk","extra":1}]}"#,
		] {
			assert!(parse(raw, "Text").is_err(), "{raw}");
		}
		let many = json!({"events":vec![json!({"title":"x"}); MAX_EVENTS + 1]}).to_string();
		assert!(parse(&many, "Text").is_err());
	}
}
