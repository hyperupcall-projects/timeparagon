//! Google Calendar day view, drawn to resemble the web UI.

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Style, Text};
use iced::widget::{
    button, canvas as canvas_widget, column, container, mouse_area, row, scrollable, stack, text,
    text_input, toggler, Space,
};
use iced::{
    Alignment, Color, Element, Fill, Font, Length, Pixels, Point, Rectangle, Renderer, Size, Theme,
    mouse,
};

use crate::app::{Message, Screen, State};
use crate::calendar::{self, Event};
use crate::calendar::colors::{self, on_color};
use crate::calendar::layout::{self, Span};
use crate::engine::{Precision, format};

use super::layout::Metrics;

const GUTTER: f32 = 52.0;
const COMPACT_GUTTER: f32 = 40.0;
const EVENT_GAP: f32 = 2.0;
const EVENT_RADIUS: f32 = 4.0;
const NEEDLE_RADIUS: f32 = 5.5;

/// How close to a block's top or bottom edge a press has to land to resize it
/// rather than move it. The middle of the block is always the move handle.
const EDGE_GRAB: f32 = 8.0;
/// Dragged times snap to this many minutes, the way Google Calendar's own day
/// view does. The event keeps its offset, so a 9:07 meeting moves in 15-minute
/// steps rather than jumping to the quarter hour.
const SNAP_MINUTES: i32 = 15;
/// The shortest an event can be dragged to.
const MIN_DURATION: i32 = SNAP_MINUTES;

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    // The clock takes the day grid's place while it is open. The calendar is
    // still one tap away: the button at the foot of the view folds it away
    // again, and a timer left running keeps going.
    let body: Element<'a, Message> = if state.timer_open {
        super::clock_body(state, metrics)
    } else {
        let all_day = all_day_row(state, metrics);
        let grid = day_grid(state, metrics, Local::now());

        // The pointer watch records where the click landed without publishing a
        // message, so scrolling stays on the widget's own path.
        pointer_watch(
            column![all_day, grid]
                .spacing(metrics.spacing)
                .height(Fill)
                .width(Fill),
        )
    };

    // The month picker, the menu, the day arrows and the sync line belong to the
    // day view. With the clock open they would only crowd the readout, so they
    // are left out until the clock is folded away again.
    let mut content = column![];

    if !state.timer_open {
        let status = row![
            text(state.calendar.status_line())
                .size(metrics.label() - 1.0)
                .style(if state.calendar.error.is_some() {
                    text::danger
                } else {
                    text::secondary
                }),
            tasks_toggle(state, metrics),
        ]
        .spacing(metrics.spacing)
        .align_y(Alignment::Center);

        content = content.push(header(state, metrics)).push(status);
    }

    let content = content
        .push(body)
        .push(clock_footer(state, metrics))
        .spacing(metrics.spacing)
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill);

    // The root stays a stack whether or not a card is open. Swapping the root
    // widget throws away the day grid's scroll state, which jumps the bar.
    let overlay = if state.calendar.open_event.is_some() {
        details_overlay(state, metrics)
    } else if state.calendar.month_open {
        month_overlay(state, metrics)
    } else if state.calendar.menu_open {
        menu_overlay(state, metrics)
    } else {
        iced::widget::Space::new().width(0.0).height(0.0).into()
    };

    stack![content, overlay].into()
}

/// The top of the calendar: a month picker, the calendars menu and the way into
/// settings, then the day in view flanked by the arrows that step it a day at a
/// time.
///
/// The controls are split across two rows because at 200px a single row cannot
/// hold the arrows, the month button, the heading and the menu without the
/// heading being squeezed to nothing.
fn header<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let today = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
    let heading = calendar::day_heading(state.calendar.day, today);

    let month = button(
        text(format!("{} ▾", calendar::month_short(state.calendar.day.month())))
            .size(metrics.label())
            .center(),
    )
    .on_press(Message::CalendarMonthToggled)
    .style(if state.calendar.month_open {
        button::primary
    } else {
        button::secondary
    });

    let menu = button(text("Calendars").size(metrics.label()).center())
        .on_press(Message::CalendarMenuToggled)
        .style(if state.calendar.menu_open {
            button::primary
        } else {
            button::secondary
        });

    let settings = button(text("Settings").size(metrics.label()).center())
        .on_press(Message::ScreenSelected(crate::app::Screen::Settings))
        .style(button::secondary);

    let top = row![
        month,
        iced::widget::Space::new().width(Fill),
        menu,
        settings
    ]
    .spacing(metrics.spacing)
    .align_y(Alignment::Center);

    let nav = row![
        nav_button("◀", Message::CalendarDayShift(-1), metrics),
        text(heading)
            .size(metrics.label() + 1.0)
            .width(Fill)
            .center(),
        nav_button("▶", Message::CalendarDayShift(1), metrics),
    ]
    .spacing(metrics.spacing)
    .align_y(Alignment::Center);

    column![top, nav].spacing(metrics.spacing).into()
}

/// A square button for stepping a day or a month at a time.
fn nav_button<'a>(label: &'a str, message: Message, metrics: &Metrics) -> Element<'a, Message> {
    button(text(label).size(metrics.label()).center())
        .width(Length::Fixed(metrics.icon_button()))
        .height(Length::Fixed(metrics.icon_button()))
        .padding(0.0)
        .on_press(message)
        .style(button::secondary)
        .into()
}

/// The vertical space the two-row header takes, so an overlay can start below
/// it rather than over it.
fn header_offset(metrics: &Metrics) -> f32 {
    metrics.padding + metrics.icon_button() * 2.0 + metrics.spacing * 2.0
}

/// The small switch beside the sync line: the same setting the menu and the
/// settings pane offer, one tap away from the day grid.
fn tasks_toggle<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let shown = state.config.calendar.show_tasks;
    let label = if shown { "Hide tasks" } else { "Show tasks" };

    button(text(label).size(metrics.label() - 1.0).center())
        .padding([2.0, 6.0])
        .on_press(Message::ShowTasksToggled(!shown))
        .style(button::secondary)
        .into()
}

/// The foot of the calendar: what the clock is doing, and the way into it.
///
/// The clock lives inside the calendar rather than on a screen of its own, so
/// this row is where a timer is started and where it is put away again. The
/// status stays even with the clock folded away, because a timer left running
/// should still be readable at a glance.
fn clock_footer<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    // With the clock open the readout above already says what the footer would,
    // so the only thing left to offer is the way back.
    if state.timer_open {
        return button(text("Go back").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .height(metrics.icon_button())
            .on_press(Message::TimerClosed)
            .style(button::secondary)
            .into();
    }

    let allow_hours = metrics.allow_hours;
    let elapsed = format::readout(
        state.timer.elapsed(state.now),
        Precision::Seconds,
        allow_hours,
    );
    let target = match state.timer.target() {
        Some(target) => format!("of {}", format::readout(target, Precision::Seconds, allow_hours)),
        None => "No target".to_owned(),
    };

    // An alarm can go off with the clock put away, so this button doubles as
    // the way to silence it.
    let (label, message, style) = if state.alarm_active {
        ("Dismiss", Message::Dismiss, button::danger as fn(&_, _) -> _)
    } else {
        ("Timer", Message::TimerOpened, button::secondary as fn(&_, _) -> _)
    };

    let toggle = button(text(label).size(metrics.label()).center())
        .height(metrics.icon_button())
        .on_press(message)
        .style(style);

    let todos = button(text("Todos").size(metrics.label()).center())
        .height(metrics.icon_button())
        .on_press(Message::ScreenSelected(Screen::Todos))
        .style(button::secondary);

    // The "what am I doing" line. The caption doubles as the progress readout,
    // so "2/4" is visible from the calendar without opening the list.
    let caption = match state.config.todos.position() {
        Some((position, total)) => format!("Todos {position}/{total}:"),
        None => "Current todo:".to_owned(),
    };
    let input = text_input("what are you doing?", state.current_task())
        .on_input(Message::CurrentTaskChanged)
        .size(metrics.label())
        .width(Fill);

    // Each button sits at the end of the line it belongs to, so the "of 30:00"
    // tucks in beside the count-up to leave the clock button room.
    let status = row![
        text(elapsed).size(metrics.label()),
        text(target).size(metrics.label()).style(text::secondary),
        Space::new().width(Fill),
        toggle,
    ]
    .spacing(metrics.spacing)
    .align_y(Alignment::Center);

    let task = row![text(caption).size(metrics.label()), input, todos]
        .spacing(metrics.spacing)
        .align_y(Alignment::Center);

    column![status, task].spacing(metrics.spacing).into()
}

fn all_day_row<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let chips: Vec<Element<'a, Message>> = state
        .calendar
        .all_day_events(&state.config.calendar)
        .map(|event| all_day_chip(event, metrics))
        .collect();

    if chips.is_empty() {
        return text("").size(1.0).into();
    }

    column(chips)
        .spacing(4.0)
        .width(Fill)
        .into()
}

fn all_day_chip<'a>(event: &Event, metrics: &Metrics) -> Element<'a, Message> {
    let color = event.color;
    let ink = on_color(color);

    let title = if event.completed {
        format!("✓ {}", event.title)
    } else {
        event.title.clone()
    };

    let id = event.id.clone();
    button(text(title).size(metrics.label()).color(ink))
        .padding([4.0, 8.0])
        .width(Fill)
        .on_press(Message::EventOpened(id))
        .style(move |_theme, status| event_chip_style(color, ink, status))
        .into()
}

fn event_chip_style(color: Color, ink: Color, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(if hovered { lift(color) } else { color }.into()),
        text_color: ink,
        border: iced::Border {
            color: Color::BLACK.scale_alpha(if hovered { 0.9 } else { 0.55 }),
            width: if hovered { 2.0 } else { 1.0 },
            radius: EVENT_RADIUS.into(),
        },
        ..button::Style::default()
    }
}

/// Pulls a color toward white so a hovered chip reads as raised.
fn lift(color: Color) -> Color {
    Color {
        r: color.r + (1.0 - color.r) * 0.28,
        g: color.g + (1.0 - color.g) * 0.28,
        b: color.b + (1.0 - color.b) * 0.28,
        a: color.a,
    }
}

/// The little square that shows a calendar's color in the menu.
fn static_swatch<'a>(color: Color) -> Element<'a, Message> {
    container(iced::widget::Space::new().width(10.0).height(10.0))
        .style(move |_theme| container::Style {
            background: Some(color.into()),
            border: iced::Border {
                radius: 2.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// A swatch that can be tapped, used for a task list's color and in the
/// picker itself. `selected` draws a brighter ring so the current color reads.
fn swatch_button<'a>(color: Color, selected: bool, on_press: Message) -> Element<'a, Message> {
    button(iced::widget::Space::new())
        .width(Length::Fixed(14.0))
        .height(Length::Fixed(14.0))
        .padding(0.0)
        .on_press(on_press)
        .style(move |_theme, status| swatch_style(color, selected, status))
        .into()
}

fn swatch_style(color: Color, selected: bool, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(if hovered { lift(color) } else { color }.into()),
        text_color: on_color(color),
        border: iced::Border {
            color: if selected {
                Color::WHITE
            } else {
                Color::BLACK.scale_alpha(0.45)
            },
            width: if selected { 2.0 } else { 1.0 },
            radius: 3.0.into(),
        },
        ..button::Style::default()
    }
}

/// The palette shown under a task list whose color is being changed. Six
/// swatches per row keeps it inside the sheet even at 200px.
fn task_color_picker<'a>(
    id: &str,
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
                    Message::TaskColorPicked(id.to_owned(), Some(color)),
                )
            })
            .collect();
        rows.push(row(swatches).spacing(6.0).into());
    }

    let reset = button(text("Reset to default").size(metrics.label() - 1.0).center().width(Fill))
        .width(Fill)
        .on_press(Message::TaskColorPicked(id.to_owned(), None))
        .style(button::secondary);

    column![
        text("Task color")
            .size(metrics.label() - 1.0)
            .style(text::secondary),
        column(rows).spacing(6.0),
        reset,
    ]
    .spacing(6.0)
    .width(Fill)
    .into()
}

fn day_grid<'a>(state: &'a State, metrics: &Metrics, now: DateTime<Local>) -> Element<'a, Message> {
    let hour_height = state.calendar.hour_height;
    let gutter = if metrics.breakpoint.is_compact() {
        COMPACT_GUTTER
    } else {
        GUTTER
    };

    let day = state.calendar.day;
    let offset = state.config.calendar.day_end_minutes();
    let day_start_hour = state.config.calendar.day_start_hour();

    let timed: Vec<&Event> = state.calendar.timed_events(&state.config.calendar).collect();
    let spans: Vec<Span> = timed
        .iter()
        .map(|event| {
            let start = minutes_into_day(event.start, day, offset);
            let end = minutes_into_day(event.end, day, offset);
            let (start_min, end_min) = layout::clamp_day(start, end).unwrap_or_else(|| {
                let start = start.clamp(0, DAY_MINUTES);
                (start, (start + 15).min(DAY_MINUTES))
            });
            Span {
                start_min,
                end_min,
            }
        })
        .collect();
    let packed = layout::pack(&spans);

    let blocks: Vec<Block> = timed
        .iter()
        .zip(spans.iter())
        .zip(packed.iter())
        .map(|((event, span), slot)| Block {
            id: event.id.clone(),
            title: event.title.clone(),
            when: calendar::event_time_label(event),
            start_min: span.start_min,
            end_min: span.end_min,
            column: slot.column,
            columns: slot.columns,
            color: event.color,
            // A Google Task has no time to move, and an event whose time is
            // already being written cannot be dragged again until it lands.
            editable: !event.is_task(),
            pending: state.calendar.pending_time(&event.id).is_some(),
        })
        .collect();

    // The needle only belongs to the day whose window holds "now".
    let needle = (day == calendar::day_at(now, day_start_hour))
        .then(|| minutes_into_day(now, day, offset).clamp(0, DAY_MINUTES));

    let height = hour_height * 24.0;
    let compact = metrics.breakpoint.is_compact();

    scrollable(
        canvas_widget(DayCanvas {
            blocks,
            needle,
            hour_height,
            gutter,
            compact,
            day_start_hour,
            day,
        })
        .width(Fill)
        .height(Length::Fixed(height)),
    )
    .id(iced::widget::Id::new(DAY_SCROLL))
    .height(Fill)
    .width(Fill)
    .into()
}

/// Minutes in a day, the height of the grid in minutes.
const DAY_MINUTES: i32 = 24 * 60;

/// Minutes from the start of the displayed day.
///
/// The day's window opens at the configured end-of-day hour, so an event at
/// 1am lands at the foot of the day that ends at 3am. The event's own date and
/// time are never touched; only this offset moves. Values outside the window
/// come back negative or past 24h and are clamped away by the caller.
fn minutes_into_day(when: DateTime<Local>, day: chrono::NaiveDate, offset: i32) -> i32 {
    let days = (when.date_naive() - day).num_days() as i32;
    days * DAY_MINUTES + calendar::minutes_from_midnight(when) - offset
}

const DAY_SCROLL: &str = "cal-day";

/// Scrolls the day grid so the needle sits a little below the top, the way
/// Google Calendar opens on "now" rather than the start of the day.
pub fn scroll_to_now(hour_height: f32, day_end_minutes: i32) -> iced::Task<Message> {
    let minutes = (calendar::minutes_from_midnight(Local::now()) - day_end_minutes)
        .rem_euclid(DAY_MINUTES);
    let y = ((minutes - 90) as f32 * (hour_height / 60.0)).max(0.0);

    iced::widget::operation::scroll_to(
        iced::widget::Id::new(DAY_SCROLL),
        iced::widget::operation::AbsoluteOffset { x: 0.0, y },
    )
}

/// Scrolls the day grid by a pixel amount.
///
/// Used after a zoom to keep the row under the pointer still: the grid grows
/// or shrinks around that row, so the scroll follows it by the same delta.
pub fn scroll_by(delta: f32) -> iced::Task<Message> {
    iced::widget::operation::scroll_by(
        iced::widget::Id::new(DAY_SCROLL),
        iced::widget::operation::AbsoluteOffset { x: 0.0, y: delta },
    )
}

/// The scroll adjustment that keeps the row under the pointer still while the
/// hour height changes from `before` to `after`.
///
/// `anchor` is the pointer's row measured from the top of the grid. The grid
/// grows by `after / before`, so that row slides down by `anchor * (ratio - 1)`
/// and the scroll has to follow it by the same amount to stay under the mouse.
pub fn zoom_scroll_delta(anchor: f32, before: f32, after: f32) -> f32 {
    if !before.is_finite() || !after.is_finite() || before <= 0.0 {
        return 0.0;
    }

    anchor * (after / before - 1.0)
}

fn menu_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let dismiss = mouse_area(container(iced::widget::Space::new()).width(Fill).height(Fill))
        .on_press(Message::CalendarMenuToggled);

    let mut panel = column![
        text("Calendars").size(metrics.label() + 1.0),
        row![
            text("Show Google Tasks").size(metrics.label()).width(Fill),
            toggler(state.config.calendar.show_tasks).on_toggle(Message::ShowTasksToggled),
        ]
        .spacing(8.0)
        .align_y(Alignment::Center),
    ]
    .spacing(metrics.spacing)
    .width(Fill);

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
    } else {
        for calendar in &state.calendar.calendars {
            let is_task_list = calendar.is_task_list();

            // Task lists are Google Tasks, so they follow the same switch.
            if !state.config.calendar.show_tasks && is_task_list {
                continue;
            }

            let id = calendar.id.clone();
            let enabled = state.calendar.calendar_enabled(&calendar.id, &state.config.calendar);
            let swatch = calendar.color;
            let editing = state.calendar.color_editor.as_deref() == Some(id.as_str());

            // A task list's swatch opens the color picker; other calendars keep
            // a plain dot, since only Google Tasks colors can be changed.
            let dot: Element<'a, Message> = if is_task_list {
                swatch_button(swatch, editing, Message::TaskColorEditorToggled(id.clone()))
            } else {
                static_swatch(swatch)
            };

            let toggle_id = id.clone();
            panel = panel.push(
                row![
                    dot,
                    text(calendar.name.clone())
                        .size(metrics.label())
                        .width(Fill),
                    toggler(enabled)
                        .on_toggle(move |value| Message::CalendarToggled(toggle_id.clone(), value)),
                ]
                .spacing(8.0)
                .align_y(Alignment::Center),
            );

            if is_task_list && editing {
                let current = state.config.calendar.task_color(&id);
                panel = panel.push(task_color_picker(&id, current, metrics));
            }
        }
    }

    let sync_label = if state.calendar.is_busy() {
        "Syncing…"
    } else {
        "Sync now"
    };

    let mut sync = button(text(sync_label).size(metrics.label()).center().width(Fill))
        .width(Fill)
        .style(button::primary);

    if state.calendar.is_signed_in() && !state.calendar.is_busy() {
        sync = sync.on_press(Message::CalendarSyncRequested);
    }

    panel = panel.push(sync);
    panel = panel.push(
        button(text("Open Google Calendar").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(Message::OpenGoogleCalendar)
            .style(button::secondary),
    );

    let card = container(panel)
        .padding(metrics.padding)
        .width(Length::Fill)
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                border: iced::Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..container::Style::default()
            }
        });

    let aligned = column![
        // Keep the header row tappable; the sheet starts below it.
        iced::widget::Space::new().height(header_offset(metrics)),
        container(card).padding(metrics.padding).width(Fill),
    ]
    .width(Fill);

    stack![dismiss, aligned].into()
}

/// The month picker: a grid of the month's days, with arrows to step a month
/// at a time. Tapping a day jumps the day view to it.
fn month_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let dismiss = mouse_area(container(iced::widget::Space::new()).width(Fill).height(Fill))
        .on_press(Message::CalendarMonthToggled);

    let month = state.calendar.month;
    let today = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
    let selected = state.calendar.day;

    let title = row![
        nav_button("◀", Message::CalendarMonthShift(-1), metrics),
        text(calendar::month_heading(month))
            .size(metrics.label() + 1.0)
            .width(Fill)
            .center(),
        nav_button("▶", Message::CalendarMonthShift(1), metrics),
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
    .spacing(2.0)
    .width(Fill);

    let days = calendar::month_grid(month);
    let mut weeks: Vec<Element<'a, Message>> = Vec::new();
    for week in days.chunks(7) {
        let cells: Vec<Element<'a, Message>> = week
            .iter()
            .map(|&date| day_cell(date, month, selected, today, metrics))
            .collect();
        weeks.push(row(cells).spacing(2.0).width(Fill).into());
    }

    let panel = column![
        title,
        weekdays,
        column(weeks).spacing(2.0).width(Fill),
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    let card = container(panel)
        .padding(metrics.padding)
        .width(Length::Fill)
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                border: iced::Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..container::Style::default()
            }
        });

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
fn day_cell<'a>(
    date: NaiveDate,
    month: NaiveDate,
    selected: NaiveDate,
    today: NaiveDate,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let in_month = date.month() == month.month() && date.year() == month.year();
    let is_selected = date == selected;
    let is_today = date == today;

    button(
        text(date.day().to_string())
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill)
    .padding([4.0, 0.0])
    .on_press(Message::CalendarDaySelected(date))
    .style(move |theme, status| {
        if is_selected {
            button::primary(theme, status)
        } else if is_today {
            button::secondary(theme, status)
        } else {
            let mut style = button::text(theme, status);
            if !in_month {
                style.text_color = style.text_color.scale_alpha(0.4);
            }
            style
        }
    })
    .into()
}

fn details_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(event) = state
        .calendar
        .open_event
        .as_ref()
        .and_then(|id| state.calendar.event(id))
    else {
        return iced::widget::Space::new().width(0.0).height(0.0).into();
    };

    let calendar_name = state
        .calendar
        .calendars
        .iter()
        .find(|calendar| calendar.id == event.calendar_id)
        .map(|calendar| calendar.name.as_str())
        .unwrap_or("Calendar");

    let mut body = column![
        text(event.title.clone()).size(metrics.label() + 2.0),
        text(schedule_line(event)).size(metrics.label()).style(text::secondary),
        detail_line("Calendar", calendar_name.to_owned(), metrics),
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    let details = &event.details;
    if !is_task(event) && !details.status.is_empty() {
        body = body.push(detail_line("Status", details.status.clone(), metrics));
    }
    if !details.location.is_empty() {
        body = body.push(detail_line("Location", details.location.clone(), metrics));
    }
    if !details.organizer.is_empty() {
        body = body.push(detail_line("Organizer", details.organizer.clone(), metrics));
    }
    if !details.attendees.is_empty() {
        body = body.push(detail_line(
            "Guests",
            details.attendees.join("\n"),
            metrics,
        ));
    }
    if !details.description.is_empty() {
        body = body.push(detail_line("Description", details.description.clone(), metrics));
    }

    // The action buttons sit outside the scrollable, so they stay reachable
    // however long the description above them runs.
    let mut buttons: Vec<Element<'a, Message>> = Vec::new();

    // A Google Task gets a way to tick it off; events never do.
    if is_task(event) {
        let label = if event.completed {
            "Mark incomplete"
        } else {
            "Mark complete"
        };
        let id = event.id.clone();
        buttons.push(
            button(text(label).size(metrics.label()).center().width(Fill))
                .width(Fill)
                .on_press(Message::TaskToggled(id))
                .style(button::primary)
                .into(),
        );
    }

    if !details.meeting.is_empty() {
        let meeting = details.meeting.clone();
        buttons.push(
            button(text("Join meeting").size(metrics.label()).center().width(Fill))
                .width(Fill)
                .on_press(Message::OpenEventLink(meeting))
                .style(button::primary)
                .into(),
        );
    }
    if !details.link.is_empty() {
        let link = details.link.clone();
        let label = if is_task(event) {
            "Open in Google Tasks"
        } else {
            "Open in Google Calendar"
        };
        buttons.push(
            button(text(label).size(metrics.label()).center().width(Fill))
                .width(Fill)
                .on_press(Message::OpenEventLink(link))
                .style(button::secondary)
                .into(),
        );
    }
    buttons.push(
        button(text("Close").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(Message::EventClosed)
            .style(button::secondary)
            .into(),
    );

    let max_height = (metrics.size.height * 0.42).clamp(140.0, 420.0);
    // A button is normally one line of text plus its vertical padding, so
    // reserving that leaves the scroll every pixel the buttons do not need.
    // The reserve is generous enough that a label wrapping at the narrowest
    // widths still lands inside the card rather than under the scrollbar.
    let button_height = metrics.label() * 1.3 + 10.0;
    let reserved = button_height * buttons.len() as f32
        + metrics.spacing * buttons.len().saturating_sub(1) as f32;
    // The buttons come first in the budget: the scroll gets whatever is left,
    // even if that is nothing, so the actions can never be squeezed out.
    let scroll_max = (max_height - reserved - metrics.spacing).max(0.0);

    let scrolling = container(scrollable(body).width(Fill).height(Length::Shrink))
        .max_height(scroll_max)
        .width(Fill);

    let card = container(
        column![scrolling, column(buttons).spacing(metrics.spacing).width(Fill)]
            .spacing(metrics.spacing)
            .width(Fill),
    )
    .padding(metrics.padding)
    .width(Fill)
    .style(|theme: &Theme| {
        let palette = theme.extended_palette();
        container::Style {
            background: Some(palette.background.base.color.into()),
            border: iced::Border {
                color: palette.background.strong.color,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        }
    });

    let card = container(card).padding(metrics.padding).width(Fill);

    // Swallow presses on the card itself so a click on its blank area cannot
    // fall through to the day grid and open whatever event sits underneath.
    // There is deliberately no full-screen dismiss layer: with one, clicking a
    // different event would only close the card instead of opening the new one.
    let card = mouse_area(card).on_press(Message::Ignored);

    // A click in the top half parks the card at the bottom, and the reverse,
    // so the event that was clicked stays visible.
    let aligned = if state.calendar.details_below {
        column![iced::widget::Space::new().height(Fill), card]
    } else {
        column![
            iced::widget::Space::new().height(metrics.icon_button() + metrics.spacing),
            card,
            iced::widget::Space::new().height(Fill),
        ]
    }
    .width(Fill)
    .height(Fill);

    aligned.into()
}

fn is_task(event: &Event) -> bool {
    event.is_task()
}

fn detail_line<'a>(label: &'a str, value: String, metrics: &Metrics) -> Element<'a, Message> {
    column![
        text(label).size(metrics.label() - 1.0).style(text::secondary),
        text(value).size(metrics.label()).width(Fill),
    ]
    .spacing(2.0)
    .width(Fill)
    .into()
}

fn schedule_line(event: &Event) -> String {
    let date = event.start.format("%A, %B %d, %Y");
    if event.all_day {
        format!("{date} · All day")
    } else if event.start.date_naive() != event.end.date_naive() {
        format!(
            "{date} · {} – {}",
            calendar::event_time_label(event),
            event.end.format("%A, %B %d")
        )
    } else {
        format!("{date} · {}", calendar::event_time_label(event))
    }
}

#[derive(Clone, Copy)]
struct PointerSample {
    y: f32,
    height: f32,
}

thread_local! {
    /// The last cursor position seen inside the day view. Kept per-thread so
    /// it needs no locking and never leaks across a test's simulations.
    static SAMPLE: std::cell::Cell<PointerSample> = const {
        std::cell::Cell::new(PointerSample {
            y: 0.0,
            height: 0.0,
        })
    };
}

pub(crate) fn record_pointer(y: f32, height: f32) {
    SAMPLE.with(|sample| sample.set(PointerSample { y, height }));
}

/// True when the latest click landed in the top half of the day view.
pub fn click_was_in_the_top_half() -> bool {
    SAMPLE.with(|sample| {
        let sample = sample.get();
        sample.height <= 0.0 || sample.y <= sample.height / 2.0
    })
}

/// Remembers the cursor inside the day view without sending a message.
///
/// A message on every move would rebuild the grid while the pointer travels.
/// The click handler reads the sample after this widget has stored it.
fn pointer_watch<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    Element::new(PointerWatch {
        content: content.into(),
    })
}

struct PointerWatch<'a> {
    content: Element<'a, Message>,
}

impl iced::advanced::widget::Widget<Message, Theme, Renderer> for PointerWatch<'_> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<iced::advanced::widget::Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut iced::advanced::widget::Tree) {
        self.content.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        renderer: &Renderer,
        limits: &iced::advanced::layout::Limits,
    ) -> iced::advanced::layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        operation.traverse(&mut |operation| {
            self.content
                .as_widget_mut()
                .operate(tree, layout, renderer, operation);
        });
    }

    fn update(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        event: &iced::Event,
        layout: iced::advanced::Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if let Some(point) = cursor.position_in(layout.bounds()) {
            record_pointer(point.y, layout.bounds().height);
        }

        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }

    fn draw(
        &self,
        tree: &iced::advanced::widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &iced::advanced::renderer::Style,
        layout: iced::advanced::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            tree, renderer, theme, style, layout, cursor, viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, Theme, Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

#[derive(Debug, Clone)]
struct Block {
    id: String,
    title: String,
    when: String,
    start_min: i32,
    end_min: i32,
    column: usize,
    columns: usize,
    color: Color,
    /// Whether the block can be dragged. Google Tasks cannot: they have no
    /// time of their own to move.
    editable: bool,
    /// Whether a time change for this event is still being written to Google.
    /// The block is drawn lighter until the write is confirmed.
    pending: bool,
}

struct DayCanvas {
    blocks: Vec<Block>,
    needle: Option<i32>,
    hour_height: f32,
    gutter: f32,
    compact: bool,
    /// The hour drawn at the top of the grid. The day's window opens here, so
    /// with a 3am end of day the top row reads 3 AM and the small hours sit at
    /// the foot.
    day_start_hour: u32,
    /// The date the grid is showing, so a dragged block can be turned back into
    /// a real time when it is dropped.
    day: NaiveDate,
}

/// What a drag on a block is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragMode {
    /// Slide the whole event, keeping its length.
    Move,
    /// Pull the top edge up or down, keeping the end fixed.
    ResizeStart,
    /// Pull the bottom edge up or down, keeping the start fixed.
    ResizeEnd,
}

/// A drag in progress on the day grid.
///
/// The preview is kept here rather than in the store: the event is only written
/// once, when the pointer is released, so a drag does not rebuild the day on
/// every pixel of travel. The event is named by id rather than by its place in
/// the block list, so a sync landing mid-drag cannot move the wrong one.
#[derive(Debug, Clone)]
struct Drag {
    /// The event being dragged.
    id: String,
    mode: DragMode,
    /// Where the pointer was when the drag started, in grid pixels.
    origin_y: f32,
    /// The block's start and end when the drag started, in minutes.
    start_min: i32,
    end_min: i32,
    /// The times the block is drawn at while the drag is in progress.
    preview_start: i32,
    preview_end: i32,
}

/// Geometry for the grid, the events and their labels, reused while scrolling.
///
/// The labels are traced into glyph outlines by [`fill_glyphs`], and that
/// tracing is the expensive part: it shapes every string and builds a path for
/// every glyph. The cache keeps that work for when the events or the zoom
/// actually change. The needle is drawn outside the cache so it can move
/// without rebuilding the titles.
struct DrawState {
    grid: canvas::Cache,
    key: std::cell::Cell<u64>,
    shift: std::cell::Cell<bool>,
    /// Index of the timed event under the pointer. Kept out of the grid cache.
    hovered: std::cell::Cell<Option<usize>>,
    /// The drag in progress, if any. Kept out of the grid cache so the preview
    /// can follow the pointer without retracing the day.
    drag: std::cell::RefCell<Option<Drag>>,
}

impl DrawState {
    /// A copy of the drag in progress, if there is one.
    fn drag(&self) -> Option<Drag> {
        self.drag.borrow().as_ref().cloned()
    }

    /// Replaces the drag in progress, or clears it with `None`.
    fn set_drag(&self, drag: Option<Drag>) {
        *self.drag.borrow_mut() = drag;
    }
}

impl Default for DrawState {
    fn default() -> Self {
        Self {
            grid: canvas::Cache::new(),
            key: std::cell::Cell::new(0),
            shift: std::cell::Cell::new(false),
            hovered: std::cell::Cell::new(None),
            drag: std::cell::RefCell::new(None),
        }
    }
}

impl canvas::Program<Message> for DayCanvas {
    type State = DrawState;

    fn update(
        &self,
        state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => {
                state.shift.set(modifiers.shift());
                None
            }
            iced::Event::Mouse(mouse::Event::WheelScrolled { delta })
                if state.shift.get() && cursor.position_in(bounds).is_some() =>
            {
                let steps = zoom_steps(*delta);
                if steps == 0.0 {
                    return None;
                }

                // The row under the pointer, in grid pixels, so the zoom can
                // keep it anchored instead of letting the grid slide away.
                let anchor = cursor.position_in(bounds)?.y;

                // Scroll up spreads the hours apart; scroll down packs them.
                Some(
                    canvas::Action::publish(Message::CalendarZoomed {
                        factor: 1.12_f32.powf(steps),
                        anchor,
                    })
                    .and_capture(),
                )
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                // A drag owns the pointer until it is released, so the hover
                // highlight stays out of the way while the block follows it.
                if let Some(drag) = state.drag() {
                    if let Some(point) = cursor.position_in(bounds) {
                        let moved = self.preview(&drag, point.y);
                        if moved.preview_start != drag.preview_start
                            || moved.preview_end != drag.preview_end
                        {
                            state.set_drag(Some(moved));
                            return Some(canvas::Action::request_redraw().and_capture());
                        }
                    }
                    return Some(canvas::Action::capture());
                }

                self.track_hover(state, bounds, cursor)
            }
            iced::Event::Mouse(mouse::Event::CursorLeft) => self.track_hover(state, bounds, cursor),
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let point = cursor.position_in(bounds)?;

                // A press on a draggable block starts a drag. The click that
                // opens the details card is decided on release, so a press that
                // never moves still opens it.
                if let Some((index, mode)) = self.grab(point, bounds.width)
                    && self.draggable(index)
                {
                    let block = self.blocks.get(index)?;
                    state.set_drag(Some(Drag {
                        id: block.id.clone(),
                        mode,
                        origin_y: point.y,
                        start_min: block.start_min,
                        end_min: block.end_min,
                        preview_start: block.start_min,
                        preview_end: block.end_min,
                    }));
                    return Some(canvas::Action::request_redraw().and_capture());
                }

                // Empty time closes any open card, which is how clicking away
                // still works now that there is no dismiss layer.
                match self.hit(Some(point), bounds.width) {
                    Some(index) => {
                        let id = self.blocks.get(index)?.id.clone();
                        Some(canvas::Action::publish(Message::EventOpened(id)).and_capture())
                    }
                    None => Some(canvas::Action::publish(Message::EventClosed).and_capture()),
                }
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                let drag = state.drag.take()?;

                // A press that never moved is a click, not a drag.
                let action = if drag.preview_start == drag.start_min
                    && drag.preview_end == drag.end_min
                {
                    canvas::Action::publish(Message::EventOpened(drag.id))
                } else {
                    canvas::Action::publish(Message::EventTimeChanged {
                        id: drag.id,
                        start: self.time_at(drag.preview_start),
                        end: self.time_at(drag.preview_end),
                    })
                };

                Some(action.and_capture())
            }
            _ => None,
        }
    }

    fn mouse_interaction(
        &self,
        state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if let Some(drag) = state.drag() {
            return match drag.mode {
                DragMode::Move => mouse::Interaction::Grabbing,
                DragMode::ResizeStart | DragMode::ResizeEnd => {
                    mouse::Interaction::ResizingVertically
                }
            };
        }

        match cursor
            .position_in(bounds)
            .and_then(|point| self.grab(point, bounds.width))
        {
            Some((index, mode)) if self.draggable(index) => match mode {
                DragMode::Move => mouse::Interaction::Grab,
                DragMode::ResizeStart | DragMode::ResizeEnd => {
                    mouse::Interaction::ResizingVertically
                }
            },
            Some(_) => mouse::Interaction::Pointer,
            None => mouse::Interaction::default(),
        }
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        // The dragged block is left out of the cached grid and drawn on top
        // instead, so the preview can follow the pointer without retracing the
        // day. The cache key carries which block that is, so the grid rebuilds
        // once when the drag starts and once when it ends.
        let drag = state.drag();
        let dragging = drag
            .as_ref()
            .and_then(|drag| self.blocks.iter().position(|block| block.id == drag.id));
        let key = cache_key(
            &self.blocks,
            self.hour_height,
            self.gutter,
            self.compact,
            self.day_start_hour,
            dragging,
        );
        if state.key.get() != key {
            state.grid.clear();
            state.key.set(key);
        }

        let grid = state.grid.draw(renderer, bounds.size(), |frame| {
            self.draw_grid(frame, dragging);
        });

        let mut overlay = Frame::new(renderer, bounds.size());
        self.draw_needle(&mut overlay);
        if let Some(drag) = drag {
            self.draw_drag(&mut overlay, &drag);
        } else if let Some(index) = self.hit(cursor.position_in(bounds), bounds.width) {
            self.draw_hover(&mut overlay, index);
        }

        vec![grid, overlay.into_geometry()]
    }
}

impl DayCanvas {
    fn draw_grid(&self, frame: &mut Frame, dragging: Option<usize>) {
        let grid_left = self.gutter;
        let label_size = if self.compact { 10.0 } else { 11.0 };

        for hour in 0..24 {
            let y = hour as f32 * self.hour_height;
            frame.stroke(
                &Path::line(Point::new(grid_left, y), Point::new(frame.width(), y)),
                Stroke {
                    style: Style::Solid(colors::GRID_LINE),
                    width: 1.0,
                    ..Stroke::default()
                },
            );

            fill_glyphs(frame, Text {
                content: calendar::hour_label((self.day_start_hour + hour) % 24),
                position: Point::new(self.gutter - 6.0, y + 2.0),
                color: colors::GUTTER_TEXT,
                size: Pixels(label_size),
                font: Font::DEFAULT,
                align_x: iced::alignment::Horizontal::Right.into(),
                align_y: iced::alignment::Vertical::Top,
                line_height: text::LineHeight::default(),
                shaping: text::Shaping::Basic,
                max_width: self.gutter - 8.0,
            });
        }

        for (index, block) in self.blocks.iter().enumerate() {
            // The dragged block is drawn on top of the cache instead.
            if Some(index) == dragging {
                continue;
            }

            let block_bounds = self.block_rect(block, frame.width());
            let shape = Path::rounded_rectangle(
                Point::new(block_bounds.x, block_bounds.y),
                Size::new(block_bounds.width, block_bounds.height),
                EVENT_RADIUS.into(),
            );

            // An unconfirmed write is drawn lighter, so the chip reads as not
            // yet final until Google accepts the new time.
            let fill = if block.pending {
                lighten(block.color)
            } else {
                block.color
            };
            frame.fill(&shape, fill);
            frame.stroke(
                &shape,
                Stroke {
                    style: Style::Solid(Color::BLACK.scale_alpha(0.55)),
                    width: 1.0,
                    ..Stroke::default()
                },
            );

            let ink = on_color(fill);
            let title_size = if self.compact { 11.0 } else { 12.0 };
            let text_width = (block_bounds.width - 10.0).max(8.0);
            let title_top = block_bounds.y + 3.0;
            fill_glyphs(frame, Text {
                content: block.title.clone(),
                position: Point::new(block_bounds.x + 6.0, title_top),
                color: ink,
                size: Pixels(title_size),
                font: Font::DEFAULT,
                align_x: iced::alignment::Horizontal::Left.into(),
                align_y: iced::alignment::Vertical::Top,
                line_height: text::LineHeight::default(),
                shaping: text::Shaping::Basic,
                max_width: text_width,
            });

            // The time sits under the title, so it has to clear however many
            // lines the title wrapped to rather than a fixed one-line offset.
            let when_size = title_size - 1.0;
            let when_top = title_top + wrapped_height(&block.title, title_size, text_width) + 2.0;
            if block_bounds.height >= 28.0
                && when_top + when_size <= block_bounds.y + block_bounds.height - 2.0
            {
                fill_glyphs(frame, Text {
                    content: block.when.clone(),
                    position: Point::new(block_bounds.x + 6.0, when_top),
                    color: ink.scale_alpha(0.85),
                    size: Pixels(when_size),
                    font: Font::DEFAULT,
                    align_x: iced::alignment::Horizontal::Left.into(),
                    align_y: iced::alignment::Vertical::Top,
                    line_height: text::LineHeight::default(),
                    shaping: text::Shaping::Basic,
                    max_width: text_width,
                });
            }
        }
    }

    fn draw_needle(&self, frame: &mut Frame) {
        let Some(minutes) = self.needle else {
            return;
        };

        let y = (minutes as f32 * (self.hour_height / 60.0)).clamp(0.0, frame.height());
        frame.stroke(
            &Path::line(Point::new(self.gutter, y), Point::new(frame.width(), y)),
            Stroke {
                style: Style::Solid(colors::NEEDLE),
                width: 2.0,
                ..Stroke::default()
            },
        );
        frame.fill(&Path::circle(Point::new(self.gutter, y), NEEDLE_RADIUS), colors::NEEDLE);
    }

    fn draw_hover(&self, frame: &mut Frame, index: usize) {
        let Some(block) = self.blocks.get(index) else {
            return;
        };
        let bounds = self.block_rect(block, frame.width());
        let shape = Path::rounded_rectangle(
            Point::new(bounds.x, bounds.y),
            Size::new(bounds.width, bounds.height),
            EVENT_RADIUS.into(),
        );
        frame.fill(&shape, Color::WHITE.scale_alpha(0.22));
        frame.stroke(
            &shape,
            Stroke {
                style: Style::Solid(Color::BLACK),
                width: 2.0,
                ..Stroke::default()
            },
        );
    }

    /// Draws the block being dragged at its preview position, in the lighter
    /// shade that marks a time change as not yet final.
    fn draw_drag(&self, frame: &mut Frame, drag: &Drag) {
        let Some(block) = self.blocks.iter().find(|block| block.id == drag.id) else {
            return;
        };
        let bounds = self.rect_for(block, frame.width(), drag.preview_start, drag.preview_end);
        let shape = Path::rounded_rectangle(
            Point::new(bounds.x, bounds.y),
            Size::new(bounds.width, bounds.height),
            EVENT_RADIUS.into(),
        );

        let fill = lighten(block.color);
        frame.fill(&shape, fill);
        frame.stroke(
            &shape,
            Stroke {
                style: Style::Solid(Color::BLACK.scale_alpha(0.55)),
                width: 1.0,
                ..Stroke::default()
            },
        );

        let ink = on_color(fill);
        let title_size = if self.compact { 11.0 } else { 12.0 };
        let text_width = (bounds.width - 10.0).max(8.0);
        let title_top = bounds.y + 3.0;
        fill_glyphs(frame, Text {
            content: block.title.clone(),
            position: Point::new(bounds.x + 6.0, title_top),
            color: ink,
            size: Pixels(title_size),
            font: Font::DEFAULT,
            align_x: iced::alignment::Horizontal::Left.into(),
            align_y: iced::alignment::Vertical::Top,
            line_height: text::LineHeight::default(),
            shaping: text::Shaping::Basic,
            max_width: text_width,
        });

        // The label shows the time the event will land on, not the one it had.
        let when = format!(
            "{} – {}",
            self.clock_label(drag.preview_start),
            self.clock_label(drag.preview_end)
        );
        let when_size = title_size - 1.0;
        let when_top = title_top + wrapped_height(&block.title, title_size, text_width) + 2.0;
        if bounds.height >= 28.0 && when_top + when_size <= bounds.y + bounds.height - 2.0 {
            fill_glyphs(frame, Text {
                content: when,
                position: Point::new(bounds.x + 6.0, when_top),
                color: ink.scale_alpha(0.85),
                size: Pixels(when_size),
                font: Font::DEFAULT,
                align_x: iced::alignment::Horizontal::Left.into(),
                align_y: iced::alignment::Vertical::Top,
                line_height: text::LineHeight::default(),
                shaping: text::Shaping::Basic,
                max_width: text_width,
            });
        }
    }

    /// Which block is under the pointer, and which part of it was grabbed.
    ///
    /// The top and bottom edges resize; the middle moves. The edge zone is
    /// capped at a third of the block so a short event still has a middle.
    fn grab(&self, point: Point, width: f32) -> Option<(usize, DragMode)> {
        let index = self.hit(Some(point), width)?;
        let block = self.blocks.get(index)?;
        let bounds = self.block_rect(block, width);
        let edge = EDGE_GRAB.min(bounds.height / 3.0);

        let mode = if point.y - bounds.y <= edge {
            DragMode::ResizeStart
        } else if bounds.y + bounds.height - point.y <= edge {
            DragMode::ResizeEnd
        } else {
            DragMode::Move
        };

        Some((index, mode))
    }

    /// Whether the block at this index can be dragged at all.
    fn draggable(&self, index: usize) -> bool {
        self.blocks
            .get(index)
            .is_some_and(|block| block.editable && !block.pending)
    }

    /// The drag as it stands with the pointer at `cursor_y`.
    ///
    /// The travel is snapped to [`SNAP_MINUTES`] and added to the times the
    /// drag started from, so the event keeps its offset and a press that does
    /// not move leaves the times untouched.
    fn preview(&self, drag: &Drag, cursor_y: f32) -> Drag {
        let minute_height = self.hour_height / 60.0;
        let travel = (cursor_y - drag.origin_y) / minute_height;
        let delta = (travel / SNAP_MINUTES as f32).round() as i32 * SNAP_MINUTES;
        let duration = drag.end_min - drag.start_min;

        let (start, end) = match drag.mode {
            DragMode::Move => {
                let start = (drag.start_min + delta).clamp(0, DAY_MINUTES - duration);
                (start, start + duration)
            }
            DragMode::ResizeStart => {
                let start = (drag.start_min + delta).clamp(0, drag.end_min - MIN_DURATION);
                (start, drag.end_min)
            }
            DragMode::ResizeEnd => {
                let end = (drag.end_min + delta).clamp(drag.start_min + MIN_DURATION, DAY_MINUTES);
                (drag.start_min, end)
            }
        };

        Drag {
            id: drag.id.clone(),
            mode: drag.mode,
            origin_y: drag.origin_y,
            start_min: drag.start_min,
            end_min: drag.end_min,
            preview_start: start,
            preview_end: end,
        }
    }

    /// The real time a previewed minute-of-day lands on.
    fn time_at(&self, minutes: i32) -> DateTime<Local> {
        let offset = self.day_start_hour as i32 * 60;
        let naive = self
            .day
            .and_hms_opt(0, 0, 0)
            .unwrap_or_default()
            + chrono::Duration::minutes(i64::from(offset + minutes));

        match Local.from_local_datetime(&naive) {
            chrono::offset::LocalResult::Single(when)
            | chrono::offset::LocalResult::Ambiguous(when, _) => when,
            chrono::offset::LocalResult::None => naive.and_utc().with_timezone(&Local),
        }
    }

    /// A previewed minute-of-day as the grid spells a time, matching the
    /// 24-hour labels the chips already use.
    fn clock_label(&self, minutes: i32) -> String {
        let total = (self.day_start_hour as i32 * 60 + minutes).rem_euclid(DAY_MINUTES);
        format!("{:02}:{:02}", total / 60, total % 60)
    }

    fn track_hover(
        &self,
        state: &mut DrawState,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        let hit = self.hit(cursor.position_in(bounds), bounds.width);
        if state.hovered.get() == hit {
            return None;
        }
        state.hovered.set(hit);
        Some(canvas::Action::request_redraw())
    }

    fn hit(&self, local: Option<Point>, width: f32) -> Option<usize> {
        let point = local?;
        self.blocks.iter().enumerate().rev().find_map(|(index, block)| {
            self.block_rect(block, width).contains(point).then_some(index)
        })
    }

    fn block_rect(&self, block: &Block, width: f32) -> Rectangle {
        self.rect_for(block, width, block.start_min, block.end_min)
    }

    /// The rectangle a block occupies for a given pair of times, so the drag
    /// preview can be drawn at times the block does not hold yet.
    fn rect_for(&self, block: &Block, width: f32, start_min: i32, end_min: i32) -> Rectangle {
        let minute_height = self.hour_height / 60.0;
        let grid_width = (width - self.gutter).max(40.0);
        let columns = block.columns.max(1) as f32;
        let column = block.column as f32;
        let block_width = (grid_width / columns) - EVENT_GAP;
        Rectangle {
            x: self.gutter + column * (grid_width / columns) + EVENT_GAP / 2.0,
            y: start_min as f32 * minute_height,
            width: block_width.max(8.0),
            height: ((end_min - start_min) as f32 * minute_height).max(16.0),
        }
    }
}

/// Blends a color toward white, for the lighter shade an unconfirmed time
/// change is drawn in.
fn lighten(color: Color) -> Color {
    const AMOUNT: f32 = 0.3;
    Color {
        r: color.r + (1.0 - color.r) * AMOUNT,
        g: color.g + (1.0 - color.g) * AMOUNT,
        b: color.b + (1.0 - color.b) * AMOUNT,
        a: color.a,
    }
}

/// Fills text as glyph outlines instead of leaving it for the canvas to draw.
///
/// Canvas text is replayed uncullably: the tiny-skia backend cannot clip it to
/// the visible slice of the day, and it resets its whole window-sized clip mask
/// once per label on every frame, which is what made a busy day stutter while
/// scrolling. Outlines are ordinary paths — they are culled to the visible
/// slice like the grid lines — and, kept in the geometry cache, they are traced
/// once per change to the events or the zoom rather than once per frame.
fn fill_glyphs(frame: &mut Frame, text: impl Into<Text>) {
    let text = text.into();
    text.draw_with(|glyph, color| frame.fill(&glyph, color));
}

/// The height a title occupies once wrapped to `max_width`.
///
/// The time label is drawn below the title, so a title that wraps to two lines
/// would otherwise have the time run straight through it. Measuring the
/// paragraph keeps the label clear of however many lines the title takes.
fn wrapped_height(content: &str, size: f32, max_width: f32) -> f32 {
    use iced::advanced::graphics::text::Paragraph;
    use iced::advanced::text::Paragraph as _;

    Paragraph::with_text(iced::advanced::text::Text {
        content,
        bounds: Size::new(max_width, f32::INFINITY),
        size: Pixels(size),
        line_height: text::LineHeight::default(),
        font: Font::DEFAULT,
        align_x: iced::advanced::text::Alignment::Left,
        align_y: iced::alignment::Vertical::Top,
        shaping: text::Shaping::Basic,
        wrapping: text::Wrapping::default(),
    })
    .min_height()
}

/// Everything the cached geometry is drawn from, so a rebuilt [`Block`] with a
/// different label or shade retraces instead of going stale.
fn cache_key(
    blocks: &[Block],
    hour_height: f32,
    gutter: f32,
    compact: bool,
    day_start_hour: u32,
    dragging: Option<usize>,
) -> u64 {
    let mut hash = hour_height.to_bits() as u64;
    hash = hash.wrapping_mul(31).wrapping_add(gutter.to_bits() as u64);
    hash = hash.wrapping_mul(31).wrapping_add(u64::from(compact));
    hash = hash.wrapping_mul(31).wrapping_add(u64::from(day_start_hour));
    // Which block is being dragged decides which one the cached grid leaves
    // out, so the grid has to retrace when a drag starts or ends.
    hash = hash
        .wrapping_mul(31)
        .wrapping_add(dragging.map_or(u64::MAX, |index| index as u64));

    for block in blocks {
        hash = hash.wrapping_mul(31).wrapping_add(block.start_min as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.end_min as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.column as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.columns as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.color.r.to_bits() as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.color.g.to_bits() as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.color.b.to_bits() as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.color.a.to_bits() as u64);
        // An unconfirmed write changes the shade, so it has to retrace too.
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(block.pending));
        for byte in block.title.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
        for byte in block.when.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
    }

    hash
}

/// Wheel notches to apply. Scroll up is positive.
///
/// Pixel deltas from a trackpad are large, so they are scaled down to the
/// same step as a mouse notch. Whichever axis moved more wins, because
/// holding Shift turns the wheel into a horizontal scroll on some desktops.
fn zoom_steps(delta: mouse::ScrollDelta) -> f32 {
    let (x, y, scale) = match delta {
        mouse::ScrollDelta::Lines { x, y } => (x, y, 1.0),
        mouse::ScrollDelta::Pixels { x, y } => (x, y, 1.0 / 60.0),
    };
    let dominant = if x.abs() > y.abs() { x } else { y };
    dominant * scale
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    #[test]
    fn hour_labels_match_google_calendar() {
        assert_eq!(calendar::hour_label(0), "12 AM");
        assert_eq!(calendar::hour_label(9), "9 AM");
        assert_eq!(calendar::hour_label(12), "12 PM");
        assert_eq!(calendar::hour_label(13), "1 PM");
        assert_eq!(calendar::hour_label(23), "11 PM");
    }

    #[test]
    fn a_late_boundary_moves_the_small_hours_to_the_foot_of_the_day() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let at = |hour, minute| {
            day.and_hms_opt(hour, minute, 0)
                .unwrap()
                .and_local_timezone(Local)
                .single()
                .unwrap()
        };

        // A 3am boundary: the day opens at 3am, so 9am is 6 hours in.
        assert_eq!(minutes_into_day(at(3, 0), day, 3 * 60), 0);
        assert_eq!(minutes_into_day(at(9, 0), day, 3 * 60), 6 * 60);

        // 1am on the day itself is before the window, so it belongs to the
        // previous day's view and comes back negative to be clamped away.
        assert_eq!(minutes_into_day(at(1, 0), day, 3 * 60), -120);

        // The next morning's small hours belong to this view, at the foot.
        let next = day.succ_opt().unwrap();
        let next_1am = next.and_hms_opt(1, 0, 0).unwrap().and_local_timezone(Local).single().unwrap();
        assert_eq!(minutes_into_day(next_1am, day, 3 * 60), 22 * 60);

        // Midnight keeps the plain mapping.
        assert_eq!(minutes_into_day(at(1, 0), day, 0), 60);
    }

    #[test]
    fn scroll_up_zooms_in_and_scroll_down_zooms_out() {
        let up = zoom_steps(mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 });
        let down = zoom_steps(mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 });

        assert!(up > 0.0);
        assert!(down < 0.0);
        assert!((up + down).abs() < f32::EPSILON);
    }

    #[test]
    fn zooming_keeps_the_row_under_the_pointer_still() {
        // A row 300px down the grid, zoomed in by 1.5x, slides to 450px. The
        // scroll has to move by the 150px difference so it stays under the
        // mouse.
        assert!((zoom_scroll_delta(300.0, 48.0, 72.0) - 150.0).abs() < 0.001);

        // Zooming out pulls the row back up, so the scroll moves the other way.
        assert!((zoom_scroll_delta(300.0, 72.0, 48.0) + 100.0).abs() < 0.001);

        // The row at the very top never moves, whatever the zoom.
        assert_eq!(zoom_scroll_delta(0.0, 48.0, 180.0), 0.0);

        // A no-op zoom, or a degenerate height, asks for no scroll at all.
        assert_eq!(zoom_scroll_delta(300.0, 48.0, 48.0), 0.0);
        assert_eq!(zoom_scroll_delta(300.0, 0.0, 48.0), 0.0);
    }

    /// A canvas with one 9:00–9:30 block, for the drag tests.
    fn canvas_with_one_block() -> DayCanvas {
        DayCanvas {
            blocks: vec![Block {
                id: "standup".into(),
                title: "Standup".into(),
                when: "09:00 – 09:30".into(),
                start_min: 9 * 60,
                end_min: 9 * 60 + 30,
                column: 0,
                columns: 1,
                color: Color::WHITE,
                editable: true,
                pending: false,
            }],
            needle: None,
            hour_height: 60.0,
            gutter: GUTTER,
            compact: false,
            day_start_hour: 0,
            day: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        }
    }

    #[test]
    fn the_cache_key_holds_until_a_traced_detail_changes() {
        let block = |title: &str, when: &str, alpha: f32| Block {
            id: title.to_owned(),
            title: title.to_owned(),
            when: when.to_owned(),
            start_min: 9 * 60,
            end_min: 9 * 60 + 30,
            column: 0,
            columns: 1,
            color: Color {
                a: alpha,
                ..Color::WHITE
            },
            editable: true,
            pending: false,
        };

        let key = cache_key(
            &[block("Standup", "09:00 – 09:30", 1.0)],
            48.0,
            GUTTER,
            false,
            0,
            None,
        );

        // A key that drifts between frames would retrace the whole day while
        // scrolling, and one that ignores a label would draw it stale.
        assert_eq!(
            key,
            cache_key(
                &[block("Standup", "09:00 – 09:30", 1.0)],
                48.0,
                GUTTER,
                false,
                0,
                None,
            )
        );

        for changed in [
            block("Lunch", "09:00 – 09:30", 1.0),
            block("Standup", "09:00 – 10:00", 1.0),
            block("Standup", "09:00 – 09:30", 0.5),
        ] {
            assert_ne!(
                key,
                cache_key(std::slice::from_ref(&changed), 48.0, GUTTER, false, 0, None),
                "a changed block has to be retraced"
            );
        }

        // Moving the day's boundary relabels every hour, so the grid has to be
        // retraced even though the events did not move.
        assert_ne!(
            key,
            cache_key(
                &[block("Standup", "09:00 – 09:30", 1.0)],
                48.0,
                GUTTER,
                false,
                3,
                None,
            ),
            "a new day boundary has to relabel the grid"
        );

        // An unconfirmed write lightens the chip, and a drag leaves the dragged
        // block out of the cached grid. Both have to retrace.
        let pending = Block {
            pending: true,
            ..block("Standup", "09:00 – 09:30", 1.0)
        };
        assert_ne!(
            key,
            cache_key(std::slice::from_ref(&pending), 48.0, GUTTER, false, 0, None),
            "a pending write has to retrace the lighter chip"
        );
        assert_ne!(
            key,
            cache_key(
                &[block("Standup", "09:00 – 09:30", 1.0)],
                48.0,
                GUTTER,
                false,
                0,
                Some(0),
            ),
            "a drag has to leave the dragged block out of the cached grid"
        );
    }

    #[test]
    fn a_wrapped_title_measures_taller_than_a_single_line() {
        // The time label is placed below the title using this measurement, so a
        // title that wraps to two lines has to report more than one line's
        // height or the time would be drawn through the second line.
        let one_line = wrapped_height("Standup", 12.0, 200.0);
        let two_lines = wrapped_height("Programming Team Meeting", 12.0, 90.0);

        assert!(one_line > 0.0);
        assert!(
            two_lines > one_line,
            "a wrapped title ({two_lines}) should be taller than a single line ({one_line})"
        );
    }

    #[test]
    fn a_click_in_the_top_half_opens_the_details_below_it() {
        record_pointer(40.0, 400.0);
        assert!(click_was_in_the_top_half());
    }

    #[test]
    fn a_click_in_the_bottom_half_opens_the_details_above_it() {
        record_pointer(360.0, 400.0);
        assert!(!click_was_in_the_top_half());
    }

    #[test]
    fn without_a_pointer_sample_the_details_stay_clear_of_the_header() {
        record_pointer(0.0, 0.0);
        assert!(click_was_in_the_top_half());
    }

    #[test]
    fn the_pointer_hits_the_event_under_it() {
        let canvas = canvas_with_one_block();

        let rect = canvas.block_rect(&canvas.blocks[0], 240.0);
        assert_eq!(
            canvas.hit(Some(Point::new(rect.x + 4.0, rect.y + 4.0)), 240.0),
            Some(0)
        );
        assert_eq!(canvas.hit(Some(Point::new(8.0, rect.y + 4.0)), 240.0), None);
    }

    #[test]
    fn the_edges_resize_and_the_middle_moves() {
        let canvas = canvas_with_one_block();
        let rect = canvas.block_rect(&canvas.blocks[0], 240.0);
        let at = |y: f32| canvas.grab(Point::new(rect.x + 4.0, y), 240.0);

        assert_eq!(at(rect.y + 1.0), Some((0, DragMode::ResizeStart)));
        assert_eq!(at(rect.y + rect.height - 1.0), Some((0, DragMode::ResizeEnd)));
        assert_eq!(at(rect.y + rect.height / 2.0), Some((0, DragMode::Move)));
        assert_eq!(canvas.grab(Point::new(8.0, rect.y + 1.0), 240.0), None);
    }

    #[test]
    fn a_short_block_still_has_a_middle_to_grab() {
        let mut canvas = canvas_with_one_block();
        // A 15-minute block at the shortest hour row is 16px tall, so a fixed
        // 8px edge zone would leave no middle at all.
        canvas.hour_height = 24.0;
        canvas.blocks[0].end_min = canvas.blocks[0].start_min + 15;
        let rect = canvas.block_rect(&canvas.blocks[0], 240.0);
        assert_eq!(rect.height, 16.0);

        assert_eq!(
            canvas.grab(Point::new(rect.x + 4.0, rect.y + rect.height / 2.0), 240.0),
            Some((0, DragMode::Move))
        );
    }

    #[test]
    fn moving_keeps_the_length_and_snaps_to_the_quarter_hour() {
        let canvas = canvas_with_one_block();
        let drag = Drag {
            id: "standup".into(),
            mode: DragMode::Move,
            origin_y: 0.0,
            start_min: 9 * 60,
            end_min: 9 * 60 + 30,
            preview_start: 9 * 60,
            preview_end: 9 * 60 + 30,
        };

        // One hour down the grid is 60px at 60px/hour, so the event lands an
        // hour later with the same 30-minute length.
        let moved = canvas.preview(&drag, 60.0);
        assert_eq!(moved.preview_start, 10 * 60);
        assert_eq!(moved.preview_end, 10 * 60 + 30);

        // A press that does not move leaves the times alone.
        assert_eq!(canvas.preview(&drag, 0.0).preview_start, 9 * 60);

        // A few pixels of travel is not a whole snap step.
        assert_eq!(canvas.preview(&drag, 5.0).preview_start, 9 * 60);
    }

    #[test]
    fn resizing_the_top_keeps_the_end_and_the_bottom_keeps_the_start() {
        let canvas = canvas_with_one_block();
        let drag = |mode| Drag {
            id: "standup".into(),
            mode,
            origin_y: 0.0,
            start_min: 9 * 60,
            end_min: 9 * 60 + 30,
            preview_start: 9 * 60,
            preview_end: 9 * 60 + 30,
        };

        // Pulling the top up by 30 minutes moves the start and leaves the end.
        let top = canvas.preview(&drag(DragMode::ResizeStart), -30.0);
        assert_eq!(top.preview_start, 8 * 60 + 30);
        assert_eq!(top.preview_end, 9 * 60 + 30);

        // Pulling the bottom down by 30 minutes moves the end and leaves the
        // start.
        let bottom = canvas.preview(&drag(DragMode::ResizeEnd), 30.0);
        assert_eq!(bottom.preview_start, 9 * 60);
        assert_eq!(bottom.preview_end, 10 * 60);
    }

    #[test]
    fn a_resize_cannot_invert_the_event() {
        let canvas = canvas_with_one_block();
        let drag = Drag {
            id: "standup".into(),
            mode: DragMode::ResizeStart,
            origin_y: 0.0,
            start_min: 9 * 60,
            end_min: 9 * 60 + 30,
            preview_start: 9 * 60,
            preview_end: 9 * 60 + 30,
        };

        // Dragging the top far past the bottom stops a snap step short of it.
        let squashed = canvas.preview(&drag, 600.0);
        assert_eq!(squashed.preview_end, 9 * 60 + 30);
        assert_eq!(squashed.preview_start, 9 * 60 + 30 - MIN_DURATION);
    }

    #[test]
    fn a_previewed_minute_becomes_a_real_time() {
        let canvas = canvas_with_one_block();

        // Midnight boundary: minute 540 is 9:00 on the day in view.
        let nine = canvas.time_at(9 * 60);
        assert_eq!(nine.date_naive(), canvas.day);
        assert_eq!(nine.hour(), 9);
        assert_eq!(nine.minute(), 0);

        // A 3am boundary shifts the window, so minute 0 is 3am.
        let mut late = canvas_with_one_block();
        late.day_start_hour = 3;
        let three = late.time_at(0);
        assert_eq!(three.hour(), 3);
        assert_eq!(late.clock_label(0), "03:00");
    }

    #[test]
    fn a_pending_chip_is_lighter_than_a_final_one() {
        let color = Color::from_rgb8(0xDC, 0x21, 0x27);
        let lighter = lighten(color);

        assert!(lighter.r > color.r && lighter.g > color.g && lighter.b > color.b);
        assert_eq!(lighter.a, color.a);
        // White cannot get any lighter.
        assert_eq!(lighten(Color::WHITE), Color::WHITE);
    }
}
