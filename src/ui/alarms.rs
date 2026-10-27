//! The popup that edits an alarm set on the calendar.

use chrono::Local;
use iced::font::Weight;
use iced::widget::{
    Space, button, column, container, mouse_area, row, scrollable, slider, stack, text, text_input,
};
use iced::{Alignment, Element, Fill, Font, Length, Theme};

use crate::alarms::{self, Action, Edit};
use crate::app::{Message, State};
use crate::audio::Tone;
use crate::calendar::colors;
use crate::config;

use super::layout::Metrics;
use super::settings::{choice, field, indent, repeat_label, switch};

/// Room left for the scrollbar, which is drawn over the content rather than
/// beside it.
const SCROLLBAR_GUTTER: f32 = 14.0;

fn edit(edit: Edit) -> Message {
    Message::Alarms(Action::Edit(edit))
}

/// The alarm popup: when it goes off, and everything it does when it does.
///
/// It is a card over the day view, like the new-event popup. Clicking outside
/// it, or Done, puts it away; Delete takes the alarm off the calendar.
pub fn editor_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let nothing = || Space::new().width(0.0).height(0.0).into();

    let Some(editor) = state.alarms.editor.as_ref() else {
        return nothing();
    };
    let Some(alarm) = state.alarms.get(editor.id) else {
        return nothing();
    };

    let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill))
        .on_press(Message::Alarms(Action::CloseEditor));

    let header = row![
        text("Alarm").size(metrics.label() + 1.0).width(Fill),
        button(text("✕").size(metrics.label()).center())
            .width(Length::Fixed(metrics.icon_button()))
            .height(Length::Fixed(metrics.icon_button()))
            .padding(0.0)
            .on_press(Message::Alarms(Action::CloseEditor))
            .style(button::secondary),
    ]
    .align_y(Alignment::Center);

    // Always measured from the clock, so the card keeps saying how far off the
    // alarm is for as long as it is open.
    let minutes = alarms::minutes_between(Local::now(), alarm.at);
    let countdown = column![
        text(alarms::countdown_label(minutes))
            .size(metrics.label() + 3.0)
            .color(colors::ALARM)
            .font(Font {
                weight: Weight::Bold,
                ..Font::DEFAULT
            }),
        text(alarm.at.format("%A, %B %-d").to_string())
            .size(metrics.label())
            .style(text::secondary),
    ]
    .spacing(2.0);

    let mut when = column![field(
        "Time",
        text_input("07:30", &editor.time_text)
            .on_input(|time| edit(Edit::Time(time)))
            .size(metrics.label())
            .width(Fill)
            .into(),
        metrics,
    )]
    .spacing(2.0);
    if let Some(error) = &editor.time_error {
        when = when.push(text(error).size(metrics.label() - 1.0).style(text::danger));
    }

    let label = field(
        "Label",
        text_input("Alarm", &alarm.label)
            .on_input(|label| edit(Edit::Label(label)))
            .size(metrics.label())
            .width(Fill)
            .into(),
        metrics,
    );

    let body = column![
        countdown,
        when,
        label,
        text("When it goes off").size(metrics.label() + 1.0),
        notification(state, metrics),
        sound(state, metrics),
        flash(state, metrics),
    ]
    .spacing(metrics.spacing)
    .padding(iced::Padding {
        right: SCROLLBAR_GUTTER,
        ..iced::Padding::ZERO
    })
    .width(Fill);

    let actions = row![
        button(text("Delete").size(metrics.label()).center())
            .on_press(Message::Alarms(Action::Delete(alarm.id)))
            .style(button::danger),
        Space::new().width(Fill),
        button(text("Done").size(metrics.label()).center())
            .on_press(Message::Alarms(Action::CloseEditor))
            .style(button::primary),
    ]
    .spacing(metrics.spacing)
    .align_y(Alignment::Center);

    // The options run long, so they scroll while the header and the buttons
    // stay put. What is left of the window after those is the scroller's.
    // The tabs above, the card's own padding and the container's around it, the
    // header and the buttons, and the gaps between them.
    let chrome = metrics.icon_button() * 4.0 + metrics.spacing * 3.0 + metrics.padding * 4.0 + 10.0;
    let room = (metrics.size.height - chrome).max(80.0);

    let panel = column![
        header,
        container(scrollable(body).width(Fill)).max_height(room),
        actions,
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    let card = container(panel)
        .padding(metrics.padding)
        .width(Fill)
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                border: iced::Border {
                    color: colors::ALARM,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..container::Style::default()
            }
        });

    // Swallow presses on the card so a click on its blank area cannot fall
    // through to the dismiss layer and close the popup.
    let card = mouse_area(card).on_press(Message::Ignored);

    let aligned = column![
        Space::new().height(Fill),
        container(card).padding(metrics.padding).width(Fill),
        Space::new().height(Fill),
    ]
    .width(Fill)
    .height(Fill);

    stack![dismiss, aligned].into()
}

fn notification<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(profile) = edited_profile(state) else {
        return Space::new().into();
    };

    let mut section = column![switch(
        "Show a notification",
        profile.notification.enabled,
        |on| edit(Edit::Notification(on)),
        metrics,
    )]
    .spacing(metrics.spacing);

    if profile.notification.enabled {
        section = section.push(choice(
            "Urgency",
            config::Urgency::ALL.to_vec(),
            Some(profile.notification.urgency),
            |urgency| edit(Edit::Urgency(urgency)),
            metrics,
        ));
    }

    section = section.push(switch(
        "Bring the window forward",
        profile.raise_window,
        |on| edit(Edit::Raise(on)),
        metrics,
    ));

    indent(section.into(), metrics)
}

fn sound<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(profile) = edited_profile(state) else {
        return Space::new().into();
    };
    let settings = &profile.sound;

    let mut section = column![switch(
        "Play a sound",
        settings.enabled,
        |on| edit(Edit::Sound(on)),
        metrics,
    )]
    .spacing(metrics.spacing);

    if let Some(error) = &state.audio_error {
        section = section.push(
            text(format!("Sound unavailable - {error}"))
                .size(metrics.label())
                .style(text::danger),
        );
    }

    if settings.enabled {
        let selected = match &settings.choice {
            config::SoundChoice::Builtin(tone) => Some(*tone),
            config::SoundChoice::Custom { .. } => None,
        };

        section = section.push(choice(
            "Sound",
            Tone::ALL.to_vec(),
            selected,
            |tone| edit(Edit::Tone(tone)),
            metrics,
        ));

        if let config::SoundChoice::Custom { .. } = &settings.choice {
            section = section.push(
                text(format!("Using {}", settings.choice))
                    .size(metrics.label())
                    .width(Fill),
            );
        }

        let preview = button(text("Preview").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .style(button::secondary);
        let preview = if state.audio.status().is_unavailable() {
            preview
        } else {
            preview.on_press(Message::Alarms(Action::Preview))
        };

        section = section.push(
            row![
                button(
                    text("Choose file...")
                        .size(metrics.label())
                        .center()
                        .width(Fill)
                )
                .width(Fill)
                .on_press(Message::Alarms(Action::SoundFileRequested))
                .style(button::secondary),
                preview,
            ]
            .spacing(metrics.spacing),
        );

        section = section.push(field(
            format!("Volume {}%", settings.volume),
            slider(0..=100_u8, settings.volume, |volume| {
                edit(Edit::Volume(volume))
            })
            .into(),
            metrics,
        ));

        section = section.push(field(
            repeat_label(settings.repeats),
            slider(0..=9_u8, settings.repeats, |repeats| {
                edit(Edit::Repeats(repeats))
            })
            .into(),
            metrics,
        ));
    }

    indent(section.into(), metrics)
}

fn flash<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(profile) = edited_profile(state) else {
        return Space::new().into();
    };
    let settings = &profile.flash;

    let mut section = column![switch(
        "Flash the screen",
        settings.enabled,
        |on| edit(Edit::Flash(on)),
        metrics,
    )]
    .spacing(metrics.spacing);

    if settings.enabled {
        section = section.push(field(
            format!("Strength {}%", (settings.peak() * 100.0).round()),
            slider(0.05..=0.8_f32, settings.peak(), |strength| {
                edit(Edit::Strength(strength))
            })
            .step(0.05_f32)
            .into(),
            metrics,
        ));

        section = section.push(field(
            format!("Pulses {}", settings.pulses.max(1)),
            slider(1..=5_u8, settings.pulses.max(1), |pulses| {
                edit(Edit::Pulses(pulses))
            })
            .into(),
            metrics,
        ));

        section = section.push(switch(
            "Let clicks pass through",
            settings.passthrough,
            |on| edit(Edit::Passthrough(on)),
            metrics,
        ));
    }

    indent(section.into(), metrics)
}

/// What the alarm being edited does when it goes off.
fn edited_profile(state: &State) -> Option<&config::AlarmProfile> {
    let editor = state.alarms.editor.as_ref()?;

    state.alarms.get(editor.id).map(|alarm| &alarm.profile)
}
