//! Notes: Markdown documents kept in Google Drive's app folder.
//!
//! The documents and how they sync are in `timeslay_core::notes`, the same as
//! on the desktop. This is the phone's screen for them, and the section that
//! shows a note's connections in the editors.

use std::time::Duration;

use chrono::NaiveDate;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use timeslay_core::calendar::{self, Event};
use timeslay_core::config::CalendarSettings;
use timeslay_core::notes::markdown::{self, Block, Inline, Marker};
use timeslay_core::notes::{self, Library, Link, LinkKind, Pastel};
use timeslay_core::timeblocking as tb;

/// How long typing must pause before the local copy is written.
const SAVE_AFTER: Duration = Duration::from_millis(400);
/// How much longer before the note goes to Drive. Each upload is a whole file.
const UPLOAD_AFTER: Duration = Duration::from_millis(2600);
const MAX_CANDIDATES: usize = 8;

/// The note an editor asked to open, shared through context.
#[derive(Clone, Copy)]
pub struct NoteOpening(pub Signal<Option<String>>);

/// The notes on this device, and whether they are on their way to Drive.
#[derive(Clone, Copy, PartialEq)]
pub struct NotesStore {
    pub library: Signal<Library>,
    pub syncing: Signal<bool>,
    pub error: Signal<Option<String>>,
    /// Something asked for a sync while one was running.
    again: Signal<bool>,
    /// Counts edits, so only the last of a burst of typing starts a sync.
    edits: Signal<u64>,
    saved_edits: Signal<u64>,
}

impl NotesStore {
    pub fn new() -> Self {
        let library = timeslay_core::task_db::path()
            .map(|path| Library::load(&path))
            .unwrap_or_default();
        Self {
            library: Signal::new(library),
            syncing: Signal::new(false),
            error: Signal::new(None),
            again: Signal::new(false),
            edits: Signal::new(0),
            saved_edits: Signal::new(0),
        }
    }

    pub fn reload_shared(mut self) {
        if *self.edits.peek() != *self.saved_edits.peek() || *self.syncing.peek() {
            return;
        }
        let Some(path) = timeslay_core::task_db::path() else {
            return;
        };
        if let Ok(Some(library)) = timeslay_core::task_db::read_document::<Library>(
            &path,
            timeslay_core::task_db::keys::NOTES,
        ) {
            if library != *self.library.peek() {
                self.library.set(library);
            }
        }
    }
    fn save_local(mut self) {
        let Some(path) = timeslay_core::task_db::path() else {
            return;
        };
        if let Err(error) = self.library.peek().save(&path) {
            let mut message = self.error;
            message.set(Some(format!("Could not save notes: {error}")));
        } else {
            self.saved_edits.set(*self.edits.peek());
        }
    }

    /// Records an edit: saved shortly, and uploaded once typing has stopped.
    pub fn touched(mut self, settings: CalendarSettings) {
        let edit = *self.edits.peek() + 1;
        self.edits.set(edit);
        // Not tied to the sheet: closing it must not lose the save or the upload.
        spawn_forever(async move {
            tokio::time::sleep(SAVE_AFTER).await;
            if *self.edits.peek() != edit {
                return;
            }
            self.save_local();
            tokio::time::sleep(UPLOAD_AFTER).await;
            if *self.edits.peek() == edit {
                self.sync(settings);
            }
        });
    }

    /// Brings the library and Drive into step, in the background.
    pub fn sync(mut self, settings: CalendarSettings) {
        if *self.syncing.peek() {
            self.again.set(true);
            return;
        }
        self.syncing.set(true);
        let library = self.library.peek().clone();
        let retry = settings.clone();
        spawn_forever(async move {
            let outcome = tokio::task::spawn_blocking(move || {
                notes::sync(&library, &settings.client_id, &settings.client_secret)
            })
            .await;
            self.syncing.set(false);
            match outcome {
                Ok(outcome) => {
                    self.library.write().apply(outcome.changes);
                    self.save_local();
                    self.error.set(outcome.error);
                }
                Err(error) => self.error.set(Some(format!("The sync stopped: {error}"))),
            }
            if *self.again.peek() {
                self.again.set(false);
                self.sync(retry);
            }
        });
    }

    /// Puts a note away. An untouched new note is not worth keeping.
    fn close(mut self, id: &str, settings: CalendarSettings) {
        let blank = self
            .library
            .peek()
            .note(id)
            .is_some_and(notes::Note::is_blank);
        if blank {
            self.library.write().delete(id);
        }
        self.save_local();
        if self.library.peek().has_unsynced() {
            self.sync(settings);
        }
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

fn pastel_css(color: Pastel) -> String {
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

    let sync_settings = settings.clone();
    use_hook(move || store.sync(sync_settings));

    if let Some(id) = open().filter(|id| library.read().note(id).is_some()) {
        let leave_settings = settings.clone();
        return rsx! {
            NoteEditor {
                key: "{id}",
                id: id.clone(),
                store,
                settings: settings.clone(),
                events,
                on_follow,
                on_back: move |()| {
                    store.close(&id, leave_settings.clone());
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
            let unsynced = note.dirty;
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
                    if unsynced {
                        span { class: "note-card-state", "Not on Drive yet" }
                    }
                }
            }
        })
        .collect();
    let empty = rows.is_empty();
    let refresh_settings = settings.clone();

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
                        disabled: (store.syncing)(),
                        onclick: move |_| store.sync(refresh_settings.clone()),
                        if (store.syncing)() { "Syncing…" } else { "Sync now" }
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
        let settings = settings.clone();
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
                    store.touched(settings.clone());
                },
            }
        }
    });

    let connections = note.links.iter().map(|link| {
        let id = id.clone();
        let settings = settings.clone();
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
                        store.touched(settings.clone());
                    },
                    "Remove"
                }
            }
        }
    });

    let offers = found.into_iter().map(|link| {
        let id = id.clone();
        let settings = settings.clone();
        let label = link_label(&link);
        let key = format!("{:?}-{}", link.kind, link.id);
        rsx! {
            button {
                key: "{key}",
                class: "editor-item editor-pick",
                onclick: move |_| {
                    library.write().connect(&id, link.clone());
                    connect_query.set(String::new());
                    store.touched(settings.clone());
                },
                span { class: "editor-item-name", "{label}" }
            }
        }
    });

    let title_id = id.clone();
    let title_settings = settings.clone();
    let body_id = id.clone();
    let body_settings = settings.clone();
    let delete_id = id.clone();
    let delete_settings = settings.clone();

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
                                store.touched(title_settings.clone());
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
                                    store.touched(body_settings.clone());
                                },
                            }
                        }
                    }
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
                        p { class: "hint", "Delete this note permanently, here and on Google Drive?" }
                        div { class: "sheet-row",
                            button {
                                class: "btn danger",
                                onclick: move |_| {
                                    library.write().delete(&delete_id);
                                    store.touched(delete_settings.clone());
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
fn NoteView(source: String) -> Element {
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
pub fn LinkedNotes(target: Link, settings: CalendarSettings) -> Element {
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
                    store.touched(settings.clone());
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
