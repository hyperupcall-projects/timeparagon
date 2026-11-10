//! The chronological schedule, with a date gutter and compact event rows.
use super::*;
use iced::widget::{column, rule};

pub fn schedule_view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let now = crate::clock::now();
    let today = now.date_naive();
    let compact = metrics.size.width < 600.0;
    let mut days = column![].width(Fill);
    for day in calendar::schedule::dates(state.week_anchor) {
        let deadlines = state
            .calendar
            .deadline_markers_on(day, &state.config.calendar);
        let mut events: Vec<_> = state
            .calendar
            .all_day_events_on(day, &state.config.calendar)
            .chain(state.calendar.timed_events_on(day, &state.config.calendar))
            .chain(deadlines.iter())
            .filter(|event| calendar::schedule::occurs_on(event, day))
            .filter(|event| {
                state.habits.matches_calendar_habit(event)
                    && state
                        .calendar
                        .search_shows(event, day, &state.config.calendar)
            })
            .collect();
        events.sort_by_key(|event| (!event.all_day, event.start, &event.title));
        events.dedup_by_key(|event| &event.id);
        let mut rows = column![].width(Fill).spacing(2);
        let mut needle = false;
        for event in events.iter() {
            if day == today && !event.all_day && event.start > now && !needle {
                rows = rows.push(rule::horizontal(1).style(|theme: &Theme| rule::Style {
                    color: theme.extended_palette().danger.base.color,
                    snap: true,
                    radius: 0.0.into(),
                    fill_mode: rule::FillMode::Full,
                }));
                needle = true;
            }
            let hidden = state.calendar.search.hides_details(event);
            let past = !event.all_day && event.end <= now;
            let mut color = if hidden {
                PRIVATE_COLOR
            } else {
                event.display_color(&state.config.calendar)
            };
            if past {
                color.a = 0.3;
            }
            let dot =
                container(Space::new().width(10).height(10)).style(move |_| container::Style {
                    background: Some(color.into()),
                    border: iced::Border {
                        radius: 5.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                });
            let time = text(calendar::schedule::time_label(
                event,
                state.config.appearance.time_format,
            ))
            .size(metrics.label());
            let title = text(if hidden {
                PRIVATE_TITLE.into()
            } else {
                calendar::schedule::title(event, day)
            })
            .size(metrics.label());
            let time = if past {
                time.style(text::secondary)
            } else {
                time
            };
            let title = if past {
                title.style(text::secondary)
            } else {
                title
            };
            let content: Element<'_, Message> = if compact {
                row![dot, column![time, title].spacing(3).width(Fill)]
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .into()
            } else {
                row![dot, time.width(190), title.width(Fill)]
                    .spacing(20)
                    .align_y(Alignment::Center)
                    .into()
            };
            rows = rows.push(crate::ui::controls::locked(
                button(content)
                    .padding([9, 12])
                    .width(Fill)
                    .style(button::text)
                    .on_press(Message::Calendar(CalendarMessage::WeekDayAction(
                        day,
                        Box::new(Message::Event(EventMessage::EventOpened(event.id.clone()))),
                    ))),
                event.details.sending,
            ));
        }
        if day == today && !needle {
            rows = rows.push(rule::horizontal(1).style(|theme: &Theme| rule::Style {
                color: theme.extended_palette().danger.base.color,
                snap: true,
                radius: 0.0.into(),
                fill_mode: rule::FillMode::Full,
            }));
        }
        if events.is_empty() {
            rows = rows.push(
                text("No events")
                    .size(metrics.label())
                    .style(text::secondary),
            );
        }
        let is_today = day == today;
        let number = container(text(day.format("%-d").to_string()).size(26))
            .width(44)
            .height(44)
            .center_x(Fill)
            .center_y(Fill)
            .style(move |theme: &Theme| {
                if is_today {
                    let primary = theme.extended_palette().primary.base;
                    container::Style {
                        background: Some(primary.color.into()),
                        text_color: Some(primary.text),
                        border: iced::Border {
                            radius: 22.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                } else {
                    container::Style::default()
                }
            });
        let label = text(day.format("%b, %a").to_string().to_uppercase()).size(12);
        let date_content: Element<'_, Message> = if compact {
            column![number, label]
                .spacing(4)
                .align_x(Alignment::Center)
                .into()
        } else {
            row![number, label]
                .spacing(8)
                .align_y(Alignment::Center)
                .into()
        };
        let date = button(date_content)
            .padding([6, 0])
            .width(if compact { 78 } else { 150 })
            .style(button::text)
            .on_press(Message::Calendar(CalendarMessage::CalendarDayOpened(day)));
        days = days
            .push(row![date, rows].spacing(8).padding([12, 0]))
            .push(rule::horizontal(1));
    }
    let heading = range_heading(
        state,
        format!("{} ▾", state.week_anchor.format("%b %-d, %Y")),
        metrics,
    );
    let navigation = row![
        nav_button(
            "‹",
            Message::Calendar(CalendarMessage::CalendarDayShift(-30)),
            metrics
        ),
        text("Schedule").size(metrics.label()),
        nav_button(
            "›",
            Message::Calendar(CalendarMessage::CalendarDayShift(30)),
            metrics
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let list = crate::ui::controls::scrollable(days)
        .id(iced::widget::Id::new(RANGE_SCROLL))
        .height(Fill)
        .width(Fill);
    range_layers(
        state,
        metrics,
        column![heading, navigation, list]
            .spacing(metrics.spacing)
            .padding(metrics.padding)
            .height(Fill)
            .into(),
    )
}
