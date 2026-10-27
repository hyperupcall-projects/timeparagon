//! The renderer-agnostic half of TimeTracker.
//!
//! Both the iced application and the Dioxus day view need calendars, OAuth,
//! Google Tasks and time-block metadata. Those modules live here without any
//! desktop UI dependencies, so they also cross-compile for Android.
//!
//! `calendar`, `config_sync`, `hex` and `task_db` are shared verbatim with the iced crate via
//! `#[path]`; `config` and `timeblocking` are the minimal data-only parts of
//! those modules. The iced application keeps its own full versions and remains
//! unchanged on desktop.

pub mod config;
pub mod timeblocking;

#[path = "../../src/calendar/mod.rs"]
pub mod calendar;
#[path = "../../src/config_sync.rs"]
pub mod config_sync;
#[path = "../../src/hex.rs"]
pub mod hex;
#[path = "../../src/task_db.rs"]
pub mod task_db;

/// The color type used by calendar events and settings.
pub use iced::Color;
