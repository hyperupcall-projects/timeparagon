//! Google sign-in and the empty-calendar setup screen.
use super::*;
#[derive(Clone, Copy)]
pub(super) struct Auth {
	pub connecting: Signal<bool>,
	pub connect_error: Signal<Option<String>>,
	pub signed_in: Signal<bool>,
	pub connect_google: Callback<CalendarSettings>,
}

pub(super) fn use_auth() -> Auth {
	let Session { revision, .. } = use_context::<Session>();
	let connecting = use_signal(|| false);
	let connect_error = use_signal(|| None::<String>);
	let signed_in = use_signal(day::signed_in);
	// The callback belongs to the session, so closing the requesting sheet
	// does not cancel sign-in or lose the refresh after it succeeds.
	let connect_google = use_callback(move |settings| {
		begin_connect(settings, connecting, connect_error, signed_in, revision);
	});
	Auth {
		connect_google,
		connecting,
		connect_error,
		signed_in,
	}
}

fn begin_connect(
	settings: CalendarSettings,
	connecting: Signal<bool>,
	connect_error: Signal<Option<String>>,
	signed_in: Signal<bool>,
	revision: Signal<u32>,
) {
	let mut connecting = connecting;
	if connecting() {
		return;
	}
	connecting.set(true);
	let mut connect_error = connect_error;
	connect_error.set(None);

	spawn(async move {
		let mut connecting = connecting;
		let mut connect_error = connect_error;
		let mut signed_in = signed_in;
		let mut revision = revision;
		let result = tokio::task::spawn_blocking(move || day::connect_google(&settings))
			.await
			.map_err(|error| format!("The sign-in worker stopped: {error}"))
			.and_then(|result| result);

		connecting.set(false);
		match result {
			Ok(()) => {
				signed_in.set(true);
				revision.set(revision() + 1);
			}
			Err(error) => connect_error.set(Some(error)),
		}
	});
}

#[component]
pub(super) fn CalendarUnavailable(pending: bool, fetch_error: Option<String>) -> Element {
	let Session {
		mut config,
		mut revision,
		..
	} = use_context::<Session>();
	let Auth {
		connect_google,
		connecting,
		connect_error,
		..
	} = use_context::<Auth>();
	let mut client_id_input = use_signal(String::new);
	let mut client_secret_input = use_signal(String::new);
	let mut client_error = use_signal(|| None::<String>);
	let settings = config.read().calendar.clone();
	let config_hint = timeparagon_core::task_db::path()
		.map(|path| path.display().to_string())
		.unwrap_or_else(|| "timeparagon.sqlite3".to_owned());
	match settings.client_id.trim().is_empty() && !pending {
		true => {
			let client_error_text = client_error().map(|message| {
				rsx! { p { class: "error-text", "{message}" } }
			});
			rsx! {
				div { class: "state-screen",
					div { class: "state-card",
						div { class: "state-mark", "⏳" }
						h2 { "Set up Google" }
						p {
							"Enter the OAuth client ID and secret from your Google Cloud "
							"project. They stay in this app's private storage."
						}
						{client_error_text}
						form {
							class: "client-form",
							onsubmit: move |event| {
								event.prevent_default();
								match day::save_google_client(&client_id_input(), &client_secret_input()) {
									Ok(saved) => {
										client_error.set(None);
										client_secret_input.set(String::new());
										let saved_settings = saved.calendar.clone();
										config.set(saved);
										connect_google.call(saved_settings);
									}
									Err(error) => client_error.set(Some(error)),
								}
							},
							input {
								class: "client-input",
								r#type: "text",
								name: "client-id",
								placeholder: "Client ID",
								autocomplete: "off",
								autocapitalize: "off",
								spellcheck: "false",
								value: "{client_id_input}",
								oninput: move |event| client_id_input.set(event.value()),
							}
							input {
								class: "client-input",
								r#type: "password",
								name: "client-secret",
								placeholder: "Client secret",
								autocomplete: "off",
								autocapitalize: "off",
								spellcheck: "false",
								value: "{client_secret_input}",
								oninput: move |event| client_secret_input.set(event.value()),
							}
							div { class: "state-actions",
								button { class: "btn primary", r#type: "submit",
									if connecting() { "Waiting for Google…" } else { "Save and connect" }
								}
							}
						}
					}
				}
			}
		}
		false => {
			let state_connect_settings = settings.clone();
			let state_connect_error = connect_error().map(|connect_error| {
				rsx! { p { class: "error-text", "{connect_error}" } }
			});
			rsx! {
				div { class: "state-screen",
					div { class: "state-card",
						div { class: "state-mark", "⏳" }
						if pending {
							h2 { "Loading your day…" }
							p { "Fetching calendars, events and Google Tasks." }
							div { class: "loading-dots",
								span { class: "dot" }
								span { class: "dot" }
								span { class: "dot" }
							}
						} else {
							h2 { "Your day could not be loaded" }
							p { "{fetch_error.clone().unwrap_or_else(|| \"No calendar data is available.\".to_owned())}" }
							{state_connect_error}
							p { class: "hint",
								"The day view uses the same OAuth token and settings as the iced app. "
								"The settings live in {config_hint}."
							}
							div { class: "state-actions",
								button {
									class: "btn primary",
									onclick: move |_| {
										connect_google.call(state_connect_settings.clone());
									},
									if connecting() { "Waiting for Google…" } else { "Connect Google" }
								}
								button {
									class: "btn",
									onclick: move |_| {
										let next = revision() + 1;
										revision.set(next);
									},
									"Try again"
								}
							}
						}
					}
				}
			}
		}
	}
}

#[component]
pub(super) fn AuthProvider(children: Element) -> Element {
	let auth = use_auth();
	use_context_provider(|| auth);
	children
}
