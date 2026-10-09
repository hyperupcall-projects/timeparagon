//! TimeTracker: a count-up timer that stays out of the way.
//!
//! The binary is a thin wrapper around this library so that the layout and
//! the clock engine can be exercised by tests without a compositor.

pub mod alarm;
pub mod app;
pub mod audio;
pub mod blocking;
pub mod calendar;
pub mod config;
pub mod engine;
pub mod hex;
pub mod notify;
pub mod theme;
pub mod tracks;
pub mod ui;
