//! The calendar scope: cache selection, day models and independently rendered surfaces.
use super::authentication::CalendarUnavailable;
use super::calendar_state::{CalendarEffects, DayData, use_day_data};
use super::*;

#[component]
pub(super) fn Workspace() -> Element {
	let data = use_day_data();
	use_context_provider(|| data);
	rsx! {
		navigation::SelectionLifetime {}
		CalendarEffects {}
		Warnings {}
		sleep_controls::SleepStatus {}
		DayScreen {}
		sheets::CalendarSheets {}
		sheets::AuxiliarySheets {}
		sheets::LogSheet {}
		settings::SettingsSheet {}
		sheets::ViewSheet {}
	}
}

#[component]
pub(super) fn HeaderActions() -> Element {
	let Session { mut revision, .. } = use_context::<Session>();
	let Panels {
		mut show_actions,
		mut view_sheet,
		..
	} = use_context::<Panels>();
	let Selection { search_state, .. } = use_context::<Selection>();
	let Auth { signed_in, .. } = use_context::<Auth>();
	let DayData {
		snapshot: snapshot_memo,
		loaded: data,
		..
	} = use_context::<DayData>();
	let begin_alarm = navigation::use_begin_alarm();
	let pending = matches!(*data.state().read(), UseResourceState::Pending);
	let searching = search_state.read().is_modified();
	// View (Calendars and Filter) and Actions, beside the picker.
	let view_lit = view_sheet().is_some() || searching;
	let can_place_alarm = snapshot_memo.read().is_some();

	rsx! {
		button {
			class: if view_lit { "btn primary" } else { "btn" },
			aria_expanded: view_sheet().is_some().to_string(),
			onclick: move |_| {
				show_actions.set(false);
				view_sheet.set(Some(ViewTab::Calendars));
			},
			"View"
		}
		div { class: "month-anchor",
			button {
				class: if show_actions() { "btn primary" } else { "btn" },
				aria_expanded: show_actions().to_string(),
				onclick: move |_| show_actions.set(!show_actions()),
				"Actions"
			}
			if show_actions() {
				div { class: "month-dismiss", onclick: move |_| show_actions.set(false) }
				div { class: "actions-menu", role: "menu", aria_label: "Actions",
					button {
						class: "btn",
						disabled: pending || !signed_in(),
						onclick: move |_| {
							show_actions.set(false);
							revision.set(revision() + 1);
						},
						if pending { "Syncing…" } else { "Sync now" }
					}
					button { class: "btn", disabled: !can_place_alarm,
						onclick: move |_| begin_alarm.call(()), "Set alarm"
					}
				}
			}
		}
	}
}
#[component]
pub(super) fn LoadingStatus() -> Element {
	let SyncState { google_sending, .. } = use_context::<SyncState>();
	let DayData { loaded: data, .. } = use_context::<DayData>();
	let pending = matches!(*data.state().read(), UseResourceState::Pending);

	rsx! {
		if google_sending() > 0 {span { class: "hint", role: "status", "Saving…" }}
		if pending {
			p { class: "day-loading", "Loading..." }
		}
	}
}
#[component]
pub(super) fn DayHeader() -> Element {
	let Session { config, .. } = use_context::<Session>();
	let Navigation {
		current_day,
		page,
		month_open,
		month_shown,
		scrolled,
		follow_today,
		pending_open,
		..
	} = use_context::<Navigation>();
	let Selection { selected_block, .. } = use_context::<Selection>();
	let day_nav = DayNav {
		current_day,
		selected_block,
		scrolled,
		follow_today,
		pending_open,
	};
	let settings = config.read().calendar.clone();
	let selected_day = current_day();
	let today = day::today(&settings);
	// The day's picker is labelled with the day in view, and is the way to
	// move between days.
	// Tasks keeps the grid beside its lists where there is room.
	let day_header_class = if page() == Page::Tasks {
		"wide-only"
	} else {
		""
	};

	rsx! {
		div { class: "day-header {day_header_class}",
			div { class: "day-controls",
				{month_picker(
					calendar::month_day_label(selected_day),
					selected_day,
					Vec::new(),
					selected_day == today,
					today,
					settings.first_day_of_week,
					month_open,
					month_shown,
					Pick::Day(day_nav),
				)}
				HeaderActions {}
			}
			LoadingStatus {}
		}
	}
}
#[component]
pub(super) fn DayBody() -> Element {
	let Session {
		config,
		mut revision,
		..
	} = use_context::<Session>();
	let Navigation { page, task_tab, .. } = use_context::<Navigation>();
	let Panels { show_create, .. } = use_context::<Panels>();
	let Selection {
		selected_block,
		editing,
		search_state,
		calendar_zoom,
		..
	} = use_context::<Selection>();
	let Auth { signed_in, .. } = use_context::<Auth>();
	let DayData {
		snapshot: snapshot_memo,
		display,
		loaded: data,
		..
	} = use_context::<DayData>();
	let settings = config.read().calendar.clone();
	let snapshot = snapshot_memo();
	let (model, travel) = display();
	let time_format = config.read().appearance.time_format;
	let pending = matches!(*data.state().read(), UseResourceState::Pending);
	let fetch_error = data
		.read()
		.as_ref()
		.and_then(|result| result.as_ref().err())
		.cloned();

	let SleepControl {
		sleep_busy,
		press_sleep,
		..
	} = use_context::<SleepControl>();
	let sleep_timer = SleepTimer {
		busy: sleep_busy(),
		press: press_sleep,
	};
	match &model {
		Some(model) => {
			// Tasks shows its list, with the grid beside it on wide windows.
			let lists = (page() == Page::Tasks)
				.then_some(snapshot.as_ref())
				.flatten()
				.map(|snapshot| {
					task_list(
						task_tab(),
						snapshot,
						config,
						signed_in(),
						revision,
						editing,
						show_create,
					)
				});
			let view = if lists.is_some() { "tasks" } else { "calendar" };
			rsx! {
				div { class: "day-body", "data-view": view,
					if let Some(lists) = lists {
						aside { id: "tasks-pane", class: "sidebar", aria_label: "Tasks",
							div { class: "sidebar-scroll", {lists} }
						}
					}
					{timeline(model, &travel, &settings, time_format, selected_block, editing, sleep_timer, calendar_zoom(), &search_state.read())}
					if let Some(snapshot) = &snapshot {
						calendar_gestures::Gestures {
							context: EditContext::new(&config.read(), snapshot),
							zoom: calendar_zoom,
							on_saved: move |_| revision.set(revision() + 1),
						}
					}
				}
			}
		}
		None => rsx! { CalendarUnavailable { pending, fetch_error } },
	}
}
#[component]
pub(super) fn Warnings() -> Element {
	let Session {
		config,
		mut revision,
		..
	} = use_context::<Session>();
	let Auth {
		connect_google,
		connecting,
		connect_error,
		..
	} = use_context::<Auth>();
	let DayData {
		snapshot: snapshot_memo,
		loaded: data,
		..
	} = use_context::<DayData>();
	let settings = config.read().calendar.clone();
	let snapshot = snapshot_memo();
	let pending = matches!(*data.state().read(), UseResourceState::Pending);
	let fetch_error = data
		.read()
		.as_ref()
		.and_then(|result| result.as_ref().err())
		.cloned();
	let warning_connect_settings = settings.clone();
	rsx! {
		warnings::WarningCenter {
			sync_error: fetch_error.clone(),
			connect_error: connect_error(),
			notice: snapshot.as_ref().and_then(|snapshot| snapshot.notice.clone()),
			pending,
			connecting: connecting(),
			on_retry: move |_| revision.set(revision() + 1),
			on_connect: move |_| connect_google.call(warning_connect_settings.clone()),
		}
	}
}
#[component]
pub(super) fn DayScreen() -> Element {
	let Session {
		config, revision, ..
	} = use_context::<Session>();
	let Navigation {
		current_day,
		page,
		plan_view,
		plan_anchor,
		scrolled,
		follow_today,
		mut pending_open,
		..
	} = use_context::<Navigation>();
	let Panels {
		mut agenda_filter,
		mut show_create,
		mut show_agenda,
		mut create_range,
		..
	} = use_context::<Panels>();
	let Selection {
		mut selected_block,
		mut editing,
		alarm_placement,
		mut draft_range,
		search_state,
		calendar_zoom,
		..
	} = use_context::<Selection>();
	let Auth { signed_in, .. } = use_context::<Auth>();
	let DayData {
		snapshot: snapshot_memo,
		..
	} = use_context::<DayData>();
	let day_nav = DayNav {
		current_day,
		selected_block,
		scrolled,
		follow_today,
		pending_open,
	};
	let settings = config.read().calendar.clone();
	let today = day::today(&settings);
	let snapshot = snapshot_memo();
	let time_format = config.read().appearance.time_format;
	let range_page = (page() == Page::Plan).then(|| {
		rsx! {
			div { class: "day-header",
				div { class: "day-controls", HeaderActions {} }
				LoadingStatus {}
			}
			schedule::Planner {
				config,
				zoom: calendar_zoom,
				settings: settings.clone(),
				view: plan_view,
				anchor: plan_anchor,
				today,
				revision,
				time_format,
				search: search_state.read().clone(),
				on_day: move |day| day_nav.go_to(day, day == day::today(&config.peek().calendar)),
				on_block: move |event: Event| {
					let day = day::event_day(&event, &config.peek().calendar);
					day_nav.go_to(day, day == day::today(&config.peek().calendar));
					selected_block.set(Some(event.id));
				},
				on_open: move |event: Event| {
					let day = day::event_day(&event, &config.peek().calendar);
					day_nav.go_to(day, day == day::today(&config.peek().calendar));
					pending_open.set(Some(event.id));
				},
			}
		}
	});
	rsx! {
			if page() == Page::Habits {
				habits::HabitsScreen {
					today,
					time_format,
					settings: settings.clone(),
					signed_in: signed_in(),
					revision,
					day_events: snapshot.as_ref().map(|snapshot| snapshot.events.clone()).unwrap_or_default(),
					on_open_event: move |event| editing.set(Some(event)),
					on_show_calendar: move |filter| {
						agenda_filter.set(Some(filter));
						show_agenda.set(true);
					},
				}
			} else if let Some(range_page) = range_page {
				{range_page}
			} else {
				DayHeader {}
				DayBody {}
			}
			alarms::PlacementControls { config }
			calendar_gestures::RangeControls {
				time_format,
				on_confirm: move |range| {
					create_range.set(Some(range));
					draft_range.set(None);
					show_create.set(true);
				},
			}
			if snapshot.is_some()
				&& page() != Page::Habits
				&& alarm_placement().is_none()
				&& draft_range().is_none()
			{
				button {
					class: "fab",
					title: "Add",
					aria_label: "Add",
					onclick: move |_| show_create.set(true),
					"+"
				}
			}
	}
}
