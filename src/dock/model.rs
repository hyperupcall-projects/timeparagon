//! What the dock draws: every timed span of the day, stacked by importance.
//!
//! The day view packs overlapping events into columns, with the sources the
//! user ranked highest on the left. The dock keeps them all: each span is
//! drawn full width, and the most important source is painted last so it sits
//! on top. Spans behind it still show wherever the top span does not cover.

use crate::calendar::time_format::TimeFormat;
use chrono::{DateTime, Local, NaiveDate};
use iced::Color;

use crate::calendar::layout::{self, Span};
use crate::calendar::{self, Event, Store};
use crate::config::CalendarSettings;
use crate::timeblocking;

/// Minutes in a day, the height of the strip in minutes.
pub const DAY_MINUTES: i32 = 24 * 60;

/// How many characters of a description the hover card keeps.
const DESCRIPTION_CHARS: usize = 360;

/// One coloured stretch of the strip.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// The event this span belongs to, so a promotion survives a refresh.
    pub id: String,
    pub start_min: i32,
    pub end_min: i32,
    pub color: Color,
    /// A ticked-off task is drawn faded rather than hidden.
    pub completed: bool,
    /// Rank of the source it came from; 0 is the highest priority.
    pub priority: usize,
    /// The short card shown while hovering.
    pub card: Card,
    /// Everything about the segment, shown once it is clicked.
    pub full: Card,
}

/// What the hover card says about a segment.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub title: String,
    pub when: String,
    pub lines: Vec<String>,
}

/// How far to the front a segment is painted. Larger is painted later, so it
/// covers the ones behind it. A promoted event outranks every source.
fn frontness(segment: &Segment, promoted: Option<&str>) -> i64 {
    if promoted == Some(segment.id.as_str()) {
        i64::MAX
    } else {
        -(segment.priority.min(1_000_000) as i64)
    }
}

/// Every timed span of `day`, ordered back to front: the last element is the
/// highest-priority source and is drawn on top of the rest.
pub fn segments(
    store: &Store,
    settings: &CalendarSettings,
    day: NaiveDate,
    time_format: TimeFormat,
    promoted: Option<&str>,
) -> Vec<Segment> {
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

    let all = store.events_on(day);
    let sources = store.display_sources(settings);

    let mut found: Vec<Segment> = timed
        .iter()
        .zip(&spans)
        .map(|(event, span)| {
            let assigned = assigned_tasks(all, event, settings);
            let priority = sources
                .iter()
                .position(|source| source.id == event.source_id())
                .unwrap_or(usize::MAX);
            Segment {
                id: event.id.clone(),
                start_min: span.start_min,
                end_min: span.end_min,
                color: event.display_color(settings),
                completed: event.is_task() && event.completed,
                priority,
                card: card(event, &assigned, false, time_format),
                full: card(event, &assigned, true, time_format),
            }
        })
        .collect();

    found.sort_by(|a, b| {
        frontness(a, promoted)
            .cmp(&frontness(b, promoted))
            .then(a.start_min.cmp(&b.start_min))
            .then_with(|| a.id.cmp(&b.id))
    });
    found
}

/// The tasks assigned to `event`, when it is a time block, in the order they
/// were arranged in.
fn assigned_tasks<'a>(
    all: &'a [Event],
    event: &Event,
    settings: &CalendarSettings,
) -> Vec<&'a Event> {
    if !timeblocking::is_block(event, settings) {
        return Vec::new();
    }
    timeblocking::tasks_in(event, all, settings.day_start_hour())
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

/// `full` keeps the whole description and lists a time block's tasks; the
/// hover card only says how many there are.
fn card(event: &Event, assigned: &[&Event], full: bool, time_format: TimeFormat) -> Card {
    let title = if event.details.sleep.is_some() {
        format!(
            "{} · {}",
            calendar::sleep::title(event),
            calendar::sleep::time_label(event, time_format)
        )
    } else if event.is_task() {
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
        lines.push(if full {
            format!("Attendees: {}", details.attendees.join(", "))
        } else {
            format!("{} attendees", details.attendees.len())
        });
    }
    let description = details.description.trim();
    if !description.is_empty() {
        lines.push(if full {
            description.to_owned()
        } else {
            truncate(description, DESCRIPTION_CHARS)
        });
    }

    if full && !assigned.is_empty() {
        lines.push(format!("Assigned tasks ({})", assigned.len()));
        lines.extend(assigned.iter().map(|task| task_line(task)));
    } else if !assigned.is_empty() {
        lines.push(format!("{} assigned tasks", assigned.len()));
    }
    if !full {
        lines.push("Click for everything".to_owned());
    }

    Card {
        title,
        when: calendar::time_format::event(event, time_format),
        lines,
    }
}

/// One task in a block's list: a checkbox, its name, and its length.
fn task_line(task: &Event) -> String {
    let mark = if task.completed { "☑" } else { "☐" };
    let minutes = timeblocking::metadata(task).minutes;
    let label = &task.title;
    if minutes > 0 {
        format!("{mark} {label} ({minutes}m)")
    } else {
        format!("{mark} {label}")
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
    fn every_overlapping_segment_is_kept_with_the_highest_priority_on_top() {
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

        let found = segments(&store, &settings, day, TimeFormat::TwentyFourHour, None);
        let titles: Vec<_> = found
            .iter()
            .map(|segment| segment.card.title.as_str())
            .collect();

        // Back to front, so `dentist` (home ranks first) is painted last.
        assert_eq!(titles, ["standup", "lunch", "dentist"]);
        assert!(found[0].priority > found[2].priority);
    }

    #[test]
    fn a_promoted_segment_is_painted_last_whatever_its_source() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        let settings = CalendarSettings {
            source_order: vec!["home".into(), "work".into()],
            ..CalendarSettings::default()
        };
        let store = store(
            vec![
                event("standup", "work", at(day, 9, 0), 60),
                event("dentist", "home", at(day, 9, 30), 60),
            ],
            day,
        );

        let found = segments(
            &store,
            &settings,
            day,
            TimeFormat::TwentyFourHour,
            Some("standup"),
        );

        assert_eq!(found.last().map(|s| s.id.as_str()), Some("standup"));
    }

    #[test]
    fn segments_carry_their_minutes_into_the_day() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        let settings = CalendarSettings::default();
        let store = store(vec![event("standup", "work", at(day, 9, 15), 45)], day);

        let found = segments(&store, &settings, day, TimeFormat::TwentyFourHour, None);

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

        let card = card(&long, &[], false, TimeFormat::TwentyFourHour);

        let description = &card.lines[card.lines.len() - 2];
        assert_eq!(description.chars().count(), DESCRIPTION_CHARS + 1);
        assert_eq!(
            card.lines.last().map(String::as_str),
            Some("Click for everything")
        );

        let full = self::card(&long, &[], true, TimeFormat::TwentyFourHour);
        assert_eq!(full.lines.last().unwrap().chars().count(), 1000);
    }

    #[test]
    fn a_time_block_lists_every_task_assigned_to_it_when_clicked() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 27).unwrap();
        let settings = CalendarSettings {
            app_calendar: Some("blocks".into()),
            ..CalendarSettings::default()
        };
        let task = |id: &str, block: &str, order: i64| {
            let mut task = event(id, "tasks:list", at(day, 0, 0), 0);
            task.id = crate::calendar::ids::event_id(id);
            task.all_day = true;
            task.details.task_data = Some(crate::task_db::TaskData {
                id: 100,
                metadata: crate::timeblocking::Metadata {
                    block: Some(block.to_owned()),
                    minutes: 30,
                    order,
                    ..Default::default()
                },
            });
            task
        };
        let store = store(
            vec![
                event("b1", "blocks", at(day, 9, 0), 60),
                task("second", "blocks/b1", 2),
                task("first", "blocks/b1", 1),
                task("elsewhere", "blocks/other", 1),
            ],
            day,
        );

        let found = segments(&store, &settings, day, TimeFormat::TwentyFourHour, None);
        let block = found.iter().find(|s| s.card.title == "b1").unwrap();

        let tasks: Vec<_> = block
            .full
            .lines
            .iter()
            .skip_while(|line| !line.starts_with("Assigned tasks"))
            .cloned()
            .collect();
        assert_eq!(
            tasks,
            ["Assigned tasks (2)", "☐ first (30m)", "☐ second (30m)"]
        );
        assert!(block.card.lines.contains(&"2 assigned tasks".to_owned()));
        assert!(!block.card.lines.iter().any(|line| line.contains("first")));
    }
}
