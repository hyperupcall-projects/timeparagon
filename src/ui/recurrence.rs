//! The repeat controls shared by events, time blocks and habits.
//!
//! Every editor renders the same words and the same choices; only the message
//! that applies a change differs, so a schedule learned in one place behaves
//! exactly the same in the others.

use chrono::NaiveDate;
use iced::widget::{button, column, row, text, text_input};
use iced::{Alignment, Element, Length, Theme};

use crate::app::Message;
use crate::recurrence::{self, EndKind, MonthlyMode, RepeatMode};

use super::layout::Metrics;

const WEEKDAY_LABELS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

const NTH_LABELS: [(i8, &str); 6] = [
    (1, "First"),
    (2, "Second"),
    (3, "Third"),
    (4, "Fourth"),
    (5, "Fifth"),
    (-1, "Last"),
];

/// A choice that looks selected when it is the current one.
fn choice<'a>(
    label: &str,
    selected: bool,
    on_press: Message,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let style: fn(&Theme, button::Status) -> button::Style = if selected {
        button::primary
    } else {
        button::secondary
    };
    button(text(label.to_owned()).size(metrics.label()).center())
        .padding([3, 7])
        .style(style)
        .on_press(on_press)
        .into()
}

/// The full repeat editor: simple modes, the controls each mode needs, when
/// the series ends and the dates skipped out of it.
pub fn editor<'a, F>(
    rule: &'a recurrence::Rule,
    anchor: NaiveDate,
    metrics: &Metrics,
    busy: bool,
    map: F,
) -> Element<'a, Message>
where
    F: Fn(recurrence::Action) -> Message + Clone + 'a,
{
    let label = |name: &'static str| text(name).size(metrics.label()).style(text::secondary);
    let send_map = map.clone();
    let send = move |action| send_map(action);

    let mut modes = Vec::new();
    for mode in RepeatMode::SIMPLE {
        modes.push(choice(
            mode.label(),
            rule.repeat == mode,
            send(recurrence::Action::Repeat(mode)),
            metrics,
        ));
    }

    let mut body = column![
        label("Repeat"),
        row(modes).spacing(4).wrap().vertical_spacing(4),
        text(rule.summary(anchor))
            .size(metrics.label() - 1.0)
            .style(text::secondary),
    ]
    .spacing(4)
    .width(Length::Fill);

    match rule.repeat {
        RepeatMode::Custom => {
            body = body
                .push(
                    text("This item has a custom schedule that the simple editor cannot show. You can keep it as it is, or replace it with a simple schedule.")
                        .size(metrics.label() - 1.0)
                        .style(text::secondary),
                )
                .push(
                    button(text("Use a simple schedule").size(metrics.label()).center())
                        .padding([3, 7])
                        .style(button::secondary)
                        .on_press(send(recurrence::Action::SimpleRepeat)),
                );
        }
        RepeatMode::Never => {}
        mode => {
            if mode != RepeatMode::Weekdays {
                let mut interval = text_input("1", &rule.interval)
                    .size(metrics.label())
                    .width(52);
                if !busy {
                    let send = map.clone();
                    interval =
                        interval.on_input(move |value| send(recurrence::Action::Interval(value)));
                }
                body = body.push(
                    row![
                        text("Every").size(metrics.label()),
                        interval,
                        text(mode.unit()).size(metrics.label()),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center),
                );
            }

            if mode == RepeatMode::Weekly {
                let mut days = Vec::new();
                for (index, day) in WEEKDAY_LABELS.iter().enumerate() {
                    days.push(choice(
                        day,
                        rule.weekdays.get(index).copied().unwrap_or(false),
                        send(recurrence::Action::ToggleWeekday(index)),
                        metrics,
                    ));
                }
                body = body
                    .push(label("On"))
                    .push(row(days).spacing(4).wrap().vertical_spacing(4));
            }

            if mode == RepeatMode::Monthly {
                let monthly: Element<'a, Message> = match rule.monthly {
                    MonthlyMode::Day => {
                        let mut day = text_input("28", &rule.monthly_day)
                            .size(metrics.label())
                            .width(52);
                        if !busy {
                            let send = map.clone();
                            day = day
                                .on_input(move |value| send(recurrence::Action::MonthlyDay(value)));
                        }
                        row![
                            text("On day").size(metrics.label()),
                            day,
                            text("of the month").size(metrics.label()),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center)
                        .into()
                    }
                    MonthlyMode::NthWeekday => {
                        let mut positions = Vec::new();
                        for (nth, name) in NTH_LABELS {
                            positions.push(choice(
                                name,
                                rule.monthly_nth == nth,
                                send(recurrence::Action::MonthlyNth(nth)),
                                metrics,
                            ));
                        }
                        let mut weekdays = Vec::new();
                        for (index, day) in WEEKDAY_LABELS.iter().enumerate() {
                            weekdays.push(choice(
                                day,
                                recurrence::WEEKDAYS[index] == rule.monthly_weekday,
                                send(recurrence::Action::MonthlyWeekday(
                                    recurrence::WEEKDAYS[index],
                                )),
                                metrics,
                            ));
                        }
                        column![
                            row(positions).spacing(4).wrap().vertical_spacing(4),
                            row(weekdays).spacing(4).wrap().vertical_spacing(4)
                        ]
                        .spacing(4)
                        .into()
                    }
                    MonthlyMode::LastDay => text("On the last day of each month.")
                        .size(metrics.label() - 1.0)
                        .style(text::secondary)
                        .into(),
                };
                body = body
                    .push(
                        row![
                            choice(
                                "Day of the month",
                                rule.monthly == MonthlyMode::Day,
                                send(recurrence::Action::MonthlyMode(MonthlyMode::Day)),
                                metrics,
                            ),
                            choice(
                                "Weekday",
                                rule.monthly == MonthlyMode::NthWeekday,
                                send(recurrence::Action::MonthlyMode(MonthlyMode::NthWeekday)),
                                metrics,
                            ),
                            choice(
                                "Last day",
                                rule.monthly == MonthlyMode::LastDay,
                                send(recurrence::Action::MonthlyMode(MonthlyMode::LastDay)),
                                metrics,
                            ),
                        ]
                        .spacing(4)
                        .wrap()
                        .vertical_spacing(4),
                    )
                    .push(monthly);
            }

            if mode == RepeatMode::Yearly {
                body = body.push(
                    text("Once a year on the day it starts.")
                        .size(metrics.label() - 1.0)
                        .style(text::secondary),
                );
            }

            body = body.push(
                row![
                    label("Ends"),
                    choice(
                        "Never",
                        rule.ends == EndKind::Never,
                        send(recurrence::Action::Ends(EndKind::Never)),
                        metrics,
                    ),
                    choice(
                        "On date",
                        rule.ends == EndKind::OnDate,
                        send(recurrence::Action::Ends(EndKind::OnDate)),
                        metrics,
                    ),
                    choice(
                        "After",
                        rule.ends == EndKind::After,
                        send(recurrence::Action::Ends(EndKind::After)),
                        metrics,
                    ),
                ]
                .spacing(4)
                .align_y(Alignment::Center)
                .wrap()
                .vertical_spacing(4),
            );

            match rule.ends {
                EndKind::OnDate => {
                    let mut until = text_input("YYYY-MM-DD", &rule.until)
                        .size(metrics.label())
                        .width(120);
                    if !busy {
                        let send = map.clone();
                        until = until.on_input(move |value| send(recurrence::Action::Until(value)));
                    }
                    body = body.push(
                        row![text("Last day").size(metrics.label()), until,]
                            .spacing(6)
                            .align_y(Alignment::Center),
                    );
                }
                EndKind::After => {
                    let mut count = text_input("10", &rule.count)
                        .size(metrics.label())
                        .width(52);
                    if !busy {
                        let send = map.clone();
                        count = count.on_input(move |value| send(recurrence::Action::Count(value)));
                    }
                    body = body.push(
                        row![
                            text("For").size(metrics.label()),
                            count,
                            text("repeats").size(metrics.label()),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    );
                }
                EndKind::Never => {}
            }

            body = body.push(label("Skip dates"));
            if rule.skips.is_empty() {
                body = body.push(
                    text("No skipped dates.")
                        .size(metrics.label() - 1.0)
                        .style(text::secondary),
                );
            } else {
                let mut chips = Vec::new();
                for day in &rule.skips {
                    chips.push(
                        row![
                            text(day.format("%B %-d").to_string()).size(metrics.label() - 1.0),
                            button(text("×").size(metrics.label()).center())
                                .padding([0, 5])
                                .style(button::text)
                                .on_press(send(recurrence::Action::SkipRemove(*day))),
                        ]
                        .spacing(2)
                        .align_y(Alignment::Center)
                        .into(),
                    );
                }
                body = body.push(row(chips).spacing(4).wrap().vertical_spacing(4));
            }
            let mut skip_input = text_input("YYYY-MM-DD", &rule.skip_input)
                .on_submit(send(recurrence::Action::SkipAdd))
                .size(metrics.label())
                .width(120);
            if !busy {
                let send = map.clone();
                skip_input =
                    skip_input.on_input(move |value| send(recurrence::Action::SkipInput(value)));
            }
            body = body.push(
                row![
                    skip_input,
                    button(text("Skip this date").size(metrics.label()).center())
                        .padding([3, 7])
                        .style(button::secondary)
                        .on_press(send(recurrence::Action::SkipAdd)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
    }

    body.into()
}
