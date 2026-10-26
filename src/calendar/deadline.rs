//! Reserved lines at the top of a Google Task's notes.
//!
//! Google Tasks only stores a day for `due`, and has no deadline at all, so the
//! app keeps the rest in the notes. Any leading lines that start with one of
//! these keys are the header; the first other line starts the human prose:
//!
//! ```text
//! DEADLINE: 2026-10-30 17:00   the deadline, day and time
//! DUE: 2026-10-30              the deadline, day only (DEADLINE wins if both)
//! DUE_TIME: 09:30              clock time to show the task at on its due day
//! ```
use std::fmt;

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, Timelike};

use super::Event;

const DEADLINE: &str = "DEADLINE:";
const DUE: &str = "DUE:";
const DUE_TIME: &str = "DUE_TIME:";
const KEYS: [&str; 3] = [DEADLINE, DUE, DUE_TIME];

/// How long a task occupies the grid when its notes give a start time but the
/// task has no estimate.
const DEFAULT_MINUTES: u32 = 30;

const DATE_ERROR: &str = "Enter a deadline as YYYY-MM-DD";
const DATE_TIME_ERROR: &str = "Enter a deadline as YYYY-MM-DD HH:MM";
const INPUT_ERROR: &str = "Enter a deadline as YYYY-MM-DD or YYYY-MM-DD HH:MM";
const TIME_ERROR: &str = "Enter a time as HH:MM";

/// When a task must be finished. Without a time the whole day counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadline {
    pub date: NaiveDate,
    pub time: Option<NaiveTime>,
}

impl Deadline {
    /// The text a person types: `YYYY-MM-DD` or `YYYY-MM-DD HH:MM`.
    pub fn value(&self) -> String {
        match self.time {
            Some(time) => format!("{} {}", self.date, time.format("%H:%M")),
            None => self.date.to_string(),
        }
    }

    /// Whether the deadline has passed. A day-only deadline lapses once the
    /// day is over; one with a time lapses at that minute.
    pub fn is_past(&self, now: NaiveDateTime) -> bool {
        match self.time {
            Some(time) => self.date.and_time(time) < now,
            None => self.date < now.date(),
        }
    }
}

/// The notes line this deadline is stored as.
impl fmt::Display for Deadline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let key = if self.time.is_some() { DEADLINE } else { DUE };
        write!(f, "{key} {}", self.value())
    }
}

pub fn parse_date(value: &str) -> Result<NaiveDate, String> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| DATE_ERROR.to_owned())?;
    if date.to_string() != value {
        return Err(DATE_ERROR.into());
    }
    Ok(date)
}

pub fn parse_time(value: &str) -> Result<NaiveTime, String> {
    let time = NaiveTime::parse_from_str(value, "%H:%M").map_err(|_| TIME_ERROR.to_owned())?;
    if time.format("%H:%M").to_string() != value {
        return Err(TIME_ERROR.into());
    }
    Ok(time)
}

fn parse_date_time(value: &str) -> Result<Deadline, String> {
    let (date, time) = value
        .split_once(' ')
        .ok_or_else(|| DATE_TIME_ERROR.to_owned())?;
    match (parse_date(date.trim()), parse_time(time.trim())) {
        (Ok(date), Ok(time)) => Ok(Deadline {
            date,
            time: Some(time),
        }),
        _ => Err(DATE_TIME_ERROR.into()),
    }
}

/// What the deadline field accepts: a day, or a day and a time.
pub fn parse_input(input: &str) -> Result<Deadline, String> {
    let input = input.trim();
    if input.contains(char::is_whitespace) {
        parse_date_time(input).map_err(|_| INPUT_ERROR.to_owned())
    } else {
        parse_date(input)
            .map(|date| Deadline { date, time: None })
            .map_err(|_| INPUT_ERROR.to_owned())
    }
}

/// Splits the notes into the leading reserved lines and everything after them,
/// which is returned untouched.
fn split(notes: &str) -> (Vec<&str>, &str) {
    let mut header = Vec::new();
    let mut rest = notes;
    loop {
        let (line, tail) = match rest.split_once('\n') {
            Some((line, tail)) => (line, Some(tail)),
            None => (rest, None),
        };
        if !KEYS.iter().any(|key| line.starts_with(key)) {
            break;
        }
        header.push(line);
        match tail {
            Some(tail) => rest = tail,
            None => {
                rest = "";
                break;
            }
        }
    }
    (header, rest)
}

fn value<'a>(header: &[&'a str], key: &str) -> Option<&'a str> {
    header
        .iter()
        .find_map(|line| line.strip_prefix(key))
        .map(str::trim)
}

/// The deadline in the notes. `DEADLINE:` takes precedence over `DUE:`.
pub fn read_deadline(notes: &str) -> Result<Option<Deadline>, String> {
    let (header, _) = split(notes);
    if let Some(text) = value(&header, DEADLINE) {
        return parse_date_time(text).map(Some);
    }
    match value(&header, DUE) {
        Some(text) => parse_date(text).map(|date| Some(Deadline { date, time: None })),
        None => Ok(None),
    }
}

/// The clock time the task should be shown at on its due day.
pub fn read_due_time(notes: &str) -> Result<Option<NaiveTime>, String> {
    let (header, _) = split(notes);
    value(&header, DUE_TIME).map(parse_time).transpose()
}

/// Whether every reserved line parses, so a typo can be reported.
pub fn check(notes: &str) -> Result<(), String> {
    read_deadline(notes)?;
    read_due_time(notes)?;
    Ok(())
}

/// The deadline as it would be typed into the field, or the raw text of a
/// malformed line so it can be corrected.
pub fn input_text(notes: &str) -> String {
    if let Ok(Some(deadline)) = read_deadline(notes) {
        return deadline.value();
    }
    let (header, _) = split(notes);
    value(&header, DEADLINE)
        .or_else(|| value(&header, DUE))
        .unwrap_or("")
        .to_owned()
}

/// Replace only the deadline line; other reserved lines and the prose stay
/// exactly as they were.
pub fn write(notes: &str, deadline: Option<Deadline>) -> String {
    let (header, body) = split(notes);
    let mut lines: Vec<String> = deadline.iter().map(Deadline::to_string).collect();
    lines.extend(
        header
            .iter()
            .filter(|line| !line.starts_with(DEADLINE) && !line.starts_with(DUE))
            .map(|line| (*line).to_owned()),
    );
    if lines.is_empty() {
        return body.to_owned();
    }
    let head = lines.join("\n");
    if body.is_empty() {
        head
    } else {
        format!("{head}\n{body}")
    }
}

/// Moves a task that is only dated onto the grid at its `DUE_TIME:`. The block
/// lasts as long as the task's estimate. Tasks with a time of their own, such
/// as ones backed by a Calendar event, are left alone, as are undated tasks.
pub fn place(event: &mut Event, minutes: u32) {
    if !event.all_day {
        return;
    }
    let day = event.start.date_naive();
    if day == crate::timeblocking::UNDATED {
        return;
    }
    let Ok(Some(time)) = read_due_time(&event.details.description) else {
        return;
    };
    let minutes = if minutes == 0 { DEFAULT_MINUTES } else { minutes };
    event.start = super::at_clock(day, time.hour(), time.minute());
    event.end = event.start + Duration::minutes(i64::from(minutes));
    event.all_day = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        parse_date(text).unwrap()
    }

    fn at(text: &str) -> Deadline {
        parse_input(text).unwrap()
    }

    #[test]
    fn deadlines_round_trip_without_changing_prose() {
        let deadline = at("2026-10-30");
        let body = "\nPrepare report.\nDUE: this is ordinary prose\n";
        let notes = write(body, Some(deadline));
        assert_eq!(read_deadline(&notes).unwrap(), Some(deadline));
        assert_eq!(write(&notes, None), body);
        assert_eq!(write(&notes, Some(deadline)), notes);
        assert_eq!(read_deadline(body).unwrap(), None);
    }

    #[test]
    fn a_deadline_with_a_time_round_trips() {
        let deadline = at("2026-10-30 17:05");
        assert_eq!(deadline.date, date("2026-10-30"));
        assert_eq!(deadline.time, Some(parse_time("17:05").unwrap()));
        let notes = write("Prepare report.", Some(deadline));
        assert_eq!(notes, "DEADLINE: 2026-10-30 17:05\nPrepare report.");
        assert_eq!(read_deadline(&notes).unwrap(), Some(deadline));
        assert_eq!(write(&notes, None), "Prepare report.");
    }

    #[test]
    fn deadline_wins_over_due_and_replaces_it_on_write() {
        let notes = "DUE: 2026-10-01\nDEADLINE: 2026-10-30 17:00\nNotes";
        assert_eq!(read_deadline(notes).unwrap(), Some(at("2026-10-30 17:00")));
        assert_eq!(
            write(notes, Some(at("2026-11-02"))),
            "DUE: 2026-11-02\nNotes"
        );
    }

    #[test]
    fn due_time_is_read_and_survives_deadline_edits() {
        let notes = "DUE_TIME: 09:30\nDEADLINE: 2026-10-30 17:00\nNotes";
        assert_eq!(
            read_due_time(notes).unwrap(),
            Some(parse_time("09:30").unwrap())
        );
        let edited = write(notes, Some(at("2026-11-02")));
        assert_eq!(edited, "DUE: 2026-11-02\nDUE_TIME: 09:30\nNotes");
        assert_eq!(write(&edited, None), "DUE_TIME: 09:30\nNotes");
        assert_eq!(read_deadline("DUE_TIME: 09:30").unwrap(), None);
        assert_eq!(read_due_time("DUE: 2026-10-30").unwrap(), None);
    }

    #[test]
    fn reserved_lines_only_count_at_the_top() {
        let notes = "Prose first\nDEADLINE: 2026-10-30 17:00\nDUE_TIME: 09:30";
        assert_eq!(read_deadline(notes).unwrap(), None);
        assert_eq!(read_due_time(notes).unwrap(), None);
        assert!(check("DUE: tomorrow").is_err());
        assert!(check("Prose first\nDUE: tomorrow").is_ok());
    }

    #[test]
    fn malformed_deadlines_can_be_repaired_or_cleared() {
        for value in ["2026-02-30", "2026-1-2", "tomorrow", ""] {
            assert!(parse_date(value).is_err());
            let notes = format!("DUE: {value}\nKeep my notes");
            assert!(read_deadline(&notes).is_err());
            assert_eq!(input_text(&notes), value);
            assert_eq!(write(&notes, None), "Keep my notes");
        }
        for value in [
            "2026-10-30",
            "2026-10-30 5:00",
            "2026-10-30 24:00",
            "2026-10-30 17:00:00",
            "17:00",
        ] {
            let notes = format!("DEADLINE: {value}\nKeep my notes");
            assert!(read_deadline(&notes).is_err(), "{value}");
            assert_eq!(write(&notes, None), "Keep my notes");
        }
        for value in ["9:30", "24:00", "noon", ""] {
            let notes = format!("DUE_TIME: {value}\nKeep my notes");
            assert!(read_due_time(&notes).is_err(), "{value}");
            assert!(check(&notes).is_err());
        }
    }

    #[test]
    fn typed_input_accepts_a_day_or_a_day_and_time() {
        assert_eq!(at("2026-10-30").time, None);
        assert_eq!(at(" 2026-10-30 17:00 ").value(), "2026-10-30 17:00");
        assert_eq!(at("2026-10-30").to_string(), "DUE: 2026-10-30");
        assert_eq!(
            at("2026-10-30 17:00").to_string(),
            "DEADLINE: 2026-10-30 17:00"
        );
        for bad in ["2026-02-30", "2026-10-30 25:00", "soon", "2026-10-30 1700"] {
            assert!(parse_input(bad).unwrap_err().contains("YYYY-MM-DD"));
        }
    }

    #[test]
    fn a_timed_deadline_lapses_at_its_minute_and_a_dated_one_at_midnight() {
        let now = date("2026-10-30").and_hms_opt(17, 30, 0).unwrap();
        assert!(at("2026-10-30 17:00").is_past(now));
        assert!(!at("2026-10-30 18:00").is_past(now));
        assert!(!at("2026-10-30").is_past(now));
        assert!(at("2026-10-29").is_past(now));
    }

    fn all_day_task(notes: &str, day: &str) -> Event {
        let mut task = super::super::sample_snapshot().events.remove(0);
        task.id = "task:timed".into();
        task.calendar_id = "tasks:list".into();
        task.all_day = true;
        task.start = super::super::at_clock(date(day), 0, 0);
        task.end = super::super::at_clock(date(day).succ_opt().unwrap(), 0, 0);
        task.details.description = notes.into();
        task
    }

    #[test]
    fn a_due_time_puts_the_task_on_the_grid_for_its_estimate() {
        let mut task = all_day_task("DUE_TIME: 09:30\nNotes", "2026-10-30");
        place(&mut task, 45);
        assert!(!task.all_day);
        assert_eq!(task.start, super::super::at_clock(date("2026-10-30"), 9, 30));
        assert_eq!(task.end, task.start + Duration::minutes(45));

        let mut unestimated = all_day_task("DUE_TIME: 23:45", "2026-10-30");
        place(&mut unestimated, 0);
        assert_eq!(
            unestimated.end,
            unestimated.start + Duration::minutes(i64::from(DEFAULT_MINUTES))
        );
    }

    #[test]
    fn tasks_without_a_usable_due_time_stay_all_day() {
        for notes in ["Notes", "DUE_TIME: late", "DEADLINE: 2026-10-30 17:00"] {
            let mut task = all_day_task(notes, "2026-10-30");
            let before = task.clone();
            place(&mut task, 0);
            assert_eq!(task, before, "{notes}");
        }
        let mut undated = all_day_task("DUE_TIME: 09:30", "1970-01-01");
        let before = undated.clone();
        place(&mut undated, 0);
        assert_eq!(undated, before);
    }
}
