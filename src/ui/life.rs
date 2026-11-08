//! The Life tab: a list of life areas, and a side panel for the one that is
//! open, with its vision and goals in one tab and, in the other, each goal's
//! objectives and their habits, then what is unassigned, then the projects.

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Color, Element, Fill, Length, Padding, Theme};
use timeslay_core::notes::Pastel;

use crate::app::{Message, State};
use crate::calendar::colors;
use crate::config::{LifeArea, ProjectsSettings};
use crate::life::{Action, Open, ProjectEdit, Tab, VisionMode};

use super::calendar::{color_circle, swatch_button};
use super::layout::Metrics;

/// Room left for the scrollbar, which is drawn over the content.
const SCROLLBAR_GUTTER: f32 = 14.0;

/// What an area or project without a color of its own is drawn with.
const NO_COLOR: Color = Color::from_rgb(0.62, 0.62, 0.64);

/// Room for the vision editor and implementation lists in the side window.
pub const PANEL_WIDTH: f32 = 560.0;

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

    // Keep the list visible when the area has its own side window. Embedded
    // hosts without a main window still need somewhere to show the details.
    if state.editor_window.is_none() && state.life_panel_open() {
        return panel(state, metrics);
    }
    page
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
        plural(area.objectives.len(), "objective", "objectives"),
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

/// The open life area's content, hosted in the separate side window.
pub fn panel<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(open) = &state.life.open else {
        return Space::new().into();
    };
    let Some(area) = state.config.projects.area(open.area) else {
        return Space::new().into();
    };
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
        Tab::Implementation => implementation_tab(state, area, open, metrics),
    };

    container(
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
        .height(Fill),
    )
    .padding(metrics.padding)
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

/// A habit offered in an objective's "add habit" picker.
#[derive(Debug, Clone, PartialEq)]
struct HabitChoice {
    id: String,
    name: String,
}

impl std::fmt::Display for HabitChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// A goal offered in an objective's "goal" picker; `None` is no goal.
#[derive(Debug, Clone, PartialEq)]
struct GoalChoice {
    id: Option<u64>,
    name: String,
}

impl std::fmt::Display for GoalChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

fn implementation_tab<'a>(
    state: &'a State,
    area: &'a LifeArea,
    open: &'a Open,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let gap = || Space::new().height(Length::Fixed(4.0));

    // Goals, each with the objectives that serve it.
    let mut goals = column![heading("Goals", metrics)].spacing(metrics.spacing);
    for goal in &area.goals {
        let mut words = column![
            text(&goal.name)
                .size(metrics.label() + 1.0)
                .wrapping(text::Wrapping::WordOrGlyph)
        ]
        .spacing(2)
        .width(Fill);
        if !goal.description.trim().is_empty() {
            words = words.push(
                text(&goal.description)
                    .size(metrics.label() - 1.0)
                    .style(text::secondary)
                    .wrapping(text::Wrapping::WordOrGlyph),
            );
        }
        let header = row![color_circle(area.color().unwrap_or(NO_COLOR)), words]
            .spacing(8)
            .align_y(Alignment::Center);
        goals = goals.push(
            container(
                column![
                    header,
                    objective_list(state, area, open, Some(goal.id), metrics)
                ]
                .spacing(metrics.spacing),
            )
            .padding(10)
            .width(Fill)
            .style(super::controls::row_surface),
        );
    }
    if area.goals.is_empty() {
        goals = goals.push(label("No goals yet. Add them under Vision.", metrics));
    }

    let unassigned_objectives = column![
        heading("Unassigned objectives", metrics),
        objective_list(state, area, open, None, metrics),
    ]
    .spacing(metrics.spacing);

    column![
        goals,
        gap(),
        unassigned_objectives,
        gap(),
        unassigned_habits(state, area, metrics),
        gap(),
        project_list(state, area, open, metrics),
    ]
    .spacing(metrics.spacing)
    .width(Fill)
    .into()
}

/// The objectives under `goal` (or with none), and a box to add another.
fn objective_list<'a>(
    state: &'a State,
    area: &'a LifeArea,
    open: &'a Open,
    goal: Option<u64>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let size = metrics.label();
    let mut list = column![].spacing(metrics.spacing).width(Fill);
    let mut any = false;
    for objective in &area.objectives {
        if area.objective_goal(objective) != goal {
            continue;
        }
        any = true;
        list = list.push(objective_card(state, area, objective, metrics));
    }
    if !any {
        list = list.push(label(
            if goal.is_some() {
                "No objectives for this goal yet."
            } else {
                "No unassigned objectives."
            },
            metrics,
        ));
    }

    let typed = open
        .new_objective
        .get(&goal)
        .map(String::as_str)
        .unwrap_or_default();
    let input = crate::ui::controls::text_input("New objective", typed)
        .on_input(move |name| msg(Action::NewObjective(goal, name)))
        .on_submit(msg(Action::AddObjective(goal)))
        .size(size)
        .width(Fill);
    let add = button(text("Add objective").size(size).center())
        .on_press(msg(Action::AddObjective(goal)))
        .style(crate::ui::controls::secondary);
    list.push(row![input, add].spacing(6).align_y(Alignment::Center))
        .into()
}

fn objective_card<'a>(
    state: &'a State,
    area: &'a LifeArea,
    objective: &'a crate::config::Objective,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let size = metrics.label();
    let id = objective.id;

    let name = crate::ui::controls::text_input("Objective", &objective.name)
        .on_input(move |name| msg(Action::ObjectiveName(id, name)))
        .size(size)
        .width(Fill);
    let remove = button(text("Remove").size(size - 1.0))
        .on_press(msg(Action::RemoveObjective(id)))
        .style(crate::ui::controls::ghost);
    let description = crate::ui::controls::text_input("Short description", &objective.description)
        .on_input(move |description| msg(Action::ObjectiveDescription(id, description)))
        .size(size - 1.0)
        .width(Fill);

    let mut goal_options = vec![GoalChoice {
        id: None,
        name: "No goal".to_owned(),
    }];
    goal_options.extend(area.goals.iter().map(|goal| GoalChoice {
        id: Some(goal.id),
        name: goal.name.clone(),
    }));
    let current_goal = area.objective_goal(objective);
    let selected_goal = goal_options.iter().find(|c| c.id == current_goal).cloned();
    let goal_picker = crate::ui::controls::pick_list(goal_options, selected_goal, move |to| {
        msg(Action::ObjectiveGoal(id, to.id))
    })
    .text_size(size - 1.0);

    let mut habits = column![].spacing(4).width(Fill);
    for habit_id in &objective.habits {
        let Some(habit) = state.habits.library.habit(habit_id) else {
            continue;
        };
        let mut caption = habit.name.clone();
        if habit.archived {
            caption.push_str(" · Archived");
        }
        let habit_id = habit_id.clone();
        habits = habits.push(
            row![
                habit_dot(habit.color),
                text(caption)
                    .size(size - 1.0)
                    .wrapping(text::Wrapping::WordOrGlyph)
                    .width(Fill),
                button(text("×").size(size))
                    .padding([0, 6])
                    .on_press(msg(Action::RemoveHabit(id, habit_id)))
                    .style(crate::ui::controls::ghost),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
    }

    // Any existing habit not already here can be added; it moves out of
    // whatever objective it served before.
    let habit_options: Vec<HabitChoice> = state
        .habits
        .library
        .habits
        .iter()
        .filter(|habit| !habit.archived && !objective.habits.contains(&habit.id))
        .map(|habit| HabitChoice {
            id: habit.id.clone(),
            name: habit.name.clone(),
        })
        .collect();
    let add_habit: Element<'a, Message> = if habit_options.is_empty() {
        label(
            if state.habits.library.habits.is_empty() {
                "No habits yet. Create them in Habits."
            } else {
                "Every habit is already here."
            },
            metrics,
        )
        .into()
    } else {
        crate::ui::controls::pick_list(habit_options, None::<HabitChoice>, move |habit| {
            msg(Action::AddHabit(id, habit.id))
        })
        .placeholder("Add habit…")
        .text_size(size - 1.0)
        .into()
    };

    container(
        column![
            row![name, remove].spacing(6).align_y(Alignment::Center),
            description,
            row![label("Goal", metrics), goal_picker]
                .spacing(6)
                .align_y(Alignment::Center),
            label("Habits", metrics),
            habits,
            add_habit,
        ]
        .spacing(4),
    )
    .padding(8)
    .width(Fill)
    .style(super::controls::row_surface)
    .into()
}

/// Habits in this area that no objective of it is carried out by.
fn unassigned_habits<'a>(
    state: &'a State,
    area: &'a LifeArea,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let size = metrics.label();
    let projects = &state.config.projects;
    let options = choices(projects);
    let current = options.iter().find(|choice| choice.id == area.id).cloned();

    let mut habits = column![heading("Unassigned habits", metrics)].spacing(6);
    let mut any = false;
    for habit in &state.habits.library.habits {
        if projects.resolve_area(habit.area) != area.id || area.has_habit(&habit.id) {
            continue;
        }
        any = true;
        let id = habit.id.clone();
        let picker = crate::ui::controls::pick_list(options.clone(), current.clone(), move |to| {
            msg(Action::MoveHabit(id.clone(), to.id))
        })
        .text_size(size - 1.0);
        let goal = habit.goal_label();
        let detail = if habit.archived {
            format!("{goal} · Archived")
        } else {
            goal
        };
        habits = habits.push(
            container(
                row![
                    habit_dot(habit.color),
                    column![
                        text(&habit.name)
                            .size(size)
                            .wrapping(text::Wrapping::WordOrGlyph),
                        text(detail)
                            .size(size - 1.0)
                            .style(text::secondary)
                            .wrapping(text::Wrapping::WordOrGlyph),
                    ]
                    .spacing(2)
                    .width(Fill),
                    picker,
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding([6, 8])
            .width(Fill)
            .style(super::controls::row_surface),
        );
    }
    if !any {
        habits = habits.push(label("No unassigned habits in this area.", metrics));
    }
    habits.into()
}

/// The area's projects, each opening to edit its name, color, directory and
/// area, and a box to add another.
fn project_list<'a>(
    state: &'a State,
    area: &'a LifeArea,
    open: &'a Open,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let size = metrics.label();
    let projects = &state.config.projects;
    let options = choices(projects);
    let current = options.iter().find(|choice| choice.id == area.id).cloned();

    let mut list = column![heading("Projects", metrics)].spacing(6);
    let mut any = false;
    for project in projects.sorted() {
        if projects.resolve_area(project.area) != area.id {
            continue;
        }
        any = true;
        let id = project.id;
        let editing = open.project.as_ref().filter(|edit| edit.id == id);

        let mut words = column![
            text(&project.name)
                .size(size)
                .wrapping(text::Wrapping::WordOrGlyph)
        ]
        .spacing(2)
        .width(Fill);
        if let Some(directory) = &project.directory {
            words = words.push(
                text(directory.display().to_string())
                    .size(size - 1.0)
                    .style(text::secondary)
                    .wrapping(text::Wrapping::WordOrGlyph),
            );
        }
        let summary = button(
            row![
                color_circle(project.color().unwrap_or(NO_COLOR)),
                words,
                text(if editing.is_some() { "⌄" } else { "›" }).size(size + 2.0),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .on_press(msg(Action::EditProject(id)))
        .padding([6, 8])
        .width(Fill)
        .style(super::controls::secondary);

        let Some(edit) = editing else {
            list = list.push(summary);
            continue;
        };

        let picker = crate::ui::controls::pick_list(options.clone(), current.clone(), move |to| {
            msg(Action::MoveProject(id, to.id))
        })
        .text_size(size - 1.0);
        list = list.push(
            container(
                column![
                    summary,
                    project_editor(project, edit, picker.into(), metrics)
                ]
                .spacing(8),
            )
            .padding(4)
            .width(Fill)
            .style(super::controls::row_surface),
        );
    }
    if !any {
        list = list.push(label("No projects in this area.", metrics));
    }

    let input =
        crate::ui::controls::text_input("New project, e.g. School/Physics", &open.new_project)
            .on_input(|name| msg(Action::NewProject(name)))
            .on_submit(msg(Action::AddProject))
            .size(size)
            .width(Fill);
    let add = button(text("Add project").size(size).center())
        .on_press(msg(Action::AddProject))
        .style(crate::ui::controls::secondary);
    list = list.push(row![input, add].spacing(6).align_y(Alignment::Center));
    if let Some(error) = &open.new_project_error {
        list = list.push(text(error).size(size).style(text::danger));
    }
    list.into()
}

fn project_editor<'a>(
    project: &'a crate::config::Project,
    edit: &'a ProjectEdit,
    area_picker: Element<'a, Message>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let size = metrics.label();
    let name = crate::ui::controls::text_input("Project name", &edit.name)
        .on_input(|name| msg(Action::ProjectName(name)))
        .size(size)
        .width(Fill);
    let directory =
        crate::ui::controls::text_input("Directory, e.g. ~/Projects/site", &edit.directory)
            .on_input(|directory| msg(Action::ProjectDirectory(directory)))
            .size(size)
            .width(Fill);

    let current = project.color();
    let mut swatches = column![].spacing(6);
    for chunk in colors::palette().chunks(if metrics.size.width < 300.0 { 3 } else { 6 }) {
        let mut line = row![].spacing(6);
        for &color in chunk {
            line = line.push(swatch_button(
                color,
                current == Some(color),
                msg(Action::ProjectColor(Some(color))),
            ));
        }
        swatches = swatches.push(line);
    }
    let no_color = button(text("No color").size(size - 1.0).center())
        .on_press(msg(Action::ProjectColor(None)))
        .style(if current.is_none() {
            crate::ui::controls::primary
        } else {
            crate::ui::controls::secondary
        });

    let mut content = column![label("Name", metrics), name].spacing(4).width(Fill);
    if let Some(error) = &edit.name_error {
        content = content.push(text(error).size(size).style(text::danger));
    }
    content = content
        .push(label("Color", metrics))
        .push(swatches)
        .push(no_color)
        .push(label("Directory", metrics))
        .push(directory)
        .push(label("Life area", metrics))
        .push(area_picker);

    content = if edit.confirm_remove {
        content.push(
            column![
                text("Remove this project? Tasks filed under it lose their project.").size(size),
                row![
                    button(text("Remove").size(size))
                        .on_press(msg(Action::ConfirmRemoveProject))
                        .style(crate::ui::controls::danger),
                    button(text("Keep").size(size))
                        .on_press(msg(Action::CancelRemoveProject))
                        .style(super::controls::secondary),
                ]
                .spacing(6),
            ]
            .spacing(4),
        )
    } else {
        content.push(
            button(text("Remove project").size(size))
                .on_press(msg(Action::RemoveProject))
                .style(crate::ui::controls::danger),
        )
    };

    container(content).padding([4, 8]).width(Fill).into()
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
