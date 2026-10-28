//! The renderer-agnostic half of TimeSlay.
//!
//! Both the iced application and the Dioxus day view need calendars, OAuth,
//! Google Tasks and time-block metadata. Those modules live here without any
//! desktop UI dependencies, so they also cross-compile for Android.
//!
//! `calendar`, `config_sync`, `hex`, `task_db` and the pure part of
//! `timeblocking` are shared verbatim with the iced crate via `#[path]`;
//! `config` is the minimal data-only part of the iced crate's config. The iced
//! application keeps its own full version and remains unchanged on desktop.

pub mod config;

#[path = "../../src/calendar/mod.rs"]
pub mod calendar;
#[path = "../../src/timeblocking/model.rs"]
pub mod timeblocking;
#[path = "../../src/config_sync.rs"]
pub mod config_sync;
#[path = "../../src/hex.rs"]
pub mod hex;
#[path = "../../src/task_db.rs"]
pub mod task_db;

/// The color type used by calendar events and settings.
pub use iced::Color;
