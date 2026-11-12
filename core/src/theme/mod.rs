//! The look both apps share: the palettes, and the radii, shadows and fixed
//! colors drawn on top of them.
//!
//! The iced app turns these into widget styles and the Dioxus app turns them
//! into CSS variables, so a change here reaches both.

pub mod palettes;

use iced_core::Color;
use iced_core::theme::Palette;
use iced_core::theme::palette::Extended;

/// One of the four selectable color schemes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
	Light,
	LightHighContrast,
	Dark,
	DarkHighContrast,
}

impl Scheme {
	/// In the order the theme pickers offer them.
	pub const ALL: [Self; 4] = [
		Self::Light,
		Self::LightHighContrast,
		Self::Dark,
		Self::DarkHighContrast,
	];

	pub fn name(self) -> &'static str {
		match self {
			Self::Light => "Light",
			Self::LightHighContrast => "Light high contrast",
			Self::Dark => "Dark",
			Self::DarkHighContrast => "Dark high contrast",
		}
	}

	pub fn palette(self) -> Palette {
		match self {
			Self::Light => palettes::LIGHT,
			Self::LightHighContrast => palettes::LIGHT_HIGH_CONTRAST,
			Self::Dark => palettes::DARK,
			Self::DarkHighContrast => palettes::DARK_HIGH_CONTRAST,
		}
	}

	/// Every surface, border and accent of the scheme.
	pub fn extended(self) -> Extended {
		let palette = self.palette();
		match self {
			Self::Light => palettes::calm_extended(
				palette,
				palettes::LIGHT_SURFACES,
				Color::from_rgb8(0x76, 0x71, 0x6A),
			),
			Self::Dark => palettes::calm_extended(
				palette,
				palettes::DARK_SURFACES,
				Color::from_rgb8(0xA3, 0x9E, 0x95),
			),
			Self::LightHighContrast => {
				palettes::high_contrast_extended(palette, palettes::LIGHT_HIGH_CONTRAST_SURFACES)
			}
			Self::DarkHighContrast => {
				palettes::high_contrast_extended(palette, palettes::DARK_HIGH_CONTRAST_SURFACES)
			}
		}
	}
}

/// Corner radii shared by every surface.
pub const RADIUS_CARD: f32 = 10.0;
pub const RADIUS_ROW: f32 = 8.0;
pub const RADIUS_CONTROL: f32 = 7.0;
pub const RADIUS_SMALL: f32 = 5.0;

/// A soft black shadow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Elevation {
	pub alpha: f32,
	pub y: f32,
	pub blur: f32,
}

impl Elevation {
	/// The shadow's color, deepened on dark themes where it would otherwise vanish.
	pub fn color(self, is_dark: bool) -> Color {
		let depth = if is_dark { 3.0 } else { 1.0 };
		Color::BLACK.scale_alpha((self.alpha * depth).min(0.6))
	}
}

const fn elevation(alpha: f32, y: f32, blur: f32) -> Elevation {
	Elevation { alpha, y, blur }
}

/// A section or list card.
pub const CARD: Elevation = elevation(0.16, 3.0, 12.0);
/// One item inside a card or list.
pub const ROW: Elevation = elevation(0.14, 2.0, 8.0);
/// Floating dialogs: the same card, lifted well off the page.
pub const FLOATING: Elevation = elevation(0.26, 10.0, 30.0);
/// A dropdown's list.
pub const MENU: Elevation = elevation(0.2, 6.0, 18.0);
/// A button at rest, under the pointer, and held down.
pub const BUTTON: Elevation = elevation(0.14, 1.0, 4.0);
pub const BUTTON_HOVERED: Elevation = elevation(0.2, 3.0, 9.0);
pub const BUTTON_PRESSED: Elevation = elevation(0.08, 0.0, 2.0);
/// The navigation entry whose page is on show.
pub const NAV_ACTIVE: Elevation = elevation(0.08, 1.0, 3.0);

/// How far a filled button darkens toward black for its border, when
/// hovered, and when pressed.
pub const FILLED_EDGE_SHADE: f32 = 0.14;
pub const FILLED_HOVER_SHADE: f32 = 0.08;
pub const FILLED_PRESS_SHADE: f32 = 0.16;

/// A color pulled toward black, the way a filled button darkens.
pub fn shade(color: Color, by: f32) -> Color {
	iced_core::theme::palette::mix(color, Color::BLACK, by)
}

/// How much of the accent tints a surface that marks today.
pub const TODAY_TINT: f32 = 0.15;
pub const TODAY_HOVER_TINT: f32 = 0.22;
pub const TODAY_PRESS_TINT: f32 = 0.30;

/// A surface pulled toward `accent`, for a very light accent fill.
pub fn tint(surface: Color, accent: Color, by: f32) -> Color {
	iced_core::theme::palette::mix(surface, accent, by)
}

/// The dim sheet behind a dialog.
pub const SCRIM: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.3);

/// The geometry of an event on a day grid. Colors of the events themselves
/// are in [`crate::calendar::colors`].
pub const EVENT_RADIUS: f32 = 6.0;
/// The space between events side by side.
pub const EVENT_GAP: f32 = 2.0;
/// The space inside a block on the left of its text.
pub const BLOCK_PADDING: f32 = 6.0;
/// The space inside a block on the right of its text.
pub const BLOCK_PADDING_RIGHT: f32 = 4.0;
/// The space inside a block above its first line.
pub const BLOCK_TOP: f32 = 3.0;
/// A time block takes this share of its column, after this much is taken off
/// it, leaving the rest for its task rows.
pub const TIME_BLOCK_SHARE: f32 = 0.5;
pub const TIME_BLOCK_INSET: f32 = 6.0;
/// The share of a time block taken by its label, beside its actions.
pub const TIME_BLOCK_LABEL_SHARE: f32 = 0.6;
/// How far apart a time block's task rows sit.
pub const TIME_BLOCK_ROW: f32 = 20.0;
/// How far a time block's task rows sit from it.
pub const TIME_BLOCK_ROW_GAP: f32 = 4.0;
pub const NEEDLE_RADIUS: f32 = 5.5;
/// The shortest a travel box is drawn, so its words fit in it.
pub const TRAVEL_MIN_HEIGHT: f32 = 16.0;

/// The hour lines across a day grid.
pub const HOUR_LINE: Color = Color::from_rgba(
	0x8A as f32 / 255.0,
	0x85 as f32 / 255.0,
	0x7C as f32 / 255.0,
	0.28,
);
/// The edge of an event, and of a travel box.
pub const EVENT_EDGE: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.28);
pub const TRAVEL_EDGE: Color = Color::from_rgba(0.0, 0.0, 0.0, 0.45);
/// The border around anything whose change is still on its way to the cloud
/// (Google Calendar, Tasks, Contacts, …). A mid grey, so it reads as neither
/// confirmed nor failed, on either canvas.
pub const PENDING_BORDER: Color = Color::from_rgb8(0x80, 0x80, 0x80);
/// An event chip's shadow.
pub const EVENT_CHIP: Elevation = elevation(0.12, 1.0, 3.0);
/// Two faint offset copies stand in for an event's shadow on the grid.
pub const EVENT_DROPS: [(f32, f32); 2] = [(1.0, 0.10), (2.5, 0.05)];
/// The part of a time block its tasks have already taken up.
pub const BLOCK_FILLED: Color = Color::from_rgba(0.15, 0.15, 0.15, 0.28);
/// A time block's action buttons, over the block's own color.
pub const BLOCK_ACTION: Color = Color::from_rgba(1.0, 1.0, 1.0, 0.27);
/// The label of a task in a time block, and of one that runs past its end.
pub const BLOCK_ROW_TEXT: Color = Color::from_rgb(125.0 / 255.0, 125.0 / 255.0, 125.0 / 255.0);
pub const BLOCK_ROW_OVER_TEXT: Color = Color::from_rgb(220.0 / 255.0, 65.0 / 255.0, 55.0 / 255.0);
pub const BLOCK_ROW_OVER: Color = Color::from_rgba(0.9, 0.2, 0.15, 0.22);

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_the_dark_schemes_are_dark() {
		for scheme in Scheme::ALL {
			let dark = matches!(scheme, Scheme::Dark | Scheme::DarkHighContrast);
			assert_eq!(scheme.extended().is_dark, dark, "{}", scheme.name());
		}
	}

	#[test]
	fn a_shadow_deepens_on_dark_themes_up_to_a_limit() {
		assert_eq!(CARD.color(false).a, 0.16);
		assert!((CARD.color(true).a - 0.48).abs() < 1e-6);
		assert_eq!(elevation(0.5, 0.0, 0.0).color(true).a, 0.6);
	}
}
