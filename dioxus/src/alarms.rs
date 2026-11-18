//! The mobile editor uses the same alarm document and profiles as desktop.
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike};
use dioxus::prelude::*;
use timeparagon_core::{
	alarms::{self, Alarm, Library, SoundChoice, Urgency},
	calendar::time_format::{self, TimeFormat},
	config::Config,
	sounds::Tone,
};

/// How long an edit waits for the next one before it is written.
const SAVE_AFTER: std::time::Duration = std::time::Duration::from_millis(400);

/// Above this many minutes into the day, a needle's pill fits above its line.
const PILL_ROOM_MINUTES: i64 = 40;

/// An unsaved wall-clock selection, shared by the calendar and its action bar.
#[derive(Clone, Copy)]
pub struct Placement(pub Signal<Option<NaiveDateTime>>);

/// The saved alarms the calendar draws, and the one open in the editor.
#[derive(Clone, Copy)]
pub struct Shared {
	library: Signal<Library>,
	editing: Signal<Option<Alarm>>,
	/// Moves once a minute, which is when an alarm can come due.
	minute: Signal<DateTime<Local>>,
}

impl Shared {
	/// Reads the alarms again, leaving the view alone if nothing changed.
	pub fn reload(mut self) {
		match load() {
			Ok(found) if found.alarms != self.library.peek().alarms => self.library.set(found),
			Ok(_) => {}
			Err(message) => eprintln!("Could not read the alarms: {message}"),
		}
	}
}

/// Provides the saved alarms to the calendar grids and the editor.
pub fn use_shared() -> Shared {
	let library = use_signal(Library::default);
	let editing = use_signal(|| None::<Alarm>);
	let mut minute = use_signal(Local::now);
	use_future(move || async move {
		loop {
			tokio::time::sleep(std::time::Duration::from_secs(1)).await;
			let now = Local::now();
			if now.timestamp() / 60 != minute.peek().timestamp() / 60 {
				minute.set(now);
			}
		}
	});
	use_context_provider(|| Shared {
		library,
		editing,
		minute,
	})
}

/// Minutes from the top of `day`'s grid, for a wall-clock time that falls on it.
fn grid_minute(at: NaiveDateTime, day: NaiveDate, hour_start: u32) -> Option<i64> {
	let minute = (at - day.and_hms_opt(hour_start, 0, 0)?).num_minutes();
	(0..1440).contains(&minute).then_some(minute)
}

/// The alarms still to come on `day`, each with its minute on the grid.
fn needles_on(
	library: &Library,
	day: NaiveDate,
	hour_start: u32,
	now: DateTime<Local>,
) -> Vec<(i64, Alarm)> {
	library
		.alarms
		.iter()
		.filter(|alarm| alarm.at > now)
		.filter_map(|alarm| {
			let minute = grid_minute(alarm.at.naive_local(), day, hour_start)?;
			Some((minute, alarm.clone()))
		})
		.collect()
}

fn grid_percent(minute: i64) -> f32 {
	minute as f32 / 1440.0 * 100.0
}

/// The tappable length of each needle. Drawn before the events so they stay on
/// top: a needle never blocks the event it crosses.
#[component]
pub fn AlarmLines(day: NaiveDate, hour_start: u32) -> Element {
	let Some(shared) = try_use_context::<Shared>() else {
		return rsx! {};
	};
	let mut editing = shared.editing;
	let needles = needles_on(&(shared.library)(), day, hour_start, (shared.minute)());
	let lines = needles.into_iter().map(|(minute, alarm)| {
		let id = alarm.id;
		let top = grid_percent(minute);
		rsx! {
			button {
				key: "{id}",
				class: "alarm-tap alarm-line-target",
				style: "top:{top}%;",
				tabindex: "-1",
				aria_hidden: "true",
				onclick: move |_| editing.set(Some(alarm.clone())),
			}
		}
	});
	rsx! { {lines} }
}

/// Each saved alarm's purple needle, with its dot and its time and label.
#[component]
pub fn AlarmMarks(day: NaiveDate, hour_start: u32, time_format: TimeFormat) -> Element {
	let Some(shared) = try_use_context::<Shared>() else {
		return rsx! {};
	};
	let mut editing = shared.editing;
	let needles = needles_on(&(shared.library)(), day, hour_start, (shared.minute)());
	let marks = needles.into_iter().map(|(minute, alarm)| {
		let id = alarm.id;
		let top = grid_percent(minute);
		let label = alarm.needle_label(time_format);
		let class = if minute < PILL_ROOM_MINUTES {
			"alarm-tap alarm-mark below"
		} else {
			"alarm-tap alarm-mark"
		};
		rsx! {
			div {
				key: "{id}",
				class,
				style: "top:{top}%;",
				onclick: move |_| editing.set(Some(alarm.clone())),
				span { class: "alarm-mark-dot" }
				button {
					class: "alarm-mark-pill",
					aria_label: "Edit alarm, {label}",
					"{label}"
				}
			}
		}
	});
	rsx! { {marks} }
}

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
	let Some(minute) = grid_minute(at, day, hour_start) else {
		return rsx! {};
	};
	let top = grid_percent(minute);
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
	let shared = use_context::<Shared>();
	let mut editing = shared.editing;
	let mut error = use_signal(|| None::<String>);
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
		let _ = placement();
		error.set(None);
	});
	let format = config.read().appearance.time_format;
	rsx! {
		if let Some(at) = placement() {
			div { class: "placement-bar alarm-placement-bar", role: "region", aria_label: "Set alarm",
				div { class: "placement-summary", aria_live: "polite",
					strong { "{time_format::clock(at, format)} · {at.format(\"%a, %b %-d\")}" }
					span { class: "hint", "Drag the purple needle or tap a time. Scroll and pinch to zoom." }
				}
				if let Some(message) = error().or_else(|| future_time(at, now()).err()) {
					p { class: "error-text", role: "alert", "{message}" }
				}
				div { class: "placement-actions",
					button { class: "btn", onclick: move |_| placement.set(None), "Cancel" }
					button { class: "btn primary", disabled: future_time(at, now()).is_err(), onclick: move |_| {
						let result = (|| {
							let at = future_time(placement().ok_or("Choose a time")?, Local::now())?;
							let mut profile = config.peek().alarm.clone();
							profile.notification.summary = "Alarm".into();
							let alarm = Alarm { id: alarms::new_id(), at, label: String::new(), profile };
							change(alarm.id, Some(alarm.clone()), None)?;
							Ok::<_, String>(alarm)
						})();
						match result {
							Ok(alarm) => {
								placement.set(None);
								shared.reload();
								// As on desktop, a new alarm opens ready to be adjusted.
								editing.set(Some(alarm));
								crate::reminders::request_permission();
								crate::reminders::republish(&config.peek());
							}
							Err(message) => error.set(Some(message)),
						}
					}, "Set alarm" }
				}
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
		use_shared();
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

	fn saved(id: u64, at: DateTime<Local>, label: &str) -> Alarm {
		Alarm {
			id,
			at,
			label: label.into(),
			profile: alarms::AlarmProfile::default(),
		}
	}

	fn local(day: NaiveDate, hour: u32, minute: u32) -> DateTime<Local> {
		Local
			.from_local_datetime(&day.and_hms_opt(hour, minute, 0).unwrap())
			.earliest()
			.unwrap()
	}

	#[test]
	fn needles_sit_on_the_day_whose_window_holds_them_until_they_go_off() {
		let day = NaiveDate::from_ymd_opt(2099, 11, 17).unwrap();
		let next = day.succ_opt().unwrap();
		let library = Library {
			alarms: vec![
				saved(1, local(day, 2, 59), "Before the window"),
				saved(2, local(day, 10, 30), "Morning"),
				saved(3, local(next, 2, 59), "After midnight"),
				saved(4, local(next, 3, 0), "Next day"),
			],
		};
		let now = local(day, 0, 0);
		let shown: Vec<_> = needles_on(&library, day, 3, now)
			.into_iter()
			.map(|(minute, alarm)| (minute, alarm.id))
			.collect();
		assert_eq!(shown, [(450, 2), (1439, 3)]);
		let later = needles_on(&library, day, 3, local(day, 10, 30));
		assert_eq!(later.len(), 1, "an alarm leaves the grid as it goes off");
		assert_eq!(later[0].1.id, 3);
	}

	#[test]
	fn an_edit_keeps_what_another_device_changed_meanwhile() {
		let at = Local::now() + Duration::hours(1);
		let before = saved(1, at, "Tea");
		let mut desired = before.clone();
		desired.profile.sound.volume = 20;
		desired.profile.sound.repeats = 5;
		desired.profile.notification.urgency = Urgency::Low;
		desired.profile.flash.pulses = 4;
		desired.profile.flash.peak_opacity = 0.5;
		desired.profile.flash.passthrough = true;
		desired.profile.raise_window = true;
		let mut current = before.clone();
		current.label = "Renamed elsewhere".into();
		merge_edit(&mut current, &desired, &before);
		assert_eq!(current.label, "Renamed elsewhere");
		assert_eq!(current.profile, desired.profile);
	}

	fn grid_with_a_saved_alarm() -> Element {
		let day = NaiveDate::from_ymd_opt(2099, 11, 17).unwrap();
		let mut shared = use_shared();
		let placement = use_signal(|| None);
		use_context_provider(|| Placement(placement));
		let alarm = saved(7, local(day, 10, 30), "Tea");
		use_hook(|| {
			shared.library.set(Library {
				alarms: vec![alarm.clone()],
			});
			shared.editing.set(Some(alarm));
		});
		let config = use_signal(Config::default);
		rsx! {
			AlarmLines { day, hour_start: 0 }
			AlarmMarks { day, hour_start: 0, time_format: TimeFormat::TwentyFourHour }
			PlacementControls { config }
			AlarmEditor { config }
		}
	}

	#[test]
	fn a_saved_alarm_is_a_needle_that_opens_the_desktop_settings() {
		let mut dom = VirtualDom::new(grid_with_a_saved_alarm);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("alarm-mark-pill"), "{html}");
		assert!(html.contains("10:30 Tea"), "{html}");
		assert!(!html.contains("Alarm set for"), "{html}");
		for setting in [
			"Goes off in",
			"Time",
			"Label",
			"When it goes off",
			"Show a notification",
			"Urgency",
			"Bring the window forward",
			"Play a sound",
			"Volume 70%",
			"Play 2 times",
			"Flash the screen",
			"Strength 35%",
			"Pulses 2",
			"Delete",
			"Done",
		] {
			assert!(html.contains(setting), "missing {setting}: {html}");
		}
	}
}

fn load() -> Result<Library, String> {
	let path = timeparagon_core::task_db::path().ok_or("The alarm database is unavailable")?;
	alarms::read(&path).map_err(|e| e.to_string())
}

/// Applies only what this device changed, keeping other devices' edits.
fn merge_edit(current: &mut Alarm, desired: &Alarm, before: &Alarm) {
	macro_rules! keep_changed {
		($($($field:ident).+),+ $(,)?) => {$(
			if desired.$($field).+ != before.$($field).+ {
				current.$($field).+ = desired.$($field).+.clone();
			}
		)+};
	}
	keep_changed!(
		label,
		at,
		profile.raise_window,
		profile.notification.enabled,
		profile.notification.urgency,
		profile.sound.enabled,
		profile.sound.choice,
		profile.sound.volume,
		profile.sound.repeats,
		profile.flash.enabled,
		profile.flash.peak_opacity,
		profile.flash.pulses,
		profile.flash.passthrough,
	);
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
		merge_edit(&mut current, desired, &before);
		*desired = current;
	}
	library.alarms.retain(|alarm| alarm.id != id);
	if let Some(alarm) = replacement {
		library.alarms.push(alarm);
	}
	library.alarms.sort_by_key(|alarm| alarm.at);
	alarms::write(&path, &library).map_err(|e| e.to_string())
}

/// The editor for the alarm tapped on the calendar.
#[component]
pub fn AlarmEditor(config: Signal<Config>) -> Element {
	let editing = use_context::<Shared>().editing;
	let Some(alarm) = editing() else {
		return rsx! {};
	};
	rsx! { EditorSheet { key: "{alarm.id}", alarm, config } }
}

fn slider(
	label: String,
	range: (&'static str, &'static str, &'static str),
	value: String,
	mut on_change: impl FnMut(String) + 'static,
) -> Element {
	let (min, max, step) = range;
	rsx! {
		label { class: "alarm-slider",
			span { "{label}" }
			input {
				r#type: "range",
				min,
				max,
				step,
				value,
				oninput: move |event| on_change(event.value()),
			}
		}
	}
}

/// What an alarm does and when, with the desktop popup's settings. Edits apply
/// as they are made, so there is nothing to confirm.
#[component]
fn EditorSheet(alarm: Alarm, config: Signal<Config>) -> Element {
	let shared = use_context::<Shared>();
	let mut editing = shared.editing;
	let id = alarm.id;
	let mut time = use_signal(|| alarm.at.format("%H:%M").to_string());
	let mut time_error = use_signal(|| None::<String>);
	let mut error = use_signal(|| None::<String>);
	let mut saved = use_signal(|| alarm.clone());
	let mut draft = use_signal(|| alarm.clone());

	let save = use_callback(move |()| {
		let desired = draft.peek().clone();
		let before = saved.peek().clone();
		if desired == before {
			return true;
		}
		match change(id, Some(desired.clone()), Some(before)) {
			Ok(()) => {
				saved.set(desired);
				error.set(None);
				shared.reload();
				crate::reminders::republish(&config.peek());
				true
			}
			Err(message) => {
				error.set(Some(message));
				false
			}
		}
	});
	use_future(move || async move {
		loop {
			tokio::time::sleep(SAVE_AFTER).await;
			save.call(());
		}
	});
	// An alarm that goes off or is deleted elsewhere takes its editor along.
	use_effect(move || {
		let gone = !(shared.library)().alarms.iter().any(|alarm| alarm.id == id);
		if gone || draft.peek().at <= (shared.minute)() {
			editing.set(None);
		}
	});
	let mut close = move || {
		if save.call(()) {
			editing.set(None);
		}
	};

	let current = draft();
	let profile = &current.profile;
	let countdown = alarms::countdown_label(alarms::minutes_between((shared.minute)(), current.at));
	let date = current.at.date_naive();
	let urgency = profile.notification.urgency;
	let choice = profile.sound.choice.clone();
	let custom = matches!(choice, SoundChoice::Custom { .. }).then(|| choice.to_string());
	let peak = profile.flash.peak();
	let pulses = profile.flash.pulses.max(1);
	rsx! {
		div { class: "modal-backdrop", onclick: move |_| close(),
			div { class: "modal alarm-editor", role: "dialog", aria_label: "Alarm",
				onclick: move |event| event.stop_propagation(),
				div { class: "modal-head",
					h2 { class: "modal-title", "Alarm" }
					button { class: "modal-close", title: "Close", aria_label: "Close",
						onclick: move |_| {
							save.call(());
							editing.set(None);
						},
						"×"
					}
				}
				div { class: "modal-body",
					if let Some(message) = error() { p { class: "error-text", role: "alert", "{message}" } }
					div { class: "alarm-countdown",
						strong { "{countdown}" }
						span { class: "hint", "{current.at.format(\"%A, %B %-d\")}" }
					}
					label { class: "alarm-field",
						span { "Time" }
						input { class: "client-input", r#type: "time", value: "{time}",
							oninput: move |event| {
								let text = event.value();
								let moved = alarms::retimed(date, &text, Local::now());
								time.set(text);
								match moved {
									Ok(at) => {
										draft.write().at = at;
										time_error.set(None);
									}
									Err(message) => time_error.set(Some(message)),
								}
							},
						}
						if let Some(message) = time_error() { span { class: "error-text", role: "alert", "{message}" } }
					}
					label { class: "alarm-field",
						span { "Label" }
						input { class: "client-input", placeholder: "Alarm", value: "{current.label}",
							oninput: move |event| draft.write().label = event.value(),
						}
					}
					h3 { class: "alarm-editor-heading", "When it goes off" }
					p { class: "hint", "This phone uses the notification and sound choices. The other settings apply on desktop." }
					section { class: "alarm-section", aria_label: "Notification",
						{crate::habits::toggle("Show a notification", profile.notification.enabled, move |on| draft.write().profile.notification.enabled = on)}
						if profile.notification.enabled {
							label { class: "alarm-field",
								span { "Urgency" }
								select { class: "client-input",
									onchange: move |event| {
										if let Some(chosen) = Urgency::ALL.into_iter().find(|level| level.to_string() == event.value()) {
											draft.write().profile.notification.urgency = chosen;
										}
									},
									for level in Urgency::ALL {
										option { key: "{level}", value: "{level}", selected: level == urgency, "{level}" }
									}
								}
							}
						}
						{crate::habits::toggle("Bring the window forward", profile.raise_window, move |on| draft.write().profile.raise_window = on)}
					}
					section { class: "alarm-section", aria_label: "Sound",
						{crate::habits::toggle("Play a sound", profile.sound.enabled, move |on| draft.write().profile.sound.enabled = on)}
						if profile.sound.enabled {
							label { class: "alarm-field",
								span { "Sound" }
								select { class: "client-input",
									onchange: move |event| {
										if let Some(tone) = event.value().parse::<usize>().ok().and_then(|index| Tone::ALL.get(index)) {
											draft.write().profile.sound.choice = SoundChoice::Builtin(*tone);
										}
									},
									if let Some(file) = custom {
										option { value: "custom", selected: true, "{file} (this phone uses its default sound)" }
									}
									for (index, tone) in Tone::ALL.into_iter().enumerate() {
										option { key: "{index}", value: "{index}", selected: choice == SoundChoice::Builtin(tone), "{tone}" }
									}
								}
							}
							{slider(format!("Volume {}%", profile.sound.volume), ("0", "100", "1"), profile.sound.volume.to_string(), move |value| {
								if let Ok(volume) = value.parse() { draft.write().profile.sound.volume = volume; }
							})}
							{slider(alarms::repeat_label(profile.sound.repeats), ("0", "9", "1"), profile.sound.repeats.to_string(), move |value| {
								if let Ok(repeats) = value.parse() { draft.write().profile.sound.repeats = repeats; }
							})}
						}
					}
					section { class: "alarm-section", aria_label: "Flash",
						{crate::habits::toggle("Flash the screen", profile.flash.enabled, move |on| draft.write().profile.flash.enabled = on)}
						if profile.flash.enabled {
							{slider(format!("Strength {}%", (peak * 100.0).round()), ("0.05", "0.8", "0.05"), peak.to_string(), move |value| {
								if let Ok(strength) = value.parse() { draft.write().profile.flash.peak_opacity = strength; }
							})}
							{slider(format!("Pulses {pulses}"), ("1", "5", "1"), pulses.to_string(), move |value| {
								if let Ok(pulses) = value.parse() { draft.write().profile.flash.pulses = pulses; }
							})}
							{crate::habits::toggle("Let clicks pass through", profile.flash.passthrough, move |on| draft.write().profile.flash.passthrough = on)}
						}
					}
				}
				div { class: "alarm-editor-actions",
					button { class: "btn danger",
						onclick: move |_| match change(id, None, None) {
							Ok(()) => {
								shared.reload();
								crate::reminders::republish(&config.peek());
								editing.set(None);
							}
							Err(message) => error.set(Some(message)),
						},
						"Delete"
					}
					button { class: "btn primary", onclick: move |_| close(), "Done" }
				}
			}
		}
	}
}

#[component]
pub fn AlarmsSheet(
	config: Signal<Config>,
	day: chrono::NaiveDate,
	#[props(default)] on_place: Option<EventHandler<()>>,
	on_close: EventHandler<()>,
) -> Element {
	let shared = use_context::<Shared>();
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
								Ok(()) => { draft.set(None); error.set(None); if let Ok(found) = load() { library.set(found); } shared.reload(); crate::reminders::republish(&config.peek()); }
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
									Ok(()) => { if draft.peek().as_ref().is_some_and(|a| a.id == alarm.id) { draft.set(None); } if let Ok(found) = load() { library.set(found); } shared.reload(); crate::reminders::republish(&config.peek()); }
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
