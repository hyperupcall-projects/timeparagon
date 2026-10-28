//! Moves the Google Keep "Habits & Rituals" notes and the Google Tasks
//! "Habits & Recurring" list into Calendar habits.
//!
//! Run it with no arguments to see what it would do:
//!
//! ```sh
//! cargo run --example migrate_keep_habits
//! ```
//!
//! and with `--apply` to create the habits and remove the task instances they
//! replace:
//!
//! ```sh
//! cargo run --example migrate_keep_habits -- --apply
//! ```
//!
//! Pass `--minutes N` to change how long each habit lasts (the default is 30).
//! Pass `--keep-tasks` to create the habits without removing the Google Tasks
//! they replace.
//! The Keep notes themselves are left untouched; the browser helper is
//! read-only, so archive them by hand once you are happy with the habits.

use time_slay::migrate_habits::{self, DEFAULT_MINUTES};

fn main() {
    let mut apply = false;
    let mut keep_tasks = false;
    let mut minutes = DEFAULT_MINUTES;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--apply" => apply = true,
            "--keep-tasks" => keep_tasks = true,
            "--minutes" => match arguments.next().map(|value| value.parse::<i64>()) {
                Some(Ok(value)) => minutes = value,
                _ => {
                    eprintln!("--minutes needs a whole number of minutes");
                    std::process::exit(2);
                }
            },
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!("usage: migrate_keep_habits [--apply] [--keep-tasks] [--minutes N]");
                std::process::exit(2);
            }
        }
    }
    if minutes < 1 {
        eprintln!("--minutes must be at least 1");
        std::process::exit(2);
    }

    match migrate_habits::run(apply, minutes, !keep_tasks) {
        Ok(report) => {
            println!(
                "{} habits migration",
                if report.dry_run { "Planned" } else { "Applied" }
            );
            if report.planned.is_empty() && report.deleted.is_empty() && report.retained.is_empty()
            {
                println!("  no Keep notes or tasks to migrate");
            }
            for habit in &report.planned {
                println!(
                    "  habit: {} — {}–{} on {}, {}",
                    habit.title,
                    habit.start.format("%H:%M"),
                    habit.end.format("%H:%M"),
                    habit.date,
                    habit.recurrence.join(" | ")
                );
            }
            if !report.created.is_empty() {
                println!("  created: {}", report.created.join(", "));
            }
            if !report.deleted.is_empty() {
                println!(
                    "  {} {} task instance(s):",
                    if report.dry_run {
                        "would remove"
                    } else {
                        "removed"
                    },
                    report.deleted.len()
                );
                for task in &report.deleted {
                    let due = task
                        .due
                        .map(|due| due.to_string())
                        .unwrap_or_else(|| "no date".to_owned());
                    println!("    {} ({due})", task.title);
                }
            }
            if !report.retained.is_empty() {
                println!(
                    "  left {} task instance(s) in Google Tasks:",
                    report.retained.len()
                );
                for task in &report.retained {
                    println!("    {}", task.title);
                }
            }
            for reason in &report.kept {
                println!("  leave: {reason}");
            }
            for reason in &report.skipped {
                println!("  skipped: {reason}");
            }
            for error in &report.errors {
                eprintln!("  error: {error}");
            }
            if report.dry_run {
                println!("Re-run with --apply to make these changes.");
            }
            if !report.errors.is_empty() {
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("migration failed: {error}");
            std::process::exit(1);
        }
    }
}
