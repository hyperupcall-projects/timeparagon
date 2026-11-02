//! High contrast palettes with explicit surface and border colors.

use iced::Color;
use iced::theme::{
    Palette,
    palette::{Background, Extended, Pair},
};

const fn rgb(hex: u32) -> Color {
    Color::from_rgb8(
        ((hex >> 16) & 0xFF) as u8,
        ((hex >> 8) & 0xFF) as u8,
        (hex & 0xFF) as u8,
    )
}

pub const LIGHT_HIGH_CONTRAST: Palette = Palette {
    background: Color::WHITE,
    text: Color::BLACK,
    primary: rgb(0x0349B4),
    success: rgb(0x116329),
    warning: rgb(0x744500),
    danger: rgb(0xA40E26),
};

pub const DARK_HIGH_CONTRAST: Palette = Palette {
    background: rgb(0x0A0C10),
    text: rgb(0xF0F3F6),
    primary: rgb(0x71B7FF),
    success: rgb(0x26CD4D),
    warning: rgb(0xF0B72F),
    danger: rgb(0xFF6A69),
};

/// Seven steps from subtle fills to clearly contrasting borders.
pub const LIGHT_HIGH_CONTRAST_SURFACES: [u32; 7] = [
    0xFAFAFA, 0xF2F2F2, 0xE5E5E5, 0xD5D5D5, 0x646464, 0x464646, 0x242424,
];
pub const DARK_HIGH_CONTRAST_SURFACES: [u32; 7] = [
    0x010409, 0x171B21, 0x272B33, 0x343941, 0x7A828E, 0x9EA7B3, 0xBDC4CC,
];

pub fn high_contrast_extended(palette: Palette, surfaces: [u32; 7]) -> Extended {
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
    // The readout needs the same high contrast accent on either canvas.
    extended.primary.strong = extended.primary.base;
    // Secondary colors also carry labels and placeholders.
    extended.secondary.base = Pair::new(
        iced::theme::palette::mix(
            palette.background,
            palette.text,
            if extended.is_dark { 0.70 } else { 0.85 },
        ),
        palette.background,
    );
    // Hovered buttons keep the base label color, so their fill must stay on
    // the same side of the contrast range as the base secondary fill.
    extended.secondary.strong = Pair::new(
        iced::theme::palette::mix(
            palette.background,
            palette.text,
            if extended.is_dark { 0.85 } else { 0.95 },
        ),
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
    #[test]
    fn high_contrast_themes_have_readable_surfaces_and_accents() {
        let catalog = super::super::Catalog::built_in();
        for (name, is_dark) in [("Light high contrast", false), ("Dark high contrast", true)] {
            let entry = catalog.get(name).unwrap();
            let palette = entry.theme.palette();
            let extended = entry.theme.extended_palette();
            assert_eq!(extended.is_dark, is_dark, "{name}");
            assert!(
                palette.text.relative_contrast(palette.background) >= 7.0,
                "{name}: body text"
            );
            assert!(
                entry.accents.digits.relative_contrast(palette.background) >= 4.5,
                "{name}: timer digits"
            );
            assert!(
                extended
                    .secondary
                    .base
                    .color
                    .relative_contrast(palette.background)
                    >= 4.5,
                "{name}: muted text"
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
                extended.secondary.base,
                extended.secondary.weak,
                extended.secondary.strong,
                extended.success.base,
                extended.success.weak,
                extended.success.strong,
                extended.warning.base,
                extended.warning.weak,
                extended.warning.strong,
                extended.danger.base,
                extended.danger.weak,
                extended.danger.strong,
            ] {
                assert!(
                    pair.color.relative_contrast(pair.text) >= 4.5,
                    "{name}: unreadable surface {pair:?}"
                );
            }
        }
    }

    #[test]
    fn high_contrast_borders_stand_out_against_controls() {
        let catalog = super::super::Catalog::built_in();
        for name in ["Light high contrast", "Dark high contrast"] {
            let extended = catalog.get(name).unwrap().theme.extended_palette();
            for surface in [
                extended.background.base,
                extended.background.weaker,
                extended.background.weak,
            ] {
                assert!(
                    extended
                        .background
                        .strong
                        .color
                        .relative_contrast(surface.color)
                        >= 3.0,
                    "{name}: border against {surface:?}"
                );
            }
        }
    }
}
