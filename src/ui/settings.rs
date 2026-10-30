//! The settings tab and the timer-hits-zero menu.

use iced::widget::{
    button, column, container, pick_list, row, scrollable, slider, text, text_input, toggler,
};
use iced::{Center, Element, Fill, Length};

use crate::app::{LayoutField, Message, State};
use crate::calendar;
use crate::config;
use crate::integrations::accounts;

use super::layout::Metrics;

/// Room left for the scrollbar, which is drawn over the content rather than
/// beside it. Without this the right edge of every control sits underneath
/// it, which at 200px means visibly clipped labels.
const SCROLLBAR_GUTTER: f32 = 14.0;

/// A calendar offered in the default-calendar picker. The name is shown to
/// the user; the id is what gets saved as the default.
#[derive(Clone, PartialEq)]
struct CalendarOption {
    id: String,
    name: String,
}

impl std::fmt::Display for CalendarOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name)
    }
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    // There is one settings page rather than one per tab: everything is set
    // from the same place.
    let content = column![
        heading("Appearance", metrics),
        appearance(state, metrics),
        heading("View layout", metrics),
        view_layout(state, metrics),
        heading("Dock", metrics),
        dock(state, metrics),
        heading("Side panel", metrics),
        panel(state, metrics),
        heading("Windowing", metrics),
        windowing(state, metrics),
        heading("Application", metrics),
        restart(metrics),
        heading("Keyboard shortcuts", metrics),
        shortcuts(state, metrics),
        heading("Calendar view order", metrics),
        calendar_order(state, metrics),
        heading("Google Calendar", metrics),
        calendar(state, metrics),
        heading("Travel", metrics),
        super::travel::settings_section(state, metrics),
        heading("Account integrations", metrics),
        integrations(state, metrics),
    ]
    .spacing(metrics.spacing * 1.5)
    .padding(iced::Padding {
        right: SCROLLBAR_GUTTER,
        ..iced::Padding::ZERO
    })
    .width(Fill);

    column![
        super::controls::section_heading(
            "Settings",
            None,
            if metrics.breakpoint.is_compact() {
                18.0
            } else {
                23.0
            }
        ),
        scrollable(content).height(Fill).width(Fill),
    ]
    .spacing(metrics.spacing)
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill)
    .into()
}

fn heading<'a>(label: &'a str, metrics: &Metrics) -> Element<'a, Message> {
    super::controls::section_heading(label, None, metrics.label() + 3.0).into()
}

/// A labelled row that stacks vertically once there is no room for two
/// columns, which is what keeps every control reachable at 200px.
///
/// The label is owned rather than borrowed because most callers build it
/// from the value they are showing.
pub(super) fn field<'a>(
    label: impl Into<String>,
    control: Element<'a, Message>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let caption = text(label.into()).size(metrics.label());

    if metrics.breakpoint.is_compact() {
        column![caption, control].spacing(2.0).width(Fill).into()
    } else {
        row![caption.width(Length::FillPortion(2)), control]
            .spacing(metrics.spacing)
            .align_y(Center)
            .into()
    }
}

/// A chooser that survives a narrow window.
///
/// Dropdown overlays need more width than they have in compact mode, so there
/// the control becomes a button that cycles to the next option on each tap.
pub(super) fn choice<'a, T>(
    label: &'a str,
    options: Vec<T>,
    selected: Option<T>,
    on_select: impl Fn(T) -> Message + 'a,
    metrics: &Metrics,
) -> Element<'a, Message>
where
    T: ToString + PartialEq + Clone + 'a,
{
    if metrics.breakpoint.is_compact() {
        let shown = selected
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "-".to_owned());

        let next = next_option(&options, selected.as_ref());

        let control = button(text(shown).size(metrics.label()).center().width(Fill)).width(Fill);

        let control = match next {
            Some(value) => control.on_press(on_select(value)),
            None => control,
        };

        field(
            label,
            control.style(crate::ui::controls::secondary).into(),
            metrics,
        )
    } else {
        field(
            label,
            pick_list(options, selected, on_select)
                .text_size(metrics.label())
                .width(Fill)
                .into(),
            metrics,
        )
    }
}

/// The option after `selected`, wrapping around at the end.
fn next_option<T: PartialEq + Clone>(options: &[T], selected: Option<&T>) -> Option<T> {
    if options.is_empty() {
        return None;
    }

    let index = selected
        .and_then(|current| options.iter().position(|option| option == current))
        .map_or(0, |index| (index + 1) % options.len());

    options.get(index).cloned()
}

fn appearance<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let theme = choice(
        "Theme",
        state.catalog.names(),
        Some(state.config.appearance.theme.clone()),
        Message::ThemeSelected,
        metrics,
    );

    column![
        theme,
        switch(
            "Follow system light/dark",
            state.config.appearance.follow_system,
            Message::FollowSystemToggled,
            metrics,
        ),
        switch(
            "Always on top",
            state.config.window.always_on_top,
            Message::AlwaysOnTopToggled,
            metrics,
        ),
    ]
    .spacing(metrics.spacing)
    .into()
}

fn view_layout<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    column![
        text("Change the appearance of the application and its panels.")
            .size(metrics.label())
            .style(text::secondary),
        layout_number(
            "Calendar day width (px)",
            LayoutField::CalendarWidth,
            state,
            metrics
        ),
        layout_number(
            "Time block panel width (px)",
            LayoutField::TimeBlockWidth,
            state,
            metrics
        ),
        layout_number(
            "Task details width (px)",
            LayoutField::DetailWidth,
            state,
            metrics
        ),
        layout_number(
            "Screen margin (px, 0–30)",
            LayoutField::ScreenMargin,
            state,
            metrics
        ),
        layout_number(
            "Border radius (px, 0–30)",
            LayoutField::BorderRadius,
            state,
            metrics
        ),
    ]
    .spacing(metrics.spacing)
    .into()
}

fn dock<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let views = &state.config.views;

    column![
        container(switch(
            "Reserve screen space",
            views.dock_reserve_space,
            Message::DockReserveSpaceToggled,
            metrics,
        ))
        .id("dock-reservation")
        .width(Fill),
        text("Keep maximized windows beside the dock. Popups do not reserve space.")
            .size(metrics.label())
            .style(text::secondary),
        layout_number(
            "Corner radius (px, 0–30)",
            LayoutField::DockBorderRadius,
            state,
            metrics
        ),
        text("Change the appearance of the dock.")
            .size(metrics.label())
            .style(text::secondary),
        choice(
            "Screen edge",
            crate::config::DockSide::ALL.to_vec(),
            Some(views.dock_side),
            Message::DockSideSelected,
            metrics,
        ),
        layout_number(
            "Dock width (px, 2–40)",
            LayoutField::DockWidth,
            state,
            metrics
        ),
        layout_number(
            "Left/right margin (px, 0–100)",
            LayoutField::DockMargin,
            state,
            metrics,
        ),
        layout_number(
            "Outer border (first layer, px, 0–5)",
            LayoutField::DockBorderOuterWidth,
            state,
            metrics,
        ),
        dock_color_input(
            "Outer border color",
            0,
            views.dock_border_outer_color,
            state,
            metrics,
        ),
        layout_number(
            "Inner border (second layer, px, 0–5)",
            LayoutField::DockBorderInnerWidth,
            state,
            metrics,
        ),
        dock_color_input(
            "Inner border color",
            1,
            views.dock_border_inner_color,
            state,
            metrics,
        )
    ]
    .spacing(metrics.spacing)
    .into()
}

fn panel<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let views = &state.config.views;

    column![
        container(switch(
            "Reserve screen space",
            views.panel_reserve_space,
            Message::PanelReserveSpaceToggled,
            metrics,
        ))
        .id("panel-reservation")
        .width(Fill),
        text("Keep maximized windows beside the panel. Tooltips do not reserve space.")
            .size(metrics.label())
            .style(text::secondary),
        layout_number(
            "Corner radius (px, 0–30)",
            LayoutField::PanelBorderRadius,
            state,
            metrics
        ),
        text("Update the borders of the dock and panel.")
            .size(metrics.label())
            .style(text::secondary),
        layout_number(
            "Outer border (first layer, px, 0–5)",
            LayoutField::PanelBorderOuterWidth,
            state,
            metrics,
        ),
        panel_color_input(
            "Outer border color",
            0,
            views.panel_border_outer_color,
            state,
            metrics,
        ),
        layout_number(
            "Inner border (second layer, px, 0–5)",
            LayoutField::PanelBorderInnerWidth,
            state,
            metrics,
        ),
        panel_color_input(
            "Inner border color",
            1,
            views.panel_border_inner_color,
            state,
            metrics,
        )
    ]
    .spacing(metrics.spacing)
    .into()
}

fn dock_color_input<'a>(
    label: &str,
    layer: usize,
    current: crate::hex::HexColor,
    state: &'a State,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let input = state.dock_border_input.as_ref();
    border_color_input(
        label,
        "dock-border-color",
        layer,
        current,
        input,
        Message::DockBorderColorInput,
        Message::DockBorderColorSubmitted,
        metrics,
    )
}

fn panel_color_input<'a>(
    label: &str,
    layer: usize,
    current: crate::hex::HexColor,
    state: &'a State,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let input = state.panel_border_input.as_ref();
    border_color_input(
        label,
        "panel-border-color",
        layer,
        current,
        input,
        Message::PanelBorderColorInput,
        Message::PanelBorderColorSubmitted,
        metrics,
    )
}

#[allow(clippy::too_many_arguments)]
fn border_color_input<'a>(
    label: &str,
    id_prefix: &str,
    layer: usize,
    current: crate::hex::HexColor,
    edited: Option<&(usize, String)>,
    on_input: impl Fn(usize, String) -> Message + 'a,
    on_submit: impl Fn(usize) -> Message + 'a,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let shown = edited
        .filter(|(edited, _)| *edited == layer)
        .map_or_else(|| current.to_string(), |(_, input)| input.clone());
    let submitted = on_submit(layer);
    let input = text_input("#RRGGBB", &shown)
        .id(format!("{id_prefix}-{layer}"))
        .size(metrics.label())
        .width(Fill)
        .on_input(move |input| on_input(layer, input))
        .on_submit(submitted);

    field(label, input.into(), metrics)
}

fn restart<'a>(metrics: &Metrics) -> Element<'a, Message> {
    column![
        button(
            text("Restart and load the latest version")
                .size(metrics.label())
                .center()
                .width(Fill),
        )
        .width(Fill)
        .on_press(Message::RestartRequested)
    ]
    .spacing(metrics.spacing)
    .into()
}

fn windowing<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let views = &state.config.views;
    let mut section = column![choice(
        "When the window appears",
        crate::config::WindowAnimation::ALL.to_vec(),
        Some(views.window_animation),
        Message::WindowAnimationSelected,
        metrics,
    )]
    .spacing(metrics.spacing);
    if views.window_animation == crate::config::WindowAnimation::FadeIn {
        section = section.push(layout_number(
            "Fade duration (ms, 0–5000)",
            LayoutField::FadeDuration,
            state,
            metrics,
        ));
    }
    section
        .push(switch(
            "Dock hour lines",
            views.dock_hour_lines,
            Message::DockHourLinesToggled,
            metrics,
        ))
        .push(
            text("Customize the app's transition effects for KDE.")
                .size(metrics.label())
                .style(text::secondary),
        )
        .into()
}

fn layout_number<'a>(
    label: &str,
    kind: LayoutField,
    state: &'a State,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let value = kind.value(state);
    let step = match kind {
        LayoutField::CalendarWidth | LayoutField::TimeBlockWidth | LayoutField::DetailWidth => 10.0,
        LayoutField::ScreenMargin
        | LayoutField::BorderRadius
        | LayoutField::DockBorderRadius
        | LayoutField::PanelBorderRadius
        | LayoutField::DockWidth
        | LayoutField::DockMargin
        | LayoutField::DockBorderOuterWidth
        | LayoutField::DockBorderInnerWidth
        | LayoutField::PanelBorderOuterWidth
        | LayoutField::PanelBorderInnerWidth => 1.0,
        LayoutField::FadeDuration => 10.0,
    };
    let (min, max) = kind.bounds();
    let shown = state
        .view_layout_input
        .as_ref()
        .filter(|(field, _)| *field == kind)
        .map_or_else(|| format!("{value:.0}"), |(_, input)| input.clone());
    let decrement = button(
        text(format!("-{step:.0}"))
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .padding([4, 4])
    .width(38)
    .on_press_maybe((value > min).then(|| kind.changed((value - step).max(min))));
    let increment = button(
        text(format!("+{step:.0}"))
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .padding([4, 4])
    .width(38)
    .on_press_maybe((value < max).then(|| kind.changed((value + step).min(max))));
    let input = text_input("", &shown)
        .id(format!("layout-{kind:?}"))
        .size(metrics.label())
        .width(Fill)
        .on_input(move |input| Message::ViewLayoutInputChanged(kind, input))
        .on_submit(Message::ViewLayoutInputSubmitted(kind));
    field(
        label,
        row![decrement, input, increment]
            .spacing(3)
            .align_y(Center)
            .into(),
        metrics,
    )
}

fn shortcuts<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut section = column![
        text("Global shortcuts. Each action can have several, and any of them triggers it. Combine Ctrl, Alt, Shift or Meta with a letter, number or named key (e.g. Space or F2).")
            .size(metrics.label()).style(text::secondary),
    ].spacing(metrics.spacing * 1.5);
    for action in crate::panel::Action::ALL {
        section = section.push(shortcut_action(state, action, metrics));
    }
    indent(section.into(), metrics)
}

fn shortcut_action<'a>(
    state: &'a State,
    action: crate::panel::Action,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let editor = &state.shortcut_editors[action.index()];
    let bound = state.config.panel.shortcuts(action);

    let mut section = column![
        text(action.label()).size(metrics.label()),
        text(action.description())
            .size(metrics.label())
            .style(text::secondary),
    ]
    .spacing(metrics.spacing);

    if bound.is_empty() {
        section = section.push(
            text("No shortcut")
                .size(metrics.label())
                .style(text::secondary),
        );
    }
    for (index, shortcut) in bound.iter().enumerate() {
        let mut remove =
            button(text("Remove").size(metrics.label())).style(crate::ui::controls::secondary);
        if !editor.busy {
            remove = remove.on_press(Message::ShortcutRemoved(action, index));
        }
        section = section.push(
            row![
                text(shortcut.as_str()).size(metrics.label()).width(Fill),
                remove
            ]
            .spacing(metrics.spacing)
            .align_y(Center),
        );
    }

    let mut input = text_input("e.g. Meta+U", &editor.input).size(metrics.label());
    let mut add = button(
        text(if editor.busy {
            "Applying…"
        } else {
            "Add shortcut"
        })
        .size(metrics.label())
        .center()
        .width(Fill),
    )
    .width(Fill);
    if !editor.busy {
        input = input
            .on_input(move |value| Message::ShortcutInputChanged(action, value))
            .on_submit(Message::ShortcutAdded(action));
        if !editor.input.trim().is_empty() {
            add = add.on_press(Message::ShortcutAdded(action));
        }
    }
    section = section
        .push(field("New shortcut", input.into(), metrics))
        .push(add);

    if let Some(result) = &editor.result {
        section = section.push(match result {
            Ok(message) => text(message).size(metrics.label()).style(text::success),
            Err(error) => text(error).size(metrics.label()).style(text::danger),
        });
    }
    section.into()
}

fn calendar_order<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let sources = state.calendar.display_sources(&state.config.calendar);
    let mut section = column![
        text("Top to bottom sets left to right when events overlap. App-managed task lists are omitted.")
            .size(metrics.label()).style(text::secondary),
    ].spacing(metrics.spacing).width(Fill);
    for (index, source) in sources.iter().enumerate() {
        let mut up = button(text("Up").size(metrics.label())).style(crate::ui::controls::secondary);
        let mut down =
            button(text("Down").size(metrics.label())).style(crate::ui::controls::secondary);
        if index > 0 {
            up = up.on_press(Message::CalendarSourceMoved(source.id.clone(), true));
        }
        if index + 1 < sources.len() {
            down = down.on_press(Message::CalendarSourceMoved(source.id.clone(), false));
        }
        let controls = row![up, down].spacing(4.0);
        let entry: Element<'a, Message> = if metrics.breakpoint.is_compact() {
            column![text(source.name.clone()).size(metrics.label()), controls]
                .spacing(2.0)
                .width(Fill)
                .into()
        } else {
            row![
                text(source.name.clone()).size(metrics.label()).width(Fill),
                controls
            ]
            .spacing(metrics.spacing)
            .align_y(Center)
            .into()
        };
        section = section.push(entry);
    }
    indent(section.into(), metrics)
}

fn calendar<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = &state.config.calendar;
    let connected = state.calendar.is_signed_in();

    let mut section = column![
    text("Create a Desktop OAuth client in Google Cloud; enable the Calendar API, Tasks API, Drive API, and the Gmail API. Allow http://127.0.0.1 as a redirect.")
        .size((metrics.label() - 1.0).max(10.0))
        .style(text::secondary),
        field(
            "OAuth client ID",
            text_input("Desktop client ID", &settings.client_id)
                .on_input(Message::GoogleClientIdChanged)
                .size(metrics.label())
                .into(),
            metrics,
        ),
        field(
            "Client secret",
            text_input("optional", &settings.client_secret)
                .on_input(Message::GoogleClientSecretChanged)
                .secure(true)
                .size(metrics.label())
                .into(),
            metrics,
        )
    ]
    .spacing(metrics.spacing);

    let action = if connected {
        let mut disconnect = button(
            text("Disconnect")
                .size(metrics.label())
                .center()
                .width(Fill),
        )
        .width(Fill)
        .style(crate::ui::controls::secondary);
        if !state.calendar.is_busy() {
            disconnect = disconnect.on_press(Message::GoogleDisconnectRequested);
        }
        disconnect
    } else {
        let mut connect = button(
            text("Connect Google Calendar")
                .size(metrics.label())
                .center()
                .width(Fill),
        )
        .width(Fill)
        .style(crate::ui::controls::primary);
        if !state.calendar.is_busy() {
            connect = connect.on_press(Message::GoogleConnectRequested);
        }
        connect
    };

    section = section.push(action);

    if let Some(error) = &state.calendar.error {
        section = section.push(
            text(error.clone())
                .size(metrics.label())
                .style(text::danger),
        );
    } else {
        section = section.push(
            text(state.calendar.status_line())
                .size(metrics.label())
                .style(text::secondary),
        );
    }

    let resync_label = if state.calendar.is_busy() {
        "Syncing…"
    } else {
        "Resync calendar and tasks data"
    };
    let mut resync = button(
        text(resync_label)
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill)
    .style(crate::ui::controls::secondary);
    if connected && !state.calendar.is_busy() {
        resync = resync.on_press(Message::CalendarResyncRequested);
    }
    section = section.push(resync);

    section = section.push(field(
        sync_label(settings.sync_minutes),
        slider(
            config::CalendarSettings::MIN_SYNC_MINUTES..=config::CalendarSettings::MAX_SYNC_MINUTES,
            settings.sync_minutes.clamp(
                config::CalendarSettings::MIN_SYNC_MINUTES,
                config::CalendarSettings::MAX_SYNC_MINUTES,
            ),
            Message::CalendarSyncMinutesChanged,
        )
        .into(),
        metrics,
    ));

    section = section.push(field(
        day_end_label(settings.day_start_hour()),
        slider(
            0..=config::CalendarSettings::MAX_DAY_END_HOUR,
            settings.day_start_hour(),
            Message::CalendarDayEndChanged,
        )
        .into(),
        metrics,
    ));

    section = section.push(field(
        format!("Weekly days: {} (+ previous day)", settings.weekly_days()),
        slider(
            7..=8,
            settings.weekly_days(),
            Message::CalendarWeeklyDaysChanged,
        )
        .into(),
        metrics,
    ));
    section = section.push(field(
        format!("Weekly day width: {} px", settings.weekly_day_width()),
        slider(
            100..=400,
            settings.weekly_day_width(),
            Message::CalendarWeeklyWidthChanged,
        )
        .into(),
        metrics,
    ));

    // The calendar new events are created on. Task lists are excluded since
    // they hold Google Tasks rather than calendar events.
    let options: Vec<calendar::Calendar> = state
        .calendar
        .calendars
        .iter()
        .filter(|calendar| !calendar.is_task_list())
        .cloned()
        .collect();

    let selected = options
        .iter()
        .find(|calendar| settings.default_calendar.as_deref() == Some(calendar.id.as_str()))
        .cloned();

    let option_map: Vec<CalendarOption> = options
        .iter()
        .map(|calendar| CalendarOption {
            id: calendar.id.clone(),
            name: calendar.name.clone(),
        })
        .collect();

    let selected_option = selected.map(|calendar| CalendarOption {
        id: calendar.id.clone(),
        name: calendar.name.clone(),
    });

    section = section.push(field(
        "Default calendar for new events",
        pick_list(option_map, selected_option, |option: CalendarOption| {
            Message::DefaultCalendarSelected(option.id)
        })
        .text_size(metrics.label())
        .width(Fill)
        .into(),
        metrics,
    ));

    section = section.push(heading("Time Blocking", metrics));
    {
        let mut options = vec![CalendarOption {
            id: String::new(),
            name: format!(
                "Default: {}",
                crate::config::CalendarSettings::DEFAULT_TIME_BLOCK_LIST
            ),
        }];
        options.extend(
            state
                .calendar
                .calendars
                .iter()
                .filter(|c| c.is_task_list())
                .map(|c| CalendarOption {
                    id: c.id.clone(),
                    name: c.name.clone(),
                }),
        );
        let selected = options
            .iter()
            .find(|o| settings.time_block_list.as_deref().unwrap_or("") == o.id)
            .cloned();
        section = section.push(field(
            "Google Tasks list for time-blocked and scheduled tasks",
            pick_list(options, selected, |o: CalendarOption| {
                Message::TimeBlocking(crate::timeblocking::Action::List(o.id))
            })
            .text_size(metrics.label())
            .width(Fill)
            .into(),
            metrics,
        ));
    }
    let mut backlog_options = vec![CalendarOption {
        id: String::new(),
        name: format!(
            "Default: {}",
            crate::config::CalendarSettings::DEFAULT_BACKLOG_LIST
        ),
    }];
    backlog_options.extend(
        state
            .calendar
            .calendars
            .iter()
            .filter(|c| c.is_task_list())
            .map(|c| CalendarOption {
                id: c.id.clone(),
                name: c.name.clone(),
            }),
    );
    let selected = backlog_options
        .iter()
        .find(|option| settings.backlog_list.as_deref().unwrap_or("") == option.id)
        .cloned();
    section = section.push(field(
        "Google Tasks list for backlog tasks",
        pick_list(backlog_options, selected, |option: CalendarOption| {
            Message::TimeBlocking(crate::timeblocking::Action::BacklogDestination(
                (!option.id.is_empty()).then_some(option.id),
            ))
        })
        .text_size(metrics.label())
        .width(Fill)
        .into(),
        metrics,
    ));
    let mut triaged_options = vec![CalendarOption {
        id: String::new(),
        name: format!(
            "Default: {}",
            crate::config::CalendarSettings::DEFAULT_TRIAGED_LIST
        ),
    }];
    triaged_options.extend(
        state
            .calendar
            .calendars
            .iter()
            .filter(|c| c.is_task_list())
            .map(|c| CalendarOption {
                id: c.id.clone(),
                name: c.name.clone(),
            }),
    );
    let selected = triaged_options
        .iter()
        .find(|option| settings.triaged_list.as_deref().unwrap_or("") == option.id)
        .cloned();
    section = section.push(field(
        "Google Tasks list for triaged tasks",
        pick_list(triaged_options, selected, |option: CalendarOption| {
            Message::TimeBlocking(crate::timeblocking::Action::TriagedDestination(
                (!option.id.is_empty()).then_some(option.id),
            ))
        })
        .text_size(metrics.label())
        .width(Fill)
        .into(),
        metrics,
    ));
    let mut inbox_options = vec![CalendarOption {
        id: String::new(),
        name: format!("Default: {}", crate::inbox::DEFAULT_LIST),
    }];
    inbox_options.extend(
        state
            .calendar
            .calendars
            .iter()
            .filter(|c| c.is_task_list())
            .map(|c| CalendarOption {
                id: c.id.clone(),
                name: c.name.clone(),
            }),
    );
    let selected = inbox_options
        .iter()
        .find(|option| settings.inbox_list.as_deref().unwrap_or("") == option.id)
        .cloned();
    section = section.push(field(
        "Google Tasks list for new tasks (the Inbox)",
        pick_list(inbox_options, selected, |option: CalendarOption| {
            Message::Inbox(crate::inbox::Action::List(
                (!option.id.is_empty()).then_some(option.id),
            ))
        })
        .text_size(metrics.label())
        .width(Fill)
        .into(),
        metrics,
    ));
    section = section.push(heading("Inbox triage", metrics));
    section = section.push(
        text(
            "A task cannot leave the Inbox until every field switched on here has been filled in.",
        )
        .size(metrics.label())
        .style(text::secondary),
    );
    section = section.push(switch(
        "Require an estimated time",
        settings.triage.estimate,
        |required| Message::TimeBlocking(crate::timeblocking::Action::RequireEstimate(required)),
        metrics,
    ));
    section = section.push(switch(
        "Require a project",
        settings.triage.project,
        |required| Message::TimeBlocking(crate::timeblocking::Action::RequireProject(required)),
        metrics,
    ));
    section = section.push(switch(
        "Require a priority",
        settings.triage.priority,
        |required| Message::TimeBlocking(crate::timeblocking::Action::RequirePriority(required)),
        metrics,
    ));
    section = section.push(heading("TimeSlay colors", metrics));
    section = section.push(
        text("Change the color of different types of events that is written to the calendar")
            .size(metrics.label())
            .style(text::secondary),
    );
    section = section.push(field(
        "Time Blocks",
        kind_color_picker(
            settings.time_block_color,
            Message::TimeBlockColorPicked,
            metrics,
        ),
        metrics,
    ));
    section = section.push(field(
        "Tasks",
        kind_color_picker(
            settings.task_event_color,
            Message::TaskEventColorPicked,
            metrics,
        ),
        metrics,
    ));
    section = section.push(field(
        "Habits",
        kind_color_picker(settings.habit_color, Message::HabitColorPicked, metrics),
        metrics,
    ));

    indent(section.into(), metrics)
}

/// A palette for one of the app calendar's event kinds, with a reset button
/// that puts the color back to its built-in default.
fn kind_color_picker<'a>(
    current: Option<crate::hex::HexColor>,
    pick: fn(Option<iced::Color>) -> Message,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let selected = current.map(|color| color.0);
    let mut swatches = column![].spacing(6);
    for chunk in calendar::colors::palette().chunks(6) {
        let mut line = row![].spacing(6);
        for &color in chunk {
            line = line.push(super::calendar::swatch_button(
                color,
                selected == Some(color),
                pick(Some(color)),
            ));
        }
        swatches = swatches.push(line);
    }
    let reset = button(text("Default").size(metrics.label() - 1.0))
        .width(Fill)
        .on_press(pick(None))
        .style(if selected.is_none() {
            crate::ui::controls::primary
        } else {
            crate::ui::controls::secondary
        });
    swatches.push(reset).into()
}

/// Services a pasted link can come from, and the accounts each one reads as.
fn integrations<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let accounts = &state.accounts;
    let small = (metrics.label() - 1.0).max(10.0);

    let mut section = column![
        text("Gmail").size(metrics.label()),
        text("Use to fetch the title and description of Gmail emails from their URL")
            .size(small)
            .style(text::secondary),
    ]
    .spacing(metrics.spacing);

    if accounts.emails.is_empty() {
        section = section.push(
            text("No extra accounts connected")
                .size(metrics.label())
                .style(text::secondary),
        );
    }
    for email in &accounts.emails {
        let remove = button(text("Remove").size(metrics.label()))
            .style(crate::ui::controls::secondary)
            .on_press(Message::Accounts(accounts::Action::Remove(email.clone())));
        let entry: Element<'a, Message> = if metrics.breakpoint.is_compact() {
            column![text(email.as_str()).size(metrics.label()), remove]
                .spacing(2.0)
                .width(Fill)
                .into()
        } else {
            row![
                text(email.as_str()).size(metrics.label()).width(Fill),
                remove
            ]
            .spacing(metrics.spacing)
            .align_y(Center)
            .into()
        };
        section = section.push(entry);
    }

    let mut add = button(
        text(if accounts.connecting {
            "Waiting for Google…"
        } else {
            "Add Google account"
        })
        .size(metrics.label())
        .center()
        .width(Fill),
    )
    .width(Fill)
    .style(crate::ui::controls::primary);
    if !accounts.connecting {
        add = add.on_press(Message::Accounts(accounts::Action::Add));
    }
    section = section.push(add);

    if let Some(error) = &accounts.error {
        section = section.push(
            text(error.as_str())
                .size(metrics.label())
                .style(text::danger),
        );
    }

    indent(section.into(), metrics)
}

fn sync_label(minutes: u32) -> String {
    match minutes {
        1 => "Sync every minute".to_owned(),
        n => format!("Sync every {n} minutes"),
    }
}

fn day_end_label(hour: u32) -> String {
    if hour == 0 {
        "Day ends at midnight".to_owned()
    } else {
        format!("Day ends at {}", calendar::hour_label(hour))
    }
}

pub(super) fn repeat_label(repeats: u8) -> String {
    match repeats {
        0 => "Repeat until dismissed".to_owned(),
        1 => "Play once".to_owned(),
        count => format!("Play {count} times"),
    }
}

pub(super) fn switch<'a>(
    label: &'a str,
    value: bool,
    on_toggle: impl Fn(bool) -> Message + 'a,
    metrics: &Metrics,
) -> Element<'a, Message> {
    toggler(value)
        .label(label)
        .text_size(metrics.label())
        .on_toggle(on_toggle)
        .width(Fill)
        .into()
}

/// Indents a subsection so the sliders visibly belong to their toggle.
pub(super) fn indent<'a>(content: Element<'a, Message>, metrics: &Metrics) -> Element<'a, Message> {
    container(content)
        .padding(iced::Padding {
            left: if metrics.breakpoint.is_compact() {
                0.0
            } else {
                6.0
            },
            ..iced::Padding::ZERO
        })
        .width(Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_switches_are_reachable_and_independent_at_narrow_widths() {
        use crate::{app, config::Config, theme::Catalog};
        use iced_test::{Simulator, core::Settings};

        for width in [180.0, 420.0] {
            let mut state = State::new(Config::default(), Catalog::built_in());
            let size = iced::Size::new(width, 400.0);
            let metrics = Metrics::of(size);
            let mut ui = Simulator::with_size(Settings::default(), size, dock(&state, &metrics));
            ui.click(iced::widget::Id::from("dock-reservation"))
                .unwrap();
            for message in ui.into_messages() {
                let _ = app::update(&mut state, message);
            }
            assert!(state.config.views.dock_reserve_space);
            assert!(!state.config.views.panel_reserve_space);

            let mut ui = Simulator::with_size(Settings::default(), size, panel(&state, &metrics));
            ui.click(iced::widget::Id::from("panel-reservation"))
                .unwrap();
            for message in ui.into_messages() {
                let _ = app::update(&mut state, message);
            }
            assert!(state.config.views.dock_reserve_space);
            assert!(state.config.views.panel_reserve_space);
        }
    }

    #[test]
    fn layout_number_buttons_step_and_stop_at_the_limits() {
        use crate::{app, config::Config, theme::Catalog};
        use iced_test::{Simulator, core::Settings};

        let mut state = State::new(Config::default(), Catalog::built_in());
        for (kind, step) in [
            (LayoutField::CalendarWidth, 10.0),
            (LayoutField::TimeBlockWidth, 10.0),
            (LayoutField::DetailWidth, 10.0),
            (LayoutField::ScreenMargin, 1.0),
            (LayoutField::DockWidth, 1.0),
            (LayoutField::DockBorderOuterWidth, 1.0),
            (LayoutField::DockBorderInnerWidth, 1.0),
            (LayoutField::PanelBorderOuterWidth, 1.0),
            (LayoutField::PanelBorderInnerWidth, 1.0),
        ] {
            for width in [180.0, 420.0] {
                let size = iced::Size::new(width, 100.0);
                let metrics = Metrics::of(size);
                for delta in [-step, step] {
                    let before = kind.value(&state);
                    let mut ui = Simulator::with_size(
                        Settings::default(),
                        size,
                        layout_number("Size (px)", kind, &state, &metrics),
                    );
                    let input = ui
                        .find(iced::widget::Id::from(format!("layout-{kind:?}")))
                        .unwrap()
                        .bounds();
                    assert!(
                        input.width >= 30.0,
                        "number must remain editable at {width}px"
                    );
                    assert!(input.x >= 0.0 && input.x + input.width <= width);
                    ui.click(format!("{delta:+.0}")).unwrap();
                    for message in ui.into_messages() {
                        let _ = app::update(&mut state, message);
                    }
                    assert_eq!(kind.value(&state), before + delta);
                }
            }
        }
        for (value, label) in [(0.0, "-1"), (30.0, "+1")] {
            state.config.views.margin = value;
            let size = iced::Size::new(180.0, 100.0);
            let mut ui = Simulator::with_size(
                Settings::default(),
                size,
                layout_number(
                    "Margin",
                    LayoutField::ScreenMargin,
                    &state,
                    &Metrics::of(size),
                ),
            );
            ui.click(label).unwrap();
            assert_eq!(ui.into_messages().count(), 0);
        }
    }

    #[test]
    fn cycling_wraps_around_the_options() {
        let options = vec!["a", "b", "c"];

        assert_eq!(next_option(&options, Some(&"a")), Some("b"));
        assert_eq!(next_option(&options, Some(&"c")), Some("a"));
    }

    #[test]
    fn cycling_from_nothing_selects_the_first_option() {
        let options = vec!["a", "b"];

        assert_eq!(next_option(&options, None), Some("a"));
    }

    #[test]
    fn cycling_an_empty_list_selects_nothing() {
        assert_eq!(next_option::<&str>(&[], None), None);
    }

    #[test]
    fn repeat_labels_read_naturally() {
        assert_eq!(repeat_label(0), "Repeat until dismissed");
        assert_eq!(repeat_label(1), "Play once");
        assert_eq!(repeat_label(3), "Play 3 times");
    }

    #[test]
    fn sync_labels_read_naturally() {
        assert_eq!(sync_label(1), "Sync every minute");
        assert_eq!(sync_label(15), "Sync every 15 minutes");
    }

    #[test]
    fn day_end_labels_read_naturally() {
        assert_eq!(day_end_label(0), "Day ends at midnight");
        assert_eq!(day_end_label(3), "Day ends at 3 AM");
        assert_eq!(day_end_label(5), "Day ends at 5 AM");
    }
}
