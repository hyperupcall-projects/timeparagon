//! Shared native form controls, following calendar/popover and stacked-form layouts.
use super::layout::Metrics;
use crate::app::{Message, State};
use crate::calendar::first_day::FirstDayOfWeek;
use crate::calendar::time_format::TimeFormat;
use chrono::{Datelike, Months, NaiveDate, NaiveTime, Timelike};
use iced::advanced::widget::{Operation, Tree, tree};
use iced::advanced::{Layout, Widget, layout, renderer};
use iced::widget::{Space, button, column, container, mouse_area, row, stack, text};
use iced::{Border, Color, Element, Fill, Renderer, Shadow, Theme, Vector, mouse};
use std::sync::Arc;
use timeslay_core::theme as look;

type Change = Arc<dyn Fn(String) -> Message + Send + Sync>;
#[derive(Clone)]
pub struct Picker {
    pub value: String,
    pub month: NaiveDate,
    pub focus: NaiveDate,
    pub change: Change,
    pub optional: bool,
    pub with_time: bool,
    pub time: Option<NaiveTime>,
}
impl Picker {
    fn selected_value(&self, day: NaiveDate) -> String {
        match self.time.filter(|_| self.with_time) {
            Some(t) => format!("{} {}", day, t.format("%H:%M")),
            None => day.to_string(),
        }
    }
}
impl std::fmt::Debug for Picker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Picker")
            .field("value", &self.value)
            .finish()
    }
}
#[derive(Debug, Clone)]
pub enum Action {
    Open(Picker),
    Close,
    Month(i32),
    Move(i64),
    Year(i32),
    Select(NaiveDate),
    Clear,
    Time(String),
    ToggleTime,
}

pub fn update(state: &mut State, action: Action) -> iced::Task<Message> {
    match action {
        Action::Open(p) => {
            state.picker = Some(p);
            // Blur the underlying editor so its caret cannot consume calendar keys.
            return iced::widget::operation::focus(iced::widget::Id::new("calendar-picker-focus"));
        }
        Action::Close => state.picker = None,
        Action::Move(days) => {
            if let Some(p) = &mut state.picker
                && let Some(day) = p.focus.checked_add_signed(chrono::Duration::days(days))
            {
                p.focus = day;
                p.month = crate::calendar::first_of_month(day);
            }
        }
        Action::Month(delta) => {
            if let Some(p) = &mut state.picker {
                let next = if delta < 0 {
                    p.month
                        .checked_sub_months(Months::new(delta.unsigned_abs()))
                } else {
                    p.month.checked_add_months(Months::new(delta as u32))
                };
                if let Some(next) = next {
                    p.month = next;
                    p.focus = next;
                }
            }
        }
        Action::Year(year) => {
            if let Some(p) = &mut state.picker
                && let Some(day) = p.month.with_year(year)
            {
                p.month = day;
                p.focus = day;
            }
        }
        Action::ToggleTime => {
            if let Some(p) = &mut state.picker {
                p.time = if p.time.is_some() {
                    None
                } else {
                    NaiveTime::from_hms_opt(9, 0, 0)
                };
            }
        }
        Action::Time(value) => {
            if let Some(p) = &mut state.picker {
                p.time = NaiveTime::parse_from_str(&value, "%H:%M").ok();
            }
        }
        Action::Select(day) => {
            if let Some(p) = state.picker.take() {
                let value = p.selected_value(day);
                return iced::Task::done((p.change)(value));
            }
        }
        Action::Clear => {
            if let Some(p) = state.picker.take() {
                return iced::Task::done((p.change)(String::new()));
            }
        }
    }
    iced::Task::none()
}
fn msg(a: Action) -> Message {
    Message::Picker(a)
}

/// Readable date trigger; ISO formatting stays at the data boundary.
pub fn date<'a>(
    time_format: TimeFormat,
    value: &str,
    optional: bool,
    with_time: bool,
    enabled: bool,
    size: f32,
    change: impl Fn(String) -> Message + Send + Sync + 'static,
) -> iced::widget::Button<'a, Message> {
    let day = value
        .get(..10)
        .and_then(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
    let time = value
        .get(11..)
        .and_then(|v| NaiveTime::parse_from_str(v, "%H:%M").ok());
    let caption = day
        .map(|d| {
            format!(
                "{}{}",
                d.format("%b %-d, %Y"),
                time.map(|t| format!(" · {}", crate::calendar::time_format::clock(t, time_format)))
                    .unwrap_or_default()
            )
        })
        .unwrap_or_else(|| "Choose date".into());
    let mut trigger = button(
        row![
            text("▦").size(size),
            text(caption).size(size).width(Fill),
            text("⌄").size(size)
        ]
        .spacing(8)
        .align_y(iced::Center),
    )
    .padding([8, 10])
    .width(Fill)
    .style(secondary);
    if enabled {
        trigger = trigger.on_press(msg(Action::Open(Picker {
            value: value.into(),
            month: crate::calendar::first_of_month(
                day.unwrap_or_else(|| crate::clock::now().date_naive()),
            ),
            focus: day.unwrap_or_else(|| crate::clock::now().date_naive()),
            change: Arc::new(change),
            optional,
            with_time,
            time,
        })));
    }
    trigger
}

/// Separate hour and minute columns, retaining minute precision.
pub fn time<'a>(
    time_format: TimeFormat,
    value: &str,
    enabled: bool,
    size: f32,
    change: impl Fn(String) -> Message + Clone + 'a,
) -> Element<'a, Message> {
    let parsed = NaiveTime::parse_from_str(value, "%H:%M").ok();
    let hour_label = move |h: u32| {
        if time_format.is_24_hour() {
            format!("{h:02}")
        } else {
            ((h + 11) % 12 + 1).to_string()
        }
    };
    let hour = parsed.map(|t| hour_label(t.hour()));
    let minute = parsed.map(|t| format!("{:02}", t.minute()));
    let h = parsed.map(|t| t.hour()).unwrap_or(9);
    let m = parsed.map(|t| t.minute()).unwrap_or(0);
    let c = change.clone();
    let period_change = change.clone();
    let hours = pick_list(
        if time_format.is_24_hour() {
            (0..24).map(hour_label).collect::<Vec<_>>()
        } else {
            (1..=12).map(|h| h.to_string()).collect::<Vec<_>>()
        },
        hour,
        move |v| {
            if enabled {
                let selected = v.parse::<u32>().unwrap_or(h);
                let hour = if time_format.is_24_hour() {
                    selected
                } else {
                    selected % 12 + (h / 12) * 12
                };
                c(format!("{hour:02}:{m:02}"))
            } else {
                Message::Ignored
            }
        },
    )
    .placeholder("HH")
    .text_size(size)
    .padding([7, 8])
    .width(Fill);
    let minutes = pick_list(
        (0..60).map(|n| format!("{n:02}")).collect::<Vec<_>>(),
        minute,
        move |v| {
            if enabled {
                change(format!("{h:02}:{v}"))
            } else {
                Message::Ignored
            }
        },
    )
    .placeholder("MM")
    .text_size(size)
    .padding([7, 8])
    .width(Fill);
    let compact = time_format == TimeFormat::Compact;
    let periods = if compact { ["a", "p"] } else { ["AM", "PM"] };
    let mut fields = row![
        hours,
        text(if compact { "." } else { ":" }).size(size),
        minutes
    ]
    .spacing(3)
    .align_y(iced::Center);
    if !time_format.is_24_hour() {
        fields = fields.push(
            pick_list(
                periods.to_vec(),
                parsed.map(|t| periods[usize::from(t.hour() >= 12)]),
                move |period| {
                    if enabled {
                        let hour = h % 12 + if period == periods[1] { 12 } else { 0 };
                        period_change(format!("{hour:02}:{m:02}"))
                    } else {
                        Message::Ignored
                    }
                },
            )
            .placeholder(if compact { "a/p" } else { "AM/PM" })
            .text_size(size)
            .padding([7, 8])
            .width(Fill),
        );
    }
    fields.into()
}

pub fn overlay<'a>(
    p: &'a Picker,
    metrics: &Metrics,
    time_format: TimeFormat,
    first_day: FirstDayOfWeek,
) -> Element<'a, Message> {
    let size = metrics.label();
    let selected = p
        .value
        .get(..10)
        .and_then(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
    let today = crate::clock::now().date_naive();
    let arrow = |caption, delta| {
        button(text(caption).center())
            .padding([6, 10])
            .style(secondary)
            .on_press(msg(Action::Month(delta)))
    };
    let years: Vec<i32> =
        ((p.month.year() - 100).max(1)..=(p.month.year() + 100).min(9999)).collect();
    let header: Element<'a, Message> = if metrics.size.width < 300.0 {
        column![
            row![
                arrow("‹", -1),
                text(p.month.format("%b %Y").to_string())
                    .size(size)
                    .width(Fill)
                    .center(),
                arrow("›", 1)
            ]
            .spacing(4)
            .align_y(iced::Center),
            row![
                text("Year").size(size).width(Fill),
                pick_list(years.clone(), Some(p.month.year()), |y| msg(Action::Year(
                    y
                )))
                .text_size(size)
            ]
            .spacing(4)
            .align_y(iced::Center)
        ]
        .spacing(6)
        .into()
    } else {
        row![
            arrow("‹", -1),
            text(p.month.format("%B").to_string())
                .size(size + 2.0)
                .width(Fill)
                .center(),
            pick_list(years, Some(p.month.year()), |y| msg(Action::Year(y))).text_size(size),
            arrow("›", 1)
        ]
        .spacing(6)
        .align_y(iced::Center)
        .into()
    };
    let weekdays = row(first_day.initials().map(|s| {
        text(s)
            .size(size - 1.0)
            .style(text::secondary)
            .width(Fill)
            .center()
            .into()
    }))
    .spacing(3);
    let days = crate::calendar::month_grid(p.month, first_day);
    let weeks = column(
        days.chunks(7)
            .map(|week| {
                row(week.iter().map(|&day| {
                    button(
                        text(day.day().to_string())
                            .font(iced::Font::MONOSPACE)
                            .size(size)
                            .center(),
                    )
                    .width(Fill)
                    .height(32)
                    .padding(0)
                    .style(move |theme: &Theme, status| {
                        let mut s = if Some(day) == selected {
                            primary(theme, status)
                        } else {
                            button::text(theme, status)
                        };
                        s.border.radius = RADIUS_SMALL.into();
                        if day == today || day == p.focus {
                            s.border.width = 1.0;
                            s.border.color = theme.palette().primary;
                        }
                        if day.month() != p.month.month() {
                            s.text_color = s.text_color.scale_alpha(0.4);
                        }
                        s
                    })
                    .on_press(msg(Action::Select(day)))
                    .into()
                }))
                .spacing(3)
                .into()
            })
            .collect::<Vec<Element<'a, Message>>>(),
    )
    .spacing(3);
    let mut body = column![
        row![
            text("Choose date").size(size + 3.0).width(Fill),
            button(text("×"))
                .style(button::text)
                .on_press(msg(Action::Close))
        ]
        .align_y(iced::Center),
        header,
        weekdays,
        weeks
    ]
    .spacing(10);
    if p.with_time {
        body = body.push(
            button(
                text(if p.time.is_some() {
                    "Remove deadline time"
                } else {
                    "+ Add deadline time"
                })
                .size(size),
            )
            .style(button::text)
            .on_press(msg(Action::ToggleTime)),
        );
        if let Some(t) = p.time {
            body = body.push(time(
                time_format,
                &t.format("%H:%M").to_string(),
                true,
                size,
                |v| msg(Action::Time(v)),
            ));
        }
    }
    let mut shortcuts = row![
        button(text("Today").size(size))
            .style(secondary)
            .on_press(msg(Action::Select(today))),
        button(text("Tomorrow").size(size))
            .style(secondary)
            .on_press(msg(Action::Select(today.succ_opt().unwrap_or(today))))
    ]
    .spacing(6);
    if p.optional {
        shortcuts = shortcuts.push(
            button(text("Clear").size(size))
                .style(button::text)
                .on_press(msg(Action::Clear)),
        );
    }
    body = body.push(shortcuts.wrap()).push(
        text("Arrow keys to move · Enter to select · Esc to close")
            .size(size - 2.0)
            .style(text::secondary),
    );
    let card = container(scrollable(body))
        .padding(14)
        .width(320)
        .max_height((metrics.size.height - 32.0).max(100.0))
        .style(surface);
    stack![
        mouse_area(
            container(Space::new())
                .width(Fill)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(Color::BLACK.scale_alpha(0.28).into()),
                    ..Default::default()
                })
        )
        .on_press(msg(Action::Close)),
        container(
            mouse_area(card)
                .on_press(Message::Ignored)
                .on_scroll(|_| Message::Ignored)
        )
        .padding(12)
        .center_x(Fill)
        .center_y(Fill)
    ]
    .into()
}

/// A consistent title hierarchy with an accent marker and optional live count.
pub fn section_heading<'a>(
    label: impl Into<String>,
    count: Option<usize>,
    size: f32,
) -> iced::widget::Row<'a, Message> {
    let mut heading = row![
        text(label.into())
            .wrapping(text::Wrapping::WordOrGlyph)
            .size(size)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..iced::Font::DEFAULT
            })
            .width(Fill)
    ]
    .spacing(6)
    .align_y(iced::Center)
    .width(Fill);
    if let Some(count) = count {
        heading = heading.push(
            container(text(count.to_string()).size((size - 4.0).max(11.0)))
                .padding([0, 7])
                .style(tag),
        );
    }
    heading
}

pub fn page_heading<'a>(
    title: impl Into<String>,
    subtitle: impl Into<String>,
    count: Option<usize>,
    metrics: &Metrics,
) -> iced::widget::Column<'a, Message> {
    column![
        section_heading(
            title,
            count,
            if metrics.breakpoint.is_compact() {
                16.0
            } else {
                21.0
            }
        ),
        text(subtitle.into())
            .size(metrics.label())
            .style(text::secondary)
    ]
    .spacing(1)
    .width(Fill)
}

/// Put actions below headings when a side-by-side toolbar would squeeze the title.
pub fn page_toolbar<'a>(
    title: iced::widget::Column<'a, Message>,
    action: Element<'a, Message>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    if metrics.size.width < 300.0 {
        column![title, action].spacing(metrics.spacing).into()
    } else {
        row![title, action]
            .spacing(metrics.spacing)
            .align_y(iced::Center)
            .into()
    }
}

pub use timeslay_core::theme::{RADIUS_CARD, RADIUS_CONTROL, RADIUS_ROW, RADIUS_SMALL, SCRIM};

/// A soft black shadow, deepened on dark themes where it would otherwise vanish.
pub fn shadow(theme: &Theme, alpha: f32, y: f32, blur_radius: f32) -> Shadow {
    elevated(
        theme,
        look::Elevation {
            alpha,
            y,
            blur: blur_radius,
        },
    )
}

fn elevated(theme: &Theme, elevation: look::Elevation) -> Shadow {
    Shadow {
        color: elevation.color(theme.extended_palette().is_dark),
        offset: Vector::new(0.0, elevation.y),
        blur_radius: elevation.blur,
    }
}

/// The white surface everything sits on.
pub fn card_color(theme: &Theme) -> iced::theme::palette::Pair {
    theme.extended_palette().background.weakest
}

/// The defined edge of cards and controls.
pub fn edge_color(theme: &Theme) -> Color {
    theme.extended_palette().background.strong.color
}

/// A section or list card.
pub fn card_surface(theme: &Theme) -> container::Style {
    let card = card_color(theme);
    container::Style {
        background: Some(card.color.into()),
        text_color: Some(card.text),
        border: Border {
            color: edge_color(theme),
            width: 1.0,
            radius: RADIUS_CARD.into(),
        },
        shadow: elevated(theme, look::CARD),
        ..Default::default()
    }
}

/// One item inside a card or list.
pub fn row_surface(theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            radius: RADIUS_ROW.into(),
            ..card_surface(theme).border
        },
        shadow: elevated(theme, look::ROW),
        ..card_surface(theme)
    }
}

/// A count, duration or label pill.
pub fn tag(theme: &Theme) -> container::Style {
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.base.color.into()),
        text_color: Some(palette.secondary.base.color),
        border: Border {
            color: palette.background.neutral.color,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    }
}

/// A card tinted with the accent, for totals and summaries.
pub fn summary_surface(theme: &Theme) -> container::Style {
    let accent = theme.extended_palette().primary.weak;
    container::Style {
        background: Some(accent.color.into()),
        text_color: Some(accent.text),
        ..card_surface(theme)
    }
}

/// Floating dialogs and menus: the same card, lifted well off the page.
pub fn surface(theme: &Theme) -> container::Style {
    container::Style {
        shadow: elevated(theme, look::FLOATING),
        ..card_surface(theme)
    }
}

fn raised(mut s: button::Style, theme: &Theme, status: button::Status) -> button::Style {
    let elevation = match status {
        button::Status::Disabled => look::Elevation {
            alpha: 0.0,
            y: 0.0,
            blur: 0.0,
        },
        button::Status::Pressed => look::BUTTON_PRESSED,
        button::Status::Hovered => look::BUTTON_HOVERED,
        button::Status::Active => look::BUTTON,
    };
    s.border.radius = RADIUS_CONTROL.into();
    s.border.width = 1.0;
    s.shadow = elevated(theme, elevation);
    s
}

fn filled(
    theme: &Theme,
    status: button::Status,
    accent: iced::theme::palette::Pair,
) -> button::Style {
    let shade = |by: f32| look::shade(accent.color, by);
    let background = match status {
        button::Status::Hovered => shade(look::FILLED_HOVER_SHADE),
        button::Status::Pressed => shade(look::FILLED_PRESS_SHADE),
        _ => accent.color,
    };
    let disabled = status == button::Status::Disabled;
    raised(
        button::Style {
            background: Some(
                background
                    .scale_alpha(if disabled { 0.45 } else { 1.0 })
                    .into(),
            ),
            text_color: accent.text.scale_alpha(if disabled { 0.8 } else { 1.0 }),
            border: Border {
                color: shade(look::FILLED_EDGE_SHADE).scale_alpha(if disabled {
                    0.45
                } else {
                    1.0
                }),
                ..Default::default()
            },
            ..Default::default()
        },
        theme,
        status,
    )
}

pub fn primary(t: &Theme, s: button::Status) -> button::Style {
    filled(t, s, t.extended_palette().primary.base)
}

pub fn danger(t: &Theme, s: button::Status) -> button::Style {
    filled(t, s, t.extended_palette().danger.base)
}

pub fn secondary(t: &Theme, s: button::Status) -> button::Style {
    let palette = t.extended_palette();
    let card = card_color(t);
    let (background, text_color, edge) = match s {
        button::Status::Hovered => (
            palette.background.weaker.color,
            palette.background.weaker.text,
            palette.background.stronger.color,
        ),
        button::Status::Pressed => (
            palette.background.weak.color,
            palette.background.weak.text,
            palette.background.stronger.color,
        ),
        button::Status::Disabled => (
            palette.background.base.color,
            palette.secondary.base.color,
            palette.background.neutral.color,
        ),
        button::Status::Active => (card.color, card.text, edge_color(t)),
    };
    raised(
        button::Style {
            background: Some(background.into()),
            text_color,
            border: Border {
                color: edge,
                ..Default::default()
            },
            ..Default::default()
        },
        t,
        s,
    )
}

/// A borderless button that fills in when the pointer reaches it.
pub fn ghost(t: &Theme, s: button::Status) -> button::Style {
    let palette = t.extended_palette();
    let (background, edge, text_color) = match s {
        button::Status::Hovered => (
            palette.background.weaker.color,
            palette.background.neutral.color,
            palette.background.weaker.text,
        ),
        button::Status::Pressed => (
            palette.background.weak.color,
            palette.background.neutral.color,
            palette.background.weak.text,
        ),
        button::Status::Disabled => (
            Color::TRANSPARENT,
            Color::TRANSPARENT,
            palette.secondary.base.color,
        ),
        button::Status::Active => (
            Color::TRANSPARENT,
            Color::TRANSPARENT,
            palette.background.base.text,
        ),
    };
    button::Style {
        background: Some(background.into()),
        text_color,
        border: Border {
            color: edge,
            width: 1.0,
            radius: RADIUS_CONTROL.into(),
        },
        ..Default::default()
    }
}

/// A navigation entry: outlined in the accent while its page is on show.
pub fn nav(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |t, s| {
        if !active {
            return ghost(t, s);
        }
        let palette = t.extended_palette();
        let tint = match s {
            button::Status::Active | button::Status::Disabled => palette.primary.weak.color,
            _ => iced::theme::palette::mix(
                palette.primary.weak.color,
                palette.primary.base.color,
                0.2,
            ),
        };
        button::Style {
            background: Some(tint.into()),
            text_color: palette.primary.weak.text,
            border: Border {
                color: palette.primary.base.color,
                width: 1.0,
                radius: RADIUS_CONTROL.into(),
            },
            shadow: elevated(t, look::NAV_ACTIVE),
            ..Default::default()
        }
    }
}

fn field_border(theme: &Theme, emphasis: bool) -> Border {
    Border {
        color: if emphasis {
            theme.extended_palette().primary.base.color
        } else {
            edge_color(theme)
        },
        width: 1.0,
        radius: RADIUS_CONTROL.into(),
    }
}

pub fn input_style(
    theme: &Theme,
    status: iced::widget::text_input::Status,
) -> iced::widget::text_input::Style {
    let palette = theme.extended_palette();
    let emphasis = matches!(
        status,
        iced::widget::text_input::Status::Hovered
            | iced::widget::text_input::Status::Focused { .. }
    );
    iced::widget::text_input::Style {
        background: if status == iced::widget::text_input::Status::Disabled {
            palette.background.base.color
        } else {
            card_color(theme).color
        }
        .into(),
        border: field_border(theme, emphasis),
        icon: palette.secondary.base.color,
        placeholder: palette.secondary.base.color,
        value: card_color(theme).text,
        selection: palette.primary.weak.color,
    }
}

pub fn editor_style(
    theme: &Theme,
    status: iced::widget::text_editor::Status,
) -> iced::widget::text_editor::Style {
    let palette = theme.extended_palette();
    let emphasis = matches!(
        status,
        iced::widget::text_editor::Status::Hovered
            | iced::widget::text_editor::Status::Focused { .. }
    );
    iced::widget::text_editor::Style {
        background: if status == iced::widget::text_editor::Status::Disabled {
            palette.background.base.color
        } else {
            card_color(theme).color
        }
        .into(),
        border: field_border(theme, emphasis),
        placeholder: palette.secondary.base.color,
        value: card_color(theme).text,
        selection: palette.primary.weak.color,
    }
}

pub fn pick_list_style(
    theme: &Theme,
    status: iced::widget::pick_list::Status,
) -> iced::widget::pick_list::Style {
    let palette = theme.extended_palette();
    iced::widget::pick_list::Style {
        text_color: card_color(theme).text,
        placeholder_color: palette.secondary.base.color,
        handle_color: palette.secondary.base.color,
        background: card_color(theme).color.into(),
        border: field_border(theme, status != iced::widget::pick_list::Status::Active),
    }
}

pub fn menu_style(theme: &Theme) -> iced::widget::overlay::menu::Style {
    let accent = theme.extended_palette().primary.weak;
    iced::widget::overlay::menu::Style {
        background: card_color(theme).color.into(),
        border: Border {
            color: edge_color(theme),
            width: 1.0,
            radius: RADIUS_CONTROL.into(),
        },
        text_color: card_color(theme).text,
        selected_text_color: accent.text,
        selected_background: accent.color.into(),
        shadow: elevated(theme, look::MENU),
    }
}

pub fn scrollable_style(
    theme: &Theme,
    status: iced::widget::scrollable::Status,
) -> iced::widget::scrollable::Style {
    let palette = theme.extended_palette();
    let base = iced::widget::scrollable::default(theme, status);
    let rail = |hovered: bool| iced::widget::scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: iced::widget::scrollable::Scroller {
            background: if hovered {
                palette.background.stronger.color
            } else {
                palette.background.strong.color
            }
            .into(),
            border: Border {
                radius: 999.0.into(),
                ..Default::default()
            },
        },
    };
    let (vertical, horizontal) = match status {
        iced::widget::scrollable::Status::Active { .. } => (false, false),
        iced::widget::scrollable::Status::Hovered {
            is_vertical_scrollbar_hovered,
            is_horizontal_scrollbar_hovered,
            ..
        } => (
            is_vertical_scrollbar_hovered,
            is_horizontal_scrollbar_hovered,
        ),
        iced::widget::scrollable::Status::Dragged {
            is_vertical_scrollbar_dragged,
            is_horizontal_scrollbar_dragged,
            ..
        } => (
            is_vertical_scrollbar_dragged,
            is_horizontal_scrollbar_dragged,
        ),
    };
    iced::widget::scrollable::Style {
        vertical_rail: rail(vertical),
        horizontal_rail: rail(horizontal),
        gap: None,
        ..base
    }
}

pub fn checkbox_style(
    theme: &Theme,
    status: iced::widget::checkbox::Status,
) -> iced::widget::checkbox::Style {
    let palette = theme.extended_palette();
    let (is_checked, hovered, disabled) = match status {
        iced::widget::checkbox::Status::Active { is_checked } => (is_checked, false, false),
        iced::widget::checkbox::Status::Hovered { is_checked } => (is_checked, true, false),
        iced::widget::checkbox::Status::Disabled { is_checked } => (is_checked, false, true),
    };
    let accent = palette.primary.base;
    iced::widget::checkbox::Style {
        background: if is_checked {
            accent.color.scale_alpha(if disabled { 0.5 } else { 1.0 })
        } else {
            card_color(theme).color
        }
        .into(),
        icon_color: accent.text,
        border: Border {
            color: if is_checked || hovered {
                accent.color
            } else {
                palette.background.stronger.color
            },
            width: 1.5,
            radius: RADIUS_SMALL.into(),
        },
        text_color: None,
    }
}

pub fn toggler_style(
    theme: &Theme,
    status: iced::widget::toggler::Status,
) -> iced::widget::toggler::Style {
    let palette = theme.extended_palette();
    let on = matches!(
        status,
        iced::widget::toggler::Status::Active { is_toggled: true }
            | iced::widget::toggler::Status::Hovered { is_toggled: true }
            | iced::widget::toggler::Status::Disabled { is_toggled: true }
    );
    let edge = if on {
        palette.primary.strong.color
    } else {
        palette.background.stronger.color
    };
    iced::widget::toggler::Style {
        background: if on {
            palette.primary.base.color
        } else {
            palette.background.weak.color
        }
        .into(),
        background_border_width: 1.0,
        background_border_color: edge,
        foreground: card_color(theme).color.into(),
        foreground_border_width: 1.0,
        foreground_border_color: edge,
        text_color: None,
        border_radius: None,
        padding_ratio: 0.1,
    }
}

pub fn slider_style(
    theme: &Theme,
    status: iced::widget::slider::Status,
) -> iced::widget::slider::Style {
    let palette = theme.extended_palette();
    iced::widget::slider::Style {
        rail: iced::widget::slider::Rail {
            backgrounds: (
                palette.primary.base.color.into(),
                palette.background.neutral.color.into(),
            ),
            width: 4.0,
            border: Border {
                radius: 2.0.into(),
                ..Default::default()
            },
        },
        handle: iced::widget::slider::Handle {
            shape: iced::widget::slider::HandleShape::Circle { radius: 7.0 },
            background: card_color(theme).color.into(),
            border_width: if status == iced::widget::slider::Status::Active {
                1.0
            } else {
                2.0
            },
            border_color: palette.primary.base.color,
        },
    }
}

// The themed constructors. A style set here applies everywhere the widget is
// used, and a call site can still override it with its own `.style`.

/// The main action of a form or page.
pub fn action_button<'a, M: 'a>(
    content: impl Into<Element<'a, M>>,
) -> button::Button<'a, M, Theme, iced::Renderer> {
    button(content).style(primary)
}

/// A supporting action.
pub fn quiet_button<'a, M: 'a>(
    content: impl Into<Element<'a, M>>,
) -> button::Button<'a, M, Theme, iced::Renderer> {
    button(content).style(secondary)
}

pub fn text_input<'a, M: Clone + 'a>(
    placeholder: &str,
    value: &str,
) -> iced::widget::text_input::TextInput<'a, M, Theme, iced::Renderer> {
    iced::widget::text_input(placeholder, value).style(input_style)
}

pub fn text_editor<'a, M: Clone + 'a>(
    content: &'a iced::widget::text_editor::Content<iced::Renderer>,
) -> iced::widget::text_editor::TextEditor<
    'a,
    iced::advanced::text::highlighter::PlainText,
    M,
    Theme,
    iced::Renderer,
> {
    iced::widget::text_editor(content).style(editor_style)
}

pub fn pick_list<'a, T, L, V, M>(
    options: L,
    selected: Option<V>,
    on_selected: impl Fn(T) -> M + 'a,
) -> iced::widget::pick_list::PickList<'a, T, L, V, M, Theme, iced::Renderer>
where
    T: ToString + PartialEq + Clone + 'a,
    L: std::borrow::Borrow<[T]> + 'a,
    V: std::borrow::Borrow<T> + 'a,
    M: Clone + 'a,
{
    iced::widget::pick_list(options, selected, on_selected)
        .style(pick_list_style)
        .menu_style(menu_style)
}

pub fn scrollable<'a, M: 'a>(
    content: impl Into<Element<'a, M>>,
) -> iced::widget::scrollable::Scrollable<'a, M, Theme, iced::Renderer> {
    iced::widget::scrollable(content).style(scrollable_style)
}

pub fn checkbox<'a, M: 'a>(
    is_checked: bool,
) -> iced::widget::checkbox::Checkbox<'a, M, Theme, iced::Renderer> {
    iced::widget::checkbox(is_checked).style(checkbox_style)
}

pub fn toggler<'a, M: 'a>(
    is_checked: bool,
) -> iced::widget::toggler::Toggler<'a, M, Theme, iced::Renderer> {
    iced::widget::toggler(is_checked).style(toggler_style)
}

pub fn slider<'a, T, M>(
    range: std::ops::RangeInclusive<T>,
    value: T,
    on_change: impl Fn(T) -> M + 'a,
) -> iced::widget::slider::Slider<'a, T, M, Theme>
where
    T: Copy + From<u8> + PartialOrd,
    M: Clone + 'a,
{
    iced::widget::slider(range, value, on_change).style(slider_style)
}

/// Shows the normal arrow where a widget reports [`mouse::Interaction::Idle`],
/// such as `opaque` or a disabled text input. `iced_layershell` draws that
/// interaction as the busy spinner. Wrap the root of a layer surface's view: the
/// layers of a stack still need to see `Idle` to stop hovers reaching those below.
pub fn arrow_when_idle<'a, M: 'a>(content: Element<'a, M>) -> Element<'a, M> {
    Element::new(ArrowWhenIdle { content })
}

struct ArrowWhenIdle<'a, M> {
    content: Element<'a, M>,
}

impl<M> Widget<M, Theme, Renderer> for ArrowWhenIdle<'_, M> {
    fn tag(&self) -> tree::Tag {
        self.content.as_widget().tag()
    }
    fn state(&self) -> tree::State {
        self.content.as_widget().state()
    }
    fn children(&self) -> Vec<Tree> {
        self.content.as_widget().children()
    }
    fn diff(&self, tree: &mut Tree) {
        self.content.as_widget().diff(tree);
    }
    fn size(&self) -> iced::Size<iced::Length> {
        self.content.as_widget().size()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn iced::advanced::Clipboard,
        shell: &mut iced::advanced::Shell<'_, M>,
        viewport: &iced::Rectangle,
    ) {
        self.content.as_widget_mut().update(
            tree, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &iced::Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        match self
            .content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
        {
            mouse::Interaction::Idle => mouse::Interaction::None,
            interaction => interaction,
        }
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &iced::Rectangle,
        translation: iced::Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, M, Theme, Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::EventDraftMessage;
    use iced::advanced::renderer::Headless;

    #[test]
    fn opaque_areas_show_the_arrow_and_still_cover_the_layers_below() {
        let view: Element<'_, Message> = arrow_when_idle(
            stack![
                mouse_area(Space::new().width(Fill).height(Fill))
                    .interaction(mouse::Interaction::Pointer),
                iced::widget::opaque(Space::new().width(Fill).height(Fill)),
            ]
            .into(),
        );
        let mut renderer = iced::futures::executor::block_on(Renderer::new(
            iced::Font::DEFAULT,
            iced::Pixels(16.0),
            Some("tiny-skia"),
        ))
        .unwrap();
        let mut ui = iced_runtime::UserInterface::build(
            view,
            iced::Size::new(100.0, 100.0),
            iced_runtime::user_interface::Cache::default(),
            &mut renderer,
        );
        let position = iced::Point::new(50.0, 50.0);
        let (state, _) = ui.update(
            &[iced::Event::Mouse(mouse::Event::CursorMoved { position })],
            mouse::Cursor::Available(position),
            &mut renderer,
            &mut iced::advanced::clipboard::Null,
            &mut Vec::new(),
        );
        assert!(matches!(
            state,
            iced_runtime::user_interface::State::Updated {
                mouse_interaction: mouse::Interaction::None,
                ..
            }
        ));
    }

    #[test]
    fn high_contrast_neutral_buttons_keep_readable_text_in_each_active_state() {
        let catalog = crate::theme::Catalog::built_in();
        for name in ["Light high contrast", "Dark high contrast"] {
            let theme = &catalog.get(name).unwrap().theme;
            for status in [
                button::Status::Active,
                button::Status::Hovered,
                button::Status::Pressed,
            ] {
                let style = secondary(theme, status);
                let Some(iced::Background::Color(background)) = style.background else {
                    panic!("{name}: missing button fill");
                };
                assert!(
                    background.relative_contrast(style.text_color) >= 4.5,
                    "{name}: button text"
                );
            }
            // Secondary palette colors also carry form labels and placeholders.
            assert!(
                theme
                    .palette()
                    .background
                    .relative_contrast(theme.extended_palette().secondary.base.color)
                    >= 4.5,
                "{name}: muted text"
            );
        }
    }

    fn picker(value: &str, with_time: bool) -> Picker {
        let day = NaiveDate::from_ymd_opt(2028, 2, 29).unwrap();
        Picker {
            value: value.into(),
            month: crate::calendar::first_of_month(day),
            focus: day,
            change: Arc::new(|v0| {
                Message::EventDraft(EventDraftMessage::EventDraftDateChanged(v0))
            }),
            optional: true,
            with_time,
            time: NaiveTime::from_hms_opt(23, 47, 0),
        }
    }
    #[test]
    fn selecting_dates_preserves_optional_time_and_minute_precision() {
        let p = picker("", true);
        assert_eq!(p.selected_value(p.focus), "2028-02-29 23:47");
        let p = picker("", false);
        assert_eq!(p.selected_value(p.focus), "2028-02-29");
    }
    #[test]
    fn calendar_navigation_crosses_month_and_year_boundaries() {
        let mut state = State::blank();
        let _ = update(&mut state, Action::Open(picker("", false)));
        let _ = update(&mut state, Action::Move(1));
        assert_eq!(
            state.picker.as_ref().unwrap().focus.to_string(),
            "2028-03-01"
        );
        let _ = update(&mut state, Action::Month(-3));
        assert_eq!(
            state.picker.as_ref().unwrap().month.to_string(),
            "2027-12-01"
        );
        let _ = update(&mut state, Action::Close);
        assert!(state.picker.is_none());
    }
}
