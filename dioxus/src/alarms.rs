//! The mobile editor uses the same alarm document and profiles as desktop.
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike};
use dioxus::prelude::*;
use timeparagon_core::{
	alarms::{self, Alarm, Library, SoundChoice},
	calendar::time_format::{self, TimeFormat},
	config::Config,
	sounds::Tone,
};

/// An unsaved wall-clock selection, shared by the calendar and its action bar.
#[derive(Clone, Copy)]
pub struct Placement(pub Signal<Option<NaiveDateTime>>);

pub fn default_placement(day: NaiveDate, hour_start: u32) -> NaiveDateTime {
	let soon = (Local::now() + Duration::minutes(5)).naive_local();
	let start = day.and_hms_opt(hour_start, 0, 0).unwrap();
	if soon < start {
		day.and_hms_opt(hour_start.max(9), 0, 0).unwrap()
	} else {
		soon.with_second(0).unwrap().with_nanosecond(0).unwrap()
	}
}

/// Grid minutes are wall-clock minutes, including across a DST transition.
pub fn placement_time(day: NaiveDate, hour_start: u32, minute: u32) -> Option<NaiveDateTime> {
	if minute >= 1440 {
		return None;
	}
	day.and_hms_opt(hour_start, 0, 0)?
		.checked_add_signed(Duration::minutes(i64::from(minute)))
}

fn future_time(at: NaiveDateTime, now: DateTime<Local>) -> Result<DateTime<Local>, String> {
	let at = Local
		.from_local_datetime(&at)
		.earliest()
		.ok_or("That time does not exist on this day")?;
	if at.timestamp() / 60 <= now.timestamp() / 60 {
		return Err("Choose a future time".into());
	}
	Ok(at)
}

#[component]
pub fn AlarmNeedle(day: NaiveDate, hour_start: u32, time_format: TimeFormat) -> Element {
	let Some(Placement(placement)) = try_use_context::<Placement>() else {
		return rsx! {};
	};
	let Some(at) = placement() else {
		return rsx! {};
	};
	let start = day.and_hms_opt(hour_start, 0, 0).unwrap();
	let minute = (at - start).num_minutes();
	if !(0..1440).contains(&minute) {
		return rsx! {};
	}
	let top = minute as f32 / 1440.0 * 100.0;
	let clock = time_format::clock(at, time_format);
	rsx! {
		div {
			class: "alarm-needle", style: "top:{top}%;", role: "slider", tabindex: "0",
			aria_label: "Alarm time", aria_orientation: "vertical",
			aria_valuemin: "0", aria_valuemax: "1439", aria_valuenow: "{minute}",
			aria_valuetext: "{at.format(\"%A, %B %-d\")} {clock}",
			"data-minute": minute,
			span { class: "alarm-needle-dot" }
			span { class: "alarm-needle-time", "{clock}" }
		}
	}
}

/// A nonmodal footer leaves the whole calendar available for scrolling/zooming.
#[component]
pub fn PlacementControls(config: Signal<Config>) -> Element {
	let Placement(mut placement) = use_context::<Placement>();
	let mut error = use_signal(|| None::<String>);
	let mut saved = use_signal(|| None::<DateTime<Local>>);
	let mut now = use_signal(Local::now);
	use_future(move || async move {
		loop {
			tokio::time::sleep(std::time::Duration::from_secs(1)).await;
			if placement.peek().is_some() {
				now.set(Local::now());
			}
		}
	});
	// Changing the selection clears an error from a previous attempt.
	use_effect(move || {
		if placement().is_some() {
			saved.set(None);
		}
		error.set(None);
	});
	let format = config.read().appearance.time_format;
	rsx! {
		if let Some(at) = placement() {
			div { class: "alarm-placement-bar", role: "region", aria_label: "Set alarm",
				div { class: "alarm-placement-summary", aria_live: "polite",
					strong { "{time_format::clock(at, format)} · {at.format(\"%a, %b %-d\")}" }
					span { class: "hint", "Drag the purple needle or tap a time. Scroll and pinch to zoom." }
				}
				if let Some(message) = error().or_else(|| future_time(at, now()).err()) {
					p { class: "error-text", role: "alert", "{message}" }
				}
				div { class: "alarm-placement-actions",
					button { class: "btn", onclick: move |_| placement.set(None), "Cancel" }
					button { class: "btn primary", disabled: future_time(at, now()).is_err(), onclick: move |_| {
						let result = (|| {
							let at = future_time(placement().ok_or("Choose a time")?, Local::now())?;
							let mut profile = config.peek().alarm.clone();
							profile.notification.summary = "Alarm".into();
							let alarm = Alarm { id: alarms::new_id(), at, label: String::new(), profile };
							change(alarm.id, Some(alarm), None)?;
							Ok::<_, String>(at)
						})();
						match result {
							Ok(at) => {
								placement.set(None);
								saved.set(Some(at));
								crate::reminders::request_permission();
								crate::reminders::republish(&config.peek());
							}
							Err(message) => error.set(Some(message)),
						}
					}, "Set alarm" }
				}
			}
		} else if let Some(at) = saved() {
			div { class: "banner", role: "status",
				span { class: "banner-text", "Alarm set for {time_format::clock(at, format)} · {at.format(\"%a, %b %-d\")}" }
				button { class: "btn small", onclick: move |_| saved.set(None), "Dismiss" }
			}
		}
	}
}

#[cfg(test)]
mod placement_tests {
	use super::*;
	use timeparagon_core::calendar::{self, Snapshot};

	#[test]
	fn selection_uses_wall_clock_minutes_and_the_calendar_day_boundary() {
		let day = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
		assert_eq!(placement_time(day, 3, 0), day.and_hms_opt(3, 0, 0));
		assert_eq!(
			placement_time(day, 3, 1290),
			day.succ_opt().unwrap().and_hms_opt(0, 30, 0)
		);
		assert_eq!(placement_time(day, 0, 1439), day.and_hms_opt(23, 59, 0));
		assert!(placement_time(day, 0, 1440).is_none());
		assert!(placement_time(day, 24, 0).is_none());
	}

	#[test]
	fn confirmation_requires_at_least_the_next_clock_minute() {
		let now = Local::now();
		let minute = now
			.naive_local()
			.with_second(0)
			.unwrap()
			.with_nanosecond(0)
			.unwrap();
		assert!(future_time(minute, now).is_err());
		assert!(future_time(minute - Duration::minutes(1), now).is_err());
		let later = now + Duration::hours(24);
		assert!(future_time(later.naive_local(), now).is_ok());
	}

	fn preview() -> Element {
		let day = NaiveDate::from_ymd_opt(2099, 11, 17).unwrap();
		let at = day.and_hms_opt(10, 30, 0).unwrap();
		let placement = use_signal(|| Some(at));
		use_context_provider(|| Placement(placement));
		let config = use_signal(Config::default);
		let zoom = use_signal(|| 1.0);
		let open_block = use_signal(|| None::<String>);
		let editing = use_signal(|| None::<calendar::Event>);
		let snapshot = Snapshot {
			day,
			calendars: vec![],
			events: vec![],
			fetched_at: Local::now(),
			notice: None,
		};
		let settings = config.read().calendar.clone();
		let model = crate::day::build_model(&snapshot, &settings);
		let context = crate::day::EditContext::new(&config.read(), &snapshot);
		rsx! {
			div { class: "app",
				header { class: "nav-card", div { class: "nav-row", "TimeParagon" } }
				div { class: "day-header", "Tuesday, November 17" }
				div { class: "day-body",
					{crate::timeline(&model, &[], &settings, TimeFormat::TwelveHour, open_block, editing, crate::SleepTimer { busy: false, press: Callback::new(|_| {}) }, 1.0, &calendar::search::Search::default())}
					crate::calendar_gestures::Gestures { context, zoom, on_saved: |_| {} }
				}
				PlacementControls { config }
			}
		}
	}

	#[test]
	fn alarm_placement_keeps_the_calendar_and_explicit_confirmation_available() {
		let mut dom = VirtualDom::new(preview);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("alarm-needle"));
		assert!(html.contains("role=\"slider\""));
		assert!(html.contains("aria-valuenow=\"630\""));
		assert!(html.contains("timeline-scroll"));
		assert!(html.contains("Set alarm"));
		assert!(html.contains("Cancel"));
		assert!(!html.contains("modal-backdrop"));
		if let Ok(path) = std::env::var("TIMEPARAGON_ALARM_PREVIEW") {
			std::fs::write(path, format!("<!doctype html><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><style>{}</style>{html}", crate::style::STYLESHEET.as_str())).unwrap();
		}
	}
}

fn load() -> Result<Library, String> {
	let path = timeparagon_core::task_db::path().ok_or("The alarm database is unavailable")?;
	alarms::read(&path).map_err(|e| e.to_string())
}

// Read just before changing a single alarm, preserving other devices' edits.
fn change(id: u64, mut replacement: Option<Alarm>, baseline: Option<Alarm>) -> Result<(), String> {
	let path = timeparagon_core::task_db::path().ok_or("The alarm database is unavailable")?;
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
	#[props(default)] on_place: Option<EventHandler<()>>,
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
						if let Some(on_place) = on_place { on_place.call(()); return; }
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
