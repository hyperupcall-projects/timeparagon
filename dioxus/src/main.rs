//! TimeSlay's Dioxus day view.
//!
//! This is deliberately just the day: a single screen showing every calendar
//! event, every Google Task, and every time block for one day. Clicking a time
//! block opens a popup with all of the tasks assigned to it.

mod day;

use chrono::Datelike;
use dioxus::prelude::*;
use time_slay_core::calendar::{self, Event};
use time_slay_core::config::CalendarSettings;
use time_slay_core::config_sync;
use time_slay_core::timeblocking as tb;

use day::{AssignedTask, BlockRef, DayModel, TaskGroup, TaskRow, Theme, TimeBlock, TimedEvent};

/// Height of one hour on the timeline, in pixels.
const HOUR_HEIGHT: f32 = 64.0;

const SCROLL_SCRIPT: &str = r#"
await new Promise(resolve => requestAnimationFrame(resolve));
const scroller = document.querySelector('.timeline-scroll');
const anchor = document.getElementById('now-line') || document.getElementById('timeline-anchor');
if (scroller && anchor && scroller.clientHeight) {
    scroller.scrollTop += anchor.getBoundingClientRect().top
        - scroller.getBoundingClientRect().top - scroller.clientHeight / 2;
    return true;
}
return false;
"#;

const CSS: &str = include_str!("../assets/day.css");

#[cfg(not(target_os = "android"))]
fn main() {
    config_sync::start();
    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            dioxus::desktop::Config::new()
                .with_window(
                    dioxus::desktop::WindowBuilder::new()
                        .with_title("TimeSlay — Day")
                        .with_inner_size(dioxus::desktop::LogicalSize::new(1180.0, 900.0))
                        .with_min_inner_size(dioxus::desktop::LogicalSize::new(760.0, 560.0)),
                )
                .with_menu(None::<dioxus::desktop::muda::Menu>)
                .with_background_color((11, 13, 19, 255)),
        )
        .launch(App);
}

#[cfg(target_os = "android")]
fn main() {
    prepare_android_storage();
    config_sync::start();
    dioxus::launch(App);
}

/// Point the shared data layer at the app's private internal storage.
#[cfg(target_os = "android")]
fn prepare_android_storage() {
    let Some((files, cache)) = android_storage_dirs() else {
        return;
    };

    let config = files.join("config");
    let data = files.join("data");
    for directory in [&config, &data, &cache] {
        let _ = std::fs::create_dir_all(directory);
    }

    // SAFETY: this runs before the Dioxus runtime starts any threads.
    unsafe {
        std::env::set_var("HOME", &files);
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_DATA_HOME", &data);
        std::env::set_var("XDG_CACHE_HOME", &cache);
    }
}

#[cfg(target_os = "android")]
fn android_storage_dirs() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    use jni::objects::JObject;

    let context = std::panic::catch_unwind(|| ndk_context::android_context()).ok()?;
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let activity = unsafe { JObject::from_raw(context.context().cast()) };
    let files = env
        .call_method(&activity, "getFilesDir", "()Ljava/io/File;", &[])
        .ok()?
        .l()
        .ok()?;
    let cache = env
        .call_method(&activity, "getCacheDir", "()Ljava/io/File;", &[])
        .ok()?
        .l()
        .ok()?;
    let files = java_file_path(&mut env, &files)?;
    let cache = java_file_path(&mut env, &cache)?;
    Some((files, cache))
}

#[cfg(target_os = "android")]
fn java_file_path(
    env: &mut jni::JNIEnv<'_>,
    file: &jni::objects::JObject<'_>,
) -> Option<std::path::PathBuf> {
    use jni::objects::JString;

    let path = env
        .call_method(file, "getAbsolutePath", "()Ljava/lang/String;", &[])
        .ok()?
        .l()
        .ok()?;
    let path = JString::from(path);
    let path: String = env.get_string(&path).ok()?.into();
    Some(std::path::PathBuf::from(path))
}

#[component]
fn App() -> Element {
    // The config can change under the view: an edit in the iced app, or a copy
    // from another device through Google Drive.
    let mut config = use_signal(day::load_config);
    let settings = config.read().calendar.clone();
    let mut config_status = use_signal(config_sync::status);
    let config_hint = time_slay_core::config::config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "config.toml".to_owned());

    let mut current_day = use_signal(|| day::today(&settings));
    let mut revision = use_signal(|| 0u32);
    let mut selected_block = use_signal(|| None::<String>);
    let mut show_tasks = use_signal(|| false);
    let mut scrolled = use_signal(|| false);
    let mut follow_today = use_signal(|| true);
    let connecting = use_signal(|| false);
    let mut connect_error = use_signal(|| None::<String>);
    let mut theme = use_signal(Theme::load);
    let mut client_id_input = use_signal(String::new);
    let mut client_secret_input = use_signal(String::new);
    let mut client_error = use_signal(|| None::<String>);
    let mut signed_in = use_signal(day::signed_in);
    let mut cached_days = use_signal(move || {
        let mut store = calendar::Store::new(config.peek().calendar.day_start_hour());
        store.cache_days_in(time_slay_core::config::cache_days_dir());
        store
    });
    // Navigation only changes the cached view. It never restarts the resource.
    use_effect(move || {
        let day = current_day();
        let today = day::today(&config.peek().calendar);
        cached_days.write().set_day(day, today);
    });

    let data = use_resource(move || {
        let refresh = revision();
        let requested_day = *current_day.peek();
        let settings = config.peek().calendar.clone();
        let initial_is_fresh = refresh == 0
            && !cached_days
                .peek()
                .sync_due(settings.sync_every(), chrono::Local::now());
        let cached = cached_days.peek().cached_snapshot();
        async move {
            if initial_is_fresh {
                return cached
                    .ok_or_else(|| "No cached calendar data yet. Refresh to load it.".to_owned());
            }
            if !day::signed_in() {
                return Err(
                    "Not connected to Google Calendar. Connect an account to load your day."
                        .to_owned(),
                );
            }
            tokio::task::spawn_blocking(move || day::load_day(requested_day, &settings))
                .await
                .map_err(|error| format!("The calendar worker stopped: {error}"))
                .and_then(|result| result)
        }
    });

    // Mark the document with the chosen theme. `system` leaves the decision to
    // the CSS `prefers-color-scheme` query, so it follows the OS live.
    use_effect(move || {
        let name = theme().name();
        let _ = document::eval(&format!(
            "document.documentElement.dataset.theme = '{name}';"
        ));
    });
    let cycle_theme = move |_| {
        let next = theme().next();
        theme.set(next);
        next.save();
    };

    // Keep the last good day on screen through a transient sync failure.
    use_effect(move || {
        let value = data.read();
        if let Some(Ok(snapshot)) = value.as_ref() {
            cached_days
                .write()
                .cache_fetched_snapshot(snapshot.clone(), &[snapshot.day]);
            signed_in.set(true);
            connect_error.set(None);
        }
    });

    // Pull again on the same cadence as the iced app, and roll the view over
    // to the new day at the configured boundary unless the user stepped away.
    use_future(move || async move {
        let mut last_refresh = tokio::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            if follow_today() {
                let today = day::today(&config.peek().calendar);
                if today != current_day() {
                    current_day.set(today);
                    selected_block.set(None);
                    scrolled.set(false);
                }
            }
            if last_refresh.elapsed() >= config.peek().calendar.sync_every() {
                last_refresh = tokio::time::Instant::now();
                revision.set(revision() + 1);
            }
        }
    });

    // Follow the config file. Only the calendar settings drive this view, so
    // anything else changing (the iced app saving its window size, say) does
    // not trigger a reload of the day.
    use_future(move || async move {
        let mut seen = config_sync::generation();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            let status = config_sync::status();
            if *config_status.peek() != status {
                config_status.set(status);
            }

            let generation = config_sync::generation();
            if generation != seen {
                seen = generation;
                let reloaded = day::load_config();
                if reloaded.calendar != config.peek().calendar {
                    config.set(reloaded);
                }
            }
        }
    });

    // Put the current time in view once, without yanking the scroll position
    // on every later refresh.
    use_effect(move || {
        // Retry when Calendar becomes visible if a day loaded in the Tasks tab.
        let _ = show_tasks();
        let pending = matches!(*data.state().read(), UseResourceState::Pending);
        let loaded_day = match data.read().as_ref() {
            Some(Ok(snapshot)) => Some(snapshot.day),
            _ => None,
        };
        let requested_day = current_day();
        if loaded_day == Some(requested_day) && !pending && !scrolled() {
            spawn(async move {
                if matches!(document::eval(SCROLL_SCRIPT).join::<bool>().await, Ok(true))
                    && current_day() == requested_day
                {
                    scrolled.set(true);
                }
            });
        }
    });

    let pending = matches!(*data.state().read(), UseResourceState::Pending);
    let resource = data.read();
    let fetch_error = match resource.as_ref() {
        Some(Err(error)) => Some(error.clone()),
        _ => None,
    };
    let requested_day = current_day();
    let snapshot = match resource.as_ref() {
        // A resource keeps the previous value while its next fetch is running.
        // Never paint yesterday's events under today's heading.
        Some(Ok(snapshot)) if snapshot.day == requested_day => Some(snapshot.clone()),
        _ => cached_days
            .read()
            .cached_snapshot()
            .filter(|snapshot| snapshot.day == requested_day),
    };
    drop(resource);

    let model = snapshot
        .as_ref()
        .map(|snapshot| day::build_model(snapshot, &settings));
    let selected_day = current_day();
    let today = day::today(&settings);

    let heading = calendar::day_heading(selected_day, today);
    let stats = model.as_ref().map(header_stats).unwrap_or_default();
    let status = if pending {
        "Syncing…".to_owned()
    } else if let Some(model) = &model {
        format!("Synced {}", calendar::clock_label(model.synced_at))
    } else if fetch_error.is_some() {
        "Could not sync".to_owned()
    } else if !signed_in() {
        "Not connected".to_owned()
    } else {
        "Not synced yet".to_owned()
    };

    let config_summary = config_status().summary();
    let config_detail = match config_status() {
        config_sync::Status::Failed(error) => error,
        _ => "The config is kept in Google Drive's hidden app folder".to_owned(),
    };

    let prev_day = move |_| {
        current_day.set(day::shift_day(current_day(), -1));
        selected_block.set(None);
        scrolled.set(false);
        follow_today.set(false);
    };
    let next_day = move |_| {
        current_day.set(day::shift_day(current_day(), 1));
        selected_block.set(None);
        scrolled.set(false);
        follow_today.set(false);
    };
    let today_settings = settings.clone();
    let go_today = move |_| {
        current_day.set(day::today(&today_settings));
        selected_block.set(None);
        scrolled.set(false);
        follow_today.set(true);
    };
    let refresh = move |_| revision.set(revision() + 1);

    let topbar = rsx! {
        header { class: "topbar",
            div { class: "brand",
                div { class: "brand-mark", "⏳" }
                div {
                    div { class: "brand-name", "TimeSlay" }
                    div { class: "brand-sub", "Day view" }
                }
            }
            div { class: "day-titles",
                h1 { class: "day-title", "{heading}" }
                div { class: "day-sub", "{selected_day.year()} · {stats}" }
            }
            div { class: "spacer" }
            div { class: "sync",
                div { class: "sync-label", "{status}" }
                div { class: "sync-detail", title: "{config_detail}", "Google Calendar + Google Tasks · {config_summary}" }
            }
            div { class: "controls",
                button {
                    class: "btn theme-toggle",
                    title: "Theme: {theme().label()} (click to change)",
                    onclick: cycle_theme,
                    span { class: "theme-icon", "{theme().icon()}" }
                    span { class: "theme-label", "{theme().label()}" }
                }
                button { class: "btn icon", title: "Previous day", onclick: prev_day, "‹" }
                button { class: "btn", onclick: go_today, "Today" }
                button { class: "btn icon", title: "Next day", onclick: next_day, "›" }
                button {
                    class: "btn primary",
                    title: "Refresh now",
                    onclick: refresh,
                    "⟳ Refresh"
                }
            }
        }
    };

    let banner_connect_settings = settings.clone();
    let banner = fetch_error.as_ref().map(|error| {
        let error = error.clone();
        let banner_connect_error = connect_error().map(|connect_error| {
            rsx! { span { class: "banner-detail", "{connect_error}" } }
        });
        rsx! {
            div { class: "banner error",
                span { class: "banner-text", "Sync failed: {error}" }
                {banner_connect_error}
                div { class: "spacer" }
                button {
                    class: "btn small",
                    onclick: move |_| {
                        let next = revision() + 1;
                        revision.set(next);
                    },
                    "Retry"
                }
                button {
                    class: "btn small",
                    onclick: move |_| {
                        begin_connect(
                            banner_connect_settings.clone(),
                            connecting,
                            connect_error,
                            signed_in,
                            revision,
                        );
                    },
                    if connecting() { "Connecting…" } else { "Connect Google" }
                }
            }
        }
    });

    let notice = model.as_ref().and_then(|model| {
        model.notice.as_ref().map(|notice| {
            rsx! {
                div { class: "banner warn",
                    span { class: "banner-text", "{notice}" }
                }
            }
        })
    });

    let body: Element = match &model {
        Some(model) => rsx! {
            nav { class: "view-tabs", aria_label: "Day view",
                button {
                    class: "view-tab",
                    aria_pressed: (!show_tasks()).to_string(),
                    aria_controls: "calendar-pane",
                    onclick: move |_| show_tasks.set(false),
                    "Calendar"
                }
                button {
                    class: "view-tab",
                    aria_pressed: show_tasks().to_string(),
                    aria_controls: "tasks-pane",
                    onclick: move |_| show_tasks.set(true),
                    "Tasks"
                }
            }
            div { class: "day-body", "data-view": if show_tasks() { "tasks" } else { "calendar" },
                {sidebar(model, selected_block)}
                {timeline(model, selected_block)}
            }
        },
        None if settings.client_id.trim().is_empty() && !pending => {
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
                                        begin_connect(
                                            saved_settings,
                                            connecting,
                                            connect_error,
                                            signed_in,
                                            revision,
                                        );
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
        None => {
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
                                "The configuration lives at {config_hint}."
                            }
                            div { class: "state-actions",
                                button {
                                    class: "btn primary",
                                    onclick: move |_| {
                                        begin_connect(
                                            state_connect_settings.clone(),
                                            connecting,
                                            connect_error,
                                            signed_in,
                                            revision,
                                        );
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
    };

    let modal = model.as_ref().and_then(|model| {
        let id = selected_block()?;
        let block = model.blocks.iter().find(|block| block.event.id == id)?;
        Some(time_block_modal(block, selected_block))
    });

    rsx! {
        document::Title { "TimeSlay · Day" }
        document::Style { {CSS} }
        div { class: "app",
            {topbar}
            {banner}
            {notice}
            {body}
            {modal}
        }
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

fn header_stats(model: &DayModel) -> String {
    let events = model.all_day.len()
        + model
            .timed
            .iter()
            .filter(|event| !event.event.is_task() && !event.time_block)
            .count();
    let tasks: usize = model
        .task_groups
        .iter()
        .map(|group| group.tasks.len())
        .sum();
    let blocks = model.blocks.len();

    let event_word = if events == 1 { "event" } else { "events" };
    let task_word = if tasks == 1 { "task" } else { "tasks" };
    let block_word = if blocks == 1 { "block" } else { "blocks" };
    format!("{events} {event_word} · {tasks} {task_word} · {blocks} {block_word}")
}

fn sidebar(model: &DayModel, open_block: Signal<Option<String>>) -> Element {
    let all_day_empty = model.all_day.is_empty();
    let groups_empty = model.task_groups.is_empty();

    let all_day_chips: Vec<Element> = model.all_day.iter().map(all_day_chip).collect();
    let groups: Vec<Element> = model
        .task_groups
        .iter()
        .map(|group| task_group_view(group, open_block))
        .collect();

    rsx! {
        aside { id: "tasks-pane", class: "sidebar", aria_label: "Tasks",
            div { class: "sidebar-scroll",
                section { class: "panel-section sidebar-all-day",
                    div { class: "section-head",
                        span { class: "section-title", "All day" }
                        span { class: "count-pill", "{model.all_day.len()}" }
                    }
                    if all_day_empty {
                        p { class: "empty-note", "No all-day events." }
                    } else {
                        div { class: "all-day-list", {all_day_chips.into_iter()} }
                    }
                }
                section { class: "panel-section",
                    div { class: "section-head",
                        span { class: "section-title", "Google Tasks" }
                        span {
                            class: "count-pill",
                            "{model.task_groups.iter().map(|group| group.tasks.len()).sum::<usize>()}"
                        }
                    }
                    if model.tasks_hidden {
                        p { class: "empty-note",
                            "Google Tasks are hidden in the calendar settings. Tasks inside time blocks still appear when you open a block."
                        }
                    } else if groups_empty {
                        p { class: "empty-note", "No Google Tasks for this day." }
                    } else {
                        div { class: "task-groups", {groups.into_iter()} }
                    }
                }
            }
        }
    }
}

fn all_day_chip(event: &Event) -> Element {
    let dot = css_color(event.color);
    let title = event.title.clone();
    let completed = event.completed.then(|| {
        rsx! { span { class: "all-day-check", "✓" } }
    });
    rsx! {
        div { class: "all-day-chip", title: "{title}",
            span { class: "day-dot", style: "background: {dot};" }
            span { class: "all-day-title", "{title}" }
            {completed}
        }
    }
}

fn task_group_view(group: &TaskGroup, open_block: Signal<Option<String>>) -> Element {
    let dot = group
        .tasks
        .first()
        .map(|task| css_color(task.event.color))
        .unwrap_or_else(|| "rgba(84,132,237,1)".to_owned());
    let open = group
        .tasks
        .iter()
        .filter(|task| task.status == tb::TaskStatus::Active)
        .count();
    let total = group.tasks.len();
    let list_name = group.list_name.clone();
    let tasks: Vec<Element> = group
        .tasks
        .iter()
        .map(|task| task_row_view(&list_name, task, open_block))
        .collect();

    rsx! {
        div { class: "task-group",
            div { class: "task-list-name",
                span { class: "day-dot", style: "background: {dot};" }
                span { "{list_name}" }
                span { class: "task-list-count", "{open} open / {total}" }
            }
            div { class: "task-rows", {tasks.into_iter()} }
        }
    }
}

fn task_row_view(list_name: &str, task: &TaskRow, open_block: Signal<Option<String>>) -> Element {
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
    let icon = status_icon(task.status);
    let title = task.event.title.clone();
    let tooltip = format!("{title}\n{list_name}");
    let minutes = (task.minutes > 0).then(|| day::duration_label(task.minutes));
    let block = task
        .block
        .as_ref()
        .map(|block| block_link_view(block, open_block));
    let status_note = match task.status {
        tb::TaskStatus::Active => None,
        tb::TaskStatus::Completed => Some("Completed"),
        tb::TaskStatus::Ignored => Some("Ignored"),
    }
    .map(|note| rsx! { span { class: "meta-pill", "{note}" } });
    let minutes = minutes.map(|minutes| rsx! { span { class: "meta-pill", "{minutes}" } });

    rsx! {
        div { class: "task-row", title: "{tooltip}",
            span { class: "task-status {status_class}", "{icon}" }
            div { class: "task-main",
                div { class: "{title_class}", "{title}" }
                div { class: "task-meta",
                    {status_note}
                    {minutes}
                    {block}
                }
            }
        }
    }
}

fn block_link_view(block: &BlockRef, open_block: Signal<Option<String>>) -> Element {
    let mut open = open_block;
    let id = block.event_id.clone();
    let label = format!("◧ {} · {}", block.title, block.time);
    rsx! {
        button {
            class: "block-link",
            title: "Open this time block",
            onclick: move |_| open.set(Some(id.clone())),
            "{label}"
        }
    }
}

fn timeline(model: &DayModel, open_block: Signal<Option<String>>) -> Element {
    let grid_height = 24.0 * HOUR_HEIGHT;
    let hours: Vec<Element> = (0..24u32)
        .map(|row| {
            let top = row as f32 * HOUR_HEIGHT;
            let label_top = if row == 0 { top + 2.0 } else { top - 7.0 };
            let label = calendar::hour_label(day::hour_at(model.hour_start, row));
            rsx! {
                span {
                    key: "label-{row}",
                    class: "hour-label",
                    style: "top: {label_top}px;",
                    "{label}"
                }
            }
        })
        .collect();
    let lines: Vec<Element> = (0..24u32)
        .map(|row| {
            let top = row as f32 * HOUR_HEIGHT;
            rsx! {
                div { key: "line-{row}", class: "hour-line", style: "top: {top}px;" }
            }
        })
        .collect();
    let events: Vec<Element> = model
        .timed
        .iter()
        .map(|timed| timed_event_view(timed, open_block))
        .collect();

    let now_line = model.now_min.map(|minutes| {
        let top = minutes as f32 * HOUR_HEIGHT / 60.0;
        rsx! {
            div { id: "now-line", class: "now-line", style: "top: {top}px;",
                span { class: "now-dot" }
            }
        }
    });
    let empty = model.timed.is_empty().then(|| {
        rsx! {
            div { class: "timeline-empty",
                span { "Nothing scheduled on the grid for this day." }
            }
        }
    });
    let anchor_top = 8.0 * HOUR_HEIGHT;

    let count = model.timed.len();

    rsx! {
        section { id: "calendar-pane", class: "timeline-pane", aria_label: "Calendar",
            div { class: "timeline-head",
                h2 { "Schedule" }
                span { class: "hint", "Click a time block to see every task assigned to it." }
                div { class: "spacer" }
                span { class: "timeline-count", "{count} on the grid" }
            }
            div { class: "timeline-scroll",
                if !model.all_day.is_empty() {
                    section { class: "calendar-all-day panel-section",
                        div { class: "section-head",
                            span { class: "section-title", "All day" }
                            span { class: "count-pill", "{model.all_day.len()}" }
                        }
                        div { class: "all-day-list",
                            for event in &model.all_day {
                                {all_day_chip(event)}
                            }
                        }
                    }
                }
                div { class: "timeline", style: "height: {grid_height}px;",
                    div { class: "gutter", style: "height: {grid_height}px;", {hours.into_iter()} }
                    div { class: "grid", style: "height: {grid_height}px;",
                        {lines.into_iter()}
                        {events.into_iter()}
                        {now_line}
                        {empty}
                        div {
                            id: "timeline-anchor",
                            class: "anchor",
                            style: "top: {anchor_top}px;"
                        }
                    }
                }
            }
        }
    }
}

fn timed_event_view(timed: &TimedEvent, open_block: Signal<Option<String>>) -> Element {
    let id = timed.event.id.clone();
    let title = timed.title.clone();
    let time = calendar::event_time_label(&timed.event);
    let calendar_name = timed.calendar_name.clone();
    let style = event_style(timed);
    let tooltip = format!("{title}\n{time}\n{calendar_name}");
    let task_badge = timed.time_block.then(|| {
        let count = if timed.task_count == 1 {
            "1 task".to_owned()
        } else {
            format!("{} tasks", timed.task_count)
        };
        if timed.planned_minutes == 0 {
            count
        } else {
            format!("{count} · {}", day::duration_label(timed.planned_minutes))
        }
    });
    let badge = match task_badge {
        Some(badge) => rsx! { div { class: "block-badge", "{badge}" } },
        None => rsx! { div { class: "event-cal", "{calendar_name}" } },
    };

    if timed.time_block {
        let mut open = open_block;
        rsx! {
            button {
                key: "{id}",
                class: "grid-event block",
                style: "{style}",
                title: "{tooltip}",
                onclick: move |_| open.set(Some(id.clone())),
                div { class: "event-title", "{title}" }
                div { class: "event-time", "{time}" }
                {badge}
            }
        }
    } else {
        rsx! {
            div {
                key: "{id}",
                class: "grid-event",
                style: "{style}",
                title: "{tooltip}",
                div { class: "event-title", "{title}" }
                div { class: "event-time", "{time}" }
                div { class: "event-cal", "{calendar_name}" }
            }
        }
    }
}

fn time_block_modal(block: &TimeBlock, close: Signal<Option<String>>) -> Element {
    let mut backdrop_close = close;
    let mut button_close = close;
    let title = block.event.title.clone();
    let time = calendar::event_time_label(&block.event);
    let accent = css_color(block.event.color);
    let planned = block.planned_minutes as i64;
    let capacity = block.capacity_minutes.max(1);
    let pct = (planned as f32 / capacity as f32 * 100.0).clamp(0.0, 100.0);
    let over_by = (planned - capacity).max(0);
    let fill_class = if over_by > 0 {
        "capacity-fill over"
    } else {
        "capacity-fill"
    };
    let planned_label = if block.planned_minutes == 0 {
        "0m".to_owned()
    } else {
        day::duration_label(block.planned_minutes)
    };
    let summary = if over_by > 0 {
        format!(
            "{planned_label} planned of {} · over by {}",
            day::capacity_label(block.capacity_minutes),
            day::duration_label(over_by as u32)
        )
    } else {
        format!(
            "{planned_label} planned of {}",
            day::capacity_label(block.capacity_minutes)
        )
    };
    let task_word = if block.tasks.len() == 1 {
        "task"
    } else {
        "tasks"
    };
    let tasks: Vec<Element> = block.tasks.iter().map(assigned_task_row).collect();
    let no_tasks = block.tasks.is_empty();

    rsx! {
        // Close on the press that starts outside the sheet. Using pointerdown
        // (rather than click) means the tap that opened the sheet cannot also
        // close it, which matters on Android where a synthesized click can
        // arrive after the modal has mounted.
        div { class: "modal-backdrop", onpointerdown: move |_| backdrop_close.set(None),
            div {
                class: "modal",
                onpointerdown: move |event| event.stop_propagation(),
                onclick: move |event| event.stop_propagation(),
                div { class: "modal-head",
                    div { class: "modal-mark", style: "background: {accent};" }
                    div { class: "modal-titles",
                        h2 { class: "modal-title", "{title}" }
                        div { class: "modal-sub", "{time}" }
                    }
                    button {
                        class: "modal-close",
                        title: "Close",
                        onclick: move |_| button_close.set(None),
                        "×"
                    }
                }
                div { class: "modal-body",
                    div { class: "block-summary",
                        div { class: "summary-line",
                            span { "{summary}" }
                            span { class: "summary-tasks", "{block.tasks.len()} {task_word}" }
                        }
                        div { class: "capacity",
                            div { class: "{fill_class}", style: "width: {pct}%;" }
                        }
                    }
                    div { class: "assigned-head",
                        span { class: "section-title", "Assigned tasks" }
                        span { class: "count-pill", "{block.tasks.len()}" }
                    }
                    if no_tasks {
                        div { class: "modal-empty",
                            p { "No tasks are assigned to this time block yet." }
                            span { "Assign tasks to it from the iced app's block view." }
                        }
                    } else {
                        div { class: "assigned-list", {tasks.into_iter()} }
                    }
                }
            }
        }
    }
}

fn assigned_task_row(task: &AssignedTask) -> Element {
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
    let icon = status_icon(task.status);
    let title = task.event.title.clone();
    let list_name = task.list_name.clone();
    let minutes = day::duration_label(task.minutes);
    let status_note = match task.status {
        tb::TaskStatus::Active => "Open",
        tb::TaskStatus::Completed => "Completed",
        tb::TaskStatus::Ignored => "Ignored",
    };

    rsx! {
        div { class: "assigned-task",
            span { class: "task-status {status_class}", "{icon}" }
            div { class: "task-main",
                div { class: "{title_class}", "{title}" }
                div { class: "task-meta",
                    span { class: "meta-pill", "{list_name}" }
                    span { class: "meta-pill", "{status_note}" }
                }
            }
            span { class: "assigned-minutes", "{minutes}" }
        }
    }
}

fn status_icon(status: tb::TaskStatus) -> &'static str {
    match status {
        tb::TaskStatus::Active => "○",
        tb::TaskStatus::Completed => "✓",
        tb::TaskStatus::Ignored => "⊘",
    }
}

fn event_style(timed: &TimedEvent) -> String {
    let top = timed.start_min as f32 * HOUR_HEIGHT / 60.0;
    let height = ((timed.end_min - timed.start_min) as f32 * HOUR_HEIGHT / 60.0).max(22.0);
    let width = 100.0 / timed.columns.max(1) as f32;
    let left = timed.column as f32 * width;
    let background = css_color(timed.event.color);
    let ink = css_color(calendar::colors::on_color(timed.event.color));

    format!(
        "top:{top:.1}px;height:{height:.1}px;left:calc({left:.4}% + 3px);width:calc({width:.4}% - 6px);background:{background};color:{ink};"
    )
}

fn css_color(color: time_slay_core::Color) -> String {
    let red = (color.r.clamp(0.0, 1.0) * 255.0).round() as u8;
    let green = (color.g.clamp(0.0, 1.0) * 255.0).round() as u8;
    let blue = (color.b.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "rgba({red}, {green}, {blue}, {:.2})",
        color.a.clamp(0.0, 1.0)
    )
}
