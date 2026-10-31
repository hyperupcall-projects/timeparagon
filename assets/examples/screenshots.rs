//! Renders the interface at each breakpoint and theme, headlessly.
//!
//! Run with `cargo run --example screenshots`. The PNGs land in
//! `screenshots/` and are how the narrow layouts get eyeballed without
//! resizing a window by hand forty times.

use iced::{Element, Size};
use iced_test::Simulator;
use iced_test::core::Settings;

use timeslay::app::{self, Message, Screen, State};
use timeslay::config::Config;
use timeslay::theme::Catalog;

/// The widths the layout is designed around.
const WIDTHS: [f32; 4] = [180.0, 200.0, 260.0, 420.0];

/// Our palettes, the curated dark themes, and representative built-ins.
const THEMES: [&str; 12] = [
    "Sakura",
    "Peony",
    "Matcha",
    "Mint",
    "Sage",
    "Catppuccin Latte",
    "Dark",
    "Absolute Black",
    "GitHub Dark",
    "GitHub Dark High Contrast",
    "Nord",
    "Dracula",
];

fn main() {
    let directory = std::path::Path::new("screenshots");
    std::fs::create_dir_all(directory).expect("create the screenshots directory");

    // Start from a clean slate so old renders are actually replaced.
    for entry in std::fs::read_dir(directory).expect("read the screenshots directory") {
        let path = entry.expect("read a directory entry").path();

        if path.extension().is_some_and(|extension| extension == "png") {
            std::fs::remove_file(path).expect("remove a stale screenshot");
        }
    }

    let mut count = 0;

    for theme in THEMES {
        for width in WIDTHS {
            let size = Size::new(width, 360.0);

            count += shoot(directory, theme, size, "finished", |state| {
                state.alarm_active = true;
            });

            count += shoot(directory, theme, size, "alarm-mode", |state| {
                state.alarms.mode = true;
            });

            count += shoot(directory, theme, size, "alarm-popup", |state| {
                let at = timeslay::alarms::minute_of(chrono::Local::now())
                    + chrono::Duration::minutes(45);
                let id = state.alarms.add(at, state.config.alarm.clone());
                let _ = app::update(state, Message::Alarms(timeslay::alarms::Action::Open(id)));
            });

            count += shoot(directory, theme, size, "settings", |state| {
                state.screen = Screen::Settings;
            });

            count += shoot(
                directory,
                theme,
                Size::new(width, 720.0),
                "calendar",
                |state| {
                    state.screen = Screen::Calendar;
                    state.calendar = timeslay::calendar::Store::with_snapshot(
                        timeslay::calendar::sample_snapshot(),
                    );
                },
            );

            // A Google summary is free text, and it is often far longer than
            // the event it names. This one brings its own line breaks and a
            // word too wide for the block, so the labels can be seen staying
            // inside their boxes.
            count += shoot(
                directory,
                theme,
                Size::new(width, 720.0),
                "calendar-long-title",
                |state| {
                    state.screen = Screen::Calendar;

                    let mut snapshot = timeslay::calendar::sample_snapshot();
                    let day = snapshot.day;
                    let at = |hour, minute| {
                        day.and_hms_opt(hour, minute, 0)
                            .and_then(|naive| naive.and_local_timezone(chrono::Local).single())
                            .expect("a local time")
                    };
                    let work = snapshot
                        .calendars
                        .iter()
                        .find(|calendar| calendar.id == "work")
                        .expect("the work calendar");

                    // A quarter of an hour is barely a line of text, which is
                    // the case that used to spill over the events below it.
                    snapshot.events.push(timeslay::calendar::Event {
                        id: "long-title".into(),
                        calendar_id: work.id.clone(),
                        title: "17-4 HW10/22 (Thu)2pm: L18\nReading.Inaka vs Tokai-->Changed\nfrom the L18 department store\nSupercalifragilisticexpialidocious".into(),
                        start: at(7, 0),
                        end: at(7, 15),
                        all_day: false,
                        color: work.color,
                        completed: false,
                        details: Default::default(),
                    });

                    state.calendar = timeslay::calendar::Store::with_snapshot(snapshot);
                },
            );

            count += shoot(
                directory,
                theme,
                Size::new(width, 720.0),
                "calendar-month",
                |state| {
                    state.screen = Screen::Calendar;
                    state.calendar = timeslay::calendar::Store::with_snapshot(
                        timeslay::calendar::sample_snapshot(),
                    );
                    state.calendar.month_open = true;
                },
            );

            count += shoot(
                directory,
                theme,
                Size::new(width, 720.0),
                "calendar-todos",
                |state| {
                    state.screen = Screen::Calendar;
                    state.calendar = timeslay::calendar::Store::with_snapshot(
                        timeslay::calendar::sample_snapshot(),
                    );
                    state.config.todos.add("Draft the report");
                    state.config.todos.add("Collect the numbers");
                    state.config.todos.indent(1, 1);
                    state.config.todos.add("Send it round");
                    state.config.todos.set_current(1);
                },
            );

            count += shoot(
                directory,
                theme,
                Size::new(width, 720.0),
                "todos",
                |state| {
                    state.screen = Screen::Backlog;

                    // The sample events are in the past, so a couple are moved to
                    // later today to give the "Coming up" list something to show.
                    let mut snapshot = timeslay::calendar::sample_snapshot();
                    let soon = chrono::Local::now() + chrono::Duration::minutes(20);
                    snapshot.events[1].start = soon;
                    snapshot.events[1].end = soon + chrono::Duration::minutes(30);
                    snapshot.events[3].start = soon + chrono::Duration::minutes(45);
                    snapshot.events[3].end = soon + chrono::Duration::minutes(105);
                    state.calendar = timeslay::calendar::Store::with_snapshot(snapshot);

                    state.config.todos.add("Draft the report");
                    state.config.todos.add("Collect the numbers");
                    state.config.todos.indent(1, 1);
                    state.config.todos.add("Send it round");
                    state.config.todos.set_current(1);
                },
            );

            count += shoot(
                directory,
                theme,
                Size::new(width, 720.0),
                "notepad",
                |state| {
                    state.screen = Screen::Notepad;
                    state.set_notepad(
                    "Things to sort out before Friday:\n\n- the draft\n- the numbers\n- who is picking up the room",
                );
                },
            );

            // The text at its largest, which is the size the narrow layout has
            // to survive.
            count += shoot(
                directory,
                theme,
                Size::new(width, 720.0),
                "notepad-large",
                |state| {
                    state.screen = Screen::Notepad;
                    state.config.notepad.font_size = timeslay::config::Notepad::MAX_FONT_SIZE;
                    state.set_notepad(
                        "Things to sort out before Friday:\n\n- the draft\n- the numbers"
                            .to_owned(),
                    );
                },
            );

            count += shoot(
                directory,
                theme,
                Size::new(width, 720.0),
                "notepad-small",
                |state| {
                    state.screen = Screen::Notepad;
                    state.config.notepad.font_size = timeslay::config::Notepad::MIN_FONT_SIZE;
                    state.set_notepad(
                        "Things to sort out before Friday:\n\n- the draft\n- the numbers"
                            .to_owned(),
                    );
                },
            );
        }
    }

    // And the widest layout, where the settings sit beside the calendar.
    for theme in ["Sakura", "Matcha"] {
        count += shoot(
            directory,
            theme,
            Size::new(900.0, 560.0),
            "wide-settings",
            |state| {
                state.screen = Screen::Settings;
            },
        );
    }

    println!("wrote {count} screenshots to {}", directory.display());
}

/// Renders one state and writes it out.
fn shoot(
    directory: &std::path::Path,
    theme: &str,
    size: Size,
    name: &str,
    setup: impl FnOnce(&mut State),
) -> usize {
    let mut config = Config::default();
    config.appearance.theme = theme.to_owned();

    let mut state = State::new(config, Catalog::built_in());
    setup(&mut state);

    let palette = state
        .catalog
        .resolve(&state.config.appearance.theme)
        .theme
        .clone();

    let element: Element<'_, Message> = app::view(&state, iced::window::Id::unique());
    let mut ui = Simulator::with_size(Settings::default(), size, element);

    let snapshot = ui.snapshot(&palette).expect("render the interface");

    let slug = format!(
        "{}-{}-{name}",
        theme.to_lowercase().replace(' ', "-"),
        size.width as u32
    );

    snapshot
        .matches_image(directory.join(slug))
        .expect("write the screenshot");

    1
}
