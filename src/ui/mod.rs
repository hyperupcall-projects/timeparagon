//! The interface.
//!
//! The root is wrapped in `responsive`, so the layout is chosen from the
//! space the window actually has rather than from a guess made at startup.
//! That is what lets the same code look deliberate at 180px and at 900px.

pub mod alarms;
pub mod calendar;
pub mod flash;
pub mod habits;
pub mod inbox;
pub mod keep;
pub mod layout;
pub mod links;
pub mod notepad;
pub mod projects;
pub mod recurrence;
pub mod settings;
pub mod source_icon;
mod task_drag;
pub mod timeblocking;
mod timeblocking_focus;
pub mod travel;

use iced::widget::{Space, button, column, container, mouse_area, responsive, row, text};
use iced::{Alignment, Center, Element, Fill, Length, Padding, Size};

use crate::app::{Message, Screen, State};

use layout::Metrics;

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

        // The menu keeps the width the window had before a pane widened it.
        // The details pane and block editor grow out of the view underneath,
        // not out of the navigation above it.
        let nav_width = if let Some(original) = state.timeblocking.original_width {
            (original - state.screen_margin()).max(1.0).min(size.width)
        } else if details_open {
            main_width
        } else {
            size.width
        };
        let nav_metrics = Metrics::of(Size::new(nav_width, size.height));

        let content: Element<'_, Message> = match (state.screen, metrics.breakpoint) {
            (Screen::Settings, _) => settings::view(state, &metrics),
            // The notepad takes the whole window for the same reason: it is
            // somewhere to write, not somewhere to watch the clock from.
            (Screen::KeepNotes, _) => keep::view(state, &metrics),
            (Screen::Notepad, _) => notepad::view(state, &metrics),
            (Screen::Inbox, _) => inbox::view(state, &main_metrics),
            (Screen::Backlog, _) => timeblocking::backlog(state, &main_metrics),
            (Screen::Projects, _) => projects::view(state, &metrics),
            (Screen::Travel, _) => travel::view(state, &main_metrics),
            (Screen::Habits, _) => habits::view(state, &main_metrics),
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
                (size.height - tabs_height(state, &nav_metrics)).max(1.0),
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
        // on show rather than inside it. Constraining the bar to `nav_width`
        // keeps it the size it was before a pane widened the window.
        let base = column![
            container(tabs(state, &nav_metrics)).width(Length::Fixed(nav_width)),
            content
        ]
        .width(Fill)
        .height(Fill);
        // Adding/removing a wrapper resets the state of all its descendants.
        // Keep one stack with the calendar first, and append active dialogs.
        let mut layers = vec![base.into()];
        if state.more_menu_open {
            layers.push(more_menu(state, &nav_metrics));
        }
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
        if state.habits.draft.is_some() {
            layers.push(habits::editor_overlay(state, &metrics));
        }
        if state.habits.confirm_delete.is_some() {
            layers.push(habits::delete_confirmation(state, &metrics));
        }
        if state.inbox.draft.is_some() {
            layers.push(inbox::new_task_overlay(state, &metrics));
        }
        // The floating task card is drawn last so it is never hidden, and it is
        // only added while a task is actually in the air.
        if let Some(task) = state
            .timeblocking
            .dragging
            .as_deref()
            .and_then(|id| state.calendar.event(id))
            .filter(|_| state.timeblocking.drag_pointer.is_some())
        {
            let pointer = state.timeblocking.drag_pointer.unwrap_or_default();
            // The sidebar body is inset from the window, and the pointer is in
            // window coordinates, so shift it into the view's own space.
            let inset = if let Some(sidebar) = &state.sidebar {
                let margin = state.screen_margin();
                iced::Point::new(
                    if sidebar.side == crate::panel::Side::Left {
                        margin
                    } else {
                        0.0
                    },
                    margin,
                )
            } else {
                iced::Point::ORIGIN
            };
            layers.push(drag_ghost(
                state,
                task,
                &metrics,
                iced::Point::new(pointer.x - inset.x, pointer.y - inset.y),
                size,
            ));
        }
        iced::widget::stack(layers).into()
    });
    let radius = state.config.views.border_radius();
    let content = container(content).width(Fill).height(Fill).clip(true);
    let body: Element<'_, Message> = if state.sidebar.is_some() {
        let outer_width = state.config.views.panel_border_outer_width();
        let inner_width = state.config.views.panel_border_inner_width();
        let outer_color = state.config.views.panel_border_outer_color.0;
        let inner_color = state.config.views.panel_border_inner_color.0;
        let body_radius = (radius - outer_width - inner_width).max(0.0);
        let inner_radius = (radius - outer_width).max(0.0);

        let body = container(content)
            .width(Fill)
            .height(Fill)
            .clip(true)
            .style(move |theme: &iced::Theme| container::Style {
                background: Some(theme.palette().background.into()),
                border: iced::Border {
                    radius: body_radius.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        let inner = container(body)
            .width(Fill)
            .height(Fill)
            .clip(true)
            .padding(inner_width)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(inner_color.into()),
                border: iced::Border {
                    radius: inner_radius.into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        container(inner)
            .id("sidebar-body")
            .width(Fill)
            .height(Fill)
            .clip(true)
            .padding(outer_width)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(outer_color.into()),
                border: iced::Border {
                    radius: radius.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    } else {
        container(content)
            .id("application-body")
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
            })
            .into()
    };
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
        body
    }
}

/// Places the floating task card just under the pointer, clamped to the window
/// so it stays fully on screen.
fn drag_ghost<'a>(
    state: &'a State,
    task: &'a crate::calendar::Event,
    metrics: &Metrics,
    pointer: iced::Point,
    size: Size,
) -> Element<'a, Message> {
    /// How far the card trails the pointer, so the pointer reads as holding it.
    const OFFSET: f32 = 12.0;
    let width = (size.width - 24.0).clamp(160.0, 320.0);
    let left = (pointer.x + OFFSET)
        .min((size.width - width - 4.0).max(0.0))
        .max(0.0);
    let top = (pointer.y + OFFSET)
        .min((size.height - 80.0).max(0.0))
        .max(0.0);
    container(timeblocking::drag_ghost(state, task, metrics, width))
        .width(Fill)
        .height(Fill)
        .padding(Padding {
            top,
            left,
            ..Padding::default()
        })
        .into()
}

/// How many top-level navigation items there are: Day/Week, Tasks/Projects,
/// then Note/More/Settings.
const TAB_COUNT: usize = 7;

/// The visible top-level names, in the order they appear.
const TAB_LABELS: [&str; TAB_COUNT] = [
    "Day", "Week", "Tasks", "Projects", "Note", "More", "Settings",
];

/// Short names for windows too narrow to fit the full ones legibly.
const TAB_SHORT_LABELS: [&str; TAB_COUNT] = ["Day", "Week", "Tasks", "Proj", "Note", "More", "Set"];

/// The two tabs that live inside Tasks, shown beneath the top menu while
/// either of them is open.
pub const TASKS_LABELS: [&str; 2] = ["Inbox", "Backlog"];

/// The destinations kept inside the More menu.
pub const MORE_LABELS: [&str; 3] = ["Travel", "Habits", "Keep"];

/// A little breathing room between the menu and the top of the window.
const TAB_TOP_MARGIN: f32 = 6.0;
/// The gap between the top menu and the Tasks tabs below it.
const TASKS_ROW_GAP: f32 = 2.0;

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
        // Seven tabs, gaps within the three groups, and two gaps between them.
        let fixed = 14.0 * pad + 4.0 * gap + 2.0 * (gap * 2.0).max(4.0);
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
/// Day/Week, then Tasks/Projects, then Note/More/Settings. Labels shrink
/// (and shorten, when very narrow) rather than wrapping.
///
/// While a Tasks screen is open, a second row with the Inbox and Backlog tabs
/// is placed beneath the menu.
pub(crate) fn tabs<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let fit = tab_fit(*metrics);
    let labels = if fit.short {
        TAB_SHORT_LABELS
    } else {
        TAB_LABELS
    };
    let tasks_open = matches!(state.screen, Screen::Inbox | Screen::Backlog);
    let more_active = state.more_menu_open
        || matches!(
            state.screen,
            Screen::Travel | Screen::Habits | Screen::KeepNotes
        );

    let day = tab(
        labels[0],
        state.screen == Screen::Calendar && !state.weekly,
        Message::CalendarModeSelected(false),
        metrics,
        fit,
    );
    let week = tab(
        labels[1],
        state.screen == Screen::Calendar && state.weekly,
        Message::CalendarModeSelected(true),
        metrics,
        fit,
    );
    let tasks = tab(
        labels[2],
        tasks_open,
        match state.screen {
            // Within the section, Tasks keeps the tab that is open.
            Screen::Backlog => Message::ScreenSelected(Screen::Backlog),
            _ => Message::ScreenSelected(Screen::Inbox),
        },
        metrics,
        fit,
    );
    let projects = tab(
        labels[3],
        state.screen == Screen::Projects,
        Message::ScreenSelected(Screen::Projects),
        metrics,
        fit,
    );
    let note = tab(
        labels[4],
        state.screen == Screen::Notepad,
        Message::ScreenSelected(Screen::Notepad),
        metrics,
        fit,
    );
    let more = tab(
        labels[5],
        more_active,
        Message::MoreMenuToggled,
        metrics,
        fit,
    );
    let settings = tab(
        labels[6],
        state.screen == Screen::Settings,
        Message::ScreenSelected(Screen::Settings),
        metrics,
        fit,
    );

    let group_gap = (fit.gap * 2.0).max(4.0);
    let bar = container(
        row![
            row![day, week].spacing(fit.gap),
            Space::new().width(Fill),
            row![tasks, projects].spacing(fit.gap),
            Space::new().width(Fill),
            row![note, more, settings].spacing(fit.gap),
        ]
        .spacing(group_gap)
        .align_y(Center),
    )
    .padding(Padding {
        left: metrics.padding,
        right: metrics.padding,
        top: TAB_TOP_MARGIN,
        bottom: 0.0,
    })
    .width(Fill);

    if !tasks_open {
        return bar.into();
    }

    let subtabs = row![
        tab(
            TASKS_LABELS[0],
            state.screen == Screen::Inbox,
            Message::ScreenSelected(Screen::Inbox),
            metrics,
            fit,
        ),
        tab(
            TASKS_LABELS[1],
            state.screen == Screen::Backlog,
            Message::ScreenSelected(Screen::Backlog),
            metrics,
            fit,
        ),
    ]
    .spacing(fit.gap);

    column![
        bar,
        container(subtabs).padding(Padding {
            left: metrics.padding,
            right: metrics.padding,
            top: TASKS_ROW_GAP,
            bottom: 0.0,
        }),
    ]
    .width(Fill)
    .into()
}

/// The height `tabs` takes: the menu, plus the Tasks row when it is showing.
fn tabs_height(state: &State, metrics: &Metrics) -> f32 {
    let tasks_row = if matches!(state.screen, Screen::Inbox | Screen::Backlog) {
        TASKS_ROW_GAP + tab_height(metrics)
    } else {
        0.0
    };
    TAB_TOP_MARGIN + tab_height(metrics) + tasks_row
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
    message: Message,
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
    .on_press(message)
    .style(if active {
        button::primary
    } else {
        button::secondary
    })
    .into()
}

/// The More menu: Travel, Habits and Keep, in a card below the menu bar.
fn more_menu<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill))
        .on_press(Message::MoreMenuToggled);

    let mut panel = column![].spacing(4.0).width(Fill);
    for (label, screen) in
        MORE_LABELS
            .iter()
            .zip([Screen::Travel, Screen::Habits, Screen::KeepNotes])
    {
        panel = panel.push(
            button(text(*label).size(metrics.label()).center().width(Fill))
                .width(Fill)
                .on_press(Message::ScreenSelected(screen))
                .style(if state.screen == screen {
                    button::primary
                } else {
                    button::secondary
                }),
        );
    }

    let width = (metrics.size.width - metrics.padding * 2.0).clamp(96.0, 180.0);
    let card = container(panel)
        .width(Length::Fixed(width))
        .padding(6.0)
        .style(|theme: &iced::Theme| {
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
        Space::new().height(tabs_height(state, metrics) + metrics.spacing),
        container(row![Space::new().width(Fill), card])
            .padding(Padding {
                right: metrics.padding,
                ..Default::default()
            })
            .width(Fill),
    ]
    .width(Fill)
    .align_x(Alignment::Start);

    let layers: Vec<Element<'a, Message>> = vec![dismiss.into(), aligned.into()];
    iced::widget::stack(layers).into()
}
