use super::*;
use iced::futures::{StreamExt, executor::block_on};

#[derive(Debug)]
enum GeometryAction {
	Resize(Size),
	Min(Option<Size>),
	Max(Option<Size>),
	Open(window::Settings),
}

fn geometry_actions(task: Task<Message>) -> Vec<GeometryAction> {
	use iced_test::runtime::{Action, task, window::Action as WindowAction};
	block_on(async {
		let Some(mut stream) = task::into_stream(task) else {
			return Vec::new();
		};
		let mut actions = Vec::new();
		while let Some(action) = stream.next().await {
			match action {
				Action::Window(WindowAction::Resize(_, size)) => {
					actions.push(GeometryAction::Resize(size));
				}
				Action::Window(WindowAction::SetMinSize(_, size)) => {
					actions.push(GeometryAction::Min(size));
				}
				Action::Window(WindowAction::SetMaxSize(_, size)) => {
					actions.push(GeometryAction::Max(size));
				}
				Action::Window(WindowAction::GetMonitorSize(_, sender)) => {
					let _ = sender.send(None);
				}
				Action::Window(WindowAction::Open(id, settings, sender)) => {
					actions.push(GeometryAction::Open(settings));
					let _ = sender.send(id);
				}
				_ => {}
			}
		}
		actions
	})
}

fn flash(settings: config::Flash) -> Flash {
	Flash {
		window: window::Id::unique(),
		started: Instant::now(),
		settings,
	}
}

#[test]
fn choosing_an_event_opens_the_creation_pane_without_saving() {
	let mut state = State::blank();
	let start = calendar::at_clock(chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(), 9, 0);
	let end = start + chrono::Duration::minutes(45);
	state.calendar.begin_draft("primary".into(), start, end);
	let _ = update(
		&mut state,
		Message::EventDraft(EventDraftMessage::EventDraftTitleChanged("Planning".into())),
	);
	let _ = update(
		&mut state,
		Message::EventDraft(EventDraftMessage::EventDraftKindSelected(
			calendar::DraftKind::Event,
		)),
	);
	let draft = state.calendar.draft.as_ref().unwrap();
	assert!(draft.editor_open);
	assert!(state.details_pane_open);
	assert_eq!(draft.title, "Planning");
	assert_eq!((draft.start, draft.end), (start, end));
	assert_eq!(draft.kind, calendar::DraftKind::Event);
	let _ = update(
		&mut state,
		Message::EventDraft(EventDraftMessage::EventDraftCancelled),
	);
	assert!(state.calendar.draft.is_none());
	assert!(!state.details_pane_open);
}

#[test]
fn choosing_a_time_block_starts_the_block_write_instead_of_the_event_form() {
	let mut state = State::blank();
	state.config.calendar.app_calendar = Some("timeparagon".into());
	let start = calendar::at_clock(chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(), 9, 0);
	let end = start + chrono::Duration::minutes(45);
	state.calendar.begin_draft("primary".into(), start, end);
	let _ = update(
		&mut state,
		Message::EventDraft(EventDraftMessage::EventDraftTitleChanged(
			"Deep work".into(),
		)),
	);
	let _ = update(
		&mut state,
		Message::EventDraft(EventDraftMessage::EventDraftKindSelected(
			calendar::DraftKind::TimeBlock,
		)),
	);
	let draft = state.calendar.draft.as_ref().unwrap();
	assert_eq!(draft.kind, calendar::DraftKind::TimeBlock);
	assert_eq!(draft.calendar_id, "timeparagon");
	assert!(draft.saving, "the block write is already in flight");
	assert!(
		!draft.editor_open,
		"a time block is planned in the panel, not the event form"
	);
}

#[test]
fn a_saved_time_block_opens_the_block_panel() {
	let mut state = State::blank();
	state.config.calendar.app_calendar = Some("timeparagon".into());
	let start = calendar::at_clock(chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(), 9, 0);
	let end = start + chrono::Duration::minutes(45);
	let mut draft = calendar::Draft::new("timeparagon".into(), start, end);
	draft.kind = calendar::DraftKind::TimeBlock;
	draft.saving = true;
	state.calendar.draft = Some(draft);
	let block = calendar::Event {
		id: "block-1".into(),
		calendar_id: "timeparagon".into(),
		title: "Deep work".into(),
		start,
		end,
		all_day: false,
		color: calendar::colors::TIME_BLOCK,
		completed: false,
		details: calendar::Details::default(),
	};
	let task = update(
		&mut state,
		Message::EventDraft(EventDraftMessage::EventDraftSaved(Ok(block))),
	);
	assert!(state.calendar.draft.is_none());
	assert!(
		messages(task).iter().any(|message| matches!(
			message,
			Message::TimeBlocking(crate::timeblocking::Action::Plan(id)) if id == "block-1"
		)),
		"a new block is planned in the right-hand panel"
	);
}

/// The messages a task resolves to, for asserting on what an update queued.
fn messages(task: Task<Message>) -> Vec<Message> {
	use iced_test::runtime::{Action, task};
	block_on(async {
		let Some(mut stream) = task::into_stream(task) else {
			return Vec::new();
		};
		let mut out = Vec::new();
		while let Some(action) = stream.next().await {
			if let Action::Output(value) = action {
				out.push(value);
			}
		}
		out
	})
}

#[test]
fn external_config_updates_live_state_and_cancels_pending_save() {
	let mut state = State::blank();
	state.save_queued = true;
	let mut external = state.config.clone();
	external.notepad.text = "edited outside the app".into();
	external.appearance.theme = "Ocean".into();
	external.calendar.day_end_hour = 3;
	let _ = update(
		&mut state,
		Message::System(SystemMessage::ConfigReloaded(external.clone())),
	);
	assert_eq!(state.config, external);
	assert_eq!(state.notepad.text(), "edited outside the app");
	assert!(!state.save_queued);
}

#[test]
fn own_save_echo_does_not_undo_newer_unsaved_edits() {
	let mut state = State::blank();
	let saved = state.disk_config.clone();
	state.config.notepad.text = "new unsaved text".into();
	state.save_queued = true;
	let _ = update(
		&mut state,
		Message::System(SystemMessage::ConfigReloaded(saved)),
	);
	assert_eq!(state.config.notepad.text, "new unsaved text");
	assert!(state.save_queued);
}

#[test]
fn the_window_icon_decodes() {
	assert!(window_icon().is_some(), "the embedded icon should decode");
}

#[test]
fn the_flash_fades_in_and_out_within_a_pulse() {
	let flash = flash(config::Flash {
		pulses: 1,
		pulse_ms: 1_000,
		peak_opacity: 1.0,
		..config::Flash::default()
	});

	let at = |millis| flash.alpha(flash.started + Duration::from_millis(millis));

	assert_eq!(at(0), Some(0.0));
	assert!(at(500).unwrap() > 0.99, "should peak mid-pulse");
	assert!(at(950).unwrap() < 0.05, "should fade back out");
}

#[test]
fn the_flash_reports_when_it_is_over() {
	let flash = flash(config::Flash {
		pulses: 2,
		pulse_ms: 500,
		..config::Flash::default()
	});

	let total = flash.settings.total_duration();
	assert_eq!(total, Duration::from_millis(1_000));

	assert!(
		flash
			.alpha(flash.started + total - Duration::from_millis(1))
			.is_some()
	);
	assert!(flash.alpha(flash.started + total).is_none());
}

#[test]
fn a_flash_configured_faster_than_the_safety_cap_is_slowed_down() {
	let flash = flash(config::Flash {
		pulses: 2,
		pulse_ms: 20,
		..config::Flash::default()
	});

	// The config asked for 50 pulses a second; it gets three.
	assert_eq!(
		flash.settings.total_duration(),
		config::Flash::MIN_PULSE * 2
	);
}

#[test]
fn the_flash_never_exceeds_the_configured_peak() {
	let flash = flash(config::Flash {
		pulses: 3,
		peak_opacity: 0.35,
		..config::Flash::default()
	});

	for millis in (0..3_000).step_by(7) {
		if let Some(alpha) = flash.alpha(flash.started + Duration::from_millis(millis)) {
			assert!((0.0..=0.35).contains(&alpha), "alpha {alpha} out of range");
		}
	}
}

#[test]
fn a_zero_opacity_flash_draws_nothing() {
	let flash = flash(config::Flash {
		peak_opacity: 0.0,
		..config::Flash::default()
	});

	assert_eq!(
		flash.alpha(flash.started + Duration::from_millis(100)),
		Some(0.0)
	);
}

#[test]
fn showing_tasks_is_remembered() {
	let mut state = State::blank();
	assert!(state.config.calendar.show_tasks);

	let _ = update(
		&mut state,
		Message::Settings(SettingsMessage::ShowTasksToggled(false)),
	);

	assert!(!state.config.calendar.show_tasks);
	assert!(state.save_queued);
}

#[test]
fn the_notepad_keeps_what_was_written() {
	let mut state = State::blank();

	for letter in "milk".chars() {
		let _ = update(
			&mut state,
			Message::Notepad(NotepadMessage::NotepadEdited(text_editor::Action::Edit(
				text_editor::Edit::Insert(letter),
			))),
		);
	}

	assert_eq!(state.config.notepad.text, "milk");
	assert!(state.save_queued);
}

#[test]
fn the_notepad_opens_on_what_was_written_last_time() {
	let mut config = Config::default();
	config.notepad.text = "remember the milk".to_owned();

	let state = State::new(config, Catalog::built_in());

	assert_eq!(state.notepad.text(), "remember the milk");
}

#[test]
fn a_bad_or_repeated_shortcut_is_refused_without_touching_the_desktop() {
	let mut state = State::blank();
	let action = panel::Action::ToggleVisibility;

	for input in ["nonsense", "super + u"] {
		let _ = update(
			&mut state,
			Message::Settings(SettingsMessage::ShortcutInputChanged(action, input.into())),
		);
		let _ = update(
			&mut state,
			Message::Settings(SettingsMessage::ShortcutAdded(action)),
		);

		let editor = &state.shortcut_editors[action.index()];
		assert!(!editor.busy, "{input:?} was sent to the desktop");
		assert!(matches!(editor.result, Some(Err(_))), "{input:?}");
	}
	assert_eq!(state.config.panel.toggle_visibility, ["Meta+U"]);
}

#[test]
fn an_action_keeps_every_shortcut_that_was_applied() {
	let mut state = State::blank();
	let action = panel::Action::ToggleVisibility;
	let both = vec!["Meta+J".to_owned(), "Meta+F".to_owned()];

	let _ = update(
		&mut state,
		Message::Settings(SettingsMessage::ShortcutsApplied(action, Ok(both.clone()))),
	);

	assert_eq!(state.config.panel.toggle_visibility, both);
	assert_eq!(
		state.config.panel.toggle_shape,
		panel::PanelConfig::default().toggle_shape
	);
	assert!(state.save_queued);

	let _ = update(
		&mut state,
		Message::Settings(SettingsMessage::ShortcutsApplied(
			action,
			Err("taken".into()),
		)),
	);
	assert_eq!(state.config.panel.toggle_visibility, both);
}

#[test]
fn the_notepad_text_size_is_remembered() {
	let mut state = State::blank();
	assert_eq!(state.config.notepad.font_size, 100);

	let _ = update(
		&mut state,
		Message::Notepad(NotepadMessage::NotepadFontSizeChanged(10)),
	);
	assert_eq!(state.config.notepad.font_size, 110);

	let _ = update(
		&mut state,
		Message::Notepad(NotepadMessage::NotepadFontSizeChanged(-20)),
	);
	assert_eq!(state.config.notepad.font_size, 90);
	assert!(state.save_queued);
}

#[test]
fn the_notepad_text_size_stops_at_the_ends_of_its_range() {
	let mut state = State::blank();

	for _ in 0..40 {
		let _ = update(
			&mut state,
			Message::Notepad(NotepadMessage::NotepadFontSizeChanged(10)),
		);
	}
	assert_eq!(
		state.config.notepad.font_size,
		config::Notepad::MAX_FONT_SIZE
	);

	for _ in 0..40 {
		let _ = update(
			&mut state,
			Message::Notepad(NotepadMessage::NotepadFontSizeChanged(-10)),
		);
	}
	assert_eq!(
		state.config.notepad.font_size,
		config::Notepad::MIN_FONT_SIZE
	);
}

#[test]
fn the_notepad_is_a_tab_of_its_own() {
	let mut state = State::blank();

	let _ = update(
		&mut state,
		Message::Navigation(NavigationMessage::ScreenSelected(Screen::Notepad)),
	);
	assert_eq!(state.screen, Screen::Notepad);

	let _ = update(
		&mut state,
		Message::Navigation(NavigationMessage::ScreenSelected(Screen::Calendar)),
	);
	assert_eq!(state.screen, Screen::Calendar);
}

#[test]
fn the_clock_shortcuts_are_left_off_while_writing() {
	let mut state = State::blank();
	state.screen = Screen::Notepad;

	let space = key_message(
		&mut state,
		Key::Named(keyboard::key::Named::Space),
		keyboard::Modifiers::default(),
	);
	let reset = key_message(
		&mut state,
		Key::Character("r".into()),
		keyboard::Modifiers::default(),
	);

	assert!(space.is_none(), "a space would start the clock mid-word");
	assert!(reset.is_none(), "an r would reset the clock mid-word");

	// Escape still leaves the page.
	let escape = key_message(
		&mut state,
		Key::Named(keyboard::key::Named::Escape),
		keyboard::Modifiers::default(),
	);
	assert!(
		matches!(
			escape,
			Some(Message::Navigation(NavigationMessage::ScreenSelected(
				Screen::Calendar
			)))
		),
		"escape should leave the notepad"
	);
}

#[test]
fn a_space_on_the_calendar_only_silences_a_ringing_alarm() {
	let mut state = State::blank();
	let space = |state: &mut State| {
		key_message(
			state,
			Key::Named(keyboard::key::Named::Space),
			keyboard::Modifiers::default(),
		)
	};

	assert!(
		space(&mut state).is_none(),
		"there is no clock on the calendar for a space to start"
	);

	state.alarm_active = true;

	assert!(matches!(
		space(&mut state),
		Some(Message::System(SystemMessage::Dismiss))
	));
}

#[test]
fn changing_the_end_of_day_moves_the_day_in_view() {
	let mut state = State::blank();
	assert_eq!(state.config.calendar.day_end_hour, 0);

	let _ = update(
		&mut state,
		Message::Settings(SettingsMessage::CalendarDayEndChanged(3)),
	);

	assert_eq!(state.config.calendar.day_end_hour, 3);
	assert!(state.save_queued);
	// The day in view is recomputed from the new boundary, not left on the
	// date the old one produced.
	assert_eq!(state.calendar.day, calendar::day_at(crate::clock::now(), 3));
}

#[test]
fn an_out_of_range_end_of_day_is_clamped() {
	let mut state = State::blank();

	let _ = update(
		&mut state,
		Message::Settings(SettingsMessage::CalendarDayEndChanged(99)),
	);

	assert_eq!(
		state.config.calendar.day_end_hour,
		config::CalendarSettings::MAX_DAY_END_HOUR
	);
}

#[test]
fn picking_a_task_color_recolors_the_list_and_its_events() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());

	// The sample uses a shortened id; a real task list is `tasks:<list>`.
	state.calendar.calendars[2].id = "tasks:list".into();
	let deep = state
		.calendar
		.events
		.iter_mut()
		.find(|event| event.id == "deep")
		.unwrap();
	deep.calendar_id = "tasks:list".into();

	let red = iced::Color::from_rgb8(0xDC, 0x21, 0x27);
	let _ = update(
		&mut state,
		Message::Settings(SettingsMessage::TaskColorPicked(
			"tasks:list".into(),
			Some(red),
		)),
	);

	assert_eq!(state.config.calendar.task_color("tasks:list"), Some(red));
	assert!(state.save_queued);
	assert_eq!(state.calendar.calendars[2].color, red);
	assert_eq!(
		state
			.calendar
			.event("deep")
			.unwrap()
			.display_color(&state.config.calendar, &state.calendar.calendars),
		red,
		"the task's own chip should follow the list color"
	);

	// Resetting has to repaint the chip, not just forget the override.
	let _ = update(
		&mut state,
		Message::Settings(SettingsMessage::TaskColorPicked("tasks:list".into(), None)),
	);
	assert_eq!(state.config.calendar.task_color("tasks:list"), None);
	assert_eq!(
		state
			.calendar
			.event("deep")
			.unwrap()
			.display_color(&state.config.calendar, &state.calendar.calendars),
		calendar::colors::TASK
	);
}

#[test]
fn the_task_color_editor_opens_and_closes() {
	let mut state = State::blank();

	let _ = update(
		&mut state,
		Message::Settings(SettingsMessage::SourceColorEditorToggled(
			"tasks:list".into(),
		)),
	);
	assert_eq!(state.calendar.color_editor.as_deref(), Some("tasks:list"));

	let _ = update(
		&mut state,
		Message::Settings(SettingsMessage::SourceColorEditorToggled(
			"tasks:list".into(),
		)),
	);
	assert_eq!(state.calendar.color_editor, None);
}

#[test]
fn the_window_opens_at_two_thirds_of_the_monitor() {
	assert_eq!(launch_width(1920.0), 1280.0);
	assert_eq!(launch_width(3840.0), 2560.0);
	// A tiny monitor must not fold the layout below its minimum.
	assert_eq!(launch_width(120.0), MIN_SIZE.width);
}

#[test]
fn the_panel_window_is_pinned_to_its_width() {
	let panel = panel::PanelConfig {
		width: 200,
		..Default::default()
	};
	let settings = sidebar_settings(&panel, &config::ViewSettings::default());

	// The width is fixed from both sides, so the compositor cannot stretch
	// the strip, but the height is free to fill the screen.
	assert_eq!(settings.min_size.unwrap().width, 216.0);
	assert_eq!(settings.max_size.unwrap().width, 216.0);
	assert!(settings.min_size.unwrap().height < settings.max_size.unwrap().height);
	assert!(!settings.decorations);
	assert!(settings.transparent);
	assert_eq!(settings.level, window::Level::AlwaysOnTop);
	assert_eq!(settings.platform_specific.application_id, APPLICATION_ID);
}

#[test]
fn range_width_fits_beside_the_day_panel() {
	let mut state = State::blank();
	state.config.calendar.weekly_days = 7;
	state.config.calendar.weekly_day_width = 100;
	assert_eq!(state.weekly_window_width(), 716.0);
	state.config.calendar.weekly_days = 8;
	state.config.calendar.weekly_day_width = 220;
	assert_eq!(state.weekly_window_width(), 1776.0);
	state.sidebar = Some(Sidebar {
		width: 200.0,
		side: panel::Side::Left,
	});
	assert_eq!(state.weekly_window_width(), 1792.0);
	state.monitor_size = Some(Size::new(1200.0, 900.0));
	assert_eq!(state.weekly_window_width(), 984.0);
	state.monitor_size = Some(Size::new(f32::NAN, 900.0));
	assert_eq!(state.weekly_window_width(), 1792.0);
	state.sidebar = None;
	state.monitor_size = None;
	state.config.window.width = 3000.0;
	assert_eq!(state.weekly_window_width(), 3000.0);
}

#[test]
fn explicit_range_widths_are_independent_of_day_width() {
	let mut state = State::blank();
	state.config.views.calendar_width = Some(1500.0);
	state.config.views.calendar_week_width = Some(900.0);
	state.config.views.calendar_month_width = Some(1100.0);
	state.view = CalendarView::Week;
	assert_eq!(state.weekly_window_width(), 900.0);
	state.view = CalendarView::Month;
	assert_eq!(state.weekly_window_width(), 1100.0);
	assert_eq!(state.calendar_width(), 1500.0);
}

#[test]
fn day_width_changes_keep_fixed_panel_constraints_with_a_range_open() {
	let mut state = State::blank();
	state.main_window = Some(window::Id::unique());
	state.sidebar = Some(Sidebar {
		width: 200.0,
		side: panel::Side::Left,
	});
	state.main_size = Size::new(216.0, 900.0);
	let _ = geometry_actions(update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarModeSelected(true)),
	));
	let actions = geometry_actions(update(
		&mut state,
		Message::Settings(SettingsMessage::CalendarWidthChanged(300.0)),
	));
	assert!(matches!(actions.as_slice(), [GeometryAction::Max(None),
        GeometryAction::Min(Some(min)), GeometryAction::Resize(size), GeometryAction::Max(Some(max))]
        if min.width == 316.0 && size.width == 316.0 && max.width == 316.0));
}

#[test]
fn plan_day_floats_and_returns_to_do_without_resizing_the_panel() {
	let mut state = State::blank();
	let main = window::Id::unique();
	state.main_window = Some(main);
	state.main_size = Size::new(328.0, 900.0);
	let actions = geometry_actions(update(
		&mut state,
		Message::Navigation(NavigationMessage::PlanDaySelected),
	));
	assert!(
		matches!(actions.as_slice(), [GeometryAction::Open(settings)]
        if !settings.decorations && settings.size.width == state.calendar_width())
	);
	let floating = state.editor_window.expect("day calendar window");
	assert_ne!(floating, main);
	assert_eq!(state.main_size, Size::new(328.0, 900.0));
	let _ = update(
		&mut state,
		Message::Navigation(NavigationMessage::PlanTabSelected(PlanTab::Inbox)),
	);
	assert_eq!(state.editor_window, Some(floating));
	let _ = update(
		&mut state,
		Message::Navigation(NavigationMessage::ScreenSelected(Screen::Calendar)),
	);
	assert!(!state.day_plan_active());
	assert!(state.editor_window.is_none());
	assert_eq!(state.main_size, Size::new(328.0, 900.0));
}

#[test]
fn range_views_float_without_resizing_the_day_panel() {
	for monthly in [false, true] {
		let mut state = State::blank();
		let main = window::Id::unique();
		state.main_window = Some(main);
		state.sidebar = Some(Sidebar {
			width: 200.0,
			side: panel::Side::Left,
		});
		state.main_size = Size::new(216.0, 900.0);
		let day = state.calendar.day;
		let actions = geometry_actions(update(
			&mut state,
			if monthly {
				Message::Calendar(CalendarMessage::CalendarMonthViewSelected)
			} else {
				Message::Calendar(CalendarMessage::CalendarModeSelected(true))
			},
		));
		assert!(
			matches!(actions.as_slice(), [GeometryAction::Open(settings)]
            if !settings.decorations && settings.size.width > 216.0)
		);
		let floating = state.editor_window.unwrap();
		assert_ne!(floating, main);
		assert_eq!(state.main_size, Size::new(216.0, 900.0));
		assert_eq!(state.calendar.day, day);
		let saved_width = state.config.window.width;
		let _ = update(
			&mut state,
			Message::System(SystemMessage::WindowResized(
				floating,
				Size::new(1000.0, 800.0),
			)),
		);
		assert_eq!(state.config.window.width, saved_width);
		let actions = geometry_actions(update(
			&mut state,
			Message::Calendar(CalendarMessage::CalendarModeSelected(false)),
		));
		assert!(actions.is_empty());
		assert!(state.editor_window.is_none());
		assert_eq!(state.main_window, Some(main));
		assert_eq!(state.main_size, Size::new(216.0, 900.0));
	}
}

#[test]
fn range_surface_survives_escape_and_hides_with_the_panel() {
	let mut state = State::blank();
	state.main_window = Some(window::Id::unique());
	let _ = geometry_actions(update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarModeSelected(true)),
	));
	let floating = state.editor_window.unwrap();
	let _ = geometry_actions(update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarMonthViewSelected),
	));
	assert_eq!(state.editor_window, Some(floating));
	assert!(state.monthly());
	let escape = key_message(
		&mut state,
		Key::Named(keyboard::key::Named::Escape),
		keyboard::Modifiers::default(),
	);
	assert!(escape.is_none());
	assert!(state.range_view_active());
	assert_eq!(state.editor_window, Some(floating));
	state.panel_expected_close = true;
	let _ = update(&mut state, Message::Calendar(CalendarMessage::CalendarTick));
	assert!(state.editor_window.is_none());
}

#[test]
fn overview_refreshes_and_follows_today_on_other_panel_tabs() {
	for screen in [
		Screen::Inbox,
		Screen::Overdue,
		Screen::Life,
		Screen::Notepad,
		Screen::Travel,
		Screen::Habits,
		Screen::Notes,
		Screen::BrainDump,
		Screen::Settings,
	] {
		for monthly in [false, true] {
			let mut state = State::blank();
			state.screen = screen;
			state.view = if monthly {
				CalendarView::Month
			} else {
				CalendarView::Week
			};
			let today =
				calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
			state.week_anchor = today - chrono::Duration::days(1);
			state.calendar.follow_today = true;
			let _ = update(&mut state, Message::Calendar(CalendarMessage::CalendarTick));
			assert_eq!(state.week_anchor, today);
			assert_eq!(state.screen, screen);
			state.calendar.connection = Connection::SignedIn;
			let _ = update(
				&mut state,
				Message::Calendar(CalendarMessage::CalendarSyncRequested),
			);
			assert_eq!(state.calendar.sync, SyncState::Syncing);
			assert!(state.range_view_active());
		}
	}
}

#[test]
fn closing_the_range_surface_keeps_the_overview_toggle_enabled() {
	let mut state = State::blank();
	let main = window::Id::unique();
	state.main_window = Some(main);
	let _ = geometry_actions(update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarMonthViewSelected),
	));
	let floating = state.editor_window.unwrap();
	let _ = update(
		&mut state,
		Message::System(SystemMessage::WindowClosed(floating)),
	);
	assert!(state.range_view_active());
	assert!(state.monthly());
	assert!(state.editor_window.is_some());
	assert_ne!(state.editor_window, Some(floating));
	assert_eq!(state.main_window, Some(main));
}

#[test]
fn the_panel_does_not_widen_to_the_default_fraction() {
	let mut state = State::blank();
	let id = window::Id::unique();
	state.main_window = Some(id);
	state.sidebar = Some(Sidebar {
		width: 200.0,
		side: panel::Side::Left,
	});

	let before = state.config.window.width;
	let _ = update(
		&mut state,
		Message::System(SystemMessage::MonitorSized(
			id,
			Some(Size::new(1920.0, 1080.0)),
		)),
	);

	// The main window would have adopted `launch_width` (1280px) here; the
	// panel keeps the width it was told to.
	assert_eq!(state.config.window.width, before);
}

#[test]
fn the_panel_does_not_write_its_geometry_over_the_window() {
	let mut state = State::blank();
	let id = window::Id::unique();
	state.main_window = Some(id);
	state.sidebar = Some(Sidebar {
		width: 200.0,
		side: panel::Side::Left,
	});

	let before = state.config.window.clone();
	let _ = update(
		&mut state,
		Message::System(SystemMessage::WindowResized(id, Size::new(200.0, 1080.0))),
	);

	assert_eq!(
		state.config.window, before,
		"the panel must leave the saved window alone"
	);
}

#[test]
fn a_main_window_still_remembers_its_own_size() {
	let mut state = State::blank();
	let id = window::Id::unique();
	state.main_window = Some(id);

	let _ = update(
		&mut state,
		Message::System(SystemMessage::WindowResized(id, Size::new(640.0, 480.0))),
	);

	assert_eq!(state.config.window.width, 640.0);
	assert_eq!(state.config.window.height, 480.0);
}

#[test]
fn opening_an_event_in_the_top_half_puts_the_details_below() {
	let mut state = State::blank();
	ui::calendar::record_pointer(20.0, 500.0);

	let _ = update(
		&mut state,
		Message::Event(EventMessage::EventOpened("standup".into())),
	);

	assert_eq!(state.calendar.open_event.as_deref(), Some("standup"));
	assert!(state.calendar.details_below);
}

#[test]
fn opening_an_event_seeds_the_inline_editor() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());

	let _ = update(
		&mut state,
		Message::Event(EventMessage::EventOpened("standup".into())),
	);

	let draft = state
		.calendar
		.edit_draft
		.as_ref()
		.expect("the details pane edits the event in place");
	assert_eq!(draft.id, "standup");
	assert_eq!(draft.title, "Standup");
	assert_eq!(draft.rule.repeat, crate::recurrence::RepeatMode::Never);
}

#[test]
fn event_project_changes_are_staged_and_cancelled_without_changing_the_event() {
	use calendar::projects::Action;
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	let ids: Vec<_> = state
		.config
		.projects
		.items
		.iter()
		.take(2)
		.map(|p| p.id)
		.collect();
	let _ = update(
		&mut state,
		Message::Event(EventMessage::EventOpened("standup".into())),
	);
	for id in [ids[0], ids[1], ids[0], u64::MAX] {
		let _ = update(
			&mut state,
			Message::EventEdit(EventEditMessage::EventEditProjects(Action::Add(id))),
		);
	}
	let mut expected = ids.clone();
	expected.sort_unstable();
	assert_eq!(
		state.calendar.edit_draft.as_ref().unwrap().projects.ids,
		expected
	);
	assert!(
		state
			.calendar
			.event("standup")
			.unwrap()
			.details
			.projects
			.is_empty()
	);
	let _ = update(
		&mut state,
		Message::EventEdit(EventEditMessage::EventEditProjects(Action::Remove(ids[0]))),
	);
	assert_eq!(
		state.calendar.edit_draft.as_ref().unwrap().projects.ids,
		vec![ids[1]]
	);
	let _ = update(
		&mut state,
		Message::EventEdit(EventEditMessage::EventEditCancelled),
	);
	assert!(state.calendar.edit_draft.is_none());
	assert!(
		state
			.calendar
			.event("standup")
			.unwrap()
			.details
			.projects
			.is_empty()
	);

	let event = state.calendar.event("standup").unwrap();
	state.calendar.draft = Some(calendar::Draft::new(
		"primary".into(),
		event.start,
		event.end,
	));
	let _ = update(
		&mut state,
		Message::EventDraft(EventDraftMessage::EventDraftProjects(Action::Add(ids[0]))),
	);
	let _ = update(
		&mut state,
		Message::EventDraft(EventDraftMessage::EventDraftProjects(Action::Add(ids[1]))),
	);
	assert_eq!(state.calendar.draft.as_ref().unwrap().projects.ids.len(), 2);
}

#[test]
fn opening_an_event_in_the_bottom_half_keeps_the_details_on_top() {
	let mut state = State::blank();
	ui::calendar::record_pointer(480.0, 500.0);

	let _ = update(
		&mut state,
		Message::Event(EventMessage::EventOpened("lunch".into())),
	);

	assert_eq!(state.calendar.open_event.as_deref(), Some("lunch"));
	assert!(!state.calendar.details_below);
}

#[test]
fn editors_leave_the_main_window_geometry_unchanged_on_every_destination() {
	for screen in [
		Screen::Calendar,
		Screen::Inbox,
		Screen::Overdue,
		Screen::Triaged,
		Screen::Backlog,
		Screen::Habits,
	] {
		for sidebar in [false, true] {
			for block in [false, true] {
				let mut state = State::blank();
				state.screen = screen;
				state.sidebar = sidebar.then_some(Sidebar {
					width: 200.0,
					side: panel::Side::Left,
				});
				let main = window::Id::unique();
				state.main_window = Some(main);
				state.main_size = Size::new(216.0, 900.0);
				state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
				let task = if block {
					assert!(
						geometry_actions(crate::timeblocking::open(&mut state, "standup".into()))
							.is_empty()
					);
					update(&mut state, Message::Calendar(CalendarMessage::CalendarTick))
				} else {
					update(
						&mut state,
						Message::Event(EventMessage::EventOpened("standup".into())),
					)
				};
				let actions = geometry_actions(task);
				assert!(
					matches!(actions.as_slice(), [GeometryAction::Open(settings)]
            if settings.level == window::Level::AlwaysOnTop && !settings.decorations)
				);
				let editor = state.editor_window.expect("an independent editor window");
				assert_ne!(editor, main);
				assert_eq!(state.main_window, Some(main));
				assert_eq!(state.main_size, Size::new(216.0, 900.0));
				let escape = key_message(
					&mut state,
					Key::Named(keyboard::key::Named::Escape),
					keyboard::Modifiers::default(),
				)
				.unwrap();
				let actions = geometry_actions(update(&mut state, escape));
				if state.day_plan_active() {
					assert!(matches!(actions.as_slice(), [GeometryAction::Resize(size)]
                        if size.width == state.calendar_width()));
					assert_eq!(state.editor_window, Some(editor));
				} else {
					assert!(actions.is_empty());
					assert!(state.editor_window.is_none());
				}
				assert!(state.calendar.open_event.is_none());
				assert!(state.timeblocking.block.is_none());
				// A delayed opened notification cannot replace the main window.
				let _ = update(
					&mut state,
					Message::System(SystemMessage::EditorWindowOpened(editor)),
				);
				assert_eq!(state.main_window, Some(main));
			}
		}
	}
}

#[test]
fn escape_closes_an_open_contact_before_leaving_people() {
	let mut state = State::blank();
	state.screen = Screen::People;
	let escape = key_message(
		&mut state,
		Key::Named(keyboard::key::Named::Escape),
		keyboard::Modifiers::default(),
	);
	assert!(matches!(
		escape,
		Some(Message::Navigation(NavigationMessage::ScreenSelected(
			Screen::Calendar
		)))
	));
}

#[test]
fn escape_closes_the_event_details_pane() {
	let mut state = State::blank();
	state.calendar.open_event = Some("standup".into());

	let escape = key_message(
		&mut state,
		Key::Named(keyboard::key::Named::Escape),
		keyboard::Modifiers::default(),
	);

	assert!(
		matches!(escape, Some(Message::Event(EventMessage::EventClosed))),
		"escape should close the details pane, got {escape:?}"
	);
}

#[test]
fn populated_day_navigation_retains_cached_events_until_scheduled_sync() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	state.calendar.connection = Connection::SignedIn;
	state.calendar.last_sync = Some(crate::clock::now() - chrono::Duration::hours(3));
	let day = state.calendar.day;
	let count = state.calendar.events.len();
	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarDaySelected(day)),
	);
	assert_eq!(state.calendar.sync, SyncState::Idle);
	assert_eq!(state.calendar.events.len(), count);
	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarAutoSyncRequested),
	);
	assert_eq!(state.calendar.sync, SyncState::Syncing);
}

#[test]
fn switching_to_uncached_views_fetches_despite_a_recent_sync() {
	for message in [
		Message::Calendar(CalendarMessage::CalendarModeSelected(true)),
		Message::Calendar(CalendarMessage::CalendarMonthViewSelected),
		Message::Calendar(CalendarMessage::CalendarDayShift(1)),
	] {
		let mut state = State::blank();
		state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
		state.calendar.connection = Connection::SignedIn;
		state.calendar.last_sync = Some(crate::clock::now());
		let _ = update(&mut state, message);
		assert_eq!(state.calendar.sync, SyncState::Syncing);
	}
}

#[test]
fn empty_cached_days_are_fetched_again_on_navigation() {
	let mut state = State::blank();
	let mut snapshot = calendar::sample_snapshot();
	snapshot.events.clear();
	let day = snapshot.day;
	state.calendar = calendar::Store::with_snapshot(snapshot);
	state.calendar.connection = Connection::SignedIn;
	state.calendar.last_sync = Some(crate::clock::now());
	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarDaySelected(day)),
	);
	assert_eq!(state.calendar.sync, SyncState::Syncing);
}

#[test]
fn navigation_during_a_fetch_loads_the_new_view_after_completion() {
	let mut state = State::blank();
	let snapshot = calendar::sample_snapshot();
	state.calendar = calendar::Store::with_snapshot(snapshot.clone());
	state.calendar.connection = Connection::SignedIn;
	state.calendar.sync = SyncState::Syncing;
	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarModeSelected(true)),
	);
	assert!(!state.calendar_fetch_pending.is_empty());
	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarSynced(Ok(snapshot))),
	);
	assert_eq!(state.calendar.sync, SyncState::Syncing);
	assert!(state.calendar_fetch_pending.is_empty());
}

#[test]
fn an_empty_range_result_does_not_fetch_forever() {
	let mut state = State::blank();
	let mut snapshot = calendar::sample_snapshot();
	state.calendar = calendar::Store::with_snapshot(snapshot.clone());
	state.calendar.connection = Connection::SignedIn;
	state.calendar.sync = SyncState::Syncing;
	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarModeSelected(true)),
	);
	let dates = state.calendar_dates();
	snapshot.events.clear();
	let revision = state.timeblocking.revision;
	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarRangeSynced(
			revision,
			dates,
			Ok(snapshot),
		)),
	);
	assert_eq!(state.calendar.sync, SyncState::Idle);
	assert!(state.calendar_fetch_pending.is_empty());
}

#[test]
fn the_footer_returns_to_logs_as_soon_as_a_day_sync_result_arrives() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	state.calendar.connection = Connection::SignedIn;
	state.calendar.last_sync = Some(crate::clock::now());
	state.calendar.sync = SyncState::Syncing;
	let _ = update(&mut state, Message::System(SystemMessage::ActivityTick));
	assert_eq!(state.activity.label(), "Loading from cloud");

	// The result has landed: the data is in, so the label must say Logs in
	// this very update, not one deferred message later.
	let revision = state.timeblocking.revision;
	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarSyncedAt(
			revision,
			Ok(calendar::sample_snapshot()),
		)),
	);
	assert_eq!(state.calendar.sync, SyncState::Idle);
	assert_eq!(state.activity.label(), crate::activity::IDLE_LABEL);
}

#[test]
fn a_queued_fetch_does_not_keep_the_footer_on_loading() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	state.calendar.connection = Connection::SignedIn;
	state.calendar.sync = SyncState::Idle;
	// Days waiting for a free moment are not a request in flight.
	state.calendar_fetch_pending = vec![state.calendar.day];

	assert!(
		state.current_activities().is_empty(),
		"a queued fetch is not loading yet"
	);
	state
		.activity
		.observe(state.current_activities(), Instant::now());
	assert_eq!(state.activity.label(), crate::activity::IDLE_LABEL);
}

#[test]
fn calendar_sync_refreshes_habit_links_for_a_new_series_without_opening_habits() {
	use crate::habits::{Action, Habit, Mark};

	for range in [false, true] {
		let mut state = State::blank();
		state.screen = Screen::Calendar;
		let mut snapshot = calendar::sample_snapshot();
		snapshot.calendars.clear();
		let day = snapshot.day;
		let yesterday = day.pred_opt().unwrap();
		let mut habit = Habit::new("breakfast", "Breakfast");
		habit.marks.insert(yesterday, Mark::Yes);
		state.habits.library.habits.push(habit);
		let mut event = snapshot.events[0].clone();
		event.id = "new-breakfast-series_today".into();
		event.title = "Breakfast".into();
		event.details = calendar::Details {
			habit: true,
			recurring_event_id: Some("new-breakfast-series".into()),
			..Default::default()
		};
		snapshot.events = vec![event.clone()];
		state.calendar.day = day;
		assert_eq!(state.habits.habit_of(&event), None);

		let message = if range {
			CalendarMessage::CalendarRangeSynced(
				state.timeblocking.revision,
				vec![day, day.succ_opt().unwrap()],
				Ok(snapshot),
			)
		} else {
			CalendarMessage::CalendarSynced(Ok(snapshot))
		};
		let task = update(&mut state, Message::Calendar(message));
		let refreshes = messages(task)
			.iter()
			.filter(|message| matches!(message, Message::Habits(Action::Refresh)))
			.count();
		assert_eq!(refreshes, 1, "range sync: {range}");

		// Simulate the listing returned by the scheduled refresh, without
		// contacting Google or changing the screen.
		let _ = update(
			&mut state,
			Message::Habits(Action::Loaded(Ok(calendar::api::HabitListing {
				series: vec![calendar::api::HabitEvent {
					id: "new-breakfast-series".into(),
					calendar_id: event.calendar_id.clone(),
					title: event.title.clone(),
					notes: String::new(),
					start: event.start,
					end: event.end,
					recurrence: vec!["RRULE:FREQ=DAILY".into()],
					color_id: None,
					habit_id: Some("breakfast".into()),
				}],
				completions: vec![],
			}))),
		);
		let synced = state.calendar.event(&event.id).unwrap();
		assert_eq!(state.habits.habit_of(synced), Some("breakfast"));
		assert_eq!(state.habits.library.habits.len(), 1);
		assert_eq!(state.habits.library.habits[0].marks[&yesterday], Mark::Yes);
		assert_eq!(state.screen, Screen::Calendar);
	}
}

#[test]
fn a_recent_sync_prevents_an_automatic_fetch_but_manual_sync_still_works() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	let template = state.calendar.events[1].clone();
	for wake_date in [state.calendar.day, state.calendar.day.succ_opt().unwrap()] {
		let mut night = template.clone();
		night.id = format!("sleep-{wake_date}");
		night.details.sleep = Some(calendar::sleep::Record::planned(wake_date));
		state.calendar.events.push(night);
	}
	state.calendar.connection = Connection::SignedIn;
	state.calendar.last_sync = Some(crate::clock::now());
	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarAutoSyncRequested),
	);
	assert_eq!(state.calendar.sync, SyncState::Idle);
	let _ = update(
		&mut state,
		Message::Settings(SettingsMessage::CalendarSyncMinutesChanged(120)),
	);
	assert_eq!(state.calendar.sync, SyncState::Idle);
	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarSyncRequested),
	);
	assert_eq!(state.calendar.sync, SyncState::Syncing);
}

#[test]
fn stepping_the_day_moves_the_view_and_drops_the_old_events() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	let start = state.calendar.day;

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarDayShift(1)),
	);

	assert_eq!(state.calendar.day, start.succ_opt().unwrap());
	assert!(
		state.calendar.events.is_empty(),
		"the old day's events must not paint onto the new day"
	);
	assert!(!state.calendar.follow_today);
}

#[test]
fn stepping_back_to_a_day_paints_its_cached_events_at_once() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	let start = state.calendar.day;
	let cached = state.calendar.events.len();
	assert!(cached > 0);

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarDayShift(1)),
	);
	assert!(
		state.calendar.events.is_empty(),
		"a day not seen yet starts empty"
	);

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarDayShift(-1)),
	);

	assert_eq!(state.calendar.day, start);
	assert_eq!(
		state.calendar.events.len(),
		cached,
		"the day we came back to should paint from the cache"
	);
}

#[test]
fn selecting_today_makes_the_view_follow_it_again() {
	let mut state = State::blank();
	let today = calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
	state.calendar.day = today;
	state.calendar.follow_today = false;

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarDaySelected(today)),
	);

	assert!(state.calendar.follow_today);
}

#[test]
fn the_month_picker_opens_on_the_month_of_the_day_in_view() {
	let mut state = State::blank();
	state.calendar.day = chrono::NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarMonthToggled(MonthPicker::Panel)),
	);

	assert!(state.calendar.month_open);
	assert_eq!(
		state.calendar.month,
		chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
	);

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarMonthToggled(MonthPicker::Panel)),
	);
	assert!(!state.calendar.month_open);
}

#[test]
fn stepping_the_month_crosses_the_year_boundary() {
	let mut state = State::blank();
	state.calendar.month = chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarMonthShift(MonthPicker::Panel, 1)),
	);
	assert_eq!(
		state.calendar.month,
		chrono::NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
	);

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarMonthShift(MonthPicker::Panel, -2)),
	);
	assert_eq!(
		state.calendar.month,
		chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
	);
}

#[test]
fn picking_a_day_closes_the_month_picker_and_moves_the_view() {
	let mut state = State::blank();
	state.calendar.month_open = true;
	let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarMonthDayPicked(
			MonthPicker::Panel,
			day,
		)),
	);

	assert_eq!(state.calendar.day, day);
	assert!(!state.calendar.month_open);
}

#[test]
fn the_panel_and_overview_month_pickers_open_independently() {
	let mut state = State::blank();
	state.view = CalendarView::Week;
	let toggle = |picker| Message::Calendar(CalendarMessage::CalendarMonthToggled(picker));

	let _ = update(&mut state, toggle(MonthPicker::Panel));
	assert!(state.calendar.month_open);
	assert!(!state.overview_month_open);

	let _ = update(&mut state, toggle(MonthPicker::Overview));
	assert!(state.calendar.month_open);
	assert!(state.overview_month_open);

	let _ = update(&mut state, toggle(MonthPicker::Panel));
	assert!(!state.calendar.month_open);
	assert!(state.overview_month_open);
}

#[test]
fn the_panel_and_overview_month_pickers_step_independently() {
	let mut state = State::blank();
	state.view = CalendarView::Week;
	let panel = state.calendar.month;
	let overview = state.overview_month;

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarMonthShift(
			MonthPicker::Overview,
			2,
		)),
	);
	assert_eq!(state.calendar.month, panel);
	assert_ne!(state.overview_month, overview);
}

#[test]
fn a_day_picked_in_one_month_picker_leaves_the_other_side_alone() {
	let mut state = State::blank();
	state.view = CalendarView::Week;
	state.calendar.month_open = true;
	state.overview_month_open = true;
	let anchor = state.week_anchor;
	let far = |days| anchor + chrono::Duration::days(days);

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarMonthDayPicked(
			MonthPicker::Panel,
			far(60),
		)),
	);
	assert_eq!(state.calendar.day, far(60));
	assert_eq!(state.week_anchor, anchor);
	assert!(!state.calendar.month_open);
	assert!(state.overview_month_open);

	state.calendar.month_open = true;
	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarMonthDayPicked(
			MonthPicker::Overview,
			far(90),
		)),
	);
	assert_eq!(state.week_anchor, far(90));
	assert_eq!(state.calendar.day, far(60));
	assert!(!state.overview_month_open);
	assert!(state.calendar.month_open);
}

#[test]
fn a_panel_day_outside_the_range_is_fetched_on_its_own() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	state.calendar.connection = Connection::SignedIn;
	state.view = CalendarView::Week;
	let day = state.week_anchor + chrono::Duration::days(60);

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarMonthDayPicked(
			MonthPicker::Panel,
			day,
		)),
	);

	assert_eq!(state.calendar.sync, SyncState::Syncing);
	assert!(!state.panel_day_wanted);
}

#[test]
fn a_panel_day_picked_during_a_sync_is_fetched_when_it_finishes() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	state.calendar.connection = Connection::SignedIn;
	state.calendar.sync = SyncState::Syncing;
	state.view = CalendarView::Week;
	let day = state.week_anchor + chrono::Duration::days(60);

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarMonthDayPicked(
			MonthPicker::Panel,
			day,
		)),
	);
	assert!(state.panel_day_wanted);

	state.calendar.sync = SyncState::Idle;
	let _ = fetch_pending_calendar_days(&mut state, &[]);
	assert!(!state.panel_day_wanted);
	assert_eq!(state.calendar.sync, SyncState::Syncing);
}

#[test]
fn a_snapshot_for_a_day_we_left_does_not_snap_the_view_back() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	let moved_to = state.calendar.day.succ_opt().unwrap();
	state.calendar.day = moved_to;
	state.calendar.sync = SyncState::Syncing;

	// The sample snapshot is for the day we navigated away from.
	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarSynced(Ok(
			calendar::sample_snapshot(),
		))),
	);

	assert_eq!(state.calendar.day, moved_to);
	assert_eq!(state.calendar.sync, SyncState::Idle);
	state
		.calendar
		.set_day(moved_to.pred_opt().unwrap(), moved_to);
	assert!(
		!state.calendar.events.is_empty(),
		"late results should remain cached"
	);
}

#[test]
fn resync_forgets_cached_days() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	// Stay signed out so the fetch is not started against a real account.
	// The cache drop still has to happen; a signed-in store would also
	// move into Syncing, which `sync_calendar` owns.
	state.calendar.connection = Connection::SignedOut;
	let today = state.calendar.day;
	let tomorrow = today.succ_opt().unwrap();
	state.calendar.set_day(tomorrow, today);

	let _ = update(
		&mut state,
		Message::Calendar(CalendarMessage::CalendarResyncRequested),
	);

	state.calendar.set_day(today, today);
	assert!(
		state.calendar.events.is_empty(),
		"a resync should forget days that were only cached"
	);
	assert_eq!(state.calendar.sync, SyncState::Idle);
}

fn at_in(minutes: i64) -> DateTime<Local> {
	alarms::minute_of(crate::clock::now()) + chrono::Duration::minutes(minutes)
}

fn place(state: &mut State, minutes: i64) {
	let _ = update(
		state,
		Message::Alarms(alarms::Action::Place(at_in(minutes))),
	);
}

#[test]
fn set_alarm_enters_and_leaves_alarm_mode() {
	let mut state = State::blank();
	state.calendar.menu_open = true;

	let _ = update(&mut state, Message::Alarms(alarms::Action::ModeToggled));

	assert!(state.alarms.mode);
	assert!(!state.calendar.menu_open, "the menu gets out of the way");

	let _ = update(&mut state, Message::Alarms(alarms::Action::ModeToggled));

	assert!(!state.alarms.mode);
}

#[test]
fn escape_leaves_alarm_mode() {
	let mut state = State::blank();
	state.alarms.mode = true;

	let escape = key_message(
		&mut state,
		Key::Named(keyboard::key::Named::Escape),
		keyboard::Modifiers::default(),
	);

	assert!(matches!(
		escape,
		Some(Message::Alarms(alarms::Action::ModeToggled))
	));
}

#[test]
fn clicking_a_future_time_sets_the_alarm_and_opens_its_options() {
	// Both this test and the app read the clock; it must not tick between them.
	let _clock = crate::clock::freeze(crate::clock::utc_now());
	let mut state = State::blank();
	state.alarms.mode = true;

	place(&mut state, 45);

	assert_eq!(state.alarms.items().len(), 1);
	assert!(!state.alarms.mode, "placing an alarm ends the mode");

	let editor = state.alarms.editor.as_ref().expect("the popup opens");
	assert_eq!(editor.id, state.alarms.items()[0].id);
	assert_eq!(
		alarms::minutes_between(crate::clock::now(), state.alarms.items()[0].at),
		45
	);
}

#[test]
fn an_alarm_cannot_be_set_in_the_past() {
	let mut state = State::blank();
	state.alarms.mode = true;

	place(&mut state, -30);
	place(&mut state, 0);

	assert!(state.alarms.is_empty());
	assert!(state.alarms.editor.is_none());
}

#[test]
fn a_new_alarm_starts_from_the_alarm_profile_without_the_timer_wording() {
	let mut state = State::blank();
	state.config.alarm.sound.volume = 33;

	place(&mut state, 10);

	let alarm = &state.alarms.items()[0];
	assert_eq!(alarm.profile.sound.volume, 33);
	assert_eq!(alarm.profile.notification.summary, "Alarm");
}

#[test]
fn opening_an_alarm_shows_its_time_and_closing_puts_it_away() {
	let mut state = State::blank();
	place(&mut state, 90);
	let id = state.alarms.items()[0].id;

	let _ = update(&mut state, Message::Alarms(alarms::Action::CloseEditor));
	assert!(state.alarms.editor.is_none());

	let _ = update(&mut state, Message::Alarms(alarms::Action::Open(id)));
	assert_eq!(
		state.alarms.editor.as_ref().map(|e| e.time_text.clone()),
		Some(state.alarms.items()[0].at.format("%H:%M").to_string())
	);
}

#[test]
fn an_alarm_can_be_removed_before_it_goes_off() {
	let mut state = State::blank();
	place(&mut state, 60);
	place(&mut state, 120);
	let id = state
		.alarms
		.editor
		.as_ref()
		.expect("the last one is open")
		.id;

	let _ = update(&mut state, Message::Alarms(alarms::Action::Delete(id)));

	assert_eq!(state.alarms.items().len(), 1);
	assert!(state.alarms.get(id).is_none());
	assert!(state.alarms.editor.is_none(), "its popup goes with it");
}

#[test]
fn editing_an_alarm_changes_what_it_does_and_nothing_else() {
	let mut state = State::blank();
	place(&mut state, 60);
	let before = state.alarms.items()[0].at;

	for edit in [
		alarms::Edit::Label("Tea".into()),
		alarms::Edit::Sound(false),
		alarms::Edit::Volume(12),
		alarms::Edit::Flash(false),
	] {
		let _ = update(&mut state, Message::Alarms(alarms::Action::Edit(edit)));
	}

	let alarm = &state.alarms.items()[0];
	assert_eq!(alarm.label, "Tea");
	assert!(!alarm.profile.sound.enabled);
	assert_eq!(alarm.profile.sound.volume, 12);
	assert!(!alarm.profile.flash.enabled);
	assert_eq!(alarm.at, before);
	assert_eq!(
		state.config.alarm,
		Config::default().alarm,
		"defaults stay put"
	);
}

#[test]
fn an_alarm_time_can_be_edited_but_not_into_the_past() {
	let mut state = State::blank();
	place(&mut state, 24 * 60 + 30);
	let alarm = state.alarms.items()[0].clone();

	// The same time of day, tomorrow, is still ahead.
	let later = (alarm.at + chrono::Duration::minutes(5))
		.format("%H:%M")
		.to_string();
	let _ = update(
		&mut state,
		Message::Alarms(alarms::Action::Edit(alarms::Edit::Time(later.clone()))),
	);
	assert_eq!(
		state.alarms.items()[0].at.format("%H:%M").to_string(),
		later
	);
	assert!(state.alarms.editor.as_ref().unwrap().time_error.is_none());

	// Nonsense is refused and the alarm stays where it was.
	let kept = state.alarms.items()[0].at;
	let _ = update(
		&mut state,
		Message::Alarms(alarms::Action::Edit(alarms::Edit::Time("soon".into()))),
	);
	assert_eq!(state.alarms.items()[0].at, kept);
	assert!(state.alarms.editor.as_ref().unwrap().time_error.is_some());
}

#[test]
fn an_alarm_for_today_cannot_be_edited_into_the_past() {
	let mut state = State::blank();
	place(&mut state, 2);
	let alarm = state.alarms.items()[0].clone();
	if alarm.at.date_naive() != crate::clock::now().date_naive() {
		return; // set a hair before midnight, so it landed on tomorrow
	}

	// Midnight has always passed by the time today has an alarm in it.
	let _ = update(
		&mut state,
		Message::Alarms(alarms::Action::Edit(alarms::Edit::Time("00:00".into()))),
	);

	assert_eq!(
		state.alarms.items()[0].at,
		alarm.at,
		"the alarm did not move"
	);
	assert!(state.alarms.editor.as_ref().unwrap().time_error.is_some());
}

#[test]
fn leaving_the_calendar_ends_alarm_setting() {
	let mut state = State::blank();
	state.alarms.mode = true;

	let _ = update(
		&mut state,
		Message::Navigation(NavigationMessage::ScreenSelected(Screen::Notepad)),
	);

	assert!(!state.alarms.mode);
}

#[test]
fn the_alarm_poll_only_runs_while_an_alarm_is_waiting() {
	let mut state = State::blank();
	assert!(!alarms::needs_poll(&state.alarms));

	place(&mut state, 5);

	assert!(alarms::needs_poll(&state.alarms));
}

/// A state with the standup already dragged an hour later and pending.
fn state_with_a_pending_drag() -> (State, DateTime<Local>, DateTime<Local>) {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());

	let event = state.calendar.event("standup").unwrap();
	let start = event.start + chrono::Duration::hours(1);
	let end = event.end + chrono::Duration::hours(1);
	state.calendar.begin_time_change("standup", start, end);
	(state, start, end)
}

#[test]
fn a_confirmed_drag_makes_the_chip_final() {
	let (mut state, start, _) = state_with_a_pending_drag();

	let _ = update(
		&mut state,
		Message::Event(EventMessage::EventTimeSaved("standup".into(), Ok(()))),
	);

	assert!(state.calendar.pending_time("standup").is_none());
	assert_eq!(state.calendar.event("standup").unwrap().start, start);
}

#[test]
fn a_refused_drag_puts_the_old_time_back_and_says_so() {
	let (mut state, start, _) = state_with_a_pending_drag();
	let before = calendar::sample_snapshot()
		.events
		.into_iter()
		.find(|event| event.id == "standup")
		.unwrap()
		.start;

	let _ = update(
		&mut state,
		Message::Event(EventMessage::EventTimeSaved(
			"standup".into(),
			Err("Google Calendar: nope".into()),
		)),
	);

	assert!(state.calendar.pending_time("standup").is_none());
	assert_eq!(state.calendar.event("standup").unwrap().start, before);
	assert_ne!(state.calendar.event("standup").unwrap().start, start);
	assert_eq!(
		state.calendar.error.as_deref(),
		Some("Google Calendar: nope")
	);
}

#[test]
fn a_task_time_change_uses_task_scheduling_instead_of_a_calendar_event_write() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());

	// The sample's `deep` event stands in for a Google Task.
	let deep = state.calendar.event_mut("deep").unwrap();
	deep.id = "task:deep".into();
	deep.calendar_id = "tasks:list".into();
	let start = deep.start + chrono::Duration::hours(1);
	let end = deep.end + chrono::Duration::hours(1);

	let _ = update(
		&mut state,
		Message::Event(EventMessage::EventTimeChanged {
			id: "task:deep".into(),
			start,
			end,
		}),
	);

	assert!(state.calendar.pending_time("task:deep").is_none());
	assert!(state.timeblocking.busy.contains_task("task:deep"));
	assert_ne!(state.calendar.event("task:deep").unwrap().start, start);
}
#[test]
fn estimate_popup_takes_keyboard_priority_over_the_time_block_view() {
	let mut state = State::blank();
	state.timeblocking.focus = Some("standup".into());
	state.timeblocking.estimate_task = Some("task:deep".into());
	assert!(
		key_message(
			&mut state,
			Key::Named(keyboard::key::Named::Space),
			keyboard::Modifiers::default()
		)
		.is_none()
	);
	assert!(matches!(
		key_message(
			&mut state,
			Key::Named(keyboard::key::Named::Escape),
			keyboard::Modifiers::default()
		),
		Some(Message::TimeBlocking(
			crate::timeblocking::Action::CloseEstimate
		))
	));
}

#[test]
fn escape_closes_the_distraction_choice_before_the_time_block_view() {
	let mut state = State::blank();
	state.timeblocking.focus = Some("standup".into());
	state.timeblocking.distraction_open = true;
	assert!(matches!(
		key_message(
			&mut state,
			Key::Named(keyboard::key::Named::Escape),
			keyboard::Modifiers::default()
		),
		Some(Message::TimeBlocking(
			crate::timeblocking::Action::CloseDistraction
		))
	));
}

#[test]
fn dragging_a_task_can_be_cancelled_with_escape() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	let task = state.calendar.event_mut("deep").unwrap();
	task.id = "task:deep".into();
	task.calendar_id = "tasks:list".into();

	let position = iced::Point::new(120.0, 240.0);
	let _ = update(
		&mut state,
		Message::TimeBlocking(crate::timeblocking::Action::Drag(
			"task:deep".into(),
			position,
		)),
	);
	assert_eq!(state.timeblocking.dragging.as_deref(), Some("task:deep"));
	assert_eq!(state.timeblocking.drag_pointer, Some(position));

	// While a task is in the air, Escape cancels the drag instead of
	// closing whatever is underneath it, and other keys are ignored.
	state.calendar.open_event = Some("deep".into());
	assert!(
		key_message(
			&mut state,
			Key::Named(keyboard::key::Named::Space),
			keyboard::Modifiers::default()
		)
		.is_none()
	);
	let message = key_message(
		&mut state,
		Key::Named(keyboard::key::Named::Escape),
		keyboard::Modifiers::default(),
	);
	assert!(matches!(
		message,
		Some(Message::TimeBlocking(
			crate::timeblocking::Action::CancelDrag
		))
	));
	let _ = update(&mut state, message.unwrap());
	assert!(state.timeblocking.dragging.is_none());
	assert!(state.timeblocking.drag_pointer.is_none());
}

#[test]
fn the_floating_card_follows_the_pointer_only_while_dragging() {
	let mut state = State::blank();
	let _ = update(
		&mut state,
		Message::TimeBlocking(crate::timeblocking::Action::DragMoved(iced::Point::new(
			10.0, 20.0,
		))),
	);
	assert!(state.timeblocking.drag_pointer.is_none());

	state.timeblocking.dragging = Some("task:deep".into());
	let _ = update(
		&mut state,
		Message::TimeBlocking(crate::timeblocking::Action::DragMoved(iced::Point::new(
			30.0, 40.0,
		))),
	);
	assert_eq!(
		state.timeblocking.drag_pointer,
		Some(iced::Point::new(30.0, 40.0))
	);
}

#[test]
fn notes_typing_does_not_trigger_timer_shortcuts() {
	let mut state = State::blank();
	state.screen = Screen::Notes;
	assert!(
		key_message(
			&mut state,
			Key::Named(keyboard::key::Named::Space),
			keyboard::Modifiers::default()
		)
		.is_none()
	);
	assert!(matches!(
		key_message(
			&mut state,
			Key::Named(keyboard::key::Named::Escape),
			keyboard::Modifiers::default()
		),
		Some(Message::Navigation(NavigationMessage::ScreenSelected(
			Screen::Calendar
		)))
	));
}

#[test]
fn choosing_habit_opens_the_shared_details_editor() {
	let mut state = State::blank();
	let start = calendar::at_clock(chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(), 9, 0);
	let mut draft = calendar::Draft::new(
		"primary".into(),
		start,
		start + chrono::Duration::minutes(45),
	);
	draft.title = "Stretch".into();
	draft.description = "Slow and steady".into();
	state.calendar.draft = Some(draft);

	let _ = update(
		&mut state,
		Message::EventDraft(EventDraftMessage::EventDraftKindSelected(
			calendar::DraftKind::Habit,
		)),
	);

	assert!(state.calendar.draft.is_none());
	assert!(state.details_pane_open);
	let habit = state
		.calendar
		.edit_draft
		.as_ref()
		.expect("the shared editor opens");
	assert_eq!(habit.title, "Stretch");
	assert_eq!(habit.description, "Slow and steady");
	assert_eq!(habit.date_text, "2026-10-28");
	assert_eq!(habit.start_text, "09:00");
	assert_eq!(habit.end_text, "09:45");
	assert_eq!(habit.rule.repeat, crate::habits::RepeatMode::Weekly);
	assert!(habit.rule.weekdays[2], "Wednesday is preselected");
}

#[test]
fn the_system_themes_follow_the_system_tone() {
	use iced::theme::Mode;

	let mut state = State::blank();
	let id = window::Id::unique();

	state.config.appearance.theme = crate::theme::SYSTEM.into();
	assert!(active_theme(&state, id).is_none());

	state.config.appearance.theme = crate::theme::SYSTEM_HIGH_CONTRAST.into();
	state.system_theme = Mode::Dark;
	assert_eq!(
		active_theme(&state, id).unwrap().to_string(),
		"Dark high contrast"
	);
	state.system_theme = Mode::Light;
	assert_eq!(
		active_theme(&state, id).unwrap().to_string(),
		"Light high contrast"
	);

	state.config.appearance.theme = "Dark".into();
	assert_eq!(active_theme(&state, id).unwrap().to_string(), "Dark");
}

fn command_key(state: &mut State, character: &str) -> Option<Message> {
	key_message(
		state,
		Key::Character(character.into()),
		keyboard::Modifiers::COMMAND,
	)
}

#[test]
fn the_zoom_keys_step_in_and_out_and_reset() {
	let mut state = State::blank();
	state.screen = Screen::Calendar;
	let default = state.calendar.hour_height;

	// Plain `=` is the key `+` is on, so it counts without Shift held.
	for character in ["+", "="] {
		let message = command_key(&mut state, character).expect("zooms in");
		let _ = update(&mut state, message);
	}
	assert!(state.calendar.hour_height > default);

	let grown = state.calendar.hour_height;
	let message = command_key(&mut state, "-").expect("zooms out");
	let _ = update(&mut state, message);
	assert!(state.calendar.hour_height < grown);

	let message = command_key(&mut state, "0").expect("resets");
	let _ = update(&mut state, message);
	assert_eq!(state.calendar.hour_height, default);
}

#[test]
fn the_zoom_keys_work_on_every_calendar_view() {
	for view in [
		CalendarView::Day,
		CalendarView::Week,
		CalendarView::TwoWeek,
		CalendarView::Month,
	] {
		let mut state = State::blank();
		state.screen = Screen::Calendar;
		state.view = view;
		assert!(command_key(&mut state, "+").is_some(), "{view:?}");
	}
}

#[test]
fn the_zoom_keys_leave_a_screen_without_the_grid_alone() {
	let mut state = State::blank();
	state.screen = Screen::Notepad;

	assert!(command_key(&mut state, "+").is_none());
	assert!(command_key(&mut state, "-").is_none());
	assert!(command_key(&mut state, "0").is_none());
}

#[test]
fn the_zoom_keys_need_the_modifier() {
	let mut state = State::blank();
	state.screen = Screen::Calendar;

	for character in ["+", "-", "0"] {
		let pressed = key_message(
			&mut state,
			Key::Character(character.into()),
			keyboard::Modifiers::default(),
		);
		assert!(pressed.is_none(), "{character} alone must not zoom");
	}
}

/// The sample calendars plus the app's own, with the "standup" event open.
fn event_open_with_app_calendar() -> State {
	let mut state = State::blank();
	let mut snapshot = calendar::sample_snapshot();
	snapshot.calendars.push(calendar::Calendar {
		id: "app".into(),
		name: "TimeParagon".into(),
		color: calendar::colors::DEFAULT_EVENT,
		primary: false,
		selected_by_google: true,
	});
	state.calendar = calendar::Store::with_snapshot(snapshot);
	let _ = update(
		&mut state,
		Message::Event(EventMessage::EventOpened("standup".into())),
	);
	state
}

#[test]
fn an_event_can_be_staged_onto_another_calendar_but_not_timeparagon() {
	let mut state = event_open_with_app_calendar();
	let original = state.calendar.event("standup").unwrap().calendar_id.clone();
	let other = state
		.calendar
		.calendars
		.iter()
		.find(|calendar| calendar.id != original && calendar.id != "app")
		.unwrap()
		.id
		.clone();

	let select =
		|id: &str| Message::EventEdit(EventEditMessage::EventEditCalendarSelected(id.to_owned()));
	let _ = update(&mut state, select("app"));
	assert_eq!(
		state.calendar.edit_draft.as_ref().unwrap().calendar_id,
		original
	);

	let _ = update(&mut state, select(&other));
	assert_eq!(
		state.calendar.edit_draft.as_ref().unwrap().calendar_id,
		other
	);
	// Staged only: the event itself has not moved until Google says so.
	assert_eq!(
		state.calendar.event("standup").unwrap().calendar_id,
		original
	);
}

#[test]
fn a_saved_move_updates_the_event_only_for_the_draft_being_saved() {
	let mut state = event_open_with_app_calendar();
	let original = state.calendar.event("standup").unwrap().calendar_id.clone();
	let other = state
		.calendar
		.calendars
		.iter()
		.find(|calendar| calendar.id != original && calendar.id != "app")
		.unwrap()
		.id
		.clone();
	let saved = Message::EventEdit(EventEditMessage::EventEditSaved(Ok(Default::default())));

	let draft = state.calendar.edit_draft.as_mut().unwrap();
	draft.calendar_id = other.clone();
	draft.saving = false;
	let _ = update(&mut state, saved.clone());
	assert_eq!(
		state.calendar.event("standup").unwrap().calendar_id,
		original
	);

	state.calendar.edit_draft.as_mut().unwrap().saving = true;
	let _ = update(&mut state, saved);
	let event = state.calendar.event("standup").unwrap();
	assert_eq!(event.calendar_id, other);
	assert_eq!(event.color, state.calendar.calendar_color(&other));
}

#[test]
fn a_failed_reply_puts_the_previous_answer_back() {
	use calendar::meeting::{Attendance, Guest, Response};
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	let event = state
		.calendar
		.events
		.iter_mut()
		.find(|e| !e.is_task())
		.unwrap();
	event.details.attendance = Attendance {
		guests: vec![Guest {
			email: "me@example.com".into(),
			name: String::new(),
			response: Response::NeedsAction,
			optional: false,
		}],
		own_email: Some("me@example.com".into()),
		..Default::default()
	};
	let id = event.id.clone();
	let answer = |state: &State| {
		state
			.calendar
			.event(&id)
			.unwrap()
			.details
			.attendance
			.own_response()
	};
	let _ = update(
		&mut state,
		Message::Event(EventMessage::EventReply(id.clone(), Response::Accepted)),
	);
	assert_eq!(answer(&state), Some(Response::Accepted));
	let _ = update(
		&mut state,
		Message::Event(EventMessage::EventReplied(
			id.clone(),
			Some(Response::NeedsAction),
			Err("offline".into()),
		)),
	);
	assert_eq!(answer(&state), Some(Response::NeedsAction));
	assert_eq!(state.calendar.error.as_deref(), Some("offline"));
}

#[test]
fn dragging_an_event_without_guests_does_not_ask() {
	let mut state = State::blank();
	state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
	let event = state.calendar.events.iter().find(|e| !e.is_task()).unwrap();
	let (id, start, end) = (event.id.clone(), event.start, event.end);
	let hour = chrono::Duration::hours(1);
	let _ = update(
		&mut state,
		Message::Event(EventMessage::EventTimeChanged {
			id,
			start: start + hour,
			end: end + hour,
		}),
	);
	assert!(state.calendar.pending_move.is_none());
}

#[test]
fn recurring_moves_and_both_resize_edges_require_scope_and_cancel_restores_times() {
	for kind in ["event", "block", "habit"] {
		for edge in ["move", "start", "end"] {
			for has_master in [false, true] {
				let mut state = State::blank();
				state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
				let event = state
					.calendar
					.events
					.iter_mut()
					.find(|e| !e.is_task() && !e.all_day)
					.unwrap();
				event.details.attendance = Default::default();
				event.details.habit = kind == "habit";
				if kind == "block" {
					state.config.calendar.app_calendar = Some(event.calendar_id.clone());
				}
				event.details.recurring_event_id = has_master.then(|| "series".into());
				event.details.recurrence = if has_master {
					vec![]
				} else {
					vec!["RRULE:FREQ=DAILY".into()]
				};
				let original = event.clone();
				let step = chrono::Duration::minutes(15);
				let start = original.start
					+ if edge == "end" {
						chrono::Duration::zero()
					} else {
						step
					};
				let end = original.end
					+ if edge == "start" {
						chrono::Duration::zero()
					} else {
						step
					};
				let _ = update(
					&mut state,
					Message::Event(EventMessage::EventTimeChanged {
						id: original.id.clone(),
						start,
						end,
					}),
				);
				let pending = state
					.calendar
					.pending_move
					.as_ref()
					.expect("recurring timing changes need a scope");
				assert_eq!(pending.scope, None);
				assert_eq!(pending.original, original);
				assert_eq!(state.calendar.event(&original.id).unwrap().start, start);
				assert!(matches!(
					key_message(
						&mut state,
						Key::Named(keyboard::key::Named::Escape),
						keyboard::Modifiers::default()
					),
					Some(Message::Event(EventMessage::EventMoveCancelled))
				));
				// A stale notification answer cannot bypass scope selection.
				let _ = update(
					&mut state,
					Message::Event(EventMessage::EventMoveConfirmed(false)),
				);
				assert!(state.calendar.pending_move.is_some());
				let _ = update(&mut state, Message::Event(EventMessage::EventMoveCancelled));
				assert_eq!(state.calendar.event(&original.id).unwrap(), &original);
				assert!(state.calendar.pending_time(&original.id).is_none());
			}
		}
	}
}

#[test]
fn recurring_meetings_choose_scope_before_guest_notifications() {
	for scope in [
		calendar::SaveScope::Current,
		calendar::SaveScope::CurrentAndFuture,
		calendar::SaveScope::All,
	] {
		let mut state = State::blank();
		state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
		let event = state
			.calendar
			.events
			.iter_mut()
			.find(|e| !e.is_task() && !e.all_day)
			.unwrap();
		event.details.recurring_event_id = Some("series".into());
		event.details.attendance.organizer = true;
		event
			.details
			.attendance
			.guests
			.push(calendar::meeting::Guest {
				email: "ada@example.com".into(),
				..Default::default()
			});
		let original = event.clone();
		let step = chrono::Duration::minutes(15);
		let _ = update(
			&mut state,
			Message::Event(EventMessage::EventTimeChanged {
				id: original.id.clone(),
				start: original.start + step,
				end: original.end + step,
			}),
		);
		let _ = update(
			&mut state,
			Message::Event(EventMessage::EventMoveScoped(scope)),
		);
		let pending = state.calendar.pending_move.as_ref().unwrap();
		assert_eq!(pending.scope, Some(scope));
		assert_eq!(pending.original, original);
		let _ = update(&mut state, Message::Event(EventMessage::EventMoveCancelled));
		assert_eq!(state.calendar.event(&original.id).unwrap(), &original);
	}
}

#[test]
fn quick_create_cycles_types_without_committing_and_escape_dismisses() {
	let mut state = State::blank();
	let start = calendar::at_clock(state.calendar.day, 9, 0);
	state
		.calendar
		.begin_draft("primary".into(), start, start + chrono::Duration::hours(1));
	for expected in [
		calendar::DraftKind::Event,
		calendar::DraftKind::TimeBlock,
		calendar::DraftKind::Task,
		calendar::DraftKind::Habit,
		calendar::DraftKind::Event,
	] {
		let _ = update(
			&mut state,
			Message::EventDraft(EventDraftMessage::EventDraftCycleKind {
				reverse: false,
				title_focused: false,
			}),
		);
		assert_eq!(state.draft_kind_focus, Some(expected));
		let draft = state.calendar.draft.as_ref().unwrap();
		assert!(!draft.editor_open && !draft.saving);
	}
	let _ = update(
		&mut state,
		Message::EventDraft(EventDraftMessage::EventDraftCycleKind {
			reverse: true,
			title_focused: false,
		}),
	);
	assert_eq!(state.draft_kind_focus, Some(calendar::DraftKind::Habit));
	assert!(matches!(
		key_message(
			&mut state,
			Key::Named(keyboard::key::Named::Enter),
			keyboard::Modifiers::default()
		),
		Some(Message::EventDraft(
			EventDraftMessage::EventDraftKindSelected(calendar::DraftKind::Habit)
		))
	));
	// Clicking the title before another Tab restarts the cycle at Event.
	let _ = update(
		&mut state,
		Message::EventDraft(EventDraftMessage::EventDraftCycleKind {
			reverse: false,
			title_focused: true,
		}),
	);
	assert_eq!(state.draft_kind_focus, Some(calendar::DraftKind::Event));
	let escape = key_message(
		&mut state,
		Key::Named(keyboard::key::Named::Escape),
		keyboard::Modifiers::default(),
	)
	.unwrap();
	let _ = update(&mut state, escape);
	assert!(state.calendar.draft.is_none());
}

#[test]
fn life_area_opens_in_a_separate_panel_and_escape_closes_it() {
	use crate::life::Action;
	use iced::Settings;
	use iced_test::Simulator;

	let mut state = State::blank();
	state.screen = Screen::Life;
	state.main_window = Some(window::Id::unique());
	let main_size = state.main_size;
	let actions = geometry_actions(update(
		&mut state,
		Message::Life(Action::Open(crate::life::Selection::Area(
			config::PERSONAL_AREA,
		))),
	));
	assert!(
		matches!(actions.as_slice(), [GeometryAction::Open(settings)]
        if settings.size.width == ui::life::PANEL_WIDTH)
	);
	assert!(state.editor_window.is_some());
	assert_eq!(state.main_size, main_size);

	let size = Size::new(560.0, 900.0);
	{
		let mut list = Simulator::with_size(Settings::default(), size, ui::view(&state));
		assert!(list.find("This week's focus").is_ok());
		assert!(list.find("Vision").is_err());
		list.click("Areas").unwrap();
		assert!(list.into_messages().any(|message| matches!(
			message,
			Message::Life(Action::ShowPage(crate::life::Page::Areas))
		)));
		let mut panel = Simulator::with_size(Settings::default(), size, ui::editor_window(&state));
		assert!(panel.find("Vision").is_ok());
		assert!(panel.find("Personal").is_ok());
		panel.click("Add goal").unwrap();
		assert!(
			panel
				.into_messages()
				.any(|message| matches!(message, Message::Life(Action::AddGoal)))
		);
	}
	let escape = key_message(
		&mut state,
		Key::Named(keyboard::key::Named::Escape),
		keyboard::Modifiers::default(),
	)
	.unwrap();
	let _ = update(&mut state, escape);
	assert!(state.life.open.is_none());
	assert!(state.editor_window.is_none());
	assert_eq!(state.screen, Screen::Life);
}

#[test]
fn closing_the_life_side_window_dismisses_the_area() {
	let mut state = State::blank();
	state.screen = Screen::Life;
	state.main_window = Some(window::Id::unique());
	let _ = update(
		&mut state,
		Message::Life(crate::life::Action::Open(crate::life::Selection::Area(
			config::PERSONAL_AREA,
		))),
	);
	let editor = state.editor_window.unwrap();
	let _ = update(
		&mut state,
		Message::System(SystemMessage::WindowClosed(editor)),
	);
	assert!(state.life.open.is_none());
	assert!(state.editor_window.is_none());
}

#[test]
fn an_opened_occurrence_shows_its_series_repeat_rule_unless_it_was_already_changed() {
	for already_changed in [false, true] {
		let mut state = State::blank();
		state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
		let event = state
			.calendar
			.events
			.iter_mut()
			.find(|e| !e.is_task() && !e.all_day)
			.unwrap();
		event.details.habit = false;
		event.details.recurring_event_id = Some("series".into());
		event.details.recurrence.clear();
		let event = event.clone();
		let mut draft = calendar::EditDraft::from_event(&event);
		draft.recurrence_changed = already_changed;
		let before = draft.rule.clone();
		state.calendar.open_event = Some(event.id.clone());
		state.calendar.edit_draft = Some(draft);

		let mut master = event.clone();
		master.id = "series".into();
		master.details.recurring_event_id = None;
		master.details.recurrence = vec!["RRULE:FREQ=WEEKLY;BYDAY=MO,WE".into()];
		let loaded = calendar::EditDraft::from_habit_occurrence(&event, &master);
		let _ = update(
			&mut state,
			Message::Event(EventMessage::SeriesRuleLoaded(
				event.id.clone(),
				Ok((event.clone(), loaded.clone())),
			)),
		);

		let draft = state.calendar.edit_draft.as_ref().unwrap();
		assert!(!draft.loading);
		if already_changed {
			assert_eq!(draft.rule, before);
			assert_eq!(draft.series_id, None);
		} else {
			assert_eq!(draft.rule, loaded.rule);
			assert_eq!(draft.series_id.as_deref(), Some("series"));
		}
	}
}

#[test]
fn every_life_page_and_panel_renders_with_goals_objectives_and_reviews() {
	use crate::life::{Action, Page, Selection};
	use iced::Settings;
	use iced_test::Simulator;

	let mut state = State::blank();
	state.screen = Screen::Life;
	state.main_window = Some(window::Id::unique());
	let life = |state: &mut State, action| {
		let _ = update(state, Message::Life(action));
	};
	life(&mut state, Action::NewName("Health".into()));
	life(&mut state, Action::Add);
	let health = state.config.projects.areas[1].id;
	life(&mut state, Action::NewGoal("Run a marathon".into()));
	life(&mut state, Action::AddGoal);
	let goal = state.config.projects.areas[1].goals[0].id;
	life(&mut state, Action::Open(Selection::Goal(goal)));
	for name in ["Run 30 km a week", "Finish a half", "Stretch daily"] {
		life(&mut state, Action::NewObjective(name.into()));
		life(&mut state, Action::AddObjective);
	}
	let ids: Vec<u64> = state.config.projects.areas[1]
		.objectives
		.iter()
		.map(|o| o.id)
		.collect();
	life(&mut state, Action::Measure(ids[0], config::Measure::Number));
	life(&mut state, Action::Open(Selection::Objective(ids[0])));
	life(&mut state, Action::Baseline("0".into()));
	life(&mut state, Action::Target("30".into()));
	life(&mut state, Action::CheckInValue("12".into()));
	life(&mut state, Action::CheckIn);
	life(&mut state, Action::Focus(ids[0], true));
	life(
		&mut state,
		Action::DueBy(ids[0], crate::life::Horizon::Month),
	);
	life(
		&mut state,
		Action::Measure(ids[1], config::Measure::Milestones),
	);
	life(&mut state, Action::Open(Selection::Objective(ids[1])));
	life(&mut state, Action::NewMilestone("10k".into()));
	life(&mut state, Action::AddMilestone);
	life(&mut state, Action::Due(ids[1], "2020-01-01".into()));
	life(&mut state, Action::Measure(ids[2], config::Measure::Habits));
	life(&mut state, Action::Rate(health, 6));
	let size = Size::new(560.0, 900.0);
	for page in Page::ALL {
		life(&mut state, Action::ShowPage(page));
		let mut view = Simulator::with_size(Settings::default(), size, ui::view(&state));
		assert!(view.find(page.label()).is_ok(), "{page:?}");
	}
	{
		let mut view = Simulator::with_size(Settings::default(), size, ui::view(&state));
		assert!(view.find("1. How is each area going?").is_ok());
	}
	life(&mut state, Action::ShowPage(Page::Focus));
	{
		let mut view = Simulator::with_size(Settings::default(), size, ui::view(&state));
		assert!(view.find("Run 30 km a week").is_ok());
		assert!(
			view.find("Needs attention").is_ok(),
			"the overdue objective"
		);
	}

	let project = state.config.projects.items[0].id;
	life(&mut state, Action::LinkProject(ids[0], project));
	for selection in [
		Selection::Area(health),
		Selection::Goal(goal),
		Selection::Objective(ids[0]),
		Selection::Objective(ids[1]),
		Selection::Objective(ids[2]),
		Selection::Project(project),
	] {
		life(&mut state, Action::Open(selection));
		let _ = Simulator::with_size(Settings::default(), size, ui::editor_window(&state));
	}
	let mut panel = Simulator::with_size(Settings::default(), size, ui::editor_window(&state));
	assert!(panel.find("Objectives it serves").is_ok());
	panel.click("‹ Back").unwrap();
	assert!(
		panel
			.into_messages()
			.any(|message| matches!(message, Message::Life(Action::Back)))
	);
}
