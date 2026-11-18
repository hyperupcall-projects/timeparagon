use chrono::{Duration, NaiveDate};
use timeparagon_core::{calendar, config::CalendarSettings, task_db::TaskData, timeblocking};

#[test]
fn an_assigned_task_spilling_past_its_block_stays_visible_on_the_next_day() {
	for hour in [0, 3] {
		let day = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
		let next = day.succ_opt().unwrap();
		let boundary = calendar::at_clock(next, hour, 0);
		let mut snapshot = calendar::sample_snapshot();
		let mut block = snapshot.events.remove(0);
		block.calendar_id = "app".into();
		block.start = boundary - Duration::minutes(30);
		block.end = boundary;
		block.all_day = false;
		let mut task = block.clone();
		task.id = "spilling-task".into();
		task.start = boundary - Duration::minutes(15);
		task.end = boundary + Duration::minutes(45);
		task.details.task_data = Some(TaskData {
			id: 101,
			metadata: timeblocking::Metadata {
				block: Some(timeblocking::block_key(&block)),
				minutes: 60,
				..Default::default()
			},
		});
		let settings = CalendarSettings {
			app_calendar: Some("app".into()),
			day_end_hour: hour,
			..Default::default()
		};
		for (view_day, expected) in [(day, block.id.as_str()), (next, task.id.as_str())] {
			snapshot.day = view_day;
			snapshot.events = vec![block.clone(), task.clone()];
			let store = calendar::Store::with_snapshot(snapshot.clone());
			let ids: Vec<_> = store
				.timed_events(&settings)
				.map(|e| e.id.as_str())
				.collect();
			assert_eq!(ids, [expected], "day {view_day}, boundary {hour}");
			let ids: Vec<_> = store
				.timed_events_on(view_day, &settings)
				.into_iter()
				.map(|e| e.id.as_str())
				.collect();
			assert_eq!(ids, [expected], "day {view_day}, boundary {hour}");
		}
	}
}
