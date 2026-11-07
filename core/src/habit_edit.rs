//! Editing habits: the add and edit form, the dialog for one day, and naming
//! sections. The desktop and the phone both drive these, so the two cannot
//! drift apart in what a habit can record or how a form is read.

use chrono::{NaiveDate, NaiveTime};

use crate::calendar::time_format::TimeFormat;
use crate::habits::{
    DayEntry, Frequency, Habit, Icons, Kind, Library, LogEntry, Measure, Outcome, Section,
};

/// The radio buttons of the frequency section, which are Loop's plus a
/// times-per-day goal and a goal on chosen weekdays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrequencyKind {
    EveryDay,
    TimesPerDay,
    EveryXDays,
    TimesPerWeek,
    TimesPerMonth,
    TimesInDays,
    Weekdays,
}

/// The add and edit page for a habit.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    /// `None` when this is a new habit.
    pub id: Option<String>,
    /// Fixed once a habit exists, since its record would mean something else.
    pub habit_kind: Kind,
    /// Whether a value is recorded each day.
    pub measured: bool,
    pub unit: String,
    /// Blank for a value with no target.
    pub target: String,
    /// The period the target covers: 1, 7 or 30 days.
    pub target_days: u32,
    pub name: String,
    pub question: String,
    /// The saved copy of the text in the notes editor.
    pub notes: String,
    /// The section it is listed under, by id.
    pub section: Option<String>,
    pub color: u8,
    pub kind: FrequencyKind,
    pub per_day: String,
    pub every_days: String,
    pub per_week: String,
    pub per_month: String,
    pub in_times: String,
    pub in_days: String,
    pub weekdays: [bool; 7],
    pub skip_days: bool,
    pub question_marks: bool,
    pub icons: Icons,
    pub error: Option<String>,
}

impl Form {
    pub fn new() -> Self {
        Self {
            id: None,
            habit_kind: Kind::Yes,
            measured: false,
            unit: String::new(),
            target: String::new(),
            target_days: 1,
            name: String::new(),
            question: String::new(),
            notes: String::new(),
            section: None,
            color: crate::habits::DEFAULT_COLOR,
            kind: FrequencyKind::EveryDay,
            per_day: "2".into(),
            every_days: "2".into(),
            per_week: "3".into(),
            per_month: "10".into(),
            in_times: "3".into(),
            in_days: "7".into(),
            weekdays: [true, false, false, false, false, false, false],
            skip_days: false,
            question_marks: false,
            icons: Icons::Symbols,
            error: None,
        }
    }

    pub fn from_habit(habit: &Habit) -> Self {
        let mut form = Self::new();
        form.id = Some(habit.id.clone());
        form.habit_kind = habit.kind;
        if let Some(measure) = &habit.measure {
            form.measured = true;
            form.unit = measure.unit.clone();
            form.target = measure
                .target
                .map_or(String::new(), crate::habits::format_amount);
            form.target_days = measure.days;
        }
        form.name = habit.name.clone();
        form.question = habit.question.clone();
        form.notes = habit.notes.clone();
        form.section = habit.section.clone();
        form.color = habit.color;
        form.skip_days = habit.skip_days;
        form.question_marks = habit.question_marks;
        form.icons = habit.icons;
        match habit.frequency {
            Frequency::Interval { times, days } if times >= days => {}
            Frequency::Interval { times: 1, days } => {
                form.kind = FrequencyKind::EveryXDays;
                form.every_days = days.to_string();
            }
            Frequency::Interval { times, days: 7 } => {
                form.kind = FrequencyKind::TimesPerWeek;
                form.per_week = times.to_string();
            }
            Frequency::Interval { times, days: 30 } => {
                form.kind = FrequencyKind::TimesPerMonth;
                form.per_month = times.to_string();
            }
            Frequency::Interval { times, days } => {
                form.kind = FrequencyKind::TimesInDays;
                form.in_times = times.to_string();
                form.in_days = days.to_string();
            }
            Frequency::TimesPerDay { times } => {
                form.kind = FrequencyKind::TimesPerDay;
                form.per_day = times.to_string();
            }
            Frequency::Weekdays { days } => {
                form.kind = FrequencyKind::Weekdays;
                form.weekdays = days;
            }
        }
        form
    }

    /// The value the form describes, if it records one.
    pub fn measure(&self) -> Result<Option<Measure>, String> {
        if !self.measured {
            return Ok(None);
        }
        let target = self.target.trim();
        let target = if target.is_empty() {
            None
        } else {
            Some(
                target
                    .parse::<f64>()
                    .ok()
                    .filter(|target| target.is_finite() && *target >= 0.0)
                    .ok_or("The target has to be a number of zero or more")?,
            )
        };
        Ok(Some(Measure {
            unit: self.unit.trim().to_owned(),
            target,
            days: self.target_days,
        }))
    }

    /// The goal the form describes.
    pub fn frequency(&self) -> Result<Frequency, String> {
        if self.habit_kind == Kind::No {
            return Ok(Frequency::DAILY);
        }
        let number = |text: &str| {
            text.trim()
                .parse::<u32>()
                .map_err(|_| "A frequency needs whole numbers".to_owned())
        };
        let frequency = match self.kind {
            FrequencyKind::EveryDay => Frequency::DAILY,
            FrequencyKind::TimesPerDay => Frequency::TimesPerDay {
                times: number(&self.per_day)?,
            },
            FrequencyKind::EveryXDays => Frequency::Interval {
                times: 1,
                days: number(&self.every_days)?,
            },
            FrequencyKind::TimesPerWeek => Frequency::Interval {
                times: number(&self.per_week)?,
                days: 7,
            },
            FrequencyKind::TimesPerMonth => Frequency::Interval {
                times: number(&self.per_month)?,
                days: 30,
            },
            FrequencyKind::TimesInDays => Frequency::Interval {
                times: number(&self.in_times)?,
                days: number(&self.in_days)?,
            },
            FrequencyKind::Weekdays => Frequency::Weekdays {
                days: self.weekdays,
            },
        };
        frequency.validate()?;
        Ok(frequency)
    }
}

/// A change on the habit form.
#[derive(Debug, Clone)]
pub enum FormAction {
    HabitKind(Kind),
    Measured(bool),
    Unit(String),
    Target(String),
    TargetDays(u32),
    Name(String),
    Question(String),
    Section(Option<String>),
    Color(u8),
    Kind(FrequencyKind),
    PerDay(String),
    EveryDays(String),
    PerWeek(String),
    PerMonth(String),
    InTimes(String),
    InDays(String),
    Weekday(usize),
    SkipDays(bool),
    QuestionMarks(bool),
    Icons(Icons),
    /// The notes, for a form whose notes field is plain text.
    Notes(String),
}

/// One of a day's two logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Log {
    /// Each time it was done, or each slip on a habit to avoid.
    Happened,
    /// On a habit to avoid: each time it came to mind and was not done.
    Resisted,
}

/// A log entry as it is being edited: the time and the amount as typed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogRow {
    /// Blank for an entry from before times were kept.
    pub time: String,
    /// Blank for no amount.
    pub amount: String,
    /// Blank for no comment.
    pub note: String,
}

impl LogRow {
    pub fn of(entry: LogEntry, format: TimeFormat) -> Self {
        Self {
            time: entry.at.map_or(String::new(), |at| {
                crate::calendar::time_format::clock(at, format)
            }),
            amount: entry
                .amount()
                .map_or(String::new(), crate::habits::format_amount),
            note: entry.note,
        }
    }

    /// The entry, or why it cannot be read.
    pub fn entry(&self, amounts: bool) -> Result<LogEntry, String> {
        let time = self.time.trim();
        let at = if time.is_empty() {
            None
        } else {
            Some(crate::habits::parse_log_time(time).ok_or_else(|| {
                format!("Could not read the time \"{time}\". Try 8:05pm or 20:05")
            })?)
        };
        let amount = self.amount.trim();
        let amount = if !amounts || amount.is_empty() {
            None
        } else {
            Some(
                crate::habits::parse_log_amount(amount)
                    .ok_or_else(|| format!("Could not read \"{amount}\" as a number"))?,
            )
        };
        Ok(LogEntry::new(at, amount).with_note(self.note.trim()))
    }
}

/// The dialog for one day, holding its edits until they are saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DayDialog {
    pub habit_id: String,
    pub day: NaiveDate,
    pub outcome: Outcome,
    /// Whether this is a habit to avoid, whose "Avoided it" and "Slipped"
    /// buttons each show their own log, and whose answer follows from them.
    pub avoid: bool,
    /// On a habit to avoid, which log shows: `NotDone` for "Avoided it",
    /// `Done` for "Slipped".
    pub view: Outcome,
    /// How many times make the day complete, on a habit to do.
    pub goal: u32,
    /// Whether each time it happened records an amount in the habit's unit.
    pub amounts: bool,
    /// Each time it happened. How many there are is how many times.
    pub happened: Vec<LogRow>,
    /// Each time it was resisted, on a habit to avoid.
    pub resisted: Vec<LogRow>,
    pub time_format: TimeFormat,
    /// What is typed for the value. Blank for no value.
    pub value: String,
    pub error: Option<String>,
}

impl DayDialog {
    pub fn new(habit: &Habit, day: NaiveDate, time_format: TimeFormat) -> Self {
        let entry = habit.entry(day);
        let avoid = habit.kind == Kind::No;
        let rows = |entries: Vec<LogEntry>| {
            entries
                .into_iter()
                .map(|entry| LogRow::of(entry, time_format))
                .collect::<Vec<_>>()
        };
        let happened = rows(habit.happened_log_on(day));
        Self {
            habit_id: habit.id.clone(),
            day,
            outcome: entry.outcome,
            avoid,
            view: if avoid && !happened.is_empty() {
                Outcome::Done
            } else {
                Outcome::NotDone
            },
            goal: habit.frequency.per_day(),
            amounts: habit.measure.is_some(),
            happened,
            resisted: rows(habit.resisted_log_on(day)),
            time_format,
            value: entry
                .value
                .map_or(String::new(), crate::habits::format_amount),
            error: None,
        }
    }

    /// How many times it happened.
    pub fn times(&self) -> u32 {
        self.happened.len() as u32
    }

    pub fn log(&self, log: Log) -> &[LogRow] {
        match log {
            Log::Happened => &self.happened,
            Log::Resisted => &self.resisted,
        }
    }

    pub fn log_mut(&mut self, log: Log) -> &mut Vec<LogRow> {
        match log {
            Log::Happened => &mut self.happened,
            Log::Resisted => &mut self.resisted,
        }
    }

    /// Whether the log of each time it happened is showing.
    pub fn shows_happened(&self) -> bool {
        if self.avoid {
            self.view == Outcome::Done && self.outcome != Outcome::Skipped
        } else {
            self.outcome == Outcome::Done
        }
    }

    /// What a habit to avoid's logs answer: slipping if any slip is logged.
    pub fn logged_outcome(&self) -> Outcome {
        if self.happened.is_empty() {
            Outcome::NotDone
        } else {
            Outcome::Done
        }
    }

    pub fn now_row(&self) -> LogRow {
        LogRow {
            time: crate::calendar::time_format::clock(now_time(), self.time_format),
            amount: String::new(),
            note: String::new(),
        }
    }

    /// Chooses an answer. On a habit to avoid, "Avoided it" and "Slipped"
    /// switch between their logs, which decide the answer. On a habit to do,
    /// having done it starts from the whole of the day's goal.
    pub fn choose(&mut self, outcome: Outcome) {
        self.error = None;
        if self.avoid {
            match outcome {
                Outcome::Done | Outcome::NotDone => {
                    self.view = outcome;
                    self.outcome = self.logged_outcome();
                }
                _ => self.outcome = outcome,
            }
            return;
        }
        self.outcome = outcome;
        if outcome == Outcome::Done {
            while self.times() < self.goal {
                let row = self.now_row();
                self.happened.push(row);
            }
        } else {
            self.happened.clear();
        }
    }

    /// Logs an entry at the current time, which answers the day: it happened,
    /// or, having been resisted, it was avoided.
    pub fn add(&mut self, log: Log) {
        let row = self.now_row();
        self.log_mut(log).push(row);
        self.error = None;
        match log {
            Log::Happened => {
                self.outcome = Outcome::Done;
                self.view = Outcome::Done;
            }
            Log::Resisted => {
                if matches!(self.outcome, Outcome::Unanswered | Outcome::Skipped) {
                    self.outcome = self.logged_outcome();
                }
            }
        }
    }

    /// Takes an entry out of a log. With nothing left that happened, it did not.
    pub fn remove(&mut self, log: Log, index: usize) {
        let rows = self.log_mut(log);
        if index < rows.len() {
            rows.remove(index);
        }
        self.error = None;
        if log == Log::Happened && self.happened.is_empty() && self.outcome == Outcome::Done {
            self.outcome = Outcome::NotDone;
        }
    }

    pub fn edit(&mut self, log: Log, index: usize, edit: impl FnOnce(&mut LogRow)) {
        if let Some(row) = self.log_mut(log).get_mut(index) {
            edit(row);
        }
        self.error = None;
    }

    /// The total of the amounts logged, when any are.
    pub fn logged_total(&self) -> Option<f64> {
        if !self.amounts {
            return None;
        }
        let amounts: Vec<f64> = self
            .happened
            .iter()
            .filter_map(|row| crate::habits::parse_log_amount(&row.amount))
            .collect();
        (!amounts.is_empty()).then(|| amounts.iter().sum())
    }

    /// What the dialog holds, with the day's two logs, unless something typed
    /// cannot be read.
    pub fn entry(&self) -> Result<(DayEntry, Vec<LogEntry>, Vec<LogEntry>), String> {
        let happened = if self.outcome == Outcome::Done {
            self.happened
                .iter()
                .map(|row| row.entry(self.amounts))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        let resisted = if self.avoid {
            self.resisted
                .iter()
                .map(|row| row.entry(self.amounts))
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        let logged: Vec<f64> = happened.iter().filter_map(LogEntry::amount).collect();
        // A habit to do that was not done has no value to record.
        let value = if !self.avoid && self.outcome == Outcome::NotDone {
            ""
        } else {
            self.value.trim()
        };
        let value = if !logged.is_empty() {
            Some(logged.iter().sum())
        } else if value.is_empty() {
            None
        } else {
            Some(
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite() && *value >= 0.0)
                    .ok_or("The value has to be a number of zero or more")?,
            )
        };
        let entry = DayEntry {
            outcome: self.outcome,
            times: happened.len() as u32,
            resisted: resisted.len() as u32,
            value,
        };
        Ok((entry, happened, resisted))
    }
}

/// The time of day to log something as happening at.
pub fn now_time() -> NaiveTime {
    crate::habits::log_time(crate::clock::now().time())
}

impl Default for Form {
    fn default() -> Self {
        Self::new()
    }
}

impl Form {
    /// Applies a change. Typing in a frequency's number also picks it.
    pub fn apply(&mut self, action: FormAction) {
        self.error = None;
        match action {
            FormAction::HabitKind(value) => {
                // A habit that exists keeps its kind.
                if self.id.is_none() {
                    self.habit_kind = value;
                }
            }
            FormAction::Measured(value) => self.measured = value,
            FormAction::Unit(value) => self.unit = value,
            FormAction::Target(value) => self.target = value,
            FormAction::TargetDays(value) => self.target_days = value,
            FormAction::Name(value) => self.name = value,
            FormAction::Question(value) => self.question = value,
            FormAction::Section(value) => self.section = value,
            FormAction::Color(value) => self.color = value,
            FormAction::Kind(value) => self.kind = value,
            FormAction::PerDay(value) => {
                self.kind = FrequencyKind::TimesPerDay;
                self.per_day = value;
            }
            FormAction::EveryDays(value) => {
                self.kind = FrequencyKind::EveryXDays;
                self.every_days = value;
            }
            FormAction::PerWeek(value) => {
                self.kind = FrequencyKind::TimesPerWeek;
                self.per_week = value;
            }
            FormAction::PerMonth(value) => {
                self.kind = FrequencyKind::TimesPerMonth;
                self.per_month = value;
            }
            FormAction::InTimes(value) => {
                self.kind = FrequencyKind::TimesInDays;
                self.in_times = value;
            }
            FormAction::InDays(value) => {
                self.kind = FrequencyKind::TimesInDays;
                self.in_days = value;
            }
            FormAction::Weekday(index) => {
                self.kind = FrequencyKind::Weekdays;
                if let Some(day) = self.weekdays.get_mut(index) {
                    *day = !*day;
                }
            }
            FormAction::SkipDays(value) => self.skip_days = value,
            FormAction::QuestionMarks(value) => self.question_marks = value,
            FormAction::Icons(value) => self.icons = value,
            FormAction::Notes(value) => self.notes = value,
        }
    }

    /// Validates the form and stores it in `library`, creating the habit if it
    /// is new. Returns the habit's id, or why the form cannot be saved.
    pub fn save(&self, library: &mut Library, today: NaiveDate) -> Result<String, String> {
        let name = self.name.trim().to_owned();
        if name.is_empty() {
            return Err("Give the habit a name".to_owned());
        }
        let measure = self.measure()?;
        let frequency = self.frequency()?;
        let id = self.id.clone().unwrap_or_else(new_id);
        if library.habit(&id).is_none() {
            let mut habit = Habit::new(id.clone(), name.clone());
            habit.kind = self.habit_kind;
            habit.created = Some(today);
            library.habits.push(habit);
        }
        if let Some(habit) = library.habit_mut(&id) {
            habit.measure = measure;
            habit.name = name;
            habit.question = self.question.trim().to_owned();
            habit.notes = self.notes.trim().to_owned();
            habit.section = self.section.clone();
            habit.color = self.color;
            habit.frequency = frequency;
            habit.skip_days = self.skip_days;
            habit.question_marks = self.question_marks;
            habit.icons = self.icons;
        }
        Ok(id)
    }
}

impl DayDialog {
    /// One more time it happened, or one fewer, never going below one.
    pub fn step_times(&mut self, up: bool) {
        if up {
            self.add(Log::Happened);
        } else if self.happened.len() > 1 {
            self.remove(Log::Happened, self.happened.len() - 1);
        }
    }

    /// One more time it was resisted, or one fewer.
    pub fn step_resisted(&mut self, up: bool) {
        if up {
            self.add(Log::Resisted);
        } else if let Some(last) = self.resisted.len().checked_sub(1) {
            self.remove(Log::Resisted, last);
        }
    }

    /// What is typed for the value. A number on a habit to do means it was done.
    pub fn set_value(&mut self, value: String) {
        let some = value.trim().parse::<f64>().is_ok_and(|value| value > 0.0);
        self.value = value;
        self.error = None;
        if !self.avoid && some && matches!(self.outcome, Outcome::NotDone | Outcome::Unanswered) {
            self.choose(Outcome::Done);
        }
    }

    /// Records what the dialog holds on its habit in `library`, unless
    /// something typed cannot be read.
    pub fn record(&self, library: &mut Library) -> Result<(), String> {
        let (entry, happened, resisted) = self.entry()?;
        if let Some(habit) = library.habit_mut(&self.habit_id) {
            habit.set_entry(self.day, entry);
            habit.set_log(self.day, &happened, &resisted);
        }
        Ok(())
    }
}

/// A section being named, in place in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionEdit {
    /// `None` when this is a new section.
    pub id: Option<String>,
    pub name: String,
}

impl SectionEdit {
    /// Names the section, or adds it. A blank name changes nothing, and
    /// returns false.
    pub fn save(&self, library: &mut Library) -> bool {
        let name = self.name.trim().to_owned();
        if name.is_empty() {
            return false;
        }
        match &self.id {
            Some(id) => {
                if let Some(section) = library.section_mut(id) {
                    section.name = name;
                }
            }
            None => library.sections.push(Section {
                id: new_id(),
                name,
                collapsed: false,
            }),
        }
        true
    }
}

/// An id for a new habit or section.
pub fn new_id() -> String {
    let mut bytes = [0_u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // Only uniqueness among this person's habits matters.
        let now = crate::clock::now().timestamp_micros();
        bytes = now.to_le_bytes();
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
