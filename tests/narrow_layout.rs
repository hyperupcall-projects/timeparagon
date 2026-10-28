//! Regression tests for the narrow layouts.
//!
//! The claim that TimeSlay is usable at 200px is easy to break by accident,
//! so these render the real interface headlessly at each breakpoint and check
//! that the controls are still there and still reachable. No compositor and
//! no golden images are involved, which is what makes them worth running on
//! every change.


use chrono::Local;
use iced::{Element, Point, Size};
use iced_test::core::Settings;
use iced_test::{Error, Simulator as Sim};

use time_slay::app::{self, Message, Screen, State};
use time_slay::config::Config;
use time_slay::theme::Catalog;

/// The widths that matter: the enforced minimum, the width the layout was
/// designed around, the compact/normal boundary, and the jump to two columns.
const WIDTHS: [f32; 4] = [180.0, 200.0, 260.0, 420.0];

/// Tall enough for the extras to be included in the layout.
const HEIGHT: f32 = 340.0;

fn state() -> State {
    State::new(Config::default(), Catalog::built_in())
}

type Simulator<'a> = Sim<'a, Message>;

fn render(state: &State, size: Size) -> Simulator<'_> {
    at(size, app::view(state, iced::window::Id::unique()))
}

fn at<'a>(size: Size, element: Element<'a, Message>) -> Simulator<'a> {
    Sim::with_size(Settings::default(), size, element)
}

/// Set Alarm has to be there, and has to work, at the narrowest size: it is
/// the one way into alarm setting mode.
#[test]
fn set_alarm_is_clickable_at_the_minimum_width() -> Result<(), Error> {
    let state = state();
    let mut ui = render(&state, Size::new(180.0, 240.0));

    let _ = ui.click("Set Alarm")?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages.iter().any(|message| matches!(
            message,
            Message::Alarms(time_slay::alarms::Action::ModeToggled)
        )),
        "clicking Set Alarm produced {messages:?}"
    );

    Ok(())
}

/// Set Alarm sits beside Menu at every width, and the old timer button and
/// its status line are gone from the foot of the calendar.
#[test]
fn set_alarm_replaces_the_timer_at_the_foot_of_the_calendar() {
    for width in WIDTHS {
        let state = state();
        let mut ui = render(&state, Size::new(width, 600.0));

        for label in ["Set Alarm", "Menu"] {
            assert!(ui.find(label).is_ok(), "{label:?} is missing at {width}px");
        }
        for label in ["Timer", "No target", "Go back"] {
            assert!(
                ui.find(label).is_err(),
                "{label:?} is still there at {width}px"
            );
        }
    }
}

/// Alarm setting mode says what a click will do, and the button stays put.
#[test]
fn alarm_mode_explains_itself() {
    let mut state = state();

    {
        let mut ui = render(&state, Size::new(260.0, 600.0));
        assert!(
            ui.find("Click a time to set an alarm. Esc cancels.")
                .is_err()
        );
    }

    state.alarms.mode = true;

    let mut ui = render(&state, Size::new(260.0, 600.0));
    assert!(
        ui.find("Click a time to set an alarm. Esc cancels.")
            .is_ok()
    );
    assert!(ui.find("Set Alarm").is_ok());
}

/// The popup offers what an alarm does, a way to remove it, and how far off it
/// is, and it fits the narrowest window.
#[test]
fn the_alarm_popup_is_usable_at_every_width() -> Result<(), Error> {
    for width in WIDTHS {
        let mut state = state();
        let at = time_slay::alarms::minute_of(Local::now()) + chrono::Duration::minutes(45);
        let id = state.alarms.add(at, state.config.alarm.clone());
        let _ = app::update(
            &mut state,
            Message::Alarms(time_slay::alarms::Action::Open(id)),
        );

        let mut ui = render(&state, Size::new(width, 700.0));

        for label in [
            "Alarm",
            "Goes off in 45m",
            "Time",
            "Label",
            // One control from each section the switches reveal: the
            // switches' own labels are not something the simulator can find.
            "Urgency",
            "Preview",
            "Volume 70%",
            "Strength 35%",
            "Delete",
            "Done",
        ] {
            assert!(ui.find(label).is_ok(), "{label:?} is missing at {width}px");
        }

        let _ = ui.click("Delete")?;
        let messages: Vec<Message> = ui.into_messages().collect();
        assert!(
            messages.iter().any(|message| matches!(
                message,
                Message::Alarms(time_slay::alarms::Action::Delete(deleted)) if *deleted == id
            )),
            "Delete produced {messages:?} at {width}px"
        );
    }

    Ok(())
}

/// The popup shows the alarm's own settings, not the timer's.
#[test]
fn the_alarm_popup_reflects_the_alarm_being_edited() {
    let mut state = state();
    let at = time_slay::alarms::minute_of(Local::now()) + chrono::Duration::minutes(150);
    let mut profile = state.config.alarm.clone();
    profile.sound.volume = 41;
    let id = state.alarms.add(at, profile);
    let _ = app::update(
        &mut state,
        Message::Alarms(time_slay::alarms::Action::Open(id)),
    );

    let mut ui = render(&state, Size::new(420.0, 900.0));

    assert!(ui.find("Goes off in 2h 30m").is_ok());
    assert!(ui.find("Volume 41%").is_ok());
}

/// Every setting has to be reachable at 200px, including the ones that are
/// dropdowns on a wider window.
///
/// The controls checked here are the ones each section reveals when it is
/// switched on, which is a stronger claim than the presence of the toggle:
/// it means the whole expanded section fits and laid out.
#[test]
fn the_settings_tab_is_usable_at_two_hundred_pixels() {
    let mut state = state();
    state.screen = Screen::Settings;

    let mut ui = render(&state, Size::new(200.0, 400.0));

    for label in [
        "Settings",
        // Appearance.
        "Theme",
        // Keyboard shortcuts.
        "Toggle Visibility",
        "Toggle Shape",
        "New shortcut",
        "Add shortcut",
        "Calendar width (px)",
        "Time block width (px)",
        "Details width (px)",
        "Screen margin (px, 0–30)",
        // Google Calendar.
        "OAuth client ID",
        "Sync every 15 minutes",
    ] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing at 200px");
    }
}

/// The compact theme picker is a button that cycles, because a dropdown
/// overlay has nowhere to go at this width.
#[test]
fn the_compact_theme_picker_cycles_on_press() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Settings;

    let mut ui = render(&state, Size::new(200.0, 400.0));

    let _ = ui.click("Sakura")?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::ThemeSelected(name) if name == "Peony")),
        "expected the next theme to be selected, got {messages:?}"
    );

    Ok(())
}

/// A fired alarm has to be silenceable from the foot of the calendar, and the
/// foot is empty the rest of the time.
#[test]
fn a_firing_alarm_offers_a_dismiss_button() {
    let mut state = state();

    for width in WIDTHS {
        state.alarm_active = false;
        let mut ui = render(&state, Size::new(width, HEIGHT));
        assert!(
            ui.find("Dismiss").is_err(),
            "nothing is ringing at {width}px"
        );
        drop(ui);

        state.alarm_active = true;
        let mut ui = render(&state, Size::new(width, HEIGHT));
        assert!(
            ui.find("Dismiss").is_ok(),
            "no way to dismiss the alarm at {width}px"
        );
    }
}

/// Rendering must not panic at any size, including absurd ones. A layout
/// that divides by a dimension is easy to break with a zero.
#[test]
fn rendering_never_panics_at_any_size() {
    let state = state();

    for size in [
        Size::new(1.0, 1.0),
        Size::new(180.0, 100.0),
        Size::new(200.0, 2_000.0),
        Size::new(4_000.0, 4_000.0),
    ] {
        let mut ui = render(&state, size);
        let _: Result<_, _> = ui.find("Set Alarm");
    }
}

/// Settings occupy their own tab at every width.
#[test]
fn the_settings_tab_uses_the_whole_window() {
    let mut state = state();
    state.screen = Screen::Settings;

    for width in [200.0, 420.0, 900.0] {
        let mut ui = render(&state, Size::new(width, 600.0));
        assert!(
            ui.find("Start").is_err(),
            "the clock is visible at {width}px"
        );
        assert!(
            ui.find("Done").is_err(),
            "settings have a Done button at {width}px"
        );
        assert!(
            ui.find("Show Google Tasks").is_err(),
            "settings have a task toggle at {width}px"
        );
        assert!(
            ui.find("Theme").is_ok(),
            "the settings are missing at {width}px"
        );
    }
}

/// The calendar keeps the day it already has on screen while a fresh sync is
/// in flight, and the status line says so. Both have to survive every width.
#[test]
fn the_calendar_keeps_the_day_while_it_reloads() {
    let mut state = state();
    state.screen = Screen::Calendar;
    let snapshot = time_slay::calendar::sample_snapshot();
    state.calendar = time_slay::calendar::Store::with_snapshot(snapshot);
    state.calendar.sync = time_slay::calendar::SyncState::Syncing;

    for width in WIDTHS {
        let mut ui = render(&state, Size::new(width, 720.0));

        assert!(
            ui.find("Team offsite").is_ok(),
            "the cached day is missing at {width}px"
        );
        assert!(
            ui.find("Syncing…").is_ok(),
            "the loading status is missing at {width}px"
        );
    }
}

/// The view used by the flash overlay draws without needing the clock's
/// widgets, and must not panic when there is no flash in flight.
#[test]
fn the_overlay_renders_with_and_without_an_active_flash() {
    let mut state = state();

    {
        let mut ui = at(
            Size::new(800.0, 600.0),
            time_slay::ui::flash::view(&state),
        );
        let _: Result<_, _> = ui.find("nothing");
    }

    state.flash = Some(app::Flash {
        window: iced::window::Id::unique(),
        started: std::time::Instant::now(),
        settings: time_slay::config::Flash::default(),
    });

    let mut ui = at(
        Size::new(800.0, 600.0),
        time_slay::ui::flash::view(&state),
    );
    let _: Result<_, _> = ui.find("nothing");
}

/// The calendar has to stay usable at 200px: the day heading, the menu, a
/// way to open Google Calendar even before anyone has signed in, and the way
/// in to setting an alarm.
#[test]
fn the_calendar_view_is_usable_at_two_hundred_pixels() {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar =
        time_slay::calendar::Store::with_snapshot(time_slay::calendar::sample_snapshot());

    let mut ui = render(&state, Size::new(200.0, 500.0));

    for label in ["Menu", "Team offsite", "Set Alarm"] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing at 200px");
    }
}

/// The left/right day arrows are gone: the month button opens a picker where
/// the user can tap a day directly instead of stepping one day at a time.
#[test]
fn the_calendar_offers_a_month_button_instead_of_day_arrows() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar =
        time_slay::calendar::Store::with_snapshot(time_slay::calendar::sample_snapshot());

    let mut ui = render(&state, Size::new(200.0, 500.0));

    for label in ["◀", "▶"] {
        assert!(
            ui.find(label).is_err(),
            "{label:?} should no longer be in the header"
        );
    }

    for label in ["Oct ▾", "Menu"] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing at 200px");
    }

    // The day in view sits in the title bar between the month picker and the menu.
    let today = time_slay::calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
    let heading = time_slay::calendar::day_heading(state.calendar.day, today);
    assert!(
        ui.find(heading.as_str()).is_ok(),
        "the day in view is missing from the title bar"
    );

    let _ = ui.click("Oct ▾")?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::CalendarMonthToggled)),
        "expected the month button to open the month picker, got {messages:?}"
    );

    Ok(())
}

/// The month picker has to fit at 200px, show the month, and let a day be
/// picked.
#[test]
fn the_month_picker_shows_the_month_and_picks_a_day() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar =
        time_slay::calendar::Store::with_snapshot(time_slay::calendar::sample_snapshot());
    state.calendar.month_open = true;

    let mut ui = render(&state, Size::new(200.0, 700.0));

    assert!(
        ui.find("October 2026").is_ok(),
        "the month title is missing"
    );
    for label in ["M", "T", "W", "F", "S"] {
        assert!(ui.find(label).is_ok(), "weekday {label:?} is missing");
    }

    let _ = ui.click("15")?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages.iter().any(|message| matches!(
            message,
            Message::CalendarDaySelected(day)
                if *day == chrono::NaiveDate::from_ymd_opt(2026, 10, 15).unwrap()
        )),
        "expected the tapped day to be selected, got {messages:?}"
    );

    Ok(())
}

#[test]
fn the_calendar_menu_lists_calendars_and_the_website_link() {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar =
        time_slay::calendar::Store::with_snapshot(time_slay::calendar::sample_snapshot());
    state.calendar.menu_open = true;

    let mut ui = render(&state, Size::new(200.0, 500.0));

    for label in [
        "Calendars",
        "Google Tasks",
        "Personal",
        "Work",
        "Tasks",
        "Sync now",
        "Open Google Calendar",
        "Dock to screen edge",
    ] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing from the menu");
    }
}

/// The tab label at `index` (Calendar, Inbox, Projects, Backlog, Travel, Keep,
/// Notepad, Settings) for a window of `width`.
fn tab_label(width: f32, index: usize) -> &'static str {
    time_slay::ui::tab_labels(width)[index]
}

/// Navigation is one row in three groups at every width, never overflowing:
/// Calendar, then Inbox/Projects/Backlog/Travel, then Keep/Notepad/Settings.
#[test]
fn the_tabs_are_one_row_in_three_groups() {
    let state = state();
    for width in [180.0, 200.0, 260.0, 420.0, 640.0, 900.0] {
        let mut ui = render(&state, Size::new(width, 500.0));
        let labels = time_slay::ui::tab_labels(width);
        let mut bounds = Vec::new();
        for label in labels {
            let found = ui.find(label).unwrap().bounds();
            assert!(
                found.x >= 0.0 && found.x + found.width <= width,
                "{label} extends beyond the window at {width}px"
            );
            bounds.push(found);
        }
        // Order: Calendar, Inbox, Projects, Backlog, Travel, Keep, Notepad, Settings.
        for pair in bounds.windows(2) {
            assert!(pair[0].x + pair[0].width <= pair[1].x, "order at {width}px");
            assert!(
                (pair[0].y - pair[1].y).abs() < 1.0,
                "not one row at {width}px"
            );
        }
        // Space separates the groups more than it separates tabs within one.
        let gap = |a: usize, b: usize| bounds[b].x - (bounds[a].x + bounds[a].width);
        assert!(gap(0, 1) > gap(1, 2) && gap(4, 5) > gap(5, 6));
    }
}

#[test]
fn projects_lists_the_defaults_alphabetically_and_can_be_edited() -> Result<(), Error> {
    let mut state = state();
    let mut ui = render(&state, Size::new(200.0, 500.0));
    ui.click(tab_label(200.0, 2))?;
    let message = ui
        .into_messages()
        .find(|message| matches!(message, Message::ScreenSelected(Screen::Projects)))
        .expect("Projects should select its view");
    let _ = app::update(&mut state, message);
    assert_eq!(state.screen, Screen::Projects);

    let mut ui = render(&state, Size::new(200.0, 500.0));
    let mut tops = Vec::new();
    for name in [
        "Personal",
        "Personal/Chores",
        "School/Calculus",
        "School/Homework",
        "Work",
    ] {
        tops.push(ui.find(name).expect(name).bounds().y);
    }
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{tops:?}");
    assert!(ui.find("Add").is_ok());
    assert!(ui.find("Set Alarm").is_err());
    assert!(ui.find("Theme").is_err());
    assert!(ui.find(tab_label(200.0, 0)).is_ok());
    drop(ui);

    // Editing a project exposes its name, color and folder.
    let id = state.config.projects.sorted()[0].id;
    let _ = app::update(
        &mut state,
        Message::Projects(time_slay::projects::Action::Edit(id)),
    );
    let mut ui = render(&state, Size::new(200.0, 700.0));
    for label in ["Name", "Color", "Folder", "Browse…", "Save", "Cancel"] {
        assert!(
            ui.find(label).is_ok(),
            "{label:?} is missing from the editor"
        );
    }
    Ok(())
}

/// The settings tab opens the settings, which is where it belongs now.
#[test]
fn the_settings_tab_opens_the_settings() -> Result<(), Error> {
    let state = state();
    let mut ui = render(&state, Size::new(420.0, 600.0));

    let _ = ui.click(tab_label(420.0, 7))?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::ScreenSelected(Screen::Settings))),
        "clicking the tab produced {messages:?}"
    );

    Ok(())
}

/// The aggregate Google Tasks swatch offers a shared color and a reset.
#[test]
fn google_tasks_can_be_given_a_shared_color() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar =
        time_slay::calendar::Store::with_snapshot(time_slay::calendar::sample_snapshot());
    state.calendar.menu_open = true;
    state.calendar.color_editor = Some("tasks:".into());

    let mut ui = render(&state, Size::new(200.0, 700.0));

    assert!(
        ui.find("Task color").is_ok(),
        "the picker is missing at 200px"
    );

    let _ = ui.click("Reset to default")?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::TaskColorPicked(id, None) if id == "tasks:")),
        "expected the reset to be offered, got {messages:?}"
    );

    Ok(())
}

#[test]
fn toggling_a_calendar_off_hides_its_events_in_the_day_view() {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar =
        time_slay::calendar::Store::with_snapshot(time_slay::calendar::sample_snapshot());
    state
        .config
        .calendar
        .set_calendar_enabled("personal", false);

    let mut ui = render(&state, Size::new(260.0, 600.0));

    assert!(
        ui.find("Team offsite").is_err(),
        "disabled calendar all-day events should not be listed"
    );
}

#[test]
fn opening_an_event_shows_its_google_details() {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar =
        time_slay::calendar::Store::with_snapshot(time_slay::calendar::sample_snapshot());
    let standup = state
        .calendar
        .events
        .iter_mut()
        .find(|event| event.id == "standup")
        .unwrap();
    standup.details = time_slay::calendar::Details {
        task_data: None,
        scheduled: None,
        location: "Room 4".into(),
        description: "Bring notes".into(),
        status: "Confirmed".into(),
        link: "https://calendar.google.com/event?eid=1".into(),
        meeting: "https://meet.google.com/abc-defg-hij".into(),
        organizer: "Ada".into(),
        attendees: vec!["Grace (accepted)".into()],
    };
    state.calendar.open_event = Some("standup".into());

    let mut ui = render(&state, Size::new(260.0, 600.0));

    for label in [
        "Standup",
        "Room 4",
        "Bring notes",
        "Confirmed",
        "Ada",
        "Grace (accepted)",
        "Join meeting",
        "Open in Google Calendar",
        "Close",
        "Work",
    ] {
        assert!(
            ui.find(label).is_ok(),
            "{label:?} is missing from the event details"
        );
    }
}

#[test]
fn a_task_details_card_links_to_google_tasks_without_a_status() {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar =
        time_slay::calendar::Store::with_snapshot(time_slay::calendar::sample_snapshot());
    let task = state
        .calendar
        .events
        .iter_mut()
        .find(|event| event.id == "all-day")
        .unwrap();
    task.id = "task:abc".into();
    task.calendar_id = "tasks:list".into();
    task.details.status = "Needs action".into();
    task.details.link = "https://tasks.google.com/task/abc".into();
    task.details.description = "Bring the form".into();
    state.calendar.open_event = Some("task:abc".into());

    let mut ui = render(&state, Size::new(260.0, 600.0));

    assert!(ui.find("Open in Google Tasks").is_ok());
    assert!(ui.find("Complete task").is_ok());
    assert!(ui.find("Name").is_ok());
    assert!(ui.find("Rename").is_ok());
    assert!(ui.find("Bring the form").is_ok());
    assert!(ui.find("Needs action").is_err());
    assert!(ui.find("Status").is_err());
}

/// A long description must not bury the action buttons. They live in a fixed
/// footer, so clicking one has to succeed without scrolling the details.
/// `Simulator::click` only fires when the target is actually visible, which is
/// exactly the property this is checking.
#[test]
fn a_long_event_keeps_its_buttons_out_of_the_scroll() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar =
        time_slay::calendar::Store::with_snapshot(time_slay::calendar::sample_snapshot());

    let standup = state
        .calendar
        .events
        .iter_mut()
        .find(|event| event.id == "standup")
        .unwrap();
    standup.details.location = "Room 4".into();
    standup.details.meeting = "https://meet.google.com/abc-defg-hij".into();
    standup.details.link = "https://calendar.google.com/event?eid=1".into();
    standup.details.description = (1..=80)
        .map(|line| format!("Line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    state.calendar.open_event = Some("standup".into());

    for size in [
        Size::new(180.0 + 360.0, 320.0),
        Size::new(200.0 + 360.0, 360.0),
        Size::new(260.0 + 360.0, 420.0),
    ] {
        let mut ui = render(&state, size);
        let _ = ui.click("Close")?;

        let messages: Vec<Message> = ui.into_messages().collect();

        assert!(
            messages
                .iter()
                .any(|message| matches!(message, Message::EventClosed)),
            "the Close button should be reachable at {size:?} without scrolling, got {messages:?}"
        );
    }

    Ok(())
}

/// Clicking an event near the top of the day view must ask for the details
/// card to open below it, so the clicked event stays visible. This drives the
/// real wire: the pointer watcher records the click, the canvas publishes the
/// open, and the update places the card. Nothing here may touch the scroll
/// position of the day grid.
#[test]
fn clicking_a_top_event_asks_for_the_details_below() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar =
        time_slay::calendar::Store::with_snapshot(time_slay::calendar::sample_snapshot());

    let mut ui = render(&state, Size::new(260.0, 600.0));
    let _ = ui.click("Team offsite")?;

    let messages: Vec<Message> = ui.into_messages().collect();
    for message in messages {
        let _ = app::update(&mut state, message);
    }

    assert!(
        state.calendar.open_event.is_some(),
        "clicking the all-day chip should open an event"
    );
    assert!(
        state.calendar.details_below,
        "an event in the top half should place the details below it"
    );

    Ok(())
}

/// The day view counts tasks without a time or time block. Tasks on the grid
/// via DUE_TIME, undated tasks, and tasks on other days stay out of the count.
#[test]
fn the_calendar_shows_how_many_tasks_are_not_planned() -> Result<(), Error> {
    let mut state = timeblocking_state();
    let day = state.calendar.day;
    let mut task = state.calendar.event("task:backlog").unwrap().clone();
    task.id = "task:today".into();
    task.title = "Outline today".into();
    task.all_day = true;
    task.start = day
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap();
    task.end = task.start + chrono::Duration::days(1);
    state.calendar.add_event(task.clone());

    task.id = "task:due-time".into();
    task.title = "Already on the calendar".into();
    task.details.description = "DUE_TIME: 09:30".into();
    time_slay::calendar::deadline::place(&mut task, 30);
    assert!(!task.all_day);
    // A due time places the task on the grid without a time-block assignment.
    assert!(time_slay::timeblocking::in_backlog(&task));
    state.calendar.add_event(task);
    assert!(
        state
            .calendar
            .timed_events(&state.config.calendar)
            .any(|event| event.id == "task:due-time")
    );

    let mut ui = render(&state, Size::new(260.0, 600.0));
    assert!(ui.find("1 task not planned").is_ok());
    assert!(ui.find("Show tasks").is_err());
    assert!(ui.find("Hide tasks").is_err());
    assert!(ui.find("Hide overdue tasks").is_err());
    assert!(ui.find("Show overdue tasks").is_err());
    assert!(ui.find("Drop tasks here to schedule this day").is_err());
    drop(ui);

    state.calendar.day = day.pred_opt().unwrap();
    let mut ui = render(&state, Size::new(260.0, 600.0));
    assert!(ui.find("0 tasks not planned").is_ok());
    assert!(ui.find("1 task not planned").is_err());

    Ok(())
}

/// Opening the details for one event and then clicking a different one must
/// switch straight to the new event. A full-screen dismiss layer used to eat
/// that click and only close the card, so the switch needed two clicks.
#[test]
fn clicking_another_event_switches_the_open_details() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Calendar;

    let mut snapshot = time_slay::calendar::sample_snapshot();
    let mut second = snapshot.events[0].clone();
    second.id = "second".into();
    second.title = "Second thing".into();
    snapshot.events.push(second);

    state.calendar = time_slay::calendar::Store::with_snapshot(snapshot);
    state.calendar.open_event = Some("all-day".into());
    state.calendar.details_below = true;

    let mut ui = render(&state, Size::new(320.0 + 360.0, 700.0));
    let _ = ui.click("Second thing")?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::EventOpened(id) if id == "second")),
        "clicking another event should open it, got {messages:?}"
    );

    Ok(())
}

/// Calendar connection and sync controls remain available in settings.
#[test]
fn the_settings_pane_includes_calendar_sync_controls() {
    let mut state = state();
    state.screen = Screen::Settings;

    let mut ui = render(&state, Size::new(200.0, 400.0));

    for label in [
        "Google Calendar",
        "OAuth client ID",
        "Resync calendar and tasks data",
        "Sync every 15 minutes",
        "Day ends at midnight",
    ] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing from settings");
    }
}

/// Resync is the settings way to throw away cached days and fetch again.
#[test]
fn resync_calendar_and_tasks_data_is_offered_once_signed_in() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Settings;
    state.calendar.connection = time_slay::calendar::Connection::SignedIn;

    let mut ui = render(&state, Size::new(420.0, 1800.0));

    let _ = ui.click("Resync calendar and tasks data")?;

    let messages: Vec<Message> = ui.into_messages().collect();
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::CalendarResyncRequested)),
        "clicking resync produced {messages:?}"
    );

    Ok(())
}

/// The notepad is a tab of its own and is reachable at every width.
#[test]
fn the_notepad_is_a_tab_at_the_top_of_the_calendar() {
    for width in WIDTHS {
        let state = state();

        let mut ui = render(&state, Size::new(width, HEIGHT));

        let notepad = tab_label(width, 6);
        assert!(ui.find(notepad).is_ok(), "the tab is missing at {width}px");

        let calendar = tab_label(width, 0);
        assert!(
            ui.find(calendar).is_ok(),
            "the way back is missing at {width}px"
        );
    }
}

/// The notepad tab opens the notepad.
#[test]
fn the_notepad_tab_opens_the_notepad() -> Result<(), Error> {
    let state = state();
    let mut ui = render(&state, Size::new(420.0, 600.0));

    let _ = ui.click(tab_label(420.0, 6))?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::ScreenSelected(Screen::Notepad))),
        "clicking the tab produced {messages:?}"
    );

    Ok(())
}

/// The notepad page offers the way back and the size control, and neither of
/// them may be lost in a narrow window.
#[test]
fn the_notepad_screen_is_usable_at_two_hundred_pixels() {
    let mut state = state();
    state.screen = Screen::Notepad;

    let mut ui = render(&state, Size::new(200.0, 600.0));

    for label in [tab_label(200.0, 6), tab_label(200.0, 0), "A-", "A+", "100%"] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing at 200px");
    }
}

/// The size control belongs to the notepad, so it changes the size of the
/// text on the page and nothing else.
#[test]
fn the_text_size_can_be_changed_from_the_notepad() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Notepad;

    let mut ui = render(&state, Size::new(260.0, 600.0));

    let _ = ui.click("A+")?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::NotepadFontSizeChanged(10))),
        "clicking A+ produced {messages:?}"
    );

    Ok(())
}

/// The size of the text is the notepad's own, so the rest of the interface is
/// left as it is drawn.
#[test]
fn the_settings_pane_has_no_text_size() {
    let mut state = state();
    state.screen = Screen::Settings;

    let mut ui = render(&state, Size::new(200.0, 400.0));

    assert!(
        ui.find("Text size").is_err(),
        "the text size should be the notepad's"
    );
    assert!(
        ui.find("A-").is_err(),
        "the size buttons should be on the notepad"
    );
}

/// Typing in the notepad has to be kept: the editor's actions have to reach
/// the state, and the state has to reach the config that is saved.
#[test]
fn typing_in_the_notepad_is_kept() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Notepad;

    let mut ui = render(&state, Size::new(260.0, 600.0));

    // Put the cursor in the page, which is what focuses the editor, and then
    // type into it.
    ui.point_at(Point::new(100.0, 400.0));
    ui.simulate(iced_test::simulator::click());
    ui.typewrite("milk");

    let messages: Vec<Message> = ui.into_messages().collect();

    for message in messages {
        let _ = app::update(&mut state, message);
    }

    assert_eq!(state.config.notepad.text, "milk");

    Ok(())
}

fn timeblocking_state() -> State {
    use time_slay::timeblocking::{Metadata, UNDATED, block_key};
    let mut state = state();
    let mut snapshot = time_slay::calendar::sample_snapshot();
    snapshot.day = Local::now().date_naive();
    state.config.calendar.time_block_calendar = Some("work".into());
    state.config.calendar.time_block_list = Some("tasks:list".into());
    let block = snapshot
        .events
        .iter_mut()
        .find(|e| e.id == "standup")
        .unwrap();
    block.start = chrono::Local::now()
        .date_naive()
        .and_hms_opt(1, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap();
    block.end = block.start + chrono::Duration::hours(1);
    let key = block_key(block);
    let mut task = block.clone();
    task.id = "task:planned".into();
    task.calendar_id = "tasks:list".into();
    task.title = "Write proposal".into();
    task.all_day = true;
    set_task_metadata(
        &mut task,
        "Proposal notes",
        Metadata {
            block: Some(key),
            minutes: 90,
            order: 1,
            ..Default::default()
        },
    );
    snapshot.events.push(task.clone());
    task.id = "task:backlog".into();
    task.title = "Research outline".into();
    task.start = UNDATED
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap();
    task.end = task.start + chrono::Duration::days(1);
    set_task_metadata(&mut task, "", Metadata::default());
    snapshot.events.push(task.clone());
    task.id = "task:overdue".into();
    task.title = "Send invoice".into();
    set_task_metadata(
        &mut task,
        &format!(
            "DUE: {}",
            (Local::now() - chrono::Duration::days(8)).date_naive()
        ),
        Metadata::default(),
    );
    task.start = Local::now() - chrono::Duration::days(8);
    task.end = task.start + chrono::Duration::days(1);
    snapshot.events.push(task);
    snapshot.calendars.push(time_slay::calendar::Calendar {
        id: "tasks:list".into(),
        name: "Inbox".into(),
        color: iced::Color::from_rgb8(66, 133, 244),
        primary: false,
        selected_by_google: true,
    });
    state.calendar = time_slay::calendar::Store::with_snapshot(snapshot);
    state
}

#[test]
fn time_block_click_opens_a_panel_and_keeps_it_open_when_switching_blocks() -> Result<(), Error> {
    let mut state = timeblocking_state();
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    assert_eq!(state.timeblocking.block.as_deref(), Some("standup"));
    assert_eq!(state.timeblocking.original_width, Some(260.0));
    assert!(state.calendar.open_event.is_none());
    let mut ui = render(&state, Size::new(660.0, 720.0));
    for label in [
        "Edit",
        "Delete",
        "Close",
        "Planned Tasks",
        "Write proposal",
        "Available tasks",
        "All tasks",
        "Show planned tasks",
        "Search available tasks…",
    ] {
        assert!(ui.find(label).is_ok(), "missing {label}");
    }
    assert!(ui.find("Show Event Details").is_err());
    assert!(ui.find("Edit time block").is_err());
    assert!(ui.find("90m planned / 60m available").is_err());
    assert!(ui.find("Add a todo").is_err());
    // Generate a preview only when explicitly requested for visual inspection.
    if std::env::var_os("TB_PREVIEW").is_some() {
        ui.snapshot(&iced::Theme::Light)
            .unwrap()
            .matches_image(std::path::Path::new("/tmp/timeblocking-preview"))
            .unwrap();
    }
    drop(ui);
    state.timeblocking.todo_open = true;
    state.timeblocking.todo_draft = "Call the client".into();
    let mut ui = render(&state, Size::new(900.0, 900.0));
    let _ = ui.click("Add task")?;
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::TimeBlocking(time_slay::timeblocking::Action::AddTodo(id)) if id == "standup"
    )));
    drop(state);
    let mut state = timeblocking_state();
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    let _ = app::update(&mut state, Message::ScreenSelected(Screen::Backlog));
    assert!(state.timeblocking.block.is_none());
    assert_eq!(state.timeblocking.original_width, None);
    Ok(())
}

#[test]
fn time_block_actions_share_one_row_and_delete_requires_confirmation() -> Result<(), Error> {
    use time_slay::timeblocking::Action;
    let mut state = timeblocking_state();
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));

    for width in [500.0, 660.0] {
        let mut ui = render(&state, Size::new(width, 720.0));
        let edit = ui.find("Edit")?.bounds();
        let delete = ui.find("Delete")?.bounds();
        let close = ui.find("Close")?.bounds();
        assert_eq!(edit.y, delete.y);
        assert_eq!(delete.y, close.y);
        assert!(edit.x + edit.width <= delete.x);
        assert!(delete.x + delete.width <= close.x);
        assert!(close.x + close.width <= width);
        let _ = ui.click("Delete")?;
        assert!(ui.into_messages().any(|message| matches!(
            message,
            Message::EventDeleteRequested(id) if id == "standup"
        )));
    }

    let mut ui = render(&state, Size::new(660.0, 720.0));
    let _ = ui.click("Close")?;
    assert!(
        ui.into_messages()
            .any(|message| matches!(message, Message::TimeBlocking(Action::Close)))
    );

    let _ = app::update(&mut state, Message::EventDeleteRequested("standup".into()));
    assert_eq!(state.calendar.pending_delete.as_deref(), Some("standup"));
    let mut ui = render(&state, Size::new(660.0, 720.0));
    assert!(ui.find("Delete this event?").is_ok());
    let _ = ui.click("Cancel")?;
    assert!(
        ui.into_messages()
            .any(|message| matches!(message, Message::EventDeleteCancelled))
    );
    let _ = app::update(&mut state, Message::EventDeleteCancelled);
    assert!(state.calendar.event("standup").is_some());
    assert!(state.timeblocking.block.is_some());
    Ok(())
}

#[test]
fn event_details_sit_to_the_right_of_the_time_block_editor() -> Result<(), Error> {
    use time_slay::timeblocking::Action;
    let mut state = timeblocking_state();
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::Details("task:planned".into())),
    );
    assert!(state.timeblocking.block.is_some());
    assert!(state.calendar.open_event.is_some());

    let mut ui = render(&state, Size::new(260.0 + 400.0 + 360.0, 900.0));
    let calendar = ui.find("Menu")?.bounds();
    let block = ui.find("Edit")?.bounds();
    let details = ui.find("Complete task")?.bounds();

    assert!(
        calendar.x < block.x,
        "the calendar should be left of the editor: {calendar:?} {block:?}"
    );
    assert!(
        block.x < details.x,
        "the editor should be left of the details: {block:?} {details:?}"
    );

    Ok(())
}

#[test]
fn planned_task_status_can_be_changed_and_undone_without_hiding_the_task() -> Result<(), Error> {
    use time_slay::timeblocking::{self as tb, TaskStatus};
    let mut state = timeblocking_state();
    let id = "task:planned";
    let metrics = time_slay::ui::layout::Metrics::of(Size::new(400.0, 1200.0));
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    let original = state.calendar.event(id).unwrap().clone();
    for (ignore, expected) in [
        (false, TaskStatus::Completed),
        (false, TaskStatus::Active),
        (true, TaskStatus::Ignored),
        (true, TaskStatus::Active),
        (true, TaskStatus::Ignored),
        (false, TaskStatus::Completed),
        (true, TaskStatus::Ignored),
        (true, TaskStatus::Active),
    ] {
        let previous = tb::task_status(state.calendar.event(id).unwrap());
        let message = if ignore {
            Message::TaskIgnoreToggled(id.into())
        } else {
            Message::TaskToggled(id.into())
        };
        let _ = app::update(&mut state, message);
        let task = state.calendar.event(id).unwrap().clone();
        assert_eq!(tb::task_status(&task), expected);
        assert_eq!(task.completed, expected != TaskStatus::Active);
        assert_eq!((task.start, task.end), (original.start, original.end));
        let _ = app::update(
            &mut state,
            Message::TaskCompletionSaved(id.into(), previous, Ok(task)),
        );
        assert!(!state.timeblocking.busy.contains(id));
        let task = state.calendar.event(id).unwrap();
        let mut actions = at(
            Size::new(260.0, 240.0),
            time_slay::ui::timeblocking::task_actions(&state, task, &metrics),
        );
        assert!(
            actions
                .find(if expected == TaskStatus::Completed {
                    "Uncomplete task"
                } else {
                    "Complete task"
                })
                .is_ok()
        );
        let _ = actions.click(if expected == TaskStatus::Ignored {
            "Unignore Task"
        } else {
            "Ignore Task"
        })?;
        assert!(actions.into_messages().any(|message| matches!(message,
            Message::TaskIgnoreToggled(id) if id == "task:planned")));
        assert!(
            tb::assigned(&state, state.calendar.event("standup").unwrap())
                .iter()
                .any(|task| task.id == id)
        );
        let mut ui = at(
            metrics.size,
            time_slay::ui::timeblocking::panel(&state, &metrics),
        );
        assert!(ui.find("−").is_ok());
        match expected {
            TaskStatus::Completed => assert!(ui.find("Completed ✓").is_ok()),
            TaskStatus::Ignored => assert!(ui.find("Ignored").is_ok()),
            TaskStatus::Active => assert!(ui.find("Write proposal").is_ok()),
        }
    }

    // A rejected unignore restores the ignored marker and keeps the assignment.
    let _ = app::update(&mut state, Message::TaskIgnoreToggled(id.into()));
    let task = state.calendar.event(id).unwrap().clone();
    let _ = app::update(
        &mut state,
        Message::TaskCompletionSaved(id.into(), TaskStatus::Active, Ok(task)),
    );
    let _ = app::update(&mut state, Message::TaskIgnoreToggled(id.into()));
    let _ = app::update(
        &mut state,
        Message::TaskCompletionSaved(id.into(), TaskStatus::Ignored, Err("Offline".into())),
    );
    assert_eq!(
        tb::task_status(state.calendar.event(id).unwrap()),
        TaskStatus::Ignored
    );
    assert_eq!(state.calendar.error.as_deref(), Some("Offline"));
    let mut ui = at(
        metrics.size,
        time_slay::ui::timeblocking::panel(&state, &metrics),
    );
    let _ = ui.click("−")?;
    assert!(ui.into_messages().any(|m| matches!(m,
        Message::TimeBlocking(tb::Action::Unassign(id)) if id == "task:planned")));
    Ok(())
}

/// The task editor has a project, a priority and a location. Picking one
/// saves it without closing the details.
#[test]
fn the_task_editor_sets_project_and_priority_and_offers_a_location() -> Result<(), Error> {
    use time_slay::timeblocking::{Action, Priority};
    use time_slay::travel::{Action as Travel, PickTarget};

    let mut state = timeblocking_state();
    let _ = app::update(&mut state, Message::EventOpened("task:backlog".into()));
    let size = Size::new(560.0, 1000.0);

    let mut ui = render(&state, size);
    for label in ["Project", "Priority", "None", "P1", "P2", "Location"] {
        assert!(
            ui.find(label).is_ok(),
            "{label:?} is missing from the editor"
        );
    }
    let _ = ui.click("P1")?;
    assert!(ui.into_messages().any(|message| matches!(message,
        Message::TimeBlocking(Action::Priority(id, Some(Priority::P1))) if id == "task:backlog")));

    // The location button opens the place search for this task.
    let mut ui = render(&state, size);
    let _ = ui.click("Add location")?;
    assert!(ui.into_messages().any(|message| matches!(message,
        Message::Travel(Travel::OpenPicker(PickTarget::Task(id))) if id == "task:backlog")));

    // The saved choice comes back on the task, and the details stay open.
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::Project("task:backlog".into(), Some(2))),
    );
    let mut saved = state.calendar.event("task:backlog").unwrap().clone();
    let mut data = saved
        .details
        .task_data
        .take()
        .unwrap_or(time_slay::task_db::TaskData {
            id: 100,
            metadata: Default::default(),
            legacy_key: None,
        });
    data.metadata.project = Some(2);
    data.metadata.priority = Some(Priority::P2);
    saved.details.task_data = Some(data);
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::Saved("task:backlog".into(), Ok(saved))),
    );
    assert_eq!(state.calendar.open_event.as_deref(), Some("task:backlog"));

    let task = state.calendar.event("task:backlog").unwrap();
    let metadata = time_slay::timeblocking::metadata(task);
    assert_eq!(
        (metadata.project, metadata.priority),
        (Some(2), Some(Priority::P2))
    );
    Ok(())
}

#[test]
fn task_actions_are_ordered_reachable_and_delete_requires_confirmation() -> Result<(), Error> {
    for screen in [Screen::Calendar, Screen::Backlog] {
        for width in WIDTHS {
            let mut state = timeblocking_state();
            state.screen = screen;
            let _ = app::update(&mut state, Message::EventOpened("task:backlog".into()));
            let size = Size::new(width, 800.0);
            // The details pane sits to the right of the view, so render the
            // window at the width it would be widened to.
            let render_size = Size::new(width + 360.0, 800.0);
            let mut ui = render(&state, render_size);
            let mut bottom = 0.0;
            for label in [
                "Complete task",
                "Ignore Task",
                "Move to backlog",
                "Delete task",
            ] {
                let bounds = ui.find(label)?.bounds();
                assert!(bounds.y >= bottom, "{label} is out of order");
                assert!(
                    bounds.y + bounds.height <= size.height,
                    "{label} is outside {width}px {screen:?}: {bounds:?}"
                );
                bottom = bounds.y + bounds.height;
            }
            let _ = ui.click("Ignore Task")?;
            assert!(ui.into_messages().any(|message| matches!(message,
                Message::TaskIgnoreToggled(id) if id == "task:backlog")));
            let mut ui = render(&state, render_size);
            let _ = ui.click("Delete task")?;
            assert!(ui.into_messages().any(|message| matches!(message,
                Message::TaskDeleteRequested(id) if id == "task:backlog")));
            let _ = app::update(
                &mut state,
                Message::TaskDeleteRequested("task:backlog".into()),
            );
            assert!(state.calendar.event("task:backlog").is_some());
            assert!(state.timeblocking.busy.is_empty());
            let mut ui = render(&state, render_size);
            assert!(
                ui.find("Are you sure you want to delete this task?")
                    .is_ok()
            );
            let _ = ui.click("Cancel")?;
            assert!(
                ui.into_messages()
                    .any(|message| matches!(message, Message::EventDeleteCancelled))
            );
            let _ = app::update(&mut state, Message::EventDeleteCancelled);
            assert!(state.calendar.event("task:backlog").is_some());
            assert!(state.calendar.pending_delete.is_none());
        }
    }
    let mut state = timeblocking_state();
    let task = state.calendar.event("task:planned").unwrap();
    let key = time_slay::timeblocking::task_key(task);
    state.config.task_work.start(
        key.clone(),
        task.title.clone(),
        time_slay::timeblocking::metadata(task).block,
        chrono::Utc::now(),
    );
    let _ = app::update(
        &mut state,
        Message::TaskDeleteConfirmed("task:planned".into()),
    );
    assert!(
        state.timeblocking.busy.is_empty(),
        "must request confirmation first"
    );
    let _ = app::update(
        &mut state,
        Message::TaskDeleteRequested("task:planned".into()),
    );
    let _ = app::update(
        &mut state,
        Message::TaskDeleteConfirmed("task:planned".into()),
    );
    assert!(state.timeblocking.busy.contains("task:planned"));
    let _ = app::update(
        &mut state,
        Message::TaskDeleted("task:planned".into(), Ok(())),
    );
    assert!(state.calendar.event("task:planned").is_none());
    assert!(state.calendar.event("standup").is_some());
    assert!(state.timeblocking.busy.is_empty());
    assert!(!state.config.task_work.running(&key));
    Ok(())
}

#[test]
fn the_planned_tasks_box_opens_a_task_field_and_accepts_a_drop() -> Result<(), Error> {
    use time_slay::timeblocking::Action;
    let mut state = timeblocking_state();
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::AvailableAll(true)),
    );
    let mut ui = render(&state, Size::new(900.0, 900.0));
    let _ = ui.click("Edit")?;
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::EventEditRequested(id) if id == "standup"
    )));

    let mut ui = render(&state, Size::new(900.0, 900.0));
    let _ = ui.click("+")?;
    assert!(
        ui.into_messages()
            .any(|message| matches!(message, Message::TimeBlocking(Action::OpenTodo)))
    );

    let mut ui = render(&state, Size::new(900.0, 900.0));
    let slot = ui.find("+")?;
    let task = ui.find("Research outline")?;
    let start = task.bounds().center();
    let end = slot.bounds().center();
    assert!(start.distance(end) >= 4.0);
    ui.point_at(start);
    ui.simulate([iced::Event::Mouse(iced::mouse::Event::ButtonPressed(
        iced::mouse::Button::Left,
    ))]);
    ui.point_at(end);
    ui.simulate([iced::Event::Mouse(iced::mouse::Event::CursorMoved {
        position: end,
    })]);
    ui.simulate([iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
        iced::mouse::Button::Left,
    ))]);
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(messages.iter().any(
        |message| matches!(message, Message::TimeBlocking(Action::Drag(id)) if id == "task:backlog")
    ));
    assert!(messages.iter().any(|message| {
        matches!(message, Message::TimeBlocking(Action::DropBlock(id)) if id == "standup")
    }));
    assert!(
        !messages
            .iter()
            .any(|message| matches!(message, Message::TimeBlocking(Action::OpenTodo)))
    );
    Ok(())
}

#[test]
fn available_tasks_can_be_filtered_without_hiding_planned_tasks() -> Result<(), Error> {
    use time_slay::timeblocking::Action;
    let mut state = timeblocking_state();
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::AvailableAll(true)),
    );
    let mut ui = render(&state, Size::new(900.0, 1000.0));
    assert!(ui.find("Research outline").is_ok());
    assert!(ui.find("Send invoice").is_ok());
    let search = ui.find("Search available tasks…")?;
    ui.point_at(search.bounds().center());
    ui.simulate(iced_test::simulator::click());
    ui.typewrite("invoice");
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::TimeBlocking(Action::AvailableQuery(query)) if query == "invoice"
    )));

    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::AvailableQuery("INVOICE".into())),
    );
    let mut ui = render(&state, Size::new(900.0, 1000.0));
    assert!(ui.find("Send invoice").is_ok());
    assert!(ui.find("Research outline").is_err());
    assert!(ui.find("Write proposal").is_ok());
    drop(ui);

    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::AvailableQuery("nope".into())),
    );
    let mut ui = render(&state, Size::new(900.0, 1000.0));
    assert!(ui.find("No matching tasks").is_ok());
    assert!(ui.find("Research outline").is_err());
    assert!(ui.find("Write proposal").is_ok());
    Ok(())
}

#[test]
fn available_tasks_hide_due_times_until_show_planned_is_enabled() -> Result<(), Error> {
    use time_slay::timeblocking::Action;
    let mut state = timeblocking_state();
    let mut task = state.calendar.event("task:backlog").unwrap().clone();
    task.id = "task:due-time".into();
    task.title = "Scheduled call".into();
    task.start = state
        .calendar
        .day
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap();
    task.end = task.start + chrono::Duration::days(1);
    task.details.description = "DUE_TIME: 09:30".into();
    time_slay::calendar::deadline::place(&mut task, 30);
    assert!(!task.all_day);
    state.calendar.add_event(task);

    // Check the field itself, including when the task has no due day yet.
    let mut task = state.calendar.event("task:backlog").unwrap().clone();
    task.id = "task:undated-due-time".into();
    task.title = "Undated call".into();
    task.details.description = "DUE_TIME: 10:00".into();
    state.calendar.add_event(task);
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::AvailableAll(true)),
    );

    // Render just the panel so a task on the calendar cannot mask list filtering.
    let size = Size::new(400.0, 1500.0);
    let metrics = time_slay::ui::layout::Metrics::of(size);
    let mut ui = at(size, time_slay::ui::timeblocking::panel(&state, &metrics));
    assert!(ui.find("Research outline").is_ok());
    assert!(ui.find("Scheduled call").is_err());
    assert!(ui.find("Undated call").is_err());
    let all = ui.find("All tasks")?.bounds();
    let planned = ui.find("Show planned tasks")?.bounds();
    assert!(planned.y > all.y);
    ui.point_at(Point::new(
        planned.x + planned.width + 20.0,
        planned.center().y,
    ));
    ui.simulate(iced_test::simulator::click());
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::TimeBlocking(Action::AvailablePlanned(true))
    )));

    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::AvailablePlanned(true)),
    );
    let mut ui = at(size, time_slay::ui::timeblocking::panel(&state, &metrics));
    assert!(ui.find("Scheduled call").is_ok());
    assert!(ui.find("Undated call").is_ok());
    assert!(ui.find("Research outline").is_ok());
    drop(ui);

    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::AvailableAll(false)),
    );
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::AvailableQuery("CALL".into())),
    );
    let mut ui = at(size, time_slay::ui::timeblocking::panel(&state, &metrics));
    assert!(ui.find("Scheduled call").is_ok());
    assert!(ui.find("Undated call").is_err());
    assert!(ui.find("Research outline").is_err());
    drop(ui);

    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::AvailablePlanned(false)),
    );
    let mut ui = at(size, time_slay::ui::timeblocking::panel(&state, &metrics));
    assert!(ui.find("Scheduled call").is_err());
    assert!(ui.find("No matching tasks").is_ok());
    drop(ui);

    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::AvailablePlanned(true)),
    );
    let _ = app::update(&mut state, Message::TimeBlocking(Action::Close));
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    assert!(!state.timeblocking.available_planned);
    assert!(!state.timeblocking.available_all);
    Ok(())
}

#[test]
fn available_tasks_default_to_today_and_can_show_all_dates() -> Result<(), Error> {
    use time_slay::timeblocking::Action;
    let mut state = timeblocking_state();
    let today = Local::now().date_naive();
    let mut later = state.calendar.event("task:backlog").unwrap().clone();
    later.id = "task:later".into();
    later.title = "Plan Friday".into();
    let tomorrow = today + chrono::Duration::days(1);
    later.start = tomorrow
        .and_hms_opt(9, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap();
    later.end = later.start + chrono::Duration::days(1);
    state.calendar.add_event(later);
    let task = state.calendar.event_mut("task:backlog").unwrap();
    task.start = today
        .and_hms_opt(9, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .unwrap();
    task.end = task.start + chrono::Duration::days(1);
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));

    let size = Size::new(400.0, 1000.0);
    let metrics = time_slay::ui::layout::Metrics::of(size);
    let mut ui = at(size, time_slay::ui::timeblocking::panel(&state, &metrics));
    assert!(!state.timeblocking.available_all);
    assert!(ui.find("Research outline").is_ok());
    assert!(ui.find("Plan Friday").is_err());
    assert!(ui.find("Send invoice").is_err());
    let label = ui.find("All tasks")?;
    let bounds = label.bounds();
    ui.point_at(Point::new(
        bounds.x + bounds.width + 20.0,
        bounds.center().y,
    ));
    ui.simulate(iced_test::simulator::click());
    assert!(
        ui.into_messages()
            .any(|message| matches!(message, Message::TimeBlocking(Action::AvailableAll(true))))
    );

    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::AvailableAll(true)),
    );
    let mut ui = at(size, time_slay::ui::timeblocking::panel(&state, &metrics));
    assert!(ui.find("Research outline").is_ok());
    assert!(ui.find("Plan Friday").is_ok());
    assert!(ui.find("Send invoice").is_ok());
    assert!(ui.find("Write proposal").is_ok());
    drop(ui);

    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::AvailableAll(false)),
    );
    let mut ui = at(size, time_slay::ui::timeblocking::panel(&state, &metrics));
    assert!(ui.find("Research outline").is_ok());
    assert!(ui.find("Plan Friday").is_err());
    assert!(ui.find("Send invoice").is_err());
    Ok(())
}

#[test]
fn backlog_shows_unscheduled_tasks_with_independent_deadline_and_time_editors() -> Result<(), Error>
{
    let mut state = timeblocking_state();
    state.screen = Screen::Backlog;
    let mut ui = render(&state, Size::new(420.0, 720.0));
    assert!(ui.find("Research outline").is_ok());
    assert!(ui.find("Inbox").is_ok());
    assert!(ui.find("Write proposal").is_err());
    assert!(ui.find("Send invoice").is_ok());
    let _ = ui.click("Research outline")?;
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::EventOpened(id) if id == "task:backlog"))
    );
    let _ = app::update(&mut state, Message::EventOpened("task:backlog".into()));
    assert_eq!(state.timeblocking.title, "Research outline");
    let mut ui = render(&state, Size::new(420.0, 720.0));
    for label in [
        "Inbox",
        "Name",
        "Rename",
        "Deadline",
        "Save deadline",
        "Scheduled day and time",
        "Schedule time",
        "Move to backlog",
    ] {
        assert!(ui.find(label).is_ok());
    }
    drop(ui);
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(time_slay::timeblocking::Action::Title(
            "Revised outline".into(),
        )),
    );
    let mut ui = render(&state, Size::new(420.0, 720.0));
    let _ = ui.click("Rename")?;
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::TimeBlocking(time_slay::timeblocking::Action::SaveTitle(id)) if id == "task:backlog"
    )));
    Ok(())
}

#[test]
fn backlog_shows_each_task_list_and_filters_by_search_and_list() -> Result<(), Error> {
    use time_slay::timeblocking::{Action, Metadata};
    let mut state = timeblocking_state();
    state.screen = Screen::Backlog;
    state
        .calendar
        .calendars
        .push(time_slay::calendar::Calendar {
            id: "tasks:errands".into(),
            name: "Errands".into(),
            color: iced::Color::from_rgb8(52, 168, 83),
            primary: false,
            selected_by_google: true,
        });
    let mut errand = state.calendar.event("task:backlog").unwrap().clone();
    errand.id = "task:errand".into();
    errand.calendar_id = "tasks:errands".into();
    errand.title = "Buy stamps".into();
    set_task_metadata(&mut errand, "", Metadata::default());
    state.calendar.add_event(errand);

    let mut ui = render(&state, Size::new(420.0, 720.0));
    for label in [
        "All lists",
        "Inbox",
        "Errands",
        "Research outline",
        "Buy stamps",
    ] {
        assert!(ui.find(label).is_ok(), "missing {label}");
    }
    let search = ui.find("Search tasks or lists…")?;
    ui.point_at(search.bounds().center());
    ui.simulate(iced_test::simulator::click());
    ui.typewrite("stamp");
    assert!(ui.into_messages().any(|message| matches!(
        message,
        Message::TimeBlocking(Action::BacklogQuery(query)) if query == "stamp"
    )));

    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::BacklogQuery("STAMP".into())),
    );
    let mut ui = render(&state, Size::new(420.0, 720.0));
    assert!(ui.find("Buy stamps").is_ok());
    assert!(ui.find("Research outline").is_err());
    assert!(ui.find("Send invoice").is_err());
    drop(ui);

    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::BacklogQuery("errands".into())),
    );
    let mut ui = render(&state, Size::new(420.0, 720.0));
    assert!(ui.find("Buy stamps").is_ok());
    assert!(ui.find("Research outline").is_err());
    drop(ui);

    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::BacklogQuery(String::new())),
    );
    let mut ui = render(&state, Size::new(420.0, 720.0));
    let _ = ui.click("Errands")?;
    let list_message = ui.into_messages().find(|message| {
        matches!(message, Message::TimeBlocking(Action::BacklogList(Some(id))) if id == "tasks:errands")
    });
    assert!(
        list_message.is_some(),
        "the Errands filter did not select that list"
    );
    let _ = app::update(&mut state, list_message.unwrap());
    let mut ui = render(&state, Size::new(420.0, 720.0));
    assert!(ui.find("Buy stamps").is_ok());
    assert!(ui.find("Research outline").is_err());
    drop(ui);
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::BacklogList(Some("tasks:list".into()))),
    );
    let mut ui = render(&state, Size::new(180.0, 720.0));
    assert!(ui.find("Research outline").is_ok());
    assert!(ui.find("Send invoice").is_ok());
    assert!(ui.find("Buy stamps").is_err());
    for label in ["Search tasks or lists…", "Inbox", "Research outline"] {
        let bounds = ui.find(label)?.bounds();
        assert!(
            bounds.x >= -1.0 && bounds.x + bounds.width <= 181.0,
            "{label} escapes the 180px window: {bounds:?}"
        );
    }
    drop(ui);

    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::BacklogQuery("nope".into())),
    );
    let mut ui = render(&state, Size::new(420.0, 720.0));
    assert!(ui.find("No matching tasks").is_ok());
    assert!(ui.find("Research outline").is_err());
    Ok(())
}

#[test]
fn overdue_tasks_can_be_hidden_without_hiding_backlog_or_events() {
    let mut state = timeblocking_state();
    assert!(
        state
            .calendar
            .all_day_events(&state.config.calendar)
            .any(|e| e.id == "task:overdue")
    );
    assert!(
        !state
            .calendar
            .all_day_events(&state.config.calendar)
            .any(|e| e.id == "task:backlog")
    );
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(time_slay::timeblocking::Action::HideOverdue(true)),
    );
    assert!(
        !state
            .calendar
            .all_day_events(&state.config.calendar)
            .any(|e| e.id == "task:overdue")
    );
    assert!(state.calendar.event("task:backlog").is_some());
    assert!(state.calendar.event("standup").is_some());
}

#[test]
fn the_calendar_has_a_backlog_control_and_no_todos_menu() -> Result<(), Error> {
    let state = state();
    let mut ui = render(&state, Size::new(260.0, 600.0));
    assert!(ui.find("Todos").is_err());
    assert!(ui.find("0 tasks not planned").is_ok());
    let _ = ui.click(tab_label(260.0, 3))?;
    assert!(
        ui.into_messages()
            .any(|m| matches!(m, Message::ScreenSelected(Screen::Backlog)))
    );
    Ok(())
}

#[test]
fn task_drag_handles_start_a_drag_and_task_rows_accept_a_drop() -> Result<(), Error> {
    use time_slay::timeblocking::Action;
    let state = timeblocking_state();
    let task = state.calendar.event("task:backlog").unwrap();
    let metrics = time_slay::ui::layout::Metrics::of(Size::new(400.0, 70.0));
    let mut ui = at(
        Size::new(400.0, 70.0),
        time_slay::ui::timeblocking::task_row(&state, task, &metrics, false),
    );
    let origin = ui.find("⠿")?.bounds().center();
    ui.point_at(origin);
    ui.simulate([iced::Event::Mouse(iced::mouse::Event::ButtonPressed(
        iced::mouse::Button::Left,
    ))]);
    let position = Point::new(origin.x, origin.y + 8.0);
    ui.point_at(position);
    ui.simulate([iced::Event::Mouse(iced::mouse::Event::CursorMoved {
        position,
    })]);
    ui.simulate([iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
        iced::mouse::Button::Left,
    ))]);
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::TimeBlocking(Action::Drag(id)) if id == "task:backlog"))
    );
    assert!(
        messages.iter().any(
            |m| matches!(m, Message::TimeBlocking(Action::DropTask(id)) if id == "task:backlog")
        )
    );
    Ok(())
}

#[test]
fn invalid_rescheduling_keeps_the_task_and_reports_the_date_problem() {
    let mut state = timeblocking_state();
    state.timeblocking.date = "not a date".into();
    let before = state.calendar.event("task:backlog").unwrap().clone();
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(time_slay::timeblocking::Action::Reschedule(
            "task:backlog".into(),
        )),
    );
    assert_eq!(state.calendar.event("task:backlog"), Some(&before));
    assert!(state.timeblocking.busy.is_empty());
    assert!(
        state
            .calendar
            .error
            .as_deref()
            .unwrap()
            .contains("YYYY-MM-DD")
    );
}

#[test]
fn a_sync_started_before_a_task_edit_cannot_replace_the_edited_tasks() {
    let mut state = timeblocking_state();
    state.timeblocking.revision = 2;
    let _ = app::update(
        &mut state,
        Message::CalendarSyncedAt(1, Ok(time_slay::calendar::sample_snapshot())),
    );
    assert!(state.calendar.event("task:planned").is_some());
    assert!(state.calendar.event("task:backlog").is_some());
}

fn focus_state() -> State {
    use time_slay::timeblocking::{self as tb, Action};
    let mut state = timeblocking_state();
    let now = chrono::Utc::now();
    let block = state.calendar.event_mut("standup").unwrap();
    block.start = (now - chrono::Duration::minutes(20)).with_timezone(&Local);
    block.end = block.start + chrono::Duration::hours(1);
    let block_key = tb::block_key(block);
    let planned = state.calendar.event_mut("task:planned").unwrap();
    let mut meta = tb::metadata(&planned);
    meta.minutes = 30;
    planned.details.task_data.as_mut().unwrap().metadata = meta.clone();
    let key = tb::task_key(planned);
    let mut second = planned.clone();
    second.id = "task:second".into();
    second.title = "Review the draft".into();
    meta.minutes = 20;
    meta.order = 2;
    set_task_metadata(&mut second, "", meta.clone());
    state.calendar.add_event(second.clone());
    second.id = "task:done".into();
    second.title = "Gather references".into();
    second.completed = true;
    meta.order = 3;
    set_task_metadata(&mut second, "", meta.clone());
    state.calendar.add_event(second);
    state.config.task_work.start(
        key,
        "Write proposal".into(),
        Some(block_key),
        now - chrono::Duration::minutes(31),
    );
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::View("standup".into())),
    );
    state
}

#[test]
fn focused_view_uses_a_modal_without_opening_or_expanding_the_sidebar() {
    let mut state = focus_state();
    assert_eq!(state.timeblocking.focus.as_deref(), Some("standup"));
    assert!(state.timeblocking.block.is_none());
    assert!(state.timeblocking.original_width.is_none());
    let mut ui = render(&state, Size::new(900.0, 720.0));
    for label in [
        "FOCUS · TIME BLOCK",
        "Write proposal",
        "Review the draft",
        "Gather references",
        "Start",
        "Stop",
        "Complete",
        "Completed ✓",
    ] {
        assert!(ui.find(label).is_ok(), "missing {label}");
    }
    if std::env::var_os("FOCUS_PREVIEW").is_some() {
        ui.snapshot(&iced::Theme::Light)
            .unwrap()
            .matches_image(std::path::Path::new("/tmp/focus-preview"))
            .unwrap();
    }
    drop(ui);
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(time_slay::timeblocking::Action::CloseView),
    );
    assert!(state.timeblocking.focus.is_none());
    assert!(
        state.config.task_work.active().is_some(),
        "closing the view should retain the running task"
    );
}

#[test]
fn work_sessions_stop_on_completion_and_remain_visible_and_saved() {
    use time_slay::timeblocking::{self as tb, Action};
    let mut state = focus_state();
    let key = tb::task_key(state.calendar.event("task:planned").unwrap());
    let _ = app::update(&mut state, Message::TaskToggled("task:planned".into()));
    assert!(!state.config.task_work.running(&key));
    assert!(state.calendar.event("task:planned").unwrap().completed);
    let block = state.calendar.event("standup").unwrap();
    assert!(
        tb::assigned_all(&state, block)
            .iter()
            .any(|e| e.id == "task:planned")
    );
    let elapsed = state.config.task_work.seconds(&key, chrono::Utc::now());
    assert!(elapsed >= 31 * 60);
    assert_eq!(
        elapsed,
        state
            .config
            .task_work
            .seconds(&key, chrono::Utc::now() + chrono::Duration::days(1))
    );
    let saved = toml::to_string(&state.config).unwrap();
    let restored: Config = toml::from_str(&saved).unwrap();
    assert_eq!(restored.task_work, state.config.task_work);
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::StartWork("task:done".into())),
    );
    assert!(
        state.config.task_work.active().is_none(),
        "completed tasks cannot start a timer"
    );
}

#[test]
fn start_and_stop_are_local_and_do_not_change_google_task_status() {
    use time_slay::timeblocking::{self as tb, Action};
    let mut state = focus_state();
    let second = state.calendar.event("task:second").unwrap();
    let key = tb::task_key(second);
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::StartWork("task:second".into())),
    );
    assert!(state.config.task_work.running(&key));
    assert_eq!(
        state
            .config
            .task_work
            .sessions
            .iter()
            .filter(|s| s.stopped_at.is_none())
            .count(),
        1
    );
    let _ = app::update(
        &mut state,
        Message::TimeBlocking(Action::StopWork("task:second".into())),
    );
    assert!(state.config.task_work.active().is_none());
    assert!(!state.calendar.event("task:second").unwrap().completed);
    assert!(state.timeblocking.busy.is_empty());
}

#[test]
fn the_focus_overlay_blocks_controls_in_the_underlying_calendar() -> Result<(), Error> {
    let state = focus_state();
    let mut ui = render(&state, Size::new(900.0, 720.0));
    let _ = ui.click("Set Alarm")?;
    assert!(!ui.into_messages().any(|m| matches!(m, Message::Alarms(_))));
    Ok(())
}

#[test]
fn focused_timer_controls_remain_reachable_at_narrow_widths() -> Result<(), Error> {
    use time_slay::timeblocking::Action;
    for width in [200.0, 320.0, 900.0] {
        let state = focus_state();
        let mut ui = render(&state, Size::new(width, 720.0));
        let _ = ui.click("Stop")?;
        assert!(
            ui.into_messages().any(
                |m| matches!(m,Message::TimeBlocking(Action::StopWork(id)) if id == "task:planned")
            ),
            "Stop did not work at {width}px"
        );
    }
    Ok(())
}

#[test]
fn google_sync_keeps_completed_block_tasks_and_stops_their_running_session() {
    let mut state = focus_state();
    let mut snapshot = time_slay::calendar::sample_snapshot();
    snapshot.day = state.calendar.day;
    snapshot.events = state.calendar.events.clone();
    snapshot
        .events
        .iter_mut()
        .find(|e| e.id == "task:planned")
        .unwrap()
        .completed = true;
    let _ = app::update(&mut state, Message::CalendarSynced(Ok(snapshot)));
    assert!(state.config.task_work.active().is_none());
    let block = state.calendar.event("standup").unwrap();
    assert!(
        time_slay::timeblocking::assigned_all(&state, block)
            .iter()
            .any(|e| e.id == "task:planned" && e.completed)
    );
}

#[test]
fn an_unconfigured_event_offers_a_direct_way_to_enable_its_time_block_controls() -> Result<(), Error>
{
    use time_slay::timeblocking::{self as tb, Action};
    let mut state = timeblocking_state();
    state.config.calendar.time_block_calendar = None;
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    assert!(!tb::is_block(
        state.calendar.event("standup").unwrap(),
        &state.config.calendar
    ));
    let mut ui = render(&state, Size::new(420.0, 720.0));
    let _ = ui.click("Use this calendar for time blocks")?;
    let action = ui
        .into_messages()
        .find(|m| matches!(m, Message::TimeBlocking(Action::Calendar(_))))
        .expect("calendar selection action");
    let _ = app::update(&mut state, action);
    assert!(tb::is_block(
        state.calendar.event("standup").unwrap(),
        &state.config.calendar
    ));
    assert!(state.calendar.open_event.is_none());
    assert!(state.timeblocking.block.is_none());
    assert!(state.timeblocking.original_width.is_none());
    Ok(())
}

#[test]
fn keep_navigation_and_filters_fit_every_width() -> Result<(), Error> {
    for width in WIDTHS {
        let mut state = state();
        let mut ui = render(&state, Size::new(width, HEIGHT));
        ui.click(tab_label(width, 5))?;
        assert!(
            ui.into_messages()
                .any(|m| matches!(m, Message::ScreenSelected(Screen::KeepNotes)))
        );
        let mut ui = render(&state, Size::new(width, HEIGHT));
        ui.click(tab_label(width, 3))?;
        assert!(
            ui.into_messages()
                .any(|m| matches!(m, Message::ScreenSelected(Screen::Backlog)))
        );
        state.screen = Screen::KeepNotes;
        let mut ui = render(&state, Size::new(width, HEIGHT));
        for label in ["Refresh", "Reconnect", "Active", "Labels"] {
            assert!(ui.find(label).is_ok(), "{label} missing at {width}");
        }
    }
    Ok(())
}

#[test]
fn keep_edit_opens_the_selected_note() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::KeepNotes;
    state.keep.snapshot.notes.push(time_slay::keep::Note {
        id: "test-note".into(),
        url: "https://keep.google.com/#NOTE/test-note".into(),
        title: "Example".into(),
        description: "Body".into(),
        ..Default::default()
    });
    let mut ui = render(&state, Size::new(260.0, 720.0));
    ui.click("Edit")?;
    assert!(ui.into_messages().any(
        |m| matches!(m, Message::KeepEdit(url) if url == "https://keep.google.com/#NOTE/test-note")
    ));
    Ok(())
}

#[test]
fn switching_to_keep_closes_time_block_views_and_starts_refresh() {
    let mut state = focus_state();
    state.timeblocking.block = Some("standup".into());
    state.timeblocking.original_width = Some(260.0);
    state.timeblocking.dragging = Some("task:planned".into());
    let _ = app::update(&mut state, Message::ScreenSelected(Screen::KeepNotes));
    assert_eq!(state.screen, Screen::KeepNotes);
    assert!(state.timeblocking.block.is_none());
    assert!(state.timeblocking.focus.is_none());
    assert!(state.timeblocking.original_width.is_none());
    assert!(state.timeblocking.dragging.is_none());
    assert!(state.keep.loading);
    assert!(state.config.task_work.active().is_some());
}

fn set_task_metadata(
    event: &mut time_slay::calendar::Event,
    prose: &str,
    metadata: time_slay::timeblocking::Metadata,
) {
    let id = match event.id.as_str() {
        "task:planned" => 100,
        "task:backlog" => 101,
        "task:overdue" => 102,
        "task:second" => 103,
        "task:done" => 104,
        "task:errand" => 105,
        other => panic!("Unknown fixture task: {other}"),
    };
    event.details.description = prose.into();
    event.details.task_data = Some(time_slay::task_db::TaskData {
        id,
        metadata,
        legacy_key: None,
    });
}

#[test]
fn deadline_and_time_editors_stay_inside_narrow_backlog_windows() -> Result<(), Error> {
    for width in WIDTHS {
        let mut state = timeblocking_state();
        state.screen = Screen::Backlog;
        state
            .calendar
            .event_mut("task:backlog")
            .unwrap()
            .details
            .description = "DUE: 2026-12-31\nNotes".into();
        let _ = app::update(&mut state, Message::EventOpened("task:backlog".into()));
        assert_eq!(state.timeblocking.deadline, "2026-12-31");
        let mut ui = render(&state, Size::new(width, 720.0));
        for label in [
            "Rename",
            "Save deadline",
            "Schedule time",
            "Move to backlog",
        ] {
            let bounds = ui.find(label)?.bounds();
            assert!(
                bounds.x >= 0.0 && bounds.x + bounds.width <= width + 1.0,
                "{label} escapes the {width}px window: {bounds:?}"
            );
        }
    }
    Ok(())
}
