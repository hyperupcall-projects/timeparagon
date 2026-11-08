//! The Life tab: a list of life areas, and a side panel for the one that is
//! open, with its vision and goals in one tab and the objectives, habits and
//! projects that carry it out in the other.

use iced::widget::{Space, button, column, container, mouse_area, opaque, row, stack, text};
use iced::{Alignment, Color, Element, Fill, Length, Padding, Theme};
use timeslay_core::notes::Pastel;

use crate::app::{Message, State};
use crate::calendar::colors;
use crate::config::{LifeArea, ProjectsSettings};
use crate::life::{Action, Open, Tab, VisionMode};

use super::calendar::{color_circle, swatch_button};
use super::layout::Metrics;

/// Room left for the scrollbar, which is drawn over the content.
const SCROLLBAR_GUTTER: f32 = 14.0;

/// What an area or project without a color of its own is drawn with.
const NO_COLOR: Color = Color::from_rgb(0.62, 0.62, 0.64);

/// The widest the side panel gets.
const PANEL_MAX_WIDTH: f32 = 560.0;

fn msg(action: Action) -> Message {
    Message::Life(action)
}

/// An area offered in a "move to" picker.
#[derive(Debug, Clone, PartialEq)]
struct AreaChoice {
    id: u64,
    name: String,
}

impl std::fmt::Display for AreaChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

fn choices(projects: &ProjectsSettings) -> Vec<AreaChoice> {
    projects
        .areas
        .iter()
        .map(|area| AreaChoice {
            id: area.id,
            name: area.name.clone(),
        })
        .collect()
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let projects = &state.config.projects;
    let mut items = column![add_box(state, metrics)]
        .spacing(metrics.spacing)
        .width(Fill);

    for area in &projects.areas {
        items = items.push(area_row(state, area, metrics));
    }

    let list = crate::ui::controls::scrollable(container(items).padding(Padding {
        right: SCROLLBAR_GUTTER,
        ..Padding::ZERO
    }))
    .width(Fill)
    .height(Fill);

    let page: Element<'a, Message> = container(
        column![
            super::controls::page_heading(
                "Life",
                "Select a life area to see its vision and what carries it out",
                Some(projects.areas.len()),
                metrics,
            ),
            list,
        ]
        .spacing(metrics.spacing)
        .height(Fill),
    )
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill)
    .into();

    let Some(open) = &state.life.open else {
        return page;
    };
    let Some(area) = projects.area(open.area) else {
        return page;
    };

    let width = if metrics.size.width < 600.0 {
        metrics.size.width
    } else {
        (metrics.size.width * 0.6).min(PANEL_MAX_WIDTH)
    };
    let scrim = mouse_area(container(Space::new()).width(Fill).height(Fill).style(
        |_theme: &Theme| container::Style {
            background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.25).into()),
            ..container::Style::default()
        },
    ))
    .on_press(msg(Action::Close));
    let panel = container(panel(state, area, open, metrics))
        .padding(metrics.padding)
        .width(Length::Fixed(width))
        .height(Fill)
        .style(super::controls::card_surface);

    stack![page, row![scrim, opaque(panel)].height(Fill)].into()
}

fn add_box<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let input = crate::ui::controls::text_input("New life area, e.g. Health", &state.life.new_name)
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

    if let Some(error) = &state.life.add_error {
        content = content.push(text(error).size(metrics.label()).style(text::danger));
    }

    content.into()
}

/// How much is in an area, for its row.
fn summary(state: &State, area: &LifeArea) -> String {
    let projects = &state.config.projects;
    let project_count = projects
        .items
        .iter()
        .filter(|project| projects.resolve_area(project.area) == area.id)
        .count();
    let habit_count = state
        .habits
        .library
        .habits
        .iter()
        .filter(|habit| projects.resolve_area(habit.area) == area.id)
        .count();
    let plural = |count: usize, one: &str, many: &str| {
        format!("{count} {}", if count == 1 { one } else { many })
    };
    [
        plural(area.goals.len(), "goal", "goals"),
        plural(project_count, "project", "projects"),
        plural(habit_count, "habit", "habits"),
    ]
    .join(" · ")
}

fn area_row<'a>(state: &'a State, area: &'a LifeArea, metrics: &Metrics) -> Element<'a, Message> {
    let selected = state
        .life
        .open
        .as_ref()
        .is_some_and(|open| open.area == area.id);
    let content = column![
        row![
            color_circle(area.color().unwrap_or(NO_COLOR)),
            text(&area.name)
                .wrapping(text::Wrapping::WordOrGlyph)
                .size(metrics.label() + 1.0)
                .width(Fill),
            text("›").size(metrics.label() + 2.0),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        text(summary(state, area))
            .size(metrics.label() - 1.0)
            .style(text::secondary),
    ]
    .spacing(4)
    .width(Fill);

    button(content)
        .on_press(msg(Action::Open(area.id)))
        .padding([10, 12])
        .width(Fill)
        .style(if selected {
            super::controls::primary
        } else {
            super::controls::secondary
        })
        .into()
}

fn label<'a>(caption: impl text::IntoFragment<'a>, metrics: &Metrics) -> text::Text<'a, Theme> {
    text(caption)
        .size(metrics.label() - 1.0)
        .style(text::secondary)
}

fn heading<'a>(caption: &'a str, metrics: &Metrics) -> Element<'a, Message> {
    text(caption).size(metrics.label() + 2.0).into()
}

fn panel<'a>(
    state: &'a State,
    area: &'a LifeArea,
    open: &'a Open,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let size = metrics.label();
    let tab = |caption: &'static str, which: Tab| {
        button(text(caption).size(size))
            .on_press(msg(Action::ShowTab(which)))
            .style(crate::ui::controls::nav(open.tab == which))
    };
    let top = row![
        color_circle(area.color().unwrap_or(NO_COLOR)),
        text(&area.name)
            .size(size + 4.0)
            .wrapping(text::Wrapping::WordOrGlyph)
            .width(Fill),
        button(text("Close").size(size))
            .on_press(msg(Action::Close))
            .style(crate::ui::controls::ghost),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    let tabs = row![
        tab("Vision", Tab::Vision),
        tab("Implementation", Tab::Implementation)
    ]
    .spacing(metrics.spacing);

    let body = match open.tab {
        Tab::Vision => vision_tab(area, open, metrics),
        Tab::Implementation => implementation_tab(state, area, metrics),
    };

    column![
        top,
        tabs,
        crate::ui::controls::scrollable(container(body).padding(Padding {
            right: SCROLLBAR_GUTTER,
            ..Padding::ZERO
        }))
        .width(Fill)
        .height(Fill),
    ]
    .spacing(metrics.spacing)
    .width(Fill)
    .height(Fill)
    .into()
}

fn vision_tab<'a>(area: &'a LifeArea, open: &'a Open, metrics: &Metrics) -> Element<'a, Message> {
    let size = metrics.label();
    let mode = |caption: &'static str, which: VisionMode| {
        button(text(caption).size(size - 1.0))
            .on_press(msg(Action::VisionMode(which)))
            .style(crate::ui::controls::nav(open.vision_mode == which))
    };
    let vision_header = row![
        text("Vision").size(size + 2.0).width(Fill),
        mode("Edit", VisionMode::Edit),
        mode("View", VisionMode::View),
    ]
    .spacing(4)
    .align_y(Alignment::Center);

    let vision: Element<'a, Message> = match open.vision_mode {
        VisionMode::Edit => crate::ui::controls::text_editor(&open.vision)
            .placeholder(
                "Describe where this part of your life is going, in Markdown: # headings, - bullets, **bold**",
            )
            .on_action(|action| msg(Action::VisionEdited(action)))
            .size(size + 1.0)
            .height(Length::Fixed((metrics.size.height * 0.35).max(140.0)))
            .into(),
        VisionMode::View => container(super::notes::rendered(&area.vision, size + 1.0))
            .padding(10.0)
            .width(Fill)
            .style(super::notes::note_surface(Pastel::default()))
            .into(),
    };

    column![
        vision_header,
        vision,
        Space::new().height(Length::Fixed(4.0)),
        heading("Goals", metrics),
        goals(area, open, metrics),
        Space::new().height(Length::Fixed(4.0)),
        area_settings(area, open, metrics),
    ]
    .spacing(metrics.spacing)
    .width(Fill)
    .into()
}

fn goals<'a>(area: &'a LifeArea, open: &'a Open, metrics: &Metrics) -> Element<'a, Message> {
    let size = metrics.label();
    let mut list = column![].spacing(metrics.spacing).width(Fill);

    for goal in &area.goals {
        let id = goal.id;
        let name = crate::ui::controls::text_input("Goal", &goal.name)
            .on_input(move |name| msg(Action::GoalName(id, name)))
            .size(size)
            .width(Fill);
        let remove = button(text("Remove").size(size - 1.0))
            .on_press(msg(Action::RemoveGoal(id)))
            .style(crate::ui::controls::ghost);
        let description = crate::ui::controls::text_input("Short description", &goal.description)
            .on_input(move |description| msg(Action::GoalDescription(id, description)))
            .size(size - 1.0)
            .width(Fill);
        list = list.push(
            container(
                column![
                    row![name, remove].spacing(6).align_y(Alignment::Center),
                    description,
                ]
                .spacing(4),
            )
            .padding(8)
            .width(Fill)
            .style(super::controls::row_surface),
        );
    }

    if area.goals.is_empty() {
        list = list.push(label("No goals yet. Add one below.", metrics));
    }

    let input = crate::ui::controls::text_input("New goal", &open.new_goal)
        .on_input(|name| msg(Action::NewGoal(name)))
        .on_submit(msg(Action::AddGoal))
        .size(size)
        .width(Fill);
    let add = button(text("Add goal").size(size).center())
        .on_press(msg(Action::AddGoal))
        .style(crate::ui::controls::primary);

    list.push(row![input, add].spacing(6).align_y(Alignment::Center))
        .into()
}

/// The area's name and color, and removing it.
fn area_settings<'a>(
    area: &'a LifeArea,
    open: &'a Open,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let size = metrics.label();
    let name = crate::ui::controls::text_input("Life area name", &open.name)
        .on_input(|name| msg(Action::Rename(name)))
        .size(size)
        .width(Fill);

    let current = area.color();
    let mut swatches = column![].spacing(6);
    for chunk in colors::palette().chunks(if metrics.size.width < 300.0 { 3 } else { 6 }) {
        let mut line = row![].spacing(6);
        for &color in chunk {
            line = line.push(swatch_button(
                color,
                current == Some(color),
                msg(Action::Color(Some(color))),
            ));
        }
        swatches = swatches.push(line);
    }
    let no_color = button(text("No color").size(size - 1.0).center())
        .on_press(msg(Action::Color(None)))
        .style(if current.is_none() {
            crate::ui::controls::primary
        } else {
            crate::ui::controls::secondary
        });

    let mut content = column![heading("Life area", metrics), label("Name", metrics), name]
        .spacing(4)
        .width(Fill);
    if let Some(error) = &open.name_error {
        content = content.push(text(error).size(size).style(text::danger));
    }
    content = content
        .push(label("Color", metrics))
        .push(swatches)
        .push(no_color);

    if area.id == crate::config::PERSONAL_AREA {
        content = content.push(label(
            "Projects and habits not in another area are kept here, so it cannot be removed.",
            metrics,
        ));
    } else if open.confirm_remove {
        content = content.push(
            column![
                text("Remove this life area? What is in it moves to Personal.").size(size),
                row![
                    button(text("Remove").size(size))
                        .on_press(msg(Action::ConfirmRemove))
                        .style(crate::ui::controls::danger),
                    button(text("Keep").size(size))
                        .on_press(msg(Action::CancelRemove))
                        .style(super::controls::secondary),
                ]
                .spacing(6),
            ]
            .spacing(4),
        );
    } else {
        content = content.push(
            button(text("Remove life area").size(size))
                .on_press(msg(Action::Remove))
                .style(crate::ui::controls::danger),
        );
    }

    content.into()
}

fn implementation_tab<'a>(
    state: &'a State,
    area: &'a LifeArea,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let size = metrics.label();
    let projects = &state.config.projects;
    let options = choices(projects);
    let current = options.iter().find(|choice| choice.id == area.id).cloned();

    // A picker on each item moves it to another area.
    let item = |dot: Element<'a, Message>,
                name: String,
                detail: Option<String>,
                picker: Element<'a, Message>|
     -> Element<'a, Message> {
        let mut words = column![text(name).size(size).wrapping(text::Wrapping::WordOrGlyph)]
            .spacing(2)
            .width(Fill);
        if let Some(detail) = detail {
            words = words.push(
                text(detail)
                    .size(size - 1.0)
                    .style(text::secondary)
                    .wrapping(text::Wrapping::WordOrGlyph),
            );
        }
        container(
            row![dot, words, picker]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .padding([6, 8])
        .width(Fill)
        .style(super::controls::row_surface)
        .into()
    };

    // Objectives: the goals this area is working towards.
    let mut objectives = column![heading("Objectives", metrics)].spacing(6);
    for goal in &area.goals {
        let description = (!goal.description.trim().is_empty()).then(|| goal.description.clone());
        objectives = objectives.push(item(
            color_circle(area.color().unwrap_or(NO_COLOR)),
            goal.name.clone(),
            description,
            Space::new().into(),
        ));
    }
    if area.goals.is_empty() {
        objectives = objectives.push(label("No goals yet. Add them under Vision.", metrics));
    }

    let mut habits = column![heading("Habits", metrics)].spacing(6);
    let mut any_habit = false;
    for habit in &state.habits.library.habits {
        if projects.resolve_area(habit.area) != area.id {
            continue;
        }
        any_habit = true;
        let id = habit.id.clone();
        let picker = crate::ui::controls::pick_list(options.clone(), current.clone(), move |to| {
            msg(Action::MoveHabit(id.clone(), to.id))
        })
        .text_size(size - 1.0)
        .into();
        habits = habits.push(item(
            habit_dot(habit.color),
            habit.name.clone(),
            Some(habit.goal_label()).map(|goal| {
                if habit.archived {
                    format!("{goal} · Archived")
                } else {
                    goal
                }
            }),
            picker,
        ));
    }
    if !any_habit {
        habits = habits.push(label("No habits in this area.", metrics));
    }

    let mut project_list = column![heading("Projects", metrics)].spacing(6);
    let mut any_project = false;
    for project in projects.sorted() {
        if projects.resolve_area(project.area) != area.id {
            continue;
        }
        any_project = true;
        let id = project.id;
        let picker = crate::ui::controls::pick_list(options.clone(), current.clone(), move |to| {
            msg(Action::MoveProject(id, to.id))
        })
        .text_size(size - 1.0)
        .into();
        project_list = project_list.push(item(
            color_circle(project.color().unwrap_or(NO_COLOR)),
            project.name.clone(),
            project
                .directory
                .as_ref()
                .map(|directory| directory.display().to_string()),
            picker,
        ));
    }
    if !any_project {
        project_list = project_list.push(label("No projects in this area.", metrics));
    }

    column![
        objectives,
        Space::new().height(Length::Fixed(4.0)),
        habits,
        Space::new().height(Length::Fixed(4.0)),
        project_list,
    ]
    .spacing(metrics.spacing)
    .width(Fill)
    .into()
}

/// A habit's color, which depends on whether the theme is dark.
fn habit_dot<'a>(index: u8) -> Element<'a, Message> {
    container(Space::new().width(10.0).height(10.0))
        .style(move |theme: &Theme| container::Style {
            background: Some(
                timeslay_core::habits::palette_color(index, theme.extended_palette().is_dark)
                    .into(),
            ),
            border: iced::Border {
                radius: crate::ui::controls::RADIUS_ROW.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into()
}
