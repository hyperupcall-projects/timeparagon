//! TimeSlay: slay the day with TimeSlay.
//!
//! The library backs a single binary, `time-slay`, so the interface can be
//! exercised by tests without a compositor. A flag picks what it runs (see
//! [`cli`]):
//!
//! * no flag — the main window (this crate's [`app`]);
//! * `--sidebar` — the side panel, the full app in a wide edge strip;
//! * `--dock` — the dock (see [`dock`]), a very thin strip of today;
//! * `--daemon` — the shortcut daemon (see [`panel`]), which shows, hides and
//!   switches between the panel and the dock.

pub mod alarm;
pub mod alarms;
pub mod app;
pub mod audio;
pub mod blocking;
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
pub mod notify;
pub mod panel;
pub mod projects;
pub mod theme;
pub mod todos;
pub mod travel;
pub mod ui;
pub mod window_effect;

pub mod timeblocking;

pub mod task_work;

pub mod task_db;
