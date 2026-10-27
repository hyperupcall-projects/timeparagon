//! Settings for the dock strip, from `$XDG_CONFIG_HOME/timetracker/dock.toml`.
//!
//! Like the panel's config this is a file of its own, so a broken dock config
//! can never stop the clock from starting. Every field is optional.

use std::path::PathBuf;

use serde::Deserialize;

use crate::panel::Side;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct DockConfig {
    /// Screen edge the strip hugs.
    pub side: Side,
    /// Width of the strip itself, in logical pixels.
    pub width: u32,
    /// Space left above the strip, in logical pixels.
    pub margin_top: u32,
    /// Space left below the strip, in logical pixels.
    pub margin_bottom: u32,
    /// Width of the hover card, in logical pixels.
    pub popup_width: u32,
    /// Space between the strip and the hover card, in logical pixels.
    pub popup_gap: u32,
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            side: Side::Left,
            width: 10,
            margin_top: 12,
            margin_bottom: 12,
            popup_width: 300,
            popup_gap: 6,
        }
    }
}

impl DockConfig {
    /// Loads the config, falling back to defaults on any error, and clamps
    /// values that would make the surface unusable.
    pub fn load() -> Self {
        let Some(path) = path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };

        match toml::from_str::<Self>(&text) {
            Ok(config) => config.sanitized(),
            Err(error) => {
                log::warn!(
                    "ignoring invalid dock config at {}: {error}",
                    path.display()
                );
                Self::default()
            }
        }
    }

    fn sanitized(mut self) -> Self {
        self.width = self.width.clamp(2, 200);
        self.popup_width = self.popup_width.clamp(120, 1000);
        self.popup_gap = self.popup_gap.min(100);
        self
    }

    /// Width of the whole layer surface: the strip plus room for the hover
    /// card beside it.
    pub fn surface_width(&self) -> u32 {
        self.width + self.popup_gap + self.popup_width
    }
}

/// `dock.toml`, beside the main `config.toml`.
pub fn path() -> Option<PathBuf> {
    Some(
        crate::config::project_dirs()?
            .config_dir()
            .join("dock.toml"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dock_defaults_to_a_ten_pixel_strip_on_the_left() {
        let config = DockConfig::default();

        assert_eq!(config.side, Side::Left);
        assert_eq!(config.width, 10);
        assert_eq!(config.surface_width(), 10 + 6 + 300);
    }

    #[test]
    fn a_partial_config_keeps_the_other_defaults() {
        let config: DockConfig = toml::from_str("width = 14\nside = \"right\"").unwrap();

        assert_eq!(config.width, 14);
        assert_eq!(config.side, Side::Right);
        assert_eq!(config.margin_top, 12);
    }

    #[test]
    fn absurd_widths_are_clamped() {
        let config: DockConfig = toml::from_str("width = 0\npopup_width = 5").unwrap();
        let config = config.sanitized();

        assert_eq!(config.width, 2);
        assert_eq!(config.popup_width, 120);
    }
}
