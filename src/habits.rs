//! Habits: recurring events on the app's single TimeSlay calendar.
//!
//! A habit is an ordinary Google Calendar event tagged with `ttKind=habit`,
//! so Google expands it and every other client sees a normal recurring event.
//! The Habits tab edits the master rather than an instance, which is what lets
//! it expose the whole RFC 5545 recurrence array: `RRULE`, `RDATE` and
//! `EXDATE` lines.

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime, Timelike};
use iced::{Color, Task};

use crate::app::{Message, State};
use crate::blocking;
use crate::calendar::{self, api::HabitEvent, colors, deadline};

/// The `ttKind` Google extended property that marks a habit event.
pub const KIND: &str = "habit";

/// The text input that holds a habit's name in the editor.
pub const TITLE_FIELD: &str = "habit-title";

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

    /// A one-line description of the recurrence, or "Does not repeat".
    pub fn recurrence_label(&self) -> String {
        if self.recurrence.is_empty() {
            "Does not repeat".to_owned()
        } else {
            self.recurrence.lines().join(" · ")
        }
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
/// write, so any rule Google accepts can be round-tripped untouched.
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

    /// The first `RRULE` value without its prefix, for the editor.
    pub fn rrule(&self) -> String {
        self.lines
            .iter()
            .find_map(|line| strip_prefix(line, "RRULE:"))
            .unwrap_or("")
            .to_owned()
    }

    /// Every `RDATE` line, one per line, for the editor.
    pub fn rdates(&self) -> String {
        self.by_prefix("RDATE")
    }

    /// Every `EXDATE` line, one per line, for the editor.
    pub fn exdates(&self) -> String {
        self.by_prefix("EXDATE")
    }

    fn by_prefix(&self, kind: &str) -> String {
        self.lines
            .iter()
            .filter(|line| starts_with_kind(line, kind))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
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
    /// The `RRULE` value, without the `RRULE:` prefix.
    pub rrule: String,
    /// One `RDATE` line per newline or semicolon.
    pub rdates: String,
    /// One `EXDATE` line per newline or semicolon.
    pub exdates: String,
    pub color_id: Option<String>,
    pub error: Option<String>,
}

impl Draft {
    pub fn new(day: NaiveDate, color_id: Option<String>) -> Self {
        Self {
            id: None,
            calendar_id: None,
            title: String::new(),
            notes: String::new(),
            date: day.to_string(),
            start: "09:00".to_owned(),
            end: "09:30".to_owned(),
            rrule: "FREQ=WEEKLY".to_owned(),
            rdates: String::new(),
            exdates: String::new(),
            color_id,
            error: None,
        }
    }

    pub fn from_habit(habit: &Habit) -> Self {
        Self {
            id: Some(habit.id.clone()),
            calendar_id: Some(habit.calendar_id.clone()),
            title: habit.title.clone(),
            notes: habit.notes.clone(),
            date: habit.start.date_naive().to_string(),
            start: habit.start.format("%H:%M").to_string(),
            end: habit.end.format("%H:%M").to_string(),
            rrule: habit.recurrence.rrule(),
            rdates: habit.recurrence.rdates(),
            exdates: habit.recurrence.exdates(),
            color_id: habit.color_id.clone(),
            error: None,
        }
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
        let recurrence = self.recurrence_lines()?;
        Ok(ParsedHabit {
            title: title.to_owned(),
            date,
            start,
            end,
            recurrence,
        })
    }

    /// Builds and validates the full recurrence array, with prefixes added
    /// where the user left them off.
    pub fn recurrence_lines(&self) -> Result<Vec<String>, String> {
        let mut lines = Vec::new();
        let rrule = self.rrule.trim();
        let rrule = rrule
            .strip_prefix("RRULE:")
            .or_else(|| rrule.strip_prefix("rrule:"))
            .unwrap_or(rrule)
            .trim();
        if !rrule.is_empty() {
            validate_rrule(rrule)?;
            lines.push(format!("RRULE:{rrule}"));
        }
        lines.extend(validate_date_lines(&self.rdates, "RDATE")?);
        lines.extend(validate_date_lines(&self.exdates, "EXDATE")?);
        Ok(lines)
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

/// Shortcuts that seed the raw RRULE editor. Every rule remains editable after
/// a shortcut is chosen, so these are starting points rather than limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Daily,
    Weekdays,
    Weekly,
    Monthly,
    Yearly,
    None,
}

impl Preset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Daily => "Daily",
            Self::Weekdays => "Weekdays",
            Self::Weekly => "Weekly",
            Self::Monthly => "Monthly",
            Self::Yearly => "Yearly",
            Self::None => "One-off",
        }
    }

    pub fn rrule(self, date: Option<NaiveDate>) -> String {
        let date = date.unwrap_or_else(|| Local::now().date_naive());
        match self {
            Self::Daily => "FREQ=DAILY".to_owned(),
            Self::Weekdays => "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_owned(),
            Self::Weekly => format!("FREQ=WEEKLY;BYDAY={}", weekday_code(date)),
            Self::Monthly => format!("FREQ=MONTHLY;BYMONTHDAY={}", date.day()),
            Self::Yearly => "FREQ=YEARLY".to_owned(),
            Self::None => String::new(),
        }
    }
}

fn weekday_code(date: NaiveDate) -> &'static str {
    match date.weekday() {
        chrono::Weekday::Mon => "MO",
        chrono::Weekday::Tue => "TU",
        chrono::Weekday::Wed => "WE",
        chrono::Weekday::Thu => "TH",
        chrono::Weekday::Fri => "FR",
        chrono::Weekday::Sat => "SA",
        chrono::Weekday::Sun => "SU",
    }
}

fn validate_date_lines(value: &str, kind: &str) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for line in value.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let before = lines.len();
        let rest = if starts_with_kind(line, kind) {
            line[kind.len()..].trim()
        } else {
            line
        };
        if rest.starts_with(';') {
            // A parameter section makes the whole line one RFC 5545 entry:
            // `RDATE;VALUE=DATE:20261102,20261109`.
            lines.push(date_line(kind, rest)?);
        } else {
            // The editor also accepts several bare dates separated by
            // semicolons, with or without the `RDATE:` prefix.
            for part in rest.trim_start_matches(':').split(';') {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                // A second prefixed line may follow a semicolon.
                let part = if starts_with_kind(part, kind) {
                    part[kind.len()..].trim_start_matches(':').trim()
                } else {
                    part
                };
                if !part.is_empty() {
                    lines.push(date_line(kind, part)?);
                }
            }
        }
        if lines.len() == before {
            return Err(format!("{kind} needs at least one date"));
        }
    }
    Ok(lines)
}

/// One `RDATE` or `EXDATE` line, with any parameters kept in front of the
/// colon.
fn date_line(kind: &str, rest: &str) -> Result<String, String> {
    let rest = rest.trim();
    let (params, values) = if let Some(after) = rest.strip_prefix(';') {
        let (params, values) = after
            .split_once(':')
            .ok_or_else(|| format!("{kind} parameters need a `:` before the dates"))?;
        (Some(params.trim()), values.trim())
    } else {
        (None, rest.trim_start_matches(':').trim())
    };
    if values.is_empty() || values.split(',').any(|value| value.trim().is_empty()) {
        return Err(format!("{kind} has an empty date"));
    }
    Ok(match params.filter(|params| !params.is_empty()) {
        Some(params) => format!("{kind};{params}:{values}"),
        None => format!("{kind}:{values}"),
    })
}

fn validate_rrule(rrule: &str) -> Result<(), String> {
    let mut has_freq = false;
    for part in rrule.split(';') {
        let part = part.trim();
        if part.is_empty() {
            return Err("RRULE has an empty part".to_owned());
        }
        let (key, value) = part
            .split_once('=')
            .ok_or_else(|| format!("RRULE part `{part}` needs a value"))?;
        let key = key.trim().to_ascii_uppercase();
        let value = value.trim();
        if value.is_empty() {
            return Err(format!("RRULE part `{key}` needs a value"));
        }
        match key.as_str() {
            "FREQ" => {
                has_freq = true;
                if !matches!(
                    value.to_ascii_uppercase().as_str(),
                    "SECONDLY" | "MINUTELY" | "HOURLY" | "DAILY" | "WEEKLY" | "MONTHLY" | "YEARLY"
                ) {
                    return Err(format!("Unknown RRULE frequency `{value}`"));
                }
            }
            "INTERVAL" | "COUNT" => {
                value
                    .parse::<u32>()
                    .ok()
                    .filter(|number| *number > 0)
                    .ok_or_else(|| format!("RRULE {key} has to be a positive number"))?;
            }
            "BYDAY" => {
                for day in value.split(',') {
                    if !valid_byday(day.trim()) {
                        return Err(format!("Unknown RRULE weekday `{day}`"));
                    }
                }
            }
            "BYMONTHDAY" => {
                for day in value.split(',') {
                    let day: i32 = day
                        .trim()
                        .parse()
                        .map_err(|_| format!("Invalid RRULE BYMONTHDAY `{day}`"))?;
                    if day == 0 || day.abs() > 31 {
                        return Err(format!("Invalid RRULE BYMONTHDAY `{day}`"));
                    }
                }
            }
            "BYMONTH" => {
                for month in value.split(',') {
                    let month: u32 = month
                        .trim()
                        .parse()
                        .map_err(|_| format!("Invalid RRULE BYMONTH `{month}`"))?;
                    if !(1..=12).contains(&month) {
                        return Err(format!("Invalid RRULE BYMONTH `{month}`"));
                    }
                }
            }
            "BYSETPOS" => {
                for position in value.split(',') {
                    let position: i32 = position
                        .trim()
                        .parse()
                        .map_err(|_| format!("Invalid RRULE BYSETPOS `{position}`"))?;
                    if position == 0 {
                        return Err("RRULE BYSETPOS cannot be zero".to_owned());
                    }
                }
            }
            "WKST" => {
                if !valid_weekday(value) {
                    return Err(format!("Unknown RRULE week start `{value}`"));
                }
            }
            "UNTIL" => {
                if !valid_until(value) {
                    return Err(format!("Invalid RRULE UNTIL `{value}`"));
                }
            }
            // The remaining RFC 5545 parts (BYHOUR, BYMINUTE, BYSECOND,
            // BYWEEKNO, BYYEARDAY, and any future extension) are passed
            // through so the editor is not a subset of the standard.
            _ => {}
        }
    }
    if !has_freq {
        return Err("RRULE needs a FREQ part".to_owned());
    }
    Ok(())
}

fn valid_weekday(value: &str) -> bool {
    matches!(
        value.to_ascii_uppercase().as_str(),
        "MO" | "TU" | "WE" | "TH" | "FR" | "SA" | "SU"
    )
}

fn valid_byday(value: &str) -> bool {
    if value.len() < 2 || !value.is_ascii() {
        return false;
    }
    let (prefix, day) = value.split_at(value.len() - 2);
    valid_weekday(day)
        && (prefix.is_empty()
            || prefix
                .parse::<i32>()
                .is_ok_and(|number| (1..=53).contains(&number.abs())))
}

fn valid_until(value: &str) -> bool {
    let value = value.trim_end_matches('Z');
    matches!(value.len(), 8 | 15) && value.chars().all(|ch| ch.is_ascii_digit() || ch == 'T')
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
    Rule(String),
    Rdates(String),
    Exdates(String),
    Preset(Preset),
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
        Action::Rule(value) => edit(state, |draft| draft.rrule = value),
        Action::Rdates(value) => edit(state, |draft| draft.rdates = value),
        Action::Exdates(value) => edit(state, |draft| draft.exdates = value),
        Action::Color(color) => edit(state, |draft| {
            draft.color_id = color.and_then(colors::event_id).map(str::to_owned);
        }),

        Action::Preset(preset) => {
            if let Some(draft) = state.habits.draft.as_mut() {
                let date = deadline::parse_date(draft.date.trim()).ok();
                draft.rrule = preset.rrule(date);
                draft.error = None;
            }
            Task::none()
        }

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
    fn recurrence_lines_round_trip_without_losing_rdate_or_exdate() {
        let lines = vec![
            "RRULE:FREQ=WEEKLY;BYDAY=MO,WE;INTERVAL=2".to_owned(),
            "RDATE;VALUE=DATE:20261102,20261109".to_owned(),
            "EXDATE;TZID=Europe/London:20261111T090000".to_owned(),
        ];
        let recurrence = Recurrence::from_lines(&lines);
        assert_eq!(recurrence.lines(), lines);
        assert_eq!(recurrence.rrule(), "FREQ=WEEKLY;BYDAY=MO,WE;INTERVAL=2");
        assert_eq!(recurrence.rdates(), "RDATE;VALUE=DATE:20261102,20261109");
        assert_eq!(
            recurrence.exdates(),
            "EXDATE;TZID=Europe/London:20261111T090000"
        );
    }

    #[test]
    fn the_editor_builds_every_recurrence_line() {
        let mut draft = draft();
        draft.rrule = "FREQ=MONTHLY;BYMONTHDAY=1;COUNT=5".into();
        draft.rdates = "20261201T090000;20270101T090000".into();
        draft.exdates = "RDATE:20270101T090000".into();
        // The typo in the last field is on purpose: an EXDATE prefix is added.
        draft.exdates = "EXDATE:20270201T090000".into();
        let lines = draft.recurrence_lines().unwrap();
        assert_eq!(
            lines,
            [
                "RRULE:FREQ=MONTHLY;BYMONTHDAY=1;COUNT=5",
                "RDATE:20261201T090000",
                "RDATE:20270101T090000",
                "EXDATE:20270201T090000",
            ]
        );
    }

    #[test]
    fn the_full_rrule_editor_accepts_standard_and_extension_parts() {
        for rule in [
            "FREQ=DAILY;INTERVAL=3",
            "FREQ=WEEKLY;BYDAY=MO,WE,FR;WKST=SU",
            "FREQ=MONTHLY;BYMONTHDAY=-1;BYSETPOS=1",
            "FREQ=YEARLY;BYMONTH=10;BYMONTHDAY=28;UNTIL=20301231T235959Z",
            "FREQ=WEEKLY;BYHOUR=9;BYMINUTE=30;BYSECOND=0",
        ] {
            assert!(validate_rrule(rule).is_ok(), "{rule}");
        }
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
    fn the_editor_keeps_rdate_and_exdate_parameters() {
        let mut draft = draft();
        draft.rrule = "FREQ=WEEKLY".into();
        draft.rdates = "RDATE;VALUE=DATE:20261102,20261109".into();
        draft.exdates = "EXDATE;TZID=Europe/London:20261111T090000".into();
        assert_eq!(
            draft.recurrence_lines().unwrap(),
            [
                "RRULE:FREQ=WEEKLY",
                "RDATE;VALUE=DATE:20261102,20261109",
                "EXDATE;TZID=Europe/London:20261111T090000",
            ]
        );
    }

    #[test]
    fn a_malformed_rule_is_reported_before_anything_is_written() {
        for rule in [
            "BYDAY=MO",
            "FREQ=SOMETIMES",
            "FREQ=WEEKLY;INTERVAL=0",
            "FREQ=WEEKLY;BYDAY=XX",
            "FREQ=MONTHLY;BYMONTHDAY=32",
            "FREQ=WEEKLY;UNTIL=tomorrow",
            "",
        ] {
            let mut draft = draft();
            draft.rrule = rule.into();
            if rule.is_empty() {
                assert!(draft.recurrence_lines().is_ok());
            } else {
                assert!(draft.recurrence_lines().is_err(), "{rule}");
            }
        }
    }

    #[test]
    fn a_preset_only_seeds_the_editor_and_can_be_changed_further() {
        let mut draft = draft();
        draft.rrule = Preset::Weekdays.rrule(Some(date("2026-10-28")));
        assert_eq!(draft.rrule, "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR");
        draft.rrule = Preset::Monthly.rrule(Some(date("2026-10-28")));
        assert_eq!(draft.rrule, "FREQ=MONTHLY;BYMONTHDAY=28");
        draft.rrule = Preset::None.rrule(Some(date("2026-10-28")));
        assert!(draft.rrule.is_empty());
        assert!(draft.recurrence_lines().unwrap().is_empty());
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
    fn the_recurrence_label_is_readable_without_hiding_the_rule() {
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
        assert_eq!(habit.recurrence_label(), "RRULE:FREQ=DAILY");
        assert_eq!(habit.time_label(), "09:00 – 09:30");
        let one_off = Habit {
            recurrence: Recurrence::default(),
            ..habit
        };
        assert_eq!(one_off.recurrence_label(), "Does not repeat");
    }
}
