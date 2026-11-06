//! Watches local settings so the Dioxus app reloads edits made by another process.

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

static GENERATION: AtomicU64 = AtomicU64::new(0);
static WATCHER: OnceLock<Mutex<Option<notify::RecommendedWatcher>>> = OnceLock::new();

/// Starts watching local settings once per process. Settings never leave this device.
pub fn start() {
    WATCHER.get_or_init(|| {
        let watcher = crate::config::config_path().and_then(|path| {
            watch(&path, || {
                GENERATION.fetch_add(1, Ordering::Relaxed);
            })
            .map_err(|error| log::warn!("could not watch local settings: {error}"))
            .ok()
        });
        Mutex::new(watcher)
    });
}

pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Watch the directory: editors and app saves replace the file by renaming.
fn watch(
    path: &Path,
    changed: impl Fn() + Send + 'static,
) -> io::Result<notify::RecommendedWatcher> {
    use notify::{EventKind, Watcher};

    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("settings have no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let watched = path.to_owned();
    let mut watcher =
        notify::recommended_watcher(move |event: notify::Result<notify::Event>| match event {
            Ok(event)
                if event.paths.iter().any(|path| path == &watched)
                    && matches!(
                        event.kind,
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                    ) =>
            {
                changed();
            }
            Err(error) => log::warn!("settings watcher error: {error}"),
            _ => {}
        })
        .map_err(io::Error::other)?;
    watcher
        .watch(parent, notify::RecursiveMode::NonRecursive)
        .map_err(io::Error::other)?;
    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_still_watched_after_atomic_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        std::fs::write(&path, "value = 1").unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let _watcher = watch(&path, move || {
            let _ = sender.send(());
        })
        .unwrap();

        for value in [2, 3] {
            while receiver.try_recv().is_ok() {}
            let replacement = directory.path().join("replacement.toml");
            std::fs::write(&replacement, format!("value = {value}")).unwrap();
            std::fs::rename(&replacement, &path).unwrap();
            receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            assert_eq!(
                std::fs::read_to_string(&path).unwrap(),
                format!("value = {value}")
            );
        }
    }
}
