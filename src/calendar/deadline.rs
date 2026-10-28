//! Deadlines kept in a Google Task's notes.
//!
//! Google Tasks only stores a day for `due`, and has no deadline at all, so the
//! app keeps the rest in the notes. Any line that starts with one of these
//! keys is reserved; the others are human prose:
//!
//! ```text
//! DEADLINE: 2026-10-30 17:00   the deadline, day and time
//! DUE: 2026-10-30              the deadline, day only (DEADLINE wins if both)
//! ```
//!
//! A task's place on the grid comes from its Calendar-backed schedule, not
//! from the notes. Google Tasks itself has no clock time, so the app does not
//! try to infer one.
use std::fmt;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

use super::notes::{Notes, line_value};

const DEADLINE: &str = "DEADLINE:";
const DUE: &str = "DUE:";

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

/// The deadline in the notes. `DEADLINE:` takes precedence over `DUE:`, and
/// every reserved line is checked so a typo is reported wherever it appears.
pub fn read_deadline(notes: &str) -> Result<Option<Deadline>, String> {
    let mut timed = None;
    let mut dated = None;
    for line in Notes::new(notes).lines() {
        if let Some(text) = line_value(line, DEADLINE) {
            timed.get_or_insert(parse_date_time(text)?);
        } else if let Some(text) = line_value(line, DUE) {
            dated.get_or_insert(parse_date(text).map(|date| Deadline { date, time: None })?);
        }
    }
    Ok(timed.or(dated))
}

/// Whether every reserved line parses, so a typo can be reported.
pub fn check(notes: &str) -> Result<(), String> {
    read_deadline(notes)?;
    Ok(())
}

/// The deadline as it would be typed into the field, or the raw text of a
/// malformed line so it can be corrected.
pub fn input_text(notes: &str) -> String {
    if let Ok(Some(deadline)) = read_deadline(notes) {
        return deadline.value();
    }
    let notes = Notes::new(notes);
    notes
        .value(DEADLINE)
        .or_else(|| notes.value(DUE))
        .unwrap_or("")
        .to_owned()
}

/// Replace the deadline lines; every other line stays exactly as it was.
pub fn write(notes: &str, deadline: Option<Deadline>) -> String {
    let body = Notes::new(notes).without(&[DEADLINE, DUE]);
    match deadline {
        Some(deadline) if body.is_empty() => deadline.to_string(),
        Some(deadline) => format!("{deadline}\n{body}"),
        None => body,
    }
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
        let body = "\nPrepare report.\nDUE this is ordinary prose\n";
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
    fn a_former_due_time_line_is_ordinary_prose_now() {
        let notes = "DUE_TIME: 09:30\nDEADLINE: 2026-10-30 17:00\nNotes";
        let edited = write(notes, Some(at("2026-11-02")));
        assert_eq!(edited, "DUE: 2026-11-02\nDUE_TIME: 09:30\nNotes");
        assert_eq!(write(&edited, None), "DUE_TIME: 09:30\nNotes");
        assert_eq!(read_deadline("DUE_TIME: 09:30").unwrap(), None);
        assert!(check("DUE_TIME: 09:30").is_ok());
    }

    #[test]
    fn reserved_lines_are_found_on_any_line() {
        let notes = "Prose first\nDEADLINE: 2026-10-30 17:00\nMore prose\nDUE_TIME: 09:30";
        assert_eq!(read_deadline(notes).unwrap(), Some(at("2026-10-30 17:00")));
        assert!(check("DUE: tomorrow").is_err());
        assert!(check("Prose first\nDUE: tomorrow").is_err());
        let edited = write(notes, Some(at("2026-11-02")));
        assert_eq!(
            edited,
            "DUE: 2026-11-02\nProse first\nMore prose\nDUE_TIME: 09:30"
        );
        assert_eq!(
            write(&edited, None),
            "Prose first\nMore prose\nDUE_TIME: 09:30"
        );
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
}
