//! Moves the legacy TimeSlay calendars into the single TimeSlay calendar.
//!
//! Run it with no arguments to see what it would do:
//!
//! ```sh
//! cargo run --example migrate_calendar
//! ```
//!
//! and with `--apply` to actually move the events and remove the empty source
//! calendars:
//!
//! ```sh
//! cargo run --example migrate_calendar -- --apply
//! ```
//!
//! It uses the Google token the app already stored, so connect Google in the
//! app first if you have not.

use time_slay::migrate;

fn main() {
    let apply = std::env::args().any(|argument| argument == "--apply");
    match migrate::run(apply) {
        Ok(report) => {
            println!(
                "{} migration into {:?}",
                if report.dry_run { "Planned" } else { "Applied" },
                report.target
            );
            if report.moved.is_empty() {
                println!("  no source calendars found");
            }
            for calendar in &report.moved {
                println!("  {}: {} event(s)", calendar.name, calendar.events);
            }
            if !report.deleted.is_empty() {
                println!(
                    "  {}: {}",
                    if report.dry_run {
                        "would remove"
                    } else {
                        "removed"
                    },
                    report.deleted.join(", ")
                );
            }
            for error in &report.errors {
                eprintln!("  error: {error}");
            }
            if report.dry_run {
                println!("Re-run with --apply to make these changes.");
            }
        }
        Err(error) => {
            eprintln!("migration failed: {error}");
            std::process::exit(1);
        }
    }
}
