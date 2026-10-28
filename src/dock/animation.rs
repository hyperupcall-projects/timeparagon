//! Opacity animation for the dock's first appearance, as in the overlay app.

use std::time::{Duration, Instant};

use crate::config::WindowAnimation;

pub struct Opening {
    mode: WindowAnimation,
    duration: Duration,
    started: Option<Instant>,
    /// When the fade-out began, and the opacity it fades from.
    closing: Option<(Instant, u8)>,
    opacity: u8,
}

impl Opening {
    pub fn new(mode: WindowAnimation, duration_ms: u32) -> Self {
        Self {
            mode,
            duration: Duration::from_millis(u64::from(duration_ms)),
            started: None,
            closing: None,
            opacity: 255,
        }
    }

    /// Starts the fade-out from the current opacity. Returns false when this
    /// mode has no fade, so the caller can quit at once.
    pub fn begin_close(&mut self, now: Instant) -> bool {
        if self.mode != WindowAnimation::FadeIn || self.duration.is_zero() {
            return false;
        }
        self.closing.get_or_insert((now, self.opacity));
        true
    }

    /// Whether a started fade-out has reached full transparency.
    pub fn is_closed(&self) -> bool {
        self.closing.is_some() && self.opacity == 0
    }

    /// The opacity at `now`, and whether another frame is needed. The fade
    /// starts at the first buffer, not while waiting for the configure.
    pub fn sample(&mut self, now: Instant) -> (u8, bool) {
        if let Some((started, from)) = self.closing {
            let elapsed = now.saturating_duration_since(started);
            let remaining = 1.0 - (elapsed.as_secs_f64() / self.duration.as_secs_f64()).min(1.0);
            self.opacity = (f64::from(from) * remaining).round() as u8;
            return (self.opacity, self.opacity > 0);
        }
        if self.mode != WindowAnimation::FadeIn || self.duration.is_zero() {
            return (255, false);
        }
        let started = *self.started.get_or_insert(now);
        let elapsed = now.saturating_duration_since(started);
        if elapsed >= self.duration {
            self.opacity = 255;
            return (255, false);
        }
        let alpha = (elapsed.as_secs_f64() / self.duration.as_secs_f64() * 255.0).round() as u8;
        self.opacity = alpha;
        (alpha, true)
    }
}

/// ARGB8888 buffers hold premultiplied color, so every channel scales together.
pub fn apply_opacity(canvas: &mut [u8], opacity: u8) {
    if opacity == 255 {
        return;
    }
    for channel in canvas {
        *channel = ((u16::from(*channel) * u16::from(opacity) + 127) / 255) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fade_starts_at_the_first_sample_and_ends_at_the_duration() {
        let now = Instant::now();
        let mut opening = Opening::new(WindowAnimation::FadeIn, 400);

        assert_eq!(opening.sample(now), (0, true));
        assert_eq!(
            opening.sample(now + Duration::from_millis(200)),
            (128, true)
        );
        assert_eq!(
            opening.sample(now + Duration::from_millis(400)),
            (255, false)
        );
    }

    #[test]
    fn a_close_fades_out_from_the_current_opacity() {
        let now = Instant::now();
        let mut opening = Opening::new(WindowAnimation::FadeIn, 400);
        opening.sample(now);
        opening.sample(now + Duration::from_millis(400));
        let later = now + Duration::from_secs(1);

        assert!(opening.begin_close(later));
        assert_eq!(opening.sample(later), (255, true));
        assert!(!opening.is_closed());
        assert_eq!(
            opening.sample(later + Duration::from_millis(200)),
            (128, true)
        );
        assert_eq!(
            opening.sample(later + Duration::from_millis(400)),
            (0, false)
        );
        assert!(opening.is_closed());
    }

    #[test]
    fn modes_without_a_fade_close_at_once() {
        let now = Instant::now();
        assert!(!Opening::new(WindowAnimation::Instant, 400).begin_close(now));
        assert!(!Opening::new(WindowAnimation::FadeIn, 0).begin_close(now));
    }

    #[test]
    fn other_modes_and_zero_durations_are_opaque_at_once() {
        let now = Instant::now();
        for mut opening in [
            Opening::new(WindowAnimation::DesktopDefault, 400),
            Opening::new(WindowAnimation::Instant, 400),
            Opening::new(WindowAnimation::FadeIn, 0),
        ] {
            assert_eq!(opening.sample(now), (255, false));
        }
    }

    #[test]
    fn opacity_scales_every_channel() {
        let mut pixel = [200, 100, 50, 255];
        apply_opacity(&mut pixel, 128);
        assert_eq!(pixel, [100, 50, 25, 128]);
    }
}
