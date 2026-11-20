//! The More menu: its destinations, how they are laid out, and their pictures.
//!
//! The desktop and the phone both draw their tiles from this, so the two menus
//! cannot drift apart.

use iced_core::Color;

/// A destination in the More menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
	Notepad,
	Habits,
	Notes,
	People,
	Travel,
	BrainDump,
	Routines,
	Alarms,
	Logs,
}

/// The desktop's tiles, top to bottom. A row of one fills the menu's width.
/// Its logs are at the bottom left of the panel instead.
pub const ROWS: [&[Item]; 5] = [
	&[Item::Notepad],
	&[Item::Habits, Item::Notes],
	&[Item::People],
	&[Item::Travel, Item::BrainDump],
	&[Item::Routines, Item::Alarms],
];

/// The phone's tiles: the desktop's without routines, which the phone does
/// not have, then the logs.
pub const PHONE_ROWS: [&[Item]; 6] = [
	ROWS[0],
	ROWS[1],
	ROWS[2],
	ROWS[3],
	&[Item::Alarms],
	&[Item::Logs],
];

/// One stroke of a picture, on a 24-unit square with y pointing down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
	/// Straight lines through each point in turn.
	Lines(&'static [(f32, f32)]),
	/// Straight lines through each point, back to the first.
	Polygon(&'static [(f32, f32)]),
	Circle {
		x: f32,
		y: f32,
		radius: f32,
	},
	/// Clockwise from `from` to `to`, in degrees from the positive x-axis.
	Arc {
		x: f32,
		y: f32,
		radius: f32,
		from: f32,
		to: f32,
	},
	RoundedRect {
		x: f32,
		y: f32,
		width: f32,
		height: f32,
		radius: f32,
	},
}

impl Item {
	pub fn label(self) -> &'static str {
		match self {
			Self::Notepad => "Notepad",
			Self::Habits => "Habits",
			Self::Notes => "Notes",
			Self::People => "People",
			Self::Travel => "Travel",
			Self::BrainDump => "Brain dump",
			Self::Routines => "Routines",
			Self::Alarms => "Alarms",
			Self::Logs => "Logs",
		}
	}

	/// A name for the item in stylesheets.
	pub fn slug(self) -> &'static str {
		match self {
			Self::Notepad => "notepad",
			Self::Habits => "habits",
			Self::Notes => "notes",
			Self::People => "people",
			Self::Travel => "travel",
			Self::BrainDump => "brain-dump",
			Self::Routines => "routines",
			Self::Alarms => "alarms",
			Self::Logs => "logs",
		}
	}

	/// The tile's color, deep on a light theme and bright on a dark one.
	pub fn color(self, dark: bool) -> Color {
		let (light, night) = match self {
			Self::Notepad => (0xB4_6A_0B, 0xF2_C1_4E),
			Self::Habits => (0x2F_85_5A, 0x68_D3_91),
			Self::Notes => (0x2B_6C_B0, 0x63_B3_ED),
			Self::People => (0xB8_32_80, 0xF6_87_B3),
			Self::Travel => (0x0F_76_6E, 0x4F_D1_C5),
			Self::BrainDump => (0x6B_46_C1, 0xB7_94_F4),
			Self::Routines => (0x43_38_CA, 0x81_8C_F8),
			Self::Alarms => (0xC5_30_30, 0xFC_81_81),
			Self::Logs => (0x4A_55_68, 0xA0_AE_C0),
		};
		let rgb: u32 = if dark { night } else { light };
		Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
	}

	/// The strokes of the item's picture.
	pub fn picture(self) -> &'static [Shape] {
		match self {
			Self::Notepad => NOTEPAD,
			Self::Habits => HABITS,
			Self::Notes => NOTES,
			Self::People => PEOPLE,
			Self::Travel => TRAVEL,
			Self::BrainDump => BRAIN_DUMP,
			Self::Routines => ROUTINES,
			Self::Alarms => ALARMS,
			Self::Logs => LOGS,
		}
	}
}

/// A pad bound at the top, with two lines written on it.
const NOTEPAD: &[Shape] = &[
	Shape::RoundedRect {
		x: 5.0,
		y: 4.0,
		width: 14.0,
		height: 17.0,
		radius: 2.0,
	},
	Shape::Lines(&[(9.0, 2.0), (9.0, 6.0)]),
	Shape::Lines(&[(15.0, 2.0), (15.0, 6.0)]),
	Shape::Lines(&[(9.0, 11.0), (15.0, 11.0)]),
	Shape::Lines(&[(9.0, 15.5), (13.0, 15.5)]),
];

/// A ticked circle.
const HABITS: &[Shape] = &[
	Shape::Circle {
		x: 12.0,
		y: 12.0,
		radius: 9.0,
	},
	Shape::Lines(&[(8.0, 12.5), (11.0, 15.5), (16.5, 9.0)]),
];

/// A page with a folded corner.
const NOTES: &[Shape] = &[
	Shape::Polygon(&[
		(6.0, 3.0),
		(14.0, 3.0),
		(19.0, 8.0),
		(19.0, 21.0),
		(6.0, 21.0),
	]),
	Shape::Lines(&[(14.0, 3.0), (14.0, 8.0), (19.0, 8.0)]),
	Shape::Lines(&[(9.5, 13.0), (15.5, 13.0)]),
	Shape::Lines(&[(9.5, 17.0), (13.5, 17.0)]),
];

/// One person in front of another.
const PEOPLE: &[Shape] = &[
	Shape::Circle {
		x: 9.0,
		y: 8.0,
		radius: 3.5,
	},
	Shape::Arc {
		x: 9.0,
		y: 21.0,
		radius: 7.0,
		from: 180.0,
		to: 360.0,
	},
	Shape::Arc {
		x: 16.0,
		y: 8.0,
		radius: 3.5,
		from: 280.0,
		to: 440.0,
	},
	Shape::Arc {
		x: 17.0,
		y: 21.0,
		radius: 5.0,
		from: 300.0,
		to: 360.0,
	},
];

/// A map pin.
const TRAVEL: &[Shape] = &[
	Shape::Arc {
		x: 12.0,
		y: 10.0,
		radius: 7.0,
		from: 150.0,
		to: 390.0,
	},
	Shape::Lines(&[(5.94, 13.5), (12.0, 22.0), (18.06, 13.5)]),
	Shape::Circle {
		x: 12.0,
		y: 10.0,
		radius: 2.5,
	},
];

/// A thought cloud with its trail of bubbles.
const BRAIN_DUMP: &[Shape] = &[
	Shape::Arc {
		x: 7.0,
		y: 11.5,
		radius: 3.5,
		from: 90.0,
		to: 270.0,
	},
	Shape::Arc {
		x: 12.0,
		y: 8.0,
		radius: 5.0,
		from: 180.0,
		to: 360.0,
	},
	Shape::Arc {
		x: 17.0,
		y: 11.5,
		radius: 3.5,
		from: 270.0,
		to: 450.0,
	},
	Shape::Lines(&[(7.0, 15.0), (17.0, 15.0)]),
	Shape::Circle {
		x: 9.5,
		y: 18.5,
		radius: 1.4,
	},
	Shape::Circle {
		x: 6.0,
		y: 21.5,
		radius: 0.8,
	},
];

/// An arrow coming back around to where it started.
const ROUTINES: &[Shape] = &[
	Shape::Arc {
		x: 12.0,
		y: 12.0,
		radius: 8.0,
		from: 60.0,
		to: 330.0,
	},
	Shape::Lines(&[(15.47, 6.0), (18.93, 8.0), (18.93, 4.0)]),
];

/// An alarm clock with two bells and two feet.
const ALARMS: &[Shape] = &[
	Shape::Circle {
		x: 12.0,
		y: 13.0,
		radius: 8.0,
	},
	Shape::Lines(&[(12.0, 9.0), (12.0, 13.0), (15.0, 15.0)]),
	Shape::Lines(&[(3.0, 6.0), (6.0, 3.0)]),
	Shape::Lines(&[(21.0, 6.0), (18.0, 3.0)]),
	Shape::Lines(&[(6.5, 19.5), (5.0, 21.5)]),
	Shape::Lines(&[(17.5, 19.5), (19.0, 21.5)]),
];

/// A bulleted list.
const LOGS: &[Shape] = &[
	Shape::Circle {
		x: 5.0,
		y: 6.0,
		radius: 1.0,
	},
	Shape::Circle {
		x: 5.0,
		y: 12.0,
		radius: 1.0,
	},
	Shape::Circle {
		x: 5.0,
		y: 18.0,
		radius: 1.0,
	},
	Shape::Lines(&[(9.5, 6.0), (20.0, 6.0)]),
	Shape::Lines(&[(9.5, 12.0), (20.0, 12.0)]),
	Shape::Lines(&[(9.5, 18.0), (20.0, 18.0)]),
];

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_destination_appears_once_and_only_the_phone_has_logs() {
		let items: Vec<Item> = PHONE_ROWS
			.iter()
			.flat_map(|row| row.iter().copied())
			.collect();
		assert_eq!(
			items,
			[
				Item::Notepad,
				Item::Habits,
				Item::Notes,
				Item::People,
				Item::Travel,
				Item::BrainDump,
				Item::Alarms,
				Item::Logs,
			]
		);
		assert!(!ROWS.iter().any(|row| row.contains(&Item::Logs)));
	}

	#[test]
	fn only_the_desktop_has_routines() {
		let items: Vec<Item> = ROWS.iter().flat_map(|row| row.iter().copied()).collect();
		assert_eq!(
			items,
			[
				Item::Notepad,
				Item::Habits,
				Item::Notes,
				Item::People,
				Item::Travel,
				Item::BrainDump,
				Item::Routines,
				Item::Alarms,
			]
		);
		assert!(!PHONE_ROWS.iter().any(|row| row.contains(&Item::Routines)));
	}

	#[test]
	fn every_picture_stays_on_its_square() {
		let on_square = |(x, y): (f32, f32)| (0.0..=24.0).contains(&x) && (0.0..=24.0).contains(&y);
		for item in ROWS.iter().chain(&PHONE_ROWS).flat_map(|row| row.iter()) {
			assert!(!item.picture().is_empty(), "{item:?}");
			for shape in item.picture() {
				let inside = match *shape {
					Shape::Lines(points) | Shape::Polygon(points) => {
						points.iter().all(|point| on_square(*point))
					}
					Shape::Circle { x, y, radius } | Shape::Arc { x, y, radius, .. } => {
						on_square((x - radius, y - radius)) && on_square((x + radius, y))
					}
					Shape::RoundedRect {
						x,
						y,
						width,
						height,
						..
					} => on_square((x, y)) && on_square((x + width, y + height)),
				};
				assert!(inside, "{item:?} leaves its square: {shape:?}");
			}
		}
	}
}
