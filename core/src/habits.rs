//! Habits, modelled on Loop Habit Tracker.
//!
//! A [`Habit`] is a goal ("every Monday", "2 times a week") with a day-by-day
//! record of whether it happened. It is independent of the calendar: any number
//! of habit events can point at one through their `ttHabit` property, and a
//! habit needs no event at all.
//!
//! Interval snapping, scores and streaks follow Loop's `EntryList`, `ScoreList`
//! and `StreakList`, so a record scores the same here as in Loop. Added to that:
//! a times-per-day goal, a goal on chosen weekdays, a "No" habit that is a success
//! on every day it is not done, recording how many times a day it was done, and
//! skip days and question marks being set per habit instead of globally.

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

/// The Google extended property naming the habit an event belongs to.
pub const LINK_PROPERTY: &str = "ttHabit";

/// What a habit records, and so what counts as success.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Doing it is the goal. Doing it again on the same day is noted, but the
    /// day counts the same.
    #[default]
    Yes,
    /// Never doing it is the goal, so every day it is not done is a success.
    No,
    /// An amount tracked against a target, like Loop's numeric habits.
    Measurable,
}

/// Which side of a measurable habit's target is the goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetType {
    #[default]
    AtLeast,
    AtMost,
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
    /// A measurable habit's value, in thousandths as Loop stores it.
    Amount(i64),
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
    /// On a measurable habit: the day's amount in thousandths, and whether the
    /// period it falls in met the target.
    Measured {
        amount: i64,
        met: bool,
    },
}

impl Status {
    /// What the day adds to the totals and charts.
    pub fn counted(self) -> f64 {
        match self {
            Self::Yes | Self::Clean => 1.0,
            Self::Measured { amount, .. } => amount.max(0) as f64 / 1000.0,
            _ => 0.0,
        }
    }

    /// Whether the day keeps a streak going.
    pub fn completed(self) -> bool {
        match self {
            Self::Yes | Self::Auto | Self::Skip | Self::Clean => true,
            Self::Measured { met, .. } => met,
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

/// One habit: its goal, its settings and everything recorded for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Habit {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    /// The day it was added, from which a "No" habit starts counting.
    #[serde(default)]
    pub created: Option<NaiveDate>,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub target: f64,
    #[serde(default)]
    pub target_type: TargetType,
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
}

impl Habit {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind: Kind::Yes,
            created: None,
            unit: String::new(),
            target: 0.0,
            target_type: TargetType::AtLeast,
            question: String::new(),
            notes: String::new(),
            color: DEFAULT_COLOR,
            frequency: Frequency::DAILY,
            archived: false,
            skip_days: false,
            question_marks: false,
            icons: Icons::Symbols,
            marks: BTreeMap::new(),
        }
    }

    /// Every day's status, its score and its streaks, as of `today`.
    pub fn compute(&self, today: NaiveDate) -> Computed {
        let (statuses, scores, from) = match self.kind {
            Kind::Yes => {
                let statuses = statuses(self.frequency, &self.marks);
                let from = first_day(&statuses, today);
                let scores = scores(self.frequency, &statuses, from, today);
                (statuses, scores, from)
            }
            Kind::No => {
                let statuses = abstain_statuses(self, today);
                let from = first_day(&statuses, today);
                let scores = scores(Frequency::DAILY, &statuses, from, today);
                (statuses, scores, from)
            }
            Kind::Measurable => {
                let statuses = measured_statuses(self, today);
                let from = first_day(&statuses, today);
                let scores = measured_scores(self, &statuses, from, today);
                (statuses, scores, from)
            }
        };
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

    /// Whether a click needs a number, as it does for a measurable habit.
    pub fn asks_for_amount(&self) -> bool {
        self.kind == Kind::Measurable
    }

    /// Whether a day comes down to a plain yes or no. A measurable habit needs
    /// an amount, and a times-per-day goal needs to know how many.
    pub fn asks_yes_or_no(&self) -> bool {
        self.kind != Kind::Measurable && self.frequency.per_day() == 1
    }

    /// Records a plain answer to whether it was done on a day. Saying yes to a
    /// day already done keeps the number of times recorded for it.
    pub fn answer(&mut self, day: NaiveDate, done: bool) {
        if self.kind == Kind::Measurable || (done && self.times_on(day) > 0) {
            return;
        }
        let mark = match (done, self.kind) {
            (false, _) => Mark::No,
            (true, Kind::No) => Mark::Reps(1),
            (true, _) => Mark::Yes,
        };
        self.marks.insert(day, mark);
    }

    /// The goal, in words.
    pub fn goal_label(&self) -> String {
        match self.kind {
            Kind::Yes => self.frequency.label(),
            Kind::No => "Never".to_owned(),
            Kind::Measurable => {
                let side = match self.target_type {
                    TargetType::AtLeast => "At least",
                    TargetType::AtMost => "At most",
                };
                let period = match self.frequency.fraction().1 {
                    1 => "day",
                    7 => "week",
                    _ => "month",
                };
                let amount = format_amount(self.target);
                let unit = if self.unit.is_empty() {
                    String::new()
                } else {
                    format!(" {}", self.unit)
                };
                format!("{side} {amount}{unit} per {period}")
            }
        }
    }

    /// Records an amount or a number of times done, from the day dialog. Zero
    /// on a "Yes" or "No" habit returns the day to its default.
    pub fn set_day(&mut self, day: NaiveDate, value: f64) {
        match self.kind {
            Kind::Measurable => {
                self.marks
                    .insert(day, Mark::Amount((value.max(0.0) * 1000.0).round() as i64));
            }
            Kind::Yes | Kind::No => {
                let times = value.max(0.0).round() as u32;
                if times == 0 && self.kind == Kind::No && self.question_marks {
                    self.marks.insert(day, Mark::No);
                } else if times == 0 {
                    self.marks.remove(&day);
                } else {
                    self.marks.insert(day, Mark::Reps(times));
                }
            }
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

/// A number without the noise: `2.5`, `10`, `0.25`.
pub fn format_amount(value: f64) -> String {
    let text = format!("{value:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn reps(mark: Mark, goal: u32) -> u32 {
    match mark {
        Mark::Yes => goal,
        Mark::Reps(reps) => reps,
        Mark::No | Mark::Skip | Mark::Amount(_) => 0,
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
                    Mark::No | Mark::Amount(_) => Status::No,
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
            Mark::No | Mark::Reps(0) | Mark::Amount(_) => {
                // A "no" inside a covered stretch stays covered, as in Loop.
                result.entry(*day).or_insert(Status::No);
            }
        }
    }
    result
}

/// A "No" habit: every day since it began is a success unless it was done. With
/// question marks on, only a day said to be clean counts.
fn abstain_statuses(habit: &Habit, today: NaiveDate) -> BTreeMap<NaiveDate, Status> {
    let oldest = habit.marks.keys().next().copied();
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
            None if habit.question_marks => Status::Unknown,
            None => Status::Clean,
        };
        if status != Status::Unknown {
            result.insert(day, status);
        }
        day += Duration::days(1);
    }
    result
}

fn period_start(day: NaiveDate, days: u32) -> NaiveDate {
    match days {
        0 | 1 => day,
        2..=7 => Bucket::Week.start(day, Weekday::Mon),
        _ => Bucket::Month.start(day, Weekday::Mon),
    }
}

/// Loop's `recomputeCompletion` for numeric habits: amounts are summed over the
/// day, week or month, against a target reduced by the days skipped.
fn measured_statuses(habit: &Habit, today: NaiveDate) -> BTreeMap<NaiveDate, Status> {
    let (_, denominator) = habit.frequency.fraction();
    let period_length: f64 = match denominator {
        0 | 1 => 1.0,
        2..=7 => 7.0,
        _ => 30.0,
    };
    let daily_target = habit.target / f64::from(denominator);
    let base_target = if f64::from(denominator) == period_length {
        habit.target
    } else {
        daily_target * period_length
    };

    let from = habit
        .marks
        .keys()
        .next()
        .copied()
        .unwrap_or(today)
        .min(today);
    let mut periods: BTreeMap<NaiveDate, (i64, u32)> = BTreeMap::new();
    let mut day = from;
    while day <= today {
        let slot = periods.entry(period_start(day, denominator)).or_default();
        match habit.marks.get(&day) {
            Some(Mark::Amount(amount)) => slot.0 += (*amount).max(0),
            Some(Mark::Skip) => slot.1 += 1,
            _ => {}
        }
        day += Duration::days(1);
    }

    let met = |(sum, skipped): (i64, u32)| {
        let target = (base_target - daily_target * f64::from(skipped)).max(0.0);
        let sum = sum as f64 / 1000.0;
        match habit.target_type {
            TargetType::AtLeast => sum >= target,
            TargetType::AtMost => sum <= target,
        }
    };

    let mut result = BTreeMap::new();
    let mut day = from;
    while day <= today {
        let status = match habit.marks.get(&day) {
            Some(Mark::Skip) => Status::Skip,
            mark => Status::Measured {
                amount: match mark {
                    Some(Mark::Amount(amount)) => *amount,
                    _ => 0,
                },
                met: periods
                    .get(&period_start(day, denominator))
                    .copied()
                    .is_some_and(met),
            },
        };
        result.insert(day, status);
        day += Duration::days(1);
    }
    result
}

/// The numeric half of Loop's `ScoreList.recompute`.
fn measured_scores(
    habit: &Habit,
    statuses: &BTreeMap<NaiveDate, Status>,
    from: NaiveDate,
    today: NaiveDate,
) -> Vec<f64> {
    let (_, denominator) = habit.frequency.fraction();
    let freq = habit.frequency.as_f64();
    let at_most = habit.target_type == TargetType::AtMost;
    let mut scores = Vec::new();
    let mut window = std::collections::VecDeque::new();
    let mut rolling = 0_i64;
    let mut previous = if at_most { 1.0 } else { 0.0 };
    let mut day = from;
    while day <= today {
        let status = statuses.get(&day).copied().unwrap_or(Status::Unknown);
        if status != Status::Skip {
            let amount = match status {
                Status::Measured { amount, .. } => amount.max(0),
                _ => 0,
            };
            window.push_back(amount);
            rolling += amount;
            if window.len() > denominator as usize {
                rolling -= window.pop_front().unwrap_or(0);
            }
            let sum = rolling as f64 / 1000.0;
            let target = habit.target;
            let completed = if !at_most {
                if target > 0.0 {
                    (sum / target).min(1.0)
                } else {
                    1.0
                }
            } else if target > 0.0 {
                (1.0 - (sum - target) / target).clamp(0.0, 1.0)
            } else if sum > 0.0 {
                0.0
            } else {
                1.0
            };
            previous = compute_score(freq, previous, completed);
        }
        scores.push(previous);
        day += Duration::days(1);
    }
    scores
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

/// Everything saved between runs. The order of `habits` is the manual order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub habits: Vec<Habit>,
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

    /// Moves a habit to where another one is in the manual order.
    pub fn move_habit(&mut self, id: &str, onto: &str) {
        let from = self.habits.iter().position(|habit| habit.id == id);
        let to = self.habits.iter().position(|habit| habit.id == onto);
        if let (Some(from), Some(to)) = (from, to)
            && from != to
        {
            let habit = self.habits.remove(from);
            self.habits.insert(to, habit);
        }
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
        Status::Yes | Status::Clean | Status::Measured { met: true, .. } => 0,
        Status::Auto => 1,
        Status::Skip => 2,
        Status::Partial(_) => 3,
        Status::Measured { met: false, .. } => 4,
        Status::No | Status::Slipped(_) => 5,
        Status::Unknown => 6,
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
        Status::Measured { amount, .. } => format_amount(amount as f64 / 1000.0),
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

fn habit_list_csv(habits: &[&Habit]) -> String {
    let mut out = csv_line(&[
        "Position",
        "Name",
        "Type",
        "Unit",
        "Target Type",
        "Target Value",
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
        out += &csv_line(&[
            &format!("{:03}", position + 1),
            &habit.name,
            match habit.kind {
                Kind::Yes => "YES",
                Kind::No => "NO",
                Kind::Measurable => "MEASURABLE",
            },
            &habit.unit,
            match (habit.kind, habit.target_type) {
                (Kind::Measurable, TargetType::AtLeast) => "AT_LEAST",
                (Kind::Measurable, TargetType::AtMost) => "AT_MOST",
                _ => "",
            },
            &if habit.kind == Kind::Measurable {
                format_amount(habit.target)
            } else {
                String::new()
            },
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

/// A CSV archive laid out as Loop's export is: a `Habits.csv` list, a folder per
/// habit, and `Scores.csv` and `Checkmarks.csv` with a column per habit.
pub fn export_archive(habits: &[&Habit], today: NaiveDate) -> Vec<u8> {
    let computed: Vec<Computed> = habits.iter().map(|habit| habit.compute(today)).collect();
    let mut files = vec![("Habits.csv".to_owned(), habit_list_csv(habits))];

    for (position, (habit, computed)) in habits.iter().zip(&computed).enumerate() {
        let folder = folder_name(position, &habit.name);
        let mut scores = csv_line(&["Date", "Score"]);
        let mut checks = csv_line(&["Date", "Value", "Times"]);
        let mut day = today;
        while day >= computed.first_day() {
            let date = day.to_string();
            scores += &csv_line(&[&date, &format!("{:.4}", computed.score(day))]);
            checks += &csv_line(&[
                &date,
                &status_text(computed.status(day)),
                &habit.times_on(day).to_string(),
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
    let mut header = vec!["Date"];
    header.extend(habits.iter().map(|habit| habit.name.as_str()));
    let mut scores = csv_line(&header);
    let mut checks = csv_line(&header);
    let mut day = today;
    while day >= oldest {
        let date = day.to_string();
        let score_cells: Vec<String> = computed
            .iter()
            .map(|computed| format!("{:.4}", computed.score(day)))
            .collect();
        let check_cells: Vec<String> = computed
            .iter()
            .map(|computed| status_text(computed.status(day)))
            .collect();
        let row = |cells: &[String]| {
            let mut fields = vec![date.as_str()];
            fields.extend(cells.iter().map(String::as_str));
            csv_line(&fields)
        };
        scores += &row(&score_cells);
        checks += &row(&check_cells);
        day -= Duration::days(1);
    }
    files.push(("Scores.csv".to_owned(), scores));
    files.push(("Checkmarks.csv".to_owned(), checks));
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
        thrice.set_day(ago(1), 3.0);
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

        habit.set_day(ago(5), 2.0);
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

    fn measured(target_type: TargetType) -> Habit {
        let mut habit = Habit::new("m", "Run");
        habit.kind = Kind::Measurable;
        habit.target = 2.0;
        habit.target_type = target_type;
        habit.unit = "miles".into();
        habit
    }

    // Loop's NumericalAtLeastScoreListTest and NumericalAtMostScoreListTest.
    #[test]
    fn measurable_scores_match_loop() {
        let mut habit = measured(TargetType::AtLeast);
        for days in 0..20 {
            habit.set_day(ago(days), 2.0);
        }
        assert_scores(
            &habit,
            &[
                0.655747, 0.636894, 0.617008, 0.596033, 0.573910, 0.550574, 0.525961, 0.500000,
                0.472617, 0.443734, 0.413270, 0.381137, 0.347244, 0.311495, 0.273788, 0.234017,
                0.192067, 0.147820, 0.101149, 0.051922, 0.0, 0.0, 0.0,
            ],
        );

        let mut habit = measured(TargetType::AtMost);
        habit.set_day(ago(20), 1.0);
        for days in 0..20 {
            habit.set_day(ago(days), 5.0);
        }
        assert_scores(
            &habit,
            &[
                0.344253, 0.363106, 0.382992, 0.403967, 0.426090, 0.449426, 0.474039, 0.500000,
                0.527383, 0.556266, 0.586730, 0.618863, 0.652756, 0.688505, 0.726212, 0.765983,
                0.807933, 0.852180, 0.898851, 0.948078, 1.0, 0.0, 0.0,
            ],
        );
    }

    #[test]
    fn a_measurable_target_is_met_across_its_whole_period() {
        // 2015-01-25 is a Sunday, the last day of its week.
        let mut habit = measured(TargetType::AtLeast);
        habit.target = 10.0;
        habit.frequency = Frequency::Interval { times: 1, days: 7 };
        habit.set_day(ago(6), 4.0);
        habit.set_day(ago(1), 6.0);
        let computed = habit.compute(today());
        assert_eq!(
            computed.status(ago(1)),
            Status::Measured {
                amount: 6000,
                met: true
            }
        );
        assert_eq!(
            computed.status(ago(3)),
            Status::Measured {
                amount: 0,
                met: true
            },
            "every day of a week that met it counts"
        );
        assert_eq!(computed.total(), 10.0);
        assert_eq!(habit.goal_label(), "At least 10 miles per week");

        // A skipped day lowers the week's target by a day's share.
        habit.set_day(ago(1), 3.0);
        assert!(!habit.compute(today()).status(ago(1)).completed());
        for days in [2, 3, 4, 5] {
            habit.marks.insert(ago(days), Mark::Skip);
        }
        assert!(habit.compute(today()).status(ago(1)).completed());
    }

    #[test]
    fn a_measurable_target_of_zero_stays_finite() {
        let mut habit = measured(TargetType::AtMost);
        habit.target = 0.0;
        habit.set_day(ago(1), 1.0);
        assert!(habit.compute(today()).score(today()).is_finite());
        habit.target_type = TargetType::AtLeast;
        assert!(habit.compute(today()).score(today()).is_finite());
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
        let bytes = export_archive(&[&habit], today());
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
    fn a_plain_answer_records_the_day_for_each_kind_of_habit() {
        let day = NaiveDate::from_ymd_opt(2026, 3, 4).unwrap();

        let mut yes = Habit::new("y", "Read");
        assert!(yes.asks_yes_or_no());
        yes.answer(day, true);
        assert_eq!(yes.marks[&day], Mark::Yes);
        yes.answer(day, false);
        assert_eq!(yes.marks[&day], Mark::No);
        // Saying yes again does not lose how many times it was done.
        yes.marks.insert(day, Mark::Reps(3));
        yes.answer(day, true);
        assert_eq!(yes.marks[&day], Mark::Reps(3));

        // On a "No" habit a yes is a slip, and a no is a clean day.
        let mut no = Habit::new("n", "Smoke");
        no.kind = Kind::No;
        no.question_marks = true;
        assert!(no.asks_yes_or_no());
        no.answer(day, true);
        assert_eq!(no.compute(day).status(day), Status::Slipped(1));
        no.answer(day, false);
        assert_eq!(no.compute(day).status(day), Status::Clean);

        // Amounts and counts cannot be answered with a yes or a no.
        let mut amount = Habit::new("m", "Run");
        amount.kind = Kind::Measurable;
        assert!(!amount.asks_yes_or_no());
        amount.answer(day, true);
        assert!(amount.marks.is_empty());
        let mut twice = Habit::new("t", "Brush");
        twice.frequency = Frequency::TimesPerDay { times: 2 };
        assert!(!twice.asks_yes_or_no());
    }

    #[test]
    fn a_habit_saved_before_icons_existed_keeps_symbols() {
        let habit: Habit = serde_json::from_str(r#"{"id":"h","name":"Read"}"#).unwrap();
        assert_eq!(habit.icons, Icons::Symbols);
    }
}
