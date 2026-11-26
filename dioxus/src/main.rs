//! TimeParagon's Dioxus app.
//!
//! Do is the day's calendar, Plan shows a day, multiple days or month,
//! Tasks holds the Unfinished, Inbox, Today and Backlog lists,
//! More holds the notepad and other pages, and
//! Settings the settings. Tapping an item opens a sheet that edits it; a time
//! block first lists its assigned tasks.

mod agenda;
mod ai_input;
mod alarms;
mod app;
mod calendar_data;
mod calendar_gestures;
mod create;
mod day;
mod editor;
mod habits;
mod habits_charts;
mod media_input;
mod more;
mod notes;
mod people;
mod plan;
mod reminders;
mod schedule;
mod scratch;
mod search;
mod setup;
mod sleep;
mod style;
mod travel;
mod warnings;
#[cfg(any(target_os = "android", test))]
mod widget;

use chrono::{Datelike, Months, NaiveDate};
use dioxus::prelude::*;
use timeparagon_core::calendar::first_day::FirstDayOfWeek;
use timeparagon_core::calendar::search::Search;
use timeparagon_core::calendar::time_format::{self, TimeFormat};
use timeparagon_core::calendar::{self, Event};
use timeparagon_core::config::CalendarSettings;
use timeparagon_core::theme as look;
use timeparagon_core::timeblocking as tb;

use day::{DayModel, EditContext, TimeBlock, TimedEvent};
use plan::{PlanView, TaskTab};

/// Default height of one hour on the timeline, in pixels.
const HOUR_HEIGHT: f32 = 60.0;
/// The shortest an event is drawn, so a finger can land on it.
const EVENT_MIN_HEIGHT: f32 = 22.0;
/// The line heights of an event's title and time in the stylesheet.
const TITLE_LINE: f32 = 16.0;
const TIME_LINE: f32 = 15.0;

const SCROLL_SCRIPT: &str = r#"
await new Promise(resolve => requestAnimationFrame(resolve));
const scroller = document.querySelector('.timeline-scroll');
const anchor = document.querySelector('.alarm-needle') || document.getElementById('now-line') || document.getElementById('timeline-anchor');
if (scroller && anchor && scroller.clientHeight) {
    scroller.scrollTop += anchor.getBoundingClientRect().top
        - scroller.getBoundingClientRect().top - scroller.clientHeight / 2;
    return true;
}
return false;
"#;

#[cfg(not(target_os = "android"))]
fn main() {
	timeparagon_core::database_sync::start();
	dioxus::LaunchBuilder::desktop()
		.with_cfg(
			dioxus::desktop::Config::new()
				.with_window(
					dioxus::desktop::WindowBuilder::new()
						.with_title("TimeParagon — Day")
						.with_inner_size(dioxus::desktop::LogicalSize::new(1180.0, 900.0))
						.with_min_inner_size(dioxus::desktop::LogicalSize::new(760.0, 560.0)),
				)
				.with_menu(None::<dioxus::desktop::muda::Menu>)
				.with_background_color(window_background()),
		)
		.launch(App);
}

/// What shows before the first paint: the default light scheme's page color.
#[cfg(not(target_os = "android"))]
fn window_background() -> (u8, u8, u8, u8) {
	let [red, green, blue, alpha] = look::Scheme::Light
		.extended()
		.background
		.base
		.color
		.into_rgba8();
	(red, green, blue, alpha)
}

#[cfg(target_os = "android")]
fn main() {
	prepare_android_storage();
	timeparagon_core::database_sync::start();
	// Dioxus disables the context menu by default in release builds by injecting
	// a document-wide `contextmenu` handler that calls `preventDefault()`. On
	// Android the long-press text-selection and paste UI *is* the context menu,
	// so that would leave the OAuth client ID and secret fields with no way to
	// paste. Keep the context menu so those fields work.
	dioxus::LaunchBuilder::new()
		.with_cfg(dioxus::mobile::Config::new().with_disable_context_menu(false))
		.launch(App);
}

/// Point the shared data layer at the app's private internal storage.
#[cfg(target_os = "android")]
fn prepare_android_storage() {
	let Some((files, cache)) = android_storage_dirs() else {
		return;
	};

	let config = files.join("config");
	let data = files.join("data");
	let state = files.join("state");
	for directory in [&config, &data, &state, &cache] {
		let _ = std::fs::create_dir_all(directory);
	}

	// SAFETY: this runs before the Dioxus runtime starts any threads.
	unsafe {
		std::env::set_var("HOME", &files);
		std::env::set_var("XDG_CONFIG_HOME", &config);
		std::env::set_var("XDG_DATA_HOME", &data);
		std::env::set_var("XDG_STATE_HOME", &state);
		std::env::set_var("XDG_CACHE_HOME", &cache);
		// The Rust sync client uses rustls-native-certs. Its Unix defaults
		// probe Termux, so point it at Android's public system trust store.
		if std::env::var_os("SSL_CERT_DIR").is_none() {
			let certificates: Vec<_> = [
				"/apex/com.android.conscrypt/cacerts",
				"/system/etc/security/cacerts",
			]
			.into_iter()
			.filter(|path| std::fs::read_dir(path).is_ok())
			.collect();
			if !certificates.is_empty() {
				if let Ok(paths) = std::env::join_paths(certificates) {
					std::env::set_var("SSL_CERT_DIR", paths);
				}
			}
		}
	}
}

#[cfg(target_os = "android")]
fn android_storage_dirs() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
	use jni::objects::JObject;

	let context = std::panic::catch_unwind(|| ndk_context::android_context()).ok()?;
	let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) }.ok()?;
	let mut env = vm.attach_current_thread().ok()?;
	let activity = unsafe { JObject::from_raw(context.context().cast()) };
	let files = env
		.call_method(&activity, "getFilesDir", "()Ljava/io/File;", &[])
		.ok()?
		.l()
		.ok()?;
	let cache = env
		.call_method(&activity, "getCacheDir", "()Ljava/io/File;", &[])
		.ok()?
		.l()
		.ok()?;
	let files = java_file_path(&mut env, &files)?;
	let cache = java_file_path(&mut env, &cache)?;
	Some((files, cache))
}

#[cfg(target_os = "android")]
fn java_file_path(
	env: &mut jni::JNIEnv<'_>,
	file: &jni::objects::JObject<'_>,
) -> Option<std::path::PathBuf> {
	use jni::objects::JString;

	let path = env
		.call_method(file, "getAbsolutePath", "()Ljava/lang/String;", &[])
		.ok()?
		.l()
		.ok()?;
	let path = JString::from(path);
	let path: String = env.get_string(&path).ok()?.into();
	Some(std::path::PathBuf::from(path))
}

/// Setup until it is finished, then the app itself.
#[component]
fn App() -> Element {
	let mut finished = use_signal(|| setup::Progress::load().finished);
	use_context_provider(|| setup::Finished(finished));
	if finished() {
		rsx! { app::Main {} }
	} else {
		rsx! { setup::SetupFlow { on_finish: move |_| finished.set(true) } }
	}
}

/// One entry of the navigation card, lit while its page is on show.
fn nav_tab(
	label: &'static str,
	active: bool,
	extra_class: &'static str,
	mut on_press: impl FnMut(()) + 'static,
) -> Element {
	rsx! {
		button {
			class: "nav-tab {extra_class}",
			aria_pressed: active.to_string(),
			onclick: move |_| on_press(()),
			"{label}"
		}
	}
}

/// Navbar icons use the same 24-unit line drawings as the desktop app.
fn nav_icon(settings: bool) -> Element {
	let gear_points = settings.then(|| {
		(0..32)
			.map(|i| {
				let angle = (i as f32 - 0.5) * std::f32::consts::TAU / 32.0;
				let radius = if i % 4 < 2 { 10.0 } else { 8.0 };
				format!(
					"{},{}",
					12.0 + radius * angle.cos(),
					12.0 + radius * angle.sin()
				)
			})
			.collect::<Vec<_>>()
			.join(" ")
	});
	rsx! {
		svg {
			width: "20", height: "20", view_box: "0 0 24 24",
			fill: "none", stroke: "currentColor", stroke_width: "1.8",
			stroke_linecap: "round", stroke_linejoin: "round",
			"aria-hidden": "true", "focusable": "false",
			if let Some(points) = gear_points {
				polygon { points: "{points}" }
				circle { cx: "12", cy: "12", r: "3" }
			} else {
				path { d: "M4 6h16 M4 12h16 M4 18h16" }
			}
		}
	}
}

/// Resolve display-only deadline chips and note links to their stored entry.
fn linked_event(snapshot: calendar::Snapshot, id: &str) -> Option<Event> {
	let id = calendar::deadline::source_id(id);
	snapshot.events.into_iter().find(|event| {
		event.id == id
			|| event
				.details
				.task_data
				.as_ref()
				.is_some_and(|data| data.id.to_string() == id)
	})
}

/// Moves the view to another day.
#[derive(Clone, Copy)]
struct DayNav {
	current_day: Signal<NaiveDate>,
	selected_block: Signal<Option<String>>,
	scrolled: Signal<bool>,
	follow_today: Signal<bool>,
	/// Going somewhere else drops whatever was waiting to open.
	pending_open: Signal<Option<String>>,
}

impl DayNav {
	fn go_to(mut self, day: NaiveDate, follow_today: bool) {
		self.current_day.set(day);
		self.selected_block.set(None);
		self.scrolled.set(false);
		self.follow_today.set(follow_today);
		self.pending_open.set(None);
	}
}

/// Where the navigation is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
	/// The day's calendar.
	Do,
	/// The period being planned.
	Plan,
	/// Unfinished, Inbox, Today and Backlog.
	Tasks,
	Habits,
	Settings,
}

/// The tabs of the View sheet.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ViewTab {
	Calendars,
	Filter,
}

/// Chooses what Plan plans, as the desktop's Plan button and menu do.
#[derive(Clone, Copy)]
struct PlanNav {
	page: Signal<Page>,
	view: Signal<PlanView>,
	menu_open: Signal<bool>,
	plan_anchor: Signal<NaiveDate>,
	current_day: Signal<NaiveDate>,
}

impl PlanNav {
	/// Open the selected period, starting a range from the day in view.
	fn choose(mut self, view: PlanView) {
		let was_planning = *self.page.peek() == Page::Plan;
		let changed = *self.view.peek() != view;
		self.menu_open.set(false);
		self.view.set(view);
		if changed || !was_planning {
			self.plan_anchor.set(*self.current_day.peek());
		}
		self.page.set(Page::Plan);
	}
}

/// One of the task lists, over everything loaded with its local edits.
fn task_list(
	tab: TaskTab,
	snapshot: &calendar::Snapshot,
	config: Signal<timeparagon_core::config::Config>,
	signed_in: bool,
	revision: Signal<u32>,
	editing: Signal<Option<Event>>,
	show_create: Signal<bool>,
) -> Element {
	let mut revision = revision;
	let mut show_create = show_create;
	let context = EditContext::new(&config.read(), snapshot);
	rsx! {
		plan::TaskList {
			key: "{tab:?}",
			tab,
			config,
			events: snapshot.events.clone(),
			calendars: snapshot.calendars.clone(),
			context: Some(context),
			signed_in,
			revision,
			editing,
			on_new_task: move |()| show_create.set(true),
			on_saved: move |()| revision.set(revision() + 1),
		}
	}
}

/// What picking a day in a month picker moves.
#[derive(Clone, Copy)]
enum Pick {
	/// The day in view.
	Day(DayNav),
}

impl Pick {
	fn pick(self, day: NaiveDate, today: NaiveDate) {
		match self {
			Self::Day(nav) => nav.go_to(day, day == today),
		}
	}
}

/// The picker button, labelled with what is in view, and the month it opens:
/// the desktop's way between days and ranges. The month is named on the left,
/// with a month back, today (●) and a month forward on the right; `preview`
/// marks the days of the range in view.
#[allow(clippy::too_many_arguments)]
fn month_picker(
	label: String,
	selected: NaiveDate,
	preview: Vec<NaiveDate>,
	shows_today: bool,
	today: NaiveDate,
	first_day: FirstDayOfWeek,
	mut open: Signal<bool>,
	mut shown: Signal<NaiveDate>,
	pick: Pick,
) -> Element {
	let month = shown();
	let days = if open() {
		calendar::month_grid(month, first_day)
	} else {
		Vec::new()
	};
	let cells = days.into_iter().map(move |day| {
		let mut class = String::from("month-day");
		if day.month() != month.month() {
			class.push_str(" outside");
		}
		if preview.contains(&day) {
			class.push_str(" in-range");
		}
		if day == selected && preview.is_empty() {
			class.push_str(" selected");
		}
		if day == today {
			class.push_str(" today");
		}
		let number = day.day();
		rsx! {
			button {
				key: "{day}",
				class: "{class}",
				onclick: move |_| {
					open.set(false);
					pick.pick(day, today);
				},
				"{number}"
			}
		}
	});
	let picker = open().then(|| {
		let heading = calendar::month_heading(month);
		rsx! {
			div { class: "month-dismiss", onclick: move |_| open.set(false) }
			div { class: "month-picker", role: "dialog", aria_label: "Choose a day",
				div { class: "month-head",
					span { class: "month-name", "{heading}" }
					button {
						class: "btn icon",
						title: "Previous month",
						aria_label: "Previous month",
						onclick: move |_| {
							if let Some(previous) = month.checked_sub_months(Months::new(1)) {
								shown.set(previous);
							}
						},
						"‹"
					}
					button {
						class: "btn icon",
						title: "Today",
						aria_label: "Today",
						onclick: move |_| {
							open.set(false);
							pick.pick(today, today);
						},
						"●"
					}
					button {
						class: "btn icon",
						title: "Next month",
						aria_label: "Next month",
						onclick: move |_| {
							if let Some(next) = month.checked_add_months(Months::new(1)) {
								shown.set(next);
							}
						},
						"›"
					}
				}
				div { class: "month-grid",
					for weekday in first_day.short_labels() {
						span { key: "{weekday}", class: "month-weekday", "{weekday}" }
					}
					{cells}
				}
			}
		}
	});

	rsx! {
		div { class: "month-anchor",
			button {
				class: if shows_today { "btn today" } else if open() { "btn primary" } else { "btn" },
				aria_expanded: open().to_string(),
				onclick: move |_| {
					if !open() {
						shown.set(calendar::first_of_month(selected));
					}
					open.set(!open());
				},
				"{label} ▾"
			}
			{picker}
		}
	}
}

/// An all-day event: a full-width bar in the event's own color.
fn all_day_chip(
	event: &Event,
	settings: &CalendarSettings,
	calendars: &[calendar::Calendar],
	editing: Signal<Option<Event>>,
	search: &Search,
) -> Element {
	let mut editing = editing;
	let opened = event.clone();
	let color = day::display_color(event, settings, calendars, search);
	let style = format!(
		"background:{};color:{};",
		css_color(color),
		css_color(calendar::colors::on_color(color))
	);
	let title = if event.completed && !search.hides_details(event) {
		format!("✓ {}", event.title)
	} else {
		day::display_title(event, search)
	};
	rsx! {
		button {
			class: "all-day-chip",
			"data-sending": event.details.sending,
			disabled: event.details.sending,
			style: "{style}",
			title: "{title}",
			onclick: move |_| editing.set(Some(opened.clone())),
			"{title}"
		}
	}
}

#[allow(clippy::too_many_arguments)]
fn timeline(
	model: &DayModel,
	travel: &[day::TravelBox],
	settings: &CalendarSettings,
	time_format: TimeFormat,
	open_block: Signal<Option<String>>,
	editing: Signal<Option<Event>>,
	sleep_timer: SleepTimer,
	zoom: f32,
	search: &Search,
) -> Element {
	let grid_height = scaled_px(24.0 * HOUR_HEIGHT);
	let hours: Vec<Element> = (0..24u32)
		.map(|row| {
			let top = scaled_px(row as f32 * HOUR_HEIGHT);
			let label = time_format::hour(day::hour_at(model.hour_start, row), time_format);
			rsx! {
				span {
					key: "label-{row}",
					class: "hour-label",
					style: "top: calc({top} + 2px);",
					"{label}"
				}
			}
		})
		.collect();
	let lines: Vec<Element> = (0..24u32)
		.map(|row| {
			let top = scaled_px(row as f32 * HOUR_HEIGHT);
			rsx! {
				div { key: "line-{row}", class: "hour-line", style: "top: {top};" }
			}
		})
		.collect();
	let events: Vec<Element> = model
		.timed
		.iter()
		.map(|timed| {
			timed_event_view(
				timed,
				time_format,
				open_block,
				editing,
				sleep_timer,
				zoom,
				search,
			)
		})
		.collect();
	let trips: Vec<Element> = travel
		.iter()
		.map(|trip| {
			let style = travel_style(trip);
			rsx! {
				div { key: "travel-{trip.event_id}", class: "grid-travel", style: "{style}", "{trip.label}" }
			}
		})
		.collect();

	let now_line = model.now_min.map(|minutes| {
		let top = scaled_px(minutes as f32 * HOUR_HEIGHT / 60.0);
		rsx! {
			div { id: "now-line", class: "now-line", style: "top: {top};",
				span { class: "now-dot" }
			}
		}
	});
	let empty = model.timed.is_empty().then(|| {
		rsx! {
			div { class: "timeline-empty",
				span { "Nothing scheduled on the grid for this day." }
			}
		}
	});
	let anchor_top = scaled_px(8.0 * HOUR_HEIGHT);

	let summary = model.summary;
	let summary_class = if summary.needs_attention() {
		"day-summary attention"
	} else {
		"day-summary"
	};

	rsx! {
		section { id: "calendar-pane", class: "timeline-pane", aria_label: "Schedule",
			div { class: "day-overview",
				p { class: "{summary_class}",
					"{summary.unfinished} unfinished. \u{a0}{summary.inbox} in inbox. \u{a0}{summary.unslotted} not slotted in"
				}
				for event in &model.all_day {
					{all_day_chip(event, settings, &model.calendars, editing, search)}
				}
			}
			div { class: "timeline-card",
				div { class: "timeline-scroll",
					div { class: "timeline", style: "height: {grid_height};",
						div { class: "gutter", style: "height: {grid_height};", {hours.into_iter()} }
						div { class: "grid", style: "height: {grid_height};",
							"data-calendar-day": model.day.to_string(),
							"data-hour-start": model.hour_start,
							"data-clock": if time_format == TimeFormat::TwelveHour { "12" } else { "24" },
							{lines.into_iter()}
							alarms::AlarmLines { day: model.day, hour_start: model.hour_start }
							{events.into_iter()}
							{trips.into_iter()}
							{now_line}
							alarms::AlarmMarks { day: model.day, hour_start: model.hour_start, time_format }
							alarms::AlarmNeedle { day: model.day, hour_start: model.hour_start, time_format }
							{empty}
							div {
								id: "timeline-anchor",
								class: "anchor",
								style: "top: {anchor_top};"
							}
						}
					}
				}
			}
		}
	}
}

/// How an event's text fits in its height.
#[derive(Debug, PartialEq, Eq)]
struct TextFit {
	/// The gap above the title, which shrinks on a short event.
	top: u32,
	/// The most lines the title can take.
	title_lines: u32,
	/// The lines of its time that fit under the title.
	time_lines: usize,
}

/// A title always gets a line, then the time, then the rest goes to the title.
fn text_fit(height: f32, time_lines: usize) -> TextFit {
	let top = (height - TITLE_LINE).clamp(0.0, look::BLOCK_TOP);
	let room = height - top;
	let time_lines = (((room - TITLE_LINE) / TIME_LINE).floor().max(0.0) as usize).min(time_lines);
	let title_lines = ((room - time_lines as f32 * TIME_LINE) / TITLE_LINE)
		.floor()
		.max(1.0) as u32;
	TextFit {
		top: top as u32,
		title_lines,
		time_lines,
	}
}

/// The time written on an event: a sleep block says when to go to bed and rise.
fn event_when(event: &Event, time_format: TimeFormat) -> String {
	if event.details.sleep.is_some() {
		calendar::sleep::time_label(event, time_format)
	} else {
		time_format::event(event, time_format)
	}
}

fn event_label(
	timed: &TimedEvent,
	height: f32,
	time_format: TimeFormat,
	search: &Search,
) -> Element {
	let when = event_when(&timed.event, time_format);
	let fit = text_fit(height, when.lines().count());
	let title = if search.hides_details(&timed.event) {
		calendar::search::PRIVATE_TITLE.to_owned()
	} else {
		timed.title.clone()
	};
	let times = when
		.lines()
		.take(fit.time_lines)
		.map(str::to_owned)
		.collect::<Vec<_>>();
	rsx! {
		div {
			class: "event-title",
			style: "margin-top:{fit.top}px;-webkit-line-clamp:{fit.title_lines};",
			"{title}"
		}
		for time in times {
			div { class: "event-time", "{time}" }
		}
	}
}

/// The Start sleep or Finish sleep button nights offer on the grid.
#[derive(Clone, Copy)]
struct SleepTimer {
	/// A press is being saved, so the buttons wait.
	busy: bool,
	press: Callback<Event>,
}

fn timed_event_view(
	timed: &TimedEvent,
	time_format: TimeFormat,
	open_block: Signal<Option<String>>,
	editing: Signal<Option<Event>>,
	sleep_timer: SleepTimer,
	zoom: f32,
	search: &Search,
) -> Element {
	let mut editing = editing;
	let opened = timed.event.clone();
	let id = timed.event.id.clone();
	let style = event_style(timed, search);
	let height = event_height(timed, zoom);
	let tooltip = format!(
		"{}\n{}\n{}",
		if search.hides_details(&timed.event) {
			calendar::search::PRIVATE_TITLE
		} else {
			&timed.title
		},
		event_when(&timed.event, time_format),
		if search.hides_details(&timed.event) {
			""
		} else {
			&timed.calendar_name
		}
	);
	let label = event_label(timed, height, time_format, search);

	if timed.time_block {
		let mut open = open_block;
		let planned: u32 = timed.tasks.iter().map(|task| task.minutes).sum();
		let duration = (timed.end_min - timed.start_min).max(1) as f32;
		let occupied_end =
			(timed.scheduled_start_min as f32 + planned as f32).min(timed.end_min as f32);
		let filled = ((occupied_end - timed.start_min as f32).max(0.0) / duration).min(1.0) * 100.0;
		rsx! {
			button {
				key: "{id}",
				class: "grid-event block",
				"data-sending": timed.event.details.sending,
				"data-calendar-event": timed.event.id.clone(),
				"data-event-calendar": timed.event.calendar_id.clone(),
				"data-draggable": calendar_gestures::can_drag(&timed.event).to_string(),
				"data-start-min": timed.scheduled_start_min,
				"data-end-min": timed.scheduled_end_min,
				"data-duration": (timed.event.end - timed.event.start).num_minutes(),
				"data-before-start": timed.event.start.timestamp(),
				"data-before-end": timed.event.end.timestamp(),
				disabled: timed.event.details.sending,
				style: "{style}",
				title: "{tooltip}",
				onclick: move |_| open.set(Some(id.clone())),
				div { class: "block-filled", style: "height:{filled:.2}%;" }
				div { class: "block-label", {label} }
				span { class: "block-plan", "Plan" }
			}
		}
	} else if !search.hides_details(&timed.event)
		&& let Some(timer) = calendar::sleep::timer(&timed.event, timeparagon_core::clock::now())
	{
		// A night splits like a time block: its label opens it, and the
		// button beside it starts or finishes the timer.
		let night = timed.event.clone();
		let disabled = sleep_timer.busy || !sleep::can_press(&night);
		rsx! {
			div {
				key: "{id}",
				class: "grid-event sleep",
				"data-sending": timed.event.details.sending,
				"data-calendar-event": timed.event.id.clone(),
				"data-event-calendar": timed.event.calendar_id.clone(),
				"data-draggable": calendar_gestures::can_drag(&timed.event).to_string(),
				"data-start-min": timed.scheduled_start_min,
				"data-end-min": timed.scheduled_end_min,
				"data-duration": (timed.event.end - timed.event.start).num_minutes(),
				"data-before-start": timed.event.start.timestamp(),
				"data-before-end": timed.event.end.timestamp(),
				style: "{style}",
				title: "{tooltip}",
				button {
					class: "sleep-open",
					onclick: move |_| editing.set(Some(opened.clone())),
					{label}
				}
				button {
					class: "sleep-timer",
					disabled,
					onclick: move |_| sleep_timer.press.call(night.clone()),
					"{timer.label()}"
				}
			}
		}
	} else {
		rsx! {
			button {
				key: "{id}",
				class: "grid-event",
				"data-sending": timed.event.details.sending,
				"data-calendar-event": timed.event.id.clone(),
				"data-event-calendar": timed.event.calendar_id.clone(),
				"data-draggable": calendar_gestures::can_drag(&timed.event).to_string(),
				"data-start-min": timed.scheduled_start_min,
				"data-end-min": timed.scheduled_end_min,
				"data-duration": (timed.event.end - timed.event.start).num_minutes(),
				"data-before-start": timed.event.start.timestamp(),
				"data-before-end": timed.event.end.timestamp(),
				disabled: timed.event.details.sending,
				style: "{style}",
				title: "{tooltip}",
				onclick: move |_| editing.set(Some(opened.clone())),
				{label}
			}
		}
	}
}

fn time_block_modal(
	block: &TimeBlock,
	context: &EditContext,
	close: Signal<Option<String>>,
	editing: Signal<Option<Event>>,
	writing: Signal<bool>,
	on_saved: impl FnMut(()) + 'static,
) -> Element {
	let mut editing = editing;
	let edited_block = block.event.clone();
	let planner = block.clone();
	let planner_context = context.clone();
	let on_saved = EventHandler::new(on_saved);
	let mut backdrop_close = close;
	let mut button_close = close;
	let title = block.event.title.clone();
	let time = calendar::event_time_label(&block.event);
	let accent = css_color(
		block
			.event
			.display_color(&context.settings, &context.calendars),
	);
	let planned = block.planned_minutes as i64;
	let capacity = block.capacity_minutes.max(1);
	let pct = (planned as f32 / capacity as f32 * 100.0).clamp(0.0, 100.0);
	let over_by = (planned - capacity).max(0);
	let fill_class = if over_by > 0 {
		"capacity-fill over"
	} else {
		"capacity-fill"
	};
	let planned_label = if block.planned_minutes == 0 {
		"0m".to_owned()
	} else {
		day::duration_label(block.planned_minutes)
	};
	let summary = if over_by > 0 {
		format!(
			"{planned_label} planned of {} · over by {}",
			day::capacity_label(block.capacity_minutes),
			day::duration_label(over_by as u32)
		)
	} else {
		format!(
			"{planned_label} planned of {}",
			day::capacity_label(block.capacity_minutes)
		)
	};
	let note_target = notes::target_of(&block.event, &context.settings);
	let task_word = if block.tasks.len() == 1 {
		"task"
	} else {
		"tasks"
	};

	rsx! {
		// Close on the press that starts outside the sheet. Using pointerdown
		// (rather than click) means the tap that opened the sheet cannot also
		// close it, which matters on Android where a synthesized click can
		// arrive after the modal has mounted.
		div {
			class: "modal-backdrop",
			onpointerdown: move |_| {
				if !writing() {
					backdrop_close.set(None);
				}
			},
			div {
				class: "modal",
				onpointerdown: move |event| event.stop_propagation(),
				onclick: move |event| event.stop_propagation(),
				div { class: "modal-head",
					div { class: "modal-mark", style: "background: {accent};" }
					div { class: "modal-titles",
						h2 { class: "modal-title", "{title}" }
						div { class: "modal-sub", "{time}" }
					}
					button {
						class: "modal-close",
						title: "Close",
						disabled: writing(),
						onclick: move |_| button_close.set(None),
						"×"
					}
				}
				div { class: "modal-body",
					button {
						class: "btn editor-wide",
						onclick: move |_| editing.set(Some(edited_block.clone())),
						"Edit block"
					}
					div { class: "block-summary",
						div { class: "summary-line",
							span { "{summary}" }
							span { class: "summary-tasks", "{block.tasks.len()} {task_word}" }
						}
						div { class: "capacity",
							div { class: "{fill_class}", style: "width: {pct}%;" }
						}
					}
					if let Some(target) = note_target {
						notes::LinkedNotes { target }
					}
					div { class: "assigned-head",
						span { class: "section-title", "Assigned tasks" }
						span { class: "count-pill", "{block.tasks.len()}" }
					}
					editor::BlockTasks {
						block: planner,
						context: planner_context,
						busy: writing,
						on_open: move |task| editing.set(Some(task)),
						on_saved,
					}
				}
			}
		}
	}
}

pub(crate) fn status_icon(status: tb::TaskStatus) -> &'static str {
	match status {
		tb::TaskStatus::Active => "○",
		tb::TaskStatus::Completed => "✓",
		tb::TaskStatus::Ignored => "⊘",
	}
}

/// Scale calendar geometry without scaling fonts or finger targets. The webview
/// updates this variable directly during a pinch, avoiding native round trips.
fn scaled_px(pixels: f32) -> String {
	format!("calc({pixels:.4}px * var(--calendar-zoom, 1))")
}

fn event_height(timed: &TimedEvent, zoom: f32) -> f32 {
	((timed.end_min - timed.start_min) as f32 * HOUR_HEIGHT * zoom / 60.0).max(EVENT_MIN_HEIGHT)
}

/// Where a column starts and how wide its span is, as percentages of the grid.
fn column_position(column: usize, columns: usize, span: usize) -> (f32, f32) {
	let width = 100.0 / columns.max(1) as f32;
	(column as f32 * width, span.max(1) as f32 * width)
}

fn column_width(span: f32) -> String {
	format!("calc({span:.4}% - {}px)", look::EVENT_GAP)
}

fn event_style(timed: &TimedEvent, search: &Search) -> String {
	let top = scaled_px(timed.start_min as f32 * HOUR_HEIGHT / 60.0);
	let duration = scaled_px((timed.end_min - timed.start_min) as f32 * HOUR_HEIGHT / 60.0);
	let height = format!("max({EVENT_MIN_HEIGHT}px, {duration})");
	let (left, span) = column_position(timed.column, timed.columns, timed.column_span);
	let width = column_width(span);
	let inset = look::EVENT_GAP / 2.0;
	let color = if search.hides_details(&timed.event) {
		calendar::search::PRIVATE_COLOR
	} else {
		timed.color
	};
	let background = css_color(color);
	let ink = css_color(calendar::colors::on_color(color));

	format!(
		"top:{top};height:{height};left:calc({left:.4}% + {inset}px);width:{width};background:{background};color:{ink};"
	)
}

/// Where a trip's box sits: ending at the start of the event it leads to, in
/// that event's column.
fn travel_style(trip: &day::TravelBox) -> String {
	let duration = scaled_px(trip.minutes as f32 * HOUR_HEIGHT / 60.0);
	let height = format!("max({}px, {duration})", look::TRAVEL_MIN_HEIGHT);
	let anchor = scaled_px(trip.anchor_min as f32 * HOUR_HEIGHT / 60.0);
	let top = format!("max(0px, calc({anchor} - {height}))");
	let (left, span) = column_position(trip.column, trip.columns, trip.column_span);
	let width = column_width(span);
	let inset = look::EVENT_GAP / 2.0;
	let background = css_color(calendar::colors::TRAVEL);
	let ink = css_color(calendar::colors::on_color(calendar::colors::TRAVEL));

	format!(
		"top:{top};height:{height};left:calc({left:.4}% + {inset}px);width:{width};background:{background};color:{ink};"
	)
}

pub(crate) fn css_color(color: timeparagon_core::Color) -> String {
	let red = (color.r.clamp(0.0, 1.0) * 255.0).round() as u8;
	let green = (color.g.clamp(0.0, 1.0) * 255.0).round() as u8;
	let blue = (color.b.clamp(0.0, 1.0) * 255.0).round() as u8;
	format!(
		"rgba({red}, {green}, {blue}, {:.2})",
		color.a.clamp(0.0, 1.0)
	)
}

/// Sync and Google log entries in one list, newest first, as the Log sheet
/// shows them: time, text, whether it failed, and a key for the list.
fn merged_log(
	sync: Vec<timeparagon_core::database_sync::LogEntry>,
	google: Vec<calendar::operations::LogEntry>,
) -> Vec<(String, String, bool, String)> {
	let mut entries: Vec<(chrono::DateTime<chrono::Local>, String, bool, String)> = sync
		.into_iter()
		.map(|entry| {
			(
				entry.at,
				entry.text,
				false,
				format!("sync-{}", entry.sequence),
			)
		})
		.chain(google.into_iter().map(|entry| {
			(
				entry.at,
				entry.text,
				entry.failed,
				format!("google-{}", entry.sequence),
			)
		}))
		.collect();
	entries.sort_by(|a, b| b.0.cmp(&a.0));
	entries
		.into_iter()
		.map(|(at, text, failed, key)| (at.format("%H:%M:%S").to_string(), text, failed, key))
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::TimeZone;

	#[test]
	fn the_log_sheet_merges_sync_and_google_entries_newest_first() {
		let at = chrono::Local
			.with_ymd_and_hms(2026, 11, 18, 9, 0, 0)
			.unwrap();
		let sync = timeparagon_core::database_sync::LogEntry {
			sequence: 7,
			at,
			text: "Database synced".into(),
		};
		let google = calendar::operations::LogEntry {
			sequence: 3,
			at: at + chrono::Duration::seconds(10),
			text: "Saved task".into(),
			failed: false,
		};
		let entries = merged_log(vec![sync], vec![google]);
		assert_eq!(entries.len(), 2);
		assert_eq!((&*entries[0].1, &*entries[0].3), ("Saved task", "google-3"));
		assert_eq!(
			(&*entries[1].1, &*entries[1].3),
			("Database synced", "sync-7")
		);
	}

	#[test]
	fn event_links_and_deadline_chips_open_the_original_task() {
		let mut snapshot = calendar::sample_snapshot();
		let mut task = snapshot.events.remove(0);
		task.id = "task:linked".into();
		task.details.task_data = Some(timeparagon_core::task_db::TaskData {
			id: 42,
			metadata: tb::Metadata {
				deadline: Some(calendar::deadline::Deadline {
					date: snapshot.day.succ_opt().unwrap(),
					time: None,
				}),
				..Default::default()
			},
		});
		snapshot.events = vec![task.clone()];
		let marker = calendar::deadline::marker(&task).unwrap();
		for id in [&task.id, "42", &marker.id] {
			assert_eq!(linked_event(snapshot.clone(), id), Some(task.clone()));
		}
		assert_eq!(linked_event(snapshot, "deleted"), None);
	}

	#[test]
	fn a_short_event_has_room_for_its_title_only() {
		assert_eq!(
			text_fit(15.0, 1),
			TextFit {
				top: 0,
				title_lines: 1,
				time_lines: 0
			}
		);
		assert_eq!(text_fit(30.0, 1).time_lines, 0);
	}

	#[test]
	fn a_tall_event_gives_what_its_time_leaves_to_the_title() {
		let fit = text_fit(60.0, 1);
		assert_eq!(fit.time_lines, 1);
		assert_eq!(fit.title_lines, 2);
	}

	#[test]
	fn a_sleep_block_keeps_both_of_its_time_lines_when_they_fit() {
		assert_eq!(text_fit(64.0, 2).time_lines, 2);
		assert_eq!(text_fit(40.0, 2).time_lines, 1);
	}
}

#[cfg(test)]
mod sheet_tests {
	use super::*;
	use chrono::{Local, TimeZone};
	use timeparagon_core::config::{Config, ProjectsSettings};
	use timeparagon_core::notes::Library;

	fn today() -> NaiveDate {
		NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
	}

	fn render(app: fn() -> Element) -> String {
		let mut dom = VirtualDom::new(app);
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	fn assert_offers(html: &str, wanted: &[&str]) {
		for text in wanted {
			assert!(html.contains(text), "{text} is missing from {html}");
		}
	}

	fn library() -> Library {
		let mut library = Library::default();
		let id = library.create("Trip plan", Vec::new());
		library.edit(&id, |note| {
			note.body = "# Packing\n\n- [x] Passport".to_owned()
		});
		library
	}

	#[test]
	fn the_notes_list_offers_its_notes_search_and_new() {
		fn app() -> Element {
			let mut store = use_hook(notes::NotesStore::new);
			use_hook(move || store.library.set(library()));
			let opening = use_signal(|| None::<String>);
			rsx! {
				notes::NotesSheet {
					store,
					settings: CalendarSettings::default(),
					events: Vec::new(),
					opening,
					on_follow: |_| {},
					on_close: |_| {},
				}
			}
		}
		assert_offers(
			&render(app),
			&["Search notes", ">New<", "Trip plan", "Refresh"],
		);
	}

	#[test]
	fn an_open_note_offers_its_tabs_colors_and_delete() {
		fn app() -> Element {
			let mut store = use_hook(notes::NotesStore::new);
			let id = use_hook(move || {
				let library = library();
				let id = library.notes[0].id.clone();
				store.library.set(library);
				id
			});
			let opening = use_signal(move || Some(id));
			rsx! {
				notes::NotesSheet {
					store,
					settings: CalendarSettings::default(),
					events: Vec::new(),
					opening,
					on_follow: |_| {},
					on_close: |_| {},
				}
			}
		}
		assert_offers(
			&render(app),
			&[
				">Edit<",
				">View<",
				"Open in default app",
				"Trip plan",
				"Butter",
				"Mint",
				"Connected to",
				"Delete note",
			],
		);
	}

	#[test]
	fn adding_offers_every_kind_starting_with_a_task() {
		fn app() -> Element {
			rsx! {
				create::CreateSheet {
					settings: CalendarSettings::default(),
					projects: ProjectsSettings::default(),
					calendars: Vec::new(),
					default_calendar: None,
					day: today(),
					on_created: |_| {},
					on_close: |_| {},
				}
			}
		}
		assert_offers(
			&render(app),
			&[
				">Task<",
				">Event<",
				">Time block<",
				"@home ~30m",
				"goes to the Inbox",
			],
		);
	}

	#[test]
	fn the_filter_tab_offers_each_filter() {
		fn app() -> Element {
			let search = use_signal(|| Search {
				time: chrono::NaiveTime::from_hms_opt(9, 30, 0),
				..Search::default()
			});
			rsx! {
				search::SearchFields {
					search,
					labels: vec!["deep".to_owned()],
					locations: vec!["Office".to_owned()],
					projects: ProjectsSettings::default(),
				}
			}
		}
		let html = render(app);
		assert_offers(
			&html,
			&[
				"Search description",
				"Search filters",
				"Time of day",
				"Any time",
				"Projects",
				"+ Add project",
				"Labels",
				"Event filters",
				"Show events that are",
				"Only show events with public visibility",
				"Busy or free",
				"Office",
				"deep",
				"Detail filter",
				"Show details by visibility",
				">Clear search<",
			],
		);
		let order = [
			"Search description",
			"Search filters",
			"Time of day",
			"Projects",
			"Labels",
			"Event filters",
			"Show events that are",
			"Only show events with public visibility",
			"Location",
			"Detail filter",
			"Show details by visibility",
			"Clear search",
		];
		for pair in order.windows(2) {
			assert!(
				html.find(pair[0]).unwrap() < html.find(pair[1]).unwrap(),
				"{} must be above {}",
				pair[0],
				pair[1]
			);
		}
	}

	#[test]
	fn the_filter_keeps_empty_fields_visible() {
		fn app() -> Element {
			let search = use_signal(Search::default);
			rsx! { search::SearchFields { search, labels: Vec::new(), locations: Vec::new(), projects: ProjectsSettings::default() } }
		}
		let html = render(app);
		assert_offers(
			&html,
			&[
				"No projects",
				"+ Add project",
				"Labels",
				"No labels on the events loaded so far",
				"Location",
				"Any location",
			],
		);
	}

	#[test]
	fn the_detail_filter_masks_timed_and_all_day_events_on_the_calendar() {
		fn app() -> Element {
			let start = Local.with_ymd_and_hms(2026, 11, 4, 9, 0, 0).unwrap();
			let mut private = Event {
				id: "private-event".into(),
				calendar_id: "work".into(),
				title: "Sensitive appointment".into(),
				start,
				end: start + chrono::Duration::hours(1),
				all_day: false,
				color: calendar::colors::DEFAULT_EVENT,
				completed: false,
				details: calendar::Details::default(),
			};
			let mut public = private.clone();
			public.id = "public-event".into();
			public.title = "Public lecture".into();
			public.details.options.visibility = calendar::event_options::Visibility::Public;
			let mut all_day = private.clone();
			all_day.id = "all-day".into();
			all_day.title = "Personal day".into();
			all_day.all_day = true;
			all_day.start = Local.with_ymd_and_hms(2026, 11, 4, 0, 0, 0).unwrap();
			all_day.end = all_day.start + chrono::Duration::days(1);
			private.details.location = "Secret office".into();
			let snapshot = calendar::Snapshot {
				revision: 0,
				calendars: Vec::new(),
				events: vec![private, public, all_day],
				day: today(),
				fetched_at: start,
				notice: None,
			};
			let settings = CalendarSettings::default();
			let model = day::build_model(&snapshot, &settings);
			let editing = use_signal(|| None::<Event>);
			let block = use_signal(|| None::<String>);
			let search = Search {
				details_by_visibility: true,
				..Search::default()
			};
			assert_eq!(
				model
					.timed
					.iter()
					.find(|timed| timed.event.id == "private-event")
					.unwrap()
					.event
					.title,
				"Sensitive appointment"
			);
			timeline(
				&model,
				&[],
				&settings,
				TimeFormat::TwentyFourHour,
				block,
				editing,
				SleepTimer {
					busy: false,
					press: Callback::new(|_| {}),
				},
				1.0,
				&search,
			)
		}
		let html = render(app);
		assert!(html.contains("Public lecture"));
		assert!(html.matches(">Private<").count() >= 2);
		assert!(!html.contains("Sensitive appointment"));
		assert!(!html.contains("Personal day"));
		assert!(!html.contains("Secret office"));
	}

	#[test]
	fn the_notepad_sheet_shows_only_the_notepad() {
		fn app() -> Element {
			let config = use_signal(Config::default);
			rsx! { scratch::ScratchSheet { config, on_close: |_| {} } }
		}
		let html = render(app);
		assert_offers(&html, &[">Notepad<", ">Edit<", ">View<", "Markdown"]);
		for other in [">Brain dump<", ">Projects<"] {
			assert!(!html.contains(other), "{other} is in {html}");
		}
	}

	#[test]
	fn brain_dump_has_its_own_searchable_sheet() {
		fn app() -> Element {
			let config = use_signal(Config::default);
			rsx! { scratch::ScratchSheet { config, page: scratch::Page::BrainDump, on_close: |_| {} } }
		}
		let html = render(app);
		assert_offers(&html, &[">Brain dump<", "Search every page", ">Add today<"]);
		assert!(!html.contains(">Notepad<"), "{html}");
		assert!(!html.contains(">Projects<"), "{html}");
	}

	fn render_list(app: fn() -> Element) -> String {
		render(app)
	}

	fn list_app(tab: TaskTab) -> Element {
		let start = Local.with_ymd_and_hms(2026, 11, 4, 0, 0, 0).unwrap();
		let snapshot = calendar::Snapshot {
			revision: 0,
			calendars: Vec::new(),
			events: Vec::new(),
			day: today(),
			fetched_at: start,
			notice: None,
		};
		let config = use_signal(Config::default);
		let revision = use_signal(|| 0u32);
		let editing = use_signal(|| None::<Event>);
		let create = use_signal(|| false);
		task_list(tab, &snapshot, config, false, revision, editing, create)
	}

	#[test]
	fn plan_opens_each_period_from_tasks_and_preserves_range_navigation() {
		fn app() -> Element {
			let mut page = use_signal(|| Page::Tasks);
			let view = use_signal(|| PlanView::Day);
			let mut menu_open = use_signal(|| true);
			let mut plan_anchor = use_signal(|| today() - chrono::Duration::days(7));
			let current_day = use_signal(today);
			use_hook(move || {
				let nav = PlanNav {
					page,
					view,
					menu_open,
					plan_anchor,
					current_day,
				};
				for period in PlanView::ALL {
					page.set(Page::Tasks);
					menu_open.set(true);
					nav.choose(period);
					assert_eq!(*page.peek(), Page::Plan);
					assert_eq!(*view.peek(), period);
					assert!(!*menu_open.peek());
					{
						assert_eq!(*plan_anchor.peek(), today());
						let next = today() + chrono::Duration::days(7);
						plan_anchor.set(next);
						nav.choose(period);
						assert_eq!(*plan_anchor.peek(), next);
					}
				}
			});
			rsx! { div {} }
		}
		render(app);
	}

	#[test]
	fn each_task_list_has_the_desktops_heading_and_empty_note() {
		assert_offers(
			&render_list(|| list_app(TaskTab::Overdue)),
			&[
				"Unfinished",
				"Sort by",
				"Manual order",
				"Nothing unfinished",
			],
		);
		assert_offers(
			&render_list(|| list_app(TaskTab::Inbox)),
			&[
				"Inbox",
				"Tasks waiting to be triaged",
				">New task<",
				"Sort by",
				"Nothing to triage",
			],
		);
		assert_offers(
			&render_list(|| list_app(TaskTab::Today)),
			&["Today", "No tasks left to slot in today"],
		);
		let backlog = render_list(|| list_app(TaskTab::Backlog));
		assert_offers(
			&backlog,
			&[
				"Backlog",
				"Search tasks",
				"No tasks on the Backlog list",
				"Next week or two",
				"Never",
			],
		);
		assert!(!backlog.contains("Sort by"));
	}

	#[test]
	fn every_task_list_can_be_searched_by_title_or_description() {
		for html in [
			render_list(|| list_app(TaskTab::Overdue)),
			render_list(|| list_app(TaskTab::Inbox)),
			render_list(|| list_app(TaskTab::Today)),
			render_list(|| list_app(TaskTab::Backlog)),
		] {
			assert_offers(&html, &["Search title or description…", "Search tasks"]);
		}
	}
}
