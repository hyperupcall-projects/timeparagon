//! The week board: compact paper cards and precise insertion targets.
use chrono::NaiveDate;
use iced::advanced::{Layout, Shell, Widget, layout, overlay, renderer, widget};
use iced::widget::{
    Space, button, column, container, mouse_area, progress_bar, responsive, row, scrollable, text,
};
use iced::{
    Alignment, Color, Element, Event, Fill, Font, Length, Rectangle, Renderer, Shadow, Size, Theme,
    Vector, mouse,
};

use super::layout::Metrics;
use crate::{
    app::{Message, State},
    calendar::{self, Event as CalendarEvent},
    timeblocking::{self as tb, Action},
};

#[derive(Clone, Copy)]
struct Palette {
    board: Color,
    paper: Color,
    ink: Color,
    muted: Color,
    edge: Color,
    badge: Color,
}
fn palette(theme: &Theme) -> Palette {
    let p = theme.extended_palette();
    if p.is_dark {
        Palette {
            board: p.background.weak.color,
            paper: p.background.base.color,
            ink: p.background.base.text,
            muted: p.background.strong.text,
            edge: p.background.strong.color,
            badge: p.background.weak.color,
        }
    } else {
        Palette {
            board: Color::from_rgb8(248, 246, 245),
            paper: Color::WHITE,
            ink: Color::from_rgb8(67, 66, 63),
            muted: Color::from_rgb8(125, 125, 125),
            edge: Color::from_rgb8(235, 234, 233),
            badge: Color::from_rgb8(240, 240, 240),
        }
    }
}
fn card_style(theme: &Theme) -> container::Style {
    let p = palette(theme);
    container::Style {
        background: Some(p.paper.into()),
        text_color: Some(p.ink),
        border: iced::Border {
            color: p.edge,
            width: 1.0,
            radius: 3.0.into(),
        },
        shadow: Shadow {
            color: Color::BLACK.scale_alpha(0.12),
            offset: Vector::new(0.0, 1.0),
            blur_radius: 1.0,
        },
        ..Default::default()
    }
}
fn duration(minutes: i64) -> String {
    format!("{}:{:02}", minutes.max(0) / 60, minutes.max(0) % 60)
}
fn duration_badge<'a>(minutes: i64) -> Element<'a, Message> {
    container(text(duration(minutes)).size(10))
        .padding([2, 4])
        .style(|theme: &Theme| {
            let p = palette(theme);
            container::Style {
                background: Some(p.badge.into()),
                text_color: Some(p.muted),
                border: iced::Border {
                    radius: 2.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        })
        .into()
}

/// The reference's quiet toolbar, with week navigation available between its ends.
pub(super) fn toolbar<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let today = calendar::day_at(chrono::Local::now(), state.config.calendar.day_start_hour());
    let control = |label: String, action: Message| {
        button(text(label).size(11))
            .padding([3, 6])
            .on_press(action)
            .style(|theme: &Theme, status| {
                let p = palette(theme);
                button::Style {
                    background: Some(
                        if matches!(status, button::Status::Hovered) {
                            p.badge
                        } else {
                            p.paper
                        }
                        .into(),
                    ),
                    text_color: p.muted,
                    border: iced::Border {
                        color: p.edge,
                        width: 1.0,
                        radius: 3.0.into(),
                    },
                    ..Default::default()
                }
            })
    };
    let shift = i64::from(state.config.calendar.weekly_days());
    let mut bar = row![
        control("▦ Today".into(), Message::CalendarDaySelected(today)),
        Space::new().width(Fill),
        control("‹".into(), Message::CalendarDayShift(-shift))
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    if metrics.size.width >= 700.0 {
        let days = calendar::week_dates(state.week_anchor, state.config.calendar.weekly_days());
        bar = bar.push(control(
            format!(
                "{} – {}",
                days[0].format("%b %-d"),
                days.last().unwrap().format("%b %-d")
            ),
            Message::CalendarMonthToggled,
        ));
    }
    bar = bar
        .push(control("›".into(), Message::CalendarDayShift(shift)))
        .push(Space::new().width(Fill))
        .push(control(
            "▦ Board".into(),
            Message::CalendarTaskListToggled(false),
        ));
    container(bar)
        .padding([9, 8])
        .width(Fill)
        .style(|theme: &Theme| container::Style {
            background: Some(palette(theme).board.into()),
            border: iced::Border {
                color: palette(theme).edge,
                width: 1.0,
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

pub(super) fn view<'a>(
    state: &'a State,
    _metrics: &Metrics,
    days: &[NaiveDate],
    width: f32,
) -> Element<'a, Message> {
    let days = days.to_vec();
    responsive(move |size| {
        container(
            scrollable(row(days
                .iter()
                .map(|&day| day_column(state, day, width, size.height))))
            .direction(scrollable::Direction::Both {
                vertical: scrollable::Scrollbar::new().width(4).scroller_width(4),
                horizontal: scrollable::Scrollbar::new().width(4).scroller_width(4),
            })
            .id(widget::Id::new("cal-week"))
            .on_scroll(|viewport| Message::WeekScrolled(viewport.absolute_offset().x))
            .width(Fill)
            .height(Fill),
        )
        .style(|theme: &Theme| container::Style {
            background: Some(palette(theme).board.into()),
            text_color: Some(palette(theme).ink),
            ..Default::default()
        })
        .width(Fill)
        .height(Fill)
        .into()
    })
    .into()
}

/// Scroll both axes while a lifted card is held near the viewport edges.
pub(super) fn auto_scroll(state: &State) -> iced::Task<Message> {
    if !state.weekly
        || !state.config.calendar.weekly_task_list
        || state.timeblocking.dragging.is_none()
    {
        return iced::Task::none();
    }
    let Some(window) = state.timeblocking.drag_window.or(state.editor_window) else {
        return iced::Task::none();
    };
    let Some(point) = state
        .timeblocking
        .drag_pointer
        .and_then(|p| super::task_drop::range_position(state, window, p))
    else {
        return iced::Task::none();
    };
    let delta = edge_scroll(point, state.editor_window_size);
    if delta == Vector::ZERO {
        return iced::Task::none();
    }
    iced::widget::operation::scroll_by(
        widget::Id::new("cal-week"),
        iced::widget::operation::AbsoluteOffset {
            x: delta.x,
            y: delta.y,
        },
    )
}
fn edge_scroll(point: iced::Point, size: Size) -> Vector {
    if point.y < 40.0 {
        return Vector::ZERO;
    }
    let edge = |position: f32, start: f32, end: f32| {
        if position < start + 36.0 {
            -18.0
        } else if position > end - 36.0 {
            18.0
        } else {
            0.0
        }
    };
    Vector::new(
        edge(point.x, 0.0, size.width),
        edge(point.y, 40.0, size.height),
    )
}

fn day_column<'a>(
    state: &'a State,
    day: NaiveDate,
    width: f32,
    height: f32,
) -> Element<'a, Message> {
    let events = tb::weekly_list_events(state, day);
    let total: i64 = events
        .iter()
        .filter(|e| !e.all_day)
        .map(|e| (e.end - e.start).num_minutes())
        .sum();
    let tasks: Vec<_> = events.iter().filter(|e| e.is_task()).collect();
    let completed = tasks.iter().filter(|e| e.completed).count();
    let progress = completed as f32 / tasks.len().max(1) as f32;
    let heading = column![
        text(day.format("%A").to_string()).size(18).font(Font {
            weight: iced::font::Weight::Bold,
            ..Font::DEFAULT
        }),
        text(day.format("%B %-d").to_string())
            .size(12)
            .style(|theme: &Theme| text::Style {
                color: Some(palette(theme).muted)
            }),
        Space::new().height(4),
        progress_bar(0.0..=1.0, progress)
            .girth(7)
            .style(|theme: &Theme| progress_bar::Style {
                background: palette(theme).edge.into(),
                bar: theme.extended_palette().primary.base.color.into(),
                border: iced::Border {
                    radius: 4.0.into(),
                    ..Default::default()
                }
            }),
    ]
    .spacing(3)
    .padding([8, 8]);
    let add = button(
        row![
            text("＋  Add task").size(12),
            Space::new().width(Fill),
            duration_badge(total)
        ]
        .align_y(Alignment::Center),
    )
    .width(Fill)
    .padding([5, 10])
    .on_press(Message::TimeBlocking(Action::NewWeekTask))
    .style(|theme: &Theme, status| {
        let p = palette(theme);
        button::Style {
            background: Some(
                if matches!(status, button::Status::Hovered) {
                    p.badge
                } else {
                    p.paper
                }
                .into(),
            ),
            text_color: p.muted,
            border: iced::Border {
                color: p.edge,
                width: 1.0,
                radius: 3.0.into(),
            },
            shadow: Shadow {
                color: Color::BLACK.scale_alpha(0.1),
                offset: Vector::new(0.0, 1.0),
                blur_radius: 1.0,
            },
            ..Default::default()
        }
    });
    let first = events.first().map(|e| e.id.clone());
    let mut cards = column![
        heading,
        drop_target(container(add).padding([4, 0]), state, first, false)
    ]
    .width(Fill);
    for event in events {
        cards = cards.push(drop_target(
            container(card(state, event)).padding([4, 0]),
            state,
            Some(event.id.clone()),
            true,
        ));
    }
    cards = cards.push(drop_target(
        Space::new().width(Fill).height(60),
        state,
        None,
        false,
    ));
    let day_view = mouse_area(
        container(row![cards, Space::new().height(height)])
            .padding([8, 8])
            .width(width),
    )
    .on_release(if state.timeblocking.dragging.is_some() {
        Message::TimeBlocking(Action::DropWeekList {
            target: None,
            after: true,
        })
    } else {
        Message::Ignored
    });
    Element::from(day_view).map(move |message| Message::WeekDayAction(day, Box::new(message)))
}

fn card<'a>(state: &'a State, event: &CalendarEvent) -> Element<'a, Message> {
    let task = event.is_task();
    let completed = event.completed;
    let metadata = tb::metadata(event);
    let projects = if task {
        &metadata.projects
    } else {
        &event.details.projects
    };
    let color = projects
        .iter()
        .filter_map(|id| state.config.projects.get(*id).and_then(|p| p.color()))
        .next()
        .unwrap_or_else(|| event.display_color(&state.config.calendar));
    let mut badges = row![].align_y(Alignment::Center);
    if !event.all_day {
        badges = badges.push(
            container(
                text(calendar::time_format::clock(
                    event.start,
                    state.config.appearance.time_format,
                ))
                .size(10)
                .color(Color::WHITE),
            )
            .padding([2, 4])
            .style(move |_| container::Style {
                background: Some(color.into()),
                border: iced::Border {
                    radius: 3.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
    }
    badges = badges.push(Space::new().width(Fill));
    if !event.all_day {
        badges = badges.push(duration_badge((event.end - event.start).num_minutes()));
    }
    let mut body = column![
        badges,
        text(if task {
            tb::task_label(event)
        } else {
            event.title.clone()
        })
        .size(13)
    ]
    .spacing(5)
    .width(Fill);
    // Markdown checklist notes are useful context without opening the editor.
    for line in tb::description(&event.details.description)
        .lines()
        .filter_map(|line| {
            let line = line.trim().strip_prefix("- ").unwrap_or(line.trim());
            line.strip_prefix("[ ] ").map(|s| (false, s)).or_else(|| {
                line.strip_prefix("[x] ")
                    .or_else(|| line.strip_prefix("[X] "))
                    .map(|s| (true, s))
            })
        })
        .take(6)
    {
        body = body.push(
            row![
                text(if line.0 { "☑" } else { "○" }).size(14),
                text(line.1.to_owned()).size(12)
            ]
            .spacing(5),
        );
    }
    let open = button(body)
        .width(Fill)
        .padding([8, 10])
        .on_press(Message::EventOpened(event.id.clone()))
        .style(|theme: &Theme, _| button::Style {
            text_color: palette(theme).ink,
            ..Default::default()
        });
    let mut footer = row![].align_y(Alignment::Center).spacing(4);
    if task {
        let mut check = button(text("✓").size(11).center())
            .width(18)
            .height(18)
            .padding(0)
            .style(move |theme: &Theme, _| {
                let p = palette(theme);
                button::Style {
                    text_color: if completed {
                        theme.extended_palette().primary.base.color
                    } else {
                        p.muted.scale_alpha(0.65)
                    },
                    border: iced::Border {
                        color: p.muted.scale_alpha(0.55),
                        width: 1.0,
                        radius: 9.0.into(),
                    },
                    ..Default::default()
                }
            });
        if state.timeblocking.busy.is_empty() {
            check = check.on_press(Message::TaskToggled(event.id.clone()));
        }
        footer = footer.push(check);
    }
    if let Some(source) = crate::integrations::source_in(&event.details.description) {
        footer = footer.push(super::source_icon::link(&source, 12.0));
    }
    if event.all_day {
        footer = footer.push(text("All day").size(10));
    }
    footer = footer.push(Space::new().width(Fill));
    let labels = if task {
        &metadata.labels
    } else {
        &event.details.labels
    };
    let mut tags = column![].spacing(2);
    for project in projects
        .iter()
        .filter_map(|id| state.config.projects.get(*id))
    {
        let tint = project.color().unwrap_or(color);
        tags = tags.push(
            row![
                text("#").size(13).color(tint),
                text(project.name.clone()).size(11)
            ]
            .spacing(3)
            .align_y(Alignment::Center),
        );
    }
    for label in labels {
        tags = tags.push(
            row![
                text("#").size(13).color(color),
                text(label.clone()).size(11)
            ]
            .spacing(3)
            .align_y(Alignment::Center),
        );
    }
    if projects.is_empty() && labels.is_empty() {
        if let Some(calendar) = state
            .calendar
            .calendars
            .iter()
            .find(|c| c.id == event.calendar_id)
        {
            tags = tags.push(
                row![
                    text("#").size(13).color(color),
                    text(calendar.name.clone()).size(11)
                ]
                .spacing(3)
                .align_y(Alignment::Center),
            );
        }
    }
    footer = footer.push(container(tags).style(|theme: &Theme| container::Style {
        text_color: Some(palette(theme).muted),
        ..Default::default()
    }));
    let content = container(column![
        open,
        container(footer).padding(iced::Padding {
            top: 0.0,
            right: 10.0,
            bottom: 8.0,
            left: 10.0
        })
    ])
    .width(Fill)
    .style(card_style);
    if task {
        super::task_drag::task_drag(content, &event.id, state.timeblocking.dragging.as_deref())
    } else {
        content.into()
    }
}

fn drop_target<'a>(
    content: impl Into<Element<'a, Message>>,
    state: &State,
    target: Option<String>,
    split: bool,
) -> Element<'a, Message> {
    Element::new(DropTarget {
        content: content.into(),
        active: state.timeblocking.dragging.is_some(),
        target,
        split,
    })
}
struct DropTarget<'a> {
    content: Element<'a, Message>,
    active: bool,
    target: Option<String>,
    split: bool,
}
impl DropTarget<'_> {
    fn after(&self, bounds: Rectangle, cursor: mouse::Cursor) -> bool {
        self.split && cursor.position().is_some_and(|p| p.y >= bounds.center_y())
    }
}
impl Widget<Message, Theme, Renderer> for DropTarget<'_> {
    fn children(&self) -> Vec<widget::Tree> {
        vec![widget::Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut widget::Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        operation.traverse(&mut |op| {
            self.content
                .as_widget_mut()
                .operate(&mut tree.children[0], layout, renderer, op)
        });
    }
    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if self.active
            && matches!(
                event,
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
            )
            && cursor.is_over(layout.bounds())
            && cursor.is_over(*viewport)
        {
            shell.publish(Message::TimeBlocking(Action::DropWeekList {
                target: self.target.clone(),
                after: self.after(layout.bounds(), cursor),
            }));
            shell.capture_event();
            return;
        }
        if self.active && matches!(event, Event::Mouse(mouse::Event::CursorMoved { .. })) {
            shell.request_redraw();
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        if self.active && cursor.is_over(layout.bounds()) && cursor.is_over(*viewport) {
            use iced::advanced::Renderer as _;
            let b = layout.bounds();
            renderer.fill_quad(
                renderer::Quad {
                    bounds: Rectangle {
                        x: b.x,
                        y: if self.after(b, cursor) {
                            b.y + b.height - 2.0
                        } else {
                            b.y
                        },
                        width: b.width,
                        height: 2.0,
                    },
                    ..Default::default()
                },
                theme.extended_palette().primary.base.color,
            );
        }
    }
    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.active && cursor.is_over(layout.bounds()) {
            mouse::Interaction::Grabbing
        } else {
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                layout,
                cursor,
                viewport,
                renderer,
            )
        }
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut widget::Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dragging_at_board_edges_scrolls_both_axes_but_toolbar_does_not() {
        let size = Size::new(492.0, 653.0);
        assert_eq!(
            edge_scroll(iced::Point::new(490.0, 650.0), size),
            Vector::new(18.0, 18.0)
        );
        assert_eq!(
            edge_scroll(iced::Point::new(2.0, 42.0), size),
            Vector::new(-18.0, -18.0)
        );
        assert_eq!(
            edge_scroll(iced::Point::new(240.0, 300.0), size),
            Vector::ZERO
        );
        assert_eq!(edge_scroll(iced::Point::new(2.0, 20.0), size), Vector::ZERO);
    }
}
