//! The Brain dump tab: one page per day, with search across every page.
//!
//! A page is written in a full-window editor, the same way the notepad is,
//! and is saved into the config as it is typed.

use chrono::Local;
use iced::widget::{
    Space, button, column, container, mouse_area, row, scrollable, stack, text, text_editor,
    text_input,
};
use iced::{Element, Fill, Length, Theme};

use super::layout::Metrics;
use crate::app::{Message, State};
use crate::brain_dump::Action;

fn msg(action: Action) -> Message {
    Message::BrainDump(action)
}

/// The id the editor focuses when a page is opened.
pub fn editor_id() -> iced::widget::Id {
    iced::widget::Id::new("brain-dump-editor")
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let today = Local::now().date_naive();
    let add = button(text("Add today").size(metrics.label()).center())
        .on_press(msg(Action::Open(today)))
        .style(crate::ui::controls::primary);
    let entries = state.config.brain_dump.search(&state.brain_dump.query);
    let title = super::controls::page_heading(
        "Brain dump",
        "One page for every day",
        Some(entries.len()),
        metrics,
    );
    let search = text_input("Search every page…", &state.brain_dump.query)
        .on_input(|query| msg(Action::Query(query)))
        .size(metrics.label())
        .width(Fill);

    let mut body = column![
        super::controls::page_toolbar(title, add.into(), metrics),
        search,
    ]
    .spacing(metrics.spacing);

    if entries.is_empty() {
        body = body.push(
            text(if state.brain_dump.query.trim().is_empty() {
                "No brain dumps yet. Add today's to start one."
            } else {
                "No page contains that text."
            })
            .size(metrics.label())
            .style(text::secondary),
        );
    }
    for entry in entries {
        body = body.push(entry_card(entry.date, &entry.text, metrics));
    }

    container(scrollable(body.width(Fill)))
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill)
        .into()
}

/// One day's page in the list. The date is the title, and clicking it opens
/// the editor; a short preview makes long pages distinguishable at a glance.
fn entry_card<'a>(
    date: chrono::NaiveDate,
    body: &'a str,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let title = button(
        text(date.format("%A, %B %-d, %Y").to_string())
            .size(metrics.label() + 1.0)
            .width(Fill),
    )
    .width(Fill)
    .padding(0)
    .style(button::text)
    .on_press(msg(Action::Open(date)));
    let preview = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let preview = if preview.is_empty() {
        "Empty page".to_owned()
    } else {
        preview.chars().take(140).collect()
    };
    let edit = button(text("Edit").size(metrics.label()))
        .on_press(msg(Action::Open(date)))
        .style(crate::ui::controls::secondary);
    let content = column![
        row![title, edit]
            .spacing(metrics.spacing)
            .align_y(iced::Center),
        text(preview)
            .size(metrics.label() - 1.0)
            .style(text::secondary),
    ]
    .spacing(3);
    container(content)
        .padding([7, 9])
        .width(Fill)
        .style(super::controls::card_surface)
        .into()
}

/// The full-window page editor, over whatever screen opened it.
pub fn editor_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(date) = state.brain_dump.open else {
        return Space::new().width(0.0).height(0.0).into();
    };
    let width = metrics.size.width * 0.95;
    let height = metrics.size.height * 0.90;
    let editor = text_editor(&state.brain_dump.editor)
        .id(editor_id())
        .placeholder("What is on your mind today?")
        .on_action(|action| msg(Action::Edited(action)))
        .size(metrics.label() * 1.1)
        .height(Fill);

    let header = row![
        column![
            text("BRAIN DUMP").size(11).style(text::secondary),
            text(date.format("%A, %B %-d, %Y").to_string()).size(if width < 360.0 {
                19
            } else {
                24
            }),
        ]
        .width(Fill),
        button("Delete")
            .on_press(msg(Action::Delete(date)))
            .style(button::danger),
        button("Close")
            .on_press(msg(Action::Close))
            .style(button::text),
    ]
    .spacing(8)
    .align_y(iced::Center);

    let body = column![
        header,
        text("Saved as you type").size(11).style(text::secondary),
        editor,
    ]
    .spacing(10);

    let card = container(body)
        .padding(if width < 320.0 { 10 } else { 20 })
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                text_color: Some(palette.background.base.text),
                border: iced::Border {
                    radius: 12.0.into(),
                    color: palette.background.strong.color,
                    width: 1.0,
                },
                ..Default::default()
            }
        });
    let backdrop =
        mouse_area(
            container(Space::new())
                .width(Fill)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(iced::Color::BLACK.scale_alpha(0.58).into()),
                    ..Default::default()
                }),
        )
        .on_press(msg(Action::Close));
    let foreground = container(
        mouse_area(card)
            .on_press(Message::Ignored)
            .on_scroll(|_| Message::Ignored),
    )
    .center_x(Fill)
    .center_y(Fill);
    stack![backdrop, foreground].into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::Screen, config::Config, theme::Catalog};
    use iced::Size;

    fn state() -> State {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.screen = Screen::BrainDump;
        state
    }

    #[test]
    fn the_screen_titles_each_entry_with_its_date_and_filters_by_substring() {
        let mut state = state();
        let first = chrono::NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        let second = chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        state
            .config
            .brain_dump
            .set(first, "Book the dentist".into());
        state.config.brain_dump.set(second, "Buy milk".into());
        state.brain_dump.query = "DENT".into();

        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            Size::new(500.0, 600.0),
            crate::ui::view(&state),
        );
        assert!(ui.find("Tuesday, October 27, 2026").is_ok());
        assert!(ui.find("Book the dentist").is_ok());
        assert!(ui.find("Buy milk").is_err());
    }

    #[test]
    fn the_editor_shows_the_page_and_its_actions() {
        let mut state = state();
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        state.config.brain_dump.set(day, "remember the milk".into());
        state.brain_dump.open = Some(day);
        state.brain_dump.editor =
            iced::widget::text_editor::Content::with_text("remember the milk");

        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            Size::new(500.0, 600.0),
            crate::ui::view(&state),
        );
        assert!(ui.find("BRAIN DUMP").is_ok());
        assert!(ui.find("Wednesday, October 28, 2026").is_ok());
        assert!(ui.find("Delete").is_ok());
        assert!(ui.find("Close").is_ok());
    }
}
