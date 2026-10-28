//! Switching between the full window, the side panel and the dock.
//!
//! The dock is its own process, so moving to it means starting it and then
//! getting the window you came from out of the way, and moving back means the
//! reverse. The dock is told where it came from on its command line so its
//! "back" button can return there.

use std::path::PathBuf;
use std::process::{Child, Command};

use anyhow::{Context, Result};

/// The flag the dock is started with to say what it replaced.
pub const FROM_FLAG: &str = "--from";

/// What the dock replaced, and so what its back button returns to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Origin {
    /// The regular `timetracker` window.
    #[default]
    Window,
    /// The `timetracker --sidebar` panel.
    Sidebar,
}

impl Origin {
    fn name(self) -> &'static str {
        match self {
            Self::Window => "window",
            Self::Sidebar => "sidebar",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "window" => Some(Self::Window),
            "sidebar" => Some(Self::Sidebar),
            _ => None,
        }
    }

    /// What the back button says it will open.
    pub fn back_label(self) -> &'static str {
        match self {
            Self::Window => "Open the full window",
            Self::Sidebar => "Open the side panel",
        }
    }
}

/// The switch that makes the main binary run the dock.
pub const DOCK_FLAG: &str = "--dock";

/// The switch that makes the main binary run the side panel.
pub const SIDEBAR_FLAG: &str = "--sidebar";

/// Starts the dock. The caller is expected to put its own window away once
/// this succeeds.
///
/// The dock is the running program itself with [`DOCK_FLAG`], which is always
/// there, whatever was installed.
pub fn open_dock(origin: Origin) -> Result<Child> {
    let exe = main_binary()?;
    Command::new(&exe)
        .arg(DOCK_FLAG)
        .arg(FROM_FLAG)
        .arg(origin.name())
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

/// Brings back what the dock replaced. The dock exits afterwards.
pub fn close_dock(origin: Origin) -> Result<()> {
    match origin {
        Origin::Window => {
            let exe = main_binary()?;
            Command::new(&exe)
                .spawn()
                .with_context(|| format!("failed to run {}", exe.display()))?;
            Ok(())
        }
        Origin::Sidebar => {
            // The panel process outlives its window, so ask it to show again,
            // and only start a fresh one if nothing answers.
            if crate::panel::show_panel().is_ok() {
                return Ok(());
            }
            open_sidebar()?;
            Ok(())
        }
    }
}

/// The `timetracker` binary: this very program when that is what is running,
/// and otherwise the one installed beside it.
pub(crate) fn main_binary() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && exe.file_stem().is_some_and(|stem| stem == "timetracker")
    {
        return Ok(exe);
    }
    find_binary("timetracker")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_origin_defaults_to_the_window() {
        assert_eq!(Origin::default(), Origin::Window);
    }

    #[test]
    fn an_unknown_origin_is_not_parsed() {
        assert_eq!(Origin::parse("moon"), None);
    }

    #[test]
    fn what_the_dock_is_told_is_what_it_reads_back() {
        for origin in [Origin::Window, Origin::Sidebar] {
            assert_eq!(Origin::parse(origin.name()), Some(origin));
        }
    }
}
