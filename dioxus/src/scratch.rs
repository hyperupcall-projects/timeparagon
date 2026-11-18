//! The notepad and daily brain dumps, shared through database sync. Each
//! is a sheet of its own.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use chrono::NaiveDate;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use timeparagon_core::config::{self, Config};
use timeparagon_core::notes::Pastel;

/// How long typing must pause before the file is written.
const SAVE_AFTER: Duration = Duration::from_millis(800);

fn written(result: std::io::Result<()>, what: &str) -> Result<Config, String> {
	result.map_err(|error| format!("Could not save the {what}: {error}"))?;
	Ok(Config::load())
}

/// The page a sheet shows. Each has its own entry under More.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum Page {
	#[default]
	Notepad,
	BrainDump,
}

impl Page {
	fn title(self) -> &'static str {
		match self {
			Page::Notepad => "Notepad",
			Page::BrainDump => "Brain dump",
		}
	}
}

#[component]
pub fn ScratchSheet(
	config: Signal<Config>,
	#[props(default)] page: Page,
	on_close: EventHandler<()>,
) -> Element {
	let title = page.title();
	rsx! {
		div { class: "modal-backdrop editor-backdrop",
			div { class: "modal editor", role: "dialog", aria_label: "{title}",
				div { class: "modal-head",
					div { class: "modal-titles", h2 { class: "modal-title", "{title}" } }
					button {
						class: "modal-close",
						title: "Close",
						onclick: move |_| on_close.call(()),
						"×"
					}
				}
				div { class: "modal-body editor-body",
					match page {
						Page::Notepad => rsx! { Notepad { config } },
						Page::BrainDump => rsx! { BrainDump { config } },
					}
				}
			}
		}
	}
}

/// Which page of text a [`SavingText`] holds.
#[derive(Clone, Copy, PartialEq)]
enum TextPage {
	Notepad,
	BrainDump(NaiveDate),
}

/// Writes a page, returning why it could not be.
fn write_page(page: TextPage, text: String, mut config: Signal<Config>) -> Result<(), String> {
	let saved = match page {
		TextPage::Notepad => written(config::save_notepad_text(&text), "notepad")?,
		TextPage::BrainDump(day) => {
			let mut dumps = config.peek().brain_dump.clone();
			// A page emptied again is as if it was never started.
			if text.trim().is_empty() {
				dumps.remove(day);
			} else {
				dumps.set(day, text);
			}
			written(config::save_brain_dump(&dumps), "brain dump")?
		}
	};
	config.set(saved);
	Ok(())
}

/// A page of text saved shortly after typing stops, and at once when the field
/// loses focus. The delayed save outlives the sheet, so closing it right after
/// typing keeps the last words.
#[component]
fn SavingText(
	config: Signal<Config>,
	page: TextPage,
	initial: String,
	placeholder: String,
	#[props(default)] on_delete: Option<EventHandler<()>>,
) -> Element {
	let mut text = use_signal(move || initial);
	let edits = use_hook(|| Arc::new(AtomicU64::new(0)));
	let deleted = use_hook(|| Arc::new(AtomicBool::new(false)));
	let mut error = use_signal(|| None::<String>);
	let blur_edits = edits.clone();
	let blur_deleted = deleted.clone();
	let delete_edits = edits.clone();
	let delete_deleted = deleted.clone();
	rsx! {
		if let Some(message) = error() {
			p { class: "editor-error", role: "alert", "{message}" }
		}
		textarea {
			class: "client-input scratch-text",
			placeholder: "{placeholder}",
			aria_label: "{placeholder}",
			value: "{text}",
			oninput: move |event| {
				let value = event.value();
				text.set(value.clone());
				let edit = edits.fetch_add(1, Ordering::SeqCst) + 1;
				let edits = edits.clone();
				let deleted = deleted.clone();
				spawn_forever(async move {
					tokio::time::sleep(SAVE_AFTER).await;
					if !deleted.load(Ordering::SeqCst) && edits.load(Ordering::SeqCst) == edit {
						let outcome = write_page(page, value, config);
						// The sheet may be gone by now, with nowhere to say so.
						if let Ok(mut shown) = error.try_write() {
							*shown = outcome.err();
						}
					}
				});
			},
			onblur: move |_| {
				// Cancels a delayed save: this one has the same text.
				blur_edits.fetch_add(1, Ordering::SeqCst);
				if !blur_deleted.load(Ordering::SeqCst) {
					error.set(write_page(page, text.peek().clone(), config).err());
				}
			},
		}
		if let Some(on_delete) = on_delete {
			button {
				class: "btn danger",
				onclick: move |_| {
					// Neither a delayed save nor a late blur may restore a deleted page.
					delete_edits.fetch_add(1, Ordering::SeqCst);
					delete_deleted.store(true, Ordering::SeqCst);
					match write_page(page, String::new(), config) {
						Ok(()) => on_delete.call(()),
						Err(message) => {
							delete_deleted.store(false, Ordering::SeqCst);
							error.set(Some(message));
						}
					}
				},
				"Delete"
			}
		}
	}
}

/// The notepad, written in Markdown on Edit and read on View. Leaving Edit
/// blurs the text field, which saves it, so View reads the saved text.
#[component]
fn Notepad(config: Signal<Config>) -> Element {
	let mut viewing = use_signal(|| false);
	let tab = move |label: &'static str, view: bool| {
		rsx! {
			button {
				class: "nav-tab",
				aria_pressed: (viewing() == view).to_string(),
				onclick: move |_| viewing.set(view),
				"{label}"
			}
		}
	};
	let surface = crate::notes::pastel_css(Pastel::default());
	rsx! {
		div { class: "sheet-tabs",
			{tab("Edit", false)}
			{tab("View", true)}
		}
		if viewing() {
			div { class: "note-surface", style: "{surface}",
				crate::notes::NoteView { source: config.read().notepad.text.clone() }
			}
		} else {
			SavingText {
				config,
				page: TextPage::Notepad,
				initial: config.peek().notepad.text.clone(),
				placeholder: "Write anything in Markdown",
			}
		}
	}
}

#[component]
fn BrainDump(config: Signal<Config>) -> Element {
	let query = use_signal(String::new);
	let mut open = use_signal(|| None::<NaiveDate>);
	if let Some(date) = open() {
		rsx! { BrainDumpEditor { key: "{date}", config, date, on_back: move |()| open.set(None) } }
	} else {
		rsx! { BrainDumpList { config, query, on_open: move |date| open.set(Some(date)) } }
	}
}

#[component]
fn BrainDumpList(
	config: Signal<Config>,
	mut query: Signal<String>,
	on_open: EventHandler<NaiveDate>,
) -> Element {
	let data = config.read();
	let entries = data.brain_dump.search(&query());
	let count = entries.len();
	let count_label = if count == 1 {
		"1 page".to_owned()
	} else {
		format!("{count} pages")
	};
	rsx! {
		div { class: "sheet-row",
			div {
				p { class: "hint", "One page for every day" }
				span { class: "hint", aria_live: "polite", "{count_label}" }
			}
			div { class: "spacer" }
			button {
				class: "btn primary",
				onclick: move |_| on_open.call(timeparagon_core::clock::now().date_naive()),
				"Add today"
			}
		}
		input {
			class: "client-input",
			r#type: "search",
			placeholder: "Search every page…",
			aria_label: "Search every page",
			value: "{query}",
			oninput: move |event| query.set(event.value()),
		}
		if entries.is_empty() {
			p { class: "empty-note",
				if query().trim().is_empty() {
					"No brain dumps yet. Add today's to start one."
				} else {
					"No page contains that text."
				}
			}
		}
		div { class: "brain-dump-list", role: "list",
			for entry in entries {
				{
					let date = entry.date;
					let title = date.format("%A, %B %-d, %Y").to_string();
					let preview = entry.text.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>().join(" ");
					let preview = if preview.is_empty() { "Empty page".to_owned() } else { preview.chars().take(140).collect() };
					rsx! {
						article { key: "{date}", class: "brain-dump-card", role: "listitem",
							div { class: "sheet-row",
								button {
									class: "brain-dump-date",
									onclick: move |_| on_open.call(date),
									"{title}"
								}
								button {
									class: "btn small",
									aria_label: "Edit {title}",
									onclick: move |_| on_open.call(date),
									"Edit"
								}
							}
							p { class: "note-card-preview", "{preview}" }
						}
					}
				}
			}
		}
	}
}

#[component]
fn BrainDumpEditor(config: Signal<Config>, date: NaiveDate, on_back: EventHandler<()>) -> Element {
	let initial = config
		.peek()
		.brain_dump
		.get(date)
		.unwrap_or_default()
		.to_owned();
	let title = date.format("%A, %B %-d, %Y").to_string();
	rsx! {
		div { class: "sheet-row",
			button { class: "btn", onclick: move |_| on_back.call(()), "Back to list" }
			p { class: "brain-dump-editor-date", "{title}" }
		}
		p { class: "hint", "Saved as you type" }
		SavingText {
			config,
			page: TextPage::BrainDump(date),
			initial,
			placeholder: "What is on your mind today?",
			on_delete: on_back,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use dioxus::core::{AttributeValue, ElementId, Mutation, Mutations};
	use dioxus::html::{
		PlatformEventData, SerializedFocusData, SerializedFormData, SerializedHtmlEventConverter,
		SerializedMouseData, set_event_converter,
	};
	use std::any::Any;
	use std::rc::Rc;

	fn date(day: u32) -> NaiveDate {
		NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
	}

	fn config() -> Config {
		let mut config = Config::default();
		config
			.brain_dump
			.set(date(27), "Book the Dentist\n\nBring insurance".into());
		config.brain_dump.set(date(29), "Buy milk".into());
		config.brain_dump.set(date(28), "Plan the garden".into());
		config.brain_dump.set(date(26), String::new());
		config
	}

	#[derive(Props, Clone, PartialEq)]
	struct HarnessProps {
		query: String,
		open: Option<NaiveDate>,
		empty: bool,
	}

	#[allow(non_snake_case)]
	fn Harness(props: HarnessProps) -> Element {
		let config = use_signal(move || {
			if props.empty {
				Config::default()
			} else {
				config()
			}
		});
		let query = use_signal(move || props.query);
		if let Some(date) = props.open {
			rsx! { BrainDumpEditor { config, date, on_back: |_| {} } }
		} else {
			rsx! { BrainDumpList { config, query, on_open: |_| {} } }
		}
	}

	fn render(query: &str, open: Option<NaiveDate>, empty: bool) -> String {
		let mut dom = VirtualDom::new_with_props(
			Harness,
			HarnessProps {
				query: query.into(),
				open,
				empty,
			},
		);
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	#[test]
	fn every_page_is_listed_newest_first_with_a_preview() {
		let html = render("", None, false);
		for text in [
			"4 pages",
			"Add today",
			"Search every page",
			"Book the Dentist Bring insurance",
			"Buy milk",
			"Plan the garden",
			"Empty page",
		] {
			assert!(html.contains(text), "{text:?} is missing from {html}");
		}
		let mut previous = 0;
		for day in [29, 28, 27, 26] {
			let title = date(day).format("%A, %B %-d, %Y").to_string();
			let position = html.find(&title).unwrap();
			assert!(position > previous, "pages are not newest first: {html}");
			previous = position;
		}
		assert!(
			!html.contains("<textarea"),
			"the list should open before the editor"
		);
	}

	#[test]
	fn search_matches_page_text_and_dates_ignoring_case() {
		for (query, wanted, hidden) in [
			("  DENT  ", "Book the Dentist Bring insurance", "Buy milk"),
			("october 28", "Plan the garden", "Book the Dentist"),
		] {
			let html = render(query, None, false);
			assert!(html.contains("1 page"), "{html}");
			assert!(html.contains(wanted), "{html}");
			assert!(!html.contains(hidden), "{html}");
		}
	}

	#[test]
	fn an_empty_library_and_an_empty_search_explain_the_next_step() {
		assert!(render("", None, true).contains("No brain dumps yet. Add today"));
		let html = render("not in any page", None, false);
		assert!(html.contains("0 pages"), "{html}");
		assert!(html.contains("No page contains that text."), "{html}");
	}

	#[test]
	fn an_older_page_opens_with_its_own_text_and_editor_actions() {
		let html = render("", Some(date(27)), false);
		for text in [
			"Tuesday, October 27, 2026",
			"Book the Dentist",
			"Bring insurance",
			"Back to list",
			"Saved as you type",
			"Delete",
			"<textarea",
		] {
			assert!(html.contains(text), "{text:?} is missing from {html}");
		}
		assert!(!html.contains("Buy milk"), "{html}");
	}

	fn listener(mutations: &Mutations, event: &str) -> ElementId {
		mutations
			.edits
			.iter()
			.find_map(|mutation| match mutation {
				Mutation::NewEventListener { name, id } if name == event => Some(*id),
				_ => None,
			})
			.unwrap_or_else(|| panic!("no {event} listener in {mutations:?}"))
	}

	fn dispatch(dom: &VirtualDom, event: &str, id: ElementId, data: impl Any) {
		let data = Rc::new(PlatformEventData::new(Box::new(data))) as Rc<dyn Any>;
		dom.runtime()
			.handle_event(event, Event::new(data, true), id);
	}

	#[test]
	fn search_edit_back_and_add_today_work_through_the_list() {
		set_event_converter(Box::new(SerializedHtmlEventConverter));
		let _clock = timeparagon_core::clock::freeze(
			chrono::DateTime::parse_from_rfc3339("2026-10-31T12:00:00-07:00")
				.unwrap()
				.to_utc(),
		);
		fn app() -> Element {
			let config = use_signal(config);
			rsx! { BrainDump { config } }
		}
		let mut dom = VirtualDom::new(app);
		let initial = dom.rebuild_to_vec();
		dispatch(
			&dom,
			"input",
			listener(&initial, "input"),
			SerializedFormData::new("DENT".into(), Vec::new()),
		);
		let searched = dom.render_immediate_to_vec();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("Book the Dentist"), "{html}");
		assert!(!html.contains("Buy milk"), "{html}");

		let edit = searched
			.edits
			.iter()
			.chain(initial.edits.iter())
			.find_map(|mutation| match mutation {
				Mutation::SetAttribute {
					name: "aria-label",
					value: AttributeValue::Text(label),
					id,
					..
				} if label == "Edit Tuesday, October 27, 2026" => Some(*id),
				_ => None,
			})
			.expect("Edit the older page");
		dispatch(&dom, "click", edit, SerializedMouseData::default());
		let editor = dom.render_immediate_to_vec();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("<textarea"), "{html}");
		assert!(html.contains("Bring insurance"), "{html}");

		dispatch(
			&dom,
			"click",
			listener(&editor, "click"),
			SerializedMouseData::default(),
		);
		let list = dom.render_immediate_to_vec();
		let html = dioxus_ssr::render(&dom);
		assert!(
			html.contains("value=\"DENT\""),
			"the search was lost: {html}"
		);
		assert!(!html.contains("<textarea"), "{html}");
		dispatch(
			&dom,
			"click",
			listener(&list, "click"),
			SerializedMouseData::default(),
		);
		let _ = dom.render_immediate_to_vec();
		let html = dioxus_ssr::render(&dom);
		let today = timeparagon_core::clock::now()
			.date_naive()
			.format("%A, %B %-d, %Y")
			.to_string();
		assert!(html.contains(&today), "{html}");
		assert!(html.contains("<textarea"), "{html}");
		assert!(!html.contains("Bring insurance"), "{html}");
	}

	#[test]
	fn deleting_a_page_cancels_pending_autosave_and_ignores_late_blur() {
		set_event_converter(Box::new(SerializedHtmlEventConverter));
		let runtime = tokio::runtime::Builder::new_current_thread()
			.enable_time()
			.build()
			.unwrap();
		runtime.block_on(async {
			fn app() -> Element {
				let config = use_signal(config);
				let mut deleted = use_signal(|| false);
				rsx! {
					SavingText {
						config, page: TextPage::BrainDump(date(27)), initial: "Book the Dentist", placeholder: "Brain dump",
						on_delete: move |()| deleted.set(true),
					}
					if deleted() { p { "Deleted" } }
				}
			}
			let mut dom = VirtualDom::new(app);
			let initial = dom.rebuild_to_vec();
			dispatch(&dom, "input", listener(&initial, "input"), SerializedFormData::new("New words before deleting".into(), Vec::new()));
			let _ = dom.render_immediate_to_vec();
			dispatch(&dom, "click", listener(&initial, "click"), SerializedMouseData::default());
			let _ = dom.render_immediate_to_vec();
			assert!(dioxus_ssr::render(&dom).contains("Deleted"));
			dispatch(&dom, "blur", listener(&initial, "blur"), SerializedFocusData::default());
			tokio::time::sleep(SAVE_AFTER + Duration::from_millis(50)).await;
			let _ = dom.render_immediate_to_vec();
			let saved = Config::load();
			assert_eq!(saved.brain_dump.get(date(27)), None);
			assert_eq!(saved.brain_dump.get(date(29)), Some("Buy milk"));
		});
	}
}
