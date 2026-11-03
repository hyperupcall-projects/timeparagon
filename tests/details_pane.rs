//! Details use an independent editor window and never resize the main view.

use iced::{Size, Task, futures::StreamExt, window};
use iced_test::runtime::{self, Action, window::Action as WindowAction};
use timeslay::app::{
    CalendarMessage, EventDraftMessage, EventEditMessage, EventMessage, NavigationMessage,
    SettingsMessage,
};
use timeslay::{
    app::{self, LayoutField, Message, State},
    calendar,
    config::Config,
};

/// The pane width the default settings ship with.
const DETAIL: f32 = 360.0;

fn state() -> State {
    let mut state = State::blank();
    state.main_window = Some(window::Id::unique());
    state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
    state
}

fn actions(task: Task<Message>) -> Vec<Action<Message>> {
    iced::futures::executor::block_on(async {
        let mut result = Vec::new();
        if let Some(mut stream) = runtime::task::into_stream(task) {
            while let Some(action) = stream.next().await {
                match action {
                    Action::Window(WindowAction::Open(id, settings, sender)) => {
                        assert_eq!(settings.size.width, DETAIL);
                        let _ = sender.send(id);
                    }
                    Action::Output(_) => {}
                    action => result.push(action),
                }
            }
        }
        result
    })
}

fn assert_resize(task: Task<Message>, expected: Size) -> Vec<Action<Message>> {
    let actions = actions(task);
    let sizes: Vec<_> = actions
        .iter()
        .filter_map(|action| match action {
            Action::Window(WindowAction::Resize(_, size)) => Some(*size),
            _ => None,
        })
        .collect();
    assert_eq!(sizes, [expected]);
    actions
}

#[test]
fn opening_and_closing_details_never_resizes_the_main_window() {
    let mut state = state();
    let original = state.main_size;
    let main = state.main_window;
    assert!(
        actions(app::update(
            &mut state,
            Message::Event(EventMessage::EventOpened("all-day".into()))
        ))
        .is_empty()
    );
    let editor = state.editor_window.unwrap();
    assert_ne!(Some(editor), main);
    assert!(
        actions(app::update(
            &mut state,
            Message::Event(EventMessage::EventOpened("lunch".into()))
        ))
        .is_empty()
    );
    assert_eq!(state.editor_window, Some(editor));
    let closed = actions(app::update(
        &mut state,
        Message::Event(EventMessage::EventClosed),
    ));
    assert!(
        closed
            .iter()
            .any(|a| matches!(a, Action::Window(WindowAction::Close(id)) if *id == editor))
    );
    assert!(
        closed
            .iter()
            .all(|a| !matches!(a, Action::Window(WindowAction::Resize(..))))
    );
    assert_eq!(state.main_size, original);
    assert_eq!(state.main_window, main);
}

#[test]
fn the_details_width_setting_resizes_only_the_editor_live() {
    let mut state = state();
    let _ = actions(app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened("all-day".into())),
    ));
    let editor = state.editor_window.unwrap();
    let height = state.main_size.height - timeslay::ui::editor_window_top(&state);
    let result = assert_resize(
        app::update(
            &mut state,
            Message::Settings(SettingsMessage::DetailWidthChanged(240.0)),
        ),
        Size::new(240.0, height),
    );
    assert!(
        result
            .iter()
            .all(|a| !matches!(a, Action::Window(WindowAction::Resize(id, _)) if *id != editor))
    );
    let config: Config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
    assert_eq!(config.views.detail_width(), 240.0);
}

#[test]
fn layout_number_steps_the_details_width_through_the_message() {
    let mut state = state();
    let before = LayoutField::DetailWidth.value(&state);
    let _ = app::update(
        &mut state,
        Message::Settings(SettingsMessage::ViewLayoutInputSubmitted(
            LayoutField::DetailWidth,
        )),
    );
    assert_eq!(state.config.views.detail_width(), before);

    let _ = app::update(&mut state, LayoutField::DetailWidth.changed(before + 10.0));
    assert_eq!(state.config.views.detail_width(), before + 10.0);
}

#[test]
fn navigating_away_closes_only_the_editor_window() {
    let mut state = state();
    let original = state.main_size;
    let _ = actions(app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened("all-day".into())),
    ));
    let editor = state.editor_window.unwrap();
    let result = actions(app::update(
        &mut state,
        Message::Navigation(NavigationMessage::ScreenSelected(app::Screen::Notepad)),
    ));
    assert!(
        result
            .iter()
            .any(|a| matches!(a, Action::Window(WindowAction::Close(id)) if *id == editor))
    );
    assert!(
        result
            .iter()
            .all(|a| !matches!(a, Action::Window(WindowAction::Resize(..))))
    );
    assert!(!state.details_pane_open);
    assert!(state.calendar.open_event.is_some());
    assert_eq!(state.main_size, original);
}

#[test]
fn description_popup_cancels_without_changing_the_draft() {
    let mut state = state();
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened("standup".into())),
    );
    let before = state
        .calendar
        .edit_draft
        .as_ref()
        .unwrap()
        .description
        .clone();
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::DescriptionOpen("standup".into())),
    );
    assert_eq!(state.description_editor.text(), before);
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::DescriptionEdited(
            iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Paste(
                std::sync::Arc::new("Changed".to_owned()),
            )),
        )),
    );
    let _ = app::update(&mut state, Message::Event(EventMessage::DescriptionClose));
    assert!(state.description_popup.is_none());
    assert_eq!(
        state.calendar.edit_draft.as_ref().unwrap().description,
        before
    );
}

#[test]
fn description_popup_stages_event_changes_without_saving_the_event() {
    let mut state = state();
    let _ = actions(app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened("standup".into())),
    ));
    let editor = state.editor_window;
    let original = state.calendar.event("standup").unwrap().clone();
    let _ = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditTitleChanged("New title".into())),
    );
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::DescriptionOpen("standup".into())),
    );
    state.description_editor = iced::widget::text_editor::Content::with_text("New description");

    assert!(
        actions(app::update(
            &mut state,
            Message::Event(EventMessage::DescriptionSave)
        ))
        .is_empty()
    );

    assert!(state.description_popup.is_none());
    assert_eq!(state.editor_window, editor);
    assert!(state.details_pane_open);
    let draft = state.calendar.edit_draft.as_ref().unwrap();
    assert_eq!(draft.description, "New description");
    assert_eq!(draft.title, "New title");
    assert!(!draft.saving);
    assert_eq!(state.calendar.event("standup"), Some(&original));
}

#[test]
fn dragged_habit_stays_editable_after_description_and_project_done() {
    use calendar::projects::Action as ProjectAction;
    use timeslay::habits;

    let mut state = state();
    let start = state.calendar.event("standup").unwrap().start;
    let end = start + chrono::Duration::minutes(45);
    let _ = app::update(
        &mut state,
        Message::EventDraft(EventDraftMessage::EventDraftStarted { start, end }),
    );
    let _ = app::update(
        &mut state,
        Message::EventDraft(EventDraftMessage::EventDraftTitleChanged("Stretch".into())),
    );
    let _ = actions(app::update(
        &mut state,
        Message::EventDraft(EventDraftMessage::EventDraftKindSelected(
            calendar::DraftKind::Habit,
        )),
    ));
    let editor = state.editor_window.expect("the habit editor is open");
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::DescriptionOpen(habits::NEW_HABIT_ID.into())),
    );
    state.description_editor = iced::widget::text_editor::Content::with_text("Slow and steady");

    assert!(
        actions(app::update(
            &mut state,
            Message::Event(EventMessage::DescriptionSave)
        ))
        .is_empty()
    );
    assert!(state.description_popup.is_none());
    assert!(
        !state.habits.busy,
        "saving a description must not create the habit"
    );
    let project = state.config.projects.items[0].id;
    for action in [
        ProjectAction::Toggle, // Add project
        ProjectAction::Add(project),
        ProjectAction::Toggle, // Done
    ] {
        assert!(
            actions(app::update(
                &mut state,
                Message::EventEdit(EventEditMessage::EventEditProjects(action))
            ))
            .is_empty()
        );
        assert_eq!(state.editor_window, Some(editor));
        assert!(state.details_pane_open);
        assert_eq!(
            state.calendar.open_event.as_deref(),
            Some(habits::NEW_HABIT_ID)
        );
        assert!(!state.habits.busy);
    }

    let _ = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditStartChanged("10:00".into())),
    );
    let _ = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditTitleChanged(
            "Morning stretch".into(),
        )),
    );
    let draft = state.calendar.edit_draft.as_ref().unwrap();
    assert_eq!(draft.title, "Morning stretch");
    assert_eq!(draft.description, "Slow and steady");
    assert_eq!(draft.projects.ids, [project]);
    assert!(!draft.projects.open);
    assert_eq!(draft.start_text, "10:00");
    assert_eq!(draft.end_text, "10:45");
    assert!(!draft.saving);
    assert_eq!(state.editor_window, Some(editor));
    assert!(state.calendar.error.is_none());
}

#[test]
fn moving_a_habit_start_keeps_its_duration() {
    let mut state = state();
    let mut snapshot = calendar::sample_snapshot();
    snapshot
        .events
        .iter_mut()
        .find(|e| e.id == "standup")
        .unwrap()
        .details
        .habit = true;
    state.calendar = calendar::Store::with_snapshot(snapshot);
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened("standup".into())),
    );
    let original = state.calendar.edit_draft.as_ref().unwrap();
    let duration = original.end - original.start;
    let _ = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditDateChanged("2026-11-02".into())),
    );
    let _ = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditStartChanged("23:30".into())),
    );
    let draft = state.calendar.edit_draft.as_ref().unwrap();
    assert_eq!(draft.start.date_naive().to_string(), "2026-11-02");
    assert_eq!(calendar::clock_label(draft.start), "23:30");
    assert_eq!(draft.end - draft.start, duration);
}

#[test]
fn habit_series_response_is_ignored_after_selecting_another_event() {
    let mut state = state();
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened("standup".into())),
    );
    let event = state.calendar.event("standup").unwrap().clone();
    let series = state.calendar.edit_draft.clone().unwrap();
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::EventOpened("lunch".into())),
    );
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::HabitDetailsLoaded(
            "standup".into(),
            Ok((event, series)),
        )),
    );
    assert_eq!(state.calendar.edit_draft.as_ref().unwrap().id, "lunch");
}

#[test]
fn loading_a_calendar_habit_keeps_the_selected_occurrence_and_sync_source() {
    let mut state = state();
    let mut event = state.calendar.events[1].clone();
    event.details.habit = true;
    event.details.recurring_event_id = Some("habit-series".into());
    state.calendar.events[1] = event.clone();
    // Seed the editor as EventOpened does while its series is loading.
    state.calendar.open_event = Some(event.id.clone());
    let mut master = event.clone();
    master.id = "habit-series".into();
    master.start -= chrono::Duration::days(10);
    master.end -= chrono::Duration::days(10);
    master.details.recurrence = vec!["RRULE:FREQ=DAILY".into()];
    let draft = calendar::EditDraft::from_habit_occurrence(&event, &master);
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::HabitDetailsLoaded(
            event.id.clone(),
            Ok((event.clone(), draft)),
        )),
    );
    assert_eq!(
        state.calendar.open_event.as_deref(),
        Some(event.id.as_str())
    );
    let draft = state.calendar.edit_draft.as_ref().unwrap();
    assert_eq!(draft.start, event.start);
    assert_eq!(draft.end, event.end);
    assert!(
        state.calendar.detail_event.is_none(),
        "Calendar sync must keep supplying the original occurrence"
    );
}

/// The week and month overview stays open across the panel's tabs and has a
/// details pane of its own, so a new habit is edited there rather than in a
/// second window.
#[test]
fn new_habit_with_the_overview_open_is_edited_inside_the_overview() {
    use timeslay::{app::Screen, habits};
    for open_overview in [
        Message::Calendar(CalendarMessage::CalendarModeSelected(true)),
        Message::Calendar(CalendarMessage::CalendarMonthViewSelected),
    ] {
        let mut state = state();
        let _ = app::update(&mut state, open_overview);
        let _ = app::update(
            &mut state,
            Message::Navigation(NavigationMessage::ScreenSelected(Screen::Habits)),
        );
        assert!(state.range_view_active());

        state
            .habits
            .library
            .habits
            .push(habits::Habit::new("h", "Stretch"));
        let _ = app::update(&mut state, Message::Habits(habits::Action::AddEvent("h".into())));

        assert_eq!(
            state.calendar.open_event.as_deref(),
            Some(habits::NEW_HABIT_ID)
        );
        assert!(!state.details_pane_open);
        let mut ui = iced_test::simulator::Simulator::with_size(
            iced::Settings::default(),
            Size::new(1216.0, 900.0),
            timeslay::ui::editor_window(&state),
        );
        assert!(
            ui.find("Notifications").is_ok(),
            "no editor in {:?}",
            state.view
        );
    }
}

#[test]
fn new_habit_uses_the_shared_right_pane_and_width_setting() {
    use timeslay::{app::Screen, habits};
    let mut state = state();
    state.screen = Screen::Habits;
    let original = state.main_size;
    state
        .habits
        .library
        .habits
        .push(habits::Habit::new("h", "Stretch"));
    let task = app::update(&mut state, Message::Habits(habits::Action::AddEvent("h".into())));
    let _ = actions(task);
    assert!(state.editor_window.is_some());
    assert_eq!(state.main_size, original);
    assert_eq!(
        state.calendar.open_event.as_deref(),
        Some(habits::NEW_HABIT_ID)
    );
    assert!(state.calendar.edit_draft.as_ref().unwrap().editing_title);
    assert!(
        state
            .calendar
            .events
            .iter()
            .all(|event| event.id != habits::NEW_HABIT_ID)
    );
    assert_resize(
        app::update(
            &mut state,
            Message::Settings(SettingsMessage::DetailWidthChanged(280.0)),
        ),
        Size::new(
            280.0,
            original.height - timeslay::ui::editor_window_top(&state),
        ),
    );
    let _ = actions(app::update(
        &mut state,
        Message::Event(EventMessage::EventClosed),
    ));
    assert!(state.editor_window.is_none());
    assert!(state.calendar.detail_event.is_none());
    assert!(state.calendar.edit_draft.is_none());
}

#[test]
fn editing_a_habit_outside_the_visible_day_uses_the_calendar_editor() {
    use timeslay::{app::Screen, habits};
    let mut state = state();
    state.screen = Screen::Habits;
    let start = state.calendar.events[1].start + chrono::Duration::days(10);
    state.habits.series.push(habits::Series {
        id: "off-day-master".into(),
        calendar_id: "personal".into(),
        title: "Stretch".into(),
        notes: "Slowly".into(),
        start,
        end: start + chrono::Duration::minutes(30),
        recurrence: habits::Recurrence::from_lines(&["RRULE:FREQ=DAILY".into()]),
        color_id: None,
        habit_id: None,
    });
    let original = state.main_size;
    let task = app::update(
        &mut state,
        Message::Habits(habits::Action::EditEvent("off-day-master".into())),
    );
    let _ = actions(task);
    assert!(state.editor_window.is_some());
    assert_eq!(state.main_size, original);
    let draft = state.calendar.edit_draft.as_ref().unwrap();
    assert!(draft.loading);
    assert_eq!(draft.title, "Stretch");
    let mut loaded = draft.clone();
    loaded.loading = false;
    loaded.series_id = Some("off-day-master".into());
    loaded.projects.ids = vec![7];
    let mut event = state.calendar.event("off-day-master").unwrap().clone();
    event.id = "off-day-instance".into();
    event.details.recurring_event_id = Some("off-day-master".into());
    event.details.recurrence.clear();
    loaded.id = event.id.clone();
    let _ = app::update(
        &mut state,
        Message::Event(EventMessage::HabitDetailsLoaded(
            "off-day-master".into(),
            Ok((event, loaded)),
        )),
    );
    let _ = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditStartChanged("10:00".into())),
    );
    let draft = state.calendar.edit_draft.as_ref().unwrap();
    assert_eq!(draft.start_text, "10:00");
    assert_eq!(draft.end_text, "10:30");
    assert_eq!(draft.projects.ids, [7]);
    state.calendar.apply_snapshot(calendar::sample_snapshot());
    assert_eq!(
        state.calendar.open_event.as_deref(),
        Some("off-day-instance")
    );
    assert_eq!(
        state.calendar.detail_event.as_ref().unwrap().id,
        "off-day-instance"
    );
}

#[test]
fn a_new_habit_keeps_its_draft_on_save_failure_and_closes_on_success() {
    use timeslay::{app::Screen, habits};
    let mut state = state();
    state.screen = Screen::Habits;
    state
        .habits
        .library
        .habits
        .push(habits::Habit::new("h", "Stretch"));
    let _ = app::update(&mut state, Message::Habits(habits::Action::AddEvent("h".into())));
    let _ = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditTitleChanged(String::new())),
    );
    let _ = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditSaveRequested),
    );
    assert_eq!(state.calendar.error.as_deref(), Some("Enter a title"));
    let _ = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditTitleChanged("Stretch".into())),
    );
    let _ = app::update(
        &mut state,
        Message::Habits(habits::Action::Saved(Err("Offline".into()))),
    );
    assert_eq!(state.calendar.error.as_deref(), Some("Offline"));
    let draft = state.calendar.edit_draft.as_ref().unwrap();
    assert_eq!(draft.title, "Stretch");
    let event = calendar::api::HabitEvent {
        id: "created-master".into(),
        calendar_id: "personal".into(),
        title: draft.title.clone(),
        notes: draft.description.clone(),
        start: draft.start,
        end: draft.end,
        recurrence: draft.recurrence_lines().unwrap(),
        color_id: None,
        habit_id: None,
    };
    let _ = app::update(
        &mut state,
        Message::Habits(habits::Action::Saved(Ok(event))),
    );
    assert!(!state.details_pane_open);
    assert!(state.calendar.detail_event.is_none());
    assert_eq!(state.habits.series[0].id, "created-master");
}

#[test]
fn floating_editor_matches_panel_height_with_screen_margins() {
    for side in [timeslay::panel::Side::Left, timeslay::panel::Side::Right] {
        for margin in [0.0, 8.0, 30.0] {
            let mut state = state();
            state.sidebar = Some(app::Sidebar { width: 200.0, side });
            state.config.views.margin = margin;
            state.main_size = Size::new(200.0 + margin * 2.0, 900.0);
            let task = app::update(
                &mut state,
                Message::Event(EventMessage::EventOpened("all-day".into())),
            );
            iced::futures::executor::block_on(async {
                let mut stream = runtime::task::into_stream(task).unwrap();
                let mut opened = false;
                while let Some(action) = stream.next().await {
                    if let Action::Window(WindowAction::Open(id, settings, sender)) = action {
                        assert_eq!(settings.size, Size::new(DETAIL, 900.0 - 2.0 * margin));
                        assert_eq!(timeslay::ui::editor_window_top(&state), margin);
                        let _ = sender.send(id);
                        opened = true;
                    }
                }
                assert!(opened);
            });
        }
    }
}

#[test]
fn saved_habit_edits_refresh_both_the_habits_list_and_calendar() {
    let mut state = state();
    state.screen = app::Screen::Habits;
    let event = state.calendar.events[1].clone();
    let mut draft = calendar::EditDraft::from_event(&event);
    draft.saving = true;
    state.calendar.edit_draft = Some(draft);
    let task = app::update(
        &mut state,
        Message::EventEdit(EventEditMessage::EventEditSaved(Ok(()))),
    );
    let messages = iced::futures::executor::block_on(async {
        let mut messages = Vec::new();
        let mut stream = runtime::task::into_stream(task).unwrap();
        while let Some(action) = stream.next().await {
            if let Action::Output(message) = action {
                messages.push(message);
            }
        }
        messages
    });
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::Habits(timeslay::habits::Action::Refresh)))
    );
    assert!(messages.iter().any(|message| matches!(
        message,
        Message::Calendar(CalendarMessage::CalendarSyncRequested)
    )));
    assert!(!state.calendar.edit_draft.as_ref().unwrap().saving);
}
