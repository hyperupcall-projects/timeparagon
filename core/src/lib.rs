//! The renderer-agnostic half of TimeParagon.
//!
//! Both the iced application and the Dioxus day view need calendars, OAuth,
//! Google Tasks and time-block metadata. Those modules live here without any
//! desktop UI dependencies, so they also cross-compile for Android.
//!
//! `config` holds the calendar settings both apps share and the minimal
//! data-only config the Dioxus app reads. The iced application embeds those
//! same settings in its full config. `settings` keeps both in the database.

pub mod alarms;
pub mod brain_dump;
pub mod calendar;
pub mod clock;
pub mod config;
pub mod database_migration;
pub mod database_sync;
pub mod fsutil;
pub mod habit_edit;
pub mod habits;
pub mod hex;
mod legacy_name;
pub mod life;
pub mod local_changes;
pub mod notes;
pub mod people;
pub mod places;
pub mod recurrence;
pub mod reminders;
pub mod secrets;
pub mod settings;
pub mod shared_data;
pub mod sounds;
pub mod task_db;
pub mod task_edit;
pub mod task_input;
pub mod theme;
pub mod timeblocking;

/// The color type used by calendar events and settings.
pub use iced_core::Color;
