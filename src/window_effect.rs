//! Opening animation for the side panel, which is an ordinary xdg-toplevel
//! window. Layer-shell namespaces (as the dock uses) can't control those, so a
//! temporary KWin script effect, loaded over D-Bus, takes over instead: it
//! fades the panel in and out or shows it at once, and skips KWin's own pop-in.
//!
//! The panel's mode travels in its application ID (`time-slay-instant`,
//! `time-slay-fade-300`), which KWin sees when the window maps, so there is no
//! race with a later D-Bus message. Anything but KWin just gets the desktop
//! default.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::config::{ViewSettings, WindowAnimation};

const SCRIPT: &str = include_str!("window_effect.js");
const PREFIX: &str = "time-slay-opening-";

/// The application ID for a window that should open with `mode`.
pub fn application_id(mode: WindowAnimation, duration_ms: u32, desktop_default: &str) -> String {
    match mode {
        WindowAnimation::DesktopDefault => desktop_default.to_owned(),
        WindowAnimation::Instant => "time-slay-instant".to_owned(),
        WindowAnimation::FadeIn => format!(
            "time-slay-fade-{}",
            duration_ms.min(ViewSettings::MAX_FADE_DURATION_MS)
        ),
    }
}

static EFFECT: Mutex<Option<Effect>> = Mutex::new(None);

/// Makes sure KWin has the effect when the settings ask for one, and returns
/// the application ID to open the window with.
pub fn prepare(views: &ViewSettings, desktop_default: &str) -> String {
    let mode = views.window_animation;
    if mode != WindowAnimation::DesktopDefault {
        let mut slot = EFFECT.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_mut() {
            Some(effect) => effect.ensure_loaded(),
            None => match Effect::start() {
                Ok(effect) => *slot = Some(effect),
                Err(error) => log::warn!("panel opening control unavailable: {error:#}"),
            },
        }
    }
    application_id(mode, views.fade_duration_ms(), desktop_default)
}

/// Unloads the effect and removes its files. Call when the app is quitting.
pub fn shutdown() {
    drop(EFFECT.lock().unwrap_or_else(|e| e.into_inner()).take());
}

fn name(pid: u32) -> String {
    format!("{PREFIX}{pid}")
}

fn effects_dir() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(data.join("kwin/effects"))
}

fn connection() -> zbus::Result<zbus::blocking::Connection> {
    zbus::blocking::connection::Builder::session()?
        .method_timeout(std::time::Duration::from_secs(2))
        .build()
}

fn proxy(connection: &zbus::blocking::Connection) -> zbus::Result<zbus::blocking::Proxy<'_>> {
    zbus::blocking::Proxy::new(
        connection,
        "org.kde.KWin",
        "/Effects",
        "org.kde.kwin.Effects",
    )
}

struct Effect {
    connection: zbus::blocking::Connection,
    directory: PathBuf,
    name: String,
    loaded: bool,
}

impl Effect {
    fn start() -> anyhow::Result<Self> {
        let pid = std::process::id();
        let root = effects_dir().ok_or_else(|| anyhow::anyhow!("no user data directory"))?;
        sweep_stale(&root);

        let connection = connection()?;
        let directory = root.join(name(pid));
        std::fs::create_dir_all(&root)?;
        // A leftover from an earlier run with the same pid.
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;

        let mut effect = Self {
            connection,
            directory,
            name: name(pid),
            loaded: false,
        };
        let code = effect.directory.join("contents/code");
        std::fs::create_dir_all(&code)?;
        let metadata = serde_json::json!({
            "KPackageStructure": "KWin/Effect",
            "KPlugin": {
                "Id": effect.name,
                "Name": "TimeSlay panel opening",
                "EnabledByDefault": false,
                "License": "MIT"
            },
            "X-Plasma-API": "javascript",
            "X-KDE-Ordering": 0
        });
        for (path, contents) in [
            (effect.directory.join("metadata.json"), metadata.to_string()),
            (code.join("main.js"), SCRIPT.replace("__PID__", &pid.to_string())),
        ] {
            std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(path)?
                .write_all(contents.as_bytes())?;
        }

        let supported: bool = proxy(&effect.connection)?.call("isEffectSupported", &("scale",))?;
        anyhow::ensure!(supported, "KWin does not support window animation effects");
        effect.ensure_loaded();
        Ok(effect)
    }

    /// KPackage caches its list of installed packages, so a freshly written
    /// effect can be rejected at first; it is simply tried again on the next
    /// window.
    fn ensure_loaded(&mut self) {
        if !self.loaded {
            self.loaded = proxy(&self.connection)
                .and_then(|proxy| proxy.call("loadEffect", &(self.name.as_str(),)))
                .unwrap_or(false);
        }
    }
}

impl Drop for Effect {
    fn drop(&mut self) {
        if let Ok(proxy) = proxy(&self.connection) {
            let _ = proxy.call::<_, _, ()>("unloadEffect", &(self.name.as_str(),));
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

/// Removes the effects of panels that crashed before they could clean up.
fn sweep_stale(root: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(pid) = file_name
            .to_str()
            .and_then(|n| n.strip_prefix(PREFIX))
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
            if let Ok(connection) = connection() {
                if let Ok(proxy) = proxy(&connection) {
                    let _ = proxy.call::<_, _, ()>("unloadEffect", &(name(pid).as_str(),));
                }
            }
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_and_duration_ride_in_the_application_id() {
        assert_eq!(
            application_id(WindowAnimation::DesktopDefault, 300, "time-slay"),
            "time-slay"
        );
        assert_eq!(
            application_id(WindowAnimation::Instant, 300, "time-slay"),
            "time-slay-instant"
        );
        assert_eq!(
            application_id(WindowAnimation::FadeIn, 300, "time-slay"),
            "time-slay-fade-300"
        );
        assert_eq!(
            application_id(WindowAnimation::FadeIn, 99_999, "time-slay"),
            "time-slay-fade-5000"
        );
    }
}
