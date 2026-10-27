//! Official Google Calendar color palettes.
//!
//! These IDs and hex values are what the Calendar colors endpoint returns.
//! Hard-coding them means the day view still looks like Google Calendar when
//! we have not fetched the palette yet, and it keeps event chips stable if
//! that request fails.

use iced::Color;

use crate::hex;

/// The red Google Calendar uses for the current-time needle.
pub const NEEDLE: Color = Color {
    r: 0xEA as f32 / 255.0,
    g: 0x43 as f32 / 255.0,
    b: 0x35 as f32 / 255.0,
    a: 1.0,
};

/// The purple of an alarm's needle, picked to read apart from the red of the
/// current-time needle even where the two sit close together.
pub const ALARM: Color = Color {
    r: 0x8E as f32 / 255.0,
    g: 0x44 as f32 / 255.0,
    b: 0xC9 as f32 / 255.0,
    a: 1.0,
};

/// An alarm needle that cannot be set, because its time has passed.
pub const ALARM_REFUSED: Color = Color {
    r: 0x9A as f32 / 255.0,
    g: 0x8F as f32 / 255.0,
    b: 0xA6 as f32 / 255.0,
    a: 1.0,
};

/// Hour-grid lines, matching the light Google Calendar day view.
pub const GRID_LINE: Color = Color {
    r: 0xDA as f32 / 255.0,
    g: 0xDC as f32 / 255.0,
    b: 0xE0 as f32 / 255.0,
    a: 1.0,
};

/// Hour labels in the gutter.
pub const GUTTER_TEXT: Color = Color {
    r: 0x70 as f32 / 255.0,
    g: 0x75 as f32 / 255.0,
    b: 0x7A as f32 / 255.0,
    a: 1.0,
};

/// Google's task blue. Calendar paints every Google Tasks chip, and every
/// task list before it is given a color of its own, with this.
pub const TASK: Color = Color {
    r: 0x54 as f32 / 255.0,
    g: 0x84 as f32 / 255.0,
    b: 0xED as f32 / 255.0,
    a: 1.0,
};

/// Fallback when a calendar has no color of its own.
pub const DEFAULT_EVENT: Color = Color {
    r: 0x03 as f32 / 255.0,
    g: 0x87 as f32 / 255.0,
    b: 0xCD as f32 / 255.0,
    a: 1.0,
};

/// Event `colorId` values from the Google Calendar colors API.
const EVENT: &[(&str, &str)] = &[
    ("1", "#A4BDFC"),  // Lavender
    ("2", "#7AE7BF"),  // Sage
    ("3", "#DBADFF"),  // Grape
    ("4", "#FF887C"),  // Flamingo
    ("5", "#FBD75B"),  // Banana
    ("6", "#FFB878"),  // Tangerine
    ("7", "#46D6DB"),  // Peacock
    ("8", "#E1E1E1"),  // Graphite
    ("9", "#5484ED"),  // Blueberry
    ("10", "#51B749"), // Basil
    ("11", "#DC2127"), // Tomato
];

/// Calendar `colorId` values from the same endpoint.
const CALENDAR: &[(&str, &str)] = &[
    ("1", "#AC725E"),
    ("2", "#D06B64"),
    ("3", "#F83A22"),
    ("4", "#FA573C"),
    ("5", "#FF7537"),
    ("6", "#FFAD46"),
    ("7", "#42D692"),
    ("8", "#16A765"),
    ("9", "#7BD148"),
    ("10", "#B3DC6C"),
    ("11", "#FBE983"),
    ("12", "#FAD165"),
    ("13", "#92E1C0"),
    ("14", "#9FE1E7"),
    ("15", "#9FC6E7"),
    ("16", "#4986E7"),
    ("17", "#9A9CFF"),
    ("18", "#B99AFF"),
    ("19", "#C2C2C2"),
    ("20", "#CABDBF"),
    ("21", "#CCA6AC"),
    ("22", "#F691B2"),
    ("23", "#CD74E6"),
    ("24", "#A47AE2"),
];

pub fn event(id: &str) -> Option<Color> {
    lookup(EVENT, id)
}

pub fn calendar(id: &str) -> Option<Color> {
    lookup(CALENDAR, id)
}

/// The colors offered when customizing a Google Tasks list, in the same order
/// as Google Calendar's event palette.
pub fn palette() -> Vec<Color> {
    EVENT
        .iter()
        .filter_map(|(_, hex)| hex::parse(hex).ok())
        .collect()
}

/// Picks a readable label color for a Google Calendar chip.
///
/// Google itself uses near-black on the pastel event colors and white on the
/// few dark ones (Tomato, the deep calendar reds). Luminance is enough to
/// match that split without shipping a second palette.
pub fn on_color(background: Color) -> Color {
    let luminance = 0.2126 * background.r + 0.7152 * background.g + 0.0722 * background.b;

    if luminance > 0.55 {
        Color::from_rgb8(0x1F, 0x1F, 0x1F)
    } else {
        Color::WHITE
    }
}

fn lookup(table: &[(&str, &str)], id: &str) -> Option<Color> {
    table
        .iter()
        .find(|(key, _)| *key == id)
        .and_then(|(_, hex)| hex::parse(hex).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_eleven_event_colors_match_google() {
        assert_eq!(event("1"), Some(hex::parse("#A4BDFC").unwrap()));
        assert_eq!(event("11"), Some(hex::parse("#DC2127").unwrap()));
        assert_eq!(event("99"), None);
    }

    #[test]
    fn tomato_gets_white_text_and_sage_gets_dark_text() {
        let tomato = event("11").unwrap();
        let sage = event("2").unwrap();

        assert_eq!(on_color(tomato), Color::WHITE);
        assert_ne!(on_color(sage), Color::WHITE);
    }

    #[test]
    fn the_needle_is_google_red() {
        assert_eq!(NEEDLE, hex::parse("#EA4335").unwrap());
    }
}
