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
//! and pressing the shortcut again tells that instance to hide its window.
//! Keeping the panel a genuine instance of the app — rather than a second,
//! bespoke view — is the point: it is the same clock, the same calendar, the
//! same settings.
//!
//! The panel process is kept alive across toggles so that showing it again is
//! nearly instant: only the first press pays for a full process start. The
//! daemon talks to the panel over a Unix socket (`panel_socket_path`) to show
//! or hide its window.
//!
//! Where the panel lands is the compositor's business. [`config`] records the
//! edge and width we ask for; see the packaging README for the one caveat on
//! Wayland, where a client cannot dictate its own position.

mod config;
mod hotkey;

pub use config::{PanelConfig, Side};
pub use hotkey::HotkeyEvent;

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

/// The command-line switch that turns the main binary into the panel.
const SIDEBAR_FLAG: &str = "--sidebar";

/// Name of the Unix socket the panel daemon uses to tell a running panel
/// instance to show or hide its window.
const PANEL_SOCKET_NAME: &str = "timetracker-panel.sock";

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
    // `panel_visible` tracks whether the panel's window is currently shown,
    // so the daemon knows whether to send "show" or "hide" on the next press.
    let mut panel: Option<Child> = None;
    let mut panel_visible = false;

    while rx.recv().is_ok() {
        toggle(&mut panel, &mut panel_visible);
    }

    Ok(())
}

/// Shows the panel if it is down, or hides it if it is up.
///
/// Instead of killing the panel process on hide, we keep it alive and send it
/// a "hide" command over its IPC socket. The next press sends "show", which is
/// nearly instant because the process is already running.
///
/// A child that has already exited on its own (the user closed the window) is
/// simply reaped — the next press opens a fresh one.
fn toggle(panel: &mut Option<Child>, panel_visible: &mut bool) {
    if let Some(mut running) = panel.take() {
        match running.try_wait() {
            Ok(Some(_)) => {
                log::info!("panel had already closed");
                *panel_visible = false;
            }
            Ok(None) => {
                if *panel_visible {
                    if let Err(error) = send_panel_command("hide") {
                        log::warn!("could not hide panel: {error}");
                    }
                    *panel_visible = false;
                } else {
                    if let Err(error) = send_panel_command("show") {
                        log::warn!("could not show panel: {error}");
                    }
                    *panel_visible = true;
                }
                *panel = Some(running);
                return;
            }
            Err(error) => log::warn!("could not check on the panel: {error}"),
        }
    }

    match open_panel() {
        Ok(child) => {
            log::info!("panel opened (pid {})", child.id());
            *panel = Some(child);
            *panel_visible = true;
        }
        Err(error) => log::error!("could not open the panel: {error:#}"),
    }
}

/// Returns the path of the panel IPC socket, preferring the XDG runtime
/// directory and falling back to the temp directory.
pub fn panel_socket_path() -> Option<PathBuf> {
    std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .map(|dir| PathBuf::from(dir).join(PANEL_SOCKET_NAME))
        .or_else(|| {
            Some(
                std::env::temp_dir().join(PANEL_SOCKET_NAME),
            )
        })
}

/// Sends a `show` or `hide` command to a running panel instance over its IPC
/// socket. Retries briefly because the panel may not have bound the socket yet
/// right after it was spawned.
fn send_panel_command(command: &str) -> Result<()> {
    let path = panel_socket_path().context("no socket directory available")?;
    let mut last_error = None;
    for _ in 0..20 {
        match UnixStream::connect(&path) {
            Ok(mut stream) => {
                stream.write_all(command.as_bytes())?;
                return Ok(());
            }
            Err(error) => {
                last_error = Some(error);
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    Err(last_error
        .map(|e| anyhow!("panel socket not available: {e}"))
        .unwrap_or_else(|| anyhow!("panel socket not available")))
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
