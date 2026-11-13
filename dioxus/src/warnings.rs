//! Calendar failures and notices stay out of the day grid until opened.
use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
struct Issue {
	title: &'static str,
	message: String,
}

#[derive(Default)]
struct State {
	open: bool,
	dismissed: Vec<Issue>,
}

impl State {
	fn visible(&self, issues: &[Issue]) -> Vec<Issue> {
		issues
			.iter()
			.filter(|issue| !self.dismissed.contains(issue))
			.cloned()
			.collect()
	}

	fn dismiss(&mut self, issue: Issue) {
		self.dismissed.push(issue);
	}

	fn reconcile(&mut self, issues: &[Issue]) {
		self.dismissed.retain(|issue| issues.contains(issue));
		if self.visible(issues).is_empty() {
			self.open = false;
		}
	}
}

#[component]
pub fn WarningCenter(
	sync_error: Option<String>,
	connect_error: Option<String>,
	notice: Option<String>,
	pending: bool,
	connecting: bool,
	on_retry: EventHandler,
	on_connect: EventHandler,
) -> Element {
	let mut state = use_signal(State::default);
	let issues: Vec<_> = [
		("Sync failed", sync_error),
		("Google connection failed", connect_error),
		("Calendar warning", notice),
	]
	.into_iter()
	.filter_map(|(title, message)| message.map(|message| Issue { title, message }))
	.collect();
	use_effect(use_reactive((&issues,), move |(issues,)| {
		state.write().reconcile(&issues);
	}));
	let visible = state.read().visible(&issues);
	let count = visible.len();
	let open = state.read().open && count > 0;
	let has_error = visible
		.iter()
		.any(|issue| issue.title != "Calendar warning");

	rsx! {
		if count > 0 {
			button {
				class: "warning-button",
				r#type: "button",
				title: "Warnings and errors",
				aria_label: "Warnings and errors: {count}",
				aria_expanded: open.to_string(),
				aria_controls: "calendar-warnings",
				onclick: move |_| {
					let open = state.peek().open;
					state.write().open = !open;
				},
				span { aria_hidden: "true", "⚠" }
				if count > 1 { span { class: "warning-count", "{count}" } }
			}
		}
		if open {
			div { class: "warning-backdrop", onclick: move |_| state.write().open = false }
			section {
				id: "calendar-warnings",
				class: "warning-panel",
				role: "dialog",
				aria_label: "Warnings and errors",
				tabindex: "-1",
				onmounted: move |event| async move { let _ = event.data().set_focus(true).await; },
				onkeydown: move |event| {
					if event.key() == Key::Escape { state.write().open = false; }
				},
				div { class: "warning-heading",
					h2 { "Warnings and errors" }
					button { class: "btn small", onclick: move |_| state.write().open = false, "Close" }
				}
				div { class: "warning-list",
					for issue in visible {
						article { key: "{issue.title}", class: "warning-item",
							h3 { "{issue.title}" }
							p { "{issue.message}" }
							button {
								class: "btn small",
								aria_label: "Dismiss {issue.title}",
								onclick: move |_| {
									let mut state = state.write();
									state.dismiss(issue.clone());
									if count == 1 { state.open = false; }
								},
								"Dismiss"
							}
						}
					}
				}
				if has_error {
					div { class: "warning-actions",
						button { class: "btn small", disabled: pending, onclick: move |_| on_retry.call(()),
							if pending { "Retrying…" } else { "Retry" }
						}
						button { class: "btn small", disabled: connecting, onclick: move |_| on_connect.call(()),
							if connecting { "Connecting…" } else { "Connect Google" }
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

	fn issue(message: &str) -> Issue {
		Issue {
			title: "Sync failed",
			message: message.into(),
		}
	}

	#[test]
	fn dismissing_one_issue_keeps_other_and_new_issues_available() {
		let mut state = State::default();
		state.dismiss(issue("Old failure"));
		let visible = state.visible(&[issue("Old failure"), issue("New failure")]);
		assert_eq!(visible.len(), 1);
		assert_eq!(visible[0].message, "New failure");
		assert!(state.visible(&[issue("Old failure")]).is_empty());
	}

	#[test]
	fn resolved_issues_close_and_can_be_reported_again_later() {
		let mut state = State {
			open: true,
			..State::default()
		};
		state.dismiss(issue("Failure"));
		state.reconcile(&[]);
		assert!(!state.open);
		assert_eq!(state.visible(&[issue("Failure")]).len(), 1);
	}

	#[test]
	fn closing_details_keeps_the_warning_available() {
		let mut state = State {
			open: true,
			..State::default()
		};
		state.open = false;
		assert_eq!(state.visible(&[issue("Failure")]).len(), 1);
	}

	#[test]
	fn warnings_start_collapsed_with_an_accessible_button() {
		fn app() -> Element {
			rsx! { WarningCenter {
				sync_error: Some("Private error details".into()),
				connect_error: None, notice: None,
				pending: false, connecting: false,
				on_retry: move |_| {}, on_connect: move |_| {},
			} }
		}
		let mut dom = VirtualDom::new(app);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("Warnings and errors: 1"));
		assert!(html.contains("aria-expanded=\"false\""));
		assert!(!html.contains("Private error details"));
		assert!(!html.contains("role=\"dialog\""));
	}
	#[test]
	fn warning_button_opens_closes_and_dismisses_details() {
		use dioxus::core::{AttributeValue, ElementId, Mutation, Mutations};
		use std::{any::Any, rc::Rc};
		set_event_converter(Box::new(dioxus::html::SerializedHtmlEventConverter));

		fn app() -> Element {
			rsx! { WarningCenter {
				sync_error: Some("Empty calendar failed".into()),
				connect_error: None, notice: None,
				pending: false, connecting: false,
				on_retry: move |_| {}, on_connect: move |_| {},
			} }
		}
		fn click(dom: &mut VirtualDom, id: ElementId) -> Mutations {
			let event = Event::new(
				Rc::new(PlatformEventData::new(Box::<SerializedMouseData>::default()))
					as Rc<dyn Any>,
				true,
			);
			dom.runtime().handle_event("click", event, id);
			dom.render_immediate_to_vec()
		}
		fn clicks(mutations: Mutations) -> Vec<ElementId> {
			mutations
				.edits
				.into_iter()
				.filter_map(|edit| match edit {
					Mutation::NewEventListener { name, id } if name == "click" => Some(id),
					_ => None,
				})
				.collect()
		}
		let mut dom = VirtualDom::new(app);
		let button = clicks(dom.rebuild_to_vec())[0];
		let controls = clicks(click(&mut dom, button));
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("Empty calendar failed"));
		assert!(html.contains("role=\"dialog\""));
		assert!(html.contains("Retry"));
		assert!(html.contains("Connect Google"));
		// The toggle also closes the details without dismissing.
		assert_eq!(controls.len(), 5);
		click(&mut dom, button);
		let html = dioxus_ssr::render(&dom);
		assert!(!html.contains("Empty calendar failed"));
		assert!(html.contains("warning-button"));
		let opened = click(&mut dom, button);
		let dismiss = opened
			.edits
			.iter()
			.find_map(|edit| match edit {
				Mutation::SetAttribute {
					name: "aria-label",
					value: AttributeValue::Text(label),
					id,
					..
				} if label == "Dismiss Sync failed" => Some(*id),
				_ => None,
			})
			.expect("Dismiss button is labelled");
		click(&mut dom, dismiss);
		let html = dioxus_ssr::render(&dom);
		assert!(!html.contains("Empty calendar failed"));
		assert!(!html.contains("warning-button"));
	}
}
