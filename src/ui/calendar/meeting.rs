//! The Meeting section of the event form: the video call, the guests and what
//! they may do, and the user's answer when they are one of those guests.

use super::*;
use crate::calendar::meeting::{Action, Conference, Form, Response};
use iced::widget::column;

pub(super) fn meeting_section<'a>(
    state: &'a State,
    form: &'a Form,
    event: Option<&'a Event>,
    metrics: &Metrics,
    message: fn(Action) -> Message,
) -> Element<'a, Message> {
    let size = metrics.label();
    let caption = |name: &'static str| crate::ui::controls::field_label(name, size);
    let hint = |words: &'static str| text(words).size(size - 1.0).style(text::secondary);
    let mut section = column![crate::ui::controls::field_label("Meeting", size + 1.0)]
        .spacing(metrics.spacing)
        .width(Fill);

    if let Some(event) = event.filter(|event| event.details.attendance.can_reply()) {
        section = section.push(reply_row(form, event, metrics, message));
    }

    if form.can_manage {
        let mut choices = vec![Conference::None, Conference::Meet, Conference::Zoom];
        if form.conference == Conference::Other {
            choices.push(Conference::Other);
        }
        let mut video = column![
            caption("Video call"),
            crate::ui::controls::pick_list(choices, Some(form.conference), move |choice| {
                message(Action::Conference(choice))
            })
            .text_size(size)
            .width(Fill),
        ]
        .spacing(6)
        .width(Fill);
        if form.zoom_wanted() {
            video = video.push(hint(
                "Saving opens this event on Google Calendar, to choose Add Zoom Meeting. \
                 Guests are emailed when you save it there.",
            ));
        }
        if let Some(event) = event.filter(|event| form.zoom_unfinished(&event.details)) {
            video = video.push(
                crate::ui::controls::quiet_button(
                    text("Add Zoom on Google Calendar")
                        .size(size)
                        .center()
                        .width(Fill),
                )
                .width(Fill)
                .on_press_maybe(
                    calendar::meeting::zoom_setup_url(
                        &event.details.link,
                        state
                            .calendar
                            .calendars
                            .iter()
                            .find(|calendar| calendar.primary && !calendar.is_task_list())
                            .map(|calendar| calendar.id.as_str()),
                    )
                    .map(Message::OpenEventLink),
                ),
            );
        }
        section = section.push(video);
    }

    section = section.push(guest_list(state, form, event, metrics, message));

    if form.can_manage {
        let permission = |label: &'static str, value: bool, change: Option<fn(bool) -> Action>| {
            crate::ui::controls::toggler(value)
                .label(label)
                .text_size(size)
                .on_toggle_maybe(change.map(|change| move |v| message(change(v))))
        };
        // Whoever may modify the event can do the other two regardless.
        let locked = form.permissions.modify;
        section = section.push(
            column![
                caption("Guest permissions"),
                permission(
                    "Modify event",
                    form.permissions.modify,
                    Some(Action::Modify)
                ),
                permission(
                    "Invite others",
                    form.permissions.invite_others,
                    (!locked).then_some(Action::InviteOthers as fn(bool) -> Action),
                ),
                permission(
                    "See guest-list",
                    form.permissions.see_guests,
                    (!locked).then_some(Action::SeeGuests as fn(bool) -> Action),
                ),
            ]
            .spacing(8)
            .width(Fill),
        );
        if !form.guests.is_empty() {
            section = section.push(
                crate::ui::controls::checkbox(form.notify)
                    .label("Email invitations and updates to guests")
                    .on_toggle(move |v| message(Action::Notify(v)))
                    .text_size(size),
            );
        }
    }
    section.into()
}

/// The line under a guest's name.
fn guest_details(guest: &calendar::meeting::Guest) -> String {
    let mut parts = Vec::new();
    if !guest.name.is_empty() {
        parts.push(guest.email.as_str());
    }
    if guest.optional {
        parts.push("optional");
    }
    parts.push(guest.response.label());
    parts.join(" · ")
}

/// Yes, No and Maybe, with the current answer picked out.
pub(crate) fn reply_buttons<'a>(event: &'a Event, metrics: &Metrics) -> Element<'a, Message> {
    let size = metrics.label();
    let current = event.details.attendance.own_response();
    let answer = |label: &'static str, response: Response| {
        button(text(label).size(size).center().width(Fill))
            .width(Fill)
            .on_press(Message::Event(EventMessage::EventReply(
                event.id.clone(),
                response,
            )))
            .style(if current == Some(response) {
                crate::ui::controls::primary
            } else {
                crate::ui::controls::secondary
            })
    };
    row![
        answer("Yes", Response::Accepted),
        answer("No", Response::Declined),
        answer("Maybe", Response::Tentative),
    ]
    .spacing(6)
    .width(Fill)
    .into()
}

fn reply_row<'a>(
    form: &'a Form,
    event: &'a Event,
    metrics: &Metrics,
    message: fn(Action) -> Message,
) -> Element<'a, Message> {
    let size = metrics.label();
    let mut reply = column![
        crate::ui::controls::field_label("Going?", size),
        reply_buttons(event, metrics),
    ]
    .spacing(6)
    .width(Fill);
    if event.details.recurring_event_id.is_some() {
        reply = reply.push(
            crate::ui::controls::checkbox(form.reply_to_series)
                .label("Answer for all events in the series")
                .on_toggle(move |v| message(Action::ReplyToSeries(v)))
                .text_size(size),
        );
    }
    reply.into()
}

fn guest_list<'a>(
    state: &'a State,
    form: &'a Form,
    event: Option<&'a Event>,
    metrics: &Metrics,
    message: fn(Action) -> Message,
) -> Element<'a, Message> {
    let size = metrics.label();
    let own = event.and_then(|event| event.details.attendance.own_email.as_deref());
    let mut list = column![crate::ui::controls::field_label("Guests", size)]
        .spacing(6)
        .width(Fill);
    if form.guests.is_empty() && !form.can_invite {
        list = list.push(text("No guests").size(size).style(text::secondary));
    }
    for guest in &form.guests {
        let mine = own.is_some_and(|own| own.eq_ignore_ascii_case(&guest.email));
        let mut line = row![
            column![
                text(guest.label().to_owned()).size(size),
                text(guest_details(guest))
                    .size(size - 1.0)
                    .style(text::secondary),
            ]
            .width(Fill),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        if form.can_manage && !mine {
            let email = guest.email.clone();
            line = line.push(
                crate::ui::controls::checkbox(guest.optional)
                    .label("Optional")
                    .on_toggle(move |v| message(Action::Optional(email.clone(), v)))
                    .text_size(size - 1.0),
            );
            line = line.push(
                button(text("×").size(size))
                    .on_press(message(Action::Remove(guest.email.clone())))
                    .style(crate::ui::controls::ghost),
            );
        }
        list = list.push(line);
    }
    if !form.can_invite {
        return list.into();
    }

    let taken: Vec<String> = form.guests.iter().map(|g| g.email.clone()).collect();
    let found = state.people.invitees(&form.query, &taken);
    let submit = found.first().map_or(Action::AddTyped, |first| Action::Add {
        email: first.email.clone(),
        name: first.name.clone(),
    });
    list = list.push(
        crate::ui::controls::text_input("Add guests", &form.query)
            .on_input(move |query| message(Action::Query(query)))
            .on_submit(message(submit))
            .size(size)
            .width(Fill),
    );
    if let Some(problem) = &form.problem {
        list = list.push(text(problem).size(size - 1.0).style(text::danger));
    }
    for person in found {
        let label = if person.name.is_empty() {
            person.email.clone()
        } else {
            format!("{} · {}", person.name, person.email)
        };
        list = list.push(
            button(text(label).size(size).width(Fill))
                .width(Fill)
                .on_press(message(Action::Add {
                    email: person.email,
                    name: person.name,
                }))
                .style(crate::ui::controls::ghost),
        );
    }
    list.into()
}
