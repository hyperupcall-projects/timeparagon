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
    let mut line = row![handle, check].spacing(4).align_y(iced::Center);
    // Where the task came from, if a service we know put it here.
    if let Some(source) = crate::integrations::source_in(&task.details.description) {
        line = line.push(super::source_icon::link(&source, 14.0));
    }
    line = line
        .push(title)
        .push(estimate_button(state, task, metrics.label()));
    if backlog {
        line = line.push(
            button(text("Reschedule").size(metrics.label()))
                .on_press(Message::EventOpened(task.id.clone()))
                .style(button::text),
        );
    }
    let mut content = column![line].spacing(3);
    if !m.labels.is_empty() {
        content = content.push(
            text(
                m.labels
                    .iter()
                    .map(|label| format!("#{label}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            )
            .size(metrics.label() - 2.0)
            .style(text::secondary),
        );
    }
    if backlog {
        content = content.push(
            text(tb::task_list_name(state, &task.calendar_id))
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }
    if backlog {
        if let Some(due) = tb::due(task) {
            let late = due.is_past(chrono::Local::now().naive_local());
            content = content.push(
                text(due_label(&due))
                    .size(metrics.label() - 1.0)
                    .style(if late { text::danger } else { text::secondary }),
            );
        }
    } else if let Ok(Some(deadline)) =
        crate::calendar::deadline::read_deadline(&task.details.description)
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
    super::task_drag::task_drag(card, &task.id, state.timeblocking.dragging.as_deref())
}

/// The floating card that follows the pointer while a task is in the air.
///
/// It is drawn above everything else but never takes the pointer, so the drop
/// targets underneath still see the release. The border uses the theme's
/// primary color to echo the highlight on the places it can be dropped.
pub(super) fn drag_ghost<'a>(
    state: &'a State,
    task: &'a Event,
    metrics: &Metrics,
    width: f32,
) -> Element<'a, Message> {
    let m = tb::metadata(task);
    let mut body = column![
        row![
            text("⠿").size(metrics.label() + 2.0).style(text::secondary),
            task_title(task, metrics.label()),
        ]
        .spacing(6)
        .align_y(iced::Center),
        text(tb::task_list_name(state, &task.calendar_id))
            .size(metrics.label() - 1.0)
            .style(text::secondary),
    ]
    .spacing(3);
    if m.minutes > 0 {
        body = body.push(
            text(format!("Estimate: {}m", m.minutes))
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }
    container(body)
        .padding([6, 8])
        .width(Length::Fixed(width))
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                text_color: Some(palette.background.base.text),
                border: iced::Border {
                    color: palette.primary.base.color,
                    width: 2.0,
                    radius: 6.0.into(),
                },
                shadow: iced::Shadow {
                    color: Color::BLACK.scale_alpha(0.32),
                    offset: iced::Vector::new(0.0, 4.0),
                    blur_radius: 12.0,
                },
                ..container::Style::default()
            }
        })
        .into()
}

/// "Due Today", "Due Tomorrow", or "Due Mon, Oct 26" (with the year when it
/// isn't this one), followed by the time when the deadline has one.
fn due_label(due: &crate::calendar::deadline::Deadline) -> String {
    let today = chrono::Local::now().date_naive();
    let day = if due.date == today {
        "Today".to_owned()
    } else if today.succ_opt() == Some(due.date) {
        "Tomorrow".to_owned()
    } else if chrono::Datelike::year(&due.date) == chrono::Datelike::year(&today) {
        due.date.format("%a, %b %-d").to_string()
    } else {
        due.date.format("%a, %b %-d, %Y").to_string()
    };
    match due.time {
        Some(time) => format!("Due {day} at {}", time.format("%H:%M")),
        None => format!("Due {day}"),
    }
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
/// The title occupies its own field only while it is being edited.
pub(super) fn editable_title<'a>(
    state: &'a State,
    task: &'a Event,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let busy = state.timeblocking.busy.contains(&task.id);
    let size = metrics.label() + 6.0;
    let title: Element<'a, Message> = if state.timeblocking.editing_title {
        let mut input = text_input("Task name", &state.timeblocking.title)
            .id(iced::widget::Id::new("task-title"))
            .size(size);
        if !busy {
            input = input
                .on_input(|v| msg(Action::Title(v)))
                .on_submit(msg(Action::SaveTitle(task.id.clone())));
        }
        input.into()
    } else {
        let mut title = button(text(&task.title).size(size).font(iced::Font {
            weight: iced::font::Weight::Bold,
            ..iced::Font::DEFAULT
        }))
        .padding(0)
        .width(Fill)
        .style(button::text);
        if !busy {
            title = title.on_press(msg(Action::EditTitle));
        }
        title.into()
    };
    let mut body = column![title].spacing(4);
    if state.timeblocking.editing_title {
        body = body.push(
            text("Press Enter to save")
                .size(metrics.label() - 2.0)
                .style(text::secondary),
        );
    }
    if let Some(error) = &state.timeblocking.title_error {
        body = body.push(text(error).size(metrics.label()).style(text::danger));
    }
    body.into()
}

pub fn editor<'a>(state: &'a State, task: &'a Event, metrics: &Metrics) -> Element<'a, Message> {
    let busy = state.timeblocking.busy.contains(&task.id);
    let mut estimate =
        button(text(format!("{}m", tb::metadata(task).minutes)).size(metrics.label()))
            .style(super::controls::secondary)
            .width(Fill);
    if !busy {
        estimate = estimate.on_press(msg(Action::EditEstimate(task.id.clone())));
    }
    column![
        project_field(state, task, metrics),
        column![field_label("Estimate", metrics), estimate].spacing(6),
        date_field(state, task, metrics, false),
        date_field(state, task, metrics, true),
        labels_field(state, task, metrics),
        priority_field(state, task, metrics),
        location_field(state, task, metrics),
    ]
    .spacing(12)
    .into()
}

/// Due date and deadline share the same layout and optional date picker.
fn date_field<'a>(
    state: &'a State,
    task: &'a Event,
    metrics: &Metrics,
    deadline: bool,
) -> Element<'a, Message> {
    let busy = state.timeblocking.busy.contains(&task.id);
    let value = if deadline {
        &state.timeblocking.deadline
    } else {
        &state.timeblocking.date
    };
    let mut day = button(
        text("Schedule Day")
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill);
    let mut time = button(
        text("Schedule Time")
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill);
    let mut reset = button(
        text("Reset Time")
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill)
    .style(super::controls::secondary);
    if !busy {
        day = day.on_press(msg(if deadline {
            Action::DeadlineDay(task.id.clone())
        } else {
            Action::Reschedule(task.id.clone())
        }));
        time = time.on_press(msg(Action::EditScheduleTime(deadline)));
        reset = reset.on_press(msg(if deadline {
            Action::DeadlineDay(task.id.clone())
        } else {
            Action::Reschedule(task.id.clone())
        }));
    }
    let mut body = column![
        field_label(if deadline { "Deadline" } else { "Due Date" }, metrics),
        super::controls::date(value, true, false, !busy, metrics.label(), move |v| msg(
            if deadline {
                Action::Deadline(v)
            } else {
                Action::Date(v)
            }
        )),
        day,
        row![time, reset].spacing(8),
    ]
    .spacing(6)
    .width(Fill);
    let editing = if deadline {
        state.timeblocking.editing_deadline_time
    } else {
        state.timeblocking.editing_due_time
    };
    if editing {
        let times: Element<'a, Message> = if deadline {
            super::controls::time(
                &state.timeblocking.deadline_time,
                !busy,
                metrics.label(),
                |v| msg(Action::DeadlineTime(v)),
            )
        } else {
            row![
                container(super::controls::time(
                    &state.timeblocking.start_time,
                    !busy,
                    metrics.label(),
                    |v| msg(Action::StartTime(v))
                ))
                .width(Fill),
                container(super::controls::time(
                    &state.timeblocking.end_time,
                    !busy,
                    metrics.label(),
                    |v| msg(Action::EndTime(v))
                ))
                .width(Fill),
            ]
            .spacing(6)
            .into()
        };
        let mut save = button(text("Save time").size(metrics.label())).width(Fill);
        if !busy {
            save = save.on_press(msg(if deadline {
                Action::DeadlineWithTime(task.id.clone())
            } else {
                Action::ScheduleEntered(task.id.clone())
            }));
        }
        body = body.push(times).push(save);
    }
    if deadline {
        if let Err(error) = crate::calendar::deadline::check(&task.details.description) {
            body = body.push(text(error).size(metrics.label()).style(text::danger));
        }
    }
    body.into()
}

fn field_label<'a>(label: &'a str, metrics: &Metrics) -> Element<'a, Message> {
    text(label)
        .size(metrics.label())
        .font(iced::Font {
            weight: iced::font::Weight::Bold,
            ..iced::Font::DEFAULT
        })
        .style(text::secondary)
        .into()
}

/// The task's projects, plus a searchable picker for adding more.
fn project_field<'a>(state: &'a State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
    let projects = tb::metadata(task).projects;
    let busy = state.timeblocking.busy.contains(&task.id);

    let mut selected = column![].spacing(4).width(Fill);
    // A project that has since been removed is simply left out.
    for project in projects
        .iter()
        .filter_map(|id| state.config.projects.get(*id))
    {
        let mut remove = button(text("×").size(metrics.label()))
            .padding([0, 4])
            .style(button::text);
        if !busy {
            remove = remove.on_press(msg(Action::ProjectRemove(task.id.clone(), project.id)));
        }
        selected = selected.push(
            row![
                super::calendar::color_circle(
                    project
                        .color()
                        .unwrap_or(Color::from_rgb8(0x9a, 0x9a, 0x9e))
                ),
                text(&project.name).size(metrics.label()).width(Fill),
                remove,
            ]
            .spacing(6)
            .align_y(iced::Center),
        );
    }
    if projects.is_empty() {
        selected = selected.push(
            text("No projects")
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }

    let open = state.timeblocking.project_picker.as_deref() == Some(task.id.as_str());
    let mut toggle = button(
        text(if open { "Done" } else { "+ Add project" })
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill)
    .style(crate::ui::controls::secondary);
    if !busy {
        toggle = toggle.on_press(msg(Action::ProjectPicker(task.id.clone(), !open)));
    }

    let mut content = column![field_label("Projects", metrics), selected, toggle].spacing(6);

    if open {
        content = content.push(
            text_input("Search projects…", &state.timeblocking.project_query)
                .on_input(|query| msg(Action::ProjectQuery(query)))
                .size(metrics.label())
                .width(Fill),
        );

        let query = state.timeblocking.project_query.trim().to_lowercase();
        let available: Vec<_> = state
            .config
            .projects
            .sorted()
            .into_iter()
            .filter(|project| !projects.contains(&project.id))
            .filter(|project| query.is_empty() || project.name.to_lowercase().contains(&query))
            .collect();

        if available.is_empty() {
            content = content.push(
                text(if query.is_empty() {
                    "Every project is already added"
                } else {
                    "No matching projects"
                })
                .size(metrics.label() - 1.0)
                .style(text::secondary),
            );
        }

        for project in available {
            let mut add = button(text(&project.name).size(metrics.label()).width(Fill))
                .width(Fill)
                .style(button::text);
            if !busy {
                add = add.on_press(msg(Action::ProjectAdd(task.id.clone(), project.id)));
            }
            content = content.push(
                row![
                    super::calendar::color_circle(
                        project
                            .color()
                            .unwrap_or(Color::from_rgb8(0x9a, 0x9a, 0x9e))
                    ),
                    add,
                ]
                .spacing(6)
                .align_y(iced::Center),
            );
        }
    }

    content.into()
}

/// Free-form labels for the task: the ones it has, a field to add another, and
/// matching labels already used elsewhere as suggestions.
fn labels_field<'a>(state: &'a State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
    let labels = tb::metadata(task).labels;
    let busy = state.timeblocking.busy.contains(&task.id);

    let mut content = column![field_label("Labels", metrics)]
        .spacing(4)
        .width(Fill);

    for label in &labels {
        let mut remove = button(text("×").size(metrics.label()))
            .padding([0, 4])
            .style(button::text);
        if !busy {
            remove = remove.on_press(msg(Action::RemoveLabel(task.id.clone(), label.clone())));
        }
        content = content.push(
            row![
                text(label.clone()).size(metrics.label()).width(Fill),
                remove,
            ]
            .spacing(6)
            .align_y(iced::Center),
        );
    }

    let mut input = text_input("Add a label…", &state.timeblocking.label_draft)
        .size(metrics.label())
        .width(Fill);
    let mut add = button(text("Add").size(metrics.label())).style(crate::ui::controls::primary);
    if !busy {
        let typed = state.timeblocking.label_draft.clone();
        input = input
            .on_input(|value| msg(Action::LabelDraft(value)))
            .on_submit(msg(Action::AddLabel(task.id.clone(), typed.clone())));
        add = add.on_press(msg(Action::AddLabel(task.id.clone(), typed)));
    }
    content = content.push(row![input, add].spacing(6).align_y(iced::Center));

    if let Some(error) = &state.timeblocking.label_error {
        content = content.push(text(error).size(metrics.label()).style(text::danger));
    }

    // Labels already used on other tasks make spelling them again unnecessary.
    let query = state.timeblocking.label_draft.trim().to_lowercase();
    if !query.is_empty() {
        let mut seen: Vec<String> = Vec::new();
        for other in state.calendar.events.iter().filter(|event| event.is_task()) {
            for label in tb::metadata(other).labels {
                if labels
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(&label))
                    || !label.to_lowercase().contains(&query)
                    || seen
                        .iter()
                        .any(|existing: &String| existing.eq_ignore_ascii_case(&label))
                {
                    continue;
                }
                seen.push(label);
            }
        }
        seen.truncate(5);
        for label in seen {
            content = content.push(
                button(text(format!("+ {label}")).size(metrics.label() - 1.0))
                    .width(Fill)
                    .style(button::text)
                    .on_press(msg(Action::AddLabel(task.id.clone(), label))),
            );
        }
    }

    content.into()
}

/// The task's priority: none, P1 or P2.
fn priority_field<'a>(state: &State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
    let current = tb::metadata(task).priority;
    let busy = state.timeblocking.busy.contains(&task.id);

    let choices = std::iter::once((None, "None")).chain(
        tb::Priority::ALL
            .into_iter()
            .map(|priority| (Some(priority), priority.label())),
    );

    let mut options = row![].spacing(6);
    for (priority, label) in choices {
        let mut choice = button(text(label).size(metrics.label()).center().width(Fill))
            .width(Fill)
            .style(if priority == current {
                crate::ui::controls::primary
            } else {
                crate::ui::controls::secondary
            });
        if !busy {
            choice = choice.on_press(msg(Action::Priority(task.id.clone(), priority)));
        }
        options = options.push(choice);
    }

    column![field_label("Priority", metrics), options]
        .spacing(4)
        .into()
}

/// Where the task happens: the place it was given, or a way to search for one.
fn location_field<'a>(state: &'a State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
    let current = tb::metadata(task)
        .location
        .and_then(|id| state.config.travel.place(id));

    column![
        field_label("Location", metrics),
        super::travel::location_control(
            state,
            crate::travel::PickTarget::Task(task.id.clone()),
            current,
            "Add location",
            metrics,
        ),
    ]
    .spacing(4)
    .into()
}

/// Keep task actions in the same order in details and the backlog editor.
pub fn task_actions<'a>(state: &State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
    let status = tb::task_status(task);
    let mut actions = column![].spacing(metrics.spacing).width(Fill);
    if crate::inbox::untriaged(state, task) {
        let mut triage = button(text("Triage").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .style(crate::ui::controls::primary);
        if state.timeblocking.busy.is_empty() {
            triage = triage.on_press(msg(Action::Triage(task.id.clone())));
        }
        actions = actions.push(triage);
    }
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
                crate::ui::controls::primary
            } else {
                crate::ui::controls::secondary
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
    let compact_weekly =
        state.weekly && state.screen == crate::app::Screen::Calendar && metrics.size.width < 240.0;
    let delete = button(text("Delete").size(metrics.label()).center().width(Fill))
        .width(Fill)
        .on_press(Message::EventDeleteRequested(block_id.clone()))
        .style(crate::ui::controls::secondary);
    let close = button(text("Close").size(metrics.label()).center().width(Fill))
        .width(Fill)
        .on_press(msg(Action::Close))
        .style(crate::ui::controls::secondary);
    let actions: Element<'a, Message> = if compact_weekly {
        column![delete, close].spacing(metrics.spacing).into()
    } else {
        row![delete, close].spacing(metrics.spacing).into()
    };
    // Keep planning and drag targets above the longer block-details form.
    let editor = super::calendar::event_editor(state, block, metrics);
    let mut body = column![
        text(block.title.clone()).size(18),
        text(format!(
            "{} · {}–{}",
            block.start.format("%a, %b %-d"),
            block.start.format("%H:%M"),
            block.end.format("%H:%M")
        ))
        .size(metrics.label())
        .style(text::secondary),
        actions,
        super::controls::section_heading("Planned Tasks", Some(tasks.len()), 16.0),
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
        let card: Element<'a, Message> = if compact_weekly {
            column![
                super::calendar::weekly_task_chip(state, task, metrics),
                remove
            ]
            .into()
        } else {
            row![task_row(state, task, metrics, false), remove].into()
        };
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
    body = body.push(super::controls::section_heading(
        "Available tasks",
        None,
        16.0,
    ));
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
            state.timeblocking.available_planned || (event.all_day && tb::in_backlog(event))
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
        let add = button("+").on_press(msg(Action::Assign(task.id.clone(), block_id.clone())));
        if compact_weekly {
            body = body.push(column![
                super::calendar::weekly_task_chip(state, task, metrics),
                add
            ]);
        } else {
            body = body.push(row![task_row(state, task, metrics, false), add]);
        }
    }
    body = body
        .push(super::controls::section_heading(
            "Block details",
            None,
            16.0,
        ))
        .push(editor);
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
/// The Backlog tab: only the Google Tasks on the chosen backlog list, so a
/// task is here because it was deliberately parked, not merely because it is
/// unscheduled.
pub fn backlog<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let list_id = state
        .config
        .calendar
        .backlog_list
        .clone()
        .unwrap_or_else(|| {
            format!(
                "tasks:{}",
                crate::config::CalendarSettings::DEFAULT_BACKLOG_LIST
            )
        });
    let list_name = tb::task_list_name(state, &list_id);

    let mut tasks: Vec<_> = state
        .calendar
        .events
        .iter()
        .filter(|event| {
            event.is_task()
                && !event.completed
                && tb::metadata(event).block.is_none()
                && tb::is_backlog_list(state, &event.calendar_id)
        })
        .collect();
    tasks.sort_by_key(|event| tb::backlog_order(event));

    let query = state.timeblocking.backlog_query.trim().to_lowercase();
    let shown: Vec<_> = tasks
        .iter()
        .copied()
        .filter(|task| query.is_empty() || task.title.to_lowercase().contains(&query))
        .collect();

    let mut body = column![
        super::controls::page_heading(
            list_name.clone(),
            "Google Tasks parked on this list",
            Some(shown.len()),
            metrics
        ),
        text("Search").size(metrics.label()).style(text::secondary),
        text_input("Search tasks…", &state.timeblocking.backlog_query)
            .on_input(|value| msg(Action::BacklogQuery(value)))
            .size(metrics.label())
            .width(Fill),
    ]
    .spacing(8);
    if let Some(error) = &state.calendar.error {
        body = body.push(text(error).style(text::danger));
    }
    if tasks.is_empty() {
        body = body.push(text(format!("No tasks on the {list_name} list")).style(text::secondary));
    } else if shown.is_empty() {
        body = body.push(text("No matching tasks").style(text::secondary));
    }
    for section in tb::BacklogSection::ALL {
        let mut contents = column![text(section.label()).size(metrics.label())].spacing(6);
        for task in shown
            .iter()
            .copied()
            .filter(|task| tb::metadata(task).backlog_section == section)
        {
            contents = contents.push(task_row(state, task, metrics, true));
        }
        let dragging = state.timeblocking.dragging.is_some();
        let section_card =
            container(contents)
                .padding(8)
                .width(Fill)
                .style(move |theme: &iced::Theme| {
                    let palette = theme.extended_palette();
                    container::Style {
                        border: iced::Border {
                            color: if dragging {
                                palette.primary.base.color
                            } else {
                                palette.background.strong.color
                            },
                            width: 1.0,
                            radius: 5.0.into(),
                        },
                        ..Default::default()
                    }
                });
        body = body
            .push(mouse_area(section_card).on_release(msg(Action::DropBacklogSection(section))));
    }
    let content: Element<'a, Message> = container(scrollable(body))
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill)
        .into();
    content
}

pub use super::timeblocking_focus::session_editor;
pub use super::timeblocking_focus::view as focus_overlay;

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
        .style(crate::ui::controls::surface);
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
    fn task_title_click_starts_inline_editing() {
        let mut state = State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        );
        state.calendar = crate::calendar::Store::with_snapshot(crate::calendar::sample_snapshot());
        let task = state.calendar.event_mut("deep").unwrap();
        task.id = "task:deep".into();
        task.calendar_id = "tasks:list".into();
        let metrics = Metrics::of(Size::new(450.0, 900.0));
        let task = state.calendar.event("task:deep").unwrap();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            Size::new(450.0, 900.0),
            editable_title(&state, task, &metrics),
        );
        ui.click(task.title.as_str()).unwrap();
        assert!(
            ui.into_messages()
                .any(|m| matches!(m, Message::TimeBlocking(Action::EditTitle)))
        );
    }

    #[test]
    fn both_date_sections_offer_clear_and_matching_scheduling_actions() {
        let mut state = State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        );
        state.calendar = crate::calendar::Store::with_snapshot(crate::calendar::sample_snapshot());
        let task = state.calendar.event_mut("deep").unwrap();
        task.id = "task:deep".into();
        task.calendar_id = "tasks:list".into();
        state.timeblocking.date = "2026-10-29".into();
        state.timeblocking.deadline = "2026-10-30".into();
        let metrics = Metrics::of(Size::new(450.0, 900.0));
        for deadline in [false, true] {
            let task = state.calendar.event("task:deep").unwrap();
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                Size::new(450.0, 900.0),
                date_field(&state, task, &metrics, deadline),
            );
            assert!(
                ui.find(if deadline { "Deadline" } else { "Due Date" })
                    .is_ok()
            );
            assert!(ui.find("Schedule Day").is_ok());
            assert!(ui.find("Reset Time").is_ok());
            ui.click("Schedule Time").unwrap();
            assert!(ui.into_messages().any(|m| matches!(m, Message::TimeBlocking(Action::EditScheduleTime(value)) if value == deadline)));
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                Size::new(450.0, 900.0),
                date_field(&state, task, &metrics, deadline),
            );
            ui.click(if deadline {
                "Oct 30, 2026"
            } else {
                "Oct 29, 2026"
            })
            .unwrap();
            assert!(ui.into_messages().any(|m| matches!(m, Message::Picker(super::super::controls::Action::Open(p)) if p.optional)));
        }
    }

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
