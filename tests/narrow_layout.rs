//! Regression tests for the narrow layouts.
//!
//! The claim that TimeTracker is usable at 200px is easy to break by accident,
//! so these render the real interface headlessly at each breakpoint and check
//! that the controls are still there and still reachable. No compositor and
//! no golden images are involved, which is what makes them worth running on
//! every change.

use std::time::Duration;

use chrono::Local;
use iced::{Element, Point, Size};
use iced_test::core::Settings;
use iced_test::{Error, Simulator as Sim};

use timetracker::app::{self, Message, Screen, State};
use timetracker::config::Config;
use timetracker::theme::Catalog;

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

/// Every width has to offer the controls the clock cannot do without, once
/// the clock has been unfolded out of the calendar.
#[test]
fn the_clock_controls_survive_every_width() {
    for width in WIDTHS {
        let mut state = state();
        state.timer_open = true;
        let mut ui = render(&state, Size::new(width, HEIGHT));

        for label in ["Start", "Reset", "Set Time", "Alarm", "0:00", "of 5:00"] {
            assert!(
                ui.find(label).is_ok(),
                "{label:?} is missing at {width}px"
            );
        }
    }
}

/// Pressing Timer has to actually unfold and start the clock at the
/// narrowest size, which is the part a broken layout silently takes away.
#[test]
fn set_current_task_is_clickable_at_the_minimum_width() -> Result<(), Error> {
    let state = state();
    let mut ui = render(&state, Size::new(180.0, 240.0));

    let _ = ui.click("Timer")?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages.iter().any(|message| matches!(message, Message::TimerOpened)),
        "clicking Timer produced {messages:?}"
    );

    Ok(())
}

/// The clock lives inside the calendar: the button at the foot of the day
/// view unfolds it, and folds it away again without stopping the timer.
#[test]
fn the_calendar_unfolds_the_clock_and_folds_it_away() -> Result<(), Error> {
    let mut state = state();

    {
        let mut ui = render(&state, Size::new(260.0, 600.0));
        assert!(ui.find("Timer").is_ok(), "the way in is missing");
    }

    state.timer_open = true;
    state.timer.start(state.now);

    let mut ui = render(&state, Size::new(260.0, 600.0));

    for label in ["Pause", "Set Time", "Reset", "Alarm", "Go back"] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing with the clock open");
    }

    let _ = ui.click("Go back")?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::TimerClosed)),
        "folding the clock away produced {messages:?}"
    );

    Ok(())
}

/// Below 240px the hours group is folded into the minutes so the target
/// readout still fits; above it, hours are spelled out.
#[test]
fn the_target_readout_sheds_the_hours_group_when_it_has_to() {
    let mut state = state();
    state.timer.set_target(Some(Duration::from_secs(3_905)));

    let mut narrow = render(&state, Size::new(200.0, HEIGHT));
    assert!(narrow.find("of 65:05").is_ok(), "expected 65:05 at 200px");

    let mut wide = render(&state, Size::new(420.0, HEIGHT));
    assert!(wide.find("of 1:05:05").is_ok(), "expected 1:05:05 at 420px");
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
        // Side panel.
        "New shortcut",
        "Apply shortcut",
        // Google Calendar.
        "OAuth client ID",
        "Sync every 15 minutes",
    ] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing at 200px");
    }
}

/// The alarm settings are their own page, reached from the clock, and have to
/// stay usable at 200px just like the main settings pane.
#[test]
fn the_alarm_settings_pane_is_usable_at_two_hundred_pixels() {
    let mut state = state();
    state.screen = Screen::TimerSettings;

    let mut ui = render(&state, Size::new(200.0, 400.0));

    for label in [
        "Alarm",
        "Done",
        // Notification.
        "Urgency",
        // Sound.
        "Volume 70%",
        "Play 2 times",
        "Preview",
        // Flash.
        "Strength 35%",
        "Pulses 2",
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

/// The set-time pane has to offer presets, a way to nudge the target, and a
/// way to turn the target off, however narrow the window is.
#[test]
fn the_set_time_screen_is_usable_at_two_hundred_pixels() {
    let mut state = state();
    state.screen = Screen::SetTime;

    let mut ui = render(&state, Size::new(200.0, 400.0));

    for label in ["Set Time", "Done", "5m", "No target", "-1m", "+1m"] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing at 200px");
    }
}

/// A fired alarm has to be silenceable however the clock is showing: as the
/// clock's own button, and as the foot of the calendar when it is put away.
#[test]
fn a_firing_alarm_offers_a_dismiss_button() {
    let mut state = state();
    state.alarm_active = true;

    for width in WIDTHS {
        state.timer_open = false;

        {
            let mut ui = render(&state, Size::new(width, HEIGHT));

            assert!(
                ui.find("Dismiss").is_ok(),
                "no way to dismiss the alarm at {width}px"
            );
        }

        state.timer_open = true;

        {
            let mut ui = render(&state, Size::new(width, HEIGHT));

            assert!(
                ui.find("Dismiss").is_ok(),
                "no way to dismiss the alarm at {width}px with the clock open"
            );
        }
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
        let _: Result<_, _> = ui.find("Timer");
    }
}

/// Settings occupy their own tab at every width.
#[test]
fn the_settings_tab_uses_the_whole_window() {
    let mut state = state();
    state.screen = Screen::Settings;
    state.timer_open = true;

    for width in [200.0, 420.0, 900.0] {
        let mut ui = render(&state, Size::new(width, 600.0));
        assert!(ui.find("Start").is_err(), "the clock is visible at {width}px");
        assert!(ui.find("Done").is_err(), "settings have a Done button at {width}px");
        assert!(ui.find("Show Google Tasks").is_err(), "settings have a task toggle at {width}px");
        assert!(ui.find("Theme").is_ok(), "the settings are missing at {width}px");
    }
}

/// The calendar keeps the day it already has on screen while a fresh sync is
/// in flight, and the status line says so. Both have to survive every width.
#[test]
fn the_calendar_keeps_the_day_while_it_reloads() {
    let mut state = state();
    state.screen = Screen::Calendar;
    let snapshot = timetracker::calendar::sample_snapshot();
    state.calendar = timetracker::calendar::Store::with_snapshot(snapshot);
    state.calendar.sync = timetracker::calendar::SyncState::Syncing;

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
        let mut ui = at(Size::new(800.0, 600.0), timetracker::ui::flash::view(&state));
        let _: Result<_, _> = ui.find("nothing");
    }

    state.flash = Some(app::Flash {
        window: iced::window::Id::unique(),
        started: std::time::Instant::now(),
        settings: timetracker::config::Flash::default(),
    });

    let mut ui = at(Size::new(800.0, 600.0), timetracker::ui::flash::view(&state));
    let _: Result<_, _> = ui.find("nothing");
}

/// The calendar has to stay usable at 200px: the day heading, the menu, a
/// way to open Google Calendar even before anyone has signed in, and the way
/// in to the clock.
#[test]
fn the_calendar_view_is_usable_at_two_hundred_pixels() {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar = timetracker::calendar::Store::with_snapshot(timetracker::calendar::sample_snapshot());

    let mut ui = render(&state, Size::new(200.0, 500.0));

    for label in ["Menu", "Team offsite", "Timer"] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing at 200px");
    }
}

/// The left/right day arrows are gone: the month button opens a picker where
/// the user can tap a day directly instead of stepping one day at a time.
#[test]
fn the_calendar_offers_a_month_button_instead_of_day_arrows() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar = timetracker::calendar::Store::with_snapshot(timetracker::calendar::sample_snapshot());

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
    let today = timetracker::calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
    let heading = timetracker::calendar::day_heading(state.calendar.day, today);
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
    state.calendar = timetracker::calendar::Store::with_snapshot(timetracker::calendar::sample_snapshot());
    state.calendar.month_open = true;

    let mut ui = render(&state, Size::new(200.0, 700.0));

    assert!(ui.find("October 2026").is_ok(), "the month title is missing");
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
    state.calendar = timetracker::calendar::Store::with_snapshot(timetracker::calendar::sample_snapshot());
    state.calendar.menu_open = true;

    let mut ui = render(&state, Size::new(200.0, 500.0));

    for label in ["Calendars", "Show Google Tasks", "Personal", "Work", "Tasks", "Sync now", "Open Google Calendar"] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing from the menu");
    }
}

/// The tabs along the top are the way between the pages, so all three of them
/// have to be there: the notepad, the calendar and the settings.
#[test]
fn the_tabs_carry_the_notepad_the_calendar_and_the_settings() {
    let state = state();

    // A narrow window has room for shorter names, but all three tabs are
    // still there.
    let mut narrow = render(&state, Size::new(200.0, 500.0));
    for label in ["Notes", "Cal", "Setup"] {
        assert!(narrow.find(label).is_ok(), "{label:?} is missing from the tabs");
    }

    let mut wide = render(&state, Size::new(420.0, 500.0));
    for label in ["Notepad", "Calendar", "Settings"] {
        assert!(wide.find(label).is_ok(), "{label:?} is missing from the tabs");
    }
}

/// The settings tab opens the settings, which is where it belongs now.
#[test]
fn the_settings_tab_opens_the_settings() -> Result<(), Error> {
    let state = state();
    let mut ui = render(&state, Size::new(420.0, 600.0));

    let _ = ui.click("Settings")?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages.iter().any(|message| matches!(
            message,
            Message::ScreenSelected(Screen::Settings)
        )),
        "clicking the tab produced {messages:?}"
    );

    Ok(())
}

/// A Google Tasks list is the one kind of calendar whose color can be
/// changed, so opening its picker has to fit and offer a way back to the
/// default.
#[test]
fn a_task_list_can_be_given_its_own_color() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar =
        timetracker::calendar::Store::with_snapshot(timetracker::calendar::sample_snapshot());
    state.calendar.menu_open = true;
    // The sample shortens the task list id; real ones carry the `tasks:` prefix.
    state.calendar.calendars[2].id = "tasks:list".into();
    state.calendar.color_editor = Some("tasks:list".into());

    let mut ui = render(&state, Size::new(200.0, 700.0));

    assert!(ui.find("Task color").is_ok(), "the picker is missing at 200px");

    let _ = ui.click("Reset to default")?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::TaskColorPicked(id, None) if id == "tasks:list")),
        "expected the reset to be offered, got {messages:?}"
    );

    Ok(())
}

#[test]
fn toggling_a_calendar_off_hides_its_events_in_the_day_view() {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar = timetracker::calendar::Store::with_snapshot(timetracker::calendar::sample_snapshot());
    state.config.calendar.set_calendar_enabled("personal", false);

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
    state.calendar = timetracker::calendar::Store::with_snapshot(timetracker::calendar::sample_snapshot());
    let standup = state
        .calendar
        .events
        .iter_mut()
        .find(|event| event.id == "standup")
        .unwrap();
    standup.details = timetracker::calendar::Details {
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
        assert!(ui.find(label).is_ok(), "{label:?} is missing from the event details");
    }
}

#[test]
fn a_task_details_card_links_to_google_tasks_without_a_status() {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar = timetracker::calendar::Store::with_snapshot(timetracker::calendar::sample_snapshot());
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
    assert!(ui.find("Mark complete").is_ok());
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
        timetracker::calendar::Store::with_snapshot(timetracker::calendar::sample_snapshot());

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
        Size::new(180.0, 240.0),
        Size::new(200.0, 300.0),
        Size::new(260.0, 400.0),
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
    state.calendar = timetracker::calendar::Store::with_snapshot(timetracker::calendar::sample_snapshot());

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

/// The clock keeps reporting itself at the foot of the calendar, so the
/// elapsed time and target are visible even with the clock put away.
#[test]
fn the_calendar_view_shows_the_clock_status() {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.timer.set_target(Some(Duration::from_secs(300)));

    let mut ui = render(&state, Size::new(320.0, 600.0));

    assert!(ui.find("0:00").is_ok(), "the elapsed time is missing");
    assert!(ui.find("of 5:00").is_ok(), "the target is missing");
}

/// The day view carries its own switch for Google Tasks, beside the sync
/// line, so they can be hidden without opening the menu.
#[test]
fn the_calendar_offers_a_tasks_toggle_beside_the_sync_line() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar =
        timetracker::calendar::Store::with_snapshot(timetracker::calendar::sample_snapshot());

    let mut ui = render(&state, Size::new(200.0, 600.0));
    assert!(ui.find("Hide tasks").is_ok(), "the tasks toggle is missing");

    let _ = ui.click("Hide tasks")?;
    let messages: Vec<Message> = ui.into_messages().collect();
    assert!(
        messages
            .iter()
            .any(|message| matches!(message, Message::ShowTasksToggled(false))),
        "expected the toggle to ask for the tasks to be hidden, got {messages:?}"
    );

    // Once they are hidden the same button offers to bring them back.
    state.config.calendar.show_tasks = false;
    let mut ui = render(&state, Size::new(200.0, 600.0));
    assert!(ui.find("Show tasks").is_ok(), "the toggle should offer to restore tasks");

    Ok(())
}

/// Opening the details for one event and then clicking a different one must
/// switch straight to the new event. A full-screen dismiss layer used to eat
/// that click and only close the card, so the switch needed two clicks.
#[test]
fn clicking_another_event_switches_the_open_details() -> Result<(), Error> {
    let mut state = state();
    state.screen = Screen::Calendar;

    let mut snapshot = timetracker::calendar::sample_snapshot();
    let mut second = snapshot.events[0].clone();
    second.id = "second".into();
    second.title = "Second thing".into();
    snapshot.events.push(second);

    state.calendar = timetracker::calendar::Store::with_snapshot(snapshot);
    state.calendar.open_event = Some("all-day".into());
    state.calendar.details_below = true;

    let mut ui = render(&state, Size::new(320.0, 700.0));
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

/// The open clock keeps its way back to the calendar.
#[test]
fn the_timer_view_offers_only_a_way_back() {
    let mut state = state();
    state.timer_open = true;

    let mut ui = render(&state, Size::new(260.0, 600.0));

    assert!(ui.find("Go back").is_ok(), "the way back is missing");
    assert!(ui.find("Todos").is_err(), "the scratchpad button should be gone");
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
        "Sync every 15 minutes",
        "Day ends at midnight",
    ] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing from settings");
    }
}

/// The notepad is a tab of its own, along the top and to the left of the
/// calendar, and it has to be reachable at every width.
#[test]
fn the_notepad_is_a_tab_at_the_top_of_the_calendar() {
    for width in WIDTHS {
        let state = state();

        let mut ui = render(&state, Size::new(width, HEIGHT));

        let notepad = if width < 260.0 { "Notes" } else { "Notepad" };
        assert!(ui.find(notepad).is_ok(), "the tab is missing at {width}px");

        let calendar = if width < 260.0 { "Cal" } else { "Calendar" };
        assert!(ui.find(calendar).is_ok(), "the way back is missing at {width}px");
    }
}

/// The notepad tab opens the notepad.
#[test]
fn the_notepad_tab_opens_the_notepad() -> Result<(), Error> {
    let state = state();
    let mut ui = render(&state, Size::new(420.0, 600.0));

    let _ = ui.click("Notepad")?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages.iter().any(|message| matches!(
            message,
            Message::ScreenSelected(Screen::Notepad)
        )),
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

    for label in ["Notes", "Cal", "A-", "A+", "100%"] {
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

    assert!(ui.find("Text size").is_err(), "the text size should be the notepad's");
    assert!(ui.find("A-").is_err(), "the size buttons should be on the notepad");
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
    use timetracker::timeblocking::{Metadata, notes, block_key, UNDATED};
    let mut state = state();
    let mut snapshot = timetracker::calendar::sample_snapshot();
    snapshot.day = Local::now().date_naive();
    state.config.calendar.time_block_calendar = Some("work".into());
    state.config.calendar.time_block_list = Some("tasks:list".into());
    let block = snapshot.events.iter_mut().find(|e| e.id == "standup").unwrap();
    block.start = chrono::Local::now().date_naive().and_hms_opt(1,0,0).unwrap().and_local_timezone(Local).unwrap();
    block.end = block.start + chrono::Duration::hours(1);
    let key = block_key(block);
    let mut task = block.clone();
    task.id = "task:planned".into(); task.calendar_id = "tasks:list".into(); task.title = "Write proposal".into(); task.all_day = true;
    task.details.description = notes("Proposal notes", &Metadata { block: Some(key), minutes: 90, order: 1 }).unwrap();
    snapshot.events.push(task.clone());
    task.id = "task:backlog".into(); task.title = "Research outline".into();
    task.start = UNDATED.and_hms_opt(0,0,0).unwrap().and_local_timezone(Local).unwrap();
    task.end = task.start + chrono::Duration::days(1);
    task.details.description = notes("", &Metadata::default()).unwrap();
    snapshot.events.push(task.clone());
    task.id = "task:overdue".into(); task.title = "Send invoice".into();
    task.start = Local::now() - chrono::Duration::days(8); task.end = task.start + chrono::Duration::days(1);
    snapshot.events.push(task);
    state.calendar = timetracker::calendar::Store::with_snapshot(snapshot);
    state
}

#[test]
fn time_block_click_opens_a_panel_and_keeps_it_open_when_switching_blocks() {
    let mut state = timeblocking_state();
    let _ = app::update(&mut state, Message::EventOpened("standup".into()));
    assert_eq!(state.timeblocking.block.as_deref(), Some("standup"));
    assert_eq!(state.timeblocking.original_width, Some(260.0));
    assert!(state.calendar.open_event.is_none());
    let mut ui = render(&state, Size::new(660.0, 720.0));
    for label in ["90m planned / 60m available", "Write proposal", "Available tasks"] {
        assert!(ui.find(label).is_ok(), "missing {label}");
    }
    // Generate a preview only when explicitly requested for visual inspection.
    if std::env::var_os("TB_PREVIEW").is_some() {
        ui.snapshot(&iced::Theme::Light).unwrap().matches_image(std::path::Path::new("/tmp/timeblocking-preview")).unwrap();
    }
    drop(ui);
    let _ = app::update(&mut state, Message::ScreenSelected(Screen::Backlog));
    assert!(state.timeblocking.block.is_none());
    assert_eq!(state.timeblocking.original_width, None);
}

#[test]
fn backlog_shows_only_undated_incomplete_google_tasks_and_opens_rescheduling() -> Result<(), Error> {
    let mut state = timeblocking_state();
    state.screen = Screen::Backlog;
    let mut ui = render(&state, Size::new(420.0, 720.0));
    assert!(ui.find("Research outline").is_ok());
    assert!(ui.find("Write proposal").is_err());
    assert!(ui.find("Send invoice").is_err());
    let _ = ui.click("Reschedule")?;
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(messages.iter().any(|m| matches!(m, Message::EventOpened(id) if id == "task:backlog")));
    let _ = app::update(&mut state, Message::EventOpened("task:backlog".into()));
    let mut ui = render(&state, Size::new(420.0, 720.0));
    for label in ["Due date", "Save estimate", "Move to Backlog"] { assert!(ui.find(label).is_ok()); }
    Ok(())
}

#[test]
fn overdue_tasks_can_be_hidden_without_hiding_backlog_or_events() {
    let mut state = timeblocking_state();
    assert!(state.calendar.all_day_events(&state.config.calendar).any(|e| e.id == "task:overdue"));
    assert!(!state.calendar.all_day_events(&state.config.calendar).any(|e| e.id == "task:backlog"));
    let _ = app::update(&mut state, Message::TimeBlocking(timetracker::timeblocking::Action::HideOverdue(true)));
    assert!(!state.calendar.all_day_events(&state.config.calendar).any(|e| e.id == "task:overdue"));
    assert!(state.calendar.event("task:backlog").is_some());
    assert!(state.calendar.event("standup").is_some());
}

#[test]
fn the_calendar_has_backlog_and_overdue_controls_and_no_todos_menu() -> Result<(), Error> {
    let state = state();
    let mut ui = render(&state, Size::new(260.0, 600.0));
    assert!(ui.find("Todos").is_err());
    assert!(ui.find("Hide overdue tasks").is_ok());
    let _ = ui.click("Backlog")?;
    assert!(ui.into_messages().any(|m| matches!(m, Message::ScreenSelected(Screen::Backlog))));
    Ok(())
}

#[test]
fn task_drag_handles_start_a_drag_and_task_rows_accept_a_drop() -> Result<(), Error> {
    use timetracker::timeblocking::Action;
    let state = timeblocking_state();
    let task = state.calendar.event("task:backlog").unwrap();
    let metrics = timetracker::ui::layout::Metrics::of(Size::new(400.0, 70.0));
    let mut ui = at(Size::new(400.0, 70.0), timetracker::ui::timeblocking::task_row(&state, task, &metrics, false));
    let _ = ui.click("⠿")?;
    let messages: Vec<_> = ui.into_messages().collect();
    assert!(messages.iter().any(|m| matches!(m, Message::TimeBlocking(Action::Drag(id)) if id == "task:backlog")));
    assert!(messages.iter().any(|m| matches!(m, Message::TimeBlocking(Action::DropTask(id)) if id == "task:backlog")));
    Ok(())
}

#[test]
fn invalid_rescheduling_keeps_the_task_and_reports_the_date_problem() {
    let mut state = timeblocking_state();
    state.timeblocking.date = "not a date".into();
    let before = state.calendar.event("task:backlog").unwrap().clone();
    let _ = app::update(&mut state, Message::TimeBlocking(timetracker::timeblocking::Action::Reschedule("task:backlog".into())));
    assert_eq!(state.calendar.event("task:backlog"), Some(&before));
    assert!(state.timeblocking.busy.is_empty());
    assert!(state.calendar.error.as_deref().unwrap().contains("YYYY-MM-DD"));
}

#[test]
fn a_sync_started_before_a_task_edit_cannot_replace_the_edited_tasks() {
    let mut state = timeblocking_state();
    state.timeblocking.revision = 2;
    let _ = app::update(&mut state, Message::CalendarSyncedAt(1, Ok(timetracker::calendar::sample_snapshot())));
    assert!(state.calendar.event("task:planned").is_some());
    assert!(state.calendar.event("task:backlog").is_some());
}

fn focus_state() -> State {
    use timetracker::timeblocking::{self as tb, Action};
    let mut state = timeblocking_state();
    let now = chrono::Utc::now();
    let block = state.calendar.event_mut("standup").unwrap();
    block.start = (now - chrono::Duration::minutes(20)).with_timezone(&Local);
    block.end = block.start + chrono::Duration::hours(1);
    let block_key = tb::block_key(block);
    let planned = state.calendar.event_mut("task:planned").unwrap();
    let mut meta = tb::metadata(&planned.details.description);
    meta.minutes = 30;
    planned.details.description = tb::notes(&planned.details.description,&meta).unwrap();
    let key = tb::task_key(planned);
    let mut second = planned.clone();
    second.id = "task:second".into(); second.title = "Review the draft".into();
    meta.minutes = 20; meta.order = 2;
    second.details.description = tb::notes("",&meta).unwrap();
    state.calendar.add_event(second.clone());
    second.id = "task:done".into(); second.title = "Gather references".into(); second.completed = true;
    meta.order = 3;
    second.details.description = tb::notes("",&meta).unwrap();
    state.calendar.add_event(second);
    state.config.task_work.start(key,"Write proposal".into(),Some(block_key),now-chrono::Duration::minutes(31));
    let _ = app::update(&mut state,Message::TimeBlocking(Action::View("standup".into())));
    state
}

#[test]
fn focused_view_uses_a_modal_without_opening_or_expanding_the_sidebar() {
    let mut state = focus_state();
    assert_eq!(state.timeblocking.focus.as_deref(),Some("standup"));
    assert!(state.timeblocking.block.is_none());
    assert!(state.timeblocking.original_width.is_none());
    let mut ui = render(&state,Size::new(900.0,720.0));
    for label in ["FOCUS · TIME BLOCK", "Write proposal", "Review the draft", "Gather references", "Start", "Stop", "Complete", "Completed ✓"] {
        assert!(ui.find(label).is_ok(),"missing {label}");
    }
    if std::env::var_os("FOCUS_PREVIEW").is_some() {
        ui.snapshot(&iced::Theme::Light).unwrap().matches_image(std::path::Path::new("/tmp/focus-preview")).unwrap();
    }
    drop(ui);
    let _ = app::update(&mut state,Message::TimeBlocking(timetracker::timeblocking::Action::CloseView));
    assert!(state.timeblocking.focus.is_none());
    assert!(state.config.task_work.active().is_some(),"closing the view should retain the running task");
}

#[test]
fn work_sessions_stop_on_completion_and_remain_visible_and_saved() {
    use timetracker::timeblocking::{self as tb, Action};
    let mut state = focus_state();
    let key = tb::task_key(state.calendar.event("task:planned").unwrap());
    let _ = app::update(&mut state,Message::TaskToggled("task:planned".into()));
    assert!(!state.config.task_work.running(&key));
    assert!(state.calendar.event("task:planned").unwrap().completed);
    let block = state.calendar.event("standup").unwrap();
    assert!(tb::assigned_all(&state,block).iter().any(|e| e.id == "task:planned"));
    let elapsed = state.config.task_work.seconds(&key,chrono::Utc::now());
    assert!(elapsed >= 31*60);
    assert_eq!(elapsed,state.config.task_work.seconds(&key,chrono::Utc::now()+chrono::Duration::days(1)));
    let saved = toml::to_string(&state.config).unwrap();
    let restored: Config = toml::from_str(&saved).unwrap();
    assert_eq!(restored.task_work,state.config.task_work);
    let _ = app::update(&mut state,Message::TimeBlocking(Action::StartWork("task:done".into())));
    assert!(state.config.task_work.active().is_none(),"completed tasks cannot start a timer");
}

#[test]
fn start_and_stop_are_local_and_do_not_change_google_task_status() {
    use timetracker::timeblocking::{self as tb, Action};
    let mut state = focus_state();
    let second = state.calendar.event("task:second").unwrap();
    let key = tb::task_key(second);
    let _ = app::update(&mut state,Message::TimeBlocking(Action::StartWork("task:second".into())));
    assert!(state.config.task_work.running(&key));
    assert_eq!(state.config.task_work.sessions.iter().filter(|s| s.stopped_at.is_none()).count(),1);
    let _ = app::update(&mut state,Message::TimeBlocking(Action::StopWork("task:second".into())));
    assert!(state.config.task_work.active().is_none());
    assert!(!state.calendar.event("task:second").unwrap().completed);
    assert!(state.timeblocking.busy.is_empty());
}

#[test]
fn the_focus_overlay_blocks_controls_in_the_underlying_calendar() -> Result<(),Error> {
    let state = focus_state();
    let mut ui = render(&state,Size::new(900.0,720.0));
    let _ = ui.click("Timer")?;
    assert!(!ui.into_messages().any(|m| matches!(m,Message::TimerOpened)));
    Ok(())
}

#[test]
fn focused_timer_controls_remain_reachable_at_narrow_widths() -> Result<(),Error> {
    use timetracker::timeblocking::Action;
    for width in [200.0,320.0,900.0] {
        let state = focus_state();
        let mut ui = render(&state,Size::new(width,720.0));
        let _ = ui.click("Stop")?;
        assert!(ui.into_messages().any(|m| matches!(m,Message::TimeBlocking(Action::StopWork(id)) if id == "task:planned")),"Stop did not work at {width}px");
    }
    Ok(())
}

#[test]
fn google_sync_keeps_completed_block_tasks_and_stops_their_running_session() {
    let mut state = focus_state();
    let mut snapshot = timetracker::calendar::sample_snapshot();
    snapshot.day = state.calendar.day;
    snapshot.events = state.calendar.events.clone();
    snapshot.events.iter_mut().find(|e| e.id == "task:planned").unwrap().completed = true;
    let _ = app::update(&mut state,Message::CalendarSynced(Ok(snapshot)));
    assert!(state.config.task_work.active().is_none());
    let block = state.calendar.event("standup").unwrap();
    assert!(timetracker::timeblocking::assigned_all(&state,block).iter().any(|e| e.id == "task:planned" && e.completed));
}

#[test]
fn an_unconfigured_event_offers_a_direct_way_to_enable_its_time_block_controls() -> Result<(),Error> {
    use timetracker::timeblocking::{self as tb,Action};
    let mut state = timeblocking_state();
    state.config.calendar.time_block_calendar = None;
    let _ = app::update(&mut state,Message::EventOpened("standup".into()));
    assert!(!tb::is_block(state.calendar.event("standup").unwrap(),&state.config.calendar));
    let mut ui = render(&state,Size::new(420.0,720.0));
    let _ = ui.click("Use this calendar for time blocks")?;
    let action = ui.into_messages().find(|m| matches!(m,Message::TimeBlocking(Action::Calendar(_)))).expect("calendar selection action");
    let _ = app::update(&mut state,action);
    assert!(tb::is_block(state.calendar.event("standup").unwrap(),&state.config.calendar));
    assert!(state.calendar.open_event.is_none());
    assert!(state.timeblocking.block.is_none());
    assert!(state.timeblocking.original_width.is_none());
    Ok(())
}
