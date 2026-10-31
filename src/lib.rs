//! TimeSlay: slay the day with TimeSlay.
//!
//! The library backs a single binary, `timeslay`, so the interface can be
//! exercised by tests without a compositor. A flag picks what it runs (see
//! [`cli`]):
//!
//! * no flag — the side panel (the same as `--sidebar`);
//! * `--sidebar` — the side panel, the full app in a wide edge strip;
//! * `--dock` — the dock (see [`dock`]), a very thin strip of today;
//! * `--daemon` — the shortcut daemon (see [`panel`]), which shows, hides and
//!   switches between the panel and the dock;
//! * `--webextension` — the browser native-messaging host (see
//!   [`webextension`]), which adds a link to the inbox and exits.

pub mod alarm;
pub mod alarms;
pub mod app;
pub mod audio;
pub mod blocking;
pub mod brain_dump;
pub mod calendar;
pub mod cli;
pub mod config;
pub mod config_sync;
pub mod dock;
pub mod habits;
pub mod hex;
pub mod inbox;
pub mod integrations;
pub mod ipc;
pub mod keep;
pub mod migrate;
pub mod migrate_habits;
pub mod notify;
pub mod panel;
pub mod projects;
pub mod recurrence;
pub mod theme;
pub mod todos;
pub mod travel;
pub mod ui;
pub mod webextension;
pub mod window_effect;

pub mod timeblocking;

pub mod pomodoro;
pub mod task_work;

pub mod task_db;
pub mod task_input;

pub mod voice;
