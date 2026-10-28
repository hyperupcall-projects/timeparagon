use iced::{Size, widget::image::Handle};
use iced_test::{Simulator, core::Settings};
use time_slay::{
    app::{self, Screen, State},
    config::Config,
    theme::Catalog,
};

fn state() -> State {
    let mut state = State::new(Config::default(), Catalog::built_in());
    state.screen = Screen::KeepNotes;
    for index in 0..6 {
        state.keep.snapshot.notes.push(time_slay::keep::Note {
            id: index.to_string(),
            title: format!("Note {index}"),
            description: "A clear native text note.".into(),
            color: "rgb(255, 248, 184)".into(),
            preview_handle: Some(Handle::from_rgba(1, 1, vec![255; 4])),
            ..Default::default()
        });
    }
    state
}

#[test]
fn keep_has_one_appearance_and_distinct_metadata() {
    let mut state = state();
    state.keep.snapshot.notes[0].metadata = [
        "Created Sep 17, 2026",
        "Edited Oct 24, 2026",
        "Category: Personal",
        "Tags: routine",
        "Reminder: Today, 8:00 AM",
        "Source: Schedule",
    ]
    .map(str::to_owned)
    .to_vec();
    state.keep.snapshot.notes[0].labels = vec!["Habits & Rituals".into()];
    let mut ui = Simulator::with_size(
        Settings::default(),
        Size::new(320.0, 1000.0),
        app::view(&state, iced::window::Id::unique()),
    );
    for label in [
        "Note 0",
        "Created Sep 17, 2026",
        "Edited Oct 24, 2026",
        "Category · Personal",
        "#routine",
        "Reminder · Today, 8:00 AM",
        "Source · Schedule",
        "Habits & Rituals",
    ] {
        assert!(ui.find(label).is_ok(), "Missing {label}");
    }
    assert!(ui.find("Original appearance").is_err());
    assert!(ui.find("Readable text").is_err());
    let body = ui.find("A clear native text note.").unwrap().bounds();
    let label = ui.find("Habits & Rituals").unwrap().bounds();
    let timestamp = ui.find("Created Sep 17, 2026").unwrap().bounds();
    assert!(label.y > body.y);
    assert!(timestamp.y > label.y);
    assert!(timestamp.height < body.height);
}

#[test]
fn keep_adapts_from_one_to_three_independent_columns() {
    let state = state();
    for (width, count) in [(200.0, 1), (640.0, 2), (960.0, 3), (1600.0, 3)] {
        let mut ui = Simulator::with_size(
            Settings::default(),
            Size::new(width, 1000.0),
            app::view(&state, iced::window::Id::unique()),
        );
        let first = ui.find("Note 0").unwrap().bounds();
        for index in 1..count {
            let next = ui.find(format!("Note {index}")).unwrap().bounds();
            assert!(next.x > first.x);
            assert!((next.y - first.y).abs() < 1.0);
            assert!(next.x + next.width <= width);
        }
        let below = ui.find(format!("Note {count}")).unwrap().bounds();
        assert!((below.x - first.x).abs() < 1.0);
        assert!(below.y > first.y);
    }
}
