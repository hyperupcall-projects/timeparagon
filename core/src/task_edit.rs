//! Task editing shared by both apps. Dated tasks are native Calendar events;
//! undated tasks live in Google Tasks. Every change is shown locally at once
//! and sent to Google straight away (see `calendar::operations`).

use crate::calendar::google::GoogleError;
use crate::calendar::ids;
use chrono::{DateTime, Local, NaiveDate, TimeZone};

use crate::calendar::{self, Event};
use crate::config::{CalendarSettings, TriageRequirements};
use crate::timeblocking::{BacklogSection, Priority, UNDATED, block_key, metadata};

/// The longest name Google Tasks accepts.
pub const MAX_TITLE_CHARS: usize = 1024;

/// How a write gets its Google access token. The iced app passes its own, so
/// its unit tests can refuse to reach a real account.
pub type Access = fn(&CalendarSettings) -> Result<String, GoogleError>;

/// The token from the stored sign-in, refreshed if it has expired.
pub fn google_access(settings: &CalendarSettings) -> Result<String, GoogleError> {
    if !calendar::operations::replaying() {
        let tokens = calendar::oauth::load_tokens()
            .ok_or_else(|| GoogleError::auth("Not connected to Google"))?;
        return Ok(tokens.access_token);
    }
    calendar::GoogleClient::from_settings(settings).access_token()
}

/// The instants a task's due date and clock fields describe. An end before the
/// start is taken to run past midnight.
pub fn schedule_range(
    date: &str,
    start: &str,
    end: &str,
) -> Result<(DateTime<Local>, DateTime<Local>), String> {
    let day = calendar::deadline::parse_date(date.trim())?;
    let (sh, sm) = calendar::parse_clock(start).ok_or("Enter a start time as HH:MM")?;
    let (eh, em) = calendar::parse_clock(end).ok_or("Enter an end time as HH:MM")?;
    let start = Local
        .from_local_datetime(&day.and_hms_opt(sh, sm, 0).unwrap())
        .earliest()
        .ok_or("This start time does not exist in your time zone")?;
    let end_day = if (eh, em) < (sh, sm) {
        day.succ_opt().ok_or("Invalid end date")?
    } else {
        day
    };
    let end = Local
        .from_local_datetime(&end_day.and_hms_opt(eh, em, 0).unwrap())
        .earliest()
        .ok_or("This end time does not exist in your time zone")?;
    Ok((start, end))
}

/// What a task still needs before it can leave the Inbox, if anything.
pub fn triage_error(task: &Event, required: &TriageRequirements) -> Option<String> {
    let metadata = metadata(task);
    let mut missing = Vec::new();
    if required.estimate && metadata.minutes == 0 {
        missing.push("an estimate");
    }
    if required.project && metadata.projects.is_empty() {
        missing.push("a project");
    }
    if required.priority && metadata.priority.is_none() {
        missing.push("a priority");
    }
    match missing.as_slice() {
        [] => None,
        [only] => Some(format!("This task needs {only} before it can be triaged.")),
        [first, second] => Some(format!(
            "This task needs {first} and {second} before it can be triaged."
        )),
        [first, second, third] => Some(format!(
            "This task needs {first}, {second} and {third} before it can be triaged."
        )),
        _ => None,
    }
}

/// Trims a typed label and collapses runs of whitespace. Labels are short
/// enough to fit on a task card, so very long ones are rejected.
pub fn clean_label(input: &str) -> Result<String, &'static str> {
    let label = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if label.is_empty() {
        Err("A label needs a name")
    } else if label.chars().count() > 40 {
        Err("A label can be at most 40 characters")
    } else {
        Ok(label)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Change {
    Description(String),
    Title(String),
    Estimate(u32),
    Projects(Vec<u64>),
    Labels(Vec<String>),
    Priority(Option<Priority>),
    Location(Option<u64>),
    Deadline(Option<calendar::deadline::Deadline>),
    Backlog(Option<BacklogSection>),
    Inbox,
    BacklogSection(BacklogSection),
    /// Confirm a fully filled-in task and move it to the triaged list.
    Triaged,
    Unassign,
    Day(NaiveDate),
    ClearSchedule,
    Block(String, DateTime<Local>, DateTime<Local>),
    Reorder(Option<String>, i64, NaiveDate),
    Time(DateTime<Local>, DateTime<Local>),
    /// Automatic layout changes times without changing membership or estimate.
    Reflow(DateTime<Local>, DateTime<Local>),
    /// Repeat with a recurring time block, from its occurrence starting at
    /// `from`. The rule is read from Google when the change is sent.
    JoinBlockSeries {
        calendar: String,
        series: String,
        from: DateTime<Local>,
    },
}

impl Change {
    /// Joining the series of `block`, when it is a recurring time block.
    pub fn join_block_series(block: &Event) -> Option<Self> {
        Some(Self::JoinBlockSeries {
            calendar: block.calendar_id.clone(),
            series: block.details.recurring_event_id.clone()?,
            from: block.start,
        })
    }
}

/// A recurring copy's source, and the block series a task repeats with,
/// belong to its current block only.
pub(crate) fn reset_repeat_source(metadata: &mut crate::timeblocking::Metadata, change: &Change) {
    let moved = match change {
        Change::Block(key, _, _) => metadata.block.as_ref() != Some(key),
        Change::Reorder(block, _, _) => block != &metadata.block,
        Change::Unassign
        | Change::Day(_)
        | Change::ClearSchedule
        | Change::Backlog(_)
        | Change::Inbox
        | Change::Triaged
        | Change::Time(_, _) => true,
        _ => false,
    };
    if moved {
        metadata.repeat_source = None;
        metadata.block_series = None;
    }
}

#[allow(clippy::too_many_arguments)]
pub fn save_and_reflow(
    task: Event,
    settings: CalendarSettings,
    change: Change,
    order: i64,
    blocks: Vec<Event>,
    mut related: Vec<Event>,
    access: Access,
) -> Result<Vec<Event>, String> {
    let original_id = task.id.clone();
    let old_block = metadata(&task).block;
    let saved = save(task, &settings, change, order, access)?;
    let new_block = metadata(&saved).block;
    related.retain(|e| e.id != original_id);
    related.push(saved.clone());
    let mut changed = vec![saved];
    for block in blocks.iter().filter(|b| {
        let key = block_key(b);
        old_block.as_ref() == Some(&key) || new_block.as_ref() == Some(&key)
    }) {
        for event in reflow(block, &related, &settings, access)? {
            if let Some(previous) = changed.iter_mut().find(|e| e.id == event.id) {
                *previous = event;
            } else {
                changed.push(event);
            }
        }
    }
    Ok(changed)
}

/// Lays a block's tasks end to end from its start, rewriting those that moved.
pub fn reflow(
    block: &Event,
    tasks: &[Event],
    settings: &CalendarSettings,
    access: Access,
) -> Result<Vec<Event>, String> {
    let key = block_key(block);
    let mut tasks: Vec<_> = tasks
        .iter()
        .filter(|e| metadata(e).block.as_deref() == Some(&key))
        .cloned()
        .collect();
    tasks.sort_by_key(|e| (metadata(e).order, e.id.clone()));
    let mut start = block.start;
    let mut changed = Vec::new();
    let mut token = None;
    for task in tasks {
        let minutes = metadata(&task).minutes;
        let end =
            start + chrono::Duration::minutes(i64::from(if minutes == 0 { 30 } else { minutes }));
        // Completed blocks are history and are never shifted by later planning.
        if !task.completed && (task.all_day || task.start != start || task.end != end) {
            let token = match &token {
                Some(token) => token,
                None => token.insert(access(settings)?),
            };
            let mut database = crate::task_db::open()?;
            let mut task = task;
            let mut data = database.import(&task)?;
            schedule(
                &mut task,
                &mut data,
                &mut database,
                settings,
                token,
                start,
                end,
            )?;
            changed.push(task);
        }
        start = end;
    }
    Ok(changed)
}

pub fn save(
    task: Event,
    settings: &CalendarSettings,
    change: Change,
    order: i64,
    access: Access,
) -> Result<Event, GoogleError> {
    if !calendar::operations::replaying() {
        return Ok(calendar::operations::edit_task(
            task, settings, change, order,
        )?);
    }
    let mut database = crate::task_db::open()?;
    let data = database.import(&task)?;
    let mut original = task;
    original.details.task_data = Some(data);
    let mut saved = calendar::calendar_tasks::apply(original.clone(), settings, &change, order)?;
    let token = access(settings)?;
    if let Change::JoinBlockSeries {
        calendar,
        series,
        from,
    } = &change
        && saved.details.recurring_event_id.is_none()
    {
        saved.details.recurrence =
            calendar::api::series_recurrence_from(&token, calendar, series, *from)?;
    }
    calendar::calendar_tasks::persist(&token, &original, &mut saved, settings)?;
    database.save(
        &saved,
        saved
            .details
            .task_data
            .as_ref()
            .ok_or("Missing task data")?,
    )?;
    Ok(saved)
}

pub fn task_url(task: &Event) -> Result<String, String> {
    let task = ids::TaskRef::of(task).ok_or("Invalid Google Task")?;
    Ok(format!(
        "{}/{}/tasks/{}",
        calendar::api::TASKS,
        urlencoding::encode(task.list),
        urlencoding::encode(task.task)
    ))
}

pub fn schedule(
    task: &mut Event,
    data: &mut crate::task_db::TaskData,
    _database: &mut crate::task_db::Database,
    settings: &CalendarSettings,
    _token: &str,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<(), String> {
    task.details.task_data = Some(data.clone());
    *task = save(
        task.clone(),
        settings,
        Change::Reflow(start, end),
        data.metadata.order,
        google_access,
    )
    .map_err(String::from)?;
    *data = task.details.task_data.clone().ok_or("Missing task data")?;
    Ok(())
}

/// Writes a new task to the inbox list and builds the local [`Event`] for it.
///
/// Blocking: callers run this on a worker thread.
pub fn create_inbox_task(
    settings: &CalendarSettings,
    token: &str,
    title: &str,
    notes: &str,
    due: Option<chrono::NaiveDate>,
    metadata: Option<crate::timeblocking::Metadata>,
) -> Result<Event, String> {
    if !calendar::operations::replaying() {
        return calendar::operations::create_task(settings, title, notes, due, metadata);
    }
    let list = calendar::scheduled::inbox_destination(token, settings.inbox_list.as_deref())?;
    let created_at = crate::clock::utc_now();
    let created = calendar::api::insert_task_with(
        token,
        &list,
        &calendar::api::NewTask { title, notes, due },
    )?;

    let calendar_id = ids::calendar_id(&list);
    let start = calendar::at_clock(due.unwrap_or(UNDATED), 0, 0);
    let mut event = Event {
        id: format!("task:{}", created.id),
        color: settings
            .task_color(&calendar_id)
            .unwrap_or(calendar::colors::TASK),
        calendar_id,
        title: created.title,
        start,
        end: start + chrono::Duration::days(1),
        all_day: true,
        completed: false,
        details: calendar::Details {
            task_updated_at: created.updated_at,
            link: created.link,
            description: notes.to_owned(),
            ..Default::default()
        },
    };
    // Allocate a stable ID and publish its metadata immediately.
    let mut database = crate::task_db::open()?;
    let mut data = database.import(&event)?;
    if let Some(metadata) = metadata {
        data.metadata = metadata;
    }
    data.metadata.created_at = Some(created_at);
    // Keep the creation instant retryable if the metadata upload fails.
    database.save(&event, &data)?;
    calendar::task_metadata::save(token, &event, &data)?;
    event.details.task_data = Some(data);
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recurring_source_is_cleared_on_detach_or_another_block_but_not_reordering() {
        let mut meta = crate::timeblocking::Metadata {
            repeat_source: Some(123),
            block: Some("app/block".into()),
            ..Default::default()
        };
        reset_repeat_source(
            &mut meta,
            &Change::Reorder(Some("app/block".into()), 10, UNDATED),
        );
        assert_eq!(meta.repeat_source, Some(123));
        for change in [
            Change::Unassign,
            Change::Backlog(None),
            Change::Reorder(Some("app/other".into()), 10, UNDATED),
        ] {
            meta.repeat_source = Some(123);
            reset_repeat_source(&mut meta, &change);
            assert_eq!(meta.repeat_source, None);
        }
    }

    #[test]
    fn a_schedule_range_reads_its_fields_and_can_run_past_midnight() {
        let (start, end) = schedule_range(" 2026-10-26 ", "09:00", "09:30").unwrap();
        assert_eq!(start.date_naive().to_string(), "2026-10-26");
        assert_eq!((end - start).num_minutes(), 30);

        let (start, end) = schedule_range("2026-10-26", "23:30", "00:15").unwrap();
        assert_eq!(end.date_naive().to_string(), "2026-10-27");
        assert_eq!((end - start).num_minutes(), 45);

        assert!(schedule_range("tomorrow", "09:00", "09:30").is_err());
        assert!(schedule_range("2026-10-26", "9am", "09:30").is_err());
        assert!(schedule_range("2026-10-26", "09:00", "").is_err());
    }
}
