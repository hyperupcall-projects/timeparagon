//! The Habits tab, laid out as the desktop's is (`src/ui/habits.rs`), which is
//! the way Loop Habit Tracker lays it out.
//!
//! The list is the tab's main screen: a score ring, the name, then today and
//! the three days before it, each a button that opens that day. Where the
//! desktop opens a habit beside the list, here it takes the screen over, as
//! Loop's habit screen does: Overview, Notes, Score, History, Value, Calendar,
//! Best streaks, Frequency and the calendar events, with Back to the list.
//!
//! The rules, the add and edit form and the day dialog are
//! `timeslay_core::habits` and `timeslay_core::habit_edit`, the same as on the
//! desktop. Habits sync between devices through the database.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{Datelike, Duration, NaiveDate};
use dioxus::prelude::*;
use timeslay_core::calendar::time_format::{self, TimeFormat};
use timeslay_core::calendar::{self, Event};
use timeslay_core::config::CalendarSettings;
use timeslay_core::habit_edit::{DayDialog, Form, FormAction, FrequencyKind, Log, SectionEdit};
use timeslay_core::habits::{
    self as model, Bucket, Computed, Habit, Icons, Kind, Library, Outcome, PALETTE_SIZE, Section,
    SortKey, Status,
};

use crate::editor::worker;
use crate::habits_charts::{self as charts, FIRST_WEEKDAY, Mark, WEEKDAYS, habit_color};

/// Today and the three days before it.
const DAY_COLUMNS: i64 = 4;

/// The id of a habit event that is not on Google yet. The event editor
/// creates it when it is saved.
pub const NEW_HABIT_ID: &str = "timeslay:new-habit";

// -------------------------------------------------------------------- saving

/// The saved habits, and where changes go. A file that exists but cannot be
/// read is left alone rather than replaced by an empty list on the next save.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Saved {
    pub library: Library,
    path: Option<PathBuf>,
    pub error: Option<String>,
}

pub fn read(path: Option<PathBuf>) -> Saved {
    let Some(path) = path else {
        return Saved {
            error: Some("There is nowhere to keep habits on this device.".to_owned()),
            ..Saved::default()
        };
    };
    match timeslay_core::task_db::read_document(&path, timeslay_core::task_db::keys::HABITS) {
        Ok(library) => Saved {
            library: library.unwrap_or_default(),
            path: Some(path),
            error: None,
        },
        Err(_) => Saved {
            error: Some(
                "The saved habits could not be read, so changes to habits will not be saved."
                    .to_owned(),
            ),
            ..Saved::default()
        },
    }
}

impl Saved {
    fn save(&mut self) {
        let Some(path) = &self.path else { return };
        let written = timeslay_core::task_db::write_document(
            path,
            timeslay_core::task_db::keys::HABITS,
            &self.library,
        );
        self.error = written
            .err()
            .map(|error| format!("Could not save habits: {error}"));
    }
}

/// Changes the library and saves it.
fn commit(mut saved: Signal<Saved>, change: impl FnOnce(&mut Library)) {
    let mut saved = saved.write();
    change(&mut saved.library);
    saved.save();
}

#[cfg(test)]
pub fn for_tests(habits: Vec<Habit>) -> Saved {
    Saved {
        library: Library {
            habits,
            ..Library::default()
        },
        ..Saved::default()
    }
}

/// Completing or reopening a habit event in its editor counts as doing the
/// habit that day, or undoes it, as it does on the desktop.
pub fn record_completion(event: &Event) {
    let mut saved = read(timeslay_core::task_db::path());
    if saved.error.is_some() {
        return;
    }
    let series = event
        .details
        .recurring_event_id
        .as_deref()
        .unwrap_or(&event.id);
    let library = &saved.library;
    let Some(id) = event
        .details
        .habit_id
        .clone()
        .or_else(|| library.adopted.get(series).cloned())
        .filter(|id| {
            library
                .habit(id)
                .is_some_and(|habit| habit.kind == Kind::Yes)
        })
    else {
        return;
    };
    let day = event.start.date_naive();
    if let Some(habit) = saved.library.habit_mut(&id) {
        if event.completed {
            habit.add_rep(day);
        } else {
            habit.remove_rep(day);
        }
    }
    saved.save();
}

// -------------------------------------------------------------------- events

/// A habit event, as the master event on the TimeSlay calendar.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub id: String,
    pub calendar_id: String,
    pub habit_id: Option<String>,
    pub title: String,
    pub schedule: String,
    pub color: timeslay_core::Color,
    pub start: chrono::DateTime<chrono::Local>,
    pub end: chrono::DateTime<chrono::Local>,
    /// What opens in the event editor.
    pub event: Event,
}

fn load_series(settings: &CalendarSettings) -> Result<Vec<Series>, String> {
    let listing = calendar::sync_habit_listing(
        &calendar::GoogleClient::from_settings(settings),
        settings.app_calendar.as_deref(),
    )?;
    let mut series: Vec<Series> = listing
        .series
        .into_iter()
        .map(|event| {
            let (id, calendar_id, habit_id) = (
                event.id.clone(),
                event.calendar_id.clone(),
                event.habit_id.clone(),
            );
            let (start, end) = (event.start, event.end);
            let row = crate::day::habit_row(event, settings);
            Series {
                id,
                calendar_id,
                habit_id,
                title: row.title,
                schedule: row.schedule,
                color: row.color,
                start,
                end,
                event: row.event,
            }
        })
        .collect();
    // Trimmed like `Library::named`, so events sharing a habit sit together.
    series.sort_by(|left, right| {
        left.title
            .trim()
            .to_lowercase()
            .cmp(&right.title.trim().to_lowercase())
            .then(left.start.cmp(&right.start))
    });
    Ok(series)
}

/// The habit a series counts towards.
fn habit_of_series<'a>(library: &'a Library, series: &'a Series) -> Option<&'a str> {
    series
        .habit_id
        .as_deref()
        .or_else(|| {
            library
                .adopted
                .get(&series.id)
                .map(String::as_str)
                .filter(|id| !id.is_empty())
        })
        .filter(|id| library.habit(id).is_some())
}

/// The habit an occurrence on the calendar counts towards. Its series has the
/// final say, because a completed occurrence keeps its own copy of the
/// properties from when it was completed.
fn habit_of<'a>(library: &'a Library, series: &'a [Series], event: &'a Event) -> Option<&'a str> {
    let series_id = event
        .details
        .recurring_event_id
        .as_deref()
        .unwrap_or(&event.id);
    if let Some(series) = series.iter().find(|series| series.id == series_id) {
        return habit_of_series(library, series);
    }
    event
        .details
        .habit_id
        .as_deref()
        .or_else(|| {
            library
                .adopted
                .get(series_id)
                .map(String::as_str)
                .filter(|id| !id.is_empty())
        })
        .filter(|id| library.habit(id).is_some())
}

fn events_of<'a>(library: &Library, series: &'a [Series], habit_id: &str) -> Vec<&'a Series> {
    series
        .iter()
        .filter(|series| habit_of_series(library, series) == Some(habit_id))
        .collect()
}

/// A new habit event for `habit`, ready for the event editor to create.
fn new_event(habit: &Habit, today: NaiveDate, settings: &CalendarSettings) -> Event {
    Event {
        id: NEW_HABIT_ID.to_owned(),
        calendar_id: settings.app_calendar.clone().unwrap_or_default(),
        title: habit.name.clone(),
        start: calendar::at_clock(today, 9, 0),
        end: calendar::at_clock(today, 9, 30),
        all_day: false,
        color: settings.habit_color(),
        completed: false,
        details: calendar::Details {
            habit: true,
            habit_id: Some(habit.id.clone()),
            ..Default::default()
        },
    }
}

/// What "Show on calendar" asks the agenda for.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarFilter {
    pub name: String,
    /// The series whose occurrences are shown.
    pub series: Vec<String>,
}

// ------------------------------------------------------------------- the tab

/// A drop-down menu that is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    /// The list's sort and filter menu.
    Filter,
    /// The list's overflow menu.
    List,
    /// The open habit's overflow menu.
    Habit,
}

/// What fills the tab.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Page {
    #[default]
    List,
    Habit(String),
    Form(Box<Form>),
}

/// A row held down, which moves through the manual order as it is dragged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drag {
    pub id: String,
    pub moved: bool,
}

/// What the tab is showing, apart from the saved habits.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ui {
    pub page: Page,
    pub menu: Option<Menu>,
    pub day: Option<DayDialog>,
    pub section_edit: Option<SectionEdit>,
    /// A deletion waiting for confirmation, by habit id.
    pub confirm_delete: Option<String>,
    /// Whether confirming also deletes the habit's calendar events.
    pub delete_events: bool,
    pub drag: Option<Drag>,
    /// A Calendar write is in flight.
    pub busy: bool,
    /// What went wrong with the calendar events.
    pub error: Option<String>,
    /// Where the last export went.
    pub notice: Option<String>,
}

impl Ui {
    fn open(&mut self, id: String) {
        self.page = Page::Habit(id);
        self.menu = None;
    }

    /// Back from a page: an edit returns to its habit, anything else to the
    /// list.
    fn back(&mut self) {
        self.menu = None;
        self.page = match &self.page {
            Page::Form(form) => form.id.clone().map_or(Page::List, Page::Habit),
            _ => Page::List,
        };
    }
}

/// Everything the pieces of the tab share. Signals are handles, so this copies.
#[derive(Clone, Copy, PartialEq)]
struct Ctx {
    saved: Signal<Saved>,
    ui: Signal<Ui>,
    series: Signal<Vec<Series>>,
    reload_series: Signal<u32>,
    /// Bumped after a calendar write, so the day shows it.
    revision: Signal<u32>,
    today: NaiveDate,
    time_format: TimeFormat,
}

const DRAG_SCRIPT: &str = r#"
if (window.__habitDrag) window.__habitDrag.abort();
const control = new AbortController();
window.__habitDrag = control;
const signal = control.signal;
let timer = null, start = null, dragging = null, moved = false, lastOver = null, quietUntil = 0;
const clear = () => { clearTimeout(timer); timer = null; start = null; };
document.addEventListener('pointerdown', (event) => {
    const row = event.target.closest('[data-habit-grab]');
    if (!row || !row.closest('.habit-list[data-manual="true"]')) return;
    start = { x: event.clientX, y: event.clientY, id: row.dataset.habitGrab };
    timer = setTimeout(() => {
        dragging = start.id; moved = false; lastOver = null; timer = null;
        if (navigator.vibrate) navigator.vibrate(20);
        dioxus.send({ kind: 'start', id: dragging });
    }, 400);
}, { signal });
document.addEventListener('pointermove', (event) => {
    if (!dragging) {
        if (start && Math.hypot(event.clientX - start.x, event.clientY - start.y) > 8) clear();
        return;
    }
    const under = document.elementFromPoint(event.clientX, event.clientY);
    const row = under && under.closest('[data-habit-row]');
    const section = under && under.closest('[data-habit-section]');
    const over = row ? 'row:' + row.dataset.habitRow
        : section ? 'section:' + section.dataset.habitSection : null;
    if (!over || over === lastOver) return;
    lastOver = over;
    if (row) {
        if (row.dataset.habitRow === dragging) return;
        moved = true;
        dioxus.send({ kind: 'row', id: row.dataset.habitRow });
    } else {
        moved = true;
        dioxus.send({ kind: 'section', id: section.dataset.habitSection });
    }
}, { signal });
const finish = () => {
    clear();
    if (!dragging) return;
    dragging = null;
    // The tap that ends a move must not open the row it lands on.
    if (moved) quietUntil = Date.now() + 500;
    dioxus.send({ kind: 'end' });
};
document.addEventListener('pointerup', finish, { signal });
document.addEventListener('pointercancel', finish, { signal });
document.addEventListener('click', (event) => {
    if (Date.now() < quietUntil) { event.stopPropagation(); event.preventDefault(); }
}, { capture: true, signal });
document.addEventListener('touchmove', (event) => {
    if (dragging) event.preventDefault();
}, { passive: false, signal });
document.addEventListener('contextmenu', (event) => {
    if (dragging || timer) event.preventDefault();
}, { signal });
await new Promise(() => {});
"#;

#[derive(serde::Deserialize)]
struct DragMessage {
    kind: String,
    #[serde(default)]
    id: String,
}

fn on_drag(ctx: Ctx, message: DragMessage) {
    let (mut saved, mut ui) = (ctx.saved, ctx.ui);
    let manual = saved.peek().library.sort == SortKey::Manual;
    match message.kind.as_str() {
        "start" => {
            ui.write().drag = Some(Drag {
                id: message.id,
                moved: false,
            });
        }
        "row" | "section" if manual => {
            let Some(id) = ui.peek().drag.as_ref().map(|drag| drag.id.clone()) else {
                return;
            };
            if message.kind == "row" {
                if id == message.id {
                    return;
                }
                saved.write().library.move_habit(&id, &message.id);
            } else {
                saved.write().library.move_to_section(&id, &message.id);
            }
            if let Some(drag) = &mut ui.write().drag {
                drag.moved = true;
            }
        }
        "end" => {
            let moved = ui.write().drag.take().is_some_and(|drag| drag.moved);
            if moved {
                saved.write().save();
            }
        }
        _ => {}
    }
}

/// The Habits tab: the list, or a habit's page or form in its place.
#[component]
pub fn HabitsScreen(
    today: NaiveDate,
    time_format: TimeFormat,
    settings: CalendarSettings,
    signed_in: bool,
    /// Bumped by every calendar write, so a saved event shows here.
    revision: Signal<u32>,
    /// The loaded day's events, kept in step when a day is recorded.
    day_events: Vec<Event>,
    on_open_event: EventHandler<Event>,
    on_show_calendar: EventHandler<CalendarFilter>,
) -> Element {
    let saved = use_signal(|| read(timeslay_core::task_db::path()));
    let ui = use_signal(Ui::default);
    let mut series = use_signal(Vec::<Series>::new);
    let reload_series = use_signal(|| 0_u32);
    let ctx = Ctx {
        saved,
        ui,
        series,
        reload_series,
        revision,
        today,
        time_format,
    };

    // Other devices and the desktop change habits through the database.
    use_future(move || async move {
        let mut saved = saved;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if ui.peek().drag.is_some() {
                continue;
            }
            let loaded = read(timeslay_core::task_db::path());
            if loaded.error.is_none() && loaded.library != saved.peek().library {
                saved.set(loaded);
            }
        }
    });

    use_future(move || async move {
        let mut eval = document::eval(DRAG_SCRIPT);
        while let Ok(message) = eval.recv::<DragMessage>().await {
            on_drag(ctx, message);
        }
    });
    use_drop(|| {
        let _ = document::eval("if (window.__habitDrag) window.__habitDrag.abort();");
    });

    let load_settings = settings.clone();
    let listing = use_resource(move || {
        let _ = revision();
        let _ = reload_series();
        let settings = load_settings.clone();
        async move {
            if !signed_in {
                return Ok(Vec::new());
            }
            worker(move || load_series(&settings)).await
        }
    });
    use_effect(move || {
        if let Some(result) = &*listing.read() {
            let mut ui = ui;
            match result {
                Ok(loaded) => {
                    series.set(loaded.clone());
                    ui.write().busy = false;
                }
                Err(message) => {
                    ui.write().busy = false;
                    ui.write().error = Some(message.clone());
                }
            }
        }
    });

    screen(
        ctx,
        settings,
        signed_in,
        day_events,
        on_open_event,
        on_show_calendar,
    )
}

/// What the tab shows, from the state in `ctx`.
fn screen(
    ctx: Ctx,
    settings: CalendarSettings,
    signed_in: bool,
    day_events: Vec<Event>,
    on_open_event: EventHandler<Event>,
    on_show_calendar: EventHandler<CalendarFilter>,
) -> Element {
    let library = ctx.saved.read().library.clone();
    let computed: HashMap<String, Computed> = library
        .habits
        .iter()
        .map(|habit| (habit.id.clone(), habit.compute(ctx.today)))
        .collect();
    let page = ctx.ui.read().page.clone();
    let content = match page {
        Page::List => list_page(ctx, &library, &computed),
        Page::Habit(id) => match library.habit(&id) {
            Some(habit) => habit_page(
                ctx,
                habit,
                computed.get(&id).cloned().unwrap_or_default(),
                &settings,
                signed_in,
                on_open_event,
                on_show_calendar,
            ),
            None => list_page(ctx, &library, &computed),
        },
        Page::Form(form) => form_page(ctx, &library, &form),
    };
    rsx! {
        div { class: "habits-screen",
            {content}
            {day_dialog(ctx, &library, day_events)}
            {delete_confirmation(ctx, &library, settings)}
        }
    }
}

fn weekday_name(day: NaiveDate) -> &'static str {
    WEEKDAYS[day.weekday().num_days_from_monday() as usize]
}

// --------------------------------------------------------------------- icons

/// Line icons, drawn so they do not depend on the fonts installed.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Glyph {
    Back,
    Pencil,
    Dots,
    Filter,
    Calendar,
    ChevronDown,
    ChevronRight,
}

fn glyph(glyph: Glyph, size: u32) -> Element {
    let lines: &[&[(f32, f32)]] = match glyph {
        Glyph::Back => &[
            &[(13.5, 8.0), (2.5, 8.0)],
            &[(7.0, 3.5), (2.5, 8.0), (7.0, 12.5)],
        ],
        Glyph::Pencil => &[&[
            (2.5, 13.5),
            (3.0, 10.5),
            (10.5, 3.0),
            (13.0, 5.5),
            (5.5, 13.0),
            (2.5, 13.5),
        ]],
        Glyph::Dots => &[],
        Glyph::Filter => &[
            &[(2.0, 4.0), (14.0, 4.0)],
            &[(4.5, 8.0), (11.5, 8.0)],
            &[(6.5, 12.0), (9.5, 12.0)],
        ],
        Glyph::Calendar => &[&[(5.0, 1.5), (5.0, 3.5)], &[(11.0, 1.5), (11.0, 3.5)]],
        Glyph::ChevronDown => &[&[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)]],
        Glyph::ChevronRight => &[&[(6.0, 4.0), (10.0, 8.0), (6.0, 12.0)]],
    };
    let paths: Vec<String> = lines
        .iter()
        .map(|points| {
            points
                .iter()
                .map(|(x, y)| format!("{x},{y}"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    rsx! {
        svg {
            class: "habit-glyph",
            width: "{size}",
            height: "{size}",
            view_box: "0 0 16 16",
            "aria-hidden": "true",
            if glyph == Glyph::Dots {
                for y in [3.5, 8.0, 12.5] {
                    circle { cx: "8", cy: "{y}", r: "1.5", style: "fill: currentColor;" }
                }
            }
            if glyph == Glyph::Calendar {
                rect { x: "2", y: "3", width: "12", height: "11", rx: "1.5", style: "fill: currentColor;" }
            }
            for points in paths {
                polyline {
                    points: "{points}",
                    style: "fill: none; stroke: currentColor; stroke-width: 1.6; stroke-linecap: round; stroke-linejoin: round;",
                }
            }
        }
    }
}

fn icon_button(
    content: Element,
    hint: &'static str,
    onclick: impl FnMut(MouseEvent) + 'static,
) -> Element {
    rsx! {
        button { class: "habit-icon-button", title: "{hint}", aria_label: "{hint}", onclick, {content} }
    }
}

// ---------------------------------------------------------------------- list

fn list_page(ctx: Ctx, library: &Library, computed: &HashMap<String, Computed>) -> Element {
    let mut ui = ctx.ui;
    let today = ctx.today;
    let visible = library.visible(&|id| computed.get(id), today);
    let empty = visible.is_empty();
    let error = ctx.saved.read().error.clone();
    let notice = ui.read().notice.clone();
    let section_edit = ui.read().section_edit.clone();
    let dragging = ui.read().drag.as_ref().map(|drag| drag.id.clone());

    let mut rows: Vec<Element> = Vec::new();
    for (section, habits) in library.grouped(visible) {
        if let Some(section) = section {
            let editing = section_edit
                .as_ref()
                .filter(|edit| edit.id.as_deref() == Some(section.id.as_str()));
            rows.push(match editing {
                Some(edit) => section_editor(ctx, edit),
                None => section_heading(ctx, section, habits.len()),
            });
            if section.collapsed {
                continue;
            }
        }
        for habit in habits {
            let computed = computed.get(&habit.id).cloned().unwrap_or_default();
            rows.push(habit_row(
                ctx,
                habit,
                &computed,
                dragging.as_deref() == Some(habit.id.as_str()),
            ));
        }
    }
    if let Some(edit) = section_edit.as_ref().filter(|edit| edit.id.is_none()) {
        rows.push(section_editor(ctx, edit));
    }

    let days = (0..DAY_COLUMNS).map(|back| {
        let day = today - Duration::days(back);
        rsx! {
            div { key: "{day}", class: "habit-day-head",
                span { "{weekday_name(day).to_uppercase()}" }
                span { "{day.day()}" }
            }
        }
    });
    let manual = library.sort == SortKey::Manual;
    let menu = ui.read().menu;

    rsx! {
        div { class: "habit-page habit-list-page",
            div { class: "habit-toolbar",
                h1 { class: "habit-list-title", "Habits" }
                {icon_button(rsx! { span { class: "habit-plus", "+" } }, "Add habit", move |_| {
                    let mut ui = ui.write();
                    ui.menu = None;
                    ui.confirm_delete = None;
                    ui.page = Page::Form(Box::default());
                })}
                {icon_button(glyph(Glyph::Filter, 16), "Sort and filter", move |_| toggle_menu(ui, Menu::Filter))}
                {icon_button(glyph(Glyph::Dots, 16), "More", move |_| toggle_menu(ui, Menu::List))}
            }
            if let Some(error) = error {
                p { class: "habit-error", role: "alert", "{error}" }
            }
            if let Some(notice) = notice {
                p { class: "habit-notice", role: "status",
                    "{notice}"
                    button { class: "habit-notice-close", aria_label: "Dismiss", onclick: move |_| ui.write().notice = None, "×" }
                }
            }
            if empty {
                p { class: "habit-empty",
                    if library.habits.is_empty() {
                        "No habits yet. Press + to add one."
                    } else {
                        "No habits to show. Change the filter to see more."
                    }
                }
            } else {
                div { class: "habit-days-head",
                    div { class: "habit-days-spacer" }
                    {days}
                }
            }
            div { class: "habit-list", "data-manual": "{manual}",
                {rows.into_iter()}
            }
            match menu {
                Some(Menu::Filter) => menu_overlay(ui, filter_menu(ctx, library)),
                Some(Menu::List) => menu_overlay(ui, list_menu(ctx, library)),
                _ => rsx! {},
            }
        }
    }
}

fn toggle_menu(mut ui: Signal<Ui>, menu: Menu) {
    let mut ui = ui.write();
    ui.menu = (ui.menu != Some(menu)).then_some(menu);
}

/// A section's name, which folds it away, with its count and a way to edit it.
/// A habit dragged here joins the section.
fn section_heading(ctx: Ctx, section: &Section, count: usize) -> Element {
    let mut ui = ctx.ui;
    let fold_id = section.id.clone();
    let edit_id = section.id.clone();
    let edit_name = section.name.clone();
    let chevron = if section.collapsed {
        Glyph::ChevronRight
    } else {
        Glyph::ChevronDown
    };
    rsx! {
        div { key: "section-{section.id}", class: "habit-section", "data-habit-section": "{section.id}",
            button {
                class: "habit-section-fold",
                aria_expanded: (!section.collapsed).to_string(),
                onclick: move |_| commit(ctx.saved, |library| {
                    if let Some(section) = library.section_mut(&fold_id) {
                        section.collapsed = !section.collapsed;
                    }
                }),
                {glyph(chevron, 16)}
                span { class: "habit-section-name", "{section.name}" }
                span { class: "habit-section-count", "{count}" }
            }
            {icon_button(glyph(Glyph::Pencil, 16), "Edit section", move |_| {
                ui.write().section_edit = Some(SectionEdit { id: Some(edit_id.clone()), name: edit_name.clone() });
            })}
        }
    }
}

/// Names a section in place. One that exists can also be moved or deleted,
/// which leaves its habits in the list.
fn section_editor(ctx: Ctx, edit: &SectionEdit) -> Element {
    let mut ui = ctx.ui;
    let mut save = move || {
        let Some(edit) = ui.write().section_edit.take() else {
            return;
        };
        let mut saved = ctx.saved;
        let mut saved = saved.write();
        if edit.save(&mut saved.library) {
            saved.save();
        }
    };
    let id = edit.id.clone();
    let key = id.clone().unwrap_or_else(|| "new".to_owned());
    rsx! {
        form {
            key: "section-edit-{key}",
            class: "habit-section-editor",
            onsubmit: move |event| {
                event.prevent_default();
                save();
            },
            input {
                class: "client-input",
                r#type: "text",
                placeholder: "Section name",
                aria_label: "Section name",
                autofocus: true,
                value: "{edit.name}",
                oninput: move |event| {
                    if let Some(edit) = &mut ui.write().section_edit {
                        edit.name = event.value();
                    }
                },
            }
            div { class: "habit-quiet-row",
                button { class: "btn small", r#type: "submit", "Save" }
                button { class: "btn small", r#type: "button", onclick: move |_| ui.write().section_edit = None, "Cancel" }
                if let Some(id) = id {
                    {
                        let (up, down, delete) = (id.clone(), id.clone(), id);
                        rsx! {
                            button { class: "btn small", r#type: "button",
                                onclick: move |_| commit(ctx.saved, |library| library.shift_section(&up, true)),
                                "Up"
                            }
                            button { class: "btn small", r#type: "button",
                                onclick: move |_| commit(ctx.saved, |library| library.shift_section(&down, false)),
                                "Down"
                            }
                            button { class: "btn small", r#type: "button",
                                onclick: move |_| {
                                    ui.write().section_edit = None;
                                    commit(ctx.saved, |library| library.remove_section(&delete));
                                },
                                "Delete"
                            }
                        }
                    }
                }
            }
        }
    }
}

fn habit_row(ctx: Ctx, habit: &Habit, computed: &Computed, dragging: bool) -> Element {
    let mut ui = ctx.ui;
    let today = ctx.today;
    let score = computed.score(today);
    let id = habit.id.clone();
    let cells = (0..DAY_COLUMNS).map(|back| {
        let day = today - Duration::days(back);
        let status = computed.status(day);
        day_button(ctx, habit, day, status)
    });
    rsx! {
        div {
            key: "{habit.id}",
            class: if dragging { "habit-row dragging" } else { "habit-row" },
            style: "{habit_color(habit.color)}",
            "data-habit-row": "{habit.id}",
            button {
                class: "habit-identity",
                "data-habit-grab": "{habit.id}",
                onclick: move |_| ui.write().open(id.clone()),
                charts::Ring { score, size: 18.0 }
                span { class: "habit-name", "{habit.name}" }
            }
            {cells}
        }
    }
}

fn day_button(ctx: Ctx, habit: &Habit, day: NaiveDate, status: Status) -> Element {
    let id = habit.id.clone();
    let label = format!(
        "{} on {} {}",
        habit.name,
        weekday_name(day),
        day.format("%b %-d")
    );
    rsx! {
        button {
            key: "{day}",
            class: "habit-day-button",
            aria_label: "{label}",
            onclick: move |_| open_day(ctx, &id, day),
            charts::DayMark { mark: Mark::of(habit, status, day) }
        }
    }
}

fn open_day(ctx: Ctx, id: &str, day: NaiveDate) {
    let saved = ctx.saved.peek();
    let Some(habit) = saved.library.habit(id) else {
        return;
    };
    let dialog = DayDialog::new(habit, day, ctx.time_format);
    drop(saved);
    let mut ui = ctx.ui;
    ui.write().day = Some(dialog);
}

// -------------------------------------------------------------------- menus

fn menu_overlay(mut ui: Signal<Ui>, menu: Element) -> Element {
    rsx! {
        div { class: "habit-menu-dismiss", onclick: move |_| ui.write().menu = None }
        div { class: "habit-menu", role: "menu", {menu} }
    }
}

fn menu_item(label: String, onclick: impl FnMut(MouseEvent) + 'static) -> Element {
    rsx! {
        button { class: "habit-menu-item", role: "menuitem", onclick, "{label}" }
    }
}

fn check_item(label: &'static str, checked: bool, onchange: impl FnMut(bool) + 'static) -> Element {
    let mut onchange = onchange;
    rsx! {
        label { class: "habit-check",
            input {
                r#type: "checkbox",
                checked,
                onchange: move |event: FormEvent| onchange(event.checked()),
            }
            span { "{label}" }
        }
    }
}

fn filter_menu(ctx: Ctx, library: &Library) -> Element {
    let mut ui = ctx.ui;
    let items = SortKey::ALL.map(|key| {
        let marker = if library.sort != key {
            ""
        } else if key == SortKey::Manual {
            " ✓"
        } else if library.reversed {
            " ↑"
        } else {
            " ↓"
        };
        menu_item(format!("{key}{marker}"), move |_| {
            ui.write().menu = None;
            commit(ctx.saved, |library| library.choose_sort(key));
        })
    });
    rsx! {
        {check_item("Hide archived", library.hide_archived, move |hide| commit(ctx.saved, |library| library.hide_archived = hide))}
        {check_item("Hide completed", library.hide_completed, move |hide| commit(ctx.saved, |library| library.hide_completed = hide))}
        span { class: "habit-menu-label", "Sort" }
        {items.into_iter()}
    }
}

fn list_menu(ctx: Ctx, library: &Library) -> Element {
    let mut ui = ctx.ui;
    let habits = library.habits.clone();
    let sections = library.sections.clone();
    let today = ctx.today;
    rsx! {
        {menu_item("Add section".to_owned(), move |_| {
            let mut ui = ui.write();
            ui.menu = None;
            ui.section_edit = Some(SectionEdit { id: None, name: String::new() });
        })}
        {menu_item("Export as CSV".to_owned(), move |_| {
            ui.write().menu = None;
            let habits: Vec<&Habit> = habits.iter().collect();
            export(ui, &habits, &sections, None, today);
        })}
    }
}

/// Saves the habits as Loop's CSV files in a ZIP, in the downloads folder.
fn export(
    mut ui: Signal<Ui>,
    habits: &[&Habit],
    sections: &[Section],
    single: Option<&str>,
    today: NaiveDate,
) {
    let name = match single {
        Some(name) => format!("{} {today}.zip", name.replace('/', "-")),
        None => format!("TimeSlay Habits {today}.zip"),
    };
    let bytes = model::export_archive(habits, sections, today);
    let folder = downloads();
    let path = folder.join(&name);
    let written = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(&path, bytes));
    ui.write().notice = Some(match written {
        Ok(()) => format!("Exported to {}", path.display()),
        Err(error) => format!("Could not export: {error}"),
    });
}

/// Where exports go: the shared Download folder on Android, which any app may
/// add files to, and ~/Downloads elsewhere.
fn downloads() -> PathBuf {
    #[cfg(target_os = "android")]
    {
        PathBuf::from("/storage/emulated/0/Download")
    }
    #[cfg(not(target_os = "android"))]
    {
        let home = std::env::var_os("HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
        let downloads = home.join("Downloads");
        if downloads.is_dir() { downloads } else { home }
    }
}

// ------------------------------------------------------------- the habit page

fn card(content: Element) -> Element {
    rsx! { section { class: "habit-card", {content} } }
}

fn card_title(title: &'static str) -> Element {
    rsx! { h3 { class: "habit-card-title", "{title}" } }
}

fn signed_percent(value: f64) -> String {
    let percent = (value * 100.0).round() as i64;
    if percent > 0 {
        format!("+{percent}%")
    } else {
        format!("{percent}%")
    }
}

/// Loop's colored bar: back, the name, then the actions.
fn page_header(ctx: Ctx, title: String, actions: Element) -> Element {
    let mut ui = ctx.ui;
    rsx! {
        header { class: "habit-bar",
            button { class: "habit-bar-button", title: "Back", aria_label: "Back",
                onclick: move |_| ui.write().back(),
                {glyph(Glyph::Back, 16)}
            }
            h2 { class: "habit-bar-title", "{title}" }
            {actions}
        }
    }
}

fn overview(ctx: Ctx, computed: &Computed) -> Element {
    let today = ctx.today;
    let now = computed.score(today);
    let stats = [
        (format!("{}%", (now * 100.0).round() as i64), "Score"),
        (
            signed_percent(now - computed.score(today - Duration::days(30))),
            "Month",
        ),
        (
            signed_percent(now - computed.score(today - Duration::days(365))),
            "Year",
        ),
        (model::format_amount(computed.total()), "Total"),
    ];
    card(rsx! {
        {card_title("Overview")}
        div { class: "habit-overview",
            div { class: "habit-overview-ring", charts::Ring { score: now, size: 44.0 } }
            for (value, name) in stats {
                div { key: "{name}", class: "habit-stat",
                    span { class: "habit-stat-value", "{value}" }
                    span { class: "habit-stat-name", "{name}" }
                }
            }
        }
    })
}

/// A card title with Loop's period spinner beside it.
fn titled_spinner(
    title: &'static str,
    options: &'static [Bucket],
    selected: Bucket,
    mut on_select: impl FnMut(Bucket) + 'static,
) -> Element {
    rsx! {
        div { class: "habit-card-head",
            {card_title(title)}
            select {
                class: "client-input habit-spinner",
                aria_label: "{title} period",
                onchange: move |event: FormEvent| {
                    if let Some(bucket) = options.iter().find(|bucket| bucket.to_string() == event.value()) {
                        on_select(*bucket);
                    }
                },
                for bucket in options.iter().copied() {
                    option { key: "{bucket}", value: "{bucket}", selected: bucket == selected, "{bucket}" }
                }
            }
        }
    }
}

fn habit_page(
    ctx: Ctx,
    habit: &Habit,
    computed: Computed,
    settings: &CalendarSettings,
    signed_in: bool,
    on_open_event: EventHandler<Event>,
    on_show_calendar: EventHandler<CalendarFilter>,
) -> Element {
    let mut ui = ctx.ui;
    let library = ctx.saved.read().library.clone();
    let today = ctx.today;
    let id = habit.id.clone();

    let mut goal = habit.goal_label();
    if habit.archived {
        goal.push_str(" · Archived");
    }
    let measure = habit
        .measure
        .as_ref()
        .map(|measure| measure.label(habit.kind));
    let streaks = computed.best_streaks(10);
    let target = timeslay_core::notes::Link {
        kind: timeslay_core::notes::LinkKind::Habit,
        id: habit.id.clone(),
        title: habit.name.clone(),
        day: None,
    };
    let value = habit.measure.as_ref().map(|measure| {
        let series = model::value_series(habit, FIRST_WEEKDAY, today);
        let chart = if series.is_empty() {
            rsx! { p { class: "habit-empty", "Nothing yet" } }
        } else {
            rsx! { charts::HistoryChart { series, bucket: measure.bucket(), target: measure.target } }
        };
        card(rsx! { {card_title("Value")} {chart} })
    });
    let error = ctx.saved.read().error.clone();
    let menu = ui.read().menu;
    let edit_id = id.clone();
    let day_id = id.clone();

    rsx! {
        div { class: "habit-page", style: "{habit_color(habit.color)}",
            {page_header(ctx, habit.name.clone(), rsx! {
                button { class: "habit-bar-button", title: "Edit", aria_label: "Edit",
                    onclick: move |_| {
                        let saved = ctx.saved.peek();
                        let Some(habit) = saved.library.habit(&edit_id) else { return };
                        let form = Form::from_habit(habit);
                        drop(saved);
                        let mut ui = ui.write();
                        ui.menu = None;
                        ui.page = Page::Form(Box::new(form));
                    },
                    {glyph(Glyph::Pencil, 16)}
                }
                button { class: "habit-bar-button", title: "More", aria_label: "More",
                    onclick: move |_| toggle_menu(ui, Menu::Habit),
                    {glyph(Glyph::Dots, 16)}
                }
            })}
            div { class: "habit-page-scroll",
                div { class: "habit-summary",
                    if !habit.question.is_empty() {
                        p { class: "habit-question", "{habit.question}" }
                    }
                    p { class: "habit-goal", {glyph(Glyph::Calendar, 14)} span { "{goal}" } }
                    if let Some(measure) = measure {
                        p { class: "habit-summary-line", "{measure}" }
                    }
                    if !habit.notes.is_empty() {
                        p { class: "habit-summary-line habit-notes", "{habit.notes}" }
                    }
                }
                div { class: "habit-cards",
                    {overview(ctx, &computed)}
                    {card(rsx! { crate::notes::LinkedNotes { target } })}
                    {card(rsx! {
                        {titled_spinner("Score", &Bucket::SCORE, library.score_bucket, move |bucket| commit(ctx.saved, |library| library.score_bucket = bucket))}
                        charts::ScoreChart {
                            series: model::score_series(&computed, library.score_bucket, FIRST_WEEKDAY, today),
                            bucket: library.score_bucket,
                        }
                    })}
                    {card(rsx! {
                        {titled_spinner("History", &Bucket::HISTORY, library.history_bucket, move |bucket| commit(ctx.saved, |library| library.history_bucket = bucket))}
                        charts::HistoryChart {
                            series: model::history_series(&computed, library.history_bucket, FIRST_WEEKDAY, today),
                            bucket: library.history_bucket,
                            target: None,
                        }
                    })}
                    {value}
                    {card(rsx! {
                        {card_title("Calendar")}
                        charts::CalendarChart {
                            computed: computed.clone(),
                            today,
                            on_day: move |day| open_day(ctx, &day_id, day),
                        }
                    })}
                    {card(rsx! {
                        {card_title("Best streaks")}
                        if streaks.is_empty() {
                            p { class: "habit-empty", "Nothing yet" }
                        } else {
                            charts::StreakChart { streaks }
                        }
                    })}
                    {card(rsx! {
                        {card_title("Frequency")}
                        charts::FrequencyChart { computed: computed.clone(), today }
                    })}
                    // A habit to avoid has nothing to schedule.
                    if habit.kind != Kind::No {
                        {events_card(ctx, habit, settings, signed_in, on_open_event, on_show_calendar)}
                    }
                    if let Some(error) = error {
                        p { class: "habit-error", role: "alert", "{error}" }
                    }
                }
            }
            if menu == Some(Menu::Habit) {
                {menu_overlay(ui, habit_menu(ctx, habit))}
            }
        }
    }
}

fn habit_menu(ctx: Ctx, habit: &Habit) -> Element {
    let mut ui = ctx.ui;
    let (archive_id, delete_id) = (habit.id.clone(), habit.id.clone());
    let exported = habit.clone();
    let today = ctx.today;
    rsx! {
        {menu_item(if habit.archived { "Unarchive" } else { "Archive" }.to_owned(), move |_| {
            ui.write().menu = None;
            commit(ctx.saved, |library| {
                if let Some(habit) = library.habit_mut(&archive_id) {
                    habit.archived = !habit.archived;
                }
            });
        })}
        {menu_item("Export as CSV".to_owned(), move |_| {
            ui.write().menu = None;
            let sections = ctx.saved.peek().library.sections.clone();
            export(ui, &[&exported], &sections, Some(&exported.name), today);
        })}
        {menu_item("Delete".to_owned(), move |_| {
            let mut ui = ui.write();
            ui.menu = None;
            ui.delete_events = false;
            ui.confirm_delete = Some(delete_id.clone());
        })}
    }
}

/// Connects a habit event to a habit, or lets it go, then reloads the events.
fn link(ctx: Ctx, settings: CalendarSettings, series: Series, habit_id: Option<String>) {
    let mut ui = ctx.ui;
    ui.write().busy = true;
    ui.write().error = None;
    spawn(async move {
        let (calendar_id, event_id, link) = (
            series.calendar_id.clone(),
            series.id.clone(),
            habit_id.clone(),
        );
        let result = worker(move || {
            calendar::link_habit_event(
                &calendar::GoogleClient::from_settings(&settings),
                &calendar_id,
                &event_id,
                link.as_deref(),
            )
        })
        .await;
        ui.write().busy = false;
        match result {
            Ok(()) => {
                // Remembered as well, so an event let go of is not claimed again.
                commit(ctx.saved, |library| {
                    library
                        .adopted
                        .insert(series.id.clone(), habit_id.unwrap_or_default());
                });
                let mut reload = ctx.reload_series;
                reload += 1;
            }
            Err(message) => ui.write().error = Some(message),
        }
    });
}

/// The habit events that count towards this habit. This part has no
/// counterpart in Loop, and is what ties a habit to the calendar.
fn events_card(
    ctx: Ctx,
    habit: &Habit,
    settings: &CalendarSettings,
    signed_in: bool,
    on_open_event: EventHandler<Event>,
    on_show_calendar: EventHandler<CalendarFilter>,
) -> Element {
    let library = ctx.saved.read().library.clone();
    let all = ctx.series.read().clone();
    let events: Vec<Series> = events_of(&library, &all, &habit.id)
        .into_iter()
        .cloned()
        .collect();
    let unlinked: Vec<Series> = all
        .iter()
        .filter(|series| habit_of_series(&library, series).is_none())
        .cloned()
        .collect();
    let busy = ctx.ui.read().busy;
    let error = ctx.ui.read().error.clone();
    let time_format = ctx.time_format;
    let new = new_event(habit, ctx.today, settings);
    let filter = CalendarFilter {
        name: habit.name.clone(),
        series: events.iter().map(|series| series.id.clone()).collect(),
    };
    let no_events = events.is_empty();
    let connect_settings = settings.clone();
    let habit_id = habit.id.clone();
    let rows = events.into_iter().map(|series| {
        let dot = crate::css_color(series.color);
        let when = format!(
            "{} · {}",
            series.schedule,
            time_format::range(series.start, series.end, time_format)
        );
        let event = series.event.clone();
        let settings = settings.clone();
        let unlink = series.clone();
        rsx! {
            div { key: "{series.id}", class: "habit-event",
                div { class: "habit-event-line",
                    span { class: "day-dot", style: "background: {dot};" }
                    span { class: "habit-event-title", "{series.title}" }
                    button { class: "btn small", disabled: busy, onclick: move |_| on_open_event.call(event.clone()), "Edit" }
                    button { class: "btn small", disabled: busy,
                        onclick: move |_| link(ctx, settings.clone(), unlink.clone(), None),
                        "Disconnect"
                    }
                }
                span { class: "habit-event-when", "{when}" }
            }
        }
    });
    card(rsx! {
        {card_title("Calendar events")}
        if no_events {
            p { class: "habit-hint", "Nothing on the calendar yet. Doing the habit is tracked either way." }
        }
        {rows}
        div { class: "habit-quiet-row",
            button { class: "btn small", disabled: !signed_in || busy,
                onclick: move |_| on_open_event.call(new.clone()),
                "Add event"
            }
            button { class: "btn small", disabled: no_events,
                onclick: move |_| on_show_calendar.call(filter.clone()),
                "Show on calendar"
            }
        }
        if !unlinked.is_empty() {
            select {
                class: "client-input",
                aria_label: "Connect an existing event",
                disabled: busy,
                onchange: move |event: FormEvent| {
                    let chosen = unlinked.iter().find(|series| series.id == event.value()).cloned();
                    if let Some(series) = chosen {
                        link(ctx, connect_settings.clone(), series, Some(habit_id.clone()));
                    }
                },
                option { value: "", selected: true, disabled: true, "Connect an existing event…" }
                for series in unlinked.iter() {
                    option { key: "{series.id}", value: "{series.id}", "{series.title} · {series.schedule}" }
                }
            }
        }
        if !signed_in {
            p { class: "habit-hint", "Connect Google Calendar in Settings to add events." }
        }
        if let Some(error) = error {
            p { class: "habit-error", role: "alert", "{error}" }
        }
    })
}

// --------------------------------------------------------- add and edit page

fn edit(mut ui: Signal<Ui>, action: FormAction) {
    if let Page::Form(form) = &mut ui.write().page {
        form.apply(action);
    }
}

fn field(name: &'static str, content: Element) -> Element {
    rsx! {
        div { class: "habit-field",
            span { class: "habit-field-label", "{name}" }
            {content}
        }
    }
}

fn hint(text: &'static str) -> Element {
    rsx! { p { class: "habit-hint", "{text}" } }
}

/// A switch with its label, as the desktop's togglers are.
fn toggle(label: &'static str, on: bool, mut onchange: impl FnMut(bool) + 'static) -> Element {
    rsx! {
        button {
            class: "habit-toggle",
            r#type: "button",
            role: "switch",
            aria_checked: on.to_string(),
            onclick: move |_| onchange(!on),
            span { class: "habit-toggle-label", "{label}" }
            span { class: "source-switch", span { class: "source-knob" } }
        }
    }
}

fn choice_buttons<T: Copy + PartialEq + 'static>(
    choices: Vec<(T, &'static str)>,
    selected: T,
    locked: bool,
    on_pick: impl Fn(T) + Copy + 'static,
) -> Element {
    rsx! {
        div { class: "habit-choices",
            for (value, label) in choices {
                button {
                    key: "{label}",
                    r#type: "button",
                    class: if value == selected { "btn primary" } else { "btn" },
                    aria_pressed: (value == selected).to_string(),
                    disabled: locked,
                    onclick: move |_| on_pick(value),
                    "{label}"
                }
            }
        }
    }
}

fn number_input(
    value: &str,
    label: &'static str,
    wrap: fn(String) -> FormAction,
    ui: Signal<Ui>,
) -> Element {
    rsx! {
        input {
            class: "client-input habit-number",
            r#type: "text",
            inputmode: "numeric",
            aria_label: "{label}",
            value: "{value}",
            oninput: move |event| edit(ui, wrap(event.value())),
        }
    }
}

fn frequency_section(ui: Signal<Ui>, form: &Form) -> Element {
    let radio = |kind: FrequencyKind, label: &'static str| {
        rsx! {
            input {
                r#type: "radio",
                name: "habit-frequency",
                aria_label: "{label}",
                checked: form.kind == kind,
                onchange: move |_| edit(ui, FormAction::Kind(kind)),
            }
        }
    };
    let words = |text: &'static str| rsx! { span { "{text}" } };
    rsx! {
        div { class: "habit-frequency",
            label { class: "habit-radio", {radio(FrequencyKind::EveryDay, "Every day")} {words("Every day")} }
            div { class: "habit-radio",
                {radio(FrequencyKind::TimesPerDay, "Times per day")}
                {number_input(&form.per_day, "Times per day", FormAction::PerDay, ui)}
                {words("times per day")}
            }
            div { class: "habit-radio",
                {radio(FrequencyKind::EveryXDays, "Every few days")}
                {words("Every")}
                {number_input(&form.every_days, "Days apart", FormAction::EveryDays, ui)}
                {words("days")}
            }
            div { class: "habit-radio",
                {radio(FrequencyKind::TimesPerWeek, "Times per week")}
                {number_input(&form.per_week, "Times per week", FormAction::PerWeek, ui)}
                {words("times per week")}
            }
            div { class: "habit-radio",
                {radio(FrequencyKind::TimesPerMonth, "Times per month")}
                {number_input(&form.per_month, "Times per month", FormAction::PerMonth, ui)}
                {words("times per month")}
            }
            div { class: "habit-radio",
                {radio(FrequencyKind::TimesInDays, "Times in days")}
                {number_input(&form.in_times, "Times", FormAction::InTimes, ui)}
                {words("times in")}
                {number_input(&form.in_days, "Days", FormAction::InDays, ui)}
                {words("days")}
            }
            label { class: "habit-radio", {radio(FrequencyKind::Weekdays, "On these days")} {words("On these days")} }
            div { class: "habit-weekdays",
                for (index, name) in WEEKDAYS.iter().enumerate() {
                    button {
                        key: "{name}",
                        r#type: "button",
                        class: if form.weekdays[index] { "btn small primary" } else { "btn small" },
                        aria_pressed: form.weekdays[index].to_string(),
                        onclick: move |_| edit(ui, FormAction::Weekday(index)),
                        "{name}"
                    }
                }
            }
        }
    }
}

/// A number to record each day, on either type of habit.
fn measure_section(ui: Signal<Ui>, form: &Form) -> Element {
    let explanation = match form.habit_kind {
        Kind::Yes => {
            "Record a number each day, such as miles run. The target is what you aim to reach, and it is drawn on the Value chart. The number does not change the score."
        }
        Kind::No => {
            "Record a number each day, such as how many you had. The target is the most you aim to stay under, and it is drawn on the Value chart. The number does not change the score."
        }
    };
    let period = |days: u32, label: &'static str| {
        rsx! {
            label { key: "{label}", class: "habit-radio",
                input {
                    r#type: "radio",
                    name: "habit-target-period",
                    checked: form.target_days == days,
                    onchange: move |_| edit(ui, FormAction::TargetDays(days)),
                }
                span { "{label}" }
            }
        }
    };
    rsx! {
        div { class: "habit-field",
            {toggle("Track a value", form.measured, move |value| edit(ui, FormAction::Measured(value)))}
            {hint(explanation)}
            if form.measured {
                div { class: "habit-measure",
                    {field("Target", rsx! {
                        input {
                            class: "client-input",
                            r#type: "text",
                            inputmode: "decimal",
                            placeholder: "(Optional)",
                            value: "{form.target}",
                            oninput: move |event| edit(ui, FormAction::Target(event.value())),
                        }
                    })}
                    {field("Unit", rsx! {
                        input {
                            class: "client-input",
                            r#type: "text",
                            placeholder: "e.g. miles",
                            value: "{form.unit}",
                            oninput: move |event| edit(ui, FormAction::Unit(event.value())),
                        }
                    })}
                }
                div { class: "habit-periods",
                    {period(1, "Per day")}
                    {period(7, "Per week")}
                    {period(30, "Per month")}
                }
            }
        }
    }
}

fn save_form(ctx: Ctx) {
    let mut ui = ctx.ui;
    let Page::Form(form) = ui.peek().page.clone() else {
        return;
    };
    let mut saved = ctx.saved;
    let result = {
        let mut saved = saved.write();
        let result = form.save(&mut saved.library, ctx.today);
        if result.is_ok() {
            saved.save();
        }
        result
    };
    match result {
        Ok(id) => ui.write().open(id),
        Err(error) => {
            if let Page::Form(form) = &mut ui.write().page {
                form.error = Some(error);
            }
        }
    }
}

fn form_page(ctx: Ctx, library: &Library, form: &Form) -> Element {
    let ui = ctx.ui;
    let title = if form.id.is_some() {
        "Edit habit"
    } else {
        "Create habit"
    };
    let kind_hint = match form.habit_kind {
        Kind::Yes => "Tick it off when you do it. Doing it again that day counts the same.",
        Kind::No => "Something to avoid. Every day you don't do it counts as a success.",
    };
    let sections: Vec<(String, String)> = library
        .sections
        .iter()
        .map(|section| (section.id.clone(), section.name.clone()))
        .collect();
    let section = form.section.clone().unwrap_or_default();
    let samples = [Status::Yes, Status::No, Status::Skip].map(|status| {
        rsx! { charts::DayMark { mark: Mark::sample(status, form.icons), width: 32.0, height: 32.0 } }
    });

    rsx! {
        div { class: "habit-page", style: "{habit_color(form.color)}",
            {page_header(ctx, title.to_owned(), rsx! {
                button { class: "habit-bar-save", onclick: move |_| save_form(ctx), "Save" }
            })}
            form {
                class: "habit-page-scroll habit-form",
                onsubmit: move |event| {
                    event.prevent_default();
                    save_form(ctx);
                },
                {field("Type", rsx! {
                    {choice_buttons(vec![(Kind::Yes, Kind::Yes.label()), (Kind::No, Kind::No.label())], form.habit_kind, form.id.is_some(), move |kind| edit(ui, FormAction::HabitKind(kind)))}
                    {hint(kind_hint)}
                })}
                {field("Name", rsx! {
                    input {
                        class: "client-input",
                        r#type: "text",
                        placeholder: "e.g. Exercise",
                        aria_label: "Name",
                        autofocus: form.id.is_none(),
                        value: "{form.name}",
                        oninput: move |event| edit(ui, FormAction::Name(event.value())),
                    }
                })}
                {field("Question", rsx! {
                    input {
                        class: "client-input",
                        r#type: "text",
                        placeholder: "e.g. Did you exercise today?",
                        aria_label: "Question",
                        value: "{form.question}",
                        oninput: move |event| edit(ui, FormAction::Question(event.value())),
                    }
                })}
                if !sections.is_empty() {
                    {field("Section", rsx! {
                        select {
                            class: "client-input",
                            aria_label: "Section",
                            onchange: move |event: FormEvent| {
                                let value = event.value();
                                edit(ui, FormAction::Section((!value.is_empty()).then_some(value)));
                            },
                            option { value: "", selected: section.is_empty(), "No section" }
                            for (id, name) in sections {
                                option { key: "{id}", value: "{id}", selected: id == section, "{name}" }
                            }
                        }
                    })}
                }
                {field("Color", rsx! {
                    div { class: "habit-swatches",
                        for index in 0..PALETTE_SIZE {
                            button {
                                key: "{index}",
                                r#type: "button",
                                class: "habit-swatch",
                                style: "background: var(--habit-{index});",
                                aria_label: "Color {index + 1}",
                                aria_pressed: (index == form.color).to_string(),
                                onclick: move |_| edit(ui, FormAction::Color(index)),
                            }
                        }
                    }
                })}
                {field("Icons", rsx! {
                    {choice_buttons(vec![(Icons::Symbols, "Symbols"), (Icons::Faces, "Faces")], form.icons, false, move |icons| edit(ui, FormAction::Icons(icons)))}
                    div { class: "habit-samples", {samples.into_iter()} }
                })}
                if form.habit_kind == Kind::Yes {
                    {field("Frequency", frequency_section(ui, form))}
                }
                {measure_section(ui, form)}
                div { class: "habit-field",
                    {toggle("Enable skip days", form.skip_days, move |value| edit(ui, FormAction::SkipDays(value)))}
                    {hint("Adds \"Skipped\" to each day's choices. Skipped days pause the score and keep the streak going, for when the habit could not be done.")}
                }
                div { class: "habit-field",
                    {toggle("Show question mark for missing data", form.question_marks, move |value| edit(ui, FormAction::QuestionMarks(value)))}
                    {hint("Days with nothing recorded show a question mark rather than a cross, and on a habit to do, each day's choices add \"Not sure\" to return it to having no data.")}
                }
                {field("Notes", rsx! {
                    textarea {
                        class: "client-input habit-notes-input",
                        rows: "4",
                        placeholder: "(Optional)",
                        aria_label: "Notes",
                        value: "{form.notes}",
                        oninput: move |event| edit(ui, FormAction::Notes(event.value())),
                    }
                })}
                if let Some(error) = &form.error {
                    p { class: "habit-error", role: "alert", "{error}" }
                }
            }
        }
    }
}

// ------------------------------------------------------------------- dialogs

fn with_dialog(mut ui: Signal<Ui>, change: impl FnOnce(&mut DayDialog)) {
    if let Some(dialog) = &mut ui.write().day {
        change(dialog);
    }
}

/// Completes or reopens the occurrences loaded for `day` to match the habit,
/// so recording a day also shows on the calendar. Local data stays the record:
/// days that are not loaded, or a failed write, leave only the habit changed.
fn sync_occurrences(ctx: Ctx, id: &str, day: NaiveDate, events: &[Event]) {
    let library = ctx.saved.peek().library.clone();
    let Some(habit) = library.habit(id) else {
        return;
    };
    // Only a "Yes" habit is done by completing an event.
    if habit.kind != Kind::Yes {
        return;
    }
    let times = habit.times_on(day) as usize;
    let wanted = match habit.compute(ctx.today).status(day) {
        Status::Yes | Status::Partial(_) => times,
        _ => 0,
    };
    let series = ctx.series.peek().clone();
    let occurrences: Vec<Event> = events
        .iter()
        .filter(|event| {
            event.details.habit
                && event.start.date_naive() == day
                && habit_of(&library, &series, event) == Some(id)
        })
        .cloned()
        .collect();
    let done = occurrences.iter().filter(|event| event.completed).count();
    let writes: Vec<Event> = if done < wanted {
        occurrences
            .into_iter()
            .filter(|event| !event.completed)
            .take(wanted - done)
            .map(|mut event| {
                event.completed = true;
                event.details.habit_completed_at = Some(timeslay_core::clock::now());
                event
            })
            .collect()
    } else {
        occurrences
            .into_iter()
            .rev()
            .filter(|event| event.completed)
            .take(done - wanted)
            .map(|mut event| {
                event.completed = false;
                event.details.habit_completed_at = None;
                event
            })
            .collect()
    };
    if writes.is_empty() {
        return;
    }
    let mut ui = ctx.ui;
    spawn(async move {
        let result = worker(move || {
            for event in &writes {
                calendar::set_habit_completed(event)?;
            }
            Ok(())
        })
        .await;
        match result {
            Ok(()) => {
                let mut revision = ctx.revision;
                revision += 1;
            }
            Err(message) => ui.write().error = Some(message),
        }
    });
}

/// Everything a day can record, opened by a tap on any of its marks. Nothing
/// changes until it is saved.
fn day_dialog(ctx: Ctx, library: &Library, events: Vec<Event>) -> Element {
    let mut ui = ctx.ui;
    let Some(dialog) = ui.read().day.clone() else {
        return rsx! {};
    };
    let Some(habit) = library.habit(&dialog.habit_id) else {
        return rsx! {};
    };
    let question = if !habit.question.is_empty() {
        habit.question.clone()
    } else {
        match habit.kind {
            Kind::Yes => "Did you do it?",
            Kind::No => "Did you avoid it?",
        }
        .to_owned()
    };
    let counted = |label: &str, count: usize| {
        if count > 0 {
            format!("{label} · {count}")
        } else {
            label.to_owned()
        }
    };
    let (good, bad) = match habit.kind {
        Kind::Yes => (
            ("Did it".to_owned(), Outcome::Done),
            ("Didn't".to_owned(), Outcome::NotDone),
        ),
        Kind::No => (
            (
                counted("Avoided it", dialog.resisted.len()),
                Outcome::NotDone,
            ),
            (counted("Slipped", dialog.happened.len()), Outcome::Done),
        ),
    };
    let mut choices = vec![good, bad];
    if habit.skip_days {
        choices.push(("Skipped".to_owned(), Outcome::Skipped));
    }
    // A habit to avoid is answered by its logs, so it has no "Not sure".
    if habit.question_marks && habit.kind == Kind::Yes {
        choices.push(("Not sure".to_owned(), Outcome::Unanswered));
    }
    let outcomes = choices.into_iter().map(|(label, outcome)| {
        // On a habit to avoid, "Avoided it" and "Slipped" pick which log shows.
        let on = if dialog.avoid && matches!(outcome, Outcome::Done | Outcome::NotDone) {
            dialog.view == outcome
                && !matches!(dialog.outcome, Outcome::Skipped | Outcome::Unanswered)
        } else {
            dialog.outcome == outcome
        };
        rsx! {
            button {
                key: "{label}",
                class: if on { "btn primary" } else { "btn" },
                aria_pressed: on.to_string(),
                onclick: move |_| with_dialog(ui, |dialog| dialog.choose(outcome)),
                "{label}"
            }
        }
    });
    let target = habit
        .measure
        .as_ref()
        .filter(|measure| measure.target.is_some())
        .map(|measure| measure.label(habit.kind));
    let unit = habit.measure.as_ref().map(|measure| measure.unit.clone());
    let log = if dialog.shows_happened() {
        let add = if habit.kind == Kind::Yes && dialog.goal > 1 {
            format!("Add entry ({} of {})", dialog.times(), dialog.goal)
        } else {
            "Add entry".to_owned()
        };
        Some(day_log(ui, &dialog, Log::Happened, add, unit.clone()))
    } else if dialog.avoid && dialog.outcome != Outcome::Skipped {
        Some(day_log(
            ui,
            &dialog,
            Log::Resisted,
            "Add entry".to_owned(),
            unit.clone(),
        ))
    } else {
        None
    };
    // Not having done a habit to do leaves nothing to measure.
    let didnt = habit.kind == Kind::Yes && dialog.outcome == Outcome::NotDone;
    let value = habit.measure.as_ref().filter(|_| !didnt).and_then(|measure| {
        // A habit to avoid records amounts on its entries alone.
        if dialog.avoid && (!dialog.shows_happened() || dialog.happened.is_empty()) {
            None
        } else if dialog.happened.is_empty() || dialog.outcome != Outcome::Done {
            let label = if measure.unit.is_empty() {
                "Value".to_owned()
            } else {
                format!("Value in {}", measure.unit)
            };
            Some(rsx! {
                label { class: "habit-field",
                    span { class: "habit-field-label", "{label}" }
                    input {
                        class: "client-input",
                        r#type: "text",
                        inputmode: "decimal",
                        placeholder: "None",
                        value: "{dialog.value}",
                        oninput: move |event| with_dialog(ui, |dialog| dialog.set_value(event.value())),
                    }
                }
            })
        } else if dialog.shows_happened()
            && let Some(total) = dialog.logged_total()
        {
            // The day's value is what its entries add up to.
            Some(rsx! {
                p { class: "habit-hint", "Total: {model::format_amount(total)} {measure.unit}" }
            })
        } else {
            None
        }
    });
    let heading = format!(
        "{} · {} {}",
        habit.name,
        weekday_name(dialog.day),
        dialog.day.format("%b %-d")
    );
    let mut save = move |events: &[Event]| {
        let Some(dialog) = ui.peek().day.clone() else {
            return;
        };
        let mut saved = ctx.saved;
        let recorded = {
            let mut saved = saved.write();
            let recorded = dialog.record(&mut saved.library);
            if recorded.is_ok() {
                saved.save();
            }
            recorded
        };
        match recorded {
            Ok(()) => {
                ui.write().day = None;
                sync_occurrences(ctx, &dialog.habit_id, dialog.day, events);
            }
            Err(error) => with_dialog(ui, |dialog| dialog.error = Some(error)),
        }
    };
    rsx! {
        div { class: "modal-backdrop habit-dialog-backdrop", onclick: move |_| ui.write().day = None,
            div {
                class: "habit-dialog",
                role: "dialog",
                aria_label: "{heading}",
                style: "{habit_color(habit.color)}",
                onclick: move |event| event.stop_propagation(),
                h2 { class: "habit-dialog-title", "{heading}" }
                p { class: "habit-hint", "{question}" }
                if let Some(target) = target {
                    p { class: "habit-hint", "Target: {target}" }
                }
                div { class: "habit-outcomes", {outcomes} }
                {log}
                {value}
                if let Some(error) = &dialog.error {
                    p { class: "habit-error", role: "alert", "{error}" }
                }
                div { class: "habit-dialog-actions",
                    button { class: "btn", onclick: move |_| ui.write().day = None, "Cancel" }
                    button { class: "btn primary", onclick: move |_| save(&events), "Save" }
                }
            }
        }
    }
}

/// One of the day dialog's logs: a button that logs an entry at the current
/// time, and each entry with its time, its amount when `unit` is given, an X
/// to remove it, and a comment under them. All can be changed in place.
fn day_log(
    ui: Signal<Ui>,
    dialog: &DayDialog,
    log: Log,
    add: String,
    unit: Option<String>,
) -> Element {
    let entries = dialog.log(log).iter().enumerate().map(|(index, entry)| {
        let unit = unit.clone();
        let wide = unit.is_none();
        rsx! {
            div { key: "{index}", class: "habit-log-entry",
                div { class: "habit-log-line",
                    input {
                        class: if wide { "client-input habit-log-time wide" } else { "client-input habit-log-time" },
                        r#type: "text",
                        placeholder: "No time recorded",
                        aria_label: "Time",
                        value: "{entry.time}",
                        oninput: move |event| with_dialog(ui, |dialog| dialog.edit(log, index, |row| row.time = event.value())),
                    }
                    if let Some(unit) = unit {
                        span { class: "habit-hint", "for" }
                        input {
                            class: "client-input habit-log-amount",
                            r#type: "text",
                            inputmode: "decimal",
                            placeholder: "0",
                            aria_label: "Amount",
                            value: "{entry.amount}",
                            oninput: move |event| with_dialog(ui, |dialog| dialog.edit(log, index, |row| row.amount = event.value())),
                        }
                        span { class: "habit-hint habit-log-unit", "{unit}" }
                    }
                    button {
                        class: "habit-log-remove",
                        aria_label: "Remove entry",
                        onclick: move |_| with_dialog(ui, |dialog| dialog.remove(log, index)),
                        "×"
                    }
                }
                input {
                    class: "client-input habit-log-note",
                    r#type: "text",
                    placeholder: "Comment",
                    aria_label: "Comment",
                    value: "{entry.note}",
                    oninput: move |event| with_dialog(ui, |dialog| dialog.edit(log, index, |row| row.note = event.value())),
                }
            }
        }
    });
    rsx! {
        div { class: "habit-log",
            button { class: "btn habit-log-add", onclick: move |_| with_dialog(ui, |dialog| dialog.add(log)), "+ {add}" }
            div { class: "habit-log-entries", {entries} }
        }
    }
}

/// The "are you sure" prompt for deleting a habit and its record.
fn delete_confirmation(ctx: Ctx, library: &Library, settings: CalendarSettings) -> Element {
    let mut ui = ctx.ui;
    let Some(id) = ui.read().confirm_delete.clone() else {
        return rsx! {};
    };
    let Some(habit) = library.habit(&id) else {
        return rsx! {};
    };
    let all = ctx.series.read().clone();
    let events: Vec<Series> = events_of(library, &all, &id).into_iter().cloned().collect();
    let count = events.len();
    let busy = ui.read().busy;
    let delete_events = ui.read().delete_events;
    let delete = move |_| {
        let id = id.clone();
        let events = events.clone();
        let settings = settings.clone();
        ui.write().busy = true;
        ui.write().error = None;
        spawn(async move {
            let ids: Vec<(String, String)> = events
                .iter()
                .map(|series| (series.calendar_id.clone(), series.id.clone()))
                .collect();
            let result = if ids.is_empty() {
                Ok(())
            } else {
                worker(move || {
                    for (calendar_id, event_id) in &ids {
                        if delete_events {
                            calendar::delete_habit(calendar_id, event_id)?;
                        } else {
                            // A kept event must not bring the habit back on the next load.
                            calendar::link_habit_event(
                                &calendar::GoogleClient::from_settings(&settings),
                                calendar_id,
                                event_id,
                                None,
                            )?;
                        }
                    }
                    Ok(())
                })
                .await
            };
            {
                let mut ui = ui.write();
                ui.busy = false;
                ui.confirm_delete = None;
            }
            if let Err(message) = result {
                ui.write().error = Some(message);
                let mut reload = ctx.reload_series;
                reload += 1;
                return;
            }
            commit(ctx.saved, |library| {
                for series in &events {
                    library.adopted.insert(series.id.clone(), String::new());
                }
                library.habits.retain(|habit| habit.id != id);
            });
            {
                let mut ui = ui.write();
                if ui.page == Page::Habit(id.clone()) {
                    ui.page = Page::List;
                }
            }
            if !events.is_empty() {
                let mut reload = ctx.reload_series;
                reload += 1;
            }
        });
    };
    rsx! {
        div { class: "modal-backdrop habit-dialog-backdrop", onclick: move |_| ui.write().confirm_delete = None,
            div { class: "habit-dialog", role: "alertdialog", aria_label: "Delete this habit?",
                onclick: move |event| event.stop_propagation(),
                h2 { class: "habit-dialog-title", "Delete this habit?" }
                p { class: "habit-hint", "{habit.name}" }
                p { class: "habit-hint", "Its whole record is removed. Archive it instead to keep the record." }
                if count > 0 {
                    {check_item(
                        if count == 1 { "Also delete its event from Google Calendar" } else { "Also delete its events from Google Calendar" },
                        delete_events,
                        move |delete| ui.write().delete_events = delete,
                    )}
                }
                div { class: "habit-dialog-actions even",
                    button { class: "btn", onclick: move |_| ui.write().confirm_delete = None, "Cancel" }
                    button { class: "btn danger", disabled: busy, onclick: delete, "Delete" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn habits_that_cannot_be_read_are_never_saved_over() {
        let directory = tempfile_dir();
        let path = directory.join("test.sqlite3");
        let key = timeslay_core::task_db::keys::HABITS;
        timeslay_core::task_db::write_document_json(&path, key, "not json").unwrap();
        let mut saved = read(Some(path.clone()));
        assert!(saved.error.is_some());
        saved.library.habits.push(Habit::new("a", "Run"));
        saved.save();
        assert!(read(Some(path)).error.is_some(), "still the bad contents");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn habits_survive_being_saved_and_read_back() {
        let directory = tempfile_dir();
        let path = directory.join("nested").join("test.sqlite3");
        let mut saved = read(Some(path.clone()));
        assert_eq!(saved.error, None);
        saved.library.habits.push(Habit::new("a", "Run"));
        saved.save();
        assert_eq!(saved.error, None);
        assert_eq!(read(Some(path)).library.habits.len(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn back_from_an_edit_returns_to_its_habit_and_from_anything_else_to_the_list() {
        let mut ui = Ui::default();
        ui.open("run".to_owned());
        let mut form = Form::new();
        form.id = Some("run".to_owned());
        ui.page = Page::Form(Box::new(form));
        ui.back();
        assert_eq!(ui.page, Page::Habit("run".to_owned()));
        ui.back();
        assert_eq!(ui.page, Page::List);
        ui.page = Page::Form(Box::default());
        ui.back();
        assert_eq!(ui.page, Page::List);
    }

    #[test]
    fn an_event_counts_towards_its_series_habit_then_its_own_then_the_adopted_one() {
        let mut library = Library::default();
        library.habits.push(Habit::new("a", "Run"));
        library.habits.push(Habit::new("b", "Read"));
        library.adopted.insert("old".to_owned(), "b".to_owned());
        let start = calendar::at_clock(NaiveDate::from_ymd_opt(2026, 11, 7).unwrap(), 9, 0);
        let mut event = Event {
            id: "old_1".to_owned(),
            calendar_id: "c".to_owned(),
            title: "Read".to_owned(),
            start,
            end: start,
            all_day: false,
            color: calendar::colors::DEFAULT_EVENT,
            completed: false,
            details: calendar::Details {
                habit: true,
                recurring_event_id: Some("old".to_owned()),
                ..Default::default()
            },
        };
        assert_eq!(habit_of(&library, &[], &event), Some("b"));
        event.details.habit_id = Some("a".to_owned());
        assert_eq!(habit_of(&library, &[], &event), Some("a"));
        event.details.habit_id = Some("gone".to_owned());
        assert_eq!(habit_of(&library, &[], &event), None);
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, 4).unwrap()
    }

    fn run() -> Habit {
        let mut habit = Habit::new("run", "Run");
        habit.created = Some(today() - Duration::days(20));
        habit.question = "Did you run today?".to_owned();
        habit.add_rep(today());
        habit.add_rep(today() - Duration::days(2));
        habit
    }

    #[derive(Props, Clone, PartialEq)]
    struct HarnessProps {
        ui: Ui,
        habits: Vec<Habit>,
    }

    #[allow(non_snake_case)]
    fn Harness(props: HarnessProps) -> Element {
        let saved = use_signal(|| for_tests(props.habits.clone()));
        let ui = use_signal(|| props.ui.clone());
        let ctx = Ctx {
            saved,
            ui,
            series: use_signal(Vec::new),
            reload_series: use_signal(|| 0),
            revision: use_signal(|| 0),
            today: today(),
            time_format: TimeFormat::TwentyFourHour,
        };
        screen(
            ctx,
            CalendarSettings::default(),
            false,
            Vec::new(),
            EventHandler::new(|_| {}),
            EventHandler::new(|_| {}),
        )
    }

    fn render(ui: Ui, habits: Vec<Habit>) -> String {
        let mut dom = VirtualDom::new_with_props(Harness, HarnessProps { ui, habits });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn assert_offers(html: &str, wanted: &[&str]) {
        for text in wanted {
            assert!(html.contains(text), "{text} is missing from {html}");
        }
    }

    #[test]
    fn an_empty_tab_offers_to_add_a_habit_without_needing_google() {
        let html = render(Ui::default(), Vec::new());
        assert_offers(
            &html,
            &[
                ">Habits<",
                "Add habit",
                "Sort and filter",
                "No habits yet. Press + to add one.",
            ],
        );
    }

    #[test]
    fn a_row_shows_the_name_and_today_with_the_three_days_before() {
        let html = render(Ui::default(), vec![run()]);
        assert_offers(
            &html,
            &[
                ">Run<",
                "WED",
                "TUE",
                "MON",
                "SUN",
                "Run on Wed Nov 4",
                "Run on Sun Nov 1",
            ],
        );
        assert!(!html.contains("SAT"), "{html}");
    }

    #[test]
    fn the_menus_offer_every_sort_the_filters_sections_and_export() {
        let ui = Ui {
            menu: Some(Menu::Filter),
            ..Ui::default()
        };
        let html = render(ui, vec![run()]);
        assert_offers(
            &html,
            &[
                "Hide archived",
                "Hide completed",
                "Manually ✓",
                "By name",
                "By color",
                "By score",
                "By status",
            ],
        );
        let ui = Ui {
            menu: Some(Menu::List),
            ..Ui::default()
        };
        assert_offers(&render(ui, vec![run()]), &["Add section", "Export as CSV"]);
    }

    #[test]
    fn a_habit_takes_the_screen_with_every_card_of_loops_page_and_a_way_back() {
        let ui = Ui {
            page: Page::Habit("run".to_owned()),
            ..Ui::default()
        };
        let html = render(ui, vec![run()]);
        assert_offers(
            &html,
            &[
                "aria-label=\"Back\"",
                "aria-label=\"Edit\"",
                "Did you run today?",
                "Every day",
                ">Overview<",
                ">Score<",
                ">History<",
                ">Calendar<",
                ">Best streaks<",
                ">Frequency<",
                ">Calendar events<",
                "Add event",
                "Show on calendar",
                "Connect Google Calendar in Settings to add events.",
            ],
        );
        assert!(
            !html.contains(">Value<"),
            "only a habit with a value has the chart"
        );
    }

    #[test]
    fn a_habit_to_avoid_has_no_events_and_a_value_has_its_chart() {
        let mut habit = run();
        habit.kind = Kind::No;
        habit.measure = Some(model::Measure {
            unit: "cups".to_owned(),
            target: Some(2.0),
            days: 1,
        });
        habit.set_value(today(), Some(3.0));
        let ui = Ui {
            page: Page::Habit("run".to_owned()),
            ..Ui::default()
        };
        let html = render(ui, vec![habit]);
        assert_offers(&html, &[">Value<", "Target 2"]);
        assert!(!html.contains(">Calendar events<"), "{html}");
    }

    #[test]
    fn the_form_offers_loops_frequencies_and_the_new_ones() {
        let ui = Ui {
            page: Page::Form(Box::default()),
            ..Ui::default()
        };
        let html = render(ui, Vec::new());
        assert_offers(
            &html,
            &[
                "Create habit",
                ">Save<",
                "Type",
                "Name",
                "Question",
                "Color",
                "Symbols",
                "Faces",
                "Every day",
                "times per day",
                "times per week",
                "times per month",
                "times in",
                "On these days",
                "Track a value",
                "Enable skip days",
                "Show question mark for missing data",
                "Notes",
            ],
        );
        assert!(!html.contains("Section"), "no sections to choose from yet");
    }

    #[test]
    fn editing_keeps_the_kind_and_lists_sections_once_there_are_any() {
        let ui = Ui {
            page: Page::Form(Box::new(Form::from_habit(&run()))),
            ..Ui::default()
        };
        let mut habits = vec![run()];
        habits[0].section = Some("s".to_owned());
        let mut dom = VirtualDom::new_with_props(Harness, HarnessProps { ui, habits });
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        assert_offers(&html, &["Edit habit", "disabled"]);
    }

    #[test]
    fn a_day_offers_everything_its_habit_can_record() {
        let mut habit = run();
        habit.skip_days = true;
        habit.question_marks = true;
        habit.measure = Some(model::Measure {
            unit: "miles".to_owned(),
            target: None,
            days: 1,
        });
        let dialog = DayDialog::new(&habit, today(), TimeFormat::TwentyFourHour);
        let ui = Ui {
            day: Some(dialog),
            ..Ui::default()
        };
        let html = render(ui, vec![habit]);
        assert_offers(
            &html,
            &[
                "Run · Wed Nov 4",
                "Did you run today?",
                "Did it",
                "Didn&#39;t",
                "Skipped",
                "Not sure",
                "+ Add entry",
                "Comment",
                "Cancel",
                ">Save<",
            ],
        );
    }

    #[test]
    fn deleting_asks_first_and_offers_archiving_instead() {
        let ui = Ui {
            confirm_delete: Some("run".to_owned()),
            ..Ui::default()
        };
        let html = render(ui, vec![run()]);
        assert_offers(
            &html,
            &[
                "Delete this habit?",
                "Archive it instead to keep the record.",
                ">Delete<",
            ],
        );
    }

    fn tempfile_dir() -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "timeslay-habits-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }
}
