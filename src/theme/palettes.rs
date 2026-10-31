//! The palettes we ship ourselves.
//!
//! Light palettes use tinted near-white backgrounds; dark palettes use
//! deliberately chosen surface steps so controls keep their depth and borders.

use iced::Color;
use iced::theme::{
    Palette,
    palette::{Background, Extended, Pair},
};

/// Helper to keep the tables below readable.
const fn rgb(hex: u32) -> Color {
    Color::from_rgb8(
        ((hex >> 16) & 0xFF) as u8,
        ((hex >> 8) & 0xFF) as u8,
        (hex & 0xFF) as u8,
    )
}

/// Soft pink, deep rose accents.
pub const SAKURA: Palette = Palette {
    background: rgb(0xFFF3F7),
    text: rgb(0x4A2C39),
    primary: rgb(0xD6558A),
    success: rgb(0x6FB07A),
    warning: rgb(0xE0A54B),
    danger: rgb(0xD2504F),
};

/// A warmer, more saturated pink.
pub const PEONY: Palette = Palette {
    background: rgb(0xFDF0F6),
    text: rgb(0x3F2233),
    primary: rgb(0xC2185B),
    success: rgb(0x5FA36A),
    warning: rgb(0xD9933D),
    danger: rgb(0xC62D42),
};

/// Pale green with a forest primary.
pub const MATCHA: Palette = Palette {
    background: rgb(0xF2F7EE),
    text: rgb(0x2C3A2B),
    primary: rgb(0x4B7F52),
    success: rgb(0x5E9B63),
    warning: rgb(0xC9922F),
    danger: rgb(0xC0504A),
};

/// Cool mint, leaning teal.
pub const MINT: Palette = Palette {
    background: rgb(0xEFF9F6),
    text: rgb(0x21393A),
    primary: rgb(0x1F8A76),
    success: rgb(0x3FA08A),
    warning: rgb(0xC99A2E),
    danger: rgb(0xC2504B),
};

/// Muted, desaturated green for when the others are too much.
pub const SAGE: Palette = Palette {
    background: rgb(0xF4F6F1),
    text: rgb(0x333A31),
    primary: rgb(0x6E8B6A),
    success: rgb(0x6E9A6A),
    warning: rgb(0xBE9750),
    danger: rgb(0xB4655F),
};

/// Our palettes, in the order they appear in the picker.
pub const CUSTOM: [(&str, Palette); 5] = [
    ("Sakura", SAKURA),
    ("Peony", PEONY),
    ("Matcha", MATCHA),
    ("Mint", MINT),
    ("Sage", SAGE),
];

/// A true black canvas with cool silver text and an ice-blue accent.
pub const ABSOLUTE_BLACK: Palette = Palette {
    background: rgb(0x000000),
    text: rgb(0xF5F5F7),
    primary: rgb(0x8AB4F8),
    success: rgb(0x81C995),
    warning: rgb(0xFDD663),
    danger: rgb(0xF28B82),
};

/// GitHub's charcoal canvas and familiar blue, green, gold, and coral accents.
pub const GITHUB_DARK: Palette = Palette {
    background: rgb(0x0D1117),
    text: rgb(0xE6EDF3),
    primary: rgb(0x58A6FF),
    success: rgb(0x3FB950),
    warning: rgb(0xD29922),
    danger: rgb(0xF85149),
};

/// GitHub's high contrast treatment, with brighter text and explicit borders.
pub const GITHUB_DARK_HIGH_CONTRAST: Palette = Palette {
    background: rgb(0x0A0C10),
    text: rgb(0xF0F3F6),
    primary: rgb(0x71B7FF),
    success: rgb(0x26CD4D),
    warning: rgb(0xF0B72F),
    danger: rgb(0xFF6A69),
};

/// Seven surface steps from the faintest fill to the strongest separator.
/// The high contrast theme deliberately makes borders much brighter than fills.
pub const DARK: [(&str, Palette, [u32; 7]); 5] = [
    (
        "Absolute Black",
        ABSOLUTE_BLACK,
        [
            0x080808, 0x101010, 0x181818, 0x242424, 0x383838, 0x484848, 0x5C5C5C,
        ],
    ),
    (
        "GitHub Dark",
        GITHUB_DARK,
        [
            0x010409, 0x161B22, 0x21262D, 0x292E36, 0x30363D, 0x484F58, 0x6E7681,
        ],
    ),
    (
        "GitHub Dark High Contrast",
        GITHUB_DARK_HIGH_CONTRAST,
        [
            0x010409, 0x171B21, 0x272B33, 0x343941, 0x7A828E, 0x9EA7B3, 0xBDC4CC,
        ],
    ),
    (
        "Nord",
        Palette::NORD,
        [
            0x303642, 0x3B4252, 0x434C5E, 0x4C566A, 0x5C687F, 0x75829B, 0x8F9BB0,
        ],
    ),
    (
        "Dracula",
        Palette::DRACULA,
        [
            0x21222C, 0x282A36, 0x343746, 0x44475A, 0x6272A4, 0x7884AA, 0x929BB8,
        ],
    ),
];

pub fn dark_extended(palette: Palette, surfaces: [u32; 7]) -> Extended {
    let mut extended = Extended::generate(palette);
    let [weakest, weaker, weak, neutral, strong, stronger, strongest] =
        surfaces.map(|hex| readable_pair(Pair::new(rgb(hex), palette.text)));
    extended.background = Background {
        base: Pair::new(palette.background, palette.text),
        weakest,
        weaker,
        weak,
        neutral,
        strong,
        stronger,
        strongest,
    };
    // The timer readout uses strong primary as text, so preserve the bright
    // accent instead of the generator's darker variant.
    extended.primary.strong = extended.primary.base;
    // Iced also uses secondary.base.color for labels and placeholders.
    // Derive it from the foreground so it stays legible on tinted dark canvases.
    extended.secondary.base = Pair::new(
        iced::theme::palette::mix(palette.background, palette.text, 0.55),
        palette.background,
    );
    // Preserve readable labels on selected, hover, and semantic button fills.
    for pair in [
        &mut extended.primary.base,
        &mut extended.primary.weak,
        &mut extended.primary.strong,
        &mut extended.secondary.base,
        &mut extended.secondary.weak,
        &mut extended.secondary.strong,
        &mut extended.success.base,
        &mut extended.success.weak,
        &mut extended.success.strong,
        &mut extended.warning.base,
        &mut extended.warning.weak,
        &mut extended.warning.strong,
        &mut extended.danger.base,
        &mut extended.danger.weak,
        &mut extended.danger.strong,
    ] {
        *pair = readable_pair(*pair);
    }
    extended
}

fn readable_pair(mut pair: Pair) -> Pair {
    // Iced's softened fallback can fall below 4.5:1 on mid-tone fills.
    if pair.color.relative_contrast(pair.text) < 4.5 {
        pair.text = if pair.color.relative_contrast(Color::WHITE)
            >= pair.color.relative_contrast(Color::BLACK)
        {
            Color::WHITE
        } else {
            Color::BLACK
        };
    }
    pair
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative luminance, per WCAG.
    fn luminance(color: Color) -> f32 {
        let channel = |c: f32| {
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };

        0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (high, low) = {
            let (x, y) = (luminance(a), luminance(b));
            if x > y { (x, y) } else { (y, x) }
        };

        (high + 0.05) / (low + 0.05)
    }

    #[test]
    fn every_palette_has_readable_body_text() {
        for (name, palette) in CUSTOM {
            let ratio = contrast(palette.text, palette.background);

            assert!(
                ratio >= 4.5,
                "{name}: text on background is only {ratio:.2}:1, needs 4.5:1"
            );
        }
    }

    #[test]
    fn every_palette_has_a_visible_primary() {
        // The primary carries the big readout, so it needs to be clearly
        // distinguishable from the background even though it is an accent.
        for (name, palette) in CUSTOM {
            let ratio = contrast(palette.primary, palette.background);

            assert!(
                ratio >= 3.0,
                "{name}: primary on background is only {ratio:.2}:1, needs 3:1"
            );
        }
    }

    #[test]
    fn every_palette_is_light() {
        for (name, palette) in CUSTOM {
            assert!(
                luminance(palette.background) > 0.5,
                "{name} is supposed to be a light theme"
            );
        }
    }

    #[test]
    fn dark_themes_have_readable_surfaces_and_accents() {
        let catalog = super::super::Catalog::built_in();
        for name in [
            "Absolute Black",
            "GitHub Dark",
            "GitHub Dark High Contrast",
            "Nord",
            "Dracula",
        ] {
            let entry = catalog.get(name).expect("dark theme is in the picker");
            let palette = entry.theme.palette();
            let extended = entry.theme.extended_palette();
            assert!(extended.is_dark, "{name} must be a dark theme");
            assert!(
                contrast(palette.text, palette.background) >= 7.0,
                "{name}: body text"
            );
            assert!(
                contrast(entry.accents.digits, palette.background) >= 4.5,
                "{name}: timer digits"
            );
            for pair in [
                extended.background.base,
                extended.background.weakest,
                extended.background.weaker,
                extended.background.weak,
                extended.background.neutral,
                extended.background.strong,
                extended.background.stronger,
                extended.background.strongest,
                extended.primary.base,
                extended.primary.weak,
                extended.primary.strong,
            ] {
                assert!(
                    contrast(pair.color, pair.text) >= 4.5,
                    "{name}: unreadable surface {pair:?}"
                );
            }
        }
    }

    #[test]
    fn absolute_black_has_a_pure_black_canvas() {
        let catalog = super::super::Catalog::built_in();
        let theme = &catalog.get("Absolute Black").unwrap().theme;
        assert_eq!(theme.palette().background, Color::BLACK);
        assert_eq!(theme.extended_palette().background.base.color, Color::BLACK);
    }

    #[test]
    fn high_contrast_borders_stand_out_against_controls() {
        let extended = dark_extended(GITHUB_DARK_HIGH_CONTRAST, DARK[2].2);
        for surface in [
            extended.background.base,
            extended.background.weaker,
            extended.background.weak,
        ] {
            assert!(contrast(extended.background.strong.color, surface.color) >= 3.0);
        }
    }
}
