//! The notepad: a page of free text that is kept between sessions.
//!
//! The text belongs to the user, so it is written to the config file as it is
//! typed. The size of it is the size of the text on this page and nothing
//! else: the rest of the interface is left as it is drawn.

use iced::widget::{button, column, row, text, text_editor, Space};
use iced::{Element, Fill, Length};

use crate::app::{Message, State};
use crate::config;

use super::layout::Metrics;

/// How much larger the notepad's own text is than the labels around it.
const TEXT_FACTOR: f32 = 1.4;

/// The id of the editor, so the cursor can be put in it.
pub fn input_id() -> iced::widget::Id {
    iced::widget::Id::from("notepad")
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    // The text on the page is the only thing the size control changes, so the
    // scale is applied here rather than to the whole interface.
    let size = metrics.label() * TEXT_FACTOR * state.config.notepad.font_scale();

    // The editor keeps the text while it is being typed, so what is drawn is
    // what the user has already written rather than what was last saved.
    let editor = text_editor(&state.notepad)
        .id(input_id())
        .placeholder("Write anything. It is kept between sessions.")
        .on_action(Message::NotepadEdited)
        .size(size)
        .height(Fill);

    // The size control sits under the tabs, where it is next to the text it
    // changes. In a narrow window the label is left off, as there is no room
    // for it and the buttons say what they do.
    let mut controls = row![].spacing(metrics.spacing).align_y(iced::Center);

    if !metrics.breakpoint.is_compact() {
        controls = controls.push(
            text("Text size")
                .size(metrics.label())
                .style(text::secondary),
        );
    }

    let controls = controls
        .push(Space::new().width(Fill))
        .push(text_size(state, metrics));

    column![
        controls,
        editor,
    ]
    .spacing(metrics.spacing)
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill)
    .into()
}

/// The control that makes the text on the page bigger or smaller.
///
/// It changes the size of the text on the notepad and nothing else, so the
/// rest of the interface is left as it is drawn.
fn text_size<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let size = state.config.notepad.font_size;
    let step = i32::from(config::Notepad::FONT_SIZE_STEP);

    let smaller = size_button(
        "A-",
        Message::NotepadFontSizeChanged(-step),
        metrics,
        size > config::Notepad::MIN_FONT_SIZE,
    );

    let bigger = size_button(
        "A+",
        Message::NotepadFontSizeChanged(step),
        metrics,
        size < config::Notepad::MAX_FONT_SIZE,
    );

    let readout = text(format!("{size}%"))
        .size(metrics.label())
        .style(text::secondary);

    row![smaller, readout, bigger]
        .spacing(metrics.spacing)
        .align_y(iced::Center)
        .into()
}

/// One of the two size buttons, which does nothing at the end of its range.
fn size_button<'a>(
    label: &'a str,
    message: Message,
    metrics: &Metrics,
    enabled: bool,
) -> Element<'a, Message> {
    let button = button(text(label).size(metrics.label()).center())
        .width(Length::Fixed(metrics.icon_button()))
        .height(Length::Fixed(metrics.icon_button()))
        .padding(0.0)
        .style(button::secondary);

    if enabled {
        button.on_press(message).into()
    } else {
        button.into()
    }
}
