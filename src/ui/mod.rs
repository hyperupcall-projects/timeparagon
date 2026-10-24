//! The interface.
//!
//! The root is wrapped in `responsive`, so the layout is chosen from the
//! space the window actually has rather than from a guess made at startup.
//! That is what lets the same code look deliberate at 180px and at 900px.

pub mod calendar;
pub mod controls;
pub mod digits;
pub mod flash;
pub mod layout;
pub mod links;
pub mod notepad;
pub mod keep;
pub mod settings;
pub mod timeblocking;
mod timeblocking_focus;

use std::time::Duration;

use iced::widget::{button, column, container, responsive, row, scrollable, text};
use iced::{Center, Element, Fill, Length, Padding};

use crate::alarm;
use crate::app::{Message, Screen, State};
use crate::engine::{Precision, format};

use layout::{Breakpoint, Metrics};

pub fn view(state: &State) -> Element<'_, Message> {
    responsive(move |size| {
        let metrics = Metrics::of(size);

        let content: Element<'_, Message> = match (state.screen, metrics.breakpoint) {
            (Screen::Settings, _) => settings::view(state, &metrics),
            // The alarm and set-time panes need the whole window below Wide.
            (Screen::TimerSettings, Breakpoint::Wide) => row![
                container(calendar::view(state, &metrics)).width(Length::FillPortion(2)),
                container(settings::timer_view(state, &metrics)).width(Length::FillPortion(3)),
            ]
            .into(),
            (Screen::TimerSettings, _) => settings::timer_view(state, &metrics),
            (Screen::SetTime, Breakpoint::Wide) => row![
                container(calendar::view(state, &metrics)).width(Length::FillPortion(2)),
                container(set_time(state, &metrics)).width(Length::FillPortion(3)),
            ]
            .into(),
            (Screen::SetTime, _) => set_time(state, &metrics),
            // The notepad takes the whole window for the same reason: it is
            // somewhere to write, not somewhere to watch the clock from.
            (Screen::KeepNotes, _) => keep::view(state, &metrics),
            (Screen::Notepad, _) => notepad::view(state, &metrics),
            (Screen::Backlog, _) => timeblocking::backlog(state, &metrics),
            (Screen::Calendar, _) if state.timeblocking.block.is_some() => row![
                container(calendar::view(state, &metrics)).width(Fill),
                container(timeblocking::panel(state, &metrics)).width(400),
            ].into(),
            (Screen::Calendar, _) => calendar::view(state, &metrics),
        };

        // The tabs are the window's navigation, so they sit above whatever is
        // on show rather than inside it.
        let base = column![tabs(state, &metrics), content].width(Fill).height(Fill);
        if state.timeblocking.focus.is_some() {
            iced::widget::stack![base, timeblocking::focus_overlay(state, &metrics)].into()
        } else { base.into() }
    })
    .into()
}

/// The tabs along the top of the window: notepad, calendar, backlog,
/// settings, and Google Keep.
///
/// Each tab is lit when its page is on show.
pub(crate) fn tabs<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = matches!(state.screen, Screen::Settings | Screen::TimerSettings);
    let notepad = state.screen == Screen::Notepad;
    let (notepad_name, calendar_name, settings_name) = tab_names(metrics);

    // Compact tabs share one row to preserve the day grid's height in a
    // 240px window. Full labels wrap onto two rows at normal widths.
    let spacing = if metrics.breakpoint.is_compact() { 2.0 } else { metrics.spacing };
    let navigation = row![
        tab(notepad_name, notepad, Screen::Notepad, metrics),
        tab(calendar_name, state.screen == Screen::Calendar, Screen::Calendar, metrics),
        tab(if metrics.breakpoint.is_compact() { "Back" } else { "Backlog" }, state.screen == Screen::Backlog, Screen::Backlog, metrics),
    ]
    .spacing(spacing)
    .align_y(Center);
    let settings = tab(settings_name, settings, Screen::Settings, metrics);
    let keep = tab(
        if metrics.breakpoint.is_compact() { "Keep" } else { "Keep Notes" },
        state.screen == Screen::KeepNotes,
        Screen::KeepNotes,
        metrics,
    );
    let navigation: Element<'a, Message> =
        if !metrics.breakpoint.is_compact() && metrics.size.width < 420.0 {
            column![navigation, row![settings, keep].spacing(metrics.spacing)]
                .spacing(metrics.spacing)
                .into()
        } else {
            navigation.push(settings).push(keep).into()
        };
    container(navigation)
        .padding(Padding {
            left: metrics.padding,
            right: metrics.padding,
            top: 0.0,
            bottom: 0.0,
        })
        .width(Fill)
        .into()
}

/// The names on the main tabs.
///
/// The compact layout uses shorter names to keep navigation reachable.
fn tab_names(metrics: &Metrics) -> (&'static str, &'static str, &'static str) {
    if metrics.breakpoint.is_compact() {
        ("Notes", "Cal", "Setup")
    } else {
        ("Notepad", "Calendar", "Settings")
    }
}

/// One tab, lit when it is the page on show.
fn tab<'a>(
    label: &'a str,
    active: bool,
    screen: Screen,
    metrics: &Metrics,
) -> Element<'a, Message> {
    button(text(label).size(metrics.label()).center())
        .height(metrics.icon_button())
        .padding(if metrics.breakpoint.is_compact() { 2.0 } else { 5.0 })
        .on_press(Message::ScreenSelected(screen))
        .style(if active {
            button::primary
        } else {
            button::secondary
        })
        .into()
}

/// A settings-style page: a title with a way back, over a scrolling body.
///
/// The settings panes and the todo scratchpad share this so they read as the
/// same kind of place.
pub(crate) fn page<'a>(
    title: &'a str,
    content: Element<'a, Message>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    column![
        row![
            text(title).size(metrics.label() + 3.0).width(Fill),
            button(text("Done").size(metrics.label()))
                .on_press(Message::ScreenSelected(Screen::Calendar))
                .style(button::primary),
        ]
        .align_y(Center),
        scrollable(content).height(Fill).width(Fill),
    ]
    .spacing(metrics.spacing)
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill)
    .into()
}

/// The clock: the readout, the one action button, and the row of secondary
/// controls.
///
/// The clock has no section of its own any more, so this is drawn inside the
/// calendar in the place the day grid usually holds. The padding around it
/// belongs to the caller, which already has some.
pub fn clock_body<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let content = column![
        container(timer(state, metrics)).height(Fill).center_y(Fill),
        controls::primary_action(state, metrics),
        controls::secondary_row(metrics),
    ]
    .spacing(metrics.spacing)
    .width(Fill)
    .height(Fill);

    // The countdown warning tints the app window rather than the screen,
    // which is the whole reason it is a separate, gentler cue. It runs over
    // the final seconds before the target, while the clock is still under it.
    let remaining = state
        .timer
        .target()
        .map_or(Duration::ZERO, |target| target.saturating_sub(state.timer.elapsed(state.now)));
    let tint = alarm::countdown_pulse(&state.config.alarm.flash, remaining);

    match tint {
        Some(alpha) => {
            let color = state.config.alarm.flash.color.0;

            container(content)
                .style(move |_theme| container::Style {
                    background: Some(color.scale_alpha(alpha).into()),
                    ..container::Style::default()
                })
                .into()
        }
        None => content.into(),
    }
}

/// The single clock: the elapsed time, with the target tucked inside the ring.
fn timer<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let entry = state.catalog.resolve(&state.config.appearance.theme);
    let elapsed = state.timer.elapsed(state.now);
    let value = digits::format(elapsed, Precision::Seconds, metrics);

    let readout = match state.timer.target() {
        // The ring only makes sense when there is a target to fill.
        Some(target) if !target.is_zero() => {
            let caption = format!(
                "of {}",
                format::readout(target, Precision::Seconds, metrics.allow_hours)
            );

            digits::with_ring(
                value,
                caption,
                metrics,
                entry.accents.digits,
                entry.accents.ring,
                state.timer.progress(state.now),
            )
        }
        _ => digits::readout(value, metrics, entry.accents.digits),
    };

    let mut content = column![container(readout).height(Fill).center_y(Fill)]
        .spacing(metrics.spacing)
        .width(Fill)
        .height(Fill);

    if state.timer.target().is_none() {
        content = content.push(no_target_line(metrics));
    }

    content.into()
}

/// A small note under the readout when there is no target at all.
fn no_target_line<'a>(metrics: &Metrics) -> Element<'a, Message> {
    text("No target").size(metrics.label()).width(Fill).center().into()
}

/// The pane for choosing the target time.
fn set_time<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let entry = state.catalog.resolve(&state.config.appearance.theme);

    let readout = match state.timer.target() {
        Some(target) => {
            let value = digits::format(target, Precision::Seconds, metrics);
            digits::readout(value, metrics, entry.accents.digits)
        }
        None => container(text("No target").size(metrics.label() + 2.0))
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    };

    let content = column![
        container(readout).height(Fill).center_y(Fill),
        no_target_button(state, metrics),
        controls::presets(state, metrics),
        controls::duration_adjust(metrics),
        done_button(metrics),
    ]
    .spacing(metrics.spacing)
    .width(Fill)
    .height(Fill);

    column![text("Set Time").size(metrics.label() + 3.0), content]
        .spacing(metrics.spacing)
        .padding(metrics.padding)
        .into()
}

/// Returns to the calendar, accepting whatever target is set.
fn done_button<'a>(metrics: &Metrics) -> Element<'a, Message> {
    button(text("Done").size(metrics.label()).center().width(Fill))
        .width(Fill)
        .height(metrics.icon_button())
        .on_press(Message::ScreenSelected(Screen::Calendar))
        .style(button::primary)
        .into()
}

/// Turns the target off, leaving a plain count-up.
fn no_target_button<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let style = if state.timer.target().is_none() {
        button::primary
    } else {
        button::secondary
    };

    button(text("No target").size(metrics.label()).center().width(Fill))
        .width(Fill)
        .height(metrics.icon_button())
        .on_press(Message::TargetCleared)
        .style(style)
        .into()
}

/// The readout as plain text, for the window title.
pub fn readout(state: &State) -> String {
    format::readout(state.timer.elapsed(state.now), Precision::Seconds, true)
}
