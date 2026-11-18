//! Pinch zoom and long-press editing shared by Do and Plan's timed grids.
use dioxus::prelude::*;
use timeparagon_core::calendar::{self, Event};

use crate::{day, editor::worker};

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
	Zoom { zoom: f32 },
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
	use_future(move || async move {
		let mut eval = document::eval(include_str!("calendar_gestures.js"));
		while let Ok(gesture) = eval.recv::<Gesture>().await {
			let change = match gesture {
				Gesture::Zoom { zoom: value } => {
					if value.is_finite() {
						zoom.set(value.clamp(0.5, 3.0));
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
