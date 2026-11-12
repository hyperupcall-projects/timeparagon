//! Remote task metadata. SQLite is a cache; Google Tasks notes are authoritative.
//! Values are JSON, one namespaced key per line, after a terminal `---`.
use crate::calendar::ids;
use crate::task_db::TaskData;
use serde_json::{Map, json};

const MARKER: &str = "tt-version: ";

fn footer(notes: &str) -> Option<(&str, &str)> {
	// Only a namespaced terminal section is ours; ordinary Markdown rules stay prose.
	let (prose, tail) = notes
		.rsplit_once("\n---\n")
		.or_else(|| notes.strip_prefix("---\n").map(|tail| ("", tail)))?;
	tail.starts_with(MARKER).then_some((prose, tail))
}

pub fn prose(notes: &str) -> &str {
	footer(notes).map_or(notes, |(prose, _)| prose)
}

pub fn read(notes: &str) -> Result<Option<TaskData>, String> {
	let Some((_, tail)) = footer(notes) else {
		return Ok(None);
	};
	let mut fields = Map::new();
	for line in tail.lines() {
		let (key, value) = line
			.split_once(": ")
			.ok_or("Malformed TimeParagon metadata line")?;
		let key = key
			.strip_prefix("tt-")
			.ok_or("Unexpected line in TimeParagon metadata")?;
		if fields
			.insert(
				key.to_owned(),
				serde_json::from_str(value)
					.map_err(|e| format!("Invalid TimeParagon metadata: {e}"))?,
			)
			.is_some()
		{
			return Err(format!("Duplicate TimeParagon metadata key: {key}"));
		}
	}
	if fields.remove("version") != Some(json!(1)) {
		return Err("Unsupported TimeParagon metadata version".into());
	}
	let id = fields.remove("id").ok_or("Missing TimeParagon task ID")?;
	// Written by the removed time block metadata propagation; ignored now.
	fields.remove("inherited");
	let allowed = [
		"deadline",
		"repeat_source",
		"block_series",
		"backlog_section",
		"block",
		"minutes",
		"order",
		"ignored",
		"projects",
		"labels",
		"priority",
		"location",
		"created_at",
	];
	if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
		return Err("Unknown TimeParagon metadata field; update the app before editing".into());
	}
	let data: TaskData = serde_json::from_value(json!({"id":id,"metadata":fields}))
		.map_err(|e| format!("Invalid TimeParagon metadata: {e}"))?;
	if data.id < 100 {
		return Err("Invalid TimeParagon task ID".into());
	}
	Ok(Some(data))
}

pub fn write(notes: &str, data: &TaskData) -> Result<String, String> {
	// Validate before replacing: never erase malformed or future-version data.
	read(notes)?;
	let prose = prose(notes);
	let mut result = if prose.is_empty() {
		String::new()
	} else {
		format!("{prose}\n")
	};
	result.push_str("---\ntt-version: 1\n");
	result.push_str(&format!("tt-id: {}", data.id));
	let fields = serde_json::to_value(&data.metadata).map_err(|e| e.to_string())?;
	for (key, value) in fields.as_object().expect("metadata object") {
		result.push_str(&format!("\ntt-{key}: {value}"));
	}
	if result.chars().count() > 8192 {
		return Err("Task notes and metadata exceed Google Tasks' 8192 character limit".into());
	}
	Ok(result)
}

/// Resolve a fetched task, allocating metadata for newly encountered tasks.
/// The callback publishes metadata before it is treated as remotely saved.
pub fn load(
	database: &mut crate::task_db::Database,
	event: &super::Event,
	mut publish: impl FnMut(&TaskData) -> Result<(), String>,
) -> Result<TaskData, String> {
	if !super::operations::replaying()
		&& let Some(data) = database.pending_task_data(event)?
	{
		return Ok(data);
	}
	if let Some(data) = read(&event.details.description)? {
		database.cache_remote(event, &data)?;
		return Ok(data);
	}
	// New Google Tasks receive their first TimeParagon metadata on encounter.
	let data = database.import(event)?;
	publish(&data)?;
	Ok(data)
}

/// Write through to Google before publishing a cache update.
pub fn save(token: &str, event: &super::Event, data: &TaskData) -> Result<(), String> {
	if event.is_calendar_task() {
		let mut event = event.clone();
		event.details.task_data = Some(data.clone());
		return super::api::patch(
			&super::calendar_tasks::event_url(&event),
			token,
			&super::calendar_tasks::body(&event)?.to_string(),
			"Save calendar task",
		)
		.map_err(String::from);
	}
	let list = event
		.calendar_id
		.strip_prefix(ids::TASK_LIST)
		.ok_or("Invalid task list")?;
	let id = event.id.strip_prefix(ids::TASK).ok_or("Invalid task ID")?;
	let url = format!(
		"{}/{}/tasks/{}",
		super::api::TASKS,
		urlencoding::encode(list),
		urlencoding::encode(id)
	);
	let notes = write(&event.details.description, data)?;
	super::api::patch(
		&url,
		token,
		&json!({"notes":notes}).to_string(),
		"Save task metadata",
	)
	.map_err(String::from)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn all_metadata_round_trips_and_prose_rules_are_preserved() {
		let data = TaskData {
			id: 123,
			metadata: crate::timeblocking::Metadata {
				deadline: Some(super::super::deadline::parse_input("2026-10-28 17:00").unwrap()),
				created_at: Some(
					chrono::DateTime::parse_from_rfc3339("2026-10-29T19:30:12.123Z")
						.unwrap()
						.to_utc(),
				),
				backlog_section: crate::timeblocking::BacklogSection::NextQuarter,
				minutes: 45,
				block: Some("calendar/block".into()),
				block_series: Some("calendar/series".into()),
				labels: vec!["two\nlines: yes".into()],
				projects: vec![1, 7],
				priority: Some(crate::timeblocking::Priority::P1),
				location: Some(4),
				..Default::default()
			},
		};
		let prose = "normal thing\n---\nordinary Markdown\n";
		let notes = write(prose, &data).unwrap();
		assert_eq!(read(&notes).unwrap(), Some(data.clone()));
		assert_eq!(super::super::task_metadata::prose(&notes), prose);
		assert_eq!(write(&notes, &data).unwrap(), notes);
		assert_eq!(read(prose).unwrap(), None);
	}
	#[test]
	fn malformed_future_and_oversize_notes_are_never_replaced() {
		let data = TaskData {
			id: 100,
			metadata: Default::default(),
		};
		for notes in [
			"---\ntt-version: 2",
			"---\ntt-version: 1\ntt-id: oops",
			"---\ntt-version: 1\ntt-id: 100\ntt-id: 101",
		] {
			assert!(write(notes, &data).is_err());
		}
		assert!(write(&"x".repeat(8192), &data).is_err());
	}
}
