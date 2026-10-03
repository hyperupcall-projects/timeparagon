//! Renders the interface at each breakpoint and theme, headlessly.
//!
//! Run with `cargo run --example screenshots`. The PNGs land in
//! `screenshots/` and are how the narrow layouts get eyeballed without
//! resizing a window by hand forty times.

use std::time::Duration;

use iced::{Element, Size};
use iced_test::Simulator;
use iced_test::core::Settings;

use timetracker::app::{self, Message, Screen, State};
use timetracker::config::Config;
use timetracker::theme::Catalog;

/// The widths the layout is designed around.
const WIDTHS: [f32; 4] = [180.0, 200.0, 260.0, 420.0];

/// The themes worth looking at: ours, plus one built-in of each tone.
const THEMES: [&str; 7] = [
    "Sakura",
    "Peony",
    "Matcha",
    "Mint",
    "Sage",
    "Catppuccin Latte",
    "Dark",
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

            count += shoot(directory, theme, size, "armed", |state| {
                state.timer.set_target(Some(Duration::from_secs(1_500)));
            });

            count += shoot(directory, theme, size, "running", |state| {
                state.timer.set_target(Some(Duration::from_secs(300)));
                state.timer.start(state.now);
                state.now += Duration::from_secs(73);
            });

            count += shoot(directory, theme, size, "count-up", |state| {
                state.timer.set_target(None);
                state.timer.start(state.now);
                state.now += Duration::from_secs(45);
            });

            count += shoot(directory, theme, size, "finished", |state| {
                state.timer.set_target(Some(Duration::from_secs(300)));
                state.timer.start(state.now);
                let _ = state.timer.poll(state.now + Duration::from_secs(300));
                state.alarm_active = true;
            });

            count += shoot(directory, theme, size, "settings", |state| {
                state.screen = Screen::Settings;
            });

            count += shoot(directory, theme, size, "set-time", |state| {
                state.screen = Screen::SetTime;
            });

            count += shoot(directory, theme, Size::new(width, 720.0), "calendar", |state| {
                state.screen = Screen::Calendar;
                state.calendar = timetracker::calendar::Store::with_snapshot(
                    timetracker::calendar::sample_snapshot(),
                );
            });
        }
    }

    // A squarish window, which is where the readout has the headroom for a
    // progress ring. The ring is dropped automatically in short windows, so
    // this is the only shape that exercises it.
    for theme in ["Sakura", "Matcha"] {
        count += shoot(
            directory,
            theme,
            Size::new(360.0, 520.0),
            "ring",
            |state| {
                state.timer.set_target(Some(Duration::from_secs(300)));
                state.timer.start(state.now);
                state.now += Duration::from_secs(73);
            },
        );
    }

    // And the widest layout, where the settings sit beside the clock.
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
