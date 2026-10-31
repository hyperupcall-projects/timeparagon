//! The Projects tab: a selectable list with an editable overview beside it.

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Color, Element, Fill, Length};

use crate::app::{Message, State};
use crate::calendar::colors;
use crate::config::Project;
use crate::projects::{Action, Draft};

use super::calendar::{color_circle, swatch_button};
use super::layout::Metrics;

/// Room left for the scrollbar, which is drawn over the content.
const SCROLLBAR_GUTTER: f32 = 14.0;

/// What a project without a color of its own is drawn with.
const NO_COLOR: Color = Color::from_rgb(0.62, 0.62, 0.64);

fn msg(action: Action) -> Message {
    Message::Projects(action)
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut items = column![add_box(state, metrics)]
        .spacing(metrics.spacing)
        .width(Fill);

    for project in state.config.projects.sorted() {
        items = items.push(project_row(state, project, metrics));
    }

    if state.config.projects.items.is_empty() {
        items = items.push(
            text("No projects yet. Add one above.")
                .size(metrics.label())
                .style(text::secondary),
        );
    }

    let list = scrollable(container(items).padding(iced::Padding {
        right: SCROLLBAR_GUTTER,
        ..iced::Padding::ZERO
    }))
    .width(Fill)
    .height(Fill);

    let body: Element<'a, Message> = if let Some(draft) = &state.projects.editing {
        let overview = container(
            scrollable(
                container(overview(state, draft, metrics)).padding(iced::Padding {
                    right: SCROLLBAR_GUTTER,
                    ..iced::Padding::ZERO
                }),
            )
            .height(Fill),
        )
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill)
        .style(super::controls::card_surface);

        if metrics.size.width >= 680.0 {
            row![
                container(list).width(Length::FillPortion(2)),
                container(overview).width(Length::FillPortion(3)),
            ]
            .spacing(metrics.spacing)
            .height(Fill)
            .into()
        } else {
            overview.into()
        }
    } else {
        list.into()
    };

    container(
        column![
            super::controls::page_heading(
                "Projects",
                "Select a project to view and edit its details",
                Some(state.config.projects.items.len()),
                metrics,
            ),
            body,
        ]
        .spacing(metrics.spacing)
        .height(Fill),
    )
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill)
    .into()
}

fn add_box<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let input = text_input("New project, e.g. School/Physics", &state.projects.new_name)
        .on_input(|name| msg(Action::NewName(name)))
        .on_submit(msg(Action::Add))
        .size(metrics.label())
        .width(Fill);

    let add = button(text("Add").size(metrics.label()).center())
        .on_press(msg(Action::Add))
        .style(crate::ui::controls::primary);

    let mut content = column![row![input, add].spacing(6).align_y(Alignment::Center)]
        .spacing(4)
        .width(Fill);

    if let Some(error) = &state.projects.add_error {
        content = content.push(text(error).size(metrics.label()).style(text::danger));
    }

    content.into()
}

fn project_row<'a>(
    state: &'a State,
    project: &'a Project,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let selected = state
        .projects
        .editing
        .as_ref()
        .is_some_and(|draft| draft.id == project.id);
    let mut content = column![
        row![
            color_circle(project.color().unwrap_or(NO_COLOR)),
            text(&project.name)
                .wrapping(text::Wrapping::WordOrGlyph)
                .size(metrics.label() + 1.0)
                .width(Fill),
            text("›").size(metrics.label() + 2.0),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    ]
    .spacing(4)
    .width(Fill);

    if let Some(directory) = &project.directory {
        content = content.push(
            text(directory.display().to_string())
                .wrapping(text::Wrapping::WordOrGlyph)
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }

    button(content)
        .on_press(msg(Action::Edit(project.id)))
        .padding([10, 12])
        .width(Fill)
        .style(if selected {
            super::controls::primary
        } else {
            super::controls::secondary
        })
        .into()
}

fn overview<'a>(state: &State, draft: &'a Draft, metrics: &Metrics) -> Element<'a, Message> {
    let mut content = column![
        row![
            text("Project overview")
                .size(metrics.label() + 3.0)
                .width(Fill),
            button(text("Close").size(metrics.label()))
                .on_press(msg(Action::CancelEdit))
                .style(button::text),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        text(format!("Project ID: {}", draft.id))
            .size(metrics.label() - 1.0)
            .style(text::secondary),
        editor(draft, metrics),
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    if state.projects.confirm_remove == Some(draft.id) {
        content = content.push(
            column![
                text("Remove this project?").size(metrics.label()),
                row![
                    button(text("Remove").size(metrics.label()))
                        .on_press(msg(Action::ConfirmRemove(draft.id)))
                        .style(button::danger),
                    button(text("Keep").size(metrics.label()))
                        .on_press(msg(Action::CancelRemove))
                        .style(super::controls::secondary),
                ]
                .spacing(6),
            ]
            .spacing(4),
        );
    } else {
        content = content.push(
            button(text("Remove project").size(metrics.label()))
                .on_press(msg(Action::Remove(draft.id)))
                .style(button::danger),
        );
    }
    content.into()
}

/// The fields of one project: name, color, folder.
fn editor<'a>(draft: &'a Draft, metrics: &Metrics) -> Element<'a, Message> {
    let label = |label: &'static str| {
        text(label)
            .size(metrics.label() - 1.0)
            .style(text::secondary)
    };

    let name = text_input("Project name", &draft.name)
        .on_input(|name| msg(Action::Name(name)))
        .on_submit(msg(Action::Save))
        .size(metrics.label())
        .width(Fill);

    let mut swatches = column![].spacing(6);
    for chunk in colors::palette().chunks(if metrics.size.width < 300.0 { 3 } else { 6 }) {
        let mut line = row![].spacing(6);
        for &color in chunk {
            line = line.push(swatch_button(
                color,
                draft.color == Some(color),
                msg(Action::Color(Some(color))),
            ));
        }
        swatches = swatches.push(line);
    }
    let no_color = button(
        text("No color")
            .size(metrics.label() - 1.0)
            .center()
            .width(Fill),
    )
    .width(Fill)
    .on_press(msg(Action::Color(None)))
    .style(if draft.color.is_none() {
        crate::ui::controls::primary
    } else {
        crate::ui::controls::secondary
    });

    let path = text_input("/path/to/project/files", &draft.directory)
        .on_input(|directory| msg(Action::Directory(directory)))
        .on_submit(msg(Action::Save))
        .size(metrics.label())
        .width(Fill);
    let browse = button(text("Browse…").size(metrics.label()).center())
        .on_press(msg(Action::Browse))
        .style(crate::ui::controls::secondary);
    let directory: Element<'a, Message> = if metrics.size.width < 420.0 {
        column![path, browse].spacing(6).into()
    } else {
        row![path, browse]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
    };

    let mut content = column![
        label("Name"),
        name,
        label("Color"),
        swatches,
        no_color,
        label("Folder path"),
        directory,
    ]
    .spacing(4)
    .width(Fill);

    if let Some(error) = &draft.error {
        content = content.push(text(error).size(metrics.label()).style(text::danger));
    }

    content
        .push(Space::new().height(Length::Fixed(2.0)))
        .push(
            row![
                button(text("Save").size(metrics.label()).center())
                    .on_press(msg(Action::Save))
                    .style(crate::ui::controls::primary),
                button(text("Cancel").size(metrics.label()).center())
                    .on_press(msg(Action::CancelEdit))
                    .style(crate::ui::controls::secondary),
            ]
            .spacing(6),
        )
        .into()
}
