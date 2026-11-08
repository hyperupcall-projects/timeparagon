//! The strip above the day and week grids, and the all-day row.

use super::*;
use iced::widget::column;

/// The top of the calendar: the month picker and the way into the calendar
/// menu.
///
/// In the day view the picker is labelled with the day in view; in the week
/// view it is labelled with the range on screen, and the picker is the way to
/// move through weeks. Every button sits at the left. The settings are reached
/// through the menu rather than from here, because a narrow window has room
/// for few buttons.
pub(super) fn header<'a>(
    state: &'a State,
    metrics: &Metrics,
    weekly: bool,
) -> Element<'a, Message> {
    let today = calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
    let (label, shows_today) = if weekly {
        let days = state.config.calendar.week_dates(state.week_anchor);
        let first = *days.first().unwrap();
        let last = *days.last().unwrap();
        let label = if metrics.breakpoint.is_compact() {
            format!("{} – {}", first.format("%-m/%-d"), last.format("%-m/%-d"))
        } else {
            format!(
                "{} – {}",
                calendar::month_day_label(first),
                calendar::month_day_label(last)
            )
        };
        (label, days.contains(&today))
    } else {
        (
            calendar::month_day_label(state.calendar.day),
            state.calendar.day == today,
        )
    };

    let month_open = if weekly {
        state.overview_month_open
    } else {
        state.calendar.month_open
    };
    let month = button(text(format!("{label} ▾")).size(metrics.label()).center())
        .on_press(Message::Calendar(CalendarMessage::CalendarMonthToggled(
            if weekly {
                MonthPicker::Overview
            } else {
                MonthPicker::Panel
            },
        )))
        .style(if shows_today {
            crate::ui::controls::subtle_primary
        } else if month_open {
            crate::ui::controls::primary
        } else {
            crate::ui::controls::secondary
        });

    // Alarm setting mode turns the day view into a place to click a time, so
    // the button that enters it is lit for as long as it lasts.
    let set_alarm = button(text("Set Alarm").size(metrics.label()).center())
        .on_press(Message::Alarms(crate::alarms::Action::ModeToggled))
        .style(if state.alarms.mode {
            crate::ui::controls::primary
        } else {
            crate::ui::controls::secondary
        });

    let menu = button(text("Menu").size(metrics.label()).center())
        .on_press(Message::Calendar(CalendarMessage::CalendarMenuToggled))
        .style(if state.calendar.menu_open {
            crate::ui::controls::primary
        } else {
            crate::ui::controls::secondary
        });

    let search = search_button(state, metrics);

    if header_lines(metrics, weekly) == 1 {
        row![month, set_alarm, search, menu, Space::new().width(Fill)]
            .spacing(metrics.spacing)
            .align_y(Alignment::Center)
            .into()
    } else {
        column![
            row![month, set_alarm, Space::new().width(Fill)]
                .spacing(metrics.spacing)
                .align_y(Alignment::Center),
            row![search, menu, Space::new().width(Fill)]
                .spacing(metrics.spacing)
                .align_y(Alignment::Center),
        ]
        .spacing(metrics.spacing)
        .into()
    }
}

/// The narrowest a week view can be and still hold the range picker, Set
/// Alarm, Search and Menu on one line.
pub(super) const SINGLE_LINE_HEADER: f32 = 420.0;
/// The narrowest a calendar can be and still hold all four buttons on one line.
pub(super) const TWO_LINE_HEADER: f32 = 320.0;

/// How many lines the header takes. The week view's picker carries a whole
/// range, so its buttons split onto two lines sooner than the day view's.
pub(super) fn header_lines(metrics: &Metrics, weekly: bool) -> u8 {
    let single = if weekly {
        SINGLE_LINE_HEADER
    } else {
        TWO_LINE_HEADER
    };
    if metrics.size.width >= single { 1 } else { 2 }
}

/// A square button for stepping a day or a month at a time.
pub(super) fn nav_button<'a>(
    label: &'a str,
    message: Message,
    metrics: &Metrics,
) -> Element<'a, Message> {
    button(text(label).size(metrics.label()).center())
        .width(Length::Fixed(metrics.icon_button()))
        .height(Length::Fixed(metrics.icon_button()))
        .padding(0.0)
        .on_press(message)
        .style(crate::ui::controls::secondary)
        .into()
}

/// The vertical space the header row takes, so an overlay can start below
/// it rather than over it.
pub(super) fn header_offset(state: &State, metrics: &Metrics) -> f32 {
    let lines = f32::from(header_lines(metrics, state.range_view_active()));

    metrics.padding + (metrics.icon_button() + metrics.spacing) * lines
}

/// Keep the heading's position stable when alarm setting mode changes.
pub(super) fn heading_with_alarm_hint<'a>(
    state: &'a State,
    metrics: &Metrics,
    weekly: bool,
) -> Element<'a, Message> {
    // Loading shows at the foot of the panel, where it moves nothing.
    let mut hint = column![];
    if state.alarms.mode {
        hint = hint.push(
            text("Click a time to set an alarm. Esc cancels.")
                .size(metrics.label() - 1.0)
                .color(colors::ALARM),
        );
    }
    column![header(state, metrics, weekly), hint].into()
}

/// The foot of the calendar while an alarm is ringing: the way to silence it.
///
/// An alarm can go off with any view on screen, so this stays until it has
/// been dismissed. With nothing ringing there is no foot at all.
pub(super) fn dismiss_bar<'a>(state: &'a State, metrics: &Metrics) -> Option<Element<'a, Message>> {
    state.alarm_active.then(|| {
        button(text("Dismiss").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .height(metrics.icon_button())
            .on_press(Message::System(SystemMessage::Dismiss))
            .style(crate::ui::controls::danger)
            .into()
    })
}

pub(super) fn all_day_row<'a>(state: &'a State, metrics: &Metrics) -> Option<Element<'a, Message>> {
    all_day_row_on(state, metrics, state.calendar.day, false)
}

/// A day task needs a slot unless its linked block is present on that day.
pub(super) fn task_needs_slot(state: &State, event: &Event, day: NaiveDate) -> bool {
    crate::timeblocking::needs_slot(event, state.calendar.events_on(day))
}

pub(super) fn all_day_row_on<'a>(
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
    events.retain(|event| {
        !event.is_task()
            || task_needs_slot(state, event, day)
            || crate::timeblocking::overdue(event)
    });
    // Counted before the search narrows the list: this is a tally of the day,
    // and a search does not slot anything in.
    let unslotted = events.iter().filter(|event| event.is_task()).count();
    events.retain(|event| {
        state
            .calendar
            .search_shows(event, day, &state.config.calendar)
    });
    let chips: Vec<Element<'a, Message>> = events
        .into_iter()
        .filter(|event| weekly || !event.is_task())
        .map(|event| {
            if event.is_task() {
                weekly_task_chip(state, event, metrics)
            } else {
                all_day_chip(state, event, metrics)
            }
        })
        .collect();

    if weekly && chips.is_empty() {
        return None;
    }

    let mut content = column![].spacing(4.0).width(Fill);
    if !weekly {
        let overdue = crate::inbox::overdue_tasks(state).len();
        let inbox = crate::inbox::tasks(state).len() + state.inbox.invitations.len();
        let mut summary = text(format!(
            "{overdue} overdue.  {inbox} in inbox.  {unslotted} not slotted in"
        ))
        .size(metrics.label());
        if overdue > 0 || inbox > 0 || unslotted > 0 {
            summary = summary
                .font(Font {
                    weight: Weight::Bold,
                    ..Font::DEFAULT
                })
                .style(text::danger);
        }
        content = content.push(summary);
    }
    content = content.extend(chips);

    let dragging = state.timeblocking.dragging.is_some();
    Some(
        mouse_area(
            container(crate::ui::controls::scrollable(content))
                .max_height(180)
                .style(move |theme: &Theme| drop_zone_style(theme, dragging)),
        )
        .on_release(Message::TimeBlocking(crate::timeblocking::Action::DropDay))
        .into(),
    )
}

/// Outlines a place a task in the air can be dropped, in the theme's primary
/// color, so the destination is clear while dragging.
pub(super) fn drop_zone_style(theme: &Theme, active: bool) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(
            if active {
                palette.primary.weak.color
            } else {
                Color::TRANSPARENT
            }
            .into(),
        ),
        border: iced::Border {
            color: if active {
                palette.primary.strong.color
            } else {
                Color::TRANSPARENT
            },
            width: if active { 1.0 } else { 0.0 },
            radius: crate::ui::controls::RADIUS_ROW.into(),
        },
        ..container::Style::default()
    }
}

pub(crate) fn weekly_task_chip<'a>(
    state: &'a State,
    task: &Event,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let hidden = state.calendar.search.hides_details(task);
    let mut check = button(
        text(if task.completed && !hidden {
            "●"
        } else {
            "○"
        })
        .size(metrics.label()),
    )
    .padding(2)
    .style(crate::ui::controls::ghost);
    if state.timeblocking.busy.is_empty() && !hidden {
        check = check.on_press(Message::Task(TaskMessage::TaskToggled(task.id.clone())));
    }
    let label = if hidden {
        PRIVATE_TITLE.to_owned()
    } else {
        crate::timeblocking::task_label(task)
    };
    let title = button(text(label).size(metrics.label()))
        .padding([3, 2])
        .width(Fill)
        .on_press(Message::Event(EventMessage::EventOpened(task.id.clone())))
        .style(crate::ui::controls::ghost);
    super::super::task_drag::task_drag(
        container(row![check, title].spacing(2).align_y(Alignment::Center))
            .width(Fill)
            .style(|theme: &Theme| container::Style {
                background: Some(theme.extended_palette().background.weakest.color.into()),
                border: iced::Border {
                    color: theme.extended_palette().background.strong.color,
                    width: 1.0,
                    radius: crate::ui::controls::RADIUS_ROW.into(),
                },
                ..container::Style::default()
            }),
        &task.id,
        state.timeblocking.dragging.as_deref(),
    )
}

pub(super) fn all_day_chip<'a>(
    state: &State,
    event: &Event,
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
    } else if event.completed {
        format!("✓ {}", event.title)
    } else {
        event.title.clone()
    };

    let id = event.id.clone();
    button(text(title).size(metrics.label()).color(ink))
        .padding([4.0, 8.0])
        .width(Fill)
        .on_press(Message::Event(EventMessage::EventOpened(id)))
        .style(move |_theme, status| event_chip_style(color, ink, status))
        .into()
}

pub(super) fn event_chip_style(color: Color, ink: Color, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(if hovered { lift(color) } else { color }.into()),
        text_color: ink,
        border: iced::Border {
            color: if hovered {
                Color::BLACK.scale_alpha(0.55)
            } else {
                timeslay_core::theme::EVENT_EDGE
            },
            width: if hovered { 2.0 } else { 1.0 },
            radius: EVENT_RADIUS.into(),
        },
        shadow: iced::Shadow {
            color: Color::BLACK.scale_alpha(if hovered { 0.2 } else { 0.12 }),
            offset: iced::Vector::new(0.0, if hovered { 3.0 } else { 1.0 }),
            blur_radius: if hovered { 8.0 } else { 3.0 },
        },
        ..button::Style::default()
    }
}

/// Pulls a color toward white so a hovered chip reads as raised.
pub(super) fn lift(color: Color) -> Color {
    Color {
        r: color.r + (1.0 - color.r) * 0.28,
        g: color.g + (1.0 - color.g) * 0.28,
        b: color.b + (1.0 - color.b) * 0.28,
        a: color.a,
    }
}

/// A swatch that can be tapped, used for a task list's color and in the
/// picker itself. `selected` draws a brighter ring so the current color reads.
pub(crate) fn swatch_button<'a>(
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

pub(super) fn swatch_style(color: Color, selected: bool, status: button::Status) -> button::Style {
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
            radius: crate::ui::controls::RADIUS_SMALL.into(),
        },
        ..button::Style::default()
    }
}
