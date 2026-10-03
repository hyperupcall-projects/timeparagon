//! Sizing decisions, all in one place.
//!
//! Every number the layout needs is derived from the space actually measured
//! by `responsive`, rather than hardcoded, because the whole point is to stay
//! usable in a 200px window without giving up on wider ones.

use iced::Size;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breakpoint {
    /// Roughly a phone-width sliver. Icons only, one column, no labels.
    Compact,
    /// Comfortable single column with labels.
    Normal,
    /// Room for the clock and its controls side by side.
    Wide,
}

impl Breakpoint {
    pub const COMPACT_BELOW: f32 = 260.0;
    pub const WIDE_FROM: f32 = 420.0;

    pub fn of(width: f32) -> Self {
        if width < Self::COMPACT_BELOW {
            Self::Compact
        } else if width < Self::WIDE_FROM {
            Self::Normal
        } else {
            Self::Wide
        }
    }

    pub fn is_compact(self) -> bool {
        self == Self::Compact
    }
}

/// Everything the views need to know about the space they have.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub breakpoint: Breakpoint,
    pub size: Size,
    /// Font size for the big readout.
    pub digits: f32,
    /// Whether the readout has room to spell out hours separately.
    pub allow_hours: bool,
    /// Whether buttons get text labels next to their icons.
    pub labels: bool,
    pub spacing: f32,
    pub padding: f32,
    /// Height of the primary action button.
    pub button: f32,
    /// How many preset chips fit on a row.
    pub preset_columns: usize,
}

/// How wide one monospaced character is, as a fraction of the font size.
///
/// Measured from the rendered output rather than assumed; the real advance is
/// closer to 0.55, and the slack keeps a readout from touching the edges.
const CHARACTER_WIDTH: f32 = 0.62;

/// The font size at which `characters` monospaced digits fill `width`.
///
/// Deriving this from the string that is actually going to be drawn, rather
/// than from a fixed guess at its length, is what keeps `1:05:05` and `3:47`
/// both filling the window instead of one overflowing and the other looking
/// lost.
pub fn digit_size(width: f32, characters: usize) -> f32 {
    let characters = characters.max(1) as f32;

    width / (CHARACTER_WIDTH * characters)
}

/// The width a readout will take at a given font size.
pub fn digit_width(size: f32, characters: usize) -> f32 {
    size * CHARACTER_WIDTH * characters.max(1) as f32
}

impl Metrics {
    pub fn of(size: Size) -> Self {
        let breakpoint = Breakpoint::of(size.width);
        let padding = if breakpoint.is_compact() { 8.0 } else { 14.0 };
        let usable = (size.width - padding * 2.0).max(60.0);

        Self {
            breakpoint,
            size,
            // A starting point for text that is not the main readout; the
            // readout itself is sized against the space it actually gets.
            digits: digit_size(usable, 5).min(size.height / 4.0).clamp(18.0, 96.0),
            // Below this the extra `H:` group pushes the readout over the
            // edge, so hours get folded into the minutes instead.
            allow_hours: size.width >= 240.0,
            labels: !breakpoint.is_compact(),
            spacing: if breakpoint.is_compact() { 6.0 } else { 10.0 },
            padding,
            button: if breakpoint.is_compact() { 34.0 } else { 40.0 },
            preset_columns: match breakpoint {
                Breakpoint::Compact => 2,
                Breakpoint::Normal => 3,
                Breakpoint::Wide => 5,
            },
        }
    }

    /// Font size for secondary text, which shrinks more gently than the
    /// readout so it never becomes unreadable.
    pub fn label(&self) -> f32 {
        if self.breakpoint.is_compact() { 11.0 } else { 13.0 }
    }

    /// Size of a square icon-only button.
    pub fn icon_button(&self) -> f32 {
        if self.breakpoint.is_compact() { 30.0 } else { 34.0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoints_split_where_the_layout_changes() {
        assert_eq!(Breakpoint::of(180.0), Breakpoint::Compact);
        assert_eq!(Breakpoint::of(200.0), Breakpoint::Compact);
        assert_eq!(Breakpoint::of(259.0), Breakpoint::Compact);
        assert_eq!(Breakpoint::of(260.0), Breakpoint::Normal);
        assert_eq!(Breakpoint::of(419.0), Breakpoint::Normal);
        assert_eq!(Breakpoint::of(420.0), Breakpoint::Wide);
    }

    #[test]
    fn the_readout_fits_the_window_it_is_given() {
        for width in [180.0, 200.0, 240.0, 300.0, 500.0, 900.0] {
            let metrics = Metrics::of(Size::new(width, 600.0));
            let estimated = digit_width(metrics.digits, 5);
            let available = width - metrics.padding * 2.0;

            assert!(
                estimated <= available.max(60.0) + 0.5,
                "at {width}px the readout would need {estimated:.0}px of {available:.0}px"
            );
        }
    }

    #[test]
    fn the_digit_size_adapts_to_how_long_the_readout_is() {
        // `1:05:05` has to be smaller than `3:47` to fill the same width.
        let short = digit_size(200.0, 4);
        let long = digit_size(200.0, 7);

        assert!(long < short);
        assert!(digit_width(short, 4) <= 200.5);
        assert!(digit_width(long, 7) <= 200.5);
    }

    #[test]
    fn digit_sizing_survives_a_degenerate_string() {
        assert!(digit_size(100.0, 0).is_finite());
    }

    #[test]
    fn the_readout_stays_legible_even_in_a_sliver() {
        let metrics = Metrics::of(Size::new(180.0, 240.0));
        assert!(metrics.digits >= 18.0);
    }

    #[test]
    fn a_short_window_does_not_get_giant_digits() {
        let tall = Metrics::of(Size::new(900.0, 600.0));
        let squat = Metrics::of(Size::new(900.0, 120.0));

        assert!(squat.digits < tall.digits);
        assert!(squat.digits <= 120.0 / 4.0);
    }

    #[test]
    fn hours_are_only_spelled_out_when_they_fit() {
        assert!(!Metrics::of(Size::new(200.0, 300.0)).allow_hours);
        assert!(Metrics::of(Size::new(240.0, 300.0)).allow_hours);
    }

    #[test]
    fn compact_drops_labels_and_wide_keeps_them() {
        assert!(!Metrics::of(Size::new(200.0, 300.0)).labels);
        assert!(Metrics::of(Size::new(300.0, 300.0)).labels);
    }

    #[test]
    fn digits_never_exceed_the_cap_on_a_huge_window() {
        assert_eq!(Metrics::of(Size::new(4_000.0, 4_000.0)).digits, 96.0);
    }
}
