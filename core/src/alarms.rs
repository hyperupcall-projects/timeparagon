//! Shared one-shot alarms and their persisted desktop-compatible profiles.
use crate::{Color, calendar::time_format::TimeFormat, hex::HexColor, sounds::Tone};
use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use std::{
	io,
	path::{Path, PathBuf},
	time::Duration,
};

/// Everything that happens when an alarm goes off.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AlarmProfile {
	pub notification: Notification,
	pub sound: Sound,
	pub flash: Flash,
	/// Bring the window to the front when the alarm fires.
	pub raise_window: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notification {
	pub enabled: bool,
	pub summary: String,
	pub body: String,
	pub urgency: Urgency,
	/// Seconds to keep the notification up; 0 leaves it to the daemon.
	pub timeout_secs: u32,
}

impl Default for Notification {
	fn default() -> Self {
		Self {
			enabled: true,
			summary: "Time's up".to_owned(),
			body: String::new(),
			urgency: Urgency::Critical,
			timeout_secs: 0,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Urgency {
	Low,
	#[default]
	Normal,
	Critical,
}

impl Urgency {
	pub const ALL: [Self; 3] = [Self::Low, Self::Normal, Self::Critical];
}

impl std::fmt::Display for Urgency {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			Self::Low => "Low",
			Self::Normal => "Normal",
			Self::Critical => "Critical",
		})
	}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sound {
	pub enabled: bool,
	pub choice: SoundChoice,
	/// 0-100, as shown on the slider.
	pub volume: u8,
	/// How many times to play the sound; 0 loops until dismissed.
	pub repeats: u8,
	pub fade_in_ms: u64,
}

impl Default for Sound {
	fn default() -> Self {
		Self {
			enabled: true,
			choice: SoundChoice::default(),
			volume: 70,
			repeats: 2,
			fade_in_ms: 150,
		}
	}
}

impl Sound {
	/// The slider position as a linear amplitude multiplier.
	///
	/// Squared because loudness is perceived roughly logarithmically: a linear
	/// mapping makes the bottom half of the slider do almost nothing.
	pub fn gain(&self) -> f32 {
		let fraction = f32::from(self.volume.min(100)) / 100.0;
		fraction * fraction
	}

	pub fn fade_in(&self) -> Duration {
		Duration::from_millis(self.fade_in_ms)
	}
}

/// Which sound to play.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SoundChoice {
	/// One of the tones we synthesize ourselves.
	Builtin(Tone),
	/// Any audio file the user picked.
	Custom { file: PathBuf },
}

impl Default for SoundChoice {
	fn default() -> Self {
		Self::Builtin(Tone::default())
	}
}

impl std::fmt::Display for SoundChoice {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Builtin(tone) => tone.fmt(f),
			Self::Custom { file } => {
				let name = file
					.file_name()
					.map(|name| name.to_string_lossy().into_owned())
					.unwrap_or_else(|| file.display().to_string());

				write!(f, "{name}")
			}
		}
	}
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Flash {
	pub enabled: bool,
	pub color: HexColor,
	/// The most opaque the overlay ever gets, 0.0 to 1.0.
	pub peak_opacity: f32,
	/// How many times to pulse.
	pub pulses: u8,
	/// Length of a single fade in and out.
	pub pulse_ms: u64,
	/// Let clicks through to whatever is underneath the flash.
	pub passthrough: bool,
}

impl Default for Flash {
	fn default() -> Self {
		Self {
			enabled: true,
			color: HexColor(Color::from_rgb8(0xFF, 0x20, 0x20)),
			peak_opacity: 0.35,
			pulses: 2,
			pulse_ms: 700,
			passthrough: false,
		}
	}
}

impl Flash {
	/// The shortest pulse we are willing to draw.
	///
	/// Fast full-screen flashing is a seizure risk, so the rate is capped here
	/// rather than trusted from the config file. Three per second is the
	/// commonly cited ceiling for general-audience content.
	pub const MIN_PULSE: Duration = Duration::from_millis(334);

	pub fn pulse_period(&self) -> Duration {
		Duration::from_millis(self.pulse_ms).max(Self::MIN_PULSE)
	}

	/// Total length of the flash, after clamping.
	pub fn total_duration(&self) -> Duration {
		self.pulse_period() * u32::from(self.pulses.max(1))
	}

	pub fn peak(&self) -> f32 {
		self.peak_opacity.clamp(0.0, 1.0)
	}
}

/// One alarm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alarm {
	pub id: u64,
	pub at: DateTime<Local>,
	/// What the alarm is for. Empty for an unnamed alarm.
	#[serde(default)]
	pub label: String,
	/// What happens when it goes off.
	#[serde(default)]
	pub profile: AlarmProfile,
}

impl Alarm {
	/// The notification and sound to run when this alarm goes off.
	///
	/// A label becomes the notification's headline, and the body always says
	/// what time was asked for, so the notification never falls back to the
	/// timer wording.
	pub fn firing_profile(&self, time_format: TimeFormat) -> AlarmProfile {
		let mut profile = self.profile.clone();

		if !self.label.trim().is_empty() {
			profile.notification.summary = self.label.trim().to_owned();
		}
		if profile.notification.body.trim().is_empty() {
			profile.notification.body = format!(
				"Alarm for {}",
				crate::calendar::time_format::clock(self.at, time_format)
			);
		}

		profile
	}

	/// What the alarm's needle says beside it: its time, and its label if it
	/// has one.
	pub fn needle_label(&self, time_format: TimeFormat) -> String {
		let time = crate::calendar::time_format::clock(self.at, time_format);
		let label = self.label.trim();

		if label.is_empty() {
			time
		} else {
			format!("{time} {label}")
		}
	}
}

/// `when` with the seconds taken off.
pub fn minute_of(when: DateTime<Local>) -> DateTime<Local> {
	// Subtracting rather than `with_second`, which gives up on a local time
	// that happens twice, as it does in the hour the clocks go back.
	when - chrono::Duration::seconds(i64::from(when.second()))
		- chrono::Duration::nanoseconds(i64::from(when.nanosecond()))
}

/// Whole minutes from `now` to `at`, counted between the clock faces rather
/// than the instants: from 10:00:40, a 10:45 alarm is 45 minutes away, which
/// is what the clock on the wall says.
pub fn minutes_between(now: DateTime<Local>, at: DateTime<Local>) -> i64 {
	(minute_of(at) - minute_of(now)).num_minutes()
}

/// Whether an alarm may be set for `at`.
///
/// It has to be at least a minute ahead of the clock: an alarm for this very
/// minute is already as good as in the past.
pub fn can_set(now: DateTime<Local>, at: DateTime<Local>) -> bool {
	minutes_between(now, at) >= 1
}

/// A length of time as short as it can be said: `45m`, `2h`, `1h 15m`,
/// `1d 3h`.
pub fn compact(minutes: u64) -> String {
	let (days, hours, minutes) = (minutes / 1_440, minutes % 1_440 / 60, minutes % 60);

	let parts: Vec<String> = [(days, 'd'), (hours, 'h'), (minutes, 'm')]
		.into_iter()
		.filter(|(amount, _)| *amount > 0)
		.map(|(amount, unit)| format!("{amount}{unit}"))
		.collect();

	// A day and some minutes reads better as "1d 5m" than "1d 0h 5m", and
	// zero has to say something.
	if parts.is_empty() {
		"0m".to_owned()
	} else {
		parts.join(" ")
	}
}

/// When an alarm goes off, relative to now: `Goes off in 45m`.
pub fn countdown_label(minutes: i64) -> String {
	if minutes >= 1 {
		format!("Goes off in {}", compact(minutes as u64))
	} else {
		"Going off now".to_owned()
	}
}

/// How many times a sound plays, as its slider says it.
pub fn repeat_label(repeats: u8) -> String {
	match repeats {
		0 => "Repeat until dismissed".to_owned(),
		1 => "Play once".to_owned(),
		count => format!("Play {count} times"),
	}
}

/// Reads `HH:MM` as a time of day.
pub fn parse_time(text: &str) -> Option<NaiveTime> {
	NaiveTime::parse_from_str(text.trim(), "%H:%M").ok()
}

/// The local moment on `date` at `time`, taking the earlier of two when a
/// clock change makes it ambiguous. `None` when the clock skips it.
pub fn local_at(date: NaiveDate, time: NaiveTime) -> Option<DateTime<Local>> {
	Local.from_local_datetime(&date.and_time(time)).earliest()
}

/// The moment a typed time puts an alarm at, on the date it already has, or
/// why that time cannot be used.
pub fn retimed(
	date: NaiveDate,
	text: &str,
	now: DateTime<Local>,
) -> Result<DateTime<Local>, String> {
	let time = parse_time(text).ok_or("Use the 24-hour form, like 07:30")?;

	match local_at(date, time) {
		Some(at) if can_set(now, at) => Ok(at),
		Some(_) => Err("That time has already passed".to_owned()),
		None => Err("That time does not exist on this day".to_owned()),
	}
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
	pub alarms: Vec<Alarm>,
}

pub fn read(path: &Path) -> io::Result<Library> {
	Ok(crate::task_db::read_document(path, crate::task_db::keys::ALARMS)?.unwrap_or_default())
}
pub fn write(path: &Path, library: &Library) -> io::Result<()> {
	crate::task_db::write_document(path, crate::task_db::keys::ALARMS, library)
}
/// Random IDs also keep alarms created offline on different devices distinct.
pub fn new_id() -> u64 {
	let mut bytes = [0; 8];
	getrandom::fill(&mut bytes).expect("alarm identity needs system randomness");
	u64::from_ne_bytes(bytes)
}

/// An unobserved poll result. A background read must not advance the editor's
/// merge baseline until the UI actually adopts that read.
#[derive(Debug, Clone)]
pub struct Snapshot {
	pub library: Library,
	json: Option<String>,
}

impl Snapshot {
	pub fn observe(&self, path: &Path) {
		crate::database_sync::projection::remember(
			path,
			crate::task_db::keys::ALARMS,
			self.json.clone(),
		);
	}
}

pub fn poll(path: &Path) -> io::Result<Snapshot> {
	use rusqlite::OptionalExtension;
	let mut db = crate::task_db::Database::open(path).map_err(io::Error::other)?;
	let json: Option<String> = db
		.connection()
		.query_row(
			"SELECT value FROM documents WHERE key=?1",
			[crate::task_db::keys::ALARMS],
			|row| row.get(0),
		)
		.optional()
		.map_err(io::Error::other)?;
	let library = json
		.as_deref()
		.map(serde_json::from_str)
		.transpose()
		.map_err(io::Error::other)?
		.unwrap_or_default();
	Ok(Snapshot { library, json })
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn an_unapplied_poll_does_not_replace_the_editors_merge_baseline() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("alarms.db");
		let at = crate::clock::now() + chrono::Duration::hours(1);
		let a = Alarm {
			id: new_id(),
			at,
			label: "A".into(),
			profile: AlarmProfile::default(),
		};
		let b = Alarm {
			id: new_id(),
			label: "B".into(),
			..a.clone()
		};
		write(
			&path,
			&Library {
				alarms: vec![a.clone()],
			},
		)
		.unwrap();
		let mut editing = read(&path).unwrap();
		let mut db = crate::task_db::Database::open(&path).unwrap();
		let remote = Library {
			alarms: vec![a, b.clone()],
		};
		db.connection()
			.execute(
				"UPDATE documents SET value=?1 WHERE key='alarms'",
				[serde_json::to_string(&remote).unwrap()],
			)
			.unwrap();
		let _unapplied = poll(&path).unwrap();
		editing.alarms[0].label = "Edited locally".into();
		write(&path, &editing).unwrap();
		let merged = read(&path).unwrap();
		assert_eq!(merged.alarms.len(), 2);
		assert!(merged.alarms.contains(&b));
		assert!(merged.alarms.iter().any(|a| a.label == "Edited locally"));
	}
}
