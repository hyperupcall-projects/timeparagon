//! Hex colors, so both the config and the theme files can speak CSS.

use std::fmt;

use iced::Color;
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An [`iced::Color`] that round-trips through TOML as `"#RRGGBB"`.
///
/// `#RRGGBBAA` is accepted on the way in so a theme can specify translucency,
/// and short `#RGB` forms are accepted because people write them by hand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HexColor(pub Color);

impl From<Color> for HexColor {
    fn from(color: Color) -> Self {
        Self(color)
    }
}

impl From<HexColor> for Color {
    fn from(hex: HexColor) -> Self {
        hex.0
    }
}

impl fmt::Display for HexColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Color { r, g, b, a } = self.0;
        let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;

        if a >= 1.0 {
            write!(f, "#{:02X}{:02X}{:02X}", byte(r), byte(g), byte(b))
        } else {
            write!(
                f,
                "#{:02X}{:02X}{:02X}{:02X}",
                byte(r),
                byte(g),
                byte(b),
                byte(a)
            )
        }
    }
}

/// Parses `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA`, with or without the `#`.
pub fn parse(input: &str) -> Result<Color, ParseError> {
    let digits = input.trim().trim_start_matches('#');

    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ParseError(input.to_owned()));
    }

    let nibble = |index: usize| -> f32 {
        let digit = digits.as_bytes()[index] as char;
        (digit.to_digit(16).unwrap_or(0) as f32) / 15.0
    };

    let byte = |index: usize| -> f32 {
        let pair = &digits[index..index + 2];
        (u8::from_str_radix(pair, 16).unwrap_or(0) as f32) / 255.0
    };

    match digits.len() {
        3 => Ok(Color::from_rgb(nibble(0), nibble(1), nibble(2))),
        4 => Ok(Color::from_rgba(nibble(0), nibble(1), nibble(2), nibble(3))),
        6 => Ok(Color::from_rgb(byte(0), byte(2), byte(4))),
        8 => Ok(Color::from_rgba(byte(0), byte(2), byte(4), byte(6))),
        _ => Err(ParseError(input.to_owned())),
    }
}

#[derive(Debug, Clone)]
pub struct ParseError(String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` is not a hex color like #FF2020", self.0)
    }
}

impl std::error::Error for ParseError {}

impl Serialize for HexColor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for HexColor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct HexVisitor;

        impl<'v> Visitor<'v> for HexVisitor {
            type Value = HexColor;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a hex color such as \"#FF2020\"")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<HexColor, E> {
                parse(value).map(HexColor).map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_str(HexVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_six_digit_colors() {
        let color = parse("#FF2020").unwrap();
        assert_eq!(color, Color::from_rgb8(0xFF, 0x20, 0x20));
    }

    #[test]
    fn hash_is_optional_and_case_insensitive() {
        assert_eq!(parse("ff2020").unwrap(), parse("#FF2020").unwrap());
    }

    #[test]
    fn parses_short_and_alpha_forms() {
        assert_eq!(parse("#FFF").unwrap(), Color::WHITE);
        assert_eq!(parse("#00000000").unwrap().a, 0.0);
    }

    #[test]
    fn rejects_nonsense() {
        assert!(parse("#GGGGGG").is_err());
        assert!(parse("#FFFFF").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn round_trips_through_display() {
        let original = "#D6558A";
        let color = HexColor(parse(original).unwrap());
        assert_eq!(color.to_string(), original);
    }

    #[test]
    fn keeps_alpha_when_displaying_translucent_colors() {
        let color = HexColor(Color::from_rgba(1.0, 0.0, 0.0, 0.0));
        assert_eq!(color.to_string(), "#FF000000");
    }
}
