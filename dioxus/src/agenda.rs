//! Week, two-week and month views, as a list: the grids the desktop draws need
//! more width than a phone has.

use chrono::{Duration, Months, NaiveDate};
use dioxus::prelude::*;
use timeslay_core::calendar::{self, Event};
use timeslay_core::config::CalendarSettings;

use crate::css_color;
use crate::day;
use crate::editor::worker;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Range {
    Week,
    TwoWeeks,
    Month,
}

/// The days of the range holding `anchor`, in order.
pub fn dates(range: Range, anchor: NaiveDate, settings: &CalendarSettings) -> Vec<NaiveDate> {
    let first_of_week = |day: NaiveDate| {
        let back = calendar::month_grid(calendar::first_of_month(day), settings.first_day_of_week)
            .into_iter()
            .position(|cell| cell == calendar::first_of_month(day))
            .unwrap_or(0) as i64;
        let from_first = (day - calendar::first_of_month(day)).num_days();
        day - Duration::days((from_first + back) % 7)
    };
    match range {
        Range::Week | Range::TwoWeeks => {
            let count = if range == Range::Week { 7 } else { 14 };
            let start = first_of_week(anchor);
            (0..count)
                .map(|offset| start + Duration::days(offset))
                .collect()
        }
        Range::Month => {
            let first = calendar::first_of_month(anchor);
            let next = first.checked_add_months(Months::new(1)).unwrap_or(first);
            first.iter_days().take_while(|day| *day < next).collect()
        }
    }
}

/// The anchor of the next or previous range.
pub fn step(range: Range, anchor: NaiveDate, forward: bool) -> NaiveDate {
    let days = |count: i64| anchor + Duration::days(if forward { count } else { -count });
    match range {
        Range::Week => days(7),
        Range::TwoWeeks => days(14),
        Range::Month => {
            let months = Months::new(1);
            let moved = if forward {
                anchor.checked_add_months(months)
            } else {
                anchor.checked_sub_months(months)
            };
            moved.unwrap_or(anchor)
        }
    }
}

#[component]
pub fn AgendaSheet(
    settings: CalendarSettings,
    day: NaiveDate,
    today: NaiveDate,
    /// Bumped by every write, so a change shows here when its editor closes.
    revision: Signal<u32>,
    /// A habit's events alone, which open on the month, as the desktop's
    /// "Show on calendar" does.
    filter: Option<crate::habits::CalendarFilter>,
    on_day: EventHandler<NaiveDate>,
    on_open: EventHandler<Event>,
    on_close: EventHandler<()>,
) -> Element {
    let starting = if filter.is_some() {
        Range::Month
    } else {
        Range::Week
    };
    let mut range = use_signal(move || starting);
    let mut anchor = use_signal(move || day);
    let load_settings = settings.clone();
    let loaded = use_resource(move || {
        let _ = revision();
        let settings = load_settings.clone();
        let dates = dates(range(), anchor(), &settings);
        async move {
            let wanted = dates.clone();
            let snapshot = worker(move || crate::day::load_agenda(&wanted, &settings)).await;
            snapshot.map(|snapshot| (snapshot, dates))
        }
    });
    let tab = |label: &'static str, this: Range| {
        rsx! {
            button {
                class: "nav-tab",
                aria_pressed: (range() == this).to_string(),
                onclick: move |_| range.set(this),
                "{label}"
            }
        }
    };
    let body = match &*loaded.read() {
        None => rsx! { p { class: "hint", role: "status", "Loading…" } },
        Some(Err(message)) => rsx! { p { class: "editor-error", role: "alert", "{message}" } },
        Some(Ok((snapshot, dates))) => {
            let mut days = day::agenda(snapshot, dates, &settings);
            if let Some(filter) = &filter {
                for (_, rows) in &mut days {
                    rows.retain(|row| shows(filter, &row.event));
                }
            }
            rsx! {
                for (date, rows) in days {
                    section { key: "{date}", class: "agenda-day",
                        button {
                            class: if date == today { "agenda-date today" } else { "agenda-date" },
                            onclick: move |_| on_day.call(date),
                            "{date.format(\"%a, %b %-d\")}"
                        }
                        if rows.is_empty() {
                            p { class: "agenda-empty", "Nothing" }
                        }
                        for row in rows {
                            {
                                let dot = css_color(row.color);
                                let done = row.event.completed;
                                let event = row.event.clone();
                                rsx! {
                                    button {
                                        key: "{row.event.id}",
                                        class: "editor-item editor-pick",
                                        onclick: move |_| on_open.call(event.clone()),
                                        span { class: "day-dot", style: "background: {dot};" }
                                        span { class: if done { "editor-item-name task-title done" } else { "editor-item-name" },
                                            "{row.event.title}"
                                            span { class: "hint editor-item-sub", "{row.time}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    };
    rsx! {
        div { class: "modal-backdrop editor-backdrop",
            div { class: "modal editor", role: "dialog", aria_label: "Agenda",
                div { class: "modal-head",
                    div { class: "modal-titles",
                        h2 { class: "modal-title", "Agenda" }
                        if let Some(filter) = &filter {
                            div { class: "modal-sub", "Events of {filter.name}" }
                        }
                    }
                    button {
                        class: "modal-close",
                        title: "Close",
                        onclick: move |_| on_close.call(()),
                        "×"
                    }
                }
                div { class: "sheet-tabs",
                    {tab("Week", Range::Week)}
                    {tab("2 weeks", Range::TwoWeeks)}
                    {tab("Month", Range::Month)}
                    div { class: "spacer" }
                    button {
                        class: "btn icon",
                        title: "Earlier",
                        aria_label: "Earlier",
                        onclick: move |_| anchor.set(step(range(), anchor(), false)),
                        "‹"
                    }
                    button {
                        class: "btn icon",
                        title: "Later",
                        aria_label: "Later",
                        onclick: move |_| anchor.set(step(range(), anchor(), true)),
                        "›"
                    }
                }
                div { class: "modal-body editor-body", {body} }
            }
        }
    }
}

/// Whether an occurrence belongs to one of the filter's series.
fn shows(filter: &crate::habits::CalendarFilter, event: &Event) -> bool {
    let series = event
        .details
        .recurring_event_id
        .as_deref()
        .unwrap_or(&event.id);
    filter.series.iter().any(|id| id == series)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;
    use timeslay_core::calendar::first_day::FirstDayOfWeek;

    fn on(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn a_week_starts_on_the_configured_day_whichever_day_it_holds() {
        for first_day in FirstDayOfWeek::ALL {
            let settings = CalendarSettings {
                first_day_of_week: first_day,
                ..CalendarSettings::default()
            };
            let wanted = calendar::month_grid(on(2026, 11, 1), first_day)[0].weekday();
            for offset in 0..40 {
                let anchor = on(2026, 10, 20) + Duration::days(offset);
                let week = dates(Range::Week, anchor, &settings);
                assert_eq!(week.len(), 7);
                assert_eq!(week[0].weekday(), wanted, "{first_day} {anchor}");
                assert!(week.contains(&anchor), "{first_day} {anchor}");
            }
        }
    }

    #[test]
    fn a_month_is_its_own_days_and_steps_keep_their_length() {
        let settings = CalendarSettings::default();
        let february = dates(Range::Month, on(2026, 2, 14), &settings);
        assert_eq!(february.len(), 28);
        assert_eq!(february[0], on(2026, 2, 1));
        assert_eq!(dates(Range::TwoWeeks, on(2026, 2, 14), &settings).len(), 14);
        assert_eq!(step(Range::Week, on(2026, 2, 14), false), on(2026, 2, 7));
        assert_eq!(step(Range::Month, on(2026, 1, 31), true), on(2026, 2, 28));
    }
}
