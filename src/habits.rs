//! Habits: recurring events on the app's single TimeSlay calendar.
//!
//! A habit is an ordinary Google Calendar event tagged with `ttKind=habit`,
//! so Google expands it and every other client sees a normal recurring event.
//! The Habits tab edits the master rather than an instance. Its editor uses a
//! structured repeat model (daily, weekly days, monthly day or weekday, and an
//! end date), so nobody has to type RFC 5545 parts by hand. The structured
//! model is turned into `RRULE`/`RDATE`/`EXDATE` lines only when it is written.

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, TimeZone, Timelike, Utc, Weekday};
use iced::{Color, Task};

use crate::app::{Message, State};
use crate::blocking;
use crate::calendar::{self, api::HabitEvent, colors, deadline};

/// The `ttKind` Google extended property that marks a habit event.
pub const KIND: &str = "habit";

/// The text input that holds a habit's name in the editor.
pub const TITLE_FIELD: &str = "habit-title";

/// Monday first, the order the weekday picker uses.
pub const WEEKDAYS: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

/// One habit, as the master event on the TimeSlay calendar.
#[derive(Debug, Clone, PartialEq)]
pub struct Habit {
    pub id: String,
    pub calendar_id: String,
    pub title: String,
    pub notes: String,
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub recurrence: Recurrence,
    pub color_id: Option<String>,
}

impl Habit {
    pub fn from_event(event: HabitEvent) -> Self {
        Self {
            id: event.id,
            calendar_id: event.calendar_id,
            title: event.title,
            notes: event.notes,
            start: event.start,
            end: event.end,
            recurrence: Recurrence::from_lines(&event.recurrence),
            color_id: event.color_id,
        }
    }

    /// The habit's Google color when it has one, otherwise the app's habit
    /// color.
    pub fn color(&self, settings: &crate::config::CalendarSettings) -> Color {
        self.color_id
            .as_deref()
            .and_then(colors::event)
            .unwrap_or_else(|| settings.habit_color())
    }

    /// A readable description of the recurrence, or "Does not repeat".
    pub fn recurrence_label(&self) -> String {
        if self.recurrence.is_empty() {
            return "Does not repeat".to_owned();
        }
        Draft::from_habit(self).repeat_summary()
    }

    /// The time of day the habit runs.
    pub fn time_label(&self) -> String {
        format!(
            "{} – {}",
            self.start.format("%H:%M"),
            self.end.format("%H:%M")
        )
    }
}

/// The recurrence array exactly as Google stores it. The lines are kept in
/// order and are never interpreted by the app beyond validating them before a
/// write, so a rule Google accepts can be round-tripped untouched.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recurrence {
    lines: Vec<String>,
}

impl Recurrence {
    pub fn from_lines(lines: &[String]) -> Self {
        let mut kept = Vec::new();
        for line in lines {
            if let Some(line) = normalize_line(line) {
                kept.push(line);
            }
        }
        Self { lines: kept }
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

fn strip_prefix<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    line.get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| line[prefix.len()..].trim())
}

fn starts_with_kind(line: &str, kind: &str) -> bool {
    line.get(..kind.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(kind))
}

fn normalize_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    for kind in ["RRULE", "RDATE", "EXDATE"] {
        if starts_with_kind(line, kind) {
            let rest = line[kind.len()..].trim();
            // `RDATE` and `EXDATE` can carry parameters (`;VALUE=DATE`,
            // `;TZID=...`) before the colon. Keep them exactly as written;
            // `RRULE` always uses a colon.
            if rest.starts_with(':') || rest.starts_with(';') {
                if rest.trim_start_matches(':').trim().is_empty() {
                    return None;
                }
                return Some(format!("{kind}{rest}"));
            }
        }
    }
    None
}

/// How a habit repeats, in the words the editor offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatMode {
    Never,
    Daily,
    Weekdays,
    Weekly,
    Monthly,
    Yearly,
    /// A rule imported from another app that the structured editor cannot
    /// express. It is preserved until the user chooses a simple schedule.
    Custom,
}

impl RepeatMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Never => "Does not repeat",
            Self::Daily => "Daily",
            Self::Weekdays => "Every weekday",
            Self::Weekly => "Weekly",
            Self::Monthly => "Monthly",
            Self::Yearly => "Yearly",
            Self::Custom => "Custom",
        }
    }

    /// The noun that follows the interval number, as in "every 2 weeks".
    pub fn unit(self) -> &'static str {
        match self {
            Self::Daily => "day(s)",
            Self::Weekly | Self::Weekdays => "week(s)",
            Self::Monthly => "month(s)",
            Self::Yearly => "year(s)",
            Self::Never | Self::Custom => "",
        }
    }
}

/// How a monthly habit picks its day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonthlyMode {
    Day,
    NthWeekday,
    LastDay,
}

/// When the series stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndKind {
    Never,
    OnDate,
    After,
}

/// A habit being filled in before it is written to Google.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    /// `None` when this is a new habit.
    pub id: Option<String>,
    pub calendar_id: Option<String>,
    pub title: String,
    pub notes: String,
    pub date: String,
    pub start: String,
    pub end: String,
    pub repeat: RepeatMode,
    /// "1" in "every 1 week".
    pub interval: String,
    /// Monday first, matching [`WEEKDAYS`].
    pub weekdays: [bool; 7],
    pub monthly: MonthlyMode,
    pub monthly_day: String,
    /// 1–5, or -1 for the last weekday of the month.
    pub monthly_nth: i8,
    pub monthly_weekday: Weekday,
    pub ends: EndKind,
    pub until: String,
    pub count: String,
    /// Dates removed from the series, shown as "Skip dates".
    pub skips: Vec<NaiveDate>,
    pub skip_input: String,
    /// Extra dates kept from an imported `RDATE`; they are written back but
    /// not shown as raw text.
    pub extra: Vec<NaiveDate>,
    /// The original lines when [`RepeatMode::Custom`] is selected.
    pub custom: Option<Vec<String>>,
    pub color_id: Option<String>,
    pub error: Option<String>,
}

impl Draft {
    pub fn new(day: NaiveDate, color_id: Option<String>) -> Self {
        let nth = nth_weekday(day);
        Self {
            id: None,
            calendar_id: None,
            title: String::new(),
            notes: String::new(),
            date: day.to_string(),
            start: "09:00".to_owned(),
            end: "09:30".to_owned(),
            repeat: RepeatMode::Weekly,
            interval: "1".to_owned(),
            weekdays: weekday_flags(&[day.weekday()]),
            monthly: MonthlyMode::Day,
            monthly_day: day.day().to_string(),
            monthly_nth: nth,
            monthly_weekday: day.weekday(),
            ends: EndKind::Never,
            until: String::new(),
            count: "10".to_owned(),
            skips: Vec::new(),
            skip_input: String::new(),
            extra: Vec::new(),
            custom: None,
            color_id,
            error: None,
        }
    }

    /// A draft seeded from the calendar's drag-to-create dialog.
    pub fn for_calendar(
        start: DateTime<Local>,
        end: DateTime<Local>,
        title: String,
        notes: String,
        color_id: Option<String>,
    ) -> Self {
        let mut draft = Self::new(start.date_naive(), color_id);
        draft.title = title;
        draft.notes = notes;
        draft.start = start.format("%H:%M").to_string();
        draft.end = end.format("%H:%M").to_string();
        draft
    }

    pub fn from_habit(habit: &Habit) -> Self {
        let mut draft = Self::new(habit.start.date_naive(), habit.color_id.clone());
        draft.id = Some(habit.id.clone());
        draft.calendar_id = Some(habit.calendar_id.clone());
        draft.title = habit.title.clone();
        draft.notes = habit.notes.clone();
        draft.start = habit.start.format("%H:%M").to_string();
        draft.end = habit.end.format("%H:%M").to_string();
        if habit.recurrence.is_empty() {
            draft.repeat = RepeatMode::Never;
        } else if !draft.apply_recurrence(&habit.recurrence) {
            draft.repeat = RepeatMode::Custom;
            draft.custom = Some(habit.recurrence.lines().to_vec());
        }
        draft
    }

    /// Reads an imported recurrence into the structured fields. Returns false
    /// for anything the editor cannot show without RFC text, which then stays
    /// untouched as a custom schedule.
    fn apply_recurrence(&mut self, recurrence: &Recurrence) -> bool {
        let Ok(date) = deadline::parse_date(self.date.trim()) else {
            return false;
        };
        let mut rule = None;
        let mut skips = Vec::new();
        let mut extra = Vec::new();
        for line in recurrence.lines() {
            if starts_with_kind(line, "RRULE") {
                if rule.is_some() {
                    return false;
                }
                let Some(value) = strip_prefix(line, "RRULE:") else {
                    return false;
                };
                rule = Some(value.to_owned());
            } else if starts_with_kind(line, "EXDATE") {
                let Some(mut dates) = dates_in(line, "EXDATE") else {
                    return false;
                };
                skips.append(&mut dates);
            } else if starts_with_kind(line, "RDATE") {
                let Some(mut dates) = dates_in(line, "RDATE") else {
                    return false;
                };
                extra.append(&mut dates);
            }
        }
        let Some(rule) = rule else {
            return false;
        };

        let mut freq = String::new();
        let mut interval = 1u32;
        let mut byday: Vec<String> = Vec::new();
        let mut bymonthday: Vec<i32> = Vec::new();
        let mut bymonth: Vec<u32> = Vec::new();
        let mut ends = EndKind::Never;
        let mut until = String::new();
        let mut count = String::new();

        for part in rule.split(';') {
            let Some((key, value)) = part.split_once('=') else {
                return false;
            };
            let key = key.trim().to_ascii_uppercase();
            let value = value.trim();
            if value.is_empty() {
                return false;
            }
            match key.as_str() {
                "FREQ" => freq = value.to_ascii_uppercase(),
                "INTERVAL" => match value.parse::<u32>() {
                    Ok(number) if number > 0 => interval = number,
                    _ => return false,
                },
                "BYDAY" => {
                    byday = value
                        .split(',')
                        .map(|day| day.trim().to_ascii_uppercase())
                        .collect();
                }
                "BYMONTHDAY" => {
                    bymonthday.clear();
                    for day in value.split(',') {
                        let Ok(day) = day.trim().parse::<i32>() else {
                            return false;
                        };
                        if day == 0 || day.abs() > 31 {
                            return false;
                        }
                        bymonthday.push(day);
                    }
                }
                "BYMONTH" => {
                    bymonth.clear();
                    for month in value.split(',') {
                        let Ok(month) = month.trim().parse::<u32>() else {
                            return false;
                        };
                        if !(1..=12).contains(&month) {
                            return false;
                        }
                        bymonth.push(month);
                    }
                }
                "WKST" => {
                    if parse_weekday(value).is_none() {
                        return false;
                    }
                }
                "UNTIL" => {
                    let Some(day) = parse_until(value) else {
                        return false;
                    };
                    ends = EndKind::OnDate;
                    until = day.to_string();
                }
                "COUNT" => {
                    let Ok(number) = value.parse::<u32>() else {
                        return false;
                    };
                    if number == 0 {
                        return false;
                    }
                    ends = EndKind::After;
                    count = number.to_string();
                }
                // Anything else (BYSETPOS, BYHOUR, BYYEARDAY, …) is a custom
                // schedule the simple editor must not silently rewrite.
                _ => return false,
            }
        }

        if !bymonth.is_empty() || !bymonthday.is_empty() {
            if freq != "MONTHLY" && freq != "YEARLY" {
                return false;
            }
        }

        match freq.as_str() {
            "DAILY" => {
                if !byday.is_empty() || !bymonthday.is_empty() || !bymonth.is_empty() {
                    return false;
                }
                self.repeat = RepeatMode::Daily;
            }
            "WEEKLY" => {
                if !bymonthday.is_empty() || !bymonth.is_empty() {
                    return false;
                }
                let days = if byday.is_empty() {
                    vec![date.weekday()]
                } else {
                    let mut days = Vec::new();
                    for day in &byday {
                        let Some(day) = parse_weekday(day) else {
                            return false;
                        };
                        days.push(day);
                    }
                    days
                };
                if interval == 1
                    && days
                        == [
                            Weekday::Mon,
                            Weekday::Tue,
                            Weekday::Wed,
                            Weekday::Thu,
                            Weekday::Fri,
                        ]
                {
                    self.repeat = RepeatMode::Weekdays;
                } else {
                    self.repeat = RepeatMode::Weekly;
                }
                self.weekdays = weekday_flags(&days);
            }
            "MONTHLY" => {
                if !bymonth.is_empty() {
                    return false;
                }
                if byday.len() == 1 && bymonthday.is_empty() {
                    let Some((nth, weekday)) = parse_ordinal_weekday(&byday[0]) else {
                        return false;
                    };
                    self.monthly = MonthlyMode::NthWeekday;
                    self.monthly_nth = nth;
                    self.monthly_weekday = weekday;
                } else if byday.is_empty() && bymonthday.len() == 1 {
                    let day = bymonthday[0];
                    if day == -1 {
                        self.monthly = MonthlyMode::LastDay;
                    } else if (1..=31).contains(&day) {
                        self.monthly = MonthlyMode::Day;
                        self.monthly_day = day.to_string();
                    } else {
                        return false;
                    }
                } else {
                    return false;
                }
                self.repeat = RepeatMode::Monthly;
            }
            "YEARLY" => {
                if !byday.is_empty() || bymonth.len() > 1 || bymonthday.len() > 1 {
                    return false;
                }
                // The start date is the anchor. A yearly rule that picks a
                // different month or day is left as a custom schedule.
                if bymonth.first().is_some_and(|month| *month != date.month())
                    || bymonthday
                        .first()
                        .is_some_and(|day| *day != date.day() as i32)
                {
                    return false;
                }
                self.repeat = RepeatMode::Yearly;
            }
            _ => return false,
        }

        self.interval = interval.to_string();
        self.ends = ends;
        self.until = until;
        self.count = count;
        skips.sort_unstable();
        skips.dedup();
        extra.sort_unstable();
        extra.dedup();
        self.skips = skips;
        self.extra = extra;
        true
    }

    /// Parses the fields into the values the Calendar API write needs.
    pub fn parsed(&self) -> Result<ParsedHabit, String> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err("Give the habit a name".to_owned());
        }
        let date = deadline::parse_date(self.date.trim())?;
        let start = deadline::parse_time(self.start.trim())?;
        let end = deadline::parse_time(self.end.trim())?;
        if end <= start {
            return Err("The end time has to be after the start".to_owned());
        }
        let recurrence = self.recurrence_lines_at(start)?;
        Ok(ParsedHabit {
            title: title.to_owned(),
            date,
            start,
            end,
            recurrence,
        })
    }

    /// Builds the recurrence array the editor describes. The clock time is
    /// used for skipped and extra dates; without a valid one it falls back to
    /// the usual 09:00 default so validation can still report other problems.
    pub fn recurrence_lines(&self) -> Result<Vec<String>, String> {
        let start = deadline::parse_time(self.start.trim())
            .unwrap_or_else(|_| NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        self.recurrence_lines_at(start)
    }

    fn recurrence_lines_at(&self, start: NaiveTime) -> Result<Vec<String>, String> {
        if self.repeat == RepeatMode::Custom {
            return Ok(self.custom.clone().unwrap_or_default());
        }

        let date = deadline::parse_date(self.date.trim())?;
        let mut lines = Vec::new();
        if self.repeat != RepeatMode::Never {
            let interval = self.interval_value()?;
            let mut rule = match self.repeat {
                RepeatMode::Daily => format!("FREQ=DAILY;INTERVAL={interval}"),
                RepeatMode::Weekdays => "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_owned(),
                RepeatMode::Weekly => {
                    let days = self.selected_weekdays();
                    if days.is_empty() {
                        return Err("Pick at least one day to repeat on".to_owned());
                    }
                    let codes = days
                        .iter()
                        .map(|day| weekday_code(*day))
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("FREQ=WEEKLY;INTERVAL={interval};BYDAY={codes}")
                }
                RepeatMode::Monthly => {
                    let monthly = match self.monthly {
                        MonthlyMode::Day => {
                            let day = self.monthly_day_value()?;
                            format!("BYMONTHDAY={day}")
                        }
                        MonthlyMode::NthWeekday => {
                            format!(
                                "BYDAY={}{}",
                                self.monthly_nth,
                                weekday_code(self.monthly_weekday)
                            )
                        }
                        MonthlyMode::LastDay => "BYMONTHDAY=-1".to_owned(),
                    };
                    format!("FREQ=MONTHLY;INTERVAL={interval};{monthly}")
                }
                RepeatMode::Yearly => format!(
                    "FREQ=YEARLY;INTERVAL={interval};BYMONTH={};BYMONTHDAY={}",
                    date.month(),
                    date.day()
                ),
                RepeatMode::Never | RepeatMode::Custom => unreachable!(),
            };
            match self.ends {
                EndKind::Never => {}
                EndKind::OnDate => {
                    let until = deadline::parse_date(self.until.trim())?;
                    if until < date {
                        return Err("The repeat cannot end before it starts".to_owned());
                    }
                    rule.push_str(&format!(";UNTIL={}", until_utc(until)));
                }
                EndKind::After => {
                    let count = self.count_value()?;
                    rule.push_str(&format!(";COUNT={count}"));
                }
            }
            let rule = rule
                .strip_prefix("FREQ=")
                .map(|rest| format!("FREQ={rest}"))
                .unwrap_or(rule);
            validate_rrule(&rule)?;
            lines.push(format!("RRULE:{rule}"));
        }

        for day in &self.extra {
            lines.push(format!(
                "RDATE:{}{}",
                day.format("%Y%m%d"),
                start.format("%H%M%S")
            ));
        }
        for day in &self.skips {
            lines.push(format!(
                "EXDATE:{}{}",
                day.format("%Y%m%d"),
                start.format("%H%M%S")
            ));
        }
        Ok(lines)
    }

    fn interval_value(&self) -> Result<u32, String> {
        self.interval
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|number| *number > 0)
            .ok_or_else(|| "The repeat interval needs to be a whole number".to_owned())
    }

    fn monthly_day_value(&self) -> Result<u32, String> {
        self.monthly_day
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|day| (1..=31).contains(day))
            .ok_or_else(|| "The day of the month has to be between 1 and 31".to_owned())
    }

    fn count_value(&self) -> Result<u32, String> {
        self.count
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|number| *number > 0)
            .ok_or_else(|| "The number of repeats needs to be a whole number".to_owned())
    }

    fn selected_weekdays(&self) -> Vec<Weekday> {
        WEEKDAYS
            .iter()
            .zip(self.weekdays)
            .filter(|(_, selected)| *selected)
            .map(|(day, _)| *day)
            .collect()
    }

    /// A short, human description used by the cards.
    pub fn repeat_summary(&self) -> String {
        if self.repeat == RepeatMode::Custom {
            return "Custom schedule".to_owned();
        }
        if self.repeat == RepeatMode::Never {
            return "Does not repeat".to_owned();
        }
        let interval = self
            .interval
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|number| *number > 0)
            .unwrap_or(1);
        let mut summary = match self.repeat {
            RepeatMode::Daily => {
                if interval == 1 {
                    "Daily".to_owned()
                } else {
                    format!("Every {interval} days")
                }
            }
            RepeatMode::Weekdays => "Every weekday".to_owned(),
            RepeatMode::Weekly => {
                let days = self
                    .weekdays
                    .iter()
                    .zip(WEEKDAYS)
                    .filter(|(selected, _)| **selected)
                    .map(|(_, day)| day)
                    .collect::<Vec<_>>();
                let days = if days.is_empty() {
                    weekday_name(self.monthly_weekday).to_owned()
                } else {
                    weekday_list(&days)
                };
                if interval == 1 {
                    format!("Weekly on {days}")
                } else {
                    format!("Every {interval} weeks on {days}")
                }
            }
            RepeatMode::Monthly => {
                let base = match self.monthly {
                    MonthlyMode::Day => {
                        let day = self.monthly_day.trim().parse::<u32>().unwrap_or(1);
                        format!("on day {day}")
                    }
                    MonthlyMode::NthWeekday => format!(
                        "on the {} {}",
                        nth_label(self.monthly_nth),
                        weekday_name(self.monthly_weekday)
                    ),
                    MonthlyMode::LastDay => "on the last day".to_owned(),
                };
                if interval == 1 {
                    format!("Monthly {base}")
                } else {
                    format!("Every {interval} months {base}")
                }
            }
            RepeatMode::Yearly => {
                let date = deadline::parse_date(self.date.trim()).ok();
                let day = date
                    .map(pretty_date)
                    .unwrap_or_else(|| "the same day".to_owned());
                if interval == 1 {
                    format!("Yearly on {day}")
                } else {
                    format!("Every {interval} years on {day}")
                }
            }
            RepeatMode::Never | RepeatMode::Custom => unreachable!(),
        };
        match self.ends {
            EndKind::Never => {}
            EndKind::OnDate => {
                if let Ok(day) = deadline::parse_date(self.until.trim()) {
                    summary.push_str(&format!(" until {}", pretty_date(day)));
                }
            }
            EndKind::After => {
                if let Ok(count) = self.count.trim().parse::<u32>() {
                    summary.push_str(&format!(" for {count} times"));
                }
            }
        }
        if !self.skips.is_empty() {
            summary.push_str(&format!(" · {} skipped", self.skips.len()));
        }
        summary
    }
}

/// A draft's fields after parsing, before the Calendar write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedHabit {
    pub title: String,
    pub date: NaiveDate,
    pub start: NaiveTime,
    pub end: NaiveTime,
    pub recurrence: Vec<String>,
}

fn parse_weekday(value: &str) -> Option<Weekday> {
    match value.trim().to_ascii_uppercase().as_str() {
        "MO" => Some(Weekday::Mon),
        "TU" => Some(Weekday::Tue),
        "WE" => Some(Weekday::Wed),
        "TH" => Some(Weekday::Thu),
        "FR" => Some(Weekday::Fri),
        "SA" => Some(Weekday::Sat),
        "SU" => Some(Weekday::Sun),
        _ => None,
    }
}

fn weekday_code(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

fn weekday_name(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

fn weekday_short(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

fn weekday_list(days: &[Weekday]) -> String {
    if days.len() == 1 {
        return weekday_name(days[0]).to_owned();
    }
    days.iter()
        .map(|day| weekday_short(*day))
        .collect::<Vec<_>>()
        .join(", ")
}

fn weekday_flags(days: &[Weekday]) -> [bool; 7] {
    let mut flags = [false; 7];
    for day in days {
        flags[weekday_index(*day)] = true;
    }
    flags
}

fn weekday_index(day: Weekday) -> usize {
    day.num_days_from_monday() as usize
}

fn nth_label(nth: i8) -> &'static str {
    match nth {
        1 => "first",
        2 => "second",
        3 => "third",
        4 => "fourth",
        5 => "fifth",
        -1 => "last",
        _ => "first",
    }
}

fn nth_weekday(date: NaiveDate) -> i8 {
    let next = date + chrono::Duration::days(7);
    if next.month() == date.month() {
        ((date.day() - 1) / 7 + 1) as i8
    } else {
        -1
    }
}

fn parse_ordinal_weekday(value: &str) -> Option<(i8, Weekday)> {
    if value.len() < 3 {
        return None;
    }
    let (prefix, day) = value.split_at(value.len() - 2);
    let weekday = parse_weekday(day)?;
    if prefix.is_empty() {
        return None;
    }
    let nth = prefix.parse::<i8>().ok()?;
    if nth == 0 || nth.abs() > 5 {
        return None;
    }
    Some((nth, weekday))
}

fn parse_until(value: &str) -> Option<NaiveDate> {
    let date: String = value.chars().take(8).collect();
    if date.len() != 8 {
        return None;
    }
    NaiveDate::parse_from_str(&date, "%Y%m%d").ok()
}

fn dates_in(line: &str, kind: &str) -> Option<Vec<NaiveDate>> {
    if !starts_with_kind(line, kind) {
        return None;
    }
    let values = line[kind.len()..].split_once(':')?.1;
    let mut dates = Vec::new();
    for value in values.split(',') {
        let date: String = value.trim().chars().take(8).collect();
        if date.len() != 8 {
            return None;
        }
        dates.push(NaiveDate::parse_from_str(&date, "%Y%m%d").ok()?);
    }
    if dates.is_empty() { None } else { Some(dates) }
}

fn until_utc(date: NaiveDate) -> String {
    let naive = date
        .and_hms_opt(23, 59, 59)
        .expect("a valid end-of-day time");
    let local = Local
        .from_local_datetime(&naive)
        .single()
        .unwrap_or_else(|| Local.from_utc_datetime(&naive));
    local
        .with_timezone(&Utc)
        .format("%Y%m%dT%H%M%SZ")
        .to_string()
}

fn pretty_date(date: NaiveDate) -> String {
    format!("{} {}", date.format("%B"), date.day())
}

fn validate_rrule(rrule: &str) -> Result<(), String> {
    let mut has_freq = false;
    for part in rrule.split(';') {
        let part = part.trim();
        if part.is_empty() {
            return Err("The repeat has an empty part".to_owned());
        }
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| "The repeat has a part without a value".to_owned())?;
        let key = key.trim().to_ascii_uppercase();
        let value = value.trim();
        if value.is_empty() {
            return Err("The repeat has a part without a value".to_owned());
        }
        match key.as_str() {
            "FREQ" => {
                has_freq = true;
                if !matches!(
                    value.to_ascii_uppercase().as_str(),
                    "SECONDLY" | "MINUTELY" | "HOURLY" | "DAILY" | "WEEKLY" | "MONTHLY" | "YEARLY"
                ) {
                    return Err("The repeat frequency is not one the calendar knows".to_owned());
                }
            }
            "INTERVAL" | "COUNT" => {
                value
                    .parse::<u32>()
                    .ok()
                    .filter(|number| *number > 0)
                    .ok_or_else(|| "The repeat numbers have to be positive".to_owned())?;
            }
            "BYDAY" => {
                for day in value.split(',') {
                    if parse_ordinal_weekday(day.trim()).is_none()
                        && parse_weekday(day.trim()).is_none()
                    {
                        return Err("The repeat has an unknown weekday".to_owned());
                    }
                }
            }
            "BYMONTHDAY" => {
                for day in value.split(',') {
                    let day: i32 = day
                        .trim()
                        .parse()
                        .map_err(|_| "The repeat has an invalid day of the month".to_owned())?;
                    if day == 0 || day.abs() > 31 {
                        return Err("The repeat has an invalid day of the month".to_owned());
                    }
                }
            }
            "BYMONTH" => {
                for month in value.split(',') {
                    let month: u32 = month
                        .trim()
                        .parse()
                        .map_err(|_| "The repeat has an invalid month".to_owned())?;
                    if !(1..=12).contains(&month) {
                        return Err("The repeat has an invalid month".to_owned());
                    }
                }
            }
            "BYSETPOS" => {
                for position in value.split(',') {
                    let position: i32 = position
                        .trim()
                        .parse()
                        .map_err(|_| "The repeat has an invalid position".to_owned())?;
                    if position == 0 {
                        return Err("The repeat position cannot be zero".to_owned());
                    }
                }
            }
            "WKST" => {
                if parse_weekday(value).is_none() {
                    return Err("The repeat has an unknown week start".to_owned());
                }
            }
            "UNTIL" => {
                if parse_until(value).is_none() {
                    return Err("The repeat has an invalid end date".to_owned());
                }
            }
            _ => {}
        }
    }
    if !has_freq {
        return Err("The repeat needs a frequency".to_owned());
    }
    Ok(())
}

/// What the Habits tab is holding.
#[derive(Debug, Default)]
pub struct Store {
    pub habits: Vec<Habit>,
    /// The habit editor, when it is open.
    pub draft: Option<Draft>,
    /// A Calendar write is in flight.
    pub busy: bool,
    pub error: Option<String>,
    /// A deletion waiting for confirmation, by habit id.
    pub confirm_delete: Option<String>,
    /// Whether the list has been fetched at least once.
    pub loaded: bool,
}

#[derive(Debug, Clone)]
pub enum Action {
    Refresh,
    Loaded(Result<Vec<HabitEvent>, String>),
    New,
    Edit(String),
    CancelEdit,
    Title(String),
    Notes(String),
    Date(String),
    Start(String),
    End(String),
    Repeat(RepeatMode),
    Interval(String),
    ToggleWeekday(usize),
    MonthlyMode(MonthlyMode),
    MonthlyDay(String),
    MonthlyNth(i8),
    MonthlyWeekday(Weekday),
    Ends(EndKind),
    Until(String),
    Count(String),
    SkipInput(String),
    SkipAdd,
    SkipRemove(NaiveDate),
    SimpleRepeat,
    Color(Option<Color>),
    Save,
    Saved(Result<HabitEvent, String>),
    DeleteRequested(String),
    DeleteCancelled,
    DeleteConfirmed(String),
    Deleted(Result<String, String>),
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
    match action {
        Action::Refresh => refresh(state),

        Action::Loaded(result) => {
            state.habits.busy = false;
            state.habits.loaded = true;
            match result {
                Ok(habits) => {
                    state.habits.habits = habits.into_iter().map(Habit::from_event).collect();
                    sort(&mut state.habits.habits);
                    state.habits.error = None;
                }
                Err(error) => state.habits.error = Some(error),
            }
            Task::none()
        }

        Action::New => {
            state.habits.confirm_delete = None;
            let day = Local::now().date_naive();
            let color_id = colors::event_id(state.config.calendar.habit_color()).map(str::to_owned);
            state.habits.draft = Some(Draft::new(day, color_id));
            state.habits.error = None;
            Task::none()
        }

        Action::Edit(id) => {
            let Some(habit) = state.habits.habits.iter().find(|habit| habit.id == id) else {
                return Task::none();
            };
            state.habits.draft = Some(Draft::from_habit(habit));
            state.habits.confirm_delete = None;
            state.habits.error = None;
            Task::none()
        }

        Action::CancelEdit => {
            state.habits.draft = None;
            Task::none()
        }

        Action::Title(value) => edit(state, |draft| draft.title = value),
        Action::Notes(value) => edit(state, |draft| draft.notes = value),
        Action::Date(value) => edit(state, |draft| draft.date = value),
        Action::Start(value) => edit(state, |draft| draft.start = value),
        Action::End(value) => edit(state, |draft| draft.end = value),
        Action::Interval(value) => edit(state, |draft| draft.interval = value),
        Action::MonthlyDay(value) => edit(state, |draft| draft.monthly_day = value),
        Action::Until(value) => edit(state, |draft| draft.until = value),
        Action::Count(value) => edit(state, |draft| draft.count = value),

        Action::Repeat(mode) => edit(state, |draft| {
            draft.repeat = mode;
            if mode != RepeatMode::Custom {
                draft.custom = None;
            }
            if mode == RepeatMode::Weekly && draft.weekdays.iter().all(|selected| !*selected) {
                if let Ok(date) = deadline::parse_date(draft.date.trim()) {
                    draft.weekdays[weekday_index(date.weekday())] = true;
                }
            }
        }),

        Action::ToggleWeekday(index) => edit(state, |draft| {
            if let Some(selected) = draft.weekdays.get_mut(index) {
                *selected = !*selected;
            }
        }),

        Action::MonthlyMode(mode) => edit(state, |draft| draft.monthly = mode),
        Action::MonthlyNth(nth) => edit(state, |draft| {
            draft.monthly = MonthlyMode::NthWeekday;
            draft.monthly_nth = nth;
        }),
        Action::MonthlyWeekday(day) => edit(state, |draft| {
            draft.monthly = MonthlyMode::NthWeekday;
            draft.monthly_weekday = day;
        }),

        Action::Ends(kind) => edit(state, |draft| draft.ends = kind),

        Action::SkipInput(value) => edit(state, |draft| draft.skip_input = value),

        Action::SkipAdd => {
            if let Some(draft) = state.habits.draft.as_mut() {
                match deadline::parse_date(draft.skip_input.trim()) {
                    Ok(day) => {
                        if !draft.skips.contains(&day) {
                            draft.skips.push(day);
                            draft.skips.sort_unstable();
                        }
                        draft.skip_input.clear();
                        draft.error = None;
                    }
                    Err(error) => draft.error = Some(error),
                }
            }
            Task::none()
        }

        Action::SkipRemove(day) => edit(state, |draft| draft.skips.retain(|date| *date != day)),

        Action::SimpleRepeat => edit(state, |draft| {
            draft.repeat = RepeatMode::Weekly;
            draft.custom = None;
            if draft.weekdays.iter().all(|selected| !*selected) {
                if let Ok(date) = deadline::parse_date(draft.date.trim()) {
                    draft.weekdays[weekday_index(date.weekday())] = true;
                }
            }
        }),

        Action::Color(color) => edit(state, |draft| {
            draft.color_id = color.and_then(colors::event_id).map(str::to_owned);
        }),

        Action::Save => save(state),

        Action::Saved(result) => {
            state.habits.busy = false;
            match result {
                Ok(event) => {
                    let habit = Habit::from_event(event);
                    state.habits.draft = None;
                    state.habits.error = None;
                    if let Some(existing) = state
                        .habits
                        .habits
                        .iter_mut()
                        .find(|candidate| candidate.id == habit.id)
                    {
                        *existing = habit;
                    } else {
                        state.habits.habits.push(habit);
                    }
                    sort(&mut state.habits.habits);
                    // The day and week grids hold instances of this series.
                    Task::done(Message::CalendarSyncRequested)
                }
                Err(error) => {
                    if let Some(draft) = state.habits.draft.as_mut() {
                        draft.error = Some(error);
                    } else {
                        state.habits.error = Some(error);
                    }
                    Task::none()
                }
            }
        }

        Action::DeleteRequested(id) => {
            state.habits.confirm_delete = Some(id);
            Task::none()
        }

        Action::DeleteCancelled => {
            state.habits.confirm_delete = None;
            Task::none()
        }

        Action::DeleteConfirmed(id) => {
            let Some(habit) = state.habits.habits.iter().find(|habit| habit.id == id) else {
                state.habits.confirm_delete = None;
                return Task::none();
            };
            let calendar_id = habit.calendar_id.clone();
            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            state.habits.busy = true;
            state.habits.error = None;
            blocking::run(move || {
                calendar::delete_habit(&client_id, &client_secret, &calendar_id, &id).map(|_| id)
            })
            .map(|result| Message::Habits(Action::Deleted(result)))
        }

        Action::Deleted(result) => {
            state.habits.busy = false;
            state.habits.confirm_delete = None;
            match result {
                Ok(id) => {
                    state.habits.habits.retain(|habit| habit.id != id);
                    state.habits.error = None;
                    Task::done(Message::CalendarSyncRequested)
                }
                Err(error) => {
                    state.habits.error = Some(error);
                    Task::none()
                }
            }
        }
    }
}

fn refresh(state: &mut State) -> Task<Message> {
    if state.habits.busy {
        return Task::none();
    }
    if !state.calendar.is_signed_in() {
        state.habits.error = Some("Connect Google Calendar in Settings to use habits".to_owned());
        return Task::none();
    }
    state.habits.busy = true;
    state.habits.error = None;
    let client_id = state.config.calendar.client_id.clone();
    let client_secret = state.config.calendar.client_secret.clone();
    let app_calendar = state.config.calendar.app_calendar.clone();
    blocking::run(move || {
        calendar::sync_habits(&client_id, &client_secret, app_calendar.as_deref())
    })
    .map(|result| Message::Habits(Action::Loaded(result)))
}

fn edit(state: &mut State, change: impl FnOnce(&mut Draft)) -> Task<Message> {
    if let Some(draft) = state.habits.draft.as_mut() {
        change(draft);
        draft.error = None;
    }
    Task::none()
}

fn save(state: &mut State) -> Task<Message> {
    let Some(draft) = state.habits.draft.clone() else {
        return Task::none();
    };
    let parsed = match draft.parsed() {
        Ok(parsed) => parsed,
        Err(error) => {
            if let Some(draft) = state.habits.draft.as_mut() {
                draft.error = Some(error);
            }
            return Task::none();
        }
    };
    let start = calendar::at_clock(parsed.date, parsed.start.hour(), parsed.start.minute());
    let end = calendar::at_clock(parsed.date, parsed.end.hour(), parsed.end.minute());
    let id = draft.id.clone();
    let calendar_id = draft
        .calendar_id
        .clone()
        .or_else(|| state.config.calendar.app_calendar.clone());
    let title = parsed.title;
    let notes = draft.notes.clone();
    let recurrence = parsed.recurrence;
    let color_id = draft.color_id.clone();
    let client_id = state.config.calendar.client_id.clone();
    let client_secret = state.config.calendar.client_secret.clone();
    state.habits.busy = true;
    state.habits.error = None;
    blocking::run(move || {
        calendar::write_habit(
            &client_id,
            &client_secret,
            calendar::HabitWrite {
                id: id.as_deref(),
                calendar_id: calendar_id.as_deref(),
                title: &title,
                notes: &notes,
                start,
                end,
                recurrence: &recurrence,
                color_id: color_id.as_deref(),
            },
        )
    })
    .map(|result| Message::Habits(Action::Saved(result)))
}

fn sort(habits: &mut [Habit]) {
    habits.sort_by(|left, right| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then(left.start.cmp(&right.start))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        deadline::parse_date(text).unwrap()
    }

    fn draft() -> Draft {
        let mut draft = Draft::new(date("2026-10-28"), Some("3".to_owned()));
        draft.title = "Stretch".to_owned();
        draft
    }

    #[test]
    fn common_rules_round_trip_through_the_structured_editor() {
        for (rule, expected) in [
            (
                "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE",
                "Every 2 weeks on Mon, Wed",
            ),
            ("RRULE:FREQ=DAILY", "Daily"),
            ("RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", "Every weekday"),
            ("RRULE:FREQ=MONTHLY;BYMONTHDAY=28", "Monthly on day 28"),
            (
                "RRULE:FREQ=MONTHLY;BYDAY=4WE",
                "Monthly on the fourth Wednesday",
            ),
            (
                "RRULE:FREQ=MONTHLY;BYMONTHDAY=-1",
                "Monthly on the last day",
            ),
            ("RRULE:FREQ=YEARLY", "Yearly on October 28"),
        ] {
            let habit = Habit {
                id: "1".into(),
                calendar_id: "time-slay".into(),
                title: "Stretch".into(),
                notes: String::new(),
                start: calendar::at_clock(date("2026-10-28"), 9, 0),
                end: calendar::at_clock(date("2026-10-28"), 9, 30),
                recurrence: Recurrence::from_lines(&[rule.to_owned()]),
                color_id: None,
            };
            let mut draft = Draft::from_habit(&habit);
            assert_eq!(draft.repeat_summary(), expected, "{rule}");
            let lines = draft.recurrence_lines().unwrap();
            assert!(
                lines.iter().all(|line| line.starts_with("RRULE:")),
                "{rule}"
            );
            // Reopening the written rule keeps the same readable summary.
            draft.custom = None;
            let rewritten = Recurrence::from_lines(&lines);
            assert!(draft.apply_recurrence(&rewritten), "{rule}");
            assert_eq!(draft.repeat_summary(), expected, "{rule}");
        }
    }

    #[test]
    fn ends_and_skips_are_written_without_showing_rfc_text() {
        let mut draft = draft();
        draft.repeat = RepeatMode::Weekly;
        draft.weekdays = weekday_flags(&[Weekday::Mon, Weekday::Wed]);
        draft.interval = "2".into();
        draft.ends = EndKind::OnDate;
        draft.until = "2026-12-31".into();
        draft.skips = vec![date("2026-11-11")];
        let lines = draft.recurrence_lines().unwrap();
        assert_eq!(
            lines[0],
            "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;UNTIL=20270101T075959Z"
        );
        assert_eq!(lines[1], "EXDATE:20261111090000");
        assert!(draft.repeat_summary().contains("until December 31"));
        assert!(draft.repeat_summary().contains("1 skipped"));
    }

    #[test]
    fn weekly_needs_at_least_one_day_and_monthly_needs_a_real_day() {
        let mut draft = draft();
        draft.repeat = RepeatMode::Weekly;
        draft.weekdays = [false; 7];
        assert!(draft.parsed().is_err());
        draft.monthly = MonthlyMode::Day;
        draft.monthly_day = "32".into();
        draft.repeat = RepeatMode::Monthly;
        assert!(draft.parsed().is_err());
        draft.monthly_day = "28".into();
        assert!(draft.parsed().is_ok());
    }

    #[test]
    fn an_unsupported_rule_stays_custom_until_the_user_simplifies_it() {
        let recurrence = Recurrence::from_lines(&[
            "RRULE:FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1".to_owned(),
            "RDATE;VALUE=DATE:20261102".to_owned(),
        ]);
        let habit = Habit {
            id: "1".into(),
            calendar_id: "time-slay".into(),
            title: "Stretch".into(),
            notes: String::new(),
            start: calendar::at_clock(date("2026-10-28"), 9, 0),
            end: calendar::at_clock(date("2026-10-28"), 9, 30),
            recurrence: recurrence.clone(),
            color_id: None,
        };
        let mut draft = Draft::from_habit(&habit);
        assert_eq!(draft.repeat, RepeatMode::Custom);
        assert_eq!(draft.repeat_summary(), "Custom schedule");
        assert_eq!(draft.recurrence_lines().unwrap(), recurrence.lines());

        draft.repeat = RepeatMode::Weekly;
        draft.custom = None;
        assert!(draft.recurrence_lines().unwrap()[0].starts_with("RRULE:FREQ=WEEKLY"));
    }

    #[test]
    fn lines_that_are_not_recurrence_parts_are_ignored() {
        let recurrence = Recurrence::from_lines(&[
            "RÉSUMÉ".into(),
            "not a rule".into(),
            "RDATE:20261102".into(),
        ]);
        assert_eq!(recurrence.lines(), ["RDATE:20261102"]);
    }

    #[test]
    fn imported_rdates_survive_without_being_shown() {
        let lines = vec![
            "RRULE:FREQ=WEEKLY;BYDAY=MO,WE;INTERVAL=2".to_owned(),
            "RDATE;VALUE=DATE:20261102,20261109".to_owned(),
            "EXDATE;TZID=Europe/London:20261111T090000".to_owned(),
        ];
        let recurrence = Recurrence::from_lines(&lines);
        let habit = Habit {
            id: "1".into(),
            calendar_id: "time-slay".into(),
            title: "Stretch".into(),
            notes: String::new(),
            start: calendar::at_clock(date("2026-10-28"), 9, 0),
            end: calendar::at_clock(date("2026-10-28"), 9, 30),
            recurrence,
            color_id: None,
        };
        let draft = Draft::from_habit(&habit);
        assert_eq!(draft.repeat, RepeatMode::Weekly);
        assert_eq!(draft.extra, [date("2026-11-02"), date("2026-11-09")]);
        assert_eq!(draft.skips, [date("2026-11-11")]);
        let written = draft.recurrence_lines().unwrap();
        assert!(written.contains(&"RDATE:20261102090000".to_owned()));
        assert!(written.contains(&"RDATE:20261109090000".to_owned()));
        assert!(written.contains(&"EXDATE:20261111090000".to_owned()));
    }

    #[test]
    fn a_malformed_schedule_is_reported_before_anything_is_written() {
        let mut draft = draft();
        draft.repeat = RepeatMode::Daily;
        draft.interval = "0".into();
        assert!(draft.parsed().is_err());
        draft.interval = "1".into();
        draft.ends = EndKind::After;
        draft.count = "0".into();
        assert!(draft.parsed().is_err());
        draft.count = "5".into();
        draft.ends = EndKind::OnDate;
        draft.until = "2026-10-01".into();
        assert!(draft.parsed().is_err());
    }

    #[test]
    fn a_draft_needs_a_name_a_real_day_and_an_end_after_the_start() {
        let mut draft = draft();
        draft.title = "  ".into();
        assert!(draft.parsed().is_err());
        draft.title = "Stretch".into();
        draft.date = "2026-02-30".into();
        assert!(draft.parsed().is_err());
        draft.date = "2026-10-28".into();
        draft.end = draft.start.clone();
        assert!(draft.parsed().is_err());
        draft.end = "10:00".into();
        assert_eq!(
            draft.parsed().unwrap().start,
            deadline::parse_time("09:00").unwrap()
        );
    }

    #[test]
    fn the_recurrence_label_is_readable_without_rfc_details() {
        let habit = Habit {
            id: "1".into(),
            calendar_id: "time-slay".into(),
            title: "Stretch".into(),
            notes: String::new(),
            start: calendar::at_clock(date("2026-10-28"), 9, 0),
            end: calendar::at_clock(date("2026-10-28"), 9, 30),
            recurrence: Recurrence::from_lines(&["RRULE:FREQ=DAILY".into()]),
            color_id: None,
        };
        assert_eq!(habit.recurrence_label(), "Daily");
        assert_eq!(habit.time_label(), "09:00 – 09:30");
        let one_off = Habit {
            recurrence: Recurrence::default(),
            ..habit
        };
        assert_eq!(one_off.recurrence_label(), "Does not repeat");
    }
}
