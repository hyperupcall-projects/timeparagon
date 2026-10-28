//! One-off migration of the Google Keep ritual notes and the Google Tasks
//! "Habits & Recurring" list into Calendar habits.
//!
//! The Keep notes already carry a time in their metadata ("Recurring task set
//! for Today, 8:00 AM"), and the old task workaround carried one in its notes
//! (`DUE_TIME: 18:00`). This turns each recurring item into a habit on the
//! TimeSlay calendar, keeping its text as the habit's notes. Tasks that a
//! habit already covers are removed from Google Tasks so they stop appearing
//! twice; one-off tasks and completed history are left alone.
//!
//! Run it with no arguments to see what it would do:
//!
//! ```sh
//! cargo run --example migrate_keep_habits
//! ```
//!
//! and with `--apply` to create the habits and remove the replaced tasks:
//!
//! ```sh
//! cargo run --example migrate_keep_habits -- --apply
//! ```
//!
//! The Keep notes are left untouched; Google Keep's browser helper is
//! read-only.

use std::collections::{BTreeMap, HashSet};

use chrono::Timelike;
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveTime, Weekday};

use crate::calendar::{
    self,
    api::{self, HabitEvent, TaskListItem},
};
use crate::config::{CalendarSettings, Config};
use crate::keep::{self, Note, Snapshot};

/// The Keep labels whose notes become habits. The second spelling is accepted
/// so a slightly differently named label also migrates.
pub const KEEP_LABELS: [&str; 2] = ["Habits & Rituals", "Habits & Recurring"];

/// The Google Tasks lists that hold recurring items. The Keep spelling is
/// accepted too, in case the list was renamed to match.
pub const TASK_LISTS: [&str; 2] = ["Habits & Recurring", "Habits & Rituals"];

/// How long a migrated habit lasts, unless the command line says otherwise.
pub const DEFAULT_MINUTES: i64 = 30;

/// One habit the migration would create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedHabit {
    /// The Keep note or task this came from, so a rerun can skip it.
    pub source_id: String,
    pub title: String,
    pub notes: String,
    pub date: NaiveDate,
    pub start: NaiveTime,
    pub end: NaiveTime,
    pub recurrence: Vec<String>,
}

/// An incomplete Google Task that a habit replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskInstance {
    pub id: String,
    pub title: String,
    pub due: Option<NaiveDate>,
}

/// A group of task instances that share one recurring title.
#[derive(Debug, Clone)]
pub struct TaskHabit {
    pub habit: PlannedHabit,
    /// The incomplete instances to remove once the habit exists.
    pub instances: Vec<TaskInstance>,
    /// True when a habit with this name or start time already exists, so
    /// only the task instances need removing.
    pub already: bool,
}

/// What the task half of the migration found.
#[derive(Debug, Default)]
pub struct TaskPlan {
    pub habits: Vec<TaskHabit>,
    /// Tasks left in Google Tasks, with the reason they were left.
    pub kept: Vec<String>,
}

/// What the migration did, or would do in a dry run.
#[derive(Debug, Default)]
pub struct Report {
    pub dry_run: bool,
    pub planned: Vec<PlannedHabit>,
    pub deleted: Vec<TaskInstance>,
    /// Task instances deliberately left in Google Tasks by `--keep-tasks`.
    pub retained: Vec<TaskInstance>,
    pub kept: Vec<String>,
    pub created: Vec<String>,
    pub skipped: Vec<String>,
    pub errors: Vec<String>,
}

/// Builds the Keep plan without touching Google. Existing habit titles are
/// skipped, so running the migration twice does not make duplicates.
pub fn plan(
    snapshot: &Snapshot,
    existing: &[HabitEvent],
    today: NaiveDate,
    minutes: i64,
) -> Vec<PlannedHabit> {
    let mut taken: HashSet<String> = existing
        .iter()
        .map(|habit| habit_key(&habit.title))
        .collect();
    let mut planned = Vec::new();

    for note in snapshot.notes.iter().filter(|note| has_label(note)) {
        if note.archived {
            continue;
        }
        let title = note.title.trim();
        if title.is_empty() {
            continue;
        }
        let Some(start) = note_time(note) else {
            continue;
        };
        if !taken.insert(habit_key(title)) {
            continue;
        }
        let end = start + chrono::Duration::minutes(minutes.clamp(1, 24 * 60 - 1));
        let (date, recurrence) = schedule_title(title, today);
        planned.push(PlannedHabit {
            source_id: note.id.clone(),
            title: title.to_owned(),
            notes: note_notes(note),
            date,
            start,
            end,
            recurrence,
        });
    }
    planned
}

/// The completion reasons for notes that cannot become habits, so a dry run
/// can say why they were left out.
pub fn skipped(snapshot: &Snapshot, existing: &[HabitEvent]) -> Vec<String> {
    let mut reasons = Vec::new();
    let existing_titles: HashSet<String> = existing
        .iter()
        .map(|habit| habit_key(&habit.title))
        .collect();
    let mut seen = HashSet::new();
    for note in snapshot.notes.iter().filter(|note| has_label(note)) {
        let title = note.title.trim();
        if note.archived {
            reasons.push(format!("{title}: archived in Keep"));
        } else if title.is_empty() {
            reasons.push("(untitled note): no title".to_owned());
        } else if note_time(note).is_none() {
            reasons.push(format!("{title}: no time in its Keep metadata"));
        } else if existing_titles.contains(&habit_key(title)) {
            reasons.push(format!("{title}: a habit with that name already exists"));
        } else if !seen.insert(habit_key(title)) {
            reasons.push(format!("{title}: another Keep note has the same title"));
        }
    }
    reasons
}

/// Groups tasks by their recurring title, then decides which need a new habit
/// and which are covered by a habit that already exists or is planned.
pub fn plan_tasks(
    tasks: &[TaskListItem],
    existing: &[HabitEvent],
    planned: &[PlannedHabit],
    today: NaiveDate,
    minutes: i64,
) -> TaskPlan {
    let mut groups: BTreeMap<String, Vec<&TaskListItem>> = BTreeMap::new();
    for task in tasks {
        let title = task.title.trim();
        if title.is_empty() {
            continue;
        }
        groups.entry(habit_key(title)).or_default().push(task);
    }

    let mut plan = TaskPlan::default();
    for (key, group) in groups {
        let title = group
            .iter()
            .map(|task| task.title.trim())
            .find(|title| !title.is_empty())
            .unwrap_or(&key)
            .to_owned();
        let instances: Vec<TaskInstance> = group
            .iter()
            .filter(|task| !task.completed)
            .map(|task| TaskInstance {
                id: task.id.clone(),
                title: task.title.trim().to_owned(),
                due: task.due,
            })
            .collect();
        if instances.is_empty() {
            // Completed tasks are history; deleting them would lose it.
            continue;
        }

        let task_time = group.iter().find_map(|task| task_time(&task.notes));
        if let Some(habit) = existing.iter().find(|habit| {
            habit_key(&habit.title) == key
                || task_time.is_some_and(|start| habit.start.time() == start)
        }) {
            let start = task_time.unwrap_or_else(|| habit.start.time());
            let (date, recurrence) = (habit.start.date_naive(), habit.recurrence.clone());
            let end = start + Duration::minutes(minutes.clamp(1, 24 * 60 - 1));
            plan.habits.push(TaskHabit {
                habit: PlannedHabit {
                    source_id: instances[0].id.clone(),
                    title: title.clone(),
                    notes: task_notes(&group),
                    date,
                    start,
                    end,
                    recurrence,
                },
                instances,
                already: true,
            });
            continue;
        }
        if let Some(habit) = planned.iter().find(|habit| {
            habit_key(&habit.title) == key || task_time.is_some_and(|start| habit.start == start)
        }) {
            plan.habits.push(TaskHabit {
                habit: habit.clone(),
                instances,
                already: true,
            });
            continue;
        }
        let Some(start) = task_time else {
            plan.kept
                .push(format!("{title}: no time in its notes, left as a task"));
            continue;
        };
        let end = start + Duration::minutes(minutes.clamp(1, 24 * 60 - 1));
        let (date, recurrence) = schedule_title(&title, today);
        plan.habits.push(TaskHabit {
            habit: PlannedHabit {
                source_id: instances[0].id.clone(),
                title: title.clone(),
                notes: task_notes(&group),
                date,
                start,
                end,
                recurrence,
            },
            instances,
            already: false,
        });
    }
    plan
}

/// Runs the migration. With `apply` false nothing is written to Google.
/// `remove_tasks` deletes the live task instances that a habit replaces; when
/// it is false they are left in place and reported as retained.
pub fn run(apply: bool, minutes: i64, remove_tasks: bool) -> Result<Report, String> {
    let settings = Config::load().calendar;
    let snapshot = keep::fetch(None)?;
    let existing = calendar::sync_habits(
        &settings.client_id,
        &settings.client_secret,
        settings.app_calendar.as_deref(),
    )?;
    let today = Local::now().date_naive();
    let keep_planned = plan(&snapshot, &existing, today, minutes);
    let tasks = habit_tasks(&settings)?;
    let task_plan = match &tasks {
        Some((_, tasks)) => plan_tasks(tasks, &existing, &keep_planned, today, minutes),
        None => TaskPlan::default(),
    };

    let mut report = Report {
        dry_run: !apply,
        planned: keep_planned
            .iter()
            .cloned()
            .chain(
                task_plan
                    .habits
                    .iter()
                    .filter(|habit| !habit.already)
                    .map(|habit| habit.habit.clone()),
            )
            .collect(),
        kept: task_plan.kept.clone(),
        skipped: skipped(&snapshot, &existing),
        ..Report::default()
    };
    if !apply {
        let instances: Vec<TaskInstance> = task_plan
            .habits
            .iter()
            .flat_map(|habit| habit.instances.clone())
            .collect();
        if remove_tasks {
            report.deleted = instances;
        } else {
            report.retained = instances;
        }
        return Ok(report);
    }

    let calendar_id = existing.first().map(|habit| habit.calendar_id.clone());
    let mut created = Vec::new();
    for habit in &keep_planned {
        match write_habit(&settings, calendar_id.as_deref(), habit) {
            Ok(()) => {
                report.created.push(habit.title.clone());
                created.push(habit.clone());
            }
            Err(error) => report.errors.push(format!("{}: {error}", habit.title)),
        }
    }

    // A failed Keep write must not cause its matching tasks to be removed, so
    // re-plan the tasks against the habits that actually exist now.
    let Some((list_id, tasks)) = tasks else {
        return Ok(report);
    };
    let mut covered = existing;
    covered.extend(created.iter().map(|habit| HabitEvent {
        id: String::new(),
        calendar_id: calendar_id.clone().unwrap_or_default(),
        title: habit.title.clone(),
        notes: habit.notes.clone(),
        start: calendar::at_clock(habit.date, habit.start.hour(), habit.start.minute()),
        end: calendar::at_clock(habit.date, habit.end.hour(), habit.end.minute()),
        recurrence: habit.recurrence.clone(),
        color_id: None,
    }));
    let task_plan = plan_tasks(&tasks, &covered, &[], today, minutes);

    for group in &task_plan.habits {
        if !group.already {
            match write_habit(&settings, calendar_id.as_deref(), &group.habit) {
                Ok(()) => report.created.push(group.habit.title.clone()),
                Err(error) => {
                    report
                        .errors
                        .push(format!("{}: {error}", group.habit.title));
                    continue;
                }
            }
        }
        if !remove_tasks {
            report.retained.extend(group.instances.iter().cloned());
            continue;
        }
        for instance in &group.instances {
            match calendar::delete_task_instance(
                &settings.client_id,
                &settings.client_secret,
                &list_id,
                &instance.id,
            ) {
                Ok(()) => report.deleted.push(instance.clone()),
                Err(error) => report
                    .errors
                    .push(format!("delete {}: {error}", instance.title)),
            }
        }
    }
    Ok(report)
}

/// The first task list named like the old habits list, with its tasks.
fn habit_tasks(settings: &CalendarSettings) -> Result<Option<(String, Vec<TaskListItem>)>, String> {
    let tokens = calendar::oauth::load_tokens()
        .ok_or_else(|| "Not connected to Google Calendar".to_owned())?;
    let tokens =
        calendar::oauth::ensure_access(&settings.client_id, &settings.client_secret, tokens)?;
    let lists = api::fetch_task_lists(&tokens.access_token)?;
    let Some((_, id)) = lists.into_iter().find(|(name, _)| {
        TASK_LISTS
            .iter()
            .any(|wanted| name.trim().eq_ignore_ascii_case(wanted))
    }) else {
        return Ok(None);
    };
    let tasks = api::fetch_task_list(&tokens.access_token, &id)?;
    Ok(Some((id, tasks)))
}

fn write_habit(
    settings: &CalendarSettings,
    calendar_id: Option<&str>,
    habit: &PlannedHabit,
) -> Result<(), String> {
    let start = calendar::at_clock(habit.date, habit.start.hour(), habit.start.minute());
    let end = calendar::at_clock(habit.date, habit.end.hour(), habit.end.minute());
    calendar::write_habit(
        &settings.client_id,
        &settings.client_secret,
        calendar::HabitWrite {
            id: None,
            calendar_id,
            title: &habit.title,
            notes: &habit.notes,
            start,
            end,
            recurrence: &habit.recurrence,
            color_id: None,
        },
    )
    .map(|_| ())
}

fn has_label(note: &Note) -> bool {
    note.labels.iter().any(|label| {
        KEEP_LABELS
            .iter()
            .any(|wanted| label.trim().eq_ignore_ascii_case(wanted))
    })
}

fn note_notes(note: &Note) -> String {
    let description = note.description.trim();
    if !description.is_empty() {
        return description.to_owned();
    }
    note.checklist
        .iter()
        .map(|item| item.text.trim())
        .filter(|text| !text.is_empty())
        .map(|text| format!("- {text}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Notes from every instance of a task group, with the retired `DUE_TIME`
/// marker removed.
fn task_notes(group: &[&TaskListItem]) -> String {
    group
        .iter()
        .map(|task| task.notes.as_str())
        .find(|notes| !notes.trim().is_empty())
        .map(clean_task_notes)
        .unwrap_or_default()
}

fn clean_task_notes(notes: &str) -> String {
    let cleaned = notes
        .lines()
        .filter(|line| {
            !line
                .trim_start()
                .to_ascii_uppercase()
                .starts_with("DUE_TIME:")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let cleaned = cleaned.trim();
    if cleaned.trim_matches('.').trim().is_empty() {
        String::new()
    } else {
        cleaned.to_owned()
    }
}

/// The time of day a Keep ritual runs, read from "Today, 8:00 AM" style
/// metadata (including the narrow no-break spaces Keep uses).
fn note_time(note: &Note) -> Option<NaiveTime> {
    note.metadata
        .iter()
        .find_map(|line| time_in(line))
        .or_else(|| time_in(&note.description))
}

/// The time the old app recorded in a task's notes.
fn task_time(notes: &str) -> Option<NaiveTime> {
    notes.lines().find_map(|line| {
        let line = line.trim();
        let rest = line
            .strip_prefix("DUE_TIME:")
            .or_else(|| line.strip_prefix("Due time:"))?;
        let rest = rest.trim();
        for format in ["%H:%M", "%I:%M %p", "%I:%M%p"] {
            if let Ok(time) = NaiveTime::parse_from_str(rest, format) {
                return Some(time);
            }
        }
        None
    })
}

fn time_in(text: &str) -> Option<NaiveTime> {
    let text = text.replace(['\u{202f}', '\u{a0}'], " ");
    for piece in text.split([',', ';']) {
        let piece = piece.trim();
        for format in ["%I:%M %p", "%I:%M%p", "%H:%M"] {
            if let Ok(time) = NaiveTime::parse_from_str(piece, format) {
                return Some(time);
            }
        }
    }
    // Keep sometimes writes the time without a comma.
    for format in ["%I:%M %p", "%I:%M%p"] {
        let words: Vec<_> = text.split_whitespace().collect();
        if words.len() >= 2 {
            let tail = words[words.len() - 2..].join(" ");
            if let Ok(time) = NaiveTime::parse_from_str(&tail, format) {
                return Some(time);
            }
        }
    }
    None
}

/// A title that says "weekend" or "weekly" repeats on Sunday; the rest are
/// daily. The date is moved to the first matching day so a single DTSTART does
/// not add an extra occurrence on the migration day.
fn schedule_title(title: &str, today: NaiveDate) -> (NaiveDate, Vec<String>) {
    let title = title.to_lowercase();
    if title.contains("weekend") || title.contains("weekly") {
        let date = next_weekday(today, Weekday::Sun);
        (date, vec!["RRULE:FREQ=WEEKLY;BYDAY=SU".to_owned()])
    } else {
        (today, vec!["RRULE:FREQ=DAILY".to_owned()])
    }
}

/// A loose comparison key: "Daily Digital Maintenance Ritual" and
/// "Digital Maintenance" are the same habit.
fn habit_key(title: &str) -> String {
    let mut key = title.trim().to_lowercase();
    for prefix in ["daily ", "weekly "] {
        if let Some(rest) = key.strip_prefix(prefix) {
            key = rest.to_owned();
            break;
        }
    }
    for suffix in [" ritual", " routine", " habit"] {
        if let Some(rest) = key.strip_suffix(suffix) {
            key = rest.to_owned();
            break;
        }
    }
    key.trim().to_owned()
}

fn next_weekday(today: NaiveDate, weekday: Weekday) -> NaiveDate {
    let current = today.weekday().num_days_from_sunday();
    let target = weekday.num_days_from_sunday();
    let days = (target + 7 - current) % 7;
    today + Duration::days(i64::from(days))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, title: &str, metadata: &[&str]) -> Note {
        Note {
            id: id.into(),
            title: title.into(),
            description: "- First item\n\n- Second item".into(),
            labels: vec!["Habits & Rituals".into()],
            metadata: metadata.iter().map(|line| (*line).to_owned()).collect(),
            ..Note::default()
        }
    }

    fn existing(title: &str, hour: u32, minute: u32) -> HabitEvent {
        let day = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        HabitEvent {
            id: "habit-1".into(),
            calendar_id: "primary".into(),
            title: title.into(),
            notes: String::new(),
            start: calendar::at_clock(day, hour, minute),
            end: calendar::at_clock(day, hour, minute + 30),
            recurrence: vec!["RRULE:FREQ=DAILY".into()],
            color_id: None,
        }
    }

    fn task(id: &str, title: &str, notes: &str) -> TaskListItem {
        TaskListItem {
            id: id.into(),
            title: title.into(),
            notes: notes.into(),
            completed: false,
            due: NaiveDate::from_ymd_opt(2026, 10, 28),
        }
    }

    #[test]
    fn reads_the_time_keep_writes_in_its_metadata() {
        assert_eq!(
            time_in("Recurring task set for Today, 8:00\u{202f}AM"),
            NaiveTime::from_hms_opt(8, 0, 0)
        );
        assert_eq!(
            time_in("Oct 25, 8:00\u{202f}PM"),
            NaiveTime::from_hms_opt(20, 0, 0)
        );
        assert_eq!(
            time_in("Recurring task set for Today 10:30 PM"),
            NaiveTime::from_hms_opt(22, 30, 0)
        );
        assert_eq!(time_in("Created Sep 27"), None);
    }

    #[test]
    fn daily_rituals_become_daily_habits_and_weekends_become_sundays() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        let mut snapshot = Snapshot::default();
        snapshot.notes = vec![
            note("one", "Night Reflection Ritual", &["Today, 10:30 PM"]),
            note("two", "Weekend Ritual", &["Oct 25, 8:00 PM"]),
        ];
        let planned = plan(&snapshot, &[], today, 30);
        assert_eq!(planned.len(), 2);
        assert_eq!(planned[0].date, today);
        assert_eq!(
            planned[0].start,
            NaiveTime::from_hms_opt(22, 30, 0).unwrap()
        );
        assert_eq!(planned[0].end, NaiveTime::from_hms_opt(23, 0, 0).unwrap());
        assert_eq!(planned[0].recurrence, ["RRULE:FREQ=DAILY"]);
        assert_eq!(
            planned[1].date,
            NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
        );
        assert_eq!(planned[1].recurrence, ["RRULE:FREQ=WEEKLY;BYDAY=SU"]);
    }

    #[test]
    fn a_rerun_skips_habits_that_already_exist() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        let mut snapshot = Snapshot::default();
        snapshot.notes = vec![note("one", "Night Reflection Ritual", &["Today, 10:30 PM"])];
        let habits = [existing("Night Reflection Ritual", 22, 30)];
        let planned = plan(&snapshot, &habits, today, 30);
        assert!(planned.is_empty());
        assert_eq!(
            skipped(&snapshot, &habits),
            ["Night Reflection Ritual: a habit with that name already exists"]
        );
    }

    #[test]
    fn notes_without_a_time_or_with_another_label_are_left_out() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        let mut snapshot = Snapshot::default();
        let mut no_time = note("one", "No time", &["Created Sep 27"]);
        no_time.description = "- do the thing".into();
        let mut other_label = note("two", "Other label", &["Today, 8:00 AM"]);
        other_label.labels = vec!["Objectives".into()];
        snapshot.notes = vec![no_time, other_label];
        assert!(plan(&snapshot, &[], today, 30).is_empty());
    }

    #[test]
    fn reads_the_old_due_time_and_strips_it_from_notes() {
        assert_eq!(
            task_time(".\nDUE_TIME: 18:00"),
            NaiveTime::from_hms_opt(18, 0, 0)
        );
        assert_eq!(
            task_time("DUE_TIME: 9:30 PM"),
            NaiveTime::from_hms_opt(21, 30, 0)
        );
        assert_eq!(clean_task_notes(".\nDUE_TIME: 18:00"), "");
        assert_eq!(clean_task_notes("Warm up\nDUE_TIME: 18:00"), "Warm up");
    }

    #[test]
    fn new_task_habits_are_created_and_covered_ones_only_removed() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        let tasks = vec![
            task("a", "Daily Kattis/LeetCode Routine", ".\nDUE_TIME: 18:00"),
            task("b", "Digital Maintenance Ritual", ".\nDUE_TIME: 09:30"),
            task("c", "Submit TA Hours", ""),
        ];
        let habits = [existing("Daily Digital Maintenance Ritual", 9, 30)];
        let plan = plan_tasks(&tasks, &habits, &[], today, 30);

        assert_eq!(plan.habits.len(), 2);
        let kattis = plan
            .habits
            .iter()
            .find(|habit| habit.habit.title.starts_with("Daily Kattis"))
            .unwrap();
        assert!(!kattis.already);
        assert_eq!(
            kattis.habit.start,
            NaiveTime::from_hms_opt(18, 0, 0).unwrap()
        );
        assert_eq!(kattis.habit.notes, "");
        assert_eq!(kattis.instances.len(), 1);

        let digital = plan
            .habits
            .iter()
            .find(|habit| habit.habit.title.starts_with("Digital"))
            .unwrap();
        assert!(digital.already);
        assert_eq!(
            digital.habit.start,
            NaiveTime::from_hms_opt(9, 30, 0).unwrap()
        );

        assert_eq!(
            plan.kept,
            ["Submit TA Hours: no time in its notes, left as a task"]
        );
    }

    #[test]
    fn stale_instances_share_one_group_and_weekend_tasks_match_by_name() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        let tasks = vec![
            task("a", "Daily Kattis/LeetCode Routine", "DUE_TIME: 18:00"),
            task("b", "Daily Kattis/LeetCode", ""),
            task("c", "Weekend Ritual", ""),
        ];
        let habits = [existing("Weekend Ritual", 20, 0)];
        let plan = plan_tasks(&tasks, &habits, &[], today, 30);
        assert_eq!(plan.habits.len(), 2);
        let kattis = plan
            .habits
            .iter()
            .find(|habit| habit.habit.title.starts_with("Daily Kattis"))
            .unwrap();
        assert_eq!(kattis.instances.len(), 2);
        let weekend = plan
            .habits
            .iter()
            .find(|habit| habit.habit.title == "Weekend Ritual")
            .unwrap();
        assert!(weekend.already);
        assert_eq!(
            weekend.habit.start,
            NaiveTime::from_hms_opt(20, 0, 0).unwrap()
        );
    }

    #[test]
    fn completed_instances_are_history_and_stay_put() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        let mut done = task("a", "Morning Wake Ritual", "DUE_TIME: 08:00");
        done.completed = true;
        let plan = plan_tasks(&[done], &[], &[], today, 30);
        assert!(plan.habits.is_empty());
        assert!(plan.kept.is_empty());
    }
}
