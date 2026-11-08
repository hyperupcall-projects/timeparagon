//! Habits, modelled on Loop Habit Tracker.
//!
//! A [`Habit`] is a goal ("every Monday", "2 times a week") with a day-by-day
//! record of whether it happened. It is independent of the calendar: any number
//! of habit events can point at one through their `ttHabit` property, and a
//! habit needs no event at all.
//!
//! Interval snapping, scores and streaks follow Loop's `EntryList`, `ScoreList`
//! and `StreakList`, so a record scores the same here as in Loop. Added to that:
//! a times-per-day goal, a goal on chosen weekdays, an "Avoided it" habit that is
//! a success on every day it is not done, recording how many times a day it was
//! done, skip days and question marks being set per habit instead of globally,
//! and habits being grouped into sections.
//!
//! A habit's [`Kind`] alone decides what counts as success. Any habit can also
//! record a number each day, with a target ([`Measure`]). That is kept for the
//! Value chart and never changes the score.

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, Months, NaiveDate, NaiveTime, Timelike, Weekday};
use serde::{Deserialize, Serialize};

/// The Google extended property naming the habit an event belongs to.
pub const LINK_PROPERTY: &str = "ttHabit";

/// What counts as success.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Doing it is the goal. Doing it again on the same day is noted, but the
    /// day counts the same.
    #[default]
    Yes,
    /// Never doing it is the goal, so every day it is not done is a success.
    No,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Yes => "Did it",
            Self::No => "Avoided it",
        }
    }
}

/// A number recorded alongside a habit, such as miles run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub target: Option<f64>,
    /// The period the target covers, and the Value chart adds up: 1, 7 or 30 days.
    #[serde(default = "one_day")]
    pub days: u32,
}

fn one_day() -> u32 {
    1
}

impl Measure {
    /// The period of the Value chart.
    pub fn bucket(&self) -> Bucket {
        match self.days {
            0 | 1 => Bucket::Day,
            2..=7 => Bucket::Week,
            _ => Bucket::Month,
        }
    }

    /// The target in words. Doing more of what is wanted is better, and doing
    /// more of what is being avoided is worse, so the side follows from the kind.
    pub fn label(&self, kind: Kind) -> String {
        let unit = if self.unit.is_empty() {
            String::new()
        } else {
            format!(" {}", self.unit)
        };
        let Some(target) = self.target else {
            return if self.unit.is_empty() {
                "Tracking a value".to_owned()
            } else {
                format!("Tracking{unit}")
            };
        };
        let side = match kind {
            Kind::Yes => "At least",
            Kind::No => "At most",
        };
        let period = match self.bucket() {
            Bucket::Day => "day",
            Bucket::Week => "week",
            _ => "month",
        };
        format!("{side} {}{unit} per {period}", format_amount(target))
    }
}

/// How a habit's days are drawn in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Icons {
    /// A check, a cross and a question mark.
    #[default]
    Symbols,
    /// A happy face, a disappointed face and a questioning face.
    Faces,
}

/// How often a habit is meant to happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frequency {
    /// `times` in every `days` days. Covers every day, every X days, X times
    /// per week (`days` 7), X times per month (`days` 30) and X times in Y days.
    Interval { times: u32, days: u32 },
    /// More than once on each day.
    TimesPerDay { times: u32 },
    /// On these weekdays, Monday first.
    Weekdays { days: [bool; 7] },
}

impl Default for Frequency {
    fn default() -> Self {
        Self::DAILY
    }
}

impl Frequency {
    pub const DAILY: Self = Self::Interval { times: 1, days: 1 };

    /// The goal as repetitions over days, which is what scores are based on.
    pub fn fraction(self) -> (u32, u32) {
        match self {
            Self::Interval { times, days } if times >= days => (1, 1),
            Self::Interval { times, days } => (times.max(1), days.max(1)),
            Self::TimesPerDay { .. } => (1, 1),
            Self::Weekdays { days } => {
                let count = days.iter().filter(|on| **on).count() as u32;
                if count >= 7 {
                    (1, 1)
                } else {
                    (count.max(1), 7)
                }
            }
        }
    }

    pub fn as_f64(self) -> f64 {
        let (times, days) = self.fraction();
        f64::from(times) / f64::from(days)
    }

    /// How many repetitions make one day complete.
    pub fn per_day(self) -> u32 {
        match self {
            Self::TimesPerDay { times } => times.max(1),
            _ => 1,
        }
    }

    /// The goal in the words Loop's habit screen uses.
    pub fn label(self) -> String {
        match self {
            Self::Interval { times, days } if times >= days => "Every day".to_owned(),
            Self::Interval { times: 1, days: 7 } => "Every week".to_owned(),
            Self::Interval { times: 1, days: 30 } => "Every month".to_owned(),
            Self::Interval { times: 1, days } => format!("Every {days} days"),
            Self::Interval { times, days: 7 } => format!("{times} times per week"),
            Self::Interval { times, days: 30 } => format!("{times} times per month"),
            Self::Interval { times, days } => format!("{times} times in {days} days"),
            Self::TimesPerDay { times } => format!("{times} times per day"),
            Self::Weekdays { days } => {
                const NAMES: [&str; 7] = [
                    "Monday",
                    "Tuesday",
                    "Wednesday",
                    "Thursday",
                    "Friday",
                    "Saturday",
                    "Sunday",
                ];
                let picked: Vec<&str> = NAMES
                    .iter()
                    .zip(days)
                    .filter(|(_, on)| *on)
                    .map(|(name, _)| *name)
                    .collect();
                match picked.as_slice() {
                    [] => "No days chosen".to_owned(),
                    [_, _, _, _, _, _, _] => "Every day".to_owned(),
                    [one] => format!("Every {one}"),
                    many => {
                        let short: Vec<&str> = many.iter().map(|name| &name[..3]).collect();
                        format!("Every {}", short.join(", "))
                    }
                }
            }
        }
    }

    /// Why this goal cannot be saved, if it cannot.
    pub fn validate(self) -> Result<(), String> {
        match self {
            Self::Interval { times: 0, .. } | Self::Interval { days: 0, .. } => {
                Err("A frequency needs numbers above zero".to_owned())
            }
            Self::TimesPerDay { times: 0 } => {
                Err("A frequency needs numbers above zero".to_owned())
            }
            Self::Weekdays { days } if !days.contains(&true) => {
                Err("Choose at least one day of the week".to_owned())
            }
            _ => Ok(()),
        }
    }
}

/// What was recorded for one day. A day with no record is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Yes,
    /// How many times it was done that day.
    Reps(u32),
    No,
    Skip,
}

/// What a day works out to, once the goal has been applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Done, by hand.
    Yes,
    /// Not done, but not expected either, as an earlier repetition covers it.
    Auto,
    /// Some but not all of a times-per-day goal.
    Partial(u32),
    /// Expected and not done.
    No,
    Skip,
    Unknown,
    /// On a "No" habit: not done, which is the goal.
    Clean,
    /// On a "No" habit: done this many times.
    Slipped(u32),
}

impl Status {
    /// What the day adds to the totals and charts.
    pub fn counted(self) -> f64 {
        match self {
            Self::Yes | Self::Clean => 1.0,
            _ => 0.0,
        }
    }

    /// Whether the day keeps a streak going.
    pub fn completed(self) -> bool {
        match self {
            Self::Yes | Self::Auto | Self::Skip | Self::Clean => true,
            Self::No | Self::Unknown | Self::Partial(_) | Self::Slipped(_) => false,
        }
    }
}

/// One Loop palette color, by index, so sorting by color matches Loop.
pub const PALETTE_SIZE: u8 = 20;
pub const DEFAULT_COLOR: u8 = 8;

/// Loop's light and dark palettes, as 0xRRGGBB.
const PALETTE: [(u32, u32); PALETTE_SIZE as usize] = [
    (0xD32F2F, 0xEF9A9A),
    (0xE64A19, 0xFFAB91),
    (0xF57C00, 0xFFCC80),
    (0xFF8F00, 0xFFECB3),
    (0xF9A825, 0xFFF59D),
    (0xAFB42B, 0xE6EE9C),
    (0x7CB342, 0xC5E1A5),
    (0x388E3C, 0x69F0AE),
    (0x00897B, 0x80CBC4),
    (0x00ACC1, 0x80DEEA),
    (0x039BE5, 0x81D4FA),
    (0x1976D2, 0x64B5F6),
    (0x303F9F, 0x9FA8DA),
    (0x5E35B1, 0xB39DDB),
    (0x8E24AA, 0xCE93D8),
    (0xD81B60, 0xF48FB1),
    (0x5D4037, 0xBCAAA4),
    (0x424242, 0xF5F5F5),
    (0x757575, 0xE0E0E0),
    (0x9E9E9E, 0x9E9E9E),
];

/// A palette color for a light or dark background.
pub fn palette_color(index: u8, dark: bool) -> crate::Color {
    let (light, night) = PALETTE[usize::from(index) % PALETTE.len()];
    let rgb = if dark { night } else { light };
    crate::Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

fn default_color() -> u8 {
    DEFAULT_COLOR
}

/// What was answered about a day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// It happened: a success on "Did it", a slip on "Avoided it".
    Done,
    /// It did not happen.
    NotDone,
    Skipped,
    /// Nothing is known, on a habit that shows question marks.
    Unanswered,
}

/// Everything that can be recorded for one day.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DayEntry {
    pub outcome: Outcome,
    /// How many times it happened, when it did.
    pub times: u32,
    /// How many times it was resisted, on an "Avoided it" habit.
    pub resisted: u32,
    pub value: Option<f64>,
}

/// One logged occurrence: when it happened and, on a habit that records a
/// number, how much (such as how many minutes it went on for), with a comment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "StoredLogEntry")]
pub struct LogEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<NaiveTime>,
    /// In the habit's unit, in thousandths, as `values` are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<i64>,
    /// Blank for no comment.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

impl LogEntry {
    pub fn new(at: Option<NaiveTime>, amount: Option<f64>) -> Self {
        Self {
            at,
            amount: amount.map(|amount| (amount.max(0.0) * 1000.0).round() as i64),
            note: String::new(),
        }
    }

    /// The entry with a comment.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = note.into();
        self
    }

    pub fn amount(&self) -> Option<f64> {
        self.amount.map(|amount| amount as f64 / 1000.0)
    }

    fn is_blank(&self) -> bool {
        self.at.is_none() && self.amount.is_none() && self.note.is_empty()
    }
}

/// The first logs kept only a time for each entry.
#[derive(Deserialize)]
#[serde(untagged)]
enum StoredLogEntry {
    Time(NaiveTime),
    Entry {
        #[serde(default)]
        at: Option<NaiveTime>,
        #[serde(default)]
        amount: Option<i64>,
        #[serde(default)]
        note: String,
    },
}

impl From<StoredLogEntry> for LogEntry {
    fn from(stored: StoredLogEntry) -> Self {
        match stored {
            StoredLogEntry::Time(at) => Self {
                at: Some(at),
                amount: None,
                note: String::new(),
            },
            StoredLogEntry::Entry { at, amount, note } => Self { at, amount, note },
        }
    }
}

/// A day's log: each time it happened (done, or slipped on a "No" habit) and
/// each time it was resisted. The counts in `marks` and `resisted` stay the
/// record; the log says when and how much. Counts from before entries were
/// logged have fewer entries than the count.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayLog {
    #[serde(default, alias = "slipped", skip_serializing_if = "Vec::is_empty")]
    pub happened: Vec<LogEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resisted: Vec<LogEntry>,
}

impl DayLog {
    fn is_empty(&self) -> bool {
        self.happened.is_empty() && self.resisted.is_empty()
    }
}

/// A day's `count` entries in order of time, blank ones (from before entries
/// were logged) first. More entries than the count keeps the latest.
fn log_entries(count: u32, logged: &[LogEntry]) -> Vec<LogEntry> {
    let count = count as usize;
    let mut logged = logged.to_vec();
    logged.sort_by_key(|entry| entry.at);
    let kept = logged.len().min(count);
    let mut entries = vec![LogEntry::default(); count - kept];
    entries.extend_from_slice(&logged[logged.len() - kept..]);
    entries
}

/// Entries as they are stored: in order of time, blank ones left out.
fn stored_entries(entries: &[LogEntry]) -> Vec<LogEntry> {
    let mut entries: Vec<LogEntry> = entries
        .iter()
        .filter(|entry| !entry.is_blank())
        .cloned()
        .collect();
    entries.sort_by_key(|entry| entry.at);
    entries
}

/// Reads a time of day the way people type one: `20:05`, `8:05pm`, `8.05p`,
/// `11p`, `2 PM`, `805p`.
pub fn parse_log_time(text: &str) -> Option<NaiveTime> {
    let text: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    let (clock, pm) = if let Some(rest) = text.strip_suffix("pm").or(text.strip_suffix('p')) {
        (rest, Some(true))
    } else if let Some(rest) = text.strip_suffix("am").or(text.strip_suffix('a')) {
        (rest, Some(false))
    } else {
        (text.as_str(), None)
    };
    let (hour, minute) = match clock.split_once([':', '.']) {
        Some((hour, minute)) if minute.len() == 2 => (hour, minute),
        Some(_) => return None,
        None if clock.len() > 2 => clock.split_at(clock.len() - 2),
        None => (clock, "0"),
    };
    if hour.is_empty() || !(hour.chars().chain(minute.chars())).all(|c| c.is_ascii_digit()) {
        return None;
    }
    let mut hour: u32 = hour.parse().ok()?;
    let minute: u32 = minute.parse().ok()?;
    match pm {
        Some(_) if !(1..=12).contains(&hour) => return None,
        Some(true) if hour != 12 => hour += 12,
        Some(false) if hour == 12 => hour = 0,
        _ => {}
    }
    NaiveTime::from_hms_opt(hour, minute, 0)
}

/// Reads an amount, allowing a unit after it: `6`, `2.5`, `12m`, `20 mins`.
pub fn parse_log_amount(text: &str) -> Option<f64> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(text.len());
    let (number, rest) = text.split_at(split);
    if !rest.chars().all(|c| c.is_alphabetic() || c.is_whitespace()) {
        return None;
    }
    number
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

/// A time of day to the second, for logging something as it happens.
pub fn log_time(time: NaiveTime) -> NaiveTime {
    time.with_nanosecond(0).unwrap_or(time)
}

/// One habit: its goal, its settings and everything recorded for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(from = "StoredHabit")]
pub struct Habit {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    /// The day it was added, from which a "No" habit starts counting.
    #[serde(default)]
    pub created: Option<NaiveDate>,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default = "default_color")]
    pub color: u8,
    #[serde(default)]
    pub frequency: Frequency,
    #[serde(default)]
    pub archived: bool,
    /// Clicking a done day offers "skipped" before "not done".
    #[serde(default)]
    pub skip_days: bool,
    /// Days with no record show a question mark rather than a cross.
    #[serde(default)]
    pub question_marks: bool,
    #[serde(default)]
    pub icons: Icons,
    #[serde(default)]
    pub marks: BTreeMap<NaiveDate, Mark>,
    /// On a "No" habit: how many times it came to mind on a day without being
    /// done. It is kept apart from `marks` so that it never changes the score.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resisted: BTreeMap<NaiveDate, u32>,
    /// On a "No" habit: when each slip and resisted urge was logged.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub log: BTreeMap<NaiveDate, DayLog>,
    /// The [`Section`] it is listed under, by id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// The number recorded each day, if this habit records one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measure: Option<Measure>,
    /// Each day's number in thousandths. It outlives `measure` being turned off.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<NaiveDate, i64>,
    /// The life area it belongs to, by id. Unset means the Personal area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area: Option<u64>,
}

/// A habit as any version has saved it.
#[derive(Deserialize)]
struct StoredHabit {
    id: String,
    name: String,
    #[serde(default)]
    kind: StoredKind,
    #[serde(default)]
    created: Option<NaiveDate>,
    #[serde(default)]
    unit: String,
    #[serde(default)]
    target: f64,
    #[serde(default)]
    target_type: StoredTargetType,
    #[serde(default)]
    question: String,
    #[serde(default)]
    notes: String,
    #[serde(default = "default_color")]
    color: u8,
    #[serde(default)]
    frequency: Frequency,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    skip_days: bool,
    #[serde(default)]
    question_marks: bool,
    #[serde(default)]
    icons: Icons,
    #[serde(default)]
    marks: BTreeMap<NaiveDate, StoredMark>,
    #[serde(default)]
    resisted: BTreeMap<NaiveDate, u32>,
    #[serde(default)]
    log: BTreeMap<NaiveDate, DayLog>,
    #[serde(default)]
    section: Option<String>,
    #[serde(default)]
    measure: Option<Measure>,
    #[serde(default)]
    values: BTreeMap<NaiveDate, i64>,
    #[serde(default)]
    area: Option<u64>,
}

#[derive(Deserialize, Default, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum StoredKind {
    #[default]
    Yes,
    No,
    Measurable,
}

#[derive(Deserialize, Default, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum StoredTargetType {
    #[default]
    AtLeast,
    AtMost,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum StoredMark {
    Yes,
    Reps(u32),
    No,
    Skip,
    Amount(i64),
}

impl From<StoredHabit> for Habit {
    /// Measurable used to be a third kind of habit, where an amount decided the
    /// day. It becomes whichever kind has the same goal, with its amounts kept as
    /// values, and each day judged against the target as it was recorded.
    fn from(stored: StoredHabit) -> Self {
        let measurable = stored.kind == StoredKind::Measurable;
        let at_most = measurable && stored.target_type == StoredTargetType::AtMost;
        let days = match stored.frequency.fraction().1 {
            0 | 1 => 1,
            2..=7 => 7,
            _ => 30,
        };
        let mut marks = BTreeMap::new();
        let mut values = stored.values;
        for (day, mark) in stored.marks {
            let mark = match mark {
                StoredMark::Yes => Some(Mark::Yes),
                StoredMark::Reps(times) => Some(Mark::Reps(times)),
                StoredMark::No => Some(Mark::No),
                StoredMark::Skip => Some(Mark::Skip),
                StoredMark::Amount(amount) => {
                    values.insert(day, amount);
                    let value = amount as f64 / 1000.0;
                    if at_most {
                        (value > stored.target / f64::from(days)).then_some(Mark::Reps(1))
                    } else {
                        // A longer period's target can be met over several days.
                        (value > 0.0 && (days != 1 || value >= stored.target)).then_some(Mark::Yes)
                    }
                }
            };
            if let Some(mark) = mark {
                marks.insert(day, mark);
            }
        }
        Self {
            id: stored.id,
            name: stored.name,
            kind: if at_most || stored.kind == StoredKind::No {
                Kind::No
            } else {
                Kind::Yes
            },
            created: stored.created,
            question: stored.question,
            notes: stored.notes,
            color: stored.color,
            frequency: if at_most {
                Frequency::DAILY
            } else {
                stored.frequency
            },
            archived: stored.archived,
            skip_days: stored.skip_days,
            question_marks: stored.question_marks,
            icons: stored.icons,
            marks,
            resisted: stored.resisted,
            log: stored.log,
            section: stored.section,
            measure: if measurable {
                Some(Measure {
                    unit: stored.unit,
                    target: (stored.target > 0.0).then_some(stored.target),
                    days,
                })
            } else {
                stored.measure
            },
            values,
            area: stored.area,
        }
    }
}

impl Habit {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind: Kind::Yes,
            created: None,
            question: String::new(),
            notes: String::new(),
            color: DEFAULT_COLOR,
            frequency: Frequency::DAILY,
            archived: false,
            skip_days: false,
            question_marks: false,
            icons: Icons::Symbols,
            marks: BTreeMap::new(),
            resisted: BTreeMap::new(),
            log: BTreeMap::new(),
            section: None,
            measure: None,
            values: BTreeMap::new(),
            area: None,
        }
    }

    /// Every day's status, its score and its streaks, as of `today`.
    pub fn compute(&self, today: NaiveDate) -> Computed {
        let (statuses, frequency) = match self.kind {
            Kind::Yes => (statuses(self.frequency, &self.marks), self.frequency),
            Kind::No => (abstain_statuses(self, today), Frequency::DAILY),
        };
        let from = first_day(&statuses, today).min(self.first_value().unwrap_or(today));
        let scores = scores(frequency, &statuses, from, today);
        let streaks = streaks(&statuses, today);
        let total = statuses.values().map(|status| status.counted()).sum();
        Computed {
            statuses,
            scores,
            from,
            streaks,
            total,
        }
    }

    /// How many times it was recorded as done on a day.
    pub fn times_on(&self, day: NaiveDate) -> u32 {
        self.marks
            .get(&day)
            .map_or(0, |mark| reps(*mark, self.frequency.per_day()))
    }

    /// How many times it was thought about but not done on a day.
    pub fn resisted_on(&self, day: NaiveDate) -> u32 {
        self.resisted.get(&day).copied().unwrap_or(0)
    }

    /// Records how many times it was thought about but not done on a day.
    pub fn set_resisted(&mut self, day: NaiveDate, times: u32) {
        if times == 0 {
            self.resisted.remove(&day);
        } else {
            self.resisted.insert(day, times);
        }
    }

    /// Each time it happened on a day (each slip, on a "No" habit), with when
    /// and how much if that was logged.
    pub fn happened_log_on(&self, day: NaiveDate) -> Vec<LogEntry> {
        let logged = self.log.get(&day).map_or(&[][..], |log| &log.happened[..]);
        log_entries(self.times_on(day), logged)
    }

    /// Each time it was resisted on a day, with when if that was logged.
    pub fn resisted_log_on(&self, day: NaiveDate) -> Vec<LogEntry> {
        let logged = self.log.get(&day).map_or(&[][..], |log| &log.resisted[..]);
        log_entries(self.resisted_on(day), logged)
    }

    /// Records a day's log. The counts are set by [`Habit::set_entry`]; blank
    /// entries are left out.
    pub fn set_log(&mut self, day: NaiveDate, happened: &[LogEntry], resisted: &[LogEntry]) {
        let log = DayLog {
            happened: stored_entries(happened),
            resisted: stored_entries(resisted),
        };
        if log.is_empty() {
            self.log.remove(&day);
        } else {
            self.log.insert(day, log);
        }
    }

    /// The number recorded on a day.
    pub fn value_on(&self, day: NaiveDate) -> Option<f64> {
        self.values.get(&day).map(|value| *value as f64 / 1000.0)
    }

    /// Records the number for a day, or removes it.
    pub fn set_value(&mut self, day: NaiveDate, value: Option<f64>) {
        match value {
            Some(value) => {
                self.values
                    .insert(day, (value.max(0.0) * 1000.0).round() as i64);
            }
            None => {
                self.values.remove(&day);
            }
        }
    }

    fn first_value(&self) -> Option<NaiveDate> {
        self.values.keys().next().copied()
    }

    /// Everything recorded for a day. With no answer on record, a day has not
    /// happened unless the habit asks for days to be answered.
    pub fn entry(&self, day: NaiveDate) -> DayEntry {
        let times = self.times_on(day);
        let resisted = self.resisted_on(day);
        DayEntry {
            outcome: match self.marks.get(&day) {
                Some(Mark::Skip) => Outcome::Skipped,
                _ if times > 0 => Outcome::Done,
                None if self.question_marks && resisted == 0 => Outcome::Unanswered,
                _ => Outcome::NotDone,
            },
            times,
            resisted,
            value: self.value_on(day),
        }
    }

    /// Replaces everything recorded for a day.
    pub fn set_entry(&mut self, day: NaiveDate, entry: DayEntry) {
        match entry.outcome {
            Outcome::Done => {
                self.marks.insert(day, Mark::Reps(entry.times.max(1)));
            }
            Outcome::Skipped => {
                self.marks.insert(day, Mark::Skip);
            }
            // Only a habit with question marks tells this apart from no answer.
            Outcome::NotDone if self.question_marks => {
                self.marks.insert(day, Mark::No);
            }
            Outcome::NotDone | Outcome::Unanswered => {
                self.marks.remove(&day);
            }
        }
        let resisted = if self.kind == Kind::No {
            entry.resisted
        } else {
            0
        };
        self.set_resisted(day, resisted);
        self.set_value(day, entry.value);
    }

    /// The goal, in words.
    pub fn goal_label(&self) -> String {
        match self.kind {
            Kind::Yes => self.frequency.label(),
            Kind::No => "Never".to_owned(),
        }
    }

    /// Loop's click cycle, with partial days added for a times-per-day goal,
    /// and turned around for a "No" habit.
    pub fn toggled(&self, status: Status) -> Option<Mark> {
        if self.kind == Kind::No {
            return match status {
                Status::Unknown if self.question_marks => Some(Mark::No),
                Status::Slipped(_) if self.skip_days => Some(Mark::Skip),
                Status::Slipped(_) | Status::Skip => None,
                _ => Some(Mark::Reps(1)),
            };
        }
        let goal = self.frequency.per_day();
        if goal > 1 {
            return match status {
                Status::No if self.question_marks => None,
                Status::Partial(reps) if reps + 1 >= goal => Some(Mark::Reps(goal)),
                Status::Partial(reps) => Some(Mark::Reps(reps + 1)),
                Status::Yes if self.skip_days => Some(Mark::Skip),
                Status::Yes | Status::Skip => Some(Mark::No),
                _ => Some(Mark::Reps(1)),
            };
        }
        match status {
            Status::Yes if self.skip_days => Some(Mark::Skip),
            Status::Yes | Status::Skip => Some(Mark::No),
            Status::No if self.question_marks => None,
            _ => Some(Mark::Yes),
        }
    }

    /// Advances a day one step through the click cycle.
    pub fn toggle(&mut self, day: NaiveDate, status: Status) {
        match self.toggled(status) {
            Some(mark) => {
                self.marks.insert(day, mark);
            }
            None => {
                self.marks.remove(&day);
            }
        }
    }

    /// Records one repetition from a completed calendar occurrence.
    pub fn add_rep(&mut self, day: NaiveDate) {
        let done = self.times_on(day);
        self.marks.insert(day, Mark::Reps(done + 1));
    }

    /// Takes back a repetition when a calendar occurrence is marked incomplete.
    pub fn remove_rep(&mut self, day: NaiveDate) {
        match self.times_on(day) {
            0 => {}
            1 => {
                self.marks.remove(&day);
            }
            done => {
                self.marks.insert(day, Mark::Reps(done - 1));
            }
        }
    }
}

fn first_day(statuses: &BTreeMap<NaiveDate, Status>, today: NaiveDate) -> NaiveDate {
    statuses.keys().next().copied().unwrap_or(today).min(today)
}

/// A number without the noise: `2.5`, `10`, `0.25`. Values are kept in thousandths,
/// so this shows all of what is stored.
pub fn format_amount(value: f64) -> String {
    let text = format!("{value:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn reps(mark: Mark, goal: u32) -> u32 {
    match mark {
        Mark::Yes => goal,
        Mark::Reps(reps) => reps,
        Mark::No | Mark::Skip => 0,
    }
}

/// A run of completed days, both ends included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Streak {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl Streak {
    pub fn length(self) -> i64 {
        (self.end - self.start).num_days() + 1
    }
}

/// A habit's record worked out for display.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Computed {
    statuses: BTreeMap<NaiveDate, Status>,
    /// One score per day, oldest first, starting at `from`.
    scores: Vec<f64>,
    from: NaiveDate,
    /// Newest first.
    streaks: Vec<Streak>,
    total: f64,
}

impl Computed {
    pub fn status(&self, day: NaiveDate) -> Status {
        self.statuses.get(&day).copied().unwrap_or(Status::Unknown)
    }

    /// Zero before the first record, as in Loop.
    pub fn score(&self, day: NaiveDate) -> f64 {
        let offset = (day - self.from).num_days();
        usize::try_from(offset)
            .ok()
            .and_then(|offset| self.scores.get(offset))
            .copied()
            .unwrap_or(0.0)
    }

    /// The oldest day with anything to show.
    pub fn first_day(&self) -> NaiveDate {
        self.from
    }

    /// The successful days, or the sum of a measurable habit's amounts. Doing
    /// it several times in one day still counts as one day.
    pub fn total(&self) -> f64 {
        self.total
    }

    /// The longest `limit` streaks, newest first.
    pub fn best_streaks(&self, limit: usize) -> Vec<Streak> {
        let mut best = self.streaks.clone();
        best.sort_by_key(|streak| {
            (
                std::cmp::Reverse(streak.length()),
                std::cmp::Reverse(streak.end),
            )
        });
        best.truncate(limit);
        best.sort_by_key(|streak| std::cmp::Reverse(streak.end));
        best
    }
}

/// Loop's period of grouping for the score and history cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    Day,
    #[default]
    Week,
    Month,
    Quarter,
    Year,
}

impl Bucket {
    /// Loop's score card offers every period; its history card drops days.
    pub const SCORE: [Self; 5] = [
        Self::Day,
        Self::Week,
        Self::Month,
        Self::Quarter,
        Self::Year,
    ];
    pub const HISTORY: [Self; 4] = [Self::Week, Self::Month, Self::Quarter, Self::Year];

    /// The first day of the period holding `day`.
    pub fn start(self, day: NaiveDate, first_weekday: Weekday) -> NaiveDate {
        match self {
            Self::Day => day,
            Self::Week => {
                let back = (7 + day.weekday().num_days_from_monday()
                    - first_weekday.num_days_from_monday())
                    % 7;
                day - Duration::days(i64::from(back))
            }
            Self::Month => day.with_day(1).unwrap_or(day),
            Self::Quarter => {
                let month = (day.month0() / 3) * 3 + 1;
                NaiveDate::from_ymd_opt(day.year(), month, 1).unwrap_or(day)
            }
            Self::Year => NaiveDate::from_ymd_opt(day.year(), 1, 1).unwrap_or(day),
        }
    }

    /// The period before the one starting on `start`.
    pub fn previous(self, start: NaiveDate) -> NaiveDate {
        match self {
            Self::Day => start - Duration::days(1),
            Self::Week => start - Duration::days(7),
            Self::Month => start.checked_sub_months(Months::new(1)).unwrap_or(start),
            Self::Quarter => start.checked_sub_months(Months::new(3)).unwrap_or(start),
            Self::Year => start.checked_sub_months(Months::new(12)).unwrap_or(start),
        }
    }
}

impl std::fmt::Display for Bucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Day => "Day",
            Self::Week => "Week",
            Self::Month => "Month",
            Self::Quarter => "Quarter",
            Self::Year => "Year",
        })
    }
}

/// Whatever was recorded in each period, newest first, with empty periods kept.
fn fill_periods(
    grouped: &BTreeMap<NaiveDate, f64>,
    bucket: Bucket,
    first_weekday: Weekday,
    today: NaiveDate,
) -> Vec<(NaiveDate, f64)> {
    let Some(oldest) = grouped.keys().next().copied() else {
        return Vec::new();
    };
    let mut series = Vec::new();
    let mut start = bucket.start(today, first_weekday);
    while start >= oldest {
        series.push((start, grouped.get(&start).copied().unwrap_or(0.0)));
        let previous = bucket.previous(start);
        if previous == start {
            break;
        }
        start = previous;
    }
    series
}

/// The values added up over each of the target's periods, newest first, for the
/// Value chart.
pub fn value_series(
    habit: &Habit,
    first_weekday: Weekday,
    today: NaiveDate,
) -> Vec<(NaiveDate, f64)> {
    let bucket = habit.measure.as_ref().map_or(Bucket::Day, Measure::bucket);
    let mut grouped: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for (day, value) in &habit.values {
        if *day <= today {
            *grouped
                .entry(bucket.start(*day, first_weekday))
                .or_default() += *value as f64 / 1000.0;
        }
    }
    fill_periods(&grouped, bucket, first_weekday, today)
}

/// The average score of each period, newest first, as Loop's score card shows.
pub fn score_series(
    computed: &Computed,
    bucket: Bucket,
    first_weekday: Weekday,
    today: NaiveDate,
) -> Vec<(NaiveDate, f64)> {
    let mut grouped: BTreeMap<NaiveDate, (f64, u32)> = BTreeMap::new();
    let mut day = computed.first_day();
    while day <= today {
        let slot = grouped.entry(bucket.start(day, first_weekday)).or_default();
        slot.0 += computed.score(day);
        slot.1 += 1;
        day += Duration::days(1);
    }
    grouped
        .into_iter()
        .rev()
        .map(|(start, (sum, count))| (start, sum / f64::from(count)))
        .collect()
}

/// What was counted in each period, newest first, as Loop's history card shows.
/// Periods with nothing in them still appear, so gaps read as gaps.
pub fn history_series(
    computed: &Computed,
    bucket: Bucket,
    first_weekday: Weekday,
    today: NaiveDate,
) -> Vec<(NaiveDate, f64)> {
    let mut grouped: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for (day, status) in &computed.statuses {
        if *day <= today {
            *grouped
                .entry(bucket.start(*day, first_weekday))
                .or_default() += status.counted();
        }
    }
    fill_periods(&grouped, bucket, first_weekday, today)
}

/// What was counted per weekday (Monday first) of each month, for the frequency
/// card.
pub fn weekday_frequency(computed: &Computed) -> BTreeMap<NaiveDate, [f64; 7]> {
    let mut months: BTreeMap<NaiveDate, [f64; 7]> = BTreeMap::new();
    for (day, status) in &computed.statuses {
        let month = day.with_day(1).unwrap_or(*day);
        let slot = day.weekday().num_days_from_monday() as usize;
        months.entry(month).or_default()[slot] += status.counted();
    }
    months
}

/// How many of each weekday (Monday first) a month has.
pub fn weekdays_in_month(month: NaiveDate) -> [u32; 7] {
    let mut counts = [0; 7];
    let mut day = month.with_day(1).unwrap_or(month);
    while day.month() == month.month() {
        counts[day.weekday().num_days_from_monday() as usize] += 1;
        day += Duration::days(1);
    }
    counts
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Interval {
    begin: NaiveDate,
    center: NaiveDate,
    end: NaiveDate,
}

fn month_length(day: NaiveDate) -> i64 {
    let first = day.with_day(1).unwrap_or(day);
    let next = first.checked_add_months(Months::new(1)).unwrap_or(first);
    (next - first).num_days()
}

/// Loop's `buildIntervals`. `done` is newest first.
fn build_intervals(times: u32, days: u32, done: &[NaiveDate]) -> Vec<Interval> {
    let times = times as usize;
    let mut intervals = Vec::new();
    for index in (times - 1)..done.len() {
        let begin = done[index];
        let center = done[index + 1 - times];
        let mut size = i64::from(days);
        if days == 30 || days == 31 {
            size = if begin.day() as i64 == month_length(begin) {
                month_length(begin + Duration::days(1))
            } else {
                month_length(begin)
            };
        }
        if (center - begin).num_days() < size {
            intervals.push(Interval {
                begin,
                center,
                end: begin + Duration::days(size - 1),
            });
        }
    }
    intervals
}

/// Loop's `snapIntervalsTogether`. `intervals` is newest first.
fn snap_intervals(intervals: &mut [Interval]) {
    for index in 1..intervals.len() {
        let current = intervals[index];
        let next = intervals[index - 1];
        let gap_next_to_current = (current.end - next.begin).num_days();
        let gap_center_to_end = (current.end - current.center).num_days();
        if gap_next_to_current >= 0 {
            let shift = Duration::days(gap_center_to_end.min(gap_next_to_current + 1));
            intervals[index] = Interval {
                begin: current.begin - shift,
                center: current.center,
                end: current.end - shift,
            };
        }
    }
}

/// A chosen-weekdays goal has no slack to slide: a repetition covers the days up
/// to the next chosen weekday, and a missed one is plainly missed.
fn weekday_intervals(chosen: [bool; 7], done: &[NaiveDate]) -> Vec<Interval> {
    done.iter()
        .map(|begin| {
            let mut end = *begin;
            for _ in 0..6 {
                let next = end + Duration::days(1);
                if chosen[next.weekday().num_days_from_monday() as usize] {
                    break;
                }
                end = next;
            }
            Interval {
                begin: *begin,
                center: *begin,
                end,
            }
        })
        .collect()
}

/// Loop's `recomputeFrom` and `buildEntriesFromInterval`.
fn statuses(
    frequency: Frequency,
    marks: &BTreeMap<NaiveDate, Mark>,
) -> BTreeMap<NaiveDate, Status> {
    let goal = frequency.per_day();
    let mut result = BTreeMap::new();

    if goal > 1 {
        for (day, mark) in marks {
            result.insert(
                *day,
                match *mark {
                    Mark::Yes => Status::Yes,
                    Mark::Reps(reps) if reps >= goal => Status::Yes,
                    Mark::Reps(reps) => Status::Partial(reps),
                    Mark::No => Status::No,
                    Mark::Skip => Status::Skip,
                },
            );
        }
        return result;
    }

    let done: Vec<NaiveDate> = marks
        .iter()
        .rev()
        .filter(|(_, mark)| matches!(mark, Mark::Yes | Mark::Reps(1..)))
        .map(|(day, _)| *day)
        .collect();
    let mut intervals = match frequency {
        Frequency::Weekdays { days } if frequency.fraction() != (1, 1) => {
            weekday_intervals(days, &done)
        }
        _ => {
            let (times, days) = frequency.fraction();
            let mut intervals = build_intervals(times, days, &done);
            snap_intervals(&mut intervals);
            intervals
        }
    };
    intervals.sort_by_key(|interval| std::cmp::Reverse(interval.begin));

    for interval in &intervals {
        let mut day = interval.begin;
        while day <= interval.end {
            result.insert(day, Status::Auto);
            day += Duration::days(1);
        }
    }
    for (day, mark) in marks {
        match *mark {
            Mark::Skip => {
                result.insert(*day, Status::Skip);
            }
            Mark::Yes | Mark::Reps(1..) => {
                result.insert(*day, Status::Yes);
            }
            Mark::No | Mark::Reps(0) => {
                // A "no" inside a covered stretch stays covered, as in Loop.
                result.entry(*day).or_insert(Status::No);
            }
        }
    }
    result
}

/// A "No" habit: every day since it began is a success unless it was done. With
/// question marks on, only a day said to be clean counts, which having resisted
/// it does say.
fn abstain_statuses(habit: &Habit, today: NaiveDate) -> BTreeMap<NaiveDate, Status> {
    let oldest = [
        habit.marks.keys().next(),
        habit.resisted.keys().next(),
        habit.values.keys().next(),
    ]
    .into_iter()
    .flatten()
    .min()
    .copied();
    let from = match (habit.created, oldest) {
        (Some(created), Some(oldest)) => created.min(oldest),
        (Some(day), None) | (None, Some(day)) => day,
        (None, None) => today,
    };
    let mut result = BTreeMap::new();
    let mut day = from;
    while day <= today {
        let status = match habit.marks.get(&day) {
            Some(Mark::Skip) => Status::Skip,
            Some(Mark::Yes) => Status::Slipped(1),
            Some(Mark::Reps(times @ 1..)) => Status::Slipped(*times),
            Some(_) => Status::Clean,
            None if habit.question_marks && habit.resisted_on(day) == 0 => Status::Unknown,
            None => Status::Clean,
        };
        if status != Status::Unknown {
            result.insert(day, status);
        }
        day += Duration::days(1);
    }
    result
}

/// Loop's `Score.compute`.
pub fn compute_score(frequency: f64, previous: f64, value: f64) -> f64 {
    let multiplier = 0.5_f64.powf(frequency.sqrt() / 13.0);
    previous * multiplier + value * (1.0 - multiplier)
}

/// Loop's `ScoreList.recompute`, where a day's credit may be a fraction for a
/// partly met times-per-day goal.
fn scores(
    frequency: Frequency,
    statuses: &BTreeMap<NaiveDate, Status>,
    from: NaiveDate,
    today: NaiveDate,
) -> Vec<f64> {
    let (mut numerator, mut denominator) = frequency.fraction();
    let freq = frequency.as_f64();
    // Doubling smooths out irregular schedules, such as a weekly habit done on
    // a different day each week.
    if freq < 1.0 {
        numerator *= 2;
        denominator *= 2;
    }
    let goal = f64::from(frequency.per_day());

    let mut scores = Vec::new();
    let mut window = std::collections::VecDeque::new();
    let mut rolling = 0.0;
    let mut previous = 0.0;
    let mut day = from;
    while day <= today {
        let status = statuses.get(&day).copied().unwrap_or(Status::Unknown);
        if status != Status::Skip {
            let credit = match status {
                Status::Yes | Status::Clean => 1.0,
                Status::Partial(reps) => f64::from(reps) / goal,
                _ => 0.0,
            };
            window.push_back(credit);
            rolling += credit;
            if window.len() > denominator as usize {
                rolling -= window.pop_front().unwrap_or(0.0);
            }
            let completed = (rolling / f64::from(numerator)).min(1.0);
            previous = compute_score(freq, previous, completed);
        }
        scores.push(previous);
        day += Duration::days(1);
    }
    scores
}

/// Loop's `StreakList.recompute`, newest first.
fn streaks(statuses: &BTreeMap<NaiveDate, Status>, today: NaiveDate) -> Vec<Streak> {
    let mut list: Vec<Streak> = Vec::new();
    for (day, status) in statuses.iter().rev() {
        if *day > today || !status.completed() {
            continue;
        }
        match list.last_mut() {
            Some(streak) if streak.start - Duration::days(1) == *day => streak.start = *day,
            _ => list.push(Streak {
                start: *day,
                end: *day,
            }),
        }
    }
    list
}

/// How the habit list is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    #[default]
    Manual,
    Name,
    Color,
    Score,
    Status,
}

impl SortKey {
    pub const ALL: [Self; 5] = [
        Self::Manual,
        Self::Name,
        Self::Color,
        Self::Score,
        Self::Status,
    ];
}

impl std::fmt::Display for SortKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Manual => "Manually",
            Self::Name => "By name",
            Self::Color => "By color",
            Self::Score => "By score",
            Self::Status => "By status",
        })
    }
}

/// A named group in the habit list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub collapsed: bool,
}

/// Everything saved between runs. The order of `habits` is the manual order, and
/// the order of `sections` is the order they are listed in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub habits: Vec<Habit>,
    pub sections: Vec<Section>,
    pub sort: SortKey,
    /// Choosing the current sort again reverses it, as in Loop.
    pub reversed: bool,
    pub hide_archived: bool,
    pub hide_completed: bool,
    pub score_bucket: Bucket,
    pub history_bucket: Bucket,
    /// Habit events that predate habits, by series id, and the habit each was
    /// given. An empty value means the event has been let go of, and it must
    /// not be claimed again.
    pub adopted: BTreeMap<String, String>,
}

impl Default for Library {
    fn default() -> Self {
        Self {
            habits: Vec::new(),
            sections: Vec::new(),
            sort: SortKey::Manual,
            reversed: false,
            hide_archived: true,
            hide_completed: false,
            score_bucket: Bucket::Week,
            history_bucket: Bucket::Month,
            adopted: BTreeMap::new(),
        }
    }
}

impl Library {
    pub fn habit(&self, id: &str) -> Option<&Habit> {
        self.habits.iter().find(|habit| habit.id == id)
    }

    pub fn habit_mut(&mut self, id: &str) -> Option<&mut Habit> {
        self.habits.iter_mut().find(|habit| habit.id == id)
    }

    /// Selects a sort. The current one reverses instead, except the manual order.
    pub fn choose_sort(&mut self, key: SortKey) {
        if key != SortKey::Manual && self.sort == key {
            self.reversed = !self.reversed;
        } else {
            self.sort = key;
            self.reversed = false;
        }
    }

    /// Moves a habit to where another one is in the manual order, and so into
    /// that one's section.
    pub fn move_habit(&mut self, id: &str, onto: &str) {
        let from = self.habits.iter().position(|habit| habit.id == id);
        let to = self.habits.iter().position(|habit| habit.id == onto);
        if let (Some(from), Some(to)) = (from, to)
            && from != to
        {
            let section = self.habits[to].section.clone();
            let mut habit = self.habits.remove(from);
            habit.section = section;
            self.habits.insert(to, habit);
        }
    }

    /// Moves a habit to the top of a section.
    pub fn move_to_section(&mut self, id: &str, section: &str) {
        let Some(from) = self.habits.iter().position(|habit| habit.id == id) else {
            return;
        };
        let mut habit = self.habits.remove(from);
        habit.section = Some(section.to_owned());
        let to = self
            .habits
            .iter()
            .position(|other| other.section.as_deref() == Some(section))
            .unwrap_or(from);
        self.habits.insert(to, habit);
    }

    pub fn section_mut(&mut self, id: &str) -> Option<&mut Section> {
        self.sections.iter_mut().find(|section| section.id == id)
    }

    /// Moves a section one place up or down the list.
    pub fn shift_section(&mut self, id: &str, up: bool) {
        let Some(index) = self.sections.iter().position(|section| section.id == id) else {
            return;
        };
        let other = if up {
            index.checked_sub(1)
        } else {
            Some(index + 1).filter(|next| *next < self.sections.len())
        };
        if let Some(other) = other {
            self.sections.swap(index, other);
        }
    }

    /// Deletes a section. Its habits stay, outside of any section.
    pub fn remove_section(&mut self, id: &str) {
        self.sections.retain(|section| section.id != id);
        for habit in &mut self.habits {
            if habit.section.as_deref() == Some(id) {
                habit.section = None;
            }
        }
    }

    /// Lays habits from [`Self::visible`] out under their sections, keeping their
    /// order. Those in no section come first, without a heading, as do any whose
    /// section is gone. A section whose habits are all filtered out is left out,
    /// while one with no habits at all stays, so that it can be filled.
    pub fn grouped<'a>(
        &'a self,
        visible: Vec<&'a Habit>,
    ) -> Vec<(Option<&'a Section>, Vec<&'a Habit>)> {
        let known = |habit: &Habit| {
            habit
                .section
                .as_deref()
                .filter(|id| self.sections.iter().any(|section| section.id == *id))
                .map(str::to_owned)
        };
        let (outside, inside): (Vec<&Habit>, Vec<&Habit>) = visible
            .into_iter()
            .partition(|habit| known(habit).is_none());
        let mut groups = Vec::new();
        if !outside.is_empty() {
            groups.push((None, outside));
        }
        for section in &self.sections {
            let listed: Vec<&Habit> = inside
                .iter()
                .copied()
                .filter(|habit| habit.section.as_deref() == Some(section.id.as_str()))
                .collect();
            let empty = !self
                .habits
                .iter()
                .any(|habit| habit.section.as_deref() == Some(section.id.as_str()));
            if !listed.is_empty() || empty {
                groups.push((Some(section), listed));
            }
        }
        groups
    }

    /// The habit that uses a name, ignoring case and surrounding spaces.
    pub fn named(&self, name: &str) -> Option<&Habit> {
        let name = name.trim().to_lowercase();
        self.habits
            .iter()
            .find(|habit| habit.name.trim().to_lowercase() == name)
    }

    /// The visible habits in display order, given each one's computed record.
    pub fn visible<'a>(
        &'a self,
        computed: &impl Fn(&str) -> Option<&'a Computed>,
        today: NaiveDate,
    ) -> Vec<&'a Habit> {
        let status = |habit: &Habit| {
            computed(&habit.id).map_or(Status::Unknown, |computed| computed.status(today))
        };
        let score =
            |habit: &Habit| computed(&habit.id).map_or(0.0, |computed| computed.score(today));
        let mut habits: Vec<&Habit> = self
            .habits
            .iter()
            .filter(|habit| !(self.hide_archived && habit.archived))
            .filter(|habit| !(self.hide_completed && status(habit).completed()))
            .collect();
        match self.sort {
            SortKey::Manual => {}
            SortKey::Name => {
                habits.sort_by_key(|habit| habit.name.to_lowercase());
            }
            SortKey::Color => habits.sort_by_key(|habit| habit.color),
            // Best first, as Loop's descending default.
            SortKey::Score => habits.sort_by(|a, b| score(b).total_cmp(&score(a))),
            SortKey::Status => habits.sort_by_key(|habit| status_rank(status(habit))),
        }
        if self.reversed && self.sort != SortKey::Manual {
            habits.reverse();
        }
        habits
    }
}

/// Done first, then whatever is still open, as Loop's status order.
fn status_rank(status: Status) -> u8 {
    match status {
        Status::Yes | Status::Clean => 0,
        Status::Auto => 1,
        Status::Skip => 2,
        Status::Partial(_) => 3,
        Status::No | Status::Slipped(_) => 4,
        Status::Unknown => 5,
    }
}

// ------------------------------------------------------------------- export

fn csv_line(fields: &[&str]) -> String {
    let quoted: Vec<String> = fields
        .iter()
        .map(|field| {
            if field.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", field.replace('"', "\"\""))
            } else {
                (*field).to_owned()
            }
        })
        .collect();
    format!("{}\n", quoted.join(","))
}

fn status_text(status: Status) -> String {
    match status {
        Status::Yes => "YES_MANUAL".to_owned(),
        Status::Auto => "YES_AUTO".to_owned(),
        Status::Partial(reps) => reps.to_string(),
        Status::Clean => "CLEAN".to_owned(),
        Status::Slipped(_) => "SLIPPED".to_owned(),
        Status::No => "NO".to_owned(),
        Status::Skip => "SKIP".to_owned(),
        Status::Unknown => "UNKNOWN".to_owned(),
    }
}

/// The folder name Loop gives a habit: its position, then its name made safe.
fn folder_name(position: usize, name: &str) -> String {
    let safe: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-'))
        .take(100)
        .collect();
    format!("{:03} {}/", position + 1, safe.trim())
}

fn habit_list_csv(habits: &[&Habit], sections: &[Section]) -> String {
    let mut out = csv_line(&[
        "Position",
        "Name",
        "Type",
        "Section",
        "Unit",
        "Target Type",
        "Target Value",
        "Target Days",
        "Question",
        "Description",
        "FrequencyNumerator",
        "FrequencyDenominator",
        "Frequency",
        "Color",
        "Skip days?",
        "Question marks?",
        "Archived?",
    ]);
    for (position, habit) in habits.iter().enumerate() {
        let (numerator, denominator) = habit.frequency.fraction();
        let section = sections
            .iter()
            .find(|section| Some(section.id.as_str()) == habit.section.as_deref())
            .map_or("", |section| section.name.as_str());
        let measure = habit.measure.as_ref();
        let target = measure.and_then(|measure| measure.target);
        out += &csv_line(&[
            &format!("{:03}", position + 1),
            &habit.name,
            match habit.kind {
                Kind::Yes => "DID_IT",
                Kind::No => "AVOIDED_IT",
            },
            section,
            measure.map_or("", |measure| measure.unit.as_str()),
            match (target, habit.kind) {
                (None, _) => "",
                (Some(_), Kind::Yes) => "AT_LEAST",
                (Some(_), Kind::No) => "AT_MOST",
            },
            &target.map_or(String::new(), format_amount),
            &measure.map_or(String::new(), |measure| measure.days.to_string()),
            &habit.question,
            &habit.notes,
            &numerator.to_string(),
            &denominator.to_string(),
            &habit.frequency.label(),
            &habit.color.to_string(),
            &habit.skip_days.to_string(),
            &habit.question_marks.to_string(),
            &habit.archived.to_string(),
        ]);
    }
    out
}

/// One row per day and one column per habit, newest day first.
fn day_grid(
    habits: &[&Habit],
    oldest: NaiveDate,
    today: NaiveDate,
    cell: impl Fn(usize, &Habit, NaiveDate) -> String,
) -> String {
    let mut header = vec!["Date"];
    header.extend(habits.iter().map(|habit| habit.name.as_str()));
    let mut out = csv_line(&header);
    let mut day = today;
    while day >= oldest {
        let date = day.to_string();
        let cells: Vec<String> = habits
            .iter()
            .enumerate()
            .map(|(index, habit)| cell(index, habit, day))
            .collect();
        let mut fields = vec![date.as_str()];
        fields.extend(cells.iter().map(String::as_str));
        out += &csv_line(&fields);
        day -= Duration::days(1);
    }
    out
}

fn value_cell(habit: &Habit, day: NaiveDate) -> String {
    habit.value_on(day).map_or(String::new(), format_amount)
}

/// A CSV archive laid out as Loop's export is: a `Habits.csv` list, a folder per
/// habit, and `Scores.csv` and `Checkmarks.csv` with a column per habit. Added to
/// that are `Values.csv` and `Resisted.csv`, for the habits that have any.
pub fn export_archive(habits: &[&Habit], sections: &[Section], today: NaiveDate) -> Vec<u8> {
    let computed: Vec<Computed> = habits.iter().map(|habit| habit.compute(today)).collect();
    let mut files = vec![("Habits.csv".to_owned(), habit_list_csv(habits, sections))];

    for (position, (habit, computed)) in habits.iter().zip(&computed).enumerate() {
        let folder = folder_name(position, &habit.name);
        let mut scores = csv_line(&["Date", "Score"]);
        let mut checks = csv_line(&["Date", "Status", "Times", "Resisted", "Value"]);
        let mut day = today;
        while day >= computed.first_day() {
            let date = day.to_string();
            scores += &csv_line(&[&date, &format!("{:.4}", computed.score(day))]);
            checks += &csv_line(&[
                &date,
                &status_text(computed.status(day)),
                &habit.times_on(day).to_string(),
                &habit.resisted_on(day).to_string(),
                &value_cell(habit, day),
            ]);
            day -= Duration::days(1);
        }
        files.push((format!("{folder}Scores.csv"), scores));
        files.push((format!("{folder}Checkmarks.csv"), checks));
    }

    let oldest = computed
        .iter()
        .map(Computed::first_day)
        .min()
        .unwrap_or(today);
    files.push((
        "Scores.csv".to_owned(),
        day_grid(habits, oldest, today, |index, _, day| {
            format!("{:.4}", computed[index].score(day))
        }),
    ));
    files.push((
        "Checkmarks.csv".to_owned(),
        day_grid(habits, oldest, today, |index, _, day| {
            status_text(computed[index].status(day))
        }),
    ));
    let measured: Vec<&Habit> = habits
        .iter()
        .copied()
        .filter(|habit| habit.measure.is_some() || !habit.values.is_empty())
        .collect();
    if !measured.is_empty() {
        files.push((
            "Values.csv".to_owned(),
            day_grid(&measured, oldest, today, |_, habit, day| {
                value_cell(habit, day)
            }),
        ));
    }
    let resisting: Vec<&Habit> = habits
        .iter()
        .copied()
        .filter(|habit| !habit.resisted.is_empty())
        .collect();
    if !resisting.is_empty() {
        files.push((
            "Resisted.csv".to_owned(),
            day_grid(&resisting, oldest, today, |_, habit, day| {
                habit.resisted_on(day).to_string()
            }),
        ));
    }
    zip(&files)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (!(crc & 1)).wrapping_add(1));
        }
    }
    !crc
}

/// A ZIP archive of uncompressed files. CSVs this size do not need compressing,
/// which saves a dependency.
fn zip(files: &[(String, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut directory = Vec::new();
    for (name, contents) in files {
        let offset = out.len() as u32;
        let crc = crc32(contents.as_bytes());
        let size = contents.len() as u32;
        let mut shared = Vec::new();
        shared.extend(20_u16.to_le_bytes()); // version needed
        shared.extend(0x0800_u16.to_le_bytes()); // names are UTF-8
        shared.extend(0_u16.to_le_bytes()); // stored
        shared.extend(0_u16.to_le_bytes()); // time
        shared.extend(0x21_u16.to_le_bytes()); // date: 1980-01-01
        shared.extend(crc.to_le_bytes());
        shared.extend(size.to_le_bytes());
        shared.extend(size.to_le_bytes());
        shared.extend((name.len() as u16).to_le_bytes());
        shared.extend(0_u16.to_le_bytes()); // extra length

        out.extend(0x0403_4b50_u32.to_le_bytes());
        out.extend(&shared);
        out.extend(name.as_bytes());
        out.extend(contents.as_bytes());

        directory.extend(0x0201_4b50_u32.to_le_bytes());
        directory.extend(20_u16.to_le_bytes()); // version made by
        directory.extend(&shared);
        directory.extend([0; 10]); // comment, disk, attributes
        directory.extend(offset.to_le_bytes());
        directory.extend(name.as_bytes());
    }
    let start = out.len() as u32;
    let length = directory.len() as u32;
    out.extend(directory);
    out.extend(0x0605_4b50_u32.to_le_bytes());
    out.extend([0; 4]); // disk numbers
    out.extend((files.len() as u16).to_le_bytes());
    out.extend((files.len() as u16).to_le_bytes());
    out.extend(length.to_le_bytes());
    out.extend(start.to_le_bytes());
    out.extend(0_u16.to_le_bytes()); // comment length
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f64 = 0.00002;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2015, 1, 25).unwrap()
    }

    fn ago(days: i64) -> NaiveDate {
        today() - Duration::days(days)
    }

    fn habit(frequency: Frequency) -> Habit {
        let mut habit = Habit::new("h", "Habit");
        habit.frequency = frequency;
        habit
    }

    fn check(habit: &mut Habit, from: i64, to: i64) {
        for days in from..to {
            habit.marks.insert(ago(days), Mark::Yes);
        }
    }

    fn assert_scores(habit: &Habit, expected: &[f64]) {
        let computed = habit.compute(today());
        for (days, value) in expected.iter().enumerate() {
            let actual = computed.score(ago(days as i64));
            assert!(
                (actual - value).abs() < EPSILON,
                "{days} days ago: expected {value}, got {actual}"
            );
        }
    }

    // The next four are Loop's own ScoreTest and ScoreListTest vectors.
    #[test]
    fn a_daily_score_step_matches_loop() {
        for (previous, check, expected) in [
            (0.0, 1.0, 0.051922),
            (0.5, 1.0, 0.525961),
            (0.75, 1.0, 0.762981),
            (0.0, 0.0, 0.0),
            (0.5, 0.0, 0.474039),
            (0.75, 0.0, 0.711058),
        ] {
            assert!((compute_score(1.0, previous, check) - expected).abs() < EPSILON);
        }
    }

    #[test]
    fn a_weekly_score_step_matches_loop() {
        let weekly = 1.0 / 3.0;
        for (previous, check, expected) in [
            (0.0, 1.0, 0.030314),
            (0.5, 1.0, 0.515157),
            (0.75, 1.0, 0.757578),
            (0.5, 0.0, 0.484842),
            (0.75, 0.0, 0.727263),
        ] {
            assert!((compute_score(weekly, previous, check) - expected).abs() < EPSILON);
        }
    }

    #[test]
    fn scores_over_a_run_match_loop() {
        let mut habit = habit(Frequency::DAILY);
        check(&mut habit, 0, 20);
        assert_scores(
            &habit,
            &[
                0.655747, 0.636894, 0.617008, 0.596033, 0.573910, 0.550574, 0.525961, 0.500000,
                0.472617, 0.443734, 0.413270, 0.381137, 0.347244, 0.311495, 0.273788, 0.234017,
                0.192067, 0.147820, 0.101149, 0.051922, 0.0, 0.0, 0.0,
            ],
        );
    }

    #[test]
    fn skipped_days_hold_the_score_as_in_loop() {
        let mut habit = habit(Frequency::DAILY);
        check(&mut habit, 0, 20);
        for days in [5, 10, 11] {
            habit.marks.insert(ago(days), Mark::Skip);
        }
        assert_scores(
            &habit,
            &[
                0.596033, 0.573910, 0.550574, 0.525961, 0.500000, 0.472617, 0.472617, 0.443734,
                0.413270, 0.381137, 0.347244, 0.347244, 0.347244, 0.311495, 0.273788, 0.234017,
                0.192067, 0.147820, 0.101149, 0.051922, 0.0, 0.0, 0.0,
            ],
        );
    }

    #[test]
    fn a_missing_day_decays_the_score_as_in_loop() {
        let mut habit = habit(Frequency::DAILY);
        habit.marks.insert(ago(5), Mark::Yes);
        habit.marks.insert(ago(4), Mark::Skip);
        assert_scores(
            &habit,
            &[
                0.041949, 0.044247, 0.046670, 0.049226, 0.051922, 0.051922, 0.0,
            ],
        );
    }

    // Loop's EntryListTest.testComputeBoolean.
    #[test]
    fn a_repetition_covers_the_days_after_it() {
        let mut habit = habit(Frequency::Interval { times: 1, days: 3 });
        for days in [4, 9, 10] {
            habit.marks.insert(ago(days), Mark::Yes);
        }
        let computed = habit.compute(today());
        for (days, expected) in [
            (1, Status::Unknown),
            (2, Status::Auto),
            (3, Status::Auto),
            (4, Status::Yes),
            (5, Status::Unknown),
            (6, Status::Unknown),
            (7, Status::Auto),
            (8, Status::Auto),
            (9, Status::Yes),
            (10, Status::Yes),
            (11, Status::Auto),
            (12, Status::Auto),
            (13, Status::Unknown),
        ] {
            assert_eq!(computed.status(ago(days)), expected, "{days} days ago");
        }
    }

    #[test]
    fn times_per_week_covers_the_week_once_met() {
        let mut habit = habit(Frequency::Interval { times: 2, days: 7 });
        habit.marks.insert(ago(6), Mark::Yes);
        habit.marks.insert(ago(4), Mark::Yes);
        let computed = habit.compute(today());
        assert_eq!(computed.status(ago(5)), Status::Auto);
        assert_eq!(computed.status(ago(0)), Status::Auto);
        assert_eq!(computed.status(ago(4)), Status::Yes);
        // One repetition alone is not enough to cover anything.
        let mut once = self::habit(Frequency::Interval { times: 2, days: 7 });
        once.marks.insert(ago(6), Mark::Yes);
        assert_eq!(once.compute(today()).status(ago(5)), Status::Unknown);
    }

    #[test]
    fn a_weekday_goal_covers_days_until_the_next_chosen_one() {
        // 2015-01-25 is a Sunday, so these are the two Mondays before it.
        let mut days = [false; 7];
        days[0] = true;
        let mut habit = habit(Frequency::Weekdays { days });
        habit.marks.insert(ago(13), Mark::Yes);
        let computed = habit.compute(today());
        assert_eq!(computed.status(ago(13)), Status::Yes);
        assert_eq!(computed.status(ago(7)), Status::Auto);
        assert_eq!(
            computed.status(ago(6)),
            Status::Unknown,
            "that Monday was missed"
        );
        assert_eq!(habit.frequency.label(), "Every Monday");
        assert_eq!(habit.frequency.fraction(), (1, 7));
    }

    #[test]
    fn a_times_per_day_goal_needs_every_repetition() {
        let mut habit = habit(Frequency::TimesPerDay { times: 3 });
        habit.marks.insert(ago(1), Mark::Reps(1));
        habit.marks.insert(ago(0), Mark::Yes);
        let computed = habit.compute(today());
        assert_eq!(computed.status(ago(1)), Status::Partial(1));
        assert!(!computed.status(ago(1)).completed());
        assert_eq!(computed.status(ago(0)), Status::Yes);
        assert_eq!(computed.total(), 1.0, "only the finished day counts");
        // A third of a day earns a third of the credit.
        let third = compute_score(1.0, 0.0, 1.0 / 3.0);
        assert!((computed.score(ago(1)) - third).abs() < EPSILON);
    }

    #[test]
    fn clicking_cycles_through_loops_states() {
        let mut plain = habit(Frequency::DAILY);
        assert_eq!(plain.toggled(Status::Unknown), Some(Mark::Yes));
        assert_eq!(plain.toggled(Status::Auto), Some(Mark::Yes));
        assert_eq!(plain.toggled(Status::Yes), Some(Mark::No));
        assert_eq!(plain.toggled(Status::No), Some(Mark::Yes));

        plain.skip_days = true;
        assert_eq!(plain.toggled(Status::Yes), Some(Mark::Skip));
        assert_eq!(plain.toggled(Status::Skip), Some(Mark::No));

        plain.question_marks = true;
        assert_eq!(plain.toggled(Status::No), None, "back to no data");
        assert_eq!(plain.toggled(Status::Unknown), Some(Mark::Yes));
    }

    #[test]
    fn clicking_a_times_per_day_goal_counts_up_to_done() {
        let habit = habit(Frequency::TimesPerDay { times: 3 });
        assert_eq!(habit.toggled(Status::Unknown), Some(Mark::Reps(1)));
        assert_eq!(habit.toggled(Status::Partial(1)), Some(Mark::Reps(2)));
        assert_eq!(habit.toggled(Status::Partial(2)), Some(Mark::Reps(3)));
        assert_eq!(habit.toggled(Status::Yes), Some(Mark::No));
    }

    #[test]
    fn calendar_occurrences_add_and_remove_repetitions() {
        let mut habit = habit(Frequency::TimesPerDay { times: 2 });
        habit.add_rep(today());
        assert_eq!(habit.marks[&today()], Mark::Reps(1));
        habit.add_rep(today());
        assert_eq!(habit.compute(today()).status(today()), Status::Yes);
        habit.remove_rep(today());
        assert_eq!(habit.marks[&today()], Mark::Reps(1));
        habit.remove_rep(today());
        assert!(!habit.marks.contains_key(&today()));
    }

    #[test]
    fn streaks_are_runs_of_completed_days_with_the_best_newest_first() {
        let mut habit = habit(Frequency::DAILY);
        check(&mut habit, 0, 2);
        check(&mut habit, 3, 8);
        check(&mut habit, 9, 10);
        habit.marks.insert(ago(11), Mark::Skip);
        let best = habit.compute(today()).best_streaks(2);
        assert_eq!(best.len(), 2);
        assert_eq!((best[0].start, best[0].end), (ago(1), ago(0)));
        assert_eq!((best[1].start, best[1].end), (ago(7), ago(3)));
    }

    #[test]
    fn periods_group_and_keep_gaps() {
        let mut habit = habit(Frequency::DAILY);
        habit.marks.insert(ago(0), Mark::Yes);
        habit.marks.insert(ago(1), Mark::Yes);
        habit.marks.insert(ago(15), Mark::Yes);
        let computed = habit.compute(today());
        let weeks = history_series(&computed, Bucket::Week, Weekday::Mon, today());
        let counts: Vec<f64> = weeks.iter().map(|(_, count)| *count).collect();
        assert_eq!(counts, [2.0, 0.0, 1.0]);
        assert_eq!(weeks[0].0, NaiveDate::from_ymd_opt(2015, 1, 19).unwrap());
        assert_eq!(
            Bucket::Quarter.start(today(), Weekday::Mon),
            NaiveDate::from_ymd_opt(2015, 1, 1).unwrap()
        );
    }

    #[test]
    fn weekday_frequency_counts_each_month() {
        let mut habit = habit(Frequency::DAILY);
        habit.marks.insert(today(), Mark::Yes); // a Sunday
        habit.marks.insert(ago(7), Mark::Yes);
        let months = weekday_frequency(&habit.compute(today()));
        let month = NaiveDate::from_ymd_opt(2015, 1, 1).unwrap();
        assert_eq!(months[&month][6], 2.0);
        assert_eq!(
            weekdays_in_month(month)[3],
            5,
            "January 2015 has 5 Thursdays"
        );
    }

    #[test]
    fn extra_times_on_a_yes_habit_are_noted_but_count_the_same() {
        let mut once = habit(Frequency::DAILY);
        let mut thrice = habit(Frequency::DAILY);
        once.marks.insert(ago(1), Mark::Yes);
        thrice.marks.insert(ago(1), Mark::Reps(3));
        assert_eq!(thrice.times_on(ago(1)), 3);
        assert_eq!(once.times_on(ago(1)), 1);
        let (once, thrice) = (once.compute(today()), thrice.compute(today()));
        assert_eq!(thrice.status(ago(1)), Status::Yes);
        assert_eq!(once.score(today()), thrice.score(today()));
        assert_eq!(once.total(), thrice.total());
        assert_eq!(once.best_streaks(1), thrice.best_streaks(1));
    }

    fn abstain(days_old: i64) -> Habit {
        let mut habit = Habit::new("n", "Smoke");
        habit.kind = Kind::No;
        habit.created = Some(ago(days_old));
        habit
    }

    #[test]
    fn a_no_habit_succeeds_on_every_day_it_is_not_done() {
        let mut habit = abstain(19);
        let computed = habit.compute(today());
        assert_eq!(computed.status(today()), Status::Clean);
        assert_eq!(computed.total(), 20.0);
        // The same climb a perfect daily habit makes.
        assert!((computed.score(today()) - 0.655747).abs() < EPSILON);
        assert_eq!(computed.best_streaks(1)[0].length(), 20);

        habit.marks.insert(ago(5), Mark::Reps(2));
        let computed = habit.compute(today());
        assert_eq!(computed.status(ago(5)), Status::Slipped(2));
        assert!(
            computed.score(ago(5)) < computed.score(ago(6)),
            "doing it hurts"
        );
        assert_eq!(computed.total(), 19.0);
        assert_eq!(computed.best_streaks(1)[0].start, ago(19));
        assert_eq!(computed.best_streaks(1)[0].end, ago(6));
    }

    #[test]
    fn clicking_a_no_habit_records_doing_it_and_back() {
        let mut habit = abstain(3);
        assert_eq!(habit.toggled(Status::Clean), Some(Mark::Reps(1)));
        assert_eq!(habit.toggled(Status::Slipped(1)), None, "clean again");
        habit.skip_days = true;
        assert_eq!(habit.toggled(Status::Slipped(1)), Some(Mark::Skip));
        assert_eq!(habit.toggled(Status::Skip), None);
    }

    #[test]
    fn a_no_habit_with_question_marks_only_trusts_days_said_to_be_clean() {
        let mut habit = abstain(3);
        habit.question_marks = true;
        assert_eq!(habit.compute(today()).status(ago(1)), Status::Unknown);
        assert_eq!(habit.toggled(Status::Unknown), Some(Mark::No));
        habit.toggle(ago(1), Status::Unknown);
        let computed = habit.compute(today());
        assert_eq!(computed.status(ago(1)), Status::Clean);
        assert_eq!(computed.total(), 1.0);
    }

    #[test]
    fn amounts_read_cleanly() {
        assert_eq!(format_amount(2.5), "2.5");
        assert_eq!(format_amount(10.0), "10");
        assert_eq!(format_amount(0.25), "0.25");
        assert_eq!(format_amount(0.0), "0");
    }

    #[test]
    fn labels_use_loops_words() {
        let interval = |times, days| Frequency::Interval { times, days };
        assert_eq!(interval(1, 1).label(), "Every day");
        assert_eq!(interval(7, 7).label(), "Every day");
        assert_eq!(interval(1, 7).label(), "Every week");
        assert_eq!(interval(1, 3).label(), "Every 3 days");
        assert_eq!(interval(2, 7).label(), "2 times per week");
        assert_eq!(interval(10, 30).label(), "10 times per month");
        assert_eq!(interval(3, 8).label(), "3 times in 8 days");
        assert_eq!(
            Frequency::TimesPerDay { times: 2 }.label(),
            "2 times per day"
        );
        let mut days = [false; 7];
        days[0] = true;
        days[2] = true;
        assert_eq!(Frequency::Weekdays { days }.label(), "Every Mon, Wed");
    }

    #[test]
    fn sorting_matches_the_chosen_key_and_reverses_on_a_second_choice() {
        let mut library = Library::default();
        for (id, color, days) in [("b", 5, 10), ("a", 9, 0), ("c", 1, 20)] {
            let mut habit = Habit::new(id, id);
            habit.color = color;
            check(&mut habit, 0, days);
            library.habits.push(habit);
        }
        let computed: BTreeMap<String, Computed> = library
            .habits
            .iter()
            .map(|habit| (habit.id.clone(), habit.compute(today())))
            .collect();
        let order = |library: &Library| -> Vec<String> {
            library
                .visible(&|id| computed.get(id), today())
                .iter()
                .map(|habit| habit.id.clone())
                .collect()
        };
        assert_eq!(order(&library), ["b", "a", "c"], "manual keeps its order");
        library.choose_sort(SortKey::Name);
        assert_eq!(order(&library), ["a", "b", "c"]);
        library.choose_sort(SortKey::Name);
        assert_eq!(order(&library), ["c", "b", "a"]);
        library.choose_sort(SortKey::Color);
        assert_eq!(order(&library), ["c", "b", "a"]);
        library.choose_sort(SortKey::Score);
        assert_eq!(order(&library), ["c", "b", "a"]);
        library.choose_sort(SortKey::Status);
        assert_eq!(order(&library).last().unwrap(), "a", "not done today");
        library.choose_sort(SortKey::Manual);
        library.move_habit("c", "b");
        assert_eq!(order(&library), ["c", "b", "a"]);
    }

    #[test]
    fn resisting_a_no_habit_is_recorded_without_changing_the_score() {
        let today = today();
        let mut plain = Habit::new("a", "Smoke");
        plain.kind = Kind::No;
        plain.created = Some(today - Duration::days(10));
        let mut resisted = plain.clone();
        resisted.set_resisted(today, 2);
        assert_eq!(resisted.resisted_on(today), 2);
        let (plain, resisted) = (plain.compute(today), resisted.compute(today));
        assert_eq!(resisted.status(today), Status::Clean);
        assert_eq!(resisted.score(today), plain.score(today));
        assert_eq!(resisted.total(), plain.total());
    }

    #[test]
    fn resisting_says_a_day_was_clean_even_where_nothing_else_is_known() {
        let today = today();
        let mut habit = Habit::new("a", "Smoke");
        habit.kind = Kind::No;
        habit.question_marks = true;
        habit.created = Some(today);
        let yesterday = today - Duration::days(1);
        assert_eq!(habit.compute(today).status(yesterday), Status::Unknown);
        habit.set_resisted(yesterday, 1);
        assert_eq!(habit.compute(today).status(yesterday), Status::Clean);
        assert_eq!(habit.compute(today).first_day(), yesterday);
        habit.set_resisted(yesterday, 0);
        assert!(habit.resisted.is_empty());
        assert_eq!(habit.compute(today).status(yesterday), Status::Unknown);
    }

    #[test]
    fn a_slip_and_a_resisted_count_do_not_overwrite_each_other() {
        let today = today();
        let mut habit = Habit::new("a", "Smoke");
        habit.kind = Kind::No;
        habit.set_resisted(today, 3);
        habit.marks.insert(today, Mark::Reps(1));
        assert_eq!(habit.compute(today).status(today), Status::Slipped(1));
        assert_eq!(habit.resisted_on(today), 3);
        habit.marks.remove(&today);
        assert_eq!(habit.compute(today).status(today), Status::Clean);
        assert_eq!(habit.resisted_on(today), 3);
    }

    fn sectioned() -> Library {
        let mut library = Library::default();
        for (id, section) in [
            ("a", None),
            ("b", Some("x")),
            ("c", Some("y")),
            ("d", Some("x")),
            ("e", None),
            ("f", Some("gone")),
        ] {
            let mut habit = Habit::new(id, id);
            habit.section = section.map(str::to_owned);
            library.habits.push(habit);
        }
        for id in ["y", "x", "empty"] {
            library.sections.push(Section {
                id: id.to_owned(),
                name: id.to_owned(),
                collapsed: false,
            });
        }
        library
    }

    fn layout(library: &Library) -> Vec<(Option<String>, Vec<String>)> {
        library
            .grouped(library.habits.iter().collect())
            .into_iter()
            .map(|(section, habits)| {
                (
                    section.map(|section| section.id.clone()),
                    habits.iter().map(|habit| habit.id.clone()).collect(),
                )
            })
            .collect()
    }

    fn group(id: Option<&str>, habits: &[&str]) -> (Option<String>, Vec<String>) {
        (
            id.map(str::to_owned),
            habits.iter().map(|habit| (*habit).to_owned()).collect(),
        )
    }

    #[test]
    fn habits_are_grouped_in_section_order_with_the_rest_first() {
        assert_eq!(
            layout(&sectioned()),
            [
                group(None, &["a", "e", "f"]),
                group(Some("y"), &["c"]),
                group(Some("x"), &["b", "d"]),
                group(Some("empty"), &[]),
            ]
        );
    }

    #[test]
    fn a_section_with_every_habit_filtered_out_is_left_out() {
        let library = sectioned();
        let visible = library.habits.iter().filter(|habit| habit.id != "c");
        let groups = library.grouped(visible.collect());
        let names: Vec<_> = groups
            .iter()
            .map(|(section, _)| section.map(|section| section.id.as_str()))
            .collect();
        assert_eq!(names, [None, Some("x"), Some("empty")]);
    }

    #[test]
    fn moving_a_habit_onto_another_takes_it_into_that_section() {
        let mut library = sectioned();
        library.move_habit("a", "c");
        assert_eq!(library.habit("a").unwrap().section.as_deref(), Some("y"));
        library.move_habit("a", "e");
        assert_eq!(library.habit("a").unwrap().section, None);
    }

    #[test]
    fn moving_a_habit_to_a_section_puts_it_at_the_top() {
        let mut library = sectioned();
        library.move_to_section("a", "x");
        assert_eq!(layout(&library)[2], group(Some("x"), &["a", "b", "d"]),);
        library.move_to_section("d", "x");
        assert_eq!(layout(&library)[2], group(Some("x"), &["d", "a", "b"]),);
        library.move_to_section("b", "empty");
        assert_eq!(layout(&library)[3], group(Some("empty"), &["b"]));
    }

    #[test]
    fn deleting_a_section_keeps_its_habits() {
        let mut library = sectioned();
        library.remove_section("x");
        assert_eq!(library.habits.len(), 6);
        assert_eq!(library.habit("b").unwrap().section, None);
        assert_eq!(library.sections.len(), 2);
    }

    #[test]
    fn sections_shift_within_the_list_and_stop_at_its_ends() {
        let mut library = sectioned();
        let order = |library: &Library| -> Vec<String> {
            library.sections.iter().map(|s| s.id.clone()).collect()
        };
        library.shift_section("y", true);
        assert_eq!(order(&library), ["y", "x", "empty"]);
        library.shift_section("y", false);
        assert_eq!(order(&library), ["x", "y", "empty"]);
        library.shift_section("empty", false);
        assert_eq!(order(&library), ["x", "y", "empty"]);
    }

    #[test]
    fn a_file_from_before_sections_still_loads() {
        let json = r#"{"habits":[{"id":"a","name":"Run"}]}"#;
        let library: Library = serde_json::from_str(json).unwrap();
        assert!(library.sections.is_empty());
        assert_eq!(library.habits[0].section, None);
        assert!(library.habits[0].resisted.is_empty());
    }

    #[test]
    fn archived_and_completed_habits_can_be_hidden() {
        let mut library = Library::default();
        let mut archived = Habit::new("archived", "Archived");
        archived.archived = true;
        let mut done = Habit::new("done", "Done");
        done.marks.insert(today(), Mark::Yes);
        library.habits = vec![archived, done, Habit::new("open", "Open")];
        let computed: BTreeMap<String, Computed> = library
            .habits
            .iter()
            .map(|habit| (habit.id.clone(), habit.compute(today())))
            .collect();
        let ids = |library: &Library| -> Vec<String> {
            library
                .visible(&|id| computed.get(id), today())
                .iter()
                .map(|habit| habit.id.clone())
                .collect()
        };
        assert_eq!(ids(&library), ["done", "open"]);
        library.hide_archived = false;
        library.hide_completed = true;
        assert_eq!(ids(&library), ["archived", "open"]);
    }

    #[test]
    fn a_library_survives_saving_and_older_files_still_load() {
        let mut library = Library::default();
        let mut habit = habit(Frequency::Weekdays {
            days: [true, false, false, false, false, false, false],
        });
        habit.marks.insert(today(), Mark::Reps(2));
        habit.marks.insert(ago(1), Mark::Skip);
        library.habits.push(habit);
        let text = serde_json::to_string(&library).unwrap();
        assert_eq!(serde_json::from_str::<Library>(&text).unwrap(), library);

        let sparse: Library =
            serde_json::from_str(r#"{"habits":[{"id":"x","name":"Old"}]}"#).unwrap();
        assert!(sparse.hide_archived);
        assert_eq!(sparse.habits[0].color, DEFAULT_COLOR);
        assert_eq!(sparse.habits[0].frequency, Frequency::DAILY);
    }

    #[test]
    fn the_crc_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn export_lays_files_out_as_loop_does() {
        let mut habit = habit(Frequency::DAILY);
        habit.name = "Read, a lot / daily".into();
        check(&mut habit, 0, 3);
        let bytes = export_archive(&[&habit], &[], today());
        let text = String::from_utf8_lossy(&bytes);
        assert!(bytes.starts_with(b"PK\x03\x04"));
        for name in [
            "Habits.csv",
            "001 Read a lot  daily/Scores.csv",
            "001 Read a lot  daily/Checkmarks.csv",
            "Scores.csv",
            "Checkmarks.csv",
        ] {
            assert!(text.contains(name), "{name}");
        }
        assert!(
            text.contains("\"Read, a lot / daily\""),
            "commas are quoted"
        );
        assert!(text.contains("2015-01-25,YES_MANUAL"));
        assert!(text.contains("2015-01-25,0.1478"));
    }

    #[test]
    fn a_day_entry_reads_back_what_was_saved_for_either_kind() {
        let day = today();
        for kind in [Kind::Yes, Kind::No] {
            let mut habit = Habit::new("h", "Run");
            habit.kind = kind;
            let entry = DayEntry {
                outcome: Outcome::Done,
                times: 2,
                resisted: if kind == Kind::No { 3 } else { 0 },
                value: Some(1.5),
            };
            habit.set_entry(day, entry);
            assert_eq!(habit.entry(day), entry);
            habit.set_entry(
                day,
                DayEntry {
                    outcome: Outcome::NotDone,
                    times: 0,
                    resisted: 0,
                    value: None,
                },
            );
            assert!(habit.marks.is_empty() && habit.values.is_empty());
            assert_eq!(habit.entry(day).outcome, Outcome::NotDone);
        }
    }

    #[test]
    fn question_marks_keep_not_done_apart_from_no_answer() {
        let day = today();
        let mut habit = Habit::new("h", "Run");
        habit.question_marks = true;
        assert_eq!(habit.entry(day).outcome, Outcome::Unanswered);
        let mut entry = habit.entry(day);
        entry.outcome = Outcome::NotDone;
        habit.set_entry(day, entry);
        assert_eq!(habit.entry(day).outcome, Outcome::NotDone);
        entry.outcome = Outcome::Unanswered;
        habit.set_entry(day, entry);
        assert!(habit.marks.is_empty());
    }

    #[test]
    fn a_value_never_changes_the_score_on_either_kind() {
        for kind in [Kind::Yes, Kind::No] {
            let mut plain = Habit::new("h", "Run");
            plain.kind = kind;
            plain.created = Some(ago(10));
            plain.marks.insert(ago(3), Mark::Reps(1));
            let mut measured = plain.clone();
            measured.measure = Some(Measure {
                unit: "miles".into(),
                target: Some(2.0),
                days: 1,
            });
            measured.set_value(ago(3), Some(5.0));
            measured.set_value(ago(1), Some(1.0));
            let (plain, measured) = (plain.compute(today()), measured.compute(today()));
            assert_eq!(plain.score(today()), measured.score(today()));
            assert_eq!(plain.status(ago(1)), measured.status(ago(1)));
            assert_eq!(plain.total(), measured.total());
        }
    }

    #[test]
    fn a_target_reads_by_kind_and_period() {
        let mut measure = Measure {
            unit: "miles".into(),
            target: Some(2.5),
            days: 7,
        };
        assert_eq!(measure.label(Kind::Yes), "At least 2.5 miles per week");
        assert_eq!(measure.label(Kind::No), "At most 2.5 miles per week");
        measure.target = None;
        assert_eq!(measure.label(Kind::Yes), "Tracking miles");
        measure.unit.clear();
        assert_eq!(measure.label(Kind::No), "Tracking a value");
    }

    #[test]
    fn values_add_up_over_the_targets_period_and_keep_gaps() {
        // 2015-01-25 is a Sunday, the last day of its week.
        let mut habit = Habit::new("h", "Run");
        habit.measure = Some(Measure {
            unit: "miles".into(),
            target: Some(10.0),
            days: 7,
        });
        habit.set_value(ago(0), Some(4.0));
        habit.set_value(ago(6), Some(6.0));
        habit.set_value(ago(14), Some(1.5));
        let series = value_series(&habit, Weekday::Mon, today());
        let week = NaiveDate::from_ymd_opt(2015, 1, 19).unwrap();
        assert_eq!(
            series,
            [
                (week, 10.0),
                (week - Duration::days(7), 0.0),
                (week - Duration::days(14), 1.5),
            ]
        );
        assert!(value_series(&Habit::new("e", "E"), Weekday::Mon, today()).is_empty());
    }

    #[test]
    fn a_value_before_any_other_record_is_still_reached() {
        let mut habit = Habit::new("h", "Run");
        habit.set_value(ago(5), Some(1.0));
        assert_eq!(habit.compute(today()).first_day(), ago(5));
    }

    fn old_habit(rest: &str) -> Habit {
        serde_json::from_str(&format!(r#"{{"id":"a","name":"Run",{rest}}}"#)).unwrap()
    }

    #[test]
    fn an_old_at_least_measurable_habit_becomes_did_it() {
        let habit = old_habit(
            r#"
            "kind":"measurable","unit":"miles","target":2.0,"target_type":"at_least",
            "frequency":{"interval":{"times":1,"days":1}},
            "marks":{
                "2015-01-20":{"amount":2500},
                "2015-01-21":{"amount":1000},
                "2015-01-22":{"amount":0},
                "2015-01-23":"skip"
            }"#,
        );
        assert_eq!(habit.kind, Kind::Yes);
        let measure = habit.measure.clone().unwrap();
        assert_eq!(
            (measure.unit.as_str(), measure.target, measure.days),
            ("miles", Some(2.0), 1)
        );
        let day = |n| NaiveDate::from_ymd_opt(2015, 1, n).unwrap();
        assert_eq!(habit.value_on(day(20)), Some(2.5), "every amount is kept");
        assert_eq!(habit.value_on(day(21)), Some(1.0));
        assert_eq!(habit.value_on(day(22)), Some(0.0));
        assert_eq!(habit.marks.get(&day(20)), Some(&Mark::Yes), "met");
        assert_eq!(habit.marks.get(&day(21)), None, "short of the target");
        assert_eq!(habit.marks.get(&day(22)), None);
        assert_eq!(habit.marks.get(&day(23)), Some(&Mark::Skip));
    }

    #[test]
    fn an_old_at_most_measurable_habit_becomes_avoided_it() {
        let habit = old_habit(
            r#"
            "kind":"measurable","unit":"cigarettes","target":7.0,"target_type":"at_most",
            "frequency":{"interval":{"times":1,"days":7}},
            "marks":{"2015-01-20":{"amount":1000},"2015-01-21":{"amount":3000}}"#,
        );
        assert_eq!(habit.kind, Kind::No);
        assert_eq!(habit.frequency, Frequency::DAILY);
        assert_eq!(habit.measure.as_ref().unwrap().days, 7);
        let day = |n| NaiveDate::from_ymd_opt(2015, 1, n).unwrap();
        assert_eq!(habit.marks.get(&day(20)), None, "within a day's share");
        assert_eq!(habit.marks.get(&day(21)), Some(&Mark::Reps(1)));
        assert_eq!(habit.value_on(day(21)), Some(3.0));
    }

    #[test]
    fn a_week_long_target_counts_any_amount_as_doing_it() {
        let habit = old_habit(
            r#"
            "kind":"measurable","target":20.0,
            "frequency":{"interval":{"times":1,"days":7}},
            "marks":{"2015-01-20":{"amount":1000}}"#,
        );
        let day = NaiveDate::from_ymd_opt(2015, 1, 20).unwrap();
        assert_eq!(habit.marks.get(&day), Some(&Mark::Yes));
    }

    #[test]
    fn a_saved_habit_with_a_measure_reads_back_the_same() {
        let mut habit = Habit::new("h", "Run");
        habit.kind = Kind::No;
        habit.measure = Some(Measure {
            unit: "miles".into(),
            target: Some(2.0),
            days: 7,
        });
        habit.set_value(today(), Some(1.25));
        habit.set_resisted(today(), 2);
        habit.section = Some("s".into());
        let json = serde_json::to_string(&habit).unwrap();
        assert!(!json.contains("target_type") && !json.contains("\"unit\":\"\",\"target\":0"));
        assert_eq!(serde_json::from_str::<Habit>(&json).unwrap(), habit);
    }

    #[test]
    fn export_carries_sections_targets_values_and_times_resisted() {
        let mut run = Habit::new("r", "Run");
        run.section = Some("s".into());
        run.measure = Some(Measure {
            unit: "miles".into(),
            target: Some(2.0),
            days: 7,
        });
        run.set_value(today(), Some(1.125));
        let mut smoke = Habit::new("n", "Smoke");
        smoke.kind = Kind::No;
        smoke.created = Some(today());
        smoke.set_resisted(today(), 4);
        let plain = Habit::new("p", "Plain");
        let sections = [Section {
            id: "s".into(),
            name: "Morning".into(),
            collapsed: false,
        }];
        let bytes = export_archive(&[&run, &smoke, &plain], &sections, today());
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            text.contains("Position,Name,Type,Section,Unit,Target Type,Target Value,Target Days")
        );
        assert!(text.contains("001,Run,DID_IT,Morning,miles,AT_LEAST,2,7,"));
        assert!(text.contains("002,Smoke,AVOIDED_IT,,,,,,"));
        assert!(text.contains("Date,Status,Times,Resisted,Value"));
        assert!(text.contains("2015-01-25,UNKNOWN,0,0,1.125"), "{text}");
        assert!(text.contains("2015-01-25,CLEAN,0,4,"), "{text}");
        assert!(text.contains("Values.csv") && text.contains("Date,Run\n2015-01-25,1.125"));
        assert!(text.contains("Resisted.csv") && text.contains("Date,Smoke\n2015-01-25,4"));
    }

    #[test]
    fn export_leaves_out_values_and_resisted_when_there_are_none() {
        let plain = Habit::new("p", "Plain");
        let bytes = export_archive(&[&plain], &[], today());
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("Values.csv") && !text.contains("Resisted.csv"));
    }

    #[test]
    fn a_habit_saved_before_icons_existed_keeps_symbols() {
        let habit: Habit = serde_json::from_str(r#"{"id":"h","name":"Read"}"#).unwrap();
        assert_eq!(habit.icons, Icons::Symbols);
    }

    #[test]
    fn log_times_read_as_typed() {
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0);
        for (text, time) in [
            ("20:05", t(20, 5)),
            ("8:05pm", t(20, 5)),
            ("8.05p", t(20, 5)),
            ("8:05 PM", t(20, 5)),
            ("11p", t(23, 0)),
            ("2pm", t(14, 0)),
            ("12a", t(0, 0)),
            ("12:30p", t(12, 30)),
            ("805p", t(20, 5)),
            ("9", t(9, 0)),
            ("13p", None),
            ("8:5", None),
            ("noon", None),
            ("", None),
        ] {
            assert_eq!(parse_log_time(text), time, "{text}");
        }
        assert_eq!(parse_log_amount("12m"), Some(12.0));
        assert_eq!(parse_log_amount("2.5 mins"), Some(2.5));
        assert_eq!(parse_log_amount("-3"), None);
        assert_eq!(parse_log_amount("lots"), None);
    }

    #[test]
    fn a_log_keeps_its_entries_and_reads_the_first_format() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let mut habit: Habit = serde_json::from_str(
            r#"{"id":"h","name":"Smoke","kind":"no","marks":{"2026-11-05":{"reps":2}},
                "log":{"2026-11-05":{"slipped":["20:05:00"]}}}"#,
        )
        .unwrap();
        let at = NaiveTime::from_hms_opt(20, 5, 0);
        assert_eq!(
            habit.happened_log_on(day),
            vec![LogEntry::default(), LogEntry::new(at, None)],
            "the count from before is kept, untimed"
        );
        let late = LogEntry::new(NaiveTime::from_hms_opt(23, 0, 0), Some(12.0));
        habit.set_log(day, &[late.clone(), LogEntry::new(at, Some(6.0))], &[]);
        let back: Habit = serde_json::from_str(&serde_json::to_string(&habit).unwrap()).unwrap();
        assert_eq!(
            back.happened_log_on(day),
            vec![LogEntry::new(at, Some(6.0)), late]
        );
    }
}
