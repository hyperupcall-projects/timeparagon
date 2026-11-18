//! The Filter tab of the View sheet, in the same order as the desktop's.
//! Filtering rules live in `timeparagon_core::calendar::search`.

use dioxus::prelude::*;
use timeparagon_core::calendar::projects;
use timeparagon_core::calendar::search::{Action, AvailabilityFilter, Search};
use timeparagon_core::config::ProjectsSettings;

#[component]
fn Toggle(label: &'static str, checked: bool, on_change: EventHandler<bool>) -> Element {
	rsx! {
		button {
			class: "source-row",
			role: "switch",
			aria_checked: checked.to_string(),
			onclick: move |_| on_change.call(!checked),
			span { class: "source-name", "{label}" }
			span { class: "source-switch", span { class: "source-knob" } }
		}
	}
}

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
				placeholder: "Search",
				aria_label: "Search",
				value: "{current.query}",
				oninput: move |event| apply(Action::Query(event.value())),
			}
			Toggle {
				label: "Search description",
				checked: current.in_description,
				on_change: move |value| apply(Action::InDescription(value)),
			}
			h3 { class: "search-heading", "Search filters" }
			div { class: "editor-field",
				label { class: "editor-label", r#for: "search-time", "Time of day" }
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
				if current.time.is_some() {
					button { class: "btn editor-wide", onclick: move |_| apply(Action::ClearTime), "Any time" }
				}
			}
			crate::editor::ProjectsField {
				selected: current.projects.ids.clone(),
				projects,
				busy: false,
				on_add: move |id| apply(Action::Projects(projects::Action::Add(id))),
				on_remove: move |id| apply(Action::Projects(projects::Action::Remove(id))),
			}
			div { class: "editor-field",
				span { class: "editor-label", "Labels" }
				if labels.is_empty() {
					p { class: "empty-note", "No labels on the events loaded so far" }
				}
				for label in labels {
					{
						let checked = current.labels.contains(&label);
						let name = label.clone();
						rsx! {
							label { key: "{label}", class: "sheet-check",
								input {
									r#type: "checkbox",
									checked,
									onchange: move |event: FormEvent| apply(Action::Label(name.clone(), event.checked())),
								}
								"{label}"
							}
						}
					}
				}
			}
			h3 { class: "search-heading", "Event filters" }
			div { class: "editor-field",
				label { class: "editor-label", r#for: "search-availability", "Show events that are" }
				select {
					id: "search-availability",
					class: "client-input",
					onchange: move |event: FormEvent| {
						if let Some(picked) = AvailabilityFilter::ALL.into_iter().find(|choice| choice.to_string() == event.value()) {
							apply(Action::Availability(picked));
						}
					},
					for choice in AvailabilityFilter::ALL {
						option { key: "{choice}", value: "{choice}", selected: choice == current.availability, "{choice}" }
					}
				}
			}
			Toggle {
				label: "Only show events with public visibility",
				checked: current.public_only,
				on_change: move |value| apply(Action::PublicOnly(value)),
			}
			div { class: "editor-field",
				label { class: "editor-label", r#for: "search-location", "Location" }
				select {
					id: "search-location",
					class: "client-input",
					onchange: move |event: FormEvent| {
						let value = event.value();
						apply(Action::Location((!value.is_empty()).then_some(value)));
					},
					option { value: "", selected: chosen_location.is_empty(), "Any location" }
					for location in locations {
						option { key: "{location}", value: "{location}", selected: location == chosen_location, "{location}" }
					}
				}
			}
			h3 { class: "search-heading", "Detail filter" }
			Toggle {
				label: "Show details by visibility",
				checked: current.details_by_visibility,
				on_change: move |value| apply(Action::DetailsByVisibility(value)),
			}
			button {
				class: "btn editor-wide",
				disabled: !current.is_modified(),
				onclick: move |_| apply(Action::Clear),
				"Clear search"
			}
		}
	}
}
