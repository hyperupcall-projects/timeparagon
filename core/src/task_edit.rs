//! Writing changes to a Google Task, shared by both apps.
//!
//! Every function here blocks on the network, so run it on a worker thread. A
//! task's metadata, its Google Tasks entry and its scheduled calendar copy are
//! written together, in an order that a failure part way through can recover from.

use crate::calendar::google::GoogleError;
use crate::calendar::ids;
use chrono::{DateTime, Local, NaiveDate, TimeZone};

use crate::calendar::{self, Event};
use crate::config::{CalendarSettings, TriageRequirements};
use crate::timeblocking::{BacklogSection, Priority, UNDATED, block_key, description, metadata};

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
    mut task: Event,
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
    let mut data = database.import(&task)?;
    // A previous attempt may have moved the task before Calendar failed. The
    // durable link retains its new identity so retrying the old chip is safe.
    if let Some(link) = &data.metadata.schedule {
        task.id = format!("task:{}", link.task_id);
        task.calendar_id = format!("tasks:{}", link.list_id);
        task.details.scheduled = Some(link.clone());
    }
    task.details.description = description(&task.details.description);
    match change {
        // Tasks own the name. A scheduled block copies that name onto its calendar event.
        Change::Title(title) => {
            let token = access(settings)?;
            calendar::api::patch(
                &task_url(&task)?,
                &token,
                &serde_json::json!({ "title": title }).to_string(),
                "Rename task",
            )?;
            task.title = title;
            if let Some(link) = &data.metadata.schedule
                && !link.pending
            {
                calendar::scheduled::request(
                    "PATCH",
                    &link.event_url(),
                    &token,
                    Some(&calendar::scheduled::content(&task, link)),
                )
                .map_err(|error| error.message)?;
            }
        }
        Change::Description(value) => {
            let deadline = calendar::deadline::read_deadline(&task.details.description)?;
            task.details.description = calendar::deadline::write(&value, deadline);
            calendar::task_metadata::save(&access(settings)?, &task, &data)?;
            database.save(&task, &data)?;
        }
        Change::Estimate(minutes) => {
            data.metadata.minutes = minutes;
            let token = access(settings)?;
            // A scheduled task lasts exactly as long as its estimate.
            if minutes > 0 && data.metadata.schedule.is_some() && !task.all_day {
                let start = task.start;
                schedule(
                    &mut task,
                    &mut data,
                    &mut database,
                    settings,
                    &token,
                    start,
                    start + chrono::Duration::minutes(i64::from(minutes)),
                )?;
            } else {
                calendar::task_metadata::save(&token, &task, &data)?;
                database.save(&task, &data)?;
            }
        }
        Change::Projects(projects) => {
            data.metadata.projects = projects;
            calendar::task_metadata::save(&access(settings)?, &task, &data)?;
            database.save(&task, &data)?;
        }
        Change::Labels(labels) => {
            data.metadata.labels = labels;
            calendar::task_metadata::save(&access(settings)?, &task, &data)?;
            database.save(&task, &data)?;
        }
        Change::BacklogSection(section) => {
            data.metadata.backlog_section = section;
            calendar::task_metadata::save(&access(settings)?, &task, &data)?;
            database.save(&task, &data)?;
        }
        Change::Priority(priority) => {
            data.metadata.priority = priority;
            calendar::task_metadata::save(&access(settings)?, &task, &data)?;
            database.save(&task, &data)?;
        }
        Change::Location(location) => {
            data.metadata.location = location;
            calendar::task_metadata::save(&access(settings)?, &task, &data)?;
            database.save(&task, &data)?;
        }
        Change::Deadline(deadline) => {
            let token = access(settings)?;
            let notes = calendar::deadline::write(&task.details.description, deadline);
            let base = task_url(&task)?;
            calendar::api::patch(
                &base,
                &token,
                &serde_json::json!({"notes": calendar::task_metadata::write(&notes, &data)?})
                    .to_string(),
                "Save task deadline",
            )?;
            task.details.description = notes;
            if let Some(link) = &data.metadata.schedule {
                calendar::scheduled::request(
                    "PATCH",
                    &link.event_url(),
                    &token,
                    Some(&calendar::scheduled::content(&task, link)),
                )
                .map_err(|e| e.message)?;
            }
        }
        Change::Backlog(_)
        | Change::Inbox
        | Change::Triaged
        | Change::Unassign
        | Change::Day(_)
        | Change::ClearSchedule => {
            let (day, destination) = match change {
                Change::Day(day) => (Some(day), None),
                Change::ClearSchedule => (None, None),
                Change::Unassign => (Some(task.start.date_naive()), None),
                Change::Inbox => (None, Some(Destination::Inbox)),
                Change::Triaged => (None, Some(Destination::Triaged)),
                Change::Backlog(section) => (None, Some(Destination::Backlog(section))),
                _ => unreachable!(),
            };
            let token = access(settings)?;
            unschedule_with(
                &mut task,
                &mut data,
                &mut database,
                settings,
                &calendar::scheduled::Google { token: &token },
                day,
                destination,
            )?;
        }
        Change::Block(key, start, _end) => {
            data.metadata.block = Some(key);
            data.metadata.order = order;
            let minutes = if data.metadata.minutes == 0 {
                30
            } else {
                data.metadata.minutes
            };
            let token = access(settings)?;
            schedule(
                &mut task,
                &mut data,
                &mut database,
                settings,
                &token,
                start,
                start + chrono::Duration::minutes(i64::from(minutes)),
            )?;
        }
        Change::Time(start, end) => {
            data.metadata.block = None;
            let token = access(settings)?;
            // Moving keeps the estimate as the duration; resizing is a new estimate.
            let moved = !task.all_day && end - start == task.end - task.start;
            let end = if moved && data.metadata.minutes > 0 {
                start + chrono::Duration::minutes(i64::from(data.metadata.minutes))
            } else {
                let minutes = (end - start).num_minutes().clamp(0, 10080);
                data.metadata.minutes = minutes as u32;
                end
            };
            schedule(
                &mut task,
                &mut data,
                &mut database,
                settings,
                &token,
                start,
                end,
            )?;
        }
        Change::Reorder(block, order, day) => {
            data.metadata.block = block;
            data.metadata.order = order;
            if data.metadata.block.is_some() {
                let token = access(settings)?;
                let start = if task.all_day {
                    Local
                        .from_local_datetime(&day.and_hms_opt(9, 0, 0).unwrap())
                        .earliest()
                        .ok_or("Invalid local date")?
                } else {
                    task.start
                };
                let minutes = if data.metadata.minutes == 0 {
                    30
                } else {
                    data.metadata.minutes
                };
                schedule(
                    &mut task,
                    &mut data,
                    &mut database,
                    settings,
                    &token,
                    start,
                    start + chrono::Duration::minutes(i64::from(minutes)),
                )?;
            } else {
                calendar::task_metadata::save(&access(settings)?, &task, &data)?;
                database.save(&task, &data)?;
            }
        }
    }
    task.details.task_data = Some(data);
    Ok(task)
}

/// The Google Tasks list a task leaves the schedule for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    Inbox,
    Backlog(Option<BacklogSection>),
    Triaged,
}

/// Remove the schedule, then move tasks to the destination while retaining
/// their local ID. Day-only rescheduling keeps the current Google Tasks list.
pub fn unschedule_with(
    task: &mut Event,
    data: &mut crate::task_db::TaskData,
    database: &mut crate::task_db::Database,
    settings: &CalendarSettings,
    remote: &impl calendar::scheduled::Remote,
    day: Option<NaiveDate>,
    destination: Option<Destination>,
) -> Result<(), String> {
    let section = match destination {
        Some(Destination::Backlog(section)) => section,
        _ => None,
    };
    // Resolve/create the list before changing the task's existing schedule.
    let destination = match destination {
        Some(Destination::Backlog(_)) => {
            Some(remote.backlog_list(settings.backlog_list.as_deref())?)
        }
        Some(Destination::Inbox) => Some(remote.inbox_list(settings.inbox_list.as_deref())?),
        Some(Destination::Triaged) => Some(remote.triaged_list(settings.triaged_list.as_deref())?),
        None => None,
    };
    let mut link = match &data.metadata.schedule {
        Some(link) => link.clone(),
        None => calendar::scheduled::Link::new(String::new(), task)?,
    };
    if data.metadata.schedule.is_some() {
        remote.remove_event(&link)?;
    }
    remote.set_day(&link, day)?;
    if let Some(section) = section {
        data.metadata.backlog_section = section;
    }
    data.metadata.block = None;
    data.metadata.schedule = None;
    task.details.scheduled = None;
    task.all_day = true;
    let day = day.unwrap_or(UNDATED);
    task.start = Local
        .from_local_datetime(&day.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .ok_or("Invalid local date")?;
    task.end = task.start + chrono::Duration::days(1);
    // If moving lists fails, sync still sees the successfully cleared schedule.
    remote.save_metadata(task, data)?;
    database.save(task, data)?;
    if let Some(destination) = destination.filter(|destination| *destination != link.list_id) {
        let response = remote.move_task(&link, &destination)?;
        link.task_id = response["id"]
            .as_str()
            .ok_or("Google did not return the moved task ID")?
            .to_owned();
        task.id = format!("task:{}", link.task_id);
        task.calendar_id = ids::calendar_id(&destination);
        task.details.link = response["webViewLink"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "https://tasks.google.com/task/{}",
                    urlencoding::encode(&link.task_id)
                )
            });
        task.color = settings
            .task_color(&task.calendar_id)
            .unwrap_or(calendar::colors::TASK);
        remote.save_metadata(task, data)?;
        database.save(task, data)?;
    }
    Ok(())
}

pub fn task_url(task: &Event) -> Result<String, String> {
    let list = task
        .calendar_id
        .strip_prefix(ids::TASK_LIST)
        .ok_or("Invalid task list")?;
    let id = task.id.strip_prefix(ids::TASK).ok_or("Invalid task")?;
    Ok(format!(
        "{}/{}/tasks/{}",
        calendar::api::TASKS,
        urlencoding::encode(list),
        urlencoding::encode(id)
    ))
}

pub fn schedule(
    task: &mut Event,
    data: &mut crate::task_db::TaskData,
    database: &mut crate::task_db::Database,
    settings: &CalendarSettings,
    token: &str,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<(), String> {
    if !calendar::operations::replaying() {
        *task = calendar::operations::edit_task(
            task.clone(),
            settings,
            Change::Time(start, end),
            data.metadata.order,
        )?;
        *data = task.details.task_data.clone().ok_or("Missing task data")?;
        return Ok(());
    }
    schedule_with(
        task,
        data,
        database,
        settings,
        &calendar::scheduled::Google { token },
        start,
        end,
    )
}

pub fn schedule_with(
    task: &mut Event,
    data: &mut crate::task_db::TaskData,
    database: &mut crate::task_db::Database,
    settings: &CalendarSettings,
    remote: &impl calendar::scheduled::Remote,
    start: DateTime<Local>,
    end: DateTime<Local>,
) -> Result<(), String> {
    let old = data.metadata.schedule.clone();
    if let Some(link) = &old
        && (task.id != format!("task:{}", link.task_id)
            || task.calendar_id != format!("tasks:{}", link.list_id))
    {
        task.id = format!("task:{}", link.task_id);
        task.calendar_id = format!("tasks:{}", link.list_id);
        task.details.link = format!(
            "https://tasks.google.com/task/{}",
            urlencoding::encode(&link.task_id)
        );
    }
    let (destination_calendar, destination_list) = remote.destinations(
        settings
            .app_calendar
            .as_deref()
            .or_else(|| old.as_ref().map(|l| l.calendar_id.as_str())),
        settings
            .time_block_list
            .as_deref()
            .or_else(|| old.as_ref().map(|l| l.list_id.as_str())),
    )?;
    let mut link = match old.clone() {
        Some(link) => link,
        None => calendar::scheduled::Link::new(destination_calendar.clone(), task)?,
    };
    // Reserve a stable event ID before any remote write. Preserve aliases and
    // this reservation even if a later API call fails.
    link.calendar_id = destination_calendar;
    // Google uses the calendar default; task colors belong to the local UI.
    link.color_id = None;
    link.pending = true;
    link.block = data.metadata.block.clone();
    link.order = data.metadata.order;
    data.metadata.schedule = Some(link.clone());
    remote.save_metadata(task, data)?;
    database.save(task, data)?;
    if link.list_id != destination_list {
        let response = remote.move_task(&link, &destination_list)?;
        link.task_id = response["id"]
            .as_str()
            .ok_or("Google did not return the moved task ID")?
            .to_owned();
        link.list_id = destination_list;
        task.id = format!("task:{}", link.task_id);
        task.calendar_id = format!("tasks:{}", link.list_id);
        task.details.link = response["webViewLink"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "https://tasks.google.com/task/{}",
                    urlencoding::encode(&link.task_id)
                )
            });
        data.metadata.schedule = Some(link.clone());
        // The move already succeeded; retain its returned identity even if
        // publishing notes fails. Sync can also repair it from the moved notes.
        database.save(task, data)?;
        remote.save_metadata(task, data)?;
    }
    // The native Tasks date is the planned day, never the description deadline.
    remote.set_day(&link, Some(start.date_naive()))?;
    task.details.task_data = Some(data.clone());
    remote.save_event(task, &link, start, end).map_err(|e| {
        format!("Task saved, but its calendar block could not be saved: {e}. Sync or retry.")
    })?;
    if let Some(old) = old.filter(|old| old.calendar_id != link.calendar_id) {
        remote.remove_event(&old)?;
    }
    link.pending = false;
    task.start = start;
    task.end = end;
    task.all_day = false;
    task.details.scheduled = Some(link.clone());
    data.metadata.schedule = Some(link);
    remote.save_metadata(task, data)?;
    database.save(task, data)?;
    task.details.task_data = Some(data.clone());
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
