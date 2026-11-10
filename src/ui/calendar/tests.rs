use super::*;
use chrono::Timelike;

#[test]
fn creation_chooser_prefers_above_then_weekly_left_then_below() {
    let viewport = Size::new(900.0, 600.0);
    let card = Size::new(340.0, 90.0);
    let anchor = Rectangle {
        x: 500.0,
        y: 250.0,
        width: 180.0,
        height: 60.0,
    };
    for weekly in [false, true] {
        assert_eq!(
            draft_position(anchor, viewport, card, weekly),
            Point::new(500.0, 152.0)
        );
    }
    let top = Rectangle { y: 20.0, ..anchor };
    assert_eq!(
        draft_position(top, viewport, card, true),
        Point::new(152.0, 20.0)
    );
    assert_eq!(
        draft_position(top, viewport, card, false),
        Point::new(500.0, 88.0)
    );
    let left = Rectangle { x: 40.0, ..top };
    assert_eq!(
        draft_position(left, viewport, card, true),
        Point::new(40.0, 88.0)
    );
    let narrow = Size::new(220.0, 300.0);
    let position = draft_position(anchor, narrow, Size::new(204.0, 90.0), true);
    assert!(position.x >= 0.0 && position.x + 204.0 <= narrow.width);
    assert!(position.y >= 0.0 && position.y + 90.0 <= narrow.height);
}

#[test]
fn event_options_show_fields_and_emit_notification_changes() {
    let mut options = calendar::event_options::EventOptions::default();
    options.update(calendar::event_options::Action::Add);
    options.update(calendar::event_options::Action::Add);
    let metrics = Metrics::of(Size::new(360.0, 700.0));
    let mut ui = iced_test::Simulator::with_size(
        iced_test::core::Settings::default(),
        Size::new(360.0, 700.0),
        event_options(&options, &metrics, |v0| {
            Message::EventEdit(EventEditMessage::EventEditOptions(v0))
        }),
    );
    assert!(ui.find("Availability").is_ok());
    assert!(ui.find("Visibility").is_ok());
    ui.click("Add notification").unwrap();
    assert!(ui.into_messages().any(|m| matches!(
        m,
        Message::EventEdit(EventEditMessage::EventEditOptions(
            calendar::event_options::Action::Add
        ))
    )));
}

#[test]
fn the_new_event_popup_offers_a_habit() {
    let mut state = State::blank();
    let start = calendar::at_clock(chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(), 9, 0);
    state.calendar.draft = Some(calendar::Draft::new(
        "primary".into(),
        start,
        start + chrono::Duration::minutes(45),
    ));
    let metrics = Metrics::of(Size::new(800.0, 600.0));
    let mut ui = iced_test::Simulator::with_size(
        iced_test::core::Settings::default(),
        Size::new(800.0, 600.0),
        draft_overlay(&state, &metrics),
    );
    assert!(ui.find("Event").is_ok());
    assert!(ui.find("Time Block").is_ok());
    assert!(ui.find("Habit").is_ok());
    for field in [
        "Sleep",
        "Repeat",
        "Calendar",
        "Description",
        "Save",
        "New event",
    ] {
        assert!(ui.find(field).is_err(), "{field} belongs in the pane");
    }
    ui.click("Habit").unwrap();
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::EventDraft(EventDraftMessage::EventDraftKindSelected(
            calendar::DraftKind::Habit
        ))
    )));
}

#[test]
fn the_new_event_popup_cancels_from_a_full_width_button() {
    let mut state = State::blank();
    let start = calendar::at_clock(chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(), 9, 0);
    state.calendar.draft = Some(calendar::Draft::new(
        "primary".into(),
        start,
        start + chrono::Duration::minutes(45),
    ));
    let metrics = Metrics::of(Size::new(800.0, 600.0));
    let mut ui = iced_test::Simulator::with_size(
        iced_test::core::Settings::default(),
        Size::new(800.0, 600.0),
        draft_overlay(&state, &metrics),
    );
    ui.click("Cancel").unwrap();
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::EventDraft(EventDraftMessage::EventDraftCancelled)
    )));
}

#[test]
fn event_project_picker_searches_and_emits_assignment_actions() {
    let state = State::blank();
    let metrics = Metrics::of(Size::new(300.0, 800.0));
    let selection = calendar::projects::Selection {
        open: true,
        query: "work".into(),
        ..Default::default()
    };
    let mut ui = iced_test::Simulator::with_size(
        iced_test::core::Settings::default(),
        Size::new(300.0, 800.0),
        event_projects(&state, &selection, &metrics, |v0| {
            Message::EventEdit(EventEditMessage::EventEditProjects(v0))
        }),
    );
    assert!(ui.find("Personal").is_err());
    ui.click("Work").unwrap();
    let id = state
        .config
        .projects
        .items
        .iter()
        .find(|p| p.name == "Work")
        .unwrap()
        .id;
    assert!(ui.into_messages().any(|m| matches!(m,
        Message::EventEdit(EventEditMessage::EventEditProjects(calendar::projects::Action::Add(value))) if value == id)));
}

#[test]
fn event_footer_and_delete_menu_keep_their_layout() {
    let mut state = State::blank();
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    let event = state
        .calendar
        .events
        .iter_mut()
        .find(|e| e.id == "standup")
        .unwrap();
    event.details.description = "Meeting notes".into();
    event.details.meeting = "https://zoom.us/j/123456789".into();
    event.details.link = "https://calendar.google.com/event?eid=1".into();
    state.calendar.open_event = Some("standup".into());
    state.calendar.edit_draft = Some(calendar::EditDraft::from_event(
        state.calendar.event("standup").unwrap(),
    ));
    for width in [200.0, 360.0] {
        let metrics = Metrics::of(Size::new(width, 600.0));
        let mut ui = iced_test::Simulator::with_size(
            Default::default(),
            metrics.size,
            details_pane(&state, &metrics),
        );
        let description = ui.find("View / edit description").unwrap().bounds();
        let zoom = ui.find("Open in Zoom").unwrap().bounds();
        let projects = ui.find("Projects").unwrap().bounds();
        assert!(description.y < zoom.y && zoom.y < projects.y);
        let labels = [
            "Save current event",
            "Delete",
            "Open in Google Calendar",
            "Close",
        ];
        let bounds: Vec<_> = labels
            .iter()
            .map(|label| ui.find(*label).unwrap().bounds())
            .collect();
        assert!(
            bounds
                .windows(2)
                .all(|pair| pair[0].y + pair[0].height <= pair[1].y)
        );
        assert!(bounds.last().unwrap().y + bounds.last().unwrap().height <= metrics.size.height);
        assert!(ui.find("Delete all events").is_err());
        drop(ui);
        let _ = crate::app::update(
            &mut state,
            Message::EventEdit(EventEditMessage::EventEditDeleteMenuToggled),
        );
        let mut ui = iced_test::Simulator::with_size(
            Default::default(),
            metrics.size,
            details_pane(&state, &metrics),
        );
        for (label, expected) in labels.iter().zip(&bounds) {
            assert_eq!(ui.find(*label).unwrap().bounds(), *expected);
        }
        ui.click("Delete all events").unwrap();
        let message = ui.into_messages().next().unwrap();
        assert!(matches!(
            message,
            Message::Event(EventMessage::EventDeleteScoped(_, calendar::SaveScope::All))
        ));
        let _ = crate::app::update(&mut state, message);
        assert_eq!(state.calendar.pending_delete.as_deref(), Some("standup"));
        assert_eq!(
            state.calendar.pending_delete_scope,
            calendar::SaveScope::All
        );
        assert!(!state.calendar.edit_draft.as_ref().unwrap().delete_menu_open);
        let mut confirmation = iced_test::Simulator::with_size(
            Default::default(),
            metrics.size,
            delete_confirmation(&state, &metrics),
        );
        assert!(
            confirmation
                .find("Delete all events in this series?")
                .is_ok()
        );
        drop(confirmation);
        let _ = crate::app::update(
            &mut state,
            Message::Event(EventMessage::EventDeleteCancelled),
        );
    }
    for scope in [
        calendar::SaveScope::CurrentAndFuture,
        calendar::SaveScope::Current,
    ] {
        let _ = crate::app::update(
            &mut state,
            Message::Event(EventMessage::EventDeleteScoped("standup".into(), scope)),
        );
        assert_eq!(state.calendar.pending_delete_scope, scope);
        assert!(state.calendar.event("standup").is_some());
        let _ = crate::app::update(
            &mut state,
            Message::Event(EventMessage::EventDeleteCancelled),
        );
    }
}

#[test]
fn save_split_button_keeps_the_arrow_target_separate_and_opens_scoped_actions() {
    let mut state = State::blank();
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    for habit in [false, true] {
        let mut event = state.calendar.event("standup").unwrap().clone();
        event.details.habit = habit;
        event.details.recurring_event_id = Some("series".into());
        state.calendar.edit_draft = Some(calendar::EditDraft::from_event(&event));
        for width in [200.0, 300.0] {
            let metrics = Metrics::of(Size::new(width, 160.0));
            // Check both edges and the middle of the 48px arrow target.
            for x in [width - 46.0, width - 24.0, width - 2.0] {
                let mut ui = iced_test::Simulator::with_size(
                    Default::default(),
                    metrics.size,
                    event_save_control(&state, &event, &metrics),
                );
                assert!(ui.find("Save current and future events").is_err());
                ui.point_at(Point::new(x, 20.0));
                ui.simulate(iced_test::simulator::click());
                let messages: Vec<_> = ui.into_messages().collect();
                assert_eq!(messages.len(), 1);
                assert!(matches!(
                    messages[0],
                    Message::EventEdit(EventEditMessage::EventEditSaveMenuToggled)
                ));
            }
            let mut ui = iced_test::Simulator::with_size(
                Default::default(),
                metrics.size,
                event_save_control(&state, &event, &metrics),
            );
            ui.click("Save current event").unwrap();
            assert!(ui.into_messages().any(|message| matches!(
                message,
                Message::EventEdit(EventEditMessage::EventEditSaveRequested)
            )));
        }
        let _ = crate::app::update(
            &mut state,
            Message::EventEdit(EventEditMessage::EventEditSaveMenuToggled),
        );
        for (label, expected) in [
            (
                "Save current and future events",
                calendar::SaveScope::CurrentAndFuture,
            ),
            ("Save all events", calendar::SaveScope::All),
        ] {
            let metrics = Metrics::of(Size::new(300.0, 200.0));
            let mut ui = iced_test::Simulator::with_size(
                Default::default(),
                metrics.size,
                event_save_control(&state, &event, &metrics),
            );
            ui.click(label).unwrap();
            assert!(ui.into_messages().any(|message| matches!(message, Message::EventEdit(EventEditMessage::EventEditSaveScoped(scope)) if scope == expected)));
        }
    }
}

#[test]
fn habit_creation_has_one_save_button_and_existing_habits_disable_scopes_while_busy() {
    let mut state = State::blank();
    let mut event = calendar::sample_snapshot().events[1].clone();
    event.id = crate::habits::NEW_HABIT_ID.into();
    event.details.habit = true;
    state.calendar.edit_draft = Some(calendar::EditDraft::from_event(&event));
    let metrics = Metrics::of(Size::new(300.0, 200.0));
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        event_save_control(&state, &event, &metrics),
    );
    assert!(ui.find("▾").is_err());
    ui.click("Save changes").unwrap();
    assert!(matches!(
        ui.into_messages().next(),
        Some(Message::EventEdit(EventEditMessage::EventEditSaveRequested))
    ));

    event.id = "habit-instance".into();
    for loading in [false, true] {
        let mut draft = calendar::EditDraft::from_event(&event);
        draft.loading = loading;
        draft.saving = !loading;
        state.calendar.edit_draft = Some(draft);
        let mut ui = iced_test::Simulator::with_size(
            Default::default(),
            metrics.size,
            event_save_control(&state, &event, &metrics),
        );
        ui.click(if loading {
            "Save current event"
        } else {
            "Saving…"
        })
        .unwrap();
        ui.click("▾").unwrap();
        assert!(ui.into_messages().next().is_none());
    }
}

#[test]
fn save_menu_overlays_the_scroll_pane_without_changing_layout() {
    let mut state = State::blank();
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    let event = state.calendar.event("standup").unwrap().clone();
    state.calendar.edit_draft = Some(calendar::EditDraft::from_event(&event));
    let metrics = Metrics::of(Size::new(400.0, 400.0));
    fn form<'a>(state: &'a State, event: &'a Event, metrics: &Metrics) -> Element<'a, Message> {
        iced::widget::column![
            row![
                scrollable(iced::widget::column![
                    Space::new().height(90.0),
                    event_save_control(state, event, metrics)
                ])
                .width(280.0)
                .height(140.0),
                text("Beside the pane"),
            ],
            button("Below the pane").on_press(Message::Event(EventMessage::EventClosed)),
        ]
        .into()
    }
    let mut closed = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        form(&state, &event, &metrics),
    );
    let below = closed.find("Below the pane").unwrap().bounds();
    let beside = closed.find("Beside the pane").unwrap().bounds();
    let save = closed.find("Save current event").unwrap().bounds();
    let snapshots =
        std::env::temp_dir().join(format!("timeslay-save-overlay-{}", std::process::id()));
    std::fs::create_dir_all(&snapshots).unwrap();
    let closed_image = snapshots.join("closed.png");
    closed
        .snapshot(&Theme::Light)
        .unwrap()
        .matches_image(&closed_image)
        .unwrap();
    closed.click("▾").unwrap();
    let toggle = closed
        .into_messages()
        .find(|message| {
            matches!(
                message,
                Message::EventEdit(EventEditMessage::EventEditSaveMenuToggled)
            )
        })
        .unwrap();
    let _ = crate::app::update(&mut state, toggle);
    let mut open = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        form(&state, &event, &metrics),
    );
    assert_eq!(open.find("Below the pane").unwrap().bounds(), below);
    assert_eq!(open.find("Beside the pane").unwrap().bounds(), beside);
    assert_eq!(open.find("Save current event").unwrap().bounds(), save);
    assert!(
        !open
            .snapshot(&Theme::Light)
            .unwrap()
            .matches_image(&closed_image)
            .unwrap(),
        "The open menu must actually paint over the unchanged pane"
    );
    std::fs::remove_dir_all(snapshots).unwrap();
    // The menu extends below the scroll pane's 140px clip, yet stays interactive.
    assert!(open.find("Save all events").unwrap().bounds().y > 140.0);
    open.click("Save all events").unwrap();
    let messages: Vec<_> = open.into_messages().collect();
    assert_eq!(messages.len(), 1);
    assert!(matches!(
        messages[0],
        Message::EventEdit(EventEditMessage::EventEditSaveScoped(
            calendar::SaveScope::All
        ))
    ));
    // Clicking outside dismisses the menu without activating the pane beneath it.
    let mut open = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        form(&state, &event, &metrics),
    );
    open.point_at(Point::new(380.0, 350.0));
    open.simulate(iced_test::simulator::click());
    assert!(matches!(
        open.into_messages().next(),
        Some(Message::EventEdit(
            EventEditMessage::EventEditSaveMenuToggled
        ))
    ));
    let mut open = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        form(&state, &event, &metrics),
    );
    open.tap_key(iced::keyboard::key::Named::Escape);
    assert!(matches!(
        open.into_messages().next(),
        Some(Message::EventEdit(
            EventEditMessage::EventEditSaveMenuToggled
        ))
    ));
}

#[test]
fn an_open_event_is_edited_in_the_pane_without_an_edit_popup() {
    let mut state = State::blank();
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    state.calendar.open_event = Some("standup".into());
    let event = state.calendar.event("standup").unwrap();
    state.calendar.edit_draft = Some(calendar::EditDraft::from_event(event));
    let metrics = Metrics::of(Size::new(300.0, 800.0));
    let mut ui = iced_test::Simulator::with_size(
        iced_test::core::Settings::default(),
        Size::new(300.0, 800.0),
        details_pane(&state, &metrics),
    );
    assert!(ui.find("Start Time").is_ok());
    assert!(ui.find("End Time").is_ok());
    assert!(ui.find("Save current event").is_ok());
    assert!(ui.find("Labels").is_ok());
    assert!(ui.find("View / edit description").is_ok() || ui.find("Add description").is_ok());
    assert!(ui.find("Edit").is_err());
}

#[test]
fn event_title_click_enters_rename_and_enter_saves() {
    let mut state = State::blank();
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    let _ = crate::app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened("standup".into())),
    );
    let metrics = Metrics::of(Size::new(400.0, 1000.0));
    let title = state.calendar.event("standup").unwrap().title.clone();
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        details_pane(&state, &metrics),
    );
    ui.click(title.as_str()).unwrap();
    assert!(
        ui.into_messages()
            .any(|m| matches!(m, Message::Event(EventMessage::EventTitleEdit)))
    );
    let _ = crate::app::update(&mut state, Message::Event(EventMessage::EventTitleEdit));
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        details_pane(&state, &metrics),
    );
    ui.click(title.as_str()).unwrap();
    ui.typewrite(" renamed");
    ui.tap_key(iced::keyboard::key::Named::Enter);
    assert!(ui.into_messages().any(|m| matches!(
        m,
        Message::EventEdit(EventEditMessage::EventEditSaveRequested)
    )));
}

#[test]
fn habit_details_show_start_day_repeat_projects_and_location() {
    let mut state = State::blank();
    let mut snapshot = calendar::sample_snapshot();
    let event = snapshot
        .events
        .iter_mut()
        .find(|e| e.id == "standup")
        .unwrap();
    event.details.habit = true;
    event.details.recurrence = vec!["RRULE:FREQ=DAILY".into()];
    state.calendar = calendar::Store::with_snapshot(snapshot);
    let _ = crate::app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened("standup".into())),
    );
    let metrics = Metrics::of(Size::new(440.0, 1800.0));
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        details_pane(&state, &metrics),
    );
    for label in [
        "Projects",
        "Start day",
        "Repeat",
        "Location",
        "Save current event",
    ] {
        assert!(ui.find(label).is_ok(), "{label}");
    }
    assert!(ui.find("End Time").is_err());
    assert!(ui.find("Calendar").is_err());
}

#[test]
fn a_time_block_panel_only_plans_tasks() {
    let mut state = State::blank();
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    state.timeblocking.block = Some("standup".into());
    let event = state.calendar.event("standup").unwrap();
    state.calendar.edit_draft = Some(calendar::EditDraft::from_event(event));
    let metrics = Metrics::of(Size::new(400.0, 800.0));
    let mut ui = iced_test::Simulator::with_size(
        iced_test::core::Settings::default(),
        Size::new(400.0, 800.0),
        crate::ui::timeblocking::panel(&state, &metrics),
    );
    assert!(ui.find("Planned Tasks").is_ok());
    assert!(ui.find("Block details").is_err());
    assert!(ui.find("Save current event").is_err());
}

#[test]
fn the_new_event_pane_shows_the_shared_repeat_controls() {
    let mut state = State::blank();
    let start = calendar::at_clock(chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(), 9, 0);
    state.calendar.draft = Some(calendar::Draft::new(
        "primary".into(),
        start,
        start + chrono::Duration::minutes(45),
    ));
    let metrics = Metrics::of(Size::new(800.0, 600.0));
    // Tall enough that the repeat controls are on screen below the Meeting section.
    let mut ui = iced_test::Simulator::with_size(
        iced_test::core::Settings::default(),
        Size::new(800.0, 1400.0),
        draft_editor(&state, &metrics),
    );
    assert!(ui.find("Repeat").is_ok());
    assert!(ui.find("Does not repeat").is_ok());
    ui.click("Weekly").unwrap();
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::EventDraft(EventDraftMessage::EventDraftRecurrence(
            crate::recurrence::Action::Repeat(crate::recurrence::RepeatMode::Weekly)
        ))
    )));
}

#[test]
fn hiding_google_tasks_preserves_task_rows_inside_time_blocks() {
    use crate::timeblocking::{self as tb, Metadata};
    let mut state = State::blank();
    let mut snapshot = calendar::sample_snapshot();
    let mut block = snapshot
        .events
        .iter()
        .find(|event| event.id == "standup")
        .unwrap()
        .clone();
    block.start = block.start.with_hour(1).unwrap().with_minute(0).unwrap();
    block.end = block.start + chrono::Duration::hours(1);
    state.config.calendar.app_calendar = Some(block.calendar_id.clone());
    let mut task = block.clone();
    task.id = "task:planned".into();
    task.calendar_id = "tasks:list".into();
    task.title = "Assigned task".into();
    task.all_day = true;
    task.details.task_data = Some(crate::task_db::TaskData {
        id: 100,
        metadata: Metadata {
            block: Some(tb::block_key(&block)),
            minutes: 30,
            ..Metadata::default()
        },
    });
    let mut standalone = task.clone();
    standalone.id = "task:standalone".into();
    standalone.all_day = false;
    standalone.start = block.start.with_hour(3).unwrap().with_minute(0).unwrap();
    standalone.end = standalone.start + chrono::Duration::minutes(30);
    standalone.details.description.clear();
    snapshot.events = vec![block, task, standalone];
    state.calendar = calendar::Store::with_snapshot(snapshot);
    let size = Size::new(600.0, 1440.0);
    let metrics = Metrics::of(size);
    for show in [true, false, true] {
        let _ = crate::app::update(
            &mut state,
            Message::Settings(SettingsMessage::ShowTasksToggled(show)),
        );
        assert_eq!(
            state
                .calendar
                .timed_events(&state.config.calendar)
                .any(|event| event.id == "task:standalone"),
            show
        );
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            size,
            day_grid(&state, &metrics, crate::clock::now()),
        );
        // The assigned task row occupies the right half of the 1am block.
        ui.point_at(Point::new(400.0, state.calendar.hour_height + 10.0));
        ui.simulate([
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ]);
        assert!(
            ui.into_messages().any(|message| matches!(message,
            Message::Event(EventMessage::EventOpened(id)) if id == "task:planned")),
            "assigned task must remain visible and clickable with show_tasks={show}"
        );
    }
}

#[test]
fn time_block_regions_dispatch_different_actions() {
    // (x fraction, y fraction, expected: 0 = focus, 1 = plan, 2 = details)
    for (width, columns, x, y, expected) in [
        (600.0, 1, 0.8, 0.1, 0),
        (600.0, 1, 0.8, 0.9, 1),
        (600.0, 1, 0.2, 0.1, 2),
        (600.0, 1, 0.2, 0.9, 2),
        (240.0, 3, 0.9, 0.2, 0),
        (240.0, 3, 0.9, 0.8, 1),
        (240.0, 3, 0.1, 0.5, 2),
    ] {
        let mut canvas = canvas_with_one_block();
        canvas.blocks[0].time_block = true;
        canvas.blocks[0].columns = columns;
        let bounds = canvas.block_rect(&canvas.blocks[0], width);
        let (label, focus, plan) = block_actions(bounds);
        assert_eq!(label.x, bounds.x);
        assert_eq!(focus.x, plan.x);
        assert_eq!(focus.y, bounds.y);
        assert_eq!(focus.height, bounds.height / 2.0);
        assert_eq!(plan.y, bounds.y + focus.height);
        let element: Element<'_, Message> = canvas_widget(canvas).width(width).height(1440).into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            Size::new(width, 1440.0),
            element,
        );
        ui.point_at(Point::new(
            bounds.x + bounds.width * x,
            bounds.y + bounds.height * y,
        ));
        ui.simulate([
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ]);
        let messages: Vec<_> = ui.into_messages().collect();
        match expected {
            0 => assert!(messages.iter().any(|m| matches!(
                m,
                Message::TimeBlocking(crate::timeblocking::Action::View(id)) if id == "standup"
            ))),
            1 => assert!(messages.iter().any(|m| matches!(
                m,
                Message::TimeBlocking(crate::timeblocking::Action::Plan(id)) if id == "standup"
            ))),
            _ => assert!(messages.iter().any(
                |m| matches!(m, Message::Event(EventMessage::EventOpened(id)) if id == "standup")
            )),
        }
    }
}

#[test]
fn a_task_released_over_a_time_block_assigns_it_instead_of_creating_an_event() {
    let mut canvas = canvas_with_one_block();
    canvas.blocks[0].time_block = true;
    canvas.external_drag = true;
    let element: Element<'_, Message> = canvas_widget(canvas).width(240).height(1440).into();
    let mut ui = iced_test::Simulator::with_size(
        iced_test::core::Settings::default(),
        Size::new(240.0, 1440.0),
        element,
    );
    ui.point_at(Point::new(90.0, 550.0));
    ui.simulate([iced::Event::Mouse(mouse::Event::ButtonReleased(
        mouse::Button::Left,
    ))]);
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(messages.iter().any(|m| matches!(m, Message::TimeBlocking(crate::timeblocking::Action::DropBlock(id)) if id == "standup")));
    assert!(!messages.iter().any(|m| matches!(
        m,
        Message::EventDraft(EventDraftMessage::EventDraftStarted { .. })
    )));
}

#[test]
fn task_rows_drag_from_the_title_and_estimate_and_show_grab_cursors() {
    for fraction in [0.1, 0.5, 0.95] {
        let mut canvas = canvas_with_one_block();
        canvas.blocks[0].time_block = true;
        canvas.blocks[0].tasks = vec![("task".into(), "Task title".into(), 30)];
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(600.0, 1440.0));
        let rect = canvas.block_rect(&canvas.blocks[0], bounds.width);
        let start = Point::new(
            rect.x + rect.width + 4.0 + fraction * rect.width,
            rect.y + 10.0,
        );
        let cursor = mouse::Cursor::Available(start);
        assert_eq!(
            canvas::Program::mouse_interaction(&canvas, &DrawState::default(), bounds, cursor),
            mouse::Interaction::Grab
        );
        canvas.external_drag = true;
        assert_eq!(
            canvas::Program::mouse_interaction(&canvas, &DrawState::default(), bounds, cursor),
            mouse::Interaction::Grabbing
        );
        canvas.external_drag = false;
        let element: Element<'_, Message> = canvas_widget(canvas).width(600).height(1440).into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            bounds.size(),
            element,
        );
        ui.point_at(start);
        ui.simulate([iced::Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left,
        ))]);
        let position = Point::new(start.x, start.y + 10.0);
        ui.point_at(position);
        ui.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position })]);
        ui.simulate([iced::Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left,
        ))]);
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(messages.iter().any(|m| matches!(m, Message::TimeBlocking(crate::timeblocking::Action::Drag(id, _)) if id == "task")));
        assert!(
            !messages
                .iter()
                .any(|m| matches!(m, Message::Event(EventMessage::EventOpened(_))))
        );
    }
}

#[test]
fn hour_lines_match_google_calendar() {
    assert_eq!(calendar::hour_label(0), "12 AM");
    assert_eq!(calendar::hour_label(9), "9 AM");
    assert_eq!(calendar::hour_label(12), "12 PM");
    assert_eq!(calendar::hour_label(13), "1 PM");
    assert_eq!(calendar::hour_label(23), "11 PM");
}

#[test]
fn a_late_boundary_moves_the_small_hours_to_the_foot_of_the_day() {
    let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
    let at = |hour, minute| {
        day.and_hms_opt(hour, minute, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap()
    };

    // A 3am boundary: the day opens at 3am, so 9am is 6 hours in.
    assert_eq!(minutes_into_day(at(3, 0), day, 3 * 60), 0);
    assert_eq!(minutes_into_day(at(9, 0), day, 3 * 60), 6 * 60);

    // 1am on the day itself is before the window, so it belongs to the
    // previous day's view and comes back negative to be clamped away.
    assert_eq!(minutes_into_day(at(1, 0), day, 3 * 60), -120);

    // The next morning's small hours belong to this view, at the foot.
    let next = day.succ_opt().unwrap();
    let next_1am = next
        .and_hms_opt(1, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .single()
        .unwrap();
    assert_eq!(minutes_into_day(next_1am, day, 3 * 60), 22 * 60);

    // Midnight keeps the plain mapping.
    assert_eq!(minutes_into_day(at(1, 0), day, 0), 60);
}

#[test]
fn scroll_up_zooms_in_and_scroll_down_zooms_out() {
    let up = zoom_steps(mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 });
    let down = zoom_steps(mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 });

    assert!(up > 0.0);
    assert!(down < 0.0);
    assert!((up + down).abs() < f32::EPSILON);
}

#[test]
fn zooming_keeps_the_row_under_the_pointer_still() {
    // A row 300px down the grid, zoomed in by 1.5x, slides to 450px. The
    // scroll has to move by the 150px difference so it stays under the
    // mouse.
    assert!((zoom_scroll_delta(300.0, 48.0, 72.0) - 150.0).abs() < 0.001);

    // Zooming out pulls the row back up, so the scroll moves the other way.
    assert!((zoom_scroll_delta(300.0, 72.0, 48.0) + 100.0).abs() < 0.001);

    // The row at the very top never moves, whatever the zoom.
    assert_eq!(zoom_scroll_delta(0.0, 48.0, 180.0), 0.0);

    // A no-op zoom, or a degenerate height, asks for no scroll at all.
    assert_eq!(zoom_scroll_delta(300.0, 48.0, 48.0), 0.0);
    assert_eq!(zoom_scroll_delta(300.0, 0.0, 48.0), 0.0);
}

/// A canvas with one 9:00–9:30 block, for the drag tests.
fn canvas_with_one_block() -> DayCanvas {
    DayCanvas {
        time_format: TimeFormat::TwentyFourHour,
        blocks: vec![Block {
            id: "standup".into(),
            title: "Standup".into(),
            when: "09:00 – 09:30".into(),
            start_min: 9 * 60,
            end_min: 9 * 60 + 30,
            column: 0,
            columns: 1,
            column_span: 1,
            color: Color::WHITE,
            editable: true,
            pending: false,
            tasks: Vec::new(),
            time_block: false,
            habit: None,
            habit_completed: None,
        }],
        travel: Vec::new(),
        needle: None,
        alarms: Vec::new(),
        alarm_mode: false,
        hour_height: 60.0,
        gutter: GUTTER,
        compact: false,
        day_start_hour: 0,
        day: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        can_create: true,
        selection: None,
        external_drag: false,
        dragged_task: None,
    }
}

/// A canvas showing `day`, with its alarm mode on or off, for the alarm
/// tests. The pointer is a pixel row per minute because the zoom is 60.
fn alarm_canvas(day: chrono::NaiveDate, alarm_mode: bool) -> DayCanvas {
    let mut canvas = canvas_with_one_block();
    canvas.day = day;
    canvas.alarm_mode = alarm_mode;
    canvas
}

fn today() -> chrono::NaiveDate {
    crate::clock::now().date_naive()
}

fn click_at(canvas: DayCanvas, point: Point) -> Vec<Message> {
    let element: Element<'_, Message> = canvas_widget(canvas).width(600).height(1440).into();
    let mut ui = iced_test::Simulator::with_size(
        iced_test::core::Settings::default(),
        Size::new(600.0, 1440.0),
        element,
    );
    ui.point_at(point);
    ui.simulate([
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
    ]);
    ui.into_messages().collect()
}

/// A trip of `minutes` hanging off the top of the 9:00 standup.
fn trip_before_standup(minutes: u32) -> TravelBox {
    TravelBox {
        leg: crate::travel::Leg { from: 1, to: 2 },
        between: ("Start of day".into(), "Standup".into()),
        label: "Walk".into(),
        anchor_min: 9 * 60,
        below: false,
        minutes,
        column: 0,
        columns: 1,
        column_span: 1,
        time_block: false,
    }
}

#[test]
fn a_travel_box_sits_just_before_its_event_as_tall_as_the_trip() {
    let mut canvas = canvas_with_one_block();
    canvas.travel = vec![trip_before_standup(25)];

    // The grid is a pixel a minute, so 25 minutes ends where 9:00 starts.
    let rect = canvas.travel_rect(&canvas.travel[0], 600.0);
    assert_eq!((rect.y, rect.height), (9.0 * 60.0 - 25.0, 25.0));
    let event = canvas.block_rect(&canvas.blocks[0], 600.0);
    assert_eq!((rect.x, rect.width), (event.x, event.width));

    // A trip too short to hold a line of text is still tall enough to read.
    let short = trip_before_standup(2);
    assert_eq!(canvas.travel_rect(&short, 600.0).height, TRAVEL_MIN_HEIGHT);

    // The trip home hangs off the end of the last event instead.
    let home = TravelBox {
        anchor_min: 9 * 60 + 30,
        below: true,
        ..trip_before_standup(20)
    };
    let rect = canvas.travel_rect(&home, 600.0);
    assert_eq!((rect.y, rect.height), (9.0 * 60.0 + 30.0, 20.0));
}

#[test]
fn clicking_a_travel_box_opens_the_trip_and_not_the_event() {
    let mut canvas = canvas_with_one_block();
    canvas.travel = vec![trip_before_standup(25)];

    let messages = click_at(canvas, Point::new(200.0, 9.0 * 60.0 - 10.0));
    assert!(
        messages.iter().any(|m| matches!(
            m,
            Message::Travel(crate::travel::Action::OpenLeg(leg, Some((from, to))))
                if *leg == (crate::travel::Leg { from: 1, to: 2 })
                    && from == "Start of day"
                    && to == "Standup"
        )),
        "got {messages:?}"
    );
    assert!(
        !messages
            .iter()
            .any(|m| matches!(m, Message::Event(EventMessage::EventOpened(_))))
    );

    // The event itself still opens where the box is not.
    let mut canvas = canvas_with_one_block();
    canvas.travel = vec![trip_before_standup(25)];
    let messages = click_at(canvas, Point::new(200.0, 9.0 * 60.0 + 15.0));
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::Event(EventMessage::EventOpened(id)) if id == "standup"))
    );
}

#[test]
fn the_trips_of_a_day_become_boxes_on_the_events_they_lead_to() {
    let mut state = State::blank();
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    let travel = &mut state.config.travel;
    let home = travel.add_place("Home", "1 Elm St", "g-home", 0.0, 0.0);
    let office = travel.add_place("Office", "2 Oak St", "g-office", 0.0, 1.0);
    travel.home = Some(home);
    travel.api_key = "key".into();
    for event in &mut state.calendar.events {
        if matches!(event.id.as_str(), "standup" | "review") {
            event.details.location = "Office".into();
        }
    }
    let route = crate::travel::Route {
        seconds: 1500,
        meters: 2100,
    };
    state.travel.routes.insert(
        crate::travel::RouteKey {
            from: home,
            to: office,
            mode: crate::config::TravelMode::Walk,
        },
        crate::travel::RouteState::Ready(route),
    );

    let at = |id: &str, start, end| Block {
        id: id.into(),
        title: id.into(),
        when: String::new(),
        start_min: start,
        end_min: end,
        column: 0,
        columns: 1,
        column_span: 1,
        color: Color::WHITE,
        editable: true,
        pending: false,
        tasks: Vec::new(),
        time_block: false,
        habit: None,
        habit_completed: None,
    };
    let blocks = [at("standup", 540, 570), at("review", 840, 900)];

    let found = travel_boxes(&state, &blocks, state.calendar.day);

    // Home to the standup only: the review is at the same place as the
    // standup, so there is nothing to travel for.
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        (found[0].anchor_min, found[0].below, found[0].minutes),
        (540, false, 25)
    );
    assert_eq!(found[0].label, "Walk · 25 min · 2.1 km");
    assert_eq!(
        found[0].between,
        ("Start of day".to_owned(), "Standup".to_owned())
    );

    // The cache key moves when a trip does, so the grid redraws.
    assert_ne!(travel_key(&found), travel_key(&[]));
}

#[test]
fn a_click_in_alarm_mode_sets_the_alarm_at_the_minute_under_the_pointer() {
    let tomorrow = today().succ_opt().unwrap();
    let canvas = alarm_canvas(tomorrow, true);
    let expected = canvas.time_at(10 * 60 + 7);

    let messages = click_at(canvas, Point::new(200.0, 10.0 * 60.0 + 7.0));

    assert!(
        messages.iter().any(|m| matches!(
            m,
            Message::Alarms(alarms::Action::Place(at)) if *at == expected
        )),
        "got {messages:?}"
    );
}

#[test]
fn a_click_in_alarm_mode_never_reaches_the_events_underneath() {
    let tomorrow = today().succ_opt().unwrap();

    // The standup sits at 9:00-9:30, and this lands in the middle of it.
    let messages = click_at(
        alarm_canvas(tomorrow, true),
        Point::new(200.0, 9.0 * 60.0 + 15.0),
    );

    assert!(messages.iter().any(|m| matches!(m, Message::Alarms(_))));
    assert!(
        !messages.iter().any(|m| matches!(
            m,
            Message::Event(EventMessage::EventOpened(_))
                | Message::EventDraft(EventDraftMessage::EventDraftStarted { .. })
        )),
        "got {messages:?}"
    );
}

#[test]
fn a_click_in_alarm_mode_on_a_past_day_sets_nothing() {
    let yesterday = today().pred_opt().unwrap();

    let messages = click_at(alarm_canvas(yesterday, true), Point::new(200.0, 600.0));

    assert!(
        !messages.iter().any(|m| matches!(m, Message::Alarms(_))),
        "got {messages:?}"
    );
}

#[test]
fn the_pointer_says_whether_an_alarm_can_go_where_it_is() {
    let bounds = Rectangle::new(Point::ORIGIN, Size::new(600.0, 1440.0));
    let cursor = mouse::Cursor::Available(Point::new(200.0, 600.0));
    let interaction = |day| {
        canvas::Program::mouse_interaction(
            &alarm_canvas(day, true),
            &DrawState::default(),
            bounds,
            cursor,
        )
    };

    assert_eq!(
        interaction(today().succ_opt().unwrap()),
        mouse::Interaction::Crosshair
    );
    assert_eq!(
        interaction(today().pred_opt().unwrap()),
        mouse::Interaction::NotAllowed
    );
}

#[test]
fn an_alarm_needle_opens_when_its_dot_or_label_is_clicked() {
    let tomorrow = today().succ_opt().unwrap();
    let mut canvas = alarm_canvas(tomorrow, false);
    canvas.alarms = vec![AlarmMark {
        id: 7,
        minutes: 12 * 60,
        label: "12:00".into(),
    }];
    let pill = canvas.alarm_pill(&canvas.alarms[0], 600.0);

    for point in [Point::new(canvas.gutter, 12.0 * 60.0), pill.center()] {
        let messages = click_at(canvas_clone(&canvas), point);

        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Message::Alarms(alarms::Action::Open(7)))),
            "{point:?} gave {messages:?}"
        );
    }
}

#[test]
fn an_alarm_needle_crossing_an_event_leaves_the_event_clickable() {
    let tomorrow = today().succ_opt().unwrap();
    let mut canvas = alarm_canvas(tomorrow, false);
    // Through the middle of the 9:00-9:30 standup.
    canvas.alarms = vec![AlarmMark {
        id: 7,
        minutes: 9 * 60 + 15,
        label: "09:15".into(),
    }];

    let messages = click_at(canvas, Point::new(120.0, 9.0 * 60.0 + 15.0));

    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::Event(EventMessage::EventOpened(id)) if id == "standup")),
        "got {messages:?}"
    );
    assert!(!messages.iter().any(|m| matches!(m, Message::Alarms(_))));
}

#[test]
fn the_needle_line_opens_an_alarm_over_empty_time() {
    let tomorrow = today().succ_opt().unwrap();
    let mut canvas = alarm_canvas(tomorrow, false);
    canvas.alarms = vec![AlarmMark {
        id: 3,
        minutes: 15 * 60,
        label: "15:00".into(),
    }];

    let near = Point::new(150.0, 15.0 * 60.0 + 3.0);
    let far = Point::new(150.0, 15.0 * 60.0 + 20.0);

    assert_eq!(canvas.alarm_at(near, 600.0), Some(3));
    assert_eq!(canvas.alarm_at(far, 600.0), None);
}

fn canvas_clone(canvas: &DayCanvas) -> DayCanvas {
    DayCanvas {
        time_format: canvas.time_format,
        blocks: canvas.blocks.clone(),
        alarms: canvas.alarms.clone(),
        ..alarm_canvas(canvas.day, canvas.alarm_mode)
    }
}

#[test]
fn the_cache_key_holds_until_a_traced_detail_changes() {
    let block = |title: &str, when: &str, alpha: f32| Block {
        id: title.to_owned(),
        title: title.to_owned(),
        when: when.to_owned(),
        start_min: 9 * 60,
        end_min: 9 * 60 + 30,
        column: 0,
        columns: 1,
        column_span: 1,
        color: Color {
            a: alpha,
            ..Color::WHITE
        },
        editable: true,
        pending: false,
        tasks: Vec::new(),
        time_block: false,
        habit: None,
        habit_completed: None,
    };

    let key = cache_key(
        &[block("Standup", "09:00 – 09:30", 1.0)],
        48.0,
        GUTTER,
        false,
        0,
        None,
    );

    // A key that drifts between frames would retrace the whole day while
    // scrolling, and one that ignores a label would draw it stale.
    assert_eq!(
        key,
        cache_key(
            &[block("Standup", "09:00 – 09:30", 1.0)],
            48.0,
            GUTTER,
            false,
            0,
            None,
        )
    );

    for changed in [
        block("Lunch", "09:00 – 09:30", 1.0),
        block("Standup", "09:00 – 10:00", 1.0),
        block("Standup", "09:00 – 09:30", 0.5),
    ] {
        assert_ne!(
            key,
            cache_key(std::slice::from_ref(&changed), 48.0, GUTTER, false, 0, None),
            "a changed block has to be retraced"
        );
    }

    // Moving the day's boundary relabels every hour, so the grid has to be
    // retraced even though the events did not move.
    assert_ne!(
        key,
        cache_key(
            &[block("Standup", "09:00 – 09:30", 1.0)],
            48.0,
            GUTTER,
            false,
            3,
            None,
        ),
        "a new day boundary has to relabel the grid"
    );

    // An unconfirmed write lightens the chip, and a drag leaves the dragged
    // block out of the cached grid. Both have to retrace.
    let pending = Block {
        pending: true,
        ..block("Standup", "09:00 – 09:30", 1.0)
    };
    assert_ne!(
        key,
        cache_key(std::slice::from_ref(&pending), 48.0, GUTTER, false, 0, None),
        "a pending write has to retrace the lighter chip"
    );
    assert_ne!(
        key,
        cache_key(
            &[block("Standup", "09:00 – 09:30", 1.0)],
            48.0,
            GUTTER,
            false,
            0,
            Some(0),
        ),
        "a drag has to leave the dragged block out of the cached grid"
    );
}

#[test]
fn a_wrapped_title_measures_taller_than_a_single_line() {
    // The time label is placed below the title using this measurement, so a
    // title that wraps to two lines has to report more than one line's
    // height or the time would be drawn through the second line.
    let one_line = wrap_title("Standup", 12.0, 200.0);
    let two_lines = wrap_title("Programming Team Meeting", 12.0, 90.0);

    assert_eq!(one_line.len(), 1);
    assert!(
        two_lines.len() > one_line.len(),
        "a wrapped title should take more lines than a single one"
    );
}

#[test]
fn a_click_in_the_top_half_opens_the_details_below_it() {
    record_pointer(40.0, 400.0);
    assert!(click_was_in_the_top_half());
}

#[test]
fn a_click_in_the_bottom_half_opens_the_details_above_it() {
    record_pointer(360.0, 400.0);
    assert!(!click_was_in_the_top_half());
}

#[test]
fn without_a_pointer_sample_the_details_stay_clear_of_the_header() {
    record_pointer(0.0, 0.0);
    assert!(click_was_in_the_top_half());
}

#[test]
fn the_pointer_hits_the_event_under_it() {
    let canvas = canvas_with_one_block();

    let rect = canvas.block_rect(&canvas.blocks[0], 240.0);
    assert_eq!(
        canvas.hit(Some(Point::new(rect.x + 4.0, rect.y + 4.0)), 240.0),
        Some(0)
    );
    assert_eq!(canvas.hit(Some(Point::new(8.0, rect.y + 4.0)), 240.0), None);
}

#[test]
fn the_edges_resize_and_the_middle_moves() {
    let canvas = canvas_with_one_block();
    let rect = canvas.block_rect(&canvas.blocks[0], 240.0);
    let at = |y: f32| canvas.grab(Point::new(rect.x + 4.0, y), 240.0);

    assert_eq!(at(rect.y + 1.0), Some((0, DragMode::ResizeStart)));
    assert_eq!(
        at(rect.y + rect.height - 1.0),
        Some((0, DragMode::ResizeEnd))
    );
    assert_eq!(at(rect.y + rect.height / 2.0), Some((0, DragMode::Move)));
    assert_eq!(canvas.grab(Point::new(8.0, rect.y + 1.0), 240.0), None);
}

#[test]
fn a_short_block_still_has_a_middle_to_grab() {
    let mut canvas = canvas_with_one_block();
    // A 15-minute block at the shortest hour row is 16px tall, so a fixed
    // 8px edge zone would leave no middle at all.
    canvas.hour_height = 24.0;
    canvas.blocks[0].end_min = canvas.blocks[0].start_min + 15;
    let rect = canvas.block_rect(&canvas.blocks[0], 240.0);
    assert_eq!(rect.height, 16.0);

    assert_eq!(
        canvas.grab(Point::new(rect.x + 4.0, rect.y + rect.height / 2.0), 240.0),
        Some((0, DragMode::Move))
    );
}

#[test]
fn moving_keeps_the_length_and_snaps_to_the_quarter_hour() {
    let canvas = canvas_with_one_block();
    let drag = Drag {
        id: "standup".into(),
        mode: DragMode::Move,
        origin_y: 0.0,
        start_min: 9 * 60,
        end_min: 9 * 60 + 30,
        preview_start: 9 * 60,
        preview_end: 9 * 60 + 30,
    };

    // One hour down the grid is 60px at 60px/hour, so the event lands an
    // hour later with the same 30-minute length.
    let moved = canvas.preview(&drag, 60.0);
    assert_eq!(moved.preview_start, 10 * 60);
    assert_eq!(moved.preview_end, 10 * 60 + 30);

    // A press that does not move leaves the times alone.
    assert_eq!(canvas.preview(&drag, 0.0).preview_start, 9 * 60);

    // A few pixels of travel is not a whole snap step.
    assert_eq!(canvas.preview(&drag, 5.0).preview_start, 9 * 60);
}

#[test]
fn resizing_the_top_keeps_the_end_and_the_bottom_keeps_the_start() {
    let canvas = canvas_with_one_block();
    let drag = |mode| Drag {
        id: "standup".into(),
        mode,
        origin_y: 0.0,
        start_min: 9 * 60,
        end_min: 9 * 60 + 30,
        preview_start: 9 * 60,
        preview_end: 9 * 60 + 30,
    };

    // Pulling the top up by 30 minutes moves the start and leaves the end.
    let top = canvas.preview(&drag(DragMode::ResizeStart), -30.0);
    assert_eq!(top.preview_start, 8 * 60 + 30);
    assert_eq!(top.preview_end, 9 * 60 + 30);

    // Pulling the bottom down by 30 minutes moves the end and leaves the
    // start.
    let bottom = canvas.preview(&drag(DragMode::ResizeEnd), 30.0);
    assert_eq!(bottom.preview_start, 9 * 60);
    assert_eq!(bottom.preview_end, 10 * 60);
}

#[test]
fn a_resize_cannot_invert_the_event() {
    let canvas = canvas_with_one_block();
    let drag = Drag {
        id: "standup".into(),
        mode: DragMode::ResizeStart,
        origin_y: 0.0,
        start_min: 9 * 60,
        end_min: 9 * 60 + 30,
        preview_start: 9 * 60,
        preview_end: 9 * 60 + 30,
    };

    // Dragging the top far past the bottom stops a snap step short of it.
    let squashed = canvas.preview(&drag, 600.0);
    assert_eq!(squashed.preview_end, 9 * 60 + 30);
    assert_eq!(squashed.preview_start, 9 * 60 + 30 - MIN_DURATION);
}

#[test]
fn a_previewed_minute_becomes_a_real_time() {
    let canvas = canvas_with_one_block();

    // Midnight boundary: minute 540 is 9:00 on the day in view.
    let nine = canvas.time_at(9 * 60);
    assert_eq!(nine.date_naive(), canvas.day);
    assert_eq!(nine.hour(), 9);
    assert_eq!(nine.minute(), 0);

    // A 3am boundary shifts the window, so minute 0 is 3am.
    let mut late = canvas_with_one_block();
    late.day_start_hour = 3;
    let three = late.time_at(0);
    assert_eq!(three.hour(), 3);
    assert_eq!(late.range_label(0, 60), "03:00 – 04:00");

    // A shared period is only shown once, as in "8 – 10p".
    late.time_format = TimeFormat::Compact;
    assert_eq!(late.range_label(17 * 60, 19 * 60), "8 – 10p");
    assert_eq!(late.range_label(8 * 60, 10 * 60), "11a – 1p");
}

#[test]
fn a_click_on_empty_space_gets_a_default_length() {
    let canvas = canvas_with_one_block();
    // 9:00 is 540px down at 60px/hour; the press and release are the same
    // pixel, so this is a click rather than a drag.
    let create = CreateDrag {
        anchor_min: canvas.snap_minute(540.0),
        head_min: canvas.snap_minute(540.0),
    };

    assert_eq!(canvas.create_range(&create), (9 * 60, 10 * 60));
}

#[test]
fn a_drag_on_empty_space_spans_what_was_drawn() {
    let canvas = canvas_with_one_block();
    let create = CreateDrag {
        anchor_min: canvas.snap_minute(540.0),
        head_min: canvas.snap_minute(660.0),
    };

    assert_eq!(canvas.create_range(&create), (9 * 60, 11 * 60));
}

#[test]
fn a_drag_upwards_still_spans_forwards() {
    let canvas = canvas_with_one_block();
    let create = CreateDrag {
        anchor_min: canvas.snap_minute(660.0),
        head_min: canvas.snap_minute(540.0),
    };

    assert_eq!(canvas.create_range(&create), (9 * 60, 11 * 60));
}

#[test]
fn a_new_event_never_runs_off_the_end_of_the_day() {
    let canvas = canvas_with_one_block();
    // A click at the very foot of the day still fits a full default span.
    let create = CreateDrag {
        anchor_min: canvas.snap_minute(24.0 * 60.0),
        head_min: canvas.snap_minute(24.0 * 60.0),
    };
    let (start, end) = canvas.create_range(&create);

    assert!(end <= DAY_MINUTES);
    assert!(end - start >= MIN_DURATION);
}

#[test]
fn the_new_event_ends_snap_to_the_quarter_hour() {
    let canvas = canvas_with_one_block();
    // 9:07 lands on 9:00, 9:08 on 9:15.
    assert_eq!(canvas.snap_minute(9.0 * 60.0 + 7.0), 9 * 60);
    assert_eq!(canvas.snap_minute(9.0 * 60.0 + 8.0), 9 * 60 + 15);
}

#[test]
fn a_pending_chip_is_lighter_than_a_final_one() {
    let color = Color::from_rgb8(0xDC, 0x21, 0x27);
    let lighter = lighten(color);

    assert!(lighter.r > color.r && lighter.g > color.g && lighter.b > color.b);
    assert_eq!(lighter.a, color.a);
    // White cannot get any lighter.
    assert_eq!(lighten(Color::WHITE), Color::WHITE);
}

#[test]
fn a_block_only_holds_as_many_lines_as_it_has_room_for() {
    // A one-hour block at the default zoom is 48 pixels tall, which holds
    // two lines of a 12 pixel title.
    let (top, room) = text_room(48.0, 12.0);
    assert_eq!(room, 2);
    assert!(top + room as f32 * line_height(12.0) <= 48.0);

    // The shortest block still holds a single line, and the gap above it
    // shrinks so that the line fits inside.
    let (top, room) = text_room(16.0, 12.0);
    assert_eq!(room, 1);
    assert!(top + room as f32 * line_height(12.0) <= 16.0);

    // A block with no room at all holds nothing.
    let (_, room) = text_room(4.0, 12.0);
    assert_eq!(room, 0);
}

#[test]
fn a_title_that_fits_stays_on_one_line() {
    let lines = wrap_title("Standup", 12.0, 200.0);

    assert_eq!(lines, vec!["Standup".to_owned()]);
}

#[test]
fn every_wrapped_line_stays_inside_the_block() {
    // A title with a line break of its own, a word too long for the block
    // and a word that only just fits.
    let title = "17-4 HW10/22 (Thu)2pm: L18\nReading.Inaka vs Tokai-->Changed\nfrom the L18 department store";
    let width = 120.0;

    for line in wrap_title(title, 12.0, width) {
        assert!(
            text_width_of(&line, 12.0) <= width,
            "{line:?} is wider than the block"
        );
    }
}

#[test]
fn a_word_too_long_for_the_block_is_broken_up() {
    let word = "Supercalifragilisticexpialidociousandthensomemoreletters";
    let width = 120.0;

    let lines = wrap_title(word, 12.0, width);

    assert!(lines.len() > 1, "the word should have been broken up");
    for line in &lines {
        assert!(text_width_of(line, 12.0) <= width, "{line:?} is too wide");
    }
}

#[test]
fn a_title_too_long_for_the_block_is_cut_short_with_an_ellipsis() {
    let title =
        "17-4 HW10/22 (Thu)2pm: L18 Reading.Inaka vs Tokai-->Changed from the L18 department store";

    let lines = fit_lines(title, 12.0, 120.0, 2);

    assert_eq!(lines.len(), 2);
    assert!(lines.last().expect("a last line").ends_with(ELLIPSIS));
    for line in &lines {
        assert!(text_width_of(line, 12.0) <= 120.0, "{line:?} is too wide");
    }
}

#[test]
fn a_title_with_no_room_is_not_drawn_at_all() {
    let lines = fit_lines("Standup", 12.0, 120.0, 0);

    assert!(lines.is_empty());
}

#[test]
fn a_title_that_fits_is_left_alone() {
    let title = "Standup";

    let lines = fit_lines(title, 12.0, 200.0, 3);

    assert_eq!(lines, vec!["Standup".to_owned()]);
}

#[test]
fn a_line_that_does_not_fit_is_cut_with_an_ellipsis() {
    let line = fit_line("01:00 – 02:00", 11.0, 40.0);

    assert!(line.ends_with(ELLIPSIS));
    assert!(text_width_of(&line, 11.0) <= 40.0);
}

#[test]
fn a_narrow_block_keeps_the_start_of_the_time() {
    // The whole range fits.
    assert_eq!(
        time_label("09:15 – 11:00", 11.0, 200.0),
        Some("09:15 – 11:00".to_owned())
    );

    // Too narrow for the range, but the start time still fits.
    assert_eq!(
        time_label("09:15 – 11:00", 11.0, 40.0),
        Some("09:15".to_owned())
    );

    // Too narrow for either, so there is no time at all.
    assert_eq!(time_label("09:15 – 11:00", 11.0, 12.0), None);
}

#[test]
fn month_and_two_week_rows_grow_with_the_zoom() {
    let month = month_cell_height(900.0, 5, 1.0);
    assert_eq!(month, 148.0);
    assert_eq!(month_cell_height(900.0, 5, 2.0), month * 2.0);
    // A short window still scrolls at the same floor before any zoom.
    assert_eq!(month_cell_height(300.0, 5, 1.0), 135.0);

    assert_eq!(two_week_height(800.0, 1.0), 800.0);
    assert_eq!(two_week_height(800.0, 1.5), 1200.0);
    assert_eq!(two_week_height(100.0, 1.0), 270.0);
}

fn zoom_messages(ui: iced_test::Simulator<'_, Message>) -> Vec<(f32, Option<f32>)> {
    ui.into_messages()
        .filter_map(|message| match message {
            Message::Calendar(CalendarMessage::CalendarZoomed { factor, anchor }) => {
                Some((factor, anchor))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn ctrl_and_the_wheel_zoom_the_month_grid_and_the_wheel_alone_does_not() {
    use iced::keyboard::{Event as KeyEvent, Modifiers};
    let wheel = iced::Event::Mouse(mouse::Event::WheelScrolled {
        delta: mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
    });
    let metrics = Metrics::of(Size::new(1000.0, 800.0));

    let mut state = State::blank();
    state.view = crate::app::CalendarView::Month;
    state.grid_scroll = (100.0, 600.0);

    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        monthly_view(&state, &metrics),
    );
    ui.point_at(Point::new(500.0, 400.0));
    let _ = ui.simulate([wheel.clone()]);
    assert!(zoom_messages(ui).is_empty(), "a bare wheel only scrolls");

    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        monthly_view(&state, &metrics),
    );
    ui.point_at(Point::new(500.0, 400.0));
    let _ = ui.simulate([
        iced::Event::Keyboard(KeyEvent::ModifiersChanged(Modifiers::COMMAND)),
        wheel,
    ]);
    let zooms = zoom_messages(ui);
    assert_eq!(zooms.len(), 1);
    assert!(zooms[0].0 > 1.0, "scrolling up zooms in");
    // The anchor is a row of the grid, so it counts the scroll above the view.
    assert!(zooms[0].1.is_some_and(|anchor| anchor > 100.0));
}

#[test]
fn the_menu_zoom_section_offers_only_what_can_still_happen() {
    let metrics = Metrics::of(Size::new(400.0, 1400.0));

    let clicked = |state: &State, label: &str| -> Vec<Message> {
        let mut ui = iced_test::Simulator::with_size(
            Default::default(),
            metrics.size,
            menu_overlay(state, &metrics),
        );
        ui.click(label).unwrap();
        ui.into_messages().collect()
    };
    let resets = |messages: &[Message]| {
        messages.iter().any(|message| {
            matches!(
                message,
                Message::Calendar(CalendarMessage::CalendarZoomReset)
            )
        })
    };
    let zoomed = |messages: &[Message]| {
        messages
            .iter()
            .filter_map(|message| match message {
                Message::Calendar(CalendarMessage::CalendarZoomed { factor, anchor }) => {
                    Some((*factor, *anchor))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };

    let mut state = State::blank();
    // The sheet is capped to the window, and the section must be on screen.
    state.main_size = metrics.size;
    let zoom_in = clicked(&state, "Zoom in");
    assert_eq!(zoomed(&zoom_in), [(calendar::ZOOM_STEP, None)]);
    let zoom_out = clicked(&state, "Zoom out");
    assert_eq!(zoomed(&zoom_out), [(1.0 / calendar::ZOOM_STEP, None)]);
    assert!(
        !resets(&clicked(&state, "Reset zoom")),
        "already at default"
    );

    state.calendar.hour_height = calendar::MAX_HOUR_HEIGHT;
    assert!(zoomed(&clicked(&state, "Zoom in")).is_empty());
    assert!(resets(&clicked(&state, "Reset zoom")));

    state.calendar.hour_height = calendar::MIN_HOUR_HEIGHT;
    assert!(zoomed(&clicked(&state, "Zoom out")).is_empty());
}

#[test]
fn ctrl_and_the_wheel_zoom_the_day_grid_around_the_pointer() {
    use iced::keyboard::{Event as KeyEvent, Modifiers};
    let metrics = Metrics::of(Size::new(800.0, 700.0));
    let state = State::blank();
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        day_grid(&state, &metrics, crate::clock::now()),
    );
    ui.point_at(Point::new(400.0, 300.0));
    let _ = ui.simulate([
        iced::Event::Keyboard(KeyEvent::ModifiersChanged(Modifiers::COMMAND)),
        iced::Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
        }),
    ]);
    let zooms = zoom_messages(ui);
    assert_eq!(zooms.len(), 1);
    assert!(zooms[0].0 < 1.0, "scrolling down zooms out");
    assert!(zooms[0].1.is_some());
}

#[test]
fn the_month_zoom_layer_lets_clicks_through_to_the_days() {
    let metrics = Metrics::of(Size::new(1000.0, 800.0));
    let mut state = State::blank();
    state.view = crate::app::CalendarView::Month;
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        monthly_view(&state, &metrics),
    );
    ui.click("15").unwrap();
    // A cell wraps what it publishes with the day it belongs to.
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::Calendar(CalendarMessage::WeekDayAction(_, inner))
            if matches!(*inner, Message::Calendar(CalendarMessage::CalendarDayOpened(_)))
    )));
}

#[test]
fn shift_and_the_wheel_do_not_zoom_the_day_grid() {
    use iced::keyboard::{Event as KeyEvent, Modifiers};
    let metrics = Metrics::of(Size::new(800.0, 700.0));
    let state = State::blank();
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        day_grid(&state, &metrics, crate::clock::now()),
    );
    ui.point_at(Point::new(400.0, 300.0));
    let _ = ui.simulate([
        iced::Event::Keyboard(KeyEvent::ModifiersChanged(Modifiers::SHIFT)),
        iced::Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
        }),
    ]);
    assert!(zoom_messages(ui).is_empty());
}

fn search_state() -> State {
    let mut state = State::blank();
    state.screen = crate::app::Screen::Calendar;
    state.main_size = Size::new(800.0, 1200.0);
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    state
}

fn search_action(state: &mut State, action: calendar::search::Action) {
    let _ = crate::app::update(
        state,
        Message::Calendar(CalendarMessage::CalendarSearch(action)),
    );
}

#[test]
fn view_sits_beside_actions_on_every_header_width() {
    let state = search_state();
    for width in [180.0, 260.0, 320.0, 539.0, 540.0, 900.0] {
        let metrics = Metrics::of(Size::new(width, 700.0));
        for weekly in [false, true] {
            let mut ui = iced_test::Simulator::with_size(
                Default::default(),
                metrics.size,
                header(&state, &metrics, weekly),
            );
            assert!(ui.find("View").is_ok(), "header at {width}");
            assert!(ui.find("Actions").is_ok(), "header at {width}");
            assert!(ui.find("Set Alarm").is_err(), "header at {width}");
        }
        let mut ui = iced_test::Simulator::with_size(
            Default::default(),
            metrics.size,
            range_heading(&state, "Oct 2026 ▾".into(), &metrics),
        );
        assert!(ui.find("View").is_ok(), "range heading at {width}");
        assert!(ui.find("Actions").is_ok(), "range heading at {width}");
    }
}

#[test]
fn search_popup_shows_each_section_and_reports_the_query() {
    let mut state = search_state();
    search_action(&mut state, calendar::search::Action::Toggle);
    assert!(state.calendar.search.open);
    let metrics = Metrics::of(state.main_size);
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        search_overlay(&state, &metrics),
    );
    for expected in [
        "Search description",
        "Search filters",
        "Time of day",
        "Projects",
        "Labels",
        "Event filters",
        "Show events that are",
        "Only show events with public visibility",
        "Location",
    ] {
        assert!(ui.find(expected).is_ok(), "{expected} is missing");
    }
    ui.click(iced::widget::Id::new(SEARCH_INPUT)).unwrap();
    ui.typewrite("st");
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::Calendar(CalendarMessage::CalendarSearch(
            calendar::search::Action::Query(_)
        ))
    )));
}

#[test]
fn search_narrows_each_view_while_a_filter_is_on() {
    const TITLE: &str = "All-day search target";
    let mut state = search_state();
    state.week_anchor = state.calendar.day;
    let mut target = state.calendar.event("standup").unwrap().clone();
    target.id = "search-target".into();
    target.title = TITLE.into();
    target.all_day = true;
    state.calendar.events.push(target);
    let metrics = Metrics::of(Size::new(1400.0, 1000.0));

    fn shown(element: Element<'_, Message>, metrics: &Metrics) -> bool {
        iced_test::Simulator::with_size(Default::default(), metrics.size, element)
            .find(TITLE)
            .is_ok()
    }
    type Render = for<'a> fn(&'a State, &Metrics) -> Element<'a, Message>;
    let views: [(&str, Render); 4] = [
        ("day", view),
        ("week", weekly_view),
        ("two weeks", two_week_view),
        ("month", monthly_view),
    ];
    for (name, render) in views {
        assert!(
            shown(render(&state, &metrics), &metrics),
            "{name}, no search"
        );
    }

    search_action(
        &mut state,
        calendar::search::Action::Query("no event has this title".into()),
    );
    for (name, render) in views {
        assert!(
            !shown(render(&state, &metrics), &metrics),
            "{name}, no match"
        );
    }

    search_action(
        &mut state,
        calendar::search::Action::Query("SEARCH TARGET".into()),
    );
    for (name, render) in views {
        assert!(shown(render(&state, &metrics), &metrics), "{name}, a match");
    }
}

#[test]
fn search_popup_and_the_other_sheets_replace_each_other() {
    let mut state = search_state();
    state.calendar.menu_open = true;
    state.calendar.month_open = true;
    state.overview_month_open = true;
    search_action(&mut state, calendar::search::Action::Toggle);
    assert!(state.calendar.search.open);
    assert!(!state.calendar.menu_open);
    assert!(!state.calendar.month_open);
    assert!(!state.overview_month_open);

    let _ = crate::app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarMenuToggled),
    );
    assert!(state.calendar.menu_open);
    assert!(!state.calendar.search.open);
}

#[test]
fn the_view_sheet_switches_tabs_and_actions_offers_sync_above_set_alarm() {
    let mut state = search_state();
    let metrics = Metrics::of(state.main_size);
    let clicked = |element: Element<'_, Message>, label: &str| -> Vec<Message> {
        let mut ui = iced_test::Simulator::with_size(Default::default(), metrics.size, element);
        ui.click(label).unwrap();
        ui.into_messages().collect()
    };

    // View opens on the Calendars tab, and Filter switches to the search.
    assert!(matches!(
        clicked(header(&state, &metrics, false), "View").as_slice(),
        [Message::Calendar(CalendarMessage::CalendarMenuToggled)]
    ));
    let _ = crate::app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarMenuToggled),
    );
    let messages = clicked(menu_overlay(&state, &metrics), "Filter");
    for message in messages {
        let _ = crate::app::update(&mut state, message);
    }
    assert!(state.calendar.search.open);
    assert!(!state.calendar.menu_open);
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        search_overlay(&state, &metrics),
    );
    assert!(ui.find("Calendars").is_ok());
    drop(ui);

    // Actions puts the View sheet away and holds Sync now above Set Alarm.
    let _ = crate::app::update(
        &mut state,
        Message::Calendar(CalendarMessage::CalendarActionsToggled),
    );
    assert!(state.calendar.actions_open);
    assert!(!state.calendar.search.open);
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        actions_overlay(&state, &metrics, true),
    );
    let sync = ui.find("Sync now").unwrap().bounds();
    let alarm = ui.find("Set Alarm").unwrap().bounds();
    assert!(sync.y < alarm.y, "Sync now sits above Set Alarm");
    assert!((sync.width - alarm.width).abs() < 0.5, "the two look alike");
    drop(ui);
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        menu_overlay(&state, &metrics),
    );
    assert!(
        ui.find("Sync now").is_err(),
        "sync has left the Calendars tab"
    );
}

#[test]
fn closing_the_search_popup_leaves_the_filter_on() {
    let mut state = search_state();
    search_action(&mut state, calendar::search::Action::Toggle);
    search_action(
        &mut state,
        calendar::search::Action::Query("standup".into()),
    );
    search_action(&mut state, calendar::search::Action::Toggle);
    assert!(!state.calendar.search.open);
    assert!(state.calendar.search.is_active());
}

#[test]
fn search_ignores_a_project_that_no_longer_exists() {
    let mut state = search_state();
    search_action(
        &mut state,
        calendar::search::Action::Projects(calendar::projects::Action::Add(u64::MAX)),
    );
    assert!(state.calendar.search.projects.ids.is_empty());
}

fn detail_state() -> (State, Metrics) {
    let mut state = search_state();
    state.week_anchor = state.calendar.day;
    let standup = state.calendar.event("standup").unwrap().clone();
    for (id, title, visibility) in [
        (
            "secret",
            "Secret all-day",
            calendar::event_options::Visibility::Private,
        ),
        (
            "open",
            "Open all-day",
            calendar::event_options::Visibility::Public,
        ),
    ] {
        let mut event = standup.clone();
        event.id = id.into();
        event.title = title.into();
        event.all_day = true;
        event.details.options.visibility = visibility;
        state.calendar.events.push(event);
    }
    (state, Metrics::of(Size::new(1400.0, 1000.0)))
}

#[test]
fn hidden_details_keep_private_events_on_screen_in_each_view() {
    fn has(element: Element<'_, Message>, metrics: &Metrics, wanted: &str) -> bool {
        iced_test::Simulator::with_size(Default::default(), metrics.size, element)
            .find(wanted)
            .is_ok()
    }
    type Render = for<'a> fn(&'a State, &Metrics) -> Element<'a, Message>;
    let views: [(&str, Render); 4] = [
        ("day", view),
        ("week", weekly_view),
        ("two weeks", two_week_view),
        ("month", monthly_view),
    ];
    let (mut state, metrics) = detail_state();
    for (name, render) in views {
        assert!(
            has(render(&state, &metrics), &metrics, "Secret all-day"),
            "{name}"
        );
        assert!(
            !has(render(&state, &metrics), &metrics, PRIVATE_TITLE),
            "{name}"
        );
    }

    search_action(
        &mut state,
        calendar::search::Action::DetailsByVisibility(true),
    );
    for (name, render) in views {
        assert!(
            !has(render(&state, &metrics), &metrics, "Secret all-day"),
            "{name}"
        );
        assert!(
            has(render(&state, &metrics), &metrics, PRIVATE_TITLE),
            "{name}"
        );
        assert!(
            has(render(&state, &metrics), &metrics, "Open all-day"),
            "{name}"
        );
    }
}

#[test]
fn hidden_details_cannot_be_opened_and_close_a_pane_showing_them() {
    let (mut state, _) = detail_state();
    let open = |state: &mut State, id: &str| {
        let _ = crate::app::update(
            state,
            Message::Event(EventMessage::EventOpened(id.to_owned())),
        );
    };
    open(&mut state, "secret");
    assert_eq!(state.calendar.open_event.as_deref(), Some("secret"));

    search_action(
        &mut state,
        calendar::search::Action::DetailsByVisibility(true),
    );
    assert_eq!(state.calendar.open_event, None, "the pane closes");
    assert!(state.calendar.edit_draft.is_none());

    open(&mut state, "secret");
    assert_eq!(state.calendar.open_event, None, "and stays closed");
    open(&mut state, "open");
    assert_eq!(state.calendar.open_event.as_deref(), Some("open"));
}

#[test]
fn a_hidden_task_chip_shows_neither_its_title_nor_a_way_to_complete_it() {
    let (mut state, metrics) = detail_state();
    let mut task = state.calendar.event("standup").unwrap().clone();
    task.id = "task:hidden".into();
    task.calendar_id = "tasks:list".into();
    task.title = "Hidden task".into();
    search_action(
        &mut state,
        calendar::search::Action::DetailsByVisibility(true),
    );
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        weekly_task_chip(&state, &task, &metrics),
    );
    assert!(ui.find("Hidden task").is_err());
    assert!(ui.find(PRIVATE_TITLE).is_ok());
    let _ = ui.click("○");
    assert!(
        !ui.into_messages()
            .any(|m| matches!(m, Message::Task(TaskMessage::TaskToggled(_))))
    );
}

/// What pressing the mouse at `position` produces.
fn messages_after_click(
    overlay: Element<'_, Message>,
    size: Size,
    position: iced::Point,
) -> Vec<Message> {
    let mut ui = iced_test::Simulator::with_size(Default::default(), size, overlay);
    ui.point_at(position);
    ui.simulate([
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
    ]);
    ui.into_messages().collect()
}

#[test]
fn clicking_a_popups_background_keeps_it_open_but_clicking_outside_closes_it() {
    let mut state = search_state();
    search_action(&mut state, calendar::search::Action::Toggle);
    state.main_size = Size::new(900.0, 800.0);
    let metrics = Metrics::of(state.main_size);
    // In each card's own padding, clear of every control.
    let top = header_offset(&state, &metrics) + metrics.padding + 2.0;
    // Both tabs share one card, hung at the left below the header.
    let search_inside = iced::Point::new(metrics.padding * 2.0 + 2.0, top);
    let menu_inside = search_inside;
    let outside = iced::Point::new(5.0, metrics.size.height - 5.0);
    let dismissed = |messages: &[Message]| {
        messages.iter().any(|message| {
            matches!(
                message,
                Message::Calendar(
                    CalendarMessage::CalendarSearch(calendar::search::Action::Toggle)
                        | CalendarMessage::CalendarMenuToggled
                )
            )
        })
    };
    for (overlay, inside) in [
        (search_overlay(&state, &metrics), search_inside),
        (menu_overlay(&state, &metrics), menu_inside),
    ] {
        assert!(!dismissed(&messages_after_click(
            overlay,
            metrics.size,
            inside
        )));
    }
    for overlay in [
        search_overlay(&state, &metrics),
        menu_overlay(&state, &metrics),
    ] {
        assert!(dismissed(&messages_after_click(
            overlay,
            metrics.size,
            outside
        )));
    }
}

#[test]
fn the_popup_offers_the_detail_filter_and_the_button_stays_lit_for_it() {
    let (mut state, metrics) = detail_state();
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        search_overlay(&state, &metrics),
    );
    assert!(ui.find("Detail filter").is_ok());
    assert!(ui.find("Show details by visibility").is_ok());
    drop(ui);
    search_action(
        &mut state,
        calendar::search::Action::DetailsByVisibility(true),
    );
    assert!(state.calendar.search.is_modified());
}

#[test]
fn the_event_pane_offers_a_calendar_dropdown_without_timeslay() {
    let mut state = State::blank();
    let mut snapshot = calendar::sample_snapshot();
    snapshot.calendars.push(calendar::Calendar {
        id: "app".into(),
        name: "TimeSlay".into(),
        color: calendar::colors::DEFAULT_EVENT,
        primary: false,
        selected_by_google: true,
    });
    state.calendar = calendar::Store::with_snapshot(snapshot);
    let _ = crate::app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened("standup".into())),
    );
    let event = state.calendar.event("standup").unwrap().clone();
    let current = event.calendar_id.clone();
    let other = state
        .calendar
        .calendars
        .iter()
        .find(|calendar| calendar.id != current && calendar.id != "app")
        .unwrap()
        .clone();
    let name = state
        .calendar
        .calendars
        .iter()
        .find(|calendar| calendar.id == current)
        .unwrap()
        .name
        .clone();
    let metrics = Metrics::of(Size::new(1200.0, 900.0));

    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        Size::new(400.0, 1200.0),
        event_editor(&state, &event, &metrics),
    );
    // The title's chip also names the calendar, so aim for the dropdown
    // beneath its "Calendar" caption instead.
    assert!(ui.find(name.as_str()).is_ok());
    let caption = ui.find("Calendar").unwrap().bounds();
    ui.point_at(iced::Point::new(
        caption.x + caption.width / 2.0,
        caption.y + caption.height + 12.0,
    ));
    let _ = ui.simulate(iced_test::simulator::click());
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::EventEdit(EventEditMessage::EventEditCalendarDropdownToggled)
    )));

    let _ = crate::app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditCalendarDropdownToggled),
    );
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        Size::new(400.0, 1200.0),
        event_editor(&state, &event, &metrics),
    );
    assert!(ui.find("TimeSlay").is_err());
    ui.click(other.name.as_str()).unwrap();
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::EventEdit(EventEditMessage::EventEditCalendarSelected(id)) if id == other.id
    )));
}

#[test]
fn a_habit_events_record_button_is_full_height_and_leaves_the_rest_clickable() {
    let day = NaiveDate::from_ymd_opt(2026, 3, 4).unwrap();
    // (x fraction, y fraction, whether it lands on the button)
    for (x, y, record) in [
        (0.99, 0.02, true),
        (0.99, 0.5, true),
        (0.99, 0.98, true),
        (0.2, 0.5, false),
    ] {
        let mut canvas = canvas_with_one_block();
        canvas.blocks[0].habit = Some(("h".into(), day));
        let bounds = canvas.block_rect(&canvas.blocks[0], 600.0);
        let (label, button) = habit_actions(bounds);
        assert_eq!((button.y, button.height), (bounds.y, bounds.height));
        assert_eq!(label.width + button.width, bounds.width);
        let element: Element<'_, Message> = canvas_widget(canvas).width(600).height(1440).into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            Size::new(600.0, 1440.0),
            element,
        );
        ui.point_at(Point::new(
            bounds.x + bounds.width * x,
            bounds.y + bounds.height * y,
        ));
        ui.simulate([
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ]);
        let messages: Vec<_> = ui.into_messages().collect();
        let recorded = messages.iter().any(|m| {
            matches!(m, Message::Habits(crate::habits::Action::OpenDay(id, on))
                if id == "h" && *on == day)
        });
        assert_eq!(recorded, record, "at {x}, {y}");
        if !record {
            assert!(messages.iter().any(
                |m| matches!(m, Message::Event(EventMessage::EventOpened(id)) if id == "standup")
            ));
        }
    }
}

#[test]
fn an_event_without_a_habit_has_no_record_button() {
    let canvas = canvas_with_one_block();
    let bounds = canvas.block_rect(&canvas.blocks[0], 600.0);
    assert!(
        canvas
            .record_at(
                Point::new(bounds.x + bounds.width - 2.0, bounds.y + 2.0),
                600.0
            )
            .is_none()
    );
}

#[test]
fn the_new_event_pane_has_a_meeting_section() {
    let mut state = State::blank();
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    let start = state.calendar.events[0].start;
    state
        .calendar
        .begin_draft("primary".into(), start, start + chrono::Duration::hours(1));
    state.calendar.draft.as_mut().unwrap().editor_open = true;
    let metrics = Metrics::of(Size::new(800.0, 600.0));
    let mut ui = iced_test::Simulator::with_size(
        iced_test::core::Settings::default(),
        Size::new(800.0, 1400.0),
        draft_editor(&state, &metrics),
    );
    // The simulator can't read a toggler's own label, so the toggles themselves
    // are covered by the form's tests in core.
    for label in ["Meeting", "Video call", "Guests", "Guest permissions"] {
        assert!(ui.find(label).is_ok(), "{label}");
    }
}

#[test]
fn an_invitation_can_be_answered_but_its_guests_cannot_be_managed() {
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
    let event = event.clone();
    let form = calendar::meeting::Form::from_event(&event.details);
    let metrics = Metrics::of(Size::new(800.0, 600.0));
    let mut ui = iced_test::Simulator::with_size(
        iced_test::core::Settings::default(),
        Size::new(800.0, 1000.0),
        meeting_section(&state, &form, Some(&event), &metrics, |action| {
            Message::EventEdit(EventEditMessage::EventEditMeeting(action))
        }),
    );
    assert!(ui.find("Video call").is_err());
    assert!(ui.find("Guest permissions").is_err());
    ui.click("Maybe").unwrap();
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::Event(EventMessage::EventReply(id, Response::Tentative)) if id == event.id
    )));
}

#[test]
fn quick_create_focuses_the_title_before_the_first_keystroke() {
    use iced::advanced::renderer::Headless;
    use iced::futures::{StreamExt, executor::block_on};
    use iced_test::runtime::{Action, UserInterface, task, user_interface};

    let mut state = State::blank();
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    let start = calendar::at_clock(state.calendar.day, 9, 0);
    let focus = crate::app::update(
        &mut state,
        Message::EventDraft(EventDraftMessage::EventDraftStarted {
            start,
            end: start + chrono::Duration::minutes(30),
        }),
    );
    let mut renderer = block_on(<iced::Renderer as Headless>::new(
        iced::Font::DEFAULT,
        iced::Pixels(16.0),
        Some("tiny-skia"),
    ))
    .unwrap();
    let size = Size::new(800.0, 600.0);
    let metrics = Metrics::of(size);
    let mut ui = UserInterface::build(
        draft_overlay(&state, &metrics),
        size,
        user_interface::Cache::default(),
        &mut renderer,
    );
    block_on(async {
        let mut stream = task::into_stream(focus).expect("title focus task");
        while let Some(action) = stream.next().await {
            if let Action::Widget(mut operation) = action {
                ui.operate(&renderer, operation.as_mut());
            }
        }
    });
    let mut messages = Vec::new();
    ui.update(
        &iced_test::simulator::typewrite("P").collect::<Vec<_>>(),
        iced::mouse::Cursor::Unavailable,
        &mut renderer,
        &mut iced::advanced::clipboard::Null,
        &mut messages,
    );
    assert!(messages.iter().any(|message| matches!(message,
        Message::EventDraft(EventDraftMessage::EventDraftTitleChanged(title)) if title == "P"
    )));
}

#[test]
fn habit_checkboxes_toggle_without_opening_or_dragging_the_event() {
    for compact in [false, true] {
        for linked in [false, true] {
            for completed in [false, true] {
                let mut canvas = canvas_with_one_block();
                canvas.compact = compact;
                canvas.blocks[0].habit_completed = Some(completed);
                if linked {
                    canvas.blocks[0].habit = Some(("h".into(), canvas.day));
                }
                let bounds = canvas.block_rect(&canvas.blocks[0], 600.0);
                let label = if linked {
                    habit_actions(bounds).0
                } else {
                    bounds
                };
                let checkbox = habit_completion_bounds(label).1;
                assert_eq!(
                    canvas.completion_at(checkbox.center(), 600.0),
                    Some("standup")
                );
                assert!(canvas.record_at(checkbox.center(), 600.0).is_none());
                let element: Element<'_, Message> =
                    canvas_widget(canvas).width(600).height(1440).into();
                let mut ui = iced_test::Simulator::with_size(
                    Default::default(),
                    Size::new(600.0, 1440.0),
                    element,
                );
                ui.point_at(checkbox.center());
                ui.simulate([
                    iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                    iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
                ]);
                let messages: Vec<_> = ui.into_messages().collect();
                assert_eq!(messages.len(), 1, "{messages:?}");
                assert!(
                    matches!(&messages[0], Message::Habits(crate::habits::Action::ToggleCompleted(id)) if id == "standup")
                );
            }
        }
    }
}

#[test]
fn a_habits_completion_change_invalidates_the_grid_cache() {
    let mut canvas = canvas_with_one_block();
    let mut keys = Vec::new();
    for completed in [None, Some(false), Some(true)] {
        canvas.blocks[0].habit_completed = completed;
        keys.push(cache_key(&canvas.blocks, 48.0, GUTTER, false, 0, None));
    }
    assert_ne!(keys[0], keys[1]);
    assert_ne!(keys[1], keys[2]);
}

#[test]
fn a_compact_habit_checkbox_captures_the_click_inside_the_event_button() {
    let mut state = State::blank();
    let mut event = calendar::sample_snapshot().events.remove(0);
    event.details.habit = true;
    let metrics = Metrics::of(Size::new(400.0, 100.0));
    for busy in [false, true] {
        if busy {
            state.habits.completing.insert(event.id.clone());
        }
        let mut ui = iced_test::Simulator::with_size(
            Default::default(),
            Size::new(400.0, 100.0),
            all_day_chip(&state, &event, &metrics),
        );
        ui.point_at(Point::new(385.0, 13.0));
        ui.simulate([
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ]);
        let messages: Vec<_> = ui.into_messages().collect();
        if !busy {
            assert_eq!(messages.len(), 1, "{messages:?}");
            assert!(
                matches!(&messages[0], Message::Habits(crate::habits::Action::ToggleCompleted(id)) if id == &event.id)
            );
        } else {
            assert!(
                !messages
                    .iter()
                    .any(|message| matches!(message, Message::Habits(_)))
            );
        }
    }
}

#[test]
fn day_unslotted_count_matches_today_and_keeps_overdue_separate() {
    let mut state = State::blank();
    let today = crate::clock::now().date_naive();
    let mut task = calendar::sample_snapshot().events.remove(0);
    task.id = "task:open".into();
    task.calendar_id = "tasks:inbox".into();
    task.title = "Open".into();
    task.all_day = true;
    task.completed = false;
    task.start = calendar::at_clock(today, 0, 0);
    task.end = task.start + chrono::Duration::days(1);
    task.details = Default::default();
    let mut overdue = task.clone();
    overdue.id = "task:overdue".into();
    overdue.details.description = "".into();
    overdue
        .details
        .task_data
        .get_or_insert_with(|| timeslay_core::task_db::TaskData {
            id: 100,
            metadata: Default::default(),
        })
        .metadata
        .deadline = Some(
        timeslay_core::calendar::deadline::parse_input(&format!(
            "{} 00:00",
            today.pred_opt().unwrap()
        ))
        .unwrap(),
    );
    let mut done = task.clone();
    done.id = "task:done".into();
    done.completed = true;
    let mut blocked = task.clone();
    blocked.id = "task:blocked".into();
    blocked.details.task_data = Some(crate::task_db::TaskData {
        id: 101,
        metadata: crate::timeblocking::Metadata {
            block: Some("missing-block".into()),
            ..Default::default()
        },
    });
    state.calendar.day = today;
    state.calendar.events = vec![task, overdue, done, blocked];
    assert_eq!(crate::inbox::today_tasks(&state).len(), 1);
    assert_eq!(crate::inbox::overdue_tasks(&state).len(), 1);
    let metrics = Metrics::of(Size::new(900.0, 700.0));
    let mut ui = iced_test::Simulator::with_size(
        iced_test::core::Settings::default(),
        Size::new(900.0, 700.0),
        header::all_day_row_on(&state, &metrics, today, false).unwrap(),
    );
    assert!(ui.find("1 overdue.  0 in inbox.  1 not slotted in").is_ok());
}

#[test]
fn recurring_time_change_prompt_offers_each_scope_at_narrow_and_wide_widths() {
    use iced_test::{Simulator, core::Settings};
    for size in [Size::new(360.0, 640.0), Size::new(1000.0, 700.0)] {
        for (label, scope) in [
            ("This event", calendar::SaveScope::Current),
            (
                "This and following events",
                calendar::SaveScope::CurrentAndFuture,
            ),
            ("All events", calendar::SaveScope::All),
        ] {
            let mut state = State::blank();
            state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
            let event = state
                .calendar
                .events
                .iter_mut()
                .find(|e| !e.is_task())
                .unwrap();
            event.details.recurring_event_id = Some("series".into());
            let original = event.clone();
            state.calendar.pending_move = Some(calendar::PendingMove {
                id: original.id.clone(),
                start: original.start,
                end: original.end,
                original,
                scope: None,
            });
            let metrics = Metrics::of(size);
            let mut ui = Simulator::with_size(
                Settings::default(),
                size,
                move_confirmation(&state, &metrics),
            );
            ui.click(label).unwrap();
            assert!(ui.into_messages().any(|m| matches!(m,
                Message::Event(EventMessage::EventMoveScoped(chosen)) if chosen == scope
            )));
        }
    }
}

#[test]
fn scheduled_task_deadline_follows_repeat_and_opens_the_original_task() {
    let mut state = State::blank();
    let mut snapshot = calendar::sample_snapshot();
    let task = snapshot
        .events
        .iter_mut()
        .find(|event| event.id == "standup")
        .unwrap();
    let deadline_day = task.start.date_naive().succ_opt().unwrap();
    task.details.task_data = Some(crate::task_db::TaskData {
        id: 81,
        metadata: crate::timeblocking::Metadata {
            deadline: Some(
                calendar::deadline::parse_input(&format!("{deadline_day} 17:00")).unwrap(),
            ),
            ..Default::default()
        },
    });
    let original_times = (task.start, task.end);
    state.calendar = calendar::Store::with_snapshot(snapshot);
    let markers = state
        .calendar
        .deadline_markers_on(deadline_day, &state.config.calendar);
    assert_eq!(markers.len(), 1);
    assert!(
        state
            .calendar
            .deadline_markers_on(deadline_day.succ_opt().unwrap(), &state.config.calendar)
            .is_empty()
    );
    let _ = crate::app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened(markers[0].id.clone())),
    );
    assert_eq!(state.calendar.open_event.as_deref(), Some("standup"));
    let draft = state.calendar.edit_draft.as_ref().unwrap();
    assert_eq!((draft.start, draft.end), original_times);
    assert_eq!(draft.deadline_text, format!("{deadline_day} 17:00"));
    let metrics = Metrics::of(Size::new(480.0, 2200.0));
    let mut ui = iced_test::Simulator::with_size(
        Default::default(),
        metrics.size,
        details_pane(&state, &metrics),
    );
    let repeat = ui.find("Repeat").unwrap().bounds();
    let deadline = ui.find("Deadline").unwrap().bounds();
    let projects = ui.find("Projects").unwrap().bounds();
    assert!(repeat.y < deadline.y && deadline.y < projects.y);
}
