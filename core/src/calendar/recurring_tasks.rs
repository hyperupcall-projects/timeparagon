//! Materialize independent tasks for the loaded occurrences of a repeating block.
use super::Event;
use crate::{
    config::CalendarSettings,
    task_edit::{self, Change},
    timeblocking as tb,
};

fn source(task: &Event) -> Option<i64> {
    task.details
        .task_data
        .as_ref()
        .map(|d| d.metadata.repeat_source.unwrap_or(d.id))
}

fn series(task: &Event) -> Option<String> {
    let meta = tb::metadata(task);
    let inherited = meta.inherited?;
    inherited.series.clone().or_else(|| {
        // Compatibility with metadata saved before recurring tasks were materialized.
        (!inherited.recurrence.is_empty())
            .then_some(meta.block)
            .flatten()
    })
}

/// Pure planning keeps refreshes idempotent, including completed occurrences.
fn missing(events: &[Event]) -> Vec<(Event, Event, bool)> {
    let mut result = Vec::new();
    let mut templates: Vec<_> = events.iter().filter(|e| e.is_task()).collect();
    templates.sort_by_key(|e| tb::metadata(e).repeat_source.is_some());
    for block in events
        .iter()
        .filter(|e| !e.is_task() && e.details.options.child_metadata.force)
    {
        let Some(master) = &block.details.recurring_event_id else {
            continue;
        };
        let series_key = format!("{}/{}", block.calendar_id, master);
        let key = tb::block_key(block);
        let mut seen = std::collections::HashSet::new();
        for template in templates
            .iter()
            .copied()
            .filter(|e| series(e).as_deref() == Some(&series_key))
        {
            let Some(source_id) = source(template) else {
                continue;
            };
            if !seen.insert(source_id)
                || events.iter().any(|e| {
                    e.is_task()
                        && source(e) == Some(source_id)
                        && tb::metadata(e).block.as_deref() == Some(&key)
                })
            {
                continue;
            }
            // Reuse the original task for its own occurrence, retaining history and completion.
            let original = tb::metadata(template).block.as_deref() == Some(&series_key)
                && template.start.date_naive() == block.start.date_naive();
            result.push((template.clone(), block.clone(), original));
        }
    }
    result
}

/// Reconcile actual children, including one-off blocks, after applying queued edits.
fn inheritance_changes(events: &[Event]) -> Vec<(Event, Change)> {
    let mut changes = Vec::new();
    for task in events.iter().filter(|e| e.is_task()) {
        let meta = tb::metadata(task);
        let Some(block) = events
            .iter()
            .find(|b| !b.is_task() && meta.block.as_deref() == Some(&tb::block_key(b)))
        else {
            continue;
        };
        if block.details.options.child_metadata.force {
            let inherited = tb::inherited_metadata(block);
            if meta.inherited.as_deref() != Some(&inherited) {
                changes.push((task.clone(), Change::Inherit(inherited)));
            }
        } else if meta.inherited.is_some() {
            changes.push((task.clone(), Change::ReleaseInheritance));
        }
    }
    changes
}

/// Persist the destination and source in the initial create operation. A refresh
/// can then recover a partially assigned task instead of creating another copy.
fn copy_metadata(template: &Event, block: &Event) -> tb::Metadata {
    let mut metadata = tb::metadata(template);
    let mut inherited = tb::inherited_metadata(block);
    if inherited.recurrence.is_empty() {
        inherited.recurrence = metadata
            .inherited
            .as_ref()
            .map(|v| v.recurrence.clone())
            .unwrap_or_default();
    }
    metadata.repeat_source = source(template);
    metadata.block = Some(tb::block_key(block));
    metadata.schedule = None;
    metadata.ignored = false;
    metadata.created_at = None;
    metadata.projects = inherited.projects.clone();
    metadata.labels = inherited.labels.clone();
    metadata.priority = inherited.fields.priority;
    metadata.location = inherited.fields.location;
    metadata.inherited = Some(Box::new(inherited));
    metadata
}

pub fn synchronize(events: &mut Vec<Event>) -> Result<(), String> {
    // Multiple calendar windows may refresh concurrently. Re-read queued writes
    // under the lock so a stale snapshot cannot create the same occurrence twice.
    static SYNC: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = SYNC.lock().map_err(|e| e.to_string())?;
    if let Some(path) = crate::task_db::path() {
        super::operations::overlay(&path, events, None)?;
    }
    for (task, change) in inheritance_changes(events) {
        let saved = task_edit::save(
            task,
            &CalendarSettings::default(),
            change,
            0,
            task_edit::google_access,
        )?;
        if let Some(old) = events.iter_mut().find(|e| e.id == saved.id) {
            *old = saved;
        }
    }
    for (template, block, original) in missing(events) {
        let meta = tb::metadata(&template);
        let settings = CalendarSettings {
            app_calendar: Some(block.calendar_id.clone()),
            inbox_list: Some(template.calendar_id.clone()),
            time_block_list: Some(template.calendar_id.clone()),
            ..Default::default()
        };
        let task = if original {
            template.clone()
        } else {
            let metadata = copy_metadata(&template, &block);
            let fields = &metadata
                .inherited
                .as_ref()
                .expect("copy has inherited metadata")
                .fields;
            let notes = super::deadline::write(
                &tb::description(&template.details.description),
                fields.deadline,
            );
            let due = fields.due.map(|due| due.date);
            task_edit::create_inbox_task(
                &settings,
                "",
                &template.title,
                &notes,
                due,
                Some(metadata),
            )?
        };
        // All writes use the durable offline queue, just like manually assigning a task.
        let mut inherited = tb::inherited_metadata(&block);
        // Expanded Google occurrences omit RRULE; retain the master's repeat rule.
        if inherited.recurrence.is_empty() {
            inherited.recurrence = meta
                .inherited
                .as_ref()
                .map(|v| v.recurrence.clone())
                .unwrap_or_default();
        }
        let saved = task_edit::save(
            task,
            &settings,
            Change::Block(tb::block_key(&block), block.start, block.end),
            meta.order,
            task_edit::google_access,
        )?;
        let saved = task_edit::save(
            saved,
            &settings,
            Change::Inherit(inherited),
            meta.order,
            task_edit::google_access,
        )?;
        if let Some(old) = events.iter_mut().find(|e| e.id == saved.id) {
            *old = saved;
        } else {
            events.push(saved);
        }
    }
    // Place each occurrence's tasks end to end using their individual estimates.
    let blocks: Vec<_> = events
        .iter()
        .filter(|e| !e.is_task() && e.details.options.child_metadata.force)
        .cloned()
        .collect();
    for block in blocks {
        let settings = CalendarSettings {
            app_calendar: Some(block.calendar_id.clone()),
            ..Default::default()
        };
        for saved in task_edit::reflow(&block, events, &settings, |_| Ok(String::new()))? {
            if let Some(old) = events.iter_mut().find(|e| e.id == saved.id) {
                *old = saved;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<Event> {
        let mut parent = super::super::sample_snapshot().events.remove(0);
        parent.id = "master".into();
        parent.calendar_id = "app".into();
        parent.all_day = false;
        parent.details.recurrence = vec!["RRULE:FREQ=DAILY".into()];
        parent.details.options.child_metadata.force = true;
        let mut child = parent.clone();
        child.id = "task:original".into();
        child.calendar_id = "tasks:list".into();
        child.completed = true;
        let mut data = crate::task_db::TaskData {
            id: 100,
            legacy_key: None,
            metadata: tb::Metadata {
                block: Some(tb::block_key(&parent)),
                minutes: 25,
                ..Default::default()
            },
        };
        tb::apply_inherited(&mut child, &mut data, tb::inherited_metadata(&parent));
        child.details.task_data = Some(data);
        let mut first = parent.clone();
        first.id = "master_first".into();
        first.details.recurring_event_id = Some(parent.id.clone());
        first.details.recurrence.clear();
        let mut next = first.clone();
        next.id = "master_next".into();
        next.start += chrono::Duration::days(1);
        next.end += chrono::Duration::days(1);
        vec![first, next, child]
    }

    #[test]
    fn repeating_block_reuses_original_and_plans_independent_future_tasks() {
        let events = fixture();
        let planned = missing(&events);
        assert_eq!(planned.len(), 2);
        assert!(planned[0].2);
        assert!(!planned[1].2);
        assert_eq!(source(&planned[1].0), Some(100));
        assert_eq!(planned[1].1.id, "master_next");
    }

    #[test]
    fn refresh_does_not_duplicate_existing_or_completed_occurrences() {
        let mut events = fixture();
        let mut child = events[2].clone();
        child.details.task_data.as_mut().unwrap().metadata.block = Some(tb::block_key(&events[0]));
        events[2] = child.clone();
        child.id = "task:next".into();
        let key = tb::block_key(&events[1]);
        let data = child.details.task_data.as_mut().unwrap();
        data.id = 101;
        data.metadata.repeat_source = Some(100);
        data.metadata.block = Some(key);
        events.push(child);
        assert!(missing(&events).is_empty());
    }

    #[test]
    fn refresh_reconciles_one_off_parent_changes_and_unlocks_disabled_children() {
        let mut events = fixture();
        events[0].id = "master".into();
        events[0].details.recurring_event_id = None;
        events[0].details.projects = vec![42];
        events[0].details.labels = vec!["updated".into()];
        events[0].details.options.child_metadata.priority = Some(tb::Priority::P2);
        let changes = inheritance_changes(&events);
        assert_eq!(changes.len(), 1);
        let Change::Inherit(value) = &changes[0].1 else {
            panic!("expected propagation")
        };
        assert_eq!(value.projects, [42]);
        assert_eq!(value.labels, ["updated"]);
        assert_eq!(value.fields.priority, Some(tb::Priority::P2));
        let task = &mut events[2];
        let mut data = task.details.task_data.take().unwrap();
        tb::apply_inherited(task, &mut data, value.clone());
        task.details.task_data = Some(data);
        assert!(inheritance_changes(&events).is_empty());
        events[0].details.options.child_metadata.force = false;
        assert!(matches!(
            inheritance_changes(&events)[0].1,
            Change::ReleaseInheritance
        ));
    }

    #[test]
    fn interrupted_assignment_is_recovered_without_another_copy() {
        let mut events = fixture();
        let mut pending = events[2].clone();
        pending.id = "task:pending".into();
        pending.all_day = true;
        pending.completed = false;
        let metadata = copy_metadata(&events[2], &events[1]);
        let data = pending.details.task_data.as_mut().unwrap();
        data.id = 101;
        data.metadata = metadata;
        events.push(pending);
        let changes = missing(&events);
        assert_eq!(changes.len(), 1);
        assert!(changes[0].2); // Only the original occurrence still needs binding.
    }

    #[test]
    fn new_occurrences_keep_planning_values_without_old_scheduling_state() {
        let mut events = fixture();
        let task = &mut events[2];
        task.details.task_data.as_mut().unwrap().metadata.ignored = true;
        let copy = copy_metadata(&events[2], &events[1]);
        assert_eq!(copy.repeat_source, Some(100));
        assert_eq!(copy.block, Some(tb::block_key(&events[1])));
        assert_eq!(copy.minutes, 25);
        assert!(!copy.ignored);
        assert!(copy.schedule.is_none());
        assert!(copy.created_at.is_none());
        assert_eq!(
            copy.inherited.unwrap().series.as_deref(),
            Some("app/master")
        );
    }

    #[test]
    fn recurrence_requires_enabled_propagation_and_supports_old_metadata() {
        let mut events = fixture();
        events[2]
            .details
            .task_data
            .as_mut()
            .unwrap()
            .metadata
            .inherited
            .as_mut()
            .unwrap()
            .series = None;
        assert_eq!(missing(&events).len(), 2);
        for block in &mut events[..2] {
            block.details.options.child_metadata.force = false;
        }
        assert!(missing(&events).is_empty());
    }
}
