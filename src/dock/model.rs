//! What the dock draws: the leftmost column of the day view, as bare spans.
//!
//! The day view packs overlapping events into columns, with the sources the
//! user ranked highest on the left. The dock keeps only that first column, so
//! at any moment it shows the most important thing going on.

use chrono::{DateTime, Local, NaiveDate};
use iced::Color;

use crate::calendar::layout::{self, Span};
use crate::calendar::{self, Event, Store};
use crate::config::CalendarSettings;

/// Minutes in a day, the height of the strip in minutes.
pub const DAY_MINUTES: i32 = 24 * 60;

/// How many characters of a description the hover card keeps.
const DESCRIPTION_CHARS: usize = 360;

/// One coloured stretch of the strip.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start_min: i32,
    pub end_min: i32,
    pub color: Color,
    /// A ticked-off task is drawn faded rather than hidden.
    pub completed: bool,
    pub card: Card,
}

/// What the hover card says about a segment.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub title: String,
    pub when: String,
    pub lines: Vec<String>,
}

/// The first-column spans of `day`, in start order.
pub fn segments(store: &Store, settings: &CalendarSettings, day: NaiveDate) -> Vec<Segment> {
    let offset = settings.day_end_minutes();
    let timed = store.timed_events_on(day, settings);

    let spans: Vec<Span> = timed
        .iter()
        .map(|event| {
            let start = minutes_into_day(event.start, day, offset);
            let end = minutes_into_day(event.end, day, offset);
            let (start_min, end_min) = layout::clamp_day(start, end).unwrap_or_else(|| {
                let start = start.clamp(0, DAY_MINUTES);
                (start, (start + 15).min(DAY_MINUTES))
            });
            Span { start_min, end_min }
        })
        .collect();

    let sources = store.display_sources(settings);
    let priorities: Vec<usize> = timed
        .iter()
        .map(|event| {
            sources
                .iter()
                .position(|source| source.id == event.source_id())
                .unwrap_or(usize::MAX)
        })
        .collect();
    let packed = layout::pack_prioritized(&spans, &priorities);

    timed
        .iter()
        .zip(&spans)
        .zip(&packed)
        .filter(|(_, slot)| slot.column == 0)
        .map(|((event, span), _)| Segment {
            start_min: span.start_min,
            end_min: span.end_min,
            color: event.color,
            completed: event.is_task() && event.completed,
            card: card(event),
        })
        .collect()
}

/// Where "right now" sits in the day, in minutes, or `None` outside the day.
pub fn now_minute(
    now: DateTime<Local>,
    day: NaiveDate,
    settings: &CalendarSettings,
) -> Option<f32> {
    let minutes = minutes_into_day(now, day, settings.day_end_minutes());
    (0..DAY_MINUTES).contains(&minutes).then(|| {
        use chrono::Timelike;
        minutes as f32 + now.second() as f32 / 60.0
    })
}

fn minutes_into_day(when: DateTime<Local>, day: NaiveDate, offset: i32) -> i32 {
    let days = (when.date_naive() - day).num_days() as i32;
    days * DAY_MINUTES + calendar::minutes_from_midnight(when) - offset
}

fn card(event: &Event) -> Card {
    let title = if event.is_task() {
        crate::timeblocking::task_label(event)
    } else {
        event.title.clone()
    };

    let details = &event.details;
    let mut lines = Vec::new();
    if event.is_task() {
        lines.push("Task".to_owned());
    }
    if !details.location.trim().is_empty() {
        lines.push(format!("📍 {}", details.location.trim()));
    }
    if !details.meeting.trim().is_empty() {
        lines.push(details.meeting.trim().to_owned());
    }
    if !details.organizer.trim().is_empty() {
        lines.push(format!("Organizer: {}", details.organizer.trim()));
    }
    if !details.attendees.is_empty() {
        lines.push(format!("{} attendees", details.attendees.len()));
    }
    let description = details.description.trim();
    if !description.is_empty() {
        lines.push(truncate(description, DESCRIPTION_CHARS));
    }

    Card {
        title,
        when: calendar::event_time_label(event),
        lines,
    }
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(limit).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::calendar::{Calendar, Details, Snapshot};

    fn at(day: NaiveDate, hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .from_local_datetime(&day.and_hms_opt(hour, minute, 0).unwrap())
            .unwrap()
    }

    fn event(id: &str, calendar_id: &str, start: DateTime<Local>, minutes: i64) -> Event {
        Event {
            id: id.into(),
            calendar_id: calendar_id.into(),
            title: id.into(),
            start,
            end: start + Duration::minutes(minutes),
            all_day: false,
            color: Color::WHITE,
            completed: false,
            details: Details::default(),
        }
    }

    fn calendar(id: &str) -> Calendar {
        Calendar {
            id: id.into(),
            name: id.into(),
            color: Color::WHITE,
            primary: false,
            selected_by_google: true,
        }
    }

    fn store(events: Vec<Event>, day: NaiveDate) -> Store {
        Store::with_snapshot(Snapshot {
            calendars: vec![calendar("work"), calendar("home")],
            events,
            day,
            fetched_at: at(day, 8, 0),
            notice: None,
        })
    }

    #[test]
    fn only_the_highest_priority_source_survives_an_overlap() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        let settings = CalendarSettings {
            source_order: vec!["home".into(), "work".into()],
            ..CalendarSettings::default()
        };
        let store = store(
            vec![
                event("standup", "work", at(day, 9, 0), 60),
                event("dentist", "home", at(day, 9, 30), 60),
                event("lunch", "work", at(day, 12, 0), 30),
            ],
            day,
        );

        let titles: Vec<_> = segments(&store, &settings, day)
            .into_iter()
            .map(|segment| segment.card.title)
            .collect();

        assert_eq!(titles, ["dentist", "lunch"]);
    }

    #[test]
    fn segments_carry_their_minutes_into_the_day() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        let settings = CalendarSettings::default();
        let store = store(vec![event("standup", "work", at(day, 9, 15), 45)], day);

        let found = segments(&store, &settings, day);

        assert_eq!(found.len(), 1);
        assert_eq!((found[0].start_min, found[0].end_min), (555, 600));
    }

    #[test]
    fn now_is_only_marked_inside_the_day() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        let settings = CalendarSettings::default();

        assert_eq!(now_minute(at(day, 10, 0), day, &settings), Some(600.0));
        assert_eq!(
            now_minute(at(day, 10, 0), day.succ_opt().unwrap(), &settings),
            None
        );
    }

    #[test]
    fn long_descriptions_are_cut_for_the_card() {
        let mut long = event(
            "long",
            "work",
            at(NaiveDate::from_ymd_opt(2026, 10, 27).unwrap(), 9, 0),
            30,
        );
        long.details.description = "x".repeat(1000);

        let card = card(&long);

        assert_eq!(
            card.lines.last().unwrap().chars().count(),
            DESCRIPTION_CHARS + 1
        );
    }
}
