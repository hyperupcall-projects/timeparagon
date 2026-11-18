//! Pinch zoom, long-press editing and new time ranges shared by Do and Plan's
//! timed grids.
use chrono::{Duration, NaiveDate, NaiveDateTime};
use dioxus::prelude::*;
use timeparagon_core::calendar::time_format::{self, TimeFormat};
use timeparagon_core::calendar::{self, Event};

use crate::{day, editor::worker};

const MIN_ZOOM: f32 = 0.5;
const MAX_ZOOM: f32 = 3.0;
const DEFAULT_ZOOM: f32 = 1.0;

/// A stretch of wall-clock time chosen on the grid for something new.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TimeRange {
	pub start: NaiveDateTime,
	pub end: NaiveDateTime,
}

impl TimeRange {
	/// Grid minutes count from the day's start hour, so a late range can fall
	/// on the next date. Bounds untrusted webview input.
	fn from_grid(day: NaiveDate, hour_start: u32, start: u32, end: u32) -> Option<Self> {
		if start >= end || end > 1440 {
			return None;
		}
		let origin = day.and_hms_opt(hour_start, 0, 0)?;
		Some(Self {
			start: origin.checked_add_signed(Duration::minutes(i64::from(start)))?,
			end: origin.checked_add_signed(Duration::minutes(i64::from(end)))?,
		})
	}

	pub fn minutes(self) -> u32 {
		(self.end - self.start).num_minutes() as u32
	}
}

/// The range being adjusted on the grid, until it is confirmed or cancelled.
#[derive(Clone, Copy)]
pub struct DraftRange(pub Signal<Option<TimeRange>>);

/// A nonmodal footer leaves the grid free to scroll and zoom around the range.
#[component]
pub fn RangeControls(time_format: TimeFormat, on_confirm: EventHandler<TimeRange>) -> Element {
	let DraftRange(mut draft) = use_context::<DraftRange>();
	let open = use_memo(move || draft().is_some());
	use_effect(move || {
		if open() {
			let _ = document::eval(
				"requestAnimationFrame(() => window.__calendarGesture?.revealDraft());",
			);
		}
	});
	let Some(range) = draft() else {
		return rsx! {};
	};
	let start = time_format::clock(range.start, time_format);
	let end = time_format::clock(range.end, time_format);
	rsx! {
		div { class: "placement-bar range-draft-bar", role: "region", aria_label: "Choose a time",
			div { class: "placement-summary", aria_live: "polite",
				strong { "{start} – {end} · {range.start.format(\"%a, %b %-d\")}" }
				span { class: "hint", "Drag to move it, or drag the handles to change when it starts and ends." }
			}
			div { class: "placement-actions",
				button { class: "btn", onclick: move |_| draft.set(None), "Cancel" }
				button { class: "btn primary", onclick: move |_| on_confirm.call(range), "Confirm time" }
			}
		}
	}
}

#[component]
pub fn ZoomControls(mut zoom: Signal<f32>) -> Element {
	let value = zoom();
	let stepped = |factor: f32| {
		let next = (value * factor).clamp(MIN_ZOOM, MAX_ZOOM);
		if (next - DEFAULT_ZOOM).abs() < 0.01 {
			DEFAULT_ZOOM
		} else {
			next
		}
	};
	let larger = stepped(calendar::ZOOM_STEP);
	let smaller = stepped(1.0 / calendar::ZOOM_STEP);
	rsx! {
		section { class: "calendar-zoom", aria_label: "Zoom",
			h3 { class: "source-heading", "Zoom" }
			button { class: "btn editor-wide", disabled: value >= MAX_ZOOM,
				onclick: move |_| zoom.set(larger), "Zoom in"
			}
			button { class: "btn editor-wide", disabled: value == DEFAULT_ZOOM,
				onclick: move |_| zoom.set(DEFAULT_ZOOM), "Reset zoom"
			}
			button { class: "btn editor-wide", disabled: value <= MIN_ZOOM,
				onclick: move |_| zoom.set(smaller), "Zoom out"
			}
		}
	}
}

/// Menu presses update the same scale as pinching, including when no grid is mounted.
pub fn sync_zoom(value: f32) {
	let _ = document::eval(&format!(
		"if (window.__calendarGesture) {{ window.__calendarGesture.setZoom({value}); }} else {{ document.documentElement.style.setProperty('--calendar-zoom', {value}); }}"
	));
}

pub fn can_drag(event: &Event) -> bool {
	!event.all_day && !event.details.sending && !calendar::deadline::is_marker(event)
}

#[derive(serde::Deserialize)]
struct Move {
	id: String,
	calendar: String,
	start_delta: i64,
	end_delta: i64,
	before_start: i64,
	before_end: i64,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Gesture {
	Zoom {
		zoom: f32,
	},
	Alarm {
		alarm_day: chrono::NaiveDate,
		hour_start: u32,
		minute: u32,
	},
	CancelAlarm {
		cancel_alarm: bool,
	},
	Range {
		range_day: NaiveDate,
		hour_start: u32,
		range_start: u32,
		range_end: u32,
	},
	CancelRange {
		cancel_range: bool,
	},
	Move(Move),
}

#[component]
pub fn Gestures(
	context: day::EditContext,
	mut zoom: Signal<f32>,
	on_saved: EventHandler<()>,
) -> Element {
	let seed = context.clone();
	let mut latest = use_signal(move || seed);
	use_effect(use_reactive!(|context| latest.set(context)));
	let mut error = use_signal(|| None::<String>);
	let placement = try_use_context::<crate::alarms::Placement>();
	let draft = try_use_context::<DraftRange>();
	// Confirming, cancelling or changing day takes the range off the grid.
	use_effect(move || {
		if let Some(DraftRange(draft)) = draft
			&& draft().is_none()
		{
			let _ = document::eval("window.__calendarGesture?.dismissDraft();");
		}
	});
	use_future(move || async move {
		let mut eval = document::eval(include_str!("calendar_gestures.js"));
		while let Ok(gesture) = eval.recv::<Gesture>().await {
			let change = match gesture {
				Gesture::Alarm {
					alarm_day,
					hour_start,
					minute,
				} => {
					if let Some(crate::alarms::Placement(mut placement)) = placement
						&& placement.peek().is_some()
						&& hour_start == latest.peek().settings.day_start_hour()
						&& let Some(at) =
							crate::alarms::placement_time(alarm_day, hour_start, minute)
					{
						placement.set(Some(at));
					}
					continue;
				}
				Gesture::CancelAlarm { cancel_alarm } => {
					if cancel_alarm && let Some(crate::alarms::Placement(mut placement)) = placement
					{
						placement.set(None);
					}
					continue;
				}
				Gesture::Range {
					range_day,
					hour_start,
					range_start,
					range_end,
				} => {
					if let Some(DraftRange(mut draft)) = draft {
						draft.set(TimeRange::from_grid(
							range_day,
							hour_start,
							range_start,
							range_end,
						));
					}
					continue;
				}
				Gesture::CancelRange { cancel_range } => {
					if cancel_range && let Some(DraftRange(mut draft)) = draft {
						draft.set(None);
					}
					continue;
				}
				Gesture::Zoom { zoom: value } => {
					if value.is_finite() {
						zoom.set(value.clamp(MIN_ZOOM, MAX_ZOOM));
					}
					continue;
				}
				Gesture::Move(change) => change,
			};
			let context = latest.peek().clone();
			let event = context
				.events
				.iter()
				.find(|event| {
					event.id == change.id && event.calendar_id == change.calendar && can_drag(event)
				})
				.cloned();
			let result = if let Some(event) = event {
				error.set(None);
				worker(move || save_move(event, change, &context)).await
			} else {
				Err("This event changed. Try again after refreshing the calendar.".into())
			};
			match result {
				Ok(()) => on_saved.call(()),
				Err(message) => error.set(Some(message)),
			}
			let _ = document::eval("window.__calendarGesture?.saved();");
		}
	});
	use_drop(|| {
		let _ = document::eval("window.__calendarGesture?.abort();");
	});
	rsx! {
		if let Some(message) = error() {
			div { class: "calendar-gesture-error", role: "alert",
				span { "{message}" }
				button { class: "btn small", onclick: move |_| error.set(None), "Dismiss" }
			}
		}
	}
}

fn save_move(event: Event, change: Move, context: &day::EditContext) -> Result<(), String> {
	if event.start.timestamp() != change.before_start || event.end.timestamp() != change.before_end
	{
		return Err(
			"This event changed while you were moving it. Try again after refreshing.".into(),
		);
	}
	// Bound untrusted webview input before constructing chrono durations.
	if change.start_delta.abs_diff(0) > 7 * 1440 || change.end_delta.abs_diff(0) > 7 * 1440 {
		return Err("That move is outside the displayed calendar.".into());
	}
	let start = event.start + chrono::Duration::minutes(change.start_delta);
	let end = event.end + chrono::Duration::minutes(change.end_delta);
	if end <= start
		|| (change.start_delta != change.end_delta && end - start < chrono::Duration::minutes(5))
	{
		return Err("An event must last at least 5 minutes.".into());
	}
	if event.details.sleep.is_some() {
		let mut night = event;
		night.start = start;
		night.end = end;
		night.details.sleep.as_mut().unwrap().automatic = false;
		calendar::sleep::save(&context.settings, night, false)?;
	} else if event.is_task() {
		day::save_task(
			event,
			timeparagon_core::task_edit::Change::Time(start, end),
			context,
		)?;
	} else {
		let mut draft = calendar::EditDraft::from_event(&event);
		draft.start = start;
		draft.end = end;
		// A gesture edits this occurrence; series editing stays in the sheet.
		day::save_event(&event, &draft, calendar::SaveScope::Current, context)?;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn at(day: NaiveDate, hour: u32, minute: u32) -> NaiveDateTime {
		day.and_hms_opt(hour, minute, 0).unwrap()
	}

	#[test]
	fn a_range_uses_wall_clock_minutes_from_the_calendar_day_start() {
		let day = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
		let next = day.succ_opt().unwrap();
		let range = TimeRange::from_grid(day, 0, 540, 600).unwrap();
		assert_eq!((range.start, range.end), (at(day, 9, 0), at(day, 10, 0)));
		assert_eq!(range.minutes(), 60);
		let late = TimeRange::from_grid(day, 3, 1230, 1320).unwrap();
		assert_eq!((late.start, late.end), (at(day, 23, 30), at(next, 1, 0)));
		let last = TimeRange::from_grid(day, 0, 1380, 1440).unwrap();
		assert_eq!(last.end, at(next, 0, 0));
	}

	#[test]
	fn a_range_from_the_webview_must_be_a_real_stretch_of_the_day() {
		let day = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
		assert!(TimeRange::from_grid(day, 0, 600, 600).is_none());
		assert!(TimeRange::from_grid(day, 0, 600, 540).is_none());
		assert!(TimeRange::from_grid(day, 0, 1400, 1441).is_none());
		assert!(TimeRange::from_grid(day, 24, 0, 60).is_none());
	}

	#[test]
	fn the_webview_messages_for_a_range_reach_their_own_gestures() {
		let range = serde_json::json!({
			"range_day": "2026-11-17", "hour_start": 3, "range_start": 540, "range_end": 600,
		});
		assert!(matches!(
			serde_json::from_value(range).unwrap(),
			Gesture::Range {
				hour_start: 3,
				range_start: 540,
				range_end: 600,
				..
			}
		));
		let cancel = serde_json::json!({ "cancel_range": true });
		assert!(matches!(
			serde_json::from_value(cancel).unwrap(),
			Gesture::CancelRange { cancel_range: true }
		));
	}

	#[test]
	fn the_bar_shows_the_range_and_asks_for_confirmation() {
		fn app() -> Element {
			let day = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
			let range = TimeRange::from_grid(day, 0, 540, 615);
			let draft = use_signal(move || range);
			use_context_provider(|| DraftRange(draft));
			rsx! { RangeControls { time_format: TimeFormat::TwelveHour, on_confirm: |_| {} } }
		}
		let mut dom = VirtualDom::new(app);
		dom.rebuild_in_place();
		let html = dioxus_ssr::render(&dom);
		assert!(html.contains("9:00AM – 10:15AM · Tue, Nov 17"), "{html}");
		assert!(html.contains("Confirm time"));
		assert!(html.contains("Cancel"));
		assert!(!html.contains("modal-backdrop"));
	}
}
