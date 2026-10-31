//! The interface.
//!
//! The root is wrapped in `responsive`, so the layout is chosen from the
//! space the window actually has rather than from a guess made at startup.
//! That is what lets the same code look deliberate at 180px and at 900px.

pub mod alarms;
mod anchored_menu;
pub mod brain_dump;
pub mod calendar;
pub mod controls;
mod description;
pub mod flash;
pub mod habits;
pub mod inbox;
pub mod keep;
pub mod layout;
pub mod links;
mod meetings;
pub mod notepad;
pub mod projects;
pub mod recurrence;
pub mod settings;
pub mod source_icon;
mod task_drag;
mod task_input;
pub mod timeblocking;
mod timeblocking_focus;
pub mod travel;

use iced::widget::{Space, button, column, container, mouse_area, responsive, row, text, tooltip};
use iced::{Alignment, Center, Element, Fill, Length, Padding, Size};

use crate::app::{Message, Screen, State};

use layout::Metrics;

/// Match the editor's previous position below the navigation.
pub fn editor_window_top(state: &State) -> f32 {
    let width = state.main_size.width - state.screen_margin_width();
    state.screen_margin()
        + tabs_height(
            state,
            &Metrics::of(Size::new(width, state.main_size.height)),
        )
}

pub fn editor_window(state: &State) -> Element<'_, Message> {
    responsive(move |size| {
        if state.travel.map.active {
            return crate::travel::map::view(state);
        }
        let mut panes = row![];
        if state.timeblocking.block.is_some() {
            let width = state.config.views.time_block_width().min(size.width);
            let metrics = Metrics::of(Size::new(width, size.height));
            panes = panes.push(container(timeblocking::panel(state, &metrics)).width(width));
        }
        if state.calendar.open_event.is_some()
            || state.calendar.draft.as_ref().is_some_and(|d| d.editor_open)
        {
            let block = if state.timeblocking.block.is_some() {
                state.config.views.time_block_width()
            } else {
                0.0
            };
            let width = state
                .config
                .views
                .detail_width()
                .min((size.width - block).max(1.0));
            let metrics = Metrics::of(Size::new(width, size.height));
            panes = panes.push(container(calendar::details_pane(state, &metrics)).width(width));
        }
        let metrics = Metrics::of(size);
        let mut layers = vec![panes.height(Fill).into()];
        if state.inbox.draft.is_some() {
            layers.push(inbox::new_task_overlay(state, &metrics));
        }
        if state.timeblocking.focus.is_some() {
            layers.push(timeblocking::focus_overlay(state, &metrics));
        }
        if state.timeblocking.session_edit.is_some() {
            layers.push(timeblocking::session_editor(state, &metrics));
        }
        if state.calendar.pending_delete.is_some() {
            layers.push(calendar::delete_confirmation(state, &metrics));
        }
        if state.description_popup.is_some() {
            layers.push(description::overlay(state, &metrics));
        }
        if state.timeblocking.estimate_task.is_some() {
            layers.push(timeblocking::estimate_overlay(state, &metrics));
        }
        if let Some(picker) = &state.picker {
            layers.push(controls::overlay(
                picker,
                &metrics,
                state.config.appearance.use_24_hour,
            ));
        }
        container(iced::widget::stack(layers))
            .width(Fill)
            .height(Fill)
            .style(controls::surface)
            .into()
    })
    .into()
}

pub fn view(state: &State) -> Element<'_, Message> {
    let content = responsive(move |size| {
        if state.travel.map.active {
            let map = crate::travel::map::view(state);
            if state.task_creation_window {
                // The dock normally accepts input only over its bottom-aligned
                // task card. The map replaces it and fills the entire surface.
                let bounds =
                    |size: Size| Message::TaskPopoverBounds(iced::Rectangle::with_size(size));
                return iced::widget::sensor(map)
                    .on_show(bounds)
                    .on_resize(bounds)
                    .into();
            }
            return map;
        }
        let metrics = Metrics::of(size);

        if state.task_creation_window {
            let mut layers = vec![inbox::new_task_overlay(state, &metrics)];
            if state.timeblocking.estimate_task.is_some() {
                layers.push(timeblocking::estimate_overlay(state, &metrics));
            }
            if let Some(picker) = &state.picker {
                layers.push(controls::overlay(
                    picker,
                    &metrics,
                    state.config.appearance.use_24_hour,
                ));
            }
            return iced::widget::stack(layers).into();
        }

        let main_metrics = metrics;
        let nav_width = size.width;
        let nav_metrics = Metrics::of(size);

        let content: Element<'_, Message> = match (state.screen, metrics.breakpoint) {
            (Screen::Settings, _) => settings::view(state, &metrics),
            // The notepad takes the whole window for the same reason: it is
            // somewhere to write, not somewhere to watch the clock from.
            (Screen::KeepNotes, _) => keep::view(state, &metrics),
            (Screen::Notepad, _) => notepad::view(state, &metrics),
            (Screen::BrainDump, _) => brain_dump::view(state, &metrics),
            (Screen::Overdue, _) => inbox::overdue(state, &main_metrics),
            (Screen::Inbox, _) => inbox::view(state, &main_metrics),
            (Screen::Triaged, _) => inbox::triaged(state, &main_metrics),
            (Screen::Backlog, _) => timeblocking::backlog(state, &main_metrics),
            (Screen::Projects, _) => projects::view(state, &metrics),
            (Screen::Travel, _) => travel::view(state, &main_metrics),
            (Screen::Habits, _) => habits::view(state, &main_metrics),
            (Screen::Calendar, _) => {
                if state.monthly {
                    calendar::monthly_view(state, &main_metrics)
                } else if state.weekly {
                    calendar::weekly_view(state, &main_metrics)
                } else {
                    row![container(calendar::view(state, &main_metrics)).width(Fill)].into()
                }
            }
        };

        let mut base = column![
            container(tabs(state, &nav_metrics)).width(Length::Fixed(nav_width)),
            content
        ]
        .width(Fill)
        .height(Fill);
        if state.sidebar.is_some() {
            // The add-task bar belongs to the main panel; floating editors
            // never participate in its layout.
            let borders = 2.0
                * (state.config.views.panel_border_outer_width()
                    + state.config.views.panel_border_inner_width());
            let footer_width = (state.calendar_width() - borders).max(1.0).min(size.width);
            base = base.push(
                container(
                    tooltip(
                        button(text("+").size(14).center().width(Fill))
                            .width(Fill)
                            .height(22)
                            .padding(0)
                            .style(crate::ui::controls::secondary)
                            .on_press(Message::Inbox(crate::inbox::Action::NewTask)),
                        text("Add task").size(12),
                        tooltip::Position::Top,
                    )
                    .style(crate::ui::controls::surface),
                )
                .id("panel-add-task")
                .width(Length::Fixed(footer_width))
                .height(22),
            );
        }
        // Adding/removing a wrapper resets the state of all its descendants.
        // Keep one stack with the calendar first, and append active dialogs.
        let mut layers = vec![base.into()];
        if state.view_menu_open {
            layers.push(view_menu(state, &nav_metrics));
        }
        if state.more_menu_open {
            layers.push(more_menu(state, &nav_metrics));
        }
        if state.timeblocking.focus.is_some() {
            layers.push(timeblocking::focus_overlay(state, &metrics));
        }
        if state.timeblocking.session_edit.is_some() {
            layers.push(timeblocking::session_editor(state, &metrics));
        }
        if state.brain_dump.open.is_some() {
            layers.push(brain_dump::editor_overlay(state, &metrics));
        }
        if state.inbox.draft.is_some() {
            layers.push(inbox::new_task_overlay(state, &metrics));
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
        if state.description_popup.is_some() {
            layers.push(description::overlay(state, &metrics));
        }
        if state.habits.confirm_delete.is_some() {
            layers.push(habits::delete_confirmation(state, &metrics));
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
            let inset = if state.sidebar.is_some() {
                let margin = state.screen_margin();
                iced::Point::new(margin, margin)
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
        if let Some(picker) = &state.picker {
            layers.push(controls::overlay(
                picker,
                &metrics,
                state.config.appearance.use_24_hour,
            ));
        }
        iced::widget::stack(layers).into()
    });
    if state.task_creation_window {
        return content.into();
    }
    let radius = if state.sidebar.is_some() {
        state.config.views.panel_border_radius()
    } else {
        state.config.views.border_radius()
    };
    let content = container(content).width(Fill).height(Fill).clip(true);
    let body: Element<'_, Message> = {
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
            .id(if state.sidebar.is_some() {
                "sidebar-body"
            } else {
                "application-body"
            })
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
    };
    if state.sidebar.is_some() {
        let margin = state.screen_margin();
        container(body)
            .padding(margin)
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

/// How many top-level navigation items there are: the Day/Week/Month switcher,
/// Tasks/Projects, then Note/More/Settings.
const TAB_COUNT: usize = 6;

/// The visible top-level names, in the order they appear. The first item is the
/// calendar view switcher, which shows the current view rather than a fixed name.
const TAB_LABELS: [&str; TAB_COUNT] = ["Day", "Tasks", "Projects", "Note", "More", "Settings"];

/// Short names for windows too narrow to fit the full ones legibly.
const TAB_SHORT_LABELS: [&str; TAB_COUNT] = ["Day", "Tasks", "Proj", "Note", "More", "Set"];

/// The longest labels the menu can show, used to reserve room for the switcher
/// whatever view it is currently displaying.
const TAB_WIDEST_LABELS: [&str; TAB_COUNT] =
    ["Month", "Tasks", "Projects", "Note", "More", "Settings"];
const TAB_WIDEST_SHORT_LABELS: [&str; TAB_COUNT] =
    ["Month", "Tasks", "Proj", "Note", "More", "Set"];

/// The label for the calendar view switcher: the view currently on show.
pub fn calendar_view_label(state: &State) -> &'static str {
    if state.monthly {
        "Month"
    } else if state.weekly {
        "Week"
    } else {
        "Day"
    }
}

/// The tabs that live inside Tasks, shown beneath the top menu while any of
/// them is open.
pub const TASKS_LABELS: [&str; 4] = ["Overdue", "Inbox", "Triaged", "Backlog"];

/// The destinations kept inside the More menu.
pub const MORE_LABELS: [&str; 4] = ["Travel", "Habits", "Keep", "Brain dump"];

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
            // The switcher's chevron is a glyph of its own, plus a little slack.
            + 4;
        // Six tabs, the switcher's second half, the gaps within the three
        // groups, and the two gaps between them.
        let fixed = 14.0 * pad + 3.0 * gap + 2.0 * (gap * 2.0).max(4.0) + 1.0;
        ((available - fixed) / (chars as f32 * TAB_GLYPH_WIDTH)).min(max_size)
    };
    let full = size_for(&TAB_WIDEST_LABELS);
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
            size: size_for(&TAB_WIDEST_SHORT_LABELS).max(TAB_MIN_SIZE),
            pad,
            gap,
        }
    }
}

/// A single row of navigation in three groups with space between them:
/// the Day/Week/Month switcher, then Tasks/Projects, then Note/More/Settings.
/// Labels shrink (and shorten, when very narrow) rather than wrapping.
///
/// While a Tasks screen is open, a second row with the task-list tabs
/// is placed beneath the menu.
pub(crate) fn tabs<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let fit = tab_fit(*metrics);
    let labels = if fit.short {
        TAB_SHORT_LABELS
    } else {
        TAB_LABELS
    };
    let tasks_open = matches!(
        state.screen,
        Screen::Overdue | Screen::Inbox | Screen::Triaged | Screen::Backlog
    );
    let more_active = state.more_menu_open
        || matches!(
            state.screen,
            Screen::Travel | Screen::Habits | Screen::KeepNotes | Screen::BrainDump
        );

    let select_view = if state.monthly {
        Message::CalendarMonthViewSelected
    } else {
        Message::CalendarModeSelected(state.weekly)
    };
    let view = view_tab(
        calendar_view_label(state),
        select_view,
        state.screen == Screen::Calendar || state.view_menu_open,
        metrics,
        fit,
    );
    let tasks = tab(
        labels[1],
        tasks_open,
        match state.screen {
            // Within the section, Tasks keeps the tab that is open.
            Screen::Overdue => Message::ScreenSelected(Screen::Overdue),
            Screen::Triaged => Message::ScreenSelected(Screen::Triaged),
            Screen::Backlog => Message::ScreenSelected(Screen::Backlog),
            _ => Message::ScreenSelected(Screen::Inbox),
        },
        metrics,
        fit,
    );
    let projects = tab(
        labels[2],
        state.screen == Screen::Projects,
        Message::ScreenSelected(Screen::Projects),
        metrics,
        fit,
    );
    let note = tab(
        labels[3],
        state.screen == Screen::Notepad,
        Message::ScreenSelected(Screen::Notepad),
        metrics,
        fit,
    );
    let more = tab(
        labels[4],
        more_active,
        Message::MoreMenuToggled,
        metrics,
        fit,
    );
    let settings = tab(
        labels[5],
        state.screen == Screen::Settings,
        Message::ScreenSelected(Screen::Settings),
        metrics,
        fit,
    );

    let group_gap = (fit.gap * 2.0).max(4.0);
    let bar = container(
        row![
            row![view].spacing(fit.gap),
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

    let subtabs = row(TASKS_LABELS
        .iter()
        .zip([
            Screen::Overdue,
            Screen::Inbox,
            Screen::Triaged,
            Screen::Backlog,
        ])
        .map(|(label, screen)| {
            tab(
                label,
                state.screen == screen,
                Message::ScreenSelected(screen),
                metrics,
                fit,
            )
        }))
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
    let tasks_row = if matches!(
        state.screen,
        Screen::Overdue | Screen::Inbox | Screen::Triaged | Screen::Backlog
    ) {
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
        crate::ui::controls::primary
    } else {
        crate::ui::controls::secondary
    })
    .into()
}

/// The single Day/Week/Month switcher. Its two halves share one button shape:
/// the name selects the current view, while the chevron opens the dropdown.
fn view_tab<'a>(
    label: &'a str,
    select_view: Message,
    active: bool,
    metrics: &Metrics,
    fit: TabFit,
) -> Element<'a, Message> {
    let name = button(
        text(label)
            .size(fit.size)
            .wrapping(text::Wrapping::None)
            .center(),
    )
    .height(tab_height(metrics))
    .padding(Padding::from([0.0, fit.pad]))
    .on_press(select_view)
    .style(view_half_style(active));

    let arrow = button(text("▾").size(fit.size).center())
        .height(tab_height(metrics))
        .padding(Padding::from([0.0, fit.pad.max(4.0)]))
        .on_press(Message::ViewMenuToggled)
        .style(view_half_style(active));

    let divider = container(Space::new().height(tab_height(metrics) * 0.6))
        .width(1.0)
        .style(|theme: &iced::Theme| container::Style {
            background: Some(
                theme
                    .extended_palette()
                    .background
                    .strong
                    .color
                    .scale_alpha(0.35)
                    .into(),
            ),
            ..Default::default()
        });

    container(row![name, divider, arrow].spacing(0.0).align_y(Center))
        .clip(true)
        .style(view_button_style(active))
        .into()
}

/// The shared background, border and shadow behind both halves of the switcher.
fn view_button_style(active: bool) -> impl Fn(&iced::Theme) -> container::Style {
    move |theme: &iced::Theme| {
        let palette = theme.extended_palette();
        let base = if active {
            palette.primary.base
        } else {
            palette.secondary.base
        };

        container::Style {
            background: Some(base.color.into()),
            text_color: Some(base.text),
            border: iced::Border {
                color: palette.background.strong.color.scale_alpha(0.65),
                width: 1.0,
                radius: 2.0.into(),
            },
            shadow: iced::Shadow {
                color: iced::Color::BLACK.scale_alpha(0.17),
                offset: iced::Vector::new(0.0, 2.5),
                blur_radius: 4.5,
            },
            ..Default::default()
        }
    }
}

/// A transparent button half that tints when hovered or pressed, so each side
/// of the switcher responds on its own without breaking the shared shape.
fn view_half_style(active: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |theme: &iced::Theme, status: button::Status| {
        let palette = theme.extended_palette();
        let base = if active {
            palette.primary.base
        } else {
            palette.secondary.base
        };
        let background = match status {
            button::Status::Hovered => Some(iced::Color::WHITE.scale_alpha(0.08).into()),
            button::Status::Pressed => Some(iced::Color::BLACK.scale_alpha(0.10).into()),
            _ => None,
        };

        button::Style {
            background,
            text_color: base.text,
            ..Default::default()
        }
    }
}

/// The calendar view switcher's menu: Day, Week and Month, in a card below the
/// left-hand navigation group.
fn view_menu<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill))
        .on_press(Message::ViewMenuToggled);

    let mut panel = column![].spacing(4.0).width(Fill);
    for (label, selected, message) in [
        (
            "Day",
            !state.weekly && !state.monthly,
            Message::CalendarModeSelected(false),
        ),
        ("Week", state.weekly, Message::CalendarModeSelected(true)),
        ("Month", state.monthly, Message::CalendarMonthViewSelected),
    ] {
        panel = panel.push(
            button(text(label).size(metrics.label()).center().width(Fill))
                .width(Fill)
                .on_press(message)
                .style(if selected {
                    crate::ui::controls::primary
                } else {
                    crate::ui::controls::secondary
                }),
        );
    }

    let width = (metrics.size.width - metrics.padding * 2.0).clamp(96.0, 180.0);
    let card = container(panel)
        .width(Length::Fixed(width))
        .padding(6.0)
        .style(crate::ui::controls::surface);

    let aligned = column![
        Space::new().height(tabs_height(state, metrics) + metrics.spacing),
        container(row![card, Space::new().width(Fill)])
            .padding(Padding {
                left: metrics.padding,
                ..Default::default()
            })
            .width(Fill),
    ]
    .width(Fill)
    .align_x(Alignment::Start);

    let layers: Vec<Element<'a, Message>> = vec![dismiss.into(), aligned.into()];
    iced::widget::stack(layers).into()
}

/// The More menu: Travel, Habits and Keep, in a card below the menu bar.
fn more_menu<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill))
        .on_press(Message::MoreMenuToggled);

    let mut panel = column![].spacing(4.0).width(Fill);
    for (label, screen) in MORE_LABELS.iter().zip([
        Screen::Travel,
        Screen::Habits,
        Screen::KeepNotes,
        Screen::BrainDump,
    ]) {
        panel = panel.push(
            button(text(*label).size(metrics.label()).center().width(Fill))
                .width(Fill)
                .on_press(Message::ScreenSelected(screen))
                .style(if state.screen == screen {
                    crate::ui::controls::primary
                } else {
                    crate::ui::controls::secondary
                }),
        );
    }

    let width = (metrics.size.width - metrics.padding * 2.0).clamp(96.0, 180.0);
    let card = container(panel)
        .width(Length::Fixed(width))
        .padding(6.0)
        .style(crate::ui::controls::surface);

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
