//! The Inbox tab: tasks waiting to be triaged, and the popup that adds one.

use iced::widget::{
    Space, button, column, container, mouse_area, pick_list, row, scrollable, stack, text,
};
use iced::{Center, Color, Element, Fill};

use super::layout::Metrics;
use super::source_icon;
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
    Triaged,
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    list_view(state, metrics, List::Inbox)
}

pub fn overdue<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    list_view(state, metrics, List::Overdue)
}

/// The Triaged tab: tasks confirmed out of the Inbox but not yet scheduled.
pub fn triaged<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    list_view(state, metrics, List::Triaged)
}

fn list_view<'a>(state: &'a State, metrics: &Metrics, list: List) -> Element<'a, Message> {
    let tasks = match list {
        List::Overdue => inbox::overdue_tasks(state),
        List::Inbox => inbox::tasks(state),
        List::Triaged => inbox::triaged_tasks(state),
    };
    let (heading, subtitle) = match list {
        List::Overdue => (
            "Overdue",
            "Incomplete tasks with a past scheduled time or deadline",
        ),
        List::Inbox => ("Inbox", "Tasks waiting to be triaged"),
        List::Triaged => (
            "Triaged",
            "Tasks ready to be scheduled. Probably don't need this view, triaged tasks should show in projects and be schedule from there.",
        ),
    };

    let title = super::controls::page_heading(heading, subtitle, Some(tasks.len()), metrics);
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
                pick_list(
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
    if tasks.is_empty() {
        body = body.push(
            text(match list {
                List::Overdue => "Nothing overdue",
                List::Inbox => "Nothing to triage",
                List::Triaged => "Nothing triaged yet",
            })
            .size(metrics.label())
            .style(text::secondary),
        );
    }
    for task in tasks {
        body = body.push(task_card(state, task, metrics, list));
    }

    container(scrollable(body.width(Fill)))
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill)
        .into()
}

/// One task and its metadata. Inbox actions live in the task's details pane.
fn task_card<'a>(
    state: &'a State,
    task: &'a Event,
    metrics: &Metrics,
    list: List,
) -> Element<'a, Message> {
    let busy = state.timeblocking.busy.contains(&task.id);
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
        .style(button::text)
        .on_press(Message::EventOpened(task.id.clone())),
    );

    let mut card = column![heading].spacing(4);
    if list != List::Overdue {
        for row in metadata_rows(state, task, metrics, list == List::Inbox) {
            card = card.push(row);
        }
    }
    if list == List::Triaged {
        card = card.push(actions(task, metrics, busy));
    }

    container(card)
        .padding([6, 8])
        .width(Fill)
        .style(move |theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                border: iced::Border {
                    color: if shown {
                        palette.primary.base.color
                    } else {
                        palette.background.strong.color
                    },
                    width: 1.0,
                    radius: 5.0.into(),
                },
                ..super::controls::card_surface(theme)
            }
        })
        .into()
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
                time.with_timezone(&Local)
                    .format(if state.config.appearance.use_24_hour {
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

    // The notes deadline.
    if let Ok(Some(deadline)) = crate::calendar::deadline::read_deadline(&task.details.description)
    {
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
    if let Some(scheduled) = scheduled_label(task, state.config.appearance.use_24_hour) {
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
fn format_duration(minutes: u32) -> String {
    let hours = minutes / 60;
    let minutes = minutes % 60;
    match (hours, minutes) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    }
}

/// The day or time a task has been scheduled for, if it has one.
fn scheduled_label(task: &Event, use_24_hour: bool) -> Option<String> {
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
            crate::calendar::time_format::clock(task.start, use_24_hour)
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
                theme.extended_palette().background.base.color,
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
        Message::TaskToggled(task.id.clone()),
        false,
    ));
    entries.push((
        if status == tb::TaskStatus::Ignored {
            "Unignore Task"
        } else {
            "Ignore Task"
        },
        Message::TaskIgnoreToggled(task.id.clone()),
        false,
    ));
    entries.push((
        "Delete Task",
        Message::TaskDeleteRequested(task.id.clone()),
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
    let busy = state.timeblocking.busy.contains(&task.id);

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

    let today = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
    let planned = (task.start.date_naive() != tb::UNDATED).then(|| task.start.date_naive());
    let arrow = |label: &'static str, months: i32| {
        button(text(label).size(metrics.label()).center())
            .padding([2, 8])
            .style(button::text)
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
    let weekdays = row(calendar::WEEKDAY_INITIALS.iter().map(|label| {
        text(*label)
            .size(metrics.label() - 1.0)
            .width(Fill)
            .center()
            .style(text::secondary)
            .into()
    }))
    .spacing(2.0)
    .width(Fill);
    let weeks = column(
        calendar::month_grid(month)
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
            let mut style = button::text(theme, status);
            if is_today {
                style.border = iced::Border {
                    color: theme.palette().primary,
                    width: 1.0,
                    radius: 2.0.into(),
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
    let card = container(scrollable(form(state, draft, metrics)))
        .padding(16)
        .width(Fill)
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
            Message::TaskPopoverBounds(iced::Rectangle {
                x: 0.0,
                y: (viewport_height - size.height).max(0.0),
                width: size.width,
                height: size.height,
            })
        };
        return container(
            iced::widget::sensor(card.id("dock-task-card"))
                .key(viewport_height.to_bits())
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
                    background: Some(Color::BLACK.scale_alpha(0.58).into()),
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
                    .style(button::text)
                    .on_press(msg(Action::ClearSource)),
            );
        }
        body = body.push(chip);
    }
    if let Some(error) = draft.error.as_ref().or(state.calendar.error.as_ref()) {
        body = body.push(text(error).size(metrics.label()).style(text::danger));
    }

    let mut add = button(
        text(if draft.saving {
            "Adding…"
        } else if draft.fetching {
            "Reading link…"
        } else {
            "Add task"
        })
        .size(metrics.label()),
    );
    if !draft.saving && !draft.fetching {
        add = add.on_press(msg(Action::Create));
    }
    let mut cancel = button(text("Cancel").size(metrics.label())).style(button::text);
    if !draft.saving {
        cancel = cancel.on_press(msg(Action::Cancel));
    }
    body.push(row![add, cancel].spacing(8)).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;
    use iced::Size;

    fn state_with_task(notes: &str) -> State {
        let mut state = State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        );
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
        let _ = crate::app::update(&mut state, Message::EventOpened("task:one".into()));
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
            assert!(
                ui.into_messages()
                    .any(|m| matches!(m, Message::EventOpened(id) if id == "task:one"))
            );
        }
    }

    #[test]
    fn the_details_pane_uses_the_clearable_date_picker_and_schedules_the_chosen_day() {
        let mut state = state_with_task("");
        let _ = crate::app::update(&mut state, Message::EventOpened("task:one".into()));
        assert_eq!(state.calendar.open_event.as_deref(), Some("task:one"));
        assert!(state.details_pane_open);
        assert!(state.timeblocking.date.is_empty());

        let mut ui = simulator_of(&state, Size::new(900.0, 2400.0));
        assert!(ui.find("Name").is_err());
        assert!(ui.find("Rename").is_err());
        assert!(ui.find("Scheduled day and time").is_err());
        ui.click("Choose date").unwrap();
        let picker = ui
            .into_messages()
            .find_map(|m| match m {
                Message::Picker(crate::ui::controls::Action::Open(p)) => Some(p),
                _ => None,
            })
            .expect("date picker opens");
        assert!(picker.optional);
        let _ = crate::ui::controls::update(&mut state, crate::ui::controls::Action::Open(picker));
        let mut ui = simulator_of(&state, Size::new(900.0, 2400.0));
        assert!(ui.find("Clear").is_ok());
        ui.click("15").unwrap();
        assert!(ui.into_messages().any(|m| matches!(m, Message::Picker(crate::ui::controls::Action::Select(day)) if day.day() == 15)));

        state.picker = None;
        let _ = tb::update(&mut state, tb::Action::Date("2026-10-15".into()));
        let mut ui = simulator_of(&state, Size::new(900.0, 2400.0));
        ui.click("Schedule Day").unwrap();
        assert!(ui.into_messages().any(
            |m| matches!(m, Message::TimeBlocking(tb::Action::Reschedule(id)) if id == "task:one")
        ));
    }

    #[test]
    fn inbox_shows_creation_and_modification_times() {
        let mut state = state_with_task("");
        let timestamp = chrono::DateTime::parse_from_rfc3339("2026-10-29T19:30:12Z")
            .unwrap()
            .to_utc();
        state.calendar.events[0].details.task_data = Some(crate::task_db::TaskData {
            id: 100,
            legacy_key: None,
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
                    .format(if state.config.appearance.use_24_hour {
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
    fn overdue_shows_sorting_and_only_clickable_titles() {
        for width in [320.0, 900.0] {
            let mut state = state_with_task("");
            state.screen = crate::app::Screen::Overdue;
            state.calendar.events[0].start =
                calendar::at_clock(Local::now().date_naive().pred_opt().unwrap(), 0, 0);
            let mut ui = simulator(&state, width);
            assert!(ui.find("Sort by").is_ok());
            for label in [
                "Project", "Priority", "Estimate", "Created", "Modified", "New task",
            ] {
                assert!(ui.find(label).is_err(), "{label}");
            }
            ui.click("Reply to Sam").unwrap();
            assert!(ui.into_messages().any(|message| matches!(message,
                Message::EventOpened(id) if id == "task:one")));
            let _ = crate::app::update(&mut state, Message::EventOpened("task:one".into()));
            assert_eq!(state.calendar.open_event.as_deref(), Some("task:one"));
            assert!(crate::app::shows_details(state.screen));
        }
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
    fn a_task_from_gmail_is_marked_and_a_plain_one_is_not() {
        let link = "https://mail.google.com/mail/u/0/#inbox/FMfcgzQgLjNPlfJCVRfnNkPGkLhWClCW";
        let state = state_with_task(&format!("DUE: 2026-11-03\n{link}"));
        let source = integrations::source_in(&state.calendar.events[0].details.description);
        assert_eq!(source.unwrap().url, link);
        // Drawing the row with the icon must not disturb the rest of it.
        let mut ui = simulator(&state, 300.0);
        assert!(ui.find("Reply to Sam").is_ok());
    }
}
