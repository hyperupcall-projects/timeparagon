//! "View in calendar": the items about to be created, laid over the day or
//! week they land in, before anything is written.
//!
//! The preview holds a list of [`Proposal`]s rather than reading the form
//! directly, so anything that comes up with several items at once (such as
//! voice input) can show them all with [`open`].
use super::*;
use crate::recurrence::{EndKind, RepeatMode, Rule};
use calendar::DraftKind;
use chrono::{DateTime, Datelike, Days, NaiveTime};
use iced::Color;

/// The span of time the preview shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Span {
    #[default]
    Day,
    Week,
}

/// Something that would be created, as the preview draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub kind: DraftKind,
    pub title: String,
    /// None for an item with no date yet, such as a task without a due date.
    pub when: Option<When>,
    pub color: Color,
}

/// When a proposal happens. A repeating one is its first occurrence plus the
/// rule that produces the rest.
#[derive(Debug, Clone, PartialEq)]
pub struct When {
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub all_day: bool,
    pub repeat: Option<Rule>,
}

/// The open preview.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub proposals: Vec<Proposal>,
    pub span: Span,
    /// The day in view, or a day of the week in view.
    pub anchor: NaiveDate,
    /// The proposal picked from the list, drawn with a ring.
    pub focus: Option<usize>,
    /// Whether the proposals came from the form, so its Add button applies.
    pub from_form: bool,
}

#[derive(Debug, Clone)]
pub enum PreviewAction {
    /// Show what the form would create.
    Open,
    Close,
    Span(Span),
    /// Move by this many days or weeks, depending on the span.
    Shift(i32),
    /// Pick a proposal: it is brought into view and ringed.
    Focus(usize),
    /// Open the day view on this day, from a week column's heading.
    ShowDay(NaiveDate),
}

/// The scrollable holding the preview's hour grid.
pub const GRID_SCROLL: &str = "preview-grid";

impl Proposal {
    pub fn kind_label(&self) -> &'static str {
        kind_label(self.kind)
    }

    /// The occurrences that touch `day`, as start and end instants.
    ///
    /// The day's window opens `day_end_minutes` after midnight, as on the
    /// calendar, so an item at 1am with a 3am end of day belongs to the
    /// evening before. A repeating item is tried on the neighbouring dates too,
    /// so an occurrence that runs past midnight still shows where it spills.
    pub fn occurrences_on(
        &self,
        day: NaiveDate,
        day_end_minutes: i32,
    ) -> Vec<(DateTime<Local>, DateTime<Local>)> {
        let Some(when) = &self.when else {
            return Vec::new();
        };
        let length = when.end - when.start;
        let starts: Vec<DateTime<Local>> = match &when.repeat {
            Some(rule) if rule.repeats() => {
                let anchor = when.start.date_naive();
                let time = when.start.time();
                // An all-day item covers whole days: look back far enough to
                // catch one that started earlier and runs on.
                let back = if when.all_day {
                    length.num_days().max(1) - 1
                } else {
                    1
                };
                (-back..=1)
                    .filter_map(|offset| day.checked_add_signed(Duration::days(offset)))
                    .filter(|date| repeats_on(rule, anchor, *date))
                    .filter_map(|date| at_time(date, time))
                    .collect()
            }
            _ => vec![when.start],
        };
        let opens = calendar::at_clock(day, 0, 0) + Duration::minutes(i64::from(day_end_minutes));
        let closes = opens + Duration::days(1);
        starts
            .into_iter()
            .map(|start| (start, start + length))
            .filter(|(start, end)| {
                if when.all_day {
                    let first = start.date_naive();
                    first <= day && day < end.date_naive().max(first + Days::new(1))
                } else {
                    *start < closes && *end > opens
                }
            })
            .collect()
    }

    /// The first day the proposal shows on, if it has a date.
    pub fn first_day(&self) -> Option<NaiveDate> {
        self.when.as_ref().map(|when| when.start.date_naive())
    }
}

pub fn kind_label(kind: DraftKind) -> &'static str {
    match kind {
        DraftKind::Task => "Task",
        DraftKind::Event => "Event",
        DraftKind::TimeBlock => "Time block",
        DraftKind::Habit => "Habit",
        DraftKind::Sleep => "Sleep",
    }
}

fn at_time(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Local>> {
    date.and_time(time).and_local_timezone(Local).earliest()
}

/// Whether a simple repeat rule anchored on `anchor` falls on `day`. Custom
/// rules are not expanded: they only show their first occurrence.
pub(crate) fn repeats_on(rule: &Rule, anchor: NaiveDate, day: NaiveDate) -> bool {
    if day < anchor {
        return false;
    }
    if rule.skips.contains(&day) {
        return false;
    }
    match rule.ends {
        EndKind::OnDate => {
            if let Ok(until) = deadline::parse_date(rule.until.trim())
                && day > until
            {
                return false;
            }
        }
        EndKind::After | EndKind::Never => {}
    }
    let every = rule.interval.trim().parse::<i64>().unwrap_or(1).max(1);
    let days = (day - anchor).num_days();
    let weeks = (week_start(day) - week_start(anchor)).num_days() / 7;
    let months = i64::from(day.year() - anchor.year()) * 12 + i64::from(day.month())
        - i64::from(anchor.month());
    let hit = match rule.repeat {
        RepeatMode::Never | RepeatMode::Custom => day == anchor,
        RepeatMode::Daily => days % every == 0,
        RepeatMode::Weekdays => day.weekday().number_from_monday() <= 5 && weeks % every == 0,
        RepeatMode::Weekly => {
            rule.weekdays[day.weekday().num_days_from_monday() as usize] && weeks % every == 0
        }
        RepeatMode::Monthly => months % every == 0 && day.day() == anchor.day(),
        RepeatMode::Yearly => {
            (day.year() - anchor.year()) as i64 % every == 0
                && day.month() == anchor.month()
                && day.day() == anchor.day()
        }
    };
    hit || (rule.extra.contains(&day))
}

fn week_start(day: NaiveDate) -> NaiveDate {
    day - Duration::days(i64::from(day.weekday().num_days_from_monday()))
}

/// What the open form would create.
pub fn from_form(state: &State) -> Vec<Proposal> {
    let Some(draft) = &state.inbox.draft else {
        return Vec::new();
    };
    if draft.kind.is_some_and(|kind| kind != DraftKind::Task) {
        return state
            .calendar
            .draft
            .as_ref()
            .map(|entry| vec![from_calendar_draft(state, entry)])
            .unwrap_or_default();
    }
    let tokens = draft.quick_tokens(&state.config.projects);
    let title = crate::task_input::remove(&draft.title, tokens.into_iter().map(|t| t.range));
    let today = crate::clock::now().date_naive();
    let when = crate::task_input::date(state.timeblocking.date.trim(), today).map(|day| {
        let start = calendar::at_clock(day, 0, 0);
        When {
            start,
            end: start + Duration::days(1),
            all_day: true,
            repeat: None,
        }
    });
    vec![Proposal {
        kind: DraftKind::Task,
        title: title.trim().to_owned(),
        when,
        color: state.config.calendar.task_event_color(),
    }]
}

/// A proposal for an event, time block, habit or sleep being filled in.
pub fn from_calendar_draft(state: &State, draft: &calendar::Draft) -> Proposal {
    let settings = &state.config.calendar;
    let color = match draft.kind {
        DraftKind::Event => settings
            .calendar_colors
            .get(&draft.calendar_id)
            .map(|color| color.0)
            .or_else(|| {
                state
                    .calendar
                    .calendars
                    .iter()
                    .find(|calendar| calendar.id == draft.calendar_id)
                    .map(|calendar| calendar.color)
            })
            .unwrap_or_else(|| settings.event_color()),
        DraftKind::TimeBlock => settings.time_block_color(),
        DraftKind::Habit => settings.habit_color(),
        DraftKind::Sleep => settings.sleep_color(),
        DraftKind::Task => settings.task_event_color(),
    };
    let title = match draft.kind {
        DraftKind::Sleep => "Sleep".to_owned(),
        _ => draft.title.trim().to_owned(),
    };
    Proposal {
        kind: draft.kind,
        title,
        when: Some(When {
            start: draft.start,
            end: draft.end.max(draft.start + Duration::minutes(15)),
            all_day: draft.all_day,
            repeat: draft.rule.repeats().then(|| draft.rule.clone()),
        }),
        color,
    }
}

/// Opens the preview on a set of proposals. The first one with a date is
/// brought into view; the week is shown when they fall on different days.
pub fn open(state: &mut State, proposals: Vec<Proposal>, from_form: bool) -> Task<Message> {
    let days: Vec<NaiveDate> = proposals.iter().filter_map(Proposal::first_day).collect();
    let today = calendar::day_at(crate::clock::now(), state.config.calendar.day_start_hour());
    let anchor = days.iter().min().copied().unwrap_or(today);
    let span = if days.iter().any(|day| *day != anchor) {
        Span::Week
    } else {
        Span::Day
    };
    let focus = proposals.iter().position(|p| p.when.is_some());
    state.inbox.preview = Some(Preview {
        proposals,
        span,
        anchor,
        focus,
        from_form,
    });
    scroll_to_focus(state)
}

pub(super) fn update(state: &mut State, action: PreviewAction) -> Task<Message> {
    if let PreviewAction::Open = action {
        if state.inbox.draft.as_ref().is_none_or(|draft| draft.saving) {
            return Task::none();
        }
        // The task form keeps its dates in the borrowed workspace; the title's
        // quick-add words are read from the title itself.
        let proposals = from_form(state);
        return open(state, proposals, true);
    }
    let Some(preview) = state.inbox.preview.as_mut() else {
        return Task::none();
    };
    match action {
        PreviewAction::Open => unreachable!(),
        PreviewAction::Close => {
            state.inbox.preview = None;
            Task::none()
        }
        PreviewAction::Span(span) => {
            preview.span = span;
            scroll_to_focus(state)
        }
        PreviewAction::Shift(steps) => {
            let days = match preview.span {
                Span::Day => i64::from(steps),
                Span::Week => i64::from(steps) * 7,
            };
            preview.anchor += Duration::days(days);
            Task::none()
        }
        PreviewAction::Focus(index) => {
            if index >= preview.proposals.len() {
                return Task::none();
            }
            preview.focus = Some(index);
            if let Some(day) = preview.proposals[index].first_day() {
                let shown = visible_days(preview, &state.config.calendar);
                if !shown.contains(&day) {
                    preview.anchor = day;
                }
            }
            scroll_to_focus(state)
        }
        PreviewAction::ShowDay(day) => {
            preview.span = Span::Day;
            preview.anchor = day;
            Task::none()
        }
    }
}

/// The days the preview is showing.
pub fn visible_days(
    preview: &Preview,
    settings: &crate::config::CalendarSettings,
) -> Vec<NaiveDate> {
    match preview.span {
        Span::Day => vec![preview.anchor],
        Span::Week => settings.week_dates(preview.anchor),
    }
}

/// Scrolls the grid so the focused proposal sits a little below the top.
fn scroll_to_focus(state: &State) -> Task<Message> {
    let Some(preview) = &state.inbox.preview else {
        return Task::none();
    };
    let Some(when) = preview
        .focus
        .and_then(|index| preview.proposals.get(index))
        .and_then(|proposal| proposal.when.as_ref())
        .filter(|when| !when.all_day)
    else {
        return Task::none();
    };
    let offset = state.config.calendar.day_end_minutes();
    let minutes = (calendar::minutes_from_midnight(when.start) - offset).rem_euclid(24 * 60);
    let y = ((minutes - 60) as f32 * state.calendar.hour_height / 60.0).max(0.0);
    operation::scroll_to(
        Id::new(GRID_SCROLL),
        operation::AbsoluteOffset { x: 0.0, y },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timed(title: &str, day: NaiveDate, hour: u32, hours: i64) -> Proposal {
        let start = calendar::at_clock(day, hour, 0);
        Proposal {
            kind: DraftKind::Event,
            title: title.into(),
            when: Some(When {
                start,
                end: start + Duration::hours(hours),
                all_day: false,
                repeat: None,
            }),
            color: Color::from_rgb(0.2, 0.4, 0.8),
        }
    }

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn a_weekly_rule_lands_on_its_weekdays_only() {
        let monday = day("2026-11-02");
        let mut rule = Rule::weekly(monday);
        rule.weekdays =
            crate::recurrence::weekday_flags(&[chrono::Weekday::Mon, chrono::Weekday::Thu]);
        assert!(repeats_on(&rule, monday, monday));
        assert!(!repeats_on(&rule, monday, day("2026-11-03")));
        assert!(repeats_on(&rule, monday, day("2026-11-05")));
        assert!(repeats_on(&rule, monday, day("2026-11-09")));
        assert!(
            !repeats_on(&rule, monday, day("2026-10-26")),
            "not before it starts"
        );
        rule.interval = "2".into();
        assert!(!repeats_on(&rule, monday, day("2026-11-09")));
        assert!(repeats_on(&rule, monday, day("2026-11-16")));
    }

    #[test]
    fn a_repeating_proposal_shows_on_each_matching_day() {
        let monday = day("2026-11-02");
        let mut habit = timed("Run", monday, 7, 1);
        habit.kind = DraftKind::Habit;
        let mut rule = Rule::weekly(monday);
        rule.repeat = RepeatMode::Daily;
        habit.when.as_mut().unwrap().repeat = Some(rule);
        for offset in 0..7 {
            let date = monday + Duration::days(offset);
            let hits = habit.occurrences_on(date, 0);
            assert_eq!(hits.len(), 1, "{date}");
            assert_eq!(hits[0].0.date_naive(), date);
        }
    }

    #[test]
    fn a_late_item_belongs_to_the_evening_before_a_late_day_end() {
        let late = timed("Gig", day("2026-11-03"), 1, 1);
        assert_eq!(late.occurrences_on(day("2026-11-03"), 0).len(), 1);
        // With the day ending at 3am, 1am belongs to the 2nd.
        assert!(late.occurrences_on(day("2026-11-03"), 180).is_empty());
        assert_eq!(late.occurrences_on(day("2026-11-02"), 180).len(), 1);
    }

    #[test]
    fn a_multi_day_all_day_proposal_covers_each_day_it_spans() {
        let start = calendar::at_clock(day("2026-11-02"), 0, 0);
        let trip = Proposal {
            when: Some(When {
                start,
                end: start + Duration::days(3),
                all_day: true,
                repeat: None,
            }),
            ..timed("Trip", day("2026-11-02"), 0, 1)
        };
        assert!(!trip.occurrences_on(day("2026-11-02"), 0).is_empty());
        assert!(!trip.occurrences_on(day("2026-11-04"), 0).is_empty());
        assert!(trip.occurrences_on(day("2026-11-05"), 0).is_empty());
    }

    #[test]
    fn several_proposals_on_different_days_open_the_week_on_the_earliest() {
        let mut state = State::blank();
        let proposals = vec![
            timed("Gym", day("2026-11-12"), 18, 1),
            timed("Dentist", day("2026-11-10"), 9, 1),
            Proposal {
                when: None,
                kind: DraftKind::Task,
                ..timed("Call mum", day("2026-11-10"), 0, 1)
            },
        ];
        let _ = open(&mut state, proposals, false);
        let preview = state.inbox.preview.as_ref().unwrap();
        assert_eq!(preview.span, Span::Week);
        assert_eq!(preview.anchor, day("2026-11-10"));
        assert_eq!(preview.focus, Some(0));

        // Picking one outside the week brings its week into view.
        state.inbox.preview.as_mut().unwrap().proposals[1] =
            timed("Dentist", day("2026-12-01"), 9, 1);
        let _ = update(&mut state, PreviewAction::Focus(1));
        let preview = state.inbox.preview.as_ref().unwrap();
        assert_eq!(preview.focus, Some(1));
        assert_eq!(preview.anchor, day("2026-12-01"));
    }

    #[test]
    fn proposals_on_one_day_open_the_day_view_and_steps_move_by_the_span() {
        let mut state = State::blank();
        let target = day("2026-11-10");
        let _ = open(
            &mut state,
            vec![timed("A", target, 9, 1), timed("B", target, 9, 2)],
            false,
        );
        assert_eq!(state.inbox.preview.as_ref().unwrap().span, Span::Day);
        let _ = update(&mut state, PreviewAction::Shift(1));
        assert_eq!(
            state.inbox.preview.as_ref().unwrap().anchor,
            target + Duration::days(1)
        );
        let _ = update(&mut state, PreviewAction::Span(Span::Week));
        let _ = update(&mut state, PreviewAction::Shift(-1));
        assert_eq!(
            state.inbox.preview.as_ref().unwrap().anchor,
            target - Duration::days(6)
        );
        let _ = update(&mut state, PreviewAction::ShowDay(target));
        let preview = state.inbox.preview.as_ref().unwrap();
        assert_eq!((preview.span, preview.anchor), (Span::Day, target));
        let _ = update(&mut state, PreviewAction::Close);
        assert!(state.inbox.preview.is_none());
    }

    #[test]
    fn the_form_previews_its_event_and_a_task_with_its_due_date() {
        let mut state = State::blank();
        let _ = super::super::update(&mut state, Action::NewTask);
        let _ = super::super::update(&mut state, Action::Title("Write report".into()));
        let proposals = from_form(&state);
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].title, "Write report");
        assert!(proposals[0].when.is_none(), "no due date yet");

        state.timeblocking.date = "2026-11-20".into();
        let when = from_form(&state)[0].when.clone().unwrap();
        assert!(when.all_day);
        assert_eq!(when.start.date_naive(), day("2026-11-20"));

        let _ = super::super::creation::select(&mut state, DraftKind::Event);
        let proposals = from_form(&state);
        assert_eq!(proposals[0].kind, DraftKind::Event);
        assert_eq!(proposals[0].title, "Write report");
        assert!(!proposals[0].when.as_ref().unwrap().all_day);

        let _ = super::super::update(&mut state, Action::Preview(PreviewAction::Open));
        assert!(state.inbox.preview.as_ref().unwrap().from_form);
        // Closing the form takes the preview with it.
        let _ = super::super::update(&mut state, Action::Cancel);
        assert!(state.inbox.preview.is_none());
    }
}
