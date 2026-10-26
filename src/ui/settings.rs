//! The settings tab and the timer-hits-zero menu.

use iced::widget::{
    button, column, container, pick_list, row, scrollable, slider, text, text_input, toggler,
};
use iced::{Center, Element, Fill, Length};

use crate::app::{Message, State};
use crate::audio::Tone;
use crate::calendar;
use crate::config;

use super::layout::Metrics;

/// Room left for the scrollbar, which is drawn over the content rather than
/// beside it. Without this the right edge of every control sits underneath
/// it, which at 200px means visibly clipped labels.
const SCROLLBAR_GUTTER: f32 = 14.0;

/// A calendar offered in the default-calendar picker. The name is shown to
/// the user; the id is what gets saved as the default.
#[derive(Clone, PartialEq)]
struct CalendarOption {
    id: String,
    name: String,
}

impl std::fmt::Display for CalendarOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    // The clock lives inside the calendar now, so there is one menu rather
    // than one per tab: everything is set from the same place.
    let content = column![
        heading("Appearance", metrics),
        appearance(state, metrics),
        heading("Side panel", metrics),
        side_panel(state, metrics),
        heading("Google Calendar", metrics),
        calendar(state, metrics),
    ]
    .spacing(metrics.spacing * 1.5)
    .padding(iced::Padding {
        right: SCROLLBAR_GUTTER,
        ..iced::Padding::ZERO
    })
    .width(Fill);

    column![
        text("Settings").size(metrics.label() + 3.0),
        scrollable(content).height(Fill).width(Fill),
    ]
    .spacing(metrics.spacing)
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill)
    .into()
}

/// What happens when the clock reaches zero.
///
/// These settings change the timer rather than the calendar, so they are kept
/// off the main settings page and reached from the clock itself.
pub fn timer_view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let content = column![
        heading("When time is up", metrics),
        notification(state, metrics),
        sound(state, metrics),
        flash(state, metrics),
    ]
    .spacing(metrics.spacing * 1.5)
    .padding(iced::Padding {
        right: SCROLLBAR_GUTTER,
        ..iced::Padding::ZERO
    })
    .width(Fill);

    super::page("Alarm", content.into(), metrics)
}

fn heading<'a>(label: &'a str, metrics: &Metrics) -> Element<'a, Message> {
    text(label).size(metrics.label() + 1.0).into()
}

/// A labelled row that stacks vertically once there is no room for two
/// columns, which is what keeps every control reachable at 200px.
///
/// The label is owned rather than borrowed because most callers build it
/// from the value they are showing.
fn field<'a>(
    label: impl Into<String>,
    control: Element<'a, Message>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let caption = text(label.into()).size(metrics.label());

    if metrics.breakpoint.is_compact() {
        column![caption, control].spacing(2.0).width(Fill).into()
    } else {
        row![caption.width(Length::FillPortion(2)), control]
            .spacing(metrics.spacing)
            .align_y(Center)
            .into()
    }
}

/// A chooser that survives a narrow window.
///
/// Dropdown overlays need more width than they have in compact mode, so there
/// the control becomes a button that cycles to the next option on each tap.
fn choice<'a, T>(
    label: &'a str,
    options: Vec<T>,
    selected: Option<T>,
    on_select: impl Fn(T) -> Message + 'a,
    metrics: &Metrics,
) -> Element<'a, Message>
where
    T: ToString + PartialEq + Clone + 'a,
{
    if metrics.breakpoint.is_compact() {
        let shown = selected
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "-".to_owned());

        let next = next_option(&options, selected.as_ref());

        let control = button(text(shown).size(metrics.label()).center().width(Fill)).width(Fill);

        let control = match next {
            Some(value) => control.on_press(on_select(value)),
            None => control,
        };

        field(label, control.style(button::secondary).into(), metrics)
    } else {
        field(
            label,
            pick_list(options, selected, on_select)
                .text_size(metrics.label())
                .width(Fill)
                .into(),
            metrics,
        )
    }
}

/// The option after `selected`, wrapping around at the end.
fn next_option<T: PartialEq + Clone>(options: &[T], selected: Option<&T>) -> Option<T> {
    if options.is_empty() {
        return None;
    }

    let index = selected
        .and_then(|current| options.iter().position(|option| option == current))
        .map_or(0, |index| (index + 1) % options.len());

    options.get(index).cloned()
}

fn appearance<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let theme = choice(
        "Theme",
        state.catalog.names(),
        Some(state.config.appearance.theme.clone()),
        Message::ThemeSelected,
        metrics,
    );

    column![
        theme,
        switch(
            "Follow system light/dark",
            state.config.appearance.follow_system,
            Message::FollowSystemToggled,
            metrics,
        ),
        switch(
            "Always on top",
            state.config.window.always_on_top,
            Message::AlwaysOnTopToggled,
            metrics,
        ),
    ]
    .spacing(metrics.spacing)
    .into()
}

fn side_panel<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut input = text_input("e.g. Meta+U", &state.panel_shortcut_input).size(metrics.label());
    let mut apply = button(
        text(if state.panel_shortcut_busy {
            "Applying…"
        } else {
            "Apply shortcut"
        })
        .size(metrics.label())
        .center()
        .width(Fill),
    )
    .width(Fill);
    if !state.panel_shortcut_busy {
        input = input
            .on_input(Message::PanelShortcutChanged)
            .on_submit(Message::PanelShortcutApply);
        if !state.panel_shortcut_input.trim().is_empty() {
            apply = apply.on_press(Message::PanelShortcutApply);
        }
    }
    let mut section = column![
        field("New shortcut", input.into(), metrics),
        text("Opens or hides the side panel. Default: Meta+U. Combine Ctrl, Alt, Shift or Meta with a letter, number or named key (e.g. Space or F2).")
            .size(metrics.label()).style(text::secondary),
        apply,
    ].spacing(metrics.spacing);
    if let Some(result) = &state.panel_shortcut_result {
        section = section.push(match result {
            Ok(shortcut) => text(format!("Shortcut set to {shortcut}"))
                .size(metrics.label())
                .style(text::success),
            Err(error) => text(error).size(metrics.label()).style(text::danger),
        });
    }
    indent(section.into(), metrics)
}

fn calendar<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = &state.config.calendar;
    let connected = state.calendar.is_signed_in();

    let mut section = column![
        field(
            "OAuth client ID",
            text_input("Desktop client ID", &settings.client_id)
                .on_input(Message::GoogleClientIdChanged)
                .size(metrics.label())
                .into(),
            metrics,
        ),
        field(
            "Client secret",
            text_input("optional", &settings.client_secret)
                .on_input(Message::GoogleClientSecretChanged)
                .secure(true)
                .size(metrics.label())
                .into(),
            metrics,
        ),
        text("Create a Desktop OAuth client in Google Cloud, enable the Calendar API and the Tasks API, and allow http://127.0.0.1 as a redirect. Reconnect after enabling Tasks so they can be read and ticked off, and so events can be dragged to a new time.")
            .size((metrics.label() - 1.0).max(10.0))
            .style(text::secondary),
    ]
    .spacing(metrics.spacing);

    let action = if connected {
        let mut disconnect = button(
            text("Disconnect")
                .size(metrics.label())
                .center()
                .width(Fill),
        )
        .width(Fill)
        .style(button::secondary);
        if !state.calendar.is_busy() {
            disconnect = disconnect.on_press(Message::GoogleDisconnectRequested);
        }
        disconnect
    } else {
        let mut connect = button(
            text("Connect Google Calendar")
                .size(metrics.label())
                .center()
                .width(Fill),
        )
        .width(Fill)
        .style(button::primary);
        if !state.calendar.is_busy() {
            connect = connect.on_press(Message::GoogleConnectRequested);
        }
        connect
    };

    section = section.push(action);

    if let Some(error) = &state.calendar.error {
        section = section.push(
            text(error.clone())
                .size(metrics.label())
                .style(text::danger),
        );
    } else {
        section = section.push(
            text(state.calendar.status_line())
                .size(metrics.label())
                .style(text::secondary),
        );
    }

    let resync_label = if state.calendar.is_busy() {
        "Syncing…"
    } else {
        "Resync calendar and tasks data"
    };
    let mut resync = button(
        text(resync_label)
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill)
    .style(button::secondary);
    if connected && !state.calendar.is_busy() {
        resync = resync.on_press(Message::CalendarResyncRequested);
    }
    section = section.push(resync);

    section = section.push(field(
        sync_label(settings.sync_minutes),
        slider(
            config::CalendarSettings::MIN_SYNC_MINUTES..=config::CalendarSettings::MAX_SYNC_MINUTES,
            settings.sync_minutes.clamp(
                config::CalendarSettings::MIN_SYNC_MINUTES,
                config::CalendarSettings::MAX_SYNC_MINUTES,
            ),
            Message::CalendarSyncMinutesChanged,
        )
        .into(),
        metrics,
    ));

    section = section.push(field(
        day_end_label(settings.day_start_hour()),
        slider(
            0..=config::CalendarSettings::MAX_DAY_END_HOUR,
            settings.day_start_hour(),
            Message::CalendarDayEndChanged,
        )
        .into(),
        metrics,
    ));

    // The calendar new events are created on. Task lists are excluded since
    // they hold Google Tasks rather than calendar events.
    let options: Vec<calendar::Calendar> = state
        .calendar
        .calendars
        .iter()
        .filter(|calendar| !calendar.is_task_list())
        .filter(|calendar| settings.tasks_calendar.as_deref() != Some(calendar.id.as_str()))
        .cloned()
        .collect();

    let selected = options
        .iter()
        .find(|calendar| settings.default_calendar.as_deref() == Some(calendar.id.as_str()))
        .cloned();

    let option_map: Vec<CalendarOption> = options
        .iter()
        .map(|calendar| CalendarOption {
            id: calendar.id.clone(),
            name: calendar.name.clone(),
        })
        .collect();

    let selected_option = selected.map(|calendar| CalendarOption {
        id: calendar.id.clone(),
        name: calendar.name.clone(),
    });

    section = section.push(field(
        "Default calendar for new events",
        pick_list(option_map, selected_option, |option: CalendarOption| {
            Message::DefaultCalendarSelected(option.id)
        })
        .text_size(metrics.label())
        .width(Fill)
        .into(),
        metrics,
    ));

    section = section.push(heading("Time Blocking", metrics));
    for (label, tasks) in [
        (
            "Google Tasks list for scheduled tasks (default: Has Time)",
            true,
        ),
        ("Calendar for time blocks", false),
    ] {
        let options: Vec<CalendarOption> = state
            .calendar
            .calendars
            .iter()
            .filter(|c| c.is_task_list() == tasks)
            .map(|c| CalendarOption {
                id: c.id.clone(),
                name: c.name.clone(),
            })
            .collect();
        let selected_id = if tasks {
            &settings.time_block_list
        } else {
            &settings.time_block_calendar
        };
        let selected = options
            .iter()
            .find(|o| Some(&o.id) == selected_id.as_ref())
            .cloned();
        section = section.push(field(
            label,
            pick_list(options, selected, move |o: CalendarOption| {
                Message::TimeBlocking(if tasks {
                    crate::timeblocking::Action::List(o.id)
                } else {
                    crate::timeblocking::Action::Calendar(o.id)
                })
            })
            .text_size(metrics.label())
            .width(Fill)
            .into(),
            metrics,
        ));
    }
    let mut backlog_options = vec![CalendarOption {
        id: String::new(),
        name: "Default: Not Time Blocked".to_owned(),
    }];
    backlog_options.extend(
        state
            .calendar
            .calendars
            .iter()
            .filter(|c| c.is_task_list())
            .map(|c| CalendarOption {
                id: c.id.clone(),
                name: c.name.clone(),
            }),
    );
    let selected = backlog_options
        .iter()
        .find(|option| settings.backlog_list.as_deref().unwrap_or("") == option.id)
        .cloned();
    section = section.push(field(
        "Google Tasks list for backlog tasks",
        pick_list(backlog_options, selected, |option: CalendarOption| {
            Message::TimeBlocking(crate::timeblocking::Action::BacklogDestination(
                (!option.id.is_empty()).then_some(option.id),
            ))
        })
        .text_size(metrics.label())
        .width(Fill)
        .into(),
        metrics,
    ));
    let options: Vec<CalendarOption> = state
        .calendar
        .calendars
        .iter()
        .filter(|c| !c.is_task_list())
        .map(|c| CalendarOption {
            id: c.id.clone(),
            name: c.name.clone(),
        })
        .collect();
    let selected = options
        .iter()
        .find(|o| settings.tasks_calendar.as_deref() == Some(o.id.as_str()))
        .cloned();
    section = section.push(field(
        "Calendar for scheduled tasks (default: Tasks)",
        pick_list(options, selected, |o: CalendarOption| {
            Message::TimeBlocking(crate::timeblocking::Action::TaskCalendar(o.id))
        })
        .text_size(metrics.label())
        .width(Fill)
        .into(),
        metrics,
    ));
    section = section.push(text("Schedule tasks by dragging onto the grid or a time block, or enter a time in task details. Moving a task to Backlog also moves it to the selected backlog list. The Tasks calendar, Has Time list, and Not Time Blocked backlog list are created on first use if needed. Task notes can start with reserved lines: DEADLINE: YYYY-MM-DD HH:MM (or DUE: YYYY-MM-DD for a day only) and DUE_TIME: HH:MM to show the task at that time on its due day. Reconnect Google to allow calendar creation.").size(metrics.label()).style(text::secondary));

    indent(section.into(), metrics)
}

fn sync_label(minutes: u32) -> String {
    match minutes {
        1 => "Sync every minute".to_owned(),
        n => format!("Sync every {n} minutes"),
    }
}

fn day_end_label(hour: u32) -> String {
    if hour == 0 {
        "Day ends at midnight".to_owned()
    } else {
        format!("Day ends at {}", calendar::hour_label(hour))
    }
}

fn notification<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = &state.config.alarm.notification;

    let mut section = column![switch(
        "Show a notification",
        settings.enabled,
        Message::NotificationToggled,
        metrics,
    )]
    .spacing(metrics.spacing);

    if settings.enabled {
        section = section.push(field(
            "Text",
            text_input("Time's up", &settings.summary)
                .on_input(Message::NotificationSummaryChanged)
                .size(metrics.label())
                .into(),
            metrics,
        ));

        section = section.push(choice(
            "Urgency",
            config::Urgency::ALL.to_vec(),
            Some(settings.urgency),
            Message::NotificationUrgencySelected,
            metrics,
        ));

        section = section.push(switch(
            "Bring the window forward",
            state.config.alarm.raise_window,
            Message::RaiseWindowToggled,
            metrics,
        ));
    }

    indent(section.into(), metrics)
}

fn sound<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = &state.config.alarm.sound;

    let mut section = column![switch(
        "Play a sound",
        settings.enabled,
        Message::SoundToggled,
        metrics,
    )]
    .spacing(metrics.spacing);

    if let Some(error) = &state.audio_error {
        // The sound section stays switched on: the setting is still valid,
        // it just cannot be honored on this machine right now.
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
            Message::ToneSelected,
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

        // No point offering a preview we already know cannot be played.
        let preview = if state.audio.status().is_unavailable() {
            preview
        } else {
            preview.on_press(Message::PreviewRequested)
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
                .on_press(Message::SoundFileRequested)
                .style(button::secondary),
                preview,
            ]
            .spacing(metrics.spacing),
        );

        section = section.push(field(
            format!("Volume {}%", settings.volume),
            slider(0..=100_u8, settings.volume, Message::VolumeChanged).into(),
            metrics,
        ));

        section = section.push(field(
            repeat_label(settings.repeats),
            slider(0..=9_u8, settings.repeats, Message::RepeatsChanged).into(),
            metrics,
        ));
    }

    indent(section.into(), metrics)
}

fn repeat_label(repeats: u8) -> String {
    match repeats {
        0 => "Repeat until dismissed".to_owned(),
        1 => "Play once".to_owned(),
        count => format!("Play {count} times"),
    }
}

fn flash<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = &state.config.alarm.flash;

    let mut section = column![switch(
        "Flash the screen",
        settings.enabled,
        Message::FlashToggled,
        metrics,
    )]
    .spacing(metrics.spacing);

    if settings.enabled {
        section = section.push(field(
            format!("Strength {}%", (settings.peak() * 100.0).round()),
            slider(
                0.05..=0.8_f32,
                settings.peak(),
                Message::FlashOpacityChanged,
            )
            .step(0.05_f32)
            .into(),
            metrics,
        ));

        section = section.push(field(
            format!("Pulses {}", settings.pulses.max(1)),
            slider(
                1..=5_u8,
                settings.pulses.max(1),
                Message::FlashPulsesChanged,
            )
            .into(),
            metrics,
        ));

        section = section.push(switch(
            "Let clicks pass through",
            settings.passthrough,
            Message::FlashPassthroughToggled,
            metrics,
        ));

        section = section.push(field(
            countdown_label(settings.final_countdown_secs),
            slider(
                0..=10_u8,
                settings.final_countdown_secs,
                Message::FinalCountdownChanged,
            )
            .into(),
            metrics,
        ));

        section = section.push(
            text("The flash rate is capped at 3 per second.")
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }

    indent(section.into(), metrics)
}

fn countdown_label(seconds: u8) -> String {
    if seconds == 0 {
        "No countdown warning".to_owned()
    } else {
        format!("Pulse the window for the last {seconds}s")
    }
}

fn switch<'a>(
    label: &'a str,
    value: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
    metrics: &Metrics,
) -> Element<'a, Message> {
    toggler(value)
        .label(label)
        .text_size(metrics.label())
        .on_toggle(on_toggle)
        .width(Fill)
        .into()
}

/// Indents a subsection so the sliders visibly belong to their toggle.
fn indent<'a>(content: Element<'a, Message>, metrics: &Metrics) -> Element<'a, Message> {
    container(content)
        .padding(iced::Padding {
            left: if metrics.breakpoint.is_compact() {
                0.0
            } else {
                6.0
            },
            ..iced::Padding::ZERO
        })
        .width(Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycling_wraps_around_the_options() {
        let options = vec!["a", "b", "c"];

        assert_eq!(next_option(&options, Some(&"a")), Some("b"));
        assert_eq!(next_option(&options, Some(&"c")), Some("a"));
    }

    #[test]
    fn cycling_from_nothing_selects_the_first_option() {
        let options = vec!["a", "b"];

        assert_eq!(next_option(&options, None), Some("a"));
    }

    #[test]
    fn cycling_an_empty_list_selects_nothing() {
        assert_eq!(next_option::<&str>(&[], None), None);
    }

    #[test]
    fn repeat_labels_read_naturally() {
        assert_eq!(repeat_label(0), "Repeat until dismissed");
        assert_eq!(repeat_label(1), "Play once");
        assert_eq!(repeat_label(3), "Play 3 times");
    }

    #[test]
    fn the_countdown_label_explains_being_switched_off() {
        assert_eq!(countdown_label(0), "No countdown warning");
        assert_eq!(countdown_label(5), "Pulse the window for the last 5s");
    }

    #[test]
    fn sync_labels_read_naturally() {
        assert_eq!(sync_label(1), "Sync every minute");
        assert_eq!(sync_label(15), "Sync every 15 minutes");
    }

    #[test]
    fn day_end_labels_read_naturally() {
        assert_eq!(day_end_label(0), "Day ends at midnight");
        assert_eq!(day_end_label(3), "Day ends at 3 AM");
        assert_eq!(day_end_label(5), "Day ends at 5 AM");
    }
}
