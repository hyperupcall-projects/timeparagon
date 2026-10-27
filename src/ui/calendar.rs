//! Google Calendar day view, drawn to resemble the web UI.

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};
use iced::font::Weight;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Style, Text};
use iced::widget::{
    Space, button, canvas as canvas_widget, column, container, mouse_area, responsive, row,
    scrollable, stack, text, text_input, toggler,
};
use iced::{
    Alignment, Color, Element, Fill, Font, Length, Pixels, Point, Rectangle, Renderer, Size, Theme,
    mouse,
};

use crate::alarms;
use crate::app::{Message, State};
use crate::calendar::colors::{self, on_color};
use crate::calendar::layout::{self, Span};
use crate::calendar::{self, Event};

use super::layout::Metrics;

const GUTTER: f32 = 52.0;
const COMPACT_GUTTER: f32 = 40.0;
const EVENT_GAP: f32 = 2.0;
const EVENT_RADIUS: f32 = 4.0;
const NEEDLE_RADIUS: f32 = 5.5;

/// The gap between a block's left edge and the text inside it.
const BLOCK_PADDING: f32 = 6.0;
/// The gap between a block's right edge and the text inside it.
const BLOCK_PADDING_RIGHT: f32 = 4.0;
/// The gap above a block's first line of text.
const BLOCK_TOP: f32 = 3.0;
/// The gap between a block's title and the time label under it.
const BLOCK_GAP: f32 = 2.0;
/// The mark that says a title was cut short to fit its block.
const ELLIPSIS: &str = "…";
/// The width a paragraph is measured at, wide enough that nothing wraps.
const MEASURE_WIDTH: f32 = 100_000.0;

/// How close to a block's top or bottom edge a press has to land to resize it
/// rather than move it. The middle of the block is always the move handle.
const EDGE_GRAB: f32 = 8.0;
/// Dragged times snap to this many minutes, the way Google Calendar's own day
/// view does. The event keeps its offset, so a 9:07 meeting moves in 15-minute
/// steps rather than jumping to the quarter hour.
const SNAP_MINUTES: i32 = 15;
/// The shortest an event can be dragged to.
const MIN_DURATION: i32 = SNAP_MINUTES;
/// The length a new event gets from a plain click, matching Google Calendar's
/// one-hour default.
const DEFAULT_DURATION: i32 = 60;

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let grid = day_grid(state, metrics, Local::now());

    // The pointer watch records where the click landed without publishing a
    // message, so scrolling stays on the widget's own path.
    let mut body = column![].spacing(metrics.spacing).height(Fill).width(Fill);
    if let Some(all_day) = all_day_row(state, metrics) {
        body = body.push(all_day);
    }
    let body = pointer_watch(body.push(grid));

    let unplanned = unplanned_task_count(state);
    let mut unplanned_line = text(unplanned_tasks_label(unplanned))
        .size(metrics.label() + 9.0)
        .width(Fill);
    if unplanned == 0 {
        unplanned_line = unplanned_line.style(text::secondary);
    } else {
        unplanned_line = unplanned_line.font(Font {
            weight: Weight::Bold,
            ..Font::DEFAULT
        });
    }
    let mut status = column![
        text(state.calendar.status_line())
            .size(metrics.label() - 1.0)
            .style(if state.calendar.error.is_some() {
                text::danger
            } else {
                text::secondary
            }),
        unplanned_line,
        pending_button(state, state.calendar.day, metrics.label() + 3.0),
    ]
    .spacing(2);

    // Alarm setting mode says what a click will do, since nothing else on the
    // screen does. It sits inside the status block so the grid below keeps its
    // place in the widget tree, and with it the scroll position.
    if state.alarms.mode {
        status = status.push(
            text("Click a time to set an alarm. Esc cancels.")
                .size(metrics.label() - 1.0)
                .color(colors::ALARM),
        );
    }

    let mut content = column![header(state, metrics), status, body]
        .spacing(metrics.spacing)
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill);

    // The bar comes last, so it appearing and going leaves the grid alone.
    if let Some(dismiss) = dismiss_bar(state, metrics) {
        content = content.push(dismiss);
    }

    // The root stays a stack whether or not a card is open. Swapping the root
    // widget throws away the day grid's scroll state, which jumps the bar.
    // Event details are no longer an overlay: they are a pane beside the view.
    let overlay = if state.calendar.draft.is_some() {
        draft_overlay(state, metrics)
    } else if state.calendar.edit_draft.is_some() {
        edit_overlay(state, metrics)
    } else if state.alarms.editor.is_some() {
        super::alarms::editor_overlay(state, metrics)
    } else if let Some(day) = state.calendar.pending_open {
        pending_overlay(state, metrics, day)
    } else if state.calendar.month_open {
        month_overlay(state, metrics)
    } else if state.calendar.menu_open {
        menu_overlay(state, metrics)
    } else {
        iced::widget::Space::new().width(0.0).height(0.0).into()
    };

    stack![content, overlay].into()
}

pub(crate) fn weekly_view_width(settings: &crate::config::CalendarSettings) -> f32 {
    let columns = (settings.weekly_days() + 1) as f32 * settings.weekly_day_width() as f32;
    let padding = Metrics::of(Size::new(columns, 0.0)).padding;
    columns + padding * 2.0
}

pub fn weekly_view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let days = calendar::week_dates(state.week_anchor, state.config.calendar.weekly_days());
    let width = state.config.calendar.weekly_day_width() as f32;
    let day_metrics = Metrics::of(Size::new(width, metrics.size.height));
    let now = Local::now();
    let all_day_height = if days.iter().any(|&day| {
        state
            .calendar
            .all_day_events_on(day, &state.config.calendar)
            .next()
            .is_some()
    }) {
        (metrics.size.height * 0.13).clamp(35.0, 110.0)
    } else {
        0.0
    };
    let headings: Vec<Element<'a, Message>> = days
        .iter()
        .map(|&day| {
            let all_day = all_day_row_on(state, &day_metrics, day, true)
                .unwrap_or_else(|| Space::new().width(Fill).into());
            let heading = text(day.format("%a %b %-d").to_string())
                .size(day_metrics.label())
                .center()
                .width(Fill);
            let content = column![
                heading,
                container(pending_button(state, day, day_metrics.label()))
                    .center_x(Fill)
                    .height(Length::Fixed(day_metrics.label() + 8.0)),
                container(all_day)
                    .height(Length::Fixed(all_day_height))
                    .width(Fill),
            ]
            .width(Length::Fixed(width));
            Element::from(content)
                .map(move |message| Message::WeekDayAction(day, Box::new(message)))
        })
        .collect();

    let grids: Vec<Element<'a, Message>> = days
        .iter()
        .map(|&day| {
            Element::from(
                container(day_canvas(state, &day_metrics, now, day, true))
                    .width(Length::Fixed(width)),
            )
            .map(move |message| Message::WeekDayAction(day, Box::new(message)))
        })
        .collect();
    let time_grid = scrollable(row(grids))
        .id(iced::widget::Id::new(DAY_SCROLL))
        .width(Fill)
        .height(Fill);
    let grid = scrollable(
        column![row(headings), time_grid]
            .width(Length::Fixed(width * days.len() as f32))
            .height(Fill),
    )
    .direction(scrollable::Direction::Horizontal(
        scrollable::Scrollbar::new(),
    ))
    .id(iced::widget::Id::new(WEEK_SCROLL))
    .on_scroll(|viewport| Message::WeekScrolled(viewport.absolute_offset().x))
    .width(Fill)
    .height(Fill);

    let panel_open = state.calendar.open_event.is_some() || state.timeblocking.block.is_some();
    let mut grid_layers: Vec<Element<'a, Message>> = vec![grid.into()];
    if panel_open && let Some(index) = days.iter().position(|day| *day == state.calendar.day) {
        let viewport = (metrics.size.width - metrics.padding * 2.0).max(1.0);
        let scroll_x = state
            .week_scroll_x
            .clamp(0.0, (width * days.len() as f32 - viewport).max(0.0));
        let x = index as f32 * width - scroll_x;
        let left = x.clamp(0.0, viewport);
        let right = (x + width).clamp(0.0, viewport);
        let dismiss = if state.calendar.open_event.is_some() {
            Message::EventClosed
        } else {
            Message::TimeBlocking(crate::timeblocking::Action::Close)
        };
        let shade = |w: f32| {
            mouse_area(
                container(Space::new().width(Fill).height(Fill))
                    .width(Length::Fixed(w))
                    .height(Fill)
                    .style(|_| container::Style {
                        background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.35).into()),
                        ..container::Style::default()
                    }),
            )
            .on_press(dismiss.clone())
        };
        grid_layers.push(
            row![
                shade(left),
                Space::new().width(Length::Fixed((right - left).max(0.0))),
                shade((viewport - right).max(0.0)),
            ]
            .height(Fill)
            .into(),
        );

        let requested_width = if state.calendar.open_event.is_some() {
            state.config.views.detail_width()
        } else {
            state.config.views.time_block_width()
        };
        let room_right = viewport - right;
        let place_right = index + 1 < days.len() && room_right >= width.min(80.0);
        let panel_width = if place_right {
            requested_width.min(room_right)
        } else if left > 0.0 {
            requested_width.min(left)
        } else {
            requested_width.min(viewport)
        };
        let panel_x = if place_right {
            right
        } else {
            (left - panel_width).max(0.0)
        };
        let pane = responsive(move |size| {
            if state.calendar.open_event.is_some() {
                let height = size.height.max(480.0);
                let panel_metrics = Metrics::of(Size::new(panel_width, height));
                scrollable(
                    container(details_pane(state, &panel_metrics)).height(Length::Fixed(height)),
                )
                .height(Fill)
                .into()
            } else {
                let panel_metrics = Metrics::of(Size::new(panel_width, size.height));
                super::timeblocking::panel(state, &panel_metrics)
            }
        });
        grid_layers.push(
            row![
                Space::new().width(Length::Fixed(panel_x)),
                container(pane)
                    .width(Length::Fixed(panel_width))
                    .height(Fill)
                    .style(|theme: &Theme| container::Style {
                        background: Some(theme.extended_palette().background.base.color.into()),
                        border: iced::Border {
                            color: theme.extended_palette().background.strong.color,
                            width: 1.0,
                            ..iced::Border::default()
                        },
                        ..container::Style::default()
                    }),
            ]
            .height(Fill)
            .into(),
        );
    }

    let mut status = column![
        text(state.calendar.status_line())
            .size(metrics.label() - 1.0)
            .style(if state.calendar.error.is_some() {
                text::danger
            } else {
                text::secondary
            }),
    ]
    .spacing(2);
    if state.alarms.mode {
        status = status.push(
            text("Click a time to set an alarm. Esc cancels.")
                .size(metrics.label() - 1.0)
                .color(colors::ALARM),
        );
    }
    let mut content = column![
        header(state, metrics),
        status,
        pointer_watch(
            iced::widget::Stack::with_children(grid_layers)
                .width(Fill)
                .height(Fill)
        ),
    ]
    .spacing(metrics.spacing)
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill);
    if let Some(dismiss) = dismiss_bar(state, metrics) {
        content = content.push(dismiss);
    }
    let overlay = if state.calendar.draft.is_some() {
        draft_overlay(state, metrics)
    } else if state.calendar.edit_draft.is_some() {
        edit_overlay(state, metrics)
    } else if state.alarms.editor.is_some() {
        super::alarms::editor_overlay(state, metrics)
    } else if let Some(day) = state.calendar.pending_open {
        pending_overlay(state, metrics, day)
    } else if state.calendar.month_open {
        month_overlay(state, metrics)
    } else if state.calendar.menu_open {
        menu_overlay(state, metrics)
    } else {
        Space::new().width(0.0).height(0.0).into()
    };
    stack![content, overlay].into()
}

/// The top of the calendar: the month picker, the day in view and the way
/// into the calendar menu.
///
/// The day sits between the month picker and the menu, and takes whatever room
/// is left over, so the whole date is readable even when the text is large.
/// The settings are reached through the menu rather than from here, because a
/// narrow window has room for one button beside the day and not for two.
fn header<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let today = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
    let weekly = state.weekly && state.screen == crate::app::Screen::Calendar;
    let day = if weekly {
        state.week_anchor
    } else {
        state.calendar.day
    };
    let heading = if weekly {
        let days = calendar::week_dates(day, state.config.calendar.weekly_days());
        let format = if metrics.breakpoint.is_compact() {
            "%-m/%-d"
        } else {
            "%b %-d"
        };
        format!(
            "{} – {}",
            days.first().unwrap().format(format),
            days.last().unwrap().format(format)
        )
    } else {
        calendar::day_heading(day, today)
    };

    let month = button(
        text(format!("{} ▾", calendar::month_short(day.month())))
            .size(metrics.label())
            .center(),
    )
    .on_press(Message::CalendarMonthToggled)
    .style(if state.calendar.month_open {
        button::primary
    } else {
        button::secondary
    });

    // Alarm setting mode turns the day view into a place to click a time, so
    // the button that enters it is lit for as long as it lasts.
    let set_alarm = button(text("Set Alarm").size(metrics.label()).center())
        .on_press(Message::Alarms(crate::alarms::Action::ModeToggled))
        .style(if state.alarms.mode {
            button::primary
        } else {
            button::secondary
        });

    let menu = button(text("Menu").size(metrics.label()).center())
        .on_press(Message::CalendarMenuToggled)
        .style(if state.calendar.menu_open {
            button::primary
        } else {
            button::secondary
        });

    // Travel shows how long the trip to each event takes, so the button is lit
    // for as long as those are on the grid.
    let toggle_travel = |wide: bool| {
        button(
            text("Toggle Travel")
                .size(metrics.label())
                .center()
                .width(if wide { Fill } else { Length::Shrink }),
        )
        .width(if wide { Fill } else { Length::Shrink })
        .on_press(Message::Travel(crate::travel::Action::Toggle))
        .style(if state.travel.show {
            button::primary
        } else {
            button::secondary
        })
    };

    let heading = text(heading)
        .size(metrics.label() + 1.0)
        .font(Font {
            weight: Weight::Bold,
            ..Font::DEFAULT
        })
        .center()
        .width(Fill);
    let day: Element<'a, Message> = if weekly {
        row![
            nav_button(
                "◀",
                Message::CalendarDayShift(-i64::from(state.config.calendar.weekly_days())),
                metrics
            ),
            heading,
            nav_button(
                "▶",
                Message::CalendarDayShift(i64::from(state.config.calendar.weekly_days())),
                metrics
            ),
        ]
        .spacing(metrics.spacing)
        .align_y(Alignment::Center)
        .width(Fill)
        .into()
    } else {
        heading.into()
    };

    match header_lines(metrics) {
        // Four buttons and a date do not fit side by side, so the date gets a
        // line to itself and is never squeezed into a few letters per row.
        // Below that there is not room for a fourth button either, so it
        // takes a line of its own.
        3 => column![
            day,
            row![month, Space::new().width(Fill), set_alarm, menu]
                .spacing(metrics.spacing)
                .align_y(Alignment::Center),
            toggle_travel(true),
        ]
        .spacing(metrics.spacing)
        .into(),
        2 => column![
            day,
            row![
                month,
                Space::new().width(Fill),
                toggle_travel(false),
                set_alarm,
                menu
            ]
            .spacing(metrics.spacing)
            .align_y(Alignment::Center),
        ]
        .spacing(metrics.spacing)
        .into(),
        _ => row![month, day, toggle_travel(false), set_alarm, menu]
            .spacing(metrics.spacing)
            .align_y(Alignment::Center)
            .into(),
    }
}

/// The narrowest a calendar can be and still hold the month picker, the date,
/// Toggle Travel, Set Alarm and Menu on one line with the date readable.
const SINGLE_LINE_HEADER: f32 = 460.0;

/// The narrowest a calendar can be and still hold the month picker and all
/// three buttons on one line under the date.
const TWO_LINE_HEADER: f32 = 340.0;

/// How many lines the header takes: one, two, or three.
fn header_lines(metrics: &Metrics) -> u8 {
    if metrics.size.width >= SINGLE_LINE_HEADER {
        1
    } else if metrics.size.width >= TWO_LINE_HEADER {
        2
    } else {
        3
    }
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

/// The vertical space the header row takes, so an overlay can start below
/// it rather than over it.
fn header_offset(metrics: &Metrics) -> f32 {
    let lines = f32::from(header_lines(metrics));

    metrics.padding + (metrics.icon_button() + metrics.spacing) * lines
}

/// Tasks on the day in view that have neither a time nor a time block.
///
/// Timed tasks (including those placed by DUE_TIME), undated backlog tasks,
/// and tasks filed on another day stay out of the count.
fn unplanned_task_count(state: &State) -> usize {
    let day = state.calendar.day;
    state
        .calendar
        .events
        .iter()
        .filter(|event| {
            event.all_day
                && event.start.date_naive() == day
                && crate::timeblocking::in_backlog(event)
        })
        .count()
}

/// Tasks Google Tasks still lists as pending (not completed) on a day.
fn pending_tasks(state: &State, day: NaiveDate) -> Vec<&Event> {
    if !state.config.calendar.show_tasks {
        return Vec::new();
    }
    state
        .calendar
        .events_on(day)
        .iter()
        .filter(|event| event.is_task() && !event.completed)
        .collect()
}

/// The pending-task count for a day; pressing it lists the tasks.
fn pending_button<'a>(state: &'a State, day: NaiveDate, size: f32) -> Element<'a, Message> {
    let count = pending_tasks(state, day).len();
    if count == 0 {
        return Space::new().into();
    }
    button(text(format!("{count} pending")).size(size))
        .padding([0, 4])
        .style(button::text)
        .on_press(Message::PendingTasksShown(Some(day)))
        .into()
}

/// A popup listing each pending task on a day.
fn pending_overlay<'a>(
    state: &'a State,
    metrics: &Metrics,
    day: NaiveDate,
) -> Element<'a, Message> {
    let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill))
        .on_press(Message::PendingTasksShown(None));

    let tasks = pending_tasks(state, day);
    let mut list = column![
        text(format!("Pending tasks · {}", day.format("%a %b %-d"))).size(metrics.label() + 1.0)
    ]
    .spacing(metrics.spacing)
    .width(Fill);
    for task in tasks {
        list = list.push(
            column![
                text(task.title.clone()).size(metrics.label()),
                text(crate::timeblocking::task_list_name(
                    state,
                    &task.calendar_id
                ))
                .size(metrics.label() - 2.0)
                .style(text::secondary),
            ]
            .width(Fill),
        );
    }
    list = list.push(
        button(text("Close").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(Message::PendingTasksShown(None))
            .style(button::secondary),
    );

    let card = container(scrollable(list).height(Length::Shrink))
        .max_height(
            (state.main_size.height - header_offset(metrics) - metrics.padding * 2.0).max(120.0),
        )
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
        Space::new().height(header_offset(metrics)),
        container(card).padding(metrics.padding).width(Fill),
    ]
    .width(Fill);

    stack![dismiss, aligned].into()
}

fn unplanned_tasks_label(count: usize) -> String {
    if count == 1 {
        "1 task not planned".to_owned()
    } else {
        format!("{count} tasks not planned")
    }
}

/// The foot of the calendar while an alarm is ringing: the way to silence it.
///
/// An alarm can go off with any view on screen, so this stays until it has
/// been dismissed. With nothing ringing there is no foot at all.
fn dismiss_bar<'a>(state: &'a State, metrics: &Metrics) -> Option<Element<'a, Message>> {
    state.alarm_active.then(|| {
        button(text("Dismiss").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .height(metrics.icon_button())
            .on_press(Message::Dismiss)
            .style(button::danger)
            .into()
    })
}

fn all_day_row<'a>(state: &'a State, metrics: &Metrics) -> Option<Element<'a, Message>> {
    all_day_row_on(state, metrics, state.calendar.day, false)
}

fn all_day_row_on<'a>(
    state: &'a State,
    metrics: &Metrics,
    day: NaiveDate,
    weekly: bool,
) -> Option<Element<'a, Message>> {
    let mut events: Vec<_> = state
        .calendar
        .all_day_events_on(day, &state.config.calendar)
        .collect();
    let sources = state.calendar.display_sources(&state.config.calendar);
    events.sort_by_key(|e| {
        (
            sources
                .iter()
                .position(|source| source.id == e.source_id())
                .unwrap_or(usize::MAX),
            !crate::timeblocking::overdue(e),
            crate::timeblocking::metadata(e).order,
        )
    });
    let chips: Vec<Element<'a, Message>> = events
        .into_iter()
        .filter(|event| {
            !event.is_task()
                || crate::timeblocking::metadata(event)
                    .block
                    .as_ref()
                    .is_none_or(|key| {
                        !state.calendar.events_on(day).iter().any(|block| {
                            !block.is_task() && crate::timeblocking::block_key(block) == *key
                        })
                    })
                || crate::timeblocking::overdue(event)
        })
        .map(|event| {
            if event.is_task() {
                if weekly {
                    weekly_task_chip(state, event, metrics)
                } else {
                    super::timeblocking::task_row(state, event, metrics, false)
                }
            } else {
                all_day_chip(event, metrics)
            }
        })
        .collect();

    if chips.is_empty() {
        return None;
    }

    Some(
        mouse_area(container(scrollable(column(chips).spacing(4.0).width(Fill))).max_height(180))
            .on_release(Message::TimeBlocking(crate::timeblocking::Action::DropDay))
            .into(),
    )
}

pub(super) fn weekly_task_chip<'a>(
    state: &'a State,
    task: &Event,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let mut check = button(text(if task.completed { "●" } else { "○" }).size(metrics.label()))
        .padding(2)
        .style(button::text);
    if state.timeblocking.busy.is_empty() {
        check = check.on_press(Message::TaskToggled(task.id.clone()));
    }
    let title = button(text(crate::timeblocking::task_label(task)).size(metrics.label()))
        .padding([3, 2])
        .width(Fill)
        .on_press(Message::EventOpened(task.id.clone()))
        .style(button::text);
    super::task_drag::task_drag(
        container(row![check, title].spacing(2).align_y(Alignment::Center))
            .width(Fill)
            .style(|theme: &Theme| container::Style {
                background: Some(theme.extended_palette().background.base.color.into()),
                border: iced::Border {
                    color: theme.extended_palette().background.strong.color,
                    width: 1.0,
                    radius: 5.0.into(),
                },
                ..container::Style::default()
            }),
        &task.id,
        state.timeblocking.dragging.is_some(),
    )
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
pub(super) fn swatch_button<'a>(
    color: Color,
    selected: bool,
    on_press: Message,
) -> Element<'a, Message> {
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

    let reset = button(
        text("Reset to default")
            .size(metrics.label() - 1.0)
            .center()
            .width(Fill),
    )
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
    scrollable(day_canvas(state, metrics, now, state.calendar.day, false))
        .id(iced::widget::Id::new(DAY_SCROLL))
        .height(Fill)
        .width(Fill)
        .into()
}

fn day_canvas<'a>(
    state: &'a State,
    metrics: &Metrics,
    now: DateTime<Local>,
    day: NaiveDate,
    weekly: bool,
) -> Element<'a, Message> {
    let hour_height = state.calendar.hour_height;
    let gutter = if weekly {
        32.0
    } else if metrics.breakpoint.is_compact() {
        COMPACT_GUTTER
    } else {
        GUTTER
    };

    let offset = state.config.calendar.day_end_minutes();
    let day_start_hour = state.config.calendar.day_start_hour();

    let timed = state.calendar.timed_events_on(day, &state.config.calendar);
    let spans: Vec<Span> = timed
        .iter()
        .map(|event| {
            let start = minutes_into_day(event.start, day, offset);
            let end = minutes_into_day(event.end, day, offset);
            let (start_min, end_min) = layout::clamp_day(start, end).unwrap_or_else(|| {
                let start = start.clamp(0, DAY_MINUTES);
                (start, (start + 15).min(DAY_MINUTES))
            });
            Span { start_min, end_min }
        })
        .collect();
    let sources = state.calendar.display_sources(&state.config.calendar);
    let priorities: Vec<_> = timed
        .iter()
        .map(|event| {
            sources
                .iter()
                .position(|source| source.id == event.source_id())
                .unwrap_or(usize::MAX)
        })
        .collect();
    let packed = layout::pack_prioritized(&spans, &priorities);

    let blocks: Vec<Block> = timed
        .iter()
        .zip(spans.iter())
        .zip(packed.iter())
        .map(|((event, span), slot)| Block {
            id: event.id.clone(),
            title: if event.is_task() {
                crate::timeblocking::task_label(event)
            } else {
                event.title.clone()
            },
            when: calendar::event_time_label(event),
            start_min: span.start_min,
            end_min: span.end_min,
            column: slot.column,
            columns: slot.columns,
            color: event.color,
            editable: !event.is_task() || event.details.scheduled.is_some(),
            pending: state.calendar.pending_time(&event.id).is_some()
                || state.timeblocking.busy.contains(&event.id),
            time_block: crate::timeblocking::is_block(event, &state.config.calendar),
            // A block owns its task rows independently of standalone task visibility.
            tasks: if crate::timeblocking::is_block(event, &state.config.calendar) {
                let key = crate::timeblocking::block_key(event);
                let mut tasks: Vec<_> = state
                    .calendar
                    .events_on(day)
                    .iter()
                    .filter(|e| {
                        e.is_task()
                            && crate::timeblocking::metadata(e).block.as_deref()
                                == Some(key.as_str())
                    })
                    .collect();
                tasks.sort_by_key(|e| (crate::timeblocking::metadata(e).order, e.id.clone()));
                tasks
                    .into_iter()
                    .map(|e| {
                        (
                            e.id.clone(),
                            crate::timeblocking::task_label(e),
                            crate::timeblocking::metadata(e).minutes,
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            },
        })
        .collect();

    // The needle only belongs to the day whose window holds "now".
    let needle = (day == calendar::day_at(now, day_start_hour))
        .then(|| minutes_into_day(now, day, offset).clamp(0, DAY_MINUTES));

    // Alarms sit on the day their time falls in, whichever day is in view
    // when they are set.
    let alarm_marks: Vec<AlarmMark> = state
        .alarms
        .items()
        .iter()
        .filter_map(|alarm| {
            let minutes = minutes_into_day(alarm.at, day, offset);
            (0..=DAY_MINUTES).contains(&minutes).then(|| AlarmMark {
                id: alarm.id,
                minutes,
                label: alarm_label(alarm),
            })
        })
        .collect();

    let travel = if state.travel.show {
        travel_boxes(state, &blocks, day)
    } else {
        Vec::new()
    };

    let height = hour_height * 24.0;
    let compact = metrics.breakpoint.is_compact();

    // Empty grid space draws out a new event, unless a card is already open:
    // then a click there is how the card is dismissed.
    let can_create =
        state.calendar.open_event.is_none() && state.calendar.draft.is_none() && !state.alarms.mode;
    let create_color = state
        .calendar
        .default_calendar_id(&state.config.calendar)
        .map_or(colors::DEFAULT_EVENT, |id| {
            state.calendar.calendar_color(&id)
        });

    canvas_widget(DayCanvas {
        blocks,
        travel,
        needle,
        alarms: alarm_marks,
        alarm_mode: state.alarms.mode,
        hour_height,
        gutter,
        compact,
        day_start_hour,
        day,
        can_create,
        create_color,
        external_drag: state.timeblocking.dragging.is_some(),
    })
    .width(Fill)
    .height(Length::Fixed(height))
    .into()
}

/// Minutes in a day, the height of the grid in minutes.
const DAY_MINUTES: i32 = 24 * 60;

/// What an alarm's needle says beside it: its time, and its label if it has
/// one.
fn alarm_label(alarm: &alarms::Alarm) -> String {
    let time = alarm.at.format("%H:%M").to_string();
    let label = alarm.label.trim();

    if label.is_empty() {
        time
    } else {
        format!("{time} {label}")
    }
}

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
const WEEK_SCROLL: &str = "cal-week";

pub fn scroll_week_to_start() -> iced::Task<Message> {
    iced::widget::operation::scroll_to(
        iced::widget::Id::new(WEEK_SCROLL),
        iced::widget::operation::AbsoluteOffset { x: 0.0, y: 0.0 },
    )
}

/// Scrolls the day grid so the needle sits a little below the top, the way
/// Google Calendar opens on "now" rather than the start of the day.
pub fn scroll_to_now(hour_height: f32, day_end_minutes: i32) -> iced::Task<Message> {
    let minutes =
        (calendar::minutes_from_midnight(Local::now()) - day_end_minutes).rem_euclid(DAY_MINUTES);
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
    let dismiss = mouse_area(
        container(iced::widget::Space::new())
            .width(Fill)
            .height(Fill),
    )
    .on_press(Message::CalendarMenuToggled);

    let mut panel = column![text("Calendars").size(metrics.label() + 1.0)]
        .spacing(metrics.spacing)
        .width(Fill);

    for source in state.calendar.display_sources(&state.config.calendar) {
        let is_tasks = source.id == crate::config::CalendarSettings::GOOGLE_TASKS_ID;
        let id = source.id;
        let enabled = state.calendar.calendar_enabled(&id, &state.config.calendar);
        let editing = state.calendar.color_editor.as_deref() == Some(id.as_str());
        let dot = if is_tasks {
            swatch_button(
                source.color,
                editing,
                Message::TaskColorEditorToggled(id.clone()),
            )
        } else {
            static_swatch(source.color)
        };
        let toggle_id = id.clone();
        panel = panel.push(
            row![
                dot,
                text(source.name).size(metrics.label()).width(Fill),
                toggler(enabled).on_toggle(move |value| {
                    if is_tasks {
                        Message::ShowTasksToggled(value)
                    } else {
                        Message::CalendarToggled(toggle_id.clone(), value)
                    }
                }),
            ]
            .spacing(8.0)
            .align_y(Alignment::Center),
        );
        if is_tasks && editing {
            panel = panel.push(task_color_picker(
                &id,
                state.config.calendar.task_color(&id),
                metrics,
            ));
        }
    }
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
        .style(button::primary);

    if state.calendar.is_signed_in() && !state.calendar.is_busy() {
        sync = sync.on_press(Message::CalendarSyncRequested);
    }

    panel = panel.push(sync);
    panel = panel.push(
        button(
            text("Open Google Calendar")
                .size(metrics.label())
                .center()
                .width(Fill),
        )
        .width(Fill)
        .on_press(Message::OpenGoogleCalendar)
        .style(button::secondary),
    );
    panel = panel.push(
        button(
            text("Dock to screen edge")
                .size(metrics.label())
                .center()
                .width(Fill),
        )
        .width(Fill)
        .on_press(Message::DockRequested)
        .style(button::secondary),
    );

    let card = container(scrollable(panel).height(Length::Shrink))
        .max_height(
            (state.main_size.height - header_offset(metrics) - metrics.padding * 2.0).max(120.0),
        )
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
    let dismiss = mouse_area(
        container(iced::widget::Space::new())
            .width(Fill)
            .height(Fill),
    )
    .on_press(Message::CalendarMonthToggled);

    let month = state.calendar.month;
    let today = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
    let weekly = state.weekly && state.screen == crate::app::Screen::Calendar;
    let selected = if weekly {
        state.week_hover.unwrap_or(state.week_anchor)
    } else {
        state.calendar.day
    };
    let preview =
        weekly.then(|| calendar::week_dates(selected, state.config.calendar.weekly_days()));

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
            .map(|&date| day_cell(date, month, selected, today, preview.as_deref(), metrics))
            .collect();
        weeks.push(row(cells).spacing(2.0).width(Fill).into());
    }

    let panel = column![title, weekdays, column(weeks).spacing(2.0).width(Fill),]
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
    preview: Option<&[NaiveDate]>,
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
    .padding([4.0, 0.0])
    .on_press(Message::CalendarDaySelected(date))
    .style(move |theme, status| {
        if is_selected {
            button::primary(theme, status)
        } else if in_preview {
            button::secondary(theme, status)
        } else if is_today {
            button::secondary(theme, status)
        } else {
            let mut style = button::text(theme, status);
            if !in_month {
                style.text_color = style.text_color.scale_alpha(0.4);
            }
            style
        }
    });
    if preview.is_some() {
        mouse_area(cell)
            .on_enter(Message::WeekHover(Some(date)))
            .on_exit(Message::WeekHover(None))
            .into()
    } else {
        cell.into()
    }
}

/// The details card as a full-height pane, drawn to the right of the view.
///
/// It is the same information the old floating card held, but it lives in a
/// column of its own rather than over the calendar, so the day grid and the
/// backlog both keep their full width.
pub fn details_pane<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
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
        super::links::text(event.title.clone(), metrics.label() + 2.0),
        text(schedule_line(event))
            .size(metrics.label())
            .style(text::secondary),
        detail_line("Calendar", calendar_name.to_owned(), metrics),
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    if !event.is_task() && !event.all_day && state.config.calendar.time_block_calendar.is_none() {
        body = body.push(
            button(
                text("Use this calendar for time blocks")
                    .size(metrics.label())
                    .center(),
            )
            .width(Fill)
            .on_press(Message::TimeBlocking(
                crate::timeblocking::Action::Calendar(event.calendar_id.clone()),
            ))
            .style(button::primary),
        );
    }

    let details = &event.details;
    if !is_task(event) && !details.status.is_empty() {
        body = body.push(detail_line("Status", details.status.clone(), metrics));
    }
    if is_task(event) {
        if !details.location.is_empty() {
            body = body.push(detail_line("Location", details.location.clone(), metrics));
        }
    } else {
        // A calendar event gets a place to search for and choose, the same as
        // a task does. Where Google holds a location no saved place matches,
        // that text stays above the button.
        let place = crate::travel::place_of(&state.config.travel, event)
            .and_then(|id| state.config.travel.place(id));
        if place.is_none() && !details.location.is_empty() {
            body = body.push(detail_line("Location", details.location.clone(), metrics));
        }
        body = body.push(super::travel::location_control(
            state,
            crate::travel::PickTarget::Event(event.id.clone()),
            place,
            "Add location",
            metrics,
        ));
    }
    if !details.organizer.is_empty() {
        body = body.push(detail_line("Organizer", details.organizer.clone(), metrics));
    }
    if !details.attendees.is_empty() {
        body = body.push(detail_line("Guests", details.attendees.join("\n"), metrics));
    }
    if !details.description.is_empty() {
        body = body.push(detail_line(
            "Description",
            if event.is_task() {
                crate::timeblocking::description(&details.description)
            } else {
                details.description.clone()
            },
            metrics,
        ));
    }

    // The action buttons sit outside the scrollable, so they stay reachable
    // however long the description above them runs.
    let mut buttons: Vec<Element<'a, Message>> = Vec::new();

    // A Google Task gets a way to tick it off; events never do.
    if is_task(event) {
        body = body.push(super::timeblocking::editor(state, event, metrics));
        buttons.push(super::timeblocking::task_actions(state, event, metrics));
    }

    if !details.meeting.is_empty() {
        let meeting = details.meeting.clone();
        buttons.push(
            button(
                text("Join meeting")
                    .size(metrics.label())
                    .center()
                    .width(Fill),
            )
            .width(Fill)
            .on_press(Message::OpenEventLink(meeting))
            .style(button::primary)
            .into(),
        );
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
            .style(button::secondary)
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
    // Editing an existing calendar event reuses the same fields as creating
    // one: title, time, calendar and description. Google Tasks are not
    // calendar events and cannot be edited this way.
    if !is_task(event) {
        let id = event.id.clone();
        buttons.push(
            button(text("Edit").size(metrics.label()).center().width(Fill))
                .width(Fill)
                .on_press(Message::EventEditRequested(id))
                .style(button::primary)
                .into(),
        );
        let id = event.id.clone();
        buttons.push(
            button(text("Delete").size(metrics.label()).center().width(Fill))
                .width(Fill)
                .on_press(Message::EventDeleteRequested(id))
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

    // The actions are a fixed footer. The scroll is capped so the footer keeps
    // the room it needs, however long the description above it runs; the
    // buttons come first in the budget, so they can never be squeezed out.
    let button_height = metrics.button;
    let button_count = buttons.len() + if event.is_task() { 3 } else { 0 };
    let reserved = button_height * button_count as f32
        + metrics.spacing * button_count.saturating_sub(1) as f32;
    let scroll_max =
        (metrics.size.height - metrics.padding * 2.0 - metrics.spacing - reserved).max(0.0);

    let scrolling = container(scrollable(body).width(Fill).height(Length::Shrink))
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
    .on_press(Message::EventDeleteCancelled);

    let title = if is_task {
        text("Are you sure you want to delete this task?").size(metrics.label() + 1.0)
    } else {
        text("Delete this event?").size(metrics.label() + 1.0)
    };
    let event_title = text(event.title.clone())
        .size(metrics.label())
        .style(text::secondary);

    let confirm = if is_task {
        Message::TaskDeleteConfirmed(id.to_owned())
    } else {
        Message::EventDeleteConfirmed(id.to_owned())
    };

    let mut delete = button(text("Delete").size(metrics.label()).center().width(Fill))
        .width(Fill)
        .style(button::secondary);
    if state.timeblocking.busy.is_empty() {
        delete = delete.on_press(confirm);
    }
    let buttons = row![
        button(text("Cancel").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(Message::EventDeleteCancelled)
            .style(button::secondary),
        delete,
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    let card = container(
        container(
            column![
                title,
                event_title,
                text(if is_task {
                    "This removes the task from Google Tasks and its planned time."
                } else {
                    "This removes the event from Google Calendar."
                })
                .size(metrics.label()),
                buttons
            ]
            .spacing(metrics.spacing),
        )
        .padding(metrics.padding)
        .width(Length::Fixed(260.0))
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
        }),
    )
    .width(Fill)
    .height(Fill)
    .center_x(Fill)
    .center_y(Fill);

    stack![dismiss, card].into()
}

/// The add-event popup, opened by a click or a drag on empty grid space.
///
/// It follows the shape of Google Calendar's own dialog: a title, the event
/// kind, the time, the calendar and a description, with Save at the foot. Only
/// the times, the calendar and the description are editable here; the event
/// kind is shown for parity but only "Event" is implemented.
fn draft_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(draft) = state.calendar.draft.as_ref() else {
        return iced::widget::Space::new().width(0.0).height(0.0).into();
    };

    // Clicking outside the card puts the draft away, the way the menu closes.
    let dismiss = mouse_area(
        container(iced::widget::Space::new())
            .width(Fill)
            .height(Fill),
    )
    .on_press(Message::EventDraftCancelled);

    let header = row![
        text("New event").size(metrics.label() + 1.0).width(Fill),
        button(text("✕").size(metrics.label()).center())
            .width(Length::Fixed(metrics.icon_button()))
            .height(Length::Fixed(metrics.icon_button()))
            .padding(0.0)
            .on_press(Message::EventDraftCancelled)
            .style(button::secondary),
    ]
    .align_y(Alignment::Center);

    let title = text_input("Add title", &draft.title)
        .on_input(Message::EventDraftTitleChanged)
        .size(metrics.label() + 2.0)
        .width(Fill);

    let kinds: Element<'a, Message> = button(text("Event").size(metrics.label()).center())
        .style(button::primary)
        .into();

    let when = column![
        text(draft.start.format("%A, %B %d").to_string()).size(metrics.label()),
        row![
            text_input("09:00", &draft.start_text)
                .on_input(Message::EventDraftStartChanged)
                .size(metrics.label())
                .width(Length::Fixed(72.0)),
            text("–").size(metrics.label()),
            text_input("10:00", &draft.end_text)
                .on_input(Message::EventDraftEndChanged)
                .size(metrics.label())
                .width(Length::Fixed(72.0)),
        ]
        .spacing(6.0)
        .align_y(Alignment::Center),
    ]
    .spacing(4.0)
    .width(Fill);

    let mut description = column![
        text("Description")
            .size(metrics.label() - 1.0)
            .style(text::secondary),
        text_input("Add description", &draft.description)
            .on_input(Message::EventDraftDescriptionChanged)
            .size(metrics.label())
            .width(Fill)
            .padding(8.0),
    ]
    .spacing(2.0)
    .width(Fill);

    if let Some(links) = super::links::for_editor(
        &format!("{}\n{}", draft.title, draft.description),
        metrics.label(),
    ) {
        description = description.push(links);
    }

    let actions = row![
        Space::new().width(Fill),
        button(text("Cancel").size(metrics.label()).center())
            .on_press(Message::EventDraftCancelled)
            .style(button::secondary),
        button(text("Save").size(metrics.label()).center())
            .on_press(Message::EventDraftSaveRequested)
            .style(button::primary),
    ]
    .spacing(metrics.spacing)
    .align_y(Alignment::Center);

    let panel = column![
        header,
        title,
        kinds,
        when,
        draft_calendar_picker(state, metrics),
        description,
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
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..container::Style::default()
            }
        });

    // Swallow presses on the card so a click on its blank area cannot fall
    // through to the dismiss layer and close the draft.
    let card = mouse_area(card).on_press(Message::Ignored);

    let aligned = column![
        iced::widget::Space::new().height(Fill),
        container(card).padding(metrics.padding).width(Fill),
        iced::widget::Space::new().height(Fill),
    ]
    .width(Fill)
    .height(Fill);

    stack![dismiss, aligned].into()
}

/// The edit-event popup, opened from the details card of an existing event.
///
/// It mirrors the add-event popup: title, time, calendar and description,
/// pre-filled from the event the user clicked. Save writes the changes to
/// Google; Cancel throws them away.
fn edit_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(draft) = state.calendar.edit_draft.as_ref() else {
        return iced::widget::Space::new().width(0.0).height(0.0).into();
    };

    // Clicking outside the card puts the edit away.
    let dismiss = mouse_area(
        container(iced::widget::Space::new())
            .width(Fill)
            .height(Fill),
    )
    .on_press(Message::EventEditCancelled);

    let header = row![
        text("Edit event").size(metrics.label() + 1.0).width(Fill),
        button(text("✕").size(metrics.label()).center())
            .width(Length::Fixed(metrics.icon_button()))
            .height(Length::Fixed(metrics.icon_button()))
            .padding(0.0)
            .on_press(Message::EventEditCancelled)
            .style(button::secondary),
    ]
    .align_y(Alignment::Center);

    let title = text_input("Add title", &draft.title)
        .on_input(Message::EventEditTitleChanged)
        .size(metrics.label() + 2.0)
        .width(Fill);

    let kinds: Element<'a, Message> = button(text("Event").size(metrics.label()).center())
        .style(button::primary)
        .into();

    let when = column![
        text(draft.start.format("%A, %B %d").to_string()).size(metrics.label()),
        row![
            text_input("09:00", &draft.start_text)
                .on_input(Message::EventEditStartChanged)
                .size(metrics.label())
                .width(Length::Fixed(72.0)),
            text("–").size(metrics.label()),
            text_input("10:00", &draft.end_text)
                .on_input(Message::EventEditEndChanged)
                .size(metrics.label())
                .width(Length::Fixed(72.0)),
        ]
        .spacing(6.0)
        .align_y(Alignment::Center),
    ]
    .spacing(4.0)
    .width(Fill);

    let mut description = column![
        text("Description")
            .size(metrics.label() - 1.0)
            .style(text::secondary),
        text_input("Add description", &draft.description)
            .on_input(Message::EventEditDescriptionChanged)
            .size(metrics.label())
            .width(Fill)
            .padding(8.0),
    ]
    .spacing(2.0)
    .width(Fill);

    if let Some(links) = super::links::for_editor(
        &format!("{}\n{}", draft.title, draft.description),
        metrics.label(),
    ) {
        description = description.push(links);
    }

    let actions = row![
        Space::new().width(Fill),
        button(text("Cancel").size(metrics.label()).center())
            .on_press(Message::EventEditCancelled)
            .style(button::secondary),
        button(text("Save").size(metrics.label()).center())
            .on_press(Message::EventEditSaveRequested)
            .style(button::primary),
    ]
    .spacing(metrics.spacing)
    .align_y(Alignment::Center);

    let panel = column![
        header,
        title,
        kinds,
        when,
        draft_calendar_picker(state, metrics),
        description,
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
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 8.0.into(),
                },
                ..container::Style::default()
            }
        });

    // Swallow presses on the card so a click on its blank area cannot fall
    // through to the dismiss layer and close the edit.
    let card = mouse_area(card).on_press(Message::Ignored);

    let aligned = column![
        iced::widget::Space::new().height(Fill),
        container(card).padding(metrics.padding).width(Fill),
        iced::widget::Space::new().height(Fill),
    ]
    .width(Fill)
    .height(Fill);

    stack![dismiss, aligned].into()
}

/// A calendar offered in the add-event popup's calendar picker.
#[derive(Debug, Clone, PartialEq)]
struct CalendarChoice {
    id: String,
    name: String,
    color: Color,
}

impl std::fmt::Display for CalendarChoice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.name)
    }
}

/// A small colored circle used as a calendar swatch in the dropdown.
pub(super) fn color_circle<'a>(color: Color) -> Element<'a, Message> {
    container(iced::widget::Space::new().width(10.0).height(10.0))
        .style(move |_theme| container::Style {
            background: Some(color.into()),
            border: iced::Border {
                radius: 5.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// The calendar the new event will be created on. A dropdown when there is
/// room, and a button that cycles the options when there is not.
fn draft_calendar_picker<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let options: Vec<CalendarChoice> = state
        .calendar
        .calendars
        .iter()
        .filter(|calendar| !calendar.is_task_list())
        // Assigned task times live on a calendar the app manages. A new event
        // is a normal calendar event, so that calendar is not a choice.
        .filter(|calendar| {
            state.config.calendar.tasks_calendar.as_deref() != Some(calendar.id.as_str())
        })
        .map(|calendar| CalendarChoice {
            id: calendar.id.clone(),
            name: calendar.name.clone(),
            color: calendar.color,
        })
        .collect();

    let current = state
        .calendar
        .draft
        .as_ref()
        .map(|draft| draft.calendar_id.clone())
        .unwrap_or_default();
    let selected = options.iter().find(|option| option.id == current).cloned();
    let dropdown_open = state
        .calendar
        .draft
        .as_ref()
        .map(|draft| draft.calendar_dropdown_open)
        .unwrap_or(false);

    let caption = text("Calendar")
        .size(metrics.label() - 1.0)
        .style(text::secondary);

    if metrics.breakpoint.is_compact() {
        let shown = selected
            .as_ref()
            .map(|option| option.name.clone())
            .unwrap_or_else(|| "Calendar".to_owned());
        let next = next_calendar(&options, selected.as_ref());

        let control = button(text(shown).size(metrics.label()).center().width(Fill)).width(Fill);
        let control = match next {
            Some(option) => control.on_press(Message::EventDraftCalendarSelected(option.id)),
            None => control,
        };

        column![caption, control.style(button::secondary)]
            .spacing(2.0)
            .width(Fill)
            .into()
    } else if dropdown_open {
        calendar_dropdown_overlay(options, selected, metrics)
    } else {
        let shown = selected
            .as_ref()
            .map(|option| option.name.clone())
            .unwrap_or_else(|| "Calendar".to_owned());
        let color = selected
            .as_ref()
            .map_or(Color::from_rgb8(0xCC, 0xCC, 0xCC), |c| c.color);

        let control = button(
            row![
                color_circle(color),
                text(shown).size(metrics.label()).width(Fill),
            ]
            .spacing(8.0)
            .align_y(Alignment::Center)
            .padding([2.0, 8.0]),
        )
        .width(Fill)
        .on_press(Message::EventDraftCalendarDropdownToggled)
        .style(button::secondary);

        column![caption, control].spacing(2.0).width(Fill).into()
    }
}

/// The open calendar dropdown overlay: a scrollable list of calendars,
/// each shown with its color circle to the left of the name.
fn calendar_dropdown_overlay<'a>(
    options: Vec<CalendarChoice>,
    selected: Option<CalendarChoice>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let dismiss = mouse_area(
        container(iced::widget::Space::new())
            .width(Fill)
            .height(Fill),
    )
    .on_press(Message::EventDraftCalendarDropdownToggled);

    let mut list = column![];
    for option in &options {
        let is_selected = selected.as_ref().map_or(false, |s| s.id == option.id);
        let id = option.id.clone();
        let color = option.color;
        let name = option.name.clone();

        let item = button(
            row![
                color_circle(color),
                text(name).size(metrics.label()).width(Fill),
            ]
            .spacing(8.0)
            .align_y(Alignment::Center)
            .padding([6.0, 10.0]),
        )
        .width(Fill)
        .on_press(Message::EventDraftCalendarSelected(id.clone()))
        .style(move |theme, status| {
            if is_selected {
                button::primary(theme, status)
            } else {
                button::text(theme, status)
            }
        });

        list = list.push(item);
        list = list.push(Space::new().height(4.0));
    }

    let scrollable = scrollable(list).width(Fill).height(Length::Fixed(320.0));

    let card = container(scrollable)
        .padding(4.0)
        .width(Fill)
        .height(Length::Fixed(328.0))
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                border: iced::Border {
                    color: palette.background.strong.color,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..container::Style::default()
            }
        });

    container(stack![dismiss, card])
        .width(Fill)
        .height(Length::Fixed(328.0))
        .into()
}

/// The calendar after `selected`, wrapping around at the end.
fn next_calendar(
    options: &[CalendarChoice],
    selected: Option<&CalendarChoice>,
) -> Option<CalendarChoice> {
    if options.is_empty() {
        return None;
    }

    let index = selected
        .and_then(|current| options.iter().position(|option| option == current))
        .map_or(0, |index| (index + 1) % options.len());

    options.get(index).cloned()
}

fn is_task(event: &Event) -> bool {
    event.is_task()
}

fn detail_line<'a>(label: &'a str, value: String, metrics: &Metrics) -> Element<'a, Message> {
    column![
        text(label)
            .size(metrics.label() - 1.0)
            .style(text::secondary),
        super::links::text(value, metrics.label()),
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
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
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
    /// Timed task blocks and calendar events can both be dragged.
    editable: bool,
    /// Whether a time change for this event is still being written to Google.
    /// The block is drawn lighter until the write is confirmed.
    pending: bool,
    tasks: Vec<(String, String, u32)>,
    time_block: bool,
}

/// An alarm's needle on the day in view.
#[derive(Debug, Clone)]
struct AlarmMark {
    id: u64,
    /// Minutes from the top of the day, 0 to 24h.
    minutes: i32,
    label: String,
}

/// A trip drawn beside the events it joins: in a gray box just before the
/// event it leads to, or just after the day's last event for the trip home.
#[derive(Debug, Clone)]
struct TravelBox {
    leg: crate::travel::Leg,
    /// What the trip leaves and what it reaches, for its details card.
    between: (String, String),
    label: String,
    /// The minute of the day the box is fixed to: the start of the event it
    /// comes before, or the end of the event it follows.
    anchor_min: i32,
    /// Whether the box hangs below its anchor rather than above it.
    below: bool,
    /// How long the trip takes, once known, which sets the box's height.
    minutes: u32,
    /// Where the event it belongs to sits, so the box lines up with it.
    column: usize,
    columns: usize,
    time_block: bool,
}

/// The shortest a travel box is drawn, so its words fit in it.
const TRAVEL_MIN_HEIGHT: f32 = 16.0;

/// The gray boxes for the day's trips, found from the blocks already laid out.
fn travel_boxes(state: &State, blocks: &[Block], day: NaiveDate) -> Vec<TravelBox> {
    crate::travel::day_hops(state, day)
        .into_iter()
        .filter_map(|hop| {
            let (event, below) = match (&hop.before, &hop.after) {
                (Some(id), _) => (id, false),
                (None, Some(id)) => (id, true),
                (None, None) => return None,
            };
            let block = blocks.iter().find(|block| &block.id == event)?;
            let minutes = match crate::travel::summary(state, hop.leg) {
                crate::travel::Summary::Ready(route) => route.seconds.div_ceil(60),
                _ => 0,
            };

            Some(TravelBox {
                leg: hop.leg,
                label: crate::travel::short_label(state, hop.leg),
                between: (hop.from_title, hop.to_title),
                anchor_min: if below {
                    block.end_min
                } else {
                    block.start_min
                },
                below,
                minutes,
                column: block.column,
                columns: block.columns,
                time_block: block.time_block,
            })
        })
        .collect()
}

struct DayCanvas {
    blocks: Vec<Block>,
    travel: Vec<TravelBox>,
    needle: Option<i32>,
    /// The alarms that fall on this day.
    alarms: Vec<AlarmMark>,
    /// Whether the grid is in alarm setting mode: a purple needle follows the
    /// pointer, and a click sets an alarm where it is.
    alarm_mode: bool,
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
    /// Whether empty grid space draws out a new event. False while a card is
    /// open, so a click there still dismisses it.
    can_create: bool,
    /// The color the new-event preview is drawn in, taken from the calendar the
    /// event will land on.
    create_color: Color,
    external_drag: bool,
}

/// A drag on empty grid space that is drawing out a new event.
///
/// Both ends are snapped to the quarter hour as the pointer moves, so the
/// preview lands on the same times the event will.
#[derive(Debug, Clone, Copy)]
struct CreateDrag {
    /// The minute the press landed on.
    anchor_min: i32,
    /// The minute under the pointer now.
    head_min: i32,
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
    /// The new-event drag in progress, if any. Kept out of the grid cache for
    /// the same reason as `drag`.
    create: std::cell::RefCell<Option<CreateDrag>>,
    task_press: Option<(String, Point)>,
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

    /// A copy of the new-event drag in progress, if there is one.
    fn create(&self) -> Option<CreateDrag> {
        *self.create.borrow()
    }

    /// Replaces the new-event drag in progress, or clears it with `None`.
    fn set_create(&self, create: Option<CreateDrag>) {
        *self.create.borrow_mut() = create;
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
            create: std::cell::RefCell::new(None),
            task_press: None,
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
        // Alarm mode takes the pointer for itself: the grid is not a place to
        // drag events or draw new ones until the mode is left. Zooming still
        // works, so the time can be picked more finely.
        if self.alarm_mode
            && !matches!(
                event,
                iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(_))
                    | iced::Event::Mouse(mouse::Event::WheelScrolled { .. })
            )
        {
            return self.alarm_mode_update(event, bounds, cursor);
        }

        if matches!(
            event,
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
        ) && self.external_drag
        {
            state.task_press = None;
            let point = cursor.position_in(bounds)?;
            if let Some(id) = self.task_at(point, bounds.width) {
                return Some(
                    canvas::Action::publish(Message::TimeBlocking(
                        crate::timeblocking::Action::DropTask(id.to_owned()),
                    ))
                    .and_capture(),
                );
            }
            let block = self.blocks.iter().find(|b| {
                b.time_block
                    && self
                        .rect_for(b, bounds.width, b.start_min, b.end_min)
                        .contains(point)
            });
            if let Some(block) = block {
                return Some(
                    canvas::Action::publish(Message::TimeBlocking(
                        crate::timeblocking::Action::DropBlock(block.id.clone()),
                    ))
                    .and_capture(),
                );
            }
            return Some(
                canvas::Action::publish(Message::TimeBlocking(
                    crate::timeblocking::Action::DropTime(self.time_at(self.snap_minute(point.y))),
                ))
                .and_capture(),
            );
        }
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
                if let Some((id, origin)) = &state.task_press {
                    if cursor
                        .position()
                        .is_some_and(|point| point.distance(*origin) >= 4.0)
                    {
                        let id = id.clone();
                        state.task_press = None;
                        return Some(
                            canvas::Action::publish(Message::TimeBlocking(
                                crate::timeblocking::Action::Drag(id),
                            ))
                            .and_capture(),
                        );
                    }
                    return Some(canvas::Action::capture());
                }
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

                // A new-event drag owns the pointer too, so the hover highlight
                // stays out of the way while the preview follows it.
                if let Some(create) = state.create() {
                    if let Some(point) = cursor.position_in(bounds) {
                        let head = self.snap_minute(point.y);
                        if head != create.head_min {
                            state.set_create(Some(CreateDrag {
                                head_min: head,
                                ..create
                            }));
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

                // An alarm's needle opens the alarm for editing.
                if let Some(id) = self.alarm_at(point, bounds.width) {
                    return Some(
                        canvas::Action::publish(Message::Alarms(alarms::Action::Open(id)))
                            .and_capture(),
                    );
                }

                // A travel box opens the trip's details. It is drawn over the
                // events, so it is asked before them.
                if let Some(travel) = self.travel_at(point, bounds.width) {
                    return Some(
                        canvas::Action::publish(Message::Travel(crate::travel::Action::OpenLeg(
                            travel.leg,
                            Some(travel.between.clone()),
                        )))
                        .and_capture(),
                    );
                }

                for block in &self.blocks {
                    if !block.time_block {
                        continue;
                    }
                    let rect = self.block_rect(block, bounds.width);
                    let (view, edit) = block_actions(rect);
                    if edit.contains(point) {
                        return Some(
                            canvas::Action::publish(Message::EventOpened(block.id.clone()))
                                .and_capture(),
                        );
                    }
                    if view.contains(point) {
                        return Some(
                            canvas::Action::publish(Message::TimeBlocking(
                                crate::timeblocking::Action::View(block.id.clone()),
                            ))
                            .and_capture(),
                        );
                    }
                }

                if let Some(id) = self.task_at(point, bounds.width) {
                    state.task_press = Some((id.to_owned(), cursor.position()?));
                    return Some(canvas::Action::request_redraw().and_capture());
                }

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

                // Empty time either draws out a new event or closes any open
                // card, which is how clicking away still works now that there
                // is no dismiss layer.
                match self.hit(Some(point), bounds.width) {
                    Some(index) => {
                        let id = self.blocks.get(index)?.id.clone();
                        Some(canvas::Action::publish(Message::EventOpened(id)).and_capture())
                    }
                    None if self.can_create => {
                        let anchor = self.snap_minute(point.y);
                        state.set_create(Some(CreateDrag {
                            anchor_min: anchor,
                            head_min: anchor,
                        }));
                        Some(canvas::Action::request_redraw().and_capture())
                    }
                    None => Some(canvas::Action::publish(Message::EventClosed).and_capture()),
                }
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if let Some((id, origin)) = state.task_press.take() {
                    return if cursor
                        .position()
                        .is_some_and(|point| point.distance(origin) < 4.0)
                    {
                        Some(canvas::Action::publish(Message::EventOpened(id)).and_capture())
                    } else {
                        Some(canvas::Action::capture())
                    };
                }
                // A new-event drag is finished first: it never shares the
                // pointer with a move or resize.
                if let Some(create) = state.create.take() {
                    let (start, end) = self.create_range(&create);
                    return Some(
                        canvas::Action::publish(Message::EventDraftStarted {
                            start: self.time_at(start),
                            end: self.time_at(end),
                        })
                        .and_capture(),
                    );
                }

                let drag = state.drag.take()?;

                // A press that never moved is a click, not a drag.
                let action =
                    if drag.preview_start == drag.start_min && drag.preview_end == drag.end_min {
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
        if self.alarm_mode {
            return match cursor.position_in(bounds) {
                Some(point) if self.alarm_can_be_set(point.y) => mouse::Interaction::Crosshair,
                Some(_) => mouse::Interaction::NotAllowed,
                None => mouse::Interaction::default(),
            };
        }

        if self.external_drag {
            return mouse::Interaction::Grabbing;
        }

        if cursor
            .position_in(bounds)
            .is_some_and(|point| self.alarm_at(point, bounds.width).is_some())
        {
            return mouse::Interaction::Pointer;
        }
        if cursor
            .position_in(bounds)
            .is_some_and(|point| self.travel_at(point, bounds.width).is_some())
        {
            return mouse::Interaction::Pointer;
        }
        if state.create().is_some() {
            return mouse::Interaction::Crosshair;
        }

        if let Some(drag) = state.drag() {
            return match drag.mode {
                DragMode::Move => mouse::Interaction::Grabbing,
                DragMode::ResizeStart | DragMode::ResizeEnd => {
                    mouse::Interaction::ResizingVertically
                }
            };
        }

        if let Some(point) = cursor.position_in(bounds) {
            for block in self.blocks.iter().filter(|block| block.time_block) {
                let (view, edit) = block_actions(self.block_rect(block, bounds.width));
                if view.contains(point) || edit.contains(point) {
                    return mouse::Interaction::Pointer;
                }
            }
        }

        if cursor
            .position_in(bounds)
            .and_then(|point| self.task_at(point, bounds.width))
            .is_some()
        {
            return mouse::Interaction::Grab;
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
        )
        .wrapping_mul(31)
        .wrapping_add(travel_key(&self.travel));
        if state.key.get() != key {
            state.grid.clear();
            state.key.set(key);
        }

        let grid = state.grid.draw(renderer, bounds.size(), |frame| {
            self.draw_grid(frame, dragging);
            self.draw_travel(frame);
        });

        let mut overlay = Frame::new(renderer, bounds.size());
        self.draw_needle(&mut overlay);
        self.draw_alarms(&mut overlay, cursor.position_in(bounds));
        if self.alarm_mode {
            if let Some(point) = cursor.position_in(bounds) {
                self.draw_alarm_cursor(&mut overlay, point);
            }
        } else if let Some(create) = state.create() {
            self.draw_create(&mut overlay, &create);
        } else if let Some(drag) = drag {
            self.draw_drag(&mut overlay, &drag);
        } else if cursor
            .position_in(bounds)
            .is_some_and(|point| self.travel_at(point, bounds.width).is_some())
        {
            // The box is what a click would open, not the event under it.
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

            fill_glyphs(
                frame,
                Text {
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
                },
            );
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
            if block.time_block {
                let total: u32 = block.tasks.iter().map(|t| t.2).sum();
                let duration = (block.end_min - block.start_min).max(1) as f32;
                let occupied = (total as f32 / duration).min(1.0);
                frame.fill_rectangle(
                    Point::new(block_bounds.x, block_bounds.y),
                    Size::new(block_bounds.width, block_bounds.height * occupied),
                    Color::from_rgba(0.15, 0.15, 0.15, 0.28),
                );
                let mut elapsed = 0;
                for (i, (_, title, minutes)) in block.tasks.iter().enumerate() {
                    elapsed += minutes;
                    let over = elapsed as f32 > duration;
                    let y = block_bounds.y + i as f32 * 20.0;
                    let x = block_bounds.x + block_bounds.width + 4.0;
                    if over {
                        frame.fill_rectangle(
                            Point::new(x, y),
                            Size::new(block_bounds.width, 19.0),
                            Color::from_rgba(0.9, 0.2, 0.15, 0.22),
                        );
                    }
                    let label = format!("○ {} · {}m", title, minutes);
                    let label = fit_lines(&label, 11.0, block_bounds.width - 6.0, 1)
                        .into_iter()
                        .next()
                        .unwrap_or_default();
                    fill_glyphs(
                        frame,
                        Text {
                            content: label,
                            position: Point::new(x + 3.0, y + 2.0),
                            color: if over {
                                Color::from_rgb8(220, 65, 55)
                            } else {
                                Color::from_rgb8(125, 125, 125)
                            },
                            size: Pixels(11.0),
                            ..Text::default()
                        },
                    );
                }
            }
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
            let mut label_bounds = block_bounds;
            if block.time_block {
                if block_bounds.width >= 160.0 {
                    label_bounds.width -= 78.0;
                } else {
                    label_bounds.y += 22.0;
                    label_bounds.height = (label_bounds.height - 22.0).max(0.0);
                }
            }
            if label_bounds.height >= 14.0 {
                draw_block_label(
                    frame,
                    label_bounds,
                    &block.title,
                    &block.when,
                    ink,
                    title_size,
                );
            }
            if block.time_block {
                for (label, bounds) in [
                    ("View", block_actions(block_bounds).0),
                    ("Edit", block_actions(block_bounds).1),
                ] {
                    frame.fill(
                        &Path::rounded_rectangle(bounds.position(), bounds.size(), 3.0.into()),
                        Color::WHITE.scale_alpha(0.27),
                    );
                    fill_glyphs(
                        frame,
                        Text {
                            content: label.into(),
                            position: Point::new(bounds.center_x(), bounds.center_y()),
                            color: ink,
                            size: Pixels(10.0),
                            align_x: iced::alignment::Horizontal::Center.into(),
                            align_y: iced::alignment::Vertical::Center,
                            ..Text::default()
                        },
                    );
                }
            }
        }
    }

    /// Draws the gray boxes for the trips, over the events they lead to.
    fn draw_travel(&self, frame: &mut Frame) {
        let size = if self.compact { 10.0 } else { 11.0 };

        for trip in &self.travel {
            let bounds = self.travel_rect(trip, frame.width());
            let shape = Path::rounded_rectangle(
                Point::new(bounds.x, bounds.y),
                Size::new(bounds.width, bounds.height),
                EVENT_RADIUS.into(),
            );

            frame.fill(&shape, colors::TRAVEL);
            frame.stroke(
                &shape,
                Stroke {
                    style: Style::Solid(Color::BLACK.scale_alpha(0.45)),
                    width: 1.0,
                    ..Stroke::default()
                },
            );

            let room = (bounds.width - BLOCK_PADDING - BLOCK_PADDING_RIGHT).max(8.0);
            fill_glyphs(
                frame,
                Text {
                    content: fit_line(&trip.label, size, room),
                    position: Point::new(bounds.x + BLOCK_PADDING, bounds.y + 2.0),
                    color: on_color(colors::TRAVEL),
                    size: Pixels(size),
                    font: Font::DEFAULT,
                    align_x: iced::alignment::Horizontal::Left.into(),
                    align_y: iced::alignment::Vertical::Top,
                    line_height: text::LineHeight::default(),
                    shaping: text::Shaping::Basic,
                    max_width: room,
                },
            );
        }
    }

    /// Where a trip's box sits: on the event it belongs to, hanging off its
    /// start (or its end, for the trip home) by as long as the trip takes.
    fn travel_rect(&self, trip: &TravelBox, width: f32) -> Rectangle {
        let minute_height = self.hour_height / 60.0;
        let grid_width = (width - self.gutter).max(40.0);
        let columns = trip.columns.max(1) as f32;
        let height = (trip.minutes as f32 * minute_height).max(TRAVEL_MIN_HEIGHT);
        let anchor = trip.anchor_min as f32 * minute_height;
        let mut box_width = (grid_width / columns - EVENT_GAP).max(8.0);
        if trip.time_block {
            box_width = (box_width - 6.0) * 0.5;
        }

        Rectangle {
            x: self.gutter + trip.column as f32 * (grid_width / columns) + EVENT_GAP / 2.0,
            y: if trip.below {
                anchor
            } else {
                (anchor - height).max(0.0)
            },
            width: box_width,
            height,
        }
    }

    /// The trip box under a grid point, if any. The last drawn is on top.
    fn travel_at(&self, point: Point, width: f32) -> Option<&TravelBox> {
        self.travel
            .iter()
            .rev()
            .find(|trip| self.travel_rect(trip, width).contains(point))
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
        frame.fill(
            &Path::circle(Point::new(self.gutter, y), NEEDLE_RADIUS),
            colors::NEEDLE,
        );
    }

    /// The pixel row a minute of the day sits on.
    fn y_of(&self, minutes: i32) -> f32 {
        minutes as f32 * (self.hour_height / 60.0)
    }

    /// The minute of the day under a grid pixel, to the whole minute.
    ///
    /// Alarms are exact where events snap to the quarter hour, so the needle
    /// follows the pointer minute by minute.
    fn alarm_minute(&self, y: f32) -> i32 {
        let minute_height = self.hour_height / 60.0;

        ((y / minute_height).round() as i32).clamp(0, DAY_MINUTES)
    }

    /// Whether an alarm can be set at the minute under a grid pixel.
    fn alarm_can_be_set(&self, y: f32) -> bool {
        alarms::can_set(Local::now(), self.time_at(self.alarm_minute(y)))
    }

    /// What alarm setting mode does with the pointer: the needle follows it,
    /// and a click sets the alarm if its time is still to come.
    fn alarm_mode_update(
        &self,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            iced::Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                Some(canvas::Action::request_redraw())
            }
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let point = cursor.position_in(bounds)?;
                let at = self.time_at(self.alarm_minute(point.y));

                // A time that has passed is refused here as well as where the
                // alarm is set, and the click is still taken so it cannot fall
                // through to an event underneath.
                if alarms::can_set(Local::now(), at) {
                    Some(
                        canvas::Action::publish(Message::Alarms(alarms::Action::Place(at)))
                            .and_capture(),
                    )
                } else {
                    Some(canvas::Action::capture())
                }
            }
            _ => None,
        }
    }

    /// The pill an alarm's label sits in, above the right end of its needle.
    fn alarm_pill(&self, mark: &AlarmMark, width: f32) -> Rectangle {
        let size = Size::new(self.alarm_pill_width(&mark.label, width), ALARM_PILL_HEIGHT);
        let y = self.y_of(mark.minutes);

        Rectangle {
            x: (width - size.width - ALARM_PILL_MARGIN).max(self.gutter),
            // Below the needle instead when there is no room above it.
            y: if y >= size.height + 2.0 {
                y - size.height - 1.0
            } else {
                y + 2.0
            },
            width: size.width,
            height: size.height,
        }
    }

    fn alarm_pill_width(&self, label: &str, width: f32) -> f32 {
        let room = (width - self.gutter - ALARM_PILL_MARGIN * 2.0).max(24.0);

        (text_width_of(label, ALARM_PILL_TEXT) + 12.0).min(room)
    }

    /// The alarm under a grid point, if any.
    ///
    /// The pill and the dot on the gutter always count. The needle's own line
    /// only counts where no event is, so an alarm never makes the event it
    /// crosses impossible to click.
    fn alarm_at(&self, point: Point, width: f32) -> Option<u64> {
        let free = self.hit(Some(point), width).is_none();

        self.alarms.iter().rev().find_map(|mark| {
            let y = self.y_of(mark.minutes);
            let on_pill = self.alarm_pill(mark, width).contains(point);
            let on_dot = point.distance(Point::new(self.gutter, y)) <= NEEDLE_RADIUS + 3.0;
            let on_line = free && point.x >= self.gutter && (point.y - y).abs() <= 4.0;

            (on_pill || on_dot || on_line).then_some(mark.id)
        })
    }

    /// Draws every alarm's needle, with the one under the pointer thicker.
    fn draw_alarms(&self, frame: &mut Frame, pointer: Option<Point>) {
        let hovered = if self.alarm_mode {
            None
        } else {
            pointer.and_then(|point| self.alarm_at(point, frame.width()))
        };

        for mark in &self.alarms {
            let y = self.y_of(mark.minutes).clamp(0.0, frame.height());
            let lit = hovered == Some(mark.id);

            frame.stroke(
                &Path::line(Point::new(self.gutter, y), Point::new(frame.width(), y)),
                Stroke {
                    style: Style::Solid(colors::ALARM),
                    width: if lit { 3.5 } else { 2.0 },
                    ..Stroke::default()
                },
            );
            frame.fill(
                &Path::circle(Point::new(self.gutter, y), NEEDLE_RADIUS),
                colors::ALARM,
            );

            let pill = self.alarm_pill(mark, frame.width());
            let label = fit_line(&mark.label, ALARM_PILL_TEXT, pill.width - 8.0);
            draw_pill(frame, pill, &label, colors::ALARM);
        }
    }

    /// Draws the needle that follows the pointer in alarm setting mode, with
    /// its offset from now beside it.
    ///
    /// The needle goes grey and the offset turns negative once the pointer is
    /// over a time that has passed, which is the one place an alarm cannot go.
    fn draw_alarm_cursor(&self, frame: &mut Frame, pointer: Point) {
        let minutes = self.alarm_minute(pointer.y);
        let at = self.time_at(minutes);
        let now = Local::now();

        let color = if alarms::can_set(now, at) {
            colors::ALARM
        } else {
            colors::ALARM_REFUSED
        };
        let y = self.y_of(minutes);

        frame.stroke(
            &Path::line(Point::new(self.gutter, y), Point::new(frame.width(), y)),
            Stroke {
                style: Style::Solid(color),
                width: 2.0,
                ..Stroke::default()
            },
        );
        frame.fill(
            &Path::circle(Point::new(self.gutter, y), NEEDLE_RADIUS),
            color,
        );

        let label = alarms::offset_label(alarms::minutes_between(now, at));
        let width = text_width_of(&label, ALARM_PILL_TEXT) + 12.0;
        let x = (pointer.x + 14.0).clamp(
            self.gutter + 4.0,
            (frame.width() - width - 4.0).max(self.gutter),
        );
        let top = if y >= ALARM_PILL_HEIGHT + 4.0 {
            y - ALARM_PILL_HEIGHT - 3.0
        } else {
            y + 5.0
        };

        draw_pill(
            frame,
            Rectangle {
                x,
                y: top,
                width,
                height: ALARM_PILL_HEIGHT,
            },
            &label,
            color,
        );
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

        // The label shows the time the event will land on, not the one it had.
        let when = format!(
            "{} – {}",
            self.clock_label(drag.preview_start),
            self.clock_label(drag.preview_end)
        );
        draw_block_label(frame, bounds, &block.title, &when, ink, title_size);
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

    /// The task row beside a time block, including its title and estimate.
    fn task_at(&self, point: Point, width: f32) -> Option<&str> {
        self.blocks
            .iter()
            .filter(|block| block.time_block)
            .find_map(|block| {
                let rect = self.block_rect(block, width);
                let rows = Rectangle {
                    x: rect.x + rect.width + 4.0,
                    y: rect.y,
                    width: rect.width,
                    height: block.tasks.len() as f32 * 20.0,
                };
                if !rows.contains(point) {
                    return None;
                }
                block
                    .tasks
                    .get(((point.y - rows.y) / 20.0) as usize)
                    .map(|task| task.0.as_str())
            })
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

    /// The minute-of-day a grid pixel lands on, snapped to the quarter hour
    /// and clamped to the day.
    fn snap_minute(&self, y: f32) -> i32 {
        let minute_height = self.hour_height / 60.0;
        let minutes = (y / minute_height).round() as i32;
        let snapped = (minutes as f32 / SNAP_MINUTES as f32).round() as i32 * SNAP_MINUTES;
        snapped.clamp(0, DAY_MINUTES)
    }

    /// The span a new-event drag covers.
    ///
    /// A click, or a drag too short to be a real span, gets
    /// [`DEFAULT_DURATION`] from where it started, the way Google Calendar
    /// does. The span is then clamped so it stays inside the day and never
    /// inverts.
    fn create_range(&self, create: &CreateDrag) -> (i32, i32) {
        let (mut start, mut end) = if create.anchor_min <= create.head_min {
            (create.anchor_min, create.head_min)
        } else {
            (create.head_min, create.anchor_min)
        };

        if end - start < MIN_DURATION {
            end = start + DEFAULT_DURATION;
        }

        start = start.clamp(0, DAY_MINUTES - MIN_DURATION);
        end = end.clamp(start + MIN_DURATION, DAY_MINUTES);
        (start, end)
    }

    /// Draws the new-event preview: a translucent block in the target
    /// calendar's color, labelled with the span being drawn.
    fn draw_create(&self, frame: &mut Frame, create: &CreateDrag) {
        let (start, end) = self.create_range(create);
        let minute_height = self.hour_height / 60.0;
        let grid_width = (frame.width() - self.gutter).max(40.0);
        let bounds = Rectangle {
            x: self.gutter + EVENT_GAP / 2.0,
            y: start as f32 * minute_height,
            width: (grid_width - EVENT_GAP).max(8.0),
            height: ((end - start) as f32 * minute_height).max(16.0),
        };
        let shape = Path::rounded_rectangle(
            Point::new(bounds.x, bounds.y),
            Size::new(bounds.width, bounds.height),
            EVENT_RADIUS.into(),
        );

        let fill = self.create_color.scale_alpha(0.75);
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
        let text_width = (bounds.width - BLOCK_PADDING - BLOCK_PADDING_RIGHT).max(8.0);
        let label = format!("{} – {}", self.clock_label(start), self.clock_label(end));
        fill_glyphs(
            frame,
            Text {
                content: fit_line(&label, title_size, text_width),
                position: Point::new(bounds.x + BLOCK_PADDING, bounds.y + 3.0),
                color: ink,
                size: Pixels(title_size),
                font: Font::DEFAULT,
                align_x: iced::alignment::Horizontal::Left.into(),
                align_y: iced::alignment::Vertical::Top,
                line_height: text::LineHeight::default(),
                shaping: text::Shaping::Basic,
                max_width: text_width,
            },
        );
    }

    /// The real time a previewed minute-of-day lands on.
    fn time_at(&self, minutes: i32) -> DateTime<Local> {
        let offset = self.day_start_hour as i32 * 60;
        let naive = self.day.and_hms_opt(0, 0, 0).unwrap_or_default()
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
        self.blocks
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, block)| {
                self.block_rect(block, width)
                    .contains(point)
                    .then_some(index)
            })
    }

    fn block_rect(&self, block: &Block, width: f32) -> Rectangle {
        let mut rect = self.rect_for(block, width, block.start_min, block.end_min);
        if block.time_block {
            rect.width = (rect.width - 6.0) * 0.5;
        }
        rect
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

/// The text size inside an alarm's pill.
const ALARM_PILL_TEXT: f32 = 10.0;
const ALARM_PILL_HEIGHT: f32 = 16.0;
/// The gap between an alarm's pill and the right edge of the grid, which keeps
/// it clear of the scrollbar.
const ALARM_PILL_MARGIN: f32 = 10.0;

/// A rounded label with white text on a solid color.
fn draw_pill(frame: &mut Frame, bounds: Rectangle, label: &str, color: Color) {
    frame.fill(
        &Path::rounded_rectangle(bounds.position(), bounds.size(), 4.0.into()),
        color,
    );
    fill_glyphs(
        frame,
        Text {
            content: label.to_owned(),
            position: Point::new(bounds.center_x(), bounds.center_y()),
            color: Color::WHITE,
            size: Pixels(ALARM_PILL_TEXT),
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..Text::default()
        },
    );
}

/// View and Edit share the event's top-right corner, in that order.
fn block_actions(bounds: Rectangle) -> (Rectangle, Rectangle) {
    let width = 36.0_f32.min(((bounds.width - 6.0) / 2.0).max(1.0));
    let height = 18.0_f32.min((bounds.height - 4.0).max(1.0));
    let edit = Rectangle {
        x: bounds.x + bounds.width - width - 2.0,
        y: bounds.y + 2.0,
        width,
        height,
    };
    let view = Rectangle {
        x: edit.x - width - 2.0,
        ..edit
    };
    (view, edit)
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

/// Draws a block's title and the time under it, inside the block.
///
/// A Google summary is free text: it can be longer than the event it names,
/// it can bring its own line breaks, and it can hold a word too wide for the
/// block on its own. The title is broken into the lines that fit and cut
/// short with an ellipsis when there are more of them than the block has
/// room for, so a label never reaches past the edge of the event it belongs
/// to and lands on top of the ones below it.
fn draw_block_label(
    frame: &mut Frame,
    bounds: Rectangle,
    title: &str,
    when: &str,
    ink: Color,
    title_size: f32,
) {
    let when_size = title_size - 1.0;
    let title_line = line_height(title_size);
    let when_line = line_height(when_size);
    let text_width = (bounds.width - BLOCK_PADDING - BLOCK_PADDING_RIGHT).max(8.0);

    let (top, room) = text_room(bounds.height, title_size);
    let title_top = bounds.y + top;

    let lines = fit_lines(title, title_size, text_width, room);

    for (index, line) in lines.iter().enumerate() {
        fill_glyphs(
            frame,
            Text {
                content: line.clone(),
                position: Point::new(
                    bounds.x + BLOCK_PADDING,
                    title_top + index as f32 * title_line,
                ),
                color: ink,
                size: Pixels(title_size),
                font: Font::DEFAULT,
                align_x: iced::alignment::Horizontal::Left.into(),
                align_y: iced::alignment::Vertical::Top,
                line_height: text::LineHeight::default(),
                shaping: text::Shaping::Basic,
                max_width: text_width,
            },
        );
    }

    // The time sits under the title, so it has to clear however many lines
    // the title took rather than a fixed one-line offset.
    let when_top = title_top + lines.len() as f32 * title_line + BLOCK_GAP;
    if when_top + when_line <= bounds.y + bounds.height
        && let Some(when) = time_label(when, when_size, text_width)
    {
        fill_glyphs(
            frame,
            Text {
                content: when,
                position: Point::new(bounds.x + BLOCK_PADDING, when_top),
                color: ink.scale_alpha(0.85),
                size: Pixels(when_size),
                font: Font::DEFAULT,
                align_x: iced::alignment::Horizontal::Left.into(),
                align_y: iced::alignment::Vertical::Top,
                line_height: text::LineHeight::default(),
                shaping: text::Shaping::Basic,
                max_width: text_width,
            },
        );
    }
}

/// The time to write under a title, if there is one that fits.
///
/// A block too narrow for the whole range keeps its start time, which is the
/// half that says when the event is.
fn time_label(when: &str, size: f32, width: f32) -> Option<String> {
    if text_width_of(when, size) <= width {
        return Some(when.to_owned());
    }

    when.split_once(" – ")
        .filter(|(start, _)| text_width_of(start, size) <= width)
        .map(|(start, _)| start.to_owned())
}

/// The gap above a block's first line of text, and how many lines fit below it.
///
/// The gap shrinks on a short block, so that a line of text still fits inside
/// it rather than hanging over the edge.
fn text_room(height: f32, size: f32) -> (f32, usize) {
    let line = line_height(size);
    let top = (height - line).clamp(0.0, BLOCK_TOP);

    (top, ((height - top) / line).floor().max(0.0) as usize)
}

/// The lines of a title that fit in `room` lines of `width`.
///
/// A title that needs more lines than the block has room for is cut short and
/// marked with an ellipsis, so that it stays inside the block.
fn fit_lines(title: &str, size: f32, width: f32, room: usize) -> Vec<String> {
    if room == 0 {
        return Vec::new();
    }

    let mut lines = wrap_title(title, size, width);

    if lines.len() > room {
        lines.truncate(room);

        if let Some(last) = lines.last_mut() {
            *last = cut_line(last, size, width);
        }
    }

    lines
}

/// Breaks a title into the lines it takes inside a block `width` wide.
///
/// Words are broken at the character level when they are too wide to sit on a
/// line of their own, so that no line reaches past the edge of the block.
fn wrap_title(title: &str, size: f32, width: f32) -> Vec<String> {
    // A title that fits on one line is the common case, and it costs a single
    // measurement.
    if !title.contains('\n') && text_width_of(title, size) <= width {
        return vec![title.to_owned()];
    }

    let mut lines = Vec::new();

    for source in title.split('\n') {
        let mut line = String::new();

        for word in source.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };

            if text_width_of(&candidate, size) <= width {
                line = candidate;
                continue;
            }

            // The word does not fit on the line being built, so that line is
            // finished off and the word goes on one of its own.
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }

            if text_width_of(word, size) <= width {
                line = word.to_owned();
                continue;
            }

            // The word is too wide for a line of its own as well, so it is
            // broken up at the character level rather than left to reach past
            // the edge of the block.
            let pieces = break_word(word, size, width);
            let last = pieces.len().saturating_sub(1);

            for (index, piece) in pieces.into_iter().enumerate() {
                if index == last {
                    line = piece;
                } else {
                    lines.push(piece);
                }
            }
        }

        lines.push(line);
    }

    lines
}

/// Breaks a word too wide for a line into the pieces that fit.
fn break_word(word: &str, size: f32, width: f32) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut piece = String::new();

    for character in word.chars() {
        let candidate = format!("{piece}{character}");

        if !piece.is_empty() && text_width_of(&candidate, size) > width {
            pieces.push(std::mem::take(&mut piece));
        }

        piece.push(character);
    }

    if !piece.is_empty() {
        pieces.push(piece);
    }

    pieces
}

/// A line with an ellipsis on the end, cut short enough to fit `width`.
fn cut_line(line: &str, size: f32, width: f32) -> String {
    let characters: Vec<char> = line.chars().collect();

    // The longest run of the line that still leaves room for the ellipsis.
    let (mut low, mut high) = (0usize, characters.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        let candidate: String = characters[..middle].iter().collect();

        if text_width_of(&format!("{candidate}{ELLIPSIS}"), size) <= width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }

    let prefix: String = characters[..low].iter().collect();
    format!("{prefix}{ELLIPSIS}")
}

/// A line as it fits `width`, cut short with an ellipsis when it does not.
fn fit_line(line: &str, size: f32, width: f32) -> String {
    if text_width_of(line, size) <= width {
        line.to_owned()
    } else {
        cut_line(line, size, width)
    }
}

/// The height of one line of text at `size`.
fn line_height(size: f32) -> f32 {
    text::LineHeight::default().to_absolute(Pixels(size)).0
}

/// The width a string takes on a single line.
fn text_width_of(content: &str, size: f32) -> f32 {
    use iced::advanced::text::Paragraph as _;

    paragraph(content, size, MEASURE_WIDTH).min_width()
}

/// A paragraph laid out for measuring.
///
/// The width is wide enough that nothing wraps, so the paragraph measures the
/// text as it is written rather than as it would be broken up.
fn paragraph(content: &str, size: f32, width: f32) -> iced::advanced::graphics::text::Paragraph {
    use iced::advanced::graphics::text::Paragraph;
    use iced::advanced::text::Paragraph as _;

    Paragraph::with_text(iced::advanced::text::Text {
        content,
        bounds: Size::new(width, f32::INFINITY),
        size: Pixels(size),
        line_height: text::LineHeight::default(),
        font: Font::DEFAULT,
        align_x: iced::advanced::text::Alignment::Left,
        align_y: iced::alignment::Vertical::Top,
        shaping: text::Shaping::Basic,
        wrapping: text::Wrapping::None,
    })
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
    hash = hash
        .wrapping_mul(31)
        .wrapping_add(u64::from(day_start_hour));
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
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(block.color.r.to_bits() as u64);
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(block.color.g.to_bits() as u64);
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(block.color.b.to_bits() as u64);
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(block.color.a.to_bits() as u64);
        // An unconfirmed write changes the shade, so it has to retrace too.
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(block.pending));
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(u64::from(block.time_block));
        for (id, title, minutes) in &block.tasks {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(*minutes));
            for byte in id.bytes().chain(title.bytes()) {
                hash = hash.wrapping_mul(31).wrapping_add(u64::from(byte));
            }
        }
        for byte in block.title.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
        for byte in block.when.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
    }

    hash
}

/// The part of the grid's cache key that comes from the travel boxes.
fn travel_key(travel: &[TravelBox]) -> u64 {
    let mut hash = 0u64;

    for trip in travel {
        hash = hash.wrapping_mul(31).wrapping_add(trip.anchor_min as u64);
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(trip.below));
        hash = hash.wrapping_mul(31).wrapping_add(u64::from(trip.minutes));
        hash = hash.wrapping_mul(31).wrapping_add(trip.column as u64);
        hash = hash.wrapping_mul(31).wrapping_add(trip.columns as u64);
        hash = hash
            .wrapping_mul(31)
            .wrapping_add(u64::from(trip.time_block));
        for byte in trip.label.bytes() {
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
    fn hiding_google_tasks_preserves_task_rows_inside_time_blocks() {
        use crate::timeblocking::{self as tb, Metadata};
        let mut state = State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        );
        let mut snapshot = calendar::sample_snapshot();
        let mut block = snapshot
            .events
            .iter()
            .find(|event| event.id == "standup")
            .unwrap()
            .clone();
        block.start = block.start.with_hour(1).unwrap().with_minute(0).unwrap();
        block.end = block.start + chrono::Duration::hours(1);
        state.config.calendar.time_block_calendar = Some(block.calendar_id.clone());
        let mut task = block.clone();
        task.id = "task:planned".into();
        task.calendar_id = "tasks:list".into();
        task.title = "Assigned task".into();
        task.all_day = true;
        task.details.description = format!(
            "tt-timeblocking: {}",
            serde_json::to_string(&Metadata {
                block: Some(tb::block_key(&block)),
                minutes: 30,
                ..Metadata::default()
            })
            .unwrap()
        );
        let mut standalone = task.clone();
        standalone.id = "task:standalone".into();
        standalone.details.description = "DUE_TIME: 03:00".into();
        calendar::deadline::place(&mut standalone, 30);
        snapshot.events = vec![block, task, standalone];
        state.calendar = calendar::Store::with_snapshot(snapshot);
        let size = Size::new(600.0, 1440.0);
        let metrics = Metrics::of(size);
        for show in [true, false, true] {
            let _ = crate::app::update(&mut state, Message::ShowTasksToggled(show));
            assert_eq!(
                state
                    .calendar
                    .timed_events(&state.config.calendar)
                    .any(|event| event.id == "task:standalone"),
                show
            );
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                size,
                day_grid(&state, &metrics, Local::now()),
            );
            // The assigned task row occupies the right half of the 1am block.
            ui.point_at(Point::new(400.0, state.calendar.hour_height + 10.0));
            ui.simulate([
                iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ]);
            assert!(
                ui.into_messages().any(|message| matches!(message,
                Message::EventOpened(id) if id == "task:planned")),
                "assigned task must remain visible and clickable with show_tasks={show}"
            );
        }
    }

    #[test]
    fn time_block_view_and_edit_buttons_dispatch_different_actions() {
        for view in [true, false] {
            let mut canvas = canvas_with_one_block();
            canvas.blocks[0].time_block = true;
            let bounds = canvas.block_rect(&canvas.blocks[0], 240.0);
            let (view_rect, edit_rect) = block_actions(bounds);
            assert!(view_rect.x < edit_rect.x);
            let element: Element<'_, Message> =
                canvas_widget(canvas).width(240).height(1440).into();
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                Size::new(240.0, 1440.0),
                element,
            );
            ui.point_at(if view {
                view_rect.center()
            } else {
                edit_rect.center()
            });
            ui.simulate([
                iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ]);
            let messages: Vec<_> = ui.into_messages().collect();
            if view {
                assert!(messages.iter().any(|m| matches!(m,Message::TimeBlocking(crate::timeblocking::Action::View(id)) if id == "standup")));
                assert!(
                    !messages
                        .iter()
                        .any(|m| matches!(m, Message::EventOpened(_)))
                );
            } else {
                assert!(
                    messages
                        .iter()
                        .any(|m| matches!(m,Message::EventOpened(id) if id == "standup"))
                );
            }
        }
    }

    #[test]
    fn a_task_released_over_a_time_block_assigns_it_instead_of_creating_an_event() {
        let mut canvas = canvas_with_one_block();
        canvas.blocks[0].time_block = true;
        canvas.external_drag = true;
        let element: Element<'_, Message> = canvas_widget(canvas).width(240).height(1440).into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            Size::new(240.0, 1440.0),
            element,
        );
        ui.point_at(Point::new(90.0, 550.0));
        ui.simulate([iced::Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left,
        ))]);
        let messages: Vec<_> = ui.into_messages().collect();
        assert!(messages.iter().any(|m| matches!(m, Message::TimeBlocking(crate::timeblocking::Action::DropBlock(id)) if id == "standup")));
        assert!(
            !messages
                .iter()
                .any(|m| matches!(m, Message::EventDraftStarted { .. }))
        );
    }

    #[test]
    fn task_rows_drag_from_the_title_and_estimate_and_show_grab_cursors() {
        for fraction in [0.1, 0.5, 0.95] {
            let mut canvas = canvas_with_one_block();
            canvas.blocks[0].time_block = true;
            canvas.blocks[0].tasks = vec![("task".into(), "Task title".into(), 30)];
            let bounds = Rectangle::new(Point::ORIGIN, Size::new(600.0, 1440.0));
            let rect = canvas.block_rect(&canvas.blocks[0], bounds.width);
            let start = Point::new(
                rect.x + rect.width + 4.0 + fraction * rect.width,
                rect.y + 10.0,
            );
            let cursor = mouse::Cursor::Available(start);
            assert_eq!(
                canvas::Program::mouse_interaction(&canvas, &DrawState::default(), bounds, cursor),
                mouse::Interaction::Grab
            );
            canvas.external_drag = true;
            assert_eq!(
                canvas::Program::mouse_interaction(&canvas, &DrawState::default(), bounds, cursor),
                mouse::Interaction::Grabbing
            );
            canvas.external_drag = false;
            let element: Element<'_, Message> =
                canvas_widget(canvas).width(600).height(1440).into();
            let mut ui = iced_test::Simulator::with_size(
                iced_test::core::Settings::default(),
                bounds.size(),
                element,
            );
            ui.point_at(start);
            ui.simulate([iced::Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            ))]);
            let position = Point::new(start.x, start.y + 10.0);
            ui.point_at(position);
            ui.simulate([iced::Event::Mouse(mouse::Event::CursorMoved { position })]);
            ui.simulate([iced::Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left,
            ))]);
            let messages: Vec<_> = ui.into_messages().collect();
            assert!(messages.iter().any(|m| matches!(m, Message::TimeBlocking(crate::timeblocking::Action::Drag(id)) if id == "task")));
            assert!(
                !messages
                    .iter()
                    .any(|m| matches!(m, Message::EventOpened(_)))
            );
        }
    }

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
        let next_1am = next
            .and_hms_opt(1, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .single()
            .unwrap();
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
                tasks: Vec::new(),
                time_block: false,
            }],
            travel: Vec::new(),
            needle: None,
            alarms: Vec::new(),
            alarm_mode: false,
            hour_height: 60.0,
            gutter: GUTTER,
            compact: false,
            day_start_hour: 0,
            day: chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            can_create: true,
            create_color: Color::WHITE,
            external_drag: false,
        }
    }

    /// A canvas showing `day`, with its alarm mode on or off, for the alarm
    /// tests. The pointer is a pixel row per minute because the zoom is 60.
    fn alarm_canvas(day: chrono::NaiveDate, alarm_mode: bool) -> DayCanvas {
        let mut canvas = canvas_with_one_block();
        canvas.day = day;
        canvas.alarm_mode = alarm_mode;
        canvas
    }

    fn today() -> chrono::NaiveDate {
        Local::now().date_naive()
    }

    fn click_at(canvas: DayCanvas, point: Point) -> Vec<Message> {
        let element: Element<'_, Message> = canvas_widget(canvas).width(600).height(1440).into();
        let mut ui = iced_test::Simulator::with_size(
            iced_test::core::Settings::default(),
            Size::new(600.0, 1440.0),
            element,
        );
        ui.point_at(point);
        ui.simulate([
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ]);
        ui.into_messages().collect()
    }

    /// A trip of `minutes` hanging off the top of the 9:00 standup.
    fn trip_before_standup(minutes: u32) -> TravelBox {
        TravelBox {
            leg: crate::travel::Leg { from: 1, to: 2 },
            between: ("Start of day".into(), "Standup".into()),
            label: "Walk".into(),
            anchor_min: 9 * 60,
            below: false,
            minutes,
            column: 0,
            columns: 1,
            time_block: false,
        }
    }

    #[test]
    fn a_travel_box_sits_just_before_its_event_as_tall_as_the_trip() {
        let mut canvas = canvas_with_one_block();
        canvas.travel = vec![trip_before_standup(25)];

        // The grid is a pixel a minute, so 25 minutes ends where 9:00 starts.
        let rect = canvas.travel_rect(&canvas.travel[0], 600.0);
        assert_eq!((rect.y, rect.height), (9.0 * 60.0 - 25.0, 25.0));
        let event = canvas.block_rect(&canvas.blocks[0], 600.0);
        assert_eq!((rect.x, rect.width), (event.x, event.width));

        // A trip too short to hold a line of text is still tall enough to read.
        let short = trip_before_standup(2);
        assert_eq!(canvas.travel_rect(&short, 600.0).height, TRAVEL_MIN_HEIGHT);

        // The trip home hangs off the end of the last event instead.
        let home = TravelBox {
            anchor_min: 9 * 60 + 30,
            below: true,
            ..trip_before_standup(20)
        };
        let rect = canvas.travel_rect(&home, 600.0);
        assert_eq!((rect.y, rect.height), (9.0 * 60.0 + 30.0, 20.0));
    }

    #[test]
    fn clicking_a_travel_box_opens_the_trip_and_not_the_event() {
        let mut canvas = canvas_with_one_block();
        canvas.travel = vec![trip_before_standup(25)];

        let messages = click_at(canvas, Point::new(200.0, 9.0 * 60.0 - 10.0));
        assert!(
            messages.iter().any(|m| matches!(
                m,
                Message::Travel(crate::travel::Action::OpenLeg(leg, Some((from, to))))
                    if *leg == (crate::travel::Leg { from: 1, to: 2 })
                        && from == "Start of day"
                        && to == "Standup"
            )),
            "got {messages:?}"
        );
        assert!(
            !messages
                .iter()
                .any(|m| matches!(m, Message::EventOpened(_)))
        );

        // The event itself still opens where the box is not.
        let mut canvas = canvas_with_one_block();
        canvas.travel = vec![trip_before_standup(25)];
        let messages = click_at(canvas, Point::new(200.0, 9.0 * 60.0 + 15.0));
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Message::EventOpened(id) if id == "standup"))
        );
    }

    #[test]
    fn the_trips_of_a_day_become_boxes_on_the_events_they_lead_to() {
        use crate::config::Config;

        let mut state = State::new(Config::default(), crate::theme::Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
        let travel = &mut state.config.travel;
        let home = travel.add_place("Home", "1 Elm St", "g-home", 0.0, 0.0);
        let office = travel.add_place("Office", "2 Oak St", "g-office", 0.0, 1.0);
        travel.start = Some(home);
        travel.end = Some(home);
        travel.api_key = "key".into();
        for event in &mut state.calendar.events {
            if matches!(event.id.as_str(), "standup" | "review") {
                event.details.location = "Office".into();
            }
        }
        let route = crate::travel::Route {
            seconds: 1500,
            meters: 2100,
        };
        state.travel.routes.insert(
            crate::travel::RouteKey {
                from: home,
                to: office,
                mode: crate::config::TravelMode::Walk,
            },
            crate::travel::RouteState::Ready(route),
        );

        let at = |id: &str, start, end| Block {
            id: id.into(),
            title: id.into(),
            when: String::new(),
            start_min: start,
            end_min: end,
            column: 0,
            columns: 1,
            color: Color::WHITE,
            editable: true,
            pending: false,
            tasks: Vec::new(),
            time_block: false,
        };
        let blocks = [at("standup", 540, 570), at("review", 840, 900)];

        let found = travel_boxes(&state, &blocks, state.calendar.day);

        // Home to the standup, then the review to home. The two meetings are
        // at the same place, and there is nothing between them to travel for.
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(
            (found[0].anchor_min, found[0].below, found[0].minutes),
            (540, false, 25)
        );
        assert_eq!(found[0].label, "Walk · 25 min · 2.1 km");
        assert_eq!(
            found[0].between,
            ("Start of day".to_owned(), "Standup".to_owned())
        );
        assert_eq!((found[1].anchor_min, found[1].below), (900, true));
        assert_eq!(found[1].between.1, "End of day");
        assert!(found[1].label.contains("calculating"), "{}", found[1].label);

        // The cache key moves when a trip does, so the grid redraws.
        assert_ne!(travel_key(&found), travel_key(&[]));
    }

    #[test]
    fn a_click_in_alarm_mode_sets_the_alarm_at_the_minute_under_the_pointer() {
        let tomorrow = today().succ_opt().unwrap();
        let canvas = alarm_canvas(tomorrow, true);
        let expected = canvas.time_at(10 * 60 + 7);

        let messages = click_at(canvas, Point::new(200.0, 10.0 * 60.0 + 7.0));

        assert!(
            messages.iter().any(|m| matches!(
                m,
                Message::Alarms(alarms::Action::Place(at)) if *at == expected
            )),
            "got {messages:?}"
        );
    }

    #[test]
    fn a_click_in_alarm_mode_never_reaches_the_events_underneath() {
        let tomorrow = today().succ_opt().unwrap();

        // The standup sits at 9:00-9:30, and this lands in the middle of it.
        let messages = click_at(
            alarm_canvas(tomorrow, true),
            Point::new(200.0, 9.0 * 60.0 + 15.0),
        );

        assert!(messages.iter().any(|m| matches!(m, Message::Alarms(_))));
        assert!(
            !messages.iter().any(|m| matches!(
                m,
                Message::EventOpened(_) | Message::EventDraftStarted { .. }
            )),
            "got {messages:?}"
        );
    }

    #[test]
    fn a_click_in_alarm_mode_on_a_past_day_sets_nothing() {
        let yesterday = today().pred_opt().unwrap();

        let messages = click_at(alarm_canvas(yesterday, true), Point::new(200.0, 600.0));

        assert!(
            !messages.iter().any(|m| matches!(m, Message::Alarms(_))),
            "got {messages:?}"
        );
    }

    #[test]
    fn the_pointer_says_whether_an_alarm_can_go_where_it_is() {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(600.0, 1440.0));
        let cursor = mouse::Cursor::Available(Point::new(200.0, 600.0));
        let interaction = |day| {
            canvas::Program::mouse_interaction(
                &alarm_canvas(day, true),
                &DrawState::default(),
                bounds,
                cursor,
            )
        };

        assert_eq!(
            interaction(today().succ_opt().unwrap()),
            mouse::Interaction::Crosshair
        );
        assert_eq!(
            interaction(today().pred_opt().unwrap()),
            mouse::Interaction::NotAllowed
        );
    }

    #[test]
    fn an_alarm_needle_opens_when_its_dot_or_label_is_clicked() {
        let tomorrow = today().succ_opt().unwrap();
        let mut canvas = alarm_canvas(tomorrow, false);
        canvas.alarms = vec![AlarmMark {
            id: 7,
            minutes: 12 * 60,
            label: "12:00".into(),
        }];
        let pill = canvas.alarm_pill(&canvas.alarms[0], 600.0);

        for point in [Point::new(canvas.gutter, 12.0 * 60.0), pill.center()] {
            let messages = click_at(canvas_clone(&canvas), point);

            assert!(
                messages
                    .iter()
                    .any(|m| matches!(m, Message::Alarms(alarms::Action::Open(7)))),
                "{point:?} gave {messages:?}"
            );
        }
    }

    #[test]
    fn an_alarm_needle_crossing_an_event_leaves_the_event_clickable() {
        let tomorrow = today().succ_opt().unwrap();
        let mut canvas = alarm_canvas(tomorrow, false);
        // Through the middle of the 9:00-9:30 standup.
        canvas.alarms = vec![AlarmMark {
            id: 7,
            minutes: 9 * 60 + 15,
            label: "09:15".into(),
        }];

        let messages = click_at(canvas, Point::new(120.0, 9.0 * 60.0 + 15.0));

        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Message::EventOpened(id) if id == "standup")),
            "got {messages:?}"
        );
        assert!(!messages.iter().any(|m| matches!(m, Message::Alarms(_))));
    }

    #[test]
    fn the_needle_line_opens_an_alarm_over_empty_time() {
        let tomorrow = today().succ_opt().unwrap();
        let mut canvas = alarm_canvas(tomorrow, false);
        canvas.alarms = vec![AlarmMark {
            id: 3,
            minutes: 15 * 60,
            label: "15:00".into(),
        }];

        let near = Point::new(150.0, 15.0 * 60.0 + 3.0);
        let far = Point::new(150.0, 15.0 * 60.0 + 20.0);

        assert_eq!(canvas.alarm_at(near, 600.0), Some(3));
        assert_eq!(canvas.alarm_at(far, 600.0), None);
    }

    fn canvas_clone(canvas: &DayCanvas) -> DayCanvas {
        DayCanvas {
            blocks: canvas.blocks.clone(),
            alarms: canvas.alarms.clone(),
            ..alarm_canvas(canvas.day, canvas.alarm_mode)
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
            tasks: Vec::new(),
            time_block: false,
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
        let one_line = wrap_title("Standup", 12.0, 200.0);
        let two_lines = wrap_title("Programming Team Meeting", 12.0, 90.0);

        assert_eq!(one_line.len(), 1);
        assert!(
            two_lines.len() > one_line.len(),
            "a wrapped title should take more lines than a single one"
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
        assert_eq!(
            at(rect.y + rect.height - 1.0),
            Some((0, DragMode::ResizeEnd))
        );
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
    fn a_click_on_empty_space_gets_a_default_length() {
        let canvas = canvas_with_one_block();
        // 9:00 is 540px down at 60px/hour; the press and release are the same
        // pixel, so this is a click rather than a drag.
        let create = CreateDrag {
            anchor_min: canvas.snap_minute(540.0),
            head_min: canvas.snap_minute(540.0),
        };

        assert_eq!(canvas.create_range(&create), (9 * 60, 10 * 60));
    }

    #[test]
    fn a_drag_on_empty_space_spans_what_was_drawn() {
        let canvas = canvas_with_one_block();
        let create = CreateDrag {
            anchor_min: canvas.snap_minute(540.0),
            head_min: canvas.snap_minute(660.0),
        };

        assert_eq!(canvas.create_range(&create), (9 * 60, 11 * 60));
    }

    #[test]
    fn a_drag_upwards_still_spans_forwards() {
        let canvas = canvas_with_one_block();
        let create = CreateDrag {
            anchor_min: canvas.snap_minute(660.0),
            head_min: canvas.snap_minute(540.0),
        };

        assert_eq!(canvas.create_range(&create), (9 * 60, 11 * 60));
    }

    #[test]
    fn a_new_event_never_runs_off_the_end_of_the_day() {
        let canvas = canvas_with_one_block();
        // A click at the very foot of the day still fits a full default span.
        let create = CreateDrag {
            anchor_min: canvas.snap_minute(24.0 * 60.0),
            head_min: canvas.snap_minute(24.0 * 60.0),
        };
        let (start, end) = canvas.create_range(&create);

        assert!(end <= DAY_MINUTES);
        assert!(end - start >= MIN_DURATION);
    }

    #[test]
    fn the_new_event_ends_snap_to_the_quarter_hour() {
        let canvas = canvas_with_one_block();
        // 9:07 lands on 9:00, 9:08 on 9:15.
        assert_eq!(canvas.snap_minute(9.0 * 60.0 + 7.0), 9 * 60);
        assert_eq!(canvas.snap_minute(9.0 * 60.0 + 8.0), 9 * 60 + 15);
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

    #[test]
    fn a_block_only_holds_as_many_lines_as_it_has_room_for() {
        // A one-hour block at the default zoom is 48 pixels tall, which holds
        // two lines of a 12 pixel title.
        let (top, room) = text_room(48.0, 12.0);
        assert_eq!(room, 2);
        assert!(top + room as f32 * line_height(12.0) <= 48.0);

        // The shortest block still holds a single line, and the gap above it
        // shrinks so that the line fits inside.
        let (top, room) = text_room(16.0, 12.0);
        assert_eq!(room, 1);
        assert!(top + room as f32 * line_height(12.0) <= 16.0);

        // A block with no room at all holds nothing.
        let (_, room) = text_room(4.0, 12.0);
        assert_eq!(room, 0);
    }

    #[test]
    fn a_title_that_fits_stays_on_one_line() {
        let lines = wrap_title("Standup", 12.0, 200.0);

        assert_eq!(lines, vec!["Standup".to_owned()]);
    }

    #[test]
    fn every_wrapped_line_stays_inside_the_block() {
        // A title with a line break of its own, a word too long for the block
        // and a word that only just fits.
        let title = "17-4 HW10/22 (Thu)2pm: L18\nReading.Inaka vs Tokai-->Changed\nfrom the L18 department store";
        let width = 120.0;

        for line in wrap_title(title, 12.0, width) {
            assert!(
                text_width_of(&line, 12.0) <= width,
                "{line:?} is wider than the block"
            );
        }
    }

    #[test]
    fn a_word_too_long_for_the_block_is_broken_up() {
        let word = "Supercalifragilisticexpialidociousandthensomemoreletters";
        let width = 120.0;

        let lines = wrap_title(word, 12.0, width);

        assert!(lines.len() > 1, "the word should have been broken up");
        for line in &lines {
            assert!(text_width_of(line, 12.0) <= width, "{line:?} is too wide");
        }
    }

    #[test]
    fn a_title_too_long_for_the_block_is_cut_short_with_an_ellipsis() {
        let title = "17-4 HW10/22 (Thu)2pm: L18 Reading.Inaka vs Tokai-->Changed from the L18 department store";

        let lines = fit_lines(title, 12.0, 120.0, 2);

        assert_eq!(lines.len(), 2);
        assert!(lines.last().expect("a last line").ends_with(ELLIPSIS));
        for line in &lines {
            assert!(text_width_of(line, 12.0) <= 120.0, "{line:?} is too wide");
        }
    }

    #[test]
    fn a_title_with_no_room_is_not_drawn_at_all() {
        let lines = fit_lines("Standup", 12.0, 120.0, 0);

        assert!(lines.is_empty());
    }

    #[test]
    fn a_title_that_fits_is_left_alone() {
        let title = "Standup";

        let lines = fit_lines(title, 12.0, 200.0, 3);

        assert_eq!(lines, vec!["Standup".to_owned()]);
    }

    #[test]
    fn a_line_that_does_not_fit_is_cut_with_an_ellipsis() {
        let line = fit_line("01:00 – 02:00", 11.0, 40.0);

        assert!(line.ends_with(ELLIPSIS));
        assert!(text_width_of(&line, 11.0) <= 40.0);
    }

    #[test]
    fn a_narrow_block_keeps_the_start_of_the_time() {
        // The whole range fits.
        assert_eq!(
            time_label("09:15 – 11:00", 11.0, 200.0),
            Some("09:15 – 11:00".to_owned())
        );

        // Too narrow for the range, but the start time still fits.
        assert_eq!(
            time_label("09:15 – 11:00", 11.0, 40.0),
            Some("09:15".to_owned())
        );

        // Too narrow for either, so there is no time at all.
        assert_eq!(time_label("09:15 – 11:00", 11.0, 12.0), None);
    }
}
