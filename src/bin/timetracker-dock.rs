//! `timetracker-dock` — a thin strip of today pinned to a screen edge.
//!
//! The strip is a `zwlr_layer_shell_v1` surface showing the highest-priority
//! calendar events and Google Tasks as coloured spans, with a red line for
//! "right now". Hover a span for its details. See `timetracker::dock`.
//!
//! `timetracker --dock` runs the same thing, which is how the window starts it.

fn main() -> anyhow::Result<()> {
    timetracker::dock::run()
}
