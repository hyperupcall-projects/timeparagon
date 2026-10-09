//! State, messages and the update loop.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::keyboard::{self, Key};
use iced::{Element, Size, Subscription, Task, Theme, window};

use chrono::{DateTime, Local};

use crate::audio::{self, Audio, Pcm, Tone};
use crate::calendar::{self, Connection, SyncState};
use crate::config::{self, Config};
use crate::engine::{TIMER_TICK, Timer};
use crate::theme::Catalog;
use crate::{alarm, blocking, notify, theme, tracks, ui};

/// The smallest window we are willing to be asked to draw.
///
/// The compact layout is built for 200px; this leaves a little room below
/// that rather than letting a tiling compositor squeeze it into nonsense.
pub const MIN_SIZE: Size = Size::new(180.0, 240.0);

/// How often the flash overlay redraws while it is animating.
///
/// This is a timer rather than `window::frames` because iced only redraws
/// when something changes, so an animation has to keep producing messages to
/// stay in motion. It only runs for the second or two the flash is up.
const FLASH_FRAME: Duration = Duration::from_millis(33);

/// How often the settings pane refreshes the audio status note, so that
/// "PipeWire unavailable" appears without needing a mouse wiggle.
const STATUS_POLL: Duration = Duration::from_millis(500);

/// How often the current-time needle is redrawn on the calendar.
const NEEDLE_TICK: Duration = Duration::from_secs(15);

/// How close, in minutes, to an event's start you have to be for a freshly
/// started timer to take the event's length as its target.
const NEAR_EVENT_START: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Calendar,
    Settings,
    /// The pane for choosing the target time.
    SetTime,
}

/// A flash overlay that is currently on screen.
pub struct Flash {
    pub window: window::Id,
    pub started: Instant,
    pub settings: config::Flash,
}

impl Flash {
    /// How opaque the overlay should be right now, or `None` once it is done.
    ///
    /// Each pulse is a full raised-cosine fade in and out, so the flash never
    /// snaps on or off.
    pub fn alpha(&self, now: Instant) -> Option<f32> {
        let elapsed = now.saturating_duration_since(self.started);

        if elapsed >= self.settings.total_duration() {
            return None;
        }

        let period = self.settings.pulse_period().as_secs_f32();
        let phase = (elapsed.as_secs_f32() % period) / period;
        let envelope = 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos();

        Some(envelope * self.settings.peak())
    }
}

pub struct State {
    pub config: Config,
    pub catalog: Catalog,

    pub timer: Timer,
    /// The clock reading used for this frame, so every widget agrees.
    pub now: Instant,

    /// Whether the clock is showing inside the calendar. The clock has no
    /// section of its own any more: it lives at the foot of the day view and
    /// is put away behind "Hide timer".
    pub timer_open: bool,
    /// Every stretch of time the timer has been running, kept on disk.
    pub recorder: tracks::Recorder,

    pub screen: Screen,
    pub main_window: Option<window::Id>,
    pub flash: Option<Flash>,
    /// True while a sound or flash is still asking to be dismissed.
    pub alarm_active: bool,

    pub audio: Audio,
    /// The last thing that went wrong with a sound, shown in settings.
    pub audio_error: Option<String>,

    /// Set while a config write is already queued, so a slider drag does not
    /// schedule one save per pixel.
    save_queued: bool,

    pub calendar: calendar::Store,
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick(Instant),
    FlashFrame(Instant),

    StartPause,
    Reset,
    PresetSelected(Duration),
    /// Adjust the target time by a number of seconds, positive or negative.
    DurationAdjusted(i64),
    /// Clear the target so the clock counts up without an alarm.
    TargetCleared,
    /// Unfold the clock inside the calendar and set it going.
    TimerOpened,
    /// Fold the clock away again; a timer that is running keeps going.
    TimerClosed,

    ScreenSelected(Screen),
    Dismiss,

    ThemeSelected(String),
    FollowSystemToggled(bool),
    AlwaysOnTopToggled(bool),

    NotificationToggled(bool),
    NotificationSummaryChanged(String),
    NotificationUrgencySelected(config::Urgency),
    RaiseWindowToggled(bool),

    SoundToggled(bool),
    ToneSelected(Tone),
    VolumeChanged(u8),
    RepeatsChanged(u8),
    SoundFileRequested,
    SoundFileSelected(Option<PathBuf>),
    PreviewRequested,

    FlashToggled(bool),
    FlashOpacityChanged(f32),
    FlashPulsesChanged(u8),
    FlashPassthroughToggled(bool),
    FinalCountdownChanged(u8),

    /// A sound finished rendering and is ready to hand to PipeWire.
    SoundRendered(Result<Pcm, String>, PlaySpec),
    NotificationPosted(Result<(), String>),
    ConfigSaved(Result<(), String>),
    SaveRequested,
    TracksSaved(Result<(), String>),
    /// Rewrite the track the running timer has open, so a crash cannot lose
    /// more than the time since the last write.
    TracksFlushRequested,
    AudioStatusPolled,

    WindowOpened(window::Id),
    /// The monitor size came back, so the window can open at its default width.
    MonitorSized(window::Id, Option<Size>),
    WindowClosed(window::Id),
    WindowResized(window::Id, Size),
    FlashFinished,

    KeyPressed(Key, keyboard::Modifiers),
    /// An event we deliberately do not act on, such as a key release or a
    /// press that only needs to be swallowed.
    Ignored,

    CalendarMenuToggled,
    CalendarToggled(String, bool),
    CalendarSyncRequested,
    CalendarSynced(Result<calendar::Snapshot, String>),
    CalendarTick,
    OpenGoogleCalendar,
    EventOpened(String),
    EventClosed,
    OpenEventLink(String),
    /// Tick a Google Task off, or put it back.
    TaskToggled(String),
    TaskCompletionSaved(String, bool, Result<(), String>),
    /// Show or hide Google Tasks in the day view.
    ShowTasksToggled(bool),
    /// Open or close the color picker for a Google Tasks list.
    TaskColorEditorToggled(String),
    /// Give a Google Tasks list a custom color, or `None` to reset it.
    TaskColorPicked(String, Option<iced::Color>),
    GoogleClientIdChanged(String),
    GoogleClientSecretChanged(String),
    GoogleConnectRequested,
    GoogleConnected(Result<calendar::oauth::Tokens, String>),
    GoogleDisconnectRequested,
    CalendarSyncMinutesChanged(u32),
    /// The hour the calendar day ends, so late-night events stay with the
    /// evening they belong to.
    CalendarDayEndChanged(u32),
    /// Multiplies the calendar hour height. Above 1 spreads the hours apart.
    ///
    /// `anchor` is the pixel row under the pointer, measured from the top of
    /// the day grid, so the zoom can keep that row still under the mouse.
    CalendarZoomed { factor: f32, anchor: f32 },
    /// Move the day in view by a number of days, positive or negative.
    CalendarDayShift(i64),
    /// Jump the day in view to a specific date, from the month picker.
    CalendarDaySelected(chrono::NaiveDate),
    /// Open or close the month picker.
    CalendarMonthToggled,
    /// Move the month picker by a number of months, positive or negative.
    CalendarMonthShift(i32),
}

/// The parts of a play request that survive the trip through rendering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaySpec {
    pub gain: f32,
    pub repeats: u8,
    pub fade_in: Duration,
}

impl State {
    /// Builds a state from settings that are already in hand.
    ///
    /// Separate from [`boot`] so tests can assemble a state without reading
    /// or writing anything in the user's config directory.
    pub fn new(config: Config, catalog: Catalog) -> Self {
        let day_end_hour = config.calendar.day_start_hour();

        Self {
            timer: Timer::new(config.timer.last_duration()),
            now: Instant::now(),
            timer_open: false,
            recorder: tracks::Recorder::default(),
            screen: Screen::Calendar,
            main_window: None,
            flash: None,
            alarm_active: false,
            audio: Audio::new(),
            audio_error: None,
            save_queued: false,
            catalog,
            config,
            calendar: calendar::Store::new(day_end_hour),
        }
    }
}

pub fn boot() -> (State, Task<Message>) {
    let mut state = State::new(Config::load(), Catalog::load());

    // Days are kept under the XDG cache directory as well as in memory, so a
    // restart opens on what the last run saw rather than an empty grid. This
    // is deliberately not part of `State::new`: a state built by hand in a
    // test must not read or rewrite the user's cache.
    state.calendar.cache_days_in(config::cache_days_dir());

    // The same goes for the time tracks: the file is the user's own, so only
    // a real boot is allowed to pick it up.
    state.recorder.keep_in(tracks::path());

    // A daemon opens no window by itself, which is what lets the flash
    // overlay be a peer window rather than a child of the clock.
    let (_id, open) = window::open(window::Settings {
        size: Size::new(state.config.window.width, state.config.window.height),
        min_size: Some(MIN_SIZE),
        level: level(state.config.window.always_on_top),
        exit_on_close_request: true,
        ..Default::default()
    });

    (state, open.map(Message::WindowOpened))
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Tick(now) => {
            state.now = now;

            if state.timer.poll(now) {
                return fire_alarm(state);
            }

            Task::none()
        }

        Message::FlashFrame(now) => {
            state.now = now;

            match &state.flash {
                Some(flash) if flash.alpha(now).is_none() => Task::done(Message::FlashFinished),
                _ => Task::none(),
            }
        }

        Message::StartPause => {
            state.now = Instant::now();

            if state.alarm_active {
                return dismiss(state);
            }

            state.timer.toggle(state.now);
            sync_tracks(state)
        }

        Message::Reset => {
            state.timer.reset();
            let tracks = sync_tracks(state);

            if state.alarm_active {
                return Task::batch([tracks, dismiss(state)]);
            }

            tracks
        }

        Message::PresetSelected(duration) => {
            state.timer.set_target(Some(duration));
            state.config.timer.last_duration_secs = duration.as_secs();
            Task::batch([queue_save(state), sync_tracks(state)])
        }

        Message::DurationAdjusted(delta) => {
            let current = state.timer.target().map_or(0, |target| target.as_secs()) as i64;
            let adjusted = (current + delta).clamp(0, 24 * 60 * 60) as u64;

            state.timer.set_target(if adjusted == 0 {
                None
            } else {
                Some(Duration::from_secs(adjusted))
            });
            state.config.timer.last_duration_secs = adjusted;
            Task::batch([queue_save(state), sync_tracks(state)])
        }

        Message::TargetCleared => {
            state.timer.set_target(None);
            state.config.timer.last_duration_secs = 0;
            Task::batch([queue_save(state), sync_tracks(state)])
        }

        Message::TimerOpened => {
            state.now = Instant::now();
            state.timer_open = true;

            // A timer set going from scratch takes the length of whatever is
            // about to happen, so opening the clock while a standup is
            // starting times the standup rather than the last thing armed.
            if state.timer.is_idle() {
                let wall = Local::now();
                let target = nearby_span(state, wall).or_else(|| state.timer.target());
                state.timer.set_target(target);
            }

            if !state.timer.is_running() {
                state.timer.start(state.now);
            }

            sync_tracks(state)
        }

        Message::TimerClosed => {
            state.timer_open = false;
            Task::none()
        }

        Message::ScreenSelected(screen) => {
            // Settings and Set Time sit on top of the calendar rather than
            // replacing it, so coming back out of one leaves the day grid
            // exactly where the user scrolled it to.
            state.calendar.menu_open = false;
            state.screen = screen;
            Task::none()
        }

        Message::Dismiss => dismiss(state),

        Message::ThemeSelected(name) => {
            state.config.appearance.theme = name;
            state.config.appearance.follow_system = false;
            queue_save(state)
        }

        Message::FollowSystemToggled(follow) => {
            state.config.appearance.follow_system = follow;
            queue_save(state)
        }

        Message::AlwaysOnTopToggled(on_top) => {
            state.config.window.always_on_top = on_top;

            let apply = state
                .main_window
                .map(|id| window::set_level(id, level(on_top)))
                .unwrap_or(Task::none());

            Task::batch([apply, queue_save(state)])
        }

        Message::NotificationToggled(enabled) => {
            state.config.alarm.notification.enabled = enabled;
            queue_save(state)
        }

        Message::NotificationSummaryChanged(summary) => {
            state.config.alarm.notification.summary = summary;
            queue_save(state)
        }

        Message::NotificationUrgencySelected(urgency) => {
            state.config.alarm.notification.urgency = urgency;
            queue_save(state)
        }

        Message::RaiseWindowToggled(raise) => {
            state.config.alarm.raise_window = raise;
            queue_save(state)
        }

        Message::SoundToggled(enabled) => {
            state.config.alarm.sound.enabled = enabled;
            queue_save(state)
        }

        Message::ToneSelected(tone) => {
            state.config.alarm.sound.choice = config::SoundChoice::Builtin(tone);
            state.audio_error = None;
            queue_save(state)
        }

        Message::VolumeChanged(volume) => {
            state.config.alarm.sound.volume = volume;
            // Applied immediately so a preview that is already playing
            // follows the slider.
            state.audio.set_gain(state.config.alarm.sound.gain());
            queue_save(state)
        }

        Message::RepeatsChanged(repeats) => {
            state.config.alarm.sound.repeats = repeats;
            queue_save(state)
        }

        Message::SoundFileRequested => blocking::run(|| {
            rfd::FileDialog::new()
                .set_title("Choose an alarm sound")
                .add_filter("Audio", &["wav", "ogg", "oga", "flac", "mp3", "m4a", "aac"])
                .pick_file()
        })
        .map(Message::SoundFileSelected),

        Message::SoundFileSelected(Some(path)) => {
            state.config.alarm.sound.choice = config::SoundChoice::Custom { file: path };
            state.audio_error = None;

            Task::batch([queue_save(state), Task::done(Message::PreviewRequested)])
        }

        Message::SoundFileSelected(None) => Task::none(),

        Message::PreviewRequested => {
            let actions = alarm::preview(&state.config.alarm.sound);
            perform(state, actions, state.timer.target().unwrap_or_default())
        }

        Message::FlashToggled(enabled) => {
            state.config.alarm.flash.enabled = enabled;
            queue_save(state)
        }

        Message::FlashOpacityChanged(opacity) => {
            state.config.alarm.flash.peak_opacity = opacity;
            queue_save(state)
        }

        Message::FlashPulsesChanged(pulses) => {
            state.config.alarm.flash.pulses = pulses;
            queue_save(state)
        }

        Message::FlashPassthroughToggled(passthrough) => {
            state.config.alarm.flash.passthrough = passthrough;
            queue_save(state)
        }

        Message::FinalCountdownChanged(seconds) => {
            state.config.alarm.flash.final_countdown_secs = seconds;
            queue_save(state)
        }

        Message::SoundRendered(Ok(pcm), spec) => {
            state.audio_error = None;
            state.audio.set_gain(spec.gain);
            state.audio.play(audio::Play {
                pcm,
                repeats: spec.repeats,
                fade_in: spec.fade_in,
            });

            Task::none()
        }

        Message::SoundRendered(Err(error), _) => {
            log::warn!("could not prepare the alarm sound: {error}");
            state.audio_error = Some(error);
            Task::none()
        }

        Message::NotificationPosted(Err(error)) => {
            log::warn!("could not post the notification: {error}");
            Task::none()
        }

        Message::NotificationPosted(Ok(())) => Task::none(),

        Message::ConfigSaved(result) => {
            if let Err(error) = result {
                log::warn!("could not save the config: {error}");
            }

            Task::none()
        }

        Message::SaveRequested => {
            state.save_queued = false;
            let config = state.config.clone();

            blocking::run(move || config.save().map_err(|error| error.to_string()))
                .map(Message::ConfigSaved)
        }

        Message::TracksFlushRequested => {
            if state.recorder.flush(Local::now()) {
                save_tracks(state)
            } else {
                Task::none()
            }
        }

        Message::TracksSaved(result) => {
            if let Err(error) = result {
                log::warn!("could not save the time tracks: {error}");
            }

            Task::none()
        }

        Message::AudioStatusPolled => {
            if let audio::Status::Unavailable(reason) = state.audio.status() {
                state.audio_error = Some(reason);
            }

            Task::none()
        }

        Message::WindowOpened(id) => {
            state.main_window = Some(id);

            // The window opens at whatever size we last saved; once the
            // monitor size is known it is widened to the default fraction of
            // the screen. The height stays as the user left it.
            let measure = window::monitor_size(id).map(move |size| Message::MonitorSized(id, size));

            // The calendar is the front page now, so it opens on the hour the
            // needle is at rather than on midnight.
            let scroll = ui::calendar::scroll_to_now(
                state.calendar.hour_height,
                state.config.calendar.day_end_minutes(),
            );

            let sync = if state.calendar.is_signed_in() {
                sync_calendar(state)
            } else {
                Task::none()
            };

            Task::batch([measure, sync, scroll])
        }

        Message::MonitorSized(id, size) => {
            if state.main_window != Some(id) {
                return Task::none();
            }

            let Some(size) = size else {
                return Task::none();
            };

            let width = launch_width(size.width);
            let height = state.config.window.height;
            state.config.window.width = width;

            Task::batch([
                window::resize(id, Size::new(width, height)),
                queue_save(state),
            ])
        }

        Message::WindowResized(id, size) => {
            if state.main_window != Some(id) {
                return Task::none();
            }

            log::debug!("window is now {}x{}", size.width, size.height);

            state.config.window.width = size.width;
            state.config.window.height = size.height;

            queue_save(state)
        }

        Message::WindowClosed(id) => {
            if state.main_window == Some(id) {
                // Closing the clock quits, even though a daemon would
                // otherwise keep running with no windows left.
                return iced::exit();
            }

            if state.flash.as_ref().is_some_and(|flash| flash.window == id) {
                state.flash = None;
            }

            Task::none()
        }

        Message::FlashFinished => {
            let Some(flash) = state.flash.take() else {
                return Task::none();
            };

            window::close(flash.window)
        }

        Message::KeyPressed(key, modifiers) => on_key(state, key, modifiers),

        Message::Ignored => Task::none(),

        Message::CalendarMenuToggled => {
            state.calendar.menu_open = !state.calendar.menu_open;
            // Dismissing the menu also puts away any open color picker, so it
            // does not pop back up the next time the menu is opened.
            state.calendar.color_editor = None;
            Task::none()
        }

        Message::CalendarToggled(id, enabled) => {
            state.config.calendar.set_calendar_enabled(&id, enabled);
            queue_save(state)
        }

        Message::CalendarSyncRequested => sync_calendar(state),

        Message::CalendarSynced(Ok(snapshot)) => {
            // The user may have stepped to another day while this was in
            // flight. The events are for the day they left, so drop them and
            // fetch the day now in view rather than snapping the view back.
            if snapshot.day != state.calendar.day {
                state.calendar.sync = SyncState::Idle;
                return sync_calendar(state);
            }

            state.calendar.apply_snapshot(snapshot);
            state.calendar.apply_task_colors(&state.config.calendar);
            Task::none()
        }

        Message::CalendarSynced(Err(error)) => {
            log::warn!("calendar sync failed: {error}");
            state.calendar.sync = SyncState::Idle;
            if matches!(error.as_str(), "Not connected to Google Calendar") {
                state.calendar.connection = Connection::SignedOut;
            }
            state.calendar.error = Some(error);
            Task::none()
        }

        Message::CalendarTick => refresh_day(state),

        Message::OpenGoogleCalendar => {
            if let Err(error) = calendar::oauth::open_in_browser(calendar::WEB_URL) {
                state.calendar.error = Some(error);
            }
            Task::none()
        }

        Message::EventOpened(id) => {
            state.calendar.menu_open = false;
            state.calendar.open_event = Some(id);
            // Recorded during this click, before the message is handled.
            state.calendar.details_below = ui::calendar::click_was_in_the_top_half();
            Task::none()
        }

        Message::EventClosed => {
            state.calendar.open_event = None;
            Task::none()
        }

        Message::OpenEventLink(url) => {
            if let Err(error) = calendar::oauth::open_in_browser(&url) {
                state.calendar.error = Some(error);
            }
            Task::none()
        }

        Message::TaskToggled(id) => {
            let Some((calendar_id, previous)) = state
                .calendar
                .event(&id)
                .filter(|event| event.is_task())
                .map(|event| (event.calendar_id.clone(), event.completed))
            else {
                return Task::none();
            };

            // Update the chip straight away, then put it back if Google
            // refuses the write.
            let completed = !previous;
            if let Some(event) = state.calendar.event_mut(&id) {
                event.completed = completed;
            }

            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            let request_id = id.clone();
            state.calendar.error = None;

            blocking::run(move || {
                calendar::set_task_completed(
                    &client_id,
                    &client_secret,
                    &calendar_id,
                    &request_id,
                    completed,
                )
            })
            .map(move |result| Message::TaskCompletionSaved(id.clone(), previous, result))
        }

        Message::TaskCompletionSaved(_, _, Ok(())) => Task::none(),

        Message::TaskCompletionSaved(id, previous, Err(error)) => {
            if let Some(event) = state.calendar.event_mut(&id) {
                event.completed = previous;
            }

            log::warn!("could not update the task: {error}");
            state.calendar.error = Some(error);
            Task::none()
        }

        Message::ShowTasksToggled(show) => {
            state.config.calendar.show_tasks = show;
            queue_save(state)
        }

        Message::TaskColorEditorToggled(id) => {
            state.calendar.color_editor =
                (state.calendar.color_editor.as_deref() != Some(id.as_str())).then_some(id);
            Task::none()
        }

        Message::TaskColorPicked(id, color) => {
            state.config.calendar.set_task_color(&id, color);
            state.calendar.apply_task_colors(&state.config.calendar);
            state.calendar.color_editor = None;
            queue_save(state)
        }

        Message::GoogleClientIdChanged(id) => {
            state.config.calendar.client_id = id;
            queue_save(state)
        }

        Message::GoogleClientSecretChanged(secret) => {
            state.config.calendar.client_secret = secret;
            queue_save(state)
        }

        Message::GoogleConnectRequested => {
            if state.calendar.is_busy() {
                return Task::none();
            }

            let client_id = state.config.calendar.client_id.clone();
            let client_secret = state.config.calendar.client_secret.clone();
            state.calendar.sync = SyncState::Connecting;
            state.calendar.error = None;

            blocking::run(move || calendar::connect(client_id, client_secret))
                .map(Message::GoogleConnected)
        }

        Message::GoogleConnected(Ok(_tokens)) => {
            state.calendar.connection = Connection::SignedIn;
            state.calendar.sync = SyncState::Idle;
            state.calendar.error = None;
            sync_calendar(state)
        }

        Message::GoogleConnected(Err(error)) => {
            log::warn!("Google sign-in failed: {error}");
            state.calendar.sync = SyncState::Idle;
            state.calendar.connection = Connection::SignedOut;
            state.calendar.error = Some(error);
            Task::none()
        }

        Message::GoogleDisconnectRequested => {
            calendar::disconnect();
            state.calendar = calendar::Store::new(state.config.calendar.day_start_hour());
            // The reset takes the store with it, so the directory is put
            // straight back: days keep being written across a disconnect.
            state.calendar.cache_days_in(config::cache_days_dir());
            state.calendar.connection = Connection::SignedOut;
            Task::none()
        }

        Message::CalendarSyncMinutesChanged(minutes) => {
            state.config.calendar.sync_minutes = minutes.clamp(
                config::CalendarSettings::MIN_SYNC_MINUTES,
                config::CalendarSettings::MAX_SYNC_MINUTES,
            );
            queue_save(state)
        }

        Message::CalendarDayEndChanged(hour) => {
            state.config.calendar.day_end_hour =
                hour.min(config::CalendarSettings::MAX_DAY_END_HOUR);
            // The day's window moved, so the day in view and the events that
            // belong to it may have changed too. This also puts the view back
            // on today if the user had navigated away.
            let day = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
            state.calendar.set_day(day, day);
            // The day's window moved, so the cached days no longer describe
            // the days they are filed under.
            state.calendar.forget_cached_days();
            let save = queue_save(state);
            Task::batch([save, sync_calendar(state)])
        }

        Message::CalendarZoomed { factor, anchor } => {
            let before = state.calendar.hour_height;
            state.calendar.zoom(factor);
            let after = state.calendar.hour_height;

            if after == before {
                return Task::none();
            }

            // Keep the row under the pointer fixed. The grid grows by `ratio`,
            // so the anchored row slides down by `anchor * (ratio - 1)` and the
            // scroll has to follow it by the same amount.
            let delta = crate::ui::calendar::zoom_scroll_delta(anchor, before, after);
            crate::ui::calendar::scroll_by(delta)
        }

        Message::CalendarDayShift(days) => {
            let today = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
            let day = state.calendar.day + chrono::Duration::days(days);
            state.calendar.set_day(day, today);
            state.calendar.menu_open = false;
            state.calendar.month_open = false;
            sync_calendar(state)
        }

        Message::CalendarDaySelected(day) => {
            let today = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
            state.calendar.set_day(day, today);
            state.calendar.month_open = false;
            sync_calendar(state)
        }

        Message::CalendarMonthToggled => {
            state.calendar.month_open = !state.calendar.month_open;
            // The picker opens on the month of the day in view, and the menu
            // gets out of its way.
            if state.calendar.month_open {
                state.calendar.month = calendar::first_of_month(state.calendar.day);
                state.calendar.menu_open = false;
            }
            Task::none()
        }

        Message::CalendarMonthShift(months) => {
            let step = chrono::Months::new(months.unsigned_abs());
            let shifted = if months >= 0 {
                state.calendar.month.checked_add_months(step)
            } else {
                state.calendar.month.checked_sub_months(step)
            };
            if let Some(month) = shifted {
                state.calendar.month = month;
            }
            Task::none()
        }
    }
}

pub fn view(state: &State, window: window::Id) -> Element<'_, Message> {
    if state.flash.as_ref().is_some_and(|flash| flash.window == window) {
        return ui::flash::view(state);
    }

    ui::view(state)
}

/// The theme for a given window.
///
/// Returning `None` is how iced is told to follow the system color scheme.
pub fn active_theme(state: &State, window: window::Id) -> Option<Theme> {
    if state.flash.as_ref().is_some_and(|flash| flash.window == window) {
        return Some(theme::FLASH.clone());
    }

    if state.config.appearance.follow_system {
        return None;
    }

    Some(state.catalog.resolve(&state.config.appearance.theme).theme.clone())
}

pub fn title(state: &State, window: window::Id) -> String {
    if state.flash.as_ref().is_some_and(|flash| flash.window == window) {
        return "TimeTracker".to_owned();
    }

    if state.timer.has_fired() {
        return "Time's up - TimeTracker".to_owned();
    }

    if state.timer.is_running() {
        return format!("{} - TimeTracker", ui::readout(state));
    }

    "TimeTracker".to_owned()
}

pub fn subscription(state: &State) -> Subscription<Message> {
    let mut subscriptions = vec![
        window::close_events().map(Message::WindowClosed),
        window::resize_events().map(|(id, size)| Message::WindowResized(id, size)),
        keyboard::listen().map(map_key),
    ];

    // The clock only ticks when it is actually running, which is what keeps
    // an idle TimeTracker at zero CPU. The open track is rewritten on the
    // same cadence as the clock's own running, so a crash costs at most one
    // flush even if the timer never pauses.
    if state.timer.is_running() {
        subscriptions.push(iced::time::every(TIMER_TICK).map(Message::Tick));
        subscriptions.push(
            iced::time::every(tracks::FLUSH_EVERY).map(|_| Message::TracksFlushRequested),
        );
    }

    // The needle has to move even when the clock is idle, but only while
    // the day view is actually on screen.
    if state.screen == Screen::Calendar {
        subscriptions.push(iced::time::every(NEEDLE_TICK).map(|_| Message::CalendarTick));
    }

    if state.calendar.is_signed_in() {
        subscriptions.push(
            iced::time::every(state.config.calendar.sync_every())
                .map(|_| Message::CalendarSyncRequested),
        );
    }

    if state.flash.is_some() {
        subscriptions.push(iced::time::every(FLASH_FRAME).map(Message::FlashFrame));
    }

    if state.screen == Screen::Settings {
        subscriptions
            .push(iced::time::every(STATUS_POLL).map(|_| Message::AudioStatusPolled));
    }

    // The debounce itself: the write lands once the interval elapses, and
    // further edits in the meantime ride along on the same pending save.
    if state.save_queued {
        subscriptions
            .push(iced::time::every(config::SAVE_DEBOUNCE).map(|_| Message::SaveRequested));
    }

    Subscription::batch(subscriptions)
}

fn map_key(event: keyboard::Event) -> Message {
    match event {
        keyboard::Event::KeyPressed { key, modifiers, .. } => {
            Message::KeyPressed(key, modifiers)
        }
        _ => Message::Ignored,
    }
}

fn on_key(state: &mut State, key: Key, modifiers: keyboard::Modifiers) -> Task<Message> {
    use keyboard::key::Named;

    if modifiers.command() {
        return match key.as_ref() {
            Key::Character(",") => Task::done(Message::ScreenSelected(Screen::Settings)),
            _ => Task::none(),
        };
    }

    match key.as_ref() {
        Key::Named(Named::Space) => Task::done(Message::StartPause),
        Key::Named(Named::Escape) => {
            if state.calendar.open_event.is_some() {
                state.calendar.open_event = None;
                Task::none()
            } else if state.calendar.menu_open {
                state.calendar.menu_open = false;
                Task::none()
            } else if state.alarm_active {
                Task::done(Message::Dismiss)
            } else if state.screen != Screen::Calendar {
                Task::done(Message::ScreenSelected(Screen::Calendar))
            } else {
                Task::none()
            }
        }
        Key::Character("r") => Task::done(Message::Reset),
        _ => Task::none(),
    }
}

/// Runs the whole alarm: sound, notification, flash and raising the window.
fn fire_alarm(state: &mut State) -> Task<Message> {
    let actions = alarm::on_finished(&state.config.alarm);
    let target = state.timer.target().unwrap_or_default();

    state.alarm_active = actions.is_dismissable();

    perform(state, actions, target)
}

fn perform(state: &mut State, actions: alarm::Actions, duration: Duration) -> Task<Message> {
    let mut tasks = Vec::new();

    if let Some(sound) = actions.sound {
        let spec = PlaySpec {
            gain: sound.gain,
            repeats: sound.repeats,
            fade_in: sound.fade_in,
        };
        let choice = sound.choice;

        // Rendering happens off the UI thread: synthesis is quick but
        // decoding a file the user picked is not necessarily.
        tasks.push(
            blocking::run(move || audio::render(&choice))
                .map(move |result| Message::SoundRendered(result, spec)),
        );
    }

    if let Some(notification) = actions.notification {
        tasks.push(
            blocking::run(move || notify::show(&notification, duration))
                .map(Message::NotificationPosted),
        );
    }

    if let Some(settings) = actions.flash {
        tasks.push(open_flash(state, settings));
    }

    if actions.raise {
        if let Some(id) = state.main_window {
            tasks.push(window::gain_focus(id));
        }
    }

    Task::batch(tasks)
}

fn open_flash(state: &mut State, settings: config::Flash) -> Task<Message> {
    // Replace any flash still on screen rather than stacking a second one.
    let close_existing = state
        .flash
        .take()
        .map(|flash| window::close::<Message>(flash.window))
        .unwrap_or(Task::none());

    let passthrough = settings.passthrough;

    let (id, open) = window::open(window::Settings {
        fullscreen: true,
        transparent: true,
        decorations: false,
        resizable: false,
        level: window::Level::AlwaysOnTop,
        exit_on_close_request: false,
        ..Default::default()
    });

    state.flash = Some(Flash {
        window: id,
        started: Instant::now(),
        settings,
    });

    let configure = if passthrough {
        // Purely visual: pointer events fall through to whatever is
        // underneath, so the flash cannot swallow a click.
        window::enable_mouse_passthrough(id)
    } else {
        Task::none()
    };

    Task::batch([close_existing, open.discard(), configure])
}

/// Silences the alarm and takes the flash off the screen.
fn dismiss(state: &mut State) -> Task<Message> {
    state.alarm_active = false;
    state.audio.stop();

    match state.flash.take() {
        Some(flash) => window::close(flash.window),
        None => Task::none(),
    }
}

/// Marks the config dirty.
///
/// Dragging a slider produces a message per pixel; without the debounce that
/// would be a config write per pixel too. The flag turns on a subscription in
/// [`subscription`] that fires the write once things have settled, and edits
/// made while it is pending are all captured by that one write.
fn queue_save(state: &mut State) -> Task<Message> {
    state.save_queued = true;

    Task::none()
}

/// Brings the recorded tracks in line with the timer, writing the file when
/// a stretch has just opened or closed.
fn sync_tracks(state: &mut State) -> Task<Message> {
    if state.recorder.follow(state.timer.is_running(), Local::now()) {
        save_tracks(state)
    } else {
        Task::none()
    }
}

/// Writes the tracks out on a worker thread, the way the config is saved.
fn save_tracks(state: &State) -> Task<Message> {
    if !state.recorder.is_kept() {
        return Task::none();
    }

    let recorder = state.recorder.clone();
    blocking::run(move || recorder.save().map_err(|error| error.to_string()))
        .map(Message::TracksSaved)
}

/// The length of the nearest event or task that is just starting, if there
/// is one.
///
/// A timer set going while a standup is starting should time the standup, so
/// an event starting within [`NEAR_EVENT_START`] minutes donates its length.
/// All-day entries are left out: they have no hour to sit next to. A task
/// that has already been ticked off is not what anyone is timing either.
fn nearby_span(state: &State, now: DateTime<Local>) -> Option<Duration> {
    let window = chrono::Duration::minutes(NEAR_EVENT_START);

    state
        .calendar
        .visible_events(&state.config.calendar)
        .filter(|event| !event.all_day && !event.completed)
        .filter_map(|event| {
            let since = now.signed_duration_since(event.start);
            (since <= window && since >= -window).then(|| (since.abs(), event))
        })
        .min_by_key(|(distance, _)| *distance)
        .and_then(|(_, event)| (event.end - event.start).to_std().ok())
        .filter(|span| !span.is_zero())
}

/// Re-syncs when the day in view has rolled over at the configured boundary.
///
/// The day ends at the configured hour, not at midnight, so a 3am boundary
/// keeps the small hours on the day that is ending. When the boundary passes,
/// the events in hand are for the wrong day and have to be pulled again.
///
/// Only the day that is following today rolls over; once the user has navigated
/// to another day, the boundary passing must not pull them back to today.
fn refresh_day(state: &mut State) -> Task<Message> {
    if !state.calendar.follow_today {
        return Task::none();
    }

    let day = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
    if day != state.calendar.day {
        state.calendar.set_day(day, day);
        return sync_calendar(state);
    }
    Task::none()
}

fn sync_calendar(state: &mut State) -> Task<Message> {
    if !state.calendar.is_signed_in() || state.calendar.is_busy() {
        return Task::none();
    }

    state.calendar.sync = SyncState::Syncing;
    state.calendar.error = None;

    let client_id = state.config.calendar.client_id.clone();
    let client_secret = state.config.calendar.client_secret.clone();
    let disabled = state.config.calendar.disabled.clone();
    let day = state.calendar.day;
    let day_end_hour = state.config.calendar.day_start_hour();

    blocking::run(move || {
        calendar::sync_day(&client_id, &client_secret, &disabled, day, day_end_hour)
    })
    .map(Message::CalendarSynced)
}

fn level(always_on_top: bool) -> window::Level {
    if always_on_top {
        window::Level::AlwaysOnTop
    } else {
        window::Level::Normal
    }
}

/// The width the clock opens at: two thirds of the monitor, but never so
/// narrow that the layout folds below the minimum it supports.
fn launch_width(monitor_width: f32) -> f32 {
    (monitor_width * 2.0 / 3.0).max(MIN_SIZE.width)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flash(settings: config::Flash) -> Flash {
        Flash {
            window: window::Id::unique(),
            started: Instant::now(),
            settings,
        }
    }

    #[test]
    fn the_flash_fades_in_and_out_within_a_pulse() {
        let flash = flash(config::Flash {
            pulses: 1,
            pulse_ms: 1_000,
            peak_opacity: 1.0,
            ..config::Flash::default()
        });

        let at = |millis| flash.alpha(flash.started + Duration::from_millis(millis));

        assert_eq!(at(0), Some(0.0));
        assert!(at(500).unwrap() > 0.99, "should peak mid-pulse");
        assert!(at(950).unwrap() < 0.05, "should fade back out");
    }

    #[test]
    fn the_flash_reports_when_it_is_over() {
        let flash = flash(config::Flash {
            pulses: 2,
            pulse_ms: 500,
            ..config::Flash::default()
        });

        let total = flash.settings.total_duration();
        assert_eq!(total, Duration::from_millis(1_000));

        assert!(flash.alpha(flash.started + total - Duration::from_millis(1)).is_some());
        assert!(flash.alpha(flash.started + total).is_none());
    }

    #[test]
    fn a_flash_configured_faster_than_the_safety_cap_is_slowed_down() {
        let flash = flash(config::Flash {
            pulses: 2,
            pulse_ms: 20,
            ..config::Flash::default()
        });

        // The config asked for 50 pulses a second; it gets three.
        assert_eq!(flash.settings.total_duration(), config::Flash::MIN_PULSE * 2);
    }

    #[test]
    fn the_flash_never_exceeds_the_configured_peak() {
        let flash = flash(config::Flash {
            pulses: 3,
            peak_opacity: 0.35,
            ..config::Flash::default()
        });

        for millis in (0..3_000).step_by(7) {
            if let Some(alpha) = flash.alpha(flash.started + Duration::from_millis(millis)) {
                assert!((0.0..=0.35).contains(&alpha), "alpha {alpha} out of range");
            }
        }
    }

    #[test]
    fn a_zero_opacity_flash_draws_nothing() {
        let flash = flash(config::Flash {
            peak_opacity: 0.0,
            ..config::Flash::default()
        });

        assert_eq!(flash.alpha(flash.started + Duration::from_millis(100)), Some(0.0));
    }

    #[test]
    fn showing_tasks_is_remembered() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        assert!(state.config.calendar.show_tasks);

        let _ = update(&mut state, Message::ShowTasksToggled(false));

        assert!(!state.config.calendar.show_tasks);
        assert!(state.save_queued);
    }

    #[test]
    fn changing_the_end_of_day_moves_the_day_in_view() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        assert_eq!(state.config.calendar.day_end_hour, 0);

        let _ = update(&mut state, Message::CalendarDayEndChanged(3));

        assert_eq!(state.config.calendar.day_end_hour, 3);
        assert!(state.save_queued);
        // The day in view is recomputed from the new boundary, not left on the
        // date the old one produced.
        assert_eq!(state.calendar.day, calendar::day_at(Local::now(), 3));
    }

    #[test]
    fn an_out_of_range_end_of_day_is_clamped() {
        let mut state = State::new(Config::default(), Catalog::built_in());

        let _ = update(&mut state, Message::CalendarDayEndChanged(99));

        assert_eq!(
            state.config.calendar.day_end_hour,
            config::CalendarSettings::MAX_DAY_END_HOUR
        );
    }

    #[test]
    fn picking_a_task_color_recolors_the_list_and_its_events() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());

        // The sample uses a shortened id; a real task list is `tasks:<list>`.
        state.calendar.calendars[2].id = "tasks:list".into();
        let deep = state
            .calendar
            .events
            .iter_mut()
            .find(|event| event.id == "deep")
            .unwrap();
        deep.calendar_id = "tasks:list".into();

        let red = iced::Color::from_rgb8(0xDC, 0x21, 0x27);
        let _ = update(
            &mut state,
            Message::TaskColorPicked("tasks:list".into(), Some(red)),
        );

        assert_eq!(state.config.calendar.task_color("tasks:list"), Some(red));
        assert!(state.save_queued);
        assert_eq!(state.calendar.calendars[2].color, red);
        assert_eq!(
            state.calendar.event("deep").unwrap().color,
            red,
            "the task's own chip should follow the list color"
        );

        // Resetting has to repaint the chip, not just forget the override.
        let _ = update(&mut state, Message::TaskColorPicked("tasks:list".into(), None));
        assert_eq!(state.config.calendar.task_color("tasks:list"), None);
        assert_eq!(state.calendar.event("deep").unwrap().color, calendar::colors::TASK);
    }

    #[test]
    fn the_task_color_editor_opens_and_closes() {
        let mut state = State::new(Config::default(), Catalog::built_in());

        let _ = update(&mut state, Message::TaskColorEditorToggled("tasks:list".into()));
        assert_eq!(state.calendar.color_editor.as_deref(), Some("tasks:list"));

        let _ = update(&mut state, Message::TaskColorEditorToggled("tasks:list".into()));
        assert_eq!(state.calendar.color_editor, None);
    }

    #[test]
    fn the_window_opens_at_two_thirds_of_the_monitor() {
        assert_eq!(launch_width(1920.0), 1280.0);
        assert_eq!(launch_width(3840.0), 2560.0);
        // A tiny monitor must not fold the layout below its minimum.
        assert_eq!(launch_width(120.0), MIN_SIZE.width);
    }

    #[test]
    fn opening_an_event_in_the_top_half_puts_the_details_below() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        ui::calendar::record_pointer(20.0, 500.0);

        let _ = update(&mut state, Message::EventOpened("standup".into()));

        assert_eq!(state.calendar.open_event.as_deref(), Some("standup"));
        assert!(state.calendar.details_below);
    }

    #[test]
    fn opening_an_event_in_the_bottom_half_keeps_the_details_on_top() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        ui::calendar::record_pointer(480.0, 500.0);

        let _ = update(&mut state, Message::EventOpened("lunch".into()));

        assert_eq!(state.calendar.open_event.as_deref(), Some("lunch"));
        assert!(!state.calendar.details_below);
    }

    #[test]
    fn stepping_the_day_moves_the_view_and_drops_the_old_events() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
        let start = state.calendar.day;

        let _ = update(&mut state, Message::CalendarDayShift(1));

        assert_eq!(state.calendar.day, start.succ_opt().unwrap());
        assert!(
            state.calendar.events.is_empty(),
            "the old day's events must not paint onto the new day"
        );
        assert!(!state.calendar.follow_today);
    }

    #[test]
    fn stepping_back_to_a_day_paints_its_cached_events_at_once() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
        let start = state.calendar.day;
        let cached = state.calendar.events.len();
        assert!(cached > 0);

        let _ = update(&mut state, Message::CalendarDayShift(1));
        assert!(
            state.calendar.events.is_empty(),
            "a day not seen yet starts empty"
        );

        let _ = update(&mut state, Message::CalendarDayShift(-1));

        assert_eq!(state.calendar.day, start);
        assert_eq!(
            state.calendar.events.len(),
            cached,
            "the day we came back to should paint from the cache"
        );
    }

    #[test]
    fn selecting_today_makes_the_view_follow_it_again() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        let today = calendar::day_at(Local::now(), state.config.calendar.day_start_hour());
        state.calendar.day = today;
        state.calendar.follow_today = false;

        let _ = update(&mut state, Message::CalendarDaySelected(today));

        assert!(state.calendar.follow_today);
    }

    #[test]
    fn the_month_picker_opens_on_the_month_of_the_day_in_view() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar.day = chrono::NaiveDate::from_ymd_opt(2026, 10, 17).unwrap();

        let _ = update(&mut state, Message::CalendarMonthToggled);

        assert!(state.calendar.month_open);
        assert_eq!(
            state.calendar.month,
            chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
        );

        let _ = update(&mut state, Message::CalendarMonthToggled);
        assert!(!state.calendar.month_open);
    }

    #[test]
    fn stepping_the_month_crosses_the_year_boundary() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar.month = chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();

        let _ = update(&mut state, Message::CalendarMonthShift(1));
        assert_eq!(
            state.calendar.month,
            chrono::NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()
        );

        let _ = update(&mut state, Message::CalendarMonthShift(-2));
        assert_eq!(
            state.calendar.month,
            chrono::NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
        );
    }

    #[test]
    fn picking_a_day_closes_the_month_picker_and_moves_the_view() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar.month_open = true;
        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();

        let _ = update(&mut state, Message::CalendarDaySelected(day));

        assert_eq!(state.calendar.day, day);
        assert!(!state.calendar.month_open);
    }

    #[test]
    fn a_snapshot_for_a_day_we_left_does_not_snap_the_view_back() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());
        let moved_to = state.calendar.day.succ_opt().unwrap();
        state.calendar.day = moved_to;
        state.calendar.sync = SyncState::Syncing;

        // The sample snapshot is for the day we navigated away from.
        let _ = update(&mut state, Message::CalendarSynced(Ok(calendar::sample_snapshot())));

        assert_eq!(state.calendar.day, moved_to);
    }

    #[test]
    fn opening_the_timer_records_a_track_that_pausing_closes() {
        let mut state = State::new(Config::default(), Catalog::built_in());

        let _ = update(&mut state, Message::TimerOpened);

        assert!(state.timer_open);
        assert!(state.timer.is_running());
        assert!(state.recorder.is_recording());
        assert_eq!(state.recorder.tracks().len(), 1);

        let _ = update(&mut state, Message::StartPause);

        assert!(!state.timer.is_running());
        assert!(!state.recorder.is_recording());
        assert_eq!(state.recorder.tracks().len(), 1);

        let _ = update(&mut state, Message::StartPause);

        assert_eq!(
            state.recorder.tracks().len(),
            2,
            "resuming opens a second track"
        );
    }

    #[test]
    fn hiding_the_timer_leaves_a_running_one_and_its_track_alone() {
        let mut state = State::new(Config::default(), Catalog::built_in());

        let _ = update(&mut state, Message::TimerOpened);
        let _ = update(&mut state, Message::TimerClosed);

        assert!(!state.timer_open);
        assert!(state.timer.is_running());
        assert!(state.recorder.is_recording());
    }

    #[test]
    fn a_timer_started_near_an_event_takes_the_event_length() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());

        // Move the standup to now, so the clock is opened as it is starting.
        let standup = state.calendar.event_mut("standup").unwrap();
        standup.start = Local::now();
        standup.end = standup.start + chrono::Duration::minutes(30);

        let _ = update(&mut state, Message::TimerOpened);

        assert_eq!(state.timer.target(), Some(Duration::from_secs(30 * 60)));
    }

    #[test]
    fn a_timer_started_far_from_anything_keeps_the_target_it_had() {
        let mut state = State::new(Config::default(), Catalog::built_in());
        state.calendar = calendar::Store::with_snapshot(calendar::sample_snapshot());

        // The sample day is a week back, so nothing is starting right now.
        let _ = update(&mut state, Message::TimerOpened);

        assert_eq!(state.timer.target(), Config::default().timer.last_duration());
    }
}
