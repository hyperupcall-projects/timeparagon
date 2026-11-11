//! "View in calendar": items about to be created, drawn over the day or week
//! they land in.
//!
//! Each proposal gets a number, shown both in the list at the top and in a
//! badge on its block, so several new items can be told apart even in a week
//! column too narrow for their titles. New items are drawn in their own color
//! with a dashed outline, the usual mark of something not yet saved, while
//! what is already on the calendar is drawn faintly behind them.

use super::*;
use crate::inbox::preview::{Preview, PreviewAction, Proposal, Span as PreviewSpan};
use iced::widget::canvas::LineDash;
use iced::widget::{column, tooltip};

/// Width of the hour labels at the left of the grid.
const PREVIEW_GUTTER: f32 = 40.0;
const BADGE_RADIUS: f32 = 7.5;

fn act(action: PreviewAction) -> Message {
    Message::Inbox(crate::inbox::Action::Preview(action))
}

#[derive(Debug, Clone)]
struct PreviewBlock {
    title: String,
    when: String,
    start_min: i32,
    end_min: i32,
    column: usize,
    columns: usize,
    column_span: usize,
    color: Color,
    /// The proposal's place in the list, or None for an existing item.
    proposal: Option<usize>,
    focused: bool,
}

/// A read-only hour grid for one day of the preview.
struct PreviewCanvas {
    blocks: Vec<PreviewBlock>,
    hour_height: f32,
    gutter: f32,
    day_start_hour: u32,
    time_format: TimeFormat,
    needle: Option<i32>,
    compact: bool,
}

impl PreviewCanvas {
    fn rect(&self, block: &PreviewBlock, width: f32) -> Rectangle {
        let minute = self.hour_height / 60.0;
        let grid = (width - self.gutter).max(8.0);
        let columns = block.columns.max(1) as f32;
        let column_width = grid / columns;
        Rectangle {
            x: self.gutter + block.column as f32 * column_width + EVENT_GAP / 2.0,
            y: block.start_min as f32 * minute,
            width: (block.column_span.max(1) as f32 * column_width - EVENT_GAP).max(6.0),
            height: ((block.end_min - block.start_min) as f32 * minute).max(16.0),
        }
    }

    /// The proposal under the pointer. Proposals are drawn last, so they are
    /// what a click lands on where they overlap an existing item.
    fn proposal_at(&self, point: Option<Point>, width: f32) -> Option<usize> {
        let point = point?;
        self.blocks
            .iter()
            .rev()
            .filter(|block| block.proposal.is_some())
            .find(|block| self.rect(block, width).contains(point))
            .and_then(|block| block.proposal)
    }
}

impl canvas::Program<Message> for PreviewCanvas {
    /// The proposal the pointer is over.
    type State = Option<usize>;

    fn update(
        &self,
        hovered: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        match event {
            iced::Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                let hit = self.proposal_at(cursor.position_in(bounds), bounds.width);
                (hit != *hovered).then(|| {
                    *hovered = hit;
                    canvas::Action::request_redraw()
                })
            }
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let index = self.proposal_at(cursor.position_in(bounds), bounds.width)?;
                Some(canvas::Action::publish(act(PreviewAction::Focus(index))).and_capture())
            }
            _ => None,
        }
    }

    fn mouse_interaction(
        &self,
        _: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self
            .proposal_at(cursor.position_in(bounds), bounds.width)
            .is_some()
        {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        hovered: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let palette = theme.extended_palette();
        let label_size = if self.compact { 10.0 } else { 11.0 };

        for hour in 0..24 {
            let y = hour as f32 * self.hour_height;
            frame.stroke(
                &Path::line(Point::new(self.gutter, y), Point::new(frame.width(), y)),
                Stroke {
                    style: Style::Solid(timeslay_core::theme::HOUR_LINE),
                    width: 1.0,
                    ..Stroke::default()
                },
            );
            if self.gutter > 0.0 {
                fill_glyphs(
                    &mut frame,
                    Text {
                        content: calendar::time_format::hour(
                            (self.day_start_hour + hour) % 24,
                            self.time_format,
                        ),
                        position: Point::new(self.gutter - 5.0, y + 2.0),
                        color: colors::GUTTER_TEXT,
                        size: Pixels(label_size),
                        align_x: iced::alignment::Horizontal::Right.into(),
                        max_width: self.gutter - 6.0,
                        ..Text::default()
                    },
                );
            }
        }
        // A thin rule between the days of the week.
        frame.stroke(
            &Path::line(
                Point::new(self.gutter, 0.0),
                Point::new(self.gutter, frame.height()),
            ),
            Stroke {
                style: Style::Solid(timeslay_core::theme::HOUR_LINE),
                width: 1.0,
                ..Stroke::default()
            },
        );

        // What is already there, faint and without a shadow, so it reads as
        // context rather than something to act on.
        let faint_fill = palette.background.strong.color.scale_alpha(0.55);
        let faint_ink = palette.background.base.text.scale_alpha(0.6);
        for block in self.blocks.iter().filter(|block| block.proposal.is_none()) {
            let rect = self.rect(block, frame.width());
            let shape = Path::rounded_rectangle(rect.position(), rect.size(), EVENT_RADIUS.into());
            frame.fill(&shape, faint_fill);
            // A sliver of the item's own color keeps calendars recognizable.
            frame.fill(
                &Path::rounded_rectangle(
                    rect.position(),
                    Size::new(3.0_f32.min(rect.width), rect.height),
                    EVENT_RADIUS.into(),
                ),
                block.color.scale_alpha(0.7),
            );
            if rect.height >= 14.0 && rect.width >= 18.0 {
                let mut label = rect;
                label.x += 2.0;
                label.width -= 2.0;
                draw_block_label(
                    &mut frame,
                    label,
                    &block.title,
                    &block.when,
                    faint_ink,
                    11.0,
                    false,
                );
            }
        }

        for block in self.blocks.iter().filter(|block| block.proposal.is_some()) {
            let rect = self.rect(block, frame.width());
            let shape = Path::rounded_rectangle(rect.position(), rect.size(), EVENT_RADIUS.into());
            let hovered = block.proposal == *hovered;
            if block.focused || hovered {
                // A halo in the item's own color marks the one picked in the
                // list, so it can be found among the others.
                let halo = Rectangle {
                    x: rect.x - 3.0,
                    y: rect.y - 3.0,
                    width: rect.width + 6.0,
                    height: rect.height + 6.0,
                };
                frame.stroke(
                    &Path::rounded_rectangle(
                        halo.position(),
                        halo.size(),
                        (EVENT_RADIUS + 3.0).into(),
                    ),
                    Stroke {
                        style: Style::Solid(if block.focused {
                            palette.primary.base.color
                        } else {
                            palette.primary.base.color.scale_alpha(0.5)
                        }),
                        width: 2.0,
                        ..Stroke::default()
                    },
                );
            }
            for (drop, alpha) in timeslay_core::theme::EVENT_DROPS {
                frame.fill(
                    &Path::rounded_rectangle(
                        Point::new(rect.x, rect.y + drop),
                        rect.size(),
                        EVENT_RADIUS.into(),
                    ),
                    Color::BLACK.scale_alpha(alpha * 2.0),
                );
            }
            frame.fill(&shape, block.color);
            let ink = on_color(block.color);
            let inset = Rectangle {
                x: rect.x + 1.5,
                y: rect.y + 1.5,
                width: (rect.width - 3.0).max(1.0),
                height: (rect.height - 3.0).max(1.0),
            };
            frame.stroke(
                &Path::rounded_rectangle(
                    inset.position(),
                    inset.size(),
                    (EVENT_RADIUS - 1.0).into(),
                ),
                Stroke {
                    style: Style::Solid(ink.scale_alpha(0.85)),
                    width: 1.5,
                    line_dash: LineDash {
                        segments: &[4.0, 3.0],
                        offset: 0,
                    },
                    ..Stroke::default()
                },
            );

            let number = block.proposal.unwrap_or_default() + 1;
            let badge_room = BADGE_RADIUS * 2.0 + 4.0;
            let badge = if rect.width >= badge_room + 4.0 {
                Point::new(
                    rect.x + rect.width - BADGE_RADIUS - 3.0,
                    rect.y + BADGE_RADIUS + 3.0,
                )
            } else {
                rect.center()
            };
            let mut label = rect;
            if rect.width >= badge_room + 22.0 {
                label.width -= badge_room;
                if label.height >= 14.0 {
                    draw_block_label(
                        &mut frame,
                        label,
                        &block.title,
                        &block.when,
                        ink,
                        12.0,
                        false,
                    );
                }
            }
            draw_badge(&mut frame, badge, number, ink, block.color);
        }

        if let Some(minutes) = self.needle {
            let y = (minutes as f32 * self.hour_height / 60.0).clamp(0.0, frame.height());
            frame.stroke(
                &Path::line(Point::new(self.gutter, y), Point::new(frame.width(), y)),
                Stroke {
                    style: Style::Solid(colors::NEEDLE),
                    width: 2.0,
                    ..Stroke::default()
                },
            );
        }
        vec![frame.into_geometry()]
    }
}

/// A numbered disc, matching the number beside the item in the list.
fn draw_badge(frame: &mut Frame, center: Point, number: usize, fill: Color, ink: Color) {
    frame.fill(&Path::circle(center, BADGE_RADIUS), fill);
    fill_glyphs(
        frame,
        Text {
            content: number.to_string(),
            position: center,
            color: ink,
            size: Pixels(10.0),
            font: Font {
                weight: Weight::Bold,
                ..Font::DEFAULT
            },
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..Text::default()
        },
    );
}

/// The read-only grid for one day: existing timed items, and the proposals
/// that land on it.
fn preview_canvas<'a>(
    state: &'a State,
    preview: &'a Preview,
    day: NaiveDate,
    gutter: f32,
    compact: bool,
) -> Element<'a, Message> {
    let settings = &state.config.calendar;
    let offset = settings.day_end_minutes();
    let format = state.config.appearance.time_format;
    let span_of = |start: DateTime<Local>, end: DateTime<Local>| {
        layout::clamp_day(
            minutes_into_day(start, day, offset),
            minutes_into_day(end, day, offset),
        )
    };

    let mut found: Vec<(Span, PreviewBlock, usize)> = Vec::new();
    for event in state.calendar.timed_events_on(day, settings) {
        if !state.calendar.search_shows(event, day, settings) {
            continue;
        }
        let Some((start_min, end_min)) = span_of(event.start, event.end) else {
            continue;
        };
        let hidden = state.calendar.search.hides_details(event);
        found.push((
            Span { start_min, end_min },
            PreviewBlock {
                title: if hidden {
                    PRIVATE_TITLE.to_owned()
                } else {
                    event.title.clone()
                },
                when: if hidden {
                    String::new()
                } else {
                    calendar::time_format::event(event, format)
                },
                start_min,
                end_min,
                column: 0,
                columns: 1,
                column_span: 1,
                color: if hidden {
                    PRIVATE_COLOR
                } else {
                    event.display_color(settings)
                },
                proposal: None,
                focused: false,
            },
            1,
        ));
    }
    for (index, proposal) in preview.proposals.iter().enumerate() {
        if proposal.when.as_ref().is_none_or(|when| when.all_day) {
            continue;
        }
        for (start, end) in proposal.occurrences_on(day, state.config.calendar.day_end_minutes()) {
            let Some((start_min, end_min)) = span_of(start, end) else {
                continue;
            };
            found.push((
                Span { start_min, end_min },
                PreviewBlock {
                    title: title_or_untitled(proposal),
                    when: calendar::time_format::range(start, end, format),
                    start_min,
                    end_min,
                    column: 0,
                    columns: 1,
                    column_span: 1,
                    color: proposal.color,
                    proposal: Some(index),
                    focused: preview.focus == Some(index),
                },
                // Proposals take the left of a crowded hour, where they are
                // easiest to see.
                0,
            ));
        }
    }
    let spans: Vec<Span> = found.iter().map(|(span, _, _)| *span).collect();
    let priorities: Vec<usize> = found.iter().map(|(_, _, priority)| *priority).collect();
    let packed = layout::pack_prioritized(&spans, &priorities);
    let blocks = found
        .into_iter()
        .zip(packed)
        .map(|((_, mut block, _), slot)| {
            block.column = slot.column;
            block.columns = slot.columns;
            block.column_span = slot.column_span;
            block
        })
        .collect();

    let now = crate::clock::now();
    let day_start_hour = settings.day_start_hour();
    let needle = (day == calendar::day_at(now, day_start_hour))
        .then(|| minutes_into_day(now, day, offset).clamp(0, DAY_MINUTES));
    let hour_height = state.calendar.hour_height;
    canvas_widget(PreviewCanvas {
        blocks,
        hour_height,
        gutter,
        day_start_hour,
        time_format: format,
        needle,
        compact,
    })
    .width(Fill)
    .height(Length::Fixed(hour_height * 24.0))
    .into()
}

fn title_or_untitled(proposal: &Proposal) -> String {
    if proposal.title.is_empty() {
        format!("(New {})", proposal.kind_label().to_lowercase())
    } else {
        proposal.title.clone()
    }
}

/// A small numbered disc for the list and the all-day strip.
fn number_chip<'a>(number: usize, color: Color, size: f32) -> Element<'a, Message> {
    let ink = on_color(color);
    container(
        text(number.to_string())
            .size(size - 2.0)
            .font(Font {
                weight: Weight::Bold,
                ..Font::DEFAULT
            })
            .color(ink)
            .center(),
    )
    .width(Length::Fixed(size + 4.0))
    .height(Length::Fixed(size + 4.0))
    .center(Length::Fixed(size + 4.0))
    .style(move |_| container::Style {
        background: Some(color.into()),
        border: iced::Border {
            radius: ((size + 4.0) / 2.0).into(),
            ..iced::Border::default()
        },
        ..container::Style::default()
    })
    .into()
}

/// The all-day items of one day: existing ones faint, proposals in color.
fn all_day_strip<'a>(
    state: &'a State,
    preview: &'a Preview,
    day: NaiveDate,
    narrow: bool,
    metrics: &Metrics,
) -> Option<Element<'a, Message>> {
    let settings = &state.config.calendar;
    let size = metrics.label() - 1.0;
    let mut chips: Vec<Element<'a, Message>> = Vec::new();
    for (index, proposal) in preview.proposals.iter().enumerate() {
        if !proposal.when.as_ref().is_some_and(|when| when.all_day)
            || proposal
                .occurrences_on(day, state.config.calendar.day_end_minutes())
                .is_empty()
        {
            continue;
        }
        let color = proposal.color;
        let focused = preview.focus == Some(index);
        let mut content = row![number_chip(index + 1, on_color(color), size)]
            .spacing(4)
            .align_y(Alignment::Center);
        if !narrow {
            content = content.push(
                text(title_or_untitled(proposal))
                    .size(size)
                    .color(on_color(color))
                    .wrapping(text::Wrapping::None),
            );
        }
        chips.push(
            button(content)
                .padding([2, 4])
                .width(Fill)
                .on_press(act(PreviewAction::Focus(index)))
                .style(move |theme: &Theme, status| button::Style {
                    background: Some(
                        if matches!(status, button::Status::Hovered) {
                            lighten(color)
                        } else {
                            color
                        }
                        .into(),
                    ),
                    text_color: on_color(color),
                    border: iced::Border {
                        color: if focused {
                            theme.extended_palette().primary.base.color
                        } else {
                            on_color(color).scale_alpha(0.6)
                        },
                        width: if focused { 2.0 } else { 1.0 },
                        radius: crate::ui::controls::RADIUS_SMALL.into(),
                    },
                    ..button::Style::default()
                })
                .into(),
        );
    }
    let existing: Vec<_> = state
        .calendar
        .all_day_events_on(day, settings)
        .filter(|event| !event.is_task() && state.calendar.search_shows(event, day, settings))
        .collect();
    if !narrow {
        for event in existing.iter().take(2) {
            let title = if state.calendar.search.hides_details(event) {
                PRIVATE_TITLE.to_owned()
            } else {
                event.title.clone()
            };
            chips.push(
                container(
                    text(title)
                        .size(size)
                        .style(text::secondary)
                        .wrapping(text::Wrapping::None),
                )
                .padding([1, 6])
                .width(Fill)
                .clip(true)
                .style(crate::ui::controls::card_surface)
                .into(),
            );
        }
    }
    let more = if narrow {
        existing.len()
    } else {
        existing.len().saturating_sub(2)
    };
    if more > 0 {
        chips.push(
            text(if narrow {
                format!("+{more}")
            } else {
                format!("+{more} more")
            })
            .size(size - 1.0)
            .style(text::secondary)
            .into(),
        );
    }
    (!chips.is_empty()).then(|| column(chips).spacing(2).width(Fill).into())
}

/// One row of the list of proposals.
fn proposal_row<'a>(
    state: &'a State,
    preview: &'a Preview,
    index: usize,
    proposal: &'a Proposal,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let format = state.config.appearance.time_format;
    let focused = preview.focus == Some(index);
    let detail = match &proposal.when {
        None => "No date yet, so it won't show on the calendar".to_owned(),
        Some(when) => {
            let day = when.start.format("%a, %b %-d");
            let mut detail = if when.all_day {
                let last = (when.end - chrono::Duration::days(1)).date_naive();
                if last > when.start.date_naive() {
                    format!("{day} – {} · All day", last.format("%a, %b %-d"))
                } else {
                    format!("{day} · All day")
                }
            } else {
                format!(
                    "{day} · {}",
                    calendar::time_format::range(when.start, when.end, format)
                )
            };
            if let Some(rule) = &when.repeat {
                detail.push_str(" · ");
                detail.push_str(&rule.summary(when.start.date_naive()));
            }
            detail
        }
    };
    let content = row![
        number_chip(index + 1, proposal.color, metrics.label()),
        column![
            text(title_or_untitled(proposal))
                .size(metrics.label())
                .font(Font {
                    weight: Weight::Semibold,
                    ..Font::DEFAULT
                }),
            text(format!("{} · {detail}", proposal.kind_label()))
                .size(metrics.label() - 2.0)
                .style(text::secondary),
        ]
        .spacing(1)
        .width(Fill),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    let mut row_button = button(content)
        .padding([4, 6])
        .width(Fill)
        .style(crate::ui::controls::nav(focused));
    if proposal.when.is_some() {
        row_button = row_button.on_press(act(PreviewAction::Focus(index)));
    }
    row_button.into()
}

fn range_label(preview: &Preview, days: &[NaiveDate]) -> String {
    match preview.span {
        PreviewSpan::Day => preview.anchor.format("%A, %B %-d").to_string(),
        PreviewSpan::Week => {
            let (first, last) = (days[0], days[days.len() - 1]);
            if first.month() == last.month() {
                format!("{} – {}", first.format("%b %-d"), last.format("%-d, %Y"))
            } else {
                format!("{} – {}", first.format("%b %-d"), last.format("%b %-d, %Y"))
            }
        }
    }
}

/// The whole preview: a header with the Day/Week switch, the numbered list,
/// the grid, and the actions at the foot.
pub fn preview_view<'a>(
    state: &'a State,
    preview: &'a Preview,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let metrics = *metrics;
    let label = metrics.label();
    let days = crate::inbox::preview::visible_days(preview, &state.config.calendar);

    let segment = |name: &'static str, span: PreviewSpan| {
        button(text(name).size(label).center())
            .padding([3, 12])
            .style(crate::ui::controls::nav(preview.span == span))
            .on_press(act(PreviewAction::Span(span)))
    };
    let back = button(text("‹ Back to edit").size(label))
        .padding([3, 6])
        .style(crate::ui::controls::ghost)
        .on_press(act(PreviewAction::Close));
    let header = row![
        back,
        Space::new().width(Fill),
        row![
            segment("Day", PreviewSpan::Day),
            segment("Week", PreviewSpan::Week)
        ]
        .spacing(2),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let step = |glyph: &'static str, steps: i32| {
        button(text(glyph).size(label + 2.0).center())
            .padding([0, 10])
            .style(crate::ui::controls::ghost)
            .on_press(act(PreviewAction::Shift(steps)))
    };
    let navigation = row![
        step("‹", -1),
        text(range_label(preview, &days))
            .size(label + 1.0)
            .font(Font {
                weight: Weight::Semibold,
                ..Font::DEFAULT
            })
            .center()
            .width(Fill),
        step("›", 1),
    ]
    .align_y(Alignment::Center);

    let count = preview.proposals.len();
    let heading = text(match count {
        0 => "Nothing to preview".to_owned(),
        1 => "1 new item".to_owned(),
        n => format!("{n} new items"),
    })
    .size(label - 1.0)
    .style(text::secondary);
    let rows: Vec<Element<'a, Message>> = preview
        .proposals
        .iter()
        .enumerate()
        .map(|(index, proposal)| proposal_row(state, preview, index, proposal, &metrics))
        .collect();
    let rows_height = (count as f32 * (label * 2.6 + 8.0)).min(metrics.size.height * 0.28);
    let list = column![
        heading,
        crate::ui::controls::scrollable(column(rows).spacing(2).width(Fill))
            .height(Length::Fixed(rows_height.max(1.0))),
    ]
    .spacing(4);

    let grid = responsive(move |size| {
        let compact = size.width < 300.0;
        match preview.span {
            PreviewSpan::Day => {
                let day = preview.anchor;
                let mut body = column![].spacing(4).width(Fill).height(Fill);
                if let Some(strip) = all_day_strip(state, preview, day, false, &metrics) {
                    body = body.push(
                        row![Space::new().width(Length::Fixed(PREVIEW_GUTTER)), strip].spacing(2),
                    );
                }
                body.push(
                    container(
                        crate::ui::controls::scrollable(preview_canvas(
                            state,
                            preview,
                            day,
                            PREVIEW_GUTTER,
                            compact,
                        ))
                        .id(iced::widget::Id::new(crate::inbox::preview::GRID_SCROLL))
                        .height(Fill),
                    )
                    .padding(1)
                    .height(Fill)
                    .style(crate::ui::controls::card_surface),
                )
                .into()
            }
            PreviewSpan::Week => {
                // The first column carries the hour labels, so the others line
                // up with it by being that much narrower.
                let column_width =
                    ((size.width - PREVIEW_GUTTER - 4.0) / days.len() as f32).max(24.0);
                let narrow = column_width < 90.0;
                let today =
                    calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
                let headings: Vec<Element<'a, Message>> = days
                    .iter()
                    .map(|&day| {
                        let new_here = preview
                            .proposals
                            .iter()
                            .filter(|proposal| {
                                !proposal
                                    .occurrences_on(day, state.config.calendar.day_end_minutes())
                                    .is_empty()
                            })
                            .count();
                        let name = if narrow {
                            day.format("%a\n%-d").to_string()
                        } else {
                            day.format("%a %-d").to_string()
                        };
                        let mut heading = column![
                            text(name)
                                .size(label - 1.0)
                                .center()
                                .width(Fill)
                                .font(Font {
                                    weight: if day == today {
                                        Weight::Bold
                                    } else {
                                        Weight::Normal
                                    },
                                    ..Font::DEFAULT
                                })
                        ]
                        .align_x(Alignment::Center)
                        .width(Fill);
                        // A dot per day with something new on it, so a busy
                        // week shows at a glance where the new items fall.
                        heading = heading.push(
                            text(if new_here > 0 {
                                "•".repeat(new_here.min(4))
                            } else {
                                " ".to_owned()
                            })
                            .size(label)
                            .style(text::primary)
                            .center()
                            .width(Fill),
                        );
                        let mut column_body = column![tooltip(
                            button(heading)
                                .padding(2)
                                .width(Fill)
                                .style(crate::ui::controls::ghost)
                                .on_press(act(PreviewAction::ShowDay(day))),
                            container(text(format!("Show {}", day.format("%A"))).size(label - 1.0))
                                .padding([2, 6])
                                .style(crate::ui::controls::card_surface),
                            tooltip::Position::Bottom,
                        )]
                        .width(Fill);
                        if let Some(strip) = all_day_strip(state, preview, day, narrow, &metrics) {
                            column_body = column_body.push(strip);
                        }
                        container(column_body)
                            .width(Length::Fixed(column_width))
                            .padding([0, 1])
                            .into()
                    })
                    .collect();
                let canvases: Vec<Element<'a, Message>> = days
                    .iter()
                    .enumerate()
                    .map(|(index, &day)| {
                        let gutter = if index == 0 { PREVIEW_GUTTER } else { 0.0 };
                        container(preview_canvas(state, preview, day, gutter, true))
                            .width(Length::Fixed(column_width + gutter))
                            .into()
                    })
                    .collect();
                column![
                    row![
                        Space::new().width(Length::Fixed(PREVIEW_GUTTER)),
                        row(headings)
                    ],
                    container(
                        crate::ui::controls::scrollable(row(canvases))
                            .id(iced::widget::Id::new(crate::inbox::preview::GRID_SCROLL))
                            .height(Fill),
                    )
                    .padding(1)
                    .height(Fill)
                    .style(crate::ui::controls::card_surface),
                ]
                .spacing(4)
                .width(Fill)
                .height(Fill)
                .into()
            }
        }
    });

    let key = text("Dashed outlines are new. Faded items are already on your calendar.")
        .size(label - 2.0)
        .style(text::secondary);
    let mut actions = row![Space::new().width(Fill)]
        .spacing(metrics.spacing)
        .align_y(Alignment::Center);
    // The form's Add saves the one item it holds; a preview of several items
    // from elsewhere brings its own way of saving them, and until then the
    // header's back button is the only action.
    let saving = state.inbox.draft.as_ref().is_some_and(|draft| draft.saving);
    let can_add = preview.from_form && count == 1;
    if can_add {
        actions = actions.push(
            button(text("Back to edit").size(label).center())
                .style(crate::ui::controls::secondary)
                .on_press(act(PreviewAction::Close)),
        );
        actions = actions.push(
            button(
                text(if saving { "Adding…" } else { "Add" })
                    .size(label)
                    .center(),
            )
            .style(crate::ui::controls::primary)
            .on_press_maybe((!saving).then_some(Message::Inbox(crate::inbox::Action::Create))),
        );
    }
    let mut body = column![header, list, navigation, grid, key]
        .spacing(8)
        .width(Fill)
        .height(Fill);
    if let Some(error) = state
        .inbox
        .draft
        .as_ref()
        .and_then(|draft| draft.error.as_ref())
        .or(state.calendar.error.as_ref())
    {
        body = body.push(text(error).size(label).style(text::danger));
    }
    if can_add {
        body = body.push(actions);
    }
    container(body)
        .padding([8, 12])
        .width(Fill)
        .height(Fill)
        .into()
}
