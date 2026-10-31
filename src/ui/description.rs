//! A compact description shared by task, event and habit details.
use super::layout::Metrics;
use crate::app::{Message, State};
use iced::widget::{Space, button, column, container, mouse_area, row, stack, text, text_editor};
use iced::{Element, Fill};

pub fn preview<'a>(id: &str, value: &str, metrics: &Metrics) -> Element<'a, Message> {
    let size = metrics.label();
    column![
        container(super::links::text(value, size))
            .max_height(size * 1.3 * 5.0)
            .clip(true)
            .width(Fill),
        button(
            text(if value.is_empty() {
                "Add description"
            } else {
                "View / edit description"
            })
            .size(size)
        )
        .on_press(Message::DescriptionOpen(id.to_owned()))
        .style(super::controls::secondary)
        .width(Fill),
    ]
    .spacing(4)
    .width(Fill)
    .into()
}

pub fn overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut description = column![
        text("Description").size(metrics.label() + 6.0),
        text_editor(&state.description_editor)
            .placeholder("Add description…")
            .on_action(Message::DescriptionEdited)
            .size(metrics.label())
            .height((metrics.size.height - 180.0).clamp(80.0, 480.0)),
    ]
    .spacing(12);
    if let Some(links) = super::links::for_editor(&state.description_editor.text(), metrics.label())
    {
        description = description.push(links);
    }
    let card = container(
        description.push(
            row![
                button("Save description").on_press(Message::DescriptionSave),
                button("Cancel")
                    .on_press(Message::DescriptionClose)
                    .style(super::controls::secondary),
            ]
            .spacing(8),
        ),
    )
    .padding(16)
    .width(Fill)
    .max_width(640)
    .style(super::controls::surface);
    stack![
        mouse_area(
            container(Space::new())
                .width(Fill)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(iced::Color::BLACK.scale_alpha(0.58).into()),
                    ..Default::default()
                })
        )
        .on_press(Message::DescriptionClose)
        .on_scroll(|_| Message::Ignored),
        container(
            mouse_area(card)
                .on_press(Message::Ignored)
                .on_scroll(|_| Message::Ignored)
        )
        .padding(16)
        .center_x(Fill)
        .center_y(Fill),
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::{Point, Size};

    #[test]
    fn description_links_open_the_full_http_or_https_url() {
        let metrics = Metrics::of(Size::new(360.0, 800.0));
        for url in [
            "http://example.com/notes",
            "https://example.com/notes?q=1&view=full#section",
        ] {
            let value = format!("{url}.\nMore notes");
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                Size::new(600.0, 200.0),
                preview("item-id", &value, &metrics),
            );
            ui.point_at(Point::new(10.0, 10.0));
            ui.simulate(iced_test::simulator::click());
            assert!(ui.into_messages().any(|message| matches!(
                message,
                Message::OpenEventLink(target) if target == url
            )));
        }
    }

    #[test]
    fn description_still_opens_for_editing() {
        let metrics = Metrics::of(Size::new(360.0, 800.0));
        for value in ["", "Notes: https://example.com"] {
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                Size::new(360.0, 200.0),
                preview("item-id", value, &metrics),
            );
            ui.click(if value.is_empty() {
                "Add description"
            } else {
                "View / edit description"
            })
            .unwrap();
            assert!(ui.into_messages().any(|message| matches!(
                message,
                Message::DescriptionOpen(id) if id == "item-id"
            )));
        }
    }
}
