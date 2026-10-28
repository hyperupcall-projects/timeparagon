//! Habits: recurring events on the app's single TimeSlay calendar.
//!
//! A habit is an ordinary Google Calendar event tagged with `ttKind=habit`,
//! so Google expands it and every other client sees a normal recurring event.
//! The Habits tab edits the master rather than an instance. Its repeat
//! controls are the shared [`crate::recurrence`] editor, the same one events
//! and time blocks use, so every recurring item is described in the same
//! words and written with the same `RRULE`/`RDATE`/`EXDATE` logic.

use chrono::{DateTime, Local, NaiveDate, NaiveTime, Timelike, Weekday};
use iced::{Color, Task};

use crate::app::{Message, State};
use crate::blocking;
use crate::calendar::{self, api::HabitEvent, colors, deadline};
use crate::recurrence::Rule;

pub use crate::recurrence::{EndKind, MonthlyMode, Recurrence, RepeatMode, WEEKDAYS};

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

    /// A readable description of the recurrence, or "Does not repeat".
    pub fn recurrence_label(&self) -> String {
        if self.recurrence.is_empty() {
            return "Does not repeat".to_owned();
        }
        Draft::from_habit(self)
            .rule
            .summary(self.start.date_naive())
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
    /// The shared repeat controls.
    pub rule: Rule,
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
            rule: Rule::weekly(day),
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

    /// The same, carrying the repeat schedule already chosen in the new-event
    /// dialog. A habit always repeats, so an empty schedule becomes weekly on
    /// the start day.
    pub fn for_calendar_with_rule(
        start: DateTime<Local>,
        end: DateTime<Local>,
        title: String,
        notes: String,
        color_id: Option<String>,
        rule: Rule,
    ) -> Self {
        let mut draft = Self::for_calendar(start, end, title, notes, color_id);
        draft.rule = rule;
        if draft.rule.repeat == RepeatMode::Never {
            draft.rule = Rule::weekly(start.date_naive());
        }
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
        draft.rule = Rule::from_recurrence(habit.start.date_naive(), &habit.recurrence);
        draft
    }

    /// The anchor day for the repeat controls.
    pub fn anchor_date(&self) -> Option<NaiveDate> {
        deadline::parse_date(self.date.trim()).ok()
    }

    /// Reads an imported recurrence into the structured fields. Returns false
    /// for anything the editor cannot show without RFC text, which then stays
    /// untouched as a custom schedule.
    pub fn apply_recurrence(&mut self, recurrence: &Recurrence) -> bool {
        let Some(anchor) = self.anchor_date() else {
            return false;
        };
        self.rule.apply(anchor, recurrence)
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
        let date = deadline::parse_date(self.date.trim())?;
        self.rule.lines_at(date, start)
    }

    /// A short, human description used by the cards.
    pub fn repeat_summary(&self) -> String {
        let anchor = self
            .anchor_date()
            .unwrap_or_else(|| Local::now().date_naive());
        self.rule.summary(anchor)
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
    /// A change made in the shared repeat controls.
    Recurrence(crate::recurrence::Action),
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

        Action::Repeat(mode) => edit(state, |draft| {
            let anchor = draft
                .anchor_date()
                .unwrap_or_else(|| Local::now().date_naive());
            draft
                .rule
                .apply_action(crate::recurrence::Action::Repeat(mode), anchor);
        }),
        Action::Interval(value) => edit(state, |draft| draft.rule.interval = value),
        Action::ToggleWeekday(index) => edit(state, |draft| {
            let anchor = draft
                .anchor_date()
                .unwrap_or_else(|| Local::now().date_naive());
            draft
                .rule
                .apply_action(crate::recurrence::Action::ToggleWeekday(index), anchor);
        }),
        Action::MonthlyMode(mode) => edit(state, |draft| draft.rule.monthly = mode),
        Action::MonthlyDay(value) => edit(state, |draft| draft.rule.monthly_day = value),
        Action::MonthlyNth(nth) => edit(state, |draft| {
            draft.rule.monthly = MonthlyMode::NthWeekday;
            draft.rule.monthly_nth = nth;
        }),
        Action::MonthlyWeekday(day) => edit(state, |draft| {
            draft.rule.monthly = MonthlyMode::NthWeekday;
            draft.rule.monthly_weekday = day;
        }),
        Action::Ends(kind) => edit(state, |draft| draft.rule.ends = kind),
        Action::Until(value) => edit(state, |draft| draft.rule.until = value),
        Action::Count(value) => edit(state, |draft| draft.rule.count = value),
        Action::SkipInput(value) => edit(state, |draft| draft.rule.skip_input = value),

        Action::SkipAdd => {
            if let Some(draft) = state.habits.draft.as_mut() {
                match draft.rule.add_skip() {
                    Ok(()) => draft.error = None,
                    Err(error) => draft.error = Some(error),
                }
            }
            Task::none()
        }

        Action::SkipRemove(day) => {
            edit(state, |draft| draft.rule.skips.retain(|date| *date != day))
        }

        Action::SimpleRepeat => edit(state, |draft| {
            let anchor = draft
                .anchor_date()
                .unwrap_or_else(|| Local::now().date_naive());
            draft
                .rule
                .apply_action(crate::recurrence::Action::SimpleRepeat, anchor);
        }),

        Action::Recurrence(action) => {
            if let Some(draft) = state.habits.draft.as_mut() {
                if matches!(&action, crate::recurrence::Action::SkipAdd) {
                    match draft.rule.add_skip() {
                        Ok(()) => draft.error = None,
                        Err(error) => draft.error = Some(error),
                    }
                } else {
                    let anchor = draft
                        .anchor_date()
                        .unwrap_or_else(|| Local::now().date_naive());
                    draft.rule.apply_action(action, anchor);
                    draft.error = None;
                }
            }
            Task::none()
        }

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
    use crate::recurrence::weekday_flags;

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
            draft.rule.custom = None;
            let rewritten = Recurrence::from_lines(&lines);
            assert!(draft.apply_recurrence(&rewritten), "{rule}");
            assert_eq!(draft.repeat_summary(), expected, "{rule}");
        }
    }

    #[test]
    fn ends_and_skips_are_written_without_showing_rfc_text() {
        let mut draft = draft();
        draft.rule.repeat = RepeatMode::Weekly;
        draft.rule.weekdays = weekday_flags(&[Weekday::Mon, Weekday::Wed]);
        draft.rule.interval = "2".into();
        draft.rule.ends = EndKind::OnDate;
        draft.rule.until = "2026-12-31".into();
        draft.rule.skips = vec![date("2026-11-11")];
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
        draft.rule.repeat = RepeatMode::Weekly;
        draft.rule.weekdays = [false; 7];
        assert!(draft.parsed().is_err());
        draft.rule.monthly = MonthlyMode::Day;
        draft.rule.monthly_day = "32".into();
        draft.rule.repeat = RepeatMode::Monthly;
        assert!(draft.parsed().is_err());
        draft.rule.monthly_day = "28".into();
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
        assert_eq!(draft.rule.repeat, RepeatMode::Custom);
        assert_eq!(draft.repeat_summary(), "Custom schedule");
        assert_eq!(draft.recurrence_lines().unwrap(), recurrence.lines());

        draft.rule.repeat = RepeatMode::Weekly;
        draft.rule.custom = None;
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
        assert_eq!(draft.rule.repeat, RepeatMode::Weekly);
        assert_eq!(draft.rule.extra, [date("2026-11-02"), date("2026-11-09")]);
        assert_eq!(draft.rule.skips, [date("2026-11-11")]);
        let written = draft.recurrence_lines().unwrap();
        assert!(written.contains(&"RDATE:20261102090000".to_owned()));
        assert!(written.contains(&"RDATE:20261109090000".to_owned()));
        assert!(written.contains(&"EXDATE:20261111090000".to_owned()));
    }

    #[test]
    fn a_malformed_schedule_is_reported_before_anything_is_written() {
        let mut draft = draft();
        draft.rule.repeat = RepeatMode::Daily;
        draft.rule.interval = "0".into();
        assert!(draft.parsed().is_err());
        draft.rule.interval = "1".into();
        draft.rule.ends = EndKind::After;
        draft.rule.count = "0".into();
        assert!(draft.parsed().is_err());
        draft.rule.count = "5".into();
        draft.rule.ends = EndKind::OnDate;
        draft.rule.until = "2026-10-01".into();
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
