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
    text_input,
};
use iced::{Color, Element, Fill, Length, Point, Rectangle, Size, Theme, mouse};

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
        let done = tasks
            .iter()
            .filter(|e| tb::task_status(e) == tb::TaskStatus::Completed)
            .count();
        let ignored = tasks
            .iter()
            .filter(|e| tb::task_status(e) == tb::TaskStatus::Ignored)
            .count();
        let planned: u64 = tasks
            .iter()
            .map(|e| u64::from(tb::metadata(e).minutes))
            .sum();
        let dump_day = block.start.date_naive();
        let dump_label = if state.config.brain_dump.get(dump_day).is_some() {
            "Brain dump"
        } else {
            "Add brain dump"
        };
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
                    "{done}/{} complete · {ignored} ignored · {planned}m planned",
                    tasks.len()
                ))
                .size(12)
                .width(Fill)
                .style(text::secondary),
                button(dump_label)
                    .on_press(Message::BrainDump(crate::brain_dump::Action::Open(
                        dump_day,
                    )))
                    .style(button::text),
                button("Work history")
                    .on_press(msg(Action::ToggleHistory))
                    .style(button::text)
            ],
        ]
        .spacing(5);
        let mut body = column![header, pomodoro(state)].spacing(10);
        if let Some(error) = &state.calendar.error {
            body = body.push(text(error).size(12).style(text::danger));
        }
        if tasks.is_empty() {
            body = body.push(
                text("No tasks assigned. Open this block to plan your work.")
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

/// One app-wide clock: changing blocks does not create competing timers.
fn pomodoro(state: &State) -> Element<'_, Message> {
    let timer = &state.config.pomodoro;
    let phase = match timer.phase {
        crate::pomodoro::Phase::Work => "Work",
        crate::pomodoro::Phase::Break => "Break",
    };
    let mut controls = row![
        text(format!(
            "Pomodoro · {phase} · {}",
            task_work::clock(timer.remaining)
        ))
        .size(18),
        button(if timer.running() {
            "Pause"
        } else if timer.fresh() {
            "Start Pomodoro"
        } else {
            "Resume"
        })
        .on_press(msg(Action::PomodoroToggle)),
        button("Reset")
            .on_press(msg(Action::PomodoroReset))
            .style(button::text),
    ]
    .spacing(8)
    .align_y(iced::Center);
    for (work, minutes, label) in [
        (true, timer.work_minutes, "Work"),
        (false, timer.break_minutes, "Break"),
    ] {
        let mut minus = button("−").style(button::text);
        let mut plus = button("+").style(button::text);
        if timer.fresh() {
            minus = minus.on_press(msg(Action::PomodoroMinutes(work, -1)));
            plus = plus.on_press(msg(Action::PomodoroMinutes(work, 1)));
        }
        controls = controls.push(
            row![text(format!("{label}: {minutes}m")).size(12), minus, plus].align_y(iced::Center),
        );
    }
    let mut body = column![
        controls.wrap(),
        text(format!(
            "Pomodoro totals · Work {} · Break {}",
            task_work::clock(timer.work_seconds),
            task_work::clock(timer.break_seconds)
        ))
        .size(12),
    ]
    .spacing(4);
    let help = if timer.running() && timer.phase == crate::pomodoro::Phase::Break {
        match &timer.target {
            Some(target) => format!("On break · Next task: {}. Choose another task with ‘Use after break’.", target.task_title),
            None => "On break · Choose a task with ‘Use after break’ to resume it in the next work session.".into(),
        }
    } else {
        "One timer across all blocks. Task Start/Stop also works independently. Breaks stop task work; work sessions resume it. Pause/Reset affects only the Pomodoro clock.".into()
    };
    body = body.push(text(help).size(11).style(text::secondary));
    container(body)
        .padding(10)
        .style(container::rounded_box)
        .into()
}

/// Edit the exact start and end of a work session. A blank end keeps the
/// session running, which is what the red "now" line shows.
pub fn session_editor<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(index) = state.timeblocking.session_edit else {
        return Space::new().width(0.0).height(0.0).into();
    };
    let Some(session) = state.config.task_work.sessions.get(index) else {
        return Space::new().width(0.0).height(0.0).into();
    };
    let width = metrics.size.width * 0.9;
    let date = state
        .timeblocking
        .session_date
        .map(|date| date.format("%A, %B %-d, %Y").to_string())
        .unwrap_or_default();
    let mut body = column![
        row![
            text("WORK SESSION")
                .size(11)
                .style(text::secondary)
                .width(Fill),
            button("Close")
                .on_press(msg(Action::CloseSession))
                .style(button::text),
        ],
        text(session.task_title.clone()).size(if width < 360.0 { 18 } else { 23 }),
        text(date).size(13).style(text::secondary),
        row![
            column![
                text("Start").size(12).style(text::secondary),
                text_input("HH:MM", &state.timeblocking.session_start)
                    .id(iced::widget::Id::new("work-session-start"))
                    .on_input(|value| msg(Action::SessionStart(value)))
                    .size(metrics.label()),
            ]
            .spacing(3)
            .width(Fill),
            column![
                text("End").size(12).style(text::secondary),
                text_input("Still working", &state.timeblocking.session_end)
                    .on_input(|value| msg(Action::SessionEnd(value)))
                    .size(metrics.label()),
            ]
            .spacing(3)
            .width(Fill),
        ]
        .spacing(8),
        text("Leave the end blank while the session is still running.")
            .size(11)
            .style(text::secondary),
    ]
    .spacing(10);
    if let Some(error) = &state.timeblocking.session_error {
        body = body.push(text(error).size(12).style(text::danger));
    }
    body = body.push(
        row![
            button("Save")
                .on_press(msg(Action::SaveSession))
                .style(button::primary)
                .width(Fill),
            button("Delete")
                .on_press(msg(Action::DeleteSession))
                .style(button::danger)
                .width(Fill),
        ]
        .spacing(8),
    );

    let card = container(body)
        .padding(if width < 320.0 { 12 } else { 20 })
        .width(Length::Fixed(width))
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                text_color: Some(palette.background.base.text),
                border: iced::Border {
                    radius: 12.0.into(),
                    color: palette.background.strong.color,
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
                    background: Some(Color::BLACK.scale_alpha(0.45).into()),
                    ..Default::default()
                }),
        )
        .on_press(msg(Action::CloseSession));
    let foreground = container(mouse_area(card).on_press(Message::Ignored))
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
    let bars = session_bars(state, block, now);
    let lane = if bars.is_empty() {
        0.0
    } else {
        SESSION_LANE + SESSION_GAP
    };
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
    if !bars.is_empty() {
        layers.push(
            canvas(WorkLane {
                start: block.start,
                end: block.end,
                scale,
                bars: bars.clone(),
            })
            .width(Fill)
            .height(Length::Fixed(height))
            .into(),
        );
    }
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
                    Space::new().width(8.0 + lane)
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
/// How wide the lane of worked-time boxes is, and the gap between it and the
/// task cards. The lane is only reserved while the block has work sessions.
const SESSION_LANE: f32 = 34.0;
const SESSION_GAP: f32 = 8.0;

/// One saved work session, placed on the block's timeline. The index points
/// back into the work log so a click can open the same session for editing.
#[derive(Debug, Clone)]
struct SessionBar {
    index: usize,
    start: DateTime<Local>,
    end: DateTime<Local>,
    running: bool,
}

fn session_bars(state: &State, block: &Event, now: DateTime<Utc>) -> Vec<SessionBar> {
    let key = tb::block_key(block);
    state
        .config
        .task_work
        .sessions
        .iter()
        .enumerate()
        .filter(|(_, session)| session.block_key.as_deref() == Some(key.as_str()))
        .filter_map(|(index, session)| {
            let end = session.stopped_at.unwrap_or(now);
            // A session that does not overlap this block would be drawn in the
            // margin and has no place on its timeline.
            if end <= block.start.with_timezone(&Utc)
                || session.started_at >= block.end.with_timezone(&Utc)
            {
                return None;
            }
            Some(SessionBar {
                index,
                start: session.started_at.with_timezone(&Local),
                end: end.with_timezone(&Local),
                running: session.stopped_at.is_none(),
            })
        })
        .collect()
}

/// The worked-time boxes themselves. They live in their own lane beside the
/// task cards so a click on one cannot land on a card's controls.
struct WorkLane {
    start: DateTime<Local>,
    end: DateTime<Local>,
    scale: f32,
    bars: Vec<SessionBar>,
}

impl WorkLane {
    fn rect(&self, bounds: Rectangle, bar: &SessionBar) -> Rectangle {
        let duration = (self.end - self.start).num_seconds().max(60) as f32;
        let start = ((bar.start - self.start).num_seconds() as f32).clamp(0.0, duration);
        let end = ((bar.end - self.start).num_seconds() as f32).clamp(0.0, duration);
        let top = 32.0 + start * self.scale;
        let bottom = 32.0 + end * self.scale;
        Rectangle {
            x: (bounds.width - SESSION_LANE - 4.0).max(0.0),
            y: top,
            width: SESSION_LANE,
            height: (bottom - top).max(6.0),
        }
        .intersection(&Rectangle {
            x: 0.0,
            y: 0.0,
            width: bounds.width,
            height: bounds.height,
        })
        .unwrap_or(Rectangle {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
        })
    }
}

impl canvas::Program<Message> for WorkLane {
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if !matches!(
            event,
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        ) {
            return None;
        }
        let point = cursor.position_in(bounds)?;
        for bar in &self.bars {
            if self.rect(bounds, bar).contains(point) {
                return Some(
                    canvas::Action::publish(Message::TimeBlocking(Action::EditSession(bar.index)))
                        .and_capture(),
                );
            }
        }
        None
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry> {
        let palette = theme.extended_palette();
        let mut frame = Frame::new(renderer, bounds.size());
        for bar in &self.bars {
            let rect = self.rect(bounds, bar);
            if rect.width <= 0.0 || rect.height <= 0.0 {
                continue;
            }
            let fill = if bar.running {
                palette.primary.base.color
            } else {
                palette.primary.weak.color
            };
            frame.fill(
                &Path::rounded_rectangle(rect.position(), rect.size(), 4.0.into()),
                fill,
            );
            frame.stroke(
                &Path::rounded_rectangle(rect.position(), rect.size(), 4.0.into()),
                Stroke {
                    style: canvas::Style::Solid(palette.primary.strong.color),
                    width: 1.0,
                    ..Default::default()
                },
            );
            if rect.height >= 14.0 {
                let minutes = (bar.end - bar.start).num_minutes().max(1);
                frame.fill_text(Text {
                    content: format!("{minutes}m"),
                    position: Point::new(rect.center_x(), rect.center_y()),
                    color: if bar.running {
                        Color::WHITE
                    } else {
                        palette.background.base.text
                    },
                    size: iced::Pixels(9.0),
                    align_x: iced::alignment::Horizontal::Center.into(),
                    align_y: iced::alignment::Vertical::Center,
                    ..Default::default()
                });
            }
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if let Some(point) = cursor.position_in(bounds)
            && self
                .bars
                .iter()
                .any(|bar| self.rect(bounds, bar).contains(point))
        {
            return mouse::Interaction::Pointer;
        }
        mouse::Interaction::default()
    }
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
    let ignored = tb::task_status(task) == tb::TaskStatus::Ignored;
    let mut completion = button(
        text(if task.completed && !ignored {
            "Completed ✓"
        } else {
            "Complete"
        })
        .size(12),
    )
    .style(button::text);
    if state.timeblocking.busy.is_empty() {
        completion = completion.on_press(Message::TaskToggled(task.id.clone()));
    }
    let mut ignore = button(
        text(if ignored {
            "Unignore Task"
        } else {
            "Ignore Task"
        })
        .size(12),
    )
    .style(button::text);
    if state.timeblocking.busy.is_empty() {
        ignore = ignore.on_press(Message::TaskIgnoreToggled(task.id.clone()));
    }
    let actions = column![completion, ignore].spacing(3);
    let title = super::timeblocking::task_title(task, if compact { 14.0 } else { 17.0 });
    let heading: Element<'a, Message> = if compact {
        column![title, actions].spacing(3).into()
    } else {
        row![title, actions].align_y(iced::Center).spacing(8).into()
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
        let mut control = button(
            text(if running {
                "Stop"
            } else if state.config.pomodoro.running()
                && state.config.pomodoro.phase == crate::pomodoro::Phase::Break
            {
                "Use after break"
            } else {
                "Start"
            })
            .size(13),
        )
        .style(if running {
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
