//! The structured recurrence editor shared by habits, events and time blocks.
//!
//! The editor never asks anyone to type RFC 5545 parts. It holds a simple
//! schedule — daily, every weekday, weekly on chosen days, monthly by day or
//! weekday, yearly, and when the series ends — and turns that into the
//! `RRULE`/`RDATE`/`EXDATE` lines Google Calendar stores only when it is
//! written. A rule imported from another app that the simple editor cannot
//! express is preserved as a custom schedule until the user replaces it.

use crate::calendar::deadline;
use chrono::{Datelike, Local, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};

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

/// How a repeating item repeats, in the words the editor offers.
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
    pub const SIMPLE: [RepeatMode; 6] = [
        RepeatMode::Never,
        RepeatMode::Daily,
        RepeatMode::Weekdays,
        RepeatMode::Weekly,
        RepeatMode::Monthly,
        RepeatMode::Yearly,
    ];

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

/// How a monthly repeat picks its day.
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

/// A change made in the shared recurrence controls.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
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
}

/// The structured repeat schedule held by every editor.
///
/// It belongs to an anchor date, the day the series starts. The weekday and
/// month-day defaults are seeded from that date, so a weekly series begins on
/// the day it is created without the user having to tick anything.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
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
}

impl Rule {
    /// A weekly rule on `day`, the default for a new repeating item.
    pub fn weekly(day: NaiveDate) -> Self {
        let nth = nth_weekday(day);
        Self {
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
        }
    }

    /// A schedule that does not repeat, the default for a new event.
    pub fn never(day: NaiveDate) -> Self {
        let mut rule = Self::weekly(day);
        rule.repeat = RepeatMode::Never;
        rule
    }

    /// Reads Google's recurrence lines into the simple fields. Unsupported
    /// rules become [`RepeatMode::Custom`] rather than being rewritten.
    pub fn from_recurrence(anchor: NaiveDate, recurrence: &Recurrence) -> Self {
        if recurrence.is_empty() {
            return Self::never(anchor);
        }
        let mut rule = Self::weekly(anchor);
        if !rule.apply(anchor, recurrence) {
            rule.repeat = RepeatMode::Custom;
            rule.custom = Some(recurrence.lines().to_vec());
        }
        rule
    }

    /// Whether the rule repeats at all.
    pub fn repeats(&self) -> bool {
        !matches!(self.repeat, RepeatMode::Never) && self.custom.is_none()
    }

    /// Applies one editor change. `anchor` is the series' first day.
    pub fn apply_action(&mut self, action: Action, anchor: NaiveDate) {
        match action {
            Action::Repeat(mode) => {
                self.repeat = mode;
                if mode != RepeatMode::Custom {
                    self.custom = None;
                }
                if mode == RepeatMode::Weekly && self.weekdays.iter().all(|selected| !*selected) {
                    self.weekdays[weekday_index(anchor.weekday())] = true;
                }
            }
            Action::Interval(value) => self.interval = value,
            Action::ToggleWeekday(index) => {
                if let Some(selected) = self.weekdays.get_mut(index) {
                    *selected = !*selected;
                }
            }
            Action::MonthlyMode(mode) => self.monthly = mode,
            Action::MonthlyDay(value) => self.monthly_day = value,
            Action::MonthlyNth(nth) => {
                self.monthly = MonthlyMode::NthWeekday;
                self.monthly_nth = nth;
            }
            Action::MonthlyWeekday(day) => {
                self.monthly = MonthlyMode::NthWeekday;
                self.monthly_weekday = day;
            }
            Action::Ends(kind) => self.ends = kind,
            Action::Until(value) => self.until = value,
            Action::Count(value) => self.count = value,
            Action::SkipInput(value) => self.skip_input = value,
            Action::SkipAdd => match deadline::parse_date(self.skip_input.trim()) {
                Ok(day) => {
                    if !self.skips.contains(&day) {
                        self.skips.push(day);
                        self.skips.sort_unstable();
                    }
                    self.skip_input.clear();
                }
                Err(_) => {}
            },
            Action::SkipRemove(day) => self.skips.retain(|date| *date != day),
            Action::SimpleRepeat => {
                self.repeat = RepeatMode::Weekly;
                self.custom = None;
                if self.weekdays.iter().all(|selected| !*selected) {
                    self.weekdays[weekday_index(anchor.weekday())] = true;
                }
            }
        }
    }

    /// Validates an interactive skip add, returning the error to show.
    pub fn add_skip(&mut self) -> Result<(), String> {
        let day = deadline::parse_date(self.skip_input.trim())?;
        if !self.skips.contains(&day) {
            self.skips.push(day);
            self.skips.sort_unstable();
        }
        self.skip_input.clear();
        Ok(())
    }

    /// Reads an imported recurrence into the structured fields. Returns false
    /// for anything the editor cannot show without RFC text, which then stays
    /// untouched as a custom schedule.
    pub fn apply(&mut self, anchor: NaiveDate, recurrence: &Recurrence) -> bool {
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
                    vec![anchor.weekday()]
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
                if bymonth
                    .first()
                    .is_some_and(|month| *month != anchor.month())
                    || bymonthday
                        .first()
                        .is_some_and(|day| *day != anchor.day() as i32)
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

    /// Builds the recurrence array the editor describes. The clock time is
    /// used for skipped and extra dates.
    pub fn lines_at(&self, anchor: NaiveDate, start: NaiveTime) -> Result<Vec<String>, String> {
        if self.repeat == RepeatMode::Custom {
            return Ok(self.custom.clone().unwrap_or_default());
        }

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
                    anchor.month(),
                    anchor.day()
                ),
                RepeatMode::Never | RepeatMode::Custom => unreachable!(),
            };
            match self.ends {
                EndKind::Never => {}
                EndKind::OnDate => {
                    let until = deadline::parse_date(self.until.trim())?;
                    if until < anchor {
                        return Err("The repeat cannot end before it starts".to_owned());
                    }
                    rule.push_str(&format!(";UNTIL={}", until_utc(until)));
                }
                EndKind::After => {
                    let count = self.count_value()?;
                    rule.push_str(&format!(";COUNT={count}"));
                }
            }
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

    /// A short, human description used by the cards and the panel.
    pub fn summary(&self, anchor: NaiveDate) -> String {
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
                let day = pretty_date(anchor);
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

pub fn weekday_flags(days: &[Weekday]) -> [bool; 7] {
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
