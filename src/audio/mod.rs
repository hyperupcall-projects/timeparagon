//! Alarm sound playback.
//!
//! Sound goes straight to PipeWire. `rodio` is here only for its Symphonia
//! decoders, so nothing in this module touches ALSA, cpal or PulseAudio.
//!
//! The whole design is arranged around one constraint: PipeWire's `process`
//! callback runs on a realtime thread, so every sound is rendered into a
//! buffer up front and the callback does nothing but copy out of it.

#[cfg(feature = "pipewire-audio")]
mod backend;

pub mod decode;
pub mod synth;

use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Everything is rendered at this rate; PipeWire resamples if the sink
/// disagrees.
pub const SAMPLE_RATE: u32 = 48_000;

/// Interleaved stereo.
pub const CHANNELS: usize = 2;

/// A rendered sound: interleaved stereo `f32` frames at [`SAMPLE_RATE`].
///
/// Shared rather than copied because the realtime callback holds onto it
/// while the rest of the app carries on.
pub type Pcm = Arc<[f32]>;

/// One of the tones we generate ourselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    #[default]
    Chime,
    Bell,
    Beep,
    Marimba,
    SoftPulse,
}

impl Tone {
    pub const ALL: [Self; 5] = [
        Self::Chime,
        Self::Bell,
        Self::Beep,
        Self::Marimba,
        Self::SoftPulse,
    ];

    pub fn render(self) -> Pcm {
        match self {
            Self::Chime => synth::chime(),
            Self::Bell => synth::bell(),
            Self::Beep => synth::beep(),
            Self::Marimba => synth::marimba(),
            Self::SoftPulse => synth::soft_pulse(),
        }
    }
}

impl fmt::Display for Tone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Chime => "Chime",
            Self::Bell => "Bell",
            Self::Beep => "Beep",
            Self::Marimba => "Marimba",
            Self::SoftPulse => "Soft Pulse",
        })
    }
}

/// A request to play something.
pub struct Play {
    pub pcm: Pcm,
    /// How many times to play it; 0 loops until stopped.
    pub repeats: u8,
    pub fade_in: Duration,
}

/// What the loop thread is being asked to do.
pub enum Command {
    Play(Play),
    Stop,
    Shutdown,
}

/// Whether we can play anything.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Status {
    /// Nothing has been played yet, so there is nothing to report.
    #[default]
    Unknown,
    Ready,
    /// PipeWire could not be reached. Carries something worth showing a user.
    Unavailable(String),
}

impl Status {
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

/// State shared with the audio thread and its realtime callback.
pub struct Shared {
    /// Playback gain, as `f32` bits.
    ///
    /// An atomic rather than a channel message so that dragging the volume
    /// slider changes the level of a sound that is already playing, without
    /// the realtime thread ever having to synchronize with anyone.
    gain: AtomicU32,

    /// Handoff to the realtime callback.
    ///
    /// The callback only ever `try_lock`s this, so it cannot be blocked by
    /// the loop thread holding it.
    pending: Mutex<Option<Command>>,

    /// Buffers the realtime callback has finished with, parked here so they
    /// are freed on the loop thread instead.
    retired: Mutex<Vec<Pcm>>,

    /// Set by the realtime callback when a sound plays out.
    finished: AtomicBool,

    /// Whether the audio thread is still running.
    alive: AtomicBool,

    status: Mutex<Status>,
}

impl Shared {
    fn new() -> Self {
        Self {
            gain: AtomicU32::new(1.0_f32.to_bits()),
            pending: Mutex::new(None),
            retired: Mutex::new(Vec::new()),
            finished: AtomicBool::new(false),
            alive: AtomicBool::new(false),
            status: Mutex::new(Status::Unknown),
        }
    }

    pub fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }

    pub fn set_gain(&self, gain: f32) {
        self.gain
            .store(gain.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Hands a command to the realtime callback.
    fn submit(&self, command: Command) {
        if let Ok(mut pending) = self.pending.lock() {
            *pending = Some(command);
        }
    }

    /// Parks a finished buffer for the loop thread to free.
    ///
    /// Called from the realtime thread, so it must not block: if the loop
    /// thread happens to hold the lock we drop the buffer here instead, which
    /// is the rare case and still better than waiting.
    fn retire(&self, pcm: Pcm) {
        if let Ok(mut retired) = self.retired.try_lock() {
            retired.push(pcm);
        }
    }

    fn collect_retired(&self) {
        if let Ok(mut retired) = self.retired.lock() {
            retired.clear();
        }
    }

    fn set_status(&self, status: Status) {
        if let Ok(mut current) = self.status.lock() {
            *current = status;
        }
    }

    pub fn status(&self) -> Status {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_default()
    }
}

/// The handle the rest of the app holds.
///
/// Cheap to construct: no thread is started and no connection is made until
/// something actually needs to be played.
pub struct Audio {
    shared: Arc<Shared>,
    #[cfg(feature = "pipewire-audio")]
    sender: Option<pipewire::channel::Sender<Command>>,
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

impl Audio {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared::new()),
            #[cfg(feature = "pipewire-audio")]
            sender: None,
        }
    }

    pub fn status(&self) -> Status {
        self.shared.status()
    }

    /// Sets the playback gain, taking effect mid-sound.
    pub fn set_gain(&self, gain: f32) {
        self.shared.set_gain(gain);
    }

    /// Starts a sound, replacing whatever was playing.
    #[cfg(feature = "pipewire-audio")]
    pub fn play(&mut self, play: Play) {
        self.send(Command::Play(play));
    }

    #[cfg(not(feature = "pipewire-audio"))]
    pub fn play(&mut self, _play: Play) {
        self.shared.set_status(Status::Unavailable(
            "this build was compiled without PipeWire support".to_owned(),
        ));
    }

    pub fn stop(&mut self) {
        #[cfg(feature = "pipewire-audio")]
        if self.is_running() {
            self.send(Command::Stop);
        }
    }

    #[cfg(feature = "pipewire-audio")]
    fn is_running(&self) -> bool {
        self.sender.is_some() && self.shared.alive.load(Ordering::Acquire)
    }

    /// Sends a command, starting the audio thread if it has gone away.
    ///
    /// The thread shuts itself down when it has been idle for a while, so the
    /// common case after a long gap is that there is nothing to send to and a
    /// fresh thread has to be spawned.
    #[cfg(feature = "pipewire-audio")]
    fn send(&mut self, command: Command) {
        if !self.is_running() {
            self.spawn();
        }

        let Some(sender) = &self.sender else {
            return;
        };

        if sender.send(command).is_err() {
            // The thread went away between the check and the send. One retry
            // on a fresh thread, then give up rather than spin.
            log::debug!("audio thread went away; restarting it");
            self.spawn();

            if let Some(sender) = &self.sender {
                let _ = sender.send(Command::Stop);
            }
        }
    }

    #[cfg(feature = "pipewire-audio")]
    fn spawn(&mut self) {
        let (sender, receiver) = pipewire::channel::channel();
        let shared = self.shared.clone();

        shared.alive.store(true, Ordering::Release);

        match std::thread::Builder::new()
            .name("timeslay-audio".to_owned())
            .spawn(move || backend::run(shared, receiver))
        {
            Ok(_handle) => self.sender = Some(sender),
            Err(error) => {
                self.shared.alive.store(false, Ordering::Release);
                self.shared.set_status(Status::Unavailable(format!(
                    "could not start the audio thread: {error}"
                )));
            }
        }
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        #[cfg(feature = "pipewire-audio")]
        if let Some(sender) = &self.sender {
            let _ = sender.send(Command::Shutdown);
        }
    }
}

/// Renders whichever sound the config asks for.
///
/// Decoding happens off the UI thread (this is called from a task), because a
/// large file can take long enough to drop a frame.
pub fn render(choice: &crate::config::SoundChoice) -> Result<Pcm, String> {
    match choice {
        crate::config::SoundChoice::Builtin(tone) => Ok(tone.render()),
        crate::config::SoundChoice::Custom { file } => load_file(file),
    }
}

fn load_file(path: &Path) -> Result<Pcm, String> {
    decode::load(path).map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tones_all_render_to_something_audible() {
        for tone in Tone::ALL {
            assert!(!tone.render().is_empty(), "{tone} rendered nothing");
        }
    }

    #[test]
    fn tone_names_round_trip_through_serde() {
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            tone: Tone,
        }

        for tone in Tone::ALL {
            let text = toml::to_string(&Wrapper { tone }).unwrap();
            let parsed: Wrapper = toml::from_str(&text).unwrap();

            assert_eq!(parsed.tone, tone);
        }

        // The names in the config file are snake_case, not the display names.
        let parsed: Wrapper = toml::from_str("tone = \"soft_pulse\"").unwrap();
        assert_eq!(parsed.tone, Tone::SoftPulse);
    }

    #[test]
    fn gain_is_clamped_and_round_trips() {
        let shared = Shared::new();

        shared.set_gain(0.25);
        assert_eq!(shared.gain(), 0.25);

        shared.set_gain(5.0);
        assert_eq!(shared.gain(), 1.0);

        shared.set_gain(-1.0);
        assert_eq!(shared.gain(), 0.0);
    }

    #[test]
    fn a_fresh_handle_starts_no_thread_and_claims_nothing() {
        let audio = Audio::new();

        assert_eq!(audio.status(), Status::Unknown);
        assert!(!audio.shared.alive.load(Ordering::Acquire));
    }

    #[test]
    fn retired_buffers_are_freed_on_collection() {
        let shared = Shared::new();
        let pcm: Pcm = vec![0.0; 16].into();

        shared.retire(pcm.clone());
        assert_eq!(Arc::strong_count(&pcm), 2);

        shared.collect_retired();
        assert_eq!(Arc::strong_count(&pcm), 1);
    }

    #[test]
    fn rendering_a_builtin_choice_goes_through_the_synth() {
        use crate::config::SoundChoice;

        let pcm = render(&SoundChoice::Builtin(Tone::Beep)).unwrap();
        assert!(!pcm.is_empty());
    }

    #[test]
    fn rendering_a_missing_file_explains_which_file() {
        use crate::config::SoundChoice;

        let error = render(&SoundChoice::Custom {
            file: "/nonexistent/alarm.wav".into(),
        })
        .unwrap_err();

        assert!(error.contains("/nonexistent/alarm.wav"), "got {error}");
    }
}
