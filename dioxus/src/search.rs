//! The Filter tab of the View sheet: narrows the calendar down to what matches. The rules, and what
//! each filter means, are `timeslay_core::calendar::search`.

use dioxus::prelude::*;
use timeslay_core::calendar::projects;
use timeslay_core::calendar::search::{Action, AvailabilityFilter, Search};
use timeslay_core::config::ProjectsSettings;

#[component]
pub fn SearchFields(
    search: Signal<Search>,
    labels: Vec<String>,
    locations: Vec<String>,
    projects: ProjectsSettings,
) -> Element {
    let current = search.read().clone();
    let mut apply = move |action: Action| search.write().update(action);
    let time = current
        .time
        .map(|time| time.format("%H:%M").to_string())
        .unwrap_or_default();
    let chosen_location = current.location.clone().unwrap_or_default();
    rsx! {
                div { class: "search-fields",
                    input {
                        class: "client-input",
                        r#type: "search",
                        placeholder: "Search titles",
                        aria_label: "Search titles",
                        value: "{current.query}",
                        oninput: move |event| apply(Action::Query(event.value())),
                    }
                    label { class: "sheet-check",
                        input {
                            r#type: "checkbox",
                            checked: current.in_description,
                            onchange: move |event: FormEvent| apply(Action::InDescription(event.checked())),
                        }
                        "Look in descriptions too"
                    }
                    label { class: "sheet-check",
                        input {
                            r#type: "checkbox",
                            checked: current.public_only,
                            onchange: move |event: FormEvent| apply(Action::PublicOnly(event.checked())),
                        }
                        "Only public events"
                    }
                    label { class: "editor-label", r#for: "search-time", "Under way at" }
                    input {
                        id: "search-time",
                        class: "client-input",
                        r#type: "time",
                        value: "{time}",
                        onchange: move |event: FormEvent| {
                            let value = event.value();
                            apply(if value.is_empty() { Action::ClearTime } else { Action::Time(value) });
                        },
                    }
                    label { class: "editor-label", r#for: "search-availability", "Availability" }
                    select {
                        id: "search-availability",
                        class: "client-input",
                        onchange: move |event: FormEvent| {
                            let picked = AvailabilityFilter::ALL
                                .into_iter()
                                .find(|choice| choice.to_string() == event.value());
                            if let Some(picked) = picked {
                                apply(Action::Availability(picked));
                            }
                        },
                        for choice in AvailabilityFilter::ALL {
                            option {
                                key: "{choice}",
                                value: "{choice}",
                                selected: choice == current.availability,
                                "{choice}"
                            }
                        }
                    }
                    if !locations.is_empty() {
                        label { class: "editor-label", r#for: "search-location", "Location" }
                        select {
                            id: "search-location",
                            class: "client-input",
                            onchange: move |event: FormEvent| {
                                let value = event.value();
                                apply(Action::Location((!value.is_empty()).then_some(value)));
                            },
                            option { value: "", selected: chosen_location.is_empty(), "Anywhere" }
                            for location in locations {
                                option {
                                    key: "{location}",
                                    value: "{location}",
                                    selected: location == chosen_location,
                                    "{location}"
                                }
                            }
                        }
                    }
                    if !labels.is_empty() {
                        span { class: "editor-label", "Labels" }
                        div { class: "token-row",
                            for label in labels {
                                {
                                    let on = current.labels.contains(&label);
                                    let name = label.clone();
                                    rsx! {
                                        button {
                                            key: "{label}",
                                            class: "nav-tab",
                                            aria_pressed: on.to_string(),
                                            onclick: move |_| apply(Action::Label(name.clone(), !on)),
                                            "{label}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !projects.items.is_empty() {
                        span { class: "editor-label", "Projects" }
                        div { class: "token-row",
                            for project in projects.sorted() {
                                {
                                    let id = project.id;
                                    let on = current.projects.ids.contains(&id);
                                    rsx! {
                                        button {
                                            key: "{id}",
                                            class: "nav-tab",
                                            aria_pressed: on.to_string(),
                                            onclick: move |_| {
                                                let action = if on { projects::Action::Remove(id) } else { projects::Action::Add(id) };
                                                apply(Action::Projects(action));
                                            },
                                            "{project.name}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                    button { class: "btn", onclick: move |_| apply(Action::Clear), "Clear" }
                }
    }
}
