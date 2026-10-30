//! Shared native form controls, following calendar/popover and stacked-form layouts.
use super::layout::Metrics;
use crate::app::{Message, State};
use chrono::{Datelike, Local, Months, NaiveDate, NaiveTime, Timelike};
use iced::widget::{
    Space, button, column, container, mouse_area, pick_list, row, scrollable, stack, text,
};
use iced::{Border, Color, Element, Fill, Shadow, Theme, Vector};
use std::sync::Arc;

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
            if let Some(p) = &mut state.picker {
                if let Some(day) = p.focus.checked_add_signed(chrono::Duration::days(days)) {
                    p.focus = day;
                    p.month = crate::calendar::first_of_month(day);
                }
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
            if let Some(p) = &mut state.picker {
                if let Some(day) = p.month.with_year(year) {
                    p.month = day;
                    p.focus = day;
                }
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
    use_24_hour: bool,
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
                time.map(|t| format!(" · {}", crate::calendar::time_format::clock(t, use_24_hour)))
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
                day.unwrap_or_else(|| Local::now().date_naive()),
            ),
            focus: day.unwrap_or_else(|| Local::now().date_naive()),
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
    use_24_hour: bool,
    value: &str,
    enabled: bool,
    size: f32,
    change: impl Fn(String) -> Message + Clone + 'a,
) -> Element<'a, Message> {
    let parsed = NaiveTime::parse_from_str(value, "%H:%M").ok();
    let hour_label = move |h: u32| {
        if use_24_hour {
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
        if use_24_hour {
            (0..24).map(hour_label).collect::<Vec<_>>()
        } else {
            (1..=12).map(|h| h.to_string()).collect::<Vec<_>>()
        },
        hour,
        move |v| {
            if enabled {
                let selected = v.parse::<u32>().unwrap_or(h);
                let hour = if use_24_hour {
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
    let mut fields = row![hours, text(":").size(size), minutes]
        .spacing(3)
        .align_y(iced::Center);
    if !use_24_hour {
        fields = fields.push(
            pick_list(
                vec!["AM", "PM"],
                parsed.map(|t| if t.hour() < 12 { "AM" } else { "PM" }),
                move |period| {
                    if enabled {
                        let hour = h % 12 + if period == "PM" { 12 } else { 0 };
                        period_change(format!("{hour:02}:{m:02}"))
                    } else {
                        Message::Ignored
                    }
                },
            )
            .placeholder("AM/PM")
            .text_size(size)
            .padding([7, 8])
            .width(Fill),
        );
    }
    fields.into()
}

pub fn overlay<'a>(p: &'a Picker, metrics: &Metrics, use_24_hour: bool) -> Element<'a, Message> {
    let size = metrics.label();
    let selected = p
        .value
        .get(..10)
        .and_then(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").ok());
    let today = Local::now().date_naive();
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
    let weekdays = row(crate::calendar::WEEKDAY_INITIALS.iter().map(|s| {
        text(*s)
            .size(size - 1.0)
            .style(text::secondary)
            .width(Fill)
            .center()
            .into()
    }))
    .spacing(3);
    let days = crate::calendar::month_grid(p.month);
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
                        s.border.radius = 2.0.into();
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
                use_24_hour,
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
    let accent = container(Space::new())
        .width(3)
        .height(size + 2.0)
        .style(|theme: &Theme| container::Style {
            background: Some(theme.palette().primary.into()),
            ..Default::default()
        });
    let mut heading = row![
        accent,
        text(label.into())
            .wrapping(text::Wrapping::WordOrGlyph)
            .size(size)
            .font(iced::Font {
                weight: iced::font::Weight::Semibold,
                ..iced::Font::DEFAULT
            })
            .width(Fill)
    ]
    .spacing(8)
    .align_y(iced::Center)
    .width(Fill);
    if let Some(count) = count {
        heading = heading.push(
            container(
                text(count.to_string())
                    .font(iced::Font::MONOSPACE)
                    .size((size - 4.0).max(11.0)),
            )
            .padding([2, 6])
            .style(|theme: &Theme| {
                let p = theme.extended_palette().primary.weak;
                container::Style {
                    background: Some(p.color.into()),
                    text_color: Some(p.text),
                    border: Border {
                        radius: 2.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }),
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
                18.0
            } else {
                23.0
            }
        ),
        text(subtitle.into())
            .size(metrics.label())
            .style(text::secondary)
    ]
    .spacing(5)
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

/// Quiet elevation for list cards, kept below the stronger dialog shadow.
pub fn card_surface(theme: &Theme) -> container::Style {
    let mut style = surface(theme);
    style.shadow = Shadow {
        color: Color::BLACK.scale_alpha(0.09),
        offset: Vector::new(0.0, 2.0),
        blur_radius: 6.0,
    };
    style
}

pub fn summary_surface(theme: &Theme) -> container::Style {
    let mut style = card_surface(theme);
    style.background = Some(
        theme
            .extended_palette()
            .primary
            .weak
            .color
            .scale_alpha(0.18)
            .into(),
    );
    style
}

pub fn surface(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(p.background.base.color.into()),
        text_color: Some(p.background.base.text),
        border: Border {
            color: p.background.strong.color,
            width: 1.0,
            radius: 3.0.into(),
        },
        shadow: Shadow {
            color: Color::BLACK.scale_alpha(0.22),
            offset: Vector::new(0.0, 5.0),
            blur_radius: 16.0,
        },
        ..Default::default()
    }
}
fn raised(mut s: button::Style, theme: &Theme, status: button::Status) -> button::Style {
    s.border = Border {
        color: theme
            .extended_palette()
            .background
            .strong
            .color
            .scale_alpha(0.65),
        width: 1.0,
        radius: 2.0.into(),
    };
    s.shadow = Shadow {
        color: Color::BLACK.scale_alpha(match status {
            button::Status::Disabled => 0.0,
            button::Status::Pressed => 0.10,
            button::Status::Hovered => 0.21,
            _ => 0.17,
        }),
        offset: Vector::new(
            0.0,
            if matches!(status, button::Status::Pressed) {
                0.0
            } else {
                2.5
            },
        ),
        blur_radius: if matches!(status, button::Status::Pressed) {
            1.0
        } else {
            4.5
        },
    };
    s
}
pub fn primary(t: &Theme, s: button::Status) -> button::Style {
    raised(button::primary(t, s), t, s)
}
pub fn secondary(t: &Theme, s: button::Status) -> button::Style {
    raised(button::secondary(t, s), t, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn picker(value: &str, with_time: bool) -> Picker {
        let day = NaiveDate::from_ymd_opt(2028, 2, 29).unwrap();
        Picker {
            value: value.into(),
            month: crate::calendar::first_of_month(day),
            focus: day,
            change: Arc::new(Message::EventDraftDateChanged),
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
        let mut state = State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        );
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
