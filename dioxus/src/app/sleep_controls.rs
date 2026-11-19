//! Sleep timer actions and notification/strip rendering.
use super::calendar_state::DayData;
use super::*;
#[derive(Clone, Copy)]
pub(super) struct SleepControl {
	pub sleep_busy: Signal<bool>,
	pub sleep_error: Signal<Option<String>>,
	pub press_sleep: Callback<Event>,
}

pub(super) fn use_sleep_control() -> SleepControl {
	let Session {
		config,
		mut revision,
		..
	} = use_context::<Session>();
	let mut sleep_busy = use_signal(|| false);
	let mut sleep_error = use_signal(|| None::<String>);
	let press_sleep = use_callback(move |night: Event| {
		if sleep_busy() {
			return;
		}
		let settings = config.peek().calendar.clone();
		let format = config.peek().appearance.time_format;
		if calendar::sleep::timer(&night, timeparagon_core::clock::now())
			== Some(calendar::sleep::Timer::Start)
			&& reminders::notifications_blocked()
		{
			reminders::request_permission();
		}
		sleep_busy.set(true);
		sleep_error.set(None);
		spawn(async move {
			let result = editor::worker(move || sleep::press(&settings, night, format)).await;
			sleep_busy.set(false);
			match result {
				Ok(_) => {
					revision.set(revision() + 1);
				}

				Err(message) => sleep_error.set(Some(message)),
			}
		});
	});
	SleepControl {
		sleep_busy,
		sleep_error,
		press_sleep,
	}
}

#[component]
pub(super) fn SleepStatus() -> Element {
	let Session { config, .. } = use_context::<Session>();
	let SyncState { nearby_events, .. } = use_context::<SyncState>();
	let SleepControl {
		sleep_busy,
		mut sleep_error,
		..
	} = use_context::<SleepControl>();
	let SleepControl { press_sleep, .. } = use_context::<SleepControl>();
	let DayData {
		snapshot: snapshot_memo,
		..
	} = use_context::<DayData>();
	let snapshot = snapshot_memo();
	let today = day::today(&config.read().calendar);
	let time_format = config.read().appearance.time_format;
	// The night being timed, wherever the view is, kept in the notification.
	let running = sleep::running(&nearby_events.read());
	let settled = snapshot
		.as_ref()
		.is_some_and(|snapshot| (snapshot.day - today).num_days().abs() <= 1);
	sleep::sync_notification(running.as_ref(), settled, time_format);
	let sleep_strip = running.map(|event| {
		rsx! {
			sleep::SleepStrip {
				event,
				time_format,
				busy: sleep_busy(),
				error: sleep_error(),
				on_finish: move |night| press_sleep.call(night),
			}
		}
	});
	// A failed Start has no strip to show its error in.
	let sleep_banner = sleep_error()
		.filter(|_| sleep_strip.is_none())
		.map(|message| {
			rsx! {
				div { class: "banner error",
					span { class: "banner-text", "Sleep: {message}" }
					div { class: "spacer" }
					button { class: "btn small", onclick: move |_| sleep_error.set(None), "Dismiss" }
				}
			}
		});
	rsx! { {sleep_banner} {sleep_strip} }
}

#[component]
pub(super) fn SleepProvider(children: Element) -> Element {
	let sleep = use_sleep_control();
	use_context_provider(|| sleep);
	children
}
