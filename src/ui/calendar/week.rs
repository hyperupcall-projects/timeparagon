//! The week grid.

use super::*;
use iced::widget::column;

pub(super) fn weekly_column_width(settings: &crate::config::CalendarSettings) -> f32 {
    let width = settings.weekly_day_width() as f32;
    if settings.weekly_task_list {
        width.max(240.0)
    } else {
        width
    }
}

pub fn weekly_view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let board_metrics = Metrics {
        padding: 0.0,
        spacing: 0.0,
        ..*metrics
    };
    let metrics = if state.config.calendar.weekly_task_list {
        &board_metrics
    } else {
        metrics
    };
    let days = state.config.calendar.week_dates(state.week_anchor);
    let width = weekly_column_width(&state.config.calendar);
    let day_metrics = Metrics::of(Size::new(width, metrics.size.height));
    let now = crate::clock::now();
    let all_day_height = if days.iter().any(|&day| {
        if state
            .calendar
            .deadline_markers_on(day, &state.config.calendar)
            .iter()
            .any(|event| event.all_day)
        {
            return true;
        }
        state
            .calendar
            .all_day_events_on(day, &state.config.calendar)
            .any(|event| {
                state
                    .calendar
                    .search_shows(event, day, &state.config.calendar)
            })
    }) {
        (metrics.size.height * 0.13).clamp(35.0, 110.0)
    } else {
        0.0
    };
    let grid: Element<'a, Message> = if state.config.calendar.weekly_task_list {
        super::super::weekly_task_list::view(state, metrics, &days, width)
    } else {
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
                    container(all_day)
                        .height(Length::Fixed(all_day_height))
                        .width(Fill),
                ]
                .width(Length::Fixed(width));
                Element::from(
                    mouse_area(container(content).style(move |theme| {
                        drop_zone_style(theme, state.timeblocking.dragging.is_some())
                    }))
                    .on_release(if state.timeblocking.dragging.is_some() {
                        Message::TimeBlocking(crate::timeblocking::Action::DropDay)
                    } else {
                        Message::Ignored
                    }),
                )
                .map(move |message| {
                    Message::Calendar(CalendarMessage::WeekDayAction(day, Box::new(message)))
                })
            })
            .collect();

        let grids: Vec<Element<'a, Message>> = days
            .iter()
            .map(|&day| {
                Element::from(
                    container(day_canvas(state, &day_metrics, now, day, true))
                        .width(Length::Fixed(width)),
                )
                .map(move |message| {
                    Message::Calendar(CalendarMessage::WeekDayAction(day, Box::new(message)))
                })
            })
            .collect();
        let time_grid = crate::ui::controls::scrollable(row(grids))
            .direction(scrollable::Direction::Vertical(grid_scrollbar()))
            .id(iced::widget::Id::new(WEEK_DAY_SCROLL))
            .on_scroll(grid_scrolled)
            .width(Fill)
            .height(Fill);
        crate::ui::controls::scrollable(
            column![row(headings), time_grid]
                // The day columns keep their fixed widths; reserve additional
                // room for the vertical scrollbar beside the final column.
                .width(Length::Fixed(
                    width * days.len() as f32 + GRID_SCROLLBAR_SPACE,
                ))
                .height(Fill),
        )
        .direction(scrollable::Direction::Horizontal(grid_scrollbar()))
        .id(iced::widget::Id::new(WEEK_SCROLL))
        .on_scroll(|viewport| {
            Message::Calendar(CalendarMessage::WeekScrolled(viewport.absolute_offset().x))
        })
        .width(Fill)
        .height(Fill)
        .into()
    };

    let draft_open = state.calendar.draft.as_ref().is_some_and(|d| d.editor_open);
    let panel_open =
        draft_open || state.calendar.open_event.is_some() || state.timeblocking.block.is_some();
    let mut grid_layers: Vec<Element<'a, Message>> = vec![grid];
    if panel_open && let Some(index) = days.iter().position(|day| *day == state.calendar.day) {
        let viewport = (metrics.size.width - metrics.padding * 2.0).max(1.0);
        let scroll_x = state
            .week_scroll_x
            .clamp(0.0, (width * days.len() as f32 - viewport).max(0.0));
        let x = index as f32 * width - scroll_x;
        let left = x.clamp(0.0, viewport);
        let right = (x + width).clamp(0.0, viewport);
        let dismiss = if draft_open {
            Message::EventDraft(EventDraftMessage::EventDraftCancelled)
        } else if state.calendar.open_event.is_some() {
            Message::Event(EventMessage::EventClosed)
        } else {
            Message::TimeBlocking(crate::timeblocking::Action::Close)
        };
        let shade = |w: f32| {
            mouse_area(
                container(Space::new().width(Fill).height(Fill))
                    .width(Length::Fixed(w))
                    .height(Fill)
                    .style(|_| container::Style {
                        background: Some(crate::ui::controls::SCRIM.into()),
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

        let requested_width = if draft_open || state.calendar.open_event.is_some() {
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
            if draft_open || state.calendar.open_event.is_some() {
                let height = size.height.max(480.0);
                let panel_metrics = Metrics::of(Size::new(panel_width, height));
                crate::ui::controls::scrollable(
                    container(details_pane(state, &panel_metrics)).height(Length::Fixed(height)),
                )
                .height(Fill)
                .into()
            } else {
                let panel_metrics = Metrics::of(Size::new(panel_width, size.height));
                super::super::timeblocking::panel(state, &panel_metrics)
            }
        });
        grid_layers.push(
            row![
                Space::new().width(Length::Fixed(panel_x)),
                container(pane)
                    .width(Length::Fixed(panel_width))
                    .height(Fill)
                    .style(|theme: &Theme| container::Style {
                        background: Some(theme.extended_palette().background.weakest.color.into()),
                        border: iced::Border {
                            color: theme.extended_palette().background.strong.color,
                            width: 1.0,
                            radius: crate::ui::controls::RADIUS_CARD.into(),
                        },
                        shadow: crate::ui::controls::shadow(theme, 0.2, 8.0, 26.0),
                        ..container::Style::default()
                    }),
            ]
            .height(Fill)
            .into(),
        );
    }

    let heading = if state.config.calendar.weekly_task_list {
        super::super::weekly_task_list::toolbar(state, metrics)
    } else {
        heading_with_alarm_hint(state, metrics, true)
    };
    let mut content = column![
        heading,
        iced::widget::Stack::with_children(grid_layers)
            .width(Fill)
            .height(Fill),
    ]
    .spacing(metrics.spacing)
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill);
    if let Some(dismiss) = dismiss_bar(state, metrics) {
        content = content.push(dismiss);
    }
    let overlay = if state
        .calendar
        .draft
        .as_ref()
        .is_some_and(|d| !d.editor_open)
    {
        draft_overlay(state, metrics)
    } else if state.alarms.editor.is_some() {
        super::super::alarms::editor_overlay(state, metrics)
    } else if state.overview_month_open {
        month_overlay(state, MonthPicker::Overview, metrics)
    } else if state.calendar.menu_open {
        menu_overlay(state, metrics)
    } else if state.calendar.search.open {
        search_overlay(state, metrics)
    } else if state.calendar.actions_open {
        actions_overlay(state, metrics, true)
    } else {
        Space::new().width(0.0).height(0.0).into()
    };
    pointer_watch(stack![content, overlay])
}
