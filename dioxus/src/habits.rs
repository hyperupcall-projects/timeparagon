//! The habit tracker: goals tracked day by day, the way Loop Habit Tracker does.
//!
//! The rules are `timeslay_core::habits`, the same as on the desktop. The file
//! they are saved in is not synced by anything, on either app, so the phone
//! keeps its own record and says so on the screen.

use std::path::{Path, PathBuf};

use chrono::{Duration, NaiveDate};
use dioxus::prelude::*;
use timeslay_core::habits::{Frequency, Habit, Kind, Library, Measure, Status, format_amount};

/// How many days the tracker's row shows, ending today.
const DAYS: i64 = 5;

/// The saved habits, and where changes go. A file that exists but cannot be
/// read is left alone rather than replaced by an empty list on the next save.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Saved {
    pub library: Library,
    path: Option<PathBuf>,
    pub error: Option<String>,
}

pub fn read(path: Option<PathBuf>) -> Saved {
    let Some(path) = path else {
        return Saved {
            error: Some("There is nowhere to keep habits on this device.".to_owned()),
            ..Saved::default()
        };
    };
    let unreadable = |path: &Path| {
        format!(
            "{} could not be read, so changes to habits will not be saved.",
            path.display()
        )
    };
    match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(library) => Saved {
                library,
                path: Some(path),
                error: None,
            },
            Err(_) => Saved {
                error: Some(unreadable(&path)),
                ..Saved::default()
            },
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Saved {
            path: Some(path),
            ..Saved::default()
        },
        Err(_) => Saved {
            error: Some(unreadable(&path)),
            ..Saved::default()
        },
    }
}

impl Saved {
    fn save(&mut self) {
        let Some(path) = &self.path else { return };
        let written = serde_json::to_vec_pretty(&self.library)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                timeslay_core::fsutil::write_atomic(
                    path,
                    &bytes,
                    timeslay_core::fsutil::Kind::Private,
                )
            });
        self.error = written
            .err()
            .map(|error| format!("Could not save habits: {error}"));
    }
}

/// An id no habit on this device has.
fn new_id(library: &Library) -> String {
    let stamp = timeslay_core::clock::now().timestamp_micros();
    let mut id = format!("phone-{stamp:x}");
    while library.habit(&id).is_some() {
        id.push('x');
    }
    id
}

#[cfg(test)]
pub fn for_tests(habits: Vec<Habit>) -> Saved {
    Saved {
        library: Library {
            habits,
            ..Library::default()
        },
        ..Saved::default()
    }
}

/// What a day's button says.
pub fn day_symbol(status: Status) -> String {
    match status {
        Status::Yes | Status::Clean => "✓".to_owned(),
        Status::Auto => "·".to_owned(),
        Status::Partial(times) | Status::Slipped(times) => times.to_string(),
        Status::No => "×".to_owned(),
        Status::Skip => "»".to_owned(),
        Status::Unknown => "–".to_owned(),
    }
}

/// Parses what was typed as a habit's value or target.
pub fn parse_amount(text: &str) -> Result<f64, String> {
    match text.trim().parse::<f64>() {
        Ok(value) if value.is_finite() && value >= 0.0 => Ok(value),
        _ => Err("Enter an amount of zero or more".to_owned()),
    }
}

/// The goal a new habit asks for.
pub fn parse_new(
    name: &str,
    kind: Kind,
    times: &str,
    days: &str,
    target: &str,
    unit: &str,
    id: String,
) -> Result<Habit, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("A habit needs a name".to_owned());
    }
    let number = |text: &str| {
        text.trim()
            .parse::<u32>()
            .map_err(|_| "A frequency needs numbers above zero".to_owned())
    };
    let frequency = Frequency::Interval {
        times: number(times)?,
        days: number(days)?,
    };
    frequency.validate()?;
    let mut habit = Habit::new(id, name);
    habit.kind = kind;
    habit.frequency = frequency;
    habit.created = Some(timeslay_core::clock::now().date_naive());
    // Filling in either one tracks a value.
    if !target.trim().is_empty() || !unit.trim().is_empty() {
        habit.measure = Some(Measure {
            unit: unit.trim().to_owned(),
            target: if target.trim().is_empty() {
                None
            } else {
                Some(parse_amount(target)?)
            },
            days: 1,
        });
    }
    Ok(habit)
}

#[component]
pub fn TrackerSheet(
    today: NaiveDate,
    on_series: EventHandler<()>,
    on_close: EventHandler<()>,
) -> Element {
    let mut saved = use_signal(|| read(timeslay_core::habits::path()));
    let mut adding = use_signal(|| false);
    let open = use_signal(|| None::<String>);

    let rows: Vec<Element> = saved
        .read()
        .library
        .habits
        .iter()
        .filter(|habit| !habit.archived)
        .map(|habit| rsx! { HabitRow { key: "{habit.id}", habit: habit.clone(), today, saved, open } })
        .collect();
    let empty = rows.is_empty();
    let archived: Vec<(String, String)> = saved
        .read()
        .library
        .habits
        .iter()
        .filter(|habit| habit.archived)
        .map(|habit| (habit.id.clone(), habit.name.clone()))
        .collect();
    let error = saved.read().error.clone();

    rsx! {
        div { class: "modal-backdrop editor-backdrop",
            div { class: "modal editor", role: "dialog", aria_label: "Habits",
                div { class: "modal-head",
                    div { class: "modal-titles",
                        h2 { class: "modal-title", "Habits" }
                        div { class: "modal-sub", "Tracked on this phone only" }
                    }
                    button {
                        class: "modal-close",
                        title: "Close",
                        onclick: move |_| on_close.call(()),
                        "×"
                    }
                }
                div { class: "modal-body editor-body",
                    if let Some(message) = error {
                        p { class: "editor-error", role: "alert", "{message}" }
                    }
                    if empty && !adding() {
                        p { class: "empty-note", "No habits are tracked here yet." }
                    }
                    {rows.into_iter()}
                    if adding() {
                        NewHabit { saved, on_done: move |()| adding.set(false) }
                    } else {
                        button { class: "btn primary", onclick: move |_| adding.set(true), "New habit" }
                    }
                    for (id, name) in archived {
                        div { key: "{id}", class: "editor-item",
                            span { class: "editor-item-name hint", "{name} (archived)" }
                            button {
                                class: "btn small",
                                onclick: move |_| {
                                    let mut saved = saved.write();
                                    if let Some(habit) = saved.library.habit_mut(&id) {
                                        habit.archived = false;
                                    }
                                    saved.save();
                                },
                                "Restore"
                            }
                        }
                    }
                    button {
                        class: "btn",
                        onclick: move |_| on_series.call(()),
                        "Habit events on the calendar"
                    }
                }
            }
        }
    }
}

#[component]
pub(crate) fn HabitRow(
    habit: Habit,
    today: NaiveDate,
    saved: Signal<Saved>,
    open: Signal<Option<String>>,
) -> Element {
    let computed = habit.compute(today);
    let score = (computed.score(today) * 100.0).round() as i64;
    let streak = computed
        .best_streaks(usize::MAX)
        .into_iter()
        .find(|streak| streak.end >= today - Duration::days(1))
        .map_or(0, |streak| streak.length());
    let days = (0..DAYS).rev().map(|back| {
        let day = today - Duration::days(back);
        let status = computed.status(day);
        let symbol = day_symbol(status);
        let done = status.completed();
        let id = habit.id.clone();
        let toggle_day_id = habit.id.clone();
        let label = day.format("%a %-d").to_string();
        rsx! {
            div { key: "{day}", class: "habit-day",
                span { class: "habit-day-name", "{label}" }
                button {
                    class: if done { "habit-mark done" } else { "habit-mark" },
                    aria_label: "{habit.name} on {label}: {symbol}",
                    onclick: move |_| {
                        let mut saved = saved.write();
                        if let Some(habit) = saved.library.habit_mut(&toggle_day_id) {
                            habit.toggle(day, status);
                        }
                        saved.save();
                    },
                    "{symbol}"
                }
                if habit.measure.is_some() {
                    input {
                        class: "client-input habit-amount",
                        r#type: "number",
                        inputmode: "decimal",
                        min: "0",
                        step: "any",
                        aria_label: "Value for {habit.name} on {label}",
                        value: habit.value_on(day).map_or(String::new(), format_amount),
                        onchange: move |event: FormEvent| {
                            let text = event.value();
                            let value = if text.trim().is_empty() {
                                Ok(None)
                            } else {
                                parse_amount(&text).map(Some)
                            };
                            let mut saved = saved.write();
                            match value {
                                Ok(value) => {
                                    if let Some(habit) = saved.library.habit_mut(&id) {
                                        habit.set_value(day, value);
                                    }
                                    saved.save();
                                }
                                Err(message) => saved.error = Some(message),
                            }
                        },
                    }
                }
            }
        }
    });

    let expanded = open().as_deref() == Some(habit.id.as_str());
    let toggle_id = habit.id.clone();
    let archive_id = habit.id.clone();
    let delete_id = habit.id.clone();
    let mut confirm = use_signal(|| false);
    let color = css_palette(habit.color);
    let total = format_amount(computed.total());
    let best = computed
        .best_streaks(1)
        .first()
        .map_or(0, |streak| streak.length());

    rsx! {
        div { class: "habit-row",
            button {
                class: "habit-head",
                aria_expanded: expanded.to_string(),
                onclick: move |_| {
                    let next = (!expanded).then(|| toggle_id.clone());
                    let mut open = open;
                    open.set(next);
                },
                span { class: "day-dot", style: "background: {color};" }
                span { class: "editor-item-name",
                    "{habit.name}"
                    span { class: "hint editor-item-sub",
                        "{habit.goal_label()} · {score}% · streak {streak}"
                    }
                }
            }
            div { class: "habit-days", {days} }
            if expanded {
                div { class: "habit-details",
                    p { class: "hint", "Total {total} · best streak {best}" }
                    div { class: "sheet-row",
                        button {
                            class: "btn small",
                            onclick: move |_| {
                                let mut saved = saved.write();
                                if let Some(habit) = saved.library.habit_mut(&archive_id) {
                                    habit.archived = true;
                                }
                                saved.save();
                            },
                            "Archive"
                        }
                        if confirm() {
                            button {
                                class: "btn small danger",
                                onclick: move |_| {
                                    let mut saved = saved.write();
                                    saved.library.habits.retain(|habit| habit.id != delete_id);
                                    saved.save();
                                },
                                "Delete for good"
                            }
                            button { class: "btn small", onclick: move |_| confirm.set(false), "Keep" }
                        } else {
                            button { class: "btn small", onclick: move |_| confirm.set(true), "Delete" }
                        }
                    }
                }
            }
        }
    }
}

fn css_palette(index: u8) -> String {
    crate::css_color(timeslay_core::habits::palette_color(index, false))
}

#[component]
fn NewHabit(saved: Signal<Saved>, on_done: EventHandler<()>) -> Element {
    let mut name = use_signal(String::new);
    let mut kind = use_signal(|| Kind::Yes);
    let mut times = use_signal(|| "1".to_owned());
    let mut days = use_signal(|| "1".to_owned());
    let mut target = use_signal(String::new);
    let mut unit = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    rsx! {
        form {
            class: "habit-form",
            onsubmit: move |event| {
                event.prevent_default();
                let id = new_id(&saved.peek().library);
                match parse_new(&name(), kind(), &times(), &days(), &target(), &unit(), id) {
                    Ok(habit) => {
                        let mut saved = saved.write();
                        saved.library.habits.push(habit);
                        saved.save();
                        on_done.call(());
                    }
                    Err(message) => error.set(Some(message)),
                }
            },
            if let Some(message) = error() {
                p { class: "editor-error", role: "alert", "{message}" }
            }
            input {
                class: "client-input",
                r#type: "text",
                placeholder: "Name",
                aria_label: "Name",
                value: "{name}",
                oninput: move |event| name.set(event.value()),
            }
            select {
                class: "client-input",
                aria_label: "Kind",
                onchange: move |event: FormEvent| kind.set(match event.value().as_str() {
                    "no" => Kind::No,
                    _ => Kind::Yes,
                }),
                option { value: "yes", selected: kind() == Kind::Yes, "{Kind::Yes.label()}" }
                option { value: "no", selected: kind() == Kind::No, "{Kind::No.label()}" }
            }
            div { class: "sheet-row",
                input {
                    class: "client-input",
                    r#type: "number",
                    inputmode: "numeric",
                    min: "1",
                    aria_label: "Times",
                    value: "{times}",
                    oninput: move |event| times.set(event.value()),
                }
                span { class: "hint", "times in" }
                input {
                    class: "client-input",
                    r#type: "number",
                    inputmode: "numeric",
                    min: "1",
                    aria_label: "Days",
                    value: "{days}",
                    oninput: move |event| days.set(event.value()),
                }
                span { class: "hint", "days" }
            }
            div { class: "sheet-row",
                input {
                    class: "client-input",
                    r#type: "number",
                    inputmode: "decimal",
                    min: "0",
                    step: "any",
                    placeholder: "Target per day (optional)",
                    aria_label: "Target",
                    value: "{target}",
                    oninput: move |event| target.set(event.value()),
                }
                input {
                    class: "client-input",
                    r#type: "text",
                    placeholder: "Unit (optional)",
                    aria_label: "Unit",
                    value: "{unit}",
                    oninput: move |event| unit.set(event.value()),
                }
            }
            div { class: "sheet-row",
                button { class: "btn primary", r#type: "submit", "Add habit" }
                button { class: "btn", r#type: "button", onclick: move |_| on_done.call(()), "Cancel" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_cannot_be_read_is_never_saved_over() {
        let directory = tempfile_dir();
        let path = directory.join("habits.json");
        std::fs::write(&path, "not json").unwrap();
        let mut saved = read(Some(path.clone()));
        assert!(saved.error.is_some());
        saved.library.habits.push(Habit::new("a", "Run"));
        saved.save();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn habits_survive_being_saved_and_read_back() {
        let directory = tempfile_dir();
        let path = directory.join("nested").join("habits.json");
        let mut saved = read(Some(path.clone()));
        assert_eq!(saved.error, None);
        saved.library.habits.push(Habit::new("a", "Run"));
        saved.save();
        assert_eq!(saved.error, None);
        assert_eq!(read(Some(path)).library.habits.len(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_new_habit_needs_a_name_and_a_real_goal() {
        let make = |name: &str, times: &str, days: &str| {
            parse_new(name, Kind::Yes, times, days, "", "", "id".to_owned())
        };
        assert!(make(" ", "1", "1").is_err());
        assert!(make("Run", "0", "1").is_err());
        assert!(make("Run", "x", "1").is_err());
        let habit = make(" Run ", "3", "7").unwrap();
        assert_eq!(habit.name, "Run");
        assert_eq!(habit.frequency, Frequency::Interval { times: 3, days: 7 });
    }

    #[test]
    fn a_new_habit_tracks_a_value_once_a_target_or_unit_is_given() {
        let make = |target: &str, unit: &str| {
            parse_new("Run", Kind::No, "1", "1", target, unit, "id".to_owned())
        };
        assert_eq!(make("", " ").unwrap().measure, None);
        let measure = make("2.5", " miles ").unwrap().measure.unwrap();
        assert_eq!(
            (measure.target, measure.unit.as_str()),
            (Some(2.5), "miles")
        );
        assert_eq!(make("", "miles").unwrap().measure.unwrap().target, None);
        assert!(make("lots", "").is_err());
    }

    #[test]
    fn an_amount_is_a_finite_number_of_zero_or_more() {
        assert_eq!(parse_amount(" 2.5 "), Ok(2.5));
        for bad in ["-1", "inf", "NaN", "lots", ""] {
            assert!(parse_amount(bad).is_err(), "{bad}");
        }
    }

    fn tempfile_dir() -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "timeslay-habits-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }
}
