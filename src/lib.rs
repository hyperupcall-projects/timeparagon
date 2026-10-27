//! TimeTracker: a count-up timer that stays out of the way.
//!
//! The library backs two thin binaries so the interface and the clock engine can
//! be exercised by tests without a compositor:
//!
//! * `timetracker` — the main window (this crate's [`app`]).
//! * `timetracker-panel` — the side-panel companion (see [`panel`]): a 200px,
//!   left-anchored strip summoned with a global shortcut.

pub mod alarm;
pub mod alarms;
pub mod app;
pub mod audio;
pub mod blocking;
pub mod calendar;
pub mod config;
pub mod config_sync;
pub mod engine;
pub mod hex;
pub mod inbox;
pub mod integrations;
pub mod keep;
pub mod notify;
pub mod panel;
pub mod projects;
pub mod theme;
pub mod todos;
pub mod tracks;
pub mod ui;

pub mod timeblocking;

pub mod task_work;

pub mod task_db;
