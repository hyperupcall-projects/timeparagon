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

    // The queued operation must retain this distinction when replayed online.
    let jobs = calendar::operations::list().unwrap();
    assert!(jobs.iter().any(|job| {
        matches!(
            serde_json::from_str::<calendar::operations::Job>(&job.payload).unwrap(),
            calendar::operations::Job::TaskEdit {
                change: task_edit::Change::Reflow(_, _),
                ..
            }
        )
    }));

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
