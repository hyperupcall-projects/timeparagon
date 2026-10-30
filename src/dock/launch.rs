//! Switching between the side panel and the dock.
//!
//! The dock runs in its own process. Its back button always returns to the
//! side panel, showing the existing panel process or starting one.

use std::path::PathBuf;
use std::process::{Child, Command};

use anyhow::{Context, Result};

/// The switch that makes the main binary run the dock.
pub const DOCK_FLAG: &str = "--dock";

/// The switch that makes the main binary run the side panel.
pub const SIDEBAR_FLAG: &str = "--sidebar";

/// Starts the dock. The caller is expected to put its own window away once
/// this succeeds.
///
/// The dock is the running program itself with [`DOCK_FLAG`], which is always
/// there, whatever was installed.
pub fn open_dock() -> Result<Child> {
    let exe = main_binary()?;
    Command::new(&exe)
        .arg(DOCK_FLAG)
        .spawn()
        .with_context(|| format!("failed to run {}", exe.display()))
}

/// Starts a side panel process, which opens its window straight away.
pub fn open_sidebar() -> Result<Child> {
    let exe = main_binary()?;
    Command::new(&exe)
        .arg(SIDEBAR_FLAG)
        .spawn()
        .with_context(|| format!("failed to run {}", exe.display()))
}

/// Shows the side panel. The dock exits afterwards.
pub fn close_dock() -> Result<()> {
    if crate::panel::show_panel().is_ok() {
        return Ok(());
    }
    open_sidebar()?;
    Ok(())
}

/// The `timeslay` binary: this very program when that is what is running,
/// and otherwise the one installed beside it.
/// Starts a fresh copy of this process, same mode and arguments, from
/// whatever binary is on disk now, a moment after this one has quit. The
/// shortcut daemon is restarted too, so it is not left on the old binary.
pub fn relaunch() -> Result<()> {
    use std::os::unix::process::CommandExt;

    // Linux marks the path of a replaced executable as deleted.
    let exe = std::env::current_exe().context("could not find the running binary")?;
    let exe = match exe
        .to_str()
        .and_then(|path| path.strip_suffix(" (deleted)"))
    {
        Some(path) => PathBuf::from(path),
        None => exe,
    };

    Command::new("sh")
        .arg("-c")
        .arg(
            "sleep 0.7; systemctl --user try-restart timeslay-shortcuts.service \
             >/dev/null 2>&1; exec \"$0\" \"$@\"",
        )
        .arg(&exe)
        .args(std::env::args_os().skip(1))
        .process_group(0)
        .spawn()
        .with_context(|| format!("failed to relaunch {}", exe.display()))?;
    Ok(())
}

pub(crate) fn main_binary() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && exe.file_stem().is_some_and(|stem| stem == "timeslay")
    {
        return Ok(exe);
    }
    find_binary("timeslay")
}

/// A binary installed beside this one, or else on `PATH`, so a manual
/// `cargo run` keeps working.
fn find_binary(name: &str) -> Result<PathBuf> {
    let sibling = std::env::current_exe().ok().and_then(|exe| {
        let candidate = exe.parent()?.join(name);
        candidate.is_file().then_some(candidate)
    });
    if let Some(path) = sibling {
        return Ok(path);
    }

    let on_path = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(name).is_file())
    });
    if on_path {
        return Ok(PathBuf::from(name));
    }

    anyhow::bail!("could not find {name} next to this program or on PATH")
}
