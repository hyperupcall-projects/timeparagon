//! Renders the Habits screens to PNGs for a visual check. Temporary.
use chrono::Duration;
use iced::{Size, Theme};
use iced_test::{Simulator, core::Settings};
use timeslay::app::{Screen, State};
use timeslay::habits::{Action, Frequency, Habit, Kind, Mark, Measure};
use timeslay::ui;

fn demo() -> State {
    let mut state = State::blank();
    state.screen = Screen::Habits;
    let today = timeslay::clock::now().date_naive();
    let make = |id: &str, name: &str, color: u8, frequency: Frequency, every: i64| {
        let mut habit = Habit::new(id, name);
        habit.color = color;
        habit.frequency = frequency;
        for back in 0..500_i64 {
            let keep = (back * 7 + back / 11) % every != 0;
            if keep {
                habit.marks.insert(today - Duration::days(back), Mark::Yes);
            }
        }
        habit
    };
    let lib = &mut state.habits.library.habits;
    let mut wake = make("a", "Wake up early", 11, Frequency::DAILY, 3);
    wake.question = "Did you wake up before 7?".into();
    lib.push(wake);
    lib.push(make(
        "b",
        "Cook healthy dinner",
        11,
        Frequency::Interval { times: 3, days: 7 },
        2,
    ));
    let mut meditate = make("c", "Meditate", 15, Frequency::DAILY, 4);
    meditate.question = "Did you meditate for at least 10 minutes today?".into();
    meditate.skip_days = true;
    meditate.marks.insert(today - Duration::days(2), Mark::Skip);
    meditate.marks.insert(today - Duration::days(1), Mark::No);
    lib.push(meditate);
    let mut water = make(
        "d",
        "Drink water",
        2,
        Frequency::TimesPerDay { times: 3 },
        3,
    );
    water.marks.insert(today, Mark::Reps(1));
    water.marks.insert(today - Duration::days(1), Mark::Reps(2));
    lib.push(water);
    let mut days = [false; 7];
    days[0] = true;
    let mut chess = Habit::new("e", "Play chess");
    chess.frequency = Frequency::Weekdays { days };
    chess.color = 13;
    chess.question_marks = true;
    lib.push(chess);
    let mut run = Habit::new("f", "Run");
    run.measure = Some(Measure {
        unit: "miles".into(),
        target: Some(2.0),
        days: 1,
    });
    run.color = 15;
    for back in 0..200_i64 {
        if back % 3 != 2 {
            run.marks.insert(today - Duration::days(back), Mark::Yes);
            run.set_value(today - Duration::days(back), Some(0.5 + (back % 5) as f64));
        }
    }
    lib.push(run);
    let mut smoke = Habit::new("g", "Smoke");
    smoke.kind = Kind::No;
    smoke.created = Some(today - Duration::days(120));
    smoke.color = 0;
    smoke.marks.insert(today - Duration::days(1), Mark::Reps(2));
    smoke
        .marks
        .insert(today - Duration::days(40), Mark::Reps(1));
    lib.push(smoke);
    lib[0].marks.insert(today, Mark::Reps(3));
    state.habits.recompute();
    state
}

fn shot(view: iced::Element<'_, timeslay::app::Message>, size: Size, theme: &Theme, name: &str) {
    let mut ui = Simulator::with_size(Settings::default(), size, view);
    let path = format!("/tmp/ts/shots/{name}.png");
    let _ = std::fs::remove_file(&path);
    assert!(ui.snapshot(theme).unwrap().matches_image(&path).unwrap());
}

#[test]
#[ignore = "writes PNGs for a visual check"]
fn render() {
    std::fs::create_dir_all("/tmp/ts/shots").unwrap();
    for (theme, label) in [(Theme::Light, "light"), (Theme::Dark, "dark")] {
        let mut state = demo();
        shot(
            ui::view(&state),
            Size::new(360.0, 560.0),
            &theme,
            &format!("list-{label}"),
        );
        let _ = timeslay::app::update(
            &mut state,
            timeslay::app::Message::Habits(Action::Menu(timeslay::habits::Menu::Filter)),
        );
        shot(
            ui::view(&state),
            Size::new(360.0, 560.0),
            &theme,
            &format!("menu-{label}"),
        );
        let _ = timeslay::app::update(
            &mut state,
            timeslay::app::Message::Habits(Action::CloseMenu),
        );
        let _ = timeslay::app::update(
            &mut state,
            timeslay::app::Message::Habits(Action::Open("c".into())),
        );
        shot(
            ui::view(&state),
            Size::new(420.0, 1500.0),
            &theme,
            &format!("page-{label}"),
        );
        let _ = timeslay::app::update(
            &mut state,
            timeslay::app::Message::Habits(Action::Open("f".into())),
        );
        shot(
            ui::view(&state),
            Size::new(420.0, 1500.0),
            &theme,
            &format!("page-run-{label}"),
        );
        let _ = timeslay::app::update(
            &mut state,
            timeslay::app::Message::Habits(Action::Open("g".into())),
        );
        shot(
            ui::view(&state),
            Size::new(420.0, 1500.0),
            &theme,
            &format!("page-smoke-{label}"),
        );
        let _ = timeslay::app::update(&mut state, timeslay::app::Message::Habits(Action::New));
        shot(
            ui::view(&state),
            Size::new(420.0, 900.0),
            &theme,
            &format!("form-{label}"),
        );
    }
}
