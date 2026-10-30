//! Calendar selection and multiline forms exercised through the real widget tree.
use iced::{Size, widget::text_editor};
use iced_test::{Simulator, core::Settings};
use timeslay::{
    app::{self, Message, State},
    config::Config,
    theme::Catalog,
    ui,
};

fn state() -> State {
    State::new(Config::default(), Catalog::built_in())
}

#[test]
fn task_dates_open_calendar_and_day_buttons_select_real_dates() {
    for width in [200.0, 320.0, 720.0] {
        let mut state = state();
        let _ = app::update(
            &mut state,
            Message::Inbox(timeslay::inbox::Action::NewTask),
        );
        let mut form: Simulator<'_, Message> = Simulator::with_size(
            Settings::default(),
            Size::new(width, 700.0),
            ui::view(&state),
        );
        form.click("Choose date").unwrap();
        let open = form
            .into_messages()
            .find(|m| matches!(m, Message::Picker(_)))
            .unwrap();
        let _ = app::update(&mut state, open);
        let mut calendar = Simulator::with_size(
            Settings::default(),
            Size::new(width, 700.0),
            ui::view(&state),
        );
        calendar.click("15").unwrap();
        assert!(calendar.into_messages().any(|m| matches!(m,Message::Picker(ui::controls::Action::Select(day)) if chrono::Datelike::day(&day)==15)));
    }
}

#[test]
fn multiline_descriptions_survive_editing_and_habit_handoff() {
    let mut state = state();
    let now = chrono::Local::now();
    state.calendar.draft = Some(timeslay::calendar::Draft::new(
        "calendar".into(),
        now,
        now + chrono::Duration::hours(1),
    ));
    let _ = app::update(
        &mut state,
        Message::EventDraftDescriptionChanged("First line\nSecond line".into()),
    );
    assert_eq!(state.form_editors[0].text(), "First line\nSecond line");
    let _ = app::update(
        &mut state,
        Message::FormEdited(0, text_editor::Action::Edit(text_editor::Edit::Insert('!'))),
    );
    let expected = state.form_editors[0].text();
    assert!(expected.contains('\n'));
    assert_eq!(state.calendar.draft.as_ref().unwrap().description, expected);
    let _ = app::update(
        &mut state,
        Message::EventDraftKindSelected(timeslay::calendar::DraftKind::Habit),
    );
    assert_eq!(state.form_editors[2].text(), expected);
    assert_eq!(state.habits.draft.as_ref().unwrap().notes, expected);
}

#[test]
fn preview_forms_in_light_and_dark_themes() {
    let Some(directory) = std::env::var_os("TIMESLAY_UI_PREVIEW_DIR") else {
        return;
    };
    for theme in ["Sakura", "Dark"] {
        for width in [200.0, 420.0, 900.0] {
            let mut state = state();
            let _ = app::update(
                &mut state,
                Message::Inbox(timeslay::inbox::Action::NewTask),
            );
            let palette = state.catalog.resolve(theme).theme.clone();
            let mut form: Simulator<'_, Message> = Simulator::with_size(
                Settings::default(),
                Size::new(width, 720.0),
                ui::view(&state),
            );
            form.snapshot(&palette)
                .unwrap()
                .matches_image(
                    std::path::PathBuf::from(&directory).join(format!("{theme}-{width}-task")),
                )
                .unwrap();
            form.click("Choose date").unwrap();
            let open = form
                .into_messages()
                .find(|m| matches!(m, Message::Picker(_)))
                .unwrap();
            let _ = app::update(&mut state, open);
            let mut form: Simulator<'_, Message> = Simulator::with_size(
                Settings::default(),
                Size::new(width, 720.0),
                ui::view(&state),
            );
            form.snapshot(&palette)
                .unwrap()
                .matches_image(
                    std::path::PathBuf::from(&directory).join(format!("{theme}-{width}-calendar")),
                )
                .unwrap();
        }
    }
}

#[test]
fn time_picker_retains_the_other_segment() {
    for (x, expected) in [(40.0, "22:47"), (160.0, "23:46")] {
        let control = ui::controls::time("23:47", true, 13.0, Message::EventDraftStartChanged);
        let mut form: Simulator<'_, Message> = Simulator::with_size(
            Settings::default(),
            Size::new(320.0, 700.0),
            iced::widget::container(control).width(200),
        );
        form.point_at(iced::Point::new(x, 15.0));
        form.simulate([
            iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(
                iced::keyboard::Modifiers::CTRL,
            )),
            iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
            }),
        ]);
        assert!(
            form.into_messages()
                .any(|m| matches!(m, Message::EventDraftStartChanged(value) if value==expected))
        );
    }
}
