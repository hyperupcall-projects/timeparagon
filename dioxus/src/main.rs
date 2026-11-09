//! TimeSlay's Dioxus app.
//!
//! The navigation is the desktop's: Do is the day's calendar, Plan plans a
//! day, week, two weeks or month with the Overdue, Inbox, Today and Backlog
//! lists beneath it, Note opens the notepad, More the other pages, and
//! Settings the settings. Tapping an item opens a sheet that edits it; a time
//! block first lists its assigned tasks.

mod agenda;
mod create;
mod day;
mod editor;
mod habits;
mod habits_charts;
mod notes;
mod people;
mod plan;
mod reminders;
mod scratch;
mod search;
mod style;
#[cfg(any(target_os = "android", test))]
mod widget;

use chrono::{Datelike, Months, NaiveDate};
use dioxus::prelude::*;
use timeslay_core::calendar::first_day::FirstDayOfWeek;
use timeslay_core::calendar::search::Search;
use timeslay_core::calendar::time_format::{self, TimeFormat};
use timeslay_core::calendar::{self, Event};
use timeslay_core::config::CalendarSettings;
use timeslay_core::config_watch;
use timeslay_core::theme as look;
use timeslay_core::timeblocking as tb;

use day::{DayModel, EditContext, Theme, TimeBlock, TimedEvent};
use plan::{PlanTab, PlanView};

/// Height of one hour on the timeline, in pixels.
const HOUR_HEIGHT: f32 = 60.0;
/// The shortest an event is drawn, so a finger can land on it.
const EVENT_MIN_HEIGHT: f32 = 22.0;
/// The line heights of an event's title and time in the stylesheet.
const TITLE_LINE: f32 = 16.0;
const TIME_LINE: f32 = 15.0;

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

#[cfg(not(target_os = "android"))]
fn main() {
    config_watch::start();
    timeslay_core::database_sync::start();
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
                .with_background_color(window_background()),
        )
        .launch(App);
}

/// What shows before the first paint: the default light scheme's page color.
#[cfg(not(target_os = "android"))]
fn window_background() -> (u8, u8, u8, u8) {
    let [red, green, blue, alpha] = look::Scheme::Light
        .extended()
        .background
        .base
        .color
        .into_rgba8();
    (red, green, blue, alpha)
}

#[cfg(target_os = "android")]
fn main() {
    prepare_android_storage();
    config_watch::start();
    timeslay_core::database_sync::start();
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
    let state = files.join("state");
    for directory in [&config, &data, &state, &cache] {
        let _ = std::fs::create_dir_all(directory);
    }

    // SAFETY: this runs before the Dioxus runtime starts any threads.
    unsafe {
        std::env::set_var("HOME", &files);
        std::env::set_var("XDG_CONFIG_HOME", &config);
        std::env::set_var("XDG_DATA_HOME", &data);
        std::env::set_var("XDG_STATE_HOME", &state);
        std::env::set_var("XDG_CACHE_HOME", &cache);
        // The Rust sync client uses rustls-native-certs. Its Unix defaults
        // probe Termux, so point it at Android's public system trust store.
        if std::env::var_os("SSL_CERT_DIR").is_none() {
            let certificates: Vec<_> = [
                "/apex/com.android.conscrypt/cacerts",
                "/system/etc/security/cacerts",
            ]
            .into_iter()
            .filter(|path| std::fs::read_dir(path).is_ok())
            .collect();
            if !certificates.is_empty() {
                if let Ok(paths) = std::env::join_paths(certificates) {
                    std::env::set_var("SSL_CERT_DIR", paths);
                }
            }
        }
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
    // Local settings can change under the view when the iced app saves them.
    let mut config = use_signal(day::load_config);
    let settings = config.read().calendar.clone();
    let config_hint = timeslay_core::config::config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "config.toml".to_owned());

    let mut current_day = use_signal(|| day::today(&settings));
    let mut revision = use_signal(|| 0u32);
    let mut selected_block = use_signal(|| None::<String>);
    let mut editing = use_signal(|| None::<Event>);
    let block_writing = use_signal(|| false);
    // Where the navigation is: Do, Plan (with its period and list) or Habits.
    let mut page = use_signal(|| Page::Do);
    let plan_view = use_signal(|| PlanView::Day);
    let plan_tab = use_signal(|| PlanTab::Overdue);
    let mut plan_menu_open = use_signal(|| false);
    // The day the range being planned holds.
    let mut week_anchor = use_signal(|| *current_day.peek());
    let month_open = use_signal(|| false);
    let month_shown = use_signal(|| calendar::first_of_month(*current_day.peek()));
    let range_month_open = use_signal(|| false);
    let range_month_shown = use_signal(|| calendar::first_of_month(*current_day.peek()));
    let mut show_actions = use_signal(|| false);
    let mut scrolled = use_signal(|| false);
    let follow_today = use_signal(|| true);
    let connecting = use_signal(|| false);
    let mut connect_error = use_signal(|| None::<String>);
    let mut theme = use_signal(Theme::load);
    let mut client_id_input = use_signal(String::new);
    let mut client_secret_input = use_signal(String::new);
    let mut client_error = use_signal(|| None::<String>);
    let mut show_settings = use_signal(|| false);
    let mut settings_id = use_signal(String::new);
    let mut settings_secret = use_signal(String::new);
    let mut settings_error = use_signal(|| None::<String>);
    // What is typed in the sync URL field, kept apart from the saved config so
    // re-renders (the sync status poll, say) never overwrite it mid-edit.
    let mut sync_url_input = use_signal(String::new);
    let mut sync_url_error = use_signal(|| None::<String>);
    let mut sync_url_saved = use_signal(|| false);
    // The View sheet, open on Calendars or Filter.
    let mut view_sheet = use_signal(|| None::<ViewTab>);
    // A habit's events, which "Show on calendar" opens the agenda on.
    let mut agenda_filter = use_signal(|| None::<habits::CalendarFilter>);
    let mut show_notes = use_signal(|| false);
    let mut show_create = use_signal(|| false);
    let mut show_agenda = use_signal(|| false);
    let mut show_scratch = use_signal(|| None::<scratch::Page>);
    let mut show_more = use_signal(|| false);
    let mut show_people = use_signal(|| false);
    // Kept while the app runs, so the sheet reopens on what it showed.
    let people_store = use_signal(people::Store::live);
    let mut show_operations = use_signal(|| false);
    let mut google_operations = use_signal(Vec::<calendar::operations::Operation>::new);
    calendar::operations::start();
    let mut database_sync_status = use_signal(timeslay_core::database_sync::status);
    let search_state = use_signal(Search::default);
    // A note's connection or an agenda row, waiting for its day to load.
    let mut pending_open = use_signal(|| None::<String>);
    let notes_store = use_context_provider(notes::NotesStore::new);
    let note_opening = use_signal(|| None::<String>);
    use_context_provider(|| notes::NoteOpening(note_opening));
    let mut calendars_error = use_signal(|| None::<String>);
    let mut signed_in = use_signal(day::signed_in);
    let mut cached_days = use_signal(move || {
        let mut store = calendar::Store::new(config.peek().calendar.day_start_hour());
        store.cache_days_in(timeslay_core::task_db::path());
        store
    });
    // Paint cached data immediately, fetching missing or empty days on navigation.
    use_effect(move || {
        let day = current_day();
        let today = day::today(&config.peek().calendar);
        cached_days.write().set_day(day, today);
        if !cached_days.peek().has_calendar_events_on(day) {
            let next = *revision.peek() + 1;
            revision.set(next);
        }
    });

    let data = use_resource(move || {
        let refresh = revision();
        let requested_day = *current_day.peek();
        let settings = config.peek().calendar.clone();
        let initial_is_fresh = refresh == 0
            && !cached_days
                .peek()
                .sync_due(settings.sync_every(), timeslay_core::clock::now());
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
    let choose_theme = move |event: FormEvent| {
        let chosen = Theme::parse(&event.value());
        theme.set(chosen);
        chosen.save();
        // The widget reads the same preference.
        #[cfg(target_os = "android")]
        widget::notify();
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

    // Re-plan reminders from the cache the effect above just wrote, whenever
    // a day loads or the settings change. Planning reads the database, so it
    // runs on a worker thread.
    #[cfg(target_os = "android")]
    use_effect(move || {
        let _ = data.read();
        reminders::republish(&config.read());
    });

    // Pull again on the same cadence as the iced app, and roll the view over
    // to the new day at the configured boundary unless the user stepped away.
    use_future(move || async move {
        let mut last_refresh = tokio::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            // Rolling over would take the day out from under an open editor.
            if follow_today()
                && editing.peek().is_none()
                && !*show_create.peek()
                && !*block_writing.peek()
            {
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

    // Follow the config file. Only the calendar settings, projects and places
    // drive this view, so anything else changing (the iced app saving its
    // window size, say) does not trigger a re-render.
    use_future(move || async move {
        let mut seen = (
            config_watch::generation(),
            timeslay_core::database_sync::generation(),
        );
        // The queue is only read again when the database says it changed,
        // which is one small query instead of a full read, and the notes only
        // when their directory's fingerprint moved.
        let mut changes: Option<timeslay_core::local_changes::Counters> = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            database_sync_status.set(timeslay_core::database_sync::status());
            let counters = timeslay_core::task_db::path()
                .and_then(|path| timeslay_core::local_changes::read(&path).ok());
            let moved = |part: fn(&timeslay_core::local_changes::Counters) -> i64| {
                counters.is_none_or(|now| changes.is_none_or(|seen| part(&seen) != part(&now)))
            };
            if moved(|c| c.queue)
                && let Ok(rows) = calendar::operations::list()
            {
                google_operations.set(rows);
            }
            notes_store.reload_shared();
            changes = counters;
            let generation = (
                config_watch::generation(),
                timeslay_core::database_sync::generation(),
            );
            if generation != seen {
                seen = generation;
                let reloaded = day::load_config();
                if reloaded != *config.peek() {
                    config.set(reloaded);
                }
            }
        }
    });

    // Put the current time in view once, without yanking the scroll position
    // on every later refresh.
    use_effect(move || {
        // Retry when the grid comes into view, if the day loaded on another page.
        let _ = page();
        let _ = plan_view();
        let _ = plan_tab();
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

    // Follow a note's connection or an agenda row once its day is here. Only a
    // finished fetch of that day can say it is gone: a cached copy may simply
    // be older than the event.
    use_effect(move || {
        let Some(id) = pending_open() else {
            return;
        };
        let day = current_day();
        let fetching = matches!(*data.state().read(), UseResourceState::Pending);
        let fetched = match data.read().as_ref() {
            Some(Ok(snapshot)) if snapshot.day == day => Some(snapshot.clone()),
            _ => None,
        };
        let known = fetched.clone().or_else(|| {
            cached_days
                .read()
                .cached_snapshot()
                .filter(|snapshot| snapshot.day == day)
        });
        let found =
            known.and_then(|snapshot| snapshot.events.into_iter().find(|event| event.id == id));
        if let Some(event) = found {
            pending_open.set(None);
            editing.set(Some(event));
        } else if fetched.is_some() && !fetching {
            pending_open.set(None);
            let mut message = notes_store.error;
            message.set(Some("That is no longer on its day.".to_owned()));
            show_notes.set(true);
        }
    });

    let pending = matches!(*data.state().read(), UseResourceState::Pending);
    let resource = data.read();
    let fetch_error = match resource.as_ref() {
        Some(Err(error)) => Some(error.clone()),
        _ => None,
    };
    let requested_day = current_day();
    let mut snapshot = match resource.as_ref() {
        // A resource keeps the previous value while its next fetch is running.
        // Never paint yesterday's events under today's heading.
        Some(Ok(snapshot)) if snapshot.day == requested_day => Some(snapshot.clone()),
        _ => cached_days
            .read()
            .cached_snapshot()
            .filter(|snapshot| snapshot.day == requested_day),
    };
    drop(resource);
    if let (Some(snapshot), Some(path)) = (&mut snapshot, timeslay_core::task_db::path())
        && let Err(error) =
            calendar::operations::overlay(&path, &mut snapshot.events, Some(snapshot.fetched_at))
    {
        eprintln!("Could not load local edits: {error}");
    }

    let model = snapshot
        .as_ref()
        .map(|snapshot| day::build_model(snapshot, &settings));
    let travel = model
        .as_ref()
        .map(|model| {
            day::travel_boxes(
                model,
                &config.read().travel,
                config.read().appearance.distance_unit,
            )
        })
        .unwrap_or_default();
    let selected_day = current_day();
    let today = day::today(&settings);
    #[cfg(target_os = "android")]
    if let (Some(model), Some(snapshot)) = (&model, &snapshot) {
        widget::publish(model, &travel, today, &settings, snapshot.fetched_at);
    }

    // Only what is drawn is filtered. The widget above keeps the whole day, and
    // trips join up the whole day's events, so they go while a search is on.
    let searching = search_state.read().is_active();
    let (model, travel) = if searching {
        let filtered = snapshot.as_ref().map(|snapshot| {
            day::build_model(
                &day::filter_snapshot(snapshot, &search_state.read(), &settings),
                &settings,
            )
        });
        (filtered, Vec::new())
    } else {
        (model, travel)
    };

    let time_format = config.read().appearance.time_format;
    let day_nav = DayNav {
        current_day,
        selected_block,
        scrolled,
        follow_today,
        pending_open,
    };
    let plan_nav = PlanNav {
        page,
        view: plan_view,
        tab: plan_tab,
        menu_open: plan_menu_open,
        week_anchor,
        current_day,
    };
    let mut open_settings = move || {
        settings_id.set(config.peek().calendar.client_id.clone());
        settings_secret.set(String::new());
        settings_error.set(None);
        sync_url_input.set(config.peek().database_sync.url.clone());
        sync_url_error.set(None);
        sync_url_saved.set(false);
        show_more.set(false);
        plan_menu_open.set(false);
        show_settings.set(true);
    };

    let planning_range = page() == Page::Plan && plan_view().range().is_some();
    let plan_active = plan_menu_open() || page() == Page::Plan;
    let more_active = show_more()
        || page() == Page::Habits
        || show_notes()
        || show_operations()
        || matches!(show_scratch(), Some(scratch::Page::BrainDump));
    let plan_tabs = PlanTab::ALL
        .into_iter()
        .filter(|tab| planning_range || *tab != PlanTab::Schedule)
        .map(|tab| {
            let mut plan_tab = plan_tab;
            nav_tab(tab.label(), plan_tab() == tab, "", move |_| {
                plan_menu_open.set(false);
                plan_tab.set(tab);
            })
        });
    let plan_menu = plan_menu_open().then(|| {
        rsx! {
            div { class: "menu-dismiss", onclick: move |_| plan_menu_open.set(false) }
            div { class: "plan-menu", role: "menu", aria_label: "Plan",
                for view in PlanView::ALL {
                    button {
                        key: "{view.menu_label()}",
                        class: "nav-tab",
                        role: "menuitemradio",
                        aria_pressed: (plan_view() == view).to_string(),
                        onclick: move |_| plan_nav.choose(view),
                        "{view.menu_label()}"
                    }
                }
            }
        }
    });

    // One card at the top, as on the desktop: Do and Plan, then Note, More
    // and Settings, with Plan's lists on a second row while planning.
    let topbar = rsx! {
        header { class: "nav-card",
            div { class: "nav-row",
                {nav_tab("Do", page() == Page::Do, "", move |_| {
                    plan_menu_open.set(false);
                    page.set(Page::Do);
                })}
                div { class: if plan_active { "plan-switch active" } else { "plan-switch" },
                    button {
                        class: "plan-main",
                        aria_pressed: (page() == Page::Plan).to_string(),
                        onclick: move |_| plan_nav.choose(plan_view()),
                        "{plan_view().label()}"
                    }
                    button {
                        class: "plan-toggle",
                        title: "Choose what to plan",
                        aria_label: "Choose what to plan",
                        aria_expanded: plan_menu_open().to_string(),
                        onclick: move |_| plan_menu_open.set(!plan_menu_open()),
                        "▾"
                    }
                }
                div { class: "spacer" }
                {nav_tab("Note", show_scratch() == Some(scratch::Page::Notepad), "", move |_| {
                    plan_menu_open.set(false);
                    show_scratch.set(Some(scratch::Page::Notepad));
                })}
                {nav_tab("More", more_active, "", move |_| {
                    plan_menu_open.set(false);
                    show_more.set(true);
                })}
                button {
                    class: "nav-tab",
                    aria_pressed: show_settings().to_string(),
                    onclick: move |_| open_settings(),
                    span { class: "label-long", "Settings" }
                    span { class: "label-short", "Set" }
                }
            }
            if page() == Page::Plan {
                div { class: "nav-row plan-tabs", {plan_tabs} }
            }
            {plan_menu}
        }
    };

    // View (Calendars and Filter) and Actions (Sync now), beside the picker.
    let view_lit = view_sheet().is_some() || searching;
    let header_buttons = move || {
        rsx! {
            button {
                class: if view_lit { "btn primary" } else { "btn" },
                aria_expanded: view_sheet().is_some().to_string(),
                onclick: move |_| {
                    calendars_error.set(None);
                    show_actions.set(false);
                    view_sheet.set(Some(ViewTab::Calendars));
                },
                "View"
            }
            div { class: "month-anchor",
                button {
                    class: if show_actions() { "btn primary" } else { "btn" },
                    aria_expanded: show_actions().to_string(),
                    onclick: move |_| show_actions.set(!show_actions()),
                    "Actions"
                }
                if show_actions() {
                    div { class: "month-dismiss", onclick: move |_| show_actions.set(false) }
                    div { class: "actions-menu", role: "menu", aria_label: "Actions",
                        button {
                            class: "btn",
                            disabled: pending || !signed_in(),
                            onclick: move |_| {
                                show_actions.set(false);
                                revision.set(revision() + 1);
                            },
                            if pending { "Syncing…" } else { "Sync now" }
                        }
                    }
                }
            }
        }
    };
    let status = move || {
        rsx! {
            if google_operations().iter().any(|operation| operation.pending()) {span { class: "hint", role: "status", "Saving…" }}
            if google_operations().iter().any(|operation| !operation.pending()) {span { class: "hint", role: "status", "Some changes could not be saved" }}
            if pending {
                p { class: "day-loading", "Loading..." }
            }
        }
    };
    // The day's picker is labelled with the day in view, and is the way to
    // move between days.
    // Planning a day keeps the grid beside its lists where there is room.
    let day_header_class = if page() == Page::Plan {
        "wide-only"
    } else {
        ""
    };
    let day_header = {
        rsx! {
            div { class: "day-header {day_header_class}",
                div { class: "day-controls",
                    {month_picker(
                        calendar::month_day_label(selected_day),
                        selected_day,
                        Vec::new(),
                        selected_day == today,
                        today,
                        settings.first_day_of_week,
                        month_open,
                        month_shown,
                        Pick::Day(day_nav),
                    )}
                    {header_buttons()}
                }
                {status()}
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
        Some(model) => {
            // Planning a day shows its list, with the grid beside it only
            // where there is room; Do shows the grid alone.
            let lists = (page() == Page::Plan)
                .then_some(snapshot.as_ref())
                .flatten()
                .map(|snapshot| {
                    task_list(
                        plan_tab(),
                        snapshot,
                        config,
                        signed_in(),
                        revision,
                        editing,
                        show_create,
                    )
                });
            let view = if lists.is_some() { "tasks" } else { "calendar" };
            rsx! {
                div { class: "day-body", "data-view": view,
                    if let Some(lists) = lists {
                        aside { id: "tasks-pane", class: "sidebar", aria_label: "Tasks",
                            div { class: "sidebar-scroll", {lists} }
                        }
                    }
                    {timeline(model, &travel, &settings, time_format, selected_block, editing)}
                }
            }
        }
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

    // A week, two weeks or a month: the Schedule tab lists the range, and
    // the other tabs are the same lists as planning a day.
    let range_page = plan_view().range().filter(|_| page() == Page::Plan).map(|range| {
        if plan_tab() != PlanTab::Schedule {
            return match &snapshot {
                Some(snapshot) => rsx! {
                    div { class: "plan-page",
                        {task_list(plan_tab(), snapshot, config, signed_in(), revision, editing, show_create)}
                    }
                },
                None => body.clone(),
            };
        }
        let anchor = week_anchor();
        let shown = agenda::dates(range, anchor, &settings);
        let shows_today = shown.contains(&today);
        let preview = if range == agenda::Range::Month { Vec::new() } else { shown };
        rsx! {
            div { class: "day-header",
                div { class: "day-controls",
                    {month_picker(
                        agenda::label(range, anchor, &settings),
                        anchor,
                        preview,
                        shows_today,
                        today,
                        settings.first_day_of_week,
                        range_month_open,
                        range_month_shown,
                        Pick::Anchor(week_anchor),
                    )}
                    if range == agenda::Range::Month {
                        button { class: "btn", onclick: move |_| week_anchor.set(today), "Today" }
                    }
                    {header_buttons()}
                }
                {status()}
            }
            div { class: "plan-page",
                agenda::AgendaDays {
                    settings: settings.clone(),
                    range,
                    anchor,
                    today,
                    revision,
                    filter: None,
                    search: search_state.read().clone(),
                    on_day: move |day| {
                        page.set(Page::Do);
                        day_nav.go_to(day, day == day::today(&config.peek().calendar));
                    },
                    // Its sheet edits it among the rest of its own day.
                    on_open: move |event: Event| {
                        let day = calendar::day_at(event.start, config.peek().calendar.day_start_hour());
                        day_nav.go_to(day, day == day::today(&config.peek().calendar));
                        pending_open.set(Some(event.id));
                    },
                }
            }
        }
    });

    let edit_context = snapshot
        .as_ref()
        .map(|snapshot| EditContext::new(&config.read(), snapshot));
    let after_write = move |()| revision.set(revision() + 1);

    let modal = model
        .as_ref()
        .zip(edit_context.as_ref())
        .and_then(|(model, context)| {
            let id = selected_block()?;
            let block = model.blocks.iter().find(|block| block.event.id == id)?;
            Some(time_block_modal(
                block,
                context,
                selected_block,
                editing,
                block_writing,
                after_write,
            ))
        });

    let create_sheet = show_create().then(|| {
        let store = cached_days.read();
        let calendars = store.calendars.clone();
        let default_calendar = store.default_calendar_id(&settings);
        drop(store);
        rsx! {
            create::CreateSheet {
                settings: settings.clone(),
                projects: config.read().projects.clone(),
                calendars,
                default_calendar,
                day: selected_day,
                on_created: after_write,
                on_close: move |()| show_create.set(false),
            }
        }
    });

    let agenda_sheet = show_agenda().then(|| {
        rsx! {
            agenda::AgendaSheet {
                settings: settings.clone(),
                day: selected_day,
                today,
                revision,
                filter: agenda_filter(),
                on_day: move |day| {
                    show_agenda.set(false);
                    agenda_filter.set(None);
                    page.set(Page::Do);
                    day_nav.go_to(day, day == day::today(&config.peek().calendar));
                },
                on_open: move |event: Event| {
                    let day = calendar::day_at(event.start, config.peek().calendar.day_start_hour());
                    show_agenda.set(false);
                    agenda_filter.set(None);
                    page.set(Page::Do);
                    day_nav.go_to(day, day == day::today(&config.peek().calendar));
                    pending_open.set(Some(event.id));
                },
                on_close: move |()| {
                    show_agenda.set(false);
                    agenda_filter.set(None);
                },
            }
        }
    });

    let scratch_sheet = show_scratch().map(|start| {
        rsx! {
            scratch::ScratchSheet {
                config,
                day: selected_day,
                start,
                on_close: move |()| show_scratch.set(None),
            }
        }
    });

    let more_sheet = show_more().then(|| {
        rsx! {
            div {
                class: "modal-backdrop",
                onclick: move |_| show_more.set(false),
                div {
                    class: "modal",
                    role: "dialog",
                    aria_label: "More",
                    onclick: move |event| event.stop_propagation(),
                    div { class: "modal-head",
                        h2 { class: "modal-title", "More" }
                        div { class: "spacer" }
                        button {
                            class: "modal-close",
                            title: "Close",
                            onclick: move |_| show_more.set(false),
                            "×"
                        }
                    }
                    // The desktop's More menu, less what stays on the desktop.
                    div { class: "modal-body more-body",
                        button {
                            class: "btn",
                            onclick: move |_| {
                                show_more.set(false);
                                page.set(Page::Habits);
                            },
                            "Habits"
                        }
                        button {
                            class: "btn",
                            onclick: move |_| {
                                show_more.set(false);
                                show_notes.set(true);
                            },
                            "Notes"
                        }
                        button {
                            class: "btn",
                            onclick: move |_| {
                                show_more.set(false);
                                show_people.set(true);
                            },
                            "People"
                        }
                        button {
                            class: "btn",
                            onclick: move |_| {
                                show_more.set(false);
                                show_scratch.set(Some(scratch::Page::BrainDump));
                            },
                            "Brain dump"
                        }
                        button {
                            class: "btn",
                            onclick: move |_| {
                                show_more.set(false);
                                show_scratch.set(Some(scratch::Page::Todos));
                            },
                            "Todos and projects"
                        }
                        button { class: "btn", onclick: move |_| {show_more.set(false);show_operations.set(true);}, "Operations" }
                        p { class: "hint", "{database_sync_status}" }
                    }
                }
            }
        }
    });

    let people_settings = settings.clone();
    let people_sheet = show_people().then(|| {
        let connect_settings = people_settings.clone();
        rsx! {
            people::PeopleSheet {
                store: people_store,
                settings: people_settings,
                connecting,
                on_reconnect: move |()| {
                    begin_connect(
                        connect_settings.clone(),
                        connecting,
                        connect_error,
                        signed_in,
                        revision,
                    );
                },
                on_close: move |()| show_people.set(false),
            }
        }
    });

    let notes_events = snapshot
        .as_ref()
        .map(|snapshot| snapshot.events.clone())
        .unwrap_or_default();
    let notes_sheet = (show_notes() || note_opening().is_some()).then(|| {
        rsx! {
            notes::NotesSheet {
                store: notes_store,
                settings: settings.clone(),
                events: notes_events,
                opening: note_opening,
                on_follow: move |link: timeslay_core::notes::Link| {
                    let mut note_opening = note_opening;
                    note_opening.set(None);
                    show_notes.set(false);
                    if let Some(day) = link.day {
                        day_nav.go_to(day, day == day::today(&config.peek().calendar));
                    }
                    pending_open.set(Some(link.id));
                },
                on_close: move |()| show_notes.set(false),
            }
        }
    });

    let edit_sheet = editing().zip(edit_context).map(|(event, context)| {
        rsx! {
            editor::Editor {
                key: "{event.id}",
                event,
                context,
                on_close: move |()| editing.set(None),
                on_saved: after_write,
                on_config: move |saved| config.set(saved),
            }
        }
    });

    let first_day = settings.first_day_of_week;
    let settings_modal = show_settings().then(|| {
        let error = settings_error().map(|message| {
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
                                match timeslay_core::config::save_database_sync_url(&sync_url_input()) {
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
                        reminders::ReminderSection { config }
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
    });

    let view_modal = view_sheet().map(|view_tab| {
        let store = cached_days.read();
        let sections = day::source_sections(&store.display_sources(&settings), &settings);
        let no_calendars = store.calendars.is_empty();
        drop(store);
        let travel_shown = config.read().travel.show;
        let error = calendars_error().map(|message| {
            rsx! { p { class: "error-text", "{message}" } }
        });
        let sections = sections
            .into_iter()
            .filter(|(_, rows)| !rows.is_empty())
            .map(|(heading, rows)| {
                let rows = rows.into_iter().map(|row| {
                    let swatch = format!("background:{};", css_color(row.color));
                    let enabled = row.enabled;
                    let id = row.id.clone();
                    rsx! {
                        button {
                            key: "{row.id}",
                            class: "source-row",
                            role: "switch",
                            aria_checked: enabled.to_string(),
                            onclick: move |_| {
                                let calendars = cached_days.peek().calendars.clone();
                                match day::set_source_enabled(&calendars, &id, !enabled) {
                                    Ok(saved) => {
                                        config.set(saved);
                                        calendars_error.set(None);
                                    }
                                    Err(error) => calendars_error.set(Some(error)),
                                }
                            },
                            span { class: "source-swatch", style: "{swatch}" }
                            span { class: "source-name", "{row.name}" }
                            span { class: "source-switch", span { class: "source-knob" } }
                        }
                    }
                });
                // Travel is not an event kind, but it is toggled with the sources
                // that are, as on the desktop.
                let travel_row = (heading == "TimeSlay").then(|| {
                    let swatch = format!("background:{};", css_color(calendar::colors::TRAVEL));
                    rsx! {
                        button {
                            class: "source-row",
                            role: "switch",
                            aria_checked: travel_shown.to_string(),
                            onclick: move |_| match day::set_travel_shown(!travel_shown) {
                                Ok(saved) => {
                                    config.set(saved);
                                    calendars_error.set(None);
                                }
                                Err(error) => calendars_error.set(Some(error)),
                            },
                            span { class: "source-swatch", style: "{swatch}" }
                            span { class: "source-name", "Travel" }
                            span { class: "source-switch", span { class: "source-knob" } }
                        }
                    }
                });
                rsx! {
                    section { key: "{heading}", class: "source-section",
                        h3 { class: "source-heading", "{heading}" }
                        {rows}
                        {travel_row}
                    }
                }
            });
        let (labels, locations) = snapshot
            .as_ref()
            .map(|snapshot| day::search_choices(&snapshot.events))
            .unwrap_or_default();
        let view_tab_button = |label: &'static str, this: ViewTab| {
            rsx! {
                button {
                    class: "nav-tab",
                    aria_pressed: (view_tab == this).to_string(),
                    onclick: move |_| view_sheet.set(Some(this)),
                    "{label}"
                }
            }
        };
        // The desktop's View sheet: which calendars show, and a filter that
        // narrows every view.
        rsx! {
            div {
                class: "modal-backdrop",
                onclick: move |_| view_sheet.set(None),
                div {
                    class: "modal",
                    role: "dialog",
                    aria_label: "View",
                    onclick: move |event| event.stop_propagation(),
                    div { class: "modal-head",
                        h2 { class: "modal-title", "View" }
                        div { class: "spacer" }
                        button {
                            class: "modal-close",
                            title: "Close",
                            onclick: move |_| view_sheet.set(None),
                            "×"
                        }
                    }
                    div { class: "sheet-tabs",
                        {view_tab_button("Calendars", ViewTab::Calendars)}
                        {view_tab_button("Filter", ViewTab::Filter)}
                    }
                    div { class: "modal-body",
                        if view_tab == ViewTab::Filter {
                            search::SearchFields {
                                search: search_state,
                                labels,
                                locations,
                                projects: config.read().projects.clone(),
                            }
                        } else {
                            {error}
                            {sections}
                            if no_calendars {
                                p { class: "empty-note",
                                    if signed_in() {
                                        "No calendars yet. Use Actions → Sync now to load them."
                                    } else {
                                        "Connect Google to load your calendars."
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    rsx! {
        document::Title { "TimeSlay · Day" }
        document::Style { {style::STYLESHEET.as_str()} }
        div { class: "app",
            {topbar}
            {banner}
            if page() == Page::Habits {
                habits::HabitsScreen {
                    today,
                    time_format,
                    settings: settings.clone(),
                    signed_in: signed_in(),
                    revision,
                    day_events: snapshot.as_ref().map(|snapshot| snapshot.events.clone()).unwrap_or_default(),
                    on_open_event: move |event| editing.set(Some(event)),
                    on_show_calendar: move |filter| {
                        agenda_filter.set(Some(filter));
                        show_agenda.set(true);
                    },
                }
            } else if let Some(range_page) = range_page {
                {range_page}
            } else {
                {day_header}
                {notice}
                {body}
            }
            if model.is_some() && page() != Page::Habits {
                button {
                    class: "fab",
                    title: "Add",
                    aria_label: "Add",
                    onclick: move |_| show_create.set(true),
                    "+"
                }
            }
            {modal}
            {agenda_sheet}
            {edit_sheet}
            {notes_sheet}
            {create_sheet}
            {scratch_sheet}
            {more_sheet}
            {people_sheet}
            if show_operations() {
                div { class: "modal-backdrop", onclick: move |_| show_operations.set(false),
                    div { class: "modal", onclick: move |event| event.stop_propagation(),
                        div { class: "modal-head", h2 { class: "modal-title", "Operations" }
                            button { class: "btn", onclick: move |_| show_operations.set(false), "Close" }
                        }
                        div { class: "modal-body",
                            if google_operations().is_empty() {p { "All changes saved to Google." }}
                            for operation in google_operations() {
                                div { key: "{operation.id}",
                                    p { "#{operation.id} · {operation.kind} · {operation.created_at}" }
                                    p { "{operation.entity} · Attempts: {operation.attempts}" }
                                    if !operation.pending() {
                                        p { "Could not be saved: {operation.error.clone().unwrap_or_default()}" }
                                        div { class: "editor-row",
                                            button { class: "btn", onclick: move |_| { let _ = calendar::operations::retry(operation.id); }, "Try again" }
                                            button { class: "btn", onclick: move |_| { let _ = calendar::operations::discard(operation.id); }, "Discard" }
                                        }
                                    } else if let Some(error)=operation.error.clone() {p { "Will try again: {error}" }} else {p { "Waiting for Google" }}
                                    pre { "{operation.payload}" }
                                }
                            }
                        }
                    }
                }
            }
            {settings_modal}
            {view_modal}
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

/// One entry of the navigation card, lit while its page is on show.
fn nav_tab(
    label: &'static str,
    active: bool,
    extra_class: &'static str,
    mut on_press: impl FnMut(()) + 'static,
) -> Element {
    rsx! {
        button {
            class: "nav-tab {extra_class}",
            aria_pressed: active.to_string(),
            onclick: move |_| on_press(()),
            "{label}"
        }
    }
}

/// Moves the view to another day.
#[derive(Clone, Copy)]
struct DayNav {
    current_day: Signal<NaiveDate>,
    selected_block: Signal<Option<String>>,
    scrolled: Signal<bool>,
    follow_today: Signal<bool>,
    /// Going somewhere else drops whatever was waiting to open.
    pending_open: Signal<Option<String>>,
}

impl DayNav {
    fn go_to(mut self, day: NaiveDate, follow_today: bool) {
        self.current_day.set(day);
        self.selected_block.set(None);
        self.scrolled.set(false);
        self.follow_today.set(follow_today);
        self.pending_open.set(None);
    }
}

/// Where the navigation is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    /// The day's calendar.
    Do,
    /// The period being planned and its lists.
    Plan,
    Habits,
}

/// The tabs of the View sheet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ViewTab {
    Calendars,
    Filter,
}

/// Chooses what Plan plans, as the desktop's Plan button and menu do.
#[derive(Clone, Copy)]
struct PlanNav {
    page: Signal<Page>,
    view: Signal<PlanView>,
    tab: Signal<PlanTab>,
    menu_open: Signal<bool>,
    week_anchor: Signal<NaiveDate>,
    current_day: Signal<NaiveDate>,
}

impl PlanNav {
    /// Coming to a range from anywhere else opens on its Schedule, starting
    /// from the day in view; moving between ranges keeps the list on show.
    /// Planning a day keeps the list on show, or opens Overdue.
    fn choose(mut self, view: PlanView) {
        let was_range = *self.page.peek() == Page::Plan && self.view.peek().range().is_some();
        let changed = *self.view.peek() != view;
        self.menu_open.set(false);
        self.view.set(view);
        if view.range().is_some() {
            if !was_range {
                self.tab.set(PlanTab::Schedule);
            }
            if changed || !was_range {
                self.week_anchor.set(*self.current_day.peek());
            }
        } else if *self.tab.peek() == PlanTab::Schedule {
            self.tab.set(PlanTab::Overdue);
        }
        self.page.set(Page::Plan);
    }
}

/// One of Plan's lists, over everything loaded with its local edits.
fn task_list(
    tab: PlanTab,
    snapshot: &calendar::Snapshot,
    config: Signal<timeslay_core::config::Config>,
    signed_in: bool,
    revision: Signal<u32>,
    editing: Signal<Option<Event>>,
    show_create: Signal<bool>,
) -> Element {
    let mut revision = revision;
    let mut show_create = show_create;
    let context = EditContext::new(&config.read(), snapshot);
    rsx! {
        plan::TaskList {
            key: "{tab:?}",
            tab,
            config,
            events: snapshot.events.clone(),
            calendars: snapshot.calendars.clone(),
            context: Some(context),
            signed_in,
            revision,
            editing,
            on_new_task: move |()| show_create.set(true),
            on_saved: move |()| revision.set(revision() + 1),
        }
    }
}

/// What picking a day in a month picker moves.
#[derive(Clone, Copy)]
enum Pick {
    /// The day in view.
    Day(DayNav),
    /// The range being planned.
    Anchor(Signal<NaiveDate>),
}

impl Pick {
    fn pick(self, day: NaiveDate, today: NaiveDate) {
        match self {
            Self::Day(nav) => nav.go_to(day, day == today),
            Self::Anchor(mut anchor) => anchor.set(day),
        }
    }
}

/// The picker button, labelled with what is in view, and the month it opens:
/// the desktop's way between days and ranges. The month is named on the left,
/// with a month back, today (●) and a month forward on the right; `preview`
/// marks the days of the range in view.
#[allow(clippy::too_many_arguments)]
fn month_picker(
    label: String,
    selected: NaiveDate,
    preview: Vec<NaiveDate>,
    shows_today: bool,
    today: NaiveDate,
    first_day: FirstDayOfWeek,
    mut open: Signal<bool>,
    mut shown: Signal<NaiveDate>,
    pick: Pick,
) -> Element {
    let month = shown();
    let days = if open() {
        calendar::month_grid(month, first_day)
    } else {
        Vec::new()
    };
    let cells = days.into_iter().map(move |day| {
        let mut class = String::from("month-day");
        if day.month() != month.month() {
            class.push_str(" outside");
        }
        if preview.contains(&day) {
            class.push_str(" in-range");
        }
        if day == selected && preview.is_empty() {
            class.push_str(" selected");
        }
        if day == today {
            class.push_str(" today");
        }
        let number = day.day();
        rsx! {
            button {
                key: "{day}",
                class: "{class}",
                onclick: move |_| {
                    open.set(false);
                    pick.pick(day, today);
                },
                "{number}"
            }
        }
    });
    let picker = open().then(|| {
        let heading = calendar::month_heading(month);
        rsx! {
            div { class: "month-dismiss", onclick: move |_| open.set(false) }
            div { class: "month-picker", role: "dialog", aria_label: "Choose a day",
                div { class: "month-head",
                    span { class: "month-name", "{heading}" }
                    button {
                        class: "btn icon",
                        title: "Previous month",
                        aria_label: "Previous month",
                        onclick: move |_| {
                            if let Some(previous) = month.checked_sub_months(Months::new(1)) {
                                shown.set(previous);
                            }
                        },
                        "‹"
                    }
                    button {
                        class: "btn icon",
                        title: "Today",
                        aria_label: "Today",
                        onclick: move |_| {
                            open.set(false);
                            pick.pick(today, today);
                        },
                        "●"
                    }
                    button {
                        class: "btn icon",
                        title: "Next month",
                        aria_label: "Next month",
                        onclick: move |_| {
                            if let Some(next) = month.checked_add_months(Months::new(1)) {
                                shown.set(next);
                            }
                        },
                        "›"
                    }
                }
                div { class: "month-grid",
                    for weekday in first_day.short_labels() {
                        span { key: "{weekday}", class: "month-weekday", "{weekday}" }
                    }
                    {cells}
                }
            }
        }
    });

    rsx! {
        div { class: "month-anchor",
            button {
                class: if shows_today { "btn today" } else if open() { "btn primary" } else { "btn" },
                aria_expanded: open().to_string(),
                onclick: move |_| {
                    if !open() {
                        shown.set(calendar::first_of_month(selected));
                    }
                    open.set(!open());
                },
                "{label} ▾"
            }
            {picker}
        }
    }
}

/// An all-day event: a full-width bar in the event's own color.
fn all_day_chip(
    event: &Event,
    settings: &CalendarSettings,
    editing: Signal<Option<Event>>,
) -> Element {
    let mut editing = editing;
    let opened = event.clone();
    let color = event.display_color(settings);
    let style = format!(
        "background:{};color:{};",
        css_color(color),
        css_color(calendar::colors::on_color(color))
    );
    let title = if event.completed {
        format!("✓ {}", event.title)
    } else {
        event.title.clone()
    };
    rsx! {
        button {
            class: "all-day-chip",
            style: "{style}",
            title: "{title}",
            onclick: move |_| editing.set(Some(opened.clone())),
            "{title}"
        }
    }
}

fn timeline(
    model: &DayModel,
    travel: &[day::TravelBox],
    settings: &CalendarSettings,
    time_format: TimeFormat,
    open_block: Signal<Option<String>>,
    editing: Signal<Option<Event>>,
) -> Element {
    let grid_height = 24.0 * HOUR_HEIGHT;
    let hours: Vec<Element> = (0..24u32)
        .map(|row| {
            let top = row as f32 * HOUR_HEIGHT + 2.0;
            let label = time_format::hour(day::hour_at(model.hour_start, row), time_format);
            rsx! {
                span {
                    key: "label-{row}",
                    class: "hour-label",
                    style: "top: {top}px;",
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
        .map(|timed| timed_event_view(timed, time_format, open_block, editing))
        .collect();
    let block_tasks: Vec<Element> = model
        .timed
        .iter()
        .flat_map(|timed| block_task_rows(timed, editing))
        .collect();

    let trips: Vec<Element> = travel
        .iter()
        .map(|trip| {
            let style = travel_style(trip);
            rsx! {
                div { key: "travel-{trip.event_id}", class: "grid-travel", style: "{style}", "{trip.label}" }
            }
        })
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

    let summary = model.summary;
    let summary_class = if summary.needs_attention() {
        "day-summary attention"
    } else {
        "day-summary"
    };

    rsx! {
        section { id: "calendar-pane", class: "timeline-pane", aria_label: "Schedule",
            div { class: "day-overview",
                p { class: "{summary_class}",
                    "{summary.overdue} overdue. \u{a0}{summary.inbox} in inbox. \u{a0}{summary.unslotted} not slotted in"
                }
                for event in &model.all_day {
                    {all_day_chip(event, settings, editing)}
                }
            }
            div { class: "timeline-card",
                div { class: "timeline-scroll",
                    div { class: "timeline", style: "height: {grid_height}px;",
                        div { class: "gutter", style: "height: {grid_height}px;", {hours.into_iter()} }
                        div { class: "grid", style: "height: {grid_height}px;",
                            {lines.into_iter()}
                            {events.into_iter()}
                            {block_tasks.into_iter()}
                            {trips.into_iter()}
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
}

/// How an event's text fits in its height.
#[derive(Debug, PartialEq, Eq)]
struct TextFit {
    /// The gap above the title, which shrinks on a short event.
    top: u32,
    /// The most lines the title can take.
    title_lines: u32,
    /// The lines of its time that fit under the title.
    time_lines: usize,
}

/// A title always gets a line, then the time, then the rest goes to the title.
fn text_fit(height: f32, time_lines: usize) -> TextFit {
    let top = (height - TITLE_LINE).clamp(0.0, look::BLOCK_TOP);
    let room = height - top;
    let time_lines = (((room - TITLE_LINE) / TIME_LINE).floor().max(0.0) as usize).min(time_lines);
    let title_lines = ((room - time_lines as f32 * TIME_LINE) / TITLE_LINE)
        .floor()
        .max(1.0) as u32;
    TextFit {
        top: top as u32,
        title_lines,
        time_lines,
    }
}

/// The time written on an event: a sleep block says when to go to bed and rise.
fn event_when(event: &Event, time_format: TimeFormat) -> String {
    if event.details.sleep.is_some() {
        calendar::sleep::time_label(event, time_format)
    } else {
        time_format::event(event, time_format)
    }
}

fn event_label(timed: &TimedEvent, height: f32, time_format: TimeFormat) -> Element {
    let when = event_when(&timed.event, time_format);
    let fit = text_fit(height, when.lines().count());
    let title = timed.title.clone();
    let times = when
        .lines()
        .take(fit.time_lines)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    rsx! {
        div {
            class: "event-title",
            style: "margin-top:{fit.top}px;-webkit-line-clamp:{fit.title_lines};",
            "{title}"
        }
        for time in times {
            div { class: "event-time", "{time}" }
        }
    }
}

fn timed_event_view(
    timed: &TimedEvent,
    time_format: TimeFormat,
    open_block: Signal<Option<String>>,
    editing: Signal<Option<Event>>,
) -> Element {
    let mut editing = editing;
    let opened = timed.event.clone();
    let id = timed.event.id.clone();
    let style = event_style(timed);
    let height = event_height(timed);
    let tooltip = format!(
        "{}\n{}\n{}",
        timed.title,
        event_when(&timed.event, time_format),
        timed.calendar_name
    );
    let label = event_label(timed, height, time_format);

    if timed.time_block {
        let mut open = open_block;
        let planned: u32 = timed.tasks.iter().map(|task| task.minutes).sum();
        let duration = (timed.end_min - timed.start_min).max(1) as f32;
        let filled = (planned as f32 / duration).min(1.0) * 100.0;
        rsx! {
            button {
                key: "{id}",
                class: "grid-event block",
                style: "{style}",
                title: "{tooltip}",
                onclick: move |_| open.set(Some(id.clone())),
                div { class: "block-filled", style: "height:{filled:.2}%;" }
                div { class: "block-label", {label} }
                span { class: "block-plan", "Plan" }
            }
        }
    } else {
        rsx! {
            button {
                key: "{id}",
                class: "grid-event",
                style: "{style}",
                title: "{tooltip}",
                onclick: move |_| editing.set(Some(opened.clone())),
                {label}
            }
        }
    }
}

/// A time block's tasks, listed in the space it leaves beside itself. The
/// ones that run past the block's end are marked.
fn block_task_rows(timed: &TimedEvent, editing: Signal<Option<Event>>) -> Vec<Element> {
    let top = timed.start_min as f32 * HOUR_HEIGHT / 60.0;
    let duration = (timed.end_min - timed.start_min).max(1) as u32;
    let (left, span) = column_position(timed.column, timed.columns, timed.column_span);
    let width = time_block_width(span);
    let mut elapsed = 0;
    timed
        .tasks
        .iter()
        .enumerate()
        .map(|(index, task)| {
            let mut editing = editing;
            elapsed += task.minutes;
            let class = if elapsed > duration {
                "block-task over"
            } else {
                "block-task"
            };
            let icon = status_icon(task.status);
            let title = tb::task_label(&task.event);
            let minutes = task.minutes;
            let row_top = top + index as f32 * look::TIME_BLOCK_ROW;
            let inset = look::EVENT_GAP / 2.0;
            let gap = look::TIME_BLOCK_ROW_GAP;
            let opened = task.event.clone();
            rsx! {
                button {
                    key: "{task.event.id}",
                    class: "{class}",
                    style: "top:{row_top:.1}px;left:calc({left:.4}% + {inset}px + {width} + {gap}px);width:{width};",
                    onclick: move |_| editing.set(Some(opened.clone())),
                    "{icon} {title} · {minutes}m"
                }
            }
        })
        .collect()
}

fn time_block_modal(
    block: &TimeBlock,
    context: &EditContext,
    close: Signal<Option<String>>,
    editing: Signal<Option<Event>>,
    writing: Signal<bool>,
    on_saved: impl FnMut(()) + 'static,
) -> Element {
    let mut editing = editing;
    let edited_block = block.event.clone();
    let planner = block.clone();
    let planner_context = context.clone();
    let on_saved = EventHandler::new(on_saved);
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

    rsx! {
        // Close on the press that starts outside the sheet. Using pointerdown
        // (rather than click) means the tap that opened the sheet cannot also
        // close it, which matters on Android where a synthesized click can
        // arrive after the modal has mounted.
        div {
            class: "modal-backdrop",
            onpointerdown: move |_| {
                if !writing() {
                    backdrop_close.set(None);
                }
            },
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
                        disabled: writing(),
                        onclick: move |_| button_close.set(None),
                        "×"
                    }
                }
                div { class: "modal-body",
                    button {
                        class: "btn editor-wide",
                        onclick: move |_| editing.set(Some(edited_block.clone())),
                        "Edit block"
                    }
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
                    editor::BlockTasks {
                        block: planner,
                        context: planner_context,
                        busy: writing,
                        on_open: move |task| editing.set(Some(task)),
                        on_saved,
                    }
                }
            }
        }
    }
}

pub(crate) fn status_icon(status: tb::TaskStatus) -> &'static str {
    match status {
        tb::TaskStatus::Active => "○",
        tb::TaskStatus::Completed => "✓",
        tb::TaskStatus::Ignored => "⊘",
    }
}

fn event_height(timed: &TimedEvent) -> f32 {
    ((timed.end_min - timed.start_min) as f32 * HOUR_HEIGHT / 60.0).max(EVENT_MIN_HEIGHT)
}

/// Where a column starts and how wide its span is, as percentages of the grid.
fn column_position(column: usize, columns: usize, span: usize) -> (f32, f32) {
    let width = 100.0 / columns.max(1) as f32;
    (column as f32 * width, span.max(1) as f32 * width)
}

fn column_width(span: f32) -> String {
    format!("calc({span:.4}% - {}px)", look::EVENT_GAP)
}

/// A time block leaves part of its column for its task rows, as on the desktop.
fn time_block_width(span: f32) -> String {
    format!(
        "calc(({span:.4}% - {}px) * {})",
        look::EVENT_GAP + look::TIME_BLOCK_INSET,
        look::TIME_BLOCK_SHARE
    )
}

fn event_style(timed: &TimedEvent) -> String {
    let top = timed.start_min as f32 * HOUR_HEIGHT / 60.0;
    let height = event_height(timed);
    let (left, span) = column_position(timed.column, timed.columns, timed.column_span);
    let width = if timed.time_block {
        time_block_width(span)
    } else {
        column_width(span)
    };
    let inset = look::EVENT_GAP / 2.0;
    let background = css_color(timed.color);
    let ink = css_color(calendar::colors::on_color(timed.color));

    format!(
        "top:{top:.1}px;height:{height:.1}px;left:calc({left:.4}% + {inset}px);width:{width};background:{background};color:{ink};"
    )
}

/// Where a trip's box sits: ending at the start of the event it leads to, in
/// that event's column. Over a time block it is half as wide, as on the desktop.
fn travel_style(trip: &day::TravelBox) -> String {
    let height = (trip.minutes as f32 * HOUR_HEIGHT / 60.0).max(look::TRAVEL_MIN_HEIGHT);
    let top = (trip.anchor_min as f32 * HOUR_HEIGHT / 60.0 - height).max(0.0);
    let (left, span) = column_position(trip.column, trip.columns, trip.column_span);
    let width = if trip.time_block {
        time_block_width(span)
    } else {
        column_width(span)
    };
    let inset = look::EVENT_GAP / 2.0;
    let background = css_color(calendar::colors::TRAVEL);
    let ink = css_color(calendar::colors::on_color(calendar::colors::TRAVEL));

    format!(
        "top:{top:.1}px;height:{height:.1}px;left:calc({left:.4}% + {inset}px);width:{width};background:{background};color:{ink};"
    )
}

pub(crate) fn css_color(color: timeslay_core::Color) -> String {
    let red = (color.r.clamp(0.0, 1.0) * 255.0).round() as u8;
    let green = (color.g.clamp(0.0, 1.0) * 255.0).round() as u8;
    let blue = (color.b.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "rgba({red}, {green}, {blue}, {:.2})",
        color.a.clamp(0.0, 1.0)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_event_has_room_for_its_title_only() {
        assert_eq!(
            text_fit(15.0, 1),
            TextFit {
                top: 0,
                title_lines: 1,
                time_lines: 0
            }
        );
        assert_eq!(text_fit(30.0, 1).time_lines, 0);
    }

    #[test]
    fn a_tall_event_gives_what_its_time_leaves_to_the_title() {
        let fit = text_fit(60.0, 1);
        assert_eq!(fit.time_lines, 1);
        assert_eq!(fit.title_lines, 2);
    }

    #[test]
    fn a_sleep_block_keeps_both_of_its_time_lines_when_they_fit() {
        assert_eq!(text_fit(64.0, 2).time_lines, 2);
        assert_eq!(text_fit(40.0, 2).time_lines, 1);
    }
}

#[cfg(test)]
mod sheet_tests {
    use super::*;
    use chrono::{Local, TimeZone};
    use timeslay_core::config::{Config, ProjectsSettings};
    use timeslay_core::notes::Library;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
    }

    fn render(app: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn assert_offers(html: &str, wanted: &[&str]) {
        for text in wanted {
            assert!(html.contains(text), "{text} is missing from {html}");
        }
    }

    fn library() -> Library {
        let mut library = Library::default();
        let id = library.create("Trip plan", Vec::new());
        library.edit(&id, |note| {
            note.body = "# Packing\n\n- [x] Passport".to_owned()
        });
        library
    }

    #[test]
    fn the_notes_list_offers_its_notes_search_and_new() {
        fn app() -> Element {
            let mut store = use_hook(notes::NotesStore::new);
            use_hook(move || store.library.set(library()));
            let opening = use_signal(|| None::<String>);
            rsx! {
                notes::NotesSheet {
                    store,
                    settings: CalendarSettings::default(),
                    events: Vec::new(),
                    opening,
                    on_follow: |_| {},
                    on_close: |_| {},
                }
            }
        }
        assert_offers(
            &render(app),
            &["Search notes", ">New<", "Trip plan", "Refresh"],
        );
    }

    #[test]
    fn an_open_note_offers_its_tabs_colors_and_delete() {
        fn app() -> Element {
            let mut store = use_hook(notes::NotesStore::new);
            let id = use_hook(move || {
                let library = library();
                let id = library.notes[0].id.clone();
                store.library.set(library);
                id
            });
            let opening = use_signal(move || Some(id));
            rsx! {
                notes::NotesSheet {
                    store,
                    settings: CalendarSettings::default(),
                    events: Vec::new(),
                    opening,
                    on_follow: |_| {},
                    on_close: |_| {},
                }
            }
        }
        assert_offers(
            &render(app),
            &[
                ">Edit<",
                ">View<",
                "Trip plan",
                "Butter",
                "Mint",
                "Connected to",
                "Delete note",
            ],
        );
    }

    #[test]
    fn adding_offers_every_kind_starting_with_a_task() {
        fn app() -> Element {
            rsx! {
                create::CreateSheet {
                    settings: CalendarSettings::default(),
                    projects: ProjectsSettings::default(),
                    calendars: Vec::new(),
                    default_calendar: None,
                    day: today(),
                    on_created: |_| {},
                    on_close: |_| {},
                }
            }
        }
        assert_offers(
            &render(app),
            &[
                ">Task<",
                ">Event<",
                ">Time block<",
                "@home ~30m",
                "goes to the Inbox",
            ],
        );
    }

    #[test]
    fn the_filter_tab_offers_each_filter() {
        fn app() -> Element {
            let search = use_signal(Search::default);
            rsx! {
                search::SearchFields {
                    search,
                    labels: vec!["deep".to_owned()],
                    locations: vec!["Office".to_owned()],
                    projects: ProjectsSettings::default(),
                }
            }
        }
        assert_offers(
            &render(app),
            &[
                "Search titles",
                "Look in descriptions",
                "Only public",
                "Under way at",
                "Busy or free",
                "Office",
                "deep",
                ">Clear<",
            ],
        );
    }

    #[test]
    fn the_lists_open_on_todos_and_offer_the_other_pages() {
        fn app() -> Element {
            let config = use_signal(|| {
                let mut config = Config::default();
                config.todos.add("Write the report");
                config.todos.add("Send it");
                config
            });
            rsx! { scratch::ScratchSheet { config, day: today(), on_close: |_| {} } }
        }
        assert_offers(
            &render(app),
            &[
                ">Todos<",
                ">Notepad<",
                ">Brain dump<",
                ">Projects<",
                "Write the report",
                "Add a step",
            ],
        );
    }

    fn render_list(app: fn() -> Element) -> String {
        render(app)
    }

    fn list_app(tab: PlanTab) -> Element {
        let start = Local.with_ymd_and_hms(2026, 11, 4, 0, 0, 0).unwrap();
        let snapshot = calendar::Snapshot {
            calendars: Vec::new(),
            events: Vec::new(),
            day: today(),
            fetched_at: start,
            notice: None,
        };
        let config = use_signal(Config::default);
        let revision = use_signal(|| 0u32);
        let editing = use_signal(|| None::<Event>);
        let create = use_signal(|| false);
        task_list(tab, &snapshot, config, false, revision, editing, create)
    }

    #[test]
    fn each_plan_list_has_the_desktops_heading_and_empty_note() {
        assert_offers(
            &render_list(|| list_app(PlanTab::Overdue)),
            &["Overdue", "Sort by", "Manual order", "Nothing overdue"],
        );
        assert_offers(
            &render_list(|| list_app(PlanTab::Inbox)),
            &[
                "Inbox",
                "Tasks waiting to be triaged",
                ">New task<",
                "Sort by",
                "Nothing to triage",
            ],
        );
        assert_offers(
            &render_list(|| list_app(PlanTab::Today)),
            &["Today", "No tasks left to slot in today"],
        );
        let backlog = render_list(|| list_app(PlanTab::Backlog));
        assert_offers(
            &backlog,
            &[
                "Backlog",
                "Search tasks",
                "No tasks on the Backlog list",
                "Next week or two",
                "Never",
            ],
        );
        assert!(!backlog.contains("Sort by"));
    }
}
