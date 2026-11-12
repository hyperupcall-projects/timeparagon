//! Phone calendar: a date picker above day grids, a month, or a schedule list.
use chrono::{Datelike, Duration, Months, NaiveDate};
use dioxus::prelude::*;
use timeparagon_core::calendar::first_day::FirstDayOfWeek;
use timeparagon_core::calendar::search::Search;
use timeparagon_core::calendar::time_format::{self, TimeFormat};
use timeparagon_core::calendar::{self, Event, Snapshot};
use timeparagon_core::config::{CalendarSettings, Config};

use crate::{HOUR_HEIGHT, day, editor::worker, plan::PlanView};

fn schedule_dates(
	view: PlanView,
	anchor: NaiveDate,
	count: u8,
	first: FirstDayOfWeek,
) -> Vec<NaiveDate> {
	match view {
		PlanView::Month => calendar::month_grid(anchor, first),
		PlanView::Schedule => calendar::schedule::dates(anchor),
		_ => (0..if view == PlanView::Day { 1 } else { count })
			.map(|offset| anchor + Duration::days(i64::from(offset)))
			.collect(),
	}
}

fn strip_dates(anchor: NaiveDate, expanded: bool, first: FirstDayOfWeek) -> Vec<NaiveDate> {
	if expanded {
		calendar::month_grid(anchor, first)
	} else {
		let start = first.week_start(anchor);
		(0..7)
			.map(|offset| start + Duration::days(offset))
			.collect()
	}
}

#[component]
pub fn Planner(
	config: Signal<Config>,
	settings: CalendarSettings,
	mut view: Signal<PlanView>,
	mut anchor: Signal<NaiveDate>,
	today: NaiveDate,
	mut revision: Signal<u32>,
	time_format: TimeFormat,
	search: Search,
	on_day: EventHandler<NaiveDate>,
	on_open: EventHandler<Event>,
	on_block: EventHandler<Event>,
) -> Element {
	let mut clock = use_signal(chrono::Local::now);
	use_future(move || async move {
		loop {
			tokio::time::sleep(std::time::Duration::from_secs(30)).await;
			clock.set(chrono::Local::now());
		}
	});
	let mut count = use_signal(|| 3u8);
	let mut expanded = use_signal(|| false);
	let selected = anchor();
	let period = view();
	let days_count = count();
	let full_month = period == PlanView::Month;
	let agenda_mode = period == PlanView::Schedule;
	let dates = schedule_dates(period, selected, days_count, settings.first_day_of_week);
	let load_settings = settings.clone();
	let wanted = dates.clone();
	let loaded = use_resource(use_reactive!(|(wanted, load_settings)| {
		let _ = revision();
		async move {
			let requested = wanted.clone();
			worker(move || day::load_agenda(&wanted, &load_settings))
				.await
				.map(|snapshot| (requested, snapshot))
		}
	}));
	// Keep Add and event sheets anchored to the date selected in the strip.
	use_effect(move || {
		on_day.call(anchor());
	});
	// Scroll a newly displayed schedule once, leaving later refreshes alone.
	let mut scrolled_for = use_signal(|| None::<(NaiveDate, PlanView, u8)>);
	use_effect(move || {
		let key = (anchor(), view(), count());
		if matches!(key.1, PlanView::Month | PlanView::Schedule) || scrolled_for() == Some(key) {
			return;
		}
		let ready = loaded.read().as_ref().is_some_and(|result| {
			result
				.as_ref()
				.is_ok_and(|(dates, _)| dates.first() == Some(&key.0))
		});
		if ready {
			spawn(async move {
				if matches!(
					document::eval(crate::SCROLL_SCRIPT).join::<bool>().await,
					Ok(true)
				) {
					scrolled_for.set(Some(key));
				}
			});
		}
	});
	let strip = strip_dates(
		selected,
		expanded() || full_month || agenda_mode,
		settings.first_day_of_week,
	);
	let heading = calendar::month_heading(calendar::first_of_month(selected));
	let snapshot = loaded
		.read()
		.as_ref()
		.and_then(|result| result.as_ref().ok())
		.filter(|(requested, _)| *requested == dates)
		.map(|(_, snapshot)| day::filter_snapshot(snapshot, &search, &settings));
	let error = loaded
		.read()
		.as_ref()
		.and_then(|result| result.as_ref().err())
		.cloned();
	let month_rows = snapshot
		.as_ref()
		.filter(|_| full_month)
		.map(|snapshot| day::agenda(snapshot, &dates, &settings))
		.unwrap_or_default();
	let shown_count = if period == PlanView::Day {
		1
	} else {
		days_count
	};

	rsx! {
		section { class: if full_month { "mobile-planner month-mode" } else { "mobile-planner" }, aria_label: "Planning calendar",
			div { class: "planner-toolbar",
				button {
					class: "planner-heading",
					aria_label: if agenda_mode { "Jump to date" } else { "Show full month" },
					onclick: move |_| { if agenda_mode { expanded.set(!expanded()); } else { view.set(PlanView::Month); } },
					"{heading} ▾"
				}
				div { class: "spacer" }
				button { class: "btn icon", aria_label: "Previous month", title: "Previous month",
					onclick: move |_| { if let Some(date) = anchor().checked_sub_months(Months::new(1)) { anchor.set(date); } }, "‹"
				}
				button { class: "btn small", onclick: move |_| anchor.set(today), "Today" }
				button { class: "btn icon", aria_label: "Next month", title: "Next month",
					onclick: move |_| { if let Some(date) = anchor().checked_add_months(Months::new(1)) { anchor.set(date); } }, "›"
				}
			}
			if !full_month && !agenda_mode {
				div { class: "planner-options",
					if period == PlanView::MultiDay {
						div { class: "planner-count", role: "group", aria_label: "Days to show",
							for number in [2u8, 3] {
								button { class: "nav-tab", aria_pressed: (days_count == number).to_string(),
									onclick: move |_| count.set(number), "{number} days"
								}
							}
						}
					} else { span { class: "hint", "{selected.format(\"%A, %b %-d\")}" } }
					div { class: "spacer" }
					button { class: "btn small", onclick: move |_| view.set(PlanView::Month), "Month" }
				}
			}
			if !agenda_mode || expanded() {
			div { class: "planner-weekdays",
				for weekday in settings.first_day_of_week.short_labels() {
					span { key: "{weekday}", "{weekday}" }
				}
			}
			div { class: if full_month { "planner-dates full-month" } else { "planner-dates" },
				for date in strip {
					{
						let mut class = String::from("planner-date");
						if date.month() != selected.month() { class.push_str(" outside"); }
						if date == today { class.push_str(" today"); }
						let in_range = !full_month && !agenda_mode && dates.contains(&date);
						if date == selected { class.push_str(" selected"); }
						else if in_range { class.push_str(" in-range"); }
						let rows = month_rows.iter().find(|(day, _)| *day == date).map(|(_, rows)| rows.as_slice()).unwrap_or(&[]);
						rsx! {
							button {
								key: "{date}", class: "{class}",
								aria_label: date.format("%A, %B %-d, %Y").to_string(),
								aria_pressed: (date == selected || in_range).to_string(),
								aria_current: if date == today { "date" } else { "false" },
								onclick: move |_| {
									anchor.set(date);
									if agenda_mode { expanded.set(false); }
									if full_month { view.set(PlanView::Day); }
								},
								span { class: "planner-number", "{date.day()}" }
								if full_month {
									for row in rows.iter().take(2) {
										span { class: "month-preview", style: "border-left-color: {crate::css_color(row.color)};", "{row.event.title}" }
									}
									if rows.len() > 2 { span { class: "month-more", "+{rows.len() - 2} more" } }
								}
							}
						}
					}
				}
			}
			}
			if !full_month && !agenda_mode {
				button { class: "planner-expand", aria_expanded: expanded().to_string(),
					aria_label: if expanded() { "Collapse to week" } else { "Expand to month" },
					onclick: move |_| expanded.set(!expanded()),
					span { class: "planner-handle" }
					span { if expanded() { "Less" } else { "Expand month" } }
				}
			}
			if let Some(message) = error {
				div { class: "planner-message", role: "alert",
					p { "{message}" }
					button { class: "btn", onclick: move |_| revision.set(revision() + 1), "Try again" }
				}
			} else if snapshot.is_none() {
				p { class: "planner-message hint", role: "status", "Loading calendar…" }
			}
			if agenda_mode {
				if let Some(snapshot) = &snapshot {
					{schedule_list(snapshot, &dates, &settings, time_format, &search, clock(), on_open, anchor)}
				}
			}
			if !full_month && !agenda_mode {
				if let Some(snapshot) = snapshot {
					{schedule_grid(&snapshot, &dates, &settings, time_format, shown_count, on_open, on_block, (!search.is_active()).then(|| config.read().clone()).as_ref())}
				}
			}
		}
	}
}

#[allow(clippy::too_many_arguments)]
fn schedule_grid(
	snapshot: &Snapshot,
	dates: &[NaiveDate],
	settings: &CalendarSettings,
	time_format: TimeFormat,
	count: u8,
	on_open: EventHandler<Event>,
	on_block: EventHandler<Event>,
	travel_config: Option<&Config>,
) -> Element {
	let models: Vec<_> = dates
		.iter()
		.map(|date| {
			let mut snapshot = snapshot.clone();
			snapshot.day = *date;
			day::build_model(&snapshot, settings)
		})
		.collect();
	let height = 24.0 * HOUR_HEIGHT;
	let hour_start = settings.day_start_hour();
	rsx! {
		div { class: "planner-schedule", "data-days": "{count}",
			div { class: "planner-column-heads", style: "grid-template-columns:44px repeat({count}, minmax(0, 1fr));",
				span { class: "hint planner-all-label", "All day" }
				for model in &models {
					div { class: "planner-column-head",
						span { class: "planner-day-title", "{model.day.format(\"%a %-d\")}" }
						for event in &model.all_day {
							{
								let opened = event.clone();
								rsx! {
									button { class: "all-day-chip", "data-sending": event.details.sending, style: "background:{crate::css_color(event.display_color(settings, &model.calendars))};color:{crate::css_color(calendar::colors::on_color(event.display_color(settings, &model.calendars)))};",
										onclick: move |_| on_open.call(opened.clone()), "{event.title}"
									}
								}
							}
						}
					}
				}
			}
			div { class: "timeline-scroll",
				div { class: "timeline planner-timeline", style: "height:{height}px;grid-template-columns:44px repeat({count}, minmax(0, 1fr));",
					div { class: "gutter",
						for row in 0..24u32 {
							span { class: "hour-label", style: "top:{row as f32 * HOUR_HEIGHT + 2.0}px;",
								"{time_format::hour(day::hour_at(hour_start, row), time_format)}"
							}
						}
					}
					for (index, model) in models.iter().enumerate() {
						div { key: "{model.day}", class: "grid planner-day-grid", aria_label: model.day.to_string(),
							for row in 0..48 {
								div { class: if row % 2 == 0 { "hour-line" } else { "hour-line half-hour" }, style: "top:{row as f32 * HOUR_HEIGHT / 2.0}px;" }
							}
							for timed in &model.timed {
								{
									let opened = timed.event.clone();
									let is_block = timed.time_block;
									// Two or three columns leave task details to the block popup.
									let mut shown = timed.clone();
									shown.time_block = count == 1 && is_block;
									rsx! {
										button { key: "{timed.event.id}", class: if is_block { "grid-event block" } else { "grid-event" }, "data-sending": timed.event.details.sending,
											style: "{crate::event_style(&shown)}",
											title: "{timed.title}\n{crate::event_when(&timed.event, time_format)}",
											onclick: move |_| { if is_block { on_block.call(opened.clone()); } else { on_open.call(opened.clone()); } },
											{crate::event_label(timed, crate::event_height(timed), time_format)}
											if is_block { span { class: "block-plan", "Plan" } }
										}
									}
								}
							}
							if count == 1 {
								for timed in &model.timed { {crate::block_task_buttons(timed, on_open).into_iter()} }
							}
							if let Some(config) = travel_config {
								for trip in day::travel_boxes(model, &config.travel, config.appearance.distance_unit) {
									div { class: "grid-travel", style: "{crate::travel_style(&trip)}", "{trip.label}" }
								}
							}
							if let Some(minutes) = model.now_min {
								div { id: "now-line", class: "now-line", style: "top:{minutes as f32 * HOUR_HEIGHT / 60.0}px;", span { class: "now-dot" } }
							}
							if index == 0 {
								div { id: "timeline-anchor", class: "anchor", style: "top:{8.0 * HOUR_HEIGHT}px;" }
							}
						}
					}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn date(year: i32, month: u32, day: u32) -> NaiveDate {
		NaiveDate::from_ymd_opt(year, month, day).unwrap()
	}

	fn preview<const MODE: u8>() -> Element {
		let day = date(2026, 11, 8);
		let view = use_signal(|| {
			if MODE == 0 {
				PlanView::Month
			} else {
				PlanView::MultiDay
			}
		});
		let anchor = use_signal(move || day);
		let revision = use_signal(|| 0);
		let config = use_signal(Config::default);
		rsx! { Planner {
			config, settings: CalendarSettings::default(), view, anchor, today: day,
			revision, time_format: TimeFormat::TwelveHour, search: Search::default(),
			on_day: |_| {}, on_open: |_| {}, on_block: |_| {},
		} }
	}

	#[test]
	fn calendar_controls_and_full_month_render_without_calendar_data() {
		for (mode, app) in [
			("month", preview::<0> as fn() -> Element),
			("multi", preview::<1>),
		] {
			let mut dom = VirtualDom::new(app);
			dom.rebuild_in_place();
			let html = dioxus_ssr::render(&dom);
			assert!(html.contains("Previous month"));
			assert!(html.contains("Next month"));
			assert!(html.contains("Sunday, November 8, 2026"));
			if mode == "month" {
				assert!(html.contains("November 1, 2026"));
				assert!(html.contains("November 30, 2026"));
				assert!(!html.contains("Expand to month"));
			} else {
				assert!(html.contains("2 days"));
				assert!(html.contains("3 days"));
				assert!(html.contains("Expand to month"));
			}
			if let Ok(directory) = std::env::var("TIMEPARAGON_PLANNER_PREVIEW_DIR") {
				let day = date(2026, 11, 8);
				let snapshot = Snapshot {
					calendars: vec![],
					events: vec![],
					day,
					fetched_at: timeparagon_core::clock::now(),
					notice: None,
				};
				let settings = CalendarSettings::default();
				let dates = schedule_dates(PlanView::MultiDay, day, 3, settings.first_day_of_week);
				let mut grid_dom =
					VirtualDom::new_with_props(grid_preview, (snapshot, dates, settings));
				grid_dom.rebuild_in_place();
				let grid = dioxus_ssr::render(&grid_dom);
				let html = if mode == "multi" {
					html.replace("</section>", &format!("{grid}</section>"))
				} else {
					html
				};
				let page = format!(
					"<!doctype html><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><style>{}</style><div class=\"app\">{html}</div>",
					crate::style::STYLESHEET.as_str()
				);
				std::fs::write(format!("{directory}/{mode}.html"), page).unwrap();
			}
		}
	}

	fn grid_preview(
		(snapshot, dates, settings): (Snapshot, Vec<NaiveDate>, CalendarSettings),
	) -> Element {
		schedule_grid(
			&snapshot,
			&dates,
			&settings,
			TimeFormat::TwelveHour,
			3,
			EventHandler::new(|_| {}),
			EventHandler::new(|_| {}),
			None,
		)
	}

	#[test]
	fn shared_grid_places_events_in_their_own_days_and_clips_overnight_events() {
		use chrono::{Local, TimeZone};
		let first = date(2026, 11, 8);
		let at = |day: NaiveDate, hour| {
			Local
				.from_local_datetime(&day.and_hms_opt(hour, 0, 0).unwrap())
				.single()
				.unwrap()
		};
		let event = |title: &str, start, end, all_day| Event {
			id: title.into(),
			calendar_id: "work".into(),
			title: title.into(),
			start,
			end,
			all_day,
			color: calendar::colors::DEFAULT_EVENT,
			completed: false,
			details: calendar::Details::default(),
		};
		let second = first + Duration::days(1);
		let third = second + Duration::days(1);
		let snapshot = Snapshot {
			calendars: vec![calendar::Calendar {
				id: "work".into(),
				name: "Work".into(),
				color: calendar::colors::DEFAULT_EVENT,
				primary: false,
				selected_by_google: true,
			}],
			events: vec![
				event("Sunday meeting", at(first, 9), at(first, 10), false),
				event("Monday meeting", at(second, 9), at(second, 10), false),
				event("Overnight", at(first, 23), at(second, 1), false),
				event(
					"Tuesday holiday",
					at(third, 0),
					at(third + Duration::days(1), 0),
					true,
				),
			],
			day: first,
			fetched_at: at(first, 12),
			notice: None,
		};
		let mut dom = VirtualDom::new_with_props(
			grid_preview,
			(
				snapshot,
				vec![first, second, third],
				CalendarSettings::default(),
			),
		);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("Tuesday holiday"));
		assert_eq!(html.matches("class=\"gutter\"").count(), 1);
		let sunday = html
			.split("aria-label=\"2026-11-08\"")
			.nth(1)
			.unwrap()
			.split("aria-label=\"2026-11-09\"")
			.next()
			.unwrap();
		let monday = html
			.split("aria-label=\"2026-11-09\"")
			.nth(1)
			.unwrap()
			.split("aria-label=\"2026-11-10\"")
			.next()
			.unwrap();
		let tuesday = html.split("aria-label=\"2026-11-10\"").nth(1).unwrap();
		assert!(sunday.contains("Sunday meeting") && sunday.contains("Overnight"));
		assert!(!sunday.contains("Monday meeting"));
		assert!(monday.contains("Monday meeting") && monday.contains("Overnight"));
		assert!(!monday.contains("Sunday meeting"));
		assert!(!tuesday.contains("meeting") && !tuesday.contains("Overnight"));
	}

	#[test]
	fn schedule_list_renders_all_day_first_with_overnight_rows_and_current_time() {
		fn app() -> Element {
			use chrono::{Local, TimeZone};
			let first = date(2026, 11, 8);
			let second = date(2026, 11, 9);
			let at = |day: NaiveDate, hour| {
				Local
					.from_local_datetime(&day.and_hms_opt(hour, 0, 0).unwrap())
					.single()
					.unwrap()
			};
			let event = |title: &str, start, end, all_day| Event {
				id: title.into(),
				calendar_id: "work".into(),
				title: title.into(),
				start,
				end,
				all_day,
				color: calendar::colors::DEFAULT_EVENT,
				completed: false,
				details: calendar::Details::default(),
			};
			let snapshot = Snapshot {
				calendars: vec![],
				events: vec![
					event("Evening work", at(first, 18), at(first, 19), false),
					event("All-day work", at(first, 0), at(second, 0), true),
					event("Morning work", at(first, 9), at(first, 10), false),
					event("Sleep", at(first, 22), at(second, 6), false),
				],
				day: first,
				fetched_at: at(first, 12),
				notice: None,
			};
			let anchor = use_signal(move || first);
			schedule_list(
				&snapshot,
				&[first, second],
				&CalendarSettings::default(),
				TimeFormat::TwentyFourHour,
				&Search::default(),
				at(first, 12),
				EventHandler::new(|_| {}),
				anchor,
			)
		}
		let mut dom = VirtualDom::new(app);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.find("All-day work").unwrap() < html.find("Morning work").unwrap());
		assert!(html.find("Morning work").unwrap() < html.find("Current time").unwrap());
		assert!(html.find("Current time").unwrap() < html.find("Evening work").unwrap());
		assert!(html.contains("Sleep (Day 1/2)"));
		assert!(html.contains("Sleep (Day 2/2)"));
		assert!(html.contains("schedule-row past"));
		assert!(html.contains("22:00 – 06:00"));
	}

	#[test]
	fn strip_keeps_selection_for_every_week_start() {
		for first in FirstDayOfWeek::ALL {
			for offset in 0..40 {
				let selected = date(2026, 10, 25) + Duration::days(offset);
				let week = strip_dates(selected, false, first);
				let month = strip_dates(selected, true, first);
				assert_eq!(week.len(), 7);
				assert_eq!(week[0], first.week_start(selected));
				assert!(week.contains(&selected));
				assert!(month.contains(&selected));
				assert_eq!(month.len() % 7, 0);
				assert!(week.iter().all(|day| month.contains(day)));
			}
		}
	}

	#[test]
	fn multi_day_selection_continues_across_month_and_year() {
		let first = CalendarSettings::default().first_day_of_week;
		let anchor = date(2026, 12, 31);
		assert_eq!(
			schedule_dates(PlanView::Day, anchor, 3, first),
			vec![anchor]
		);
		assert_eq!(
			schedule_dates(PlanView::MultiDay, anchor, 2, first),
			vec![anchor, date(2027, 1, 1)]
		);
		assert_eq!(
			schedule_dates(PlanView::MultiDay, anchor, 3, first),
			vec![anchor, date(2027, 1, 1), date(2027, 1, 2)]
		);
		let month = schedule_dates(PlanView::Month, anchor, 3, first);
		assert!(month.contains(&date(2026, 12, 1)));
		assert!(month.contains(&date(2026, 12, 31)));
	}
}

#[allow(clippy::too_many_arguments)]
fn schedule_list(
	snapshot: &Snapshot,
	dates: &[NaiveDate],
	settings: &CalendarSettings,
	format: TimeFormat,
	search: &Search,
	now: chrono::DateTime<chrono::Local>,
	on_open: EventHandler<Event>,
	mut anchor: Signal<NaiveDate>,
) -> Element {
	let days = day::schedule_agenda(snapshot, dates, settings);
	rsx! {
		div { key: "{dates.first():?}", class: "schedule-list", aria_label: "Schedule",
			for (date, rows) in days {
				{
					let today = date == now.date_naive();
					let needle = if today {
						Some(rows.iter().position(|row| !row.event.all_day && row.event.start > now).unwrap_or(rows.len()))
					} else { None };
					rsx! {
						section { key: "{date}", class: "schedule-day",
							button {
								class: if today { "schedule-date today" } else { "schedule-date" },
								aria_label: date.format("%A, %B %-d, %Y").to_string(),
								onclick: move |_| anchor.set(date),
								span { class: "schedule-day-number", {date.format("%-d").to_string()} }
								span { {date.format("%b, %a").to_string()} }
							}
							div { class: "schedule-rows",
								if rows.is_empty() { p { class: "schedule-empty hint", "No events" } }
								for (index, row) in rows.iter().enumerate() {
									{
										let event = row.event.clone();
										let hidden = search.hides_details(&event);
										let title = if hidden { timeparagon_core::calendar::search::PRIVATE_TITLE.into() } else { calendar::schedule::title(&event, date) };
										let dot = crate::css_color(if hidden { timeparagon_core::calendar::search::PRIVATE_COLOR } else { row.color });
										let past = !event.all_day && event.end <= now;
										let time = calendar::schedule::time_label(&event, format);
										rsx! {
											div { key: "{event.id}", class: "schedule-entry", "data-sending": event.details.sending,
											if needle == Some(index) { div { class: "schedule-now", role: "separator", aria_label: "Current time" } }
											button {
												class: if past { "schedule-row past" } else { "schedule-row" },
												onclick: move |_| on_open.call(event.clone()),
												span { class: "schedule-dot", style: "background:{dot}" }
												span { class: "schedule-time", "{time}" }
												span { class: "schedule-title", "{title}" }
											}
											}
										}
									}
								}
								if needle == Some(rows.len()) { div { class: "schedule-now", role: "separator", aria_label: "Current time" } }
							}
						}
					}
				}
			}
		}
	}
}
