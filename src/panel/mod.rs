//! The side-panel launcher: a global shortcut that opens TimeTracker **itself**
//! as a separate, 200px-wide instance down a screen edge.
//!
//! Two mechanisms, the ones `../sidepanel` uses:
//!
//! 1. a global shortcut registered through KGlobalAccel (see [`hotkey`]), and
//! 2. a long-running daemon that owns the shortcut, so pressing it never pays
//!    for a process to be registered first.
//!
//! The daemon does not draw anything itself. When the shortcut fires it starts
//! a **separate instance of the real application** (`timetracker --sidebar`),
//! and pressing the shortcut again closes that instance. Keeping the panel a
//! genuine instance of the app — rather than a second, bespoke view — is the
//! point: it is the same clock, the same calendar, the same settings.
//!
//! Where the panel lands is the compositor's business. [`config`] records the
//! edge and width we ask for; see the packaging README for the one caveat on
//! Wayland, where a client cannot dictate its own position.

mod config;
mod hotkey;

pub use config::{PanelConfig, Side};
pub use hotkey::HotkeyEvent;

use std::path::PathBuf;
use std::process::{Child, Command};

use anyhow::{Context, Result};

/// The command-line switch that turns the main binary into the panel.
const SIDEBAR_FLAG: &str = "--sidebar";

/// Runs the daemon: register the shortcut, then toggle the panel on every press.
pub fn run() -> Result<()> {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("timetracker=info"),
    )
    .init();

    let config = PanelConfig::load();
    log::info!(
        "timetracker-panel starting (side={:?}, width={}px, shortcut={})",
        config.side,
        config.width,
        config.shortcut
    );

    let (tx, rx) = std::sync::mpsc::channel();
    hotkey::spawn(tx, config).context("failed to start the global-shortcut listener")?;

    // The panel is a child process; `None` means it is not currently up.
    let mut panel: Option<Child> = None;

    while rx.recv().is_ok() {
        toggle(&mut panel);
    }

    Ok(())
}

/// Shows the panel if it is down, or takes it down if it is up.
///
/// A child that has already exited on its own (the user closed the window) is
/// simply reaped — the next press opens a fresh one.
fn toggle(panel: &mut Option<Child>) {
    if let Some(mut running) = panel.take() {
        match running.try_wait() {
            Ok(Some(_)) => log::info!("panel had already closed"),
            Ok(None) => {
                if let Err(error) = running.kill() {
                    log::warn!("could not close the panel: {error}");
                }
                let _ = running.wait();
                log::info!("panel closed");
            }
            Err(error) => log::warn!("could not check on the panel: {error}"),
        }
        return;
    }

    match open_panel() {
        Ok(child) => {
            log::info!("panel opened (pid {})", child.id());
            *panel = Some(child);
        }
        Err(error) => log::error!("could not open the panel: {error:#}"),
    }
}

fn open_panel() -> Result<Child> {
    let exe = sidebar_exe()?;
    Command::new(&exe)
        .arg(SIDEBAR_FLAG)
        .spawn()
        .with_context(|| format!("failed to run {}", exe.display()))
}

/// The main `timetracker` binary, which understands [`SIDEBAR_FLAG`].
///
/// It is normally installed beside this daemon, so we look there first;
/// falling back to `PATH` keeps a manual `cargo run` working.
fn sidebar_exe() -> Result<PathBuf> {
    let sibling = std::env::current_exe().ok().and_then(|exe| {
        let candidate = exe.parent()?.join("timetracker");
        candidate.is_file().then_some(candidate)
    });

    if let Some(path) = sibling {
        return Ok(path);
    }

    if which("timetracker") {
        return Ok(PathBuf::from("timetracker"));
    }

    anyhow::bail!(
        "could not find the timetracker binary next to {} and it is not on PATH",
        std::env::current_exe()
            .map(|exe| exe.display().to_string())
            .unwrap_or_else(|_| "this daemon".to_owned())
    )
}

/// A minimal `PATH` lookup, so we do not pull in a `which` crate for one call.
fn which(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };

    std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
}
