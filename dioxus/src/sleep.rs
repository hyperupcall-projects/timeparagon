//! Timing a night's sleep, as on the desktop: Start sleep and Finish sleep on
//! the night, a strip with the wake-up times while it runs, the notification
//! Android keeps up meanwhile, and the weekly schedule in Settings.

use std::sync::Mutex;

use chrono::NaiveDate;
use dioxus::prelude::*;
use timeslay_core::calendar::time_format::{self, TimeFormat};
use timeslay_core::calendar::{self, Event, Snapshot, sleep};
use timeslay_core::config::{CalendarSettings, Config};

/// The night whose timer is running among `events`, the latest if several.
pub fn active<'a>(events: impl IntoIterator<Item = &'a Event>) -> Option<&'a Event> {
    events
        .into_iter()
        .filter(|event| {
            event
                .details
                .sleep
                .as_ref()
                .is_some_and(sleep::Record::sleeping)
        })
        .max_by_key(|event| event.start)
}

/// The night being timed, looked for around today whichever day is in view:
/// a night started on another day must still be finishable. `snapshot` is the
/// day in view with local edits already applied, and wins over cached copies.
pub fn running(
    snapshot: Option<&Snapshot>,
    store: &calendar::Store,
    today: NaiveDate,
) -> Option<Event> {
    let shown = snapshot.map_or(&[][..], |snapshot| snapshot.events.as_slice());
    let mut cached: Vec<Event> = Vec::new();
    for day in [today.pred_opt(), Some(today), today.succ_opt()]
        .into_iter()
        .flatten()
    {
        for event in store.events_on(day) {
            let same =
                |other: &Event| other.id == event.id && other.calendar_id == event.calendar_id;
            if event.details.sleep.is_some() && !shown.iter().any(same) && !cached.iter().any(same)
            {
                cached.push(event.clone());
            }
        }
    }
    if !cached.is_empty()
        && let Some(path) = timeslay_core::task_db::path()
        && let Err(error) = calendar::operations::overlay(
            &path,
            &mut cached,
            store.cached_snapshot().map(|snapshot| snapshot.fetched_at),
        )
    {
        eprintln!("Could not load local edits: {error}");
    }
    active(shown.iter().chain(&cached)).cloned()
}

/// Whether the night's timer button can be pressed: a write for it must not
/// already be on its way to Google.
pub fn can_press(event: &Event) -> bool {
    !event.details.sending
}

/// Starts or finishes the night's timer now and saves it, then updates the
/// notification. Blocking: callers run it on a worker thread.
pub fn press(
    settings: &CalendarSettings,
    mut event: Event,
    format: TimeFormat,
) -> Result<Event, String> {
    let now = timeslay_core::clock::now();
    let timer = sleep::timer(&event, now).ok_or("This night's sleep timer is not available now")?;
    sleep::apply_timer(&mut event, timer, now)?;
    let saved = sleep::save(settings, event, false)?;
    match timer {
        sleep::Timer::Start => show_running(&saved, format),
        sleep::Timer::Finish => show_finished(&saved, format),
    }
    Ok(saved)
}

/// What the notification last showed, so each render does not post again.
#[derive(Debug, Clone, PartialEq)]
enum Shown {
    /// The ongoing notification, by what it says.
    Sleeping(String),
    /// How long was slept, which the user dismisses.
    Slept,
    /// Nothing ongoing.
    Cleared,
}

/// Unknown (`None`) when the process starts: an earlier run may have left the
/// ongoing notification up for a night finished elsewhere since.
static SHOWN: Mutex<Option<Shown>> = Mutex::new(None);

/// Keeps the notification in step with the night being timed: up while one
/// is running, down once none is. `settled` says the data around today has
/// loaded, so a missing night really means none is running.
pub fn sync_notification(running: Option<&Event>, settled: bool, format: TimeFormat) {
    match running {
        Some(event) => show_running(event, format),
        None if settled => {
            let mut shown = SHOWN.lock().unwrap_or_else(|error| error.into_inner());
            if matches!(*shown, None | Some(Shown::Sleeping(_))) {
                *shown = Some(Shown::Cleared);
                // Only takes down an ongoing notification, so a summary of
                // the night stays until it is dismissed.
                post("none", "", "", None);
            }
        }
        None => {}
    }
}

fn show_running(event: &Event, format: TimeFormat) {
    let Some(start) = event
        .details
        .sleep
        .as_ref()
        .and_then(|record| record.actual_start)
    else {
        return;
    };
    let (title, body) = sleep::notification(event, sleep::Timer::Start, format);
    let key = format!("{}\n{title}\n{body}\n{start}", event.id);
    let mut shown = SHOWN.lock().unwrap_or_else(|error| error.into_inner());
    if shown.as_ref() == Some(&Shown::Sleeping(key.clone())) {
        return;
    }
    *shown = Some(Shown::Sleeping(key));
    post("sleeping", &title, &body, Some(start.timestamp_millis()));
}

fn show_finished(event: &Event, format: TimeFormat) {
    let (title, body) = sleep::notification(event, sleep::Timer::Finish, format);
    *SHOWN.lock().unwrap_or_else(|error| error.into_inner()) = Some(Shown::Slept);
    post("slept", &title, &body, None);
}

/// Hands the notification to `SleepNotification.kt`. Elsewhere there is no
/// notification; the strip is all there is.
fn post(state: &str, title: &str, body: &str, since: Option<i64>) {
    #[cfg(target_os = "android")]
    {
        let since = since.map(|millis| millis.to_string()).unwrap_or_default();
        crate::reminders::broadcast_extras(
            "dev.timeslay.day.reminders.SLEEP",
            &[
                ("state", state),
                ("title", title),
                ("body", body),
                ("since", &since),
            ],
        );
    }
    #[cfg(not(target_os = "android"))]
    let _ = (state, title, body, since);
}

/// While sleep is being timed: since when and for how long, the wake-up times
/// for 6 to 9 hours and the target, and Finish sleep.
#[component]
pub fn SleepStrip(
    event: Event,
    time_format: TimeFormat,
    busy: bool,
    error: Option<String>,
    on_finish: EventHandler<Event>,
) -> Element {
    // The elapsed time moves on without anything else changing.
    let mut tick = use_signal(|| 0u32);
    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            tick += 1;
        }
    });
    let _ = tick();
    let Some(start) = event
        .details
        .sleep
        .as_ref()
        .and_then(|record| record.actual_start)
    else {
        return rsx! {};
    };
    let elapsed = sleep::minutes_label((timeslay_core::clock::now() - start).num_minutes());
    let since = time_format::clock(start, time_format);
    let predictions = sleep::predictions_line(&event, time_format);
    let disabled = busy || !can_press(&event);
    rsx! {
        div { class: "sleep-strip", role: "status", aria_label: "Sleep timer",
            div { class: "sleep-strip-text",
                span { class: "sleep-strip-title", "Sleeping since {since} · {elapsed}" }
                span { class: "sleep-strip-detail", "Wake up: {predictions}" }
                if let Some(message) = error {
                    span { class: "sleep-strip-error", role: "alert", "{message}" }
                }
            }
            button {
                class: "btn small",
                disabled,
                onclick: move |_| on_finish.call(event.clone()),
                if busy { "Saving…" } else { "Finish sleep" }
            }
        }
    }
}

/// Saves a changed weekday schedule, read afresh from the file so another
/// process's edits survive, and returns the config as it now reads.
pub fn save_weekday(
    weekday: chrono::Weekday,
    bedtime: bool,
    value: &str,
) -> Result<Config, String> {
    let time = chrono::NaiveTime::parse_from_str(value, "%H:%M")
        .map_err(|_| "Enter the time as HH:MM".to_owned())?;
    let mut config = crate::day::load_config();
    let from = sleep::next_night(timeslay_core::clock::now().date_naive(), weekday);
    let mut times = config.calendar.sleep_schedule[weekday.num_days_from_monday() as usize];
    if bedtime {
        times.bedtime = time;
    } else {
        times.wakeup = time;
    }
    sleep::set_weekday_times(&mut config.calendar, from, times);
    save_schedule(&config.calendar)?;
    Ok(crate::day::load_config())
}

/// Makes one night's new times the schedule for that weekday from then on.
pub fn save_from_night(night: &Event) -> Result<Config, String> {
    let record = night
        .details
        .sleep
        .as_ref()
        .ok_or("Missing sleep metadata")?;
    let mut config = crate::day::load_config();
    let from = sleep::change_from(record.wake_date, timeslay_core::clock::now().date_naive());
    sleep::set_weekday_times(
        &mut config.calendar,
        from,
        sleep::Times {
            bedtime: night.start.time(),
            wakeup: night.end.time(),
        },
    );
    save_schedule(&config.calendar)?;
    Ok(crate::day::load_config())
}

fn save_schedule(settings: &CalendarSettings) -> Result<(), String> {
    timeslay_core::config::save_sleep_schedule(settings)
        .map_err(|error| format!("Could not save the sleep schedule: {error}"))
}

/// The Sleep section of Settings: bedtime and wake-up for each weekday's night.
#[component]
pub fn SleepSettings(config: Signal<Config>) -> Element {
    let mut error = use_signal(|| None::<String>);
    let settings = config.read().calendar.clone();
    let rows = settings
        .first_day_of_week
        .weekdays()
        .into_iter()
        .map(|weekday| {
            let times = settings.sleep_schedule[weekday.num_days_from_monday() as usize];
            let bedtime = times.bedtime.format("%H:%M").to_string();
            let wakeup = times.wakeup.format("%H:%M").to_string();
            let field = move |label: &'static str, is_bedtime: bool, value: String| {
                rsx! {
                    label { class: "sleep-time",
                        span { class: "editor-label", "{label}" }
                        input {
                            class: "client-input",
                            r#type: "time",
                            aria_label: "{weekday} {label}",
                            value: "{value}",
                            onchange: move |event: FormEvent| {
                                match save_weekday(weekday, is_bedtime, &event.value()) {
                                    Ok(saved) => {
                                        config.set(saved);
                                        error.set(None);
                                    }
                                    Err(message) => error.set(Some(message)),
                                }
                            },
                        }
                    }
                }
            };
            rsx! {
                div { key: "{weekday}", class: "sleep-day",
                    span { class: "sleep-day-name", "{weekday_name(weekday)}" }
                    div { class: "sleep-day-times",
                        {field("Bedtime", true, bedtime)}
                        {field("Wake-up", false, wakeup)}
                    }
                }
            }
        });
    rsx! {
        h3 { class: "settings-heading", "Sleep" }
        p { class: "hint",
            "Each row is the night ending on that weekday. Bedtimes later than wake-up are on the preceding evening. Changes apply from today through four weeks ahead."
        }
        if let Some(message) = error() {
            p { class: "error-text", "{message}" }
        }
        div { class: "sleep-schedule", {rows} }
    }
}

fn weekday_name(weekday: chrono::Weekday) -> &'static str {
    use chrono::Weekday::*;
    match weekday {
        Mon => "Monday",
        Tue => "Tuesday",
        Wed => "Wednesday",
        Thu => "Thursday",
        Fri => "Friday",
        Sat => "Saturday",
        Sun => "Sunday",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration, Local};

    fn at(hour: u32) -> DateTime<Local> {
        calendar::at_clock(NaiveDate::from_ymd_opt(2030, 11, 4).unwrap(), hour, 0)
    }

    fn night(id: &str, start: u32) -> Event {
        Event {
            id: id.into(),
            calendar_id: "app".into(),
            title: "Sleep".into(),
            start: at(start),
            end: at(start + 8),
            all_day: false,
            color: sleep::COLOR,
            completed: false,
            details: calendar::Details {
                sleep: Some(sleep::Record::planned(at(9).date_naive())),
                ..Default::default()
            },
        }
    }

    #[test]
    fn only_a_started_unfinished_night_is_running() {
        let planned = night("a", 0);
        let mut started = night("b", 1);
        sleep::apply_timer(&mut started, sleep::Timer::Start, at(1)).unwrap();
        let mut finished = started.clone();
        finished.id = "c".into();
        sleep::apply_timer(&mut finished, sleep::Timer::Finish, at(8)).unwrap();
        assert_eq!(active([&planned, &finished]), None);
        assert_eq!(
            active([&planned, &started, &finished]).map(|e| e.id.as_str()),
            Some("b")
        );
    }

    #[derive(Props, Clone, PartialEq)]
    struct HarnessProps {
        event: Event,
    }

    #[allow(non_snake_case)]
    fn Harness(props: HarnessProps) -> Element {
        rsx! {
            SleepStrip {
                event: props.event,
                time_format: TimeFormat::TwentyFourHour,
                busy: false,
                error: None,
                on_finish: |_| {},
            }
        }
    }

    #[test]
    fn the_strip_offers_finish_with_the_wake_up_times() {
        let mut event = night("n", 1);
        sleep::apply_timer(
            &mut event,
            sleep::Timer::Start,
            at(1) - Duration::minutes(20),
        )
        .unwrap();
        let mut dom = VirtualDom::new_with_props(Harness, HarnessProps { event });
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        for wanted in [
            "Sleeping since 00:40",
            "Wake up: 6h 06:40",
            "Target 8h",
            "Finish sleep",
        ] {
            assert!(html.contains(wanted), "missing {wanted} in {html}");
        }
    }
}
