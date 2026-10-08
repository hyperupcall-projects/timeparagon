//! Reads the *time* of a Google Task from the browser, because the Tasks API
//! only ever returns a date (`due` is midnight UTC).
//!
//! The work lives in `tools/tasks-bridge`, a Deno program that drives a headless
//! Chrome against `tasks.google.com` using a profile the user signs into once by
//! hand. It is optional: if the helper is not installed, or is not signed in,
//! tasks stay all-day exactly as they are today.
//!
//! The helper is always run headless, and the escape hatch that would show a
//! window is stripped from its environment, so a sync never puts a Chrome window
//! on screen.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use chrono::{Duration, Local, NaiveTime, TimeZone};

use super::Snapshot;

/// How long a scheduled task's block is drawn.
///
/// Google Tasks stores a *point* in time, not a duration, so this is purely a
/// display choice. Change it here if a different block length is wanted.
const TASK_BLOCK_MINUTES: i64 = 30;

/// `(list name, task title)` in lower case, mapped to the time of day.
///
/// `None` means the helper reported a time for this pair but at least two rows
/// disagreed, so no time is trusted for it.
pub type TimeTable = HashMap<(String, String), Option<(u32, u32)>>;

#[derive(serde::Deserialize)]
struct Report {
    #[serde(default)]
    tasks: Vec<Row>,
}

#[derive(serde::Deserialize)]
struct Row {
    #[serde(default)]
    list: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    time: Option<String>,
    #[serde(default)]
    ambiguous: bool,
}

/// The helper's document format is `list` + `title` + `time` + `ambiguous`.
/// Everything else it emits is for a human reading the JSON.
fn parse_report(bytes: &[u8]) -> Option<TimeTable> {
    let report: Report = match serde_json::from_slice(bytes) {
        Ok(report) => report,
        Err(error) => {
            log::warn!("tasks-bridge returned unreadable JSON: {error}");
            return None;
        }
    };

    let mut times: TimeTable = HashMap::new();
    for row in report.tasks {
        // A time with no AM/PM is not trusted: a wrong time on the grid is
        // worse than a chip that stays all-day.
        if row.ambiguous {
            continue;
        }
        let Some((hour, minute)) = row.time.as_deref().and_then(parse_hhmm) else {
            continue;
        };
        let Some(list) = row.list.as_deref().map(normalize) else {
            continue;
        };
        let Some(title) = row.title.as_deref().map(normalize) else {
            continue;
        };
        if title.is_empty() {
            continue;
        }

        match times.entry((list, title)) {
            Entry::Vacant(entry) => {
                entry.insert(Some((hour, minute)));
            }
            Entry::Occupied(mut entry) => {
                // Same title twice: keep the time only if the rows agree.
                if entry.get() != &Some((hour, minute)) {
                    entry.insert(None);
                }
            }
        }
    }

    Some(times)
}

fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

fn parse_hhmm(value: &str) -> Option<(u32, u32)> {
    let (hour, minute) = value.split_once(':')?;
    let hour: u32 = hour.parse().ok()?;
    let minute: u32 = minute.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

/// Promotes all-day task chips to timed events where the browser knew a time.
/// Returns how many were promoted.
///
/// Split out from [`promote_task_times`] so it can be exercised without running
/// a browser.
fn apply_times(snapshot: &mut Snapshot, times: &TimeTable) -> usize {
    // The events name their list by id; the titles live on the calendars.
    let names: HashMap<&str, &str> = snapshot
        .calendars
        .iter()
        .filter(|calendar| calendar.is_task_list())
        .map(|calendar| (calendar.id.as_str(), calendar.name.as_str()))
        .collect();

    let mut promoted = 0;
    for event in snapshot.events.iter_mut() {
        if !event.is_task() || !event.all_day {
            continue;
        }
        let Some(list) = names.get(event.calendar_id.as_str()) else {
            continue;
        };
        let key = (normalize(list), normalize(&event.title));
        let Some(Some((hour, minute))) = times.get(&key) else {
            continue;
        };
        let Some(time) = NaiveTime::from_hms_opt(*hour, *minute, 0) else {
            continue;
        };
        // The event is all-day, so its start is the local midnight of the day
        // the snapshot is for; reuse that date and move it to the task's time.
        let naive = event.start.date_naive().and_time(time);
        let Some(start) = Local.from_local_datetime(&naive).single() else {
            continue;
        };

        event.start = start;
        event.end = start + Duration::minutes(TASK_BLOCK_MINUTES);
        event.all_day = false;
        promoted += 1;
    }

    if promoted > 0 {
        snapshot
            .events
            .sort_by(|left, right| left.start.cmp(&right.start).then(right.end.cmp(&left.end)));
    }

    promoted
}

/// Fills in task times if the helper is installed and signed in. Never fails a
/// sync: anything missing simply leaves the affected tasks all-day.
pub fn promote_task_times(snapshot: &mut Snapshot) {
    // Do not start a browser when there is nothing it could tell us.
    if !snapshot.calendars.iter().any(|calendar| calendar.is_task_list()) {
        return;
    }
    if !snapshot
        .events
        .iter()
        .any(|event| event.is_task() && event.all_day)
    {
        return;
    }

    let Some(bytes) = run_helper("fetch") else {
        return;
    };
    let Some(times) = parse_report(&bytes) else {
        return;
    };

    let promoted = apply_times(snapshot, &times);
    if promoted > 0 {
        log::info!("tasks-bridge placed {promoted} task(s) at their scheduled time");
    } else {
        log::debug!("tasks-bridge had no times for this day's tasks");
    }
}

/// Runs `tasks-bridge <subcommand>`, trying each place the helper might be
/// installed. The "show me a window" escape hatch is stripped, so nothing but
/// [`signin`] can put a browser on screen.
fn run_helper(subcommand: &str) -> Option<Vec<u8>> {
    for candidate in helper_candidates() {
        let mut command = Command::new(&candidate);
        command
            .arg(subcommand)
            .env_remove("TASKS_BRIDGE_HEADED")
            .stdin(Stdio::null());

        match command.output() {
            Ok(output) if output.status.success() => return Some(output.stdout),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if output.status.code() == Some(3) {
                    log::info!("tasks-bridge {subcommand}: not signed in");
                } else {
                    log::warn!("tasks-bridge {subcommand} failed: {}", stderr.trim());
                }
                return None;
            }
            // Not this one; try the next.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                log::warn!("could not run tasks-bridge at {candidate:?}: {error}");
                return None;
            }
        }
    }

    log::debug!("tasks-bridge helper not found");
    None
}

/// Whether the helper is installed and its browser profile is signed in.
///
/// Blocking: call it from [`crate::blocking::run`].
pub fn signed_in() -> bool {
    run_helper("status").is_some()
}

/// Opens a normal Chrome window for the user to sign into Google Tasks.
///
/// This is the one path that deliberately shows a window; a sync never does.
/// Blocking: call it from [`crate::blocking::run`].
pub fn signin() -> Result<(), String> {
    for candidate in helper_candidates() {
        let mut command = Command::new(&candidate);
        command.arg("signin").stdin(Stdio::null());

        match command.output() {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                return Err(if stderr.is_empty() {
                    format!("tasks-bridge signin exited with {}", output.status)
                } else {
                    stderr
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!("could not run tasks-bridge at {candidate:?}: {error}"));
            }
        }
    }

    Err("the tasks-bridge helper was not found".to_owned())
}

fn helper_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Some(path) = std::env::var_os("TIMETRACKER_TASKS_BRIDGE") {
        candidates.push(PathBuf::from(path));
    }

    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(PathBuf::from);

        // Installed: beside the application binary.
        if let Some(dir) = &dir {
            candidates.push(dir.join("tasks-bridge"));
        }

        // Run from a checkout: the app is `<crate>/target/{debug,release}/app`
        // and a test binary is `<crate>/target/debug/deps/test`, so walk up a
        // few levels. This keeps discovery independent of the working
        // directory, which a launcher or IDE does not set to the crate root.
        for _ in 0..3 {
            dir = dir.and_then(|parent| parent.parent().map(PathBuf::from));
            if let Some(dir) = &dir {
                candidates.push(dir.join("tools/tasks-bridge/tasks-bridge"));
            }
        }
    }

    // Running with the crate root as the working directory.
    candidates.push(PathBuf::from("tools/tasks-bridge/tasks-bridge"));

    // Last resort: anything named `tasks-bridge` on PATH.
    candidates.push(PathBuf::from("tasks-bridge"));

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::{colors, Calendar, Event};
    use chrono::{Duration, NaiveDate, Timelike};

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 2).expect("a real date")
    }

    fn midnight(date: NaiveDate) -> chrono::DateTime<Local> {
        date.and_hms_opt(0, 0, 0)
            .expect("a real time")
            .and_local_timezone(Local)
            .single()
            .expect("unambiguous midnight")
    }

    fn task_event(date: NaiveDate, title: &str) -> Event {
        Event {
            id: "task:abc".to_owned(),
            calendar_id: "tasks:list1".to_owned(),
            title: title.to_owned(),
            start: midnight(date),
            end: midnight(date.succ_opt().expect("a next day")),
            all_day: true,
            color: colors::TASK,
            completed: false,
            details: Default::default(),
        }
    }

    fn snapshot_with(event: Event) -> Snapshot {
        Snapshot {
            calendars: vec![Calendar {
                id: "tasks:list1".to_owned(),
                name: "My Tasks".to_owned(),
                color: colors::TASK,
                primary: false,
                selected_by_google: true,
            }],
            events: vec![event],
            day: day(),
            fetched_at: midnight(day()),
            notice: None,
        }
    }

    fn report(tasks: &str) -> TimeTable {
        let json = format!(r#"{{"tasks":[{tasks}]}}"#);
        parse_report(json.as_bytes()).expect("a readable report")
    }

    #[test]
    fn a_reported_time_is_keyed_by_list_and_title() {
        let times = report(
            r#"{"list":"My Tasks","title":"Stand-up","time":"09:30","ambiguous":false}"#,
        );
        assert_eq!(times.get(&("my tasks".to_owned(), "stand-up".to_owned())), Some(&Some((9, 30))));
    }

    #[test]
    fn a_date_only_task_contributes_no_time() {
        let times = report(r#"{"list":"My Tasks","title":"Call mum","time":null}"#);
        assert!(times.is_empty());
    }

    #[test]
    fn an_ambiguous_time_is_not_used() {
        let times = report(
            r#"{"list":"My Tasks","title":"Stand-up","time":"09:30","ambiguous":true}"#,
        );
        assert!(times.is_empty());
    }

    #[test]
    fn disagreeing_rows_for_one_title_are_dropped_rather_than_guessed() {
        let times = report(
            r#"{"list":"My Tasks","title":"Stand-up","time":"09:30"},
               {"list":"My Tasks","title":"Stand-up","time":"10:00"}"#,
        );
        assert_eq!(times.get(&("my tasks".to_owned(), "stand-up".to_owned())), Some(&None));
    }

    #[test]
    fn a_matching_task_becomes_a_timed_block() {
        let mut snapshot = snapshot_with(task_event(day(), "Stand-up"));
        let times = report(
            r#"{"list":"My Tasks","title":"Stand-up","time":"09:30","ambiguous":false}"#,
        );

        assert_eq!(apply_times(&mut snapshot, &times), 1);

        let event = &snapshot.events[0];
        assert!(!event.all_day);
        assert_eq!(event.start.hour(), 9);
        assert_eq!(event.start.minute(), 30);
        assert_eq!(event.end, event.start + Duration::minutes(TASK_BLOCK_MINUTES));
    }

    #[test]
    fn an_unmatched_task_stays_all_day() {
        let mut snapshot = snapshot_with(task_event(day(), "Stand-up"));
        let times = report(
            r#"{"list":"My Tasks","title":"Something else","time":"09:30","ambiguous":false}"#,
        );

        assert_eq!(apply_times(&mut snapshot, &times), 0);
        assert!(snapshot.events[0].all_day);
    }

    #[test]
    fn a_task_in_a_list_the_browser_did_not_report_stays_all_day() {
        let mut event = task_event(day(), "Stand-up");
        event.calendar_id = "tasks:other".to_owned();
        let mut snapshot = snapshot_with(event);
        let times = report(
            r#"{"list":"My Tasks","title":"Stand-up","time":"09:30","ambiguous":false}"#,
        );

        assert_eq!(apply_times(&mut snapshot, &times), 0);
        assert!(snapshot.events[0].all_day);
    }

    #[test]
    fn a_calendar_event_is_never_touched() {
        let mut event = task_event(day(), "Stand-up");
        event.id = "event:abc".to_owned();
        event.calendar_id = "someone@example.com".to_owned();
        let mut snapshot = snapshot_with(event);
        let times = report(
            r#"{"list":"Someone","title":"Stand-up","time":"09:30","ambiguous":false}"#,
        );

        assert_eq!(apply_times(&mut snapshot, &times), 0);
        assert!(snapshot.events[0].all_day);
    }

    /// The real thing: find the helper, run it headless, read its JSON.
    ///
    /// Skipped by default because it needs the helper built (`deno task
    /// compile` in `tools/tasks-bridge`) and a profile that has been signed in
    /// with `deno task signin`. Run it with:
    /// `cargo test --lib calendar::bridge -- --ignored`
    #[test]
    #[ignore = "needs the tasks-bridge helper built and signed in"]
    fn the_helper_reports_times_when_it_is_installed() {
        let bytes = run_helper("fetch").expect("the helper should be found and signed in");
        let times = parse_report(&bytes).expect("a readable report");
        assert!(!times.is_empty(), "expected the signed-in account to have a timed task");
    }

    #[test]
    #[ignore = "needs the tasks-bridge helper built"]
    fn the_helper_is_found_from_any_working_directory() {
        let original = std::env::current_dir().expect("a working directory");
        // A launcher or IDE does not run with the crate root as the CWD, which
        // is exactly how discovery used to miss the helper.
        std::env::set_current_dir("/").expect("the filesystem root");
        let found = helper_candidates().into_iter().find(|path| path.exists());
        std::env::set_current_dir(original).expect("the original directory");

        assert!(found.is_some(), "tasks-bridge should be found from any directory");
    }
}
