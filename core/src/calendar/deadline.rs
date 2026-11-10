//! Typed task deadlines stored in task metadata. Descriptions are user text.
use std::fmt;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

const DATE_ERROR: &str = "Enter a deadline as YYYY-MM-DD";
const DATE_TIME_ERROR: &str = "Enter a deadline as YYYY-MM-DD HH:MM";
const INPUT_ERROR: &str = "Enter a deadline as YYYY-MM-DD or YYYY-MM-DD HH:MM";
const TIME_ERROR: &str = "Enter a time as HH:MM";

/// When a task must be finished. Without a time the whole day counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

impl fmt::Display for Deadline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.value())
    }
}

/// Read the typed deadline from task metadata, never from the description.
pub fn get(event: &super::Event) -> Option<Deadline> {
    crate::timeblocking::metadata(event).deadline
}

pub fn input_text(event: &super::Event) -> String {
    get(event)
        .map(|deadline| deadline.value())
        .unwrap_or_default()
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
    fn typed_input_accepts_a_day_or_a_day_and_time() {
        assert_eq!(at("2026-10-30").time, None);
        assert_eq!(at(" 2026-10-30 17:00 ").value(), "2026-10-30 17:00");
        assert_eq!(at("2026-10-30").to_string(), "2026-10-30");
        assert_eq!(at("2026-10-30 17:00").to_string(), "2026-10-30 17:00");
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

const MARKER_PREFIX: &str = "deadline:";

pub fn source_id(id: &str) -> &str {
    id.strip_prefix(MARKER_PREFIX).unwrap_or(id)
}
pub fn is_marker(event: &super::Event) -> bool {
    event.id.starts_with(MARKER_PREFIX)
}

/// A second, display-only entry for a task's deadline. A timed deadline is a
/// point represented by a short chip; a date-only deadline belongs above the grid.
pub fn marker(task: &super::Event) -> Option<super::Event> {
    use chrono::{Duration, Local, TimeZone};
    if !task.is_task() || is_marker(task) {
        return None;
    }
    let deadline = get(task)?;
    let start = Local
        .from_local_datetime(
            &deadline
                .date
                .and_time(deadline.time.unwrap_or(NaiveTime::MIN)),
        )
        .earliest()?;
    let mut marker = task.clone();
    marker.id = format!("{MARKER_PREFIX}{}", task.id);
    marker.title = format!("Deadline · {}", task.title);
    marker.start = start;
    marker.all_day = deadline.time.is_none();
    marker.end = if marker.all_day {
        super::at_clock(deadline.date.succ_opt()?, 0, 0)
    } else {
        start + Duration::minutes(15)
    };
    Some(marker)
}

/// The calendar cell holding the marker; date-only deadlines use calendar dates.
pub fn on_day(marker: &super::Event, day: NaiveDate, day_start_hour: u32) -> bool {
    if marker.all_day {
        marker.start.date_naive() == day
    } else {
        super::day_at(marker.start, day_start_hour) == day
    }
}

#[cfg(test)]
mod marker_tests {
    use super::*;
    fn task() -> super::super::Event {
        let mut task = super::super::sample_snapshot().events.remove(0);
        task.details.task_data = Some(crate::task_db::TaskData {
            id: 42,
            metadata: Default::default(),
        });
        task
    }
    #[test]
    fn deadlines_project_to_their_own_day_and_leave_the_schedule_intact() {
        let mut task = task();
        let schedule = (task.start, task.end);
        let day = task.start.date_naive().succ_opt().unwrap();
        task.details.task_data.as_mut().unwrap().metadata.deadline =
            Some(parse_input(&format!("{day} 17:30")).unwrap());
        let timed = marker(&task).unwrap();
        assert!(is_marker(&timed));
        assert_eq!(source_id(&timed.id), task.id);
        assert_eq!(timed.start.date_naive(), day);
        assert_eq!(timed.start.time(), parse_time("17:30").unwrap());
        assert!(!timed.all_day);
        assert!(on_day(&timed, day, 0));
        assert!(!on_day(&timed, task.start.date_naive(), 0));
        assert_eq!((task.start, task.end), schedule);
        assert!(marker(&timed).is_none());
        let mut draft = super::super::EditDraft::from_event(&task);
        assert_eq!(draft.deadline().unwrap(), get(&task));
        draft.deadline_text = day.to_string();
        assert_eq!(draft.deadline().unwrap().unwrap().time, None);
        draft.deadline_text.clear();
        assert_eq!(draft.deadline().unwrap(), None);
        draft.deadline_text = "2026-02-30".into();
        assert!(draft.deadline().is_err());
        task.details.task_data.as_mut().unwrap().metadata.deadline = Some(Deadline {
            date: day,
            time: None,
        });
        let dated = marker(&task).unwrap();
        assert!(dated.all_day);
        assert!(on_day(&dated, day, 4));
        assert!(!on_day(&dated, day.pred_opt().unwrap(), 4));
    }
}
