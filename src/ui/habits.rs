//! The Habits tab: the recurring events on the TimeSlay calendar.
//!
//! A habit is a Calendar event like any other, so the list shows what Google
//! holds. The editor describes the repeat in the same words a person would
//! use — daily, every weekday, weekly on Monday and Wednesday, monthly on the
//! fourth Wednesday, and so on — rather than asking for RFC 5545 parts.

use iced::widget::{
    Space, button, column, container, mouse_area, row, scrollable, stack, text, text_input,
};
use iced::{Alignment, Color, Element, Fill, Length, Theme};

use crate::app::{Message, State};
use crate::calendar::colors;
use crate::habits::{self, Action, Draft, EndKind, Habit, MonthlyMode, RepeatMode};

use super::layout::Metrics;

/// Room left for the scrollbar, which is drawn over the content.
const SCROLLBAR_GUTTER: f32 = 14.0;

/// The simple schedules offered by the repeat picker.
const REPEAT_MODES: [RepeatMode; 6] = [
    RepeatMode::Never,
    RepeatMode::Daily,
    RepeatMode::Weekdays,
    RepeatMode::Weekly,
    RepeatMode::Monthly,
    RepeatMode::Yearly,
];

/// The weekday buttons, Monday first, matching [`WEEKDAYS`].
const WEEKDAY_LABELS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// The positions a monthly weekday repeat can take.
const NTH_LABELS: [(i8, &str); 6] = [
    (1, "First"),
    (2, "Second"),
    (3, "Third"),
    (4, "Fourth"),
    (5, "Fifth"),
    (-1, "Last"),
];

fn msg(action: Action) -> Message {
    Message::Habits(action)
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut new = button(text("New habit").size(metrics.label()));
    if !state.habits.busy {
        new = new.on_press(msg(Action::New));
    }

    let mut body = column![
        row![
            text("Habits").size(metrics.label() + 3.0).width(Fill),
            new,
        ]
        .spacing(metrics.spacing)
        .align_y(Alignment::Center),
        text("Recurring events on the TimeSlay calendar. Open one to change its name, time or repeat schedule.")
            .size(metrics.label() - 1.0)
            .style(text::secondary),
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    if let Some(error) = &state.habits.error {
        body = body.push(text(error).size(metrics.label()).style(text::danger));
    }

    if state.habits.habits.is_empty() {
        body = body.push(
            text(if state.habits.loaded {
                "No habits yet. Add one to put a recurring event on the calendar."
            } else if state.habits.busy {
                "Loading habits…"
            } else {
                "Open the tab again to load habits from Google Calendar."
            })
            .size(metrics.label())
            .style(text::secondary),
        );
    }

    for habit in &state.habits.habits {
        body = body.push(card(state, habit, metrics));
    }

    container(
        scrollable(container(body).padding(iced::Padding {
            right: SCROLLBAR_GUTTER,
            ..iced::Padding::ZERO
        }))
        .width(Fill)
        .height(Fill),
    )
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill)
    .into()
}

fn card_style(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.base.color.into()),
        border: iced::Border {
            color: palette.background.strong.color,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    }
}

/// One habit: its color, name, repeat summary and time, with its actions.
fn card<'a>(state: &'a State, habit: &'a Habit, metrics: &Metrics) -> Element<'a, Message> {
    let color = habit.color(&state.config.calendar);
    let mut edit = button(text("Edit").size(metrics.label())).padding([2, 6]);
    let mut delete = button(text("Delete").size(metrics.label()))
        .padding([2, 6])
        .style(button::secondary);
    if !state.habits.busy {
        edit = edit.on_press(msg(Action::Edit(habit.id.clone())));
        delete = delete.on_press(msg(Action::DeleteRequested(habit.id.clone())));
    }

    let actions = row![edit, delete].spacing(6).align_y(Alignment::Center);

    let header = row![
        super::calendar::color_circle(color),
        text(&habit.title).size(metrics.label() + 1.0).width(Fill),
        actions,
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let mut details = column![header].spacing(2).width(Fill);
    details = details.push(
        text(habit.recurrence_label())
            .size(metrics.label() - 1.0)
            .style(text::secondary),
    );
    if !habit.notes.is_empty() {
        details = details.push(
            text(&habit.notes)
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }
    details = details.push(
        text(habit.time_label())
            .size(metrics.label() - 1.0)
            .style(text::secondary),
    );

    container(details)
        .padding([6, 8])
        .width(Fill)
        .style(card_style)
        .into()
}

/// The add/edit popup, drawn over everything including the tabs.
pub fn editor_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(draft) = &state.habits.draft else {
        return Space::new().into();
    };
    let card = container(scrollable(form(draft, state.habits.busy, metrics)))
        .padding(16)
        .width(Fill)
        .max_width(480)
        .max_height((metrics.size.height - 32.0).max(80.0))
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                text_color: Some(palette.background.base.text),
                border: iced::Border {
                    radius: 10.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });
    overlay(card.into(), msg(Action::CancelEdit))
}

/// The "are you sure" prompt for deleting a habit and its whole series.
pub fn delete_confirmation<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(id) = state.habits.confirm_delete.as_deref() else {
        return Space::new().into();
    };
    let Some(habit) = state.habits.habits.iter().find(|habit| habit.id == id) else {
        return Space::new().into();
    };

    let mut delete = button(
        text("Delete series")
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill)
    .style(button::secondary);
    if !state.habits.busy {
        delete = delete.on_press(msg(Action::DeleteConfirmed(id.to_owned())));
    }
    let buttons = row![
        button(text("Cancel").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(msg(Action::DeleteCancelled))
            .style(button::secondary),
        delete,
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    let card = container(
        column![
            text("Delete this habit?").size(metrics.label() + 1.0),
            text(&habit.title)
                .size(metrics.label())
                .style(text::secondary),
            text("Every occurrence in the series is removed from Google Calendar.")
                .size(metrics.label() - 1.0)
                .style(text::secondary),
            buttons,
        ]
        .spacing(metrics.spacing)
        .width(Fill),
    )
    .padding(16)
    .width(Fill)
    .max_width(360)
    .style(|theme: &iced::Theme| {
        let palette = theme.extended_palette();
        container::Style {
            background: Some(palette.background.base.color.into()),
            text_color: Some(palette.background.base.text),
            border: iced::Border {
                radius: 10.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    });

    overlay(card.into(), msg(Action::DeleteCancelled))
}

/// A dimmed backdrop that dismisses on a click, with `card` centred over it.
fn overlay<'a>(card: Element<'a, Message>, dismiss: Message) -> Element<'a, Message> {
    let backdrop =
        mouse_area(
            container(Space::new())
                .width(Fill)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(Color::BLACK.scale_alpha(0.58).into()),
                    ..Default::default()
                }),
        )
        .on_press(dismiss)
        .on_scroll(|_| Message::Ignored);
    let foreground = container(
        mouse_area(card)
            .on_press(Message::Ignored)
            .on_scroll(|_| Message::Ignored),
    )
    .padding(16)
    .center_x(Fill)
    .center_y(Fill);
    stack![backdrop, foreground].into()
}

/// A choice that looks selected when it is the current one. The buttons are
/// the only way to pick a schedule, so there is no raw rule text anywhere.
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

fn form<'a>(draft: &'a Draft, busy: bool, metrics: &Metrics) -> Element<'a, Message> {
    let label = |name: &'static str| text(name).size(metrics.label()).style(text::secondary);

    let mut title = text_input("Habit name", &draft.title)
        .id(iced::widget::Id::new(habits::TITLE_FIELD))
        .size(metrics.label())
        .width(Fill);
    title = title
        .on_input(|value| msg(Action::Title(value)))
        .on_submit(msg(Action::Save));

    let mut body = column![
        text(if draft.id.is_some() {
            "Edit habit"
        } else {
            "New habit"
        })
        .size(18),
        label("Name"),
        title,
        label("Notes (optional)"),
        text_input("Notes", &draft.notes)
            .on_input(|value| msg(Action::Notes(value)))
            .size(metrics.label())
            .width(Fill),
    ]
    .spacing(6);

    // Date and times: one column when narrow, one row when there is room.
    let date = text_input("YYYY-MM-DD", &draft.date)
        .on_input(|value| msg(Action::Date(value)))
        .size(metrics.label());
    let start = text_input("HH:MM", &draft.start)
        .on_input(|value| msg(Action::Start(value)))
        .size(metrics.label());
    let end = text_input("HH:MM", &draft.end)
        .on_input(|value| msg(Action::End(value)))
        .size(metrics.label());
    let when: Element<'a, Message> = if metrics.breakpoint.is_compact() {
        column![
            date,
            row![label("From"), start].spacing(6),
            row![label("To"), end].spacing(6)
        ]
        .spacing(4)
        .into()
    } else {
        row![date.width(Length::FillPortion(2)), start, end]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
    };
    body = body.push(label("Day and time")).push(when);

    // Repeat: a row of everyday choices, then only the controls that the
    // chosen schedule needs.
    let mut modes = Vec::new();
    for mode in REPEAT_MODES {
        modes.push(choice(
            mode.label(),
            draft.repeat == mode,
            msg(Action::Repeat(mode)),
            metrics,
        ));
    }
    body = body
        .push(label("Repeat"))
        .push(row(modes).spacing(4).wrap().vertical_spacing(4))
        .push(
            text(draft.repeat_summary())
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );

    match draft.repeat {
        RepeatMode::Custom => {
            body = body
                .push(
                    text("This habit has a custom schedule that the simple editor cannot show. You can keep it as it is, or replace it with a simple schedule.")
                        .size(metrics.label() - 1.0)
                        .style(text::secondary),
                )
                .push(
                    button(text("Use a simple schedule").size(metrics.label()).center())
                        .padding([3, 7])
                        .style(button::secondary)
                        .on_press(msg(Action::SimpleRepeat)),
                );
        }
        RepeatMode::Never => {}
        mode => {
            if mode != RepeatMode::Weekdays {
                body = body.push(
                    row![
                        text("Every").size(metrics.label()),
                        text_input("1", &draft.interval)
                            .on_input(|value| msg(Action::Interval(value)))
                            .size(metrics.label())
                            .width(52),
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
                        draft.weekdays.get(index).copied().unwrap_or(false),
                        msg(Action::ToggleWeekday(index)),
                        metrics,
                    ));
                }
                body = body
                    .push(label("On"))
                    .push(row(days).spacing(4).wrap().vertical_spacing(4));
            }

            if mode == RepeatMode::Monthly {
                let monthly: Element<'a, Message> = match draft.monthly {
                    MonthlyMode::Day => row![
                        text("On day").size(metrics.label()),
                        text_input("28", &draft.monthly_day)
                            .on_input(|value| msg(Action::MonthlyDay(value)))
                            .size(metrics.label())
                            .width(52),
                        text("of the month").size(metrics.label()),
                    ]
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .into(),
                    MonthlyMode::NthWeekday => {
                        let mut positions = Vec::new();
                        for (nth, name) in NTH_LABELS {
                            positions.push(choice(
                                name,
                                draft.monthly_nth == nth,
                                msg(Action::MonthlyNth(nth)),
                                metrics,
                            ));
                        }
                        let mut weekdays = Vec::new();
                        for (index, day) in WEEKDAY_LABELS.iter().enumerate() {
                            weekdays.push(choice(
                                day,
                                habits::WEEKDAYS[index] == draft.monthly_weekday,
                                msg(Action::MonthlyWeekday(habits::WEEKDAYS[index])),
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
                                draft.monthly == MonthlyMode::Day,
                                msg(Action::MonthlyMode(MonthlyMode::Day)),
                                metrics,
                            ),
                            choice(
                                "Weekday",
                                draft.monthly == MonthlyMode::NthWeekday,
                                msg(Action::MonthlyMode(MonthlyMode::NthWeekday)),
                                metrics,
                            ),
                            choice(
                                "Last day",
                                draft.monthly == MonthlyMode::LastDay,
                                msg(Action::MonthlyMode(MonthlyMode::LastDay)),
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
                    text("Once a year on the day the habit starts.")
                        .size(metrics.label() - 1.0)
                        .style(text::secondary),
                );
            }

            // When the series stops.
            body = body.push(
                row![
                    label("Ends"),
                    choice(
                        "Never",
                        draft.ends == EndKind::Never,
                        msg(Action::Ends(EndKind::Never)),
                        metrics,
                    ),
                    choice(
                        "On date",
                        draft.ends == EndKind::OnDate,
                        msg(Action::Ends(EndKind::OnDate)),
                        metrics,
                    ),
                    choice(
                        "After",
                        draft.ends == EndKind::After,
                        msg(Action::Ends(EndKind::After)),
                        metrics,
                    ),
                ]
                .spacing(4)
                .align_y(Alignment::Center)
                .wrap()
                .vertical_spacing(4),
            );

            match draft.ends {
                EndKind::OnDate => {
                    body = body.push(
                        row![
                            text("Last day").size(metrics.label()),
                            text_input("YYYY-MM-DD", &draft.until)
                                .on_input(|value| msg(Action::Until(value)))
                                .size(metrics.label())
                                .width(120),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    );
                }
                EndKind::After => {
                    body = body.push(
                        row![
                            text("For").size(metrics.label()),
                            text_input("10", &draft.count)
                                .on_input(|value| msg(Action::Count(value)))
                                .size(metrics.label())
                                .width(52),
                            text("repeats").size(metrics.label()),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    );
                }
                EndKind::Never => {}
            }

            // Dates taken out of the series, shown as chips rather than text.
            body = body.push(label("Skip dates"));
            if draft.skips.is_empty() {
                body = body.push(
                    text("No skipped dates.")
                        .size(metrics.label() - 1.0)
                        .style(text::secondary),
                );
            } else {
                let mut chips = Vec::new();
                for day in &draft.skips {
                    chips.push(
                        container(
                            row![
                                text(day.format("%B %-d").to_string()).size(metrics.label() - 1.0),
                                button(text("×").size(metrics.label()).center())
                                    .padding([0, 5])
                                    .style(button::text)
                                    .on_press(msg(Action::SkipRemove(*day))),
                            ]
                            .spacing(2)
                            .align_y(Alignment::Center),
                        )
                        .padding([2, 5])
                        .style(card_style)
                        .into(),
                    );
                }
                body = body.push(row(chips).spacing(4).wrap().vertical_spacing(4));
            }
            body = body.push(
                row![
                    text_input("YYYY-MM-DD", &draft.skip_input)
                        .on_input(|value| msg(Action::SkipInput(value)))
                        .on_submit(msg(Action::SkipAdd))
                        .size(metrics.label())
                        .width(120),
                    button(text("Skip this date").size(metrics.label()).center())
                        .padding([3, 7])
                        .style(button::secondary)
                        .on_press(msg(Action::SkipAdd)),
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            );
        }
    }

    body = body.push(label("Color")).push(color_picker(draft, metrics));

    if let Some(error) = &draft.error {
        body = body.push(text(error).size(metrics.label()).style(text::danger));
    }

    let mut save = button(
        text(if busy { "Saving…" } else { "Save" })
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill);
    if !busy {
        save = save.on_press(msg(Action::Save));
    }
    let cancel = button(text("Cancel").size(metrics.label()).center().width(Fill))
        .width(Fill)
        .on_press(msg(Action::CancelEdit))
        .style(button::secondary);

    body.push(row![save, cancel].spacing(8).width(Fill)).into()
}

/// The habit's own Google color. "Default" clears it, so the habit follows
/// the color picked in Settings.
fn color_picker<'a>(draft: &Draft, _metrics: &Metrics) -> Element<'a, Message> {
    let selected = draft.color_id.as_deref().and_then(colors::event);
    let mut swatches = row![].spacing(4);
    for color in colors::palette() {
        swatches = swatches.push(super::calendar::swatch_button(
            color,
            selected == Some(color),
            msg(Action::Color(Some(color))),
        ));
    }
    swatches = swatches.push(
        button(text("Default").size(11.0))
            .padding([1, 4])
            .style(button::text)
            .on_press(msg(Action::Color(None))),
    );
    swatches.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Screen;

    fn state() -> State {
        let mut state = State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        );
        state.screen = Screen::Habits;
        state.habits.loaded = true;
        state
    }

    fn habit(id: &str, rule: &str) -> Habit {
        Habit {
            id: id.into(),
            calendar_id: "time-slay".into(),
            title: "Stretch".into(),
            notes: String::new(),
            start: crate::calendar::at_clock(
                chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(),
                9,
                0,
            ),
            end: crate::calendar::at_clock(
                chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(),
                9,
                30,
            ),
            recurrence: habits::Recurrence::from_lines(&[rule.to_owned()]),
            color_id: None,
        }
    }

    #[test]
    fn an_empty_habits_tab_offers_the_new_button() {
        let state = state();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced::Size::new(800.0, 600.0),
            crate::ui::view(&state),
        );
        assert!(ui.find("Habits").is_ok());
        assert!(ui.find("New habit").is_ok());
        assert!(
            ui.find("No habits yet. Add one to put a recurring event on the calendar.")
                .is_ok()
        );
    }

    #[test]
    fn a_habit_shows_its_repeat_in_words() {
        let mut state = state();
        state.habits.habits = vec![habit("one", "RRULE:FREQ=WEEKLY;BYDAY=MO")];
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced::Size::new(800.0, 600.0),
            crate::ui::view(&state),
        );
        assert!(ui.find("Stretch").is_ok());
        assert!(ui.find("Weekly on Monday").is_ok());
        assert!(ui.find("RRULE").is_err());
        assert!(ui.find("Edit").is_ok());
    }

    #[test]
    fn the_editor_offers_a_simple_schedule_without_rfc_text() {
        let mut state = state();
        state.habits.draft = Some(Draft::new(
            chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(),
            None,
        ));
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced::Size::new(800.0, 700.0),
            crate::ui::view(&state),
        );
        assert!(ui.find("Repeat").is_ok());
        assert!(ui.find("Weekly").is_ok());
        assert!(ui.find("Does not repeat").is_ok());
        assert!(ui.find("Mon").is_ok());
        assert!(ui.find("Skip dates").is_ok());
        assert!(ui.find("RRULE").is_err());
        assert!(ui.find("RDATE").is_err());
        assert!(ui.find("EXDATE").is_err());
    }

    #[test]
    fn a_monthly_habit_can_use_a_weekday_position() {
        let mut state = state();
        let mut draft = Draft::new(chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(), None);
        draft.repeat = RepeatMode::Monthly;
        draft.monthly = MonthlyMode::NthWeekday;
        state.habits.draft = Some(draft);
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced::Size::new(800.0, 700.0),
            crate::ui::view(&state),
        );
        assert!(ui.find("Weekday").is_ok());
        assert!(ui.find("Fourth").is_ok());
        assert!(ui.find("Last").is_ok());
        assert!(ui.find("Monthly on the last Wednesday").is_ok());
        assert!(ui.find("RRULE").is_err());
    }
}
