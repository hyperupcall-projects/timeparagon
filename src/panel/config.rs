//! Settings for the side-panel companion: the `[panel]` table of the main
//! `config.toml`.
//!
//! Every field is optional; a missing table or field falls back to the
//! defaults below, so the panel runs out of the box. Older installs kept these
//! in a `panel.toml` beside `config.toml`; [`legacy_path`] still finds it so
//! [`crate::config::Config::load`] can carry its values over.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which screen edge the strip is anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    #[default]
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelConfig {
    /// Screen edge to anchor the strip to. The README ships `left`.
    pub side: Side,
    /// Strip width in logical pixels. Height always fills the screen.
    pub width: u32,
    /// Global shortcut, in `Modifier+Key` form, e.g. `"Meta+U"`.
    pub shortcut: String,
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            side: Side::Left,
            width: 200,
            shortcut: "Meta+U".to_owned(),
        }
    }
}

impl PanelConfig {
    /// The `[panel]` table of the main config, falling back to defaults.
    pub fn load() -> Self {
        crate::config::Config::load().panel
    }

    /// The old standalone `panel.toml`, if it exists and parses.
    pub(crate) fn load_legacy() -> Option<Self> {
        let path = legacy_path()?;
        let text = std::fs::read_to_string(&path).ok()?;

        match toml::from_str(&text) {
            Ok(config) => Some(config),
            Err(error) => {
                log::warn!(
                    "ignoring invalid panel config at {}: {error}",
                    path.display()
                );
                None
            }
        }
    }
}

/// The old `panel.toml`, beside the main `config.toml`.
pub fn legacy_path() -> Option<PathBuf> {
    Some(
        crate::config::project_dirs()?
            .config_dir()
            .join("panel.toml"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_defaults_to_the_left_edge_at_two_hundred() {
        let config = PanelConfig::default();

        assert_eq!(config.side, Side::Left);
        assert_eq!(config.width, 200);
        assert_eq!(config.shortcut, "Meta+U");
    }

    #[test]
    fn a_partial_panel_config_keeps_the_other_defaults() {
        let config: PanelConfig = toml::from_str("width = 240").unwrap();

        assert_eq!(config.width, 240);
        assert_eq!(config.side, Side::Left);
        assert_eq!(config.shortcut, "Meta+U");
    }

    #[test]
    fn the_side_parses_lowercase() {
        assert_eq!(
            toml::from_str::<PanelConfig>("side = \"right\"")
                .unwrap()
                .side,
            Side::Right
        );
        assert_eq!(
            toml::from_str::<PanelConfig>("side = \"left\"")
                .unwrap()
                .side,
            Side::Left
        );
    }

    #[test]
    fn the_legacy_panel_config_lives_beside_the_main_one() {
        let panel = legacy_path().expect("a config directory");
        let main = crate::config::config_path().expect("a config directory");

        assert_eq!(panel.parent(), main.parent());
        assert_eq!(panel.file_name().unwrap(), "panel.toml");
    }
}
