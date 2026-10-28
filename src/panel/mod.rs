//! The shortcut daemon (`time-slay --daemon`): global shortcuts that show,
//! hide and reshape TimeSlay **itself**.
//!
//! Two mechanisms, the ones `../sidepanel` uses:
//!
//! 1. global shortcuts registered through KGlobalAccel (see [`hotkey`]), and
//! 2. a long-running daemon that owns them, so pressing one never pays for a
//!    process to be registered first.
//!
//! TimeSlay has two shapes, both started from the one binary: the **panel**
//! (`time-slay --sidebar`, the full app in a wide edge strip) and the
//! **dock** (`time-slay --dock`, a very thin strip of today). The daemon
//! draws neither. It has two actions, each of which can have several
//! shortcuts:
//!
//! * **Toggle Visibility** hides whichever shape is on screen, and when
//!   nothing is, brings back the shape that was last shown.
//! * **Toggle Shape** swaps the panel for the dock and the dock for the panel.
//!
//! The daemon keeps no record of what is on screen. It asks, over the sockets
//! in [`crate::ipc`], so the answer is right even when a shape was changed
//! with the buttons in the app rather than with a shortcut. The one thing it
//! does remember is which shape was shown last.
//!
//! The panel process is kept alive while hidden, so it comes back exactly as it
//! was (its tab, any open details pane, its width) and nearly instantly. The
//! dock holds no state worth keeping: hiding it ends the process.
//!
//! Where the panel lands is the compositor's business. [`config`] records the
//! edge and width we ask for; see the packaging README for the one caveat on
//! Wayland, where a client cannot dictate its own position.

mod config;
mod hotkey;

pub use config::{Action, PanelConfig, Side};
pub use hotkey::{rebind, same_shortcut, validate};

use std::process::Child;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};

use crate::dock::launch::{self, Origin};
use crate::ipc;

/// How long a freshly started shape gets to announce itself.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// The two forms TimeSlay takes on screen besides its regular window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Panel,
    Dock,
}

impl Shape {
    fn other(self) -> Self {
        match self {
            Self::Panel => Self::Dock,
            Self::Dock => Self::Panel,
        }
    }

    fn socket(self) -> &'static str {
        match self {
            Self::Panel => ipc::SIDEBAR,
            Self::Dock => ipc::DOCK,
        }
    }

    fn is_visible(self) -> bool {
        ipc::is_visible(self.socket())
    }

    fn hide(self) -> Result<()> {
        ipc::request(self.socket(), "hide").map(drop)
    }

    /// Brings the shape on screen, starting its process if there is none.
    fn show(self) -> Result<()> {
        match self {
            // A hidden panel is still running, and only has to be asked.
            Self::Panel if ipc::request(ipc::SIDEBAR, "show").is_ok() => Ok(()),
            Self::Panel => start(launch::open_sidebar()?, self),
            Self::Dock => start(launch::open_dock(Origin::Sidebar)?, self),
        }
    }
}

/// Waits for a freshly started `shape` to be on screen. Gives up early if its
/// process exits first, as the dock does where there is no layer shell.
fn start(mut child: Child, shape: Shape) -> Result<()> {
    let mut exited = None;
    let appeared = ipc::wait_until(START_TIMEOUT, || {
        if shape.is_visible() {
            return true;
        }
        exited = child.try_wait().ok().flatten();
        exited.is_some()
    });

    // Something has to collect the exit status whenever the process ends.
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    match exited {
        Some(status) => Err(anyhow!("the {shape:?} exited right away ({status})")),
        None if appeared => Ok(()),
        None => Err(anyhow!("the {shape:?} did not appear in time")),
    }
}

/// Reacts to the shortcuts.
struct Controller {
    /// The shape on screen last, which Toggle Visibility brings back.
    last: Shape,
}

impl Controller {
    fn handle(&mut self, action: Action) {
        match action {
            Action::ToggleVisibility => self.toggle_visibility(),
            Action::ToggleShape => self.toggle_shape(),
        }
    }

    /// The shapes on screen right now. More than one is not expected, but
    /// both are reported so that neither is left behind.
    fn visible(&self) -> Vec<Shape> {
        [Shape::Panel, Shape::Dock]
            .into_iter()
            .filter(|shape| shape.is_visible())
            .collect()
    }

    fn hide_all(&self, shapes: &[Shape]) {
        for shape in shapes {
            if let Err(error) = shape.hide() {
                log::warn!("could not hide the {shape:?}: {error:#}");
            }
        }
    }

    fn toggle_visibility(&mut self) {
        let visible = self.visible();
        if let Some(&shape) = visible.first() {
            self.last = shape;
            self.hide_all(&visible);
            return;
        }

        if let Err(error) = self.last.show() {
            log::error!("could not show the {:?}: {error:#}", self.last);
        }
    }

    fn toggle_shape(&mut self) {
        let visible = self.visible();
        // Hidden, it comes back in the shape it was not last shown in.
        let from = visible.first().copied().unwrap_or(self.last);
        let to = from.other();

        self.hide_all(&visible);
        match to.show() {
            Ok(()) => self.last = to,
            Err(error) => {
                log::error!("could not switch to the {to:?}: {error:#}");
                // Do not leave the screen empty because the other shape failed.
                if !visible.is_empty() {
                    if let Err(error) = from.show() {
                        log::error!("could not restore the {from:?}: {error:#}");
                    }
                }
            }
        }
    }
}

/// Runs the daemon: register the shortcuts, then act on every press.
pub fn run() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("time_slay=info"))
        .init();

    let config = PanelConfig::load();
    log::info!(
        "shortcut daemon starting (side={:?}, width={}px)",
        config.side,
        config.width
    );

    let (tx, rx) = std::sync::mpsc::channel();
    hotkey::spawn(tx, config).context("failed to start the global-shortcut listener")?;

    let mut controller = Controller { last: Shape::Panel };
    while let Ok(action) = rx.recv() {
        controller.handle(action);
    }

    Ok(())
}

/// Asks a running panel to show its window again. Fails when no panel is
/// running, so the caller can start one.
pub fn show_panel() -> Result<()> {
    ipc::request(ipc::SIDEBAR, "show").map(drop)
}
