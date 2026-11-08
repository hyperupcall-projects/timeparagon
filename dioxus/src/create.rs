//! Adding things: a task with the quick-add keywords, an event, a time block.

use chrono::{NaiveDate, Timelike};
use dioxus::prelude::*;
use timeslay_core::calendar::Calendar;
use timeslay_core::config::{CalendarSettings, ProjectsSettings};
use timeslay_core::task_input::{self, Value};

use crate::day::{self, NewEvent};
use crate::editor::worker;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Task,
    Event,
    Block,
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
    let (start, end) = default_times(day, today, timeslay_core::clock::now().hour());
    let mut kind = use_signal(|| Kind::Task);
    let mut title = use_signal(String::new);
    let mut on_day = use_signal(|| false);
    let mut description = use_signal(String::new);
    let mut date = use_signal(|| day.to_string());
    let mut end_date = use_signal(|| day.to_string());
    let mut all_day = use_signal(|| false);
    let mut start = use_signal(move || start);
    let mut end = use_signal(move || end);
    let mut calendar_id = use_signal(move || default_calendar.unwrap_or_default());
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
    let tab = |label: &'static str, this: Kind| {
        rsx! {
            button {
                class: "nav-tab",
                r#type: "button",
                aria_pressed: (kind() == this).to_string(),
                onclick: move |_| {
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
                    let task = day::quick_task(&text, today, &projects);
                    day::create_task(task, due_day, &settings).map(|_| ())
                }
                Kind::Event if target.is_empty() => Err("Choose a calendar".to_owned()),
                Kind::Event => day::create_event(&new, &target),
                Kind::Block => day::create_time_block(&new, &settings),
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
                }
                div { class: "modal-body editor-body",
                    if let Some(message) = error() {
                        p { class: "editor-error", role: "alert", "{message}" }
                    }
                    input {
                        class: "client-input",
                        r#type: "text",
                        placeholder: if kind() == Kind::Task { "Pay rent tomorrow p1 @home ~30m" } else { "Title" },
                        aria_label: "Title",
                        value: "{title}",
                        oninput: move |event| title.set(event.value()),
                    }
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
                                    aria_label: "Start",
                                    value: "{start}",
                                    oninput: move |event| start.set(event.value()),
                                }
                                input {
                                    class: "client-input",
                                    r#type: "time",
                                    aria_label: "End",
                                    value: "{end}",
                                    oninput: move |event| end.set(event.value()),
                                }
                            }
                        }
                        textarea {
                            class: "client-input",
                            placeholder: "Description",
                            aria_label: "Description",
                            value: "{description}",
                            oninput: move |event| description.set(event.value()),
                        }
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
