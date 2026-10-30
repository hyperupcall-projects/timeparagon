//! A compact description shared by task, event and habit details.
use super::layout::Metrics;
use crate::app::{Message, State};
use iced::widget::{Space, button, column, container, mouse_area, row, stack, text, text_editor};
use iced::{Element, Fill};

pub fn preview<'a>(id: &str, value: &str, metrics: &Metrics) -> Element<'a, Message> {
    let size = metrics.label();
    column![
        container(
            text(value.to_owned())
                .size(size)
                .line_height(1.3)
                .width(Fill)
        )
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
    let card = container(
        column![
            text("Description").size(metrics.label() + 6.0),
            text_editor(&state.description_editor)
                .placeholder("Add description…")
                .on_action(Message::DescriptionEdited)
                .size(metrics.label())
                .height((metrics.size.height - 180.0).clamp(80.0, 480.0)),
            row![
                button("Save description").on_press(Message::DescriptionSave),
                button("Cancel")
                    .on_press(Message::DescriptionClose)
                    .style(super::controls::secondary),
            ]
            .spacing(8),
        ]
        .spacing(12),
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
