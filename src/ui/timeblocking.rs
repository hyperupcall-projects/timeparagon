use super::layout::Metrics;
use crate::{
    app::{Message, State},
    calendar::Event,
    timeblocking::{self as tb, Action},
};
use iced::widget::{
    button, column, container, mouse_area, rich_text, row, scrollable, span, text, text_input,
    toggler,
};
use iced::{Color, Element, Fill, Length};
fn msg(a: Action) -> Message {
    Message::TimeBlocking(a)
}
pub fn task_row<'a>(
    state: &'a State,
    task: &'a Event,
    metrics: &Metrics,
    backlog: bool,
) -> Element<'a, Message> {
    let m = tb::metadata(task);
    let overdue = tb::overdue(task);
    let over_capacity = m
        .block
        .as_ref()
        .and_then(|key| {
            state
                .calendar
                .events
                .iter()
                .find(|e| !e.is_task() && tb::block_key(e) == *key)
        })
        .is_some_and(|block| {
            let tasks = tb::assigned(state, block);
            let mut total = 0u64;
            for item in tasks {
                total += u64::from(tb::metadata(item).minutes);
                if item.id == task.id {
                    break;
                }
            }
            total > (block.end - block.start).num_minutes().max(0) as u64
        });
    let busy = state.timeblocking.busy.contains(&task.id);
    let mut check = button(text(if task.completed { "●" } else { "○" })).style(button::text);
    if state.timeblocking.busy.is_empty() {
        check = check.on_press(Message::TaskToggled(task.id.clone()));
    }
    let mut title_content = column![task_title(task, metrics.label())].spacing(2);
    let status = if busy {
        Some("Saving…")
    } else {
        match tb::task_status(task) {
            tb::TaskStatus::Completed => Some("Completed ✓"),
            tb::TaskStatus::Ignored => Some("Ignored"),
            tb::TaskStatus::Active => None,
        }
    };
    if let Some(status) = status {
        title_content = title_content.push(
            text(status)
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }
    let title = button(title_content)
        .width(Fill)
        .style(button::text)
        .on_press(Message::EventOpened(task.id.clone()));
    let handle = text("⠿").size(17);
    let mut line = row![
        handle,
        check,
        title,
        estimate_button(state, task, metrics.label())
    ]
    .spacing(4)
    .align_y(iced::Center);
    if backlog {
        line = line.push(
            button(text("Reschedule").size(metrics.label()))
                .on_press(Message::EventOpened(task.id.clone()))
                .style(button::text),
        );
    }
    let mut content = column![line].spacing(3);
    if backlog {
        content = content.push(
            text(tb::task_list_name(state, &task.calendar_id))
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }
    if let Ok(Some(deadline)) = crate::calendar::deadline::read_deadline(&task.details.description)
    {
        content = content.push(
            text(deadline.to_string())
                .size(metrics.label() - 1.0)
                .style(if overdue {
                    text::danger
                } else {
                    text::secondary
                }),
        );
    }
    let card = container(content)
        .padding([5, 6])
        .width(Fill)
        .style(move |theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(
                    if over_capacity {
                        palette.danger.weak.color
                    } else {
                        palette.background.base.color
                    }
                    .into(),
                ),
                border: iced::Border {
                    color: if overdue {
                        Color::from_rgb8(220, 65, 55)
                    } else {
                        palette.background.strong.color
                    },
                    width: 1.0,
                    radius: 5.0.into(),
                },
                ..Default::default()
            }
        });
    super::task_drag::task_drag(card, &task.id, state.timeblocking.dragging.is_some())
}

pub(super) fn task_title<'a>(task: &'a Event, size: f32) -> Element<'a, Message> {
    if tb::task_status(task) == tb::TaskStatus::Ignored {
        rich_text::<(), _, _, _>([span(task.title.as_str()).strikethrough(true)])
            .size(size)
            .width(Fill)
            .style(text::secondary)
            .into()
    } else {
        text(&task.title)
            .size(size)
            .width(Fill)
            .style(if task.completed {
                text::secondary
            } else {
                text::default
            })
            .into()
    }
}
pub fn editor<'a>(state: &'a State, task: &'a Event, metrics: &Metrics) -> Element<'a, Message> {
    let busy = state.timeblocking.busy.contains(&task.id);
    let mut name = text_input("Task name", &state.timeblocking.title).size(metrics.label());
    let mut rename = button(text(if busy { "Renaming…" } else { "Rename" }).size(metrics.label()));
    let mut save_deadline = button(text("Save deadline").size(metrics.label()));
    let mut schedule = button(text("Schedule time").size(metrics.label()));
    let mut day = button(text("Set day only").size(metrics.label())).style(button::text);
    if !busy {
        let id = task.id.clone();
        name = name
            .on_input(|value| msg(Action::Title(value)))
            .on_submit(msg(Action::SaveTitle(id.clone())));
        rename = rename.on_press(msg(Action::SaveTitle(id)));
        save_deadline = save_deadline.on_press(msg(Action::SaveDeadline(task.id.clone())));
        schedule = schedule.on_press(msg(Action::ScheduleEntered(task.id.clone())));
        day = day.on_press(msg(Action::Reschedule(task.id.clone())));
    }
    let mut body = column![
        text("Name").size(metrics.label()).style(text::secondary),
        name,
        rename,
    ]
    .spacing(8);
    if let Some(error) = &state.timeblocking.title_error {
        body = body.push(text(error).size(metrics.label()).style(text::danger));
    }
    body = body.push(estimate_button(state, task, metrics.label()));
    body = body.push(
        text("Deadline")
            .size(metrics.label())
            .style(text::secondary),
    );
    body = body.push(
        text_input(
            "YYYY-MM-DD or YYYY-MM-DD HH:MM (blank clears deadline)",
            &state.timeblocking.deadline,
        )
        .on_input(|v| msg(Action::Deadline(v)))
        .size(metrics.label()),
    );
    body = body.push(save_deadline);
    body = body.push(
        text("Scheduled day and time")
            .size(metrics.label())
            .style(text::secondary),
    );
    body = body.push(
        text_input("YYYY-MM-DD", &state.timeblocking.date)
            .on_input(|v| msg(Action::Date(v)))
            .size(metrics.label()),
    );
    body = body.push(
        row![
            text_input("Start HH:MM", &state.timeblocking.start_time)
                .on_input(|v| msg(Action::StartTime(v)))
                .size(metrics.label()),
            text_input("End HH:MM", &state.timeblocking.end_time)
                .on_input(|v| msg(Action::EndTime(v)))
                .size(metrics.label())
        ]
        .spacing(6),
    );
    body = body.push(schedule).push(day);
    if let Err(error) = crate::calendar::deadline::check(&task.details.description) {
        body = body.push(text(error).size(metrics.label()).style(text::danger));
    }
    body.into()
}

/// Keep task actions in the same order in details and the backlog editor.
pub fn task_actions<'a>(state: &State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
    let status = tb::task_status(task);
    let mut actions = column![].spacing(metrics.spacing).width(Fill);
    for (label, action) in [
        (
            if status == tb::TaskStatus::Completed {
                "Uncomplete task"
            } else {
                "Complete task"
            },
            Message::TaskToggled(task.id.clone()),
        ),
        (
            if status == tb::TaskStatus::Ignored {
                "Unignore Task"
            } else {
                "Ignore Task"
            },
            Message::TaskIgnoreToggled(task.id.clone()),
        ),
        ("Move to backlog", msg(Action::Backlog(task.id.clone()))),
        ("Delete task", Message::TaskDeleteRequested(task.id.clone())),
    ] {
        let mut action_button = button(text(label).size(metrics.label()).center().width(Fill))
            .width(Fill)
            .style(if matches!(&action, Message::TaskToggled(_)) {
                button::primary
            } else {
                button::secondary
            });
        if state.timeblocking.busy.is_empty() {
            action_button = action_button.on_press(action);
        }
        actions = actions.push(action_button);
    }
    actions.into()
}

pub fn panel<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(block) = state
        .timeblocking
        .block
        .as_deref()
        .and_then(|id| state.calendar.event(id))
    else {
        return column![
            text("Time block unavailable"),
            button("Close").on_press(msg(Action::Close))
        ]
        .padding(16)
        .into();
    };
    let tasks = tb::assigned(state, block);
    let capacity = (block.end - block.start).num_minutes().max(0) as u32;
    let block_id = block.id.clone();
    let mut body = column![
        row![
            text(block.title.clone()).size(20).width(Fill),
            button("×").on_press(msg(Action::Close)).style(button::text)
        ],
        button("Edit time block").on_press(Message::EventEditRequested(block_id.clone())),
        button("Show Event Details").on_press(msg(Action::Details(block_id.clone()))),
        text("Planned Tasks").size(16),
        planned_slot(state, &block_id, metrics),
    ]
    .spacing(10);
    let mut elapsed = 0;
    for task in tasks {
        elapsed += tb::metadata(task).minutes;
        let mut remove = button("−");
        if state.timeblocking.busy.is_empty() {
            remove = remove.on_press(msg(Action::Unassign(task.id.clone())));
        }
        let card: Element<'a, Message> = row![task_row(state, task, metrics, false), remove].into();
        body = body.push(if elapsed > capacity {
            container(card)
                .style(|_| container::Style {
                    background: Some(Color::from_rgba(0.9, 0.2, 0.15, 0.22).into()),
                    ..Default::default()
                })
                .into()
        } else {
            card
        });
    }
    body = body.push(text("Available tasks").size(16));
    body = body.push(
        row![
            text("All tasks").size(metrics.label()).width(Fill),
            toggler(state.timeblocking.available_all)
                .on_toggle(|all| msg(Action::AvailableAll(all))),
        ]
        .spacing(8)
        .align_y(iced::Center),
    );
    body = body.push(
        row![
            text("Show planned tasks").size(metrics.label()).width(Fill),
            toggler(state.timeblocking.available_planned)
                .on_toggle(|show| msg(Action::AvailablePlanned(show))),
        ]
        .spacing(8)
        .align_y(iced::Center),
    );
    body = body.push(
        text_input(
            "Search available tasks…",
            &state.timeblocking.available_query,
        )
        .on_input(|value| msg(Action::AvailableQuery(value)))
        .size(metrics.label())
        .width(Fill),
    );
    let query = state.timeblocking.available_query.trim().to_lowercase();
    let all_tasks = state.timeblocking.available_all;
    let available: Vec<_> = state
        .calendar
        .events
        .iter()
        .filter(|event| event.is_task() && !event.completed && tb::metadata(event).block.is_none())
        .filter(|event| {
            state.timeblocking.available_planned
                || (event.all_day
                    && tb::in_backlog(event)
                    && !matches!(
                        crate::calendar::deadline::read_due_time(&event.details.description),
                        Ok(Some(_))
                    ))
        })
        .filter(|event| query.is_empty() || event.title.to_lowercase().contains(&query))
        .filter(|event| all_tasks || tb::on_today(event))
        .collect();
    if available.is_empty() && !query.is_empty() {
        body = body.push(text("No matching tasks").style(text::secondary));
    } else if available.is_empty() && !all_tasks {
        body = body.push(text("No tasks for today").style(text::secondary));
    }
    for task in available {
        body = body.push(row![
            task_row(state, task, metrics, false),
            button("+").on_press(msg(Action::Assign(task.id.clone(), block_id.clone())))
        ]);
    }
    // A release anywhere on the panel assigns the dragged task, including on the
    // plus box. A release on a task row is captured first and reorders instead.
    mouse_area(
        container(scrollable(body))
            .padding(16)
            .width(Fill)
            .height(Fill),
    )
    .on_release(msg(Action::DropBlock(block_id)))
    .into()
}

/// The planned-tasks box. The plus sits in the middle until it is clicked,
/// which opens a field that creates a Google Task in this block. Dropping a
/// task on the box assigns it without opening the field.
fn planned_slot<'a>(state: &'a State, block_id: &str, metrics: &Metrics) -> Element<'a, Message> {
    let adding = state.timeblocking.busy.contains(tb::ADDING_TODO);
    let open = state.timeblocking.todo_open || adding || state.timeblocking.todo_error.is_some();
    let dragging = state.timeblocking.dragging.is_some();
    let plus = container(text("+").size(28)).center_x(Fill);
    if !open {
        return button(plus.center_y(Length::Fixed(72.0)))
            .padding(0)
            .width(Fill)
            .on_press(msg(Action::OpenTodo))
            .style(move |theme: &iced::Theme, status| add_slot_style(theme, status, dragging))
            .into();
    }
    container(column![plus, todo_editor(state, block_id, metrics)].spacing(8))
        .padding(10)
        .width(Fill)
        .style(move |theme: &iced::Theme| add_slot_container(theme, dragging))
        .into()
}

/// A free-text line that becomes a Google Task in this block.
fn todo_editor<'a>(state: &'a State, block_id: &str, metrics: &Metrics) -> Element<'a, Message> {
    let adding = state.timeblocking.busy.contains(tb::ADDING_TODO);
    let ready =
        state.timeblocking.busy.is_empty() && !state.timeblocking.todo_draft.trim().is_empty();
    let mut input = text_input("Write a task", &state.timeblocking.todo_draft)
        .id(iced::widget::Id::new("time-block-todo"))
        .size(metrics.label())
        .width(Fill);
    let mut add = button(text(if adding { "Adding…" } else { "Add task" }).size(metrics.label()));
    let mut cancel = button(text("Cancel").size(metrics.label())).style(button::text);
    if !adding {
        input = input.on_input(|value| msg(Action::TodoDraft(value)));
        cancel = cancel.on_press(msg(Action::CloseTodo));
    }
    if ready {
        let id = block_id.to_owned();
        input = input.on_submit(msg(Action::AddTodo(id.clone())));
        add = add.on_press(msg(Action::AddTodo(id)));
    }
    let mut body = column![input, row![add, cancel].spacing(8)].spacing(6);
    if let Some(error) = &state.timeblocking.todo_error {
        body = body.push(text(error).size(metrics.label()).style(text::danger));
    }
    body.into()
}

fn add_slot_style(theme: &iced::Theme, status: button::Status, dragging: bool) -> button::Style {
    let palette = theme.extended_palette();
    let hot = dragging || matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(
            if hot {
                palette.primary.weak.color
            } else {
                palette.background.base.color
            }
            .into(),
        ),
        text_color: palette.primary.base.color,
        border: iced::Border {
            color: if hot {
                palette.primary.strong.color
            } else {
                palette.background.strong.color
            },
            width: if dragging { 2.0 } else { 1.0 },
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}

fn add_slot_container(theme: &iced::Theme, dragging: bool) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(
            if dragging {
                palette.primary.weak.color
            } else {
                palette.background.base.color
            }
            .into(),
        ),
        border: iced::Border {
            color: if dragging {
                palette.primary.strong.color
            } else {
                palette.background.strong.color
            },
            width: if dragging { 2.0 } else { 1.0 },
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}
pub fn backlog<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut tasks: Vec<_> = state
        .calendar
        .events
        .iter()
        .filter(|event| tb::in_backlog(event))
        .collect();
    tasks.sort_by_key(|event| tb::metadata(event).order);
    let mut lists = vec![ListFilter {
        id: None,
        name: "All lists".into(),
    }];
    for task in &tasks {
        if lists
            .iter()
            .any(|item| item.id.as_deref() == Some(task.calendar_id.as_str()))
        {
            continue;
        }
        lists.push(ListFilter {
            id: Some(task.calendar_id.clone()),
            name: tb::task_list_name(state, &task.calendar_id),
        });
    }
    if let Some(id) = &state.timeblocking.backlog_list {
        if !lists.iter().any(|item| item.id.as_deref() == Some(id)) {
            lists.push(ListFilter {
                id: Some(id.clone()),
                name: tb::task_list_name(state, id),
            });
        }
    }
    let query = state.timeblocking.backlog_query.trim().to_lowercase();
    let list = state.timeblocking.backlog_list.as_deref();
    let shown: Vec<_> = tasks
        .iter()
        .copied()
        .filter(|task| {
            list.is_none_or(|id| task.calendar_id == id)
                && (query.is_empty()
                    || task.title.to_lowercase().contains(&query)
                    || tb::task_list_name(state, &task.calendar_id)
                        .to_lowercase()
                        .contains(&query))
        })
        .collect();
    let mut body = column![
        text("Backlog").size(25),
        text("Unscheduled Google Tasks")
            .size(13)
            .style(text::secondary),
        text("Search").size(metrics.label()).style(text::secondary),
        text_input("Search tasks or lists…", &state.timeblocking.backlog_query)
            .on_input(|value| msg(Action::BacklogQuery(value)))
            .size(metrics.label())
            .width(Fill),
        text("Task list")
            .size(metrics.label())
            .style(text::secondary),
        list_filters(lists, state.timeblocking.backlog_list.as_deref(), metrics),
    ]
    .spacing(8);
    if let Some(error) = &state.calendar.error {
        body = body.push(text(error).style(text::danger));
    }
    if tasks.is_empty() {
        body = body.push(text("No tasks in your backlog").style(text::secondary));
    } else if shown.is_empty() {
        body = body.push(text("No matching tasks").style(text::secondary));
    }
    for task in shown {
        body = body.push(task_row(state, task, metrics, true));
    }
    let content: Element<'a, Message> = container(scrollable(body))
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill)
        .into();
    content
}

pub use super::timeblocking_focus::view as focus_overlay;

fn list_filters<'a>(
    lists: Vec<ListFilter>,
    selected: Option<&str>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let mut filters = column![].spacing(4);
    for item in lists {
        let active = item.id.as_deref() == selected;
        filters = filters.push(
            button(text(item.name).size(metrics.label()))
                .width(Fill)
                .on_press(msg(Action::BacklogList(item.id)))
                .style(if active {
                    button::primary
                } else {
                    button::secondary
                }),
        );
    }
    filters.into()
}

struct ListFilter {
    id: Option<String>,
    name: String,
}

/// Every estimate entry point opens the same app-wide dialog.
pub(super) fn estimate_button<'a>(
    state: &State,
    task: &Event,
    label_size: f32,
) -> Element<'a, Message> {
    let minutes = tb::metadata(task).minutes;
    let mut control =
        button(text(format!("Estimate: {minutes}m")).size(label_size)).style(button::text);
    if !state.timeblocking.busy.contains(&task.id) {
        control = control.on_press(msg(Action::EditEstimate(task.id.clone())));
    }
    control.into()
}

pub fn estimate_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    use iced::widget::{Space, stack};
    let task = state
        .timeblocking
        .estimate_task
        .as_deref()
        .and_then(|id| state.calendar.event(id));
    let mut body = column![text("Estimated completion time").size(18)].spacing(10);
    if let Some(task) = task {
        let busy = state.timeblocking.busy.contains(&task.id);
        let mut input = text_input("Minutes", &state.timeblocking.estimate)
            .id(iced::widget::Id::new("task-estimate"))
            .size(metrics.label());
        let mut save = button(text(if busy { "Saving…" } else { "Save" }));
        if !busy {
            input = input
                .on_input(|value| msg(Action::Estimate(value)))
                .on_submit(msg(Action::SaveEstimate(task.id.clone())));
            save = save.on_press(msg(Action::SaveEstimate(task.id.clone())));
        }
        body = body
            .push(text(task.title.clone()).size(metrics.label()))
            .push(
                text("Estimate in minutes (0 means no estimate)")
                    .size(metrics.label())
                    .style(text::secondary),
            )
            .push(input);
        if let Some(error) = &state.timeblocking.estimate_error {
            body = body.push(text(error).size(metrics.label()).style(text::danger));
        }
        body = body.push(
            row![
                save,
                button("Cancel")
                    .on_press(msg(Action::CloseEstimate))
                    .style(button::text)
            ]
            .spacing(8),
        );
    } else {
        body = body
            .push(text("This task is no longer available"))
            .push(button("Close").on_press(msg(Action::CloseEstimate)));
    }
    let card = container(scrollable(body))
        .padding(16)
        .width(Fill)
        .max_width(420)
        .max_height((metrics.size.height - 32.0).max(80.0))
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                text_color: Some(palette.background.base.text),
                border: iced::Border {
                    radius: 10.0.into(),
                    ..Default::default()
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
                    background: Some(Color::BLACK.scale_alpha(0.58).into()),
                    ..Default::default()
                }),
        )
        .on_press(msg(Action::CloseEstimate))
        .on_scroll(|_| Message::Ignored);
    let foreground = container(
        mouse_area(card)
            .on_press(Message::Ignored)
            .on_scroll(|_| Message::Ignored),
    )
    .padding(16)
    .center_x(Fill)
    .center_y(Fill);
    stack![backdrop, foreground].into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::{Point, Size, mouse};

    #[test]
    fn estimate_popup_covers_the_time_block_view_at_narrow_and_wide_widths() {
        for width in [200.0, 800.0] {
            let mut state = State::new(
                crate::config::Config::default(),
                crate::theme::Catalog::built_in(),
            );
            state.calendar =
                crate::calendar::Store::with_snapshot(crate::calendar::sample_snapshot());
            let task = state.calendar.event_mut("deep").unwrap();
            task.id = "task:deep".into();
            task.calendar_id = "tasks:list".into();
            state.timeblocking.focus = Some("standup".into());
            let _ = tb::update(&mut state, Action::EditEstimate("task:deep".into()));
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                Size::new(width, 600.0),
                super::super::view(&state),
            );
            assert!(ui.find("Save").is_ok());
            assert!(ui.find("Cancel").is_ok());
            ui.point_at(Point::new(2.0, 2.0));
            ui.simulate([
                iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ]);
            let messages: Vec<_> = ui.into_messages().collect();
            assert!(
                messages
                    .iter()
                    .any(|m| matches!(m, Message::TimeBlocking(Action::CloseEstimate)))
            );
            assert!(
                !messages
                    .iter()
                    .any(|m| matches!(m, Message::TimeBlocking(Action::CloseView)))
            );
        }
    }
}
