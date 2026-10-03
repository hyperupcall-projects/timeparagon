//! The theme catalog: built-ins, our own palettes, and user files.

pub mod loader;
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
    fn built_in(theme: Theme) -> Self {
        Self {
            name: theme.to_string(),
            accents: Accents::derive(&theme),
            theme,
        }
    }

    fn from_palette(name: &str, palette: Palette) -> Self {
        let theme = Theme::custom(name.to_owned(), palette);

        Self {
            name: name.to_owned(),
            accents: Accents::derive(&theme),
            theme,
        }
    }
}

/// The light themes iced ships that are actually nice to look at, in the
/// order we want to offer them.
const BUILT_IN_LIGHT: [Theme; 6] = [
    Theme::CatppuccinLatte,
    Theme::KanagawaLotus,
    Theme::SolarizedLight,
    Theme::GruvboxLight,
    Theme::TokyoNightLight,
    Theme::Light,
];

/// A few dark themes, for whoever wants them.
const BUILT_IN_DARK: [Theme; 4] = [
    Theme::Dark,
    Theme::CatppuccinMocha,
    Theme::TokyoNight,
    Theme::Ferra,
];

/// Every theme the picker offers.
#[derive(Debug, Clone)]
pub struct Catalog {
    entries: Vec<Entry>,
}

impl Catalog {
    /// Builds the catalog, including whatever the user has dropped in their
    /// themes directory.
    pub fn load() -> Self {
        let user = crate::config::themes_dir()
            .map(|directory| {
                loader::ensure_example(&directory);
                loader::load(&directory)
            })
            .unwrap_or_default();

        Self::assemble(user)
    }

    /// Just the themes compiled into the binary.
    ///
    /// Used by tests, so they neither read nor write the user's themes
    /// directory.
    pub fn built_in() -> Self {
        Self::assemble(Vec::new())
    }

    fn assemble(user: Vec<Entry>) -> Self {
        let mut entries: Vec<Entry> = palettes::CUSTOM
            .iter()
            .map(|(name, palette)| Entry::from_palette(name, *palette))
            .collect();

        entries.extend(user);
        entries.extend(BUILT_IN_LIGHT.iter().cloned().map(Entry::built_in));
        entries.extend(BUILT_IN_DARK.iter().cloned().map(Entry::built_in));

        // A user theme sharing a name with an earlier entry would make the
        // picker ambiguous, so the first one wins.
        let mut seen = Vec::new();
        entries.retain(|entry| {
            let fresh = !seen.contains(&entry.name);
            if fresh {
                seen.push(entry.name.clone());
            }
            fresh
        });

        Self { entries }
    }

    pub fn names(&self) -> Vec<String> {
        self.entries.iter().map(|entry| entry.name.clone()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    /// Looks up a theme, falling back to the first one if the configured name
    /// no longer exists (a renamed or deleted user theme).
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
        "TimeTracker Flash",
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
    fn our_palettes_come_first_in_the_catalog() {
        let catalog = Catalog {
            entries: palettes::CUSTOM
                .iter()
                .map(|(name, palette)| Entry::from_palette(name, *palette))
                .collect(),
        };

        assert_eq!(catalog.names().first().unwrap(), "Sakura");
    }

    #[test]
    fn an_unknown_theme_name_falls_back_instead_of_panicking() {
        let catalog = Catalog {
            entries: vec![Entry::from_palette("Sakura", palettes::SAKURA)],
        };

        assert_eq!(catalog.resolve("Deleted By User").name, "Sakura");
    }

    #[test]
    fn built_in_entries_are_named_the_way_iced_names_them() {
        let entry = Entry::built_in(Theme::CatppuccinLatte);
        assert_eq!(entry.name, "Catppuccin Latte");
    }
}
