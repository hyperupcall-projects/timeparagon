//! The clock, kept independent of the interface that draws it.

pub mod format;
pub mod timer;

use std::time::Duration;

pub use format::Precision;
pub use timer::Timer;

/// The count-up clock only shows whole seconds, so there is nothing to gain
/// from ticking faster than this; it is quick enough that the displayed second
/// never looks late.
pub const TIMER_TICK: Duration = Duration::from_millis(250);
