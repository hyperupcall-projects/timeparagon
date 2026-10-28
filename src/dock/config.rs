//! Settings for the dock strip, from `$XDG_CONFIG_HOME/time-slay/dock.toml`.
//!
//! Like the panel's config this is a file of its own, so a broken dock config
//! can never stop the clock from starting. Every field is optional.

use std::path::PathBuf;

use serde::Deserialize;

use crate::config::{DockSide, ViewSettings, WindowAnimation};
use crate::hex::HexColor;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct DockConfig {
    /// Screen edge the strip hugs. Not read from `dock.toml`: it is the
    /// "Dock edge" in the app's settings.
    #[serde(skip)]
    pub side: DockSide,
    /// Width of the strip itself, in logical pixels. Not read from `dock.toml`:
    /// it is the "Dock width" in the app's settings.
    #[serde(skip)]
    pub width: u32,
    /// Space left above and below the strip, in logical pixels. Not read from
    /// `dock.toml`: it is the "Screen margin" in the app's settings, shared
    /// with the side panel.
    #[serde(skip)]
    pub margin: u32,
    /// Thickness of the outermost border, in logical pixels. Zero hides it.
    #[serde(skip)]
    pub border_outer_width: f32,
    /// Colour of the outermost border.
    #[serde(skip)]
    pub border_outer_color: HexColor,
    /// Thickness of the border just inside the outermost one. Zero hides it.
    #[serde(skip)]
    pub border_inner_width: f32,
    /// Colour of the border just inside the outermost one.
    #[serde(skip)]
    pub border_inner_color: HexColor,
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
            side: DockSide::Left,
            width: 10,
            margin: 8,
            border_outer_width: 1.0,
            border_outer_color: HexColor(iced::Color::WHITE),
            border_inner_width: 1.0,
            border_inner_color: HexColor(iced::Color::BLACK),
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
    /// values that would make the surface unusable. The size, edge and border
    /// come from the app's own view settings.
    pub fn load(views: &ViewSettings) -> Self {
        let mut config = Self::from_file();
        config.width = views.dock_width().round() as u32;
        config.margin = views.margin().round() as u32;
        config.side = views.dock_side;
        config.border_outer_width = views.dock_border_outer_width();
        config.border_outer_color = views.dock_border_outer_color;
        config.border_inner_width = views.dock_border_inner_width();
        config.border_inner_color = views.dock_border_inner_color;
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
        self.border_outer_width = Self::border_width(self.border_outer_width);
        self.border_inner_width = Self::border_width(self.border_inner_width);
        self.popup_width = self.popup_width.clamp(120, 1000);
        self.popup_gap = self.popup_gap.min(100);
        self
    }

    fn border_width(value: f32) -> f32 {
        if value.is_finite() {
            value.clamp(0.0, crate::config::ViewSettings::MAX_DOCK_BORDER_WIDTH)
        } else {
            0.0
        }
    }

    /// How much of the strip the two borders take up, leaving at least one
    /// pixel for events even when the strip is at its thinnest.
    pub fn border_inset(&self) -> f32 {
        let wanted = self.border_outer_width + self.border_inner_width;
        wanted.min((self.width as f32 - 1.0).max(0.0))
    }

    /// Width of the whole layer surface: the strip plus room for the hover
    /// card beside it. Used when the strip runs along a vertical edge.
    pub fn surface_width(&self) -> u32 {
        self.width + self.popup_gap + self.popup_width
    }

    /// Height of the whole layer surface: the strip plus room for the hover
    /// card beside it. Used when the strip runs along a horizontal edge.
    pub fn surface_height(&self) -> u32 {
        self.surface_width()
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

        assert_eq!(config.side, DockSide::Left);
        assert_eq!(config.width, 10);
        assert_eq!(config.surface_width(), 10 + 6 + 300);
        assert_eq!(config.border_outer_width + config.border_inner_width, 2.0);
    }

    #[test]
    fn a_partial_config_keeps_the_other_defaults() {
        let config: DockConfig = toml::from_str("popup_width = 250").unwrap();

        assert_eq!(config.popup_width, 250);
        assert_eq!(config.side, DockSide::Left);
    }

    #[test]
    fn the_strip_size_margin_and_border_come_from_the_view_settings() {
        let views = ViewSettings {
            dock_width: 14.0,
            margin: 20.0,
            dock_side: DockSide::Top,
            dock_border_outer_width: 2.0,
            dock_border_outer_color: crate::hex::HexColor(iced::Color::WHITE),
            dock_border_inner_width: 3.0,
            dock_border_inner_color: crate::hex::HexColor(iced::Color::BLACK),
            ..ViewSettings::default()
        };
        let config = DockConfig::load(&views);

        assert_eq!(config.width, 14);
        assert_eq!(config.margin, 20);
        assert_eq!(config.side, DockSide::Top);
        assert_eq!(config.border_outer_width, 2.0);
        assert_eq!(config.border_inner_width, 3.0);
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
