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
    if let Ok(Some(date)) = crate::calendar::deadline::read(&task.details.description) {
        content = content.push(text(format!("DUE: {date}")).size(metrics.label() - 1.0)
            .style(if overdue { text::danger } else { text::secondary }));
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
pub fn editor<'a>(state: &'a State, task: &'a Event, metrics: &Metrics) -> Element<'a, Message> {
    let busy = state.timeblocking.busy.contains(&task.id);
    let mut save_deadline = button(text("Save deadline").size(metrics.label()));
    let mut schedule = button(text("Schedule time").size(metrics.label()));
    let mut day = button(text("Set day only").size(metrics.label())).style(button::text);
    let mut backlog = button(text("Move to Backlog").size(metrics.label())).style(button::text);
    if !busy {
        save_deadline = save_deadline.on_press(msg(Action::SaveDeadline(task.id.clone())));
        schedule = schedule.on_press(msg(Action::ScheduleEntered(task.id.clone())));
        day = day.on_press(msg(Action::Reschedule(task.id.clone())));
        backlog = backlog.on_press(msg(Action::Backlog(task.id.clone())));
    }
    let mut body = column![
        estimate_button(state, task, metrics.label()),
        text("Deadline").size(metrics.label()).style(text::secondary),
        text_input("YYYY-MM-DD (blank clears deadline)", &state.timeblocking.deadline)
            .on_input(|v| msg(Action::Deadline(v))).size(metrics.label()),
        save_deadline,
        text("Scheduled day and time").size(metrics.label()).style(text::secondary),
        text_input("YYYY-MM-DD", &state.timeblocking.date)
            .on_input(|v| msg(Action::Date(v))).size(metrics.label()),
        row![
            text_input("Start HH:MM", &state.timeblocking.start_time)
                .on_input(|v| msg(Action::StartTime(v))).size(metrics.label()),
            text_input("End HH:MM", &state.timeblocking.end_time)
                .on_input(|v| msg(Action::EndTime(v))).size(metrics.label())
        ].spacing(6),
        schedule,
        day,
        backlog,
    ].spacing(8);
    if let Err(error) = crate::calendar::deadline::read(&task.details.description) {
        body = body.push(text(error).size(metrics.label()).style(text::danger));
    }
    body.into()
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
        .map(|e| tb::metadata(e).minutes)
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
        elapsed += tb::metadata(task).minutes;
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
        e.is_task() && !e.completed && tb::metadata(e).block.is_none()
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
        .filter(|e| {
            let m = tb::metadata(e);
            let pending = m.schedule.as_ref().is_some_and(|link| link.pending);
            e.is_task() && !e.completed && (pending || (m.schedule.is_none() && m.block.is_none()))
        })
        .collect();
    tasks.sort_by_key(|e| tb::metadata(e).order);
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

/// Every estimate entry point opens the same app-wide dialog.
pub(super) fn estimate_button<'a>(state: &State, task: &Event, label_size: f32) -> Element<'a, Message> {
    let minutes = tb::metadata(task).minutes;
    let mut control = button(text(format!("Estimate: {minutes}m")).size(label_size))
        .style(button::text);
    if !state.timeblocking.busy.contains(&task.id) {
        control = control.on_press(msg(Action::EditEstimate(task.id.clone())));
    }
    control.into()
}

pub fn estimate_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    use iced::widget::{Space, stack};
    let task = state.timeblocking.estimate_task.as_deref().and_then(|id| state.calendar.event(id));
    let mut body = column![text("Estimated completion time").size(18)].spacing(10);
    if let Some(task) = task {
        let busy = state.timeblocking.busy.contains(&task.id);
        let mut input = text_input("Minutes", &state.timeblocking.estimate)
            .id(iced::widget::Id::new("task-estimate"))
            .size(metrics.label());
        let mut save = button(text(if busy { "Saving…" } else { "Save" }));
        if !busy {
            input = input.on_input(|value| msg(Action::Estimate(value)))
                .on_submit(msg(Action::SaveEstimate(task.id.clone())));
            save = save.on_press(msg(Action::SaveEstimate(task.id.clone())));
        }
        body = body.push(text(task.title.clone()).size(metrics.label()))
            .push(text("Estimate in minutes (0 means no estimate)").size(metrics.label()).style(text::secondary))
            .push(input);
        if let Some(error) = &state.timeblocking.estimate_error {
            body = body.push(text(error).size(metrics.label()).style(text::danger));
        }
        body = body.push(row![save, button("Cancel").on_press(msg(Action::CloseEstimate)).style(button::text)].spacing(8));
    } else {
        body = body.push(text("This task is no longer available"))
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
                border: iced::Border { radius: 10.0.into(), ..Default::default() },
                ..Default::default()
            }
        });
    let backdrop = mouse_area(container(Space::new()).width(Fill).height(Fill)
        .style(|_| container::Style {
            background: Some(Color::BLACK.scale_alpha(0.58).into()),
            ..Default::default()
        }))
        .on_press(msg(Action::CloseEstimate))
        .on_scroll(|_| Message::Ignored);
    let foreground = container(mouse_area(card).on_press(Message::Ignored).on_scroll(|_| Message::Ignored))
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
            let mut state = State::new(crate::config::Config::default(), crate::theme::Catalog::built_in());
            state.calendar = crate::calendar::Store::with_snapshot(crate::calendar::sample_snapshot());
            let task = state.calendar.event_mut("deep").unwrap();
            task.id = "task:deep".into();
            task.calendar_id = "tasks:list".into();
            state.timeblocking.focus = Some("standup".into());
            let _ = tb::update(&mut state, Action::EditEstimate("task:deep".into()));
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(), Size::new(width, 600.0), super::super::view(&state),
            );
            assert!(ui.find("Save").is_ok());
            assert!(ui.find("Cancel").is_ok());
            ui.point_at(Point::new(2.0, 2.0));
            ui.simulate([
                iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ]);
            let messages: Vec<_> = ui.into_messages().collect();
            assert!(messages.iter().any(|m| matches!(m, Message::TimeBlocking(Action::CloseEstimate))));
            assert!(!messages.iter().any(|m| matches!(m, Message::TimeBlocking(Action::CloseView))));
        }
    }
}
