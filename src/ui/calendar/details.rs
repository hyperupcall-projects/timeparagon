//! The read-only details of an event.

use super::*;
use iced::widget::column;

/// The details card as a full-height pane, drawn to the right of the view.
///
/// It is the same information the old floating card held, but it lives in a
/// column of its own rather than over the calendar, so the day grid and the
/// backlog both keep their full width.
pub fn details_pane<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    if state.inbox.draft.is_none() && state.calendar.draft.as_ref().is_some_and(|d| d.editor_open) {
        return draft_editor(state, metrics);
    }
    let Some(event) = state
        .calendar
        .open_event
        .as_ref()
        .and_then(|id| state.calendar.event(id))
    else {
        return iced::widget::Space::new().width(0.0).height(0.0).into();
    };

    if event.details.sleep.is_some() {
        return sleep_editor(state, event, metrics);
    }
    let mut body = column![].spacing(metrics.spacing).width(Fill);
    if is_task(event) {
        body = body.push(super::super::timeblocking::fields(state, event, metrics));
    } else {
        // An event is edited right here. There is no separate Edit button and
        // no popup: every field the app can write is in the pane.
        body = body.push(event_editor(state, event, metrics));
    }

    let details = &event.details;
    let new_habit = event.id == crate::habits::NEW_HABIT_ID;
    if !is_task(event) && !new_habit {
        // A calendar event gets a place to search for and choose, the same as
        // a task does. Where Google holds a location no saved place matches,
        // that text stays above the button.
        let place = crate::travel::place_of(&state.config.travel, event)
            .and_then(|id| state.config.travel.place(id));
        if place.is_none() && !details.location.is_empty() {
            body = body.push(detail_line("Location", details.location.clone(), metrics));
        }
        body = body.push(
            text("Location")
                .size(metrics.label())
                .style(text::secondary),
        );
        body = body.push(super::super::travel::location_control(
            state,
            crate::travel::PickTarget::Event(event.id.clone()),
            place,
            "Add location",
            metrics,
        ));
    }
    if !is_task(event)
        && !new_habit
        && !event.details.habit
        && !crate::timeblocking::is_block(event, &state.config.calendar)
        && let Some(draft) = state
            .calendar
            .edit_draft
            .as_ref()
            .filter(|draft| draft.id == event.id && !draft.loading)
    {
        body = body.push(meeting_section(
            state,
            &draft.meeting,
            Some(event),
            metrics,
            |v0| Message::EventEdit(EventEditMessage::EventEditMeeting(v0)),
        ));
    }
    if !is_task(event)
        && !event.details.habit
        && let Some(draft) = &state.calendar.edit_draft
    {
        body = body.push(super::super::recurrence::editor(
            &draft.rule,
            draft.start.date_naive(),
            state.config.calendar.first_day_of_week,
            metrics,
            false,
            |v0| Message::EventEdit(EventEditMessage::EventEditRecurrence(v0)),
        ));
    }
    if !is_task(event)
        && let Some(draft) = &state.calendar.edit_draft
    {
        body = body.push(event_options(&draft.options, metrics, |v0| {
            Message::EventEdit(EventEditMessage::EventEditOptions(v0))
        }));
    }
    if !details.organizer.is_empty() {
        body = body.push(detail_line("Organizer", details.organizer.clone(), metrics));
    }
    // Guests are listed in the Meeting section, once a sync has said who they are.
    if !details.attendees.is_empty() && details.attendance.guests.is_empty() {
        body = body.push(detail_line("Guests", details.attendees.join("\n"), metrics));
    }

    if !new_habit {
        match crate::notes::target_of(state, event) {
            Some(target) => body = body.push(super::super::notes::section(state, target, metrics)),
            None => {
                body = body.push(
                    text("Choose a habit above to keep notes on it.")
                        .size(metrics.label())
                        .style(text::secondary),
                );
            }
        }
    }

    // The action buttons sit outside the scrollable, so they stay reachable
    // however long the description above them runs.
    let mut buttons: Vec<Element<'a, Message>> = Vec::new();

    // A Google Task gets a way to tick it off; events never do.
    if is_task(event) {
        buttons.push(super::super::timeblocking::task_actions(
            state, event, metrics,
        ));
    }

    if is_task(event) {
        for meeting in super::super::meetings::links(event) {
            buttons.push(super::super::meetings::join_button(meeting, metrics));
        }
    }
    if let Some(source) = crate::integrations::source_in(&details.description) {
        buttons.push(
            button(
                text(format!("Open in {}", source.kind.name()))
                    .size(metrics.label())
                    .center()
                    .width(Fill),
            )
            .width(Fill)
            .on_press(Message::OpenEventLink(source.url))
            .style(crate::ui::controls::secondary)
            .into(),
        );
    }
    if !is_task(event) {
        buttons.push(event_save_control(state, event, metrics));
        if !new_habit {
            buttons.push(event_scope_control(state, event, metrics, true));
        }
    }
    // Tasks are managed here now; only events get a link back to Google.
    if !is_task(event) && !details.link.is_empty() {
        let link = details.link.clone();
        buttons.push(
            button(
                text("Open in Google Calendar")
                    .size(metrics.label())
                    .center()
                    .width(Fill),
            )
            .width(Fill)
            .on_press(Message::OpenEventLink(link))
            .style(crate::ui::controls::secondary)
            .into(),
        );
    }
    buttons.push(
        button(text("Close").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(Message::Event(EventMessage::EventClosed))
            .style(crate::ui::controls::secondary)
            .into(),
    );

    // The actions are a fixed footer. The scroll is capped so the footer keeps
    // the room it needs, however long the description above it runs; the
    // buttons come first in the budget, so they can never be squeezed out.
    let button_height = metrics.button.max(40.0);
    let button_count = buttons.len() + if event.is_task() { 3 } else { 0 };
    let reserved = button_height * button_count as f32
        + metrics.spacing * button_count.saturating_sub(1) as f32;
    let scroll_max =
        (metrics.size.height - metrics.padding * 2.0 - metrics.spacing - reserved).max(0.0);

    let scrolling = container(
        crate::ui::controls::scrollable(container(body).padding(iced::Padding {
            right: 14.0,
            ..iced::Padding::ZERO
        }))
        .width(Fill)
        .height(Length::Shrink),
    )
    .max_height(scroll_max)
    .width(Fill);

    let pane = container(
        column![
            scrolling,
            column(buttons).spacing(metrics.spacing).width(Fill),
        ]
        .spacing(metrics.spacing)
        .width(Fill)
        .height(Fill),
    )
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill)
    .style(crate::ui::controls::surface);

    pane.into()
}

/// The "are you sure" prompt that covers the details card while a delete is
/// being confirmed. Clicking outside the card cancels the deletion.
pub(crate) fn delete_confirmation<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(id) = state.calendar.pending_delete.as_deref() else {
        return iced::widget::Space::new().width(0.0).height(0.0).into();
    };
    let Some(event) = state.calendar.event(id) else {
        return iced::widget::Space::new().width(0.0).height(0.0).into();
    };

    let is_task = event.is_task();

    // Clicking outside the card puts the prompt away without deleting.
    let dismiss = mouse_area(
        container(iced::widget::Space::new())
            .width(Fill)
            .height(Fill),
    )
    .on_press(Message::Event(EventMessage::EventDeleteCancelled));

    let title = if is_task {
        text("Are you sure you want to delete this task?").size(metrics.label() + 1.0)
    } else {
        text(match state.calendar.pending_delete_scope {
            calendar::SaveScope::Current => "Delete this event?",
            calendar::SaveScope::CurrentAndFuture => "Delete this and future events?",
            calendar::SaveScope::All => "Delete all events in this series?",
        })
        .size(metrics.label() + 1.0)
    };
    let event_title = text(event.title.clone())
        .size(metrics.label())
        .style(text::secondary);

    let confirm = if is_task {
        Message::Task(TaskMessage::TaskDeleteConfirmed(id.to_owned()))
    } else {
        Message::Event(EventMessage::EventDeleteConfirmed(id.to_owned()))
    };

    let mut delete = button(text("Delete").size(metrics.label()).center().width(Fill))
        .width(Fill)
        .style(crate::ui::controls::secondary);
    if state.timeblocking.busy.is_empty() {
        delete = delete.on_press(confirm);
    }
    let buttons = row![
        button(text("Cancel").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(Message::Event(EventMessage::EventDeleteCancelled))
            .style(crate::ui::controls::secondary),
        delete,
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    // The choice only exists when there are guests to tell, and it's theirs to hear.
    let notify: Element<'a, Message> = if !is_task && has_guests_to_tell(event) {
        crate::ui::controls::checkbox(state.calendar.pending_delete_notify)
            .label("Email guests about the cancellation")
            .on_toggle(|v0| Message::Event(EventMessage::EventDeleteNotify(v0)))
            .text_size(metrics.label())
            .into()
    } else {
        iced::widget::Space::new().width(0.0).height(0.0).into()
    };

    let card = container(iced::widget::opaque(
        container(
            column![
                title,
                event_title,
                text(if is_task {
                    "This removes the task from Google Tasks and its planned time."
                } else {
                    match state.calendar.pending_delete_scope {
                        calendar::SaveScope::Current => "This removes the event from Google Calendar.",
                        calendar::SaveScope::CurrentAndFuture => "This removes this event and future events in its series from Google Calendar.",
                        calendar::SaveScope::All => "This removes every event in this series from Google Calendar.",
                    }
                })
                .size(metrics.label()),
                notify,
                buttons
            ]
            .spacing(metrics.spacing),
        )
        .padding(metrics.padding)
        .width(Length::Fixed(260.0))
        .style(crate::ui::controls::surface),
    ))
    .width(Fill)
    .height(Fill)
    .center_x(Fill)
    .center_y(Fill);

    stack![dismiss, card].into()
}

fn has_guests_to_tell(event: &Event) -> bool {
    let attendance = &event.details.attendance;
    attendance.can_manage() && !attendance.guests.is_empty()
}

/// Asks whether a dragged meeting's guests should hear about its new time. The
/// chip has already moved; Cancel, or a click outside the card, puts it back.
pub(crate) fn move_confirmation<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(event) = state
        .calendar
        .pending_move
        .as_ref()
        .and_then(|pending| state.calendar.event(&pending.id))
    else {
        return iced::widget::Space::new().width(0.0).height(0.0).into();
    };
    let dismiss = mouse_area(
        container(iced::widget::Space::new())
            .width(Fill)
            .height(Fill),
    )
    .on_press(Message::Event(EventMessage::EventMoveCancelled));
    let choice = |label: &'static str, message: EventMessage| {
        button(text(label).size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(Message::Event(message))
    };
    let card = container(iced::widget::opaque(
        container(
            column![
                text("Email guests about the new time?").size(metrics.label() + 1.0),
                text(event.title.clone())
                    .size(metrics.label())
                    .style(text::secondary),
                row![
                    choice("Cancel", EventMessage::EventMoveCancelled)
                        .style(crate::ui::controls::secondary),
                    choice("Don't send", EventMessage::EventMoveConfirmed(false))
                        .style(crate::ui::controls::secondary),
                    choice("Send", EventMessage::EventMoveConfirmed(true))
                        .style(crate::ui::controls::primary),
                ]
                .spacing(6)
                .width(Fill),
            ]
            .spacing(metrics.spacing),
        )
        .padding(metrics.padding)
        .width(Length::Fixed(300.0))
        .style(crate::ui::controls::surface),
    ))
    .width(Fill)
    .height(Fill)
    .center_x(Fill)
    .center_y(Fill);

    stack![dismiss, card].into()
}

pub(super) fn is_task(event: &Event) -> bool {
    event.is_task()
}

pub(super) fn detail_line<'a>(
    label: &'a str,
    value: String,
    metrics: &Metrics,
) -> Element<'a, Message> {
    column![
        text(label)
            .size(metrics.label() - 1.0)
            .style(text::secondary),
        super::super::links::text(value, metrics.label()),
    ]
    .spacing(2.0)
    .width(Fill)
    .into()
}

pub(crate) fn schedule_line(event: &Event, time_format: TimeFormat) -> String {
    let date = event.start.format("%A, %B %d, %Y");
    if event.all_day {
        format!("{date} · All day")
    } else if event.start.date_naive() != event.end.date_naive() {
        format!(
            "{date} · {} – {}",
            calendar::time_format::event(event, time_format),
            event.end.format("%A, %B %d")
        )
    } else {
        format!(
            "{date} · {}",
            calendar::time_format::event(event, time_format)
        )
    }
}
