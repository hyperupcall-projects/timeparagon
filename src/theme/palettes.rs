//! The palettes we ship ourselves.
//!
//! These are all light themes built around a tinted near-white background, a
//! saturated mid-tone primary for anything active, and text dark enough to
//! stay readable against the tint. That last part is what keeps a pastel
//! theme from turning into a low-contrast mess.

use iced::Color;
use iced::theme::Palette;

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
}
