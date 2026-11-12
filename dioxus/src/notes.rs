//! Notes: Markdown files in the notes directory, a setting of each device.
//!
//! The files are handled in `timeparagon_core::notes`, the same as on the
//! desktop. This is the phone's screen for them, and the section that
//! shows a note's connections in the editors.

use std::time::Duration;

use chrono::NaiveDate;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use timeparagon_core::calendar::{self, Event};
use timeparagon_core::config::CalendarSettings;
use timeparagon_core::notes::markdown::{self, Block, Inline, Marker};
use timeparagon_core::notes::{self, Library, Link, LinkKind, Pastel};
use timeparagon_core::timeblocking as tb;

/// How long typing must pause before the local copy is written.
const SAVE_AFTER: Duration = Duration::from_millis(400);
const MAX_CANDIDATES: usize = 8;
/// Shown while notes have nowhere to go.
const NO_DIRECTORY: &str = "Notes are not saved: choose a notes folder in Settings";

/// The note an editor asked to open, shared through context.
#[derive(Clone, Copy)]
pub struct NoteOpening(pub Signal<Option<String>>);

/// The notes on this device and their pending local saves.
#[derive(Clone, Copy, PartialEq)]
pub struct NotesStore {
	pub library: Signal<Library>,
	/// The notes directory, when one is set.
	directory: Signal<Option<std::path::PathBuf>>,
	/// The directory's fingerprint when it was last read.
	fingerprint: Signal<Option<u64>>,
	pub error: Signal<Option<String>>,
	/// Counts edits, so only the last of a burst of typing starts a save.
	edits: Signal<u64>,
	saved_edits: Signal<u64>,
}

impl NotesStore {
	pub fn new() -> Self {
		let directory = notes::directory();
		let mut error = None;
		let library = match &directory {
			Some(directory) => Library::load(directory),
			None => {
				error = Some(NO_DIRECTORY.to_owned());
				Library::default()
			}
		};
		Self {
			library: Signal::new(library),
			directory: Signal::new(directory),
			fingerprint: Signal::new(None),
			error: Signal::new(error),
			edits: Signal::new(0),
			saved_edits: Signal::new(0),
		}
	}

	/// Reads the directory again when its files changed.
	pub fn reload_shared(mut self) {
		if *self.edits.peek() != *self.saved_edits.peek() {
			return;
		}
		let Some(directory) = self.directory.peek().clone() else {
			return;
		};
		let fingerprint = notes::fingerprint(&directory);
		if *self.fingerprint.peek() == Some(fingerprint) {
			return;
		}
		match Library::read(&directory) {
			Ok(mut library) => {
				library.adopt_identities(&self.library.peek());
				if library != *self.library.peek() {
					self.library.set(library);
				}
				self.fingerprint.set(Some(fingerprint));
			}
			Err(error) => eprintln!("Could not reload notes: {error}"),
		}
	}

	fn save_local(mut self) -> bool {
		let Some(directory) = self.directory.peek().clone() else {
			self.error.set(Some(NO_DIRECTORY.into()));
			return false;
		};
		let result = self.library.peek().save(&directory);
		match result {
			Ok(saved) => {
				self.library.write().saved(&saved);
				self.error.set(None);
				self.saved_edits.set(*self.edits.peek());
				true
			}
			Err(error) => {
				let mut message = self.error;
				message.set(Some(format!("Could not save notes: {error}")));
				false
			}
		}
	}

	pub fn configure(mut self, value: &str) -> Result<(), String> {
		#[cfg(target_os = "android")]
		if !value.trim().is_empty() && !value.trim().starts_with("content://") {
			return Err("Use Choose folder to grant access to your notes folder".into());
		}
		let settings = timeparagon_core::config::NotesSettings {
			directory: value.trim().into(),
		};
		let path = settings.path();
		if path == *self.directory.peek() {
			return Ok(());
		}
		let mut next = match &path {
			Some(path) => Library::read(path).map_err(|e| e.to_string())?,
			None => Library::default(),
		};
		let configured = self.directory.peek().is_some();
		let pending = *self.edits.peek() != *self.saved_edits.peek();
		if configured && pending && !self.save_local() {
			return Err("Could not save pending notes in the previous folder".into());
		}
		if self.directory.peek().is_none() {
			next.notes.extend(self.library.peek().notes.clone());
			if !next.notes.is_empty()
				&& let Some(path) = &path
			{
				let saved = next.save(path).map_err(|e| e.to_string())?;
				next.saved(&saved);
			}
		}
		timeparagon_core::config::save_notes_directory(&settings.directory)
			.map_err(|e| e.to_string())?;
		self.library.set(next);
		self.directory.set(path.clone());
		self.fingerprint.set(None);
		self.saved_edits.set(*self.edits.peek());
		self.error.set(path.is_none().then(|| NO_DIRECTORY.into()));
		Ok(())
	}

	fn open_external(mut self, id: &str) {
		if !self.save_local() {
			return;
		}
		let directory = self.directory.peek().clone();
		let file = self.library.peek().note(id).and_then(|n| n.file.clone());
		if let (Some(directory), Some(file)) = (directory, file) {
			spawn_forever(async move {
				let result =
					tokio::task::spawn_blocking(move || notes::open_file(&directory, &file)).await;
				let error = match result {
					Ok(Ok(())) => None,
					Ok(Err(error)) => Some(error.to_string()),
					Err(error) => Some(error.to_string()),
				};
				self.error.set(error);
			});
		}
	}

	/// Saves after typing pauses.
	pub fn touched(mut self) {
		let edit = *self.edits.peek() + 1;
		self.edits.set(edit);
		// Closing the sheet must not cancel the local save.
		spawn_forever(async move {
			tokio::time::sleep(SAVE_AFTER).await;
			if *self.edits.peek() == edit {
				self.save_local();
			}
		});
	}

	pub fn refresh(self) {
		if *self.edits.peek() != *self.saved_edits.peek() {
			self.save_local();
		} else {
			self.reload_shared();
		}
	}

	/// Puts a note away. An untouched new note is not worth keeping.
	fn close(mut self, id: &str) {
		let blank = self
			.library
			.peek()
			.note(id)
			.is_some_and(notes::Note::is_blank);
		if blank {
			self.library.write().delete(id);
		}
		self.save_local();
	}
}

/// Folder setup is device-local; sync the chosen folders to share Markdown.
#[component]
pub fn NotesSettings(
	config: Signal<timeparagon_core::config::Config>,
	/// Off where the page around it already says this is about notes.
	#[props(default = true)]
	heading: bool,
) -> Element {
	let store = use_context::<NotesStore>();
	let mut input = use_signal(|| timeparagon_core::config::NotesSettings::load().directory);
	let mut problem = use_signal(|| None::<String>);
	#[cfg(target_os = "android")]
	use_future(move || async move {
		loop {
			tokio::time::sleep(Duration::from_millis(500)).await;
			match notes::android::request(serde_json::json!({"op":"selection"})) {
				Ok(result) => {
					if let Some(value) = result["directory"].as_str() {
						input.set(value.to_owned());
						match store.configure(value) {
							Ok(()) => {
								config.set(crate::day::load_config());
								problem.set(None);
							}
							Err(error) => problem.set(Some(error)),
						}
					}
				}
				Err(error) => problem.set(Some(error.to_string())),
			}
		}
	});
	#[cfg(target_os = "android")]
	let picker = rsx! {
			button { class: "btn", onclick: move |_| {
				if let Err(error) = notes::android::request(serde_json::json!({"op":"pick"})) { problem.set(Some(error.to_string())); }
			}, "Choose folder" }
	};
	#[cfg(not(target_os = "android"))]
	let picker: Element = rsx! {};
	rsx! {
		if heading { h3 { class: "settings-heading", "Notes" } }
		label { r#for: "settings-notes-folder", "Notes folder" }
		input {
			id: "settings-notes-folder", class: "client-input", value: "{input}",
			placeholder: "Notes folder", oninput: move |event| input.set(event.value()),
		}
		div { class: "sheet-row",
			{picker}
			button { class: "btn", onclick: move |_| {
				match store.configure(&input()) {
					Ok(()) => { config.set(crate::day::load_config()); problem.set(None); }
					Err(error) => problem.set(Some(error)),
				}
			}, "Apply" }
		}
		p { class: "hint", "Sync this folder with your desktop notes folder to see the same Markdown notes on both devices. Notes do not use database sync." }
		if let Some(error) = problem() { p { class: "error-text", role: "alert", "{error}" } }
	}
}

/// What a note connects to for this event. Habit events have none here: a note
/// attaches to the habit itself, and the habits on the phone are its own.
pub fn target_of(event: &Event, settings: &CalendarSettings) -> Option<Link> {
	if event.details.sleep.is_some() || event.details.habit {
		return None;
	}
	let kind = if event.is_task() {
		LinkKind::Task
	} else if tb::is_block(event, settings) {
		LinkKind::TimeBlock
	} else {
		LinkKind::Event
	};
	let day = event.start.date_naive();
	Some(Link {
		kind,
		id: event.id.clone(),
		title: event.title.clone(),
		day: (day != tb::UNDATED).then_some(day),
	})
}

/// What the open note could connect to, from the day on show.
pub fn candidates(
	note: &notes::Note,
	events: &[Event],
	query: &str,
	settings: &CalendarSettings,
) -> Vec<Link> {
	let query = query.trim().to_lowercase();
	if query.is_empty() {
		return Vec::new();
	}
	let mut found: Vec<Link> = Vec::new();
	for link in events.iter().filter_map(|event| target_of(event, settings)) {
		if link.title.to_lowercase().contains(&query)
			&& !note.is_linked(&link)
			&& !found.iter().any(|other| other.same_target(&link))
		{
			found.push(link);
		}
	}
	found.truncate(MAX_CANDIDATES);
	found
}

pub(crate) fn pastel_css(color: Pastel) -> String {
	let (red, green, blue) = color.rgb();
	format!("background: rgb({red},{green},{blue}); color: #1f1f1f;")
}

fn open_link(url: String) {
	spawn_forever(async move {
		let _ = tokio::task::spawn_blocking(move || calendar::oauth::open_in_browser(&url)).await;
	});
}

/// The list of notes, and the one open from it.
#[component]
pub fn NotesSheet(
	store: NotesStore,
	settings: CalendarSettings,
	/// Everything loaded for the day, to connect a note to.
	events: Vec<Event>,
	/// Opened straight away, from an editor's Notes section.
	opening: Signal<Option<String>>,
	on_follow: EventHandler<Link>,
	on_close: EventHandler<()>,
) -> Element {
	let mut query = use_signal(String::new);
	let initial = opening.peek().clone();
	let mut open = use_signal(move || initial);
	let library = store.library;

	if let Some(id) = open().filter(|id| library.read().note(id).is_some()) {
		return rsx! {
			NoteEditor {
				key: "{id}",
				id: id.clone(),
				store,
				settings: settings.clone(),
				events,
				on_follow,
				on_back: move |()| {
					store.close(&id);
					let mut opening = opening;
					opening.set(None);
					open.set(None);
				},
			}
		};
	}

	let rows: Vec<Element> = library
		.read()
		.search(&query())
		.into_iter()
		.map(|note| {
			let id = note.id.clone();
			let style = pastel_css(note.color);
			let title = note.display_title().to_owned();
			let preview = markdown::plain(&note.body, 120);
			rsx! {
				button {
					key: "{note.id}",
					class: "note-card",
					style: "{style}",
					onclick: move |_| open.set(Some(id.clone())),
					span { class: "note-card-title", "{title}" }
					if !preview.is_empty() {
						span { class: "note-card-preview", "{preview}" }
					}
				}
			}
		})
		.collect();
	let empty = rows.is_empty();

	rsx! {
		div { class: "modal-backdrop editor-backdrop",
			div { class: "modal editor", role: "dialog", aria_label: "Notes",
				div { class: "modal-head",
					div { class: "modal-titles", h2 { class: "modal-title", "Notes" } }
					button {
						class: "modal-close",
						title: "Close",
						onclick: move |_| on_close.call(()),
						"×"
					}
				}
				div { class: "modal-body editor-body",
					if let Some(message) = (store.error)() {
						p { class: "editor-error", role: "alert", "{message}" }
					}
					div { class: "sheet-row",
						input {
							class: "client-input",
							r#type: "search",
							placeholder: "Search notes",
							aria_label: "Search notes",
							value: "{query}",
							oninput: move |event| query.set(event.value()),
						}
						button {
							class: "btn primary",
							onclick: move |_| {
								let mut library = library;
								let id = library.write().create("", Vec::new());
								open.set(Some(id));
							},
							"New"
						}
					}
					if empty {
						p { class: "empty-note",
							if query().trim().is_empty() { "No notes yet." } else { "No notes match." }
						}
					}
					div { class: "note-cards", {rows.into_iter()} }
					button {
						class: "btn",
						onclick: move |_| store.refresh(),
						"Refresh"
					}
				}
			}
		}
	}
}

#[component]
fn NoteEditor(
	id: String,
	store: NotesStore,
	settings: CalendarSettings,
	events: Vec<Event>,
	on_follow: EventHandler<Link>,
	on_back: EventHandler<()>,
) -> Element {
	let mut library = store.library;
	let mut viewing = use_signal(|| false);
	let mut confirm_delete = use_signal(|| false);
	let mut connect_query = use_signal(String::new);

	let Some(note) = library.read().note(&id).cloned() else {
		return rsx! {};
	};
	let surface = pastel_css(note.color);
	let found = candidates(&note, &events, &connect_query(), &settings);

	let swatches = Pastel::ALL.into_iter().map(|color| {
		let id = id.clone();
		let (red, green, blue) = color.rgb();
		let chosen = color == note.color;
		rsx! {
			button {
				key: "{color.name()}",
				class: "note-swatch",
				style: "background: rgb({red},{green},{blue});",
				title: "{color.name()}",
				aria_label: "{color.name()}",
				aria_pressed: chosen.to_string(),
				onclick: move |_| {
					library.write().edit(&id, |note| note.color = color);
					store.touched();
				},
			}
		}
	});

	let connections = note.links.iter().map(|link| {
		let id = id.clone();
		let target = link.clone();
		let followed = link.clone();
		let label = link_label(link);
		rsx! {
			div { key: "{link.id}", class: "editor-item",
				button {
					class: "linklike editor-item-name",
					disabled: link.kind == LinkKind::Habit,
					onclick: move |_| on_follow.call(followed.clone()),
					"{label}"
				}
				button {
					class: "btn small",
					onclick: move |_| {
						library.write().disconnect(&id, &target);
						store.touched();
					},
					"Remove"
				}
			}
		}
	});

	let offers = found.into_iter().map(|link| {
		let id = id.clone();
		let label = link_label(&link);
		let key = format!("{:?}-{}", link.kind, link.id);
		rsx! {
			button {
				key: "{key}",
				class: "editor-item editor-pick",
				onclick: move |_| {
					library.write().connect(&id, link.clone());
					connect_query.set(String::new());
					store.touched();
				},
				span { class: "editor-item-name", "{label}" }
			}
		}
	});

	let title_id = id.clone();
	let body_id = id.clone();
	let delete_id = id.clone();
	let external_id = id.clone();

	rsx! {
		div { class: "modal-backdrop editor-backdrop",
			div { class: "modal editor note-editor", role: "dialog", aria_label: "Note",
				div { class: "modal-head",
					button {
						class: "btn small",
						onclick: move |_| on_back.call(()),
						"‹ Notes"
					}
					div { class: "spacer" }
					button {
						class: "nav-tab",
						aria_pressed: (!viewing()).to_string(),
						onclick: move |_| viewing.set(false),
						"Edit"
					}
					button {
						class: "nav-tab",
						aria_pressed: viewing().to_string(),
						onclick: move |_| viewing.set(true),
						"View"
					}
				}
				div { class: "modal-body editor-body",
					if let Some(message) = (store.error)() {
						p { class: "editor-error", role: "alert", "{message}" }
					}
					div { class: "note-surface", style: "{surface}",
						input {
							class: "note-title",
							r#type: "text",
							placeholder: "Title",
							aria_label: "Title",
							value: "{note.title}",
							oninput: move |event| {
								library.write().edit(&title_id, |note| note.title = event.value());
								store.touched();
							},
						}
						if viewing() {
							NoteView { source: note.body.clone() }
						} else {
							textarea {
								class: "note-text",
								aria_label: "Text",
								placeholder: "Write in Markdown",
								value: "{note.body}",
								oninput: move |event| {
									library.write().edit(&body_id, |note| note.body = event.value());
									store.touched();
								},
							}
						}
					}
					button { class: "btn", onclick: move |_| store.open_external(&external_id), "Open in default app" }
					div { class: "note-swatches", {swatches} }
					h3 { class: "settings-heading", "Connected to" }
					{connections}
					input {
						class: "client-input",
						r#type: "search",
						placeholder: "Connect to something on this day",
						aria_label: "Connect to something on this day",
						value: "{connect_query}",
						oninput: move |event| connect_query.set(event.value()),
					}
					{offers}
					if confirm_delete() {
						p { class: "hint", "Delete this note from all devices?" }
						div { class: "sheet-row",
							button {
								class: "btn danger",
								onclick: move |_| {
									library.write().delete(&delete_id);
									store.touched();
									on_back.call(());
								},
								"Delete"
							}
							button { class: "btn", onclick: move |_| confirm_delete.set(false), "Keep" }
						}
					} else {
						button {
							class: "btn danger",
							onclick: move |_| confirm_delete.set(true),
							"Delete note"
						}
					}
				}
			}
		}
	}
}

fn link_label(link: &Link) -> String {
	let title = if link.title.trim().is_empty() {
		"(No title)"
	} else {
		link.title.as_str()
	};
	match link.day {
		Some(day) => format!("{} · {title} · {}", link.kind.label(), short_day(day)),
		None => format!("{} · {title}", link.kind.label()),
	}
}

fn short_day(day: NaiveDate) -> String {
	day.format("%b %-d, %Y").to_string()
}

/// The View tab. Nothing is ever put in the page as HTML, so whatever a note
/// holds shows as text.
#[component]
pub(crate) fn NoteView(source: String) -> Element {
	let blocks = markdown::parse(&source);
	rsx! {
		div { class: "note-view",
			for (index, block) in blocks.into_iter().enumerate() {
				{block_view(index, block)}
			}
		}
	}
}

fn block_view(key: usize, block: Block) -> Element {
	match block {
		Block::Heading { level, text } => {
			let class = format!("note-h note-h{}", level.clamp(1, 4));
			rsx! { div { key: "{key}", class: "{class}", role: "heading", {inlines(text)} } }
		}
		Block::Paragraph(text) => rsx! { p { key: "{key}", {inlines(text)} } },
		Block::Quote(text) => rsx! { blockquote { key: "{key}", {inlines(text)} } },
		Block::Code(code) => rsx! { pre { key: "{key}", code { "{code}" } } },
		Block::Rule => rsx! { hr { key: "{key}" } },
		Block::Item {
			depth,
			marker,
			text,
		} => {
			let margin = depth * 18;
			let mark = match marker {
				Marker::Bullet => "•".to_owned(),
				Marker::Number(number) => format!("{number}."),
				Marker::Check(true) => "☑".to_owned(),
				Marker::Check(false) => "☐".to_owned(),
				Marker::Continued => String::new(),
			};
			rsx! {
				div { key: "{key}", class: "note-item", style: "margin-left: {margin}px;",
					span { class: "note-mark", "{mark}" }
					span { {inlines(text)} }
				}
			}
		}
	}
}

fn inlines(runs: Vec<Inline>) -> Element {
	rsx! {
		for (index, run) in runs.into_iter().enumerate() {
			{inline(index, run)}
		}
	}
}

fn inline(key: usize, run: Inline) -> Element {
	let mut class = String::from("note-run");
	for (on, name) in [
		(run.bold, " bold"),
		(run.italic, " italic"),
		(run.strike, " strike"),
		(run.code, " code"),
	] {
		if on {
			class.push_str(name);
		}
	}
	// `markdown` only lets web and mail links through.
	match run.link {
		Some(url) => rsx! {
			button {
				key: "{key}",
				class: "linklike {class}",
				onclick: move |_| open_link(url.clone()),
				"{run.text}"
			}
		},
		None => rsx! { span { key: "{key}", class: "{class}", "{run.text}" } },
	}
}

/// The Notes section of an editor: what is connected to this item.
#[component]
pub fn LinkedNotes(target: Link) -> Element {
	let Some(NoteOpening(mut opening)) = try_use_context::<NoteOpening>() else {
		return rsx! {};
	};
	let Some(store) = try_use_context::<NotesStore>() else {
		return rsx! {};
	};
	let mut library = store.library;
	let new_target = target.clone();
	let rows: Vec<Element> = library
		.read()
		.linked(&target)
		.into_iter()
		.map(|note| {
			let id = note.id.clone();
			let style = pastel_css(note.color);
			let title = note.display_title().to_owned();
			rsx! {
				button {
					key: "{note.id}",
					class: "note-card",
					style: "{style}",
					onclick: move |_| opening.set(Some(id.clone())),
					span { class: "note-card-title", "{title}" }
				}
			}
		})
		.collect();
	rsx! {
		div { class: "editor-field",
			span { class: "editor-label", "Notes" }
			div { class: "note-cards", {rows.into_iter()} }
			button {
				class: "btn small",
				onclick: move |_| {
					let id = library.write().create("", vec![new_target.clone()]);
					store.touched();
					opening.set(Some(id));
				},
				"New note"
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::{Local, TimeZone};

	fn event(id: &str, title: &str) -> Event {
		let start = Local.with_ymd_and_hms(2026, 11, 4, 9, 0, 0).unwrap();
		Event {
			id: id.to_owned(),
			calendar_id: "work".to_owned(),
			title: title.to_owned(),
			start,
			end: start + chrono::Duration::hours(1),
			all_day: false,
			color: calendar::colors::DEFAULT_EVENT,
			completed: false,
			details: Default::default(),
		}
	}

	fn note(links: Vec<Link>) -> notes::Note {
		let mut library = Library::default();
		let id = library.create("Plan", links);
		library.note(&id).unwrap().clone()
	}

	#[test]
	fn a_habit_event_takes_no_note_of_its_own() {
		let mut habit = event("h", "Run");
		habit.details.habit = true;
		assert_eq!(target_of(&habit, &CalendarSettings::default()), None);
		let link = target_of(&event("e", "Standup"), &CalendarSettings::default()).unwrap();
		assert_eq!(link.kind, LinkKind::Event);
		assert_eq!(link.day, NaiveDate::from_ymd_opt(2026, 11, 4));
	}

	#[test]
	fn candidates_match_the_query_and_skip_what_is_already_connected() {
		let settings = CalendarSettings::default();
		let events = [
			event("a", "Standup"),
			event("b", "Stand-in"),
			event("c", "Lunch"),
		];
		let connected = target_of(&events[0], &settings).unwrap();
		let note = note(vec![connected]);

		let found = candidates(&note, &events, "stand", &settings);
		assert_eq!(found.len(), 1);
		assert_eq!(found[0].id, "b");
		assert!(candidates(&note, &events, "  ", &settings).is_empty());
	}

	#[test]
	fn a_note_view_never_passes_html_through_and_styles_what_it_knows() {
		let html = dioxus_ssr::render_element(rsx! {
			NoteView { source: "# Hi\n\n**bold** <script>alert(1)</script>\n\n- [x] done".to_owned() }
		});
		assert!(!html.contains("<script>"), "{html}");
		assert!(html.contains("note-h1"), "{html}");
		assert!(html.contains("bold"), "{html}");
		assert!(html.contains('☑'), "{html}");
	}
}
