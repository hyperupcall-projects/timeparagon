//! The PipeWire thread.
//!
//! `libpipewire` objects are neither `Send` nor `Sync`, so the loop lives on a
//! thread of its own and the rest of the app talks to it over a
//! [`pipewire::channel`], which is the one channel type that can wake a thread
//! parked inside a PipeWire loop.
//!
//! The stream is only connected when there is something to play and is torn
//! down again once things go quiet, so an idle TimeTracker holds no audio
//! device and puts no node on the graph.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use pipewire as pw;
use pw::spa;
use pw::spa::pod::Pod;

use super::{CHANNELS, Command, Pcm, Play, SAMPLE_RATE, Shared, Status};

/// Bytes per sample. We ask for `F32LE`; the server converts if the sink
/// wants something else.
const SAMPLE_SIZE: usize = size_of::<f32>();

/// Bytes per frame.
const STRIDE: usize = SAMPLE_SIZE * CHANNELS;

/// How long to keep a connected-but-silent stream around.
///
/// Reconnecting per sound would be cheap but not free, and both a repeating
/// alarm and someone auditioning tones in the settings pane play several in
/// quick succession.
const STREAM_LINGER: Duration = Duration::from_secs(10);

/// How long the thread itself sticks around with nothing to do.
const THREAD_LINGER: Duration = Duration::from_secs(60);

/// How often the loop thread checks on the playback it handed to the
/// realtime callback.
const POLL: Duration = Duration::from_millis(100);

/// What the realtime callback is currently playing.
///
/// This lives in the stream's user data, so it is only ever touched by the
/// realtime thread. New work arrives through [`Shared::pending`].
struct Playback {
    pcm: Option<Pcm>,
    /// Offset into `pcm`, in samples rather than frames.
    cursor: usize,
    /// Plays left after the current one. [`u32::MAX`] means loop forever.
    repeats_left: u32,
    fade_frames: usize,
    frames_played: usize,
    /// Set once the buffer has been played out.
    ///
    /// The buffer itself is deliberately *not* dropped here: freeing memory
    /// on the realtime thread is exactly the kind of unbounded work this
    /// callback has to avoid. The loop thread collects it instead.
    exhausted: bool,
}

impl Playback {
    const fn silent() -> Self {
        Self {
            pcm: None,
            cursor: 0,
            repeats_left: 0,
            fade_frames: 0,
            frames_played: 0,
            exhausted: false,
        }
    }

    /// Begins a new sound, handing back the previous buffer to be freed.
    fn start(&mut self, play: Play) -> Option<Pcm> {
        let previous = self.pcm.take();

        self.fade_frames = (play.fade_in.as_secs_f32() * SAMPLE_RATE as f32) as usize;
        self.repeats_left = if play.repeats == 0 {
            u32::MAX
        } else {
            u32::from(play.repeats) - 1
        };
        self.cursor = 0;
        self.frames_played = 0;
        self.exhausted = false;
        self.pcm = Some(play.pcm);

        previous
    }

    fn stop(&mut self) -> Option<Pcm> {
        self.repeats_left = 0;
        self.cursor = 0;
        self.exhausted = true;
        self.pcm.take()
    }

    fn is_playing(&self) -> bool {
        self.pcm.is_some() && !self.exhausted
    }

    /// The gain to apply to the next frame, accounting for the fade-in.
    fn envelope(&self) -> f32 {
        if self.fade_frames == 0 {
            1.0
        } else {
            (self.frames_played as f32 / self.fade_frames as f32).min(1.0)
        }
    }

    /// Produces the next frame, or `None` once playback is over.
    fn next_frame(&mut self) -> Option<[f32; CHANNELS]> {
        if self.exhausted {
            return None;
        }

        let pcm = self.pcm.as_ref()?;

        if self.cursor + CHANNELS > pcm.len() {
            if self.repeats_left == 0 {
                self.exhausted = true;
                return None;
            }

            if self.repeats_left != u32::MAX {
                self.repeats_left -= 1;
            }

            self.cursor = 0;
        }

        let gain = self.envelope();
        let mut frame = [0.0; CHANNELS];

        for (channel, sample) in frame.iter_mut().enumerate() {
            *sample = pcm[self.cursor + channel] * gain;
        }

        self.cursor += CHANNELS;
        self.frames_played += 1;

        Some(frame)
    }
}

/// Runs the PipeWire loop until told to shut down or until it has been idle
/// for [`THREAD_LINGER`].
///
/// Any failure is reported through [`Shared::status`] and ends the thread;
/// the app stays usable with notifications and the screen flash.
pub fn run(shared: Arc<Shared>, receiver: pw::channel::Receiver<Command>) {
    if let Err(error) = try_run(&shared, receiver) {
        log::warn!("audio unavailable: {error}");
        shared.set_status(Status::Unavailable(error));
    }

    shared.alive.store(false, Ordering::Release);
}

fn try_run(
    shared: &Arc<Shared>,
    receiver: pw::channel::Receiver<Command>,
) -> Result<(), String> {
    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(describe)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(describe)?;

    // This is the call that fails when there is no PipeWire session to talk
    // to, which is the case the settings pane needs to report.
    let core = context.connect_rc(None).map_err(describe)?;

    let stream = pw::stream::StreamRc::new(core, "TimeTracker", stream_properties())
        .map_err(describe)?;

    // The listener has to stay alive for the callback to keep firing, so it
    // is bound here for as long as the loop runs.
    let _listener = stream
        .add_local_listener_with_user_data(Playback::silent())
        .process({
            let shared = shared.clone();

            move |stream, playback| process(stream, playback, &shared)
        })
        .register()
        .map_err(describe)?;

    shared.set_status(Status::Ready);

    // `connected` tracks whether the stream is on the graph; `idle_ticks`
    // counts how long it has had nothing to do.
    let connected = Rc::new(RefCell::new(false));
    let idle_ticks = Rc::new(RefCell::new(0_u32));

    let _receiver = receiver.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        let stream = stream.clone();
        let shared = shared.clone();
        let connected = connected.clone();
        let idle_ticks = idle_ticks.clone();

        move |command| {
            *idle_ticks.borrow_mut() = 0;

            match command {
                Command::Play(play) => {
                    if !*connected.borrow() {
                        if let Err(error) = connect(&stream) {
                            let error = describe(error);
                            log::warn!("could not connect the audio stream: {error}");
                            shared.set_status(Status::Unavailable(error));
                            return;
                        }

                        *connected.borrow_mut() = true;
                    }

                    shared.finished.store(false, Ordering::Release);
                    shared.submit(Command::Play(play));

                    if let Err(error) = stream.set_active(true) {
                        log::warn!("could not activate the audio stream: {error}");
                    }
                }
                Command::Stop => shared.submit(Command::Stop),
                Command::Shutdown => mainloop.quit(),
            }
        }
    });

    // Housekeeping runs here, on the loop thread, precisely because none of
    // it would be safe to do from the realtime callback.
    let timer = mainloop.loop_().add_timer({
        let mainloop = mainloop.clone();
        let stream = stream.clone();
        let shared = shared.clone();
        let connected = connected.clone();
        let idle_ticks = idle_ticks.clone();

        move |_| {
            // Free whatever the realtime thread handed back.
            shared.collect_retired();

            if shared.finished.swap(false, Ordering::AcqRel) {
                // Playback ran out; park the stream and let the realtime
                // callback release its buffer.
                shared.submit(Command::Stop);

                if let Err(error) = stream.set_active(false) {
                    log::debug!("could not deactivate the audio stream: {error}");
                }
            }

            let ticks = {
                let mut ticks = idle_ticks.borrow_mut();
                *ticks += 1;
                *ticks
            };

            let idle_for = POLL * ticks;

            if *connected.borrow() && idle_for >= STREAM_LINGER {
                if let Err(error) = stream.disconnect() {
                    log::debug!("could not disconnect the audio stream: {error}");
                }

                *connected.borrow_mut() = false;
            }

            if idle_for >= THREAD_LINGER {
                mainloop.quit();
            }
        }
    });

    timer
        .update_timer(Some(POLL), Some(POLL))
        .into_result()
        .map_err(describe)?;

    mainloop.run();

    Ok(())
}

/// The properties that decide how the session manager treats us.
///
/// `MEDIA_ROLE = Notification` is what lets it route the alarm to the right
/// sink and duck whatever else is playing, and `APP_NAME` is what gives
/// TimeTracker its own slider in the system mixer.
fn stream_properties() -> pw::properties::PropertiesBox {
    pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Notification",
        *pw::keys::APP_NAME => "TimeTracker",
        *pw::keys::APP_ID => "dev.timetracker.TimeTracker",
        *pw::keys::NODE_NAME => "timetracker",
        *pw::keys::AUDIO_CHANNELS => "2",
    }
}

/// The realtime callback.
///
/// This runs on PipeWire's realtime thread, so it must not allocate, block,
/// log or do I/O. It only copies from a buffer that was rendered ahead of
/// time, reads the gain from an atomic, and uses `try_lock` so it can never
/// wait on the loop thread.
fn process(stream: &pw::stream::Stream, playback: &mut Playback, shared: &Shared) {
    if let Ok(mut pending) = shared.pending.try_lock() {
        match pending.take() {
            Some(Command::Play(play)) => {
                if let Some(retired) = playback.start(play) {
                    shared.retire(retired);
                }
            }
            Some(Command::Stop) => {
                if let Some(retired) = playback.stop() {
                    shared.retire(retired);
                }
            }
            _ => {}
        }
    }

    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };

    let gain = shared.gain();
    let datas = buffer.datas_mut();

    let Some(data) = datas.first_mut() else {
        return;
    };

    let mut frames_written = 0;

    if let Some(slice) = data.data() {
        let frames = slice.len() / STRIDE;

        for frame_index in 0..frames {
            // Past the end of the sound we keep writing silence rather than
            // stopping short, so the sink never sees a partial buffer.
            let frame = playback.next_frame().unwrap_or([0.0; CHANNELS]);

            for (channel, sample) in frame.iter().enumerate() {
                let start = frame_index * STRIDE + channel * SAMPLE_SIZE;

                slice[start..start + SAMPLE_SIZE]
                    .copy_from_slice(&(sample * gain).to_le_bytes());
            }
        }

        frames_written = frames;
    }

    if !playback.is_playing() {
        shared.finished.store(true, Ordering::Release);
    }

    let chunk = data.chunk_mut();
    *chunk.offset_mut() = 0;
    *chunk.stride_mut() = STRIDE as _;
    *chunk.size_mut() = (STRIDE * frames_written) as _;
}

fn connect(stream: &pw::stream::StreamRc) -> Result<(), pw::Error> {
    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(SAMPLE_RATE);
    audio_info.set_channels(CHANNELS as u32);

    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    audio_info.set_position(position);

    let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: audio_info.into(),
        }),
    )
    .map_err(|_| pw::Error::CreationFailed)?
    .0
    .into_inner();

    let mut params = [Pod::from_bytes(&values).ok_or(pw::Error::CreationFailed)?];

    stream.connect(
        spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )
}

fn describe(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(frames: usize) -> Pcm {
        vec![1.0; frames * CHANNELS].into()
    }

    fn play(pcm: Pcm, repeats: u8) -> Play {
        Play {
            pcm,
            repeats,
            fade_in: Duration::ZERO,
        }
    }

    #[test]
    fn a_silent_playback_yields_nothing() {
        let mut playback = Playback::silent();

        assert!(playback.next_frame().is_none());
        assert!(!playback.is_playing());
    }

    #[test]
    fn playback_walks_the_buffer_once_by_default() {
        let mut playback = Playback::silent();
        playback.start(play(pcm(3), 1));

        assert!(playback.next_frame().is_some());
        assert!(playback.next_frame().is_some());
        assert!(playback.next_frame().is_some());
        assert!(playback.next_frame().is_none());
        assert!(!playback.is_playing());
    }

    #[test]
    fn repeats_replay_the_buffer() {
        let mut playback = Playback::silent();
        playback.start(play(pcm(2), 3));

        // Three plays of two frames each.
        for _ in 0..6 {
            assert!(playback.next_frame().is_some());
        }

        assert!(playback.next_frame().is_none());
    }

    #[test]
    fn zero_repeats_loops_until_stopped() {
        let mut playback = Playback::silent();
        playback.start(play(pcm(2), 0));

        for _ in 0..1_000 {
            assert!(playback.next_frame().is_some());
        }

        playback.stop();
        assert!(playback.next_frame().is_none());
    }

    #[test]
    fn the_buffer_is_handed_back_rather_than_dropped_in_the_callback() {
        let mut playback = Playback::silent();

        assert!(playback.start(play(pcm(10), 1)).is_none());
        // Starting a second sound returns the first buffer for the loop
        // thread to free.
        assert!(playback.start(play(pcm(10), 1)).is_some());
        assert!(playback.stop().is_some());
    }

    #[test]
    fn the_fade_in_ramps_from_silence_to_full() {
        let mut playback = Playback::silent();
        playback.start(Play {
            pcm: pcm(SAMPLE_RATE as usize),
            repeats: 1,
            fade_in: Duration::from_millis(100),
        });

        assert_eq!(playback.next_frame().unwrap()[0], 0.0);

        // Halfway through a 100ms fade at 48kHz.
        for _ in 0..2_400 {
            let _ = playback.next_frame();
        }

        let middle = playback.next_frame().unwrap()[0];
        assert!((middle - 0.5).abs() < 0.05, "expected ~0.5, got {middle}");

        // Well past the fade.
        for _ in 0..5_000 {
            let _ = playback.next_frame();
        }

        assert_eq!(playback.next_frame().unwrap()[0], 1.0);
    }

    #[test]
    fn a_frame_is_a_sample_per_channel() {
        assert_eq!(STRIDE, SAMPLE_SIZE * CHANNELS);
    }
}
