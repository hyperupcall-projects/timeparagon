//! The mobile editor uses the same alarm document and profiles as desktop.
use chrono::{Local, NaiveDateTime, TimeZone};
use dioxus::prelude::*;
use timeslay_core::{
    alarms::{self, Alarm, Library, SoundChoice},
    config::Config,
    sounds::Tone,
};

fn load() -> Result<Library, String> {
    let path = timeslay_core::task_db::path().ok_or("The alarm database is unavailable")?;
    alarms::read(&path).map_err(|e| e.to_string())
}

// Read just before changing a single alarm, preserving other devices' edits.
fn change(id: u64, mut replacement: Option<Alarm>, baseline: Option<Alarm>) -> Result<(), String> {
    let path = timeslay_core::task_db::path().ok_or("The alarm database is unavailable")?;
    let mut library = alarms::read(&path).map_err(|e| e.to_string())?;
    if let (Some(desired), Some(before)) = (replacement.as_mut(), baseline) {
        let mut current = library
            .alarms
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or("This alarm was removed on another device")?;
        if desired.label != before.label {
            current.label = desired.label.clone();
        }
        if desired.at != before.at {
            current.at = desired.at;
        }
        if desired.profile.sound.enabled != before.profile.sound.enabled {
            current.profile.sound.enabled = desired.profile.sound.enabled;
        }
        if desired.profile.sound.choice != before.profile.sound.choice {
            current.profile.sound.choice = desired.profile.sound.choice.clone();
        }
        if desired.profile.notification.enabled != before.profile.notification.enabled {
            current.profile.notification.enabled = desired.profile.notification.enabled;
        }
        *desired = current;
    }
    library.alarms.retain(|alarm| alarm.id != id);
    if let Some(alarm) = replacement {
        library.alarms.push(alarm);
    }
    library.alarms.sort_by_key(|alarm| alarm.at);
    alarms::write(&path, &library).map_err(|e| e.to_string())
}

#[component]
pub fn AlarmsSheet(
    config: Signal<Config>,
    day: chrono::NaiveDate,
    on_close: EventHandler<()>,
) -> Element {
    let mut library = use_signal(Library::default);
    let mut error = use_signal(|| None::<String>);
    let mut baseline = use_signal(|| None::<Alarm>);
    let mut draft = use_signal(|| None::<Alarm>);
    let mut date = use_signal(|| day.to_string());
    let mut time = use_signal(|| {
        (Local::now() + chrono::Duration::minutes(5))
            .format("%H:%M")
            .to_string()
    });
    use_future(move || async move {
        loop {
            match load() {
                Ok(found) => library.set(found),
                Err(message) => error.set(Some(message)),
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
    let mut open = move |alarm: Alarm| {
        date.set(alarm.at.format("%Y-%m-%d").to_string());
        time.set(alarm.at.format("%H:%M").to_string());
        baseline.set(Some(alarm.clone()));
        draft.set(Some(alarm));
        error.set(None);
    };
    rsx! {
        div { class: "modal-backdrop", onclick: move |_| on_close.call(()),
            div { class: "modal", role: "dialog", aria_label: "Alarms", onclick: move |e| e.stop_propagation(),
                div { class: "modal-head",
                    h2 { class: "modal-title", "Alarms" }
                    div { class: "spacer" }
                    button { class: "modal-close", title: "Close", onclick: move |_| on_close.call(()), "×" }
                }
                div { class: "modal-body",
                    if let Some(message) = error() { p { class: "error-text", "{message}" } }
                    p { class: "hint", "Alarms sync between your devices. Android delivers them as notifications; allow notifications in Settings → Reminders." }
                    button { class: "btn", onclick: move |_| {
                        let now = Local::now();
                        let at = now + chrono::Duration::minutes(5);
                        let mut profile = config.peek().alarm.clone();
                        profile.notification.summary = "Alarm".into();
                        open(Alarm { id: alarms::new_id(), at, label: String::new(), profile });
                        baseline.set(None);
                        if day > now.date_naive() { date.set(day.to_string()); }
                        crate::reminders::request_permission();
                    }, "Add alarm" }
                    if let Some(alarm) = draft() {
                        label { r#for: "alarm-label", "Label" }
                        input { id: "alarm-label", class: "client-input", value: "{alarm.label}", oninput: move |e| { if let Some(a) = draft.write().as_mut() { a.label = e.value(); } } }
                        label { r#for: "alarm-date", "Date" }
                        input { id: "alarm-date", class: "client-input", r#type: "date", value: "{date}", oninput: move |e| date.set(e.value()) }
                        label { r#for: "alarm-time", "Time" }
                        input { id: "alarm-time", class: "client-input", r#type: "time", value: "{time}", oninput: move |e| time.set(e.value()) }
                        label { input { r#type: "checkbox", checked: alarm.profile.sound.enabled, onchange: move |e| { if let Some(a) = draft.write().as_mut() { a.profile.sound.enabled = e.checked(); } } } "Sound" }
                        select { class: "client-input", aria_label: "Alarm sound", onchange: move |e| {
                            if let Ok(index) = e.value().parse::<usize>() {
                                if let Some(tone) = Tone::ALL.get(index) { if let Some(a) = draft.write().as_mut() { a.profile.sound.choice = SoundChoice::Builtin(*tone); } }
                            }
                        },
                            if matches!(alarm.profile.sound.choice, SoundChoice::Custom { .. }) {
                                option { value: "custom", selected: true, "Desktop audio file (Android uses default sound)" }
                            }
                            for (index, tone) in Tone::ALL.into_iter().enumerate() {
                                option { value: "{index}", selected: alarm.profile.sound.choice == SoundChoice::Builtin(tone), "{tone}" }
                            }
                        }
                        label { input { r#type: "checkbox", checked: alarm.profile.notification.enabled, onchange: move |e| { if let Some(a) = draft.write().as_mut() { a.profile.notification.enabled = e.checked(); } } } "Notification" }
                        button { class: "btn", onclick: move |_| {
                            let result = (|| {
                                let local = NaiveDateTime::parse_from_str(&format!("{} {}", date(), time()), "%Y-%m-%d %H:%M").map_err(|_| "Choose a valid date and time".to_string())?;
                                let at = Local.from_local_datetime(&local).earliest().ok_or("That time does not exist on this day")?;
                                if at.timestamp() / 60 <= Local::now().timestamp() / 60 { return Err("Choose a future time".into()); }
                                let mut alarm = draft().ok_or("Choose an alarm")?;
                                alarm.at = at;
                                change(alarm.id, Some(alarm), baseline())
                            })();
                            match result {
                                Ok(()) => { draft.set(None); error.set(None); if let Ok(found) = load() { library.set(found); } crate::reminders::republish(&config.peek()); }
                                Err(message) => error.set(Some(message)),
                            }
                        }, "Save alarm" }
                        button { class: "btn", onclick: move |_| draft.set(None), "Cancel" }
                    }
                    if library().alarms.iter().all(|a| a.at <= Local::now()) { p { class: "hint", "No upcoming alarms." } }
                    for alarm in library().alarms.into_iter().filter(|a| a.at > Local::now()) {
                        div { key: "{alarm.id}", class: "source-row",
                            button { class: "btn", onclick: { let alarm = alarm.clone(); move |_| open(alarm.clone()) },
                                {format!("{} {}", alarm.at.format("%a %b %d, %H:%M"), alarm.label)}
                            }
                            button { class: "btn", aria_label: "Delete alarm", onclick: move |_| {
                                match change(alarm.id, None, None) {
                                    Ok(()) => { if draft.peek().as_ref().is_some_and(|a| a.id == alarm.id) { draft.set(None); } if let Ok(found) = load() { library.set(found); } crate::reminders::republish(&config.peek()); }
                                    Err(message) => error.set(Some(message)),
                                }
                            }, "Delete" }
                        }
                    }
                }
            }
        }
    }
}
