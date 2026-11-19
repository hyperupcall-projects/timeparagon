//! Page navigation and the transient selections shared by calendar surfaces.
use super::*;
#[derive(Clone, Copy)]
pub(super) struct Navigation {
	pub current_day: Signal<NaiveDate>,
	pub page: Signal<Page>,
	pub plan_view: Signal<PlanView>,
	pub task_tab: Signal<TaskTab>,
	pub plan_menu_open: Signal<bool>,
	pub plan_anchor: Signal<NaiveDate>,
	pub month_open: Signal<bool>,
	pub month_shown: Signal<NaiveDate>,
	pub scrolled: Signal<bool>,
	pub follow_today: Signal<bool>,
	pub pending_open: Signal<Option<String>>,
}
#[derive(Clone, Copy)]
pub(super) struct Panels {
	pub show_actions: Signal<bool>,
	pub view_sheet: Signal<Option<ViewTab>>,
	pub agenda_filter: Signal<Option<habits::CalendarFilter>>,
	pub show_notes: Signal<bool>,
	pub show_create: Signal<bool>,
	pub show_agenda: Signal<bool>,
	pub show_scratch: Signal<Option<scratch::Page>>,
	pub show_more: Signal<bool>,
	pub show_people: Signal<bool>,
	pub show_alarms: Signal<bool>,
	pub show_travel: Signal<bool>,
	pub show_log: Signal<bool>,
	pub note_opening: Signal<Option<String>>,
	pub create_range: Signal<Option<calendar_gestures::TimeRange>>,
}
#[derive(Clone, Copy)]
pub(super) struct Selection {
	pub selected_block: Signal<Option<String>>,
	pub editing: Signal<Option<Event>>,
	pub block_writing: Signal<bool>,
	pub alarm_placement: Signal<Option<chrono::NaiveDateTime>>,
	pub draft_range: Signal<Option<calendar_gestures::TimeRange>>,
	pub search_state: Signal<Search>,
	pub calendar_zoom: Signal<f32>,
}

pub(super) fn use_navigation(config: Signal<timeparagon_core::config::Config>) -> Navigation {
	let current_day = use_signal(|| day::today(&config.peek().calendar));
	let page = use_signal(|| Page::Do);
	let plan_view = use_signal(|| PlanView::Day);
	let task_tab = use_signal(|| TaskTab::Overdue);
	let plan_menu_open = use_signal(|| false);
	let plan_anchor = use_signal(|| *current_day.peek());
	let month_open = use_signal(|| false);
	let month_shown = use_signal(|| calendar::first_of_month(*current_day.peek()));
	let scrolled = use_signal(|| false);
	let follow_today = use_signal(|| true);
	let pending_open = use_signal(|| None::<String>);
	Navigation {
		current_day,
		page,
		plan_view,
		task_tab,
		plan_menu_open,
		plan_anchor,
		month_open,
		month_shown,
		scrolled,
		follow_today,
		pending_open,
	}
}

pub(super) fn use_panels() -> Panels {
	let show_actions = use_signal(|| false);
	let view_sheet = use_signal(|| None::<ViewTab>);
	let agenda_filter = use_signal(|| None::<habits::CalendarFilter>);
	let show_notes = use_signal(|| false);
	let show_create = use_signal(|| false);
	let show_agenda = use_signal(|| false);
	let show_scratch = use_signal(|| None::<scratch::Page>);
	let show_more = use_signal(|| false);
	let show_people = use_signal(|| false);
	let show_alarms = use_signal(|| false);
	let show_travel = use_signal(|| false);
	let show_log = use_signal(|| false);
	let note_opening = use_signal(|| None::<String>);
	let create_range = use_signal(|| None::<calendar_gestures::TimeRange>);
	use_context_provider(|| notes::NoteOpening(note_opening));
	Panels {
		show_actions,
		view_sheet,
		agenda_filter,
		show_notes,
		show_create,
		show_agenda,
		show_scratch,
		show_more,
		show_people,
		show_alarms,
		show_travel,
		show_log,
		note_opening,
		create_range,
	}
}

pub(super) fn use_selection() -> Selection {
	let selected_block = use_signal(|| None::<String>);
	let editing = use_signal(|| None::<Event>);
	let block_writing = use_signal(|| false);
	let alarm_placement = use_signal(|| None::<chrono::NaiveDateTime>);
	let draft_range = use_signal(|| None::<calendar_gestures::TimeRange>);
	let search_state = use_signal(Search::default);
	let calendar_zoom = use_signal(|| 1.0f32);
	use_effect(move || calendar_gestures::sync_zoom(calendar_zoom()));
	use_context_provider(|| alarms::Placement(alarm_placement));
	use_context_provider(|| calendar_gestures::DraftRange(draft_range));
	Selection {
		selected_block,
		editing,
		block_writing,
		alarm_placement,
		draft_range,
		search_state,
		calendar_zoom,
	}
}

#[component]
pub(super) fn SelectionLifetime() -> Element {
	let Navigation {
		current_day,
		page,
		plan_view,
		..
	} = use_context::<Navigation>();
	let Selection {
		mut alarm_placement,
		mut draft_range,
		..
	} = use_context::<Selection>();
	use_effect(move || {
		if page() != Page::Do
			&& !(page() == Page::Plan && matches!(plan_view(), PlanView::Day | PlanView::MultiDay))
		{
			alarm_placement.set(None);
		}
	});
	// A range belongs to the grid it was drawn on.
	use_effect(move || {
		let _ = (page(), plan_view(), current_day());
		draft_range.set(None);
	});
	rsx! {}
}

#[component]
pub(super) fn Topbar() -> Element {
	let Navigation {
		current_day,
		mut page,
		plan_view,
		task_tab,
		mut plan_menu_open,
		plan_anchor,
		mut scrolled,
		..
	} = use_context::<Navigation>();
	let Panels {
		show_notes,
		show_scratch,
		mut show_more,
		show_travel,
		show_log,
		..
	} = use_context::<Panels>();
	let mut open_settings = move || {
		show_more.set(false);
		plan_menu_open.set(false);
		scrolled.set(false);
		page.set(Page::Settings);
	};
	let plan_nav = PlanNav {
		page,
		view: plan_view,
		menu_open: plan_menu_open,
		plan_anchor,
		current_day,
	};
	let plan_active = plan_menu_open() || page() == Page::Plan;
	let more_active = show_more()
		|| page() == Page::Habits
		|| show_notes()
		|| show_travel()
		|| show_log()
		|| show_scratch().is_some();
	let task_tabs = TaskTab::ALL.into_iter().map(|tab| {
		let mut task_tab = task_tab;
		nav_tab(tab.label(), task_tab() == tab, "", move |_| {
			plan_menu_open.set(false);
			task_tab.set(tab);
		})
	});
	let plan_menu = plan_menu_open().then(|| {
		rsx! {
			div { class: "menu-dismiss", onclick: move |_| plan_menu_open.set(false) }
			div { class: "plan-menu", role: "menu", aria_label: "Plan",
				for view in PlanView::ALL {
					button {
						key: "{view.menu_label()}",
						class: "nav-tab",
						role: "menuitemradio",
						aria_pressed: (plan_view() == view).to_string(),
						onclick: move |_| plan_nav.choose(view),
						"{view.menu_label()}"
					}
				}
			}
		}
	});

	// Tasks owns the list navigation beneath the main row.
	rsx! {
		header { class: "nav-card",
			div { class: "nav-row",
				{nav_tab("Do", page() == Page::Do, "", move |_| {
					plan_menu_open.set(false);
					page.set(Page::Do);
				})}
				div { class: if plan_active { "plan-switch active" } else { "plan-switch" },
					button {
						class: "plan-main",
						aria_pressed: (page() == Page::Plan).to_string(),
						onclick: move |_| plan_nav.choose(plan_view()),
						"{plan_view().label()}"
					}
					button {
						class: "plan-toggle",
						title: "Choose what to plan",
						aria_label: "Choose what to plan",
						aria_expanded: plan_menu_open().to_string(),
						onclick: move |_| plan_menu_open.set(!plan_menu_open()),
						"▾"
					}
				}
				{nav_tab("Tasks", page() == Page::Tasks, "", move |_| {
					plan_menu_open.set(false);
					page.set(Page::Tasks);
				})}
				div { class: "spacer" }
				button {
					class: "nav-tab nav-icon",
					title: "More",
					aria_label: "More",
					aria_pressed: more_active.to_string(),
					aria_expanded: show_more().to_string(),
					onclick: move |_| {
						plan_menu_open.set(false);
						show_more.set(true);
					},
					{nav_icon(false)}
				}
				button {
					class: "nav-tab nav-icon",
					title: "Settings",
					aria_label: "Settings",
					aria_pressed: (page() == Page::Settings).to_string(),
					onclick: move |_| open_settings(),
					{nav_icon(true)}
				}
			}
			if page() == Page::Tasks {
				div { class: "nav-row task-tabs", {task_tabs} }
			}
			{plan_menu}
		}
	}
}

pub(super) fn use_begin_alarm() -> Callback<()> {
	let Session { config, .. } = use_context::<Session>();
	let Navigation {
		current_day,
		mut page,
		scrolled,
		follow_today,
		pending_open,
		..
	} = use_context::<Navigation>();
	let Panels {
		mut show_actions,
		mut show_alarms,
		..
	} = use_context::<Panels>();
	let Selection {
		selected_block,
		mut alarm_placement,
		..
	} = use_context::<Selection>();
	let day_nav = DayNav {
		current_day,
		selected_block,
		scrolled,
		follow_today,
		pending_open,
	};
	use_callback(move |()| {
		let hour_start = config.peek().calendar.day_start_hour();
		let at = alarms::default_placement(*current_day.peek(), hour_start);
		let day = (at - chrono::Duration::hours(i64::from(hour_start))).date();
		show_actions.set(false);
		show_alarms.set(false);
		page.set(Page::Do);
		day_nav.go_to(day, day == day::today(&config.peek().calendar));
		alarm_placement.set(Some(at));
		spawn(async move {
			let _ = document::eval(
				r#"
				await new Promise(resolve => requestAnimationFrame(resolve));
				window.__calendarGesture?.saved();
				const needle = document.querySelector('.alarm-needle');
				const scroller = needle?.closest('.timeline-scroll');
				if (scroller) scroller.scrollTop += needle.getBoundingClientRect().top
					- scroller.getBoundingClientRect().top - scroller.clientHeight / 2;
				needle?.focus({preventScroll: true});
			"#,
			)
			.await;
		});
	})
}

#[component]
pub(super) fn NavigationProvider(children: Element) -> Element {
	let Session { config, .. } = use_context::<Session>();
	let navigation = use_navigation(config);
	use_context_provider(|| navigation);
	let panels = use_panels();
	use_context_provider(|| panels);
	let selection = use_selection();
	use_context_provider(|| selection);
	children
}
