//! The big readout, and the ring around it.

use iced::widget::canvas::{self, Cache, Frame, Geometry, Path, Stroke, Style};
use iced::widget::{canvas as canvas_widget, center, column, container, responsive, stack, text};
use iced::{Color, Element, Length, Radians, Rectangle, Renderer, Theme, mouse};

use crate::app::Message;
use crate::engine::format::{self, Precision};

use super::layout::{self, Metrics};

/// Smallest readout we are willing to draw. Below this it stops being a
/// glanceable clock, so the layout is expected to give it more room instead.
const MIN_SIZE: f32 = 16.0;

/// Largest, so a maximized window does not look absurd.
const MAX_SIZE: f32 = 128.0;

/// A ring needs at least this much room to be worth drawing. Any smaller and
/// it just crowds the digits it is supposed to frame.
const RING_MIN: f32 = 110.0;

/// How much of the ring's diameter the digits may span.
///
/// A chord across a circle has less room than its diameter, so the text has
/// to stay comfortably inside it.
const RING_TEXT_FRACTION: f32 = 0.72;

/// How large the "of 10:00" caption is, as a fraction of the main digits.
const RING_CAPTION_FRACTION: f32 = 0.45;

/// The smallest the caption is allowed to shrink, so it stays legible even
/// in a very short window.
const MIN_CAPTION: f32 = 12.0;

/// Formats a duration for the readout at the current size.
pub fn format(duration: std::time::Duration, precision: Precision, metrics: &Metrics) -> String {
    format::readout(duration, precision, metrics.allow_hours)
}

/// The readout as a plain line of digits, filling the width it is given.
pub fn readout<'a>(value: String, metrics: &Metrics, color: Color) -> Element<'a, Message> {
    let padding = metrics.padding;

    responsive(move |area| {
        let width = (area.width - padding).max(40.0);
        let size = fitted(width, area.height, &value);

        digits(value.clone(), size, color)
    })
    .into()
}

/// The readout with a progress ring around it.
///
/// Both the ring and the digits are sized from the space this widget is
/// actually given, measured rather than inferred from the window: the slot
/// left over after the buttons is much shorter than the window, and sizing
/// the ring from the window is what makes it collide with its own digits.
pub fn with_ring<'a>(
    value: String,
    caption: String,
    metrics: &Metrics,
    color: Color,
    ring: Color,
    progress: f32,
) -> Element<'a, Message> {
    let padding = metrics.padding;

    responsive(move |area| {
        let diameter = (area.width - padding).min(area.height).max(40.0);

        // In a cramped slot the digits alone read better than a ring
        // squeezed around them.
        if diameter < RING_MIN {
            let width = (area.width - padding).max(40.0);
            let size = fitted(width, area.height, &value);

            return captioned(value.clone(), caption.clone(), size, color);
        }

        let size = fitted(
            diameter * RING_TEXT_FRACTION,
            diameter * RING_TEXT_FRACTION,
            &value,
        );

        let ring = canvas_widget(Ring {
            progress,
            color: ring,
            cache: Cache::new(),
        })
        .width(Length::Fixed(diameter))
        .height(Length::Fixed(diameter));

        center(stack![
            center(ring),
            center(captioned(value.clone(), caption.clone(), size, color)),
        ])
        .into()
    })
    .into()
}

/// The font size that fills `width` without growing taller than `height`.
fn fitted(width: f32, height: f32, value: &str) -> f32 {
    let characters = value.chars().count();

    layout::digit_size(width, characters)
        // A glyph is roughly 1.3x its font size tall including leading.
        .min(height / 1.3)
        .clamp(MIN_SIZE, MAX_SIZE)
}

fn digits<'a>(value: String, size: f32, color: Color) -> Element<'a, Message> {
    // Monospaced so the digits do not shuffle sideways as the numbers change,
    // which is distracting on a moving readout.
    container(
        text(value)
            .size(size)
            .font(iced::Font::MONOSPACE)
            .color(color),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}

/// The big digits with the "of 10:00" caption tucked beneath them.
fn captioned<'a>(value: String, caption: String, size: f32, color: Color) -> Element<'a, Message> {
    let caption_size = (size * RING_CAPTION_FRACTION).max(MIN_CAPTION);

    container(
        column![
            text(value)
                .size(size)
                .font(iced::Font::MONOSPACE)
                .color(color),
            text(caption)
                .size(caption_size)
                .color(color.scale_alpha(0.7)),
        ]
        .align_x(iced::Center)
        .spacing(size * 0.08),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}

/// A circular progress indicator.
struct Ring {
    progress: f32,
    color: Color,
    cache: Cache,
}

impl canvas::Program<Message> for Ring {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let geometry = self
            .cache
            .draw(renderer, bounds.size(), |frame: &mut Frame| {
                let center = frame.center();
                let width = (frame.width() * 0.035).clamp(2.0, 6.0);
                // A stroked path straddles its own line, so half the stroke width
                // has to come out of the radius or it gets clipped.
                let radius = (frame.width().min(frame.height()) - width) / 2.0;

                frame.stroke(
                    &Path::circle(center, radius),
                    Stroke {
                        style: Style::Solid(theme.extended_palette().background.weak.color),
                        width,
                        ..Stroke::default()
                    },
                );

                if self.progress <= 0.0 {
                    return;
                }

                // Start at twelve o'clock and sweep clockwise, the way every
                // other timer does.
                let start = Radians(-std::f32::consts::FRAC_PI_2);
                let sweep = Radians(std::f32::consts::TAU * self.progress.clamp(0.0, 1.0));

                let arc = Path::new(|builder| {
                    builder.arc(canvas::path::Arc {
                        center,
                        radius,
                        start_angle: start,
                        end_angle: start + sweep,
                    });
                });

                frame.stroke(
                    &arc,
                    Stroke {
                        style: Style::Solid(self.color),
                        width,
                        line_cap: canvas::LineCap::Round,
                        ..Stroke::default()
                    },
                );
            });

        vec![geometry]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::Size;
    use std::time::Duration;

    #[test]
    fn the_readout_drops_hours_in_a_narrow_window() {
        let narrow = Metrics::of(Size::new(200.0, 300.0));
        let wide = Metrics::of(Size::new(400.0, 300.0));
        let duration = Duration::from_secs(3_905);

        assert_eq!(format(duration, Precision::Seconds, &narrow), "65:05");
        assert_eq!(format(duration, Precision::Seconds, &wide), "1:05:05");
    }

    #[test]
    fn the_fitted_size_keeps_the_readout_inside_its_width() {
        for value in ["3:47", "65:05", "1:05:05", "12:34.56"] {
            for width in [80.0, 160.0, 400.0, 900.0] {
                let size = fitted(width, 600.0, value);
                let drawn = layout::digit_width(size, value.chars().count());

                assert!(
                    drawn <= width + 0.5 || size == MIN_SIZE,
                    "{value:?} at {width}px would need {drawn:.0}px"
                );
            }
        }
    }

    #[test]
    fn the_fitted_size_respects_the_height_it_is_given() {
        let size = fitted(900.0, 60.0, "3:47");

        assert!(size <= 60.0 / 1.3 + 0.01, "got {size}");
    }

    #[test]
    fn digits_inside_a_ring_stay_within_the_circle() {
        // The whole bug this guards against: digits wider than the ring that
        // is supposed to frame them.
        for diameter in [RING_MIN, 200.0, 400.0, 800.0] {
            for value in ["3:47", "1:05:05"] {
                let span = diameter * RING_TEXT_FRACTION;
                let size = fitted(span, span, value);
                let drawn = layout::digit_width(size, value.chars().count());

                assert!(
                    drawn <= diameter,
                    "{value:?} spans {drawn:.0}px inside a {diameter:.0}px ring"
                );
            }
        }
    }

    #[test]
    fn the_readout_never_shrinks_below_legibility() {
        assert_eq!(fitted(10.0, 10.0, "1:05:05"), MIN_SIZE);
    }
}
