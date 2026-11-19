//! Settings drafts and theme preference; field edits subscribe only this form.
use super::*;
use timeparagon_core::calendar::time_format::{self, TimeFormat};
use timeparagon_core::config::{Config, WeekRange};
use timeparagon_core::places::DistanceUnit;

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
pub(super) fn SettingsPage() -> Element {
	let Session {
		mut config,
		mut revision,
		..
	} = use_context::<Session>();
	let SyncState {
		database_sync_status,
		..
	} = use_context::<SyncState>();
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
	let error = settings_error().map(|message| {
		rsx! {
			p { class: "error-text", "{message}" }
		}
	});
	rsx! {
		main { class: "settings-page", aria_label: "Settings",
			h2 { class: "settings-page-title", "Settings" }
			div { class: "settings-body",
				Preferences { config }
				section { class: "settings-section", aria_label: "Reminders",
					reminders::ReminderSection { config }
				}
				section {
					class: "settings-section",
					aria_label: "Database sync",
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
				}
				section { class: "settings-section", aria_label: "Notes",
					notes::NotesSettings { config }
				}
				section {
					class: "settings-section",
					aria_label: "Google Calendar",
					h3 { class: "settings-heading", "Google Calendar" }
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
						}
					}
				}
				section { class: "settings-section", aria_label: "Sleep",
					sleep::SleepSettings { config }
				}
				section { class: "settings-section", aria_label: "AI",
					ai_input::ModelSettings { config }
				}
				section { class: "settings-section", aria_label: "Application",
					h3 { class: "settings-heading", "Application" }
					div { class: "state-actions",
						button {
							class: "btn",
							r#type: "button",
							disabled: full_sync_running(),
							onclick: move |_| start_full_sync.call(()),
							if full_sync_running() {
								"Starting full sync…"
							} else {
								"Full sync"
							}
						}
						button {
							class: "btn",
							r#type: "button",
							onclick: move |_| {
								setup::Progress::restart();
								setup_finished.set(false);
							},
							"Run setup"
						}
					}
					if let Some(message) = full_sync_error() {
						p { class: "error-text", role: "alert", "{message}" }
					}
				}
			}
		}
	}
}

/// Each change edits the current database value, preserving other settings,
/// including desktop fields that the mobile Config does not contain.
#[derive(Clone, Copy)]
enum PreferenceChange {
	TimeFormat(TimeFormat),
	FirstDay(FirstDayOfWeek),
	WeekRange(WeekRange),
	DayEnd(u32),
	DistanceUnit(DistanceUnit),
}

impl PreferenceChange {
	fn save(self, path: &std::path::Path) -> Result<Config, String> {
		timeparagon_core::settings::update(path, |config: &mut Config| {
			match self {
				Self::TimeFormat(value) => config.appearance.time_format = value,
				Self::FirstDay(value) => {
					config.calendar.first_day_of_week = value;
				}
				Self::WeekRange(value) => config.calendar.set_week_range(value),
				Self::DayEnd(value) => {
					config.calendar.day_end_hour = value.min(CalendarSettings::MAX_DAY_END_HOUR);
				}
				Self::DistanceUnit(value) => {
					config.appearance.distance_unit = value;
				}
			}
			config.clone()
		})
		.map_err(|error| format!("Could not save preferences: {error}"))
	}
}

#[component]
fn Preferences(mut config: Signal<Config>) -> Element {
	let mut error = use_signal(|| None::<String>);
	let ThemePreferenceState(mut theme) = use_context::<ThemePreferenceState>();
	let save = use_callback(move |change: PreferenceChange| {
		let result = timeparagon_core::task_db::path()
			.ok_or_else(|| "Cannot locate the settings database".to_owned())
			.and_then(|path| change.save(&path));
		match result {
			Ok(saved) => {
				config.set(saved);
				error.set(None);
			}
			Err(message) => error.set(Some(message)),
		}
	});
	let settings = config.read();
	let time_format = settings.appearance.time_format;
	let day_end = settings.calendar.day_start_hour();
	let day_end_label = if day_end == 0 {
		"Day ends at midnight".to_owned()
	} else {
		format!("Day ends at {}", time_format::hour(day_end, time_format))
	};
	rsx! {
		section { class: "settings-section", aria_label: "Preferences",
			h3 { class: "settings-heading", "Preferences" }
			{
				preference_choice(
					"settings-time-format",
					"Time format",
					TimeFormat::ALL,
					time_format,
					move |value| save.call(PreferenceChange::TimeFormat(value)),
				)
			}
			{
				preference_choice(
					"settings-first-day",
					"First day of the week",
					FirstDayOfWeek::ALL,
					settings.calendar.first_day_of_week,
					move |value| save.call(PreferenceChange::FirstDay(value)),
				)
			}
			{
				preference_choice(
					"settings-week-range",
					"Days shown in week view",
					WeekRange::ALL,
					settings.calendar.week_range(),
					move |value| save.call(PreferenceChange::WeekRange(value)),
				)
			}
			div { class: "settings-field",
				label { r#for: "settings-day-end", "{day_end_label}" }
				select {
					id: "settings-day-end",
					class: "client-input",
					value: "{day_end}",
					onchange: move |event: FormEvent| {
						if let Ok(hour) = event.value().parse::<u32>() {
							save.call(PreferenceChange::DayEnd(hour));
						}
					},
					for hour in 0..=CalendarSettings::MAX_DAY_END_HOUR {
						option {
							key: "{hour}",
							value: "{hour}",
							selected: hour == day_end,
							if hour == 0 {
								"Midnight"
							} else {
								"{time_format::hour(hour, time_format)}"
							}
						}
					}
				}
			}
			{
				preference_choice(
					"settings-distance-unit",
					"Distance unit",
					DistanceUnit::ALL,
					settings.appearance.distance_unit,
					move |value| save.call(PreferenceChange::DistanceUnit(value)),
				)
			}
			div { class: "settings-field",
				label { r#for: "settings-theme", "Theme" }
				select {
					id: "settings-theme",
					class: "client-input",
					value: theme().name(),
					onchange: move |event: FormEvent| {
						let chosen = Theme::parse(&event.value());
						theme.set(chosen);
						chosen.save();
					},
					for choice in Theme::ALL {
						option {
							key: "{choice.name()}",
								value: choice.name(),
							selected: choice == theme(),
							"{choice.label()}"
						}
					}
				}
			}
			if let Some(message) = error() {
				p { class: "error-text", role: "alert", "{message}" }
			}
		}
	}
}

/// Use the shared types' labels and ordering, exactly as the desktop picker does.
fn preference_choice<T: Copy + std::fmt::Display + 'static>(
	id: &'static str,
	label: &'static str,
	options: impl IntoIterator<Item = T>,
	selected: T,
	mut on_select: impl FnMut(T) + 'static,
) -> Element {
	let options: Vec<T> = options.into_iter().collect();
	let choices = options.clone();
	let selected = selected.to_string();
	rsx! {
		div { class: "settings-field",
			label { r#for: id, "{label}" }
			select {
				id,
				class: "client-input",
				value: selected.clone(),
				onchange: move |event: FormEvent| {
					if let Some(choice) = choices
						.iter()
						.find(|choice| choice.to_string() == event.value())
					{
						on_select(*choice);
					}
				},
				for choice in options {
					option {
						key: "{choice}",
						value: "{choice}",
						selected: choice.to_string() == selected,
						"{choice}"
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

#[cfg(test)]
mod tests {
	use super::*;

	fn preview(initial: Config) -> Element {
		let config = use_signal(move || initial);
		let revision = use_signal(|| 0);
		use_context_provider(|| Session { config, revision });
		let navigation = navigation::use_navigation(config);
		use_context_provider(|| navigation);
		let mut page = navigation.page;
		use_hook(move || page.set(Page::Settings));
		let panels = navigation::use_panels();
		use_context_provider(|| panels);
		let sync = polling::use_sync_state();
		use_context_provider(|| sync);
		let finished = use_signal(|| true);
		use_context_provider(|| setup::Finished(finished));
		use_context_provider(notes::NotesStore::new);
		let theme = use_signal(|| Theme::System);
		use_context_provider(|| ThemePreferenceState(theme));
		provide_full_sync();
		rsx! { div { class: "app", navigation::Topbar {} calendar_view::PageContent {} } }
	}

	fn select<'a>(html: &'a str, id: &str) -> &'a str {
		html.split_once(&format!("id=\"{id}\""))
			.unwrap()
			.1
			.split_once("</select>")
			.unwrap()
			.0
	}

	#[test]
	fn settings_start_with_desktop_preferences_and_grouped_sections() {
		let mut config = Config::default();
		config.appearance.time_format = TimeFormat::TwelveHour;
		let mut dom = VirtualDom::new_with_props(preview, config);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("<main class=\"settings-page\""));
		assert!(!html.contains("modal-backdrop"));
		assert!(!html.contains("role=\"dialog\""));
		assert!(!html.contains("modal-close"));
		assert!(!html.contains("day-header"));
		assert!(!html.contains("class=\"fab\""));
		let mut previous = 0;
		for title in [
			"Preferences",
			"Reminders",
			"Database sync",
			"Notes",
			"Google Calendar",
			"Sleep",
			"AI",
			"Application",
		] {
			let position = html.find(&format!("aria-label=\"{title}\"")).unwrap();
			assert!(position > previous, "{title} out of order in {html}");
			previous = position;
		}
		for (id, labels) in [
			(
				"settings-time-format",
				vec!["12-hour (AM/PM)", "12-hour (a/p)", "24-hour"],
			),
			("settings-first-day", vec!["Saturday", "Sunday", "Monday"]),
			(
				"settings-week-range",
				vec!["7 days", "8 days", "8 days + ystd."],
			),
			(
				"settings-day-end",
				vec!["Midnight", "1 AM", "2 AM", "3 AM", "4 AM", "5 AM"],
			),
			("settings-distance-unit", vec!["Kilometers", "Miles"]),
			(
				"settings-theme",
				vec![
					"Light",
					"Light high contrast",
					"Dark",
					"Dark high contrast",
					"System",
					"System high contrast",
				],
			),
		] {
			let menu = select(&html, id);
			assert_eq!(
				menu.matches("<option").count(),
				labels.len(),
				"{id}: {menu}"
			);
			let mut previous = 0;
			for label in labels {
				let position = menu.find(&format!(">{label}</option>")).unwrap();
				assert!(position > previous, "{id}: {menu}");
				previous = position;
			}
		}
		assert!(html.contains("Day ends at midnight"));
		if let Ok(directory) = std::env::var("TIMEPARAGON_SETTINGS_PREVIEW_DIR") {
			let page = format!(
				"<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><style>{}</style></head><body>{html}</body></html>",
				style::STYLESHEET.as_str()
			);
			std::fs::write(format!("{directory}/settings.html"), page).unwrap();
		}
	}

	#[test]
	fn preference_changes_survive_reopening_and_preserve_other_settings() {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("settings.sqlite3");
		// Fields outside the phone's appearance model must survive its saves.
		timeparagon_core::settings::update(&path, |config: &mut serde_json::Value| {
			*config = serde_json::json!({"appearance": {"theme": "Dark"}, "calendar": {"client_id": "keep-client"}});
		}).unwrap();
		for value in TimeFormat::ALL {
			let saved = PreferenceChange::TimeFormat(value).save(&path).unwrap();
			assert_eq!(saved.appearance.time_format, value);
		}
		for value in FirstDayOfWeek::ALL {
			PreferenceChange::FirstDay(value).save(&path).unwrap();
		}
		for value in WeekRange::ALL {
			PreferenceChange::WeekRange(value).save(&path).unwrap();
		}
		for hour in 0..=CalendarSettings::MAX_DAY_END_HOUR {
			PreferenceChange::DayEnd(hour).save(&path).unwrap();
		}
		for value in DistanceUnit::ALL {
			PreferenceChange::DistanceUnit(value).save(&path).unwrap();
		}
		let saved = timeparagon_core::settings::load::<Config>(&path, |_| {})
			.unwrap()
			.value;
		assert_eq!(saved.appearance.time_format, TimeFormat::TwentyFourHour);
		assert_eq!(saved.calendar.first_day_of_week, FirstDayOfWeek::Monday);
		assert_eq!(saved.calendar.week_range(), WeekRange::EightAndYesterday);
		assert_eq!(saved.calendar.day_start_hour(), 5);
		assert_eq!(saved.appearance.distance_unit, DistanceUnit::Miles);
		assert_eq!(saved.calendar.client_id, "keep-client");
		let all = timeparagon_core::settings::load::<serde_json::Value>(&path, |_| {})
			.unwrap()
			.value;
		assert_eq!(all["appearance"]["theme"], "Dark");
		let mut dom = VirtualDom::new_with_props(preview, saved);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("Day ends at 05:00"));
		assert!(select(&html, "settings-time-format").contains("value=\"24-hour\" selected"));
		assert!(select(&html, "settings-day-end").contains("value=\"5\" selected"));
		assert!(select(&html, "settings-distance-unit").contains("value=\"Miles\" selected"));
	}
}
