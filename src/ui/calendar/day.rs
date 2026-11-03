//! The day grid and scrolling it.

use super::*;

pub(super) fn day_grid<'a>(
    state: &'a State,
    metrics: &Metrics,
    now: DateTime<Local>,
) -> Element<'a, Message> {
    container(
        crate::ui::controls::scrollable(day_canvas(state, metrics, now, state.calendar.day, false))
            .id(iced::widget::Id::new(DAY_SCROLL))
            .on_scroll(grid_scrolled)
            .height(Fill)
            .width(Fill),
    )
    .height(Fill)
    .width(Fill)
    .padding(1)
    .style(crate::ui::controls::card_surface)
    .into()
}

pub(super) fn day_canvas<'a>(
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

    let timed: Vec<_> = state
        .calendar
        .timed_events_on(day, &state.config.calendar)
        .into_iter()
        .filter(|event| {
            state
                .calendar
                .search_shows(event, day, &state.config.calendar)
        })
        .collect();
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
        .map(|((event, span), slot)| {
            let hidden = state.calendar.search.hides_details(event);
            let time_block =
                crate::timeblocking::is_block(event, &state.config.calendar) && !hidden;
            Block {
                id: event.id.clone(),
                title: if hidden {
                    PRIVATE_TITLE.to_owned()
                } else if event.details.sleep.is_some() {
                    calendar::sleep::title(event)
                } else if event.is_task() {
                    crate::timeblocking::task_label(event)
                } else {
                    event.title.clone()
                },
                when: if hidden {
                    String::new()
                } else if event.details.sleep.is_some() {
                    calendar::sleep::time_label(event, state.config.appearance.time_format)
                } else {
                    calendar::time_format::event(event, state.config.appearance.time_format)
                },
                start_min: span.start_min,
                end_min: span.end_min,
                column: slot.column,
                columns: slot.columns,
                column_span: slot.column_span,
                color: if hidden {
                    PRIVATE_COLOR
                } else {
                    event.display_color(&state.config.calendar)
                },
                editable: !hidden
                    && event.details.sleep.is_none()
                    && (!event.is_task() || event.details.scheduled.is_some()),
                pending: state.calendar.pending_time(&event.id).is_some()
                    || state.timeblocking.busy.contains_task(&event.id),
                time_block,
                habit: (event.details.habit && !hidden)
                    .then(|| state.habits.habit_of(event))
                    .flatten()
                    .map(|id| (id.to_owned(), event.start.date_naive())),
                // A block owns its task rows independently of standalone task visibility.
                tasks: if time_block {
                    let key = crate::timeblocking::block_key(event);
                    let mut tasks: Vec<_> = state
                        .calendar
                        .events_on(day)
                        .iter()
                        .filter(|e| {
                            e.is_task()
                                && crate::timeblocking::metadata(e).block.as_deref()
                                    == Some(key.as_str())
                                && state.calendar.search_shows_in_block(e, event)
                        })
                        .collect();
                    tasks.sort_by_key(|e| (crate::timeblocking::metadata(e).order, e.id.clone()));
                    tasks
                        .into_iter()
                        .map(|e| {
                            if state.calendar.search.hides_details(e) {
                                (e.id.clone(), PRIVATE_TITLE.to_owned(), 0)
                            } else {
                                (
                                    e.id.clone(),
                                    crate::timeblocking::task_label(e),
                                    crate::timeblocking::metadata(e).minutes,
                                )
                            }
                        })
                        .collect()
                } else {
                    Vec::new()
                },
            }
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
                label: alarm_label(alarm, state.config.appearance.time_format),
            })
        })
        .collect();

    // Trips join up the whole day's events, so they stop making sense once a
    // search has taken some of those events away, and they name the events
    // and places that hidden details are meant to keep out of sight.
    let travel = if state.travel.show && !state.calendar.search.is_modified() {
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
    let selection = state.calendar.draft.as_ref().and_then(|draft| {
        let start = minutes_into_day(draft.start, day, offset);
        let end = minutes_into_day(draft.end, day, offset).min(DAY_MINUTES);
        (0..DAY_MINUTES)
            .contains(&start)
            .then_some((start, end.max(start + 1)))
    });

    canvas_widget(DayCanvas {
        time_format: state.config.appearance.time_format,
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
        selection,
        external_drag: state.timeblocking.dragging.is_some(),
        dragged_task: state.timeblocking.dragging.clone(),
    })
    .width(Fill)
    .height(Length::Fixed(height))
    .into()
}

/// Minutes in a day, the height of the grid in minutes.
pub(super) const DAY_MINUTES: i32 = 24 * 60;

/// What an alarm's needle says beside it: its time, and its label if it has
/// one.
pub(super) fn alarm_label(alarm: &alarms::Alarm, time_format: TimeFormat) -> String {
    let time = calendar::time_format::clock(alarm.at, time_format);
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
pub(super) fn minutes_into_day(when: DateTime<Local>, day: chrono::NaiveDate, offset: i32) -> i32 {
    let days = (when.date_naive() - day).num_days() as i32;
    days * DAY_MINUTES + calendar::minutes_from_midnight(when) - offset
}

pub(super) const DAY_SCROLL: &str = "cal-day";
pub(super) const WEEK_DAY_SCROLL: &str = "cal-week-day";
pub(super) const WEEK_SCROLL: &str = "cal-week";
/// The grid of the month and two-week views. Only one of them shows at a time.
pub(super) const RANGE_SCROLL: &str = "cal-range";

/// Reports a grid's scroll, so a zoom knows which row is in the middle of it.
pub(super) fn grid_scrolled(viewport: scrollable::Viewport) -> Message {
    Message::Calendar(CalendarMessage::CalendarGridScrolled(
        viewport.absolute_offset().y,
        viewport.bounds().height,
    ))
}

pub fn scroll_week_to_start() -> iced::Task<Message> {
    iced::widget::operation::scroll_to(
        iced::widget::Id::new(WEEK_SCROLL),
        iced::widget::operation::AbsoluteOffset { x: 0.0, y: 0.0 },
    )
}

/// Scrolls the day grid so the needle sits a little below the top, the way
/// Google Calendar opens on "now" rather than the start of the day.
pub fn scroll_to_now(hour_height: f32, day_end_minutes: i32) -> iced::Task<Message> {
    scroll_now(DAY_SCROLL, hour_height, day_end_minutes)
}

pub fn scroll_week_to_now(hour_height: f32, day_end_minutes: i32) -> iced::Task<Message> {
    scroll_now(WEEK_DAY_SCROLL, hour_height, day_end_minutes)
}

pub(super) fn scroll_now(
    id: &'static str,
    hour_height: f32,
    day_end_minutes: i32,
) -> iced::Task<Message> {
    let minutes = (calendar::minutes_from_midnight(crate::clock::now()) - day_end_minutes)
        .rem_euclid(DAY_MINUTES);
    let y = ((minutes - 90) as f32 * (hour_height / 60.0)).max(0.0);

    iced::widget::operation::scroll_to(
        iced::widget::Id::new(id),
        iced::widget::operation::AbsoluteOffset { x: 0.0, y },
    )
}

/// Scrolls the grid of a calendar view by a pixel amount.
///
/// Used after a zoom to keep a row still: the grid grows or shrinks around that
/// row, so the scroll follows it by the same delta.
pub fn scroll_by(view: crate::app::CalendarView, delta: f32) -> iced::Task<Message> {
    use crate::app::CalendarView;
    let id = match view {
        CalendarView::Day => DAY_SCROLL,
        CalendarView::Week => WEEK_DAY_SCROLL,
        CalendarView::TwoWeek | CalendarView::Month => RANGE_SCROLL,
    };
    iced::widget::operation::scroll_by(
        iced::widget::Id::new(id),
        iced::widget::operation::AbsoluteOffset { x: 0.0, y: delta },
    )
}

/// The scroll adjustment that keeps a row still while the
/// hour height changes from `before` to `after`.
///
/// `anchor` is the row measured from the top of the grid. The grid grows by
/// `after / before`, so that row slides down by `anchor * (ratio - 1)` and the
/// scroll has to follow it by the same amount for it to stay put on screen.
pub fn zoom_scroll_delta(anchor: f32, before: f32, after: f32) -> f32 {
    if !before.is_finite() || !after.is_finite() || before <= 0.0 {
        return 0.0;
    }

    anchor * (after / before - 1.0)
}
