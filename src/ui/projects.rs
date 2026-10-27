//! The Projects tab: the list of projects, with adding, renaming, a color and
//! a folder for each, and removing.

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Color, Element, Fill, Length, Theme};

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

    container(
        scrollable(container(items).padding(iced::Padding {
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

fn add_box<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let input = text_input("New project, e.g. School/Physics", &state.projects.new_name)
        .on_input(|name| msg(Action::NewName(name)))
        .on_submit(msg(Action::Add))
        .size(metrics.label())
        .width(Fill);

    let add = button(text("Add").size(metrics.label()).center())
        .on_press(msg(Action::Add))
        .style(button::primary);

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
    let editing = state
        .projects
        .editing
        .as_ref()
        .filter(|draft| draft.id == project.id);

    let content: Element<'a, Message> = match editing {
        Some(draft) => editor(draft, metrics),
        None => summary(state, project, metrics),
    };

    container(content)
        .padding([6, 8])
        .width(Fill)
        .style(|theme: &Theme| {
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
        })
        .into()
}

/// A project as listed: color, name, folder, and the buttons for it.
fn summary<'a>(state: &State, project: &'a Project, metrics: &Metrics) -> Element<'a, Message> {
    let header = row![
        color_circle(project.color().unwrap_or(NO_COLOR)),
        text(&project.name).size(metrics.label() + 1.0).width(Fill),
        button(text("Edit").size(metrics.label()))
            .on_press(msg(Action::Edit(project.id)))
            .padding([2, 6])
            .style(button::text),
        button(text("✕").size(metrics.label()))
            .on_press(msg(Action::Remove(project.id)))
            .padding([2, 6])
            .style(button::text),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let mut content = column![header].spacing(2).width(Fill);

    if let Some(directory) = &project.directory {
        content = content.push(
            text(directory.display().to_string())
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }

    if state.projects.confirm_remove == Some(project.id) {
        content = content.push(
            column![
                text(format!("Remove {}?", project.name)).size(metrics.label()),
                row![
                    button(text("Remove").size(metrics.label()).center())
                        .on_press(msg(Action::ConfirmRemove(project.id)))
                        .style(button::danger),
                    button(text("Keep").size(metrics.label()).center())
                        .on_press(msg(Action::CancelRemove))
                        .style(button::secondary),
                ]
                .spacing(6),
            ]
            .spacing(4),
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
    for chunk in colors::palette().chunks(6) {
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
        button::primary
    } else {
        button::secondary
    });

    let directory = row![
        text_input("/path/to/project/files", &draft.directory)
            .on_input(|directory| msg(Action::Directory(directory)))
            .on_submit(msg(Action::Save))
            .size(metrics.label())
            .width(Fill),
        button(text("Browse…").size(metrics.label()).center())
            .on_press(msg(Action::Browse))
            .style(button::secondary),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let mut content = column![
        label("Name"),
        name,
        label("Color"),
        swatches,
        no_color,
        label("Folder"),
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
                    .style(button::primary),
                button(text("Cancel").size(metrics.label()).center())
                    .on_press(msg(Action::CancelEdit))
                    .style(button::secondary),
            ]
            .spacing(6),
        )
        .into()
}
