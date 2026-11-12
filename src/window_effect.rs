//! Open and close transitions for this process's windows. A temporary KWin
//! script effect, loaded over D-Bus, fades them or shows them at once, and skips
//! KWin's own pop-in.
//!
//! KWin does not show a layer surface's namespace to scripts: `windowClass` comes
//! from the executable. So the effect picks out windows by process, with the
//! duration written into its script, and is reloaded when the setting changes.
//! Anything but KWin just gets the desktop default.

use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::sync::Mutex;

use crate::config::{ViewSettings, WindowAnimation};

const SCRIPT: &str = include_str!("window_effect.js");
const PREFIX: &str = "timeslay-opening-";

/// How long the transition lasts, or `None` to leave it to the desktop.
fn duration(views: &ViewSettings) -> Option<u32> {
	match views.window_animation {
		WindowAnimation::DesktopDefault => None,
		WindowAnimation::Instant => Some(0),
		WindowAnimation::FadeIn => Some(views.fade_duration_ms()),
	}
}

fn script(pid: u32, duration_ms: u32) -> String {
	SCRIPT
		.replace("__PID__", &pid.to_string())
		.replace("__DURATION__", &duration_ms.to_string())
}

static EFFECT: Mutex<Option<Effect>> = Mutex::new(None);

/// Makes KWin's effect match the settings. KWin handles this before the call
/// returns, so a window opened afterwards cannot race it.
pub fn prepare(views: &ViewSettings) {
	let wanted = duration(views);
	let mut slot = EFFECT.lock().unwrap_or_else(|e| e.into_inner());
	match (slot.as_mut(), wanted) {
		(Some(effect), _) => {
			if let Err(error) = effect.apply(wanted) {
				log::warn!("window transition unavailable: {error:#}");
			}
		}
		(None, Some(duration_ms)) => match Effect::start(duration_ms) {
			Ok(effect) => *slot = Some(effect),
			Err(error) => log::warn!("window transition unavailable: {error:#}"),
		},
		(None, None) => {}
	}
}

/// Unloads the effect and removes its files. Call when the app is quitting, after
/// its windows are gone: unloading ends a close transition, so it has to finish
/// first, and KWin has to see the window close while the effect is still loaded.
pub fn shutdown() {
	let effect = EFFECT.lock().unwrap_or_else(|e| e.into_inner()).take();
	if let Some(duration_ms) = effect.as_ref().and_then(|effect| effect.loaded) {
		std::thread::sleep(std::time::Duration::from_millis(
			u64::from(duration_ms) + 50,
		));
	}
	drop(effect);
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
	/// What the loaded script does; `None` when nothing is loaded.
	loaded: Option<u32>,
}

impl Effect {
	fn start(duration_ms: u32) -> anyhow::Result<Self> {
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
			loaded: None,
		};
		std::fs::create_dir_all(effect.script_path().parent().expect("has a parent"))?;
		let metadata = serde_json::json!({
			"KPackageStructure": "KWin/Effect",
			"KPlugin": {
				"Id": effect.name,
				"Name": "TimeSlay window transitions",
				"EnabledByDefault": false,
				"License": "MIT"
			},
			"X-Plasma-API": "javascript",
			"X-KDE-Ordering": 0
		});
		write_private(
			&effect.directory.join("metadata.json"),
			&metadata.to_string(),
		)?;

		let supported: bool = proxy(&effect.connection)?.call("isEffectSupported", &("scale",))?;
		anyhow::ensure!(supported, "KWin does not support window animation effects");
		effect.apply(Some(duration_ms))?;
		Ok(effect)
	}

	fn script_path(&self) -> PathBuf {
		self.directory.join("contents/code/main.js")
	}

	/// Swaps the loaded script for one that does `wanted`. KWin reads the file
	/// again on each load. A failed load stays unloaded and is tried again on
	/// the next window: KPackage caches its list of installed packages, so a
	/// freshly written effect can be rejected at first.
	fn apply(&mut self, wanted: Option<u32>) -> anyhow::Result<()> {
		if self.loaded == wanted {
			return Ok(());
		}
		let proxy = proxy(&self.connection)?;
		if self.loaded.take().is_some() {
			proxy.call::<_, _, ()>("unloadEffect", &(self.name.as_str(),))?;
		}
		if let Some(duration_ms) = wanted {
			write_private(
				&self.script_path(),
				&script(std::process::id(), duration_ms),
			)?;
			if proxy.call("loadEffect", &(self.name.as_str(),))? {
				self.loaded = wanted;
			}
		}
		Ok(())
	}
}

fn write_private(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
	std::fs::OpenOptions::new()
		.create(true)
		.truncate(true)
		.write(true)
		.mode(0o600)
		.open(path)?
		.write_all(contents.as_bytes())
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
			if let Ok(connection) = connection()
				&& let Ok(proxy) = proxy(&connection)
			{
				let _ = proxy.call::<_, _, ()>("unloadEffect", &(name(pid).as_str(),));
			}
			let _ = std::fs::remove_dir_all(entry.path());
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn views(window_animation: WindowAnimation, fade_duration_ms: u32) -> ViewSettings {
		ViewSettings {
			window_animation,
			fade_duration_ms,
			..Default::default()
		}
	}

	#[test]
	fn each_setting_picks_its_duration() {
		assert_eq!(duration(&views(WindowAnimation::DesktopDefault, 300)), None);
		assert_eq!(duration(&views(WindowAnimation::Instant, 300)), Some(0));
		assert_eq!(duration(&views(WindowAnimation::FadeIn, 300)), Some(300));
		assert_eq!(
			duration(&views(WindowAnimation::FadeIn, 99_999)),
			Some(ViewSettings::MAX_FADE_DURATION_MS)
		);
	}

	#[test]
	fn the_script_is_written_for_this_process_and_duration() {
		let script = script(1234, 300);
		assert!(script.contains("window.pid !== 1234"));
		assert!(script.contains("const DURATION = 300;"));
		assert!(!script.contains("__"));
	}
}
