use super::layout::Metrics;
use crate::{
    app::{Message, State},
    calendar::Event,
    timeblocking::{self as tb, Action},
};
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, text_input};
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
    let m = tb::metadata(&task.details.description);
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
                total += u64::from(tb::metadata(&item.details.description).minutes);
                if item.id == task.id {
                    break;
                }
            }
            total > (block.end - block.start).num_minutes().max(0) as u64
        });
    let busy = state.timeblocking.busy.contains(&task.id);
    let mut check = button(text(if task.completed { "●" } else { "○" })).style(button::text);
    if !busy {
        check = check.on_press(Message::TaskToggled(task.id.clone()));
    }
    let title = button(
        text(format!(
            "{}{}",
            task.title,
            if busy { " · Saving…" } else { "" }
        ))
        .size(metrics.label()),
    )
    .width(Fill)
    .style(button::text)
    .on_press(Message::EventOpened(task.id.clone()));
    let handle = mouse_area(text("⠿").size(17)).on_press(msg(Action::Drag(task.id.clone())));
    let mut line = row![
        handle,
        check,
        title,
        text(format!("{}m", m.minutes))
            .size(metrics.label())
            .style(text::secondary)
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
    let card = container(line)
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
    mouse_area(card)
        .on_release(msg(Action::DropTask(task.id.clone())))
        .into()
}
pub fn editor<'a>(state: &'a State, task: &'a Event, metrics: &Metrics) -> Element<'a, Message> {
    let busy = state.timeblocking.busy.contains(&task.id);
    let mut estimate = button(text("Save estimate").size(metrics.label()));
    let mut schedule = button(text("Reschedule").size(metrics.label()));
    let mut backlog = button(text("Move to Backlog").size(metrics.label())).style(button::text);
    if !busy {
        estimate = estimate.on_press(msg(Action::SaveEstimate(task.id.clone())));
        schedule = schedule.on_press(msg(Action::Reschedule(task.id.clone())));
        backlog = backlog.on_press(msg(Action::Backlog(task.id.clone())));
    }
    column![
        text("Estimate · minutes")
            .size(metrics.label())
            .style(text::secondary),
        row![
            text_input("Minutes", &state.timeblocking.estimate)
                .on_input(|v| msg(Action::Estimate(v)))
                .size(metrics.label()),
            estimate
        ]
        .spacing(6),
        text("Due date")
            .size(metrics.label())
            .style(text::secondary),
        row![
            text_input("YYYY-MM-DD", &state.timeblocking.date)
                .on_input(|v| msg(Action::Date(v)))
                .size(metrics.label()),
            schedule
        ]
        .spacing(6),
        backlog,
    ]
    .spacing(8)
    .into()
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
    let total: u32 = tasks
        .iter()
        .map(|e| tb::metadata(&e.details.description).minutes)
        .sum();
    let capacity = (block.end - block.start).num_minutes().max(0) as u32;
    let mut body = column![
        row![
            text(block.title.clone()).size(20).width(Fill),
            button("×").on_press(msg(Action::Close)).style(button::text)
        ],
        text(format!("{}m planned / {}m available", total, capacity))
            .size(13)
            .style(if total > capacity {
                text::danger
            } else {
                text::secondary
            }),
        button("Edit time block")
            .on_press(Message::EventEditRequested(block.id.clone()))
            .style(button::text),
        button("Event details")
            .on_press(msg(Action::Details(block.id.clone())))
            .style(button::text),
        text("Drag tasks here to assign · Drag rows to reorder")
            .size(12)
            .style(text::secondary),
    ]
    .spacing(10);
    let mut elapsed = 0;
    for task in tasks {
        elapsed += tb::metadata(&task.details.description).minutes;
        let card = task_row(state, task, metrics, false);
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
    for task in state.calendar.events.iter().filter(|e| {
        e.is_task() && !e.completed && tb::metadata(&e.details.description).block.is_none()
    }) {
        body = body.push(row![
            task_row(state, task, metrics, false),
            button("+").on_press(msg(Action::Assign(task.id.clone(), block.id.clone())))
        ]);
    }
    mouse_area(
        container(scrollable(body))
            .padding(16)
            .width(Length::Fixed(400.0))
            .height(Fill),
    )
    .on_release(msg(Action::DropBlock(block.id.clone())))
    .into()
}
pub fn backlog<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut body = column![
        text("Backlog").size(25),
        text("Unscheduled Google Tasks")
            .size(13)
            .style(text::secondary)
    ]
    .spacing(8);
    if let Some(error) = &state.calendar.error {
        body = body.push(text(error).style(text::danger));
    }
    let mut tasks: Vec<_> = state
        .calendar
        .events
        .iter()
        .filter(|e| e.is_task() && !e.completed && e.start.date_naive() == tb::UNDATED)
        .collect();
    tasks.sort_by_key(|e| tb::metadata(&e.details.description).order);
    if tasks.is_empty() {
        body = body.push(text("No tasks in your backlog").style(text::secondary));
    }
    for task in tasks {
        body = body.push(task_row(state, task, metrics, true));
    }
    let content: Element<'a, Message> = container(scrollable(body))
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill)
        .into();
    if let Some(task) = state
        .calendar
        .open_event
        .as_deref()
        .and_then(|id| state.calendar.event(id))
        .filter(|e| e.is_task())
    {
        column![
            content,
            container(column![
                text(task.title.clone()).size(18),
                editor(state, task, metrics),
                button("Done").on_press(Message::EventClosed)
            ])
            .padding(16)
        ]
        .into()
    } else {
        content
    }
}

pub use super::timeblocking_focus::view as focus_overlay;
