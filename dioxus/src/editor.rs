//! The sheets that edit what the day shows: tasks, events, habits, time blocks
//! and sleep.
//!
//! These offer the same fields as the iced details pane, and write through the
//! same shared functions. A task saves each field as it changes. An event stages
//! its fields until Save, apart from its location and a habit's completion.
//!
//! A sheet stays open while a write is running, so the result always has
//! somewhere to land. Closing during one waits for it to finish.

use chrono::NaiveDate;
use dioxus::prelude::*;
use timeslay_core::calendar::event_options::{
    Action as OptionAction, Availability, EventOptions, Method, Unit, Visibility,
};
use timeslay_core::calendar::{self, EditDraft, Event, SaveScope};
use timeslay_core::config::{Config, ProjectsSettings};
use timeslay_core::places::{self, Found, Place, PlaceSettings};
use timeslay_core::recurrence::{self, EndKind, MonthlyMode, RepeatMode, Rule};
use timeslay_core::task_edit::{self, Change};
use timeslay_core::timeblocking as tb;

use crate::css_color;
use crate::day::{self, AssignedTask, EditContext, HabitRow, TaskFields, TimeBlock};

const WEEKDAY_LABELS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

const NTH_LABELS: [(i8, &str); 6] = [
    (1, "First"),
    (2, "Second"),
    (3, "Third"),
    (4, "Fourth"),
    (5, "Fifth"),
    (-1, "Last"),
];

const NO_PROJECT_COLOR: &str = "#9a9a9e";

/// Runs a blocking write off the UI thread.
async fn worker<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("The save worker stopped: {error}"))
        .and_then(|result| result)
}

/// The sheet for whatever kind of item `event` is.
#[component]
pub fn Editor(
    event: Event,
    context: EditContext,
    on_close: EventHandler<()>,
    on_saved: EventHandler<()>,
    on_config: EventHandler<Config>,
) -> Element {
    if event.details.sleep.is_some() {
        rsx! { SleepEditor { event, context, on_close, on_saved } }
    } else if event.is_task() {
        rsx! { TaskEditor { event, context, on_close, on_saved, on_config } }
    } else {
        rsx! { EventEditor { event, context, on_close, on_saved, on_config } }
    }
}

/// The frame every editor shares. Tapping outside does nothing: a stray tap
/// must not throw away what has been typed.
#[component]
fn Sheet(
    title: String,
    source: String,
    color: String,
    on_close: EventHandler<()>,
    footer: Option<Element>,
    children: Element,
) -> Element {
    rsx! {
        div { class: "modal-backdrop editor-backdrop",
            div { class: "modal editor", role: "dialog", aria_label: "{title}",
                div { class: "modal-head",
                    div { class: "modal-mark", style: "background: {color};" }
                    div { class: "modal-titles",
                        h2 { class: "modal-title", "{title}" }
                        div { class: "modal-sub", "{source}" }
                    }
                    button {
                        class: "modal-close",
                        title: "Close",
                        onclick: move |_| on_close.call(()),
                        "×"
                    }
                }
                div { class: "modal-body editor-body", {children} }
                if let Some(footer) = footer {
                    div { class: "editor-footer", {footer} }
                }
            }
        }
    }
}

// ------------------------------------------------------------------- tasks

#[component]
fn TaskEditor(
    event: Event,
    context: EditContext,
    on_close: EventHandler<()>,
    on_saved: EventHandler<()>,
    on_config: EventHandler<Config>,
) -> Element {
    let initial = event.clone();
    let mut task = use_signal(move || initial);
    let seed = event.clone();
    let mut fields = use_signal(move || TaskFields::new(&seed));
    let mut busy = use_signal(|| false);
    let mut closing = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut editing_due_time = use_signal(|| false);
    let mut editing_deadline_time = use_signal(|| false);
    let mut backlog_open = use_signal(|| false);
    let mut confirm_delete = use_signal(|| false);

    let close = move |_: ()| {
        if busy() {
            closing.set(true);
        } else {
            on_close.call(());
        }
    };

    // The one way a task field is written. The returned task replaces the one
    // held here, so its possibly new id is what the next write uses.
    let write_context = context.clone();
    let apply = use_callback(move |change: Change| {
        if busy() {
            return;
        }
        busy.set(true);
        error.set(None);
        let current = task();
        let context = write_context.clone();
        spawn(async move {
            let result = worker(move || day::save_task(current, change, &context)).await;
            busy.set(false);
            match result {
                Ok(saved) => {
                    fields.set(TaskFields::new(&saved));
                    task.set(saved);
                    on_saved.call(());
                    if closing() {
                        on_close.call(());
                    }
                }
                // Stay open whatever was asked meanwhile, so the error is seen.
                Err(message) => {
                    closing.set(false);
                    error.set(Some(message));
                }
            }
        });
    });
    let mut submit = move |change: Result<Change, String>| match change {
        Ok(change) => apply.call(change),
        Err(message) => error.set(Some(message)),
    };

    let status_settings = context.settings.clone();
    let set_status = use_callback(move |status: tb::TaskStatus| {
        if busy() {
            return;
        }
        busy.set(true);
        error.set(None);
        let current = task();
        let settings = status_settings.clone();
        spawn(async move {
            let result = worker(move || {
                calendar::set_linked_task_status(
                    &settings.client_id,
                    &settings.client_secret,
                    &current,
                    status,
                )
            })
            .await;
            busy.set(false);
            match result {
                Ok(saved) => {
                    task.set(saved);
                    on_saved.call(());
                    if closing() {
                        on_close.call(());
                    }
                }
                Err(message) => {
                    closing.set(false);
                    error.set(Some(message));
                }
            }
        });
    });

    let delete_settings = context.settings.clone();
    let delete = use_callback(move |()| {
        if busy() {
            return;
        }
        busy.set(true);
        error.set(None);
        let current = task();
        let settings = delete_settings.clone();
        spawn(async move {
            let result = worker(move || {
                calendar::delete_task(
                    &settings.client_id,
                    &settings.client_secret,
                    &current,
                    settings.app_calendar.as_deref(),
                )
            })
            .await;
            busy.set(false);
            match result {
                Ok(()) => {
                    on_saved.call(());
                    on_close.call(());
                }
                Err(message) => {
                    closing.set(false);
                    confirm_delete.set(false);
                    error.set(Some(message));
                }
            }
        });
    });

    let current = task();
    let metadata = tb::metadata(&current);
    let values = fields();
    let is_busy = busy();
    let status = tb::task_status(&current);
    let source = day::source_label(&current, &context);
    let color = css_color(current.color);
    let untriaged = day::untriaged(&current, &context);
    let triage_error = task_edit::triage_error(&current, &context.settings.triage);
    let deadline_error = calendar::deadline::check(&current.details.description).err();
    let place = places::place_of(
        &context.places.places,
        &context.places.event_places,
        &current,
    )
    .and_then(|id| context.places.place(id))
    .cloned();
    let projects = metadata.projects.clone();
    let projects_to_add = projects.clone();
    let projects_to_remove = projects.clone();
    let label_task = current.clone();
    let remove_labels = metadata.labels.clone();
    let title = current.title.clone();

    rsx! {
        Sheet {
            title: "{title}",
            source,
            color,
            on_close: close,
            if let Some(message) = error() {
                p { class: "editor-error", role: "alert", "{message}" }
            }
            Field { label: "Title",
                input {
                    class: "client-input",
                    r#type: "text",
                    disabled: is_busy,
                    value: "{values.title}",
                    oninput: move |event| fields.write().title = event.value(),
                    onchange: move |_| {
                        match fields.peek().title_change() {
                            Ok(title) if title != task.peek().title => apply.call(Change::Title(title)),
                            Ok(_) => {}
                            Err(message) => error.set(Some(message)),
                        }
                    },
                }
            }
            Field { label: "Description",
                textarea {
                    class: "client-input editor-notes",
                    rows: "4",
                    placeholder: "Add description…",
                    disabled: is_busy,
                    value: "{values.description}",
                    oninput: move |event| fields.write().description = event.value(),
                    onchange: move |_| {
                        let value = fields.peek().description.clone();
                        if value != TaskFields::new(&task.peek()).description {
                            apply.call(Change::Description(value));
                        }
                    },
                }
            }
            ProjectsField {
                selected: projects,
                projects: context.projects.clone(),
                busy: is_busy,
                on_add: move |id| {
                    let mut ids = projects_to_add.clone();
                    ids.push(id);
                    ids.sort_unstable();
                    apply.call(Change::Projects(ids));
                },
                on_remove: move |id| {
                    let mut ids = projects_to_remove.clone();
                    ids.retain(|existing| *existing != id);
                    apply.call(Change::Projects(ids));
                },
            }
            Field { label: "Estimate (minutes)",
                input {
                    class: "client-input",
                    r#type: "number",
                    inputmode: "numeric",
                    min: "0",
                    max: "10080",
                    disabled: is_busy,
                    value: "{values.estimate}",
                    oninput: move |event| fields.write().estimate = event.value(),
                    onchange: move |_| {
                        match fields.peek().estimate_change() {
                            Ok(minutes) if minutes != tb::metadata(&task.peek()).minutes => {
                                apply.call(Change::Estimate(minutes));
                            }
                            Ok(_) => {}
                            Err(message) => error.set(Some(message)),
                        }
                    },
                }
            }
            Field { label: "Due Date",
                input {
                    class: "client-input",
                    r#type: "date",
                    disabled: is_busy,
                    value: "{values.date}",
                    oninput: move |event| fields.write().date = event.value(),
                }
                button {
                    class: "btn primary editor-wide",
                    disabled: is_busy,
                    onclick: move |_| submit(fields.peek().day_change()),
                    "Schedule Day"
                }
                div { class: "editor-row",
                    button {
                        class: "btn editor-grow",
                        disabled: is_busy,
                        onclick: move |_| editing_due_time.toggle(),
                        "Schedule Time"
                    }
                    button {
                        class: "btn editor-grow",
                        disabled: is_busy,
                        onclick: move |_| submit(fields.peek().day_change()),
                        "Reset Time"
                    }
                }
                if editing_due_time() {
                    div { class: "editor-row",
                        input {
                            class: "client-input",
                            r#type: "time",
                            aria_label: "Start time",
                            disabled: is_busy,
                            value: "{values.start}",
                            oninput: move |event| fields.write().start = event.value(),
                        }
                        if metadata.minutes == 0 {
                            input {
                                class: "client-input",
                                r#type: "time",
                                aria_label: "End time",
                                disabled: is_busy,
                                value: "{values.end}",
                                oninput: move |event| fields.write().end = event.value(),
                            }
                        }
                    }
                    if metadata.minutes > 0 {
                        p { class: "hint", "Lasts {day::duration_label(metadata.minutes)}, its estimate." }
                    }
                    button {
                        class: "btn primary editor-wide",
                        disabled: is_busy,
                        onclick: move |_| {
                            let estimate = tb::metadata(&task.peek()).minutes;
                            submit(fields.peek().time_change(estimate));
                        },
                        "Save time"
                    }
                }
            }
            Field { label: "Deadline",
                input {
                    class: "client-input",
                    r#type: "date",
                    disabled: is_busy,
                    value: "{values.deadline}",
                    oninput: move |event| fields.write().deadline = event.value(),
                }
                button {
                    class: "btn primary editor-wide",
                    disabled: is_busy,
                    onclick: move |_| submit(fields.peek().deadline_change(false)),
                    "Schedule Day"
                }
                div { class: "editor-row",
                    button {
                        class: "btn editor-grow",
                        disabled: is_busy,
                        onclick: move |_| editing_deadline_time.toggle(),
                        "Schedule Time"
                    }
                    button {
                        class: "btn editor-grow",
                        disabled: is_busy,
                        onclick: move |_| submit(fields.peek().deadline_change(false)),
                        "Reset Time"
                    }
                }
                if editing_deadline_time() {
                    input {
                        class: "client-input",
                        r#type: "time",
                        aria_label: "Deadline time",
                        disabled: is_busy,
                        value: "{values.deadline_time}",
                        oninput: move |event| fields.write().deadline_time = event.value(),
                    }
                    button {
                        class: "btn primary editor-wide",
                        disabled: is_busy,
                        onclick: move |_| submit(fields.peek().deadline_change(true)),
                        "Save time"
                    }
                }
                if let Some(message) = deadline_error {
                    p { class: "editor-error", "{message}" }
                }
            }
            LabelsField {
                labels: metadata.labels.clone(),
                events: context.events.clone(),
                busy: is_busy,
                on_add: move |label: String| {
                    match day::with_label(&label_task, &label) {
                        Ok(Some(labels)) => apply.call(Change::Labels(labels)),
                        Ok(None) => {}
                        Err(message) => error.set(Some(message)),
                    }
                },
                on_remove: move |label: String| {
                    let mut labels = remove_labels.clone();
                    labels.retain(|existing| !existing.eq_ignore_ascii_case(&label));
                    apply.call(Change::Labels(labels));
                },
            }
            Field { label: "Priority",
                div { class: "editor-row",
                    {
                        std::iter::once((None, "None"))
                            .chain(tb::Priority::ALL.into_iter().map(|priority| (Some(priority), priority.label())))
                            .map(|(priority, label)| {
                                let selected = priority == metadata.priority;
                                rsx! {
                                    button {
                                        key: "{label}",
                                        class: if selected { "btn primary editor-grow" } else { "btn editor-grow" },
                                        aria_pressed: selected.to_string(),
                                        disabled: is_busy,
                                        onclick: move |_| {
                                            if !selected {
                                                apply.call(Change::Priority(priority));
                                            }
                                        },
                                        "{label}"
                                    }
                                }
                            })
                    }
                }
            }
            LocationField {
                current: place,
                text: String::new(),
                places: context.places.clone(),
                busy: is_busy,
                on_error: move |message| error.set(Some(message)),
                on_pick: move |place: Option<Place>| {
                    if let Some(config) = place.as_ref().map(|_| day::load_config()) {
                        on_config.call(config);
                    }
                    apply.call(Change::Location(place.map(|place| place.id)));
                },
            }
            div { class: "editor-actions",
                if untriaged {
                    button {
                        class: "btn primary editor-wide",
                        disabled: is_busy,
                        onclick: move |_| {
                            // Nothing is sent to Google until every required field is there.
                            match triage_error.clone() {
                                Some(message) => error.set(Some(message)),
                                None => apply.call(Change::Triaged),
                            }
                        },
                        "Triage"
                    }
                }
                button {
                    class: "btn primary editor-wide",
                    disabled: is_busy,
                    onclick: move |_| {
                        set_status.call(if status == tb::TaskStatus::Completed {
                            tb::TaskStatus::Active
                        } else {
                            tb::TaskStatus::Completed
                        });
                    },
                    if status == tb::TaskStatus::Completed { "Uncomplete task" } else { "Complete task" }
                }
                button {
                    class: "btn editor-wide",
                    disabled: is_busy,
                    onclick: move |_| {
                        set_status.call(if status == tb::TaskStatus::Ignored {
                            tb::TaskStatus::Active
                        } else {
                            tb::TaskStatus::Ignored
                        });
                    },
                    if status == tb::TaskStatus::Ignored { "Unignore Task" } else { "Ignore Task" }
                }
                button {
                    class: "btn editor-wide",
                    disabled: is_busy,
                    onclick: move |_| apply.call(Change::Inbox),
                    "Move to inbox"
                }
                button {
                    class: "btn editor-wide",
                    disabled: is_busy,
                    aria_expanded: backlog_open().to_string(),
                    onclick: move |_| backlog_open.toggle(),
                    "Move to backlog"
                }
                if backlog_open() {
                    div { class: "editor-choices",
                        for section in tb::BacklogSection::ALL {
                            button {
                                key: "{section.label()}",
                                class: "btn editor-wide",
                                disabled: is_busy,
                                onclick: move |_| {
                                    backlog_open.set(false);
                                    apply.call(Change::Backlog(Some(section)));
                                },
                                "{section.label()}"
                            }
                        }
                    }
                }
                if confirm_delete() {
                    div { class: "editor-confirm", role: "alert",
                        p { "Are you sure you want to delete this task?" }
                        p { class: "hint", "This removes the task from Google Tasks and its planned time." }
                        div { class: "editor-row",
                            button {
                                class: "btn editor-grow",
                                disabled: is_busy,
                                onclick: move |_| confirm_delete.set(false),
                                "Cancel"
                            }
                            button {
                                class: "btn danger editor-grow",
                                disabled: is_busy,
                                onclick: move |_| delete.call(()),
                                "Delete"
                            }
                        }
                    }
                } else {
                    button {
                        class: "btn editor-wide",
                        disabled: is_busy,
                        onclick: move |_| confirm_delete.set(true),
                        "Delete task"
                    }
                }
            }
            {is_busy.then(|| rsx! { p { class: "hint editor-status", role: "status", "Saving…" } })}
        }
    }
}

// ------------------------------------------------------------------ events

/// Calendar events, habits and time blocks.
#[component]
fn EventEditor(
    event: Event,
    context: EditContext,
    on_close: EventHandler<()>,
    on_saved: EventHandler<()>,
    on_config: EventHandler<Config>,
) -> Element {
    let initial = event.clone();
    let mut original = use_signal(move || initial);
    let recurring = event.details.recurring_event_id.is_some();
    let seed = event.clone();
    let mut draft = use_signal(move || {
        let mut draft = EditDraft::from_event(&seed);
        // A series' occurrence does not carry its repeat rule; that comes from
        // the master, which is loaded below.
        draft.loading = recurring;
        draft
    });
    let mut busy = use_signal(|| false);
    let mut closing = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut attempt = use_signal(|| 0u32);
    let mut save_menu = use_signal(|| false);
    let mut delete_menu = use_signal(|| false);
    let mut pending_delete = use_signal(|| None::<SaveScope>);
    let mut completing = use_signal(|| false);

    let load_settings = context.settings.clone();
    let _load = use_resource(move || {
        let _ = attempt();
        let settings = load_settings.clone();
        let event = original.peek().clone();
        async move {
            if !recurring {
                return;
            }
            let result = worker(move || {
                calendar::habit_edit_draft(&settings.client_id, &settings.client_secret, &event)
            })
            .await;
            match result {
                Ok((occurrence, loaded)) => {
                    original.set(occurrence);
                    draft.set(loaded);
                    error.set(None);
                }
                Err(message) => error.set(Some(message)),
            }
        }
    });

    let close = move |_: ()| {
        if busy() || completing() {
            closing.set(true);
        } else {
            on_close.call(());
        }
    };

    let save_context = context.clone();
    let save = use_callback(move |scope: SaveScope| {
        if busy() {
            return;
        }
        save_menu.set(false);
        delete_menu.set(false);
        let current = draft();
        let source = original();
        let context = save_context.clone();
        busy.set(true);
        error.set(None);
        spawn(async move {
            let result = worker(move || day::save_event(&source, &current, scope, &context)).await;
            busy.set(false);
            match result {
                Ok(()) => {
                    on_saved.call(());
                    on_close.call(());
                }
                // The sheet stays open so the half-filled edit is not lost.
                Err(message) => {
                    closing.set(false);
                    error.set(Some(message));
                }
            }
        });
    });

    let delete_settings = context.settings.clone();
    let delete = use_callback(move |scope: SaveScope| {
        if busy() {
            return;
        }
        let source = original();
        let settings = delete_settings.clone();
        busy.set(true);
        error.set(None);
        spawn(async move {
            let result = worker(move || {
                calendar::delete_event_scope(
                    &settings.client_id,
                    &settings.client_secret,
                    &source.calendar_id,
                    &source.id,
                    scope,
                )
            })
            .await;
            busy.set(false);
            pending_delete.set(None);
            match result {
                Ok(()) => {
                    on_saved.call(());
                    on_close.call(());
                }
                Err(message) => {
                    closing.set(false);
                    error.set(Some(message));
                }
            }
        });
    });

    let place_settings = context.settings.clone();
    let set_place = use_callback(move |place: Option<Place>| {
        if busy() {
            return;
        }
        let source = original();
        let settings = place_settings.clone();
        busy.set(true);
        error.set(None);
        spawn(async move {
            let chosen = place.clone();
            let written = source.clone();
            let result =
                worker(move || day::save_event_place(&written, chosen.as_ref(), &settings)).await;
            busy.set(false);
            match result {
                Ok(()) => {
                    original.write().details.location = place
                        .as_ref()
                        .map(places::location_text)
                        .unwrap_or_default();
                    on_config.call(day::load_config());
                    on_saved.call(());
                    if closing() && !completing() {
                        on_close.call(());
                    }
                }
                Err(message) => {
                    closing.set(false);
                    error.set(Some(message));
                }
            }
        });
    });

    let complete_settings = context.settings.clone();
    let toggle_completed = use_callback(move |()| {
        if completing() {
            return;
        }
        let mut updated = original();
        updated.completed = !updated.completed;
        updated.details.habit_completed_at = updated.completed.then(timeslay_core::clock::now);
        let settings = complete_settings.clone();
        completing.set(true);
        error.set(None);
        spawn(async move {
            let written = updated.clone();
            let result = worker(move || {
                calendar::set_habit_completed(
                    &settings.client_id,
                    &settings.client_secret,
                    &written,
                )
            })
            .await;
            completing.set(false);
            match result {
                Ok(()) => {
                    original.set(updated);
                    on_saved.call(());
                    if closing() && !busy() {
                        on_close.call(());
                    }
                }
                Err(message) => {
                    closing.set(false);
                    error.set(Some(message));
                }
            }
        });
    });

    let source_event = original();
    let values = draft();
    let is_busy = busy() || values.loading;
    let habit = source_event.details.habit;
    let source = day::source_label(&source_event, &context);
    let color = css_color(source_event.color);
    let place = places::place_of(
        &context.places.places,
        &context.places.event_places,
        &source_event,
    )
    .and_then(|id| context.places.place(id))
    .cloned();
    let title = source_event.title.clone();
    let series = source_event.details.recurring_event_id.is_some();
    let anchor = values.start.date_naive();
    let organizer = source_event.details.organizer.clone();
    let attendees = source_event.details.attendees.clone();
    let location_text = source_event.details.location.clone();
    let completed = source_event.completed;

    let footer = rsx! {
        if let Some(scope) = pending_delete() {
            div { class: "editor-confirm", role: "alert",
                p {
                    match scope {
                        SaveScope::Current => "Delete this event?",
                        SaveScope::CurrentAndFuture => "Delete this and future events?",
                        SaveScope::All => "Delete all events in this series?",
                    }
                }
                p { class: "hint",
                    match scope {
                        SaveScope::Current => "This removes the event from Google Calendar.",
                        SaveScope::CurrentAndFuture => "This removes this event and future events in its series from Google Calendar.",
                        SaveScope::All => "This removes every event in this series from Google Calendar.",
                    }
                }
                div { class: "editor-row",
                    button {
                        class: "btn editor-grow",
                        disabled: is_busy,
                        onclick: move |_| pending_delete.set(None),
                        "Cancel"
                    }
                    button {
                        class: "btn danger editor-grow",
                        disabled: is_busy,
                        onclick: move |_| delete.call(scope),
                        "Delete"
                    }
                }
            }
        } else {
            div { class: "editor-row",
                button {
                    class: "btn primary editor-grow",
                    disabled: is_busy,
                    onclick: move |_| save.call(SaveScope::Current),
                    if busy() { "Saving…" } else if series { "Save current event" } else { "Save" }
                }
                if series {
                    button {
                        class: "btn primary editor-more",
                        aria_label: "More ways to save",
                        aria_expanded: save_menu().to_string(),
                        disabled: is_busy,
                        onclick: move |_| {
                            save_menu.toggle();
                            delete_menu.set(false);
                        },
                        "▾"
                    }
                }
                button {
                    class: "btn editor-grow",
                    disabled: is_busy,
                    onclick: move |_| {
                        if series {
                            delete_menu.toggle();
                            save_menu.set(false);
                        } else {
                            pending_delete.set(Some(SaveScope::Current));
                        }
                    },
                    "Delete"
                }
            }
            if save_menu() {
                div { class: "editor-choices",
                    button {
                        class: "btn editor-wide",
                        disabled: is_busy,
                        onclick: move |_| save.call(SaveScope::CurrentAndFuture),
                        "Save current and future events"
                    }
                    button {
                        class: "btn editor-wide",
                        disabled: is_busy,
                        onclick: move |_| save.call(SaveScope::All),
                        "Save all events"
                    }
                }
            }
            if delete_menu() {
                div { class: "editor-choices",
                    for (label, scope) in [
                        ("Delete current event", SaveScope::Current),
                        ("Delete current and future events", SaveScope::CurrentAndFuture),
                        ("Delete all events", SaveScope::All),
                    ] {
                        button {
                            key: "{label}",
                            class: "btn editor-wide",
                            disabled: is_busy,
                            onclick: move |_| {
                                delete_menu.set(false);
                                pending_delete.set(Some(scope));
                            },
                            "{label}"
                        }
                    }
                }
            }
        }
    };

    rsx! {
        Sheet {
            title: "{title}",
            source,
            color,
            on_close: close,
            footer,
            if let Some(message) = error() {
                p { class: "editor-error", role: "alert", "{message}" }
                if values.loading {
                    button {
                        class: "btn editor-wide",
                        onclick: move |_| attempt += 1,
                        "Retry"
                    }
                }
            } else if values.loading {
                p { class: "hint", role: "status", "Loading the repeat schedule…" }
            }
            Field { label: "Title",
                input {
                    class: "client-input",
                    r#type: "text",
                    disabled: is_busy,
                    value: "{values.title}",
                    oninput: move |event| draft.write().title = event.value(),
                }
            }
            Field { label: "Description",
                textarea {
                    class: "client-input editor-notes",
                    rows: "4",
                    placeholder: "Add description…",
                    disabled: is_busy,
                    value: "{values.description}",
                    oninput: move |event| draft.write().description = event.value(),
                }
            }
            if habit {
                Field { label: "Completion",
                    button {
                        class: if completed { "btn editor-wide" } else { "btn primary editor-wide" },
                        disabled: completing(),
                        onclick: move |_| toggle_completed.call(()),
                        if completing() {
                            "Saving…"
                        } else if completed {
                            "Mark as not done"
                        } else {
                            "Mark as done"
                        }
                    }
                }
            }
            ProjectsField {
                selected: values.projects.ids.clone(),
                projects: context.projects.clone(),
                busy: is_busy,
                on_add: move |id| draft.write().projects.update(calendar::projects::Action::Add(id)),
                on_remove: move |id| draft.write().projects.update(calendar::projects::Action::Remove(id)),
            }
            Field { label: if habit { "Start day" } else { "Start Time" },
                div { class: "editor-row",
                    input {
                        class: "client-input",
                        r#type: "date",
                        aria_label: "Start date",
                        disabled: is_busy,
                        value: "{values.date_text}",
                        oninput: move |event| set_start_date(&mut draft.write(), event.value(), habit),
                    }
                    input {
                        class: "client-input",
                        r#type: "time",
                        aria_label: "Start time",
                        disabled: is_busy,
                        value: "{values.start_text}",
                        oninput: move |event| set_start_time(&mut draft.write(), event.value(), habit),
                    }
                }
            }
            if !habit {
                Field { label: "End Time",
                    div { class: "editor-row",
                        input {
                            class: "client-input",
                            r#type: "date",
                            aria_label: "End date",
                            disabled: is_busy,
                            value: "{values.end_date_text}",
                            oninput: move |event| draft.write().set_end_date_text(event.value()),
                        }
                        input {
                            class: "client-input",
                            r#type: "time",
                            aria_label: "End time",
                            disabled: is_busy,
                            value: "{values.end_text}",
                            oninput: move |event| {
                                let mut draft = draft.write();
                                draft.set_end_text(event.value());
                                draft.all_day = false;
                            },
                        }
                    }
                }
            }
            RepeatEditor {
                rule: values.rule.clone(),
                anchor,
                busy: is_busy,
                on_action: move |action: recurrence::Action| {
                    let mut draft = draft.write();
                    draft.recurrence_changed = true;
                    if matches!(action, recurrence::Action::SkipAdd) {
                        match draft.rule.add_skip() {
                            Ok(()) => error.set(None),
                            Err(message) => error.set(Some(message)),
                        }
                    } else {
                        let anchor = draft.start.date_naive();
                        draft.rule.apply_action(action, anchor);
                    }
                },
            }
            if !habit {
                LabelsField {
                    labels: values.labels.clone(),
                    events: context.events.clone(),
                    busy: is_busy,
                    on_add: move |label: String| {
                        let label = label.trim().trim_start_matches('#').to_owned();
                        let mut draft = draft.write();
                        if !label.is_empty() && !draft.labels.contains(&label) {
                            draft.labels.push(label);
                        }
                    },
                    on_remove: move |label: String| draft.write().labels.retain(|existing| existing != &label),
                }
            }
            LocationField {
                current: place,
                text: location_text,
                places: context.places.clone(),
                busy: is_busy,
                on_error: move |message| error.set(Some(message)),
                on_pick: move |place| set_place.call(place),
            }
            OptionsEditor {
                options: values.options.clone(),
                busy: is_busy,
                on_action: move |action| draft.write().options.update(action),
            }
            if !organizer.is_empty() {
                Field { label: "Organizer", p { class: "editor-value", "{organizer}" } }
            }
            if !attendees.is_empty() {
                Field { label: "Guests",
                    for guest in attendees {
                        p { key: "{guest}", class: "editor-value", "{guest}" }
                    }
                }
            }
        }
    }
}

/// A habit's end follows its start, so its length stays as it was.
fn set_start_date(draft: &mut EditDraft, text: String, habit: bool) {
    let duration = draft.end - draft.start;
    draft.set_date_text(text);
    if habit {
        draft.end = draft.start + duration;
        draft.end_date_text = draft.end.date_naive().to_string();
    }
}

fn set_start_time(draft: &mut EditDraft, text: String, habit: bool) {
    let duration = draft.end - draft.start;
    draft.set_start_text(text);
    draft.all_day = false;
    if habit {
        draft.end = draft.start + duration;
        draft.end_text = calendar::clock_label(draft.end);
        draft.end_date_text = draft.end.date_naive().to_string();
    }
}

// ------------------------------------------------------------------- sleep

/// A night has just two timestamps and independent confidence flags.
#[component]
fn SleepEditor(
    event: Event,
    context: EditContext,
    on_close: EventHandler<()>,
    on_saved: EventHandler<()>,
) -> Element {
    let seed = event.clone();
    let mut draft = use_signal(move || EditDraft::from_event(&seed));
    let mut busy = use_signal(|| false);
    let mut closing = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    let close = move |_: ()| {
        if busy() {
            closing.set(true);
        } else {
            on_close.call(());
        }
    };

    let original = event.clone();
    let settings = context.settings.clone();
    let save = use_callback(move |()| {
        if busy() {
            return;
        }
        let values = draft();
        let Some(mut record) = values.sleep.clone() else {
            return;
        };
        if calendar::parse_clock(&values.start_text).is_none()
            || calendar::parse_clock(&values.end_text).is_none()
            || calendar::deadline::parse_date(&values.date_text).is_err()
            || calendar::deadline::parse_date(&values.end_date_text).is_err()
            || values.end <= values.start
        {
            error.set(Some(
                "Enter valid dates and times, with wake-up after sleep start".to_owned(),
            ));
            return;
        }
        let mut night = original.clone();
        let new = night.id == calendar::sleep::NEW_ID;
        record.automatic = false;
        if new {
            record.wake_date = values.end.date_naive();
        }
        night.start = values.start;
        night.end = values.end;
        night.details.sleep = Some(record);
        let settings = settings.clone();
        busy.set(true);
        error.set(None);
        spawn(async move {
            let result = worker(move || calendar::sleep::save(&settings, night, new)).await;
            busy.set(false);
            match result {
                Ok(_) => {
                    on_saved.call(());
                    on_close.call(());
                }
                Err(message) => {
                    closing.set(false);
                    error.set(Some(message));
                }
            }
        });
    });

    let values = draft();
    let is_busy = busy();
    let Some(record) = values.sleep.clone() else {
        return rsx! {};
    };
    let mut preview = event.clone();
    preview.start = values.start;
    preview.end = values.end;
    let summary = format!(
        "{} · {}",
        calendar::sleep::duration(&preview),
        calendar::event_time_label(&preview)
    );
    let source = day::source_label(&event, &context);
    let color = css_color(event.color);
    let footer = rsx! {
        button {
            class: "btn primary editor-wide",
            disabled: is_busy,
            onclick: move |_| save.call(()),
            if is_busy { "Saving…" } else { "Save sleep" }
        }
    };

    rsx! {
        Sheet {
            title: "Sleep",
            source,
            color,
            on_close: close,
            footer,
            if let Some(message) = error() {
                p { class: "editor-error", role: "alert", "{message}" }
            }
            p { class: "editor-value", "{summary}" }
            Field { label: "Sleep start",
                div { class: "editor-row",
                    input {
                        class: "client-input",
                        r#type: "date",
                        aria_label: "Sleep start date",
                        disabled: is_busy,
                        value: "{values.date_text}",
                        oninput: move |event| draft.write().set_date_text(event.value()),
                    }
                    input {
                        class: "client-input",
                        r#type: "time",
                        aria_label: "Sleep start time",
                        disabled: is_busy,
                        value: "{values.start_text}",
                        oninput: move |event| draft.write().set_start_text(event.value()),
                    }
                }
                label { class: "editor-check",
                    input {
                        r#type: "checkbox",
                        disabled: is_busy,
                        checked: record.estimated_start,
                        onchange: move |event| {
                            if let Some(record) = draft.write().sleep.as_mut() {
                                record.estimated_start = event.checked();
                            }
                        },
                    }
                    "Estimated Sleep Start"
                }
            }
            Field { label: "Wake-up time",
                div { class: "editor-row",
                    input {
                        class: "client-input",
                        r#type: "date",
                        aria_label: "Wake-up date",
                        disabled: is_busy,
                        value: "{values.end_date_text}",
                        oninput: move |event| draft.write().set_end_date_text(event.value()),
                    }
                    input {
                        class: "client-input",
                        r#type: "time",
                        aria_label: "Wake-up time",
                        disabled: is_busy,
                        value: "{values.end_text}",
                        oninput: move |event| draft.write().set_end_text(event.value()),
                    }
                }
                label { class: "editor-check",
                    input {
                        r#type: "checkbox",
                        disabled: is_busy,
                        checked: record.estimated_end,
                        onchange: move |event| {
                            if let Some(record) = draft.write().sleep.as_mut() {
                                record.estimated_end = event.checked();
                            }
                        },
                    }
                    "Estimated Sleep End"
                }
            }
            p { class: "hint",
                if record.automatic {
                    "Calculated from the first timed event. Saving records your times and preserves them when your schedule changes."
                } else {
                    "Recorded sleep. These times are preserved when your schedule changes."
                }
            }
        }
    }
}

// ------------------------------------------------------------- time blocks

/// A block's assigned tasks, with ways to open, remove and add them.
#[component]
pub fn BlockTasks(
    block: TimeBlock,
    context: EditContext,
    /// Owned by the app, which keeps the popup open while a write is running.
    busy: Signal<bool>,
    on_open: EventHandler<Event>,
    on_saved: EventHandler<()>,
) -> Element {
    let mut busy = busy;
    let mut adding = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    let write_context = context.clone();
    let apply = use_callback(move |(task, change): (Event, Change)| {
        if busy() {
            return;
        }
        busy.set(true);
        error.set(None);
        let context = write_context.clone();
        spawn(async move {
            let result = worker(move || day::save_task(task, change, &context)).await;
            busy.set(false);
            match result {
                Ok(_) => on_saved.call(()),
                Err(message) => error.set(Some(message)),
            }
        });
    });

    let is_busy = busy();
    let key = tb::block_key(&block.event);
    let (start, end) = (block.event.start, block.event.end);
    let assignable = day::assignable_tasks(&context.events);

    rsx! {
        if let Some(message) = error() {
            p { class: "editor-error", role: "alert", "{message}" }
        }
        if block.tasks.is_empty() {
            div { class: "modal-empty",
                p { "No tasks are assigned to this time block yet." }
                span { "Add one below." }
            }
        } else {
            div { class: "assigned-list",
                for task in block.tasks.iter().cloned() {
                    AssignedRow {
                        key: "{task.event.id}",
                        task,
                        busy: is_busy,
                        on_open,
                        on_remove: move |event: Event| apply.call((event, Change::Unassign)),
                    }
                }
            }
        }
        button {
            class: "btn editor-wide",
            aria_expanded: adding().to_string(),
            disabled: is_busy,
            onclick: move |_| adding.toggle(),
            if adding() { "Done" } else { "+ Add task" }
        }
        if adding() {
            if assignable.is_empty() {
                p { class: "empty-note", "Every open task of this day already has a block." }
            }
            div { class: "editor-choices",
                for task in assignable {
                    {
                        let chosen = task.clone();
                        let key = key.clone();
                        rsx! {
                            button {
                                key: "{task.id}",
                                class: "btn editor-wide editor-choice",
                                disabled: is_busy,
                                onclick: move |_| {
                                    apply.call((chosen.clone(), Change::Block(key.clone(), start, end)));
                                },
                                "{task.title}"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn AssignedRow(
    task: AssignedTask,
    busy: bool,
    on_open: EventHandler<Event>,
    on_remove: EventHandler<Event>,
) -> Element {
    let status_class = match task.status {
        tb::TaskStatus::Active => "active",
        tb::TaskStatus::Completed => "done",
        tb::TaskStatus::Ignored => "ignored",
    };
    let title_class = match task.status {
        tb::TaskStatus::Active => "task-title",
        tb::TaskStatus::Completed => "task-title done",
        tb::TaskStatus::Ignored => "task-title ignored",
    };
    let status_note = match task.status {
        tb::TaskStatus::Active => "Open",
        tb::TaskStatus::Completed => "Completed",
        tb::TaskStatus::Ignored => "Ignored",
    };
    let opened = task.event.clone();
    let removed = task.event.clone();
    let title = task.event.title.clone();
    let minutes = day::duration_label(task.minutes);

    rsx! {
        div { class: "assigned-task",
            button {
                class: "assigned-open",
                title: "Edit this task",
                onclick: move |_| on_open.call(opened.clone()),
                span { class: "task-status {status_class}", "{crate::status_icon(task.status)}" }
                div { class: "task-main",
                    div { class: "{title_class}", "{title}" }
                    div { class: "task-meta",
                        span { class: "meta-pill", "{task.list_name}" }
                        span { class: "meta-pill", "{status_note}" }
                    }
                }
                span { class: "assigned-minutes", "{minutes}" }
            }
            button {
                class: "modal-close",
                title: "Remove from this block",
                aria_label: "Remove {title} from this block",
                disabled: busy,
                onclick: move |_| on_remove.call(removed.clone()),
                "−"
            }
        }
    }
}

// ------------------------------------------------------------------ habits

/// Every habit, so one can be edited whichever day it next falls on.
#[component]
pub fn HabitsSheet(
    context: EditContext,
    /// Bumped by every write, so a change shows here when its editor closes.
    revision: Signal<u32>,
    on_open: EventHandler<Event>,
    on_close: EventHandler<()>,
) -> Element {
    let settings = context.settings.clone();
    let habits = use_resource(move || {
        let _ = revision();
        let settings = settings.clone();
        async move { worker(move || day::load_habits(&settings)).await }
    });

    let body = match &*habits.read() {
        None => rsx! { p { class: "hint", role: "status", "Loading habits…" } },
        Some(Err(message)) => rsx! { p { class: "editor-error", role: "alert", "{message}" } },
        Some(Ok(rows)) => rsx! { HabitList { rows: rows.clone(), on_open } },
    };

    rsx! {
        div { class: "modal-backdrop editor-backdrop",
            div { class: "modal editor", role: "dialog", aria_label: "Habits",
                div { class: "modal-head",
                    div { class: "modal-titles",
                        h2 { class: "modal-title", "Habits" }
                    }
                    button {
                        class: "modal-close",
                        title: "Close",
                        onclick: move |_| on_close.call(()),
                        "×"
                    }
                }
                div { class: "modal-body editor-body", {body} }
            }
        }
    }
}

#[component]
fn HabitList(rows: Vec<HabitRow>, on_open: EventHandler<Event>) -> Element {
    rsx! {
        if rows.is_empty() {
            p { class: "empty-note", "No habits yet." }
        }
        for row in rows {
            {
                let dot = css_color(row.color);
                let event = row.event.clone();
                rsx! {
                    button {
                        key: "{row.event.id}",
                        class: "editor-item editor-pick",
                        onclick: move |_| on_open.call(event.clone()),
                        span { class: "day-dot", style: "background: {dot};" }
                        span { class: "editor-item-name",
                            "{row.title}"
                            span { class: "hint editor-item-sub", "{row.schedule} · {row.time}" }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------- shared controls

#[component]
fn Field(label: String, children: Element) -> Element {
    rsx! {
        div { class: "editor-field",
            span { class: "editor-label", "{label}" }
            {children}
        }
    }
}

/// The add/search/remove flow for an item's projects.
#[component]
fn ProjectsField(
    selected: Vec<u64>,
    projects: ProjectsSettings,
    busy: bool,
    on_add: EventHandler<u64>,
    on_remove: EventHandler<u64>,
) -> Element {
    let mut open = use_signal(|| false);
    let mut query = use_signal(String::new);

    let color = |project: Option<&timeslay_core::config::Project>| {
        project
            .and_then(|project| project.color())
            .map_or_else(|| NO_PROJECT_COLOR.to_owned(), css_color)
    };
    let needle = query().trim().to_lowercase();
    let available: Vec<_> = projects
        .sorted()
        .into_iter()
        .filter(|project| !selected.contains(&project.id))
        .filter(|project| project.name.to_lowercase().contains(&needle))
        .map(|project| (project.id, project.name.clone(), color(Some(project))))
        .collect();
    let none_message = if projects.items.is_empty() {
        "Create a project in the desktop app first"
    } else if needle.is_empty() {
        "Every project is already added"
    } else {
        "No matching projects"
    };

    rsx! {
        Field { label: "Projects",
            if selected.is_empty() {
                p { class: "empty-note", "No projects" }
            }
            for id in selected.iter().copied() {
                {
                    let project = projects.get(id);
                    let name = project.map_or_else(|| format!("Removed project ({id})"), |project| project.name.clone());
                    let dot = color(project);
                    rsx! {
                        div { key: "{id}", class: "editor-item",
                            span { class: "day-dot", style: "background: {dot};" }
                            span { class: "editor-item-name", "{name}" }
                            button {
                                class: "modal-close",
                                aria_label: "Remove {name}",
                                disabled: busy,
                                onclick: move |_| on_remove.call(id),
                                "×"
                            }
                        }
                    }
                }
            }
            button {
                class: "btn editor-wide",
                aria_expanded: open().to_string(),
                disabled: busy,
                onclick: move |_| {
                    open.toggle();
                    query.set(String::new());
                },
                if open() { "Done" } else { "+ Add project" }
            }
            if open() {
                input {
                    class: "client-input",
                    r#type: "search",
                    placeholder: "Search projects…",
                    value: "{query}",
                    oninput: move |event| query.set(event.value()),
                }
                if available.is_empty() {
                    p { class: "empty-note", "{none_message}" }
                }
                for (id, name, dot) in available {
                    button {
                        key: "{id}",
                        class: "editor-item editor-pick",
                        disabled: busy,
                        onclick: move |_| {
                            query.set(String::new());
                            on_add.call(id);
                        },
                        span { class: "day-dot", style: "background: {dot};" }
                        span { class: "editor-item-name", "{name}" }
                    }
                }
            }
        }
    }
}

/// Free-form labels, a field to add another, and ones already used elsewhere.
#[component]
fn LabelsField(
    labels: Vec<String>,
    events: Vec<Event>,
    busy: bool,
    on_add: EventHandler<String>,
    on_remove: EventHandler<String>,
) -> Element {
    let mut typed = use_signal(String::new);
    let suggestions = day::label_suggestions(&events, &labels, &typed());

    rsx! {
        Field { label: "Labels",
            for label in labels {
                {
                    let removed = label.clone();
                    rsx! {
                        div { key: "{label}", class: "editor-item",
                            span { class: "editor-item-name", "{label}" }
                            button {
                                class: "modal-close",
                                aria_label: "Remove {label}",
                                disabled: busy,
                                onclick: move |_| on_remove.call(removed.clone()),
                                "×"
                            }
                        }
                    }
                }
            }
            form {
                class: "editor-row",
                onsubmit: move |event| {
                    event.prevent_default();
                    let label = typed();
                    typed.set(String::new());
                    on_add.call(label);
                },
                input {
                    class: "client-input",
                    r#type: "text",
                    placeholder: "Add a label…",
                    disabled: busy,
                    value: "{typed}",
                    oninput: move |event| typed.set(event.value()),
                }
                button { class: "btn primary", r#type: "submit", disabled: busy, "Add" }
            }
            for label in suggestions {
                {
                    let chosen = label.clone();
                    rsx! {
                        button {
                            key: "{label}",
                            class: "btn editor-wide editor-choice",
                            disabled: busy,
                            onclick: move |_| {
                                typed.set(String::new());
                                on_add.call(chosen.clone());
                            },
                            "+ {label}"
                        }
                    }
                }
            }
        }
    }
}

/// Where something happens: a saved place, or a Mapbox search for a new one.
#[component]
fn LocationField(
    current: Option<Place>,
    /// Location text from Google that matches no saved place.
    text: String,
    places: PlaceSettings,
    busy: bool,
    on_pick: EventHandler<Option<Place>>,
    on_error: EventHandler<String>,
) -> Element {
    let mut open = use_signal(|| false);
    let mut query = use_signal(String::new);
    let mut searching = use_signal(|| false);
    let mut results = use_signal(Vec::<Found>::new);
    let mut searched = use_signal(|| false);

    let search_places_settings = places.clone();
    let mut search = move |_| {
        let term = query().trim().to_owned();
        if term.is_empty() || searching() {
            return;
        }
        let Some(key) = search_places_settings.key() else {
            on_error
                .call("Add a Mapbox access token in the desktop app's Settings first.".to_owned());
            return;
        };
        let proximity = search_places_settings.proximity();
        searching.set(true);
        spawn(async move {
            let result = worker(move || places::search_places(&key, &term, proximity)).await;
            searching.set(false);
            searched.set(true);
            match result {
                Ok(found) => results.set(found),
                Err(message) => {
                    results.set(Vec::new());
                    on_error.call(message);
                }
            }
        });
    };
    let mut close_picker = move || {
        open.set(false);
        query.set(String::new());
        results.set(Vec::new());
        searched.set(false);
    };

    let needle = query().trim().to_lowercase();
    let saved: Vec<Place> = places
        .sorted()
        .into_iter()
        .filter(|place| {
            place.name.to_lowercase().contains(&needle)
                || place.address.to_lowercase().contains(&needle)
        })
        .cloned()
        .collect();
    let has_current = current.is_some();

    rsx! {
        Field { label: "Location",
            if let Some(place) = current {
                p { class: "editor-value", "{place.name}" }
                if !place.address.is_empty() && place.address != place.name {
                    p { class: "hint", "{place.address}" }
                }
            } else if !text.is_empty() {
                p { class: "editor-value", "{text}" }
            }
            if open() {
                form {
                    class: "editor-row",
                    onsubmit: move |event| {
                        event.prevent_default();
                        search(());
                    },
                    input {
                        class: "client-input",
                        r#type: "search",
                        placeholder: "Search for a place",
                        value: "{query}",
                        oninput: move |event| query.set(event.value()),
                    }
                    button {
                        class: "btn primary",
                        r#type: "submit",
                        disabled: searching() || query().trim().is_empty(),
                        if searching() { "Searching…" } else { "Search" }
                    }
                }
                for found in results() {
                    {
                        let chosen = found.clone();
                        rsx! {
                            button {
                                key: "{found.place_id}",
                                class: "editor-item editor-pick",
                                disabled: busy,
                                onclick: move |_| {
                                    match day::save_found_place(&chosen) {
                                        Ok(place) => {
                                            close_picker();
                                            on_pick.call(Some(place));
                                        }
                                        Err(message) => on_error.call(message),
                                    }
                                },
                                span { class: "editor-item-name",
                                    "{found.name}"
                                    span { class: "hint editor-item-sub", "{found.address}" }
                                }
                            }
                        }
                    }
                }
                if searched() && results().is_empty() {
                    p { class: "empty-note", "No places found." }
                }
                if !saved.is_empty() {
                    span { class: "editor-label", "Saved places" }
                }
                for place in saved {
                    {
                        let chosen = place.clone();
                        rsx! {
                            button {
                                key: "{place.id}",
                                class: "editor-item editor-pick",
                                disabled: busy,
                                onclick: move |_| {
                                    close_picker();
                                    on_pick.call(Some(chosen.clone()));
                                },
                                span { class: "editor-item-name",
                                    "{place.name}"
                                    span { class: "hint editor-item-sub", "{place.address}" }
                                }
                            }
                        }
                    }
                }
                button {
                    class: "btn editor-wide",
                    onclick: move |_| close_picker(),
                    "Cancel"
                }
            } else {
                div { class: "editor-row",
                    button {
                        class: "btn editor-grow",
                        disabled: busy,
                        onclick: move |_| open.set(true),
                        if has_current || !text.is_empty() { "Change" } else { "Add location" }
                    }
                    if has_current || !text.is_empty() {
                        button {
                            class: "btn editor-grow",
                            disabled: busy,
                            onclick: move |_| on_pick.call(None),
                            "Clear"
                        }
                    }
                }
            }
        }
    }
}

/// A small row of choices where the current one looks selected.
#[component]
fn Choice(label: String, selected: bool, busy: bool, onclick: EventHandler<()>) -> Element {
    rsx! {
        button {
            class: if selected { "btn small primary" } else { "btn small" },
            aria_pressed: selected.to_string(),
            disabled: busy,
            onclick: move |_| onclick.call(()),
            "{label}"
        }
    }
}

/// The repeat controls: the same words and choices as the iced editors.
#[component]
fn RepeatEditor(
    rule: Rule,
    anchor: NaiveDate,
    busy: bool,
    on_action: EventHandler<recurrence::Action>,
) -> Element {
    use recurrence::Action;
    let mode = rule.repeat;
    let summary = rule.summary(anchor);

    rsx! {
        Field { label: "Repeat",
            div { class: "editor-wrap",
                for choice in RepeatMode::SIMPLE {
                    Choice {
                        key: "{choice.label()}",
                        label: choice.label(),
                        selected: mode == choice,
                        busy,
                        onclick: move |_| on_action.call(Action::Repeat(choice)),
                    }
                }
            }
            p { class: "hint", "{summary}" }
            if mode == RepeatMode::Custom {
                p { class: "hint",
                    "This item has a custom schedule that the simple editor cannot show. You can keep it as it is, or replace it with a simple schedule."
                }
                button {
                    class: "btn editor-wide",
                    disabled: busy,
                    onclick: move |_| on_action.call(Action::SimpleRepeat),
                    "Use a simple schedule"
                }
            } else if mode != RepeatMode::Never {
                if mode != RepeatMode::Weekdays {
                    div { class: "editor-inline",
                        "Every"
                        input {
                            class: "client-input editor-short",
                            r#type: "number",
                            inputmode: "numeric",
                            min: "1",
                            aria_label: "Repeat interval",
                            disabled: busy,
                            value: "{rule.interval}",
                            oninput: move |event| on_action.call(Action::Interval(event.value())),
                        }
                        "{mode.unit()}"
                    }
                }
                if mode == RepeatMode::Weekly {
                    span { class: "editor-label", "On" }
                    div { class: "editor-wrap",
                        for (index, day) in WEEKDAY_LABELS.iter().enumerate() {
                            Choice {
                                key: "{day}",
                                label: *day,
                                selected: rule.weekdays.get(index).copied().unwrap_or(false),
                                busy,
                                onclick: move |_| on_action.call(Action::ToggleWeekday(index)),
                            }
                        }
                    }
                }
                if mode == RepeatMode::Monthly {
                    div { class: "editor-wrap",
                        for (label, choice) in [
                            ("Day of the month", MonthlyMode::Day),
                            ("Weekday", MonthlyMode::NthWeekday),
                            ("Last day", MonthlyMode::LastDay),
                        ] {
                            Choice {
                                key: "{label}",
                                label,
                                selected: rule.monthly == choice,
                                busy,
                                onclick: move |_| on_action.call(Action::MonthlyMode(choice)),
                            }
                        }
                    }
                    match rule.monthly {
                        MonthlyMode::Day => rsx! {
                            div { class: "editor-inline",
                                "On day"
                                input {
                                    class: "client-input editor-short",
                                    r#type: "number",
                                    inputmode: "numeric",
                                    min: "1",
                                    max: "31",
                                    aria_label: "Day of the month",
                                    disabled: busy,
                                    value: "{rule.monthly_day}",
                                    oninput: move |event| on_action.call(Action::MonthlyDay(event.value())),
                                }
                                "of the month"
                            }
                        },
                        MonthlyMode::NthWeekday => rsx! {
                            div { class: "editor-wrap",
                                for (nth, name) in NTH_LABELS {
                                    Choice {
                                        key: "{name}",
                                        label: name,
                                        selected: rule.monthly_nth == nth,
                                        busy,
                                        onclick: move |_| on_action.call(Action::MonthlyNth(nth)),
                                    }
                                }
                            }
                            div { class: "editor-wrap",
                                for (index, day) in WEEKDAY_LABELS.iter().enumerate() {
                                    Choice {
                                        key: "{day}",
                                        label: *day,
                                        selected: recurrence::WEEKDAYS[index] == rule.monthly_weekday,
                                        busy,
                                        onclick: move |_| on_action.call(Action::MonthlyWeekday(recurrence::WEEKDAYS[index])),
                                    }
                                }
                            }
                        },
                        MonthlyMode::LastDay => rsx! {
                            p { class: "hint", "On the last day of each month." }
                        },
                    }
                }
                if mode == RepeatMode::Yearly {
                    p { class: "hint", "Once a year on the day it starts." }
                }
                span { class: "editor-label", "Ends" }
                div { class: "editor-wrap",
                    for (label, choice) in [
                        ("Never", EndKind::Never),
                        ("On date", EndKind::OnDate),
                        ("After", EndKind::After),
                    ] {
                        Choice {
                            key: "{label}",
                            label,
                            selected: rule.ends == choice,
                            busy,
                            onclick: move |_| on_action.call(Action::Ends(choice)),
                        }
                    }
                }
                if rule.ends == EndKind::OnDate {
                    div { class: "editor-inline",
                        "Last day"
                        input {
                            class: "client-input",
                            r#type: "date",
                            aria_label: "Last day",
                            disabled: busy,
                            value: "{rule.until}",
                            oninput: move |event| on_action.call(Action::Until(event.value())),
                        }
                    }
                }
                if rule.ends == EndKind::After {
                    div { class: "editor-inline",
                        "For"
                        input {
                            class: "client-input editor-short",
                            r#type: "number",
                            inputmode: "numeric",
                            min: "1",
                            aria_label: "Number of repeats",
                            disabled: busy,
                            value: "{rule.count}",
                            oninput: move |event| on_action.call(Action::Count(event.value())),
                        }
                        "repeats"
                    }
                }
                span { class: "editor-label", "Skip dates" }
                if rule.skips.is_empty() {
                    p { class: "empty-note", "No skipped dates." }
                }
                for skip in rule.skips.iter().copied() {
                    div { key: "{skip}", class: "editor-item",
                        span { class: "editor-item-name", "{skip}" }
                        button {
                            class: "modal-close",
                            aria_label: "Stop skipping {skip}",
                            disabled: busy,
                            onclick: move |_| on_action.call(Action::SkipRemove(skip)),
                            "×"
                        }
                    }
                }
                div { class: "editor-row",
                    input {
                        class: "client-input",
                        r#type: "date",
                        aria_label: "Date to skip",
                        disabled: busy,
                        value: "{rule.skip_input}",
                        oninput: move |event| on_action.call(Action::SkipInput(event.value())),
                    }
                    button {
                        class: "btn",
                        disabled: busy,
                        onclick: move |_| on_action.call(Action::SkipAdd),
                        "Skip"
                    }
                }
            }
        }
    }
}

/// Google-style notification rows followed by availability and visibility.
#[component]
fn OptionsEditor(
    options: EventOptions,
    busy: bool,
    on_action: EventHandler<OptionAction>,
) -> Element {
    let use_default = options.reminders.use_default;
    let overrides = options.reminders.overrides.clone();
    let can_add = overrides.len() < 5;
    // Confidential is only kept, not offered, the same as in the iced app.
    let visibilities: Vec<Visibility> =
        [Visibility::Default, Visibility::Public, Visibility::Private]
            .into_iter()
            .chain(
                (options.visibility == Visibility::Confidential)
                    .then_some(Visibility::Confidential),
            )
            .collect();

    rsx! {
        Field { label: "Notifications",
            label { class: "editor-check",
                input {
                    r#type: "checkbox",
                    disabled: busy,
                    checked: use_default,
                    onchange: move |event| on_action.call(OptionAction::Defaults(event.checked())),
                }
                "Use calendar default notifications"
            }
            if use_default {
                for (index, reminder) in options.default_reminders.iter().enumerate() {
                    p { key: "{index}", class: "hint", "{reminder.method} · {reminder.minutes} minutes before" }
                }
            } else if overrides.is_empty() {
                p { class: "empty-note", "No notifications" }
            }
            if !use_default {
                for (index, reminder) in overrides.into_iter().enumerate() {
                    div { key: "{index}", class: "editor-reminder",
                        div { class: "editor-row",
                            select {
                                class: "client-input",
                                aria_label: "Notification method",
                                disabled: busy,
                                onchange: move |event| {
                                    let method = if event.value() == "Email" { Method::Email } else { Method::Notification };
                                    on_action.call(OptionAction::Method(index, method));
                                },
                                for method in [Method::Notification, Method::Email] {
                                    option { selected: reminder.method == method, value: "{method}", "{method}" }
                                }
                            }
                            button {
                                class: "modal-close",
                                aria_label: "Remove notification",
                                disabled: busy,
                                onclick: move |_| on_action.call(OptionAction::Remove(index)),
                                "×"
                            }
                        }
                        div { class: "editor-inline",
                            input {
                                class: "client-input editor-short",
                                r#type: "number",
                                inputmode: "numeric",
                                min: "0",
                                aria_label: "Notification time",
                                disabled: busy,
                                value: reminder.input.clone().unwrap_or_else(|| (reminder.minutes / reminder.unit.factor()).to_string()),
                                oninput: move |event| on_action.call(OptionAction::Minutes(index, event.value())),
                            }
                            select {
                                class: "client-input",
                                aria_label: "Notification unit",
                                disabled: busy,
                                onchange: move |event| {
                                    let unit = match event.value().as_str() {
                                        "hours" => Unit::Hours,
                                        "days" => Unit::Days,
                                        "weeks" => Unit::Weeks,
                                        _ => Unit::Minutes,
                                    };
                                    on_action.call(OptionAction::Unit(index, unit));
                                },
                                for unit in [Unit::Minutes, Unit::Hours, Unit::Days, Unit::Weeks] {
                                    option { selected: reminder.unit == unit, value: "{unit}", "{unit}" }
                                }
                            }
                            "before"
                        }
                    }
                }
            }
            button {
                class: "btn editor-wide",
                disabled: busy || !can_add,
                onclick: move |_| on_action.call(OptionAction::Add),
                "Add notification"
            }
        }
        Field { label: "Availability",
            select {
                class: "client-input",
                aria_label: "Availability",
                disabled: busy,
                onchange: move |event| {
                    let value = if event.value() == "Free" { Availability::Free } else { Availability::Busy };
                    on_action.call(OptionAction::Availability(value));
                },
                for value in [Availability::Busy, Availability::Free] {
                    option { selected: options.transparency == value, value: "{value}", "{value}" }
                }
            }
        }
        Field { label: "Visibility",
            select {
                class: "client-input",
                aria_label: "Visibility",
                disabled: busy,
                onchange: move |event| {
                    let chosen = event.value();
                    let value = [Visibility::Default, Visibility::Public, Visibility::Private, Visibility::Confidential]
                        .into_iter()
                        .find(|value| value.to_string() == chosen)
                        .unwrap_or_default();
                    on_action.call(OptionAction::Visibility(value));
                },
                for value in visibilities {
                    option { selected: options.visibility == value, value: "{value}", "{value}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Local, TimeZone};
    use timeslay_core::calendar::{Calendar, Details, colors, sleep};
    use timeslay_core::config::CalendarSettings;
    use timeslay_core::task_db::TaskData;
    use timeslay_core::timeblocking::Metadata;

    fn at(hour: u32) -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, 26, hour, 0, 0).unwrap()
    }

    fn event(id: &str, calendar_id: &str, title: &str) -> Event {
        Event {
            id: id.to_owned(),
            calendar_id: calendar_id.to_owned(),
            title: title.to_owned(),
            start: at(9),
            end: at(10),
            all_day: false,
            color: colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        }
    }

    fn task() -> Event {
        let mut task = event("task:a", "tasks:work", "Write report");
        task.all_day = true;
        task.end = task.start + Duration::days(1);
        task.details.task_data = Some(TaskData {
            id: 1,
            metadata: Metadata {
                minutes: 45,
                projects: vec![1],
                labels: vec!["deep work".into()],
                priority: Some(tb::Priority::P2),
                ..Metadata::default()
            },
            legacy_key: None,
        });
        task
    }

    fn context() -> EditContext {
        EditContext {
            settings: CalendarSettings {
                app_calendar: Some("blocks".into()),
                ..CalendarSettings::default()
            },
            projects: ProjectsSettings::default(),
            places: PlaceSettings::default(),
            calendars: vec![Calendar {
                id: "work".into(),
                name: "Work".into(),
                color: colors::DEFAULT_EVENT,
                primary: false,
                selected_by_google: true,
            }],
            events: vec![task()],
        }
    }

    #[derive(Props, Clone, PartialEq)]
    struct HarnessProps {
        event: Event,
    }

    #[allow(non_snake_case)]
    fn Harness(props: HarnessProps) -> Element {
        rsx! {
            Editor {
                event: props.event,
                context: context(),
                on_close: |_| {},
                on_saved: |_| {},
                on_config: |_| {},
            }
        }
    }

    fn render(event: Event) -> String {
        let mut dom = VirtualDom::new_with_props(Harness, HarnessProps { event });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[track_caller]
    fn assert_offers(html: &str, wanted: &[&str]) {
        for text in wanted {
            assert!(html.contains(text), "missing {text:?} in {html}");
        }
    }

    #[test]
    fn a_task_offers_all_of_its_metadata_and_actions() {
        let html = render(task());
        assert_offers(
            &html,
            &[
                "Title",
                "Write report",
                "Description",
                "Projects",
                "Work",
                "Estimate",
                "value=\"45\"",
                "Due Date",
                "Deadline",
                "Schedule Day",
                "Schedule Time",
                "Labels",
                "deep work",
                "Priority",
                "P2",
                "Location",
                "Complete task",
                "Ignore Task",
                "Move to inbox",
                "Move to backlog",
                "Delete task",
            ],
        );
    }

    #[test]
    fn an_event_offers_every_field_with_plain_save_and_delete() {
        let mut meeting = event("m1", "work", "Standup");
        meeting.details.organizer = "boss@example.com".into();
        meeting.details.attendees = vec!["pat@example.com".into()];
        let html = render(meeting);
        assert_offers(
            &html,
            &[
                "Calendar: Work",
                "Standup",
                "Projects",
                "Start Time",
                "End Time",
                "Repeat",
                "Does not repeat",
                "Labels",
                "Location",
                "Notifications",
                "Availability",
                "Visibility",
                "boss@example.com",
                "pat@example.com",
                ">Save<",
                ">Delete<",
            ],
        );
        assert!(!html.contains("Save current event"));
    }

    #[test]
    fn an_occurrence_of_a_series_offers_scoped_saves_once_its_rule_loads() {
        let mut occurrence = event("m1_1", "work", "Standup");
        occurrence.details.recurring_event_id = Some("m1".into());
        let html = render(occurrence);
        assert_offers(
            &html,
            &["Save current event", "Loading the repeat schedule"],
        );
    }

    #[test]
    fn a_habit_offers_completion_and_a_start_day_but_no_end() {
        let mut habit = event("h1", "blocks", "Stretch");
        habit.details.habit = true;
        let html = render(habit);
        assert_offers(
            &html,
            &["Type: Habit", "Mark as done", "Start day", "Repeat"],
        );
        assert!(!html.contains("End Time"));
    }

    #[test]
    fn a_block_is_edited_as_an_event() {
        let html = render(event("b1", "blocks", "Deep work"));
        assert_offers(
            &html,
            &["Type: Time Block", "Deep work", "End Time", ">Save<"],
        );
    }

    #[allow(non_snake_case)]
    fn Habits() -> Element {
        let mut row = event("h1", "blocks", "Stretch");
        row.details.habit = true;
        rsx! {
            HabitList {
                rows: vec![HabitRow {
                    title: "Stretch".into(),
                    schedule: "Daily".into(),
                    time: "07:00 – 07:15".into(),
                    color: colors::DEFAULT_EVENT,
                    event: row,
                }],
                on_open: |_| {},
            }
        }
    }

    #[test]
    fn the_habit_list_shows_each_schedule() {
        let mut dom = VirtualDom::new(Habits);
        dom.rebuild_in_place();
        assert_offers(
            &dioxus_ssr::render(&dom),
            &["Stretch", "Daily", "07:00 – 07:15"],
        );
    }

    #[test]
    fn sleep_offers_its_times_and_confidence() {
        let mut night = event("s1", "blocks", "Sleep");
        night.details.sleep = Some(sleep::Record::planned(at(9).date_naive()));
        let html = render(night);
        assert_offers(
            &html,
            &[
                "Type: Sleep",
                "Sleep start",
                "Wake-up time",
                "Estimated Sleep Start",
                "Estimated Sleep End",
                "Save sleep",
            ],
        );
    }
}
