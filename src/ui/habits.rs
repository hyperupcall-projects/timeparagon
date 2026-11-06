//! The Habits tab, laid out as Loop Habit Tracker is.
//!
//! The list sits in the panel: a score ring, the name, then today and the three
//! days before it, each a button that moves that day through its states. A habit
//! opens beside it (or over it, with no room for a second window) as Loop's habit
//! screen: Overview, Score, History, Calendar, Best streaks and Frequency.

use chrono::{Datelike, Duration, NaiveDate};
use iced::widget::{Space, button, canvas, column, container, mouse_area, radio, row, stack, text};
use iced::{Alignment, Color, Element, Fill, Font, Theme, font, mouse};

use crate::app::{EventMessage, FormField, Message, State};
use crate::habits::{
    Action, Bucket, FIRST_WEEKDAY, Form, FormAction, FrequencyKind, Habit, Icons, Kind, Menu,
    Outcome, Section, SectionEdit, Series, SortKey, Status,
};
use timeslay_core::habits::{self as model, PALETTE_SIZE, palette_color};

use super::habits_charts::{self as charts, CalendarChart, DayMark, FrequencyChart};
use super::layout::Metrics;

/// Room for Loop's habit screen, which is laid out for a phone.
pub const PANEL_WIDTH: f32 = 420.0;

/// Today and the three days before it.
const DAY_COLUMNS: i64 = 4;
/// Room left for the scrollbar, which is drawn over the content.
const SCROLLBAR_GUTTER: f32 = 14.0;
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

fn msg(action: Action) -> Message {
    Message::Habits(action)
}

fn form_msg(action: FormAction) -> Message {
    Message::Habits(Action::Form(action))
}

fn colored(index: u8) -> impl Fn(&Theme) -> text::Style {
    move |theme: &Theme| text::Style {
        color: Some(palette_color(index, theme.extended_palette().is_dark)),
    }
}

fn medium(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(
            theme
                .extended_palette()
                .background
                .base
                .text
                .scale_alpha(0.6),
        ),
    }
}

// --------------------------------------------------------------------- list

/// Wide enough for a tick, narrow enough to leave the name some room.
fn day_width(metrics: &Metrics) -> f32 {
    ((metrics.size.width - metrics.padding * 2.0 - 90.0) / DAY_COLUMNS as f32).clamp(24.0, 40.0)
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    // With no second window to open it in, a habit takes the panel over.
    if state.editor_window.is_none() && state.habit_panel_open() {
        return panel(state, metrics);
    }
    let store = &state.habits;
    let today = store.today();
    let width = day_width(metrics);

    let title = text("Habits").size(metrics.label() + 8.0).font(Font {
        weight: font::Weight::Bold,
        ..Font::DEFAULT
    });
    let toolbar = row![
        container(title).width(Fill),
        icon_button(
            text("+").size(18).center().into(),
            "Add habit",
            msg(Action::New)
        ),
        icon_button(
            icon(Glyph::Filter),
            "Sort and filter",
            msg(Action::Menu(Menu::Filter))
        ),
        icon_button(icon(Glyph::Dots), "More", msg(Action::Menu(Menu::List))),
    ]
    .spacing(2)
    .align_y(Alignment::Center);

    let mut days = row![Space::new().width(Fill)].align_y(Alignment::Center);
    for back in 0..DAY_COLUMNS {
        let day = today - Duration::days(back);
        days = days.push(
            column![
                text(weekday_name(day).to_uppercase())
                    .size(metrics.label() - 3.0)
                    .style(medium),
                text(day.day().to_string())
                    .size(metrics.label() - 3.0)
                    .style(medium),
            ]
            .align_x(Alignment::Center)
            .width(width),
        );
    }

    let mut body = column![toolbar].spacing(metrics.spacing).width(Fill);
    if let Some(error) = &store.error {
        body = body.push(
            text(error.clone())
                .size(metrics.label())
                .style(text::danger),
        );
    }

    let visible = store.visible();
    if visible.is_empty() {
        body = body.push(
            text(if store.library.habits.is_empty() {
                "No habits yet. Press + to add one."
            } else {
                "No habits to show. Change the filter to see more."
            })
            .size(metrics.label())
            .style(text::secondary),
        );
    } else {
        body = body.push(container(days).padding(iced::Padding {
            right: SCROLLBAR_GUTTER + 4.0,
            ..iced::Padding::ZERO
        }));
    }

    let mut rows = column![].spacing(3).width(Fill);
    for (section, habits) in store.library.grouped(visible) {
        if let Some(section) = section {
            rows = rows.push(section_heading(state, section, habits.len(), metrics));
            if section.collapsed {
                continue;
            }
        }
        for habit in habits {
            rows = rows.push(habit_row(state, habit, today, width, metrics));
        }
    }
    if let Some(edit) = store.section_edit.as_ref().filter(|edit| edit.id.is_none()) {
        rows = rows.push(section_editor(edit, metrics));
    }
    let list = crate::ui::controls::scrollable(container(rows).padding(iced::Padding {
        right: SCROLLBAR_GUTTER,
        ..iced::Padding::ZERO
    }))
    .width(Fill)
    .height(Fill);
    body = body.push(list);

    let page = container(body)
        .padding(metrics.padding)
        .width(Fill)
        .height(Fill);

    match store.menu {
        Some(Menu::Filter) => menu_overlay(page.into(), filter_menu(state, metrics), metrics),
        Some(Menu::List) => menu_overlay(page.into(), list_menu(metrics), metrics),
        _ => page.into(),
    }
}

fn weekday_name(day: NaiveDate) -> &'static str {
    WEEKDAYS[day.weekday().num_days_from_monday() as usize]
}

/// A section's name, which folds it away, with its count and a way to edit it.
/// A habit dragged here joins the section.
fn section_heading<'a>(
    state: &'a State,
    section: &'a Section,
    count: usize,
    metrics: &Metrics,
) -> Element<'a, Message> {
    if let Some(edit) = &state.habits.section_edit
        && edit.id.as_deref() == Some(section.id.as_str())
    {
        return section_editor(edit, metrics);
    }
    let chevron = if section.collapsed {
        Glyph::ChevronRight
    } else {
        Glyph::ChevronDown
    };
    let name = text(&section.name)
        .size(metrics.label())
        .wrapping(text::Wrapping::None)
        .font(Font {
            weight: font::Weight::Bold,
            ..Font::DEFAULT
        });
    let fold = button(
        row![
            icon(chevron),
            container(name).clip(true),
            text(count.to_string())
                .size(metrics.label() - 1.0)
                .style(medium),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding([4, 4])
    .width(Fill)
    .style(crate::ui::controls::ghost)
    .on_press(msg(Action::ToggleSection(section.id.clone())));
    let heading = row![
        fold,
        icon_button(
            icon(Glyph::Pencil),
            "Edit section",
            msg(Action::EditSection(section.id.clone())),
        ),
    ]
    .align_y(Alignment::Center);
    mouse_area(container(heading).padding(iced::Padding {
        top: 6.0,
        ..iced::Padding::ZERO
    }))
    .on_enter(msg(Action::SectionEntered(section.id.clone())))
    .into()
}

/// Names a section in place. One that exists can also be moved or deleted, which
/// leaves its habits in the list.
fn section_editor<'a>(edit: &'a SectionEdit, metrics: &Metrics) -> Element<'a, Message> {
    let action = |label: &'static str, action: Action| {
        crate::ui::controls::quiet_button(text(label).size(metrics.label() - 1.0))
            .padding([2, 6])
            .on_press(msg(action))
    };
    let mut actions = row![
        action("Save", Action::SaveSection),
        action("Cancel", Action::CancelSection),
    ]
    .spacing(4);
    if let Some(id) = &edit.id {
        actions = actions
            .push(action("Up", Action::ShiftSection(id.clone(), true)))
            .push(action("Down", Action::ShiftSection(id.clone(), false)))
            .push(action("Delete", Action::DeleteSection(id.clone())));
    }
    container(
        column![
            crate::ui::controls::text_input("Section name", &edit.name)
                .id(crate::habits::SECTION_FIELD)
                .on_input(|name| msg(Action::SectionName(name)))
                .on_submit(msg(Action::SaveSection))
                .size(metrics.label()),
            actions.wrap().vertical_spacing(4),
        ]
        .spacing(6),
    )
    .padding(iced::Padding {
        top: 6.0,
        ..iced::Padding::ZERO
    })
    .width(Fill)
    .into()
}

fn habit_row<'a>(
    state: &'a State,
    habit: &'a Habit,
    today: NaiveDate,
    width: f32,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let store = &state.habits;
    let computed = store.computed(&habit.id);
    let score = computed.map_or(0.0, |computed| computed.score(today));
    let dragging = store
        .dragging
        .as_ref()
        .is_some_and(|drag| drag.id == habit.id);

    let name = text(&habit.name)
        .size(metrics.label() + 1.0)
        .wrapping(text::Wrapping::None)
        .style(colored(habit.color));
    let identity = row![
        charts::ring(score, habit.color, 18.0),
        container(name).width(Fill).clip(true),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    // Pressing a name is a click until the pointer reaches another row.
    let identity = mouse_area(container(identity).height(44).center_y(44).width(Fill))
        .on_press(msg(Action::RowPressed(habit.id.clone())))
        .on_release(msg(Action::RowReleased(habit.id.clone())))
        .on_enter(msg(Action::RowEntered(habit.id.clone())))
        .interaction(if store.dragging.is_some() {
            mouse::Interaction::Grabbing
        } else {
            mouse::Interaction::Pointer
        });

    let mut cells = row![container(identity).width(Fill)].align_y(Alignment::Center);
    for back in 0..DAY_COLUMNS {
        let day = today - Duration::days(back);
        let status = computed.map_or(Status::Unknown, |computed| computed.status(day));
        cells = cells.push(day_button(habit, day, status, width));
    }

    container(cells)
        .padding([0, 8])
        .width(Fill)
        .style(move |theme: &Theme| {
            let mut style = crate::ui::controls::card_surface(theme);
            if dragging {
                style.border.color = theme.extended_palette().primary.base.color;
                style.border.width = 1.0;
            }
            style
        })
        .into()
}

fn day_button<'a>(
    habit: &'a Habit,
    day: NaiveDate,
    status: Status,
    width: f32,
) -> Element<'a, Message> {
    let cell = button(
        canvas(DayMark {
            status,
            color: habit.color,
            goal: habit.frequency.per_day(),
            question_marks: habit.question_marks,
            icons: habit.icons,
            kind: habit.kind,
            times: habit.times_on(day),
            resisted: habit.resisted_on(day),
            value: habit.measure.as_ref().and(habit.value_on(day)),
        })
        .width(Fill)
        .height(Fill),
    )
    .padding(0)
    .width(width)
    .height(44)
    .style(crate::ui::controls::ghost)
    .on_press(msg(Action::OpenDay(habit.id.clone(), day)));
    mouse_area(cell)
        .on_right_press(msg(Action::OpenDay(habit.id.clone(), day)))
        .into()
}

// -------------------------------------------------------------------- menus

fn menu_overlay<'a>(
    page: Element<'a, Message>,
    menu: Element<'a, Message>,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let dismiss = mouse_area(container(Space::new()).width(Fill).height(Fill))
        .on_press(msg(Action::CloseMenu));
    let width = (metrics.size.width - metrics.padding * 2.0).clamp(120.0, 210.0);
    let card = container(menu)
        .width(width)
        .padding(6)
        .style(crate::ui::controls::surface);
    let placed = container(iced::widget::opaque(card))
        .padding(iced::Padding {
            top: 40.0,
            right: metrics.padding,
            ..iced::Padding::ZERO
        })
        .align_right(Fill);
    stack![page, dismiss, placed].into()
}

fn menu_item<'a>(
    label: impl Into<String>,
    action: Action,
    metrics: &Metrics,
) -> Element<'a, Message> {
    button(text(label.into()).size(metrics.label()))
        .width(Fill)
        .style(crate::ui::controls::ghost)
        .on_press(msg(action))
        .into()
}

fn filter_menu<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let library = &state.habits.library;
    let mut menu = column![
        crate::ui::controls::checkbox(library.hide_archived)
            .label("Hide archived")
            .text_size(metrics.label())
            .on_toggle(|hide| msg(Action::HideArchived(hide))),
        crate::ui::controls::checkbox(library.hide_completed)
            .label("Hide completed")
            .text_size(metrics.label())
            .on_toggle(|hide| msg(Action::HideCompleted(hide))),
        text("Sort").size(metrics.label() - 1.0).style(medium),
    ]
    .spacing(6)
    .width(Fill);
    for key in SortKey::ALL {
        let marker = if library.sort != key {
            ""
        } else if key == SortKey::Manual {
            " ✓"
        } else if library.reversed {
            " ↑"
        } else {
            " ↓"
        };
        menu = menu.push(menu_item(
            format!("{key}{marker}"),
            Action::Sort(key),
            metrics,
        ));
    }
    menu.into()
}

fn list_menu<'a>(metrics: &Metrics) -> Element<'a, Message> {
    column![
        menu_item("Add section", Action::NewSection, metrics),
        menu_item("Export as CSV", Action::Export(None), metrics),
    ]
    .spacing(2)
    .into()
}

// ------------------------------------------------------------- the habit page

/// The pane beside the list: the add and edit page, or a habit's page.
pub fn panel<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    if let Some(form) = &state.habits.form {
        return form_page(state, form, metrics);
    }
    let Some(habit) = state.habits.selected_habit() else {
        return Space::new().into();
    };
    habit_page(state, habit, metrics)
}

fn card<'a>(content: Element<'a, Message>) -> Element<'a, Message> {
    container(content)
        .padding([12, 14])
        .width(Fill)
        .style(crate::ui::controls::card_surface)
        .into()
}

fn card_title<'a>(title: &'a str, color: u8, metrics: &Metrics) -> Element<'a, Message> {
    text(title)
        .size(metrics.label() + 3.0)
        .style(colored(color))
        .into()
}

/// A card title with Loop's period spinner beside it.
fn titled_spinner<'a>(
    title: &'a str,
    color: u8,
    options: &'static [Bucket],
    selected: Bucket,
    on_select: impl Fn(Bucket) -> Action + 'a,
    metrics: &Metrics,
) -> Element<'a, Message> {
    row![
        container(card_title(title, color, metrics)).width(Fill),
        crate::ui::controls::pick_list(options, Some(selected), move |bucket| msg(on_select(
            bucket
        )))
        .text_size(metrics.label()),
    ]
    .align_y(Alignment::Center)
    .into()
}

fn signed_percent(value: f64) -> String {
    let percent = (value * 100.0).round() as i64;
    if percent > 0 {
        format!("+{percent}%")
    } else {
        format!("{percent}%")
    }
}

fn overview<'a>(state: &'a State, habit: &'a Habit, metrics: &Metrics) -> Element<'a, Message> {
    let today = state.habits.today();
    let computed = state.habits.computed(&habit.id);
    let score_on = |day| computed.map_or(0.0, |computed| computed.score(day));
    let now = score_on(today);
    let stat = |value: String, name: &'static str| {
        column![
            text(value)
                .size(metrics.label() + 2.0)
                .style(colored(habit.color)),
            text(name).size(metrics.label()).style(medium),
        ]
        .width(Fill)
        .spacing(1)
    };
    card(
        column![
            card_title("Overview", habit.color, metrics),
            row![
                container(charts::ring(now, habit.color, 44.0)).padding(iced::Padding {
                    right: 16.0,
                    left: 6.0,
                    ..iced::Padding::ZERO
                }),
                stat(format!("{}%", (now * 100.0).round() as i64), "Score"),
                stat(
                    signed_percent(now - score_on(today - Duration::days(30))),
                    "Month"
                ),
                stat(
                    signed_percent(now - score_on(today - Duration::days(365))),
                    "Year"
                ),
                stat(
                    model::format_amount(computed.map_or(0.0, |computed| computed.total())),
                    "Total"
                ),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(10)
        .into(),
    )
}

fn habit_page<'a>(state: &'a State, habit: &'a Habit, metrics: &Metrics) -> Element<'a, Message> {
    let store = &state.habits;
    let library = &store.library;
    let today = store.today();
    let color = habit.color;
    let empty = model::Computed::default();
    let computed = store.computed(&habit.id).unwrap_or(&empty);

    let score = charts::chart(
        charts::ScoreChart {
            series: model::score_series(computed, library.score_bucket, FIRST_WEEKDAY, today),
            bucket: library.score_bucket,
            color,
        },
        220.0,
    );
    let history = charts::chart(
        charts::HistoryChart {
            series: model::history_series(computed, library.history_bucket, FIRST_WEEKDAY, today),
            bucket: library.history_bucket,
            color,
            target: None,
        },
        200.0,
    );
    let calendar = charts::chart(
        CalendarChart {
            habit_id: habit.id.clone(),
            statuses: computed.clone(),
            color,
            today,
            first_weekday: FIRST_WEEKDAY,
            weekday_names: WEEKDAYS,
        },
        charts::CALENDAR_HEIGHT,
    );
    let frequency = charts::chart(
        FrequencyChart {
            computed: computed.clone(),
            color,
            today,
            weekday_names: WEEKDAYS,
            first_weekday: FIRST_WEEKDAY.num_days_from_monday() as usize,
        },
        charts::FREQUENCY_HEIGHT,
    );

    let streaks = computed.best_streaks(10);
    let streaks: Element<'a, Message> = if streaks.is_empty() {
        text("Nothing yet")
            .size(metrics.label())
            .style(text::secondary)
            .into()
    } else {
        let height = streaks.len() as f32 * charts::STREAK_ROW;
        charts::chart(charts::StreakChart { streaks, color }, height)
    };

    let mut subtitle = column![].spacing(4);
    if !habit.question.is_empty() {
        subtitle = subtitle.push(
            text(&habit.question)
                .size(metrics.label() + 2.0)
                .style(colored(color)),
        );
    }
    let mut goal = habit.goal_label();
    if habit.archived {
        goal.push_str(" · Archived");
    }
    subtitle = subtitle.push(
        row![
            canvas(GlyphIcon(Glyph::Calendar)).width(14).height(14),
            text(goal).size(metrics.label()).style(medium),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    );
    if let Some(measure) = &habit.measure {
        subtitle = subtitle.push(
            text(measure.label(habit.kind))
                .size(metrics.label())
                .style(medium),
        );
    }
    if !habit.notes.is_empty() {
        subtitle = subtitle.push(text(&habit.notes).size(metrics.label()).style(medium));
    }
    let subtitle = container(subtitle)
        .padding([10, 16])
        .width(Fill)
        .style(crate::ui::controls::summary_surface);

    let mut cards = column![
        overview(state, habit, metrics),
        card(crate::ui::notes::section(
            state,
            crate::notes::habit_link(&habit.id, habit.name.clone()),
            metrics,
        )),
        card(
            column![
                titled_spinner(
                    "Score",
                    color,
                    &Bucket::SCORE,
                    library.score_bucket,
                    Action::ScoreBucket,
                    metrics,
                ),
                score,
            ]
            .spacing(8)
            .into(),
        ),
        card(
            column![
                titled_spinner(
                    "History",
                    color,
                    &Bucket::HISTORY,
                    library.history_bucket,
                    Action::HistoryBucket,
                    metrics,
                ),
                history,
            ]
            .spacing(8)
            .into(),
        ),
    ]
    .spacing(3)
    .width(Fill);
    if let Some(measure) = &habit.measure {
        cards = cards.push(value_card(habit, measure, today, metrics));
    }
    cards = cards
        .push(card(
            column![card_title("Calendar", color, metrics), calendar]
                .spacing(8)
                .into(),
        ))
        .push(card(
            column![card_title("Best streaks", color, metrics), streaks]
                .spacing(8)
                .into(),
        ))
        .push(card(
            column![card_title("Frequency", color, metrics), frequency]
                .spacing(8)
                .into(),
        ));
    // A habit to avoid has nothing to schedule.
    if habit.kind != Kind::No {
        cards = cards.push(events_card(state, habit, metrics));
    }
    if let Some(error) = &store.error {
        cards = cards.push(
            container(
                text(error.clone())
                    .size(metrics.label())
                    .style(text::danger),
            )
            .padding(8),
        );
    }

    let page = column![
        page_header(&habit.name, color, habit_actions(habit)),
        crate::ui::controls::scrollable(column![subtitle, cards].spacing(0).padding(
            iced::Padding {
                right: SCROLLBAR_GUTTER,
                ..iced::Padding::ZERO
            }
        ))
        .height(Fill),
    ]
    .width(Fill)
    .height(Fill);

    if store.menu == Some(Menu::Habit) {
        menu_overlay(page.into(), habit_menu(habit, metrics), metrics)
    } else {
        page.into()
    }
}

/// The numbers recorded for a habit, added up over the target's period, with the
/// target as a line across them.
fn value_card<'a>(
    habit: &'a Habit,
    measure: &'a model::Measure,
    today: NaiveDate,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let series = model::value_series(habit, FIRST_WEEKDAY, today);
    let chart: Element<'a, Message> = if series.is_empty() {
        text("Nothing yet")
            .size(metrics.label())
            .style(text::secondary)
            .into()
    } else {
        charts::chart(
            charts::HistoryChart {
                series,
                bucket: measure.bucket(),
                color: habit.color,
                target: measure.target,
            },
            200.0,
        )
    };
    card(
        column![card_title("Value", habit.color, metrics), chart]
            .spacing(8)
            .into(),
    )
}

/// Loop's colored bar: back, the name, then the actions.
fn page_header<'a>(
    title: &'a str,
    color: u8,
    actions: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    let mut bar = row![
        header_button(Glyph::Back, "Back", msg(Action::Close), color),
        container(
            text(title)
                .size(18)
                .font(Font {
                    weight: font::Weight::Medium,
                    ..Font::DEFAULT
                })
                .wrapping(text::Wrapping::None)
                .color(Color::WHITE),
        )
        .width(Fill)
        .clip(true),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    for action in actions {
        bar = bar.push(action);
    }
    container(bar)
        .padding([6, 8])
        .width(Fill)
        .style(move |theme: &Theme| container::Style {
            // Loop's dark theme keeps the bar black, as a pale color would
            // leave its white text unreadable.
            background: Some(
                if theme.extended_palette().is_dark {
                    Color::BLACK
                } else {
                    palette_color(color, false)
                }
                .into(),
            ),
            ..Default::default()
        })
        .into()
}

fn habit_actions<'a>(habit: &'a Habit) -> Vec<Element<'a, Message>> {
    vec![
        header_button(
            Glyph::Pencil,
            "Edit",
            msg(Action::EditHabit(habit.id.clone())),
            habit.color,
        ),
        header_button(
            Glyph::Dots,
            "More",
            msg(Action::Menu(Menu::Habit)),
            habit.color,
        ),
    ]
}

fn habit_menu<'a>(habit: &'a Habit, metrics: &Metrics) -> Element<'a, Message> {
    column![
        menu_item(
            if habit.archived {
                "Unarchive"
            } else {
                "Archive"
            },
            Action::ToggleArchived(habit.id.clone()),
            metrics,
        ),
        menu_item(
            "Export as CSV",
            Action::Export(Some(habit.id.clone())),
            metrics,
        ),
        menu_item("Delete", Action::DeleteRequested(habit.id.clone()), metrics),
    ]
    .spacing(2)
    .into()
}

/// An unlinked event, offered for connecting to the open habit.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EventChoice {
    id: String,
    label: String,
}

impl std::fmt::Display for EventChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// The habit events that count towards this habit. This part has no counterpart
/// in Loop, and is what ties a habit to the calendar.
fn events_card<'a>(state: &'a State, habit: &'a Habit, metrics: &Metrics) -> Element<'a, Message> {
    let store = &state.habits;
    let mut body = column![card_title("Calendar events", habit.color, metrics)]
        .spacing(8)
        .width(Fill);
    let events = store.events_of(&habit.id);
    if events.is_empty() {
        body = body.push(
            text("Nothing on the calendar yet. Doing the habit is tracked either way.")
                .size(metrics.label())
                .style(text::secondary),
        );
    }
    for series in events {
        body = body.push(event_row(state, series, metrics));
    }

    let mut add = crate::ui::controls::quiet_button(text("Add event").size(metrics.label()));
    if state.calendar.is_signed_in() && !store.busy {
        add = add.on_press(msg(Action::AddEvent(habit.id.clone())));
    }
    let mut show =
        crate::ui::controls::quiet_button(text("Show on calendar").size(metrics.label()));
    if !store.events_of(&habit.id).is_empty() {
        show = show.on_press(msg(Action::ShowOnCalendar(habit.id.clone())));
    }
    body = body.push(row![add, show].spacing(6));

    let choices: Vec<EventChoice> = store
        .unlinked_events()
        .into_iter()
        .map(|series| EventChoice {
            id: series.id.clone(),
            label: format!("{} · {}", series.title, series.recurrence_label()),
        })
        .collect();
    if !choices.is_empty() {
        let habit_id = habit.id.clone();
        body = body.push(
            crate::ui::controls::pick_list(choices, None::<EventChoice>, move |choice| {
                msg(Action::LinkEvent(choice.id, habit_id.clone()))
            })
            .placeholder("Connect an existing event…")
            .text_size(metrics.label())
            .width(Fill),
        );
    }
    if !state.calendar.is_signed_in() {
        body = body.push(
            text("Connect Google Calendar in Settings to add events.")
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }
    card(body.into())
}

fn event_row<'a>(state: &'a State, series: &'a Series, metrics: &Metrics) -> Element<'a, Message> {
    let format = state.config.appearance.time_format;
    let busy = state.habits.busy;
    let mut edit =
        crate::ui::controls::quiet_button(text("Edit").size(metrics.label())).padding([2, 6]);
    let mut unlink =
        crate::ui::controls::quiet_button(text("Disconnect").size(metrics.label())).padding([2, 6]);
    if !busy {
        edit = edit.on_press(msg(Action::EditEvent(series.id.clone())));
        unlink = unlink.on_press(msg(Action::UnlinkEvent(series.id.clone())));
    }
    column![
        row![
            super::calendar::color_circle(series.color(&state.config.calendar)),
            text(&series.title).size(metrics.label()).width(Fill),
            edit,
            unlink,
        ]
        .spacing(6)
        .align_y(Alignment::Center),
        text(format!(
            "{} · {} – {}",
            series.recurrence_label(),
            crate::calendar::time_format::clock(series.start, format),
            crate::calendar::time_format::clock(series.end, format),
        ))
        .size(metrics.label() - 1.0)
        .style(text::secondary),
    ]
    .spacing(2)
    .into()
}

// --------------------------------------------------------- add and edit page

fn field_label<'a>(name: &'a str, metrics: &Metrics) -> Element<'a, Message> {
    text(name).size(metrics.label() - 1.0).style(medium).into()
}

fn number_input<'a>(
    value: &'a str,
    on_input: impl Fn(String) -> FormAction + 'a,
    metrics: &Metrics,
) -> Element<'a, Message> {
    crate::ui::controls::text_input("", value)
        .on_input(move |value| form_msg(on_input(value)))
        .size(metrics.label())
        .width(46)
        .into()
}

fn frequency_section<'a>(form: &'a Form, metrics: &Metrics) -> Element<'a, Message> {
    let choice = |kind: FrequencyKind, label: String| {
        radio(label, kind, Some(form.kind), |kind| {
            form_msg(FormAction::Kind(kind))
        })
        .text_size(metrics.label())
        .size(15)
    };
    let bare = |kind: FrequencyKind| {
        radio("", kind, Some(form.kind), |kind| {
            form_msg(FormAction::Kind(kind))
        })
        .size(15)
        .spacing(0)
    };
    let words = |words: &'static str| text(words).size(metrics.label());

    let weekdays = {
        let mut days = row![].spacing(2);
        for (index, name) in WEEKDAYS.iter().enumerate() {
            let on = form.weekdays[index];
            days = days.push(
                button(text(*name).size(metrics.label() - 2.0).center())
                    .padding([3, 5])
                    .style(move |theme: &Theme, status| {
                        if on {
                            crate::ui::controls::primary(theme, status)
                        } else {
                            crate::ui::controls::secondary(theme, status)
                        }
                    })
                    .on_press(form_msg(FormAction::Weekday(index))),
            );
        }
        days
    };

    column![
        choice(FrequencyKind::EveryDay, "Every day".to_owned()),
        row![
            bare(FrequencyKind::TimesPerDay),
            number_input(&form.per_day, FormAction::PerDay, metrics),
            words("times per day"),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        row![
            bare(FrequencyKind::EveryXDays),
            words("Every"),
            number_input(&form.every_days, FormAction::EveryDays, metrics),
            words("days"),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        row![
            bare(FrequencyKind::TimesPerWeek),
            number_input(&form.per_week, FormAction::PerWeek, metrics),
            words("times per week"),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        row![
            bare(FrequencyKind::TimesPerMonth),
            number_input(&form.per_month, FormAction::PerMonth, metrics),
            words("times per month"),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        row![
            bare(FrequencyKind::TimesInDays),
            number_input(&form.in_times, FormAction::InTimes, metrics),
            words("times in"),
            number_input(&form.in_days, FormAction::InDays, metrics),
            words("days"),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        choice(FrequencyKind::Weekdays, "On these days".to_owned()),
        weekdays,
    ]
    .spacing(6)
    .into()
}

/// "Did it" or "Avoided it", which a habit keeps once it is created.
fn kind_section<'a>(form: &'a Form, metrics: &Metrics) -> Element<'a, Message> {
    let locked = form.id.is_some();
    let mut buttons = row![].spacing(4);
    for kind in [Kind::Yes, Kind::No] {
        let on = form.habit_kind == kind;
        let mut choice = button(
            text(kind.label())
                .size(metrics.label())
                .center()
                .width(Fill),
        )
        .width(Fill)
        .style(move |theme: &Theme, status| {
            if on {
                crate::ui::controls::primary(theme, status)
            } else {
                crate::ui::controls::secondary(theme, status)
            }
        });
        if !locked {
            choice = choice.on_press(form_msg(FormAction::HabitKind(kind)));
        }
        buttons = buttons.push(choice);
    }
    let hint = match form.habit_kind {
        Kind::Yes => "Tick it off when you do it. Doing it again that day counts the same.",
        Kind::No => "Something to avoid. Every day you don't do it counts as a success.",
    };
    column![
        field_label("Type", metrics),
        buttons,
        text(hint).size(metrics.label() - 2.0).style(medium),
    ]
    .spacing(4)
    .into()
}

/// How often it is meant to happen. A habit to avoid is meant to never happen.
fn goal_section<'a>(form: &'a Form, metrics: &Metrics) -> Element<'a, Message> {
    match form.habit_kind {
        Kind::Yes => column![
            field_label("Frequency", metrics),
            frequency_section(form, metrics)
        ]
        .spacing(6)
        .into(),
        Kind::No => Space::new().into(),
    }
}

/// A number to record each day, on either type of habit.
fn measure_section<'a>(form: &'a Form, metrics: &Metrics) -> Element<'a, Message> {
    let hint = match form.habit_kind {
        Kind::Yes => {
            "Record a number each day, such as miles run. The target is what you aim to reach, and it is drawn on the Value chart. The number does not change the score."
        }
        Kind::No => {
            "Record a number each day, such as how many you had. The target is the most you aim to stay under, and it is drawn on the Value chart. The number does not change the score."
        }
    };
    let mut section = column![
        crate::ui::controls::toggler(form.measured)
            .label("Track a value")
            .text_size(metrics.label())
            .on_toggle(|value| form_msg(FormAction::Measured(value))),
        text(hint).size(metrics.label() - 2.0).style(medium),
    ]
    .spacing(2);
    if form.measured {
        let period = |days: u32, label: &'static str| {
            radio(label, days, Some(form.target_days), |days| {
                form_msg(FormAction::TargetDays(days))
            })
            .text_size(metrics.label())
            .size(15)
        };
        section = section.push(
            column![
                row![
                    column![
                        field_label("Target", metrics),
                        crate::ui::controls::text_input("(Optional)", &form.target)
                            .on_input(|value| form_msg(FormAction::Target(value)))
                            .size(metrics.label()),
                    ]
                    .spacing(3)
                    .width(Fill),
                    column![
                        field_label("Unit", metrics),
                        crate::ui::controls::text_input("e.g. miles", &form.unit)
                            .on_input(|value| form_msg(FormAction::Unit(value)))
                            .size(metrics.label()),
                    ]
                    .spacing(3)
                    .width(Fill),
                ]
                .spacing(8),
                row![
                    period(1, "Per day"),
                    period(7, "Per week"),
                    period(30, "Per month"),
                ]
                .spacing(16),
            ]
            .spacing(8)
            .padding(iced::Padding {
                top: 6.0,
                ..iced::Padding::ZERO
            }),
        );
    }
    section.into()
}

/// Which icons the list draws this habit's days with, with a sample of each.
fn icons_section<'a>(form: &'a Form, metrics: &Metrics) -> Element<'a, Message> {
    let mut buttons = row![].spacing(4);
    for (icons, label) in [(Icons::Symbols, "Symbols"), (Icons::Faces, "Faces")] {
        let on = form.icons == icons;
        buttons = buttons.push(
            button(text(label).size(metrics.label()).center().width(Fill))
                .width(Fill)
                .on_press(form_msg(FormAction::Icons(icons)))
                .style(move |theme: &Theme, status| {
                    if on {
                        crate::ui::controls::primary(theme, status)
                    } else {
                        crate::ui::controls::secondary(theme, status)
                    }
                }),
        );
    }
    let mut samples = row![].spacing(4);
    for status in [Status::Yes, Status::No, Status::Skip] {
        samples = samples.push(
            canvas(DayMark {
                status,
                color: form.color,
                goal: 1,
                question_marks: false,
                icons: form.icons,
                kind: Kind::Yes,
                times: 0,
                resisted: 0,
                value: None,
            })
            .width(32)
            .height(32),
        );
    }
    column![field_label("Icons", metrics), buttons, samples]
        .spacing(4)
        .into()
}

fn color_section<'a>(selected: u8) -> Element<'a, Message> {
    let mut swatches = row![].spacing(4);
    for index in 0..PALETTE_SIZE {
        let on = index == selected;
        swatches = swatches.push(
            button(Space::new().width(22).height(22))
                .padding(0)
                .style(move |theme: &Theme, _status| {
                    let palette = theme.extended_palette();
                    button::Style {
                        background: Some(palette_color(index, palette.is_dark).into()),
                        border: iced::Border {
                            color: if on {
                                palette.background.base.text
                            } else {
                                Color::TRANSPARENT
                            },
                            width: 2.0,
                            radius: 11.0.into(),
                        },
                        ..Default::default()
                    }
                })
                .on_press(form_msg(FormAction::Color(index))),
        );
    }
    swatches.wrap().vertical_spacing(4).into()
}

/// A section to list a habit under, or none.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SectionChoice {
    id: Option<String>,
    name: String,
}

impl std::fmt::Display for SectionChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// Which section the habit is listed under, once there are any to choose from.
fn section_field<'a>(state: &'a State, form: &'a Form, metrics: &Metrics) -> Element<'a, Message> {
    let sections = &state.habits.library.sections;
    if sections.is_empty() {
        return Space::new().into();
    }
    let choices: Vec<SectionChoice> = std::iter::once(SectionChoice {
        id: None,
        name: "No section".to_owned(),
    })
    .chain(sections.iter().map(|section| SectionChoice {
        id: Some(section.id.clone()),
        name: section.name.clone(),
    }))
    .collect();
    let selected = choices
        .iter()
        .find(|choice| choice.id == form.section)
        .or(choices.first())
        .cloned();
    column![
        field_label("Section", metrics),
        crate::ui::controls::pick_list(choices, selected, |choice: SectionChoice| {
            form_msg(FormAction::Section(choice.id))
        })
        .text_size(metrics.label())
        .width(Fill),
    ]
    .spacing(3)
    .into()
}

fn form_page<'a>(state: &'a State, form: &'a Form, metrics: &Metrics) -> Element<'a, Message> {
    let toggle =
        |value: bool, label: &'static str, hint: &'static str, on: fn(bool) -> FormAction| {
            column![
                crate::ui::controls::toggler(value)
                    .label(label)
                    .text_size(metrics.label())
                    .on_toggle(move |value| form_msg(on(value))),
                text(hint).size(metrics.label() - 2.0).style(medium),
            ]
            .spacing(2)
        };
    let mut fields = column![
        kind_section(form, metrics),
        column![
            field_label("Name", metrics),
            crate::ui::controls::text_input("e.g. Exercise", &form.name)
                .id(crate::habits::NAME_FIELD)
                .on_input(|value| form_msg(FormAction::Name(value)))
                .on_submit(msg(Action::SaveForm))
                .size(metrics.label()),
        ]
        .spacing(3),
        column![
            field_label("Question", metrics),
            crate::ui::controls::text_input("e.g. Did you exercise today?", &form.question)
                .on_input(|value| form_msg(FormAction::Question(value)))
                .on_submit(msg(Action::SaveForm))
                .size(metrics.label()),
        ]
        .spacing(3),
        section_field(state, form, metrics),
        column![field_label("Color", metrics), color_section(form.color)].spacing(3),
        icons_section(form, metrics),
        goal_section(form, metrics),
        measure_section(form, metrics),
        toggle(
            form.skip_days,
            "Enable skip days",
            "Adds \"Skipped\" to each day's choices. Skipped days pause the score and keep the streak going, for when the habit could not be done.",
            FormAction::SkipDays,
        ),
        toggle(
            form.question_marks,
            "Show question mark for missing data",
            "Days with nothing recorded show a question mark rather than a cross, and on a habit to do, each day's choices add \"Not sure\" to return it to having no data.",
            FormAction::QuestionMarks,
        ),
        column![
            field_label("Notes", metrics),
            crate::ui::controls::text_editor(&state.form_editors.habit_notes)
                .placeholder("(Optional)")
                .on_action(|action| Message::Event(EventMessage::FormEdited(
                    FormField::HabitNotes,
                    action,
                )))
                .size(metrics.label())
                .height(100)
                .padding(8),
        ]
        .spacing(3),
    ]
    .spacing(metrics.spacing * 2.0)
    .width(Fill);
    if let Some(error) = &form.error {
        fields = fields.push(
            text(error.clone())
                .size(metrics.label())
                .style(text::danger),
        );
    }

    let color = form.color;
    let title = if form.id.is_some() {
        "Edit habit"
    } else {
        "Create habit"
    };
    column![
        page_header(
            title,
            color,
            vec![
                button(text("Save").size(metrics.label()).color(Color::WHITE))
                    .style(crate::ui::controls::ghost)
                    .on_press(msg(Action::SaveForm))
                    .into(),
            ],
        ),
        crate::ui::controls::scrollable(container(fields).padding(iced::Padding {
            top: 14.0,
            bottom: 14.0,
            left: 14.0,
            right: SCROLLBAR_GUTTER + 6.0,
        }))
        .height(Fill),
    ]
    .width(Fill)
    .height(Fill)
    .into()
}

// ------------------------------------------------------------------- dialogs

/// Everything a day can record, opened by a click on any of its marks. Nothing
/// changes until it is saved.
pub fn day_dialog<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(dialog) = &state.habits.day_dialog else {
        return Space::new().into();
    };
    let Some(habit) = state.habits.library.habit(&dialog.habit_id) else {
        return Space::new().into();
    };
    let question = if !habit.question.is_empty() {
        habit.question.as_str()
    } else {
        match habit.kind {
            Kind::Yes => "Did you do it?",
            Kind::No => "Did you avoid it?",
        }
    };

    let (good, bad) = match habit.kind {
        Kind::Yes => (("Did it", Outcome::Done), ("Didn't", Outcome::NotDone)),
        Kind::No => (("Avoided it", Outcome::NotDone), ("Slipped", Outcome::Done)),
    };
    let mut choices = vec![good, bad];
    if habit.skip_days {
        choices.push(("Skipped", Outcome::Skipped));
    }
    // A habit to avoid is answered by its logs, so it has no "Not sure".
    if habit.question_marks && habit.kind == Kind::Yes {
        choices.push(("Not sure", Outcome::Unanswered));
    }
    let mut outcomes = row![].spacing(4);
    for (label, outcome) in choices {
        let on = dialog.outcome == outcome;
        outcomes = outcomes.push(
            button(text(label).size(metrics.label() - 1.0).center().width(Fill))
                .width(Fill)
                .padding([5, 2])
                .on_press(msg(Action::DayOutcome(outcome)))
                .style(move |theme: &Theme, status| {
                    if on {
                        crate::ui::controls::primary(theme, status)
                    } else {
                        crate::ui::controls::secondary(theme, status)
                    }
                }),
        );
    }

    let mut body = column![
        text(format!(
            "{} · {} {}",
            habit.name,
            weekday_name(dialog.day),
            dialog.day.format("%b %-d")
        ))
        .size(metrics.label() + 1.0),
        text(question).size(metrics.label() - 1.0).style(medium),
        outcomes,
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    if habit.kind == Kind::No {
        let format = state.config.appearance.time_format;
        body = body.push(day_log(
            "Avoided it",
            "Thought about it, didn't do it",
            &dialog.thoughts,
            Action::LogResisted,
            Action::UnlogResisted,
            format,
            metrics,
        ));
        body = body.push(day_log(
            "Slipped",
            "Times slipped",
            &dialog.slips,
            Action::LogSlip,
            Action::UnlogSlip,
            format,
            metrics,
        ));
    } else if dialog.outcome == Outcome::Done {
        let goal = habit.frequency.per_day();
        let label = if goal > 1 {
            format!("Times done, out of {goal}")
        } else {
            "Times done".to_owned()
        };
        body = body.push(stepper(label, dialog.times, 1, Action::StepTimes, metrics));
    }
    if let Some(measure) = &habit.measure {
        let unit = if measure.unit.is_empty() {
            "Value".to_owned()
        } else {
            format!("Value in {}", measure.unit)
        };
        let mut field = column![
            text(unit).size(metrics.label() - 1.0).style(medium),
            crate::ui::controls::text_input("None", &dialog.value)
                .id(crate::habits::DAY_FIELD)
                .on_input(|value| msg(Action::DayValue(value)))
                .on_submit(msg(Action::SaveDay))
                .size(metrics.label()),
        ]
        .spacing(3);
        if measure.target.is_some() {
            field = field.push(
                text(format!("Target: {}", measure.label(habit.kind)))
                    .size(metrics.label() - 2.0)
                    .style(medium),
            );
        }
        body = body.push(field);
    }
    if let Some(error) = &dialog.error {
        body = body.push(
            text(error.clone())
                .size(metrics.label() - 1.0)
                .style(text::danger),
        );
    }
    body = body.push(
        row![
            Space::new().width(Fill),
            button(text("Cancel").size(metrics.label()).center())
                .on_press(msg(Action::CloseDay))
                .style(crate::ui::controls::secondary),
            button(text("Save").size(metrics.label()).center())
                .on_press(msg(Action::SaveDay))
                .style(crate::ui::controls::primary),
        ]
        .spacing(6),
    );
    let card = container(body)
        .padding(16)
        .width(Fill)
        .max_width(340)
        .style(crate::ui::controls::surface);
    overlay(card.into(), msg(Action::CloseDay))
}

/// One of a habit to avoid's logs in the day dialog: a heading with its count,
/// a button that logs an entry at the current time, and each entry with the
/// time it was logged and an X to remove it.
#[allow(clippy::too_many_arguments)]
fn day_log<'a>(
    title: &'static str,
    add: &'static str,
    entries: &[Option<chrono::NaiveTime>],
    on_add: Action,
    on_remove: fn(usize) -> Action,
    format: crate::calendar::time_format::TimeFormat,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let mut log = column![
        row![
            text(title).size(metrics.label()).width(Fill),
            text(entries.len().to_string())
                .size(metrics.label())
                .style(medium),
        ]
        .align_y(Alignment::Center),
        crate::ui::controls::quiet_button(
            text(format!("+ {add}"))
                .size(metrics.label() - 1.0)
                .width(Fill),
        )
        .width(Fill)
        .padding([4, 8])
        .on_press(msg(on_add)),
    ]
    .spacing(4);
    for (index, entry) in entries.iter().enumerate() {
        let when = match entry {
            Some(time) => crate::calendar::time_format::clock(*time, format),
            None => "No time recorded".to_owned(),
        };
        log = log.push(
            row![
                text(when)
                    .size(metrics.label() - 1.0)
                    .style(medium)
                    .width(Fill),
                button(text("×").size(metrics.label()).center().width(Fill))
                    .padding([0, 0])
                    .width(24)
                    .style(crate::ui::controls::ghost)
                    .on_press(msg(on_remove(index))),
            ]
            .padding(iced::Padding {
                left: 8.0,
                ..iced::Padding::ZERO
            })
            .align_y(Alignment::Center),
        );
    }
    log.into()
}

/// A count with buttons to change it, which stops at `least`.
fn stepper<'a>(
    label: String,
    value: u32,
    least: u32,
    on_step: fn(bool) -> Action,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let step = |label: &'static str, up: bool| {
        let mut step = crate::ui::controls::quiet_button(
            text(label).size(metrics.label()).center().width(Fill),
        )
        .padding([2, 0])
        .width(28);
        if up || value > least {
            step = step.on_press(msg(on_step(up)));
        }
        step
    };
    row![
        text(label)
            .size(metrics.label() - 1.0)
            .style(medium)
            .width(Fill),
        step("−", false),
        text(value.to_string())
            .size(metrics.label())
            .center()
            .width(28),
        step("+", true),
    ]
    .spacing(4)
    .align_y(Alignment::Center)
    .into()
}

/// The "are you sure" prompt for deleting a habit and its record.
pub fn delete_confirmation<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let Some(id) = state.habits.confirm_delete.as_deref() else {
        return Space::new().into();
    };
    let Some(habit) = state.habits.library.habit(id) else {
        return Space::new().into();
    };

    let mut delete = button(text("Delete").size(metrics.label()).center().width(Fill))
        .width(Fill)
        .style(crate::ui::controls::danger);
    if !state.habits.busy {
        delete = delete.on_press(msg(Action::DeleteConfirmed(id.to_owned())));
    }
    let buttons = row![
        button(text("Cancel").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(msg(Action::DeleteCancelled))
            .style(crate::ui::controls::secondary),
        delete,
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    let mut body = column![
        text("Delete this habit?").size(metrics.label() + 1.0),
        text(&habit.name)
            .size(metrics.label())
            .style(text::secondary),
        text("Its whole record is removed. Archive it instead to keep the record.")
            .size(metrics.label() - 1.0)
            .style(text::secondary),
    ]
    .spacing(metrics.spacing)
    .width(Fill);
    let events = state.habits.events_of(id).len();
    if events > 0 {
        body = body.push(
            crate::ui::controls::checkbox(state.habits.delete_events)
                .label(if events == 1 {
                    "Also delete its event from Google Calendar".to_owned()
                } else {
                    format!("Also delete its {events} events from Google Calendar")
                })
                .text_size(metrics.label() - 1.0)
                .on_toggle(|delete| msg(Action::DeleteEventsToggled(delete))),
        );
    }
    let card = container(body.push(buttons))
        .padding(16)
        .width(Fill)
        .max_width(360)
        .style(crate::ui::controls::surface);

    overlay(card.into(), msg(Action::DeleteCancelled))
}

/// A dimmed backdrop that dismisses on a click, with `card` centred over it.
fn overlay<'a>(card: Element<'a, Message>, dismiss: Message) -> Element<'a, Message> {
    let backdrop =
        mouse_area(
            container(Space::new())
                .width(Fill)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(crate::ui::controls::SCRIM.into()),
                    ..Default::default()
                }),
        )
        .on_press(dismiss)
        .on_scroll(|_| Message::Ignored);
    let foreground = container(
        mouse_area(card)
            .on_press(Message::Ignored)
            .on_scroll(|_| Message::Ignored),
    )
    .padding(16)
    .center_x(Fill)
    .center_y(Fill);
    stack![backdrop, foreground].into()
}

// --------------------------------------------------------------------- icons

/// Line icons, drawn so they do not depend on the fonts installed.
#[derive(Debug, Clone, Copy)]
enum Glyph {
    Back,
    Pencil,
    Dots,
    Filter,
    Calendar,
    ChevronDown,
    ChevronRight,
}

struct GlyphIcon(Glyph);

/// `tint` overrides the theme's text color, for the colored header bar.
struct TintedIcon(Glyph, Color);

fn draw_glyph(
    glyph: Glyph,
    color: Color,
    renderer: &iced::Renderer,
    bounds: iced::Rectangle,
) -> Vec<canvas::Geometry> {
    use canvas::{Frame, LineCap, LineJoin, Path, Stroke};
    use iced::Point;
    let mut frame = Frame::new(renderer, bounds.size());
    let s = bounds.width.min(bounds.height) / 16.0;
    let p = |x: f32, y: f32| Point::new(x * s, y * s);
    let stroke = Stroke {
        style: canvas::Style::Solid(color),
        width: 1.6 * s,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Default::default()
    };
    let lines = |frame: &mut Frame, points: &[(f32, f32)]| {
        let shape = Path::new(|builder| {
            for (index, (x, y)) in points.iter().enumerate() {
                if index == 0 {
                    builder.move_to(p(*x, *y));
                } else {
                    builder.line_to(p(*x, *y));
                }
            }
        });
        frame.stroke(&shape, stroke);
    };
    match glyph {
        Glyph::Back => {
            lines(&mut frame, &[(13.5, 8.0), (2.5, 8.0)]);
            lines(&mut frame, &[(7.0, 3.5), (2.5, 8.0), (7.0, 12.5)]);
        }
        Glyph::Pencil => {
            lines(
                &mut frame,
                &[
                    (2.5, 13.5),
                    (3.0, 10.5),
                    (10.5, 3.0),
                    (13.0, 5.5),
                    (5.5, 13.0),
                    (2.5, 13.5),
                ],
            );
        }
        Glyph::Dots => {
            for y in [3.5, 8.0, 12.5] {
                frame.fill(&Path::circle(p(8.0, y), 1.5 * s), color);
            }
        }
        Glyph::Filter => {
            lines(&mut frame, &[(2.0, 4.0), (14.0, 4.0)]);
            lines(&mut frame, &[(4.5, 8.0), (11.5, 8.0)]);
            lines(&mut frame, &[(6.5, 12.0), (9.5, 12.0)]);
        }
        Glyph::Calendar => {
            frame.fill(
                &Path::rounded_rectangle(
                    p(2.0, 3.0),
                    iced::Size::new(12.0 * s, 11.0 * s),
                    (1.5 * s).into(),
                ),
                color,
            );
            lines(&mut frame, &[(5.0, 1.5), (5.0, 3.5)]);
            lines(&mut frame, &[(11.0, 1.5), (11.0, 3.5)]);
        }
        Glyph::ChevronDown => lines(&mut frame, &[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]),
        Glyph::ChevronRight => lines(&mut frame, &[(6.0, 4.0), (10.0, 8.0), (6.0, 12.0)]),
    }
    vec![frame.into_geometry()]
}

impl canvas::Program<Message> for GlyphIcon {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let color = theme
            .extended_palette()
            .background
            .base
            .text
            .scale_alpha(0.65);
        draw_glyph(self.0, color, renderer, bounds)
    }
}

impl canvas::Program<Message> for TintedIcon {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        _theme: &Theme,
        bounds: iced::Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        draw_glyph(self.0, self.1, renderer, bounds)
    }
}

fn icon<'a>(glyph: Glyph) -> Element<'a, Message> {
    canvas(GlyphIcon(glyph)).width(16).height(16).into()
}

fn icon_button<'a>(
    content: Element<'a, Message>,
    hint: &'a str,
    message: Message,
) -> Element<'a, Message> {
    iced::widget::tooltip(
        button(container(content).center_x(20).center_y(20))
            .padding(4)
            .style(crate::ui::controls::ghost)
            .on_press(message),
        text(hint).size(12),
        iced::widget::tooltip::Position::Bottom,
    )
    .style(crate::ui::controls::surface)
    .into()
}

fn header_button<'a>(
    glyph: Glyph,
    hint: &'a str,
    message: Message,
    _color: u8,
) -> Element<'a, Message> {
    iced::widget::tooltip(
        button(canvas(TintedIcon(glyph, Color::WHITE)).width(16).height(16))
            .padding(6)
            .style(|_theme: &Theme, status| button::Style {
                background: matches!(status, button::Status::Hovered | button::Status::Pressed)
                    .then(|| Color::WHITE.scale_alpha(0.15).into()),
                border: iced::Border {
                    radius: 4.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .on_press(message),
        text(hint).size(12),
        iced::widget::tooltip::Position::Bottom,
    )
    .style(crate::ui::controls::surface)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Screen;
    use crate::habits::{FormAction, Mark};
    use iced_test::Simulator;

    fn state() -> State {
        let mut state = State::blank();
        state.screen = Screen::Habits;
        state
    }

    fn add(state: &mut State, id: &str, name: &str) {
        state.habits.library.habits.push(Habit::new(id, name));
        state.habits.recompute();
    }

    fn act(state: &mut State, action: Action) {
        let _ = crate::app::update(state, Message::Habits(action));
    }

    fn list(state: &State) -> Simulator<'_, Message> {
        Simulator::with_size(
            iced_test::core::Settings::default(),
            iced::Size::new(320.0, 700.0),
            crate::ui::view(state),
        )
    }

    fn page(state: &State) -> Simulator<'_, Message> {
        Simulator::with_size(
            iced_test::core::Settings::default(),
            iced::Size::new(PANEL_WIDTH, 4000.0),
            crate::ui::view(state),
        )
    }

    #[test]
    fn an_empty_tab_offers_to_add_a_habit_without_needing_google() {
        let mut state = state();
        act(&mut state, Action::Refresh);
        assert!(state.habits.error.is_none());
        let mut ui = list(&state);
        assert!(ui.find("Habits").is_ok());
        assert!(ui.find("No habits yet. Press + to add one.").is_ok());
    }

    #[test]
    fn a_row_shows_the_name_and_today_with_the_three_days_before() {
        let mut state = state();
        add(&mut state, "h", "Stretch");
        let today = state.habits.today();
        let mut ui = list(&state);
        assert!(ui.find("Stretch").is_ok());
        for back in 0..4 {
            let day = today - Duration::days(back);
            assert!(ui.find(day.day().to_string().as_str()).is_ok(), "{day}");
        }
        let older = today - Duration::days(4);
        assert!(ui.find(older.day().to_string().as_str()).is_err());
    }

    fn dialog_ui(state: &State) -> Simulator<'_, Message> {
        let size = iced::Size::new(400.0, 500.0);
        Simulator::with_size(
            iced_test::core::Settings::default(),
            size,
            day_dialog(state, &Metrics::of(size)),
        )
    }

    fn entry(state: &State, id: &str, day: NaiveDate) -> model::DayEntry {
        state.habits.library.habit(id).unwrap().entry(day)
    }

    #[test]
    fn a_click_on_a_mark_opens_the_popup_and_only_saving_changes_the_day() {
        let mut state = state();
        add(&mut state, "h", "Stretch");
        let today = state.habits.today();
        let mut ui = list(&state);
        let messages: Vec<_> = {
            let bounds = ui.find("Stretch").unwrap().bounds();
            // The first day's button sits to the right of the name.
            ui.point_at(iced::Point::new(320.0 - 150.0, bounds.center_y()));
            ui.simulate([
                iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ]);
            ui.into_messages().collect()
        };
        assert!(
            messages
                .iter()
                .any(|m| matches!(m, Message::Habits(Action::OpenDay(id, _)) if id == "h")),
            "{messages:?}"
        );

        act(&mut state, Action::OpenDay("h".into(), today));
        act(&mut state, Action::DayOutcome(Outcome::Done));
        assert_eq!(
            state.habits.computed("h").unwrap().status(today),
            Status::Unknown,
            "nothing changes before saving"
        );
        act(&mut state, Action::CloseDay);
        assert!(state.habits.day_dialog.is_none());
        assert!(state.habits.library.habit("h").unwrap().marks.is_empty());

        act(&mut state, Action::OpenDay("h".into(), today));
        act(&mut state, Action::DayOutcome(Outcome::Done));
        act(&mut state, Action::SaveDay);
        assert!(state.habits.day_dialog.is_none());
        let computed = state.habits.computed("h").unwrap();
        assert_eq!(computed.status(today), Status::Yes);
        assert!(computed.score(today) > 0.0);

        act(&mut state, Action::OpenDay("h".into(), today));
        act(&mut state, Action::DayOutcome(Outcome::NotDone));
        act(&mut state, Action::SaveDay);
        assert_eq!(
            state.habits.computed("h").unwrap().status(today),
            Status::Unknown
        );
    }

    #[test]
    fn the_popup_offers_save_and_cancel_and_never_clear() {
        let mut state = state();
        add(&mut state, "h", "Stretch");
        let today = state.habits.today();
        act(&mut state, Action::OpenDay("h".into(), today));
        let mut ui = dialog_ui(&state);
        for label in ["Did you do it?", "Did it", "Didn't", "Save", "Cancel"] {
            assert!(ui.find(label).is_ok(), "{label}");
        }
        for label in ["Clear", "Skipped", "Not sure", "Slipped", "Avoided it"] {
            assert!(ui.find(label).is_err(), "{label}");
        }
        assert!(ui.find("Thought about it, didn't do it").is_err());
        assert!(ui.find("Times done").is_err(), "until it was done");
    }

    #[test]
    fn the_popup_shows_all_that_each_habit_can_record() {
        let mut state = state();
        add(&mut state, "h", "Smoke");
        let today = state.habits.today();
        let habit = state.habits.library.habit_mut("h").unwrap();
        habit.kind = Kind::No;
        habit.question = "Did you stay off them?".into();
        habit.skip_days = true;
        habit.question_marks = true;
        habit.measure = Some(model::Measure {
            unit: "cigarettes".into(),
            target: Some(5.0),
            days: 1,
        });
        state.habits.recompute();
        act(&mut state, Action::OpenDay("h".into(), today));
        act(&mut state, Action::DayOutcome(Outcome::Done));
        let mut ui = dialog_ui(&state);
        for label in [
            "Did you stay off them?",
            "Avoided it",
            "Slipped",
            "Skipped",
            "+ Times slipped",
            "+ Thought about it, didn't do it",
            "Value in cigarettes",
            "Target: At most 5 cigarettes per day",
        ] {
            assert!(ui.find(label).is_ok(), "{label}");
        }
        assert!(ui.find("Not sure").is_err(), "the logs answer it");
    }

    #[test]
    fn the_popup_logs_each_slip_and_urge_with_its_time() {
        use chrono::{NaiveTime, TimeZone};
        let mut state = state();
        add(&mut state, "h", "Smoke");
        let today = state.habits.today();
        let habit = state.habits.library.habit_mut("h").unwrap();
        habit.kind = Kind::No;
        habit.created = Some(today);
        // A count from before times were kept.
        habit.resisted.insert(today, 1);
        state.habits.recompute();
        let at = |hour, minute| {
            chrono::Local
                .from_local_datetime(&today.and_hms_opt(hour, minute, 7).unwrap())
                .unwrap()
                .with_timezone(&chrono::Utc)
        };
        let time = |hour, minute| Some(NaiveTime::from_hms_opt(hour, minute, 7).unwrap());

        act(&mut state, Action::OpenDay("h".into(), today));
        {
            let _clock = crate::clock::freeze(at(9, 30));
            act(&mut state, Action::LogResisted);
        }
        {
            let _clock = crate::clock::freeze(at(14, 5));
            act(&mut state, Action::LogSlip);
            act(&mut state, Action::LogSlip);
        }
        let dialog = state.habits.day_dialog.as_ref().unwrap();
        assert_eq!(dialog.outcome, Outcome::Done, "a slip answers it");
        assert_eq!(dialog.thoughts, vec![None, time(9, 30)]);
        assert_eq!(dialog.slips, vec![time(14, 5), time(14, 5)]);
        assert!(dialog_ui(&state).find("No time recorded").is_ok());

        act(&mut state, Action::UnlogSlip(0));
        act(&mut state, Action::SaveDay);
        let habit = state.habits.library.habit("h").unwrap();
        assert_eq!(
            state.habits.computed("h").unwrap().status(today),
            Status::Slipped(1)
        );
        assert_eq!(habit.slips_on(today), vec![time(14, 5)]);
        assert_eq!(habit.resisted_log_on(today), vec![None, time(9, 30)]);

        act(&mut state, Action::OpenDay("h".into(), today));
        act(&mut state, Action::UnlogSlip(0));
        assert_eq!(
            state.habits.day_dialog.as_ref().unwrap().outcome,
            Outcome::NotDone,
            "with no slips left it was avoided"
        );
        act(&mut state, Action::UnlogResisted(0));
        act(&mut state, Action::SaveDay);
        let habit = state.habits.library.habit("h").unwrap();
        assert_eq!(
            state.habits.computed("h").unwrap().status(today),
            Status::Clean
        );
        assert!(habit.slips_on(today).is_empty());
        assert_eq!(habit.resisted_log_on(today), vec![time(9, 30)]);
    }

    #[test]
    fn the_popup_records_every_value_of_an_avoided_habit_at_once() {
        let mut state = state();
        add(&mut state, "h", "Smoke");
        let today = state.habits.today();
        let habit = state.habits.library.habit_mut("h").unwrap();
        habit.kind = Kind::No;
        habit.created = Some(today);
        habit.measure = Some(model::Measure {
            unit: String::new(),
            target: None,
            days: 1,
        });
        state.habits.recompute();
        act(&mut state, Action::OpenDay("h".into(), today));
        assert_eq!(
            state.habits.day_dialog.as_ref().unwrap().outcome,
            Outcome::NotDone,
            "not having done it is the starting point"
        );
        act(&mut state, Action::StepResisted(true));
        act(&mut state, Action::StepResisted(true));
        act(&mut state, Action::StepResisted(false));
        act(&mut state, Action::DayValue("2.5".into()));
        assert_eq!(
            state.habits.day_dialog.as_ref().unwrap().outcome,
            Outcome::NotDone,
            "a number does not mean it slipped"
        );
        assert_eq!(entry(&state, "h", today).resisted, 0, "not saved yet");
        act(&mut state, Action::SaveDay);
        let saved = entry(&state, "h", today);
        assert_eq!((saved.resisted, saved.value), (1, Some(2.5)));
        assert_eq!(
            state.habits.computed("h").unwrap().status(today),
            Status::Clean
        );

        act(&mut state, Action::OpenDay("h".into(), today));
        act(&mut state, Action::DayOutcome(Outcome::Done));
        act(&mut state, Action::StepTimes(true));
        act(&mut state, Action::StepTimes(true));
        act(&mut state, Action::StepTimes(false));
        act(&mut state, Action::SaveDay);
        assert_eq!(
            state.habits.computed("h").unwrap().status(today),
            Status::Slipped(2)
        );
        assert_eq!(entry(&state, "h", today).resisted, 1, "both are kept");
    }

    #[test]
    fn times_never_step_below_one_and_a_times_goal_starts_complete() {
        let mut state = state();
        add(&mut state, "h", "Water");
        let today = state.habits.today();
        state.habits.library.habit_mut("h").unwrap().frequency =
            crate::habits::Frequency::TimesPerDay { times: 3 };
        state.habits.recompute();
        act(&mut state, Action::OpenDay("h".into(), today));
        act(&mut state, Action::DayOutcome(Outcome::Done));
        assert_eq!(state.habits.day_dialog.as_ref().unwrap().times, 3);
        act(&mut state, Action::StepTimes(false));
        act(&mut state, Action::SaveDay);
        assert_eq!(
            state.habits.computed("h").unwrap().status(today),
            Status::Partial(2)
        );
        act(&mut state, Action::OpenDay("h".into(), today));
        for _ in 0..5 {
            act(&mut state, Action::StepTimes(false));
        }
        assert_eq!(state.habits.day_dialog.as_ref().unwrap().times, 1);
    }

    #[test]
    fn a_value_on_a_habit_to_do_means_it_was_done_unless_changed_back() {
        let mut state = state();
        add(&mut state, "h", "Run");
        let today = state.habits.today();
        state.habits.library.habit_mut("h").unwrap().measure = Some(model::Measure {
            unit: "miles".into(),
            target: Some(2.0),
            days: 1,
        });
        act(&mut state, Action::OpenDay("h".into(), today));
        act(&mut state, Action::DayValue("0".into()));
        assert_eq!(
            state.habits.day_dialog.as_ref().unwrap().outcome,
            Outcome::NotDone
        );
        act(&mut state, Action::DayValue("1".into()));
        assert_eq!(
            state.habits.day_dialog.as_ref().unwrap().outcome,
            Outcome::Done
        );
        act(&mut state, Action::DayOutcome(Outcome::NotDone));
        act(&mut state, Action::SaveDay);
        let saved = entry(&state, "h", today);
        assert_eq!((saved.outcome, saved.value), (Outcome::NotDone, Some(1.0)));
    }

    #[test]
    fn a_value_that_cannot_be_read_keeps_the_popup_open_and_saves_nothing() {
        let mut state = state();
        add(&mut state, "h", "Run");
        let today = state.habits.today();
        state.habits.library.habit_mut("h").unwrap().measure = Some(model::Measure {
            unit: String::new(),
            target: None,
            days: 1,
        });
        for bad in ["lots", "-1", "inf", "NaN"] {
            act(&mut state, Action::OpenDay("h".into(), today));
            act(&mut state, Action::DayOutcome(Outcome::Done));
            act(&mut state, Action::DayValue(bad.into()));
            act(&mut state, Action::SaveDay);
            let dialog = state.habits.day_dialog.as_ref().unwrap();
            assert!(dialog.error.is_some(), "{bad}");
            assert!(state.habits.library.habit("h").unwrap().marks.is_empty());
        }
        act(&mut state, Action::DayValue("".into()));
        assert_eq!(state.habits.day_dialog.as_ref().unwrap().error, None);
        act(&mut state, Action::SaveDay);
        assert!(state.habits.day_dialog.is_none(), "blank is no value");
    }

    #[test]
    fn saved_values_reopen_in_full() {
        let mut state = state();
        add(&mut state, "h", "Run");
        let today = state.habits.today();
        state.habits.library.habit_mut("h").unwrap().measure = Some(model::Measure {
            unit: String::new(),
            target: None,
            days: 1,
        });
        act(&mut state, Action::OpenDay("h".into(), today));
        act(&mut state, Action::DayValue("0.125".into()));
        act(&mut state, Action::SaveDay);
        act(&mut state, Action::OpenDay("h".into(), today));
        let dialog = state.habits.day_dialog.as_ref().unwrap();
        assert_eq!(dialog.value, "0.125");
        assert_eq!(dialog.outcome, Outcome::Done);
    }

    #[test]
    fn clicking_a_name_opens_the_extended_view() {
        let mut state = state();
        add(&mut state, "h", "Stretch");
        let mut ui = list(&state);
        ui.click("Stretch").unwrap();
        let messages: Vec<Message> = ui.into_messages().collect();
        for message in messages {
            let _ = crate::app::update(&mut state, message);
        }
        assert_eq!(state.habits.selected.as_deref(), Some("h"));
    }

    #[test]
    fn the_global_release_arriving_first_does_not_lose_the_click() {
        let mut state = state();
        add(&mut state, "h", "Stretch");
        act(&mut state, Action::RowPressed("h".into()));
        act(&mut state, Action::DragEnded);
        act(&mut state, Action::RowReleased("h".into()));
        assert_eq!(state.habits.selected.as_deref(), Some("h"));
    }

    #[test]
    fn dragging_a_row_reorders_the_manual_list_and_only_that() {
        let mut state = state();
        add(&mut state, "a", "A");
        add(&mut state, "b", "B");
        add(&mut state, "c", "C");
        let order = |state: &State| -> Vec<String> {
            state
                .habits
                .visible()
                .iter()
                .map(|habit| habit.id.clone())
                .collect()
        };
        act(&mut state, Action::RowPressed("c".into()));
        act(&mut state, Action::RowEntered("a".into()));
        act(&mut state, Action::DragEnded);
        act(&mut state, Action::RowReleased("a".into()));
        assert_eq!(order(&state), ["c", "a", "b"]);
        assert_eq!(state.habits.selected, None, "a drag is not a click");
        // Hovering afterwards moves nothing.
        act(&mut state, Action::RowEntered("b".into()));
        assert_eq!(order(&state), ["c", "a", "b"]);

        act(&mut state, Action::Sort(SortKey::Name));
        act(&mut state, Action::RowPressed("c".into()));
        act(&mut state, Action::RowEntered("a".into()));
        act(&mut state, Action::DragEnded);
        assert_eq!(order(&state), ["a", "b", "c"], "sorted lists do not drag");
    }

    fn name_section(state: &mut State, name: &str) -> String {
        act(state, Action::NewSection);
        act(state, Action::SectionName(name.into()));
        act(state, Action::SaveSection);
        state.habits.library.sections.last().unwrap().id.clone()
    }

    #[test]
    fn a_section_is_named_from_the_menu_and_folds_its_habits_away() {
        let mut state = state();
        add(&mut state, "h", "Stretch");
        act(&mut state, Action::Menu(Menu::List));
        assert!(list(&state).find("Add section").is_ok());
        let id = name_section(&mut state, "  Morning ");
        assert_eq!(state.habits.library.sections[0].name, "Morning");
        assert_eq!(state.habits.section_edit, None);

        state.habits.library.habit_mut("h").unwrap().section = Some(id.clone());
        assert!(list(&state).find("Morning").is_ok());
        assert!(list(&state).find("Stretch").is_ok());
        act(&mut state, Action::ToggleSection(id.clone()));
        assert!(list(&state).find("Morning").is_ok());
        assert!(list(&state).find("Stretch").is_err());

        act(&mut state, Action::EditSection(id.clone()));
        act(&mut state, Action::SectionName("Evening".into()));
        act(&mut state, Action::SaveSection);
        assert_eq!(state.habits.library.sections[0].name, "Evening");
    }

    #[test]
    fn a_blank_section_name_changes_nothing() {
        let mut state = state();
        act(&mut state, Action::NewSection);
        act(&mut state, Action::SectionName("  ".into()));
        act(&mut state, Action::SaveSection);
        assert!(state.habits.library.sections.is_empty());
    }

    #[test]
    fn deleting_a_section_leaves_its_habits_in_the_list() {
        let mut state = state();
        add(&mut state, "h", "Stretch");
        let id = name_section(&mut state, "Morning");
        state.habits.library.habit_mut("h").unwrap().section = Some(id.clone());
        act(&mut state, Action::EditSection(id.clone()));
        act(&mut state, Action::DeleteSection(id));
        assert_eq!(state.habits.section_edit, None);
        assert!(list(&state).find("Morning").is_err());
        assert!(list(&state).find("Stretch").is_ok());
    }

    #[test]
    fn dragging_a_row_to_a_section_or_its_habits_moves_it_there() {
        let mut state = state();
        add(&mut state, "a", "A");
        add(&mut state, "b", "B");
        let id = name_section(&mut state, "Morning");
        act(&mut state, Action::RowPressed("a".into()));
        act(&mut state, Action::SectionEntered(id.clone()));
        act(&mut state, Action::DragEnded);
        act(&mut state, Action::RowReleased(id.clone()));
        assert_eq!(state.habits.selected, None, "a drag is not a click");
        let section =
            |state: &State, habit: &str| state.habits.library.habit(habit).unwrap().section.clone();
        assert_eq!(section(&state, "a"), Some(id.clone()));

        act(&mut state, Action::RowPressed("b".into()));
        act(&mut state, Action::RowEntered("a".into()));
        act(&mut state, Action::DragEnded);
        assert_eq!(section(&state, "b"), Some(id.clone()));

        // Hovering with nothing held moves nothing, nor does a sorted list drag.
        act(&mut state, Action::SectionEntered("other".into()));
        act(&mut state, Action::Sort(SortKey::Name));
        act(&mut state, Action::RowPressed("a".into()));
        act(&mut state, Action::SectionEntered("other".into()));
        act(&mut state, Action::DragEnded);
        assert_eq!(section(&state, "a"), Some(id));
    }

    #[test]
    fn the_form_lists_a_habit_under_a_section_once_there_are_any() {
        let mut state = state();
        act(&mut state, Action::New);
        assert!(page(&state).find("Section").is_err());
        act(&mut state, Action::CancelEdit);

        let id = name_section(&mut state, "Morning");
        act(&mut state, Action::New);
        assert!(page(&state).find("Section").is_ok());
        form_action(&mut state, FormAction::Name("Stretch".into()));
        form_action(&mut state, FormAction::Section(Some(id.clone())));
        act(&mut state, Action::SaveForm);
        let habit_id = state.habits.selected.clone().unwrap();
        let habit = state.habits.library.habit(&habit_id).unwrap();
        assert_eq!(habit.section, Some(id));
    }

    #[test]
    fn the_notes_field_takes_several_lines_and_shows_what_was_saved() {
        use iced::widget::text_editor::{Action as Editor, Edit};
        use std::sync::Arc;
        let mut state = state();
        act(&mut state, Action::New);
        form_action(&mut state, FormAction::Name("Stretch".into()));
        let edit = |state: &mut State, edit: Edit| {
            let _ = crate::app::update(
                state,
                Message::Event(EventMessage::FormEdited(
                    FormField::HabitNotes,
                    Editor::Edit(edit),
                )),
            );
        };
        edit(&mut state, Edit::Paste(Arc::new("one".to_owned())));
        edit(&mut state, Edit::Enter);
        edit(&mut state, Edit::Paste(Arc::new("two".to_owned())));
        act(&mut state, Action::SaveForm);
        let id = state.habits.selected.clone().unwrap();
        assert_eq!(state.habits.library.habit(&id).unwrap().notes, "one\ntwo");
        assert_eq!(state.form_editors.habit_notes.text().trim(), "");

        // Opening it again puts the text back in the editor.
        act(&mut state, Action::EditHabit(id));
        assert_eq!(state.form_editors.habit_notes.text().trim(), "one\ntwo");
        act(&mut state, Action::CancelEdit);
        assert_eq!(state.form_editors.habit_notes.text().trim(), "");
    }

    #[test]
    fn the_menu_offers_every_sort_and_the_filters() {
        let mut state = state();
        add(&mut state, "h", "Stretch");
        act(&mut state, Action::Menu(Menu::Filter));
        let mut ui = list(&state);
        for label in [
            "Hide archived",
            "Hide completed",
            "Manually ✓",
            "By name",
            "By color",
            "By score",
            "By status",
        ] {
            assert!(ui.find(label).is_ok(), "{label}");
        }
    }

    #[test]
    fn the_extended_view_has_every_card_of_loops_screen() {
        let mut state = state();
        add(&mut state, "h", "Meditate");
        let today = state.habits.today();
        let habit = state.habits.library.habit_mut("h").unwrap();
        habit.question = "Did you meditate?".into();
        for back in 0..90 {
            if back % 7 != 3 {
                habit.marks.insert(today - Duration::days(back), Mark::Yes);
            }
        }
        state.habits.recompute();
        act(&mut state, Action::Open("h".into()));
        let mut ui = page(&state);
        for label in [
            "Meditate",
            "Did you meditate?",
            "Every day",
            "Overview",
            "Score",
            "Month",
            "Year",
            "Total",
            "History",
            "Calendar",
            "Best streaks",
            "Frequency",
            "Calendar events",
        ] {
            assert!(ui.find(label).is_ok(), "{label}");
        }
    }

    #[test]
    fn the_form_offers_loops_frequencies_and_the_new_ones() {
        let mut state = state();
        act(&mut state, Action::New);
        let mut ui = page(&state);
        for label in [
            "Type",
            "Did it",
            "Avoided it",
            "Track a value",
            "Create habit",
            "Name",
            "Question",
            "Color",
            "Frequency",
            "Every day",
            "times per day",
            "Every",
            "days",
            "times per week",
            "times per month",
            "times in",
            "On these days",
            "Mon",
            "Enable skip days",
            "Show question mark for missing data",
            "Notes",
        ] {
            assert!(ui.find(label).is_ok(), "{label}");
        }
        assert!(ui.find("Reminder").is_err());
        assert!(ui.find("Measurable").is_err());
        assert!(ui.find("Target").is_err(), "until a value is tracked");
    }

    fn form_action(state: &mut State, action: FormAction) {
        act(state, Action::Form(action));
    }

    #[test]
    fn avoiding_is_a_success_from_the_first_day_and_a_slip_is_recorded_in_the_popup() {
        let mut state = state();
        act(&mut state, Action::New);
        form_action(&mut state, FormAction::HabitKind(Kind::No));
        form_action(&mut state, FormAction::Name("Smoke".into()));
        {
            let mut ui = page(&state);
            assert!(ui.find("Frequency").is_err(), "the goal is never");
        }
        act(&mut state, Action::SaveForm);
        let today = state.habits.today();
        let id = state.habits.selected.clone().unwrap();
        let computed = state.habits.computed(&id).unwrap();
        assert_eq!(computed.status(today), Status::Clean);
        assert!(computed.score(today) > 0.0, "already going well");
        act(&mut state, Action::OpenDay(id.clone(), today));
        act(&mut state, Action::DayOutcome(Outcome::Done));
        act(&mut state, Action::SaveDay);
        assert_eq!(
            state.habits.computed(&id).unwrap().status(today),
            Status::Slipped(1)
        );
        let mut ui = page(&state);
        assert!(ui.find("Never").is_ok());
        assert!(ui.find("Calendar events").is_err());
    }

    #[test]
    fn either_kind_of_habit_can_track_a_value_with_a_target() {
        for (kind, words) in [
            (Kind::Yes, "At least 2.5 miles per week"),
            (Kind::No, "At most 2.5 miles per week"),
        ] {
            let mut state = state();
            act(&mut state, Action::New);
            form_action(&mut state, FormAction::HabitKind(kind));
            form_action(&mut state, FormAction::Name("Run".into()));
            form_action(&mut state, FormAction::Measured(true));
            {
                let mut ui = page(&state);
                assert!(ui.find("Target").is_ok());
            }
            form_action(&mut state, FormAction::Target("nope".into()));
            act(&mut state, Action::SaveForm);
            assert!(state.habits.form.as_ref().unwrap().error.is_some());
            form_action(&mut state, FormAction::Target("2.5".into()));
            form_action(&mut state, FormAction::Unit(" miles ".into()));
            form_action(&mut state, FormAction::TargetDays(7));
            act(&mut state, Action::SaveForm);
            let id = state.habits.selected.clone().unwrap();
            let habit = state.habits.library.habit(&id).unwrap();
            let measure = habit.measure.clone().unwrap();
            assert_eq!(habit.kind, kind);
            assert_eq!(measure.unit, "miles");
            assert_eq!(measure.label(kind), words);

            // The page has the chart, and shows the words.
            {
                let mut ui = page(&state);
                assert!(ui.find(words).is_ok());
                assert!(ui.find("Value").is_ok());
            }

            // Reopening shows what was saved, and a habit keeps its kind.
            act(&mut state, Action::EditHabit(id.clone()));
            form_action(&mut state, FormAction::HabitKind(Kind::Yes));
            form_action(&mut state, FormAction::HabitKind(Kind::No));
            let form = state.habits.form.as_ref().unwrap();
            assert_eq!(form.habit_kind, kind);
            assert_eq!((form.target.as_str(), form.target_days), ("2.5", 7));

            // Turning it off hides the chart and keeps what was recorded.
            act(&mut state, Action::CancelEdit);
            let today = state.habits.today();
            act(&mut state, Action::OpenDay(id.clone(), today));
            act(&mut state, Action::DayValue("3".into()));
            act(&mut state, Action::SaveDay);
            act(&mut state, Action::EditHabit(id.clone()));
            form_action(&mut state, FormAction::Measured(false));
            act(&mut state, Action::SaveForm);
            let habit = state.habits.library.habit(&id).unwrap();
            assert!(habit.measure.is_none());
            assert_eq!(habit.value_on(today), Some(3.0));
            assert!(page(&state).find("Value").is_err());
        }
    }

    #[test]
    fn a_value_needs_no_target_and_a_habit_without_one_has_no_chart() {
        let mut state = state();
        add(&mut state, "plain", "Plain");
        act(&mut state, Action::Open("plain".into()));
        assert!(page(&state).find("Value").is_err());

        act(&mut state, Action::New);
        form_action(&mut state, FormAction::Name("Run".into()));
        form_action(&mut state, FormAction::Measured(true));
        act(&mut state, Action::SaveForm);
        let id = state.habits.selected.clone().unwrap();
        assert_eq!(
            state.habits.library.habit(&id).unwrap().measure,
            Some(model::Measure {
                unit: String::new(),
                target: None,
                days: 1
            })
        );
        assert!(page(&state).find("Value").is_ok());
        assert!(page(&state).find("Tracking a value").is_ok());
    }

    #[test]
    fn extra_times_recorded_in_the_popup_count_the_same() {
        let mut state = state();
        add(&mut state, "h", "Stretch");
        add(&mut state, "once", "Once");
        let today = state.habits.today();
        for id in ["h", "once"] {
            act(&mut state, Action::OpenDay(id.into(), today));
            act(&mut state, Action::DayOutcome(Outcome::Done));
            if id == "h" {
                assert!(dialog_ui(&state).find("Times done").is_ok());
                act(&mut state, Action::StepTimes(true));
                act(&mut state, Action::StepTimes(true));
            }
            act(&mut state, Action::SaveDay);
        }
        let habit = state.habits.library.habit("h").unwrap();
        assert_eq!(habit.times_on(today), 3);
        let (h, once) = (
            state.habits.computed("h").unwrap(),
            state.habits.computed("once").unwrap(),
        );
        assert_eq!(h.status(today), Status::Yes);
        assert_eq!(h.score(today), once.score(today));
        assert_eq!(h.total(), once.total());
    }

    #[test]
    fn the_form_saves_a_habits_icons() {
        let mut state = state();
        add(&mut state, "h", "Read");
        act(&mut state, Action::EditHabit("h".into()));
        form_action(&mut state, FormAction::Icons(Icons::Faces));
        act(&mut state, Action::SaveForm);
        let habit = state.habits.library.habit("h").unwrap();
        assert_eq!(habit.icons, Icons::Faces);
        act(&mut state, Action::EditHabit("h".into()));
        assert_eq!(state.habits.form.as_ref().unwrap().icons, Icons::Faces);
    }

    #[test]
    fn saving_the_form_validates_then_stores_each_goal() {
        let mut state = state();
        act(&mut state, Action::New);
        act(&mut state, Action::SaveForm);
        assert!(state.habits.form.as_ref().unwrap().error.is_some());

        form_action(&mut state, FormAction::Name("Water".into()));
        form_action(&mut state, FormAction::PerDay("0".into()));
        act(&mut state, Action::SaveForm);
        assert!(state.habits.form.as_ref().unwrap().error.is_some());

        for (action, expected) in [
            (
                FormAction::PerDay("8".into()),
                crate::habits::Frequency::TimesPerDay { times: 8 },
            ),
            (
                FormAction::PerWeek("2".into()),
                crate::habits::Frequency::Interval { times: 2, days: 7 },
            ),
            (
                FormAction::PerMonth("5".into()),
                crate::habits::Frequency::Interval { times: 5, days: 30 },
            ),
            (
                FormAction::EveryDays("3".into()),
                crate::habits::Frequency::Interval { times: 1, days: 3 },
            ),
            (
                FormAction::InDays("9".into()),
                crate::habits::Frequency::Interval { times: 3, days: 9 },
            ),
        ] {
            form_action(&mut state, action);
            assert_eq!(
                state.habits.form.as_ref().unwrap().frequency(),
                Ok(expected)
            );
        }
        form_action(&mut state, FormAction::Kind(FrequencyKind::Weekdays));
        form_action(&mut state, FormAction::SkipDays(true));
        form_action(&mut state, FormAction::QuestionMarks(true));
        act(&mut state, Action::SaveForm);
        assert!(state.habits.form.is_none());
        let habit = state.habits.selected_habit().unwrap();
        assert_eq!(habit.name, "Water");
        assert_eq!(habit.frequency.label(), "Every Monday");
        assert!(habit.skip_days && habit.question_marks);

        // Reopening shows what was saved.
        let id = habit.id.clone();
        act(&mut state, Action::EditHabit(id));
        let form = state.habits.form.as_ref().unwrap();
        assert_eq!(form.kind, FrequencyKind::Weekdays);
        assert!(form.skip_days);
    }

    #[test]
    fn archiving_hides_and_deleting_removes() {
        let mut state = state();
        add(&mut state, "h", "Stretch");
        act(&mut state, Action::ToggleArchived("h".into()));
        assert!(state.habits.visible().is_empty());
        act(&mut state, Action::HideArchived(false));
        assert_eq!(state.habits.visible().len(), 1);

        act(&mut state, Action::DeleteRequested("h".into()));
        assert!(state.habits.confirm_delete.is_some());
        act(&mut state, Action::DeleteCancelled);
        assert_eq!(state.habits.library.habits.len(), 1);
        act(&mut state, Action::Deleted("h".into(), Ok(())));
        assert!(state.habits.library.habits.is_empty());
    }

    fn series(
        id: &str,
        title: &str,
        rule: &str,
        habit: Option<&str>,
    ) -> crate::calendar::api::HabitEvent {
        let day = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        crate::calendar::api::HabitEvent {
            id: id.into(),
            calendar_id: "timeslay".into(),
            title: title.into(),
            notes: String::new(),
            start: crate::calendar::at_clock(day, 9, 0),
            end: crate::calendar::at_clock(day, 9, 30),
            recurrence: vec![rule.to_owned()],
            color_id: None,
            habit_id: habit.map(str::to_owned),
        }
    }

    fn load(
        state: &mut State,
        series: Vec<crate::calendar::api::HabitEvent>,
        done: Vec<(&str, NaiveDate)>,
    ) {
        let completions = done
            .into_iter()
            .map(|(id, day)| crate::calendar::api::HabitCompletion {
                series_id: id.into(),
                day,
            })
            .collect();
        act(
            state,
            Action::Loaded(Ok(crate::calendar::api::HabitListing {
                series,
                completions,
            })),
        );
    }

    #[test]
    fn an_old_habit_event_becomes_a_habit_with_its_history_once() {
        let mut state = state();
        let day = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        let listing = vec![
            series("one", "Stretch", "RRULE:FREQ=WEEKLY;BYDAY=MO", None),
            series("two", " stretch ", "RRULE:FREQ=DAILY", None),
        ];
        load(&mut state, listing.clone(), vec![("one", day)]);
        let library = &state.habits.library;
        assert_eq!(library.habits.len(), 1, "same name, same habit");
        let habit = &library.habits[0];
        assert_eq!(habit.name, "Stretch");
        assert_eq!(habit.frequency.label(), "Every Monday");
        assert_eq!(habit.marks[&day], Mark::Yes);
        let id = habit.id.clone();
        assert_eq!(state.habits.events_of(&id).len(), 2);

        // Deleting the habit keeps its events unclaimed on the next load.
        act(&mut state, Action::Deleted(id, Ok(())));
        load(&mut state, listing, vec![("one", day)]);
        assert!(state.habits.library.habits.is_empty());
        assert_eq!(state.habits.unlinked_events().len(), 2);
    }

    #[test]
    fn one_habit_holds_any_number_of_events_and_an_unknown_link_restores_it() {
        let mut state = state();
        add(&mut state, "h", "Run");
        load(
            &mut state,
            vec![
                series("morning", "Morning run", "RRULE:FREQ=DAILY", Some("h")),
                series("evening", "Evening run", "RRULE:FREQ=DAILY", Some("h")),
                series("other", "Yoga", "RRULE:FREQ=DAILY", Some("elsewhere")),
            ],
            vec![],
        );
        assert_eq!(state.habits.events_of("h").len(), 2);
        assert_eq!(
            state.habits.library.habit("elsewhere").unwrap().name,
            "Yoga"
        );
    }

    #[test]
    fn a_linked_event_is_listed_on_the_habit_and_a_link_is_remembered() {
        let mut state = state();
        add(&mut state, "h", "Run");
        load(
            &mut state,
            vec![series("e", "Morning run", "RRULE:FREQ=DAILY", Some("h"))],
            vec![],
        );
        act(&mut state, Action::Open("h".into()));
        let mut ui = page(&state);
        assert!(ui.find("Morning run").is_ok());
        assert!(ui.find("Add event").is_ok());
        assert!(ui.find("Disconnect").is_ok());
        drop(ui);

        act(&mut state, Action::Linked("e".into(), None, Ok(())));
        assert!(state.habits.events_of("h").is_empty());
        load(
            &mut state,
            vec![series("e", "Morning run", "RRULE:FREQ=DAILY", None)],
            vec![],
        );
        assert_eq!(state.habits.library.habits.len(), 1, "it stays let go of");
    }

    #[test]
    fn completing_an_occurrence_counts_towards_its_habit() {
        let mut state = state();
        add(&mut state, "h", "Run");
        load(
            &mut state,
            vec![series("e", "Run", "RRULE:FREQ=DAILY", Some("h"))],
            vec![],
        );
        let mut event = crate::calendar::sample_snapshot().events[0].clone();
        event.id = "instance".into();
        event.details.habit = true;
        event.details.recurring_event_id = Some("e".into());
        event.completed = true;
        let day = event.start.date_naive();
        act(
            &mut state,
            Action::CompletionSaved("instance".into(), Ok(event.clone())),
        );
        assert_eq!(
            state.habits.library.habit("h").unwrap().marks[&day],
            Mark::Yes
        );
        event.completed = false;
        act(
            &mut state,
            Action::CompletionSaved("instance".into(), Ok(event)),
        );
        assert!(
            !state
                .habits
                .library
                .habit("h")
                .unwrap()
                .marks
                .contains_key(&day)
        );
    }

    #[test]
    fn a_new_event_counts_towards_the_habit_it_names_or_a_new_one() {
        let mut state = state();
        add(&mut state, "h", "Stretch");
        let (id, _) = crate::habits::habit_for_new_event(&mut state, " stretch ");
        assert_eq!(id, "h");
        let (id, _) = crate::habits::habit_for_new_event(&mut state, "Read");
        assert_ne!(id, "h");
        assert_eq!(state.habits.library.habit(&id).unwrap().name, "Read");
    }

    #[test]
    fn a_page_opens_in_the_floating_window_and_gives_way_to_the_month() {
        let mut state = state();
        state.main_window = Some(iced::window::Id::unique());
        add(&mut state, "h", "Stretch");
        assert!(!state.habit_panel_open());
        act(&mut state, Action::Open("h".into()));
        assert!(state.habit_panel_open());
        assert!(state.floating_editors());
        assert!(state.editor_window.is_some());

        act(&mut state, Action::ShowOnCalendar("h".into()));
        assert!(state.monthly());
        assert_eq!(state.habits.calendar_habit.as_deref(), Some("h"));
        assert!(!state.habit_panel_open());
        act(&mut state, Action::CloseCalendar);
        assert!(!state.monthly());
        assert_eq!(
            state.habits.selected.as_deref(),
            Some("h"),
            "back to the page"
        );
    }

    #[test]
    fn saving_and_loading_keeps_a_library_and_bad_contents_are_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        let mut state = state();
        state.habits.keep_in(Some(path.clone()));
        add(&mut state, "h", "Stretch");
        crate::habits::write_for_test(&state, &path);
        let mut reopened = crate::habits::Store::default();
        reopened.keep_in(Some(path.clone()));
        assert_eq!(reopened.library.habits[0].name, "Stretch");

        let key = crate::task_db::keys::HABITS;
        crate::task_db::write_document_json(&path, key, "not json").unwrap();
        let mut broken = crate::habits::Store::default();
        broken.keep_in(Some(path.clone()));
        assert!(broken.error.is_some());
        assert!(!broken.persists());
    }
}
