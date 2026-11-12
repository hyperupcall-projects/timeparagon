//! The theme palettes, with explicit surface and border colors.

use iced_core::Color;
use iced_core::theme::{
	Palette,
	palette::{self, Background, Extended, Pair},
};

const fn rgb(hex: u32) -> Color {
	Color::from_rgb8(
		((hex >> 16) & 0xFF) as u8,
		((hex >> 8) & 0xFF) as u8,
		(hex & 0xFF) as u8,
	)
}

/// Warm paper, white cards and a muted sage accent.
pub const LIGHT: Palette = Palette {
	background: rgb(0xF7F5F0),
	text: rgb(0x2E2C29),
	primary: rgb(0x6F9373),
	success: rgb(0x6F9373),
	warning: rgb(0xC89A6B),
	danger: rgb(0xB85C50),
};

/// The same warm neutrals and sage accent, for dark surroundings.
pub const DARK: Palette = Palette {
	background: rgb(0x1F1E1C),
	text: rgb(0xECE8E1),
	primary: rgb(0x8FB893),
	success: rgb(0x8FB893),
	warning: rgb(0xD4A878),
	danger: rgb(0xE08A7E),
};

/// Card, hover, pressed, hairline, border, strong border, muted.
pub const LIGHT_SURFACES: [u32; 7] = [
	0xFFFFFF, 0xF2EFE9, 0xECE8E0, 0xE2DED6, 0xCFC9BE, 0xA9A398, 0x8A857C,
];
pub const DARK_SURFACES: [u32; 7] = [
	0x2A2826, 0x33312E, 0x3B3936, 0x45423E, 0x55524D, 0x6E6A64, 0x9A958C,
];

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
		palette::mix(
			palette.background,
			palette.text,
			if extended.is_dark { 0.70 } else { 0.85 },
		),
		palette.background,
	);
	// Hovered buttons keep the base label color, so their fill must stay on
	// the same side of the contrast range as the base secondary fill.
	extended.secondary.strong = Pair::new(
		palette::mix(
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

/// Explicit surfaces plus soft accent tints for the two everyday themes.
pub fn calm_extended(palette: Palette, surfaces: [u32; 7], muted: Color) -> Extended {
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
	// Secondary colors carry labels and placeholders.
	extended.secondary.base = Pair::new(muted, palette.background);

	let tint = |accent: Color| {
		let (soft, ink) = if extended.is_dark {
			(0.22, 0.25)
		} else {
			(0.13, 0.28)
		};
		let shade = if extended.is_dark {
			Color::WHITE
		} else {
			Color::BLACK
		};
		(
			palette::mix(rgb(surfaces[0]), accent, soft),
			palette::mix(accent, shade, ink),
		)
	};
	let on_accent = if extended.is_dark {
		palette.background
	} else {
		Color::WHITE
	};
	// Built by hand because `Pair::new` would swap the label for a darker one.
	let accent = |color: Color| Pair {
		color,
		text: on_accent,
	};
	let (soft, ink) = tint(palette.primary);
	extended.primary.base = accent(palette.primary);
	extended.primary.weak = Pair {
		color: soft,
		text: ink,
	};
	extended.primary.strong = accent(ink);
	let (soft, ink) = tint(palette.danger);
	extended.danger.base = accent(palette.danger);
	extended.danger.weak = Pair {
		color: soft,
		text: ink,
	};
	extended.danger.strong = accent(ink);
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
