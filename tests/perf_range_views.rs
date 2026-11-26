//! Timing harness for the month and two-week views against a real database.
//!
//! TP_PERF_DB=/path/to/copy.sqlite3 cargo test --test perf_range_views -- --ignored --nocapture

use std::time::{Duration, Instant};

use iced::advanced::renderer::Headless;
use iced::{Element, Font, Pixels, Renderer, Size, Theme, mouse};
use iced_runtime::user_interface::{self, UserInterface};
use timeparagon::app::{CalendarMessage, CalendarView, Message, State, SystemMessage};
use timeparagon::ui::layout::Metrics;

fn median(mut samples: Vec<Duration>) -> Duration {
	samples.sort();
	samples[samples.len() / 2]
}

fn time<F: FnMut()>(runs: usize, mut f: F) -> Duration {
	median(
		(0..runs)
			.map(|_| {
				let start = Instant::now();
				f();
				start.elapsed()
			})
			.collect(),
	)
}

fn state(view: CalendarView) -> State {
	let source = std::env::var("TP_PERF_DB").expect("set TP_PERF_DB");
	let target = timeparagon::task_db::path().unwrap();
	std::fs::create_dir_all(target.parent().unwrap()).unwrap();
	std::fs::copy(&source, &target).unwrap();
	let (config, _) = timeparagon::config::Config::load_checked();
	let mut state = State::new(config, timeparagon::theme::Catalog::built_in());
	state.calendar.apply_task_colors(&state.config.calendar);
	state.calendar.restore(Some(target));
	state.view = view;
	state.week_anchor = chrono::Local::now().date_naive();
	let dates = state.calendar_dates();
	state.calendar.select_range(&dates);
	state
}

fn report(view: CalendarView, renderer: &mut Renderer) {
	let mut state = state(view);
	let dates = state.calendar_dates();
	let settings = state.config.calendar.clone();
	let events = &state.calendar.events;
	let tasks = events.iter().filter(|e| e.is_task()).count();
	let blocks = events
		.iter()
		.filter(|e| timeparagon::timeblocking::is_block(e, &settings))
		.count();
	println!(
		"\n== {view:?}: {} days, {} loaded events ({tasks} tasks, {blocks} blocks), travel.show={}",
		dates.len(),
		events.len(),
		state.travel.show,
	);

	let store = &state.calendar;
	let all_day = time(5, || {
		for &day in &dates {
			std::hint::black_box(store.all_day_events_on(day, &settings).count());
		}
	});
	let timed = time(5, || {
		for &day in &dates {
			std::hint::black_box(store.timed_events_on(day, &settings));
		}
	});
	let deadlines = time(5, || {
		for &day in &dates {
			std::hint::black_box(store.deadline_markers_on(day, &settings));
		}
	});
	println!("  all_day_events_on   x{}: {all_day:?}", dates.len());
	println!("  timed_events_on     x{}: {timed:?}", dates.len());
	println!("  deadline_markers_on x{}: {deadlines:?}", dates.len());

	let size = Size::new(1600.0, 1000.0);
	let metrics = Metrics::of(size);
	let build_only = time(5, || {
		let element: Element<'_, Message> = match view {
			CalendarView::Month => timeparagon::ui::calendar::monthly_view(&state, &metrics),
			_ => timeparagon::ui::calendar::two_week_view(&state, &metrics),
		};
		std::hint::black_box(element);
	});
	println!("  view fn alone (widget construction): {build_only:?}");

	// Cold: a fresh widget tree, so every paragraph is shaped.
	let cold = time(3, || {
		let element: Element<'_, Message> = timeparagon::ui::editor_window(&state);
		let ui = UserInterface::build(element, size, user_interface::Cache::default(), renderer);
		std::hint::black_box(ui.into_cache());
	});
	// Warm: what the runtime does after each message, reusing the tree.
	let mut cache = {
		let element: Element<'_, Message> = timeparagon::ui::editor_window(&state);
		UserInterface::build(element, size, user_interface::Cache::default(), renderer).into_cache()
	};
	let mut draw = Vec::new();
	let warm = time(7, || {
		let element: Element<'_, Message> = timeparagon::ui::editor_window(&state);
		let mut ui = UserInterface::build(element, size, std::mem::take(&mut cache), renderer);
		let start = Instant::now();
		ui.draw(
			renderer,
			&Theme::Dark,
			&iced::advanced::renderer::Style::default(),
			mouse::Cursor::Unavailable,
		);
		draw.push(start.elapsed());
		cache = ui.into_cache();
	});
	println!("  rebuild cold (view+layout):      {cold:?}");
	println!(
		"  rebuild warm (view+layout+draw): {warm:?}  (draw alone {:?})",
		median(draw)
	);

	let update = time(10, || {
		let _ = timeparagon::app::update(
			&mut state,
			Message::Calendar(CalendarMessage::CalendarGridScrolled(10.0, 800.0)),
		);
	});
	println!("  app::update(CalendarGridScrolled): {update:?}");
	let poll = time(10, || {
		let _ = timeparagon::app::update(
			&mut state,
			Message::System(SystemMessage::DatabaseSyncPolled),
		);
	});
	println!("  app::update(DatabaseSyncPolled):   {poll:?}");
	state.travel.show = false;
	let no_travel = time(10, || {
		let _ = timeparagon::app::update(
			&mut state,
			Message::Calendar(CalendarMessage::CalendarGridScrolled(10.0, 800.0)),
		);
	});
	println!("  app::update with travel.show=false: {no_travel:?}");
}

#[test]
#[ignore]
fn range_view_timings() {
	let mut renderer = iced::futures::executor::block_on(Renderer::new(
		Font::DEFAULT,
		Pixels(16.0),
		std::env::var("ICED_TEST_BACKEND").ok().as_deref(),
	))
	.expect("headless renderer");
	report(CalendarView::Month, &mut renderer);
	report(CalendarView::TwoWeek, &mut renderer);
}
