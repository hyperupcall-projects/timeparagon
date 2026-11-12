//! The two-week grid: two rows of seven days, each a list of the day's events
//! that fits without scrolling, the way Google Calendar's own two-week view does.

use super::*;
use iced::widget::column;

/// The grid scrolls rather than squeezing its columns past this.
const MIN_GRID_WIDTH: f32 = 840.0;
/// The grid scrolls rather than squeezing its rows past this.
const MIN_CELL_HEIGHT: f32 = 135.0;
/// Every row in a cell is the same height, so how many fit is simple division.
const ROW_HEIGHT: f32 = 22.0;
const ROW_GAP: f32 = 2.0;
const CELL_PADDING: f32 = 5.0;
/// The date button at the top of a cell.
const DATE_HEIGHT: f32 = 24.0;
/// The weekday name above the dates of the first row.
const WEEKDAY_HEIGHT: f32 = 18.0;
const DOT_SIZE: f32 = 8.0;
const POPUP_WIDTH: f32 = 300.0;
/// How far an event that has already ended fades.
const PAST_ALPHA: f32 = 0.45;

/// The height of the two rows together. At the default zoom they fill the space
/// unless that would squeeze them; zooming scales from there, and a taller cell
/// lists more of its day.
pub(super) fn two_week_height(available: f32, zoom: f32) -> f32 {
    available.max(MIN_CELL_HEIGHT * 2.0) * zoom
}

pub fn two_week_view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let metrics = *metrics;
    let first_day = state.config.calendar.first_day_of_week;
    let dates = calendar::two_week_dates(state.week_anchor, first_day);
    let zoom = state.calendar.zoom_ratio();
    let scroll_top = state.grid_scroll.0;
    let grid = responsive(move |size| {
        let width = size.width.max(MIN_GRID_WIDTH);
        let height = two_week_height(size.height, zoom);
        let cell = Size::new(width / 7.0, height / 2.0);
        let weeks = dates.chunks(7).enumerate().map(|(index, week)| {
            row(week
                .iter()
                .map(|&day| day_cell(state, day, index == 0, cell, &metrics)))
            .into()
        });
        let grid = crate::ui::controls::scrollable(column(weeks).width(width).height(height))
            .id(iced::widget::Id::new(RANGE_SCROLL))
            .on_scroll(grid_scrolled)
            .direction(scrollable::Direction::Both {
                vertical: scrollable::Scrollbar::new(),
                horizontal: scrollable::Scrollbar::new(),
            })
            .width(Fill)
            .height(Fill);
        zoom_wheel_layer(grid.into(), scroll_top)
    });

    let days = calendar::two_week_dates(state.week_anchor, first_day);
    let (first, last) = (days[0], days[13]);
    let label = if first.year() == last.year() {
        format!(
            "{} – {} ▾",
            first.format("%b %-d"),
            last.format("%b %-d, %Y")
        )
    } else {
        format!(
            "{} – {} ▾",
            first.format("%b %-d, %Y"),
            last.format("%b %-d, %Y")
        )
    };
    let heading = range_heading(state, label, &metrics);
    let mut content = column![
        heading,
        text("Select a date to open its timeline, or an empty space to add an event.")
            .size(metrics.label() - 1.0)
            .style(text::secondary),
        grid,
    ]
    .spacing(metrics.spacing)
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill);
    if let Some(dismiss) = dismiss_bar(state, &metrics) {
        content = content.push(dismiss);
    }
    let content: Element<'a, Message> = match state.two_week_more {
        Some(day) => stack![content, more_popup(state, day, &metrics)].into(),
        None => content.into(),
    };
    pointer_watch(range_layers(state, &metrics, content))
}

/// How many rows fit under a cell's heading.
fn rows_that_fit(cell_height: f32, heading_height: f32) -> usize {
    let room = cell_height - CELL_PADDING * 2.0 - heading_height;
    (((room + ROW_GAP) / (ROW_HEIGHT + ROW_GAP)).floor() as usize).max(1)
}

fn day_cell<'a>(
    state: &'a State,
    day: NaiveDate,
    first_row: bool,
    cell: Size,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let today = calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
    // Only the first of a month says which month it is; the rest would crowd.
    let label = if day.day() == 1 {
        day.format("%b %-d").to_string()
    } else {
        day.format("%-d").to_string()
    };
    let date = button(text(label).size(metrics.label()).center().width(Fill))
        .width(Fill)
        .height(DATE_HEIGHT)
        .padding(2)
        .on_press(Message::Calendar(CalendarMessage::CalendarDayOpened(day)))
        .style(if day == today {
            crate::ui::controls::primary
        } else if day == state.calendar.day {
            crate::ui::controls::secondary
        } else {
            button::text
        });
    let mut heading_height = DATE_HEIGHT;
    let mut body = column![].spacing(ROW_GAP).width(Fill);
    if first_row {
        heading_height += WEEKDAY_HEIGHT;
        body = body.push(
            text(day.format("%a").to_string().to_uppercase())
                .size(metrics.label() - 1.0)
                .height(WEEKDAY_HEIGHT)
                .center()
                .width(Fill)
                .style(text::secondary),
        );
    }
    body = body.push(date);

    let mut items = crate::ui::drop_preview::day_list(
        state,
        day,
        day_items(state, day, metrics),
        // The cell gives its messages their day.
        |message| message,
        |task| preview_row(state, task, metrics),
    );
    // Dropped on the day as a whole, the task in the air goes first, so it
    // stays in sight in a full cell.
    if let Some(task) = crate::ui::drop_preview::on_day(state, day) {
        items.insert(
            0,
            crate::ui::drop_preview::chip(state, &task, metrics.label()),
        );
    }
    let fit = rows_that_fit(cell.height, heading_height);
    if items.len() > fit {
        // The "N more" row takes the place of the last event that would fit.
        let hidden = items.len() - (fit - 1);
        items.truncate(fit - 1);
        items.push(
            button(
                text(format!("{hidden} more"))
                    .size(metrics.label())
                    .wrapping(text::Wrapping::None),
            )
            .width(Fill)
            .height(ROW_HEIGHT)
            .padding([2, 6])
            .on_press(Message::Calendar(CalendarMessage::CalendarTwoWeekMore(
                Some(day),
            )))
            .style(crate::ui::controls::ghost)
            .into(),
        );
    }
    let body = body.extend(items);

    let dragging = state.timeblocking.dragging.is_some();
    let cell: Element<'a, Message> = container(body)
        .padding(CELL_PADDING)
        .width(Length::Fixed(cell.width))
        .height(Length::Fixed(cell.height))
        .clip(true)
        .style(move |theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.weakest.color.into()),
                border: iced::Border {
                    color: if dragging {
                        palette.primary.strong.color
                    } else {
                        palette.background.strong.color
                    },
                    width: if dragging { 1.5 } else { 0.5 },
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .into();
    Element::from(
        mouse_area(cell)
            .on_press(Message::Calendar(
                CalendarMessage::CalendarTwoWeekCellPressed(day),
            ))
            .on_release(Message::TimeBlocking(crate::timeblocking::Action::DropDay)),
    )
    .map(move |message| match message {
        // It carries its own day, and wraps the draft it starts.
        Message::Calendar(CalendarMessage::CalendarTwoWeekCellPressed(_)) => message,
        message => Message::Calendar(CalendarMessage::WeekDayAction(day, Box::new(message))),
    })
}

/// The task in the air as a row of a cell, where it would land.
fn preview_row<'a>(state: &State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
    if task.all_day {
        crate::ui::drop_preview::chip(state, task, metrics.label())
    } else {
        crate::ui::drop_preview::outlined(timed_row(state, task, false, metrics))
    }
}

/// A day's events in reading order: all-day ones first, then by start time.
/// Each row comes with the slot it fills, for dropping a task between rows.
fn day_items<'a>(
    state: &'a State,
    day: NaiveDate,
    metrics: &Metrics,
) -> Vec<(Option<crate::ui::drop_preview::Slot>, Element<'a, Message>)> {
    let now = crate::clock::now();
    let day_start_hour = state.config.calendar.day_start_hour();
    let first_shown = state
        .config
        .calendar
        .first_day_of_week
        .week_start(state.week_anchor);
    let deadlines = state
        .calendar
        .deadline_markers_on(day, &state.config.calendar);
    let mut items = Vec::new();
    for event in state
        .calendar
        .all_day_events_on(day, &state.config.calendar)
        .chain(deadlines.iter().filter(|event| event.all_day))
    {
        if !state
            .calendar
            .search_shows(event, day, &state.config.calendar)
        {
            continue;
        }
        // Overdue and undated tasks belong in the pending list rather than
        // appearing on every date.
        if event.is_task() && !calendar::deadline::is_marker(event) {
            if event.start.date_naive() == day {
                items.push((
                    Some(crate::ui::drop_preview::Slot::of(event)),
                    crate::ui::controls::locked(
                        task_row(state, event, metrics),
                        event.details.sending,
                    ),
                ));
            }
        } else {
            items.push((
                Some(crate::ui::drop_preview::Slot::of(event)),
                crate::ui::controls::locked(
                    all_day_row(state, event, event.end <= now, metrics),
                    event.details.sending,
                ),
            ));
        }
    }
    for event in state
        .calendar
        .timed_events_on(day, &state.config.calendar)
        .into_iter()
        .chain(deadlines.iter().filter(|event| !event.all_day))
    {
        if !state
            .calendar
            .search_shows(event, day, &state.config.calendar)
        {
            continue;
        }
        // An event past midnight is listed where it starts, not on each day it
        // touches, unless the day it started on is out of view.
        if calendar::day_at(event.start, day_start_hour) == day || day == first_shown {
            items.push((
                Some(crate::ui::drop_preview::Slot::of(event)),
                crate::ui::controls::locked(
                    timed_row(state, event, event.end <= now, metrics),
                    event.details.sending,
                ),
            ));
        }
    }
    items
}

/// A filled bar, so an all-day event stands apart from the timed list below it.
fn all_day_row<'a>(
    state: &State,
    event: &Event,
    past: bool,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let hidden = state.calendar.search.hides_details(event);
    let color = if hidden {
        PRIVATE_COLOR
    } else {
        event.display_color(&state.config.calendar)
    };
    let ink = on_color(color);
    let title = if hidden {
        PRIVATE_TITLE.to_owned()
    } else if event.completed && !event.details.habit {
        format!("✓ {}", event.title)
    } else {
        event.title.clone()
    };
    let alpha = if past { PAST_ALPHA } else { 1.0 };
    button(event_label(state, event, title, metrics.label()))
        .width(Fill)
        .height(ROW_HEIGHT)
        .padding([2, 6])
        .clip(true)
        .on_press(Message::Event(EventMessage::EventOpened(event.id.clone())))
        .style(move |_theme, status| {
            let mut style = event_chip_style(color, ink, status);
            style.background = style
                .background
                .map(|background| background.scale_alpha(alpha));
            style.text_color = style.text_color.scale_alpha(alpha);
            style.shadow = iced::Shadow::default();
            style
        })
        .into()
}

/// A dot in the event's color, then its time and title.
fn timed_row<'a>(
    state: &State,
    event: &Event,
    past: bool,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let hidden = state.calendar.search.hides_details(event);
    let color = if hidden {
        PRIVATE_COLOR
    } else {
        event.display_color(&state.config.calendar)
    };
    let alpha = if past { PAST_ALPHA } else { 1.0 };
    let clock = calendar::time_format::clock(event.start, state.config.appearance.time_format);
    let label = if hidden {
        PRIVATE_TITLE.to_owned()
    } else if event.completed && !event.details.habit {
        format!("{clock} ✓ {}", event.title)
    } else {
        format!("{clock} {}", event.title)
    };
    let dot = container(Space::new())
        .width(DOT_SIZE)
        .height(DOT_SIZE)
        .style(move |_theme: &Theme| container::Style {
            background: Some(color.scale_alpha(alpha).into()),
            border: iced::Border {
                radius: (DOT_SIZE / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
    button(
        row![dot, event_label(state, event, label, metrics.label()),]
            .spacing(6)
            .align_y(Alignment::Center),
    )
    .width(Fill)
    .height(ROW_HEIGHT)
    .padding([2, 6])
    .clip(true)
    .on_press(Message::Event(EventMessage::EventOpened(event.id.clone())))
    .style(move |theme, status| {
        let mut style = crate::ui::controls::ghost(theme, status);
        style.text_color = style.text_color.scale_alpha(alpha);
        style
    })
    .into()
}

/// A task with its checkbox. It can be dragged to another day, as in the week
/// and month views.
fn task_row<'a>(state: &'a State, task: &Event, metrics: &Metrics) -> Element<'a, Message> {
    let hidden = state.calendar.search.hides_details(task);
    let mut check = button(
        text(if task.completed && !hidden {
            "●"
        } else {
            "○"
        })
        .size(metrics.label()),
    )
    .padding(0)
    .style(crate::ui::controls::ghost);
    if state.timeblocking.busy.is_empty() && !hidden {
        check = check.on_press(Message::Task(TaskMessage::TaskToggled(task.id.clone())));
    }
    let label = if hidden {
        PRIVATE_TITLE.to_owned()
    } else {
        crate::timeblocking::task_label(task)
    };
    let title = button(
        text(label)
            .size(metrics.label())
            .font(crate::ui::controls::pending_font(
                iced::Font::DEFAULT,
                task.details.sending,
            ))
            .wrapping(text::Wrapping::None),
    )
    .padding(0)
    .width(Fill)
    .clip(true)
    .on_press(Message::Event(EventMessage::EventOpened(task.id.clone())))
    .style(crate::ui::controls::ghost);
    super::super::task_drag::task_drag(
        container(row![check, title].spacing(4).align_y(Alignment::Center))
            .width(Fill)
            .height(ROW_HEIGHT)
            .padding([0, 6])
            .align_y(Alignment::Center)
            .clip(true),
        &task.id,
        state.timeblocking.dragging.as_deref(),
    )
}

/// Everything on one day, for when its cell has run out of room.
fn more_popup<'a>(state: &'a State, day: NaiveDate, metrics: &Metrics) -> Element<'a, Message> {
    let close = Message::Calendar(CalendarMessage::CalendarTwoWeekMore(None));
    let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill)).on_press(close);
    let title = button(
        text(day.format("%a, %b %-d").to_string())
            .size(metrics.label() + 1.0)
            .center()
            .width(Fill),
    )
    .width(Fill)
    .on_press(Message::Calendar(CalendarMessage::CalendarDayOpened(day)))
    .style(button::text);
    let items = column(
        day_items(state, day, metrics)
            .into_iter()
            .map(|(_, row)| row),
    )
    .spacing(ROW_GAP)
    .width(Fill);
    let list = Element::from(crate::ui::controls::scrollable(items)).map(move |message| {
        Message::Calendar(CalendarMessage::WeekDayAction(day, Box::new(message)))
    });
    let card = container(column![title, list].spacing(metrics.spacing))
        .padding(metrics.padding)
        .width(POPUP_WIDTH.min(metrics.size.width))
        .max_height((metrics.size.height - metrics.padding * 2.0).max(1.0))
        .style(crate::ui::controls::surface);
    stack![
        dismiss,
        container(iced::widget::opaque(card))
            .width(Fill)
            .height(Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
    ]
    .into()
}
