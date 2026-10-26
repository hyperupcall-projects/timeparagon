//! A focused, fixed-scale timeline for one time block.
use super::layout::Metrics;
use crate::{
    app::{Message, State},
    calendar::Event,
    task_work,
    timeblocking::{self as tb, Action},
};
use chrono::{DateTime, Local, Utc};
use iced::widget::canvas::{Frame, Geometry, Path, Stroke, Text};
use iced::widget::{
    Space, button, canvas, column, container, mouse_area, responsive, row, scrollable, stack, text,
};
use iced::{Color, Element, Fill, Length, Point, Size, Theme};

fn msg(action: Action) -> Message {
    Message::TimeBlocking(action)
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let width = metrics.size.width * 0.95;
    let height = metrics.size.height * 0.90;
    let block = state
        .timeblocking
        .focus
        .as_deref()
        .and_then(|id| state.calendar.event(id));
    let body: Element<'a, Message> = if let Some(block) = block {
        let tasks = tb::assigned_all(state, block);
        let done = tasks.iter().filter(|e| e.completed).count();
        let planned: u64 = tasks
            .iter()
            .map(|e| u64::from(tb::metadata(e).minutes))
            .sum();
        let header = column![
            row![
                text("FOCUS · TIME BLOCK")
                    .size(11)
                    .style(text::secondary)
                    .width(Fill),
                button("Close")
                    .on_press(msg(Action::CloseView))
                    .style(button::text)
            ],
            text(block.title.clone()).size(if width < 360.0 { 19 } else { 25 }),
            text(format!(
                "{} · {} – {}",
                block.start.format("%a, %b %-d"),
                block.start.format("%H:%M"),
                block.end.format("%H:%M")
            ))
            .size(13)
            .style(text::secondary),
            row![
                text(format!(
                    "{done}/{} complete · {planned}m planned",
                    tasks.len()
                ))
                .size(12)
                .width(Fill)
                .style(text::secondary),
                button("Work history")
                    .on_press(msg(Action::ToggleHistory))
                    .style(button::text)
            ],
        ]
        .spacing(5);
        let mut body = column![header].spacing(10);
        if let Some(error) = &state.calendar.error {
            body = body.push(text(error).size(12).style(text::danger));
        }
        if tasks.is_empty() {
            body = body.push(
                text("No tasks assigned. Use Edit on this block to plan your work.")
                    .size(14)
                    .style(text::secondary),
            );
        }
        let block_id = block.id.clone();
        body = body.push(
            responsive(move |size| {
                let block = state
                    .calendar
                    .event(&block_id)
                    .expect("block exists while view is rendered");
                timeline(state, block, size)
            })
            .height(Fill),
        );
        if state.timeblocking.history_open {
            let key = tb::block_key(block);
            let now = state.timeblocking.work_now.unwrap_or_else(Utc::now);
            let mut history = column![text("Saved work sessions").size(14)].spacing(6);
            let sessions: Vec<_> = state
                .config
                .task_work
                .sessions
                .iter()
                .filter(|s| s.block_key.as_deref() == Some(&key))
                .collect();
            if sessions.is_empty() {
                history =
                    history.push(text("No work sessions yet").size(12).style(text::secondary));
            }
            for session in sessions.into_iter().rev() {
                history = history.push(
                    text(format!(
                        "{} · {} → {} · {}",
                        session.task_title,
                        session
                            .started_at
                            .with_timezone(&Local)
                            .format("%b %-d %H:%M"),
                        session
                            .stopped_at
                            .map(|t| t.with_timezone(&Local).format("%H:%M").to_string())
                            .unwrap_or_else(|| "running".into()),
                        task_work::clock(session.seconds(now))
                    ))
                    .size(12),
                );
            }
            body = body.push(container(scrollable(history)).max_height(135));
        }
        body.into()
    } else {
        column![
            text("This time block is no longer available"),
            button("Close").on_press(msg(Action::CloseView))
        ]
        .into()
    };
    let card = container(body)
        .padding(if width < 320.0 { 10 } else { 20 })
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .style(|theme: &Theme| {
            let p = theme.extended_palette();
            container::Style {
                background: Some(p.background.base.color.into()),
                text_color: Some(p.background.base.text),
                border: iced::Border {
                    radius: 12.0.into(),
                    color: p.background.strong.color,
                    width: 1.0,
                },
                shadow: iced::Shadow {
                    color: Color::BLACK.scale_alpha(0.4),
                    offset: iced::Vector::new(0.0, 10.0),
                    blur_radius: 30.0,
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
        .on_press(msg(Action::CloseView))
        .on_scroll(|_| Message::Ignored);
    let foreground = container(
        mouse_area(card)
            .on_press(Message::Ignored)
            .on_scroll(|_| Message::Ignored),
    )
    .center_x(Fill)
    .center_y(Fill);
    stack![backdrop, foreground].into()
}

/// Fit the block to the available height; use a larger scrollable surface only
/// when tasks need more room for their controls. The mapping stays fixed while ticking.
fn timeline<'a>(state: &'a State, block: &'a Event, size: Size) -> Element<'a, Message> {
    let tasks = tb::assigned_all(state, block);
    let duration = (block.end - block.start).num_seconds().max(60) as f32;
    let slots: Vec<f32> = tasks
        .iter()
        .map(|e| {
            let minutes = tb::metadata(e).minutes;
            if minutes == 0 {
                15.0 * 60.0
            } else {
                minutes as f32 * 60.0
            }
        })
        .collect();
    let min_card = if size.width < 280.0 {
        250.0
    } else if size.width < 380.0 {
        190.0
    } else {
        150.0
    };
    let scale = slots.iter().fold(
        (size.height - 64.0).max(100.0) / duration,
        |scale, seconds| scale.max((min_card + 10.0) / seconds),
    );
    let content_seconds: f32 = slots.iter().sum();
    let height = duration.max(content_seconds) * scale + 64.0;
    let now = state.timeblocking.work_now.unwrap_or_else(Utc::now);
    let lines = Timeline {
        start: block.start,
        end: block.end,
        now,
        scale,
        needle: false,
    };
    let mut layers: Vec<Element<'a, Message>> = vec![
        canvas(lines)
            .width(Fill)
            .height(Length::Fixed(height))
            .into(),
    ];
    let mut offset = 32.0;
    for (task, seconds) in tasks.iter().zip(slots.iter()) {
        let card = task_card(state, task, now, size.width < 380.0);
        layers.push(
            column![
                Space::new().height(Length::Fixed(offset)),
                row![
                    Space::new().width(if size.width < 280.0 { 47 } else { 67 }),
                    container(card)
                        .height(Length::Fixed(seconds * scale - 8.0))
                        .width(Fill),
                    Space::new().width(8)
                ]
            ]
            .into(),
        );
        offset += seconds * scale;
    }
    layers.push(
        canvas(Timeline {
            start: block.start,
            end: block.end,
            now,
            scale,
            needle: true,
        })
        .width(Fill)
        .height(Length::Fixed(height))
        .into(),
    );
    scrollable(stack(layers).height(Length::Fixed(height)))
        .id(iced::widget::Id::new("focused-block-timeline"))
        .width(Fill)
        .height(Fill)
        .into()
}
fn task_card<'a>(
    state: &'a State,
    task: &'a Event,
    now: DateTime<Utc>,
    compact: bool,
) -> Element<'a, Message> {
    let key = tb::task_key(task);
    let log = &state.config.task_work;
    let running = log.running(&key);
    let seconds = log.seconds(&key, now);
    let minutes = tb::metadata(task).minutes;
    let overtime = minutes > 0 && seconds > u64::from(minutes) * 60;
    let busy = state.timeblocking.busy.contains(&task.id);
    let mut completion = button(
        text(if task.completed {
            "Completed ✓"
        } else {
            "Complete"
        })
        .size(12),
    )
    .style(button::text);
    if !busy {
        completion = completion.on_press(Message::TaskToggled(task.id.clone()));
    }
    let title = text(task.title.clone())
        .size(if compact { 14 } else { 17 })
        .width(Fill)
        .style(if task.completed {
            text::secondary
        } else {
            text::default
        });
    let heading: Element<'a, Message> = if compact {
        column![title, completion].spacing(3).into()
    } else {
        row![title, completion]
            .align_y(iced::Center)
            .spacing(8)
            .into()
    };
    let clock = text(task_work::clock(seconds))
        .size(if compact { 20 } else { 24 })
        .style(if overtime && !task.completed {
            text::danger
        } else if task.completed {
            text::secondary
        } else {
            text::default
        });
    let estimate = text(if minutes == 0 {
        "No estimate · 15m on timeline".into()
    } else if overtime {
        format!(
            "{} over · {}m estimated",
            task_work::clock(seconds - u64::from(minutes) * 60),
            minutes
        )
    } else {
        format!("{}m estimated", minutes)
    })
    .size(12)
    .style(if overtime && !task.completed {
        text::danger
    } else {
        text::secondary
    });
    let mut details = column![
        heading,
        row![clock, Space::new().width(Fill)].align_y(iced::Center),
        estimate,
        super::timeblocking::estimate_button(state, task, 12.0)
    ]
    .spacing(4);
    if !task.completed {
        let mut control =
            button(text(if running { "Stop" } else { "Start" }).size(13)).style(if running {
                button::danger
            } else {
                button::primary
            });
        if running || !busy {
            control = control.on_press(msg(if running {
                Action::StopWork(task.id.clone())
            } else {
                Action::StartWork(task.id.clone())
            }));
        }
        let sessions = log.sessions.iter().filter(|s| s.task_key == key).count();
        details = details.push(
            row![
                control,
                text(if running {
                    "Working now".to_owned()
                } else if sessions == 0 {
                    "Not started".to_owned()
                } else {
                    format!("{sessions} saved sessions")
                })
                .size(11)
                .style(text::secondary)
            ]
            .spacing(10)
            .align_y(iced::Center),
        );
    }
    container(scrollable(details))
        .padding(if compact { 8 } else { 12 })
        .width(Fill)
        .height(Fill)
        .style(move |theme: &Theme| {
            let p = theme.extended_palette();
            container::Style {
                background: Some(
                    if task.completed {
                        p.background.weak.color
                    } else {
                        p.background.base.color
                    }
                    .into(),
                ),
                text_color: Some(if task.completed {
                    p.background.weak.text.scale_alpha(0.65)
                } else {
                    p.background.base.text
                }),
                border: iced::Border {
                    color: if running {
                        p.primary.strong.color
                    } else {
                        p.background.strong.color
                    },
                    width: if running { 2.0 } else { 1.0 },
                    radius: 8.0.into(),
                },
                ..Default::default()
            }
        })
        .into()
}
struct Timeline {
    start: DateTime<Local>,
    end: DateTime<Local>,
    now: DateTime<Utc>,
    scale: f32,
    needle: bool,
}
impl canvas::Program<Message> for Timeline {
    type State = ();
    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let gutter = if bounds.width < 280.0 { 45.0 } else { 64.0 };
        let duration = (self.end - self.start).num_seconds().max(60) as f32;
        let line = |frame: &mut Frame, y: f32, color: Color, width: f32| {
            frame.stroke(
                &Path::line(Point::new(gutter, y), Point::new(bounds.width, y)),
                Stroke {
                    style: canvas::Style::Solid(color),
                    width,
                    ..Default::default()
                },
            );
        };
        if self.needle {
            let elapsed = (self.now - self.start.with_timezone(&Utc)).num_seconds() as f32;
            let y = 32.0 + elapsed.clamp(0.0, duration) * self.scale;
            let red = Color::from_rgb8(231, 66, 57);
            line(&mut frame, y, red, 1.5);
            frame.fill(&Path::circle(Point::new(gutter, y), 4.5), red);
            let suffix = if elapsed < 0.0 {
                " · before block"
            } else if elapsed > duration {
                " · block ended"
            } else {
                ""
            };
            frame.fill_text(Text {
                content: format!(
                    "Now {}{}",
                    self.now.with_timezone(&Local).format("%H:%M"),
                    suffix
                ),
                position: Point::new(gutter + 8.0, (y - 16.0).max(0.0)),
                color: red,
                size: iced::Pixels(11.0),
                ..Default::default()
            });
        } else {
            let palette = theme.extended_palette();
            let mut seconds = 0.0;
            while seconds < duration {
                let y = 32.0 + seconds * self.scale;
                line(&mut frame, y, palette.background.strong.color, 1.0);
                let when = self.start + chrono::Duration::seconds(seconds as i64);
                frame.fill_text(Text {
                    content: when.format("%H:%M").to_string(),
                    position: Point::new(2.0, y + 3.0),
                    color: palette.background.base.text.scale_alpha(0.6),
                    size: iced::Pixels(11.0),
                    ..Default::default()
                });
                // Limit labels on very long events, keeping at most 96 grid lines.
                seconds += (duration / 96.0 / 900.0).ceil().max(1.0) * 900.0;
            }
            let y = 32.0 + duration * self.scale;
            line(
                &mut frame,
                y,
                palette.background.base.text.scale_alpha(0.5),
                1.5,
            );
            frame.fill_text(Text {
                content: self.end.format("%H:%M").to_string(),
                position: Point::new(2.0, y + 3.0),
                color: palette.background.base.text.scale_alpha(0.6),
                size: iced::Pixels(11.0),
                ..Default::default()
            });
        }
        vec![frame.into_geometry()]
    }
}
