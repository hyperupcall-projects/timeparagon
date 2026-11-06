//! The settings tab and the timer-hits-zero menu.

use crate::calendar::first_day::FirstDayOfWeek;
use crate::calendar::time_format::TimeFormat;
use iced::widget::{button, column, container, row, text};
use iced::{Center, Element, Fill, Length};

use crate::app::{BorderLayer, LayoutField, Message, State};
use crate::app::{CalendarMessage, SettingsMessage};
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

#[derive(Clone, PartialEq)]
struct NumberOption {
    value: u32,
    label: String,
}

impl std::fmt::Display for NumberOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// Show every value in a dropdown, including in narrow panels.
fn number_dropdown<'a>(
    range: std::ops::RangeInclusive<u32>,
    selected: u32,
    label: impl Fn(u32) -> String,
    on_select: impl Fn(u32) -> Message + 'a,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let options: Vec<_> = range
        .map(|value| NumberOption {
            value,
            label: label(value),
        })
        .collect();
    let selected = options
        .iter()
        .find(|option| option.value == selected)
        .cloned();
    super::controls::pick_list(options, selected, move |option| on_select(option.value))
        .text_size(metrics.label())
        .width(Fill)
        .into()
}

fn week_view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = &state.config.calendar;
    column![choice(
        "Days shown",
        config::WeekRange::ALL.to_vec(),
        Some(settings.week_range()),
        |range| Message::Settings(SettingsMessage::CalendarWeekRangeChanged(range)),
        metrics,
    ),]
    .spacing(metrics.spacing)
    .width(Fill)
    .into()
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    // There is one settings page rather than one per tab: everything is set
    // from the same place.
    let content = column![
        section("Preferences", preferences(state, metrics), metrics),
        section(
            "Database sync",
            text(crate::database_sync::status())
                .size(metrics.label())
                .into(),
            metrics
        ),
        section("Appearance", appearance(state, metrics), metrics),
        section("Dock", dock(state, metrics), metrics),
        section("Side panel", panel(state, metrics), metrics),
        section("Overlays", overlays(state, metrics), metrics),
        section("Windowing", windowing(state, metrics), metrics),
        section("Keyboard shortcuts", shortcuts(state, metrics), metrics),
        section(
            "Calendar view order",
            calendar_order(state, metrics),
            metrics
        ),
        section("Google Calendar", calendar(state, metrics), metrics),
        section("Google Tasks", google_tasks(state, metrics), metrics),
        section("Tasks", tasks(state, metrics), metrics),
        section("Sleep", sleep(state, metrics), metrics),
        section(
            "Travel",
            super::travel::settings_section(state, metrics),
            metrics
        ),
        section("Voice input", crate::voice::settings(state), metrics),
        section(
            "Account integrations",
            integrations(state, metrics),
            metrics
        ),
        section("Application", restart(metrics), metrics),
    ]
    .spacing(metrics.spacing)
    .padding(iced::Padding {
        top: 2.0,
        left: 3.0,
        right: SCROLLBAR_GUTTER,
        bottom: 8.0,
    })
    .width(Fill);

    column![
        super::controls::section_heading(
            "Settings",
            None,
            if metrics.breakpoint.is_compact() {
                16.0
            } else {
                21.0
            }
        ),
        crate::ui::controls::scrollable(content)
            .height(Fill)
            .width(Fill),
    ]
    .spacing(metrics.spacing)
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill)
    .into()
}

fn heading<'a>(label: &'a str, metrics: &Metrics) -> Element<'a, Message> {
    super::controls::section_heading(label, None, metrics.label() + 1.0).into()
}

fn section<'a>(
    label: &'a str,
    body: Element<'a, Message>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    container(column![heading(label, metrics), body].spacing(metrics.spacing))
        .padding(metrics.padding)
        .width(Fill)
        .style(super::controls::card_surface)
        .into()
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
            crate::ui::controls::pick_list(options, selected, on_select)
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

fn preferences<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = &state.config.calendar;
    container(
        column![
            choice(
                "Time format",
                TimeFormat::ALL.to_vec(),
                Some(state.config.appearance.time_format),
                |v0| Message::Settings(SettingsMessage::TimeFormatSelected(v0)),
                metrics,
            ),
            choice(
                "First day of the week",
                FirstDayOfWeek::ALL.to_vec(),
                Some(settings.first_day_of_week),
                |v0| Message::Settings(SettingsMessage::FirstDayOfWeekSelected(v0)),
                metrics,
            ),
            field(
                day_end_label(
                    settings.day_start_hour(),
                    state.config.appearance.time_format,
                ),
                number_dropdown(
                    0..=config::CalendarSettings::MAX_DAY_END_HOUR,
                    settings.day_start_hour(),
                    |hour| {
                        if hour == 0 {
                            "Midnight".to_owned()
                        } else {
                            calendar::time_format::hour(hour, state.config.appearance.time_format)
                        }
                    },
                    |v0| Message::Settings(SettingsMessage::CalendarDayEndChanged(v0)),
                    metrics,
                ),
                metrics,
            ),
        ]
        .spacing(metrics.spacing),
    )
    .id("time-format")
    .width(Fill)
    .into()
}

fn appearance<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let views = &state.config.views;
    column![
        choice(
            "Theme",
            crate::theme::picker_names(&state.catalog),
            Some(crate::theme::selected_name(
                &state.catalog,
                &state.config.appearance.theme,
            )),
            |v0| Message::Settings(SettingsMessage::ThemeSelected(v0)),
            metrics,
        ),
        layout_number(
            "Outer border depth (px, 0–5)",
            LayoutField::PanelBorderOuterWidth,
            state,
            metrics
        ),
        panel_color_input(
            "Outer border color",
            BorderLayer::Outer,
            views.panel_border_outer_color,
            state,
            metrics
        ),
        layout_number(
            "Inner border depth (px, 0–5)",
            LayoutField::PanelBorderInnerWidth,
            state,
            metrics
        ),
        panel_color_input(
            "Inner border color",
            BorderLayer::Inner,
            views.panel_border_inner_color,
            state,
            metrics
        ),
    ]
    .spacing(metrics.spacing)
    .into()
}

fn subsection<'a>(label: &'a str, metrics: &Metrics) -> Element<'a, Message> {
    text(label)
        .size(metrics.label() + 1.0)
        .font(iced::Font {
            weight: iced::font::Weight::Bold,
            ..iced::Font::DEFAULT
        })
        .into()
}

fn dock<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let views = &state.config.views;

    column![
        container(switch(
            "Reserve screen space",
            views.dock_reserve_space,
            |v0| Message::Settings(SettingsMessage::DockReserveSpaceToggled(v0)),
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
        container(switch(
            "Dock hour lines",
            views.dock_hour_lines,
            |v0| Message::Settings(SettingsMessage::DockHourLinesToggled(v0)),
            metrics,
        ))
        .id("dock-hour-lines")
        .width(Fill),
        choice(
            "Screen edge",
            crate::config::DockSide::ALL.to_vec(),
            Some(views.dock_side),
            |v0| Message::Settings(SettingsMessage::DockSideSelected(v0)),
            metrics,
        ),
        layout_number(
            "Dock width (px, 2–40)",
            LayoutField::DockWidth,
            state,
            metrics
        ),
        layout_number(
            "Left margin (px, 0–100)",
            LayoutField::DockLeftMargin,
            state,
            metrics,
        ),
        layout_number(
            "Right margin (px, 0–100)",
            LayoutField::DockRightMargin,
            state,
            metrics,
        ),
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
            |v0| Message::Settings(SettingsMessage::PanelReserveSpaceToggled(v0)),
            metrics,
        ))
        .id("panel-reservation")
        .width(Fill),
        text("Keep maximized windows beside the panel. Tooltips do not reserve space.")
            .size(metrics.label())
            .style(text::secondary),
        layout_number(
            "Screen margin (px, 0–30)",
            LayoutField::ScreenMargin,
            state,
            metrics,
        ),
        layout_number(
            "Corner radius (px, 0–30)",
            LayoutField::PanelBorderRadius,
            state,
            metrics
        ),
        subsection("Day", metrics),
        layout_number(
            "Calendar day width (px)",
            LayoutField::CalendarWidth,
            state,
            metrics
        ),
        subsection("Week", metrics),
        layout_number(
            "Calendar week width (px)",
            LayoutField::CalendarWeekWidth,
            state,
            metrics
        ),
        week_view(state, metrics),
        subsection("Month", metrics),
        layout_number(
            "Calendar month width (px)",
            LayoutField::CalendarMonthWidth,
            state,
            metrics
        ),
    ]
    .spacing(metrics.spacing)
    .into()
}

fn overlays<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    column![
        subsection("Event/Habit/Task overlay", metrics),
        layout_number(
            "Event/Habit/Task details width (px)",
            LayoutField::DetailWidth,
            state,
            metrics
        ),
        subsection("Time block overlay", metrics),
        layout_number(
            "Time block panel width (px)",
            LayoutField::TimeBlockWidth,
            state,
            metrics
        ),
    ]
    .spacing(metrics.spacing)
    .into()
}

fn panel_color_input<'a>(
    label: &str,
    layer: BorderLayer,
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
        |v0, v1| Message::Settings(SettingsMessage::PanelBorderColorInput(v0, v1)),
        |v0| Message::Settings(SettingsMessage::PanelBorderColorSubmitted(v0)),
        metrics,
    )
}

#[allow(clippy::too_many_arguments)]
fn border_color_input<'a>(
    label: &str,
    id_prefix: &str,
    layer: BorderLayer,
    current: crate::hex::HexColor,
    edited: Option<&(BorderLayer, String)>,
    on_input: impl Fn(BorderLayer, String) -> Message + 'a,
    on_submit: impl Fn(BorderLayer) -> Message + 'a,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let shown = edited
        .filter(|(edited, _)| *edited == layer)
        .map_or_else(|| current.to_string(), |(_, input)| input.clone());
    let submitted = on_submit(layer);
    let input = crate::ui::controls::text_input("#RRGGBB", &shown)
        .id(format!(
            "{id_prefix}-{}",
            match layer {
                BorderLayer::Outer => 0,
                BorderLayer::Inner => 1,
            }
        ))
        .size(metrics.label())
        .width(Fill)
        .on_input(move |input| on_input(layer, input))
        .on_submit(submitted);

    field(label, input.into(), metrics)
}

fn restart<'a>(metrics: &Metrics) -> Element<'a, Message> {
    column![
        crate::ui::controls::action_button(
            text("Restart and load the latest version")
                .size(metrics.label())
                .center()
                .width(Fill),
        )
        .width(Fill)
        .on_press(Message::Settings(SettingsMessage::RestartRequested))
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
        |v0| Message::Settings(SettingsMessage::WindowAnimationSelected(v0)),
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
        LayoutField::CalendarWidth
        | LayoutField::CalendarWeekWidth
        | LayoutField::CalendarMonthWidth
        | LayoutField::TimeBlockWidth
        | LayoutField::DetailWidth => 10.0,
        LayoutField::ScreenMargin
        | LayoutField::BorderRadius
        | LayoutField::DockBorderRadius
        | LayoutField::PanelBorderRadius
        | LayoutField::DockWidth
        | LayoutField::DockLeftMargin
        | LayoutField::DockRightMargin
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
    let input = crate::ui::controls::text_input("", &shown)
        .id(format!("layout-{kind:?}"))
        .size(metrics.label())
        .on_input(move |input| {
            Message::Settings(SettingsMessage::ViewLayoutInputChanged(kind, input))
        })
        .on_submit(Message::Settings(
            SettingsMessage::ViewLayoutInputSubmitted(kind),
        ));
    field(
        label,
        crate::ui::controls::stepper(
            input,
            metrics.label(),
            (value < max).then(|| kind.changed((value + step).min(max))),
            (value > min).then(|| kind.changed((value - step).max(min))),
        ),
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
            remove = remove.on_press(Message::Settings(SettingsMessage::ShortcutRemoved(
                action, index,
            )));
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

    let mut input =
        crate::ui::controls::text_input("e.g. Meta+U", &editor.input).size(metrics.label());
    let mut add = crate::ui::controls::action_button(
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
            .on_input(move |value| {
                Message::Settings(SettingsMessage::ShortcutInputChanged(action, value))
            })
            .on_submit(Message::Settings(SettingsMessage::ShortcutAdded(action)));
        if !editor.input.trim().is_empty() {
            add = add.on_press(Message::Settings(SettingsMessage::ShortcutAdded(action)));
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
            up = up.on_press(Message::Calendar(CalendarMessage::CalendarSourceMoved(
                source.id.clone(),
                true,
            )));
        }
        if index + 1 < sources.len() {
            down = down.on_press(Message::Calendar(CalendarMessage::CalendarSourceMoved(
                source.id.clone(),
                false,
            )));
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

fn sleep<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = &state.config.calendar;
    let time_format = state.config.appearance.time_format;

    column![
        field(
            format!("Sleep goal: {} hours", settings.sleep_goal_hours()),
            number_dropdown(
                6..=12,
                settings.sleep_goal_hours(),
                |hours| format!("{hours} hours"),
                |v0| Message::Settings(SettingsMessage::SleepGoalChanged(v0)),
                metrics,
            ),
            metrics,
        ),
        field(
            format!(
                "Sleep day starts at {}",
                calendar::time_format::hour(settings.sleep_day_start_hour(), time_format)
            ),
            number_dropdown(
                0..=6,
                settings.sleep_day_start_hour(),
                |hour| calendar::time_format::hour(hour, time_format),
                |v0| Message::Settings(SettingsMessage::SleepDayStartChanged(v0)),
                metrics,
            ),
            metrics,
        ),
        field(
            format!(
                "Latest wake-up: {}",
                calendar::time_format::hour(settings.latest_wakeup_hour(), time_format)
            ),
            number_dropdown(
                4..=14,
                settings.latest_wakeup_hour(),
                |hour| calendar::time_format::hour(hour, time_format),
                |v0| Message::Settings(SettingsMessage::LatestWakeupChanged(v0)),
                metrics,
            ),
            metrics,
        ),
    ]
    .spacing(metrics.spacing)
    .into()
}

fn calendar<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = &state.config.calendar;
    let connected = state.calendar.is_signed_in();

    let mut section = column![
    text("Create a Desktop OAuth client in Google Cloud; enable the Calendar API, Tasks API, and the Gmail API. Allow http://127.0.0.1 as a redirect.")
        .size((metrics.label() - 1.0).max(10.0))
        .style(text::secondary),
        field(
            "OAuth client ID",
            crate::ui::controls::text_input("Desktop client ID", &settings.client_id)
                .on_input(|v0| Message::Settings(SettingsMessage::GoogleClientIdChanged(v0)))
                .size(metrics.label())
                .into(),
            metrics,
        ),
        field(
            "Client secret",
            crate::ui::controls::text_input("optional", &settings.client_secret)
                .on_input(|v0| Message::Settings(SettingsMessage::GoogleClientSecretChanged(v0)))
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
            disconnect = disconnect.on_press(Message::Settings(
                SettingsMessage::GoogleDisconnectRequested,
            ));
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
            connect = connect.on_press(Message::Settings(SettingsMessage::GoogleConnectRequested));
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
        resync = resync.on_press(Message::Calendar(CalendarMessage::CalendarResyncRequested));
    }
    section = section.push(resync);

    let selected = SyncInterval(settings.sync_minutes.clamp(
        config::CalendarSettings::MIN_SYNC_MINUTES,
        config::CalendarSettings::MAX_SYNC_MINUTES,
    ));
    let mut intervals = vec![1, 5, 15, 30, 60, 120, 240, 480, 720, 1440];
    intervals.push(selected.0);
    intervals.sort_unstable();
    intervals.dedup();
    section = section.push(choice(
        "Automatic sync",
        intervals.into_iter().map(SyncInterval).collect(),
        Some(selected),
        |interval| Message::Settings(SettingsMessage::CalendarSyncMinutesChanged(interval.0)),
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
        crate::ui::controls::pick_list(option_map, selected_option, |option: CalendarOption| {
            Message::Settings(SettingsMessage::DefaultCalendarSelected(option.id))
        })
        .text_size(metrics.label())
        .width(Fill)
        .into(),
        metrics,
    ));

    indent(section.into(), metrics)
}

/// The Google Tasks lists the app files tasks into. Each task list is chosen
/// here: where scheduled tasks, backlog, triaged tasks and new tasks land.
fn google_tasks<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = &state.config.calendar;
    let task_lists: Vec<CalendarOption> = state
        .calendar
        .calendars
        .iter()
        .filter(|calendar| calendar.is_task_list())
        .map(|calendar| CalendarOption {
            id: calendar.id.clone(),
            name: calendar.name.clone(),
        })
        .collect();

    let mut section = column![].spacing(metrics.spacing);

    let mut options = vec![CalendarOption {
        id: String::new(),
        name: format!(
            "Default: {}",
            crate::config::CalendarSettings::DEFAULT_TIME_BLOCK_LIST
        ),
    }];
    options.extend(task_lists.clone());
    let selected = options
        .iter()
        .find(|option| settings.time_block_list.as_deref().unwrap_or("") == option.id)
        .cloned();
    section = section.push(field(
        "Google Tasks list for time-blocked and scheduled tasks",
        crate::ui::controls::pick_list(options, selected, |option: CalendarOption| {
            Message::TimeBlocking(crate::timeblocking::Action::List(option.id))
        })
        .text_size(metrics.label())
        .width(Fill)
        .into(),
        metrics,
    ));

    let mut options = vec![CalendarOption {
        id: String::new(),
        name: format!(
            "Default: {}",
            crate::config::CalendarSettings::DEFAULT_BACKLOG_LIST
        ),
    }];
    options.extend(task_lists.clone());
    let selected = options
        .iter()
        .find(|option| settings.backlog_list.as_deref().unwrap_or("") == option.id)
        .cloned();
    section = section.push(field(
        "Google Tasks list for backlog tasks",
        crate::ui::controls::pick_list(options, selected, |option: CalendarOption| {
            Message::TimeBlocking(crate::timeblocking::Action::BacklogDestination(
                (!option.id.is_empty()).then_some(option.id),
            ))
        })
        .text_size(metrics.label())
        .width(Fill)
        .into(),
        metrics,
    ));

    let mut options = vec![CalendarOption {
        id: String::new(),
        name: format!(
            "Default: {}",
            crate::config::CalendarSettings::DEFAULT_TRIAGED_LIST
        ),
    }];
    options.extend(task_lists.clone());
    let selected = options
        .iter()
        .find(|option| settings.triaged_list.as_deref().unwrap_or("") == option.id)
        .cloned();
    section = section.push(field(
        "Google Tasks list for triaged tasks",
        crate::ui::controls::pick_list(options, selected, |option: CalendarOption| {
            Message::TimeBlocking(crate::timeblocking::Action::TriagedDestination(
                (!option.id.is_empty()).then_some(option.id),
            ))
        })
        .text_size(metrics.label())
        .width(Fill)
        .into(),
        metrics,
    ));

    let mut options = vec![CalendarOption {
        id: String::new(),
        name: format!("Default: {}", crate::inbox::DEFAULT_LIST),
    }];
    options.extend(task_lists);
    let selected = options
        .iter()
        .find(|option| settings.inbox_list.as_deref().unwrap_or("") == option.id)
        .cloned();
    section = section.push(field(
        "Google Tasks list for new tasks (the Inbox)",
        crate::ui::controls::pick_list(options, selected, |option: CalendarOption| {
            Message::Inbox(crate::inbox::Action::List(
                (!option.id.is_empty()).then_some(option.id),
            ))
        })
        .text_size(metrics.label())
        .width(Fill)
        .into(),
        metrics,
    ));

    indent(section.into(), metrics)
}

/// What a task needs before it can leave the Inbox. A task cannot be triaged
/// until every field switched on here has been filled in.
fn tasks<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = &state.config.calendar;
    let mut section = column![
        text(
            "A task cannot leave the Inbox until every field switched on here has been filled in.",
        )
        .size(metrics.label())
        .style(text::secondary),
    ]
    .spacing(metrics.spacing);
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
    indent(section.into(), metrics)
}

/// Services a pasted link can come from, and the accounts each one reads as.
fn integrations<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let accounts = &state.accounts;
    let small = (metrics.label() - 1.0).max(10.0);

    let mut section = column![
        text("Gmail").size(metrics.label()),
        text("Use to fetch the title and description of Gmail emails from their URL, and to list contacts under People")
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

#[derive(Clone, Copy, PartialEq)]
struct SyncInterval(u32);

impl std::fmt::Display for SyncInterval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&sync_label(self.0))
    }
}

fn sync_label(minutes: u32) -> String {
    match minutes {
        1 => "Sync every minute".to_owned(),
        60 => "Sync every hour".to_owned(),
        n if n % 60 == 0 => format!("Sync every {} hours", n / 60),
        n => format!("Sync every {n} minutes"),
    }
}

fn day_end_label(hour: u32, time_format: TimeFormat) -> String {
    if hour == 0 {
        "Day ends at midnight".to_owned()
    } else {
        format!(
            "Day ends at {}",
            calendar::time_format::hour(hour, time_format)
        )
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
    crate::ui::controls::toggler(value)
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
    fn sleep_settings_live_in_their_own_section() {
        use iced_test::{Simulator, core::Settings};

        let state = State::blank();
        let size = iced::Size::new(420.0, 1200.0);
        let metrics = Metrics::of(size);
        for label in [
            "Sleep goal: 8 hours",
            "Sleep day starts at 00:00",
            "Latest wake-up: 08:00",
        ] {
            let mut ui = Simulator::with_size(Settings::default(), size, sleep(&state, &metrics));
            assert!(ui.find(label).is_ok(), "missing {label}");
        }

        let mut ui = Simulator::with_size(Settings::default(), size, calendar(&state, &metrics));
        assert!(ui.find("Sleep goal: 8 hours").is_err());
    }

    #[test]
    fn task_controls_live_in_their_sections() {
        use iced_test::{Simulator, core::Settings};

        let state = State::blank();
        let size = iced::Size::new(420.0, 2400.0);
        let metrics = Metrics::of(size);

        // The default Google Tasks lists belong to the Google Tasks section.
        let mut ui =
            Simulator::with_size(Settings::default(), size, google_tasks(&state, &metrics));
        for label in [
            "Google Tasks list for time-blocked and scheduled tasks",
            "Google Tasks list for backlog tasks",
            "Google Tasks list for triaged tasks",
            "Google Tasks list for new tasks (the Inbox)",
        ] {
            assert!(ui.find(label).is_ok(), "missing {label}");
        }

        // The triage requirements belong to the Tasks section.
        let mut ui = Simulator::with_size(Settings::default(), size, tasks(&state, &metrics));
        let triage_hint =
            "A task cannot leave the Inbox until every field switched on here has been filled in.";
        assert!(ui.find(triage_hint).is_ok(), "missing {triage_hint}");

        // Neither is in Google Calendar, and the day end control moved out.
        let mut ui = Simulator::with_size(Settings::default(), size, calendar(&state, &metrics));
        assert!(ui.find("Google Tasks list for backlog tasks").is_err());
        assert!(ui.find(triage_hint).is_err());
        assert!(ui.find("Day ends at midnight").is_err());

        let mut ui = Simulator::with_size(Settings::default(), size, preferences(&state, &metrics));
        assert!(ui.find("Day ends at midnight").is_ok());
    }

    #[test]
    fn week_day_choices_show_the_start_day_and_switch_in_a_narrow_panel() {
        use crate::config::WeekRange;
        use iced_test::{Simulator, core::Settings};

        let size = iced::Size::new(200.0, 400.0);
        let metrics = Metrics::of(size);
        for (range, next) in [
            (WeekRange::SevenFromToday, WeekRange::EightFromYesterday),
            (WeekRange::EightFromYesterday, WeekRange::WholeWeek),
            (WeekRange::WholeWeek, WeekRange::SevenFromToday),
        ] {
            let mut state = State::blank();
            state.config.calendar.set_week_range(range);
            let label = range.to_string();
            let mut ui =
                Simulator::with_size(Settings::default(), size, week_view(&state, &metrics));
            ui.click(label.as_str()).unwrap();
            assert!(ui.into_messages().any(|message| matches!(
                message, Message::Settings(SettingsMessage::CalendarWeekRangeChanged(value)) if value == next
            )));
        }
    }

    #[test]
    fn reservation_switches_are_reachable_and_independent_at_narrow_widths() {
        use crate::app;
        use iced_test::{Simulator, core::Settings};

        for width in [180.0, 420.0] {
            let mut state = State::blank();
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
    fn margin_controls_live_in_their_respective_sections() {
        use iced_test::{Simulator, core::Settings};
        let state = State::blank();
        let size = iced::Size::new(420.0, 1800.0);
        let metrics = Metrics::of(size);
        let mut ui = Simulator::with_size(Settings::default(), size, appearance(&state, &metrics));
        assert!(ui.find("Screen margin (px, 0–30)").is_err());
        assert!(ui.find("Border radius (px, 0–30)").is_err());
        let mut ui = Simulator::with_size(Settings::default(), size, panel(&state, &metrics));
        assert!(ui.find("Screen margin (px, 0–30)").is_ok());
        let mut ui = Simulator::with_size(Settings::default(), size, dock(&state, &metrics));
        assert!(ui.find("Left margin (px, 0–100)").is_ok());
        assert!(ui.find("Right margin (px, 0–100)").is_ok());
    }

    #[test]
    fn dock_hour_lines_and_overlay_widths_live_in_their_sections() {
        use crate::app;
        use iced_test::{Simulator, core::Settings};

        let mut state = State::blank();
        let size = iced::Size::new(420.0, 800.0);
        let metrics = Metrics::of(size);

        let mut ui = Simulator::with_size(Settings::default(), size, dock(&state, &metrics));
        ui.click(iced::widget::Id::from("dock-hour-lines")).unwrap();
        for message in ui.into_messages() {
            let _ = app::update(&mut state, message);
        }
        assert!(!state.config.views.dock_hour_lines);

        let mut ui = Simulator::with_size(Settings::default(), size, windowing(&state, &metrics));
        assert!(ui.find(iced::widget::Id::from("dock-hour-lines")).is_err());

        let mut ui = Simulator::with_size(Settings::default(), size, overlays(&state, &metrics));
        for label in [
            "Event/Habit/Task overlay",
            "Event/Habit/Task details width (px)",
            "Time block overlay",
            "Time block panel width (px)",
        ] {
            assert!(ui.find(label).is_ok(), "missing {label}");
        }
    }

    #[test]
    fn layout_number_buttons_step_and_stop_at_the_limits() {
        use crate::app;
        use iced_test::{Simulator, core::Settings};

        let mut state = State::blank();
        state.config.views.dock_left_margin = Some(10.0);
        state.config.views.dock_right_margin = Some(20.0);
        for (kind, step) in [
            (LayoutField::CalendarWidth, 10.0),
            (LayoutField::CalendarWeekWidth, 10.0),
            (LayoutField::CalendarMonthWidth, 10.0),
            (LayoutField::TimeBlockWidth, 10.0),
            (LayoutField::DetailWidth, 10.0),
            (LayoutField::ScreenMargin, 1.0),
            (LayoutField::DockWidth, 1.0),
            (LayoutField::DockLeftMargin, 1.0),
            (LayoutField::DockRightMargin, 1.0),
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
                    ui.click(if delta > 0.0 { "+" } else { "−" }).unwrap();
                    for message in ui.into_messages() {
                        let _ = app::update(&mut state, message);
                    }
                    assert_eq!(kind.value(&state), before + delta);
                }
            }
        }
        for (value, label) in [(0.0, "−"), (30.0, "+")] {
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
    fn the_first_day_of_the_week_defaults_to_sunday_and_persists() {
        use crate::{app, config::Config, theme::Catalog};
        use iced_test::{Simulator, core::Settings};

        let old: Config = toml::from_str("[calendar]\nsync_minutes = 60\n").unwrap();
        assert_eq!(old.calendar.first_day_of_week, FirstDayOfWeek::Sunday);
        let mut state = State::new(old, Catalog::built_in());
        for expected in FirstDayOfWeek::ALL {
            let _ = app::update(
                &mut state,
                Message::Settings(SettingsMessage::FirstDayOfWeekSelected(expected)),
            );
            let saved = toml::to_string(&state.config).unwrap();
            let restored: Config = toml::from_str(&saved).unwrap();
            assert_eq!(restored.calendar.first_day_of_week, expected);
        }

        // In a narrow panel the choice is a button that moves to the next day.
        let size = iced::Size::new(180.0, 400.0);
        state.config.calendar.first_day_of_week = FirstDayOfWeek::Sunday;
        let mut ui = Simulator::with_size(
            Settings::default(),
            size,
            preferences(&state, &Metrics::of(size)),
        );
        ui.click("Sunday").unwrap();
        assert!(ui.into_messages().any(|message| matches!(
            message,
            Message::Settings(SettingsMessage::FirstDayOfWeekSelected(
                FirstDayOfWeek::Monday
            ))
        )));
    }

    #[test]
    fn time_format_choices_update_and_persist_the_preference() {
        use crate::{app, calendar::time_format::TimeFormat, config::Config, theme::Catalog};
        use iced_test::{Simulator, core::Settings};

        let old: Config = toml::from_str("[appearance]\ntheme = 'Sakura'\n").unwrap();
        assert_eq!(old.appearance.time_format, TimeFormat::TwentyFourHour);
        for (legacy, expected) in [
            (false, TimeFormat::TwelveHour),
            (true, TimeFormat::TwentyFourHour),
        ] {
            let migrated: Config =
                toml::from_str(&format!("[appearance]\nuse_24_hour = {legacy}\n")).unwrap();
            assert_eq!(migrated.appearance.time_format, expected);
        }
        let mut state = State::new(old, Catalog::built_in());
        for expected in TimeFormat::ALL {
            let _ = app::update(
                &mut state,
                Message::Settings(SettingsMessage::TimeFormatSelected(expected)),
            );
            assert_eq!(state.config.appearance.time_format, expected);
            let saved = toml::to_string(&state.config).unwrap();
            let restored: Config = toml::from_str(&saved).unwrap();
            assert_eq!(restored.appearance.time_format, expected);
        }
        for width in [180.0, 420.0] {
            let size = iced::Size::new(width, 400.0);
            let mut ui = Simulator::with_size(
                Settings::default(),
                size,
                preferences(&state, &Metrics::of(size)),
            );
            if width < 200.0 {
                ui.click("Compact Time").unwrap();
                assert!(ui.into_messages().any(|message| matches!(
                    message,
                    Message::Settings(SettingsMessage::TimeFormatSelected(TimeFormat::TwelveHour))
                )));
            } else {
                // Pick lists draw their labels directly, so operate the control by position.
                ui.point_at(iced::Point::new(width - 30.0, 15.0));
                ui.simulate([
                    iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(
                        iced::keyboard::Modifiers::CTRL,
                    )),
                    iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                        delta: iced::mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
                    }),
                ]);
                assert!(ui.into_messages().any(|message| matches!(
                    message,
                    Message::Settings(SettingsMessage::TimeFormatSelected(
                        TimeFormat::TwentyFourHour
                    ))
                )));
            }
        }
    }

    #[test]
    fn calendar_widths_are_independent_and_persist() {
        use crate::{app, config::Config};
        let mut state = State::blank();
        for (field, value) in [
            (LayoutField::CalendarWidth, 300.0),
            (LayoutField::CalendarWeekWidth, 900.0),
            (LayoutField::CalendarMonthWidth, 1100.0),
        ] {
            let _ = app::update(&mut state, field.changed(value));
        }
        let restored: Config = toml::from_str(&toml::to_string(&state.config).unwrap()).unwrap();
        assert_eq!(restored.views.calendar_width(200.0), 300.0);
        assert_eq!(restored.views.calendar_week_width(200.0), 900.0);
        assert_eq!(restored.views.calendar_month_width(), 1100.0);
    }

    #[test]
    fn appearance_border_edits_are_shared_with_the_dock() {
        use crate::app;
        let mut state = State::blank();
        let _ = app::update(
            &mut state,
            Message::Settings(SettingsMessage::PanelBorderOuterWidthChanged(3.0)),
        );
        let _ = app::update(
            &mut state,
            Message::Settings(SettingsMessage::PanelBorderInnerWidthChanged(2.0)),
        );
        let _ = app::update(
            &mut state,
            Message::Settings(SettingsMessage::PanelBorderColorInput(
                BorderLayer::Outer,
                "#123456".into(),
            )),
        );
        let _ = app::update(
            &mut state,
            Message::Settings(SettingsMessage::PanelBorderColorInput(
                BorderLayer::Inner,
                "#ABCDEF".into(),
            )),
        );
        let dock = crate::dock::config::DockConfig::load(&state.config.views);
        assert_eq!(
            dock.border_outer_width,
            state.config.views.panel_border_outer_width()
        );
        assert_eq!(
            dock.border_inner_width,
            state.config.views.panel_border_inner_width()
        );
        assert_eq!(
            dock.border_outer_color,
            state.config.views.panel_border_outer_color
        );
        assert_eq!(
            dock.border_inner_color,
            state.config.views.panel_border_inner_color
        );
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
        assert_eq!(sync_label(60), "Sync every hour");
        assert_eq!(sync_label(1440), "Sync every 24 hours");
    }

    #[test]
    fn day_end_labels_read_naturally() {
        assert_eq!(
            day_end_label(0, TimeFormat::TwelveHour),
            "Day ends at midnight"
        );
        assert_eq!(day_end_label(3, TimeFormat::TwelveHour), "Day ends at 3 AM");
        assert_eq!(day_end_label(5, TimeFormat::TwelveHour), "Day ends at 5 AM");
    }
}
