//! Todos, the notepad, brain dumps and projects: the things the desktop keeps
//! in `config.toml`, which reach the phone through the config sync.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::NaiveDate;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use timeslay_core::config::{self, Config};

/// How long typing must pause before the file is written.
const SAVE_AFTER: Duration = Duration::from_millis(800);

fn written(result: std::io::Result<()>, what: &str) -> Result<Config, String> {
    result.map_err(|error| format!("Could not save the {what}: {error}"))?;
    Ok(Config::load())
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Todos,
    Notepad,
    BrainDump,
    Projects,
}

#[component]
pub fn ScratchSheet(config: Signal<Config>, day: NaiveDate, on_close: EventHandler<()>) -> Element {
    let mut page = use_signal(|| Page::Todos);
    let tab = |label: &'static str, this: Page| {
        rsx! {
            button {
                class: "nav-tab",
                aria_pressed: (page() == this).to_string(),
                onclick: move |_| page.set(this),
                "{label}"
            }
        }
    };
    rsx! {
        div { class: "modal-backdrop editor-backdrop",
            div { class: "modal editor", role: "dialog", aria_label: "Lists",
                div { class: "modal-head",
                    div { class: "modal-titles", h2 { class: "modal-title", "Lists" } }
                    button {
                        class: "modal-close",
                        title: "Close",
                        onclick: move |_| on_close.call(()),
                        "×"
                    }
                }
                div { class: "sheet-tabs",
                    {tab("Todos", Page::Todos)}
                    {tab("Notepad", Page::Notepad)}
                    {tab("Brain dump", Page::BrainDump)}
                    {tab("Projects", Page::Projects)}
                }
                div { class: "modal-body editor-body",
                    match page() {
                        Page::Todos => rsx! { Todos { config } },
                        Page::Notepad => rsx! { Notepad { config } },
                        Page::BrainDump => rsx! { BrainDump { key: "{day}", config, day } },
                        Page::Projects => rsx! { Projects { config } },
                    }
                }
            }
        }
    }
}

#[component]
fn Todos(config: Signal<Config>) -> Element {
    let mut typed = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut change = move |edit: &dyn Fn(&mut timeslay_core::todos::TodoList)| {
        let mut todos = config.peek().todos.clone();
        edit(&mut todos);
        match written(config::save_todos(&todos), "todos") {
            Ok(saved) => {
                config.set(saved);
                error.set(None);
            }
            Err(message) => error.set(Some(message)),
        }
    };
    let todos = config.read().todos.clone();
    let current = todos.current_index();
    let count = todos.len();
    rsx! {
        if let Some(message) = error() {
            p { class: "editor-error", role: "alert", "{message}" }
        }
        p { class: "hint", "A scratchpad for breaking work into steps. It never touches Google Tasks." }
        form {
            class: "sheet-row",
            onsubmit: move |event| {
                event.prevent_default();
                let text = typed().trim().to_owned();
                if !text.is_empty() {
                    change(&|todos| { todos.add(text.clone()); });
                    typed.set(String::new());
                }
            },
            input {
                class: "client-input",
                r#type: "text",
                placeholder: "Add a step",
                aria_label: "Add a step",
                value: "{typed}",
                oninput: move |event| typed.set(event.value()),
            }
            button { class: "btn primary", r#type: "submit", "Add" }
        }
        if todos.is_empty() {
            p { class: "empty-note", "Nothing here." }
        }
        for (index, todo) in todos.items().iter().enumerate() {
            div {
                key: "{index}-{todo.text}",
                class: "todo-row",
                style: "margin-left: {u32::from(todo.indent) * 16}px;",
                button {
                    class: if current == Some(index) { "todo-text current" } else { "todo-text" },
                    aria_pressed: (current == Some(index)).to_string(),
                    onclick: move |_| change(&|todos| todos.set_current(index)),
                    "{todo.text}"
                }
                button { class: "btn icon", title: "Move up", aria_label: "Move up", disabled: index == 0,
                    onclick: move |_| change(&|todos| todos.move_item(index, -1)), "↑" }
                button { class: "btn icon", title: "Move down", aria_label: "Move down", disabled: index + 1 == count,
                    onclick: move |_| change(&|todos| todos.move_item(index, 1)), "↓" }
                button { class: "btn icon", title: "Indent", aria_label: "Indent",
                    disabled: todo.indent >= timeslay_core::todos::MAX_INDENT,
                    onclick: move |_| change(&|todos| todos.indent(index, 1)), "→" }
                button { class: "btn icon", title: "Outdent", aria_label: "Outdent", disabled: todo.indent == 0,
                    onclick: move |_| change(&|todos| todos.indent(index, -1)), "←" }
                button { class: "btn icon", title: "Remove", aria_label: "Remove",
                    onclick: move |_| change(&|todos| todos.remove(index)), "×" }
            }
        }
        if !todos.is_empty() {
            button { class: "btn", onclick: move |_| change(&|todos| todos.clear()), "Clear all" }
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
) -> Element {
    let mut text = use_signal(move || initial);
    let edits = use_hook(|| Arc::new(AtomicU64::new(0)));
    let mut error = use_signal(|| None::<String>);
    let blur_edits = edits.clone();
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
                spawn_forever(async move {
                    tokio::time::sleep(SAVE_AFTER).await;
                    if edits.load(Ordering::SeqCst) == edit {
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
                error.set(write_page(page, text.peek().clone(), config).err());
            },
        }
    }
}

#[component]
fn Notepad(config: Signal<Config>) -> Element {
    let initial = config.peek().notepad.text.clone();
    rsx! {
        SavingText { config, page: TextPage::Notepad, initial, placeholder: "The notepad" }
    }
}

#[component]
fn BrainDump(config: Signal<Config>, day: NaiveDate) -> Element {
    let initial = config
        .peek()
        .brain_dump
        .get(day)
        .unwrap_or_default()
        .to_owned();
    let earlier: Vec<(NaiveDate, String)> = config
        .read()
        .brain_dump
        .newest_first()
        .into_iter()
        .filter(|entry| entry.date != day && !entry.text.trim().is_empty())
        .map(|entry| (entry.date, entry.text.clone()))
        .collect();
    rsx! {
        p { class: "hint", "{day.format(\"%A, %B %-d\")}" }
        SavingText {
            config,
            page: TextPage::BrainDump(day),
            initial,
            placeholder: "Everything on your mind",
        }
        for (date, text) in earlier {
            div { key: "{date}", class: "editor-field",
                span { class: "editor-label", "{date.format(\"%A, %B %-d, %Y\")}" }
                p { class: "editor-value scratch-earlier", "{text}" }
            }
        }
    }
}

#[component]
fn Projects(config: Signal<Config>) -> Element {
    let mut typed = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let mut confirm = use_signal(|| None::<u64>);
    let projects = config.read().projects.clone();
    rsx! {
        if let Some(message) = error() {
            p { class: "editor-error", role: "alert", "{message}" }
        }
        form {
            class: "sheet-row",
            onsubmit: move |event| {
                event.prevent_default();
                let mut projects = config.peek().projects.clone();
                let result = projects.clean_name(&typed(), None).and_then(|name| {
                    projects.add(name);
                    written(config::save_projects(&projects), "projects")
                });
                match result {
                    Ok(saved) => {
                        config.set(saved);
                        typed.set(String::new());
                        error.set(None);
                    }
                    Err(message) => error.set(Some(message)),
                }
            },
            input {
                class: "client-input",
                r#type: "text",
                placeholder: "New project, like School/Physics",
                aria_label: "New project",
                value: "{typed}",
                oninput: move |event| typed.set(event.value()),
            }
            button { class: "btn primary", r#type: "submit", "Add" }
        }
        if projects.items.is_empty() {
            p { class: "empty-note", "No projects yet." }
        }
        for project in projects.sorted() {
            {
                let id = project.id;
                let name = project.name.clone();
                rsx! {
                    div { key: "{id}", class: "editor-item",
                        span { class: "editor-item-name", "{name}" }
                        if confirm() == Some(id) {
                            button {
                                class: "btn small danger",
                                onclick: move |_| {
                                    let mut projects = config.peek().projects.clone();
                                    projects.remove(id);
                                    match written(config::save_projects(&projects), "projects") {
                                        Ok(saved) => {
                                            config.set(saved);
                                            error.set(None);
                                        }
                                        Err(message) => error.set(Some(message)),
                                    }
                                    confirm.set(None);
                                },
                                "Remove"
                            }
                            button { class: "btn small", onclick: move |_| confirm.set(None), "Keep" }
                        } else {
                            button { class: "btn small", onclick: move |_| confirm.set(Some(id)), "Remove" }
                        }
                    }
                }
            }
        }
    }
}
