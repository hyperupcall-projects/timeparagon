//! Buttons and preset chips.
//!
//! Every control here uses short text labels rather than icon glyphs. At
//! 200px there is still room for the word "Reset", and a real word beats a
//! symbol that may not exist in the system font.

use iced::widget::{button, column, row, text};
use iced::{Element, Fill};

use crate::app::{Message, Screen, State};
use crate::engine::format;

use super::layout::Metrics;

/// The one button that is always in the same place: start, pause, or dismiss.
pub fn primary_action<'a>(state: &State, metrics: &Metrics) -> Element<'a, Message> {
    let (label, message, style) = if state.alarm_active {
        ("Dismiss", Message::Dismiss, button::danger as fn(&_, _) -> _)
    } else if state.timer.is_running() {
        ("Pause", Message::StartPause, button::secondary as fn(&_, _) -> _)
    } else {
        ("Start", Message::StartPause, button::primary as fn(&_, _) -> _)
    };

    button(text(label).size(metrics.label() + 2.0).center().width(Fill))
        .width(Fill)
        .height(metrics.button)
        .on_press(message)
        .style(style)
        .into()
}

/// Set time, reset and the way into the alarm settings.
///
/// The alarm settings live here rather than under the calendar's menu because
/// they change the timer, not the calendar.
pub fn secondary_row<'a>(metrics: &Metrics) -> Element<'a, Message> {
    let small = |label: &'a str, message: Message| {
        button(text(label).size(metrics.label()).center().width(Fill))
            .width(Fill)
            .height(metrics.icon_button())
            .on_press(message)
            .style(button::secondary)
    };

    row![
        small("Set Time", Message::ScreenSelected(Screen::SetTime)),
        small("Reset", Message::Reset),
        small("Alarm", Message::ScreenSelected(Screen::TimerSettings)),
    ]
    .spacing(metrics.spacing)
    .into()
}

/// Preset chips, wrapped to as many columns as the width allows.
pub fn presets<'a>(state: &State, metrics: &Metrics) -> Element<'a, Message> {
    let armed = state.timer.target();

    let chips: Vec<Element<'a, Message>> = state
        .config
        .timer
        .presets()
        .map(|duration| {
            button(
                text(format::compact(duration))
                    .size(metrics.label())
                    .center()
                    .width(Fill),
            )
            .width(Fill)
            .height(metrics.icon_button())
            .on_press(Message::PresetSelected(duration))
            .style(if armed == Some(duration) {
                button::primary
            } else {
                button::secondary
            })
            .into()
        })
        .collect();

    let rows = chips
        .into_iter()
        .fold(Vec::<Vec<Element<'a, Message>>>::new(), |mut rows, chip| {
            match rows.last_mut() {
                Some(last) if last.len() < metrics.preset_columns => last.push(chip),
                _ => rows.push(vec![chip]),
            }

            rows
        });

    column(
        rows.into_iter()
            .map(|chips| row(chips).spacing(metrics.spacing).into()),
    )
    .spacing(metrics.spacing)
    .into()
}

/// Buttons for nudging the target time.
///
/// A narrow window only gets the minute steps; there is no room for four
/// buttons and they are the ones people reach for anyway.
pub fn duration_adjust<'a>(metrics: &Metrics) -> Element<'a, Message> {
    let step = |label: &'a str, delta: i64| {
        button(text(label).size(metrics.label()).center().width(Fill))
            .width(Fill)
            .height(metrics.icon_button())
            .on_press(Message::DurationAdjusted(delta))
            .style(button::secondary)
    };

    if metrics.breakpoint.is_compact() {
        row![step("-1m", -60), step("+1m", 60)]
            .spacing(metrics.spacing)
            .into()
    } else {
        row![
            step("-1m", -60),
            step("-10s", -10),
            step("+10s", 10),
            step("+1m", 60),
        ]
        .spacing(metrics.spacing)
        .into()
    }
}
