//! The temporary todo scratchpad.
//!
//! A flat list drawn with an indent, so a few items can read as subtasks of the
//! one above without the list itself being a tree. Every item carries its own
//! controls, which is what keeps reordering and indenting a one-tap affair
//! rather than a mode to enter.

use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Center, Element, Fill, Length, Padding};

use crate::app::{Message, Screen, State};
use crate::todos::Todo;

use super::layout::Metrics;

/// How far one indent level pushes an item in.
const INDENT_STEP: f32 = 14.0;

/// The id of the text field for the item at this index.
///
/// A freshly added item is focused through this, so the cursor is already in
/// the new row rather than the user having to find it.
pub fn input_id(index: usize) -> iced::widget::Id {
    iced::widget::Id::from(format!("todo-{index}"))
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut body = column![].spacing(metrics.spacing).width(Fill);

    if state.config.todos.is_empty() {
        body = body.push(
            text("Add a todo to break the work up, then tap one to make it current.")
                .size(metrics.label())
                .style(text::secondary),
        );
    } else {
        // The page already carries the title, so this row is only the way to
        // throw the whole list away.
        body = body.push(
            row![Space::new().width(Fill), clear_button(metrics)].align_y(Center),
        );

        for (index, todo) in state.config.todos.items().iter().enumerate() {
            body = body.push(item(state, index, todo, metrics));
        }
    }

    body = body.push(add_button(metrics));

    // The way back sits at the foot of the page, the same place the clock puts
    // its own, so leaving either view is always the same gesture.
    column![
        text("Todos").size(metrics.label() + 3.0),
        scrollable(body).height(Fill).width(Fill),
        go_back_button(metrics),
    ]
    .spacing(metrics.spacing)
    .padding(metrics.padding)
    .into()
}

/// The way back to the calendar.
fn go_back_button<'a>(metrics: &Metrics) -> Element<'a, Message> {
    button(text("Go back").size(metrics.label()).center().width(Fill))
        .width(Fill)
        .height(metrics.icon_button())
        .on_press(Message::ScreenSelected(Screen::Calendar))
        .style(crate::ui::controls::secondary)
        .into()
}

/// One item: the marker that makes it current, its text, and the controls that
/// reorder and indent it.
fn item<'a>(
    state: &'a State,
    index: usize,
    todo: &'a Todo,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let current = state.config.todos.current_index() == Some(index);

    let marker = button(
        text(if current { "●" } else { "○" })
            .size(metrics.label())
            .center(),
    )
    .width(Length::Fixed(metrics.icon_button()))
    .height(Length::Fixed(metrics.icon_button()))
    .padding(0.0)
    .on_press(Message::TodoSelected(index))
    .style(if current {
        crate::ui::controls::primary
    } else {
        crate::ui::controls::secondary
    });

    let input = text_input("todo", &todo.text)
        .id(input_id(index))
        .on_input(move |text| Message::TodoTextChanged(index, text))
        .size(metrics.label())
        .width(Fill);

    let remove = control("✕", Message::TodoRemoved(index), metrics);

    let controls = || {
        row![
            control("↑", Message::TodoMoved(index, -1), metrics),
            control("↓", Message::TodoMoved(index, 1), metrics),
            control("→", Message::TodoIndented(index, 1), metrics),
            control("←", Message::TodoIndented(index, -1), metrics),
        ]
        .spacing(metrics.spacing)
    };

    // The controls sit under the text rather than beside it: even on a wide
    // window this pane is a narrow column next to the calendar, and a single
    // line would leave the text field almost no room.
    let mut line = column![
        row![marker, input, remove]
            .spacing(metrics.spacing)
            .align_y(Center),
        controls().width(Fill),
    ]
    .spacing(2.0);

    if let Some(links) = super::links::for_editor(&todo.text, metrics.label()) {
        line = line.push(links);
    }

    container(line)
        .padding(Padding {
            left: f32::from(todo.indent) * INDENT_STEP,
            ..Padding::ZERO
        })
        .width(Fill)
        .into()
}

/// A square button for one of the per-item actions.
fn control<'a>(label: &'a str, message: Message, metrics: &Metrics) -> Element<'a, Message> {
    button(text(label).size(metrics.label()).center())
        .width(Length::Fixed(metrics.icon_button()))
        .height(Length::Fixed(metrics.icon_button()))
        .padding(0.0)
        .on_press(message)
        .style(crate::ui::controls::secondary)
        .into()
}

fn add_button<'a>(metrics: &Metrics) -> Element<'a, Message> {
    button(text("+ Add todo").size(metrics.label()).center().width(Fill))
        .width(Fill)
        .height(metrics.icon_button())
        .on_press(Message::TodoAdded)
        .style(crate::ui::controls::secondary)
        .into()
}

/// The way to throw the whole scratchpad away.
fn clear_button<'a>(metrics: &Metrics) -> Element<'a, Message> {
    button(text("Clear").size(metrics.label()))
        .on_press(Message::TodosCleared)
        .style(crate::ui::controls::secondary)
        .into()
}
