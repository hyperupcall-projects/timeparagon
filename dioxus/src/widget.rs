//! Data for the Android home-screen widget.
//!
//! A widget cannot host the Dioxus WebView, so `android/kotlin` draws it
//! natively from `widget-day.json`, which this module writes into the app's
//! files directory. The app publishes every day it shows for today, and
//! `WidgetRefreshService` calls `refresh` to sync while the app is closed.

#[cfg(any(target_os = "android", test))]
use chrono::NaiveDate;
use serde::Serialize;
use timeparagon_core::Color;
use timeparagon_core::calendar::layout::{self, Span};
use timeparagon_core::calendar::{self, Event, time_format};
use timeparagon_core::config::CalendarSettings;

use crate::day::{self, DayModel, TimedEvent, TravelBox};

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
	items: Vec<WidgetItem>,
}

#[derive(Debug, Serialize)]
struct WidgetAllDay {
	title: String,
	color: String,
}

#[derive(Debug, Serialize)]
struct WidgetItem {
	title: String,
	time: String,
	color: String,
	start: i32,
	end: i32,
	column: usize,
	columns: usize,
	/// The trip that leads to this item, drawn just above it.
	#[serde(skip_serializing_if = "Option::is_none")]
	travel: Option<WidgetTravel>,
}

#[derive(Debug, Serialize)]
struct WidgetTravel {
	label: String,
	color: String,
	/// How long the trip takes, or 0 when the desktop has not worked it out.
	minutes: u32,
	/// Half as wide as the item, as over a time block in the app.
	half_width: bool,
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

/// Tasks stay in the app. A scheduled task's calendar copy is one too.
fn is_todo(event: &Event) -> bool {
	event.is_task() || event.is_calendar_task()
}

fn widget_day(
	model: &DayModel,
	travel: &[TravelBox],
	settings: &CalendarSettings,
	updated: chrono::DateTime<chrono::Local>,
) -> WidgetDay {
	let timed: Vec<&TimedEvent> = model
		.timed
		.iter()
		.filter(|timed| !is_todo(&timed.event))
		.collect();
	let spans: Vec<Span> = timed
		.iter()
		.map(|timed| Span {
			start_min: timed.start_min,
			end_min: timed.end_min.max(timed.start_min + MIN_DISPLAY_MINUTES),
		})
		.collect();
	// The app's columns already rank the sources left to right.
	let ranks: Vec<usize> = timed.iter().map(|timed| timed.column).collect();
	let packed = layout::pack_prioritized(&spans, &ranks);

	let items = timed
		.iter()
		.zip(&spans)
		.zip(packed)
		.map(|((timed, span), slot)| WidgetItem {
			title: timed.title.clone(),
			time: time_format::event(&timed.event, FORMAT),
			color: hex(timed.color),
			start: span.start_min,
			end: span.end_min,
			column: slot.column,
			columns: slot.columns,
			travel: travel
				.iter()
				.find(|trip| trip.event_id == timed.event.id)
				.map(|trip| WidgetTravel {
					label: trip.label.clone(),
					color: hex(calendar::colors::TRAVEL),
					minutes: trip.minutes,
					half_width: trip.time_block,
				}),
		})
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
				color: hex(event.display_color(settings, &model.calendars)),
			})
			.collect(),
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
	travel: &[TravelBox],
	settings: &CalendarSettings,
	updated: chrono::DateTime<chrono::Local>,
) -> bool {
	let Some(path) = path() else {
		return false;
	};
	let Ok(json) = serde_json::to_vec(&widget_day(model, travel, settings, updated)) else {
		return false;
	};
	if std::fs::read(&path).is_ok_and(|current| current == json) {
		return false;
	}
	use timeparagon_core::fsutil::{Kind, write_atomic};
	write_atomic(&path, &json, Kind::Cache).is_ok()
}

/// Publishes the day the app is showing, if it is today.
#[cfg(target_os = "android")]
pub fn publish(
	model: &DayModel,
	travel: &[TravelBox],
	today: NaiveDate,
	settings: &CalendarSettings,
	updated: chrono::DateTime<chrono::Local>,
) {
	if model.day == today && write(model, travel, settings, updated) {
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
			let action = env.new_string("dev.kofler.timeparagon.widget.DATA_CHANGED")?;
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
		&& let Ok(env) = vm.attach_current_thread()
	{
		// A Java exception must not poison later calls on this thread.
		let _ = env.exception_clear();
	}
}

/// Syncs today and saves it for the widget. Blocking; returns false to retry.
#[cfg(target_os = "android")]
fn refresh() -> bool {
	let config = day::load_config();
	// Standalone alarms do not depend on Google sign-in or calendar reminders.
	crate::reminders::publish(&config);
	if !day::signed_in() {
		// Nothing to sync with until the user signs in, so retrying cannot help.
		return true;
	}
	let today = day::today(&config.calendar);
	let Ok(snapshot) = day::load_day(today, &config.calendar) else {
		return false;
	};
	let model = day::build_model(&snapshot, &config.calendar);
	let travel = day::travel_boxes(&model, &config.travel, config.appearance.distance_unit);
	write(&model, &travel, &config.calendar, snapshot.fetched_at);
	if config.reminders.any_enabled() {
		refresh_reminders(&config, snapshot, today);
	}
	true
}

/// Caches today and tomorrow, so reminders are planned from fresh events even
/// when the app has not been opened, then re-plans them.
#[cfg(target_os = "android")]
fn refresh_reminders(
	config: &timeparagon_core::config::Config,
	today_snapshot: calendar::Snapshot,
	today: NaiveDate,
) {
	let mut store = calendar::Store::new(config.calendar.day_start_hour());
	store.cache_days_in(timeparagon_core::task_db::path());
	store.cache_fetched_snapshot(today_snapshot, &[today]);
	// Early reminders for tomorrow are due before the next morning's sync.
	if let Some(tomorrow) = today.succ_opt()
		&& let Ok(snapshot) = day::load_day(tomorrow, &config.calendar)
	{
		store.cache_fetched_snapshot(snapshot, &[tomorrow]);
	}
	crate::reminders::publish(config);
}

/// `WidgetRefreshService.nativeRefresh`. The service runs in its own `:widget`
/// process, so nothing else in there has set up `ndk_context` or the storage
/// paths, and this cannot collide with the WebView activity doing the same.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_kofler_timeparagon_widget_WidgetRefreshService_nativeRefresh(
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
			timeparagon_core::database_sync::start();
		});
		refresh()
	}));
	u8::from(result.unwrap_or(false))
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::{DateTime, Duration, Local, TimeZone};
	use timeparagon_core::calendar::{Calendar, Details, Snapshot};
	use timeparagon_core::places::{Place, PlaceSettings, SavedRoute, TravelMode};
	use timeparagon_core::task_db::TaskData;
	use timeparagon_core::timeblocking::Metadata;

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
		build_with_travel(events, &PlaceSettings::default())
	}

	fn build_with_travel(events: Vec<Event>, travel: &PlaceSettings) -> WidgetDay {
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
		let travel = day::travel_boxes(
			&model,
			travel,
			timeparagon_core::places::DistanceUnit::Kilometers,
		);
		widget_day(&model, &travel, &settings, snapshot.fetched_at)
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

	fn campus_trip() -> PlaceSettings {
		let place = |id: u64, name: &str| Place {
			id,
			name: name.to_owned(),
			address: String::new(),
			place_id: String::new(),
			lat: 0.0,
			lng: 0.0,
			favorite: false,
		};
		PlaceSettings {
			home: Some(1),
			places: vec![place(1, "Home"), place(2, "Campus")],
			default_mode: TravelMode::Bike,
			show: true,
			routes: vec![SavedRoute {
				from: 1,
				to: 2,
				mode: TravelMode::Bike,
				seconds: 1190,
				meters: 5300,
			}],
			..PlaceSettings::default()
		}
	}

	fn at_campus(mut event: Event) -> Event {
		event.details.location = "Campus".into();
		event
	}

	#[test]
	fn a_trip_reaches_the_widget_on_the_item_it_leads_to() {
		let day = build_with_travel(
			vec![
				event("Standup", "work", at(8, 0), at(8, 30)),
				at_campus(event("Lecture", "work", at(9, 0), at(10, 0))),
			],
			&campus_trip(),
		);

		assert!(day.items[0].travel.is_none());
		let travel = day.items[1].travel.as_ref().unwrap();
		assert_eq!(travel.label, "Bike · 20 min · 5.3 km");
		assert_eq!(travel.minutes, 20);
		assert!(!travel.half_width);

		let json = serde_json::to_value(&day).unwrap();
		assert!(json["items"][0].get("travel").is_none());
		assert_eq!(json["items"][1]["travel"]["minutes"], 20);
		assert!(
			json["items"][1]["travel"]["color"]
				.as_str()
				.unwrap()
				.starts_with('#')
		);
	}

	#[test]
	fn a_trip_is_half_as_wide_over_a_time_block() {
		let day = build_with_travel(
			vec![at_campus(event("Study", "blocks", at(9, 0), at(10, 0)))],
			&campus_trip(),
		);
		assert!(day.items[0].travel.as_ref().unwrap().half_width);
	}

	#[test]
	fn the_file_format_matches_what_the_kotlin_reader_expects() {
		let day = build(vec![event("Standup", "work", at(9, 0), at(10, 0))]);
		let json = serde_json::to_value(&day).unwrap();

		assert_eq!(json["day"], "2026-10-26");
		assert_eq!(json["hours"][15], "3p");
		assert_eq!(json["updated"], "10a");
		assert_eq!(json["items"][0]["start"], 540);
		assert_eq!(json["items"][0]["end"], 600);
		assert!(json["items"][0]["color"].as_str().unwrap().starts_with('#'));
	}
}
