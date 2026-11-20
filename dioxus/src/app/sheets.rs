//! Sheet composition, separated from the calendar grid and persistent services.
use super::calendar_state::DayData;
use super::*;

#[component]
pub(super) fn CalendarSheets() -> Element {
	let Session {
		mut config,
		mut revision,
		..
	} = use_context::<Session>();
	let Navigation {
		current_day,
		mut page,
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
		selected_block,
		mut editing,
		block_writing,
		..
	} = use_context::<Selection>();
	let DayData {
		snapshot: snapshot_memo,
		display,
		calendar_sources,
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
	let selected_day = current_day();
	let today = day::today(&settings);
	let snapshot = snapshot_memo();
	let model = display.read().0.clone();
	let edit_context = snapshot
		.as_ref()
		.map(|snapshot| EditContext::new(&config.read(), snapshot));
	let after_write = move |()| revision.set(revision() + 1);

	let modal = model
		.as_ref()
		.zip(edit_context.as_ref())
		.and_then(|(model, context)| {
			let id = selected_block()?;
			let block = model.blocks.iter().find(|block| block.event.id == id)?;
			Some(time_block_modal(
				block,
				context,
				selected_block,
				editing,
				block_writing,
				after_write,
			))
		});

	let create_sheet = show_create().then(|| {
		let calendars = calendar_sources();
		let default_calendar = calendar::default_calendar_id(&calendars, &settings);
		rsx! {
			create::CreateSheet {
				models: config.read().voice.clone(),
				settings: settings.clone(),
				projects: config.read().projects.clone(),
				calendars,
				default_calendar,
				day: selected_day,
				range: create_range(),
				on_created: after_write,
				on_close: move |()| {
					show_create.set(false);
					create_range.set(None);
				},
			}
		}
	});

	let agenda_sheet = show_agenda().then(|| {
		rsx! {
			agenda::AgendaSheet {
				settings: settings.clone(),
				day: selected_day,
				today,
				revision,
				filter: agenda_filter(),
				on_day: move |day| {
					show_agenda.set(false);
					agenda_filter.set(None);
					page.set(Page::Do);
					day_nav.go_to(day, day == day::today(&config.peek().calendar));
				},
				on_open: move |event: Event| {
					let day = day::event_day(&event, &config.peek().calendar);
					show_agenda.set(false);
					agenda_filter.set(None);
					page.set(Page::Do);
					day_nav.go_to(day, day == day::today(&config.peek().calendar));
					pending_open.set(Some(event.id));
				},
				on_close: move |()| {
					show_agenda.set(false);
					agenda_filter.set(None);
				},
			}
		}
	});
	let edit_sheet = editing().zip(edit_context).map(|(event, context)| {
		rsx! {
			editor::Editor {
				key: "{event.id}",
				event,
				context,
				on_close: move |()| editing.set(None),
				on_saved: after_write,
				on_config: move |saved| config.set(saved),
			}
		}
	});

	rsx! { {modal} {create_sheet} {agenda_sheet} {edit_sheet} }
}
#[component]
pub(super) fn AuxiliarySheets() -> Element {
	let Session { config, .. } = use_context::<Session>();
	let Navigation {
		current_day,
		mut page,
		scrolled,
		follow_today,
		mut pending_open,
		..
	} = use_context::<Navigation>();
	let Panels {
		mut show_notes,
		mut show_scratch,
		mut show_more,
		mut show_people,
		mut show_alarms,
		mut show_travel,
		mut show_log,
		note_opening,
		..
	} = use_context::<Panels>();
	let Selection { selected_block, .. } = use_context::<Selection>();
	let Auth {
		connect_google,
		connecting,
		..
	} = use_context::<Auth>();
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
	let selected_day = current_day();
	let snapshot = snapshot_memo();
	let begin_alarm = navigation::use_begin_alarm();
	let people_store = use_signal(people::Store::live);
	let notes_store = use_context::<notes::NotesStore>();
	let scratch_sheet = show_scratch().map(|page| {
		rsx! {
			scratch::ScratchSheet {
				config,
				page,
				on_close: move |()| show_scratch.set(None),
			}
		}
	});

	let more_sheet = show_more().then(|| {
		rsx! {
			more::MoreSheet {
				on_close: move |()| show_more.set(false),
				on_open: move |item| {
					show_more.set(false);
					match item {
						Item::Notepad => show_scratch.set(Some(scratch::Page::Notepad)),
						Item::Habits => page.set(Page::Habits),
						Item::Notes => show_notes.set(true),
						Item::People => show_people.set(true),
						Item::Travel => show_travel.set(true),
						Item::BrainDump => show_scratch.set(Some(scratch::Page::BrainDump)),
						Item::Alarms => show_alarms.set(true),
						Item::Logs => show_log.set(true),
						// The phone's More menu has no routines tile.
						Item::Routines => {}
					}
				},
			}
		}
	});

	let travel_sheet = show_travel().then(|| {
		let connections = travel::connections(selected_day, snapshot.as_ref(), &config.read());
		rsx! {
			travel::TravelSheet {
				config,
				day: selected_day,
				connections,
				on_close: move |()| show_travel.set(false),
			}
		}
	});

	let people_settings = settings.clone();
	let people_sheet = show_people().then(|| {
		let connect_settings = people_settings.clone();
		rsx! {
			people::PeopleSheet {
				store: people_store,
				settings: people_settings,
				connecting,
				on_reconnect: move |()| {
					connect_google.call(connect_settings.clone());
				},
				on_close: move |()| show_people.set(false),
			}
		}
	});

	let notes_events = snapshot
		.as_ref()
		.map(|snapshot| snapshot.events.clone())
		.unwrap_or_default();
	let notes_sheet = (show_notes() || note_opening().is_some()).then(|| {
		rsx! {
			notes::NotesSheet {
				store: notes_store,
				settings: settings.clone(),
				events: notes_events,
				opening: note_opening,
				on_follow: move |link: timeparagon_core::notes::Link| {
					let mut note_opening = note_opening;
					note_opening.set(None);
					show_notes.set(false);
					if let Some(day) = link.day {
						day_nav.go_to(day, day == day::today(&config.peek().calendar));
					}
					pending_open.set(Some(link.id));
				},
				on_close: move |()| show_notes.set(false),
			}
		}
	});

	rsx! {
		{scratch_sheet} {more_sheet} {travel_sheet} {people_sheet} {notes_sheet}
		if show_alarms() { alarms::AlarmsSheet {
			config, day: selected_day,
			on_place: snapshot.as_ref().map(|_| begin_alarm),
			on_close: move |_| show_alarms.set(false),
		} }
		alarms::AlarmEditor { config }
	}
}
#[component]
pub(super) fn ViewSheet() -> Element {
	let Panels { view_sheet, .. } = use_context::<Panels>();
	rsx! { if view_sheet().is_some() { ViewForm {} } }
}

#[component]
pub(super) fn ViewForm() -> Element {
	let Session { mut config, .. } = use_context::<Session>();
	let Panels { mut view_sheet, .. } = use_context::<Panels>();
	let Selection {
		search_state,
		calendar_zoom,
		..
	} = use_context::<Selection>();
	let Auth { signed_in, .. } = use_context::<Auth>();
	let DayData {
		snapshot: snapshot_memo,
		calendar_sources,
		..
	} = use_context::<DayData>();
	let mut calendars_error = use_signal(|| None::<String>);
	let snapshot = snapshot_memo();
	let settings = config.read().calendar.clone();
	let Some(view_tab) = view_sheet() else {
		return rsx! {};
	};
	let calendars = calendar_sources();
	let sections =
		day::source_sections(&calendar::display_sources(&calendars, &settings), &settings);
	let no_calendars = calendars.is_empty();
	let travel_shown = config.read().travel.show;
	let error = calendars_error().map(|message| {
		rsx! { p { class: "error-text", "{message}" } }
	});
	let sections = sections
		.into_iter()
		.filter(|(_, rows)| !rows.is_empty())
		.map(|(heading, rows)| {
			let rows = rows.into_iter().map(|row| {
				let swatch = format!("background:{};", css_color(row.color));
				let enabled = row.enabled;
				let id = row.id.clone();
				rsx! {
					button {
						key: "{row.id}",
						class: "source-row",
						role: "switch",
						aria_checked: enabled.to_string(),
						onclick: move |_| {
							let calendars = calendar_sources();
							match day::set_source_enabled(&calendars, &id, !enabled) {
								Ok(saved) => {
									config.set(saved);
									calendars_error.set(None);
								}
								Err(error) => calendars_error.set(Some(error)),
							}
						},
						span { class: "source-swatch", style: "{swatch}" }
						span { class: "source-name", "{row.name}" }
						span { class: "source-switch", span { class: "source-knob" } }
					}
				}
			});
			// Travel is not an event kind, but it is toggled with the sources
			// that are, as on the desktop.
			let travel_row = (heading == "TimeParagon").then(|| {
				let swatch = format!("background:{};", css_color(calendar::colors::TRAVEL));
				rsx! {
					button {
						class: "source-row",
						role: "switch",
						aria_checked: travel_shown.to_string(),
						onclick: move |_| match day::set_travel_shown(!travel_shown) {
							Ok(saved) => {
								config.set(saved);
								calendars_error.set(None);
							}
							Err(error) => calendars_error.set(Some(error)),
						},
						span { class: "source-swatch", style: "{swatch}" }
						span { class: "source-name", "Travel" }
						span { class: "source-switch", span { class: "source-knob" } }
					}
				}
			});
			rsx! {
				section { key: "{heading}", class: "source-section",
					h3 { class: "source-heading", "{heading}" }
					{rows}
					{travel_row}
				}
			}
		});
	let (labels, locations) = day::search_choices(
		snapshot
			.as_ref()
			.map(|snapshot| snapshot.events.as_slice())
			.unwrap_or_default(),
		&search_state.read(),
	);
	let view_tab_button = |label: &'static str, this: ViewTab| {
		rsx! {
			button {
				class: "nav-tab",
				aria_pressed: (view_tab == this).to_string(),
				onclick: move |_| view_sheet.set(Some(this)),
				"{label}"
			}
		}
	};
	// The desktop's View sheet: which calendars show, and a filter that
	// narrows every view.
	rsx! {
		div {
			class: "modal-backdrop",
			onclick: move |_| view_sheet.set(None),
			div {
				class: "modal",
				role: "dialog",
				aria_label: "View",
				onclick: move |event| event.stop_propagation(),
				div { class: "modal-head",
					h2 { class: "modal-title", "View" }
					div { class: "spacer" }
					button {
						class: "modal-close",
						title: "Close",
						onclick: move |_| view_sheet.set(None),
						"×"
					}
				}
				div { class: "sheet-tabs",
					{view_tab_button("Calendars", ViewTab::Calendars)}
					{view_tab_button("Filter", ViewTab::Filter)}
				}
				div { class: "modal-body",
					if view_tab == ViewTab::Filter {
						search::SearchFields {
							search: search_state,
							labels,
							locations,
							projects: config.read().projects.clone(),
						}
					} else {
						{error}
						{sections}
						calendar_gestures::ZoomControls { zoom: calendar_zoom }
						if no_calendars {
							p { class: "empty-note",
								if signed_in() {
									"No calendars yet. Use Actions → Sync now to load them."
								} else {
									"Connect Google to load your calendars."
								}
							}
						}
					}
				}
			}
		}
	}
}
#[component]
pub(super) fn LogSheet() -> Element {
	let SyncState {
		google_log,
		database_sync_log,
		..
	} = use_context::<SyncState>();
	let Panels { mut show_log, .. } = use_context::<Panels>();
	let log_entries = move || merged_log(database_sync_log(), google_log());
	rsx! {
			if show_log() {
				div { class: "modal-backdrop", onclick: move |_| show_log.set(false),
					div { class: "modal", onclick: move |event| event.stop_propagation(),
						div { class: "modal-head", h2 { class: "modal-title", "Log" }
							button { class: "btn", onclick: move |_| show_log.set(false), "Close" }
						}
						div { class: "modal-body",
							if log_entries().is_empty() {p { "Nothing has been logged yet." }}
							for (time, text, failed, key) in log_entries() {
								p { key: "{key}", class: if failed { "editor-error" } else { "" },
									span { class: "hint", "{time} " }
									"{text}"
								}
							}
						}
					}
				}
			}
	}
}
