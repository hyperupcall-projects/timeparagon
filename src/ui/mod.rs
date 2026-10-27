//! The interface.
//!
//! The root is wrapped in `responsive`, so the layout is chosen from the
//! space the window actually has rather than from a guess made at startup.
//! That is what lets the same code look deliberate at 180px and at 900px.

pub mod alarms;
pub mod calendar;
pub mod controls;
pub mod digits;
pub mod flash;
pub mod inbox;
pub mod keep;
pub mod layout;
pub mod links;
pub mod notepad;
pub mod projects;
pub mod settings;
pub mod source_icon;
mod task_drag;
pub mod timeblocking;
mod timeblocking_focus;
pub mod travel;

use std::time::Duration;

use iced::widget::{Space, button, column, container, responsive, row, scrollable, text};
use iced::{Center, Element, Fill, Length, Padding, Size};

use crate::alarm;
use crate::app::{Message, Screen, State};
use crate::engine::{Precision, format};

use layout::{Breakpoint, Metrics};

pub fn view(state: &State) -> Element<'_, Message> {
    let content = responsive(move |size| {
        let metrics = Metrics::of(size);

        // The event details pane is a column of its own to the right of the
        // view. The view keeps its width and the window is widened to match,
        // so the metrics for the main content are measured over that width.
        let details_open = state.calendar.open_event.is_some()
            && crate::app::shows_details(state.screen)
            && !(state.screen == Screen::Calendar && state.weekly);
        let detail = if details_open {
            state.config.views.detail_width()
        } else {
            0.0
        };
        let main_width = (size.width - detail).max(1.0);
        let main_metrics = Metrics::of(Size::new(main_width, size.height));

        let content: Element<'_, Message> = match (state.screen, metrics.breakpoint) {
            (Screen::Settings, _) => settings::view(state, &metrics),
            // The alarm and set-time panes need the whole window below Wide.
            (Screen::TimerSettings, Breakpoint::Wide) => row![
                container(calendar::view(state, &metrics)).width(Length::FillPortion(2)),
                container(settings::timer_view(state, &metrics)).width(Length::FillPortion(3)),
            ]
            .into(),
            (Screen::TimerSettings, _) => settings::timer_view(state, &metrics),
            (Screen::SetTime, Breakpoint::Wide) => row![
                container(calendar::view(state, &metrics)).width(Length::FillPortion(2)),
                container(set_time(state, &metrics)).width(Length::FillPortion(3)),
            ]
            .into(),
            (Screen::SetTime, _) => set_time(state, &metrics),
            // The notepad takes the whole window for the same reason: it is
            // somewhere to write, not somewhere to watch the clock from.
            (Screen::KeepNotes, _) => keep::view(state, &metrics),
            (Screen::Notepad, _) => notepad::view(state, &metrics),
            (Screen::Inbox, _) => inbox::view(state, &main_metrics),
            (Screen::Backlog, _) => timeblocking::backlog(state, &main_metrics),
            (Screen::Projects, _) => projects::view(state, &metrics),
            (Screen::Travel, _) => travel::view(state, &main_metrics),
            (Screen::Calendar, _) => {
                if state.weekly {
                    calendar::weekly_view(state, &main_metrics)
                } else {
                    // Keep the calendar at the same place in the widget tree when
                    // the block editor opens or closes, preserving its scroll state.
                    match state
                        .timeblocking
                        .block
                        .is_some()
                        .then_some(state.timeblocking.original_width)
                        .flatten()
                    {
                        // The calendar keeps the width it had before the editor
                        // opened, and the editor takes whatever the window gained.
                        Some(base) => {
                            let pane = (base - state.screen_margin()).min(main_width);
                            let calendar_metrics = Metrics::of(Size::new(pane, size.height));
                            row![
                                container(calendar::view(state, &calendar_metrics))
                                    .width(Length::Fixed(pane)),
                                container(timeblocking::panel(state, &main_metrics)).width(Fill),
                            ]
                            .into()
                        }
                        None => {
                            row![container(calendar::view(state, &main_metrics)).width(Fill)].into()
                        }
                    }
                }
            }
        };

        // The details pane is drawn last, so on the calendar it sits to the
        // right of the time-block editor rather than over it.
        let content: Element<'_, Message> = if details_open {
            // The pane sits below the tabs, so it only has what they leave.
            let detail_metrics = Metrics::of(Size::new(
                detail,
                (size.height - tabs_height(&metrics)).max(1.0),
            ));
            row![
                container(content).width(Fill),
                container(calendar::details_pane(state, &detail_metrics))
                    .width(Length::Fixed(detail)),
            ]
            .into()
        } else {
            content
        };

        // The tabs are the window's navigation, so they sit above whatever is
        // on show rather than inside it.
        let base = column![tabs(state, &metrics), content]
            .width(Fill)
            .height(Fill);
        // Adding/removing a wrapper resets the state of all its descendants.
        // Keep one stack with the calendar first, and append active dialogs.
        let mut layers = vec![base.into()];
        if state.timeblocking.focus.is_some() {
            layers.push(timeblocking::focus_overlay(state, &metrics));
        }
        if state.timeblocking.estimate_task.is_some() {
            layers.push(timeblocking::estimate_overlay(state, &metrics));
        }
        if state.calendar.pending_delete.is_some() {
            layers.push(calendar::delete_confirmation(state, &metrics));
        }
        if state.travel.open.is_some() {
            layers.push(travel::leg_overlay(state, &metrics));
        }
        if state.inbox.draft.is_some() {
            layers.push(inbox::new_task_overlay(state, &metrics));
        }
        iced::widget::stack(layers).into()
    });
    let radius = state.config.views.border_radius();
    let body = container(content)
        .id(if state.sidebar.is_some() {
            "sidebar-body"
        } else {
            "application-body"
        })
        .width(Fill)
        .height(Fill)
        .clip(true)
        .style(move |theme: &iced::Theme| container::Style {
            background: Some(theme.palette().background.into()),
            border: iced::Border {
                radius: radius.into(),
                ..Default::default()
            },
            ..Default::default()
        });
    if let Some(sidebar) = &state.sidebar {
        let margin = state.screen_margin();
        container(body)
            .padding(Padding {
                top: margin,
                bottom: margin,
                left: if sidebar.side == crate::panel::Side::Left {
                    margin
                } else {
                    0.0
                },
                right: if sidebar.side == crate::panel::Side::Right {
                    margin
                } else {
                    0.0
                },
            })
            .width(Fill)
            .height(Fill)
            .into()
    } else {
        body.into()
    }
}

/// How many tabs there are.
const TAB_COUNT: usize = 8;

/// Full tab names, excluding the separately rendered Weekly tab.
/// Inbox/Projects/Backlog/Travel, then Keep/Notepad/Settings.
const TAB_LABELS: [&str; TAB_COUNT] = [
    "Daily", "Inbox", "Projects", "Backlog", "Travel", "Keep", "Notepad", "Settings",
];

/// Short names for windows too narrow to fit the full ones legibly.
const TAB_SHORT_LABELS: [&str; TAB_COUNT] = [
    "Daily", "Inbox", "Proj", "Back", "Trav", "Keep", "Note", "Set",
];

/// Average glyph width as a fraction of the font size, with some slack.
const TAB_GLYPH_WIDTH: f32 = 0.6;
/// The smallest tab text that is still worth showing in full.
const TAB_FULL_MIN_SIZE: f32 = 9.0;
const TAB_MIN_SIZE: f32 = 7.0;

/// How the tab row is laid out at a given width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabFit {
    pub short: bool,
    pub size: f32,
    pad: f32,
    gap: f32,
}

/// The labels shown at `width`, so tests and callers agree with `tabs`.
pub fn tab_labels(width: f32) -> [&'static str; TAB_COUNT] {
    if tab_fit(Metrics::of(Size::new(width, 0.0))).short {
        TAB_SHORT_LABELS
    } else {
        TAB_LABELS
    }
}

/// Choose the longest labels and largest text that fit on a single row.
fn tab_fit(metrics: Metrics) -> TabFit {
    let available = (metrics.size.width - metrics.padding * 2.0).max(1.0);
    let max_size = metrics.label();
    let (pad, gap) = if metrics.size.width < 300.0 {
        (1.0, 1.0)
    } else {
        (5.0, metrics.spacing.min(6.0))
    };
    let size_for = |labels: &[&str; TAB_COUNT]| {
        let chars: usize = labels
            .iter()
            .map(|label| label.chars().count())
            .sum::<usize>()
            + 6;
        // Nine tabs, gaps within groups, and two gaps between groups.
        let fixed = 18.0 * pad + 6.0 * gap + 2.0 * (gap * 2.0).max(4.0);
        ((available - fixed) / (chars as f32 * TAB_GLYPH_WIDTH)).min(max_size)
    };
    let full = size_for(&TAB_LABELS);
    if full >= TAB_FULL_MIN_SIZE.min(max_size) {
        TabFit {
            short: false,
            size: full,
            pad,
            gap,
        }
    } else {
        TabFit {
            short: true,
            size: size_for(&TAB_SHORT_LABELS).max(TAB_MIN_SIZE),
            pad,
            gap,
        }
    }
}

/// A single row of navigation in three groups with space between them:
/// Daily/Weekly, then Inbox/Projects/Backlog/Travel, then Keep/Notepad/Settings.
/// Labels shrink (and shorten, when very narrow) rather than wrapping.
pub(crate) fn tabs<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let fit = tab_fit(*metrics);
    let labels = if fit.short {
        TAB_SHORT_LABELS
    } else {
        TAB_LABELS
    };
    let settings_active = matches!(state.screen, Screen::Settings | Screen::TimerSettings);
    let screens = [
        Screen::Calendar,
        Screen::Inbox,
        Screen::Projects,
        Screen::Backlog,
        Screen::Travel,
        Screen::KeepNotes,
        Screen::Notepad,
        Screen::Settings,
    ];
    let mut tabs: Vec<Element<'a, Message>> = screens
        .iter()
        .zip(labels)
        .map(|(&screen, label)| {
            let active = if screen == Screen::Settings {
                settings_active
            } else if screen == Screen::Calendar {
                state.screen == screen && !state.weekly
            } else {
                state.screen == screen
            };
            tab(label, active, screen, metrics, fit)
        })
        .collect();
    let right: Vec<_> = tabs.drain(5..).collect();
    let middle: Vec<_> = tabs.drain(1..).collect();
    let calendar = tabs.remove(0);
    let weekly = button(
        text("Weekly")
            .size(fit.size)
            .wrapping(text::Wrapping::None)
            .center(),
    )
    .height(tab_height(metrics))
    .padding(Padding::from([0.0, fit.pad]))
    .on_press(Message::CalendarModeSelected(true))
    .style(if state.screen == Screen::Calendar && state.weekly {
        button::primary
    } else {
        button::secondary
    });

    let group_gap = (fit.gap * 2.0).max(4.0);
    container(
        row![
            row![calendar, weekly].spacing(fit.gap),
            Space::new().width(Fill),
            row(middle).spacing(fit.gap),
            Space::new().width(Fill),
            row(right).spacing(fit.gap),
        ]
        .spacing(group_gap)
        .align_y(Center),
    )
    .padding(Padding {
        left: metrics.padding,
        right: metrics.padding,
        top: 0.0,
        bottom: 0.0,
    })
    .width(Fill)
    .into()
}

/// The height `tabs` takes: always one row.
fn tabs_height(metrics: &Metrics) -> f32 {
    tab_height(metrics)
}

fn tab_height(metrics: &Metrics) -> f32 {
    if metrics.breakpoint.is_compact() {
        16.0
    } else {
        metrics.icon_button()
    }
}

/// One tab, lit when it is the page on show.
fn tab<'a>(
    label: &'a str,
    active: bool,
    screen: Screen,
    metrics: &Metrics,
    fit: TabFit,
) -> Element<'a, Message> {
    button(
        text(label)
            .size(fit.size)
            .wrapping(text::Wrapping::None)
            .center(),
    )
    .height(tab_height(metrics))
    .padding(Padding::from([0.0, fit.pad]))
    .on_press(if screen == Screen::Calendar {
        Message::CalendarModeSelected(false)
    } else {
        Message::ScreenSelected(screen)
    })
    .style(if active {
        button::primary
    } else {
        button::secondary
    })
    .into()
}

/// A settings-style page: a title with a way back, over a scrolling body.
///
/// The settings panes and the todo scratchpad share this so they read as the
/// same kind of place.
pub(crate) fn page<'a>(
    title: &'a str,
    content: Element<'a, Message>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    column![
        row![
            text(title).size(metrics.label() + 3.0).width(Fill),
            button(text("Done").size(metrics.label()))
                .on_press(Message::ScreenSelected(Screen::Calendar))
                .style(button::primary),
        ]
        .align_y(Center),
        scrollable(content).height(Fill).width(Fill),
    ]
    .spacing(metrics.spacing)
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill)
    .into()
}

/// The clock: the readout, the one action button, and the row of secondary
/// controls.
///
/// The calendar no longer unfolds the clock, so nothing draws this on the day
/// view now. It is kept with the timer engine behind it. The padding around it
/// belongs to the caller, which already has some.
pub fn clock_body<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let content = column![
        container(timer(state, metrics)).height(Fill).center_y(Fill),
        controls::primary_action(state, metrics),
        controls::secondary_row(metrics),
    ]
    .spacing(metrics.spacing)
    .width(Fill)
    .height(Fill);

    // The countdown warning tints the app window rather than the screen,
    // which is the whole reason it is a separate, gentler cue. It runs over
    // the final seconds before the target, while the clock is still under it.
    let remaining = state.timer.target().map_or(Duration::ZERO, |target| {
        target.saturating_sub(state.timer.elapsed(state.now))
    });
    let tint = alarm::countdown_pulse(&state.config.alarm.flash, remaining);

    match tint {
        Some(alpha) => {
            let color = state.config.alarm.flash.color.0;

            container(content)
                .style(move |_theme| container::Style {
                    background: Some(color.scale_alpha(alpha).into()),
                    ..container::Style::default()
                })
                .into()
        }
        None => content.into(),
    }
}

/// The single clock: the elapsed time, with the target tucked inside the ring.
fn timer<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let entry = state.catalog.resolve(&state.config.appearance.theme);
    let elapsed = state.timer.elapsed(state.now);
    let value = digits::format(elapsed, Precision::Seconds, metrics);

    let readout = match state.timer.target() {
        // The ring only makes sense when there is a target to fill.
        Some(target) if !target.is_zero() => {
            let caption = format!(
                "of {}",
                format::readout(target, Precision::Seconds, metrics.allow_hours)
            );

            digits::with_ring(
                value,
                caption,
                metrics,
                entry.accents.digits,
                entry.accents.ring,
                state.timer.progress(state.now),
            )
        }
        _ => digits::readout(value, metrics, entry.accents.digits),
    };

    let mut content = column![container(readout).height(Fill).center_y(Fill)]
        .spacing(metrics.spacing)
        .width(Fill)
        .height(Fill);

    if state.timer.target().is_none() {
        content = content.push(no_target_line(metrics));
    }

    content.into()
}

/// A small note under the readout when there is no target at all.
fn no_target_line<'a>(metrics: &Metrics) -> Element<'a, Message> {
    text("No target")
        .size(metrics.label())
        .width(Fill)
        .center()
        .into()
}

/// The pane for choosing the target time.
fn set_time<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let entry = state.catalog.resolve(&state.config.appearance.theme);

    let readout = match state.timer.target() {
        Some(target) => {
            let value = digits::format(target, Precision::Seconds, metrics);
            digits::readout(value, metrics, entry.accents.digits)
        }
        None => container(text("No target").size(metrics.label() + 2.0))
            .center_x(Fill)
            .center_y(Fill)
            .into(),
    };

    let content = column![
        container(readout).height(Fill).center_y(Fill),
        no_target_button(state, metrics),
        controls::presets(state, metrics),
        controls::duration_adjust(metrics),
        done_button(metrics),
    ]
    .spacing(metrics.spacing)
    .width(Fill)
    .height(Fill);

    column![text("Set Time").size(metrics.label() + 3.0), content]
        .spacing(metrics.spacing)
        .padding(metrics.padding)
        .into()
}

/// Returns to the calendar, accepting whatever target is set.
fn done_button<'a>(metrics: &Metrics) -> Element<'a, Message> {
    button(text("Done").size(metrics.label()).center().width(Fill))
        .width(Fill)
        .height(metrics.icon_button())
        .on_press(Message::ScreenSelected(Screen::Calendar))
        .style(button::primary)
        .into()
}

/// Turns the target off, leaving a plain count-up.
fn no_target_button<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let style = if state.timer.target().is_none() {
        button::primary
    } else {
        button::secondary
    };

    button(text("No target").size(metrics.label()).center().width(Fill))
        .width(Fill)
        .height(metrics.icon_button())
        .on_press(Message::TargetCleared)
        .style(style)
        .into()
}

/// The readout as plain text, for the window title.
pub fn readout(state: &State) -> String {
    format::readout(state.timer.elapsed(state.now), Precision::Seconds, true)
}
