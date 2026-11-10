//! Reminders on Android, and their settings.
//!
//! The app is not running when most reminders are due, so it cannot deliver
//! them itself. Instead it plans the next day and a half from the day cache
//! (`timeslay_core::reminders`), writes the plan to `reminders.json`, and
//! `android/kotlin/dev/timeslay/day/reminders` hands each entry to
//! `AlarmManager` and posts it on a notification channel that plays the
//! chosen sound. The widget's background sync re-plans while the app is
//! closed, and the plan is re-armed after a reboot.

// The plan is only written on Android; elsewhere just the settings show.
#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use timeslay_core::config::Config;
use timeslay_core::reminders::{Category, LEAD_CHOICES, ReminderSettings, Sound, lead_label};

#[cfg(any(target_os = "android", test))]
use chrono::{DateTime, Local};

/// What `ReminderScheduler.kt` reads.
#[derive(Debug, Serialize)]
struct Plan {
    enabled: bool,
    categories: Vec<CategoryPlan>,
    reminders: Vec<Item>,
}

#[derive(Debug, Serialize)]
struct CategoryPlan {
    id: String,
    label: &'static str,
    enabled: bool,
    color: String,
    sound: SoundPlan,
}

#[derive(Debug, Serialize)]
struct SoundPlan {
    /// Changes whenever the sound does; part of the channel id.
    key: String,
    /// `default`, `silent`, `tone` or `device`.
    kind: &'static str,
    /// The `res/raw` name of a tone.
    resource: String,
    /// A device sound's URI.
    uri: String,
}

#[derive(Debug, Serialize)]
struct Item {
    key: String,
    category: String,
    title: String,
    body: String,
    /// Milliseconds since the epoch.
    at: i64,
}

fn sound_plan(sound: &Sound) -> SoundPlan {
    let (kind, resource, uri) = match sound {
        Sound::Default => ("default", String::new(), String::new()),
        Sound::Silent => ("silent", String::new(), String::new()),
        Sound::Tone { tone } => ("tone", tone.android_resource(), String::new()),
        Sound::Device { uri, .. } => ("device", String::new(), uri.clone()),
    };
    SoundPlan {
        key: sound.key(),
        kind,
        resource,
        uri,
    }
}

fn hex(color: timeslay_core::Color) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(color.r),
        channel(color.g),
        channel(color.b)
    )
}

#[cfg(any(target_os = "android", test))]
fn plan(config: &Config, due: Vec<timeslay_core::reminders::Due>) -> Plan {
    let settings = &config.reminders;
    Plan {
        enabled: true,
        categories: Category::ALL
            .into_iter()
            .map(|category| CategoryPlan {
                id: category.id().into(),
                label: category.label(),
                enabled: settings.enabled && settings.get(category).enabled,
                color: hex(category.color(&config.calendar)),
                sound: sound_plan(&settings.get(category).sound),
            })
            .collect(),
        reminders: due
            .into_iter()
            .map(|due| Item {
                key: due.key,
                category: due.category.id().into(),
                title: due.title,
                body: due.body,
                at: due.at.timestamp_millis(),
            })
            .collect(),
    }
}

#[cfg(any(target_os = "android", test))]
fn append_alarms(
    plan: &mut Plan,
    library: timeslay_core::alarms::Library,
    now: DateTime<Local>,
    format: timeslay_core::calendar::time_format::TimeFormat,
) {
    for alarm in library.alarms {
        if alarm.at < now - chrono::Duration::minutes(5) {
            continue;
        }
        let profile = alarm.firing_profile(format);
        let category = format!("alarm-{}", alarm.id);
        let sound = if !profile.sound.enabled {
            Sound::Silent
        } else {
            match profile.sound.choice {
                timeslay_core::alarms::SoundChoice::Builtin(tone) => Sound::Tone { tone },
                timeslay_core::alarms::SoundChoice::Custom { .. } => Sound::Default,
            }
        };
        plan.categories.push(CategoryPlan {
            id: category.clone(),
            label: "Alarm",
            enabled: profile.notification.enabled || profile.sound.enabled,
            color: "#A855F7".into(),
            sound: sound_plan(&sound),
        });
        plan.reminders.push(Item {
            key: format!("alarm:{}:{}", alarm.id, alarm.at.timestamp_millis()),
            category,
            title: profile.notification.summary,
            body: profile.notification.body,
            at: alarm.at.timestamp_millis(),
        });
    }
}

/// Everything due from `now` to the end of the planning horizon.
#[cfg(target_os = "android")]
fn due(config: &Config, now: DateTime<Local>) -> Vec<timeslay_core::reminders::Due> {
    let Some(database) = timeslay_core::task_db::path() else {
        return Vec::new();
    };
    // A little behind now, so a reminder due this minute is still armed;
    // the scheduler drops the ones it has already posted.
    let from = now - chrono::Duration::minutes(timeslay_core::reminders::LATE_GRACE_MINUTES);
    let until = now + chrono::Duration::hours(timeslay_core::reminders::HORIZON_HOURS);
    timeslay_core::reminders::plan_from_cache(
        &database,
        &config.reminders,
        &config.calendar,
        &config.travel,
        config.appearance.time_format,
        from,
        until,
    )
}

#[cfg(target_os = "android")]
fn files_dir() -> Option<std::path::PathBuf> {
    // `prepare_android_storage` points HOME at `Context.getFilesDir()`.
    Some(std::path::PathBuf::from(std::env::var_os("HOME")?))
}

/// Plans from the day cache and hands the plan to the scheduler. Blocking:
/// it reads the database, so call it off the UI thread.
#[cfg(target_os = "android")]
pub fn publish(config: &Config) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use timeslay_core::fsutil::{Kind, write_atomic};

    // The first publish in a process always re-arms, in case the alarms were
    // lost (an update, a force stop) while the plan on disk stayed the same.
    static ARMED: AtomicBool = AtomicBool::new(false);
    // Settings changes and database sync can publish concurrently. Read and
    // publish in order so an older plan cannot replace a newer alarm edit.
    static PUBLISH: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _publishing = PUBLISH.lock().unwrap_or_else(|error| error.into_inner());

    let Some(path) = files_dir().map(|dir| dir.join("reminders.json")) else {
        return;
    };
    let mut plan = plan(config, due(config, timeslay_core::clock::now()));
    let Some(database) = timeslay_core::task_db::path() else {
        return;
    };
    let Ok(library) = timeslay_core::alarms::read(&database) else {
        return;
    };
    append_alarms(
        &mut plan,
        library,
        timeslay_core::clock::now(),
        config.appearance.time_format,
    );
    let Ok(json) = serde_json::to_vec(&plan) else {
        return;
    };
    let unchanged = std::fs::read(&path).is_ok_and(|current| current == json);
    if unchanged && ARMED.load(Ordering::Relaxed) {
        return;
    }
    if !unchanged && write_atomic(&path, &json, Kind::Private).is_err() {
        // Nothing to arm from; the next publish tries again.
        return;
    }
    ARMED.store(true, Ordering::Relaxed);
    broadcast(ACTION_SYNC, None);
}

#[cfg(target_os = "android")]
const ACTION_SYNC: &str = "dev.timeslay.day.reminders.SYNC";
#[cfg(target_os = "android")]
const ACTION_TEST: &str = "dev.timeslay.day.reminders.TEST";
#[cfg(target_os = "android")]
const ACTION_DEVICE_INFO: &str = "dev.timeslay.day.reminders.DEVICE_INFO";
#[cfg(target_os = "android")]
const PERMISSION_ACTIVITY: &str = "dev.timeslay.day.reminders.PermissionActivity";

/// Sends an intent to the app's own reminder receiver, or starts one of its
/// activities. Only framework classes are looked up, because this runs on
/// threads whose class loader cannot see the app's.
#[cfg(target_os = "android")]
fn send(action: Option<&str>, extras: &[(&str, &str)], activity_class: Option<&str>) {
    use jni::objects::{JObject, JValue};

    let result = (|| -> jni::errors::Result<()> {
        let context = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }?;
        let mut env = vm.attach_current_thread()?;
        // Borrow the runtime-owned context; do not delete its global reference.
        let activity = unsafe { JObject::from_raw(context.context().cast()) };
        env.with_local_frame(16, |env| -> jni::errors::Result<()> {
            let intent = env.new_object("android/content/Intent", "()V", &[])?;
            let package = env
                .call_method(&activity, "getPackageName", "()Ljava/lang/String;", &[])?
                .l()?;
            if let Some(action) = action {
                let action = env.new_string(action)?;
                env.call_method(
                    &intent,
                    "setAction",
                    "(Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(&action)],
                )?;
            }
            for (name, value) in extras {
                let name = env.new_string(name)?;
                let value = env.new_string(value)?;
                env.call_method(
                    &intent,
                    "putExtra",
                    "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                    &[JValue::Object(&name), JValue::Object(&value)],
                )?;
            }
            match activity_class {
                Some(class) => {
                    let class = env.new_string(class)?;
                    env.call_method(
                        &intent,
                        "setClassName",
                        "(Ljava/lang/String;Ljava/lang/String;)Landroid/content/Intent;",
                        &[JValue::Object(&package), JValue::Object(&class)],
                    )?;
                    env.call_method(
                        &activity,
                        "startActivity",
                        "(Landroid/content/Intent;)V",
                        &[JValue::Object(&intent)],
                    )?;
                }
                None => {
                    env.call_method(
                        &intent,
                        "setPackage",
                        "(Ljava/lang/String;)Landroid/content/Intent;",
                        &[JValue::Object(&package)],
                    )?;
                    env.call_method(
                        &activity,
                        "sendBroadcast",
                        "(Landroid/content/Intent;)V",
                        &[JValue::Object(&intent)],
                    )?;
                }
            }
            Ok(())
        })
    })();
    if result.is_err()
        && let Ok(context) = std::panic::catch_unwind(ndk_context::android_context)
        && let Ok(vm) = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }
        && let Ok(env) = vm.attach_current_thread()
    {
        // A Java exception must not poison later calls on this thread.
        let _ = env.exception_clear();
    }
}

#[cfg(target_os = "android")]
fn broadcast(action: &str, category: Option<&str>) {
    let extras: Vec<_> = category
        .map(|category| ("category", category))
        .into_iter()
        .collect();
    send(Some(action), &extras, None);
}

/// Sends an intent with string extras to the app's own reminder receiver,
/// which also owns the sleep timer's notification.
#[cfg(target_os = "android")]
pub(crate) fn broadcast_extras(action: &str, extras: &[(&str, &str)]) {
    send(Some(action), extras, None);
}

/// Posts a sample reminder for a category, with its sound.
fn test(category: Category) {
    #[cfg(target_os = "android")]
    broadcast(ACTION_TEST, Some(category.id()));
    #[cfg(not(target_os = "android"))]
    let _ = category;
}

/// Asks for the device's notification sounds and permission state, which
/// arrive in `reminder-device.json`.
fn request_device_info() {
    #[cfg(target_os = "android")]
    broadcast(ACTION_DEVICE_INFO, None);
}

/// Shows Android's "allow notifications" prompt where the system has one.
pub fn request_permission() {
    #[cfg(target_os = "android")]
    send(None, &[], Some(PERMISSION_ACTIVITY));
}

/// What the device told us about itself.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
struct DeviceInfo {
    notifications_enabled: Option<bool>,
    sounds: Vec<DeviceSound>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct DeviceSound {
    name: String,
    uri: String,
}

fn device_info() -> Option<DeviceInfo> {
    #[cfg(target_os = "android")]
    {
        let path = files_dir()?.join("reminder-device.json");
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }
    #[cfg(not(target_os = "android"))]
    None
}

/// Whether Android has said notifications are switched off for the app.
pub fn notifications_blocked() -> bool {
    device_info().is_some_and(|info| info.notifications_enabled == Some(false))
}

/// Saves the reminder settings and returns the config as it now reads.
fn save(settings: &ReminderSettings) -> Result<Config, String> {
    timeslay_core::config::save_reminders(settings)
        .map_err(|error| format!("Could not save the reminder settings: {error}"))?;
    Ok(Config::load())
}

/// Re-plans in the background after a settings change.
pub fn republish(config: &Config) {
    #[cfg(target_os = "android")]
    {
        let config = config.clone();
        std::thread::spawn(move || publish(&config));
    }
    #[cfg(not(target_os = "android"))]
    let _ = config;
}

fn sound_choices(info: &DeviceInfo, settings: &ReminderSettings) -> Vec<Sound> {
    let mut sounds = Sound::builtin_choices();
    sounds.extend(info.sounds.iter().map(|sound| Sound::Device {
        name: sound.name.clone(),
        uri: sound.uri.clone(),
    }));
    for category in Category::ALL {
        let chosen = &settings.get(category).sound;
        if !sounds.contains(chosen) {
            sounds.push(chosen.clone());
        }
    }
    sounds
}

/// The Reminders part of the settings sheet.
#[component]
pub fn ReminderSection(config: Signal<Config>) -> Element {
    let mut error = use_signal(|| None::<String>);
    let mut info = use_signal(|| device_info().unwrap_or_default());

    // The receiver answers asynchronously, so look for its answer a few
    // times after asking.
    use_future(move || async move {
        request_device_info();
        for _ in 0..8 {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            if let Some(found) = device_info()
                && found != *info.peek()
            {
                info.set(found);
            }
        }
    });

    let mut apply = move |change: Box<dyn FnOnce(&mut ReminderSettings)>| {
        let mut settings = config.peek().reminders.clone();
        change(&mut settings);
        match save(&settings) {
            Ok(saved) => {
                republish(&saved);
                config.set(saved);
                error.set(None);
            }
            Err(message) => error.set(Some(message)),
        }
    };

    let settings = config.read().reminders.clone();
    let calendar = config.read().calendar.clone();
    let enabled = settings.enabled;
    let error_text = error().map(|message| rsx! { p { class: "error-text", "{message}" } });
    let blocked = enabled && info().notifications_enabled == Some(false);

    let categories = enabled.then(|| {
        let sounds = sound_choices(&info(), &settings);
        let rows = Category::ALL.into_iter().map(|category| {
            let current = settings.get(category).clone();
            let swatch = format!("background:{};", crate::css_color(category.color(&calendar)));
            let on = current.enabled;
            let lead_text = if category == Category::Events && settings.use_calendar_reminders {
                "Otherwise remind"
            } else {
                category.lead_label()
            };
            let selected_sound = sounds.iter().position(|sound| *sound == current.sound);
            let sound_options = sounds.iter().enumerate().map(|(index, sound)| {
                let group_break = matches!(sound, Sound::Device { .. })
                    && !matches!(sounds.get(index.wrapping_sub(1)), Some(Sound::Device { .. }));
                let label = if group_break {
                    format!("Device · {}", sound.label())
                } else {
                    sound.label()
                };
                rsx! {
                    option {
                        key: "{index}",
                        value: "{index}",
                        selected: Some(index) == selected_sound,
                        "{label}"
                    }
                }
            });
            let sounds_for_change = sounds.clone();
            let details = on.then(|| rsx! {
                div { class: "reminder-details",
                    if category == Category::Events {
                        button {
                            class: "source-row reminder-subrow",
                            role: "switch",
                            aria_checked: settings.use_calendar_reminders.to_string(),
                            onclick: {
                                let value = !settings.use_calendar_reminders;
                                move |_| apply(Box::new(move |settings| {
                                    settings.use_calendar_reminders = value;
                                }))
                            },
                            span { class: "source-name", "Use each event's Google Calendar reminders" }
                            span { class: "source-switch", span { class: "source-knob" } }
                        }
                    }
                    label { r#for: "reminder-lead-{category.id()}", "{lead_text}" }
                    select {
                        id: "reminder-lead-{category.id()}",
                        class: "client-input",
                        onchange: move |event: FormEvent| {
                            if let Ok(minutes) = event.value().parse::<u32>() {
                                apply(Box::new(move |settings| {
                                    settings.get_mut(category).minutes_before = minutes;
                                }));
                            }
                        },
                        for minutes in LEAD_CHOICES {
                            option {
                                key: "{minutes}",
                                value: "{minutes}",
                                selected: minutes == current.minutes_before,
                                if minutes == 0 { "{lead_label(minutes)}" } else { "{lead_label(minutes)} before" }
                            }
                        }
                    }
                    label { r#for: "reminder-sound-{category.id()}", "Sound" }
                    div { class: "reminder-sound",
                        select {
                            id: "reminder-sound-{category.id()}",
                            class: "client-input",
                            onchange: move |event: FormEvent| {
                                let chosen = event
                                    .value()
                                    .parse::<usize>()
                                    .ok()
                                    .and_then(|index| sounds_for_change.get(index).cloned());
                                if let Some(sound) = chosen {
                                    apply(Box::new(move |settings| {
                                        settings.get_mut(category).sound = sound;
                                    }));
                                }
                            },
                            {sound_options}
                        }
                        button {
                            class: "btn",
                            r#type: "button",
                            title: "Post a sample reminder with this sound",
                            onclick: move |_| test(category),
                            "Test"
                        }
                    }
                }
            });
            rsx! {
                div { key: "{category.id()}", class: "reminder-category",
                    button {
                        class: "source-row",
                        role: "switch",
                        aria_checked: on.to_string(),
                        onclick: move |_| apply(Box::new(move |settings| {
                            settings.get_mut(category).enabled = !on;
                        })),
                        span { class: "source-swatch", style: "{swatch}" }
                        span { class: "source-name", "{category.label()}" }
                        span { class: "source-switch", span { class: "source-knob" } }
                    }
                    {details}
                }
            }
        });
        rsx! { {rows} }
    });

    rsx! {
        h3 { class: "settings-heading", "Reminders" }
        {error_text}
        button {
            class: "source-row",
            role: "switch",
            aria_checked: enabled.to_string(),
            onclick: move |_| {
                if !enabled {
                    request_permission();
                }
                apply(Box::new(move |settings| settings.enabled = !enabled));
            },
            span { class: "source-name", "Remind me of what is coming up" }
            span { class: "source-switch", span { class: "source-knob" } }
        }
        if blocked {
            div { class: "reminder-blocked",
                p { class: "hint", "Notifications are turned off for TimeSlay." }
                button {
                    class: "btn",
                    r#type: "button",
                    onclick: move |_| request_permission(),
                    "Allow notifications"
                }
            }
        }
        if enabled {
            p { class: "hint",
                "Each kind gets its own Android notification channel, so its sound also shows in the system's notification settings."
            }
        }
        {categories}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use timeslay_core::reminders::Due;
    use timeslay_core::sounds::Tone;

    #[test]
    fn alarms_are_planned_independently_of_reminders_and_keep_their_sound() {
        use timeslay_core::alarms::{Alarm, AlarmProfile, Library, SoundChoice};
        let config = Config::default();
        let now = timeslay_core::clock::now();
        let mut profile = AlarmProfile::default();
        profile.sound.choice = SoundChoice::Builtin(Tone::Bell);
        let alarm = Alarm {
            id: 42,
            at: now + chrono::Duration::days(3),
            label: "Tea".into(),
            profile,
        };
        let mut plan = plan(&config, vec![]);
        append_alarms(
            &mut plan,
            Library {
                alarms: vec![alarm.clone()],
            },
            now,
            config.appearance.time_format,
        );
        assert!(plan.enabled);
        assert!(
            plan.categories
                .iter()
                .filter(|c| c.id != "alarm-42")
                .all(|c| !c.enabled)
        );
        let category = plan.categories.last().unwrap();
        assert!(category.enabled);
        assert_eq!(category.sound.resource, "timeslay_tone_bell");
        assert_eq!(plan.reminders[0].title, "Tea");
        assert_eq!(plan.reminders[0].at, alarm.at.timestamp_millis());
        let mut expired = alarm;
        expired.at = now - chrono::Duration::minutes(6);
        let count = plan.reminders.len();
        append_alarms(
            &mut plan,
            Library {
                alarms: vec![expired],
            },
            now,
            config.appearance.time_format,
        );
        assert_eq!(plan.reminders.len(), count);
    }

    #[test]
    fn the_plan_carries_every_category_and_each_reminder() {
        let mut config = Config::default();
        config.reminders.enabled = true;
        config.reminders.tasks.sound = Sound::Tone {
            tone: Tone::Kalimba,
        };
        config.reminders.sleep.sound = Sound::Device {
            name: "Chirp".into(),
            uri: "content://media/internal/audio/media/7".into(),
        };
        let at: DateTime<Local> = DateTime::from_timestamp(1_800_000_000, 0).unwrap().into();
        let plan = plan(
            &config,
            vec![Due {
                key: "events:x:1:10".into(),
                category: Category::Events,
                event_id: "x".into(),
                title: "Standup".into(),
                body: "In 10 min".into(),
                at,
            }],
        );
        let json = serde_json::to_value(&plan).unwrap();

        assert_eq!(json["enabled"], true);
        assert_eq!(json["categories"].as_array().unwrap().len(), 6);
        assert_eq!(json["categories"][1]["sound"]["kind"], "tone");
        assert_eq!(
            json["categories"][1]["sound"]["resource"],
            "timeslay_tone_kalimba"
        );
        assert_eq!(json["categories"][4]["sound"]["kind"], "device");
        assert_eq!(json["reminders"][0]["at"], 1_800_000_000_000_i64);
        assert_eq!(json["reminders"][0]["category"], "events");
    }
}
