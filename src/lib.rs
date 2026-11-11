//! TimeSlay: slay the day with TimeSlay.
//!
//! The library backs a single binary, `timeslay`, so the interface can be
//! exercised by tests without a compositor. A subcommand picks what it runs
//! (see [`cli`]):
//!
//! * no subcommand — the side panel (the same as `sidebar`);
//! * `sidebar` — the side panel, the full app in a wide edge strip;
//! * `dock` — the dock (see [`dock`]), a very thin strip of today;
//! * `daemon` — the shortcut daemon (see [`panel`]), which shows, hides and
//!   switches between the panel and the dock;
//! * `webextension` — the browser native-messaging host (see
//!   [`webextension`]), which adds a link to the inbox and exits.

pub use timeslay_core::{
    calendar, clock, database_sync, fsutil, hex, recurrence, secrets, settings, shared_data,
    task_db, task_input,
};

pub mod activity;
pub mod alarm;
pub mod alarms;
pub mod app;
pub mod audio;
pub mod blocking;
pub mod brain_dump;
pub mod cli;
pub mod config;
pub mod dock;
pub mod habits;
pub mod inbox;
pub mod integrations;
pub mod ipc;
pub mod life;
pub mod local_poll;
pub mod notes;
pub mod notify;
pub mod panel;
pub mod people;
pub mod reminders;
pub mod theme;
pub mod travel;
pub mod ui;
pub mod webextension;
pub mod window_effect;

pub mod timeblocking;

pub mod pomodoro;
pub mod task_work;

pub mod image_input;
pub mod voice;
