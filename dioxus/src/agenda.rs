//! Compact agenda lists for one, two or three days, or a month.

use chrono::{Duration, Months, NaiveDate};
use dioxus::prelude::*;
use timeslay_core::calendar::search::Search;
use timeslay_core::calendar::{self, Event};
use timeslay_core::config::CalendarSettings;

use crate::css_color;
use crate::day;
use crate::editor::worker;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Range {
    Days(u8),
    Month,
}

/// The days of the range holding `anchor`, in order.
pub fn dates(range: Range, anchor: NaiveDate, _settings: &CalendarSettings) -> Vec<NaiveDate> {
    match range {
        Range::Days(count) => (0..count)
            .map(|offset| anchor + Duration::days(i64::from(offset)))
            .collect(),
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
        Range::Days(count) => days(i64::from(count)),
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

/// The days of a range with what is on each, as a list. Tapping a day opens
/// it; tapping an item opens its sheet.
#[component]
pub fn AgendaDays(
    settings: CalendarSettings,
    range: Range,
    anchor: NaiveDate,
    today: NaiveDate,
    /// Bumped by every write, so a change shows here when its editor closes.
    revision: Signal<u32>,
    /// A habit's events alone, as the desktop's "Show on calendar" does.
    filter: Option<crate::habits::CalendarFilter>,
    /// The View sheet's filter, which narrows every view.
    #[props(default)]
    search: Search,
    on_day: EventHandler<NaiveDate>,
    on_open: EventHandler<Event>,
) -> Element {
    let load_settings = settings.clone();
    let loaded = use_resource(use_reactive!(|(range, anchor)| {
        let _ = revision();
        let settings = load_settings.clone();
        let dates = dates(range, anchor, &settings);
        async move {
            let wanted = dates.clone();
            let snapshot = worker(move || crate::day::load_agenda(&wanted, &settings)).await;
            snapshot.map(|snapshot| (snapshot, dates))
        }
    }));
    match &*loaded.read() {
        None => rsx! { p { class: "hint", role: "status", "Loading…" } },
        Some(Err(message)) => rsx! { p { class: "editor-error", role: "alert", "{message}" } },
        Some(Ok((snapshot, dates))) => {
            let mut days = day::agenda(snapshot, dates, &settings);
            for (_, rows) in &mut days {
                rows.retain(|row| {
                    filter
                        .as_ref()
                        .is_none_or(|filter| shows(filter, &row.event))
                        && search.shows(&row.event, &snapshot.events, &settings)
                });
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
                                        "data-sending": row.event.details.sending,
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
    }
}

/// A habit's events on the calendar, from its page's "Show on calendar".
#[component]
pub fn AgendaSheet(
    settings: CalendarSettings,
    day: NaiveDate,
    today: NaiveDate,
    revision: Signal<u32>,
    filter: Option<crate::habits::CalendarFilter>,
    on_day: EventHandler<NaiveDate>,
    on_open: EventHandler<Event>,
    on_close: EventHandler<()>,
) -> Element {
    let starting = if filter.is_some() {
        Range::Month
    } else {
        Range::Days(1)
    };
    let mut range = use_signal(move || starting);
    let mut anchor = use_signal(move || day);
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
                    {tab("Day", Range::Days(1))}
                    {tab("2 days", Range::Days(2))}
                    {tab("3 days", Range::Days(3))}
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
                div { class: "modal-body editor-body",
                    AgendaDays {
                        settings,
                        range: range(),
                        anchor: anchor(),
                        today,
                        revision,
                        filter,
                        on_day,
                        on_open,
                    }
                }
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

    fn on(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn days_start_at_selection_and_cross_year_boundaries() {
        let settings = CalendarSettings::default();
        for count in 1..=3 {
            let days = dates(Range::Days(count), on(2026, 12, 31), &settings);
            assert_eq!(days.len(), count as usize);
            assert_eq!(days[0], on(2026, 12, 31));
            assert_eq!(
                *days.last().unwrap(),
                on(2026, 12, 31) + Duration::days(i64::from(count - 1))
            );
        }
    }

    #[test]
    fn a_month_is_its_own_days_and_steps_keep_their_length() {
        let settings = CalendarSettings::default();
        let february = dates(Range::Month, on(2026, 2, 14), &settings);
        assert_eq!(february.len(), 28);
        assert_eq!(february[0], on(2026, 2, 1));
        assert_eq!(dates(Range::Days(3), on(2026, 2, 14), &settings).len(), 3);
        assert_eq!(
            step(Range::Days(1), on(2026, 2, 14), false),
            on(2026, 2, 13)
        );
        assert_eq!(step(Range::Month, on(2026, 1, 31), true), on(2026, 2, 28));
    }
}
