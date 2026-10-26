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
mod task_drag;

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
            (Screen::Projects, _) => projects(state, &metrics),
            (Screen::Calendar, _) => {
                // Keep the calendar at the same place in the widget tree when
                // the block editor opens or closes, preserving its scroll state.
                let mut content = row![container(calendar::view(state, &metrics)).width(Fill)];
                if state.timeblocking.block.is_some() {
                    content = content.push(
                        container(timeblocking::panel(state, &metrics))
                            .width(crate::timeblocking::PANEL_WIDTH),
                    );
                }
                content.into()
            }
        };

        // The tabs are the window's navigation, so they sit above whatever is
        // on show rather than inside it.
        let base = column![tabs(state, &metrics), content].width(Fill).height(Fill);
        // Adding/removing a wrapper resets the state of all its descendants.
        // Keep one stack with the calendar first, and append active dialogs.
        let mut layers = vec![base.into()];
        if state.timeblocking.focus.is_some() {
            layers.push(timeblocking::focus_overlay(state, &metrics));
        }
        if state.timeblocking.estimate_task.is_some() {
            layers.push(timeblocking::estimate_overlay(state, &metrics));
        }
        iced::widget::stack(layers).into()
    })
    .into()
}

/// A plain list of the immediate project directories.
fn projects<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut items = column![].spacing(metrics.spacing).width(Fill);
    match &state.projects {
        None => items = items.push(text("Loading projects…").size(metrics.label())),
        Some(Err(error)) => {
            items = items.push(text(format!("Could not list projects: {error}")).size(metrics.label()));
        }
        Some(Ok(names)) => {
            for name in names {
                items = items.push(text(name).size(metrics.label()).width(Fill));
            }
        }
    }
    container(scrollable(items).width(Fill).height(Fill))
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill)
        .into()
}

/// Navigation grouped by purpose, with Settings replacing the main view.
/// Narrow windows stack the tabs within each group to keep full names visible.
pub(crate) fn tabs<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings_active = matches!(state.screen, Screen::Settings | Screen::TimerSettings);
    let spacing = if metrics.size.width < 720.0 { 2.0 } else { metrics.spacing };
    let calendar = tab("Calendar", state.screen == Screen::Calendar, Screen::Calendar, metrics);
    let backlog = tab("Backlog", state.screen == Screen::Backlog, Screen::Backlog, metrics);
    let projects = tab("Projects", state.screen == Screen::Projects, Screen::Projects, metrics);
    let notepad = tab("Notepad", state.screen == Screen::Notepad, Screen::Notepad, metrics);
    let keep = tab("Keep", state.screen == Screen::KeepNotes, Screen::KeepNotes, metrics);
    let settings = tab("Settings", settings_active, Screen::Settings, metrics);

    let (left, middle, right): (Element<'a, Message>, Element<'a, Message>, Element<'a, Message>) =
        if metrics.size.width < 720.0 {
            (
                column![calendar, backlog].spacing(spacing).into(),
                column![notepad, keep].spacing(spacing).align_x(Center).into(),
                column![settings, projects].spacing(spacing).align_x(iced::Right).into(),
            )
        } else {
            (
                row![calendar, backlog, projects].spacing(spacing).into(),
                row![notepad, keep].spacing(spacing).into(),
                settings,
            )
        };

    container(
        row![
            container(left).width(Fill).align_left(Fill),
            container(middle).width(Fill).center_x(Fill),
            container(right).width(Fill).align_right(Fill),
        ]
        .spacing(spacing),
    )
    .padding(Padding {
        left: metrics.padding,
        right: metrics.padding,
        top: 0.0,
        bottom: 0.0,
    })
    .width(Fill)
    .into()
}

/// One tab, lit when it is the page on show.
fn tab<'a>(
    label: &'a str,
    active: bool,
    screen: Screen,
    metrics: &Metrics,
) -> Element<'a, Message> {
    button(text(label).size(metrics.label()).center())
        .height(if metrics.breakpoint.is_compact() { 16.0 } else { metrics.icon_button() })
        .padding(if metrics.breakpoint.is_compact() { Padding::from([0.0, 2.0]) } else { Padding::from(5.0) })
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
