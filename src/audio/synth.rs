//! The built-in alarm sounds.
//!
//! These are generated rather than shipped as files, which keeps the repo free
//! of binary assets and sidesteps sample licensing entirely. Each one is a
//! handful of decaying partials, which is enough to sound like a struck object
//! rather than a raw beep.

use std::f32::consts::TAU;

use super::{CHANNELS, Pcm, SAMPLE_RATE};

/// One decaying sine component.
struct Partial {
    /// Hertz.
    frequency: f32,
    /// Relative level before normalization.
    amplitude: f32,
    /// Exponential decay rate; larger dies away faster.
    decay: f32,
}

/// Fade applied to the very end of every sound, long enough to avoid the
/// click of cutting a waveform off mid-cycle.
const TAIL_FADE: f32 = 0.005;

/// Attack applied to struck sounds. Not zero, for the same reason.
const STRIKE_ATTACK: f32 = 0.004;

/// Sums decaying partials into a mono buffer.
fn struck(duration: f32, partials: &[Partial]) -> Vec<f32> {
    let frames = (duration * SAMPLE_RATE as f32) as usize;
    let mut buffer = vec![0.0; frames];

    for (index, sample) in buffer.iter_mut().enumerate() {
        let t = index as f32 / SAMPLE_RATE as f32;

        let value: f32 = partials
            .iter()
            .map(|partial| {
                partial.amplitude * (-partial.decay * t).exp() * (TAU * partial.frequency * t).sin()
            })
            .sum();

        *sample = value * attack(t, STRIKE_ATTACK);
    }

    finish(buffer, duration)
}

/// A short burst of beeps, for people who want to be told rather than chimed at.
fn beeps(count: usize, on: f32, off: f32, frequency: f32) -> Vec<f32> {
    let duration = count as f32 * (on + off);
    let frames = (duration * SAMPLE_RATE as f32) as usize;
    let mut buffer = vec![0.0; frames];

    for (index, sample) in buffer.iter_mut().enumerate() {
        let t = index as f32 / SAMPLE_RATE as f32;
        let phase_in_burst = t % (on + off);

        if phase_in_burst < on {
            // Fade each burst in and out so the gaps are silent, not clicky.
            let envelope = attack(phase_in_burst, 0.008) * attack(on - phase_in_burst, 0.008);

            *sample = envelope * (TAU * frequency * t).sin();
        }
    }

    finish(buffer, duration)
}

/// A slow swell, for a gentle nudge.
fn swell(duration: f32, frequency: f32) -> Vec<f32> {
    let frames = (duration * SAMPLE_RATE as f32) as usize;
    let mut buffer = vec![0.0; frames];

    for (index, sample) in buffer.iter_mut().enumerate() {
        let t = index as f32 / SAMPLE_RATE as f32;
        let progress = t / duration;

        // Raised cosine: silent at both ends, loudest in the middle.
        let envelope = 0.5 - 0.5 * (TAU * progress).cos();

        *sample =
            envelope * ((TAU * frequency * t).sin() + 0.3 * (TAU * frequency * 2.0 * t).sin());
    }

    finish(buffer, duration)
}

/// A linear ramp from 0 to 1 over `length` seconds, clamped.
fn attack(t: f32, length: f32) -> f32 {
    (t / length).clamp(0.0, 1.0)
}

/// Normalizes to a consistent level and fades the tail out.
///
/// Normalizing matters because the tones are built from wildly different
/// numbers of partials; without it, switching sounds would also change the
/// volume, which makes the volume slider feel broken.
fn finish(mut buffer: Vec<f32>, duration: f32) -> Vec<f32> {
    let peak = buffer
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));

    if peak > 0.0 {
        let scale = 0.9 / peak;

        for sample in &mut buffer {
            *sample *= scale;
        }
    }

    let fade_frames = (TAIL_FADE * SAMPLE_RATE as f32) as usize;
    let total = buffer.len();

    for (offset, sample) in buffer.iter_mut().rev().take(fade_frames).enumerate() {
        *sample *= offset as f32 / fade_frames as f32;
    }

    debug_assert!(total <= (duration * SAMPLE_RATE as f32).ceil() as usize + 1);

    buffer
}

/// Duplicates a mono buffer into the interleaved stereo the stream expects.
fn to_stereo(mono: Vec<f32>) -> Pcm {
    let mut interleaved = Vec::with_capacity(mono.len() * CHANNELS);

    for sample in mono {
        for _ in 0..CHANNELS {
            interleaved.push(sample);
        }
    }

    interleaved.into()
}

pub fn chime() -> Pcm {
    to_stereo(struck(
        1.2,
        &[
            Partial {
                frequency: 880.0,
                amplitude: 1.0,
                decay: 3.0,
            },
            Partial {
                frequency: 1320.0,
                amplitude: 0.5,
                decay: 3.5,
            },
            Partial {
                frequency: 1760.0,
                amplitude: 0.25,
                decay: 4.2,
            },
        ],
    ))
}

pub fn bell() -> Pcm {
    // Deliberately inharmonic partials: that ratio spread is what reads as
    // metal rather than as a tuned note.
    to_stereo(struck(
        2.0,
        &[
            Partial {
                frequency: 587.0,
                amplitude: 1.0,
                decay: 1.5,
            },
            Partial {
                frequency: 1174.0,
                amplitude: 0.6,
                decay: 2.0,
            },
            Partial {
                frequency: 1761.0,
                amplitude: 0.3,
                decay: 2.6,
            },
            Partial {
                frequency: 2348.0,
                amplitude: 0.15,
                decay: 3.2,
            },
        ],
    ))
}

pub fn beep() -> Pcm {
    to_stereo(beeps(3, 0.12, 0.10, 660.0))
}

pub fn marimba() -> Pcm {
    to_stereo(struck(
        0.8,
        &[
            Partial {
                frequency: 440.0,
                amplitude: 1.0,
                decay: 6.0,
            },
            Partial {
                frequency: 1760.0,
                amplitude: 0.35,
                decay: 9.0,
            },
        ],
    ))
}

pub fn soft_pulse() -> Pcm {
    to_stereo(swell(1.4, 528.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(pcm: &Pcm) -> f32 {
        pcm.iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()))
    }

    #[test]
    fn every_tone_produces_interleaved_stereo_audio() {
        for pcm in [chime(), bell(), beep(), marimba(), soft_pulse()] {
            assert!(!pcm.is_empty());
            assert_eq!(pcm.len() % CHANNELS, 0);
        }
    }

    #[test]
    fn every_tone_is_normalized_and_never_clips() {
        for pcm in [chime(), bell(), beep(), marimba(), soft_pulse()] {
            let peak = peak(&pcm);

            assert!(peak <= 1.0, "peak {peak} would clip");
            assert!(
                peak > 0.5,
                "peak {peak} is too quiet to have been normalized"
            );
        }
    }

    #[test]
    fn every_tone_starts_and_ends_at_silence() {
        // A waveform that starts or stops abruptly clicks through the speakers.
        for pcm in [chime(), bell(), beep(), marimba(), soft_pulse()] {
            assert_eq!(pcm[0], 0.0);
            assert!(pcm[pcm.len() - 1].abs() < 1e-3);
        }
    }

    #[test]
    fn channels_carry_the_same_signal() {
        let pcm = chime();

        for frame in pcm.chunks_exact(CHANNELS) {
            assert_eq!(frame[0], frame[1]);
        }
    }

    #[test]
    fn tones_are_roughly_the_length_they_claim() {
        let seconds = |pcm: &Pcm| pcm.len() as f32 / CHANNELS as f32 / SAMPLE_RATE as f32;

        assert!((seconds(&chime()) - 1.2).abs() < 0.01);
        assert!((seconds(&bell()) - 2.0).abs() < 0.01);
        assert!((seconds(&marimba()) - 0.8).abs() < 0.01);
    }

    #[test]
    fn beeps_leave_silence_between_bursts() {
        let pcm = beep();
        let frames: Vec<f32> = pcm.chunks_exact(CHANNELS).map(|frame| frame[0]).collect();

        // Sample in the middle of the first gap, which starts at 0.12s.
        let in_gap = (0.17 * SAMPLE_RATE as f32) as usize;
        assert_eq!(frames[in_gap], 0.0);

        // And in the middle of the first burst, which should not be silent.
        let in_burst = (0.06 * SAMPLE_RATE as f32) as usize;
        assert!(frames[in_burst].abs() > 0.1);
    }
}
