//! The renderer-agnostic half of TimeSlay.
//!
//! Both the iced application and the Dioxus day view need calendars, OAuth,
//! Google Tasks and time-block metadata. Those modules live here without any
//! desktop UI dependencies, so they also cross-compile for Android.
//!
//! `config` holds the `[calendar]` settings both apps share and the minimal
//! data-only config the Dioxus app reads. The iced application embeds those
//! same settings in its full config.

pub mod calendar;
pub mod clock;
pub mod config;
pub mod config_sync;
pub mod fsutil;
pub mod hex;
pub mod places;
pub mod recurrence;
pub mod task_db;
pub mod task_edit;
pub mod theme;
pub mod timeblocking;

/// The color type used by calendar events and settings.
pub use iced_core::Color;
