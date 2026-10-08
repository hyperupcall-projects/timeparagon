//! Google Calendar day view, drawn to resemble the web UI.

use chrono::{DateTime, Local};
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Style, Text};
use iced::widget::{
    button, canvas as canvas_widget, column, container, mouse_area, row, scrollable, stack, text,
    toggler,
};
use iced::{
    Alignment, Color, Element, Fill, Font, Length, Pixels, Point, Rectangle, Renderer, Size, Theme,
    mouse,
};

use crate::app::{Message, State};
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

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let today = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
    let heading = calendar::day_heading(state.calendar.day, today);

    let header = row![
        text(heading).size(metrics.label() + 1.0).width(Fill),
        button(text("Menu").size(metrics.label()).center())
            .on_press(Message::CalendarMenuToggled)
            .style(if state.calendar.menu_open {
                button::primary
            } else {
                button::secondary
            }),
    ]
    .spacing(metrics.spacing)
    .align_y(Alignment::Center);

    let status = text(state.calendar.status_line())
        .size(metrics.label() - 1.0)
        .style(if state.calendar.error.is_some() {
            text::danger
        } else {
            text::secondary
        });

    let all_day = all_day_row(state, metrics);
    let grid = day_grid(state, metrics, Local::now());

    // The pointer watch records where the click landed without publishing a
    // message, so scrolling stays on the widget's own path.
    let body = pointer_watch(
        column![all_day, grid]
            .spacing(metrics.spacing)
            .height(Fill)
            .width(Fill),
    );

    let content = column![header, status, body, clock_footer(state, metrics)]
        .spacing(metrics.spacing)
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill);

    // The root stays a stack whether or not a card is open. Swapping the root
    // widget throws away the day grid's scroll state, which jumps the bar.
    let overlay = if state.calendar.open_event.is_some() {
        details_overlay(state, metrics)
    } else if state.calendar.menu_open {
        menu_overlay(state, metrics)
    } else {
        iced::widget::Space::new().width(0.0).height(0.0).into()
    };

    stack![content, overlay].into()
}

/// The clock's own status, repeated at the foot of the calendar: how long it
/// has been running and the target it is counting toward.
fn clock_footer<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
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

    row![
        text(elapsed).size(metrics.label()).width(Fill),
        text(target).size(metrics.label()).style(text::secondary),
    ]
    .spacing(metrics.spacing)
    .align_y(Alignment::Center)
    .into()
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
        button(text("Settings").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(Message::ScreenSelected(crate::app::Screen::Settings))
            .style(button::secondary),
    );
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
        // Keep the hamburger row tappable; the sheet starts below it.
        iced::widget::Space::new().height(metrics.icon_button() + metrics.spacing),
        container(card).padding(metrics.padding).width(Fill),
    ]
    .width(Fill);

    stack![dismiss, aligned].into()
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
}

impl Default for DrawState {
    fn default() -> Self {
        Self {
            grid: canvas::Cache::new(),
            key: std::cell::Cell::new(0),
            shift: std::cell::Cell::new(false),
            hovered: std::cell::Cell::new(None),
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
                self.track_hover(state, bounds, cursor)
            }
            iced::Event::Mouse(mouse::Event::CursorLeft) => self.track_hover(state, bounds, cursor),
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                match cursor.position_in(bounds) {
                    // Empty time closes any open card, which is how clicking
                    // away still works now that there is no dismiss layer.
                    Some(point) => match self.hit(Some(point), bounds.width) {
                        Some(index) => {
                            let id = self.blocks.get(index)?.id.clone();
                            Some(
                                canvas::Action::publish(Message::EventOpened(id))
                                    .and_capture(),
                            )
                        }
                        None => Some(
                            canvas::Action::publish(Message::EventClosed).and_capture(),
                        ),
                    },
                    None => None,
                }
            }
            _ => None,
        }
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if self.hit(cursor.position_in(bounds), bounds.width).is_some() {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
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
        let key = cache_key(&self.blocks, self.hour_height, self.gutter, self.compact, self.day_start_hour);
        if state.key.get() != key {
            state.grid.clear();
            state.key.set(key);
        }

        let grid = state.grid.draw(renderer, bounds.size(), |frame| {
            self.draw_grid(frame);
        });

        let mut needle_frame = Frame::new(renderer, bounds.size());
        self.draw_needle(&mut needle_frame);
        if let Some(index) = self.hit(cursor.position_in(bounds), bounds.width) {
            self.draw_hover(&mut needle_frame, index);
        }

        vec![grid, needle_frame.into_geometry()]
    }
}

impl DayCanvas {
    fn draw_grid(&self, frame: &mut Frame) {
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

        for block in &self.blocks {
            let block_bounds = self.block_rect(block, frame.width());
            let shape = Path::rounded_rectangle(
                Point::new(block_bounds.x, block_bounds.y),
                Size::new(block_bounds.width, block_bounds.height),
                EVENT_RADIUS.into(),
            );

            frame.fill(&shape, block.color);
            frame.stroke(
                &shape,
                Stroke {
                    style: Style::Solid(Color::BLACK.scale_alpha(0.55)),
                    width: 1.0,
                    ..Stroke::default()
                },
            );

            let ink = on_color(block.color);
            let title_size = if self.compact { 11.0 } else { 12.0 };
            fill_glyphs(frame, Text {
                content: block.title.clone(),
                position: Point::new(block_bounds.x + 6.0, block_bounds.y + 3.0),
                color: ink,
                size: Pixels(title_size),
                font: Font::DEFAULT,
                align_x: iced::alignment::Horizontal::Left.into(),
                align_y: iced::alignment::Vertical::Top,
                line_height: text::LineHeight::default(),
                shaping: text::Shaping::Basic,
                max_width: (block_bounds.width - 10.0).max(8.0),
            });

            if block_bounds.height >= 28.0 {
                fill_glyphs(frame, Text {
                    content: block.when.clone(),
                    position: Point::new(block_bounds.x + 6.0, block_bounds.y + title_size + 5.0),
                    color: ink.scale_alpha(0.85),
                    size: Pixels(title_size - 1.0),
                    font: Font::DEFAULT,
                    align_x: iced::alignment::Horizontal::Left.into(),
                    align_y: iced::alignment::Vertical::Top,
                    line_height: text::LineHeight::default(),
                    shaping: text::Shaping::Basic,
                    max_width: (block_bounds.width - 10.0).max(8.0),
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
        let minute_height = self.hour_height / 60.0;
        let grid_width = (width - self.gutter).max(40.0);
        let columns = block.columns.max(1) as f32;
        let column = block.column as f32;
        let block_width = (grid_width / columns) - EVENT_GAP;
        Rectangle {
            x: self.gutter + column * (grid_width / columns) + EVENT_GAP / 2.0,
            y: block.start_min as f32 * minute_height,
            width: block_width.max(8.0),
            height: ((block.end_min - block.start_min) as f32 * minute_height).max(16.0),
        }
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

/// Everything the cached geometry is drawn from, so a rebuilt [`Block`] with a
/// different label or shade retraces instead of going stale.
fn cache_key(blocks: &[Block], hour_height: f32, gutter: f32, compact: bool, day_start_hour: u32) -> u64 {
    let mut hash = hour_height.to_bits() as u64;
    hash = hash.wrapping_mul(31).wrapping_add(gutter.to_bits() as u64);
    hash = hash.wrapping_mul(31).wrapping_add(u64::from(compact));
    hash = hash.wrapping_mul(31).wrapping_add(u64::from(day_start_hour));

    for block in blocks {
        hash = hash.wrapping_mul(31).wrapping_add(block.start_min as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.end_min as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.column as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.columns as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.color.r.to_bits() as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.color.g.to_bits() as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.color.b.to_bits() as u64);
        hash = hash.wrapping_mul(31).wrapping_add(block.color.a.to_bits() as u64);
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
        };

        let key = cache_key(&[block("Standup", "09:00 – 09:30", 1.0)], 48.0, GUTTER, false, 0);

        // A key that drifts between frames would retrace the whole day while
        // scrolling, and one that ignores a label would draw it stale.
        assert_eq!(
            key,
            cache_key(&[block("Standup", "09:00 – 09:30", 1.0)], 48.0, GUTTER, false, 0)
        );

        for changed in [
            block("Lunch", "09:00 – 09:30", 1.0),
            block("Standup", "09:00 – 10:00", 1.0),
            block("Standup", "09:00 – 09:30", 0.5),
        ] {
            assert_ne!(
                key,
                cache_key(std::slice::from_ref(&changed), 48.0, GUTTER, false, 0),
                "a changed block has to be retraced"
            );
        }

        // Moving the day's boundary relabels every hour, so the grid has to be
        // retraced even though the events did not move.
        assert_ne!(
            key,
            cache_key(&[block("Standup", "09:00 – 09:30", 1.0)], 48.0, GUTTER, false, 3),
            "a new day boundary has to relabel the grid"
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
        let canvas = DayCanvas {
            blocks: vec![Block {
                id: "standup".into(),
                title: "Standup".into(),
                when: "9:00 – 9:30".into(),
                start_min: 9 * 60,
                end_min: 9 * 60 + 30,
                column: 0,
                columns: 1,
                color: Color::WHITE,
            }],
            needle: None,
            hour_height: 60.0,
            gutter: GUTTER,
            compact: false,
            day_start_hour: 0,
        };

        let rect = canvas.block_rect(&canvas.blocks[0], 240.0);
        assert_eq!(
            canvas.hit(Some(Point::new(rect.x + 4.0, rect.y + 4.0)), 240.0),
            Some(0)
        );
        assert_eq!(canvas.hit(Some(Point::new(8.0, rect.y + 4.0)), 240.0), None);
    }
}
