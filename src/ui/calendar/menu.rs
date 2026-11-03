//! The calendars menu and its color pickers.

use super::*;
use iced::widget::column;

/// The palette shown under a source whose color is being changed. Six
/// swatches per row keeps it inside the sheet even at 200px.
pub(super) fn source_color_picker<'a>(
    id: &str,
    current: Option<Color>,
    is_tasks: bool,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let mut rows: Vec<Element<'a, Message>> = Vec::new();
    let palette = if is_tasks {
        colors::palette()
    } else {
        colors::calendar_palette()
    };
    let pick = |color| {
        if is_tasks {
            Message::Settings(SettingsMessage::TaskColorPicked(id.to_owned(), color))
        } else {
            Message::Settings(SettingsMessage::CalendarColorPicked(id.to_owned(), color))
        }
    };
    for chunk in palette.chunks(6) {
        let swatches: Vec<Element<'a, Message>> = chunk
            .iter()
            .map(|&color| swatch_button(color, current == Some(color), pick(Some(color))))
            .collect();
        rows.push(row(swatches).spacing(6.0).into());
    }

    let reset = button(
        text("Reset to default")
            .size(metrics.label() - 1.0)
            .center()
            .width(Fill),
    )
    .width(Fill)
    .on_press(pick(None))
    .style(crate::ui::controls::secondary);

    column![
        text(if is_tasks {
            "Task color"
        } else {
            "Calendar color in TimeSlay"
        })
        .size(metrics.label() - 1.0)
        .style(text::secondary),
        column(rows).spacing(6.0),
        reset,
    ]
    .spacing(6.0)
    .width(Fill)
    .into()
}

/// The palette shown under a TimeSlay event kind whose color is being changed.
pub(super) fn time_slay_color_picker<'a>(
    source: calendar::sources::TimeSlaySource,
    current: Option<Color>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let mut rows: Vec<Element<'a, Message>> = Vec::new();
    for chunk in colors::palette().chunks(6) {
        let swatches: Vec<Element<'a, Message>> = chunk
            .iter()
            .map(|&color| {
                swatch_button(
                    color,
                    current == Some(color),
                    Message::Settings(SettingsMessage::TimeSlayColorPicked(
                        source.id().to_owned(),
                        Some(color),
                    )),
                )
            })
            .collect();
        rows.push(row(swatches).spacing(6.0).into());
    }

    let reset = button(
        text("Reset to default")
            .size(metrics.label() - 1.0)
            .center()
            .width(Fill),
    )
    .width(Fill)
    .on_press(Message::Settings(SettingsMessage::TimeSlayColorPicked(
        source.id().to_owned(),
        None,
    )))
    .style(crate::ui::controls::secondary);

    column![
        text(format!("Color for {}", source.label()))
            .size(metrics.label() - 1.0)
            .style(text::secondary),
        column(rows).spacing(6.0),
        reset,
    ]
    .spacing(6.0)
    .width(Fill)
    .into()
}

/// The swatch beside Travel: the gray its trips are drawn on the grid, with no
/// picker of its own.
fn travel_swatch<'a>() -> Element<'a, Message> {
    container(Space::new())
        .width(Length::Fixed(14.0))
        .height(Length::Fixed(14.0))
        .style(|_theme: &Theme| container::Style {
            background: Some(colors::TRAVEL.into()),
            border: iced::Border {
                color: Color::BLACK.scale_alpha(0.45),
                width: 1.0,
                radius: crate::ui::controls::RADIUS_SMALL.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// A row of the Zoom section. `message` is `None` when there is no further to go.
fn zoom_button<'a>(
    label: &'a str,
    key: &'a str,
    message: Option<CalendarMessage>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    // Cmd on a Mac and Ctrl elsewhere, the same as the shortcut itself.
    let modifier = if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    };
    button(
        row![
            text(label).size(metrics.label()).width(Fill),
            text(format!("{modifier} {key}"))
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        ]
        .spacing(8.0),
    )
    .width(Fill)
    .on_press_maybe(message.map(Message::Calendar))
    .style(crate::ui::controls::secondary)
    .into()
}

fn zoom_section<'a>(state: &State, metrics: &Metrics) -> Element<'a, Message> {
    let height = state.calendar.hour_height;
    let zoom = |factor| CalendarMessage::CalendarZoomed {
        factor,
        anchor: None,
    };
    column![
        text("Zoom").size(metrics.label()).style(text::secondary),
        zoom_button(
            "Zoom in",
            "+",
            (height < calendar::MAX_HOUR_HEIGHT).then(|| zoom(calendar::ZOOM_STEP)),
            metrics,
        ),
        zoom_button(
            "Reset zoom",
            "0",
            (height != calendar::DEFAULT_HOUR_HEIGHT).then_some(CalendarMessage::CalendarZoomReset),
            metrics,
        ),
        zoom_button(
            "Zoom out",
            "-",
            (height > calendar::MIN_HOUR_HEIGHT).then(|| zoom(1.0 / calendar::ZOOM_STEP)),
            metrics,
        ),
    ]
    .spacing(metrics.spacing)
    .width(Fill)
    .into()
}

pub(super) fn menu_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let dismiss = mouse_area(
        container(iced::widget::Space::new())
            .width(Fill)
            .height(Fill),
    )
    .on_press(Message::Calendar(CalendarMessage::CalendarMenuToggled));

    let mut panel = column![text("Calendars").size(metrics.label() + 1.0)]
        .spacing(metrics.spacing)
        .width(Fill);

    let sources = state.calendar.display_sources(&state.config.calendar);
    for (heading, is_tasks) in [("Google Calendars", false), ("Google Task Lists", true)] {
        let mut section = column![text(heading).size(metrics.label()).style(text::secondary)]
            .spacing(metrics.spacing)
            .width(Fill);
        for source in sources.iter().filter(|source| {
            source.id != calendar::sleep::SOURCE_ID
                && source.is_task_list() == is_tasks
                && !source.is_app_calendar(&state.config.calendar)
        }) {
            let id = source.id.clone();
            let enabled = state.calendar.calendar_enabled(&id, &state.config.calendar);
            let editing = state.calendar.color_editor.as_deref() == Some(id.as_str());
            let toggle_id = id.clone();
            section = section.push(
                row![
                    swatch_button(
                        source.color,
                        editing,
                        Message::Settings(SettingsMessage::SourceColorEditorToggled(id.clone())),
                    ),
                    text(source.name.clone()).size(metrics.label()).width(Fill),
                    crate::ui::controls::toggler(enabled).on_toggle(move |value| {
                        Message::Calendar(CalendarMessage::CalendarToggled(
                            toggle_id.clone(),
                            value,
                        ))
                    }),
                ]
                .spacing(8.0)
                .align_y(Alignment::Center),
            );
            if editing {
                section = section.push(source_color_picker(
                    &id,
                    if is_tasks {
                        state.config.calendar.task_color(&id)
                    } else {
                        state
                            .config
                            .calendar
                            .calendar_colors
                            .get(&id)
                            .map(|color| color.0)
                    },
                    is_tasks,
                    metrics,
                ));
            }
        }
        panel = panel.push(section);
    }

    let mut section = column![
        text("TimeSlay")
            .size(metrics.label())
            .style(text::secondary)
    ]
    .spacing(metrics.spacing)
    .width(Fill);
    for source in calendar::sources::TimeSlaySource::ALL {
        let id = source.id().to_owned();
        let editing = state.calendar.color_editor.as_deref() == Some(id.as_str());
        section = section.push(
            row![
                swatch_button(
                    source.color(&state.config.calendar),
                    editing,
                    Message::Settings(SettingsMessage::SourceColorEditorToggled(id.clone())),
                ),
                text(source.label()).size(metrics.label()).width(Fill),
                crate::ui::controls::toggler(
                    state
                        .calendar
                        .calendar_enabled(source.id(), &state.config.calendar)
                )
                .on_toggle(move |value| Message::Calendar(
                    CalendarMessage::CalendarToggled(source.id().into(), value)
                )),
            ]
            .spacing(8.0)
            .align_y(Alignment::Center),
        );
        if editing {
            section = section.push(time_slay_color_picker(
                source,
                source.custom_color(&state.config.calendar),
                metrics,
            ));
        }
    }
    // Travel is not an event kind, so its row has no color picker: the swatch
    // only shows the gray its trips are drawn in. It is toggled here with the
    // other TimeSlay sources, and off means no route is ever worked out.
    section = section.push(
        row![
            travel_swatch(),
            text("Travel").size(metrics.label()).width(Fill),
            crate::ui::controls::toggler(state.travel.show)
                .on_toggle(|_| Message::Travel(crate::travel::Action::Toggle)),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    );
    panel = panel.push(section);
    panel = panel.push(zoom_section(state, metrics));
    if state.calendar.calendars.is_empty() {
        panel = panel.push(
            text(if state.calendar.is_signed_in() {
                "No calendars yet. Sync to load them."
            } else {
                "Connect Google Calendar in Settings."
            })
            .size(metrics.label())
            .style(text::secondary),
        );
    }

    let sync_label = if state.calendar.is_busy() {
        "Syncing…"
    } else {
        "Sync now"
    };

    let mut sync = button(text(sync_label).size(metrics.label()).center().width(Fill))
        .width(Fill)
        .style(crate::ui::controls::primary);

    if state.calendar.is_signed_in() && !state.calendar.is_busy() {
        sync = sync.on_press(Message::Calendar(CalendarMessage::CalendarSyncRequested));
    }

    panel = panel.push(sync);

    let card = container(
        crate::ui::controls::scrollable(container(panel).padding(iced::Padding {
            right: 14.0,
            ..iced::Padding::ZERO
        }))
        .height(Length::Shrink),
    )
    .max_height(
        (state.main_size.height - header_offset(metrics) - metrics.padding * 2.0).max(120.0),
    )
    .padding(metrics.padding)
    .width(Length::Fill)
    .style(crate::ui::controls::surface);

    let aligned = column![
        // Keep the header row tappable; the sheet starts below it.
        iced::widget::Space::new().height(header_offset(metrics)),
        container(iced::widget::opaque(card))
            .padding(metrics.padding)
            .width(Fill),
    ]
    .width(Fill);

    stack![dismiss, aligned].into()
}
