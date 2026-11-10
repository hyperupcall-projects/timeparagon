use chrono::Duration;
use timeslay_core::{calendar, config::CalendarSettings, task_edit, timeblocking};

#[test]
fn adding_four_tasks_keeps_all_four_assigned_after_automatic_layout() {
    let settings = CalendarSettings::default();
    let mut block = calendar::sample_snapshot().events.remove(0);
    block.id = "maintenance-block".into();
    block.calendar_id = "app".into();
    block.all_day = false;
    block.end = block.start + Duration::hours(2);
    let key = timeblocking::block_key(&block);
    let mut tasks = Vec::new();

    for (order, minutes) in [8, 0, 5, 0].into_iter().enumerate() {
        let task = calendar::operations::create_task(
            &settings,
            &format!("maintenance task {order}"),
            "",
            Some(block.start.date_naive()),
            Some(timeblocking::Metadata {
                minutes,
                ..Default::default()
            }),
        )
        .unwrap();
        let changed = task_edit::save_and_reflow(
            task,
            settings.clone(),
            task_edit::Change::Block(key.clone(), block.start, block.end),
            order as i64,
            vec![block.clone()],
            tasks.clone(),
            |_| Ok("unused offline token".into()),
        )
        .unwrap();
        for task in changed {
            tasks.retain(|previous: &calendar::Event| previous.id != task.id);
            tasks.push(task);
        }
        assert_eq!(tasks.len(), order + 1);
        assert!(
            tasks.iter().all(|task| {
                timeblocking::metadata(task).block.as_deref() == Some(key.as_str())
            })
        );
    }

    tasks.sort_by_key(|task| timeblocking::metadata(task).order);
    let mut next_start = block.start;
    for (task, minutes) in tasks.iter().zip([8, 0, 5, 0]) {
        assert_eq!(timeblocking::metadata(task).minutes, minutes);
        assert_eq!(task.start, next_start);
        next_start += Duration::minutes(if minutes == 0 { 30 } else { minutes.into() });
        assert_eq!(task.end, next_start);
    }

    // The write sent to Google must keep this distinction.
    assert!(
        calendar::operations::log_since(0)
            .iter()
            .any(|entry| entry.text.contains("Edit task Reflow"))
    );

    // A deliberate manual move still removes a task from its time block.
    let moved = task_edit::save(
        tasks[1].clone(),
        &settings,
        task_edit::Change::Time(block.end, block.end + Duration::minutes(30)),
        0,
        |_| panic!("local edits must not access Google"),
    )
    .unwrap();
    assert!(timeblocking::metadata(&moved).block.is_none());
}

#[test]
fn planned_task_survives_reload_moves_with_its_block_and_is_released_on_delete() {
    let settings = CalendarSettings::default();
    let mut block = calendar::sample_snapshot().events.remove(0);
    block.id = "reload-lifecycle-block".into();
    block.calendar_id = "app".into();
    block.all_day = false;
    let key = timeblocking::block_key(&block);
    // These are the same shared calls used by the Planned Tasks plus button.
    let task = task_edit::create_inbox_task(
        &settings,
        "",
        "Lifecycle task",
        "",
        Some(block.start.date_naive()),
        None,
    )
    .unwrap();
    let id = task.id.clone();
    task_edit::save_and_reflow(
        task,
        settings.clone(),
        task_edit::Change::Block(key.clone(), block.start, block.end),
        1,
        vec![block.clone()],
        vec![],
        |_| Ok("offline".into()),
    )
    .unwrap();

    let path = timeslay_core::task_db::path().unwrap();
    let reload = || {
        let mut restored = Vec::new();
        calendar::operations::load_overlay(&path)
            .unwrap()
            .apply(&mut restored, None);
        restored.into_iter().find(|event| event.id == id)
    };
    let restored = reload().expect("task survives reopening the database");
    assert_eq!(
        timeblocking::metadata(&restored).block.as_deref(),
        Some(key.as_str())
    );
    assert!(!restored.all_day);
    assert_eq!(restored.start, block.start);

    block.start += Duration::hours(2);
    block.end += Duration::hours(2);
    task_edit::reflow(&block, &[restored], &settings, |_| Ok("offline".into())).unwrap();
    let moved = reload().unwrap();
    assert_eq!(moved.start, block.start);
    assert_eq!(
        timeblocking::metadata(&moved).block.as_deref(),
        Some(key.as_str())
    );
    assert!(!timeblocking::overdue(&moved));

    calendar::delete_event(&block.calendar_id, &block.id).unwrap();
    let released = reload().expect("deleting the container preserves its task");
    assert!(released.all_day);
    assert_eq!(released.start.date_naive(), block.start.date_naive());
    assert!(timeblocking::metadata(&released).block.is_none());
    calendar::delete_task(&released, None).unwrap();
    assert!(
        reload().is_none(),
        "task deletion survives reopening the database"
    );
}
