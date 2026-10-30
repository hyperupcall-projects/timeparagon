//! Reading a user's sound file into the format the stream wants.
//!
//! `rodio` is built here with `default-features = false` and only the
//! Symphonia decoders enabled, so this uses its format support without
//! pulling in `cpal` or any of its playback machinery.

use std::fmt;
use std::fs::File;
use std::num::NonZero;
use std::path::Path;
use std::time::Duration;

use rodio::source::UniformSourceIterator;

use super::{CHANNELS, Pcm, SAMPLE_RATE};

/// Refuse to load anything longer than this.
///
/// An alarm is a few seconds long; without a limit, pointing the picker at an
/// album would quietly try to hold the decoded PCM in memory (roughly 23 MB
/// per minute at 48 kHz stereo).
pub const MAX_DURATION: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub enum Error {
    Open(std::io::Error),
    Decode(String),
    Empty,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open(error) => write!(f, "could not open the file: {error}"),
            Self::Decode(error) => write!(f, "could not decode the file: {error}"),
            Self::Empty => f.write_str("the file contains no audio"),
        }
    }
}

impl std::error::Error for Error {}

/// Decodes `path` into interleaved stereo at our sample rate.
pub fn load(path: &Path) -> Result<Pcm, Error> {
    let file = File::open(path).map_err(Error::Open)?;

    let decoder =
        rodio::Decoder::try_from(file).map_err(|error| Error::Decode(error.to_string()))?;

    let channels = NonZero::new(CHANNELS as u16).expect("CHANNELS is not zero");
    let rate = NonZero::new(SAMPLE_RATE).expect("SAMPLE_RATE is not zero");

    // UniformSourceIterator does the resampling and channel mapping, so a mono
    // 22 kHz wav and a 96 kHz stereo flac both come out the same shape.
    let converted = UniformSourceIterator::new(decoder, channels, rate);

    let limit = MAX_DURATION.as_secs_f64() * f64::from(SAMPLE_RATE) * CHANNELS as f64;
    let samples: Vec<f32> = converted.take(limit as usize).collect();

    if samples.is_empty() {
        return Err(Error::Empty);
    }

    Ok(fade_edges(samples))
}

/// Applies a short fade at both ends.
///
/// Arbitrary files do not necessarily start or end at a zero crossing, and
/// truncating one at [`MAX_DURATION`] certainly does not.
fn fade_edges(mut samples: Vec<f32>) -> Pcm {
    let fade_frames = (0.005 * SAMPLE_RATE as f32) as usize * CHANNELS;
    let total = samples.len();
    let fade_frames = fade_frames.min(total / 2);

    for index in 0..fade_frames {
        let gain = index as f32 / fade_frames as f32;

        samples[index] *= gain;
        samples[total - 1 - index] *= gain;
    }

    samples.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_an_open_error() {
        let error = load(Path::new("/nonexistent/alarm.wav")).unwrap_err();
        assert!(matches!(error, Error::Open(_)));
    }

    #[test]
    fn a_file_that_is_not_audio_is_a_decode_error() {
        let path = std::env::temp_dir().join("timeslay-not-audio.wav");
        std::fs::write(&path, b"this is definitely not a wav file").unwrap();

        let error = load(&path).unwrap_err();
        assert!(matches!(error, Error::Decode(_)), "got {error:?}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn fading_leaves_the_edges_silent() {
        let faded = fade_edges(vec![1.0; SAMPLE_RATE as usize]);

        assert_eq!(faded[0], 0.0);
        assert_eq!(faded[faded.len() - 1], 0.0);
        // The middle should be untouched.
        assert_eq!(faded[faded.len() / 2], 1.0);
    }

    #[test]
    fn fading_a_tiny_buffer_does_not_panic() {
        let faded = fade_edges(vec![1.0, 1.0]);
        assert_eq!(faded.len(), 2);
    }
}
