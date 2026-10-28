//! Settings for the dock strip, from `$XDG_CONFIG_HOME/time-slay/dock.toml`.
//!
//! Like the panel's config this is a file of its own, so a broken dock config
//! can never stop the clock from starting. Every field is optional.

use std::path::PathBuf;

use serde::Deserialize;

use crate::config::{ViewSettings, WindowAnimation};
use crate::panel::Side;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct DockConfig {
    /// Screen edge the strip hugs.
    pub side: Side,
    /// Width of the strip itself, in logical pixels. Not read from `dock.toml`:
    /// it is the "Dock width" in the app's settings.
    #[serde(skip)]
    pub width: u32,
    /// Space left above and below the strip, in logical pixels. Not read from
    /// `dock.toml`: it is the "Screen margin" in the app's settings, shared
    /// with the side panel.
    #[serde(skip)]
    pub margin: u32,
    /// How the strip appears, from the app's "Windowing" settings.
    #[serde(skip)]
    pub animation: WindowAnimation,
    /// Fade-in length in milliseconds, from the app's settings.
    #[serde(skip)]
    pub fade_duration_ms: u32,
    /// Whether to draw hour lines, from the app's settings.
    #[serde(skip)]
    pub hour_lines: bool,
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
            margin: 8,
            animation: WindowAnimation::DesktopDefault,
            fade_duration_ms: 150,
            hour_lines: true,
            popup_width: 300,
            popup_gap: 6,
        }
    }
}

impl DockConfig {
    /// Loads the config, falling back to defaults on any error, and clamps
    /// values that would make the surface unusable. The size and margin come
    /// from the app's own view settings.
    pub fn load(views: &ViewSettings) -> Self {
        let mut config = Self::from_file();
        config.width = views.dock_width().round() as u32;
        config.margin = views.margin().round() as u32;
        config.animation = views.window_animation;
        config.fade_duration_ms = views.fade_duration_ms();
        config.hour_lines = views.dock_hour_lines;
        config.sanitized()
    }

    fn from_file() -> Self {
        let Some(path) = path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };

        match toml::from_str::<Self>(&text) {
            Ok(config) => config,
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
        let config: DockConfig = toml::from_str("side = \"right\"").unwrap();

        assert_eq!(config.side, Side::Right);
        assert_eq!(config.popup_width, 300);
    }

    #[test]
    fn the_strip_size_and_margin_come_from_the_view_settings() {
        let views = ViewSettings {
            dock_width: 14.0,
            margin: 20.0,
            ..ViewSettings::default()
        };
        let config = DockConfig::load(&views);

        assert_eq!(config.width, 14);
        assert_eq!(config.margin, 20);
    }

    #[test]
    fn the_window_options_come_from_the_view_settings() {
        let views = ViewSettings {
            window_animation: WindowAnimation::FadeIn,
            fade_duration_ms: 400,
            dock_hour_lines: false,
            ..ViewSettings::default()
        };
        let config = DockConfig::load(&views);

        assert_eq!(config.animation, WindowAnimation::FadeIn);
        assert_eq!(config.fade_duration_ms, 400);
        assert!(!config.hour_lines);
    }

    #[test]
    fn width_and_margin_in_dock_toml_are_ignored() {
        let config: DockConfig = toml::from_str("width = 99\nmargin = 99").unwrap();

        assert_eq!(config.width, DockConfig::default().width);
        assert_eq!(config.margin, DockConfig::default().margin);
    }

    #[test]
    fn absurd_widths_are_clamped() {
        let mut config: DockConfig = toml::from_str("popup_width = 5").unwrap();
        config.width = 0;
        let config = config.sanitized();

        assert_eq!(config.width, 2);
        assert_eq!(config.popup_width, 120);
    }
}
