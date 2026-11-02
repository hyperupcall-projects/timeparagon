//! Data for the Android home-screen widget.
//!
//! A widget cannot host the Dioxus WebView, so `android/kotlin` draws it
//! natively from `widget-day.json`, which this module writes into the app's
//! files directory. The app publishes every day it shows for today, and
//! `WidgetRefreshService` calls `refresh` to sync while the app is closed.

#[cfg(any(target_os = "android", test))]
use chrono::NaiveDate;
use serde::Serialize;
use timeslay_core::Color;
use timeslay_core::calendar::layout::{self, Span};
use timeslay_core::calendar::{self, Event, time_format};
use timeslay_core::config::CalendarSettings;
use timeslay_core::timeblocking as tb;

use crate::day::{self, DayModel, TimedEvent};

/// The shortest an item is drawn, so a 15-minute task still fits a line of text.
/// Overlaps are packed at this height because that is what shares the screen.
const MIN_DISPLAY_MINUTES: i32 = 30;

const FORMAT: time_format::TimeFormat = time_format::TimeFormat::Compact;

#[derive(Debug, Serialize)]
struct WidgetDay {
    /// Calendar day as `YYYY-MM-DD`.
    day: String,
    /// Minutes after midnight at which the displayed day starts.
    day_start_minutes: i32,
    updated: String,
    hours: Vec<String>,
    all_day: Vec<WidgetAllDay>,
    /// Active tasks without a time or a block.
    loose_tasks: Vec<WidgetTask>,
    items: Vec<WidgetItem>,
}

#[derive(Debug, Serialize)]
struct WidgetAllDay {
    title: String,
    color: String,
}

#[derive(Debug, Serialize)]
struct WidgetItem {
    kind: ItemKind,
    title: String,
    time: String,
    color: String,
    start: i32,
    end: i32,
    column: usize,
    columns: usize,
    done: bool,
    overdue: bool,
    tasks: Vec<WidgetTask>,
}

#[derive(Debug, Serialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum ItemKind {
    Event,
    Block,
    Task,
    Sleep,
}

#[derive(Debug, Serialize)]
struct WidgetTask {
    title: String,
    done: bool,
    overdue: bool,
}

fn hex(color: Color) -> String {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(color.r),
        channel(color.g),
        channel(color.b)
    )
}

fn widget_task(event: &Event, status: tb::TaskStatus) -> WidgetTask {
    WidgetTask {
        title: event.title.clone(),
        done: status != tb::TaskStatus::Active,
        overdue: tb::overdue(event),
    }
}

fn item_kind(timed: &TimedEvent) -> ItemKind {
    if timed.event.details.sleep.is_some() {
        ItemKind::Sleep
    } else if timed.time_block {
        ItemKind::Block
    } else if timed.event.is_task() {
        ItemKind::Task
    } else {
        ItemKind::Event
    }
}

fn widget_day(
    model: &DayModel,
    settings: &CalendarSettings,
    updated: chrono::DateTime<chrono::Local>,
) -> WidgetDay {
    let spans: Vec<Span> = model
        .timed
        .iter()
        .map(|timed| Span {
            start_min: timed.start_min,
            end_min: timed.end_min.max(timed.start_min + MIN_DISPLAY_MINUTES),
        })
        .collect();
    // The app's columns already rank the sources left to right.
    let ranks: Vec<usize> = model.timed.iter().map(|timed| timed.column).collect();
    let packed = layout::pack_prioritized(&spans, &ranks);

    let items = model
        .timed
        .iter()
        .zip(&spans)
        .zip(packed)
        .map(|((timed, span), slot)| {
            let block = timed.time_block.then(|| {
                model
                    .blocks
                    .iter()
                    .find(|block| block.event.id == timed.event.id)
            });
            // The widget only has room for some of a block's tasks, so the ones
            // left to do come first and keep their planned order.
            let mut tasks: Vec<WidgetTask> = block
                .flatten()
                .map(|block| {
                    block
                        .tasks
                        .iter()
                        .map(|task| widget_task(&task.event, task.status))
                        .collect()
                })
                .unwrap_or_default();
            tasks.sort_by_key(|task| task.done);
            WidgetItem {
                kind: item_kind(timed),
                title: timed.title.clone(),
                time: time_format::event(&timed.event, FORMAT),
                color: hex(timed.event.display_color(settings)),
                start: span.start_min,
                end: span.end_min,
                column: slot.column,
                columns: slot.columns,
                done: timed.event.is_task() && timed.event.completed,
                overdue: tb::overdue(&timed.event),
                tasks,
            }
        })
        .collect();

    // A task with a place on the grid is already drawn there.
    let placed = |event: &Event| {
        model.timed.iter().any(|timed| timed.event.id == event.id)
            || model
                .blocks
                .iter()
                .any(|block| block.tasks.iter().any(|task| task.event.id == event.id))
    };
    let loose_tasks = model
        .task_groups
        .iter()
        .flat_map(|group| &group.tasks)
        .filter(|task| task.status == tb::TaskStatus::Active && !placed(&task.event))
        .map(|task| widget_task(&task.event, task.status))
        .collect();

    WidgetDay {
        day: model.day.format("%Y-%m-%d").to_string(),
        day_start_minutes: settings.day_end_minutes(),
        updated: time_format::clock(updated, FORMAT),
        hours: (0..24)
            .map(|row| time_format::hour(day::hour_at(model.hour_start, row), FORMAT))
            .collect(),
        all_day: model
            .all_day
            .iter()
            .map(|event| WidgetAllDay {
                title: event.title.clone(),
                color: hex(event.display_color(settings)),
            })
            .collect(),
        loose_tasks,
        items,
    }
}

#[cfg(target_os = "android")]
fn path() -> Option<std::path::PathBuf> {
    // `prepare_android_storage` points HOME at `Context.getFilesDir()`.
    Some(std::path::PathBuf::from(std::env::var_os("HOME")?).join("widget-day.json"))
}

/// Saves the day for the widget. Returns whether the file changed.
#[cfg(target_os = "android")]
fn write(
    model: &DayModel,
    settings: &CalendarSettings,
    updated: chrono::DateTime<chrono::Local>,
) -> bool {
    let Some(path) = path() else {
        return false;
    };
    let Ok(json) = serde_json::to_vec(&widget_day(model, settings, updated)) else {
        return false;
    };
    if std::fs::read(&path).is_ok_and(|current| current == json) {
        return false;
    }
    use timeslay_core::fsutil::{Kind, write_atomic};
    write_atomic(&path, &json, Kind::Cache).is_ok()
}

/// Publishes the day the app is showing, if it is today.
#[cfg(target_os = "android")]
pub fn publish(
    model: &DayModel,
    today: NaiveDate,
    settings: &CalendarSettings,
    updated: chrono::DateTime<chrono::Local>,
) {
    if model.day == today && write(model, settings, updated) {
        notify();
    }
}

/// Tells the widget to redraw. Only framework classes are looked up here,
/// because this runs on threads whose class loader cannot see the app's.
#[cfg(target_os = "android")]
pub fn notify() {
    use jni::objects::{JObject, JValue};

    let result = (|| -> jni::errors::Result<()> {
        let context = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }?;
        let mut env = vm.attach_current_thread()?;
        // Borrow the runtime-owned context; do not delete its global reference.
        let activity = unsafe { JObject::from_raw(context.context().cast()) };
        env.with_local_frame(16, |env| -> jni::errors::Result<()> {
            let action = env.new_string("dev.timeslay.day.widget.DATA_CHANGED")?;
            let intent = env.new_object(
                "android/content/Intent",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&action)],
            )?;
            let package = env
                .call_method(&activity, "getPackageName", "()Ljava/lang/String;", &[])?
                .l()?;
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
            Ok(())
        })
    })();
    if result.is_err()
        && let Ok(context) = std::panic::catch_unwind(ndk_context::android_context)
        && let Ok(vm) = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }
        && let Ok(mut env) = vm.attach_current_thread()
    {
        // A Java exception must not poison later calls on this thread.
        let _ = env.exception_clear();
    }
}

/// Syncs today and saves it for the widget. Blocking; returns false to retry.
#[cfg(target_os = "android")]
fn refresh() -> bool {
    let config = day::load_config();
    if !day::signed_in() {
        // Nothing to sync with until the user signs in, so retrying cannot help.
        return true;
    }
    let today = day::today(&config.calendar);
    let Ok(snapshot) = day::load_day(today, &config.calendar) else {
        return false;
    };
    let model = day::build_model(&snapshot, &config.calendar);
    write(&model, &config.calendar, snapshot.fetched_at);
    true
}

/// `WidgetRefreshService.nativeRefresh`. The service runs in its own `:widget`
/// process, so nothing else in there has set up `ndk_context` or the storage
/// paths, and this cannot collide with the WebView activity doing the same.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_timeslay_day_widget_WidgetRefreshService_nativeRefresh(
    env: jni::JNIEnv<'_>,
    _service: jni::objects::JObject<'_>,
    context: jni::objects::JObject<'_>,
) -> jni::sys::jboolean {
    static PREPARED: std::sync::Once = std::sync::Once::new();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        PREPARED.call_once(|| {
            let vm = env.get_java_vm().expect("the JVM called us");
            let context = env.new_global_ref(context).expect("a valid context");
            // SAFETY: both stay valid for the process; the reference is leaked.
            unsafe {
                ndk_context::initialize_android_context(
                    vm.get_java_vm_pointer().cast(),
                    context.as_raw().cast(),
                );
            }
            std::mem::forget(context);
            crate::prepare_android_storage();
        });
        refresh()
    }));
    u8::from(result.unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration, Local, TimeZone};
    use timeslay_core::calendar::{Calendar, Details, Snapshot};
    use timeslay_core::task_db::TaskData;
    use timeslay_core::timeblocking::Metadata;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 26).unwrap()
    }

    fn at(hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .from_local_datetime(&date().and_hms_opt(hour, minute, 0).unwrap())
            .single()
            .unwrap()
    }

    fn event(id: &str, calendar_id: &str, start: DateTime<Local>, end: DateTime<Local>) -> Event {
        Event {
            id: id.to_owned(),
            calendar_id: calendar_id.to_owned(),
            title: id.to_owned(),
            start,
            end,
            all_day: false,
            color: calendar::colors::DEFAULT_EVENT,
            completed: false,
            details: Details::default(),
        }
    }

    fn task(id: &str, block: Option<&str>, order: i64, completed: bool) -> Event {
        let mut task = event(id, "tasks:work", at(0, 0), at(0, 0) + Duration::days(1));
        task.all_day = true;
        task.completed = completed;
        task.details.task_data = Some(TaskData {
            id: order,
            metadata: Metadata {
                block: block.map(str::to_owned),
                order,
                ..Metadata::default()
            },
            legacy_key: None,
        });
        task
    }

    fn calendar(id: &str) -> Calendar {
        Calendar {
            id: id.to_owned(),
            name: id.to_owned(),
            color: calendar::colors::DEFAULT_EVENT,
            primary: false,
            selected_by_google: true,
        }
    }

    fn build(events: Vec<Event>) -> WidgetDay {
        let settings = CalendarSettings {
            app_calendar: Some("blocks".into()),
            ..CalendarSettings::default()
        };
        let snapshot = Snapshot {
            calendars: vec![calendar("blocks"), calendar("work"), calendar("tasks:work")],
            events,
            day: date(),
            fetched_at: at(10, 0),
            notice: None,
        };
        let model = day::build_model(&snapshot, &settings);
        widget_day(&model, &settings, snapshot.fetched_at)
    }

    #[test]
    fn a_block_carries_its_tasks_with_the_unfinished_ones_first() {
        let day = build(vec![
            event("Study", "blocks", at(18, 15), at(19, 30)),
            task("Read", Some("blocks/Study"), 1, true),
            task("Write", Some("blocks/Study"), 2, false),
            task("Review", Some("blocks/Study"), 3, false),
            task("Loose", None, 4, false),
            task("Finished", None, 5, true),
        ]);

        assert_eq!(day.items.len(), 1);
        let block = &day.items[0];
        assert_eq!(block.kind, ItemKind::Block);
        assert_eq!(block.time, "6.15p – 7.30p");
        let titles: Vec<_> = block.tasks.iter().map(|task| task.title.as_str()).collect();
        assert_eq!(titles, ["Write", "Review", "Read"]);
        assert!(block.tasks[2].done);

        // Block tasks are not repeated, and finished tasks are not listed.
        let loose: Vec<_> = day
            .loose_tasks
            .iter()
            .map(|task| task.title.as_str())
            .collect();
        assert_eq!(loose, ["Loose"]);
    }

    #[test]
    fn short_items_that_would_collide_on_screen_share_columns() {
        // These do not overlap in time, but 15 minutes draws as 30.
        let day = build(vec![
            event("A", "work", at(9, 0), at(9, 15)),
            event("B", "work", at(9, 15), at(9, 30)),
            event("C", "work", at(11, 0), at(12, 0)),
        ]);

        assert_eq!(day.items[0].end, 9 * 60 + 30);
        assert_eq!((day.items[0].columns, day.items[1].columns), (2, 2));
        assert_ne!(day.items[0].column, day.items[1].column);
        assert_eq!(day.items[2].columns, 1);
    }

    #[test]
    fn the_file_format_matches_what_the_kotlin_reader_expects() {
        let day = build(vec![event("Standup", "work", at(9, 0), at(10, 0))]);
        let json = serde_json::to_value(&day).unwrap();

        assert_eq!(json["day"], "2026-10-26");
        assert_eq!(json["hours"][15], "3p");
        assert_eq!(json["updated"], "10a");
        assert_eq!(json["items"][0]["kind"], "event");
        assert_eq!(json["items"][0]["start"], 540);
        assert_eq!(json["items"][0]["end"], 600);
        assert!(json["items"][0]["color"].as_str().unwrap().starts_with('#'));
    }
}
