//! Scheduled tasks are Calendar events. Google Tasks holds only undated work.
use super::{Event, api, ids, scheduled};
use crate::{
    config::CalendarSettings,
    task_edit::{Change, reset_repeat_source},
    timeblocking as tb,
};
use chrono::{Duration, Local, TimeZone};
use serde_json::{Value, json};

pub fn event_url(event: &Event) -> String {
    format!(
        "{}/{}/events/{}",
        api::EVENTS,
        urlencoding::encode(&event.calendar_id),
        urlencoding::encode(&event.id)
    )
}

pub fn body(task: &Event) -> Result<Value, String> {
    let data = task
        .details
        .task_data
        .as_ref()
        .ok_or("Missing task metadata")?;
    let mut body = json!({
        "summary": task.title, "description": task.details.description,
        "extendedProperties": {"private": {
            "ttKind":"task",
            "ttCompleted":task.completed.to_string(),
            "ttLabels":serde_json::to_string(&task.details.labels).map_err(|e|e.to_string())?
        }}
    });
    write_data(&mut body, data)?;
    super::projects::write(&mut body, &task.details.projects)?;
    task.details.options.write(&mut body)?;
    if task.all_day {
        body["start"] = json!({"date":task.start.date_naive().to_string()});
        body["end"] = json!({"date":task.end.date_naive().to_string()});
    } else {
        body["start"] = json!({"dateTime":task.start.to_rfc3339(),"timeZone":iana_time_zone::get_timezone().map_err(|e|e.to_string())?});
        body["end"] = json!({"dateTime":task.end.to_rfc3339(),"timeZone":iana_time_zone::get_timezone().map_err(|e|e.to_string())?});
    }
    if task.details.recurring_event_id.is_none() {
        body["recurrence"] = json!(task.details.recurrence);
    }
    Ok(body)
}

pub fn apply(
    mut task: Event,
    settings: &CalendarSettings,
    change: &Change,
    order: i64,
) -> Result<Event, String> {
    let mut data = task
        .details
        .task_data
        .take()
        .ok_or("Missing task metadata")?;
    reset_repeat_source(&mut data.metadata, change);
    match change {
        Change::ReleaseInheritance => data.metadata.inherited = None,
        Change::Inherit(value) => tb::apply_inherited(&mut task, &mut data, value.clone()),
        Change::Title(v) => task.title = v.clone(),
        Change::Description(v) => task.details.description = v.clone(),
        Change::Estimate(v) => {
            data.metadata.minutes = *v;
            if !task.all_day && *v > 0 {
                task.end = task.start + Duration::minutes(i64::from(*v));
            }
        }
        Change::Projects(v) => data.metadata.projects = v.clone(),
        Change::Labels(v) => data.metadata.labels = v.clone(),
        Change::Priority(v) => data.metadata.priority = *v,
        Change::Location(v) => data.metadata.location = *v,
        Change::Deadline(v) => data.metadata.deadline = *v,
        Change::BacklogSection(v) => data.metadata.backlog_section = *v,
        Change::Time(start, end) | Change::Reflow(start, end) | Change::Block(_, start, end) => {
            if let Change::Block(key, _, _) = change {
                data.metadata.block = Some(key.clone());
                data.metadata.order = order;
                task.end = *start
                    + Duration::minutes(i64::from(if data.metadata.minutes == 0 {
                        30
                    } else {
                        data.metadata.minutes
                    }));
            } else {
                if matches!(change, Change::Time(..)) {
                    data.metadata.block = None;
                    data.metadata.minutes = (*end - *start).num_minutes().clamp(0, 10080) as u32;
                }
                task.end = *end;
            }
            task.start = *start;
            task.all_day = false;
        }
        Change::Reorder(block, order, day) => {
            data.metadata.block = block.clone();
            data.metadata.order = *order;
            if task.start.date_naive() == tb::UNDATED {
                task.start = Local
                    .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
                    .earliest()
                    .ok_or("Invalid date")?;
                task.end = task.start + Duration::days(1);
            }
        }
        Change::Day(_)
        | Change::Unassign
        | Change::ClearSchedule
        | Change::Inbox
        | Change::Triaged
        | Change::Backlog(_) => {
            let day = match change {
                Change::Day(day) => *day,
                Change::Unassign => task.start.date_naive(),
                _ => tb::UNDATED,
            };
            task.start = Local
                .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
                .earliest()
                .ok_or("Invalid local date")?;
            task.end = task.start + Duration::days(1);
            task.all_day = true;
            data.metadata.block = None;
            data.metadata.inherited = None;
            if let Change::Backlog(Some(section)) = change {
                data.metadata.backlog_section = *section;
            }
            if day == tb::UNDATED {
                let list = match change {
                    Change::Triaged => settings.triaged_list.as_deref(),
                    Change::Backlog(_) => settings.backlog_list.as_deref(),
                    _ => settings.inbox_list.as_deref(),
                };
                if task.task_ref().is_none() {
                    task.id = ids::event_id(&format!("tt{:032x}", data.id));
                }
                task.calendar_id = ids::calendar_id(list.unwrap_or(match change {
                    Change::Triaged => "local-triaged",
                    Change::Backlog(_) => "local-backlog",
                    _ => "local-inbox",
                }));
            }
        }
    }
    if task.start.date_naive() != tb::UNDATED && task.task_ref().is_some() {
        use sha2::{Digest, Sha256};
        task.id = format!(
            "tt{:x}",
            Sha256::digest(format!("{}/{}", data.id, task.id).as_bytes())
        )[..34]
            .to_owned();
        task.calendar_id = settings
            .app_calendar
            .clone()
            .unwrap_or_else(|| "local-calendar".into());
        task.details.link.clear();
    }
    task.details.projects = data.metadata.projects.clone();
    task.details.labels = data.metadata.labels.clone();
    task.details.task_data = Some(data);
    Ok(task)
}

pub fn remove(token: &str, url: &str) -> Result<(), String> {
    match scheduled::request("DELETE", url, token, None) {
        Ok(_) => Ok(()),
        Err(e) if matches!(e.status, Some(404 | 410)) => Ok(()),
        Err(e) => Err(e.message),
    }
}

pub fn persist(
    token: &str,
    original: &Event,
    saved: &mut Event,
    settings: &CalendarSettings,
) -> Result<(), String> {
    if saved.task_ref().is_none() {
        if saved.calendar_id == "local-calendar" {
            saved.calendar_id = settings
                .app_calendar
                .clone()
                .map(Ok)
                .unwrap_or_else(|| scheduled::app_calendar(token))?;
        }
        scheduled::upsert_at(
            token,
            &event_url(saved),
            &format!(
                "{}/{}/events",
                api::EVENTS,
                urlencoding::encode(&saved.calendar_id)
            ),
            &saved.id,
            body(saved)?,
        )?;
        if original.task_ref().is_some() {
            remove(token, &crate::task_edit::task_url(original)?)?;
        }
    } else {
        let list = match saved.calendar_id.as_str() {
            "tasks:local-triaged" => scheduled::find_or_create(
                token,
                api::TASK_LISTS,
                api::TASK_LISTS,
                "title",
                CalendarSettings::DEFAULT_TRIAGED_LIST,
            )?,
            "tasks:local-backlog" => scheduled::find_or_create(
                token,
                api::TASK_LISTS,
                api::TASK_LISTS,
                "title",
                CalendarSettings::DEFAULT_BACKLOG_LIST,
            )?,
            "tasks:local-inbox" => {
                scheduled::inbox_destination(token, settings.inbox_list.as_deref())?
            }
            _ => ids::bare_list(&saved.calendar_id).to_owned(),
        };
        let data = saved
            .details
            .task_data
            .as_ref()
            .ok_or("Missing task metadata")?;
        let notes = super::task_metadata::write(&saved.details.description, data)?;
        if original.task_ref().is_none() || original.calendar_id != ids::calendar_id(&list) {
            let existing = api::fetch_task_list(token, &list)?.into_iter().find(|t| {
                super::task_metadata::read(&t.notes)
                    .ok()
                    .flatten()
                    .is_some_and(|d| d.id == data.id)
            });
            let id = if let Some(t) = existing {
                t.id
            } else {
                scheduled::request("POST",&format!("{}/{}/tasks",api::TASKS,urlencoding::encode(&list)),token,Some(&json!({"title":saved.title,"notes":notes,"status":if saved.completed {"completed"} else {"needsAction"}}))).map_err(|e|e.message)?["id"].as_str().ok_or("Missing created task ID")?.to_owned()
            };
            saved.id = ids::event_id(&id);
            saved.calendar_id = ids::calendar_id(&list);
            if original.task_ref().is_none() {
                remove(token, &event_url(original))?;
            } else {
                remove(token, &crate::task_edit::task_url(original)?)?;
            }
        } else {
            api::patch(&crate::task_edit::task_url(saved)?,token,&json!({"title":saved.title,"notes":notes,"due":null,"status":if saved.completed {"completed"} else {"needsAction"}}).to_string(),"Save task").map_err(String::from)?;
        }
    }
    Ok(())
}

/// Chunk metadata below Google's per-property size limit, including UTF-8 text.
pub fn write_data(body: &mut Value, data: &crate::task_db::TaskData) -> Result<(), String> {
    let raw = serde_json::to_string(data).map_err(|e| e.to_string())?;
    if raw.len() > 28000 {
        return Err("Task metadata exceeds Calendar’s extended property limit".into());
    }
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    for c in raw.chars() {
        if chunk.len() + c.len_utf8() > 900 {
            chunks.push(std::mem::take(&mut chunk));
        }
        chunk.push(c);
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    if chunks.len() > 200 {
        return Err("Task metadata exceeds Calendar's extended property limit".into());
    }
    body["extendedProperties"]["private"]["ttDataCount"] = chunks.len().to_string().into();
    for (i, chunk) in chunks.into_iter().enumerate() {
        body["extendedProperties"]["private"][format!("ttData{i}")] = chunk.into();
    }
    Ok(())
}
pub fn read_data(properties: &Value) -> Result<crate::task_db::TaskData, String> {
    let p = &properties["private"];
    let count = p["ttDataCount"]
        .as_str()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0 && *n <= 200)
        .ok_or("Invalid calendar task metadata count")?;
    let raw = (0..count)
        .map(|i| {
            p[format!("ttData{i}")]
                .as_str()
                .ok_or("Missing calendar task metadata chunk")
        })
        .collect::<Result<String, _>>()?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn task() -> Event {
        let mut task = super::super::sample_snapshot().events.remove(0);
        task.id = "task:original".into();
        task.calendar_id = "tasks:inbox".into();
        task.all_day = true;
        task.start = super::super::at_clock(tb::UNDATED, 0, 0);
        task.end = task.start + Duration::days(1);
        task.details = Default::default();
        task.details.description = "Human notes".into();
        task.details
            .task_data
            .get_or_insert_with(|| crate::task_db::TaskData {
                id: 100,
                metadata: Default::default(),
            })
            .metadata
            .deadline = Some(crate::calendar::deadline::parse_input("2030-11-04 17:00").unwrap());
        task.details.task_data = Some(crate::task_db::TaskData {
            id: 123,
            metadata: tb::Metadata {
                deadline: Some(super::super::deadline::parse_input("2030-11-04 17:00").unwrap()),
                minutes: 45,
                projects: vec![7],
                labels: vec!["work".into()],
                ..Default::default()
            },
        });
        task
    }
    fn settings() -> CalendarSettings {
        CalendarSettings {
            app_calendar: Some("app".into()),
            inbox_list: Some("tasks:inbox".into()),
            ..Default::default()
        }
    }
    #[test]
    fn deadline_and_description_are_independent_and_round_trip_in_event_metadata() {
        let original = task();
        let expected = original
            .details
            .task_data
            .as_ref()
            .unwrap()
            .metadata
            .deadline;
        let prose = "DEADLINE: this is user text\nDUE: also user text\nPRIORITY: my notes";
        let edited = apply(original, &settings(), &Change::Description(prose.into()), 0).unwrap();
        assert_eq!(edited.details.description, prose);
        assert_eq!(super::super::deadline::get(&edited), expected);
        let body = body(&edited).unwrap();
        assert_eq!(body["description"], prose);
        let data = read_data(&body["extendedProperties"]).unwrap();
        assert_eq!(data.metadata.deadline, expected);
        let changed = apply(edited, &settings(), &Change::Deadline(None), 0).unwrap();
        assert_eq!(changed.details.description, prose);
        assert_eq!(super::super::deadline::get(&changed), None);
    }

    #[test]
    fn assigning_any_day_creates_a_calendar_task_with_all_metadata() {
        let task = task();
        let day = chrono::NaiveDate::from_ymd_opt(2030, 11, 4).unwrap();
        let saved = apply(task.clone(), &settings(), &Change::Day(day), 0).unwrap();
        assert!(saved.is_calendar_task());
        assert!(saved.task_ref().is_none());
        assert!(saved.all_day);
        assert_eq!(saved.calendar_id, "app");
        assert_eq!(saved.start.date_naive(), day);
        assert_eq!(saved.details.description, task.details.description);
        assert_eq!(saved.details.projects, [7]);
        assert_eq!(saved.details.labels, ["work"]);
        assert_eq!(saved.details.task_data.as_ref().unwrap().id, 123);
        assert_eq!(
            apply(task, &settings(), &Change::Day(day), 0).unwrap().id,
            saved.id
        );
    }
    #[test]
    fn timed_tasks_write_native_recurrence_and_plain_notes() {
        let start =
            super::super::at_clock(chrono::NaiveDate::from_ymd_opt(2030, 11, 4).unwrap(), 9, 0);
        let mut saved = apply(
            task(),
            &settings(),
            &Change::Time(start, start + Duration::minutes(45)),
            0,
        )
        .unwrap();
        saved.details.recurrence = vec!["RRULE:FREQ=WEEKLY;BYDAY=MO".into()];
        let body = body(&saved).unwrap();
        assert_eq!(body["recurrence"][0], "RRULE:FREQ=WEEKLY;BYDAY=MO");
        assert!(body["start"]["timeZone"].is_string());
        assert_eq!(body["summary"], saved.title);
        assert_eq!(body["description"], saved.details.description);
        assert_eq!(body["extendedProperties"]["private"]["ttProjects"], "[7]");
        assert!(
            body["extendedProperties"]["private"]
                .get("ttTask")
                .is_none()
        );
        assert_eq!(
            read_data(&body["extendedProperties"]).unwrap(),
            saved.details.task_data.unwrap()
        );
    }
    #[test]
    fn occurrence_updates_do_not_rewrite_series_rules() {
        let mut saved = task();
        saved.details.recurring_event_id = Some("series".into());
        saved.details.recurrence = vec!["RRULE:FREQ=DAILY".into()];
        assert!(body(&saved).unwrap().get("recurrence").is_none());
    }
    #[test]
    fn metadata_chunks_preserve_large_unicode_and_reject_missing_chunks() {
        let mut saved = task();
        saved.details.task_data.as_mut().unwrap().metadata.labels = vec!["🌈".repeat(1500)];
        let body = body(&saved).unwrap();
        let mut p = body["extendedProperties"].clone();
        assert_eq!(read_data(&p).unwrap(), saved.details.task_data.unwrap());
        for (key, value) in p["private"].as_object().unwrap() {
            if key.starts_with("ttData") {
                assert!(value.as_str().unwrap().len() <= 900);
            }
        }
        p["private"].as_object_mut().unwrap().remove("ttData1");
        assert!(read_data(&p).is_err());
    }
    #[test]
    fn unslotting_retains_calendar_ownership_and_only_clearing_the_day_returns_to_tasks() {
        let day = chrono::NaiveDate::from_ymd_opt(2030, 11, 4).unwrap();
        let start = super::super::at_clock(day, 9, 0);
        let scheduled = apply(
            task(),
            &settings(),
            &Change::Time(start, start + Duration::minutes(45)),
            0,
        )
        .unwrap();
        let day_only = apply(scheduled.clone(), &settings(), &Change::Unassign, 0).unwrap();
        assert!(day_only.is_calendar_task());
        assert!(day_only.all_day);
        assert_eq!(day_only.id, scheduled.id);
        let inbox = apply(scheduled, &settings(), &Change::Inbox, 0).unwrap();
        assert!(inbox.task_ref().is_some());
        assert_eq!(inbox.start.date_naive(), tb::UNDATED);
        assert_eq!(inbox.details.task_data.unwrap().id, 123);
    }
    #[test]
    fn calendar_task_labels_projects_and_notes_share_one_calendar_body() {
        let day = chrono::NaiveDate::from_ymd_opt(2030, 11, 4).unwrap();
        let mut task = apply(task(), &settings(), &Change::Day(day), 0).unwrap();
        for change in [
            Change::Projects(vec![42]),
            Change::Labels(vec!["new".into()]),
            Change::Description("Updated notes".into()),
        ] {
            task = apply(task, &settings(), &change, 0).unwrap();
        }
        let body = body(&task).unwrap();
        assert_eq!(body["extendedProperties"]["private"]["ttProjects"], "[42]");
        assert_eq!(
            body["extendedProperties"]["private"]["ttLabels"],
            "[\"new\"]"
        );
        assert!(
            body["description"]
                .as_str()
                .unwrap()
                .contains("Updated notes")
        );
        assert!(super::super::deadline::get(&task).is_some());
    }
}
