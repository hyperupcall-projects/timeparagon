//! One time block as a page: a Pomodoro clock, its tasks with their timers, and
//! a timeline that fits the window.
use super::layout::Metrics;
use crate::app::{EventMessage, TaskMessage};
use crate::calendar::time_format::TimeFormat;
use crate::{
    app::{Message, State},
    calendar::Event,
    task_work,
    timeblocking::{self as tb, Action},
};
use chrono::{DateTime, Local, Utc};
use iced::widget::canvas::{Frame, Geometry, Path, Stroke, Text};
use iced::widget::{
    Space, button, canvas, column, container, mouse_area, progress_bar, responsive, rich_text, row,
    span, stack, text,
};
use iced::{Color, Element, Fill, Length, Point, Rectangle, Size, Theme, mouse};

/// Below this width a task's timers move under its title.
const NARROW_BELOW: f32 = 380.0;
/// Below this height the page scrolls instead of crushing the timeline.
const SHORT_BELOW: f32 = 480.0;
const SHORT_TIMELINE: f32 = 240.0;
/// The columns of a task line, which the card's headings line up with.
const ACTUAL_WIDTH: f32 = 62.0;
const PLANNED_WIDTH: f32 = 56.0;
const ACTION_WIDTH: f32 = 54.0;
const LINE_SPACING: f32 = 6.0;
/// Room above and below the timeline for the outermost time labels.
const TIMELINE_INSET: f32 = 10.0;
/// The closest two time labels can be without overlapping.
const MARK_SPACING: f32 = 13.0;

fn msg(action: Action) -> Message {
    Message::TimeBlocking(action)
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let block = state
        .timeblocking
        .focus
        .as_deref()
        .and_then(|id| state.calendar.event(id));
    let narrow = metrics.size.width < NARROW_BELOW;
    let short = metrics.size.height < SHORT_BELOW;
    let body: Element<'a, Message> = if let Some(block) = block {
        let dump_day = block.start.date_naive();
        let dump_label = if state.config.brain_dump.get(dump_day).is_some() {
            "Brain dump"
        } else {
            "Add brain dump"
        };
        let header = column![
            row![
                text(block.title.clone())
                    .size(if narrow { 18 } else { 22 })
                    .width(Fill),
                button(text("Close").size(13))
                    .on_press(msg(Action::CloseView))
                    .style(crate::ui::controls::ghost)
            ]
            .align_y(iced::Center),
            row![
                text(format!(
                    "{} · {}",
                    block.start.format("%a, %b %-d"),
                    crate::calendar::time_format::range(
                        block.start,
                        block.end,
                        state.config.appearance.time_format
                    )
                ))
                .size(13)
                .style(text::secondary)
                .width(Fill),
                button(text(dump_label).size(12))
                    .padding([2, 6])
                    .on_press(Message::BrainDump(crate::brain_dump::Action::Open(
                        dump_day,
                    )))
                    .style(crate::ui::controls::ghost),
            ]
            .align_y(iced::Center),
        ]
        .spacing(2);
        let mut body = column![header, pomodoro(state, narrow)].spacing(8);
        if let Some(error) = &state.calendar.error {
            body = body.push(text(error).size(12).style(text::danger));
        }
        body = body.push(task_list(state, block, metrics, narrow, short));
        body = body.push(distractions(state, block));
        let block_id = block.id.clone();
        let schedule = responsive(move |size| {
            let block = state
                .calendar
                .event(&block_id)
                .expect("block exists while view is rendered");
            timeline(state, block, size)
        });
        if short {
            // Too little room for everything: only now does the page scroll,
            // and the timeline inside it still fits its own height.
            crate::ui::controls::scrollable(
                body.push(schedule.height(Length::Fixed(SHORT_TIMELINE))),
            )
            .height(Fill)
            .into()
        } else {
            body.push(schedule.height(Fill)).into()
        }
    } else {
        column![
            text("This time block is no longer available"),
            crate::ui::controls::quiet_button("Close").on_press(msg(Action::CloseView))
        ]
        .into()
    };
    // The whole window, like every other page: nothing behind it is reachable,
    // so the press and scroll handlers stop input from falling through.
    mouse_area(
        container(body)
            .padding(8)
            .width(Fill)
            .height(Fill)
            .style(|theme: &Theme| {
                let p = theme.extended_palette();
                container::Style {
                    background: Some(p.background.weakest.color.into()),
                    text_color: Some(p.background.base.text),
                    ..Default::default()
                }
            }),
    )
    .on_press(Message::Ignored)
    .on_scroll(|_| Message::Ignored)
    .into()
}

/// One app-wide clock: changing blocks does not create competing timers.
fn pomodoro(state: &State, narrow: bool) -> Element<'_, Message> {
    let timer = &state.config.pomodoro;
    let on_break = timer.phase == crate::pomodoro::Phase::Break;
    let clock = text(format!(
        "{} · {}",
        if on_break { "Break" } else { "Work" },
        task_work::clock(timer.remaining)
    ))
    .size(18);
    let mut controls = row![
        button(
            text(if timer.running() {
                "Stop Pomodoro"
            } else {
                "Start Pomodoro"
            })
            .size(13)
        )
        .on_press(msg(Action::PomodoroToggle))
        .style(if timer.running() {
            crate::ui::controls::danger
        } else {
            crate::ui::controls::primary
        })
    ]
    .spacing(4)
    .align_y(iced::Center);
    if !timer.fresh() {
        controls = controls.push(
            button(text("Reset").size(13))
                .on_press(msg(Action::PomodoroReset))
                .style(crate::ui::controls::ghost),
        );
    }
    let top: Element<'_, Message> = if narrow {
        column![clock, controls].spacing(6).into()
    } else {
        row![clock.width(Fill), controls]
            .spacing(8)
            .align_y(iced::Center)
            .into()
    };
    let detail: Element<'_, Message> = if timer.fresh() {
        let mut lengths = row![].spacing(12);
        for (work, minutes, label) in [
            (true, timer.work_minutes, "Work"),
            (false, timer.break_minutes, "Break"),
        ] {
            lengths = lengths.push(
                row![
                    text(format!("{label}: {minutes}m"))
                        .size(12)
                        .style(text::secondary),
                    button(text("−").size(13))
                        .padding([0, 7])
                        .style(crate::ui::controls::ghost)
                        .on_press(msg(Action::PomodoroMinutes(work, -1))),
                    button(text("+").size(13))
                        .padding([0, 7])
                        .style(crate::ui::controls::ghost)
                        .on_press(msg(Action::PomodoroMinutes(work, 1))),
                ]
                .align_y(iced::Center),
            );
        }
        lengths.wrap().into()
    } else {
        let spent = 1.0 - timer.remaining as f32 / timer.duration().max(1) as f32;
        progress_bar(0.0..=1.0, spent.clamp(0.0, 1.0))
            .girth(4)
            .style(move |theme: &Theme| {
                let p = theme.extended_palette();
                progress_bar::Style {
                    background: crate::ui::controls::edge_color(theme).into(),
                    bar: if on_break {
                        p.success.base.color
                    } else {
                        p.primary.base.color
                    }
                    .into(),
                    border: iced::Border {
                        radius: 2.0.into(),
                        ..Default::default()
                    },
                }
            })
            .into()
    };
    container(column![top, detail].spacing(6))
        .padding([8, 10])
        .width(Fill)
        .style(crate::ui::controls::card_surface)
        .into()
}

/// The block's tasks at a glance: tick one off, see actual against planned
/// time, and start or stop it, with the current task standing out.
fn task_list<'a>(
    state: &'a State,
    block: &'a Event,
    metrics: &Metrics,
    narrow: bool,
    short: bool,
) -> Element<'a, Message> {
    let tasks = tb::assigned_all(state, block);
    if tasks.is_empty() {
        return container(
            text("No tasks assigned. Open this block to plan your work.")
                .size(14)
                .style(text::secondary),
        )
        .padding(10)
        .width(Fill)
        .style(crate::ui::controls::card_surface)
        .into();
    }
    let now = state
        .timeblocking
        .work_now
        .unwrap_or_else(crate::clock::utc_now);
    let current = tb::current_task(state, block).map(|task| task.id.as_str());
    let mut lines = column![].spacing(2);
    for task in tasks {
        lines = lines.push(crate::ui::controls::locked(
            task_line(state, task, now, current == Some(task.id.as_str()), narrow),
            task.details.sending,
        ));
    }
    let mut card = column![].spacing(2);
    if !narrow {
        card = card.push(
            row![
                Space::new().width(Fill),
                text("ACTUAL")
                    .size(10)
                    .style(text::secondary)
                    .width(ACTUAL_WIDTH)
                    .align_x(iced::Right),
                text("PLANNED")
                    .size(10)
                    .style(text::secondary)
                    .width(PLANNED_WIDTH)
                    .align_x(iced::Right),
                Space::new().width(ACTION_WIDTH),
            ]
            .spacing(LINE_SPACING)
            .padding([0, 8]),
        );
    }
    // A long list scrolls by itself so the timeline below keeps its share.
    let list: Element<'a, Message> = if short {
        lines.into()
    } else {
        container(crate::ui::controls::scrollable(lines))
            .max_height((metrics.size.height * 0.4).max(160.0))
            .into()
    };
    container(card.push(list))
        .padding(6)
        .width(Fill)
        .style(crate::ui::controls::card_surface)
        .into()
}

fn task_line<'a>(
    state: &'a State,
    task: &'a Event,
    now: DateTime<Utc>,
    current: bool,
    narrow: bool,
) -> Element<'a, Message> {
    let key = tb::task_key(task);
    let log = &state.config.task_work;
    let running = log.running(&key);
    let seconds = log.seconds(&key, now);
    let minutes = tb::metadata(task).minutes;
    let overtime = minutes > 0 && seconds > u64::from(minutes) * 60;
    let done = tb::task_status(task) == tb::TaskStatus::Completed;
    let mut check = button(text(if done { "✓" } else { "" }).size(13).center())
        .width(22)
        .height(22)
        .padding(0)
        .style(move |theme: &Theme, status| {
            let p = theme.extended_palette();
            let edge = match status {
                button::Status::Hovered | button::Status::Pressed => p.success.base.color,
                _ => p.background.strong.color,
            };
            button::Style {
                background: done.then(|| p.success.base.color.into()),
                text_color: p.success.base.text,
                border: iced::Border {
                    color: if done { p.success.base.color } else { edge },
                    width: 1.5,
                    radius: 11.0.into(),
                },
                ..Default::default()
            }
        });
    if state.timeblocking.busy.is_empty() {
        check = check.on_press(Message::Task(TaskMessage::TaskToggled(task.id.clone())));
    }
    let title = button(title_text(
        task,
        if current { 15.0 } else { 14.0 },
        current,
        text::Wrapping::Word,
    ))
    .width(Fill)
    .padding([2, 4])
    .style(crate::ui::controls::ghost)
    .on_press(Message::Event(EventMessage::EventOpened(task.id.clone())));
    let actual = text(if seconds == 0 && !running {
        "--:--".to_owned()
    } else {
        task_work::clock(seconds)
    })
    .size(if current { 14 } else { 13 })
    .style(if overtime && !task.completed {
        text::danger
    } else if running {
        text::success
    } else if seconds == 0 || task.completed {
        text::secondary
    } else {
        text::default
    });
    let planned_label = match (minutes, narrow) {
        (0, false) => "--:--".to_owned(),
        (0, true) => "/ --:--".to_owned(),
        (minutes, false) => super::inbox::format_duration(minutes),
        (minutes, true) => format!("/ {}", super::inbox::format_duration(minutes)),
    };
    let mut planned = button(text(planned_label).size(13).style(text::secondary))
        .padding([2, 2])
        .style(crate::ui::controls::ghost);
    if !state.timeblocking.busy.contains_task(&task.id) {
        planned = planned.on_press(msg(Action::EditEstimate(task.id.clone())));
    }
    let action: Element<'a, Message> = if task.completed {
        Space::new().width(ACTION_WIDTH).into()
    } else {
        let on_break = state.config.pomodoro.running()
            && state.config.pomodoro.phase == crate::pomodoro::Phase::Break;
        let mut control = button(
            text(if running {
                "Stop"
            } else if on_break {
                "Next"
            } else {
                "Start"
            })
            .size(13)
            .center()
            .width(Fill),
        )
        .width(ACTION_WIDTH)
        .padding([3, 4])
        .style(if running {
            crate::ui::controls::danger
        } else {
            crate::ui::controls::primary
        });
        if running || !state.timeblocking.busy.contains_task(&task.id) {
            control = control.on_press(msg(if running {
                Action::StopWork(task.id.clone())
            } else {
                Action::StartWork(task.id.clone())
            }));
        }
        control.into()
    };
    let content: Element<'a, Message> = if narrow {
        column![
            row![check, title].spacing(4).align_y(iced::Center),
            row![actual, planned, Space::new().width(Fill), action]
                .spacing(4)
                .align_y(iced::Center),
        ]
        .spacing(2)
        .into()
    } else {
        row![
            check,
            title,
            container(actual)
                .width(ACTUAL_WIDTH)
                .align_right(ACTUAL_WIDTH),
            container(planned)
                .width(PLANNED_WIDTH)
                .align_right(PLANNED_WIDTH),
            action,
        ]
        .spacing(LINE_SPACING)
        .align_y(iced::Center)
        .into()
    };
    container(content)
        .padding([3, 8])
        .width(Fill)
        .style(move |theme: &Theme| container::Style {
            background: current.then(|| theme.extended_palette().background.weak.color.into()),
            border: iced::Border {
                radius: crate::ui::controls::RADIUS_ROW.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// A task's title in the focus view. Ignored tasks are struck through and
/// finished ones fade, wherever the title is drawn.
fn title_text<'a>(
    task: &'a Event,
    size: f32,
    bold: bool,
    wrapping: text::Wrapping,
) -> Element<'a, Message> {
    let font = iced::Font {
        weight: if bold {
            iced::font::Weight::Bold
        } else {
            iced::font::Weight::Normal
        },
        ..iced::Font::default()
    };
    if tb::task_status(task) == tb::TaskStatus::Ignored {
        rich_text::<(), _, _, _>([span(task.title.as_str()).strikethrough(true).font(font)])
            .size(size)
            .width(Fill)
            .wrapping(wrapping)
            .style(text::secondary)
            .into()
    } else {
        text(&task.title)
            .size(size)
            .font(font)
            .width(Fill)
            .wrapping(wrapping)
            .style(if task.completed {
                text::secondary
            } else {
                text::default
            })
            .into()
    }
}

/// What the block has lost to distractions, and a quick way to add to it.
fn distractions<'a>(state: &'a State, block: &'a Event) -> Element<'a, Message> {
    let (count, minutes) = state.config.task_work.distracted(&tb::block_key(block));
    let summary = match count {
        0 => "No distractions".to_owned(),
        1 => format!("Distracted {minutes}m · once"),
        count => format!("Distracted {minutes}m · {count} times"),
    };
    let mut line = row![text(summary).size(13).style(text::secondary).width(Fill)]
        .spacing(4)
        .align_y(iced::Center);
    if count > 0 {
        line = line.push(
            button(text("Undo").size(13))
                .on_press(msg(Action::UndoDistraction))
                .style(crate::ui::controls::ghost),
        );
    }
    container(
        line.push(
            button(text("I got distracted").size(13))
                .on_press(msg(Action::OpenDistraction))
                .style(crate::ui::controls::secondary),
        ),
    )
    .padding([6, 10])
    .width(Fill)
    .style(crate::ui::controls::card_surface)
    .into()
}

/// Ask how long a distraction lasted. One press records it and closes.
pub fn distraction_overlay<'a>(metrics: &Metrics) -> Element<'a, Message> {
    let mut choices = row![].spacing(6);
    for minutes in task_work::DISTRACTION_MINUTES {
        choices = choices.push(
            button(text(format!("+{minutes}m")).size(13))
                .on_press(msg(Action::AddDistraction(minutes)))
                .style(crate::ui::controls::primary),
        );
    }
    let card = container(
        column![
            row![
                text("How long were you distracted?").size(16).width(Fill),
                button(text("Cancel").size(13))
                    .on_press(msg(Action::CloseDistraction))
                    .style(crate::ui::controls::ghost),
            ]
            .spacing(8)
            .align_y(iced::Center),
            choices.wrap().vertical_spacing(6),
        ]
        .spacing(10),
    )
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
                    background: Some(crate::ui::controls::SCRIM.into()),
                    ..Default::default()
                }),
        )
        .on_press(msg(Action::CloseDistraction))
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
                .style(crate::ui::controls::ghost),
        ],
        text(session.task_title.clone()).size(if width < 360.0 { 18 } else { 23 }),
        text(date).size(13).style(text::secondary),
        row![
            column![
                text("Start").size(12).style(text::secondary),
                super::controls::time(
                    state.config.appearance.time_format,
                    &state.timeblocking.session_start,
                    true,
                    metrics.label(),
                    |value| msg(Action::SessionStart(value))
                ),
            ]
            .spacing(3)
            .width(Fill),
            column![
                text("End").size(12).style(text::secondary),
                container(super::controls::time(
                    state.config.appearance.time_format,
                    &state.timeblocking.session_end,
                    true,
                    metrics.label(),
                    |value| msg(Action::SessionEnd(value))
                )),
            ]
            .spacing(3)
            .width(Fill),
        ]
        .spacing(8),
        button(text("Still working — clear end time").size(11))
            .style(crate::ui::controls::ghost)
            .on_press(msg(Action::SessionEnd(String::new()))),
    ]
    .spacing(10);
    if let Some(error) = &state.timeblocking.session_error {
        body = body.push(text(error).size(12).style(text::danger));
    }
    body = body.push(
        row![
            button("Save")
                .on_press(msg(Action::SaveSession))
                .style(crate::ui::controls::primary)
                .width(Fill),
            button("Delete")
                .on_press(msg(Action::DeleteSession))
                .style(crate::ui::controls::danger)
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
                background: Some(palette.background.weakest.color.into()),
                text_color: Some(palette.background.base.text),
                border: iced::Border {
                    radius: crate::ui::controls::RADIUS_CARD.into(),
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
                    background: Some(crate::ui::controls::SCRIM.into()),
                    ..Default::default()
                }),
        )
        .on_press(msg(Action::CloseSession));
    let foreground = container(mouse_area(card).on_press(Message::Ignored))
        .center_x(Fill)
        .center_y(Fill);
    stack![backdrop, foreground].into()
}

/// The room a task takes on the timeline: its estimate, or a short default.
fn slot_seconds(task: &Event) -> f32 {
    match tb::metadata(task).minutes {
        0 => 15.0 * 60.0,
        minutes => minutes as f32 * 60.0,
    }
}

/// Pixels per second that fit the block and every task into `height`. Tasks
/// estimated past the block's end shrink the scale rather than overflowing.
fn fit_scale(height: f32, duration: f32, slots: &[f32]) -> f32 {
    let span = duration.max(slots.iter().sum());
    (height - 2.0 * TIMELINE_INSET).max(1.0) / span.max(1.0)
}

/// The block's tasks in order against the clock, fitted to the height it is
/// given. It never scrolls, so a card holds nothing but the task's title; the
/// gutter names the time each task starts and ends.
fn timeline<'a>(state: &'a State, block: &'a Event, size: Size) -> Element<'a, Message> {
    if size.height < 48.0 {
        return Space::new().width(0.0).height(0.0).into();
    }
    let tasks = tb::assigned_all(state, block);
    let duration = (block.end - block.start).num_seconds().max(60) as f32;
    let slots: Vec<f32> = tasks.iter().map(|task| slot_seconds(task)).collect();
    let scale = fit_scale(size.height, duration, &slots);
    let height = Length::Fixed(size.height);
    let now = state
        .timeblocking
        .work_now
        .unwrap_or_else(crate::clock::utc_now);
    let bars = session_bars(state, block, now);
    let lane = if bars.is_empty() {
        0.0
    } else {
        SESSION_LANE + SESSION_GAP
    };
    let mut marks = vec![0.0, duration];
    let mut elapsed = 0.0;
    for seconds in &slots {
        elapsed += seconds;
        marks.push(elapsed);
    }
    marks.sort_by(f32::total_cmp);
    marks.dedup();
    let lines = Timeline {
        time_format: state.config.appearance.time_format,
        start: block.start,
        end: block.end,
        now,
        scale,
        marks,
        needle: false,
    };
    let mut layers: Vec<Element<'a, Message>> =
        vec![canvas(lines).width(Fill).height(height).into()];
    if !bars.is_empty() {
        layers.push(
            canvas(WorkLane {
                start: block.start,
                end: block.end,
                scale,
                bars: bars.clone(),
            })
            .width(Fill)
            .height(height)
            .into(),
        );
    }
    let gutter = if size.width < 280.0 { 47 } else { 67 };
    let mut offset = TIMELINE_INSET;
    for (task, seconds) in tasks.iter().zip(slots.iter()) {
        layers.push(
            column![
                Space::new().height(Length::Fixed(offset)),
                row![
                    Space::new().width(gutter),
                    crate::ui::controls::locked(
                        timeline_card(state, task, (seconds * scale - 2.0).max(1.0)),
                        task.details.sending,
                    ),
                    Space::new().width(8.0 + lane)
                ]
            ]
            .into(),
        );
        offset += seconds * scale;
    }
    layers.push(
        canvas(Timeline {
            time_format: state.config.appearance.time_format,
            start: block.start,
            end: block.end,
            now,
            scale,
            marks: Vec::new(),
            needle: true,
        })
        .width(Fill)
        .height(height)
        .into(),
    );
    stack(layers).width(Fill).height(height).into()
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
        let top = TIMELINE_INSET + start * self.scale;
        let bottom = TIMELINE_INSET + end * self.scale;
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

/// One task on the timeline: its title, and a click away from its editor.
fn timeline_card<'a>(state: &'a State, task: &'a Event, height: f32) -> Element<'a, Message> {
    let running = state.config.task_work.running(&tb::task_key(task));
    let completed = task.completed;
    // A card too short for a second line keeps one, cut off, over a wrapped
    // first line that would hide half the title.
    let title = title_text(
        task,
        if height < 20.0 { 11.0 } else { 13.0 },
        false,
        if height < 36.0 {
            text::Wrapping::None
        } else {
            text::Wrapping::Word
        },
    );
    // A tall card's title sits by the time it starts, as on the calendar.
    let title = container(title)
        .align_y(if height < 36.0 {
            iced::Alignment::Center
        } else {
            iced::Alignment::Start
        })
        .padding(if height < 36.0 { [0, 0] } else { [4, 0] })
        .width(Fill)
        .height(Fill)
        .clip(true);
    button(title)
        .padding([0, 8])
        .width(Fill)
        .height(Length::Fixed(height))
        .on_press(Message::Event(EventMessage::EventOpened(task.id.clone())))
        .style(move |theme: &Theme, status| {
            let p = theme.extended_palette();
            let card = crate::ui::controls::card_color(theme);
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(
                    if running {
                        p.primary.weak.color
                    } else if completed || hovered {
                        p.background.weak.color
                    } else {
                        card.color
                    }
                    .into(),
                ),
                text_color: if running {
                    p.primary.weak.text
                } else {
                    card.text
                },
                border: iced::Border {
                    color: if running {
                        p.primary.strong.color
                    } else {
                        crate::ui::controls::edge_color(theme)
                    },
                    width: 1.0,
                    radius: crate::ui::controls::RADIUS_SMALL.into(),
                },
                ..Default::default()
            }
        })
        .into()
}
struct Timeline {
    time_format: TimeFormat,
    start: DateTime<Local>,
    end: DateTime<Local>,
    now: DateTime<Utc>,
    scale: f32,
    /// Seconds into the block worth naming: where each task starts and ends.
    marks: Vec<f32>,
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
            let y = TIMELINE_INSET + elapsed.clamp(0.0, duration) * self.scale;
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
            let content = format!(
                "Now {}{}",
                crate::calendar::time_format::clock(
                    self.now.with_timezone(&Local),
                    self.time_format
                ),
                suffix
            );
            // A pill at the far end, where it is least likely to sit on a title.
            let size = Size::new(content.chars().count() as f32 * 6.0 + 10.0, 16.0);
            let corner = Point::new(
                (bounds.width - size.width).max(gutter),
                (y - size.height / 2.0).clamp(0.0, (bounds.height - size.height).max(0.0)),
            );
            frame.fill(&Path::rounded_rectangle(corner, size, 8.0.into()), red);
            frame.fill_text(Text {
                content,
                position: Point::new(corner.x + size.width / 2.0, corner.y + size.height / 2.0),
                color: Color::WHITE,
                size: iced::Pixels(10.0),
                align_x: iced::alignment::Horizontal::Center.into(),
                align_y: iced::alignment::Vertical::Center,
                ..Default::default()
            });
        } else {
            let palette = theme.extended_palette();
            let mut labelled = f32::NEG_INFINITY;
            for &seconds in &self.marks {
                let y = TIMELINE_INSET + seconds * self.scale;
                if seconds == duration {
                    line(
                        &mut frame,
                        y,
                        palette.background.base.text.scale_alpha(0.5),
                        1.5,
                    );
                } else {
                    line(&mut frame, y, palette.background.strong.color, 1.0);
                }
                // Short tasks can start too close together to name each one.
                if y - labelled < MARK_SPACING {
                    continue;
                }
                labelled = y;
                let when = self.start + chrono::Duration::seconds(seconds as i64);
                frame.fill_text(Text {
                    content: crate::calendar::time_format::clock(when, self.time_format),
                    position: Point::new(2.0, y),
                    color: palette.background.base.text.scale_alpha(0.6),
                    size: iced::Pixels(11.0),
                    align_y: iced::alignment::Vertical::Center,
                    ..Default::default()
                });
            }
        }
        vec![frame.into_geometry()]
    }
}
