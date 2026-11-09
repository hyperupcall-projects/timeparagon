//! Reconcile block inheritance and layout. Calendar expands task recurrences.
use super::Event;
use crate::{
    config::CalendarSettings,
    task_edit::{self, Change},
    timeblocking as tb,
};
pub fn synchronize(events: &mut Vec<Event>) -> Result<(), String> {
    static SYNC: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = SYNC.lock().map_err(|e| e.to_string())?;
    let blocks: Vec<_> = events.iter().filter(|e| !e.is_task()).cloned().collect();
    // Expanded children inherit the occurrence membership of their block series.
    for task in events.iter_mut().filter(|e| e.is_calendar_task()) {
        let meta = tb::metadata(task);
        if task.details.recurring_event_id.is_some()
            && let Some(series) = meta.inherited.as_ref().and_then(|v| v.series.as_ref())
            && let Some(block) = blocks.iter().find(|b| {
                b.details
                    .recurring_event_id
                    .as_ref()
                    .is_some_and(|id| format!("{}/{}", b.calendar_id, id) == *series)
                    && b.start.date_naive() == task.start.date_naive()
            })
            && let Some(data) = &mut task.details.task_data
        {
            data.metadata.block = Some(tb::block_key(block));
        }
    }
    let settings = CalendarSettings::default();
    for task in events.iter_mut().filter(|e| e.is_task()) {
        let meta = tb::metadata(task);
        let Some(block) = blocks
            .iter()
            .find(|b| meta.block.as_deref() == Some(&tb::block_key(b)))
        else {
            continue;
        };
        let change = if block.details.options.child_metadata.force {
            let mut inherited = tb::inherited_metadata(block);
            if inherited.recurrence.is_empty() {
                inherited.recurrence = meta
                    .inherited
                    .as_ref()
                    .map(|v| v.recurrence.clone())
                    .unwrap_or_default();
            }
            (meta.inherited.as_deref() != Some(&inherited)).then_some(Change::Inherit(inherited))
        } else {
            meta.inherited
                .is_some()
                .then_some(Change::ReleaseInheritance)
        };
        if let Some(change) = change {
            *task = task_edit::save(
                task.clone(),
                &settings,
                change,
                meta.order,
                task_edit::google_access,
            )
            .map_err(String::from)?;
        }
    }
    for block in blocks
        .iter()
        .filter(|b| b.details.options.child_metadata.force)
    {
        let settings = CalendarSettings {
            app_calendar: Some(block.calendar_id.clone()),
            ..Default::default()
        };
        for saved in task_edit::reflow(block, events, &settings, task_edit::google_access)? {
            if let Some(old) = events.iter_mut().find(|e| e.id == saved.id) {
                *old = saved;
            }
        }
    }
    Ok(())
}
