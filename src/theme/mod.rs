//! The four selectable light, dark, and high contrast themes.

pub mod palettes;

use std::sync::LazyLock;

use iced::theme::Palette;
use iced::{Color, Theme};

/// The colors a six-slot [`Palette`] has no room for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Accents {
    /// The big readout.
    pub digits: Color,
    /// The progress ring.
    pub ring: Color,
    /// The screen flash, when the config does not override it.
    pub flash: Color,
}

impl Accents {
    /// Picks sensible accents out of a theme's generated palette.
    ///
    /// The strong primary is used for the digits because the base primary is
    /// tuned for filled buttons and tends to be too light to read as text on
    /// the background.
    pub fn derive(theme: &Theme) -> Self {
        let palette = theme.extended_palette();

        Self {
            digits: palette.primary.strong.color,
            ring: palette.primary.base.color,
            flash: palette.danger.base.color,
        }
    }
}

/// One selectable theme.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub theme: Theme,
    pub accents: Accents,
}

impl Entry {
    fn calm(name: &str, palette: Palette, surfaces: [u32; 7], muted: Color) -> Self {
        let theme = Theme::custom_with_fn(name.to_owned(), palette, |palette| {
            palettes::calm_extended(palette, surfaces, muted)
        });

        Self {
            name: name.to_owned(),
            accents: Accents::derive(&theme),
            theme,
        }
    }

    fn high_contrast(name: &str, palette: Palette, surfaces: [u32; 7]) -> Self {
        let theme = Theme::custom_with_fn(name.to_owned(), palette, |palette| {
            palettes::high_contrast_extended(palette, surfaces)
        });

        Self {
            name: name.to_owned(),
            accents: Accents::derive(&theme),
            theme,
        }
    }
}

/// Every theme the picker offers.
#[derive(Debug, Clone)]
pub struct Catalog {
    entries: Vec<Entry>,
}

impl Catalog {
    /// The catalog is fixed; user theme files do not add picker options.
    pub fn load() -> Self {
        Self::built_in()
    }

    pub fn built_in() -> Self {
        Self {
            entries: vec![
                Entry::calm(
                    "Light",
                    palettes::LIGHT,
                    palettes::LIGHT_SURFACES,
                    Color::from_rgb8(0x76, 0x71, 0x6A),
                ),
                Entry::high_contrast(
                    "Light high contrast",
                    palettes::LIGHT_HIGH_CONTRAST,
                    palettes::LIGHT_HIGH_CONTRAST_SURFACES,
                ),
                Entry::calm(
                    "Dark",
                    palettes::DARK,
                    palettes::DARK_SURFACES,
                    Color::from_rgb8(0xA3, 0x9E, 0x95),
                ),
                Entry::high_contrast(
                    "Dark high contrast",
                    palettes::DARK_HIGH_CONTRAST,
                    palettes::DARK_HIGH_CONTRAST_SURFACES,
                ),
            ],
        }
    }

    pub fn names(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    /// Falls back to Light when the configured theme has been removed.
    pub fn resolve(&self, name: &str) -> &Entry {
        self.get(name).unwrap_or_else(|| {
            self.entries
                .first()
                .expect("the catalog always has the built-in themes")
        })
    }
}

/// The theme used by the flash overlay window.
///
/// The transparent background is the whole point: iced paints a window with
/// its theme's background color before drawing anything, so anything opaque
/// here would black out the screen instead of tinting it. `Palette` keeps the
/// background color verbatim, alpha included, which is what lets this work.
pub static FLASH: LazyLock<Theme> = LazyLock::new(|| {
    Theme::custom(
        "TimeSlay Flash",
        Palette {
            background: Color::TRANSPARENT,
            text: Color::WHITE,
            primary: Color::from_rgb8(0xFF, 0x20, 0x20),
            success: Color::from_rgb8(0x6F, 0xB0, 0x7A),
            warning: Color::from_rgb8(0xE0, 0xA5, 0x4B),
            danger: Color::from_rgb8(0xFF, 0x20, 0x20),
        },
    )
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flash_theme_paints_nothing_of_its_own() {
        use iced::theme::Base;

        assert_eq!(FLASH.base().background_color.a, 0.0);
    }

    #[test]
    fn the_catalog_only_offers_the_four_supported_themes() {
        let expected = ["Light", "Light high contrast", "Dark", "Dark high contrast"];
        assert_eq!(Catalog::built_in().names(), expected);
        assert_eq!(Catalog::load().names(), expected);
    }

    #[test]
    fn a_removed_or_unknown_theme_falls_back_to_light() {
        let catalog = Catalog::built_in();
        for name in ["Sakura", "GitHub Dark", "Deleted By User"] {
            assert!(catalog.get(name).is_none());
            assert_eq!(catalog.resolve(name).name, "Light");
        }
    }
}
