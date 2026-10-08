//! Regression tests for the narrow layouts.
//!
//! The claim that TimeTracker is usable at 200px is easy to break by accident,
//! so these render the real interface headlessly at each breakpoint and check
//! that the controls are still there and still reachable. No compositor and
//! no golden images are involved, which is what makes them worth running on
//! every change.

use std::time::Duration;

use iced::{Element, Size};
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

/// Every width has to offer the controls the clock cannot do without.
#[test]
fn the_clock_controls_survive_every_width() {
    for width in WIDTHS {
        let state = state();
        let mut ui = render(&state, Size::new(width, HEIGHT));

        for label in ["Clock", "Calendar", "Start", "Reset", "Set Time", "Settings", "0:00", "of 5:00"] {
            assert!(
                ui.find(label).is_ok(),
                "{label:?} is missing at {width}px"
            );
        }
    }
}

/// Pressing Start has to actually start the timer at the narrowest size,
/// which is the part a broken layout silently takes away.
#[test]
fn start_is_clickable_at_the_minimum_width() -> Result<(), Error> {
    let state = state();
    let mut ui = render(&state, Size::new(180.0, 240.0));

    let _ = ui.click("Start")?;

    let messages: Vec<Message> = ui.into_messages().collect();

    assert!(
        messages.iter().any(|message| matches!(message, Message::StartPause)),
        "clicking Start produced {messages:?}"
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
fn the_settings_pane_is_usable_at_two_hundred_pixels() {
    let mut state = state();
    state.screen = Screen::Settings;

    let mut ui = render(&state, Size::new(200.0, 400.0));

    for label in [
        "Settings",
        "Done",
        // Appearance.
        "Theme",
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

/// A fired alarm replaces Start with a way to silence the alarm.
#[test]
fn a_firing_alarm_offers_a_dismiss_button() {
    let mut state = state();
    state.alarm_active = true;

    for width in WIDTHS {
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
        let _: Result<_, _> = ui.find("Start");
    }
}

/// The widest layout puts the settings beside the clock rather than over it,
/// so both are on screen at once.
#[test]
fn the_wide_layout_shows_the_clock_and_the_settings_together() {
    let mut state = state();
    state.screen = Screen::Settings;

    let mut ui = render(&state, Size::new(900.0, 600.0));

    assert!(ui.find("Start").is_ok(), "the clock is missing");
    assert!(ui.find("Theme").is_ok(), "the settings are missing");
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

/// The Cal tab has to stay usable at 200px: tabs, the day heading, the
/// menu, and a way to open Google Calendar even before anyone has signed in.
#[test]
fn the_calendar_tab_is_usable_at_two_hundred_pixels() {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar = timetracker::calendar::Store::with_snapshot(timetracker::calendar::sample_snapshot());

    let mut ui = render(&state, Size::new(200.0, 500.0));

    for label in ["Clock", "Calendar", "Menu", "Team offsite"] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing at 200px");
    }
}

#[test]
fn the_calendar_menu_lists_calendars_and_the_website_link() {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.calendar = timetracker::calendar::Store::with_snapshot(timetracker::calendar::sample_snapshot());
    state.calendar.menu_open = true;

    let mut ui = render(&state, Size::new(200.0, 500.0));

    for label in ["Calendars", "Show Google Tasks", "Personal", "Work", "Tasks", "Sync now", "Settings", "Open Google Calendar"] {
        assert!(ui.find(label).is_ok(), "{label:?} is missing from the menu");
    }
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
/// elapsed time and target are visible without switching tabs.
#[test]
fn the_calendar_view_shows_the_clock_status() {
    let mut state = state();
    state.screen = Screen::Calendar;
    state.timer.set_target(Some(Duration::from_secs(300)));

    let mut ui = render(&state, Size::new(320.0, 600.0));

    assert!(ui.find("0:00").is_ok(), "the elapsed time is missing");
    assert!(ui.find("of 5:00").is_ok(), "the target is missing");
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

#[test]
fn the_settings_pane_includes_calendar_sync_controls() {
    let mut state = state();
    state.last_tab = Screen::Calendar;
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
