//! Adding things: a task with the quick-add keywords, an event, a time block,
//! or a night's sleep.

use chrono::{Datelike, NaiveDate, Timelike};
use dioxus::prelude::*;
use timeparagon_core::calendar::{Calendar, Details, Event, sleep};
use timeparagon_core::config::{CalendarSettings, ProjectsSettings};
use timeparagon_core::task_input::{self, Value};

use crate::ai_input::{self, TaskDraft};
use crate::day::{self, NewEvent};
use crate::editor::worker;
use serde_json::json;

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
	#[props(default)] models: timeparagon_core::ai_input::Settings,
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
	let ai_active = use_signal(|| false);
	let mut task_details = use_signal(TaskDraft::default);
	let mut proposals = use_signal(Vec::<NewEvent>::new);
	let mut selected = use_signal(|| None::<usize>);
	let form = EventForm {
		title,
		description,
		date,
		end_date,
		start,
		end,
		all_day,
		color_input,
	};
	let mut review = Review {
		proposals,
		selected,
		form,
	};
	let (context_title, mut context_task) = task_details().merge(&title(), today, &projects);
	if context_task.due.is_empty() && on_day() {
		context_task.due = day.to_string();
	}
	let ai_context = json!({"title":context_title,"description":description(),"due":context_task.due,"due_time":context_task.due_time,"deadline":context_task.deadline,"metadata":context_task.metadata,"projects":projects.items,"today":today.to_string()});
	let patch_projects = projects.clone();

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
				disabled: busy() || ai_active(),
				aria_pressed: (kind() == this).to_string(),
				onclick: move |_| {
					if kind() == Kind::Event { review.stash(); selected.set(None); }
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

	let submit_projects = projects.clone();
	let submit = move |event: FormEvent| {
		event.prevent_default();
		if busy() || ai_active() {
			return;
		}
		let settings = settings.clone();
		let projects = submit_projects.clone();
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
		let (task_title, mut task) = task_details().merge(&text, today, &projects);
		if task.due.is_empty() && on_day() {
			task.due = day.to_string();
		}
		let saving_proposal = if chosen == Kind::Event {
			selected()
		} else {
			None
		};
		busy.set(true);
		error.set(None);
		spawn(async move {
			let result = worker(move || match chosen {
				Kind::Task => {
					task.metadata.color =
						timeparagon_core::calendar::colors::selection(&new.color_input)?;
					task.save(&task_title, &new.description, &settings)
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
					if let Some(index) = saving_proposal {
						proposals.write().remove(index);
						selected.set(None);
						if !proposals.read().is_empty() {
							review.select(index.min(proposals.read().len() - 1));
							return;
						}
					}
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
						disabled: busy(),
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
					if !proposals.read().is_empty() {
						div { class: "ai-review",
							p { class:"hint", "Detected events — select one to review, or remove it." }
							for (index, proposal) in proposals().iter().enumerate() {
								div { class:"ai-proposal",
									button { class:"btn", r#type:"button",disabled:busy() || ai_active(),aria_pressed:(selected() == Some(index)).to_string(),
										onclick:move |_| { review.select(index); kind.set(Kind::Event); }, "{proposal.title}" }
									button {class:"btn",r#type:"button",aria_label:"Remove event proposal",disabled:busy() || ai_active(),onclick:move |_|review.remove(index), "×"}
								}
							}
						}
					}
					div { class:"ai-title-row",

					if kind() != Kind::Sleep { input {
						class: "client-input",
						r#type: "text",
						placeholder: if kind() == Kind::Task { "Pay rent tomorrow p1 @home ~30m" } else { "Title" },
						aria_label: "Title",
						value: "{title}",
						oninput: move |event| title.set(event.value()),
					} }
					if matches!(kind(), Kind::Task | Kind::Event) {
						ai_input::Controls {
							settings: models, context: ai_context, voice: kind() == Kind::Task, disabled: busy(), active: ai_active,
							on_patch: move |patch: timeparagon_core::ai_input::voice::Patch| {
								let (plain_title, mut draft) = task_details().merge(&title(), today, &patch_projects);
								if draft.due.is_empty() && on_day() { draft.due = day.to_string(); }
								draft.apply(&patch);
								title.set(patch.title.unwrap_or(plain_title));
								if let Some(value) = patch.description { description.set(value); }
								task_details.set(draft); on_day.set(false);
							},
							on_images: move |found: Vec<timeparagon_core::ai_input::image::Found>| {
								match found.into_iter().map(picture_draft).collect::<Result<Vec<_>,_>>() {
									Ok(events) => {
										if kind() == Kind::Event { review.stash(); }
										selected.set(None);
										let first = proposals.read().len();
										proposals.write().extend(events);
										review.select(first); kind.set(Kind::Event);
									},
									Err(message) => error.set(Some(message)),
								}
							},
						}
					}
					}
					if kind() == Kind::Task {
						TaskDetails { draft: task_details, projects: projects.clone(), disabled: busy() }
						textarea { class:"client-input",aria_label:"Task description",placeholder:"Description",value:"{description}",oninput:move |event|description.set(event.value()) }
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
					button { class: "btn primary editor-wide", r#type: "submit", disabled: busy() || ai_active(),
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

#[derive(Clone, Copy)]
struct EventForm {
	title: Signal<String>,
	description: Signal<String>,
	date: Signal<String>,
	end_date: Signal<String>,
	start: Signal<String>,
	end: Signal<String>,
	all_day: Signal<bool>,
	color_input: Signal<String>,
}
impl EventForm {
	fn get(self) -> NewEvent {
		NewEvent {
			title: (self.title)(),
			description: (self.description)(),
			date: (self.date)(),
			end_date: (self.end_date)(),
			start: (self.start)(),
			end: (self.end)(),
			all_day: (self.all_day)(),
			color_input: (self.color_input)(),
		}
	}
	fn set(mut self, value: NewEvent) {
		self.title.set(value.title);
		self.description.set(value.description);
		self.date.set(value.date);
		self.end_date.set(value.end_date);
		self.start.set(value.start);
		self.end.set(value.end);
		self.all_day.set(value.all_day);
		self.color_input.set(value.color_input);
	}
}
#[derive(Clone, Copy)]
struct Review {
	proposals: Signal<Vec<NewEvent>>,
	selected: Signal<Option<usize>>,
	form: EventForm,
}
impl Review {
	fn stash(&mut self) {
		if let Some(index) = (self.selected)() {
			if let Some(slot) = self.proposals.write().get_mut(index) {
				*slot = self.form.get();
			}
		}
	}
	fn select(&mut self, index: usize) {
		self.stash();
		let value = self.proposals.read().get(index).cloned();
		if let Some(value) = value {
			self.selected.set(Some(index));
			self.form.set(value);
		}
	}
	fn remove(&mut self, index: usize) {
		self.stash();
		self.proposals.write().remove(index);
		let old = (self.selected)();
		self.selected.set(None);
		if let Some(old) = old {
			if !self.proposals.read().is_empty() {
				self.select(if old > index {
					old - 1
				} else {
					old.min(self.proposals.read().len() - 1)
				});
			} else {
				self.form.title.set(String::new());
				self.form.description.set(String::new());
			}
		}
	}
}

fn picture_draft(found: timeparagon_core::ai_input::image::Found) -> Result<NewEvent, String> {
	use chrono::{NaiveDate, NaiveTime};
	let parse_date = |v: &str| {
		NaiveDate::parse_from_str(v, "%Y-%m-%d")
			.map_err(|_| "Image model returned an invalid date".to_owned())
	};
	let parse_time = |v: &str| {
		NaiveTime::parse_from_str(v, "%H:%M")
			.map_err(|_| "Image model returned an invalid time".to_owned())
	};
	let date = found.date.as_deref().map(parse_date).transpose()?;
	let end_date = found
		.end_date
		.as_deref()
		.map(parse_date)
		.transpose()?
		.or(date);
	let start = found.start_time.as_deref().map(parse_time).transpose()?;
	let end = found.end_time.as_deref().map(parse_time).transpose()?;

	if let Some(date) = date {
		use chrono::TimeZone;
		for (day, time) in [
			(date, start.unwrap_or_default()),
			(end_date.unwrap_or(date), end.unwrap_or_default()),
		] {
			if chrono::Local
				.from_local_datetime(&day.and_time(time))
				.single()
				.is_none()
			{
				return Err("Picture contains an ambiguous or nonexistent local time; review the source or try another image".into());
			}
		}
		if let (Some(start), Some(end)) = (start, end) {
			if end_date.unwrap_or(date).and_time(end) <= date.and_time(start) {
				return Err("Image model returned an event ending before it starts".into());
			}
		}
	}
	let mut description = found.description.unwrap_or_default();
	if let Some(location) = found.location.filter(|v| !v.is_empty()) {
		description.push_str(&format!("\nLocation: {location}"));
	}
	if date.is_none() {
		description.push_str("\nDate not found in picture; enter a date before adding.");
	}
	let mut draft = NewEvent {
		title: found.title,
		description,
		date: date.map(|v| v.to_string()).unwrap_or_default(),
		end_date: end_date.map(|v| v.to_string()).unwrap_or_default(),
		start: start
			.map(|v| v.format("%H:%M").to_string())
			.unwrap_or_default(),
		end: end
			.map(|v| v.format("%H:%M").to_string())
			.unwrap_or_default(),
		all_day: start.is_none(),
		color_input: String::new(),
	};
	if let Some(start) = start {
		if end.is_none() {
			let (end, rollover) = start.overflowing_add_signed(chrono::Duration::hours(1));
			draft.end = end.format("%H:%M").to_string();
			if rollover > 0 {
				draft.end_date = end_date
					.and_then(|v| v.succ_opt())
					.map(|v| v.to_string())
					.unwrap_or_default();
			}
		}
	}
	if date.is_some() {
		draft.range()?;
	}
	Ok(draft)
}

#[component]
fn TaskDetails(
	mut draft: Signal<TaskDraft>,
	projects: ProjectsSettings,
	disabled: bool,
) -> Element {
	rsx! {
		details { class:"ai-task-details", open: !draft().due.is_empty() || !draft().deadline.is_empty() || draft().metadata != Default::default(),
			summary {"Task details"}
			div {class:"sheet-row",
				label {"Due date" input {class:"client-input",r#type:"date",value:"{draft().due}",disabled,oninput:move |event| { let mut d = draft.write(); d.due = event.value(); if d.due.is_empty() {d.due_time.clear();} }}}
				label {"Due time" input {class:"client-input",r#type:"time",value:"{draft().due_time}",disabled,oninput:move |event|draft.write().due_time = event.value()}}
			}
			label {"Deadline" input {class:"client-input",r#type:"text",placeholder:"YYYY-MM-DD or YYYY-MM-DD HH:MM",value:"{draft().deadline}",disabled,oninput:move |event|draft.write().deadline = event.value()}}
			div {class:"sheet-row",
				label {"Priority" select {class:"client-input",disabled,value:draft().metadata.priority.map(|p|format!("{p:?}")).unwrap_or_default(),onchange:move |event|draft.write().metadata.priority = match event.value().as_str() {"P1"=>Some(timeparagon_core::timeblocking::Priority::P1),"P2"=>Some(timeparagon_core::timeblocking::Priority::P2),_=>None},
					option {value:"","None"} option {value:"P1","P1"} option {value:"P2","P2"}
				}}
				label {"Estimate (minutes)" input {class:"client-input",r#type:"number",min:0,max:10080,value:"{draft().metadata.minutes}",disabled,oninput:move |event| {if let Ok(value) = event.value().parse::<u32>() {draft.write().metadata.minutes = value.min(10080);}}}}
			}
			label {"Labels" input {class:"client-input",r#type:"text",value:draft().metadata.labels.join(", "),disabled,onchange:move |event|draft.write().metadata.labels = event.value().split(',').map(str::trim).filter(|v|!v.is_empty()).map(str::to_owned).collect()}}
			for project in projects.items {
				label {class:"sheet-check",input {r#type:"checkbox",disabled,checked:draft().metadata.projects.contains(&project.id),onchange:move |event:FormEvent| {let mut d=draft.write();d.metadata.projects.retain(|p|*p != project.id);if event.checked() {d.metadata.projects.push(project.id);}}},"{project.name}"}
			}
		}
	}
}

#[cfg(test)]
mod media_tests {
	use super::*;
	use timeparagon_core::ai_input::image::Found;
	fn found() -> Found {
		Found {
			title: "Talk".into(),
			date: None,
			end_date: None,
			start_time: None,
			end_time: None,
			description: None,
			location: Some("Library".into()),
		}
	}
	#[test]
	fn pictures_require_missing_dates_and_default_to_all_day_or_one_hour() {
		let draft = picture_draft(found()).unwrap();
		assert!(draft.date.is_empty() && draft.all_day);
		assert!(draft.description.contains("Library"));
		assert!(draft.range().is_err());
		let mut event = found();
		event.date = Some("2026-11-17".into());
		event.start_time = Some("23:30".into());
		let draft = picture_draft(event).unwrap();
		assert_eq!(draft.end_date, "2026-11-18");
		assert_eq!(draft.end, "00:30");
		let (start, end) = draft.range().unwrap();
		assert_eq!(end - start, chrono::Duration::hours(1));
		let mut invalid = found();
		invalid.date = Some("2026-02-30".into());
		assert!(picture_draft(invalid).is_err());
		let mut backwards = found();
		backwards.date = Some("2026-11-17".into());
		backwards.start_time = Some("10:00".into());
		backwards.end_time = Some("09:00".into());
		assert!(picture_draft(backwards).is_err());
	}
	#[test]
	fn switching_and_removing_proposals_preserves_edits() {
		fn app() -> Element {
			let title = use_signal(String::new);
			let description = use_signal(String::new);
			let date = use_signal(String::new);
			let end_date = use_signal(String::new);
			let start = use_signal(String::new);
			let end = use_signal(String::new);
			let all_day = use_signal(|| false);
			let color_input = use_signal(String::new);
			let form = EventForm {
				title,
				description,
				date,
				end_date,
				start,
				end,
				all_day,
				color_input,
			};
			let proposals = use_signal(|| {
				vec![
					picture_draft(found()).unwrap(),
					picture_draft(Found {
						title: "Concert".into(),
						..found()
					})
					.unwrap(),
				]
			});
			let selected = use_signal(|| None);
			use_hook(move || {
				let mut review = Review {
					proposals,
					selected,
					form,
				};
				review.select(0);
				review.form.title.set("Edited talk".into());
				review.select(1);
				review.form.description.set("Keep my notes".into());
				review.select(0);
				assert_eq!((form.title)(), "Edited talk");
				review.remove(0);
				assert_eq!((form.title)(), "Concert");
				assert_eq!((form.description)(), "Keep my notes");
				review.remove(0);
				assert!((form.title)().is_empty());
			});
			rsx! {div {}}
		}
		let mut dom = VirtualDom::new(app);
		dom.rebuild_in_place();
	}
	#[test]
	fn media_controls_follow_the_title_and_offer_manual_task_review() {
		fn app() -> Element {
			rsx! {CreateSheet {settings:CalendarSettings::default(),projects:ProjectsSettings::default(),calendars:vec![],default_calendar:None,day:NaiveDate::from_ymd_opt(2026,11,17).unwrap(),on_created:|_|{},on_close:|_|{}}}
		}
		let mut dom = VirtualDom::new(app);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		let title = html.find("aria-label=\"Title\"").unwrap();
		let mic = html.find("aria-label=\"Voice input\"").unwrap();
		let image = html.find("aria-label=\"Picture input\"").unwrap();
		assert!(title < mic && mic < image);
		for field in [
			"Task details",
			"Due date",
			"Due time",
			"Deadline",
			"Estimate (minutes)",
			"Labels",
		] {
			assert!(html.contains(field), "{field} missing");
		}
	}
}
