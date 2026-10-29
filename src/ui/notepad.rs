//! The notepad: a page of free text that is kept between sessions.
//!
//! The text belongs to the user, so it is written to the config file as it is
//! typed. The size of it is the size of the text on this page and nothing
//! else: the rest of the interface is left as it is drawn.

use iced::widget::{button, column, container, row, scrollable, text, text_editor};
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

    let controls = row![
        super::controls::section_heading(
            if metrics.breakpoint.is_compact() {
                "Note"
            } else {
                "Notepad"
            },
            None,
            metrics.label() + 4.0
        ),
        text_size(state, metrics),
    ]
    .spacing(metrics.spacing)
    .align_y(iced::Center);

    let mut body = column![controls, editor];
    if let Some(links) = super::links::for_editor(&state.config.notepad.text, metrics.label()) {
        body = body.push(container(scrollable(links).height(Length::Shrink)).max_height(100.0));
    }

    body.spacing(metrics.spacing)
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
        .style(crate::ui::controls::secondary);

    if enabled {
        button.on_press(message).into()
    } else {
        button.into()
    }
}
