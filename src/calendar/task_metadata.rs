//! Remote task metadata. SQLite is a cache; Google Tasks notes are authoritative.
//! Values are JSON, one namespaced key per line, after a terminal `---`.
use crate::task_db::TaskData;
use serde_json::{Map, Value, json};

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
            .ok_or("Malformed TimeSlay metadata line")?;
        let key = key
            .strip_prefix("tt-")
            .ok_or("Unexpected line in TimeSlay metadata")?;
        if fields
            .insert(
                key.to_owned(),
                serde_json::from_str(value)
                    .map_err(|e| format!("Invalid TimeSlay metadata: {e}"))?,
            )
            .is_some()
        {
            return Err(format!("Duplicate TimeSlay metadata key: {key}"));
        }
    }
    if fields.remove("version") != Some(json!(1)) {
        return Err("Unsupported TimeSlay metadata version".into());
    }
    if let Some(value) = fields.remove("deadline") {
        if !value.is_null() {
            super::deadline::parse_input(value.as_str().ok_or("Invalid metadata deadline")?)?;
        }
    }
    let id = fields.remove("id").ok_or("Missing TimeSlay task ID")?;
    let legacy_key = fields.remove("legacy_key").unwrap_or(Value::Null);
    let allowed = [
        "backlog_section",
        "block",
        "minutes",
        "order",
        "ignored",
        "schedule",
        "projects",
        "project",
        "labels",
        "priority",
        "location",
        "created_at",
    ];
    if fields.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("Unknown TimeSlay metadata field; update the app before editing".into());
    }
    let data: TaskData =
        serde_json::from_value(json!({"id":id,"legacy_key":legacy_key,"metadata":fields}))
            .map_err(|e| format!("Invalid TimeSlay metadata: {e}"))?;
    if data.id < 100 {
        return Err("Invalid TimeSlay task ID".into());
    }
    Ok(Some(data))
}

pub fn write(notes: &str, data: &TaskData) -> Result<String, String> {
    // Validate before replacing: never erase malformed or future-version data.
    read(notes)?;
    let notes = crate::timeblocking::description(notes);
    let deadline = super::deadline::read_deadline(&notes)?;
    let prose = super::notes::Notes::new(&notes).without(&["DUE:", "DEADLINE:"]);
    let mut result = if prose.is_empty() {
        String::new()
    } else {
        format!("{prose}\n")
    };
    result.push_str("---\ntt-version: 1\n");
    result.push_str(&format!(
        "tt-id: {}\ntt-legacy_key: {}",
        data.id,
        json!(data.legacy_key)
    ));
    result.push_str(&format!(
        "\ntt-deadline: {}",
        json!(deadline.map(|d| d.value()))
    ));
    let fields = serde_json::to_value(&data.metadata).map_err(|e| e.to_string())?;
    for (key, value) in fields.as_object().expect("metadata object") {
        result.push_str(&format!("\ntt-{key}: {value}"));
    }
    if result.chars().count() > 8192 {
        return Err("Task notes and metadata exceed Google Tasks' 8192 character limit".into());
    }
    Ok(result)
}

/// Resolve a fetched task, migrating legacy sources exactly once. The callback
/// is the remote write boundary so a failed export leaves legacy data retryable.
pub fn load(
    database: &mut crate::task_db::Database,
    event: &super::Event,
    mut publish: impl FnMut(&TaskData) -> Result<(), String>,
) -> Result<TaskData, String> {
    if let Some(mut data) = read(&event.details.description)? {
        // Tasks.move preserves notes but may change the native task ID. Repair
        // the link from the returned resource after an interrupted move.
        if let Some(link) = &mut data.metadata.schedule {
            let list = event.calendar_id.trim_start_matches("tasks:");
            let id = event.id.trim_start_matches("task:");
            if link.list_id != list || link.task_id != id {
                link.list_id = list.to_owned();
                link.task_id = id.to_owned();
                publish(&data)?;
            }
        }
        database.cache_remote(event, &data)?;
        return Ok(data);
    }
    // LEGACY_METADATA_COMPAT: SQLite and inline tt-timeblocking/ttid values
    // are fallback sources only. Never consult them when a footer exists.
    let data = database.import(event)?;
    publish(&data)?;
    Ok(data)
}

/// Write through to Google before publishing a cache update.
pub fn save(token: &str, event: &super::Event, data: &TaskData) -> Result<(), String> {
    let list = event
        .calendar_id
        .strip_prefix("tasks:")
        .ok_or("Invalid task list")?;
    let id = event.id.strip_prefix("task:").ok_or("Invalid task ID")?;
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
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event() -> super::super::Event {
        let mut event = super::super::sample_snapshot().events.remove(0);
        event.id = "task:one".into();
        event.calendar_id = "tasks:list".into();
        event.details = Default::default();
        event
    }

    #[test]
    fn legacy_sqlite_is_exported_retryably_then_remote_wins_and_rebuilds_a_fresh_cache() {
        let mut event = event();
        event.details.description = "Human notes\ntt-timeblocking: {\"minutes\":30}\nttid: 12345678-1234-4234-8234-123456789abc".into();
        let mut db = crate::task_db::Database::open(std::path::Path::new(":memory:")).unwrap();
        let mut local = db.import(&event).unwrap();
        local.metadata.minutes = 75;
        db.save(&event, &local).unwrap();
        assert!(load(&mut db, &event, |_| Err("offline".into())).is_err());
        assert_eq!(db.lookup(&event).unwrap(), Some(local.clone()));
        let mut exported = String::new();
        let data = load(&mut db, &event, |data| {
            exported = write(&event.details.description, data)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(data, local);
        assert!(!exported.contains("tt-timeblocking:"));
        assert!(!exported.contains("ttid:"));
        assert_eq!(prose(&exported), "Human notes");
        // An edit on another device is authoritative over the old cache.
        local.metadata.minutes = 90;
        event.details.description = write(&exported, &local).unwrap();
        let loaded = load(&mut db, &event, |_| panic!("already migrated")).unwrap();
        assert_eq!(loaded, local);
        assert_eq!(db.lookup(&event).unwrap(), Some(local.clone()));
        let mut fresh = crate::task_db::Database::open(std::path::Path::new(":memory:")).unwrap();
        assert_eq!(
            load(&mut fresh, &event, |_| panic!("no export needed")).unwrap(),
            local
        );
        assert_eq!(fresh.import(&event).unwrap(), local);
    }

    #[test]
    fn deadlines_migrate_to_footer_and_remain_editable_without_metadata_loss() {
        let data = TaskData {
            id: 100,
            legacy_key: None,
            metadata: Default::default(),
        };
        let notes = "Human notes\nDEADLINE: 2026-10-30 17:05\nMore notes";
        let remote = write(notes, &data).unwrap();
        assert_eq!(prose(&remote), "Human notes\nMore notes");
        assert!(remote.contains("tt-deadline: \"2026-10-30 17:05\""));
        assert_eq!(
            super::super::deadline::read_deadline(&remote).unwrap(),
            super::super::deadline::read_deadline(notes).unwrap()
        );
        let display = crate::timeblocking::description(&remote);
        let cleared = super::super::deadline::write(&display, None);
        let remote = write(&cleared, &data).unwrap();
        assert_eq!(
            super::super::deadline::read_deadline(&remote).unwrap(),
            None
        );
        assert_eq!(read(&remote).unwrap(), Some(data));
    }

    #[test]
    fn interrupted_task_move_repairs_the_remote_schedule_identity() {
        let mut event = event();
        let mut data = TaskData {
            id: 100,
            legacy_key: None,
            metadata: Default::default(),
        };
        data.metadata.schedule =
            Some(super::super::scheduled::Link::new("cal".into(), &event).unwrap());
        event.details.description = write("Notes", &data).unwrap();
        event.id = "task:moved".into();
        event.calendar_id = "tasks:destination".into();
        let mut db = crate::task_db::Database::open(std::path::Path::new(":memory:")).unwrap();
        let mut published = None;
        let loaded = load(&mut db, &event, |data| {
            published = Some(data.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(published, Some(loaded.clone()));
        let link = loaded.metadata.schedule.unwrap();
        assert_eq!(link.task_id, "moved");
        assert_eq!(link.list_id, "destination");
    }

    #[test]
    fn all_metadata_round_trips_and_prose_rules_are_preserved() {
        let data = TaskData {
            id: 123,
            legacy_key: Some("old-history-key".into()),
            metadata: crate::timeblocking::Metadata {
                created_at: Some(
                    chrono::DateTime::parse_from_rfc3339("2026-10-29T19:30:12.123Z")
                        .unwrap()
                        .to_utc(),
                ),
                backlog_section: crate::timeblocking::BacklogSection::NextQuarter,
                minutes: 45,
                block: Some("calendar/block".into()),
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
            legacy_key: None,
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
