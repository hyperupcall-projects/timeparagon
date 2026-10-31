//! Details use an independent editor window and never resize the main view.

use iced::{Size, Task, futures::StreamExt, window};
use iced_test::runtime::{self, Action, window::Action as WindowAction};
use timeslay::{
    app::{self, LayoutField, Message, State},
    calendar,
    config::Config,
    theme::Catalog,
};

/// The pane width the default settings ship with.
const DETAIL: f32 = 360.0;

fn state() -> State {
    let mut state = State::new(Config::default(), Catalog::built_in());
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
            Message::EventOpened("all-day".into())
        ))
        .is_empty()
    );
    let editor = state.editor_window.unwrap();
    assert_ne!(Some(editor), main);
    assert!(
        actions(app::update(
            &mut state,
            Message::EventOpened("lunch".into())
        ))
        .is_empty()
    );
    assert_eq!(state.editor_window, Some(editor));
    let closed = actions(app::update(&mut state, Message::EventClosed));
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
        Message::EventOpened("all-day".into()),
    ));
    let editor = state.editor_window.unwrap();
    let height = state.main_size.height - timeslay::ui::editor_window_top(&state);
    let result = assert_resize(
        app::update(&mut state, Message::DetailWidthChanged(240.0)),
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
        Message::ViewLayoutInputSubmitted(LayoutField::DetailWidth),
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
        Message::EventOpened("all-day".into()),
    ));
    let editor = state.editor_window.unwrap();
    let result = actions(app::update(
        &mut state,
        Message::ScreenSelected(app::Screen::Notepad),
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
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    let before = state
        .calendar
        .edit_draft
        .as_ref()
        .unwrap()
        .description
        .clone();
    let _ = app::update(&mut state, Message::DescriptionOpen("standup".into()));
    assert_eq!(state.description_editor.text(), before);
    let _ = app::update(
        &mut state,
        Message::DescriptionEdited(iced::widget::text_editor::Action::Edit(
            iced::widget::text_editor::Edit::Paste(std::sync::Arc::new("Changed".to_owned())),
        )),
    );
    let _ = app::update(&mut state, Message::DescriptionClose);
    assert!(state.description_popup.is_none());
    assert_eq!(
        state.calendar.edit_draft.as_ref().unwrap().description,
        before
    );
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
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    let original = state.calendar.edit_draft.as_ref().unwrap();
    let duration = original.end - original.start;
    let _ = app::update(
        &mut state,
        Message::EventEditDateChanged("2026-11-02".into()),
    );
    let _ = app::update(&mut state, Message::EventEditStartChanged("23:30".into()));
    let draft = state.calendar.edit_draft.as_ref().unwrap();
    assert_eq!(draft.start.date_naive().to_string(), "2026-11-02");
    assert_eq!(calendar::clock_label(draft.start), "23:30");
    assert_eq!(draft.end - draft.start, duration);
}

#[test]
fn habit_series_response_is_ignored_after_selecting_another_event() {
    let mut state = state();
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    let series = state.calendar.edit_draft.clone().unwrap();
    let _ = app::update(&mut state, Message::EventOpened("lunch".into()));
    let _ = app::update(
        &mut state,
        Message::HabitDetailsLoaded("standup".into(), Ok(series)),
    );
    assert_eq!(state.calendar.edit_draft.as_ref().unwrap().id, "lunch");
}

#[test]
fn new_habit_uses_the_shared_right_pane_and_width_setting() {
    use timeslay::{app::Screen, habits};
    let mut state = state();
    state.screen = Screen::Habits;
    state.weekly = true; // Calendar range preferences must not affect this tab.
    let original = state.main_size;
    let task = app::update(&mut state, Message::Habits(habits::Action::New));
    let _ = actions(task);
    assert!(state.editor_window.is_some());
    assert_eq!(state.main_size, original);
    assert!(state.habits.draft.is_none());
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
        app::update(&mut state, Message::DetailWidthChanged(280.0)),
        Size::new(
            280.0,
            original.height - timeslay::ui::editor_window_top(&state),
        ),
    );
    let _ = actions(app::update(&mut state, Message::EventClosed));
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
    state.habits.habits.push(habits::Habit {
        id: "off-day-master".into(),
        calendar_id: "personal".into(),
        title: "Stretch".into(),
        notes: "Slowly".into(),
        start,
        end: start + chrono::Duration::minutes(30),
        recurrence: habits::Recurrence::from_lines(&["RRULE:FREQ=DAILY".into()]),
        color_id: None,
    });
    let original = state.main_size;
    let task = app::update(
        &mut state,
        Message::Habits(habits::Action::Edit("off-day-master".into())),
    );
    let _ = actions(task);
    assert!(state.editor_window.is_some());
    assert_eq!(state.main_size, original);
    let draft = state.calendar.edit_draft.as_ref().unwrap();
    assert!(draft.loading);
    assert_eq!(draft.title, "Stretch");
    assert!(state.habits.draft.is_none());
    let mut loaded = draft.clone();
    loaded.loading = false;
    loaded.series_id = Some("off-day-master".into());
    loaded.projects.ids = vec![7];
    let _ = app::update(
        &mut state,
        Message::HabitDetailsLoaded("off-day-master".into(), Ok(loaded)),
    );
    let _ = app::update(&mut state, Message::EventEditStartChanged("10:00".into()));
    let draft = state.calendar.edit_draft.as_ref().unwrap();
    assert_eq!(draft.start_text, "10:00");
    assert_eq!(draft.end_text, "10:30");
    assert_eq!(draft.projects.ids, [7]);
    state.calendar.apply_snapshot(calendar::sample_snapshot());
    assert_eq!(state.calendar.open_event.as_deref(), Some("off-day-master"));
}

#[test]
fn a_new_habit_keeps_its_draft_on_save_failure_and_closes_on_success() {
    use timeslay::{app::Screen, habits};
    let mut state = state();
    state.screen = Screen::Habits;
    let _ = app::update(&mut state, Message::Habits(habits::Action::New));
    let _ = app::update(&mut state, Message::EventEditSaveRequested);
    assert_eq!(state.calendar.error.as_deref(), Some("Enter a title"));
    let _ = app::update(&mut state, Message::EventEditTitleChanged("Stretch".into()));
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
    };
    let _ = app::update(
        &mut state,
        Message::Habits(habits::Action::Saved(Ok(event))),
    );
    assert!(!state.details_pane_open);
    assert!(state.calendar.detail_event.is_none());
    assert_eq!(state.habits.habits[0].id, "created-master");
}

#[test]
fn floating_editor_matches_panel_height_with_screen_margins() {
    for side in [timeslay::panel::Side::Left, timeslay::panel::Side::Right] {
        for margin in [0.0, 8.0, 30.0] {
            let mut state = state();
            state.sidebar = Some(app::Sidebar { width: 200.0, side });
            state.config.views.margin = margin;
            state.main_size = Size::new(200.0 + margin * 2.0, 900.0);
            let task = app::update(&mut state, Message::EventOpened("all-day".into()));
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
