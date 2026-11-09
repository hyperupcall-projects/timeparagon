//! Shared one-shot alarms and their persisted desktop-compatible profiles.
use crate::{Color, calendar::time_format::TimeFormat, hex::HexColor, sounds::Tone};
use chrono::{DateTime, Local};
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
