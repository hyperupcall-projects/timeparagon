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
use timeslay_core::calendar::first_day::FirstDayOfWeek;
use timeslay_core::calendar::meeting;
use timeslay_core::calendar::{self, EditDraft, Event, SaveScope};
use timeslay_core::config::{CalendarSettings, Config, ProjectsSettings};
use timeslay_core::places::{self, Found, Place, PlaceSettings};
use timeslay_core::recurrence::{self, EndKind, MonthlyMode, RepeatMode, Rule};
use timeslay_core::task_edit::{self, Change};
use timeslay_core::timeblocking as tb;

use crate::css_color;
use crate::day::{self, AssignedTask, EditContext, TaskFields, TimeBlock};
use timeslay_core::calendar::colors::on_color;

const NTH_LABELS: [(i8, &str); 6] = [
    (1, "First"),
    (2, "Second"),
    (3, "Third"),
    (4, "Fourth"),
    (5, "Fifth"),
    (-1, "Last"),
];

const NO_PROJECT_COLOR: &str = "#9a9a9e";

/// Shared visual grouping for every editor except sleep.
#[component]
fn FormSection(title: &'static str, children: Element) -> Element {
    rsx! {
        section { class: "editor-section", aria_label: title,
            h3 { class: "editor-section-title", "{title}" }
            {children}
        }
    }
}

/// Runs a blocking write off the UI thread.
pub(crate) async fn worker<T: Send + 'static>(
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
    let event = if calendar::deadline::is_marker(&event) {
        let Some(original) = context
            .events
            .iter()
            .find(|original| original.id == calendar::deadline::source_id(&event.id))
        else {
            return rsx! {};
        };
        original.clone()
    } else {
        event
    };
    if event.details.sleep.is_some() {
        rsx! { SleepEditor { event, context, on_close, on_saved, on_config } }
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
    source: day::Source,
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
                        div { class: "modal-title-row",
                            h2 { class: "modal-title", "{title}" }
                            span {
                                class: "kind-chip",
                                style: "background: {css_color(source.color)}; color: {css_color(on_color(source.color))};",
                                "{source.name}"
                            }
                        }
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
    let repeat_seed = event.clone();
    let mut task_rule = use_signal(move || EditDraft::from_event(&repeat_seed).rule);
    let repeat_loading_seed = event.details.recurring_event_id.is_some();
    let mut repeat_loading = use_signal(move || repeat_loading_seed);
    let load_repeat_settings = context.settings.clone();
    let load_repeat_event = event.clone();
    let _repeat_load = use_resource(move || {
        let settings = load_repeat_settings.clone();
        let event = load_repeat_event.clone();
        async move {
            if !repeat_loading_seed {
                return;
            }
            match worker(move || {
                calendar::habit_edit_draft(
                    &calendar::GoogleClient::from_settings(&settings),
                    &event,
                )
            })
            .await
            {
                Ok((_, draft)) => {
                    task_rule.set(draft.rule);
                    repeat_loading.set(false);
                }
                Err(message) => error.set(Some(message)),
            }
        }
    });
    // A task repeating with its block stops from this occurrence on: it and
    // the later ones are deleted.
    let stop_repeating = use_callback(move |()| {
        if busy() {
            return;
        }
        let current = task();
        busy.set(true);
        error.set(None);
        spawn(async move {
            let result = worker(move || {
                calendar::delete_event_scope(
                    &current.calendar_id,
                    &current.id,
                    SaveScope::CurrentAndFuture,
                    false,
                )
            })
            .await;
            busy.set(false);
            match result {
                Ok(()) => {
                    on_saved.call(());
                    on_close.call(());
                }
                Err(message) => error.set(Some(message)),
            }
        });
    });
    let repeat_settings = context.settings.clone();
    let repeat = use_callback(move |action: recurrence::Action| {
        if busy() {
            return;
        }
        let current = task();
        if !current.is_calendar_task() {
            return;
        }
        busy.set(true);
        error.set(None);
        let settings = repeat_settings.clone();
        let rule = task_rule();
        spawn(async move {
            let result = worker(move || {
                let (source, mut draft) = if current.details.recurring_event_id.is_some() {
                    calendar::habit_edit_draft(
                        &calendar::GoogleClient::from_settings(&settings),
                        &current,
                    )?
                } else {
                    (current.clone(), EditDraft::from_event(&current))
                };
                if source.details.recurring_event_id.is_none() {
                    draft.rule = rule;
                }
                draft.rule.apply_action(action, draft.start.date_naive());
                draft.recurrence_changed = true;
                calendar::save_event_scope(
                    &source,
                    &draft,
                    SaveScope::All,
                    &draft.description,
                    None,
                )?;
                let mut updated = current;
                updated.details.recurrence = draft.recurrence_lines()?;
                Ok((updated, draft.rule))
            })
            .await;
            busy.set(false);
            match result {
                Ok((updated, rule)) => {
                    task.set(updated);
                    task_rule.set(rule);
                    on_saved.call(());
                }
                Err(message) => error.set(Some(message)),
            }
            if closing() {
                on_close.call(());
            }
        });
    });
    let mut editing_due_time = use_signal(|| false);
    let timed_deadline =
        calendar::deadline::get(&event).is_some_and(|deadline| deadline.time.is_some());
    let mut editing_deadline_time = use_signal(move || timed_deadline);
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

    let set_status = use_callback(move |status: tb::TaskStatus| {
        if busy() {
            return;
        }
        busy.set(true);
        error.set(None);
        let current = task();
        spawn(async move {
            let result = worker(move || calendar::set_linked_task_status(&current, status)).await;
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
            let result =
                worker(move || calendar::delete_task(&current, settings.app_calendar.as_deref()))
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
            FormSection { title: "Scheduling",
                if tb::is_follower(&current) {
                    p { class: "hint",
                        "Repeats with its time block, on the block's days and at its time. Change it from the time block."
                    }
                    button {
                        class: "btn editor-wide",
                        disabled: is_busy,
                        onclick: move |_| stop_repeating.call(()),
                        "Stop from this day"
                    }
                } else {
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
                    if current.is_calendar_task() {
                        RepeatEditor {
                            rule: task_rule(),
                            anchor: current.start.date_naive(),
                            first_day: context.settings.first_day_of_week,
                            busy: is_busy || repeat_loading(),
                            on_action: move |action| repeat.call(action),
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
                }
            }
            FormSection { title: "Information",
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
                if let Some(target) = crate::notes::target_of(&event, &context.settings) {
                    crate::notes::LinkedNotes { target }
                }
            }
            FormSection { title: "Organization",
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
    let mut pending_time_scope = use_signal(|| false);
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
                calendar::habit_edit_draft(
                    &calendar::GoogleClient::from_settings(&settings),
                    &event,
                )
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
        pending_time_scope.set(false);
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

    let delete = use_callback(move |scope: SaveScope| {
        if busy() {
            return;
        }
        let source = original();
        busy.set(true);
        error.set(None);
        spawn(async move {
            let result = worker(move || {
                calendar::delete_event_scope(&source.calendar_id, &source.id, scope, false)
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

    let set_place = use_callback(move |place: Option<Place>| {
        if busy() {
            return;
        }
        let source = original();
        busy.set(true);
        error.set(None);
        spawn(async move {
            let chosen = place.clone();
            let written = source.clone();
            let result = worker(move || day::save_event_place(&written, chosen.as_ref())).await;
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

    let toggle_completed = use_callback(move |()| {
        if completing() {
            return;
        }
        let mut updated = original();
        updated.completed = !updated.completed;
        updated.details.habit_completed_at = updated.completed.then(timeslay_core::clock::now);
        completing.set(true);
        error.set(None);
        spawn(async move {
            let written = updated.clone();
            let result = worker(move || calendar::set_habit_completed(&written)).await;
            completing.set(false);
            match result {
                Ok(()) => {
                    crate::habits::record_completion(&updated);
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
    let series = source_event.is_recurring();
    let anchor = values.start.date_naive();
    let organizer = source_event.details.organizer.clone();
    let attendees = source_event.details.attendees.clone();
    let location_text = source_event.details.location.clone();
    let completed = source_event.completed;
    // A habit event that is not on Google yet has nothing to delete, complete
    // or locate until it is created.
    let new = source_event.id == crate::habits::NEW_HABIT_ID;

    let footer = rsx! {
        if pending_time_scope() {
            div { class: "editor-choices", role: "dialog", aria_label: "Apply this change to",
                p { "Apply this change to…" }
                for (label, scope) in [
                    ("This event", SaveScope::Current),
                    ("This and following events", SaveScope::CurrentAndFuture),
                    ("All events", SaveScope::All),
                ] {
                    button { class: "btn editor-wide", disabled: is_busy,
                        onclick: move |_| save.call(scope), "{label}"
                    }
                }
                button { class: "btn editor-wide", onclick: move |_| pending_time_scope.set(false), "Cancel" }
            }
        } else if let Some(scope) = pending_delete() {
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
                    onclick: move |_| {
                        let current = draft();
                        let source = original();
                        if !new && source.is_recurring() && (current.start != source.start || current.end != source.end || current.all_day != source.all_day) {
                            pending_time_scope.set(true);
                        } else {
                            save.call(SaveScope::Current);
                        }
                    },
                    if busy() { "Saving…" } else if new { "Create" } else if series { "Save current event" } else { "Save" }
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
                    hidden: new,
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
            FormSection { title: "Scheduling",
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
                    first_day: context.settings.first_day_of_week,
                    busy: is_busy,
                    on_action: move |action: recurrence::Action| {
                        let mut draft = draft.write();
                        draft.recurrence_changed = true;
                        let anchor = draft.start.date_naive();
                        draft.rule.apply_action(action, anchor);
                    },
                }
                NotificationsEditor {
                    options: values.options.clone(),
                    busy: is_busy,
                    on_action: move |action| draft.write().options.update(action),
                }
            }
            FormSection { title: "Information",
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
                if let Some(target) = crate::notes::target_of(&source_event, &context.settings) {
                    crate::notes::LinkedNotes { target }
                }
                if habit && !new {
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
            }
            FormSection { title: "Organization",
                ProjectsField {
                    selected: values.projects.ids.clone(),
                    projects: context.projects.clone(),
                    busy: is_busy,
                    on_add: move |id| draft.write().projects.update(calendar::projects::Action::Add(id)),
                    on_remove: move |id| draft.write().projects.update(calendar::projects::Action::Remove(id)),
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
                if !new {
                    LocationField {
                        current: place,
                        text: location_text,
                        places: context.places.clone(),
                        busy: is_busy,
                        on_error: move |message| error.set(Some(message)),
                        on_pick: move |place| set_place.call(place),
                    }
                }
            }
            if !habit && !source_event.is_task() {
                FormSection { title: "Meeting",
                    if !new {
                        MeetingActions {
                            event: source_event.clone(),
                            settings: context.settings.clone(),
                            on_saved,
                        }
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
                    PresenceEditor {
                        options: values.options.clone(),
                        busy: is_busy,
                        on_action: move |action| draft.write().options.update(action),
                    }
                }
            }

        }
    }
}

/// Joining a call, and answering an invitation.
#[component]
fn MeetingActions(event: Event, settings: CalendarSettings, on_saved: EventHandler<()>) -> Element {
    let attendance = event.details.attendance.clone();
    let mut answer = use_signal(move || attendance.own_response());
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let join = meeting::join_link(&event.details);
    let email = event
        .details
        .attendance
        .can_reply()
        .then(|| event.details.attendance.own_email.clone())
        .flatten();
    if join.is_none() && email.is_none() {
        return rsx! {};
    }
    let choices = [
        (meeting::Response::Accepted, "Yes"),
        (meeting::Response::Tentative, "Maybe"),
        (meeting::Response::Declined, "No"),
    ];
    rsx! {
        if let Some(url) = join {
            button {
                class: "btn primary editor-wide",
                onclick: move |_| {
                    let url = url.clone();
                    spawn(async move {
                        if let Err(message) = worker(move || calendar::oauth::open_in_browser(&url)).await {
                            error.set(Some(message));
                        }
                    });
                },
                "Join meeting"
            }
        }
        if let Some(email) = email {
            Field { label: "Going?",
                div { class: "editor-row",
                    for (response, label) in choices {
                        {
                            let email = email.clone();
                            let event = event.clone();
                            let chosen = answer() == Some(response);
                            rsx! {
                                button {
                                    key: "{label}",
                                    class: if chosen { "btn primary editor-grow" } else { "btn editor-grow" },
                                    aria_pressed: chosen.to_string(),
                                    disabled: busy(),
                                    onclick: move |_| {
                                        if chosen {
                                            return;
                                        }
                                        let previous = answer();
                                        // Shown straight away, and taken back if Google refuses.
                                        answer.set(Some(response));
                                        busy.set(true);
                                        error.set(None);
                                        let email = email.clone();
                                        let event = event.clone();
                                        spawn(async move {
                                            let result = worker(move || {
                                                calendar::reply_to_event(
                                                    &event.calendar_id,
                                                    &event.id,
                                                    &email,
                                                    response,
                                                )
                                            })
                                            .await;
                                            busy.set(false);
                                            match result {
                                                Ok(()) => on_saved.call(()),
                                                Err(message) => {
                                                    answer.set(previous);
                                                    error.set(Some(message));
                                                }
                                            }
                                        });
                                    },
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(message) = error() {
            p { class: "editor-error", role: "alert", "{message}" }
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

/// A night has its planned times with independent confidence flags, and the
/// sleep actually recorded, from the timer or typed in.
#[component]
fn SleepEditor(
    event: Event,
    context: EditContext,
    on_close: EventHandler<()>,
    on_saved: EventHandler<()>,
    on_config: EventHandler<Config>,
) -> Element {
    let seed = event.clone();
    let mut draft = use_signal(move || EditDraft::from_event(&seed));
    // The night as Google has it, which the timer works from.
    let seed = event.clone();
    let mut saved_night = use_signal(move || seed);
    let mut busy = use_signal(|| false);
    let mut closing = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    // Moving a night's times asks whether its weekday's schedule moves too.
    let mut pending_scope = use_signal(|| false);

    let close = move |_: ()| {
        if busy() {
            closing.set(true);
        } else {
            on_close.call(());
        }
    };

    let time_format = context.time_format;
    let settings = context.settings.clone();
    let write = use_callback(move |scope: Option<SaveScope>| {
        if busy() {
            return;
        }
        pending_scope.set(false);
        let values = draft();
        let Some(mut record) = values.sleep.clone() else {
            return;
        };
        let mut night = saved_night();
        let new = night.id == calendar::sleep::NEW_ID;
        // A single night becomes an exception to the schedule; following the
        // schedule keeps it planned from it.
        record.automatic = scope == Some(SaveScope::CurrentAndFuture);
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
            let result = worker(move || {
                let saved = calendar::sleep::save(&settings, night, new)?;
                let config = (scope == Some(SaveScope::CurrentAndFuture))
                    .then(|| crate::sleep::save_from_night(&saved))
                    .transpose()?;
                Ok((saved, config))
            })
            .await;
            busy.set(false);
            match result {
                Ok((_, config)) => {
                    if let Some(config) = config {
                        on_config.call(config);
                    }
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

    let original = event.clone();
    let save = use_callback(move |()| {
        if busy() {
            return;
        }
        let values = draft();
        if !values.actual_sleep_valid() {
            error.set(Some(
                "Enter actual sleep times as HH:MM, or leave them empty".to_owned(),
            ));
            return;
        }
        if calendar::parse_clock(&values.start_text).is_none()
            || calendar::parse_clock(&values.end_text).is_none()
            || calendar::deadline::parse_date(&values.date_text).is_err()
            || calendar::deadline::parse_date(&values.end_date_text).is_err()
            || values.end <= values.start
        {
            error.set(Some("Enter bedtime and wake-up time as HH:MM".to_owned()));
            return;
        }
        if original.id != calendar::sleep::NEW_ID
            && (original.start != values.start || original.end != values.end)
        {
            pending_scope.set(true);
            return;
        }
        write.call(None);
    });

    let timer_settings = context.settings.clone();
    let press = use_callback(move |()| {
        if busy() {
            return;
        }
        let night = saved_night();
        if calendar::sleep::timer(&night, timeslay_core::clock::now())
            == Some(calendar::sleep::Timer::Start)
            && crate::reminders::notifications_blocked()
        {
            crate::reminders::request_permission();
        }
        let settings = timer_settings.clone();
        busy.set(true);
        error.set(None);
        spawn(async move {
            let result = worker(move || crate::sleep::press(&settings, night, time_format)).await;
            busy.set(false);
            match result {
                Ok(saved) => {
                    // Keep unsaved time edits; take the recorded sleep.
                    draft.with_mut(|draft| {
                        let record = saved.details.sleep.as_ref();
                        let clock = |at: Option<chrono::DateTime<chrono::Local>>| {
                            at.map(calendar::clock_label).unwrap_or_default()
                        };
                        draft.set_actual_start_text(clock(record.and_then(|r| r.actual_start)));
                        draft.set_actual_end_text(clock(record.and_then(|r| r.actual_end)));
                    });
                    saved_night.set(saved);
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
    let now = timeslay_core::clock::now();
    let actual_summary = match (record.actual_start, record.actual_end) {
        (Some(start), Some(end)) => format!(
            "Slept {} (planned {})",
            calendar::sleep::minutes_label((end - start).num_minutes()),
            calendar::sleep::duration(&preview)
        ),
        (Some(start), None) => format!(
            "Sleeping since {} · {} so far",
            calendar::time_format::clock(start, time_format),
            calendar::sleep::minutes_label((now - start).num_minutes())
        ),
        _ => "Not recorded yet. Use Start sleep at bedtime, or enter the times.".to_owned(),
    };
    preview.details.sleep = Some(record.clone());
    let predictions: Vec<(String, String)> = calendar::sleep::wake_predictions(&preview)
        .into_iter()
        .map(|(label, at)| (label, calendar::time_format::clock(at, time_format)))
        .collect();
    let basis = if record.actual_start.is_some() {
        "Counted from when sleep actually started."
    } else {
        "Counted from the planned bedtime."
    };
    // The timer works from the saved night, not from unsaved edits.
    let night = saved_night();
    let timer = (night.id != calendar::sleep::NEW_ID)
        .then(|| calendar::sleep::timer(&night, now))
        .flatten();
    let timer_disabled = is_busy || !crate::sleep::can_press(&night);
    let source = day::source_label(&event, &context);
    let color = css_color(event.color);
    let weekday = calendar::sleep::weekday_label(record.wake_date);
    let footer = if pending_scope() {
        rsx! {
            p { class: "editor-status", "Change only this night, or the {weekday} schedule from here on?" }
            button {
                class: "btn editor-wide",
                onclick: move |_| write.call(Some(SaveScope::Current)),
                "This {weekday}"
            }
            button {
                class: "btn editor-wide",
                onclick: move |_| write.call(Some(SaveScope::CurrentAndFuture)),
                "This and future {weekday}s"
            }
            button { class: "btn editor-wide", onclick: move |_| pending_scope.set(false), "Cancel" }
        }
    } else {
        rsx! {
            if let Some(timer) = timer {
                button {
                    class: "btn editor-wide",
                    disabled: timer_disabled,
                    onclick: move |_| press.call(()),
                    "{timer.label()}"
                }
            }
            button {
                class: "btn primary editor-wide",
                disabled: is_busy,
                onclick: move |_| save.call(()),
                if is_busy { "Saving…" } else { "Save sleep" }
            }
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
            FormSection { title: "Scheduling",
                Field { label: "Bedtime",
                    input {
                        class: "client-input",
                        r#type: "time",
                        aria_label: "Bedtime",
                        disabled: is_busy,
                        value: "{values.start_text}",
                        oninput: move |event| draft.write().set_start_text(event.value()),
                    }
                }
                Field { label: "Wake-up time",
                    input {
                        class: "client-input",
                        r#type: "time",
                        aria_label: "Wake-up time",
                        disabled: is_busy,
                        value: "{values.end_text}",
                        oninput: move |event| draft.write().set_end_text(event.value()),
                    }
                }
                p { class: "hint",
                    if record.automatic {
                        "Planned from your weekly sleep schedule. Saving records your times and preserves them when your schedule changes."
                    } else {
                        "Recorded sleep. These times are preserved when your schedule changes."
                    }
                }
            }
            FormSection { title: "Actual sleep",
                p { class: "hint", "{actual_summary}" }
                Field { label: "Actual sleep start",
                    div { class: "editor-row",
                        input {
                            class: "client-input",
                            r#type: "time",
                            aria_label: "Actual sleep start",
                            disabled: is_busy,
                            value: "{values.actual_start_text}",
                            oninput: move |event| draft.write().set_actual_start_text(event.value()),
                        }
                        if !values.actual_start_text.is_empty() && !is_busy {
                            button {
                                class: "btn small",
                                onclick: move |_| draft.write().set_actual_start_text(String::new()),
                                "Clear"
                            }
                        }
                    }
                }
                Field { label: "Actual wake-up",
                    div { class: "editor-row",
                        input {
                            class: "client-input",
                            r#type: "time",
                            aria_label: "Actual wake-up",
                            disabled: is_busy,
                            value: "{values.actual_end_text}",
                            oninput: move |event| draft.write().set_actual_end_text(event.value()),
                        }
                        if !values.actual_end_text.is_empty() && !is_busy {
                            button {
                                class: "btn small",
                                onclick: move |_| draft.write().set_actual_end_text(String::new()),
                                "Clear"
                            }
                        }
                    }
                }
            }
            FormSection { title: "Wake up at",
                div { class: "sleep-predictions",
                    for (label, at) in predictions {
                        div { key: "{label}", class: "sleep-prediction",
                            span { "{label}" }
                            span { "{at}" }
                        }
                    }
                }
                p { class: "hint", "{basis}" }
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
    // An item in a repeating block either repeats with it or is a one-off.
    let mut repeat = use_signal(|| true);
    let block_repeats = block.event.details.recurring_event_id.is_some();
    let join = Change::join_block_series(&block.event);

    let write_context = context.clone();
    let apply = use_callback(move |(task, change): (Event, Change)| {
        if busy() {
            return;
        }
        busy.set(true);
        error.set(None);
        let context = write_context.clone();
        // A task that does not repeat yet repeats with a repeating block
        // when asked to; one that repeats already only goes in this day.
        let join = join
            .clone()
            .filter(|_| matches!(change, Change::Block(..)) && repeat() && !task.is_recurring());
        spawn(async move {
            let result = worker(move || {
                let saved = day::save_task(task, change, &context)?;
                match join {
                    Some(join) => day::save_task(saved, join, &context),
                    None => Ok(saved),
                }
            })
            .await;
            busy.set(false);
            match result {
                Ok(_) => on_saved.call(()),
                Err(message) => error.set(Some(message)),
            }
        });
    });
    let stop = use_callback(move |task: Event| {
        if busy() {
            return;
        }
        busy.set(true);
        error.set(None);
        spawn(async move {
            let result = worker(move || {
                calendar::delete_event_scope(
                    &task.calendar_id,
                    &task.id,
                    calendar::SaveScope::CurrentAndFuture,
                    false,
                )
            })
            .await;
            busy.set(false);
            match result {
                Ok(()) => on_saved.call(()),
                Err(message) => error.set(Some(message)),
            }
        });
    });
    // A one-off starts repeating with the block from this occurrence on. A
    // habit with a repeating event of its own is asked about first, since
    // that event then ends from this day: these are its next occurrences.
    let mut confirming = use_signal(|| None::<(Event, Vec<(String, String)>)>);
    let hour = context.settings.day_start_hour();
    let from = calendar::at_clock(
        tb::membership::planning_day(block.event.start, hour),
        hour.min(23),
        0,
    );
    let repeat_context = context.clone();
    let repeat_join = Change::join_block_series(&block.event);
    let repeat_with_block = use_callback(
        move |(task, ended): (Event, Option<Vec<(String, String)>>)| {
            let Some(join) = repeat_join.clone() else {
                return;
            };
            if busy() {
                return;
            }
            busy.set(true);
            error.set(None);
            confirming.set(None);
            let context = repeat_context.clone();
            spawn(async move {
                let result = worker(move || {
                    let habit = tb::metadata(&task).habit_id;
                    if ended.is_none()
                        && let Some(habit) = habit
                    {
                        let found = calendar::habit_repeats_from(&context.settings, &habit, from)?;
                        if !found.is_empty() {
                            return Ok(Some((task, found)));
                        }
                    }
                    day::save_task(task, join, &context)?;
                    for (calendar_id, occurrence) in ended.unwrap_or_default() {
                        calendar::delete_event_scope(
                            &calendar_id,
                            &occurrence,
                            SaveScope::CurrentAndFuture,
                            false,
                        )?;
                    }
                    Ok(None)
                })
                .await;
                busy.set(false);
                match result {
                    Ok(Some(ask)) => confirming.set(Some(ask)),
                    Ok(None) => on_saved.call(()),
                    Err(message) => error.set(Some(message)),
                }
            });
        },
    );

    let is_busy = busy();
    let key = tb::block_key(&block.event);
    let (start, end) = (block.event.start, block.event.end);
    let assignable = day::assignable_tasks(&context.events);

    rsx! {
        if let Some(message) = error() {
            p { class: "editor-error", role: "alert", "{message}" }
        }
        if let Some((task, ended)) = confirming() {
            div { class: "editor-confirm", role: "alert",
                p { "Repeat “{task.title}” with this block?" }
                p { class: "hint",
                    "It also has a repeating event of its own. Repeating with the time block ends that one from this day on, so the habit is not on the calendar twice."
                }
                div { class: "editor-row",
                    button {
                        class: "btn editor-grow",
                        disabled: is_busy,
                        onclick: move |_| confirming.set(None),
                        "Cancel"
                    }
                    button {
                        class: "btn primary editor-grow",
                        disabled: is_busy,
                        onclick: move |_| repeat_with_block.call((task.clone(), Some(ended.clone()))),
                        "Repeat with this block"
                    }
                }
            }
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
                        can_repeat: tb::can_repeat_with(&task.event, &block.event, hour),
                        task,
                        busy: is_busy,
                        on_open,
                        on_remove: move |event: Event| apply.call((event, Change::Unassign)),
                        on_stop: move |event: Event| stop.call(event),
                        on_repeat: move |event: Event| repeat_with_block.call((event, None)),
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
        if adding() && block_repeats {
            label { class: "editor-check",
                input {
                    r#type: "checkbox",
                    checked: repeat(),
                    disabled: is_busy,
                    onchange: move |event: FormEvent| repeat.set(event.checked()),
                }
                span { "Repeat with this block" }
            }
            p { class: "empty-note",
                if repeat() {
                    "Tasks added here come back every time the block does. Ones that already repeat go in this day only."
                } else {
                    "Tasks added here are for this day only."
                }
            }
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
    /// Stop a task repeating with its block from this occurrence on.
    on_stop: EventHandler<Event>,
    /// A one-off in a repeating block, which can start repeating with it.
    can_repeat: bool,
    on_repeat: EventHandler<Event>,
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
    let stopped = task.event.clone();
    let joined = task.event.clone();
    let repeating = tb::is_follower(&task.event);
    let title = task.event.title.clone();
    let minutes = day::duration_label(task.minutes);

    rsx! {
        div { class: "assigned-task", "data-sending": task.event.details.sending,
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
                        if repeating {
                            span { class: "meta-pill", "↻ Repeats with this block" }
                        }
                        if can_repeat {
                            span { class: "meta-pill", "This day only" }
                        }
                    }
                }
                span { class: "assigned-minutes", "{minutes}" }
            }
            if repeating {
                button {
                    class: "btn small",
                    title: "Stop repeating from this day",
                    aria_label: "Stop {title} repeating from this day",
                    disabled: busy,
                    onclick: move |_| on_stop.call(stopped.clone()),
                    "Stop"
                }
            }
            if can_repeat {
                button {
                    class: "btn small",
                    title: "Repeat with this block from this day",
                    aria_label: "Repeat {title} with this block from this day",
                    disabled: busy,
                    onclick: move |_| on_repeat.call(joined.clone()),
                    "Repeat"
                }
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

/// The weekdays in the order the week shows them, each with its Monday-first
/// index into the rule.
fn weekday_choices(first_day: FirstDayOfWeek) -> impl Iterator<Item = (usize, &'static str)> {
    first_day
        .weekdays()
        .into_iter()
        .map(|day| day.num_days_from_monday() as usize)
        .zip(first_day.short_labels())
}

/// The repeat controls: the same words and choices as the iced editors.
#[component]
fn RepeatEditor(
    rule: Rule,
    anchor: NaiveDate,
    first_day: FirstDayOfWeek,
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
                        for (index, day) in weekday_choices(first_day) {
                            Choice {
                                key: "{day}",
                                label: day,
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
                                for (index, day) in weekday_choices(first_day) {
                                    Choice {
                                        key: "{day}",
                                        label: day,
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
            }
        }
    }
}

/// Google-style notification rows followed by availability and visibility.
#[component]
fn NotificationsEditor(
    options: EventOptions,
    busy: bool,
    on_action: EventHandler<OptionAction>,
) -> Element {
    let use_default = options.reminders.use_default;
    let overrides = options.reminders.overrides.clone();
    let can_add = overrides.len() < 5;

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
    }
}

#[component]
fn PresenceEditor(
    options: EventOptions,
    busy: bool,
    on_action: EventHandler<OptionAction>,
) -> Element {
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
            time_format: calendar::time_format::TimeFormat::TwelveHour,
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

    #[test]
    fn a_task_repeating_with_its_block_takes_its_schedule_from_the_block() {
        let mut repeating = task();
        repeating.id = "t_20261026T160000Z".into();
        repeating.calendar_id = "blocks".into();
        repeating.all_day = false;
        repeating.details.recurring_event_id = Some("t".into());
        repeating
            .details
            .task_data
            .as_mut()
            .unwrap()
            .metadata
            .block_series = Some("blocks/b".into());
        let html = render(repeating);
        assert_offers(
            &html,
            &["Repeats with its time block", "Stop from this day"],
        );
        assert!(!html.contains(">Due Date<"));
        assert!(!html.contains("class=\"editor-label\">Repeat<"));
        // A task of its own keeps its scheduling.
        assert!(render(task()).contains(">Due Date<"));
    }

    #[derive(Props, Clone, PartialEq)]
    struct BlockHarnessProps {
        block: TimeBlock,
    }

    #[allow(non_snake_case)]
    fn BlockHarness(props: BlockHarnessProps) -> Element {
        let busy = use_signal(|| false);
        rsx! {
            BlockTasks {
                block: props.block,
                context: context(),
                busy,
                on_open: |_| {},
                on_saved: |_| {},
            }
        }
    }

    /// A repeating block holding `task`, as its block task list shows it.
    fn render_block(task: Event) -> String {
        let mut block = event("b_20261026T160000Z", "blocks", "Deep work");
        block.details.recurring_event_id = Some("b".into());
        let block = TimeBlock {
            event: block,
            tasks: vec![AssignedTask {
                event: task,
                status: tb::TaskStatus::Active,
                minutes: 45,
                list_name: "Work".into(),
            }],
            capacity_minutes: 60,
            planned_minutes: 45,
        };
        let mut dom = VirtualDom::new_with_props(BlockHarness, BlockHarnessProps { block });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn a_one_off_in_a_repeating_block_offers_to_repeat_with_it() {
        let mut one_off = task();
        one_off.id = "one-off".into();
        one_off.calendar_id = "blocks".into();
        one_off.all_day = false;
        let data = one_off.details.task_data.as_mut().unwrap();
        data.metadata.block = Some("blocks/b_20261026T160000Z".into());
        let html = render_block(one_off.clone());
        assert_offers(
            &html,
            &["This day only", "Repeat Write report with this block"],
        );
        assert!(!html.contains("Repeats with this block"));

        // One repeating with it offers to stop instead.
        let mut repeating = one_off;
        repeating.id = "t_20261026T160000Z".into();
        repeating.details.recurring_event_id = Some("t".into());
        let data = repeating.details.task_data.as_mut().unwrap();
        data.metadata.block = None;
        data.metadata.block_series = Some("blocks/b".into());
        let html = render_block(repeating);
        assert_offers(&html, &["↻ Repeats with this block", "Stop Write report"]);
        assert!(!html.contains("This day only"));
    }

    #[track_caller]
    fn assert_offers(html: &str, wanted: &[&str]) {
        for text in wanted {
            assert!(html.contains(text), "missing {text:?} in {html}");
        }
    }

    #[test]
    fn a_scheduled_task_places_deadline_directly_after_repeat() {
        let mut task = task();
        task.id = "scheduled-report".into();
        task.calendar_id = "blocks".into();
        let html = render(task);
        let repeat = html.find("class=\"editor-label\">Repeat").unwrap();
        let deadline = html.find("class=\"editor-label\">Deadline").unwrap();
        let description = html.find("class=\"editor-label\">Description").unwrap();
        assert!(repeat < deadline && deadline < description);
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
    fn weekday_choices_start_on_the_first_day_and_keep_their_rule_index() {
        let choices: Vec<_> = weekday_choices(FirstDayOfWeek::Sunday).collect();
        assert_eq!(choices[0], (6, "Sun"));
        assert_eq!(choices[1], (0, "Mon"));
        let choices: Vec<_> = weekday_choices(FirstDayOfWeek::Saturday).collect();
        assert_eq!(choices[0], (5, "Sat"));
        assert_eq!(choices[6], (4, "Fri"));
        assert_eq!(
            weekday_choices(FirstDayOfWeek::Monday).next(),
            Some((0, "Mon"))
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
                "kind-chip",
                ">Event<",
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
            &[
                "kind-chip",
                ">Habit<",
                "Mark as done",
                "Start day",
                "Repeat",
            ],
        );
        assert!(!html.contains("End Time"));
    }

    #[test]
    fn a_block_is_edited_as_an_event() {
        let html = render(event("b1", "blocks", "Deep work"));
        assert_offers(
            &html,
            &[
                "kind-chip",
                ">Time Block<",
                "Deep work",
                "End Time",
                ">Save<",
            ],
        );
    }

    #[test]
    fn a_night_offers_its_timer_actual_sleep_and_wake_up_times() {
        let mut night = event("s1", "blocks", "Sleep");
        night.start = at(1);
        night.end = at(9);
        night.details.sleep = Some(sleep::Record::planned(at(9).date_naive()));
        // Half an hour before the planned bedtime.
        let _clock = timeslay_core::clock::freeze(
            (at(1) - chrono::Duration::minutes(30)).with_timezone(&chrono::Utc),
        );
        let html = render(night.clone());
        assert_offers(
            &html,
            &[
                "Actual sleep",
                "Actual sleep start",
                "Actual wake-up",
                "Not recorded yet",
                "Wake up at",
                "6h",
                "9h",
                "Target 8h",
                "Counted from the planned bedtime.",
                ">Start sleep<",
            ],
        );

        sleep::apply_timer(&mut night, sleep::Timer::Start, at(1)).unwrap();
        let html = render(night);
        assert_offers(
            &html,
            &[
                ">Finish sleep<",
                "Sleeping since",
                "Counted from when sleep actually started.",
            ],
        );
        assert!(!html.contains(">Start sleep<"));
    }

    #[test]
    fn a_night_far_from_now_offers_no_timer() {
        let mut night = event("s1", "blocks", "Sleep");
        night.start = at(1);
        night.end = at(9);
        night.details.sleep = Some(sleep::Record::planned(at(9).date_naive()));
        let _clock = timeslay_core::clock::freeze(
            (at(1) - chrono::Duration::days(2)).with_timezone(&chrono::Utc),
        );
        let html = render(night);
        assert!(!html.contains(">Start sleep<") && !html.contains(">Finish sleep<"));
    }

    #[test]
    fn sleep_offers_its_times_and_confidence() {
        let mut night = event("s1", "blocks", "Sleep");
        night.details.sleep = Some(sleep::Record::planned(at(9).date_naive()));
        let html = render(night);
        assert_offers(
            &html,
            &[
                ">Sleep<",
                "kind-chip",
                "Scheduling",
                "Bedtime",
                "Wake-up time",
                "Actual sleep",
                "Wake up at",
                "Save sleep",
            ],
        );
    }
}
