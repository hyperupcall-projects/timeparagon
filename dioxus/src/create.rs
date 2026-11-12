//! Adding things: a task with the quick-add keywords, an event, a time block,
//! or a night's sleep.

use chrono::{Datelike, NaiveDate, Timelike};
use dioxus::prelude::*;
use timeparagon_core::calendar::{Calendar, Details, Event, sleep};
use timeparagon_core::config::{CalendarSettings, ProjectsSettings};
use timeparagon_core::task_input::{self, Value};

use crate::day::{self, NewEvent};
use crate::editor::worker;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
	Task,
	Event,
	Block,
	Sleep,
}

/// When a new event starts by default: the next hour today, or 9 on another day.
fn default_times(day: NaiveDate, today: NaiveDate, now_hour: u32) -> (String, String) {
	let hour = if day == today {
		(now_hour + 1).min(22)
	} else {
		9
	};
	(format!("{hour:02}:00"), format!("{:02}:00", hour + 1))
}

/// A night ending on `day`, starting from the weekly schedule: (start date,
/// wake-up date, bedtime, wake-up).
fn sleep_defaults(day: NaiveDate, settings: &CalendarSettings) -> (String, String, String, String) {
	let mut schedule = settings.sleep_schedule;
	schedule[day.weekday().num_days_from_monday() as usize] = sleep::times_for(settings, day);
	let (start, end) = sleep::plan(day, &schedule);
	(
		start.date_naive().to_string(),
		end.date_naive().to_string(),
		start.format("%H:%M").to_string(),
		end.format("%H:%M").to_string(),
	)
}

/// Records a night typed in by hand. Blocking: callers run it on a worker.
fn create_sleep(new: &NewEvent, settings: &CalendarSettings) -> Result<(), String> {
	let (start, end) = NewEvent {
		title: "Sleep".to_owned(),
		..new.clone()
	}
	.range()?;
	let event = Event {
		id: sleep::NEW_ID.into(),
		calendar_id: settings.app_calendar.clone().unwrap_or_default(),
		title: "Sleep".into(),
		start,
		end,
		all_day: false,
		color: sleep::COLOR,
		completed: false,
		details: Details {
			sleep: Some(sleep::Record {
				automatic: false,
				estimated_start: false,
				estimated_end: false,
				..sleep::Record::planned(end.date_naive())
			}),
			..Default::default()
		},
	};
	sleep::save(settings, event, true).map(|_| ())
}

/// What a recognized keyword will set, in words.
fn token_label(value: &Value, projects: &ProjectsSettings) -> String {
	match value {
		Value::Due(day) => format!("Due {}", day.format("%b %-d")),
		Value::Deadline(day) => format!("Deadline {}", day.format("%b %-d")),
		Value::Label(label) => format!("Label {label}"),
		Value::Project(id) => format!(
			"Project {}",
			projects
				.get(*id)
				.map_or("?", |project| project.name.as_str())
		),
		Value::Priority(priority) => format!("Priority {priority:?}"),
		Value::Minutes(minutes) => format!("Estimate {}", day::duration_label(*minutes)),
	}
}

#[component]
pub fn CreateSheet(
	settings: CalendarSettings,
	projects: ProjectsSettings,
	calendars: Vec<Calendar>,
	default_calendar: Option<String>,
	day: NaiveDate,
	on_created: EventHandler<()>,
	on_close: EventHandler<()>,
) -> Element {
	let today = day::today(&settings);
	let (start, end) = default_times(day, today, timeparagon_core::clock::now().hour());
	let mut kind = use_signal(|| Kind::Task);
	let mut title = use_signal(String::new);
	let mut on_day = use_signal(|| false);
	let mut description = use_signal(String::new);
	let mut color_input = use_signal(String::new);
	let mut date = use_signal(|| day.to_string());
	let mut end_date = use_signal(|| day.to_string());
	let mut all_day = use_signal(|| false);
	let mut start = use_signal(move || start);
	let mut end = use_signal(move || end);
	let event_default = default_calendar
		.filter(|id| {
			calendars.iter().any(|calendar| {
				&calendar.id == id
					&& !calendar.is_task_list()
					&& !calendar.is_app_calendar(&settings)
			})
		})
		.or_else(|| {
			calendars
				.iter()
				.find(|calendar| !calendar.is_task_list() && !calendar.is_app_calendar(&settings))
				.map(|calendar| calendar.id.clone())
		});
	let mut calendar_id = use_signal(move || event_default.unwrap_or_default());
	let mut busy = use_signal(|| false);
	let mut error = use_signal(|| None::<String>);

	let targets: Vec<Calendar> = calendars
		.into_iter()
		.filter(|calendar| !calendar.is_task_list() && !calendar.is_app_calendar(&settings))
		.collect();
	let tokens: Vec<String> = if kind() == Kind::Task {
		task_input::detect(&title(), today, &projects)
			.iter()
			.map(|token| token_label(&token.value, &projects))
			.collect()
	} else {
		Vec::new()
	};
	let sleep_settings = settings.clone();
	let tab = |label: &'static str, this: Kind| {
		let sleep_settings = sleep_settings.clone();
		rsx! {
			button {
				class: "nav-tab",
				r#type: "button",
				aria_pressed: (kind() == this).to_string(),
				onclick: move |_| {
					if this == Kind::Sleep && kind() != Kind::Sleep {
						let (first, last, bedtime, wakeup) = sleep_defaults(day, &sleep_settings);
						date.set(first);
						end_date.set(last);
						start.set(bedtime);
						end.set(wakeup);
					}
					kind.set(this);
					error.set(None);
				},
				"{label}"
			}
		}
	};

	let submit = move |event: FormEvent| {
		event.prevent_default();
		if busy() {
			return;
		}
		let settings = settings.clone();
		let projects = projects.clone();
		// A time block is always timed.
		let chosen_all_day = kind() == Kind::Event && all_day();
		let new = NewEvent {
			color_input: color_input(),
			title: title(),
			description: description(),
			date: date(),
			end_date: end_date(),
			start: start(),
			end: end(),
			all_day: chosen_all_day,
		};
		let target = calendar_id();
		let chosen = kind();
		let text = title();
		let due_day = on_day().then_some(day);
		busy.set(true);
		error.set(None);
		spawn(async move {
			let result = worker(move || match chosen {
				Kind::Task => {
					let mut task = day::quick_task(&text, today, &projects);
					task.metadata.color =
						timeparagon_core::calendar::colors::selection(&new.color_input)?;
					day::create_task(task, due_day, &settings).map(|_| ())
				}
				Kind::Event if target.is_empty() => Err("Choose a calendar".to_owned()),
				Kind::Event => day::create_event(&new, &target),
				Kind::Block => day::create_time_block(&new, &settings),
				Kind::Sleep => create_sleep(&new, &settings),
			})
			.await;
			busy.set(false);
			match result {
				Ok(()) => {
					on_created.call(());
					on_close.call(());
				}
				Err(message) => error.set(Some(message)),
			}
		});
	};

	rsx! {
		div { class: "modal-backdrop editor-backdrop",
			form { class: "modal editor", role: "dialog", aria_label: "Add", onsubmit: submit,
				div { class: "modal-head",
					div { class: "modal-titles", h2 { class: "modal-title", "Add" } }
					button {
						class: "modal-close",
						r#type: "button",
						title: "Close",
						onclick: move |_| on_close.call(()),
						"×"
					}
				}
				div { class: "sheet-tabs",
					{tab("Task", Kind::Task)}
					{tab("Event", Kind::Event)}
					{tab("Time block", Kind::Block)}
					{tab("Sleep", Kind::Sleep)}
				}
				div { class: "modal-body editor-body",
					if let Some(message) = error() {
						p { class: "editor-error", role: "alert", "{message}" }
					}
					if kind() != Kind::Sleep { input {
						class: "client-input",
						r#type: "text",
						placeholder: if kind() == Kind::Task { "Pay rent tomorrow p1 @home ~30m" } else { "Title" },
						aria_label: "Title",
						value: "{title}",
						oninput: move |event| title.set(event.value()),
					} }
					if kind() == Kind::Task {
						div { class: "token-row",
							for token in tokens {
								span { key: "{token}", class: "meta-pill", "{token}" }
							}
						}
						label { class: "sheet-check",
							input {
								r#type: "checkbox",
								checked: on_day(),
								onchange: move |event: FormEvent| on_day.set(event.checked()),
							}
							"Due {day.format(\"%b %-d\")}, unless it names a day"
						}
						p { class: "hint", "Without a day, it goes to the Inbox." }
					} else {
						if kind() == Kind::Event {
							select {
								class: "client-input",
								aria_label: "Calendar",
								onchange: move |event: FormEvent| calendar_id.set(event.value()),
								for target in targets {
									option {
										key: "{target.id}",
										value: "{target.id}",
										selected: target.id == calendar_id(),
										"{target.name}"
									}
								}
							}
						}
						if kind() == Kind::Event {
							label { class: "sheet-check",
								input {
									r#type: "checkbox",
									checked: all_day(),
									onchange: move |event: FormEvent| all_day.set(event.checked()),
								}
								"All day"
							}
						}
						div { class: "sheet-row",
							input {
								class: "client-input",
								r#type: "date",
								aria_label: "Start date",
								value: "{date}",
								oninput: move |event| {
									let value = event.value();
									// Keep the end from falling before the start.
									if end_date() < value {
										end_date.set(value.clone());
									}
									date.set(value);
								},
							}
							input {
								class: "client-input",
								r#type: "date",
								aria_label: "End date",
								value: "{end_date}",
								oninput: move |event| end_date.set(event.value()),
							}
						}
						if !(kind() == Kind::Event && all_day()) {
							div { class: "sheet-row",
								input {
									class: "client-input",
									r#type: "time",
									aria_label: if kind() == Kind::Sleep { "Bedtime" } else { "Start" },
									value: "{start}",
									oninput: move |event| start.set(event.value()),
								}
								input {
									class: "client-input",
									r#type: "time",
									aria_label: if kind() == Kind::Sleep { "Wake-up" } else { "End" },
									value: "{end}",
									oninput: move |event| end.set(event.value()),
								}
							}
						}
						if kind() != Kind::Sleep {
							crate::editor::ColorField { value: color_input(), busy: busy(),
								on_input: move |v| color_input.set(v),
								on_pick: move |v| color_input.set(v),
							}
						}
						if kind() == Kind::Sleep {
							p { class: "hint", "Bedtime and wake-up, from your weekly schedule. A night added here is kept when the schedule changes." }
						} else { textarea {
							class: "client-input",
							placeholder: "Description",
							aria_label: "Description",
							value: "{description}",
							oninput: move |event| description.set(event.value()),
						} }
					}
				}
				div { class: "editor-footer",
					button { class: "btn primary editor-wide", r#type: "submit", disabled: busy(),
						if busy() { "Adding…" } else { "Add" }
					}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_new_event_starts_at_the_next_hour_today_and_at_nine_otherwise() {
		let today = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
		let other = today.succ_opt().unwrap();
		assert_eq!(
			default_times(today, today, 13),
			("14:00".into(), "15:00".into())
		);
		assert_eq!(
			default_times(other, today, 13),
			("09:00".into(), "10:00".into())
		);
		// Late at night it still ends on the same day.
		assert_eq!(
			default_times(today, today, 23),
			("22:00".into(), "23:00".into())
		);
	}
}
