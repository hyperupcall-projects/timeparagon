//! `timetracker-panel` — the side-panel companion.
//!
//! A slim, left-anchored TimeTracker strip that is summoned with a global
//! shortcut (`Meta+U` by default). It is a separate, long-running process from
//! the main window so that it can be a `zwlr_layer_shell_v1` surface — the only
//! thing on Wayland that can promise a fixed 200px width down one screen edge,
//! above every other window.
//!
//! The daemon lives in the library, so this is just the entry point.

fn main() -> anyhow::Result<()> {
    timetracker::panel::run()
}
