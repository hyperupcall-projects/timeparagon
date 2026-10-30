//! The Habits tab: the recurring events on the TimeSlay calendar.
//!
//! A habit is a Calendar event like any other, so the list shows what Google
//! holds. The editor describes the repeat in the same words a person would
//! use — daily, every weekday, weekly on Monday and Wednesday, monthly on the
//! fourth Wednesday, and so on — rather than asking for RFC 5545 parts.

use iced::widget::{Space, button, column, container, mouse_area, row, scrollable, stack, text};
use iced::{Alignment, Color, Element, Fill};

use crate::app::{Message, State};
use crate::habits::{Action, Habit};

use super::layout::Metrics;

/// Room left for the scrollbar, which is drawn over the content.
const SCROLLBAR_GUTTER: f32 = 14.0;

fn msg(action: Action) -> Message {
    Message::Habits(action)
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut new = button(text("New habit").size(metrics.label()));
    if !state.habits.busy {
        new = new.on_press(msg(Action::New));
    }

    let title = super::controls::page_heading(
        "Habits",
        "Build a rhythm for recurring work",
        Some(state.habits.habits.len()),
        metrics,
    );
    let mut body = column![super::controls::page_toolbar(title, new.into(), metrics)]
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

/// One habit: its color, name, repeat summary and time, with its actions.
fn card<'a>(state: &'a State, habit: &'a Habit, metrics: &Metrics) -> Element<'a, Message> {
    let color = habit.color(&state.config.calendar);
    let mut edit = button(text("Edit").size(metrics.label())).padding([2, 6]);
    let mut delete = button(text("Delete").size(metrics.label()))
        .padding([2, 6])
        .style(crate::ui::controls::secondary);
    if !state.habits.busy {
        edit = edit.on_press(msg(Action::Edit(habit.id.clone())));
        delete = delete.on_press(msg(Action::DeleteRequested(habit.id.clone())));
    }

    let actions = row![edit, delete].spacing(6).align_y(Alignment::Center);

    let title = row![
        super::calendar::color_circle(color),
        text(&habit.title)
            .size(metrics.label() + 1.0)
            .font(iced::Font {
                weight: iced::font::Weight::Semibold,
                ..iced::Font::DEFAULT
            })
            .wrapping(text::Wrapping::WordOrGlyph)
            .width(Fill),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .width(Fill);
    let header: Element<'a, Message> = if metrics.size.width < 360.0 {
        column![title, actions].spacing(6).into()
    } else {
        row![title, actions]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
    };

    let mut details = column![header].spacing(2).width(Fill);
    details = details.push(
        text(habit.recurrence_label())
            .size(metrics.label() - 1.0)
            .style(text::secondary),
    );
    details = details.push(
        text(format!(
            "{} – {}",
            crate::calendar::time_format::clock(habit.start, state.config.appearance.use_24_hour),
            crate::calendar::time_format::clock(habit.end, state.config.appearance.use_24_hour)
        ))
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

    container(details)
        .padding([6, 8])
        .width(Fill)
        .style(super::controls::card_surface)
        .into()
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
    .style(crate::ui::controls::secondary);
    if !state.habits.busy {
        delete = delete.on_press(msg(Action::DeleteConfirmed(id.to_owned())));
    }
    let buttons = row![
        button(text("Cancel").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(msg(Action::DeleteCancelled))
            .style(crate::ui::controls::secondary),
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
    .style(crate::ui::controls::surface);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Screen;
    use crate::habits::{self, Draft, RepeatMode};

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
            calendar_id: "timeslay".into(),
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
        let _ = habits::open_new(
            &mut state,
            Draft::new(chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap(), None),
        );
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced::Size::new(800.0, 700.0),
            crate::ui::editor_window(&state),
        );
        let start_day = ui.find("Start day").unwrap().bounds();
        assert!(
            start_day.x >= 0.0 && start_day.x < state.config.views.detail_width(),
            "habit fields belong to the independent editor"
        );
        assert!(ui.find("Delete").is_err());
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
        draft.rule.repeat = RepeatMode::Monthly;
        draft.rule.monthly = habits::MonthlyMode::NthWeekday;
        let _ = habits::open_new(&mut state, draft);
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            iced::Size::new(800.0, 700.0),
            crate::ui::editor_window(&state),
        );
        assert!(ui.find("Weekday").is_ok());
        assert!(ui.find("Fourth").is_ok());
        assert!(ui.find("Last").is_ok());
        assert!(ui.find("Monthly on the last Wednesday").is_ok());
        assert!(ui.find("RRULE").is_err());
    }
}
