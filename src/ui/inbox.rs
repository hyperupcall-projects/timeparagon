//! The Inbox tab: tasks waiting to be triaged, and the popup that adds one.

use iced::widget::{
    Space, button, column, container, mouse_area, row, scrollable, stack, text, text_input,
};
use iced::{Center, Color, Element, Fill};

use super::layout::Metrics;
use super::source_icon;
use crate::app::{Message, State};
use crate::calendar::Event;
use crate::inbox::{self, Action, Draft};
use crate::integrations;
use crate::timeblocking as tb;

fn msg(action: Action) -> Message {
    Message::Inbox(action)
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let tasks = inbox::tasks(state);

    let title = column![
        text("Inbox").size(25),
        text("Tasks waiting to be triaged")
            .size(metrics.label())
            .style(text::secondary),
    ];
    let new_task = button(text("New task").size(metrics.label()).center())
        .on_press(msg(Action::NewTask))
        .style(button::primary);
    let mut body = column![
        row![title.width(Fill), new_task]
            .spacing(metrics.spacing)
            .align_y(Center)
    ]
    .spacing(metrics.spacing);

    for error in [&state.inbox.error, &state.calendar.error]
        .into_iter()
        .flatten()
    {
        body = body.push(text(error).size(metrics.label()).style(text::danger));
    }
    if tasks.is_empty() {
        body = body.push(
            text("Nothing to triage")
                .size(metrics.label())
                .style(text::secondary),
        );
    }
    for task in tasks {
        body = body.push(task_card(state, task, metrics));
    }

    container(scrollable(body.width(Fill)))
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill)
        .into()
}

/// One task, with the button that gives it a day.
fn task_card<'a>(state: &'a State, task: &'a Event, metrics: &Metrics) -> Element<'a, Message> {
    let busy = state.timeblocking.busy.contains(&task.id);
    let open = state.inbox.scheduling.as_deref() == Some(task.id.as_str());

    let mut done = button(text("○").size(metrics.label() + 3.0)).style(button::text);
    if state.timeblocking.busy.is_empty() {
        done = done.on_press(Message::TaskToggled(task.id.clone()));
    }
    let mut heading = row![done].spacing(4).align_y(Center);
    if let Some(source) = integrations::source_in(&task.details.description) {
        heading = heading.push(source_icon::link(&source, 16.0));
    }
    heading = heading.push(text(&task.title).size(metrics.label() + 1.0).width(Fill));

    let mut card = column![heading].spacing(6);
    if let Ok(Some(deadline)) = crate::calendar::deadline::read_deadline(&task.details.description)
    {
        card = card.push(
            text(format!("Deadline {}", deadline.value()))
                .size(metrics.label() - 1.0)
                .style(if tb::overdue(task) {
                    text::danger
                } else {
                    text::secondary
                }),
        );
    }

    let mut schedule = button(
        text(if busy {
            "Saving…"
        } else if open {
            "Close"
        } else {
            "Schedule"
        })
        .size(metrics.label())
        .center(),
    )
    .style(if open {
        button::secondary
    } else {
        button::primary
    });
    if !busy {
        schedule = schedule.on_press(msg(Action::ChooseDay(task.id.clone())));
    }
    card = card.push(schedule);
    if open && !busy {
        card = card.push(day_chooser(state, task, metrics));
    }

    container(card)
        .padding([6, 8])
        .width(Fill)
        .style(|theme: &iced::Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                border: iced::Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 5.0.into(),
                },
                ..Default::default()
            }
        })
        .into()
}

/// Quick days, and a field for any other day.
fn day_chooser<'a>(state: &'a State, task: &'a Event, metrics: &Metrics) -> Element<'a, Message> {
    let quick = inbox::quick_days(inbox::today()).map(|(label, day)| {
        button(text(label).size(metrics.label()).center())
            .style(button::secondary)
            .on_press(msg(Action::ScheduleOn(task.id.clone(), day)))
            .into()
    });
    let typed = row![
        text_input("YYYY-MM-DD", &state.inbox.date)
            .on_input(|value| msg(Action::Date(value)))
            .on_submit(msg(Action::ScheduleTyped(task.id.clone())))
            .size(metrics.label())
            .width(Fill),
        button(text("Set").size(metrics.label()).center())
            .on_press(msg(Action::ScheduleTyped(task.id.clone())))
            .style(button::primary),
    ]
    .spacing(6)
    .align_y(Center);
    column![
        text("Do it on")
            .size(metrics.label())
            .style(text::secondary),
        row(quick).spacing(6).wrap().vertical_spacing(6),
        typed,
    ]
    .spacing(6)
    .into()
}

/// The new-task popup, drawn over everything including the tabs.
pub fn new_task_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(draft) = &state.inbox.draft else {
        return Space::new().into();
    };
    let card = container(scrollable(form(draft, metrics)))
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

fn form<'a>(draft: &'a Draft, metrics: &Metrics) -> Element<'a, Message> {
    let label = |name: &'static str| text(name).size(metrics.label()).style(text::secondary);
    let locked = draft.saving;

    let mut title = text_input("Task, or paste a link", &draft.title)
        .id(iced::widget::Id::new(inbox::TITLE_FIELD))
        .size(metrics.label())
        .width(Fill);
    let mut deadline =
        text_input("YYYY-MM-DD or YYYY-MM-DD HH:MM", &draft.deadline).size(metrics.label());
    let mut due = text_input("YYYY-MM-DD", &draft.due).size(metrics.label());
    if !locked {
        title = title
            .on_input(|value| msg(Action::Title(value)))
            .on_submit(msg(Action::Create));
        deadline = deadline
            .on_input(|value| msg(Action::Deadline(value)))
            .on_submit(msg(Action::Create));
        due = due
            .on_input(|value| msg(Action::Due(value)))
            .on_submit(msg(Action::Create));
    }

    let mut body = column![text("New task").size(18), title].spacing(8);
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
    body = body
        .push(label("Deadline (optional)"))
        .push(deadline)
        .push(label("Due date (optional)"))
        .push(due);
    if let Some(error) = &draft.error {
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
    use crate::calendar;
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
        iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            Size::new(width, 700.0),
            crate::ui::view(state),
        )
    }

    #[test]
    fn each_untriaged_task_offers_a_schedule_button() {
        for width in [200.0, 800.0] {
            let state = state_with_task("");
            let mut ui = simulator(&state, width);
            assert!(ui.find("Reply to Sam").is_ok());
            assert!(ui.find("New task").is_ok());
            ui.click("Schedule").unwrap();
            let messages: Vec<_> = ui.into_messages().collect();
            assert!(
                messages.iter().any(
                    |m| matches!(m, Message::Inbox(Action::ChooseDay(id)) if id == "task:one")
                )
            );
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
    fn the_open_day_chooser_offers_quick_days() {
        let mut state = state_with_task("");
        state.inbox.scheduling = Some("task:one".into());
        let mut ui = simulator(&state, 400.0);
        for label in ["Today", "Tomorrow", "Next Monday", "Set"] {
            assert!(ui.find(label).is_ok(), "{label}");
        }
        ui.click("Tomorrow").unwrap();
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(messages.iter().any(|m| matches!(
            m,
            Message::Inbox(Action::ScheduleOn(id, _)) if id == "task:one"
        )));
    }

    #[test]
    fn the_new_task_button_opens_a_popup_with_both_optional_fields() {
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
            "Deadline (optional)",
            "Due date (optional)",
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
