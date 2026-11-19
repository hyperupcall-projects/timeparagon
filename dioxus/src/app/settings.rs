//! Settings drafts and theme preference; field edits subscribe only this form.
use super::*;

#[derive(Clone, Copy)]
struct ThemePreferenceState(Signal<Theme>);

#[component]
pub(super) fn ThemePreference() -> Element {
	let theme = use_context::<ThemePreferenceState>().0;
	// Mark the document with the chosen theme. `system` leaves the decision to
	// the CSS `prefers-color-scheme` query, so it follows the OS live.
	use_effect(move || {
		let name = theme().name();
		let _ = document::eval(&format!(
			"document.documentElement.dataset.theme = '{name}';"
		));
		// Also publish on startup, before a calendar day may be available.
		#[cfg(target_os = "android")]
		spawn(async move {
			let _ = editor::worker(|| {
				widget::publish_theme();
				Ok(())
			})
			.await;
		});
	});

	rsx! {}
}

pub(super) fn provide_theme() {
	let theme = use_signal(Theme::load);
	use_context_provider(|| ThemePreferenceState(theme));
}

#[derive(Clone, Copy)]
struct FullSync {
	running: Signal<bool>,
	error: Signal<Option<String>>,
	start: Callback<()>,
}

pub(super) fn provide_full_sync() {
	let Session { mut revision, .. } = use_context::<Session>();
	let mut running = use_signal(|| false);
	let mut error = use_signal(|| None::<String>);
	let start = use_callback(move |()| {
		if *running.peek() {
			return;
		}
		running.set(true);
		error.set(None);
		spawn(async move {
			let reset = tokio::task::spawn_blocking(calendar::forget_sync_state)
				.await
				.map_err(|error| error.to_string())
				.and_then(|result| result);
			running.set(false);
			match reset {
				Ok(()) => revision.set(revision() + 1),
				Err(message) => error.set(Some(format!("Could not start a full sync: {message}"))),
			}
		});
	});
	use_context_provider(|| FullSync {
		running,
		error,
		start,
	});
}

#[component]
pub(super) fn SettingsSheet() -> Element {
	let Panels { show_settings, .. } = use_context::<Panels>();
	rsx! { if show_settings() { SettingsForm {} } }
}

#[component]
fn SettingsForm() -> Element {
	let Session {
		mut config,
		mut revision,
		..
	} = use_context::<Session>();
	let SyncState {
		database_sync_status,
		..
	} = use_context::<SyncState>();
	let Panels {
		mut show_settings, ..
	} = use_context::<Panels>();
	let setup::Finished(mut setup_finished) = use_context::<setup::Finished>();
	let mut settings_id = use_signal(|| config.peek().calendar.client_id.clone());
	let mut settings_secret = use_signal(String::new);
	let mut settings_error = use_signal(|| None::<String>);
	let mut sync_url_input = use_signal(|| config.peek().database_sync.url.clone());
	let mut sync_url_error = use_signal(|| None::<String>);
	let mut sync_url_saved = use_signal(|| false);
	let FullSync {
		running: full_sync_running,
		error: mut full_sync_error,
		start: start_full_sync,
	} = use_context::<FullSync>();
	use_hook(move || full_sync_error.set(None));
	let ThemePreferenceState(mut theme) = use_context::<ThemePreferenceState>();
	let choose_theme = move |event: FormEvent| {
		let chosen = Theme::parse(&event.value());
		theme.set(chosen);
		chosen.save();
	};
	let first_day = config.read().calendar.first_day_of_week;
	let error = settings_error().or(full_sync_error()).map(|message| {
		rsx! { p { class: "error-text", "{message}" } }
	});
	rsx! {
		div {
			class: "modal-backdrop",
			onclick: move |_| show_settings.set(false),
			div {
				class: "modal",
				role: "dialog",
				aria_label: "Settings",
				onclick: move |event| event.stop_propagation(),
				div { class: "modal-head",
					h2 { "Settings" }
					div { class: "spacer" }
					button {
						class: "modal-close",
						title: "Close",
						onclick: move |_| show_settings.set(false),
						"×"
					}
				}
				div { class: "modal-body settings-body",
					label { r#for: "settings-theme", "Theme" }
					select {
						id: "settings-theme",
						class: "client-input",
						onchange: choose_theme,
						for choice in Theme::ALL {
							option {
								key: "{choice.name()}",
								value: "{choice.name()}",
								selected: choice == theme(),
								"{choice.label()}"
							}
						}
					}
					label { r#for: "settings-first-day", "First day of the week" }
					select {
						id: "settings-first-day",
						class: "client-input",
						onchange: move |event: FormEvent| {
							let Some(chosen) = FirstDayOfWeek::parse(&event.value()) else {
								return;
							};
							match day::set_first_day_of_week(chosen) {
								Ok(saved) => {
									config.set(saved);
									settings_error.set(None);
								}
								Err(error) => settings_error.set(Some(error)),
							}
						},
						for choice in FirstDayOfWeek::ALL {
							option {
								key: "{choice.name()}",
								value: "{choice.name()}",
								selected: choice == first_day,
								"{choice}"
							}
						}
					}
					h3 { class: "settings-heading", "Database sync" }
					label { r#for: "settings-sync-url", "Sync server URL (blank disables sync)" }
					form {
						class: "client-form settings-form",
						onsubmit: move |event| {
							event.prevent_default();
							sync_url_saved.set(false);
							match timeparagon_core::config::save_database_sync_url(&sync_url_input()) {
								Ok(()) => {
									let saved = day::load_config();
									sync_url_input.set(saved.database_sync.url.clone());
									config.set(saved);
									sync_url_error.set(None);
									sync_url_saved.set(true);
								}
								Err(error) => sync_url_error.set(Some(error.to_string())),
							}
						},
						input {
							id: "settings-sync-url",
							class: "client-input",
							r#type: "url",
							inputmode: "url",
							autocomplete: "off",
							autocapitalize: "off",
							spellcheck: "false",
							placeholder: "https://sync.example.com",
							value: "{sync_url_input}",
							oninput: move |event| {
								sync_url_input.set(event.value());
								sync_url_error.set(None);
								sync_url_saved.set(false);
							},
						}
						if let Some(message) = sync_url_error() {
							p { class: "error-text", "{message}" }
						} else if sync_url_saved() {
							p { class: "hint", "Saved." }
						}
						div { class: "state-actions",
							button {
								class: "btn primary",
								r#type: "submit",
								disabled: sync_url_input().trim() == config().database_sync.url.trim(),
								"Save sync URL"
							}
						}
					}
					p { class: "hint", "{database_sync_status}" }
					ai_input::ModelSettings { config }
					notes::NotesSettings { config }
					reminders::ReminderSection { config }
					sleep::SleepSettings { config }
					h3 { class: "settings-heading", "Application" }
					div { class: "state-actions",
						button {
							class: "btn",
							r#type: "button",
							disabled: full_sync_running(),
							onclick: move |_| start_full_sync.call(()),
							if full_sync_running() { "Starting full sync…" } else { "Full sync" }
						}
						button {
							class: "btn",
							r#type: "button",
							onclick: move |_| {
								setup::Progress::restart();
								show_settings.set(false);
								setup_finished.set(false);
							},
							"Run setup"
						}
					}
					h3 { class: "settings-heading", "Google" }
					form {
						class: "client-form settings-form",
						onsubmit: move |event| {
							event.prevent_default();
							// An empty secret field keeps the stored secret.
							let secret = if settings_secret().trim().is_empty() {
								config.peek().calendar.client_secret.clone()
							} else {
								settings_secret()
							};
							match day::save_google_client(&settings_id(), &secret) {
								Ok(saved) => {
									config.set(saved);
									settings_error.set(None);
									show_settings.set(false);
									revision.set(revision() + 1);
								}
								Err(error) => settings_error.set(Some(error)),
							}
						},
						{error}
						label { r#for: "settings-client-id", "Client ID" }
						input {
							id: "settings-client-id",
							class: "client-input",
							r#type: "text",
							autocomplete: "off",
							autocapitalize: "off",
							spellcheck: "false",
							value: "{settings_id}",
							oninput: move |event| settings_id.set(event.value()),
						}
						label { r#for: "settings-client-secret", "Client secret" }
						input {
							id: "settings-client-secret",
							class: "client-input",
							r#type: "password",
							placeholder: "Leave blank to keep the current secret",
							autocomplete: "off",
							autocapitalize: "off",
							spellcheck: "false",
							value: "{settings_secret}",
							oninput: move |event| settings_secret.set(event.value()),
						}
						p { class: "hint",
							"After changing the client, use Connect Google to sign in again."
						}
						div { class: "state-actions",
							button { class: "btn primary", r#type: "submit", "Save" }
							button {
								class: "btn",
								r#type: "button",
								onclick: move |_| show_settings.set(false),
								"Cancel"
							}
						}
					}
				}
			}
		}
	}
}

#[component]
pub(super) fn SettingsProvider(children: Element) -> Element {
	provide_theme();
	provide_full_sync();
	children
}
