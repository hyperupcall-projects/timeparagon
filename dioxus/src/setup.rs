//! First-run setup: one page for each thing the app needs from outside
//! itself, from the Google OAuth client to the notes folder.
//!
//! Where the person is, and what they have typed but not yet saved, is kept
//! in `setup.json` beside the secrets, after every change. Leaving the app
//! halfway through therefore reopens the same page with the same text, and
//! the app proper only opens once every required page is done.
//!
//! The pages are explained at length in `docs/setup.md`; each page links to
//! its own section there.

use std::collections::BTreeMap;
use std::path::PathBuf;

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use timeparagon_core::config::Config;
use timeparagon_core::secrets;

use crate::day::{self, Theme};

const DOCS: &str = "https://github.com/hyperupcall-projects/timeparagon/blob/main/docs/setup.md";

/// The pages, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
	Welcome,
	Sync,
	GoogleClient,
	GoogleSignIn,
	Notes,
	Mapbox,
	OpenRouter,
	Notifications,
	Done,
}

impl Step {
	pub const ALL: [Step; 9] = [
		Step::Welcome,
		Step::Sync,
		Step::GoogleClient,
		Step::GoogleSignIn,
		Step::Notes,
		Step::Mapbox,
		Step::OpenRouter,
		Step::Notifications,
		Step::Done,
	];

	/// What `setup.json` calls the page. Stable, unlike its position.
	fn id(self) -> &'static str {
		match self {
			Step::Welcome => "welcome",
			Step::GoogleClient => "google-client",
			Step::GoogleSignIn => "google-sign-in",
			Step::Sync => "database-sync",
			Step::Notes => "notes-folder",
			Step::Mapbox => "mapbox",
			Step::OpenRouter => "openrouter",
			Step::Notifications => "notifications",
			Step::Done => "done",
		}
	}

	/// The page called `id`, or the first one for anything unknown.
	fn parse(id: &str) -> Self {
		Self::ALL
			.into_iter()
			.find(|step| step.id() == id)
			.unwrap_or(Step::Welcome)
	}

	fn index(self) -> usize {
		Self::ALL.iter().position(|step| *step == self).unwrap_or(0)
	}

	fn next(self) -> Self {
		Self::ALL
			.get(self.index() + 1)
			.copied()
			.unwrap_or(Step::Done)
	}

	fn previous(self) -> Self {
		Self::ALL[self.index().saturating_sub(1)]
	}

	fn title(self) -> &'static str {
		match self {
			Step::Welcome => "Welcome to TimeParagon",
			Step::GoogleClient => "Google Cloud OAuth client",
			Step::GoogleSignIn => "Sign in to Google",
			Step::Sync => "Database sync",
			Step::Notes => "Notes folder",
			Step::Mapbox => "Mapbox",
			Step::OpenRouter => "OpenRouter",
			Step::Notifications => "Notifications",
			Step::Done => "All set",
		}
	}

	/// Pages that can be skipped.
	fn optional(self) -> bool {
		matches!(self, Step::Mapbox | Step::OpenRouter | Step::Notifications)
	}

	/// Its section of `docs/setup.md`.
	fn docs(self) -> Option<String> {
		let anchor = match self {
			Step::Welcome | Step::Done => return None,
			Step::GoogleClient => "google-cloud-oauth-client",
			Step::GoogleSignIn => "google-sign-in",
			Step::Sync => "database-sync",
			Step::Notes => "notes-folder",
			Step::Mapbox => "mapbox-optional",
			Step::OpenRouter => "openrouter-optional",
			Step::Notifications => "notifications-optional",
		};
		Some(format!("{DOCS}#{anchor}"))
	}
}

/// Draft keys: what is typed on a page before it is saved.
mod keys {
	pub const CLIENT_ID: &str = "client_id";
	pub const CLIENT_SECRET: &str = "client_secret";
	pub const SYNC_URL: &str = "sync_url";
	/// The sync URL that last connected, so a test survives a restart.
	pub const SYNC_TESTED: &str = "sync_tested";
	pub const MAPBOX: &str = "mapbox";
	pub const OPENROUTER: &str = "openrouter";
}

/// How far setup has got on this device.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Progress {
	/// The page last shown, by [`Step::id`].
	pub step: String,
	pub finished: bool,
	/// What was typed on each page, by the names in [`keys`].
	pub drafts: BTreeMap<String, String>,
}

/// `setup.json`, kept next to the secrets since drafts can hold them.
fn path() -> Option<PathBuf> {
	let directories = timeparagon_core::config::project_dirs()?;
	Some(
		directories
			.state_dir()
			.unwrap_or_else(|| directories.data_local_dir())
			.join("setup.json"),
	)
}

/// Whether this device was set up before setup existed, so a person
/// updating the app is not walked through it again.
fn already_configured(config: &Config) -> bool {
	!config.calendar.client_id.trim().is_empty()
		&& day::signed_in()
		&& !config.database_sync.url.trim().is_empty()
		&& !config.notes.is_empty()
}

impl Progress {
	pub fn load() -> Self {
		let Some(path) = path() else {
			return Self::default();
		};
		match std::fs::read(&path) {
			Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
				eprintln!("could not read {}: {error}", path.display());
				Self::default()
			}),
			Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
				let progress = Self {
					finished: already_configured(&day::load_config()),
					..Self::default()
				};
				if progress.finished {
					progress.save();
				}
				progress
			}
			Err(error) => {
				eprintln!("could not read {}: {error}", path.display());
				Self::default()
			}
		}
	}

	fn save(&self) {
		let Some(path) = path() else {
			return;
		};
		let result = path
			.parent()
			.map_or(Ok(()), std::fs::create_dir_all)
			.and_then(|()| serde_json::to_vec_pretty(self).map_err(std::io::Error::other))
			.and_then(|bytes| {
				timeparagon_core::fsutil::write_atomic(
					&path,
					&bytes,
					timeparagon_core::fsutil::Kind::Private,
				)
			});
		if let Err(error) = result {
			eprintln!("could not save the setup progress: {error}");
		}
	}

	/// Starts setup over from the first page, as Settings → Run setup does.
	pub fn restart() {
		Self::default().save();
	}

	fn draft(&self, key: &str) -> String {
		self.drafts.get(key).cloned().unwrap_or_default()
	}

	/// Fills the drafts of `step` from what is saved, which can have arrived
	/// from the desktop by database sync, while none of them has been typed.
	fn prefill(&mut self, step: Step, config: &Config) {
		let saved: Vec<(&str, String)> = match step {
			Step::GoogleClient => vec![
				(keys::CLIENT_ID, config.calendar.client_id.clone()),
				(keys::CLIENT_SECRET, config.calendar.client_secret.clone()),
			],
			Step::Sync => vec![(keys::SYNC_URL, config.database_sync.url.clone())],
			Step::Mapbox => vec![(keys::MAPBOX, config.travel.api_key.clone())],
			Step::OpenRouter => vec![(keys::OPENROUTER, openrouter_token())],
			_ => Vec::new(),
		};
		let untouched = saved
			.iter()
			.all(|(key, _)| self.draft(key).trim().is_empty());
		for (key, value) in saved {
			if untouched && !value.trim().is_empty() {
				self.drafts.insert(key.to_owned(), value);
			}
		}
	}
}

/// The `finished` flag the app opens on, so Settings can run setup again.
#[derive(Clone, Copy)]
pub struct Finished(pub Signal<bool>);

fn openrouter_token() -> String {
	secrets::path()
		.and_then(|path| {
			secrets::read::<String>(&path, secrets::keys::OPENROUTER_TOKEN)
				.ok()
				.flatten()
		})
		.unwrap_or_default()
}

fn save_openrouter_token(token: &str) -> Result<(), String> {
	let path = secrets::path().ok_or("No place to keep the token on this device")?;
	secrets::write(&path, secrets::keys::OPENROUTER_TOKEN, token.trim())
		.map_err(|error| format!("Could not save the OpenRouter token: {error}"))
}

const TESTING: &str = "Testing the connection…";
const SYNCING: &str = "Syncing the database…";

/// How long the first sync may take before setup gives up on it.
const FIRST_SYNC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// Saves `url` as the sync server and waits for the first sync with it to
/// finish, so the pages after this one find what the server shares.
/// Blocking.
fn sync_from_server(url: &str) -> Result<(), String> {
	use timeparagon_core::database_sync;
	let after = database_sync::synced_cycles();
	timeparagon_core::config::save_database_sync_url(url)
		.map_err(|error| format!("Connected, but could not save the URL: {error}"))?;
	database_sync::wait_for_sync(url, after, FIRST_SYNC_TIMEOUT)
}

fn open_link(url: String) {
	spawn(async move {
		let _ = tokio::task::spawn_blocking(move || {
			timeparagon_core::calendar::oauth::open_in_browser(&url)
		})
		.await;
	});
}

/// Saves what `step` asks for, or says why it cannot be left yet.
fn complete(step: Step, progress: &Progress) -> Result<(), String> {
	match step {
		Step::Welcome | Step::Notifications | Step::Done => Ok(()),
		Step::GoogleClient => {
			let secret = progress.draft(keys::CLIENT_SECRET);
			if secret.trim().is_empty() {
				return Err("Enter the Google OAuth client secret.".into());
			}
			day::save_google_client(&progress.draft(keys::CLIENT_ID), &secret).map(|_| ())
		}
		Step::GoogleSignIn => day::signed_in()
			.then_some(())
			.ok_or_else(|| "Sign in with Google first.".into()),
		Step::Sync => {
			let url = progress.draft(keys::SYNC_URL);
			let tested = progress.draft(keys::SYNC_TESTED);
			if url.trim().is_empty() || url.trim() != tested.trim() {
				return Err(
					"Press Connect and sync first: the server has to connect and sync before you can go on.".into(),
				);
			}
			Ok(())
		}
		Step::Notes => {
			if timeparagon_core::config::NotesSettings::load().is_empty() {
				Err("Choose a notes folder first.".into())
			} else {
				Ok(())
			}
		}
		Step::Mapbox => {
			let token = progress.draft(keys::MAPBOX);
			if token.trim().is_empty() {
				return Ok(());
			}
			timeparagon_core::config::save_mapbox_token(&token)
				.map_err(|error| format!("Could not save the Mapbox token: {error}"))
		}
		Step::OpenRouter => {
			let token = progress.draft(keys::OPENROUTER);
			if token.trim().is_empty() {
				return Ok(());
			}
			save_openrouter_token(&token)
		}
	}
}

#[component]
pub fn SetupFlow(on_finish: EventHandler<()>) -> Element {
	let mut config = use_signal(day::load_config);
	let mut progress = use_signal(|| {
		let mut progress = Progress::load();
		let step = Step::parse(&progress.step);
		progress.prefill(step, &day::load_config());
		progress
	});
	// The notes folder page is the one Settings has, which wants the store.
	use_context_provider(crate::notes::NotesStore::new);
	let mut error = use_signal(|| None::<String>);
	let mut busy = use_signal(|| false);
	let mut signed_in = use_signal(day::signed_in);
	// What the sync page is waiting for, while it waits.
	let mut sync_phase = use_signal(|| None::<&'static str>);

	use_effect(move || {
		let name = Theme::load().name();
		let _ = document::eval(&format!(
			"document.documentElement.dataset.theme = '{name}';"
		));
	});

	let step = Step::parse(&progress.read().step);
	let draft = move |key: &str| progress.read().draft(key);
	let mut set_draft = move |key: &'static str, value: String| {
		let mut progress = progress.write();
		progress.drafts.insert(key.to_owned(), value);
		progress.save();
	};
	let mut go = move |to: Step| {
		error.set(None);
		let saved = day::load_config();
		let mut progress = progress.write();
		progress.step = to.id().to_owned();
		progress.prefill(to, &saved);
		progress.save();
		config.set(saved);
	};
	let next = move |_| {
		let current = Step::parse(&progress.peek().step);
		let result = complete(current, &progress.peek());
		match result {
			Ok(()) => go(current.next()),
			Err(message) => error.set(Some(message)),
		}
	};
	let finish = move |_| {
		let done = Progress {
			step: Step::Done.id().to_owned(),
			finished: true,
			drafts: BTreeMap::new(),
		};
		done.save();
		progress.set(done);
		on_finish.call(());
	};

	let body = match step {
		Step::Welcome => rsx! {
			p { "TimeParagon needs a few things before it can show your day. This takes about ten minutes the first time." }
			ol { class: "setup-list",
				li { "The address of your database sync server" }
				li { "A Google Cloud OAuth client, and signing in with it" }
				li { "A folder for your Markdown notes" }
				li { "Optionally, Mapbox and OpenRouter tokens and notification access" }
			}
			p { class: "hint",
				"You can leave at any time: what you have entered is kept, and setup opens on the same page next time until every step is done."
			}
		},
		Step::GoogleClient => rsx! {
			p { "Calendar, Tasks and contacts come from Google, through an OAuth client in your own Google Cloud project." }
			ol { class: "setup-list",
				li { "In Google Cloud Console, create a project." }
				li { "Enable the Calendar, Tasks, Gmail and People APIs." }
				li { "Configure the OAuth consent screen and add yourself as a test user." }
				li { "Create an OAuth client ID of type " b { "Desktop app" } "." }
				li { "Copy its client ID and client secret here." }
			}
			p { class: "hint", "If your computer already has a client, database sync brought it over and the fields are filled in." }
			label { r#for: "setup-client-id", "Client ID" }
			input {
				id: "setup-client-id",
				class: "client-input",
				r#type: "text",
				autocomplete: "off",
				autocapitalize: "off",
				spellcheck: "false",
				placeholder: "1234-abc.apps.googleusercontent.com",
				value: "{draft(keys::CLIENT_ID)}",
				oninput: move |event| set_draft(keys::CLIENT_ID, event.value()),
			}
			label { r#for: "setup-client-secret", "Client secret" }
			input {
				id: "setup-client-secret",
				class: "client-input",
				r#type: "password",
				autocomplete: "off",
				autocapitalize: "off",
				spellcheck: "false",
				placeholder: "GOCSPX-…",
				value: "{draft(keys::CLIENT_SECRET)}",
				oninput: move |event| set_draft(keys::CLIENT_SECRET, event.value()),
			}
		},
		Step::GoogleSignIn => rsx! {
			p { "Sign in with the Google account whose calendar and tasks you want to see. Your browser opens; approve every permission and then return here." }
			p { class: "hint",
				"Google warns that the app is unverified, since the OAuth client is your own: choose Advanced → Go to (your app)."
			}
			if signed_in() {
				p { class: "setup-ok", "✓ Signed in to Google." }
			}
			div { class: "state-actions",
				button {
					class: if signed_in() { "btn" } else { "btn primary" },
					disabled: busy(),
					onclick: move |_| {
						busy.set(true);
						error.set(None);
						let settings = day::load_config().calendar;
						spawn(async move {
							let result = tokio::task::spawn_blocking(move || day::connect_google(&settings))
								.await
								.map_err(|error| format!("The sign-in worker stopped: {error}"))
								.and_then(|result| result);
							busy.set(false);
							match result {
								Ok(()) => signed_in.set(true),
								Err(message) => error.set(Some(message)),
							}
						});
					},
					if busy() {
						"Waiting for Google…"
					} else if signed_in() {
						"Sign in again"
					} else {
						"Sign in with Google"
					}
				}
			}
		},
		Step::Sync => {
			let url = draft(keys::SYNC_URL);
			let connected = !url.trim().is_empty() && url.trim() == draft(keys::SYNC_TESTED).trim();
			rsx! {
				p { "Every device keeps its own database and swaps changes with a Turso sync server you run, usually on your computer or a home server." }
				p {
					"Use the "
					b { "Tailscale" }
					" address of that server, such as "
					code { "https://desktop.tail1234.ts.net:8443" }
					". With Tailscale on this phone too, the server is reachable from anywhere without opening it to the internet."
				}
				label { r#for: "setup-sync-url", "Sync server URL" }
				input {
					id: "setup-sync-url",
					class: "client-input",
					r#type: "url",
					inputmode: "url",
					autocomplete: "off",
					autocapitalize: "off",
					spellcheck: "false",
					placeholder: "https://desktop.tail1234.ts.net:8443",
					value: "{url}",
					oninput: move |event| set_draft(keys::SYNC_URL, event.value()),
				}
				if let Some(phase) = sync_phase() {
					div { class: "setup-syncing", role: "status",
						div { class: "setup-bar", div { class: "setup-bar-fill" } }
						p { "{phase}" }
						if phase == SYNCING {
							p { class: "hint",
								"Getting everything from the server, including the settings your computer shares, such as the Google client. The first time can take a minute."
							}
						}
					}
				} else if connected {
					p { class: "setup-ok", "✓ Connected and synced. The URL is saved." }
				}
				div { class: "state-actions",
					button {
						class: if connected { "btn" } else { "btn primary" },
						disabled: busy() || url.trim().is_empty(),
						onclick: move |_| {
							let url = draft(keys::SYNC_URL).trim().to_owned();
							busy.set(true);
							error.set(None);
							sync_phase.set(Some(TESTING));
							spawn(async move {
								let tested = url.clone();
								let result = tokio::task::spawn_blocking(move || {
									timeparagon_core::database_sync::test_connection(&tested)
								})
								.await
								.map_err(|error| format!("The connection test stopped: {error}"))
								.and_then(|result| result);
								let result = match result {
									Ok(()) => {
										sync_phase.set(Some(SYNCING));
										let saved = url.clone();
										tokio::task::spawn_blocking(move || sync_from_server(&saved))
											.await
											.map_err(|error| format!("Syncing stopped: {error}"))
											.and_then(|result| result)
									}
									Err(message) => Err(message),
								};
								busy.set(false);
								sync_phase.set(None);
								match result {
									Ok(()) => set_draft(keys::SYNC_TESTED, url),
									Err(message) => error.set(Some(message)),
								}
							});
						},
						if busy() { "Connecting…" } else { "Connect and sync" }
					}
				}
			}
		}
		Step::Notes => rsx! {
			p { "Notes are Markdown files, one per note, in a folder on this phone." }
			p {
				"Use "
				b { "Syncthing" }
				" to keep that folder in step with the notes folder on your computer; notes do not travel through database sync."
			}
			p { class: "hint",
				"Create and share the folder in Syncthing first, then choose it here with Choose folder."
			}
			crate::notes::NotesSettings { config, heading: false }
		},
		Step::Mapbox => rsx! {
			p { "A Mapbox access token lets you search for places and see travel time between events." }
			p { class: "hint",
				"This setting is shared: if your computer already has a token, database sync brings it here. Leave it blank to skip."
			}
			label { r#for: "setup-mapbox", "Mapbox access token" }
			input {
				id: "setup-mapbox",
				class: "client-input",
				r#type: "password",
				autocomplete: "off",
				autocapitalize: "off",
				spellcheck: "false",
				placeholder: "pk.…",
				value: "{draft(keys::MAPBOX)}",
				oninput: move |event| set_draft(keys::MAPBOX, event.value()),
			}
		},
		Step::OpenRouter => rsx! {
			p { "An OpenRouter API key lets TimeParagon turn speech and pictures into text, and pull event and task details out of it." }
			p { class: "hint",
				"The key stays on this phone only. Everything else, such as which models to use, is configured on your computer. Leave it blank to skip."
			}
			label { r#for: "setup-openrouter", "OpenRouter API key" }
			input {
				id: "setup-openrouter",
				class: "client-input",
				r#type: "password",
				autocomplete: "off",
				autocapitalize: "off",
				spellcheck: "false",
				placeholder: "sk-or-v1-…",
				value: "{draft(keys::OPENROUTER)}",
				oninput: move |event| set_draft(keys::OPENROUTER, event.value()),
			}
		},
		Step::Notifications => rsx! {
			p { "Reminders and alarms for events, tasks and bedtime are shown as notifications." }
			if crate::reminders::notifications_blocked() {
				p { class: "error-text", "Notifications are switched off for TimeParagon. Turn them on in Android's settings under Apps → TimeParagon → Notifications." }
			}
			div { class: "state-actions",
				button {
					class: "btn primary",
					onclick: move |_| crate::reminders::request_permission(),
					"Allow notifications"
				}
			}
		},
		Step::Done => rsx! {
			p { "TimeParagon is ready. Your day loads from Google, and habits, goals and settings arrive from your other devices through database sync." }
			p { class: "hint",
				"You can change any of this in Settings, or go through these pages again with Settings → Run setup."
			}
			p { class: "hint",
				"Tip: long-press the home screen to add the TimeParagon day widget."
			}
		},
	};

	let total = Step::ALL.len();
	let number = step.index() + 1;
	let dots = Step::ALL.into_iter().map(|each| {
		let class = if each == step {
			"setup-dot current"
		} else if each.index() < step.index() {
			"setup-dot done"
		} else {
			"setup-dot"
		};
		rsx! { span { key: "{each.id()}", class: class } }
	});
	let docs = step.docs();

	rsx! {
		document::Title { "TimeParagon · Setup" }
		document::Style { {crate::style::STYLESHEET.as_str()} }
		div { class: "app",
			div { class: "state-screen setup-screen",
				div { class: "state-card setup-card", role: "form", aria_label: "Setup",
					div { class: "setup-progress", aria_label: "Step {number} of {total}",
						{dots}
					}
					p { class: "hint",
						"Step {number} of {total}"
						if step.optional() { " · optional" }
					}
					h2 { "{step.title()}" }
					div { class: "client-form setup-body", {body} }
					if let Some(url) = docs {
						p { class: "hint",
							"Detailed instructions: "
							button {
								class: "link-button",
								onclick: move |_| open_link(url.clone()),
								"docs/setup.md"
							}
						}
					}
					if let Some(message) = error() {
						p { class: "error-text", role: "alert", "{message}" }
					}
					div { class: "state-actions setup-actions",
						if step != Step::Welcome {
							button {
								class: "btn",
								disabled: busy(),
								onclick: move |_| go(step.previous()),
								"Back"
							}
						}
						div { class: "spacer" }
						if step.optional() {
							button {
								class: "btn",
								disabled: busy(),
								onclick: move |_| go(step.next()),
								"Skip"
							}
						}
						if step == Step::Done {
							button { class: "btn primary", onclick: finish, "Start using TimeParagon" }
						} else {
							button {
								class: "btn primary",
								disabled: busy(),
								onclick: next,
								if step == Step::Welcome { "Get started" } else { "Next" }
							}
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

	#[test]
	fn a_new_device_opens_on_the_welcome_page() {
		fn app() -> Element {
			rsx! { SetupFlow { on_finish: |_| {} } }
		}
		let mut dom = VirtualDom::new(app);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("Welcome to TimeParagon"), "{html}");
		assert!(html.contains("Step 1 of 9"), "{html}");
		assert!(html.contains("Get started"), "{html}");
	}

	#[test]
	fn every_step_reads_back_from_its_id() {
		for step in Step::ALL {
			assert_eq!(Step::parse(step.id()), step);
		}
		assert_eq!(Step::parse("something-removed"), Step::Welcome);
	}

	#[test]
	fn steps_run_in_order_and_stop_at_done() {
		assert_eq!(Step::Welcome.previous(), Step::Welcome);
		assert_eq!(Step::Done.next(), Step::Done);
		for pair in Step::ALL.windows(2) {
			assert_eq!(pair[0].next(), pair[1]);
			assert_eq!(pair[1].previous(), pair[0]);
		}
	}

	#[test]
	fn only_mapbox_openrouter_and_notifications_can_be_skipped() {
		let optional: Vec<Step> = Step::ALL.into_iter().filter(|s| s.optional()).collect();
		assert_eq!(
			optional,
			[Step::Mapbox, Step::OpenRouter, Step::Notifications]
		);
	}

	#[test]
	fn the_sync_page_needs_the_typed_url_to_have_connected() {
		let mut progress = Progress::default();
		progress
			.drafts
			.insert(keys::SYNC_URL.into(), "https://a.example".into());
		assert!(complete(Step::Sync, &progress).is_err());
		progress
			.drafts
			.insert(keys::SYNC_TESTED.into(), "https://b.example".into());
		assert!(complete(Step::Sync, &progress).is_err());
		progress
			.drafts
			.insert(keys::SYNC_TESTED.into(), "https://a.example".into());
		assert!(complete(Step::Sync, &progress).is_ok());
	}

	#[test]
	fn progress_keeps_its_page_and_drafts_through_json() {
		let mut progress = Progress {
			step: Step::Notes.id().into(),
			..Progress::default()
		};
		progress.drafts.insert(keys::CLIENT_ID.into(), "id".into());
		let text = serde_json::to_string(&progress).unwrap();
		let back: Progress = serde_json::from_str(&text).unwrap();
		assert_eq!(back, progress);
		assert_eq!(Step::parse(&back.step), Step::Notes);
	}
}
