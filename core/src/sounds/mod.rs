//! The sounds TimeParagon generates itself, shared by the desktop alarms, the
//! desktop reminders and the Android notification channels.
//!
//! Everything is synthesized, so no audio files live in the repository. The
//! Android build renders each tone to a WAV in `res/raw` (see the
//! `render_sounds` example), because a notification channel can only play a
//! sound the system can open by URI.

pub mod synth;

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Everything is rendered at this rate; the player resamples if the sink
/// disagrees.
pub const SAMPLE_RATE: u32 = 48_000;

/// Interleaved stereo.
pub const CHANNELS: usize = 2;

/// A rendered sound: interleaved stereo `f32` frames at [`SAMPLE_RATE`].
///
/// Shared rather than copied because a realtime audio callback may hold onto
/// it while the rest of the app carries on.
pub type Pcm = Arc<[f32]>;

/// One of the tones we generate ourselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
	#[default]
	Chime,
	Bell,
	Beep,
	Marimba,
	SoftPulse,
	Ding,
	DingDong,
	Rise,
	Fall,
	Kalimba,
	Glass,
	Knock,
	Bubble,
}

impl Tone {
	pub const ALL: [Self; 13] = [
		Self::Chime,
		Self::Bell,
		Self::Beep,
		Self::Marimba,
		Self::SoftPulse,
		Self::Ding,
		Self::DingDong,
		Self::Rise,
		Self::Fall,
		Self::Kalimba,
		Self::Glass,
		Self::Knock,
		Self::Bubble,
	];

	pub fn render(self) -> Pcm {
		match self {
			Self::Chime => synth::chime(),
			Self::Bell => synth::bell(),
			Self::Beep => synth::beep(),
			Self::Marimba => synth::marimba(),
			Self::SoftPulse => synth::soft_pulse(),
			Self::Ding => synth::ding(),
			Self::DingDong => synth::ding_dong(),
			Self::Rise => synth::rise(),
			Self::Fall => synth::fall(),
			Self::Kalimba => synth::kalimba(),
			Self::Glass => synth::glass(),
			Self::Knock => synth::knock(),
			Self::Bubble => synth::bubble(),
		}
	}

	/// The config name, which is also the Android resource name after
	/// [`ANDROID_RAW_PREFIX`].
	pub fn name(self) -> &'static str {
		match self {
			Self::Chime => "chime",
			Self::Bell => "bell",
			Self::Beep => "beep",
			Self::Marimba => "marimba",
			Self::SoftPulse => "soft_pulse",
			Self::Ding => "ding",
			Self::DingDong => "ding_dong",
			Self::Rise => "rise",
			Self::Fall => "fall",
			Self::Kalimba => "kalimba",
			Self::Glass => "glass",
			Self::Knock => "knock",
			Self::Bubble => "bubble",
		}
	}

	pub fn from_name(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|tone| tone.name() == name)
	}

	/// The file `res/raw` holds this tone in on Android, without extension.
	pub fn android_resource(self) -> String {
		format!("{ANDROID_RAW_PREFIX}{}", self.name())
	}
}

/// Prefixes the generated Android resources so they cannot collide with
/// anything else in `res/raw`.
pub const ANDROID_RAW_PREFIX: &str = "timeparagon_tone_";

impl fmt::Display for Tone {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(match self {
			Self::Chime => "Chime",
			Self::Bell => "Bell",
			Self::Beep => "Beep",
			Self::Marimba => "Marimba",
			Self::SoftPulse => "Soft Pulse",
			Self::Ding => "Ding",
			Self::DingDong => "Ding Dong",
			Self::Rise => "Rise",
			Self::Fall => "Fall",
			Self::Kalimba => "Kalimba",
			Self::Glass => "Glass",
			Self::Knock => "Knock",
			Self::Bubble => "Bubble",
		})
	}
}

/// Encodes a sound as a mono 16-bit PCM WAV file. The tones are the same in
/// both channels, so keeping one halves the size.
pub fn wav(pcm: &[f32]) -> Vec<u8> {
	let mono: Vec<f32> = pcm.iter().step_by(CHANNELS).copied().collect();
	let channels = 1u16;
	let bytes_per_sample = 2u16;
	let data_len = (mono.len() * bytes_per_sample as usize) as u32;
	let mut out = Vec::with_capacity(44 + data_len as usize);

	out.extend_from_slice(b"RIFF");
	out.extend_from_slice(&(36 + data_len).to_le_bytes());
	out.extend_from_slice(b"WAVEfmt ");
	out.extend_from_slice(&16u32.to_le_bytes());
	// Integer PCM.
	out.extend_from_slice(&1u16.to_le_bytes());
	out.extend_from_slice(&channels.to_le_bytes());
	out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
	let block_align = channels * bytes_per_sample;
	out.extend_from_slice(&(SAMPLE_RATE * u32::from(block_align)).to_le_bytes());
	out.extend_from_slice(&block_align.to_le_bytes());
	out.extend_from_slice(&(bytes_per_sample * 8).to_le_bytes());
	out.extend_from_slice(b"data");
	out.extend_from_slice(&data_len.to_le_bytes());
	for sample in &mono {
		let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
		out.extend_from_slice(&value.to_le_bytes());
	}

	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn names_round_trip_and_suit_android_resources() {
		for tone in Tone::ALL {
			assert_eq!(Tone::from_name(tone.name()), Some(tone));
			assert!(
				tone.android_resource()
					.chars()
					.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
				"{} is not a valid resource name",
				tone.android_resource()
			);
		}
	}

	#[test]
	fn serde_names_match_the_config_names() {
		#[derive(Serialize, Deserialize)]
		struct Wrapper {
			tone: Tone,
		}
		for tone in Tone::ALL {
			let text = toml::to_string(&Wrapper { tone }).unwrap();
			assert_eq!(text.trim(), format!("tone = \"{}\"", tone.name()));
		}
	}

	#[test]
	fn a_wav_has_a_correct_header_and_length() {
		let pcm = Tone::Ding.render();
		let bytes = wav(&pcm);

		assert_eq!(&bytes[0..4], b"RIFF");
		assert_eq!(&bytes[8..12], b"WAVE");
		assert_eq!(bytes.len(), 44 + pcm.len() / CHANNELS * 2);
		assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 1);
		let riff_len = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
		assert_eq!(riff_len as usize, bytes.len() - 8);
	}
}
