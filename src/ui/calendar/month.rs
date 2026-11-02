//! The month grid and the small month picker.

use super::*;
use iced::widget::column;

/// A month overview with whole Monday–Sunday weeks. Selecting a date updates
/// the day panel while keeping the month overview anchored in place.
pub fn monthly_view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let dates = calendar::month_grid(state.week_anchor);
    let today = calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
    let grid_width = (metrics.size.width - metrics.padding * 2.0).max(840.0);
    let cell_width = grid_width / 7.0;
    let cell_height = ((metrics.size.height - 160.0) / (dates.len() / 7) as f32).max(135.0);
    let cell_metrics = Metrics::of(Size::new(cell_width - 12.0, cell_height));
    let weekdays = row(
        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map(|day| {
            container(text(day).size(cell_metrics.label()).center().width(Fill))
                .width(Length::Fixed(cell_width))
                .into()
        }),
    );
    let weeks = dates.chunks(7).map(|week| {
        row(week.iter().map(|&day| {
            let in_month = day.month() == state.week_anchor.month();
            let label = if in_month {
                day.format("%-d").to_string()
            } else {
                day.format("%b %-d").to_string()
            };
            let heading = button(text(label).size(cell_metrics.label()).center().width(Fill))
                .width(Fill)
                .padding(2)
                .on_press(Message::Calendar(CalendarMessage::CalendarDayOpened(day)))
                .style(if day == today {
                    crate::ui::controls::primary
                } else if day == state.calendar.day {
                    crate::ui::controls::secondary
                } else {
                    button::text
                });
            let mut events = column![].spacing(3).width(Fill);
            for event in state
                .calendar
                .all_day_events_on(day, &state.config.calendar)
            {
                if !state.habits.matches_calendar_habit(event) {
                    continue;
                }
                // Overdue and undated tasks belong in the pending list rather
                // than appearing in every date of a month.
                if event.is_task() {
                    if event.start.date_naive() == day {
                        events = events.push(weekly_task_chip(state, event, &cell_metrics));
                    }
                } else {
                    events = events.push(all_day_chip(state, event, &cell_metrics));
                }
            }
            for event in state.calendar.timed_events_on(day, &state.config.calendar) {
                if event.details.sleep.is_some() {
                    continue;
                }
                if !state.habits.matches_calendar_habit(event) {
                    continue;
                }
                if state.habits.calendar_habit.is_some() {
                    events = events.push(habit_occurrence(state, event, &cell_metrics));
                    continue;
                }
                let label = format!(
                    "{} {}",
                    calendar::time_format::clock(event.start, state.config.appearance.time_format),
                    event.title
                );
                let color = event.display_color(&state.config.calendar);
                events = events.push(
                    button(
                        text(label)
                            .size(cell_metrics.label())
                            .wrapping(text::Wrapping::None),
                    )
                    .width(Fill)
                    .padding([3, 4])
                    .on_press(Message::Event(EventMessage::EventOpened(event.id.clone())))
                    .style(move |theme, status| {
                        let mut style = crate::ui::controls::secondary(theme, status);
                        style.border = iced::Border {
                            color,
                            width: 1.0,
                            radius: crate::ui::controls::RADIUS_SMALL.into(),
                        };
                        style
                    }),
                );
            }
            if state.habits.calendar_habit.is_some()
                && !state
                    .calendar
                    .events_on(day)
                    .iter()
                    .any(|event| state.habits.matches_calendar_habit(event))
            {
                events = events.push(
                    text(if state.calendar_loading() {
                        "Loading…"
                    } else {
                        "Not scheduled"
                    })
                    .size(cell_metrics.label() - 1.0)
                    .style(text::secondary),
                );
            }
            let cell: Element<'a, Message> = container(
                column![
                    heading,
                    crate::ui::controls::scrollable(events).height(Fill),
                ]
                .spacing(3),
            )
            .padding(5)
            .width(Length::Fixed(cell_width))
            .height(Length::Fixed(cell_height))
            .style(move |theme: &Theme| {
                let palette = theme.extended_palette();
                container::Style {
                    background: Some(
                        if in_month {
                            palette.background.weakest.color
                        } else {
                            palette.background.weak.color
                        }
                        .into(),
                    ),
                    border: iced::Border {
                        color: if state.timeblocking.dragging.is_some() {
                            palette.primary.strong.color
                        } else {
                            palette.background.strong.color
                        },
                        width: if state.timeblocking.dragging.is_some() {
                            1.5
                        } else {
                            0.5
                        },
                        ..Default::default()
                    },
                    ..Default::default()
                }
            })
            .into();
            Element::from(
                mouse_area(cell)
                    .on_release(Message::TimeBlocking(crate::timeblocking::Action::DropDay)),
            )
            .map(move |message| {
                Message::Calendar(CalendarMessage::WeekDayAction(day, Box::new(message)))
            })
        }))
        .into()
    });
    let grid = crate::ui::controls::scrollable(
        column![weekdays, column(weeks)].width(Length::Fixed(grid_width)),
    )
    .direction(scrollable::Direction::Both {
        vertical: scrollable::Scrollbar::new(),
        horizontal: scrollable::Scrollbar::new(),
    })
    .width(Fill)
    .height(Fill);
    let heading = range_heading(
        state,
        state.week_anchor.format("%B %Y ▾").to_string(),
        CalendarMessage::CalendarViewMonthShift(-1),
        CalendarMessage::CalendarViewMonthShift(1),
        metrics,
    );
    let mut content = column![
        heading,
        text(if state.calendar_loading() {
            "Loading..."
        } else {
            "Select a date to open its timeline."
        })
        .size(metrics.label() - 1.0)
        .style(text::secondary),
        grid,
    ]
    .spacing(metrics.spacing)
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill);
    if let Some(habit) = &state.habits.calendar_habit {
        content = content.push(
            row![
                text(&habit.title).size(metrics.label()),
                crate::ui::controls::quiet_button(
                    text("Close habit calendar").size(metrics.label())
                )
                .on_press(Message::Habits(crate::habits::Action::CloseCalendar)),
            ]
            .spacing(metrics.spacing),
        );
        if let Some(error) = &state.habits.error {
            content = content.push(text(error).size(metrics.label()).style(text::danger));
        }
    }
    if let Some(dismiss) = dismiss_bar(state, metrics) {
        content = content.push(dismiss);
    }
    range_layers(state, metrics, content.into())
}

/// The month picker, the steps back and forward and the menu above a month-shaped
/// grid.
pub(super) fn range_heading<'a>(
    state: &'a State,
    label: String,
    back: CalendarMessage,
    forward: CalendarMessage,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let today = calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
    let title = button(text(label).size(metrics.label()))
        .on_press(Message::Calendar(CalendarMessage::CalendarMonthToggled(
            MonthPicker::Overview,
        )))
        .style(crate::ui::controls::secondary);
    let menu = button(text("Menu").size(metrics.label()))
        .on_press(Message::Calendar(CalendarMessage::CalendarMenuToggled))
        .style(crate::ui::controls::secondary);
    let controls = row![
        nav_button("◀", Message::Calendar(back), metrics),
        button(text("Today").size(metrics.label()))
            .on_press(Message::Calendar(CalendarMessage::CalendarDaySelected(
                today
            )))
            .style(crate::ui::controls::secondary),
        nav_button("▶", Message::Calendar(forward), metrics),
    ]
    .spacing(metrics.spacing)
    .align_y(Alignment::Center);
    if metrics.size.width < 500.0 {
        column![
            row![title, Space::new().width(Fill), menu].align_y(Alignment::Center),
            controls
        ]
        .spacing(metrics.spacing)
        .into()
    } else {
        row![title, controls, Space::new().width(Fill), menu]
            .spacing(metrics.spacing)
            .align_y(Alignment::Center)
            .into()
    }
}

/// What floats over a month-shaped grid: the details pane or time block panel,
/// the new-event popup, the month picker and the menu.
pub(super) fn range_layers<'a>(
    state: &'a State,
    metrics: &Metrics,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    let mut layers: Vec<Element<'a, Message>> = vec![content];
    let draft_open = state.calendar.draft.as_ref().is_some_and(|d| d.editor_open);
    if state.calendar.open_event.is_some() || draft_open || state.timeblocking.block.is_some() {
        let details = state.calendar.open_event.is_some() || draft_open;
        let dismiss = if draft_open {
            Message::EventDraft(EventDraftMessage::EventDraftCancelled)
        } else if details {
            Message::Event(EventMessage::EventClosed)
        } else {
            Message::TimeBlocking(crate::timeblocking::Action::Close)
        };
        layers.push(
            mouse_area(container(Space::new()).width(Fill).height(Fill).style(|_| {
                container::Style {
                    background: Some(crate::ui::controls::SCRIM.into()),
                    ..Default::default()
                }
            }))
            .on_press(dismiss)
            .into(),
        );
        let width = (if details {
            state.config.views.detail_width()
        } else {
            state.config.views.time_block_width()
        })
        .min(metrics.size.width);
        let pane_metrics = Metrics::of(Size::new(width, metrics.size.height));
        let pane = if details {
            crate::ui::controls::scrollable(details_pane(state, &pane_metrics))
                .height(Fill)
                .into()
        } else {
            super::super::timeblocking::panel(state, &pane_metrics)
        };
        layers.push(
            row![
                Space::new().width(Fill),
                container(pane)
                    .width(Length::Fixed(width))
                    .height(Fill)
                    .style(crate::ui::controls::surface)
            ]
            .height(Fill)
            .into(),
        );
    }
    let overlay = if state
        .calendar
        .draft
        .as_ref()
        .is_some_and(|d| !d.editor_open)
    {
        draft_overlay(state, metrics)
    } else if state.overview_month_open {
        month_overlay(state, MonthPicker::Overview, metrics)
    } else if state.calendar.menu_open {
        menu_overlay(state, metrics)
    } else {
        Space::new().width(0).height(0).into()
    };
    layers.push(overlay);
    iced::widget::Stack::with_children(layers).into()
}

pub(crate) fn scroll_week_drag(state: &State) -> iced::Task<Message> {
    super::super::weekly_task_list::auto_scroll(state)
}

pub(crate) fn weekly_view_width(settings: &crate::config::CalendarSettings) -> f32 {
    let columns = settings.weekly_days() as f32 * weekly_column_width(settings);
    let padding = if settings.weekly_task_list {
        0.0
    } else {
        Metrics::of(Size::new(columns, 0.0)).padding
    };
    columns + padding * 2.0
}

/// How wide a day is in the month picker, unless the window is narrower.
const PICKER_CELL_WIDTH: f32 = 36.0;
const PICKER_CELL_GAP: f32 = 2.0;

/// The month picker: a grid of the month's days, with arrows to step a month
/// at a time. Tapping a day jumps whatever `picker` belongs to there, and the
/// card is only as wide as its seven columns.
pub(super) fn month_overlay<'a>(
    state: &'a State,
    picker: MonthPicker,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let dismiss = mouse_area(
        container(iced::widget::Space::new())
            .width(Fill)
            .height(Fill),
    )
    .on_press(Message::Calendar(CalendarMessage::CalendarMonthToggled(
        picker,
    )));

    let month = match picker {
        MonthPicker::Panel => state.calendar.month,
        MonthPicker::Overview => state.overview_month,
    };
    let today = calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
    let selected = match picker {
        MonthPicker::Panel => state.calendar.day,
        MonthPicker::Overview if state.monthly() => state.week_anchor,
        MonthPicker::Overview => state.week_hover.unwrap_or(state.week_anchor),
    };
    let preview = match picker {
        MonthPicker::Overview if state.weekly() => Some(calendar::week_dates(
            selected,
            state.config.calendar.weekly_days(),
        )),
        MonthPicker::Overview if state.two_weekly() => Some(calendar::two_week_dates(selected)),
        _ => None,
    };

    let title = row![
        nav_button(
            "◀",
            Message::Calendar(CalendarMessage::CalendarMonthShift(picker, -1)),
            metrics
        ),
        text(calendar::month_heading(month))
            .size(metrics.label() + 1.0)
            .width(Fill)
            .center(),
        nav_button(
            "▶",
            Message::Calendar(CalendarMessage::CalendarMonthShift(picker, 1)),
            metrics
        ),
    ]
    .spacing(metrics.spacing)
    .align_y(Alignment::Center);

    let weekdays = row(calendar::WEEKDAY_INITIALS.iter().map(|label| {
        text(*label)
            .size(metrics.label() - 1.0)
            .width(Fill)
            .center()
            .style(text::secondary)
            .into()
    }))
    .spacing(PICKER_CELL_GAP)
    .width(Fill);

    let days = calendar::month_grid(month);
    let mut weeks: Vec<Element<'a, Message>> = Vec::new();
    for week in days.chunks(7) {
        let cells: Vec<Element<'a, Message>> = week
            .iter()
            .map(|&date| {
                day_cell(
                    date,
                    month,
                    selected,
                    today,
                    preview.as_deref(),
                    picker,
                    metrics,
                )
            })
            .collect();
        weeks.push(row(cells).spacing(PICKER_CELL_GAP).width(Fill).into());
    }

    let panel = column![
        title,
        weekdays,
        column(weeks).spacing(PICKER_CELL_GAP).width(Fill),
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    let width = PICKER_CELL_WIDTH * 7.0 + PICKER_CELL_GAP * 6.0 + metrics.padding * 2.0;
    let card = container(panel)
        .padding(metrics.padding)
        .width(width.min(metrics.size.width - metrics.padding * 2.0))
        .style(crate::ui::controls::surface);

    let aligned = column![
        // Keep the header row tappable; the sheet starts below it.
        iced::widget::Space::new().height(header_offset(metrics)),
        container(card).padding(metrics.padding).width(Fill),
    ]
    .width(Fill);

    stack![dismiss, aligned].into()
}

/// One day in the month grid. The selected day is filled, today is outlined,
/// and days from the neighbouring months are dimmed.
pub(super) fn day_cell<'a>(
    date: NaiveDate,
    month: NaiveDate,
    selected: NaiveDate,
    today: NaiveDate,
    preview: Option<&[NaiveDate]>,
    picker: MonthPicker,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let in_month = date.month() == month.month() && date.year() == month.year();
    let is_selected = date == selected;
    let is_today = date == today;
    let in_preview = preview.is_some_and(|days| days.contains(&date));

    let cell = button(
        text(date.day().to_string())
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill)
    .height(32)
    .padding(0)
    .on_press(Message::Calendar(CalendarMessage::CalendarMonthDayPicked(
        picker, date,
    )))
    .style(move |theme, status| {
        if is_selected {
            crate::ui::controls::primary(theme, status)
        } else if in_preview {
            crate::ui::controls::secondary(theme, status)
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
    });
    if preview.is_some() {
        mouse_area(cell)
            .on_enter(Message::Calendar(CalendarMessage::WeekHover(Some(date))))
            .on_exit(Message::Calendar(CalendarMessage::WeekHover(None)))
            .into()
    } else {
        cell.into()
    }
}

/// The calendar or app-specific type shown beneath an item's title.
pub(crate) fn event_source<'a>(
    state: &State,
    event: &Event,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let calendar = state
        .calendar
        .calendars
        .iter()
        .find(|calendar| calendar.id == event.calendar_id);
    let managed = state.config.calendar.app_calendar.as_deref() == Some(&event.calendar_id)
        || calendar.is_some_and(|calendar| {
            crate::config::CalendarSettings::is_app_calendar_name(&calendar.name)
        });
    let kind = if event.details.sleep.is_some() {
        Some("Sleep")
    } else if event.details.habit {
        Some("Habit")
    } else if event.is_task() || event.details.scheduled.is_some() {
        Some("Task")
    } else if managed {
        Some(if event.all_day { "Event" } else { "Time Block" })
    } else {
        None
    };
    let label = if let Some(kind) = kind {
        format!("Type: {kind}")
    } else {
        format!(
            "Calendar: {}",
            calendar.map_or(event.calendar_id.as_str(), |calendar| calendar
                .name
                .as_str())
        )
    };
    text(label)
        .size(metrics.label())
        .style(text::secondary)
        .into()
}
