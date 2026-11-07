//! The stylesheet: `assets/day.css`, under the variables it is written in.
//!
//! The values come from `timeslay_core::theme`, the same place the iced app's
//! widget styles get them, so the two apps cannot drift apart.

use std::fmt::Write;
use std::sync::LazyLock;

use timeslay_core::Color;
use timeslay_core::calendar::colors;
use timeslay_core::theme::{self as look, Elevation, Scheme};

use crate::css_color;

const LAYOUT: &str = include_str!("../assets/day.css");

pub static STYLESHEET: LazyLock<String> =
    LazyLock::new(|| format!("{}{}\n{LAYOUT}", fixed_variables(), scheme_rules(),));

fn shadow(elevation: Elevation, is_dark: bool) -> String {
    format!(
        "0 {}px {}px {}",
        elevation.y,
        elevation.blur,
        css_color(elevation.color(is_dark))
    )
}

fn scheme_variables(scheme: Scheme) -> String {
    let palette = scheme.extended();
    let dark = palette.is_dark;
    let colors: [(&str, Color); 27] = [
        ("bg", palette.background.base.color),
        ("text", palette.background.base.text),
        ("card", palette.background.weakest.color),
        ("on-card", palette.background.weakest.text),
        ("hover", palette.background.weaker.color),
        ("pressed", palette.background.weak.color),
        ("hairline", palette.background.neutral.color),
        ("edge", palette.background.strong.color),
        ("edge-strong", palette.background.stronger.color),
        ("muted", palette.secondary.base.color),
        ("accent", palette.primary.base.color),
        ("on-accent", palette.primary.base.text),
        (
            "accent-edge",
            look::shade(palette.primary.base.color, look::FILLED_EDGE_SHADE),
        ),
        (
            "accent-hover",
            look::shade(palette.primary.base.color, look::FILLED_HOVER_SHADE),
        ),
        ("accent-soft", palette.primary.weak.color),
        (
            "accent-faint",
            look::tint(
                palette.background.weakest.color,
                palette.primary.base.color,
                look::TODAY_TINT,
            ),
        ),
        (
            "accent-faint-hover",
            look::tint(
                palette.background.weaker.color,
                palette.primary.base.color,
                look::TODAY_HOVER_TINT,
            ),
        ),
        (
            "accent-faint-pressed",
            look::tint(
                palette.background.weak.color,
                palette.primary.base.color,
                look::TODAY_PRESS_TINT,
            ),
        ),
        ("on-accent-soft", palette.primary.weak.text),
        ("danger", palette.danger.base.color),
        ("on-danger", palette.danger.base.text),
        (
            "danger-edge",
            look::shade(palette.danger.base.color, look::FILLED_EDGE_SHADE),
        ),
        ("danger-soft", palette.danger.weak.color),
        ("on-danger-soft", palette.danger.weak.text),
        ("success", palette.success.base.color),
        ("warning-soft", palette.warning.weak.color),
        ("on-warning-soft", palette.warning.weak.text),
    ];
    let shadows = [
        ("shadow-card", look::CARD),
        ("shadow-row", look::ROW),
        ("shadow-floating", look::FLOATING),
        ("shadow-menu", look::MENU),
        ("shadow-button", look::BUTTON),
        ("shadow-button-hover", look::BUTTON_HOVERED),
        ("shadow-button-press", look::BUTTON_PRESSED),
        ("shadow-nav", look::NAV_ACTIVE),
    ];

    let mut css = format!("color-scheme: {};", if dark { "dark" } else { "light" });
    for (name, color) in colors {
        let _ = write!(css, "--{name}:{};", css_color(color));
    }
    for (name, elevation) in shadows {
        let _ = write!(css, "--{name}:{};", shadow(elevation, dark));
    }
    css.push_str(&habit_variables(dark));
    css
}

/// Loop's palette and chart grays for one scheme, which the habit screens draw
/// with, as the iced app's charts pick them by whether the theme is dark.
fn habit_variables(dark: bool) -> String {
    let mut css = String::new();
    for index in 0..timeslay_core::habits::PALETTE_SIZE {
        let color = timeslay_core::habits::palette_color(index, dark);
        let _ = write!(
            css,
            "--habit-{index}:{};--habit-ink-{index}:{};",
            css_color(color),
            css_color(colors::on_color(color))
        );
    }
    let gray = |light: u8, night: u8| {
        let value = if dark { night } else { light };
        Color::from_rgb8(value, value, value)
    };
    let quiet = gray(0xBD, 0x75);
    for (name, color) in [
        ("habit-low", gray(0xE0, 0x42)),
        ("habit-medium", gray(0x9E, 0x9E)),
        ("habit-quiet", quiet),
        ("habit-quiet-ink", colors::on_color(quiet)),
        ("habit-day-text", gray(0x75, 0xBD)),
    ] {
        let _ = write!(css, "--{name}:{};", css_color(color));
    }
    // Loop's dark theme keeps the page bar black, as a pale color would leave
    // its white text unreadable.
    let _ = write!(css, "--habit-bar-black:{}%;", if dark { 100 } else { 0 });
    css
}

/// What does not change with the theme.
fn fixed_variables() -> String {
    let [(near_y, near_alpha), (far_y, far_alpha)] = look::EVENT_DROPS;
    let mut css = String::from(":root{");
    for (name, value) in [
        ("radius-card", look::RADIUS_CARD),
        ("radius-row", look::RADIUS_ROW),
        ("radius-control", look::RADIUS_CONTROL),
        ("radius-small", look::RADIUS_SMALL),
        ("event-radius", look::EVENT_RADIUS),
        ("event-pad-left", look::BLOCK_PADDING),
        ("event-pad-right", look::BLOCK_PADDING_RIGHT),
        ("needle-radius", look::NEEDLE_RADIUS),
    ] {
        let _ = write!(css, "--{name}:{value}px;");
    }
    for (name, color) in [
        ("scrim", look::SCRIM),
        ("hour-line", look::HOUR_LINE),
        ("event-edge", look::EVENT_EDGE),
        ("travel-edge", look::TRAVEL_EDGE),
        ("block-filled", look::BLOCK_FILLED),
        ("block-action", look::BLOCK_ACTION),
        ("block-row-text", look::BLOCK_ROW_TEXT),
        ("block-row-over-text", look::BLOCK_ROW_OVER_TEXT),
        ("block-row-over", look::BLOCK_ROW_OVER),
        ("needle", colors::NEEDLE),
        ("gutter-text", colors::GUTTER_TEXT),
    ] {
        let _ = write!(css, "--{name}:{};", css_color(color));
    }
    let _ = write!(
        css,
        "--shadow-event:0 {near_y}px 0 {},0 {far_y}px 0 {};",
        css_color(Color::BLACK.scale_alpha(near_alpha)),
        css_color(Color::BLACK.scale_alpha(far_alpha)),
    );
    let _ = write!(css, "--shadow-chip:{};", shadow(look::EVENT_CHIP, false));
    // A habit's own color and the ink on it, which each habit sets for itself,
    // and a streak's length against the longest, which each bar sets.
    css.push_str("--hc:var(--accent);--hc-ink:var(--on-accent);--share:1;");
    css.push_str("}\n");
    css
}

/// One rule per theme. The two `system` choices start on their light scheme
/// and take the dark one when the operating system asks for it.
fn scheme_rules() -> String {
    let rule = |selector: &str, scheme| format!("{selector}{{{}}}\n", scheme_variables(scheme));
    format!(
        "{}{}{}{}{}{}@media (prefers-color-scheme: dark){{\n{}{}}}\n",
        rule(
            ":root:not([data-theme]),:root[data-theme=\"system\"]",
            Scheme::Light
        ),
        rule(
            ":root[data-theme=\"system-high-contrast\"]",
            Scheme::LightHighContrast
        ),
        rule(":root[data-theme=\"light\"]", Scheme::Light),
        rule(
            ":root[data-theme=\"light-high-contrast\"]",
            Scheme::LightHighContrast
        ),
        rule(":root[data-theme=\"dark\"]", Scheme::Dark),
        rule(
            ":root[data-theme=\"dark-high-contrast\"]",
            Scheme::DarkHighContrast
        ),
        rule(
            ":root:not([data-theme]),:root[data-theme=\"system\"]",
            Scheme::Dark
        ),
        rule(
            ":root[data-theme=\"system-high-contrast\"]",
            Scheme::DarkHighContrast
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variable_the_layout_uses_is_defined_by_every_theme() {
        let mut used: Vec<&str> = LAYOUT
            .split("var(--")
            .skip(1)
            .map(|rest| rest.split([')', ',']).next().unwrap())
            .collect();
        used.sort_unstable();
        used.dedup();
        assert!(!used.is_empty());

        let fixed = fixed_variables();
        for scheme in Scheme::ALL {
            let variables = scheme_variables(scheme);
            for name in &used {
                let declared = format!("--{name}:");
                assert!(
                    variables.contains(&declared) || fixed.contains(&declared),
                    "{name} is not defined for {}",
                    scheme.name()
                );
            }
        }
    }

    #[test]
    fn the_light_high_contrast_theme_matches_the_desktop_palette() {
        let css = scheme_variables(Scheme::LightHighContrast);
        assert!(css.contains("--card:rgba(250, 250, 250, 1.00);"), "{css}");
        assert!(css.contains("--edge:rgba(100, 100, 100, 1.00);"), "{css}");
        assert!(css.contains("--accent:rgba(3, 73, 180, 1.00);"), "{css}");
    }

    #[test]
    fn every_saved_theme_name_has_a_rule() {
        let rules = scheme_rules();
        for theme in crate::day::Theme::ALL {
            assert!(
                rules.contains(&format!("[data-theme=\"{}\"]", theme.name())),
                "{}",
                theme.name()
            );
        }
    }
}
