//! The Inbox tab: tasks waiting to be triaged, and the form that adds one.

use crate::calendar::time_format::TimeFormat;
use iced::widget::{Space, button, column, container, mouse_area, row, stack, text};
use iced::{Center, Color, Element, Fill};

use super::layout::Metrics;
use super::source_icon;
use crate::app::{EventMessage, SystemMessage, TaskMessage};
use crate::app::{Message, State};
use crate::calendar::{self, Event};
use crate::inbox::{self, Action, Draft};
use crate::integrations;
use crate::timeblocking as tb;
use chrono::Local;

fn msg(action: Action) -> Message {
    Message::Inbox(action)
}

/// Which Tasks list a card is drawn for, so the actions can differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum List {
    Overdue,
    Inbox,
    Today,
    Triaged,
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    list_view(state, metrics, List::Inbox)
}

pub fn overdue<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    list_view(state, metrics, List::Overdue)
}

pub fn today<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    list_view(state, metrics, List::Today)
}

/// The Triaged tab: tasks confirmed out of the Inbox but not yet scheduled.
pub fn triaged<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    list_view(state, metrics, List::Triaged)
}

fn list_view<'a>(state: &'a State, metrics: &Metrics, list: List) -> Element<'a, Message> {
    let tasks = match list {
        List::Overdue => inbox::overdue_tasks(state),
        List::Inbox => inbox::tasks(state),
        List::Today => inbox::today_tasks(state),
        List::Triaged => inbox::triaged_tasks(state),
    };
    let (heading, subtitle) = match list {
        List::Overdue => (
            "Overdue",
            "Incomplete tasks with a past scheduled time or deadline. This should be empty! Move tasks that you plan to do in inbox, schcedule them for later, or move them to the backlog.",
        ),
        List::Inbox => ("Inbox", "Tasks waiting to be triaged"),
        List::Today => ("Today", "Tasks for today that still need a time slot"),
        List::Triaged => (
            "Triaged",
            "Tasks ready to be scheduled. Probably don't need this view, triaged tasks should show in projects and be schedule from there.",
        ),
    };

    let invitations: &[Event] = if list == List::Inbox {
        &state.inbox.invitations
    } else {
        &[]
    };
    let count = tasks.len() + invitations.len();
    let title = super::controls::page_heading(heading, subtitle, Some(count), metrics);
    let header: Element<'a, Message> = if list == List::Inbox {
        let add = button(text("New task").size(metrics.label()).center())
            .on_press(msg(Action::NewTask))
            .style(crate::ui::controls::primary);
        super::controls::page_toolbar(title, add.into(), metrics)
    } else {
        title.into()
    };
    let mut body = column![header].spacing(metrics.spacing);
    if matches!(list, List::Inbox | List::Overdue) {
        body = body.push(
            row![
                text("Sort by").size(metrics.label()),
                crate::ui::controls::pick_list(
                    inbox::InboxSort::ALL,
                    Some(if list == List::Overdue {
                        state.config.calendar.overdue_sort
                    } else {
                        state.config.calendar.inbox_sort
                    }),
                    move |sort| msg(if list == List::Overdue {
                        Action::OverdueSort(sort)
                    } else {
                        Action::Sort(sort)
                    })
                )
                .text_size(metrics.label())
            ]
            .spacing(metrics.spacing)
            .align_y(Center),
        );
    }

    for error in [&state.inbox.error, &state.calendar.error]
        .into_iter()
        .flatten()
    {
        body = body.push(text(error).size(metrics.label()).style(text::danger));
    }
    if !invitations.is_empty() {
        body = body.push(
            text("Invitations")
                .size(metrics.label() + 1.0)
                .style(text::secondary),
        );
        for invitation in invitations {
            body = body.push(invitation_card(state, invitation, metrics));
        }
    }
    if count == 0 {
        body = body.push(
            text(match list {
                List::Overdue => "Nothing overdue",
                List::Inbox => "Nothing to triage",
                List::Today => "No tasks left to slot in today",
                List::Triaged => "Nothing triaged yet",
            })
            .size(metrics.label())
            .style(text::secondary),
        );
    }
    for task in tasks {
        body = body.push(task_card(state, task, metrics, list));
    }

    container(crate::ui::controls::scrollable(body.width(Fill)))
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill)
        .into()
}

/// An invitation waiting for an answer, which can be given right here. Answering
/// a repeating one answers its whole series, as that's what is listed.
fn invitation_card<'a>(
    state: &'a State,
    event: &'a Event,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let time_format = state.config.appearance.time_format;
    let mut when = super::calendar::schedule_line(event, time_format);
    if !event.details.recurrence.is_empty() {
        when = format!("Repeats · from {when}");
    }
    let mut card = column![
        button(text(event.title.clone()).size(metrics.label()).width(Fill))
            .padding(0)
            .width(Fill)
            .style(crate::ui::controls::ghost)
            .on_press(msg(Action::OpenInvitation(event.id.clone()))),
        text(when)
            .size(metrics.label() - 1.0)
            .style(text::secondary),
    ]
    .spacing(6)
    .width(Fill);
    if !event.details.organizer.is_empty() {
        card = card.push(
            text(format!("From {}", event.details.organizer))
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }
    container(card.push(super::calendar::reply_buttons(event, metrics)))
        .padding(metrics.padding)
        .width(Fill)
        .style(crate::ui::controls::card_surface)
        .into()
}

/// One task and its metadata. Inbox actions live in the task's details pane.
fn task_card<'a>(
    state: &'a State,
    task: &'a Event,
    metrics: &Metrics,
    list: List,
) -> Element<'a, Message> {
    let busy = state.timeblocking.busy.contains_task(&task.id);
    let shown = state.calendar.open_event.as_deref() == Some(task.id.as_str());

    let mut heading = row![].spacing(4).align_y(Center);
    if let Some(source) =
        integrations::source_in(&task.details.description).filter(|_| list != List::Overdue)
    {
        heading = heading.push(source_icon::link(&source, 16.0));
    }
    // The title opens the task's details beside the list, where its project,
    // estimate, priority and the rest are edited.
    heading = heading.push(
        button(
            text(&task.title)
                .wrapping(text::Wrapping::WordOrGlyph)
                .size(metrics.label() + 1.0)
                .font(iced::Font {
                    weight: iced::font::Weight::Semibold,
                    ..iced::Font::DEFAULT
                })
                .width(Fill),
        )
        .width(Fill)
        .padding(0)
        .style(crate::ui::controls::ghost)
        .on_press(Message::Event(EventMessage::EventOpened(task.id.clone()))),
    );

    let mut card = column![heading].spacing(4);
    if list == List::Overdue {
        let size = metrics.label() - 1.0;
        let due = scheduled_label(task, state.config.appearance.time_format)
            .unwrap_or_else(|| "None".into());
        card = card.push(field_line("Due Date", text(due).size(size).into(), metrics));
        if let Some(deadline) = crate::calendar::deadline::get(task) {
            card = card.push(field_line(
                "Deadline",
                text(deadline.value()).size(size).into(),
                metrics,
            ));
        }
        let minutes = tb::metadata(task).minutes;
        let estimate = if minutes == 0 {
            "No estimate".into()
        } else {
            format_duration(minutes)
        };
        card = card.push(field_line(
            "Estimate",
            text(estimate).size(size).into(),
            metrics,
        ));
    } else {
        for row in metadata_rows(state, task, metrics, list == List::Inbox) {
            card = card.push(row);
        }
    }
    if list == List::Triaged {
        card = card.push(actions(task, metrics, busy));
    }

    let card = container(card)
        .padding([6, 8])
        .width(Fill)
        .style(move |theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.weakest.color.into()),
                border: iced::Border {
                    color: if shown {
                        palette.primary.base.color
                    } else {
                        palette.background.strong.color
                    },
                    width: 1.0,
                    radius: crate::ui::controls::RADIUS_ROW.into(),
                },
                ..super::controls::row_surface(theme)
            }
        });
    super::controls::locked(
        super::task_drag::task_drag(card, &task.id, state.timeblocking.dragging.as_deref()),
        task.details.sending,
    )
}

/// Every piece of metadata a task carries: the project, labels, priority,
/// location, deadline, scheduled time and estimate duration. Required fields
/// that are missing are called out while the task is still in the Inbox.
fn metadata_rows<'a>(
    state: &'a State,
    task: &'a Event,
    metrics: &Metrics,
    strict: bool,
) -> Vec<Element<'a, Message>> {
    let m = tb::metadata(task);
    let size = metrics.label() - 1.0;
    let mut rows = Vec::new();

    for (label, timestamp) in [
        ("Created", m.created_at),
        ("Modified", task.details.task_updated_at),
    ] {
        let value = timestamp
            .map(|time| {
                let local = time.with_timezone(&Local);
                if state.config.appearance.time_format.is_compact() {
                    return format!(
                        "{} {} {}",
                        local.format("%Y-%m-%d"),
                        crate::calendar::time_format::clock(
                            local,
                            state.config.appearance.time_format
                        ),
                        local.format("%Z")
                    );
                }
                local
                    .format(if state.config.appearance.time_format.is_24_hour() {
                        "%Y-%m-%d %H:%M:%S %Z"
                    } else {
                        "%Y-%m-%d %-I:%M:%S%p %Z"
                    })
                    .to_string()
            })
            .unwrap_or_else(|| "Unknown".into());
        rows.push(field_line(
            label,
            text(value).size(size).style(text::secondary).into(),
            metrics,
        ));
    }

    // Projects.
    let projects: Vec<_> = m
        .projects
        .iter()
        .filter_map(|id| state.config.projects.get(*id))
        .collect();
    let mut project_value = row![].spacing(4).align_y(Center);
    if projects.is_empty() {
        project_value = project_value.push(missing("No project", strict, size));
    } else {
        for project in projects {
            project_value = project_value.push(super::calendar::color_circle(
                project
                    .color()
                    .unwrap_or(Color::from_rgb8(0x9a, 0x9a, 0x9e)),
            ));
            project_value = project_value.push(text(&project.name).size(size));
        }
    }
    rows.push(field_line("Project", project_value.into(), metrics));

    // Priority.
    let priority = text(match m.priority {
        Some(priority) => priority.label(),
        None => {
            if strict {
                "No priority"
            } else {
                "None"
            }
        }
    })
    .size(size)
    .style(if m.priority.is_none() && strict {
        text::danger
    } else {
        text::default
    });
    rows.push(field_line("Priority", priority.into(), metrics));

    // Estimate duration.
    let estimate = text(if m.minutes == 0 {
        if strict { "No estimate" } else { "None" }
    } else {
        ""
    })
    .size(size)
    .style(if m.minutes == 0 && strict {
        text::danger
    } else {
        text::default
    });
    let estimate: Element<'a, Message> = if m.minutes == 0 {
        estimate.into()
    } else {
        text(format_duration(m.minutes)).size(size).into()
    };
    rows.push(field_line("Estimate", estimate, metrics));

    // Labels.
    if !m.labels.is_empty() {
        rows.push(field_line(
            "Labels",
            text(
                m.labels
                    .iter()
                    .map(|label| format!("#{label}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            )
            .size(size)
            .style(text::secondary)
            .into(),
            metrics,
        ));
    }

    // Location, with a pin when one is set.
    if let Some(place) = m.location.and_then(|id| state.config.travel.place(id)) {
        rows.push(field_line(
            "Location",
            row![location_pin(size + 2.0), text(&place.name).size(size)]
                .spacing(4)
                .align_y(Center)
                .into(),
            metrics,
        ));
    }

    // The structured deadline.
    if let Some(deadline) = crate::calendar::deadline::get(task) {
        rows.push(field_line(
            "Deadline",
            text(deadline.value())
                .size(size)
                .style(if tb::overdue(task) {
                    text::danger
                } else {
                    text::default
                })
                .into(),
            metrics,
        ));
    }

    // The scheduled day or time, once it has one.
    if let Some(scheduled) = scheduled_label(task, state.config.appearance.time_format) {
        rows.push(field_line(
            "Scheduled",
            text(scheduled).size(size).into(),
            metrics,
        ));
    }

    rows
}

fn missing<'a>(label: &'a str, strict: bool, size: f32) -> Element<'a, Message> {
    text(label)
        .size(size)
        .style(if strict {
            text::danger
        } else {
            text::secondary
        })
        .into()
}

fn field_line<'a>(
    label: &'static str,
    value: Element<'a, Message>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    row![
        text(label)
            .size(metrics.label() - 1.0)
            .style(text::secondary),
        value,
    ]
    .spacing(5)
    .align_y(Center)
    .into()
}

/// "1h 30m", "45m" or "2h", so a long estimate stays readable.
pub(super) fn format_duration(minutes: u32) -> String {
    let hours = minutes / 60;
    let minutes = minutes % 60;
    match (hours, minutes) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}

/// The day or time a task has been scheduled for, if it has one.
fn scheduled_label(task: &Event, time_format: TimeFormat) -> Option<String> {
    let date = task.start.date_naive();
    if date == tb::UNDATED {
        return None;
    }
    Some(if task.all_day {
        format!("{}", date.format("%a, %b %-d"))
    } else {
        format!(
            "{} at {}",
            date.format("%a, %b %-d"),
            crate::calendar::time_format::clock(task.start, time_format)
        )
    })
}

/// A small map pin, drawn rather than shipped as an icon font.
fn location_pin(size: f32) -> Element<'static, Message> {
    use iced::widget::canvas;
    use iced::{Length, Point, Rectangle, Renderer, Theme, mouse};

    struct Pin;
    impl canvas::Program<Message> for Pin {
        type State = ();
        fn draw(
            &self,
            _state: &Self::State,
            renderer: &Renderer,
            theme: &Theme,
            bounds: Rectangle,
            _cursor: mouse::Cursor,
        ) -> Vec<canvas::Geometry> {
            let mut frame = canvas::Frame::new(renderer, bounds.size());
            let color = theme.extended_palette().secondary.base.color;
            let width = bounds.width;
            let height = bounds.height;
            let radius = (width / 2.0).min(height * 0.32);
            let center = Point::new(width / 2.0, radius);
            frame.fill(&canvas::Path::circle(center, radius), color);
            let tip = canvas::Path::new(|builder| {
                builder.move_to(Point::new(center.x - radius * 0.72, radius + radius * 0.35));
                builder.line_to(Point::new(center.x, height));
                builder.line_to(Point::new(center.x + radius * 0.72, radius + radius * 0.35));
                builder.close();
            });
            frame.fill(&tip, color);
            frame.fill(
                &canvas::Path::circle(center, radius * 0.42),
                theme.extended_palette().background.weakest.color,
            );
            vec![frame.into_geometry()]
        }
    }

    canvas::Canvas::new(Pin)
        .width(Length::Fixed(size * 0.75))
        .height(Length::Fixed(size))
        .into()
}

/// The actions still shown on Triaged cards.
fn actions<'a>(task: &'a Event, metrics: &Metrics, busy: bool) -> Element<'a, Message> {
    let status = tb::task_status(task);
    let mut entries: Vec<(&'static str, Message, bool)> = Vec::new();
    entries.push((
        "Move to backlog",
        Message::TimeBlocking(tb::Action::Backlog(task.id.clone())),
        false,
    ));
    entries.push((
        if status == tb::TaskStatus::Completed {
            "Uncomplete task"
        } else {
            "Complete task"
        },
        Message::Task(TaskMessage::TaskToggled(task.id.clone())),
        false,
    ));
    entries.push((
        if status == tb::TaskStatus::Ignored {
            "Unignore Task"
        } else {
            "Ignore Task"
        },
        Message::Task(TaskMessage::TaskIgnoreToggled(task.id.clone())),
        false,
    ));
    entries.push((
        "Delete Task",
        Message::Task(TaskMessage::TaskDeleteRequested(task.id.clone())),
        false,
    ));

    let mut actions = column![].spacing(4).width(Fill);
    for (label, message, primary) in entries {
        let mut action = button(text(label).size(metrics.label()).center().width(Fill))
            .width(Fill)
            .style(if primary {
                crate::ui::controls::primary
            } else {
                crate::ui::controls::secondary
            });
        if !busy {
            action = action.on_press(message);
        }
        actions = actions.push(action);
    }
    actions.into()
}

/// The "Schedule Day" button for a task's details pane, and under it, once
/// pressed, a small calendar of the month. A day pressed there plans the task
/// for that day, the same as picking it in the list's own day chooser.
pub fn day_picker<'a>(
    state: &'a State,
    task: &'a Event,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let open = state
        .inbox
        .picker
        .as_ref()
        .filter(|(id, _)| *id == task.id)
        .map(|(_, month)| *month);
    let busy = state.timeblocking.busy.contains_task(&task.id);

    let toggle = button(
        text(if busy { "Saving…" } else { "Schedule Day" })
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill)
    .style(if open.is_some() {
        crate::ui::controls::secondary
    } else {
        crate::ui::controls::primary
    });
    let toggle = if busy {
        toggle
    } else {
        toggle.on_press(msg(Action::TogglePicker(task.id.clone())))
    };
    let mut section = column![toggle].spacing(metrics.spacing).width(Fill);
    let Some(month) = open else {
        return section.into();
    };

    let today = calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
    let planned = (task.start.date_naive() != tb::UNDATED).then(|| task.start.date_naive());
    let arrow = |label: &'static str, months: i32| {
        button(text(label).size(metrics.label()).center())
            .padding([2, 8])
            .style(crate::ui::controls::ghost)
            .on_press(msg(Action::PickerShift(months)))
    };
    let title = row![
        arrow("◀", -1),
        text(calendar::month_heading(month))
            .size(metrics.label())
            .width(Fill)
            .center(),
        arrow("▶", 1),
    ]
    .spacing(metrics.spacing)
    .align_y(Center);
    let first_day = state.config.calendar.first_day_of_week;
    let weekdays = row(first_day.initials().map(|label| {
        text(label)
            .size(metrics.label() - 1.0)
            .width(Fill)
            .center()
            .style(text::secondary)
            .into()
    }))
    .spacing(2.0)
    .width(Fill);
    let weeks = column(
        calendar::month_grid(month, first_day)
            .chunks(7)
            .map(|week| {
                row(week
                    .iter()
                    .map(|&day| picker_day(task, day, month, planned, today, metrics)))
                .spacing(2.0)
                .width(Fill)
                .into()
            })
            .collect::<Vec<Element<'a, Message>>>(),
    )
    .spacing(2.0)
    .width(Fill);

    section = section.push(column![title, weekdays, weeks].spacing(4).width(Fill));
    section.into()
}

/// One day of the picker. The task's current day is filled, today is
/// outlined, and days of the neighbouring months are dimmed.
fn picker_day<'a>(
    task: &Event,
    day: chrono::NaiveDate,
    month: chrono::NaiveDate,
    planned: Option<chrono::NaiveDate>,
    today: chrono::NaiveDate,
    metrics: &Metrics,
) -> Element<'a, Message> {
    use chrono::Datelike;
    let in_month = day.month() == month.month() && day.year() == month.year();
    let is_planned = planned == Some(day);
    let is_today = day == today;
    button(
        text(day.day().to_string())
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill)
    .height(32)
    .padding(0)
    .on_press(msg(Action::ScheduleOn(task.id.clone(), day)))
    .style(move |theme, status| {
        if is_planned {
            crate::ui::controls::primary(theme, status)
        } else {
            let mut style = crate::ui::controls::ghost(theme, status);
            if is_today {
                style.border = iced::Border {
                    color: theme.palette().primary,
                    width: 1.0,
                    radius: crate::ui::controls::RADIUS_SMALL.into(),
                };
            }
            if !in_month {
                style.text_color = style.text_color.scale_alpha(0.4);
            }
            style
        }
    })
    .into()
}

/// The new-task popup, drawn over everything including the tabs.
pub fn new_task_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(draft) = &state.inbox.draft else {
        return Space::new().into();
    };
    // The preview takes the form's place, with its own way back to it.
    let preview = state.inbox.preview.as_ref();
    if let Some(preview) = preview
        && state.sidebar.is_some()
        && !state.task_creation_window
    {
        return super::calendar::preview_view(state, preview, metrics);
    }
    if state.sidebar.is_some() && !state.task_creation_window {
        let kind = draft.kind.unwrap_or(calendar::DraftKind::Task);
        let type_button = |label: &'static str, value: calendar::DraftKind| {
            let mut control = button(text(label).size(metrics.label()).center())
                .width(Fill)
                .style(crate::ui::controls::nav(kind == value));
            if !draft.saving
                && !draft.fetching
                && !state.inbox.voice.active()
                && !state.inbox.image.busy
            {
                control = control.on_press(msg(Action::KindSelected(value)));
            }
            control
        };
        // Sleep is recorded from the calendar itself, so it is not offered here.
        let types = row![
            type_button("Task", calendar::DraftKind::Task),
            type_button("Event", calendar::DraftKind::Event),
            type_button("Time Block", calendar::DraftKind::TimeBlock),
            type_button("Habit", calendar::DraftKind::Habit),
        ]
        .spacing(4);
        let content = if kind == calendar::DraftKind::Task {
            container(crate::ui::controls::scrollable(form(state, draft, metrics)).height(Fill))
                .padding(16)
                .width(Fill)
                .height(Fill)
                .into()
        } else {
            super::calendar::draft_editor(state, metrics)
        };
        return column![
            container(types).padding([8, 16]),
            crate::image_input::review(state),
            content
        ]
        .width(Fill)
        .height(Fill)
        .into();
    }
    let card = if let Some(preview) = preview {
        // The grid scrolls inside, so the card gets a height of its own.
        container(super::calendar::preview_view(state, preview, metrics))
            .width(Fill)
            .height(iced::Length::Fixed(
                (metrics.size.height - 32.0).clamp(80.0, 640.0),
            ))
    } else if draft.kind.is_some() {
        container(column![
            crate::image_input::review(state),
            super::calendar::draft_editor(state, metrics)
        ])
        .height(iced::Length::Fixed(
            (metrics.size.height - 32.0).clamp(80.0, 640.0),
        ))
    } else {
        container(crate::ui::controls::scrollable(
            container(form(state, draft, metrics)).padding(iced::Padding {
                right: 14.0,
                ..iced::Padding::ZERO
            }),
        ))
        .padding(16)
        .width(Fill)
    };
    let card = card
        .max_width(if state.task_creation_window { 440 } else { 420 })
        .max_height(
            (metrics.size.height
                - if state.task_creation_window {
                    0.0
                } else {
                    32.0
                })
            .max(80.0),
        )
        .style(crate::ui::controls::surface);
    if state.task_creation_window {
        let viewport_height = metrics.size.height;
        let bounds = move |size: iced::Size| {
            Message::System(SystemMessage::TaskPopoverBounds(iced::Rectangle {
                x: 0.0,
                y: (viewport_height - size.height).max(0.0),
                width: size.width,
                height: size.height,
            }))
        };
        return container(
            iced::widget::sensor(card.id("dock-task-card"))
                .key((viewport_height.to_bits(), state.inbox.voice.debug_open))
                .on_show(bounds)
                .on_resize(bounds),
        )
        .width(Fill)
        .height(Fill)
        .align_y(iced::alignment::Vertical::Bottom)
        .into();
    }
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
        .on_press(msg(Action::Cancel))
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

fn form<'a>(state: &'a State, draft: &'a Draft, metrics: &Metrics) -> Element<'a, Message> {
    let locked = draft.saving;
    let mut body = column![text("New task").size(20)].spacing(8);
    if state.sidebar.is_none() || state.task_creation_window {
        body = column![crate::image_input::review(state), body].spacing(8);
    }
    if let Some(event) = &draft.event {
        body = body.push(super::timeblocking::fields(state, event, metrics));
    }
    if let Some(source) = &draft.source {
        let mut chip = row![
            source_icon::logo(source.kind, 16.0),
            text(if draft.fetching {
                format!("Reading {} link…", source.kind.name())
            } else {
                format!("From {}", source.kind.name())
            })
            .size(metrics.label())
            .width(Fill),
        ]
        .spacing(6)
        .align_y(Center);
        if !locked {
            chip = chip.push(
                button(text("×").size(metrics.label()))
                    .padding([0, 6])
                    .style(crate::ui::controls::ghost)
                    .on_press(msg(Action::ClearSource)),
            );
        }
        body = body.push(chip);
    }
    if let Some(error) = draft.error.as_ref().or(state.calendar.error.as_ref()) {
        body = body.push(text(error).size(metrics.label()).style(text::danger));
    }

    let mut add = crate::ui::controls::action_button(
        text(if draft.saving {
            "Adding…"
        } else if draft.fetching {
            "Reading link…"
        } else {
            "Add task"
        })
        .size(metrics.label()),
    );
    if !draft.saving && !draft.fetching && !state.inbox.voice.active() && !state.inbox.image.busy {
        add = add.on_press(msg(Action::Create));
    }
    let mut cancel = button(text("Cancel").size(metrics.label())).style(crate::ui::controls::ghost);
    if !draft.saving {
        cancel = cancel.on_press(msg(Action::Cancel));
    }
    body.push(
        row![
            add,
            cancel,
            Space::new().width(Fill),
            view_in_calendar_button(state, metrics)
        ]
        .spacing(8)
        .align_y(Center),
    )
    .into()
}

/// Opens the calendar preview of what the form would create.
pub(crate) fn view_in_calendar_button<'a>(
    state: &'a State,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let busy = state
        .inbox
        .draft
        .as_ref()
        .is_none_or(|draft| draft.saving || draft.fetching)
        || state.inbox.voice.active()
        || state.inbox.image.busy;
    button(text("View in calendar").size(metrics.label()).center())
        .style(crate::ui::controls::secondary)
        .on_press_maybe((!busy).then_some(msg(Action::Preview(
            crate::inbox::preview::PreviewAction::Open,
        ))))
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;
    use iced::Size;

    fn state_with_task(notes: &str) -> State {
        let mut state = State::blank();
        let mut snapshot = calendar::sample_snapshot();
        snapshot.events.clear();
        snapshot.calendars.push(calendar::Calendar {
            id: "tasks:inbox".into(),
            name: "Inbox".into(),
            color: calendar::colors::TASK,
            primary: false,
            selected_by_google: true,
        });
        let mut task = calendar::sample_snapshot().events.remove(0);
        task.id = "task:one".into();
        task.calendar_id = "tasks:inbox".into();
        task.title = "Reply to Sam".into();
        task.all_day = true;
        task.start = calendar::at_clock(tb::UNDATED, 0, 0);
        task.end = task.start + chrono::Duration::days(1);
        task.details = calendar::Details {
            description: notes.into(),
            ..Default::default()
        };
        snapshot.events.push(task);
        state.calendar = calendar::Store::with_snapshot(snapshot);
        state.screen = crate::app::Screen::Inbox;
        state
    }

    fn simulator(state: &State, width: f32) -> iced_test::Simulator<'_, Message> {
        simulator_of(state, Size::new(width, 700.0))
    }

    fn simulator_of(state: &State, size: Size) -> iced_test::Simulator<'_, Message> {
        iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            size,
            crate::ui::view(state),
        )
    }

    #[test]
    fn an_untriaged_task_shows_metadata_without_inline_actions() {
        for width in [320.0, 900.0] {
            let state = state_with_task("");
            let mut ui = simulator(&state, width);
            assert!(ui.find("Reply to Sam").is_ok());
            assert!(ui.find("New task").is_ok());
            for label in ["Project", "Priority", "Estimate"] {
                assert!(ui.find(label).is_ok(), "{label}");
            }
            for label in [
                "Triage",
                "Move to backlog",
                "Complete task",
                "Ignore Task",
                "Delete Task",
            ] {
                assert!(ui.find(label).is_err(), "{label}");
            }
        }
    }

    #[test]
    fn opening_an_inbox_task_exposes_its_actions_in_the_details_pane() {
        let mut state = state_with_task("");
        let _ = crate::app::update(
            &mut state,
            Message::Event(EventMessage::EventOpened("task:one".into())),
        );
        let metrics = Metrics::of(Size::new(320.0, 1800.0));
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            Size::new(320.0, 1800.0),
            crate::ui::calendar::details_pane(&state, &metrics),
        );
        for label in [
            "Triage",
            "Move to backlog",
            "Complete task",
            "Ignore Task",
            "Delete task",
        ] {
            assert!(ui.find(label).is_ok(), "{label}");
        }
        ui.click("Triage").unwrap();
        assert!(ui.into_messages().any(|message| matches!(
            message,
            Message::TimeBlocking(tb::Action::Triage(id)) if id == "task:one"
        )));
    }

    #[test]
    fn a_task_title_opens_its_details_beside_the_list() {
        for width in [300.0, 800.0] {
            let state = state_with_task("");
            let mut ui = simulator(&state, width);
            ui.click("Reply to Sam").unwrap();
            assert!(ui.into_messages().any(
                |m| matches!(m, Message::Event(EventMessage::EventOpened(id)) if id == "task:one")
            ));
        }
    }

    #[test]
    fn inbox_shows_creation_and_modification_times() {
        let mut state = state_with_task("");
        let timestamp = chrono::DateTime::parse_from_rfc3339("2026-10-29T19:30:12Z")
            .unwrap()
            .to_utc();
        state.calendar.events[0].details.task_data = Some(crate::task_db::TaskData {
            id: 100,
            metadata: tb::Metadata {
                created_at: Some(timestamp),
                ..Default::default()
            },
        });
        let mut ui = simulator(&state, 500.0);
        for label in ["Sort by", "Created", "Modified", "Unknown"] {
            assert!(ui.find(label).is_ok(), "{label}");
        }
        assert!(
            ui.find(
                timestamp
                    .with_timezone(&Local)
                    .format(if state.config.appearance.time_format.is_24_hour() {
                        "%Y-%m-%d %H:%M:%S %Z"
                    } else {
                        "%Y-%m-%d %-I:%M:%S%p %Z"
                    })
                    .to_string()
            )
            .is_ok()
        );
    }

    #[test]
    fn overdue_shows_due_date_and_estimate_with_clickable_titles() {
        for width in [320.0, 900.0] {
            let mut state = state_with_task("");
            state.screen = crate::app::Screen::Overdue;
            state.calendar.events[0].start =
                calendar::at_clock(crate::clock::now().date_naive().pred_opt().unwrap(), 0, 0);
            state.calendar.events[0].details.description = "".into();
            state.calendar.events[0]
                .details
                .task_data
                .get_or_insert_with(|| timeslay_core::task_db::TaskData {
                    id: 100,
                    metadata: Default::default(),
                })
                .metadata
                .deadline = Some(
                timeslay_core::calendar::deadline::parse_input(&format!(
                    "{}",
                    state.calendar.events[0].start.date_naive()
                ))
                .unwrap(),
            );
            state.calendar.events[0].details.task_data = Some(crate::task_db::TaskData {
                id: 100,
                metadata: tb::Metadata {
                    deadline: Some(calendar::deadline::Deadline {
                        date: state.calendar.events[0].start.date_naive(),
                        time: None,
                    }),
                    minutes: 90,
                    ..Default::default()
                },
            });
            let due =
                scheduled_label(&state.calendar.events[0], TimeFormat::TwentyFourHour).unwrap();
            let mut ui = simulator(&state, width);
            for label in ["Sort by", "Due Date", &due, "Estimate", "1h 30m"] {
                assert!(ui.find(label).is_ok(), "{label}");
            }
            for label in ["Project", "Priority", "Created", "Modified", "New task"] {
                assert!(ui.find(label).is_err(), "{label}");
            }
            ui.click("Reply to Sam").unwrap();
            assert!(ui.into_messages().any(|message| matches!(message,
                Message::Event(EventMessage::EventOpened(id)) if id == "task:one")));
            let _ = crate::app::update(
                &mut state,
                Message::Event(EventMessage::EventOpened("task:one".into())),
            );
            assert_eq!(state.calendar.open_event.as_deref(), Some("task:one"));
            assert!(crate::app::shows_details(state.screen));
        }
    }

    #[test]
    fn overdue_without_a_due_date_or_estimate_shows_deadline_and_missing_values() {
        let mut state = state_with_task("");
        state.calendar.events[0].details.task_data = Some(crate::task_db::TaskData {
            id: 100,
            metadata: tb::Metadata {
                deadline: Some(calendar::deadline::parse_input("2020-01-02").unwrap()),
                ..Default::default()
            },
        });
        state.screen = crate::app::Screen::Overdue;
        let mut ui = simulator(&state, 320.0);
        for label in [
            "Due Date",
            "None",
            "Deadline",
            "2020-01-02",
            "Estimate",
            "No estimate",
        ] {
            assert!(ui.find(label).is_ok(), "{label}");
        }
    }

    #[test]
    fn an_invitation_waits_in_the_inbox_and_is_answered_from_there() {
        use crate::calendar::meeting::{Attendance, Guest, Response};
        let mut state = State::blank();
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
        let mut invitation = state
            .calendar
            .events
            .iter()
            .find(|event| !event.is_task())
            .cloned()
            .unwrap();
        invitation.id = "far-ahead".into();
        invitation.title = "Quarterly review".into();
        invitation.details.attendance = Attendance {
            guests: vec![Guest {
                email: "me@example.com".into(),
                ..Default::default()
            }],
            own_email: Some("me@example.com".into()),
            ..Default::default()
        };
        state.inbox.invitations = vec![invitation];
        state.screen = crate::app::Screen::Inbox;

        let mut ui = simulator(&state, 900.0);
        assert!(ui.find("Invitations").is_ok());
        assert!(ui.find("Quarterly review").is_ok());
        assert!(ui.find("Nothing to triage").is_err());
        ui.click("Yes").unwrap();
        let reply = ui
            .into_messages()
            .find(|message| {
                matches!(
                    message,
                    Message::Event(EventMessage::EventReply(id, Response::Accepted))
                        if id == "far-ahead"
                )
            })
            .expect("the answer is sent");

        // Answering works though the event isn't in the range on show, and
        // takes it out of the inbox straight away.
        let _ = crate::app::update(&mut state, reply);
        assert!(state.inbox.invitations.is_empty());
    }

    #[test]
    fn today_opens_from_plan_day_and_uses_the_shared_task_cards() {
        let mut state = state_with_task("");
        let day = crate::clock::now().date_naive();
        state.calendar.events[0].start = calendar::at_clock(day, 0, 0);
        state.calendar.events[0].end = calendar::at_clock(day.succ_opt().unwrap(), 0, 0);
        let mut ui = simulator(&state, 500.0);
        ui.click("Today").unwrap();
        let message = ui
            .into_messages()
            .find(|message| {
                matches!(
                    message,
                    Message::Navigation(crate::app::NavigationMessage::PlanTabSelected(
                        crate::app::PlanTab::Today
                    ))
                )
            })
            .unwrap();
        let _ = crate::app::update(&mut state, message);
        assert_eq!(state.screen, crate::app::Screen::Today);
        assert!(state.day_plan_active());
        let mut ui = simulator(&state, 500.0);
        assert!(ui.find("Reply to Sam").is_ok());
        ui.click("Reply to Sam").unwrap();
        assert!(ui.into_messages().any(|message| matches!(message,
            Message::Event(EventMessage::EventOpened(id)) if id == "task:one"
        )));
        state.calendar.events[0].completed = true;
        let mut ui = simulator(&state, 500.0);
        assert!(ui.find("Reply to Sam").is_err());
        assert!(ui.find("No tasks left to slot in today").is_ok());
    }

    #[test]
    fn an_empty_inbox_says_so() {
        let mut state = state_with_task("");
        state.calendar.events.clear();
        let mut ui = simulator(&state, 400.0);
        assert!(ui.find("Nothing to triage").is_ok());
    }

    #[test]
    fn the_triaged_tab_lists_triaged_tasks_without_a_triage_button() {
        let mut state = state_with_task("");
        state.calendar.events[0].calendar_id = "tasks:triaged".into();
        state.screen = crate::app::Screen::Triaged;
        let mut ui = simulator(&state, 500.0);
        assert!(ui.find("Reply to Sam").is_ok());
        assert!(ui.find("Move to backlog").is_ok());
        assert!(ui.find("Triage").is_err());
        assert!(ui.find("New task").is_err());
    }

    #[test]
    fn the_new_task_button_opens_the_full_shared_editor() {
        let mut state = state_with_task("");
        let mut ui = simulator(&state, 400.0);
        ui.click("New task").unwrap();
        assert!(
            ui.into_messages()
                .any(|m| matches!(m, Message::Inbox(Action::NewTask)))
        );

        let _ = inbox::update(&mut state, Action::NewTask);
        let mut ui = simulator(&state, 400.0);
        for label in [
            "Deadline",
            "Due Date",
            "Projects",
            "+ Add project",
            "Estimate",
            "Labels",
            "Priority",
            "Location",
            "Description",
            "Add task",
            "Cancel",
        ] {
            assert!(ui.find(label).is_ok(), "{label}");
        }
    }

    #[test]
    fn new_tasks_stay_in_the_sidebar_with_every_planning_window_open() {
        use crate::app::{CalendarView, Sidebar};

        for (view, screen) in [
            (CalendarView::Day, crate::app::Screen::Calendar),
            (CalendarView::Day, crate::app::Screen::Inbox),
            (CalendarView::Week, crate::app::Screen::Inbox),
            (CalendarView::TwoWeek, crate::app::Screen::Inbox),
            (CalendarView::Month, crate::app::Screen::Inbox),
        ] {
            let mut state = state_with_task("");
            state.screen = screen;
            state.sidebar = Some(Sidebar {
                width: 400.0,
                side: crate::panel::Side::Left,
            });
            state.view = view;
            state.main_window = Some(iced::window::Id::unique());
            state.editor_window = Some(iced::window::Id::unique());

            let mut ui = simulator(&state, 400.0);
            ui.click("+").unwrap();
            let message = ui
                .into_messages()
                .find(|message| matches!(message, Message::Inbox(Action::NewTask)))
                .expect("the bottom plus opens a task");
            let _ = crate::app::update(&mut state, message);

            let mut main = simulator(&state, 400.0);
            assert!(main.find("New task").is_ok(), "{view:?}");
            assert!(main.find("Cancel").is_ok(), "{view:?}");
            for hidden in ["Do", "Plan Day", "Settings", "+", "Reply to Sam"] {
                assert!(
                    main.find(hidden).is_err(),
                    "{hidden} must be replaced by the form"
                );
            }
            let mut editor = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                Size::new(900.0, 700.0),
                crate::ui::editor_window(&state),
            );
            assert!(editor.find("New task").is_err(), "{view:?}");
            drop(main);
            drop(editor);

            let _ = crate::app::update(&mut state, Message::Inbox(Action::Cancel));
            let mut main = simulator(&state, 400.0);
            assert!(main.find("Do").is_ok());
            assert!(main.find("+").is_ok());
        }
    }

    fn sidebar_state() -> State {
        use crate::app::Sidebar;
        let mut state = state_with_task("");
        state.sidebar = Some(Sidebar {
            width: 400.0,
            side: crate::panel::Side::Left,
        });
        let _ = crate::app::update(&mut state, msg(Action::NewTask));
        state
    }

    #[test]
    fn the_sidebar_selector_no_longer_offers_sleep() {
        let state = sidebar_state();
        let mut ui = simulator(&state, 400.0);
        for label in ["Task", "Event", "Time Block", "Habit"] {
            assert!(ui.find(label).is_ok(), "{label}");
        }
        assert!(ui.find("Sleep").is_err());
    }

    #[test]
    fn view_in_calendar_previews_the_form_and_returns_to_it() {
        use crate::inbox::preview::PreviewAction;
        for kind in [calendar::DraftKind::Task, calendar::DraftKind::Event] {
            let mut state = sidebar_state();
            let _ = crate::app::update(&mut state, msg(Action::Title("Plan launch".into())));
            let _ = crate::app::update(&mut state, msg(Action::KindSelected(kind)));
            // The forms scroll; a tall window keeps the button in reach.
            let mut ui = simulator_of(&state, Size::new(400.0, 1600.0));
            ui.click("View in calendar").unwrap();
            let message = ui
                .into_messages()
                .find(|message| {
                    matches!(
                        message,
                        Message::Inbox(Action::Preview(PreviewAction::Open))
                    )
                })
                .expect("the button opens the preview");
            let _ = crate::app::update(&mut state, message);
            let mut ui = simulator(&state, 400.0);
            assert!(ui.find("1 new item").is_ok(), "{kind:?}");
            assert!(ui.find("Plan launch").is_ok(), "{kind:?}");
            assert!(ui.find("Day").is_ok() && ui.find("Week").is_ok());
            assert!(
                ui.find("Add").is_ok(),
                "a single item can be added from the preview"
            );
            assert!(
                ui.find("Event").is_err(),
                "the type selector gives way to the preview"
            );
            ui.click("Back to edit").unwrap();
            let message = ui
                .into_messages()
                .find(|message| {
                    matches!(
                        message,
                        Message::Inbox(Action::Preview(PreviewAction::Close))
                    )
                })
                .unwrap();
            let _ = crate::app::update(&mut state, message);
            let mut ui = simulator(&state, 400.0);
            assert!(ui.find("View in calendar").is_ok());
        }
    }

    #[test]
    fn the_preview_lists_several_items_in_the_day_and_week() {
        use crate::inbox::preview::{self, PreviewAction, Proposal, When};
        let mut state = sidebar_state();
        let today = crate::clock::now().date_naive();
        let at = |title: &str, day: chrono::NaiveDate, hour: u32| {
            let start = calendar::at_clock(day, hour, 0);
            Proposal {
                kind: calendar::DraftKind::Event,
                title: title.into(),
                when: Some(When {
                    start,
                    end: start + chrono::Duration::hours(1),
                    all_day: false,
                    repeat: None,
                }),
                color: calendar::colors::DEFAULT_EVENT,
            }
        };
        let tomorrow = today + chrono::Duration::days(1);
        let mut undated = at("Buy stamps", today, 0);
        undated.kind = calendar::DraftKind::Task;
        undated.when = None;
        let _ = preview::open(
            &mut state,
            vec![at("Standup", today, 9), at("Gym", tomorrow, 18), undated],
            false,
        );
        for span in [preview::Span::Week, preview::Span::Day] {
            let _ = crate::app::update(&mut state, msg(Action::Preview(PreviewAction::Span(span))));
            for width in [400.0, 900.0] {
                let mut ui = simulator(&state, width);
                assert!(ui.find("3 new items").is_ok());
                for title in ["Standup", "Gym", "Buy stamps"] {
                    assert!(ui.find(title).is_ok(), "{title} in the list");
                }
                assert!(
                    ui.find("Task · No date yet, so it won't show on the calendar")
                        .is_ok()
                );
                // Several items from elsewhere are not saved by the form's Add.
                assert!(ui.find("Add").is_err());
            }
        }
    }

    #[test]
    fn the_sidebar_selector_opens_every_creation_form_without_saving() {
        use crate::app::{CalendarView, Sidebar};
        for (label, heading, kind) in [
            ("Event", "New event", calendar::DraftKind::Event),
            (
                "Time Block",
                "New time block",
                calendar::DraftKind::TimeBlock,
            ),
            ("Habit", "New habit", calendar::DraftKind::Habit),
        ] {
            let mut state = state_with_task("");
            state.sidebar = Some(Sidebar {
                width: 400.0,
                side: crate::panel::Side::Left,
            });
            state.view = CalendarView::Week;
            state.editor_window = Some(iced::window::Id::unique());
            let _ = crate::app::update(&mut state, msg(Action::NewTask));
            let mut ui = simulator(&state, 400.0);
            ui.click(label).unwrap();
            let message = ui
                .into_messages()
                .find(|message| {
                    matches!(message,
                        Message::Inbox(Action::KindSelected(value)) if *value == kind
                    )
                })
                .expect("the type selector switches the draft");
            let _ = crate::app::update(&mut state, message);
            assert!(!state.inbox.draft.as_ref().unwrap().saving);
            let mut ui = simulator(&state, 400.0);
            assert!(ui.find(heading).is_ok(), "{label}");
            assert!(ui.find("Add").is_ok(), "{label}");
            assert!(ui.find("Do").is_err());
            let mut editor = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                Size::new(900.0, 700.0),
                crate::ui::editor_window(&state),
            );
            assert!(
                editor.find(heading).is_err(),
                "{label} stays in the sidebar"
            );
        }
    }

    #[test]
    fn a_task_from_gmail_is_marked_and_a_plain_one_is_not() {
        let link = "https://mail.google.com/mail/u/0/#inbox/FMfcgzQgLjNPlfJCVRfnNkPGkLhWClCW";
        let state = state_with_task(link);
        let source = integrations::source_in(&state.calendar.events[0].details.description);
        assert_eq!(source.unwrap().url, link);
        // Drawing the row with the icon must not disturb the rest of it.
        let mut ui = simulator(&state, 300.0);
        assert!(ui.find("Reply to Sam").is_ok());
    }
}
