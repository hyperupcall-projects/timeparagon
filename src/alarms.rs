//! Alarms set on the calendar.
//!
//! An alarm is a wall-clock time with a set of things to do when it arrives:
//! the same sound, notification and flash a finished timer uses, but chosen
//! per alarm. They are placed from the day view's alarm mode, drawn there as a
//! purple needle, and removed the moment they go off.
//!
//! This module holds the alarms, the rules about when one may be set, and the
//! updates that change them. Drawing lives in `ui::calendar` and
//! `ui::alarms`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone, Timelike};
use directories::BaseDirs;
use iced::Task;
use serde::{Deserialize, Serialize};

use crate::app::{self, Message, State};
use crate::audio::Tone;
use crate::blocking;
use crate::config::{self, AlarmProfile};

/// How long an alarm may have been overdue at startup and still be rung.
///
/// An alarm that came due while the app was closed is worth a late ring if it
/// is only just missed, and worth dropping quietly if it is hours old.
const MISSED_GRACE_MINUTES: i64 = 5;

/// The text a new alarm's notification carries unless the profile says
/// otherwise, so it never announces itself as a finished timer.
const DEFAULT_SUMMARY: &str = "Alarm";

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
    pub fn firing_profile(&self) -> AlarmProfile {
        let mut profile = self.profile.clone();

        if !self.label.trim().is_empty() {
            profile.notification.summary = self.label.trim().to_owned();
        }
        if profile.notification.body.trim().is_empty() {
            profile.notification.body = format!("Alarm for {}", self.at.format("%H:%M"));
        }

        profile
    }
}

/// The popup that edits one alarm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    pub id: u64,
    /// The time field as typed, kept apart from the alarm so a half-typed
    /// time does not move it.
    pub time_text: String,
    /// Why the time field was refused, if it was.
    pub time_error: Option<String>,
}

/// Every alarm, and the state of the interface for setting them.
#[derive(Debug, Default)]
pub struct Alarms {
    items: Vec<Alarm>,
    /// Whether the day view is in alarm setting mode.
    pub mode: bool,
    /// The alarm being edited, if the popup is open.
    pub editor: Option<Editor>,
    /// Where the alarms are written. `None` for a state built by hand in a
    /// test, which keeps them in memory and leaves the disk alone.
    path: Option<PathBuf>,
    /// Set while an edit has not been written yet.
    dirty: bool,
}

impl Alarms {
    /// Points the alarms at a file and takes in whatever an earlier run left
    /// there. Alarms too far past due are dropped.
    ///
    /// Called at boot only: a state built by hand must not read or rewrite
    /// the user's own alarms.
    pub fn keep_in(&mut self, path: Option<PathBuf>) {
        self.path = path.clone();

        if let Some(mut alarms) = path.as_deref().and_then(read) {
            let now = Local::now();
            alarms.retain(|alarm| minutes_between(now, alarm.at) > -MISSED_GRACE_MINUTES);
            self.items = alarms;
        }
    }

    pub fn items(&self) -> &[Alarm] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, id: u64) -> Option<&Alarm> {
        self.items.iter().find(|alarm| alarm.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Alarm> {
        self.items.iter_mut().find(|alarm| alarm.id == id)
    }

    /// Adds an alarm and returns its id.
    pub fn add(&mut self, at: DateTime<Local>, profile: AlarmProfile) -> u64 {
        let id = self
            .items
            .iter()
            .map(|alarm| alarm.id)
            .max()
            .map_or(1, |max| max + 1);

        self.items.push(Alarm {
            id,
            at,
            label: String::new(),
            profile,
        });
        self.items.sort_by_key(|alarm| alarm.at);

        id
    }

    /// Removes an alarm, returning whether there was one.
    pub fn remove(&mut self, id: u64) -> bool {
        let before = self.items.len();
        self.items.retain(|alarm| alarm.id != id);

        self.items.len() != before
    }

    /// Takes out every alarm that has come due, earliest first.
    pub fn take_due(&mut self, now: DateTime<Local>) -> Vec<Alarm> {
        let (due, waiting) = std::mem::take(&mut self.items)
            .into_iter()
            .partition(|alarm| alarm.at <= now);
        self.items = waiting;

        due
    }

    /// What a save needs, so the write can happen off the UI thread.
    fn snapshot(&self) -> Option<(PathBuf, Vec<Alarm>)> {
        Some((self.path.clone()?, self.items.clone()))
    }
}

/// `when` with the seconds taken off.
pub fn minute_of(when: DateTime<Local>) -> DateTime<Local> {
    when.with_second(0)
        .and_then(|when| when.with_nanosecond(0))
        .unwrap_or(when)
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

/// The offset drawn beside the needle: `+45m`, `+2h`, `-30m`.
pub fn offset_label(minutes: i64) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };

    format!("{sign}{}", compact(minutes.unsigned_abs()))
}

/// When an alarm goes off, relative to now: `Goes off in 45m`.
pub fn countdown_label(minutes: i64) -> String {
    if minutes >= 1 {
        format!("Goes off in {}", compact(minutes as u64))
    } else {
        "Going off now".to_owned()
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

/// The alarms file: `$XDG_STATE_HOME/timetracker/alarms.json`.
pub fn path() -> Option<PathBuf> {
    Some(
        BaseDirs::new()?
            .state_dir()?
            .join("timetracker")
            .join("alarms.json"),
    )
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct File {
    alarms: Vec<Alarm>,
}

fn read(path: &Path) -> Option<Vec<Alarm>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(error) => {
            log::warn!("{} could not be read: {error}", path.display());
            return None;
        }
    };

    match serde_json::from_slice::<File>(&bytes) {
        Ok(file) => Some(file.alarms),
        Err(error) => {
            log::warn!(
                "{} could not be parsed, ignoring it: {error}",
                path.display()
            );
            None
        }
    }
}

/// Writes happen on worker threads, so two quick ones must not share the
/// temporary file.
static WRITING: Mutex<()> = Mutex::new(());

/// Replaces the file, going through a temporary one so a crash mid-write
/// cannot leave a truncated one behind.
fn write(path: &Path, alarms: Vec<Alarm>) -> io::Result<()> {
    let _guard = WRITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let bytes = serde_json::to_vec_pretty(&File { alarms })
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)
}

#[derive(Debug, Clone)]
pub enum Action {
    /// The Set Alarm button: enter or leave alarm setting mode.
    ModeToggled,
    /// A click in alarm setting mode, at the time under the needle.
    Place(DateTime<Local>),
    /// An alarm's needle was clicked.
    Open(u64),
    CloseEditor,
    Delete(u64),
    Edit(Edit),
    SoundFileRequested,
    SoundFileSelected(Option<PathBuf>),
    Preview,
    /// The once-a-second check for alarms that have come due.
    Poll,
    /// The debounce for edits that have not been written yet.
    Flush,
    Saved(Result<(), String>),
}

/// One change in the alarm popup, applied to the alarm being edited.
#[derive(Debug, Clone)]
pub enum Edit {
    Label(String),
    Time(String),
    Notification(bool),
    Urgency(config::Urgency),
    Raise(bool),
    Sound(bool),
    Tone(Tone),
    Volume(u8),
    Repeats(u8),
    Flash(bool),
    Strength(f32),
    Pulses(u8),
    Passthrough(bool),
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
    match action {
        Action::ModeToggled => {
            if state.alarms.mode {
                state.alarms.mode = false;
                return Task::none();
            }

            // Alarm mode owns the grid, so whatever else was floating over
            // it is put away first.
            state.alarms.mode = true;
            state.calendar.menu_open = false;
            state.calendar.month_open = false;
            state.calendar.open_event = None;

            close_editor(state)
        }

        Action::Place(at) => {
            // The grid already refuses these clicks; this is the rule itself,
            // so no other path can ever set an alarm in the past.
            if !can_set(Local::now(), at) {
                return Task::none();
            }

            let mut profile = state.config.alarm.clone();
            if profile.notification.summary == config::Notification::default().summary {
                profile.notification.summary = DEFAULT_SUMMARY.to_owned();
            }

            let id = state.alarms.add(at, profile);
            state.alarms.mode = false;
            open(state, id);

            save(state)
        }

        Action::Open(id) => {
            state.alarms.mode = false;
            open(state, id);
            Task::none()
        }

        Action::CloseEditor => close_editor(state),

        Action::Delete(id) => {
            if let Some(editor) = &state.alarms.editor
                && editor.id == id
            {
                state.alarms.editor = None;
            }

            if state.alarms.remove(id) {
                save(state)
            } else {
                Task::none()
            }
        }

        Action::Edit(edit) => {
            edit_alarm(state, edit);
            Task::none()
        }

        Action::SoundFileRequested => blocking::run(|| {
            rfd::FileDialog::new()
                .set_title("Choose an alarm sound")
                .add_filter("Audio", &["wav", "ogg", "oga", "flac", "mp3", "m4a", "aac"])
                .pick_file()
        })
        .map(|file| Message::Alarms(Action::SoundFileSelected(file))),

        Action::SoundFileSelected(Some(file)) => {
            let Some(alarm) = edited(state) else {
                return Task::none();
            };

            alarm.profile.sound.choice = config::SoundChoice::Custom { file };
            state.alarms.dirty = true;
            state.audio_error = None;

            Task::done(Message::Alarms(Action::Preview))
        }

        Action::SoundFileSelected(None) => Task::none(),

        Action::Preview => {
            let Some(alarm) = state
                .alarms
                .editor
                .as_ref()
                .and_then(|e| state.alarms.get(e.id))
            else {
                return Task::none();
            };

            let actions = crate::alarm::preview(&alarm.profile.sound);
            app::perform(state, actions)
        }

        Action::Poll => ring_due(state),

        Action::Flush => {
            if state.alarms.dirty {
                save(state)
            } else {
                Task::none()
            }
        }

        Action::Saved(result) => {
            if let Err(error) = result {
                log::warn!("could not save the alarms: {error}");
            }

            Task::none()
        }
    }
}

/// Opens the popup on an alarm.
fn open(state: &mut State, id: u64) {
    let Some(alarm) = state.alarms.get(id) else {
        return;
    };

    state.alarms.editor = Some(Editor {
        id,
        time_text: alarm.at.format("%H:%M").to_string(),
        time_error: None,
    });
    state.calendar.menu_open = false;
    state.calendar.month_open = false;
}

/// Puts the popup away, writing any edit that was still waiting.
fn close_editor(state: &mut State) -> Task<Message> {
    state.alarms.editor = None;

    if state.alarms.dirty {
        save(state)
    } else {
        Task::none()
    }
}

/// The alarm the popup is editing.
fn edited(state: &mut State) -> Option<&mut Alarm> {
    let id = state.alarms.editor.as_ref()?.id;

    state.alarms.get_mut(id)
}

fn edit_alarm(state: &mut State, edit: Edit) {
    let Some(id) = state.alarms.editor.as_ref().map(|editor| editor.id) else {
        return;
    };

    if let Edit::Time(text) = edit {
        set_time(state, id, text);
        return;
    }

    let Some(alarm) = state.alarms.get_mut(id) else {
        return;
    };
    let profile = &mut alarm.profile;

    match edit {
        Edit::Label(label) => alarm.label = label,
        Edit::Time(_) => {}
        Edit::Notification(enabled) => profile.notification.enabled = enabled,
        Edit::Urgency(urgency) => profile.notification.urgency = urgency,
        Edit::Raise(raise) => profile.raise_window = raise,
        Edit::Sound(enabled) => profile.sound.enabled = enabled,
        Edit::Tone(tone) => profile.sound.choice = config::SoundChoice::Builtin(tone),
        Edit::Volume(volume) => profile.sound.volume = volume,
        Edit::Repeats(repeats) => profile.sound.repeats = repeats,
        Edit::Flash(enabled) => profile.flash.enabled = enabled,
        Edit::Strength(opacity) => profile.flash.peak_opacity = opacity,
        Edit::Pulses(pulses) => profile.flash.pulses = pulses,
        Edit::Passthrough(passthrough) => profile.flash.passthrough = passthrough,
    }

    state.alarms.dirty = true;
}

/// Applies the time field: the alarm moves when the text is a time that is
/// still ahead, and the field says why when it is not.
fn set_time(state: &mut State, id: u64, text: String) {
    let outcome = match parse_time(&text) {
        None => Err("Use the 24-hour form, like 07:30".to_owned()),
        Some(time) => {
            let date = state
                .alarms
                .get(id)
                .map(|alarm| alarm.at.date_naive())
                .unwrap_or_default();

            match local_at(date, time) {
                Some(at) if can_set(Local::now(), at) => Ok(at),
                Some(_) => Err("That time has already passed".to_owned()),
                None => Err("That time does not exist on this day".to_owned()),
            }
        }
    };

    if let Some(editor) = state.alarms.editor.as_mut() {
        editor.time_text = text;
        editor.time_error = outcome.as_ref().err().cloned();
    }

    if let Ok(at) = outcome
        && let Some(alarm) = state.alarms.get_mut(id)
    {
        alarm.at = at;
        state.alarms.items.sort_by_key(|alarm| alarm.at);
        state.alarms.dirty = true;
    }
}

/// Runs every alarm that has come due and takes it off the calendar.
fn ring_due(state: &mut State) -> Task<Message> {
    let due = state.alarms.take_due(Local::now());
    if due.is_empty() {
        return Task::none();
    }

    let mut tasks = Vec::new();

    for alarm in due {
        // An alarm that goes off under its own open popup takes it along.
        if state
            .alarms
            .editor
            .as_ref()
            .is_some_and(|editor| editor.id == alarm.id)
        {
            state.alarms.editor = None;
        }

        tasks.push(app::fire(state, &alarm.firing_profile()));
    }

    tasks.push(save(state));

    Task::batch(tasks)
}

/// Writes the alarms on a worker thread, the way the config is saved.
fn save(state: &mut State) -> Task<Message> {
    state.alarms.dirty = false;

    let Some((path, alarms)) = state.alarms.snapshot() else {
        return Task::none();
    };

    blocking::run(move || write(&path, alarms).map_err(|error| error.to_string()))
        .map(|result| Message::Alarms(Action::Saved(result)))
}

/// Whether anything wants the once-a-second poll or the write debounce.
pub fn needs_poll(alarms: &Alarms) -> bool {
    !alarms.items.is_empty()
}

/// Whether an edit is waiting to be written.
pub fn needs_flush(alarms: &Alarms) -> bool {
    alarms.dirty
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as Span;

    fn at(hour: u32, minute: u32, second: u32) -> DateTime<Local> {
        local_at(
            NaiveDate::from_ymd_opt(2026, 10, 26).unwrap(),
            NaiveTime::from_hms_opt(hour, minute, second).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn offsets_read_the_way_they_are_spoken() {
        assert_eq!(offset_label(45), "+45m");
        assert_eq!(offset_label(120), "+2h");
        assert_eq!(offset_label(-30), "-30m");
        assert_eq!(offset_label(75), "+1h 15m");
        assert_eq!(offset_label(1_500), "+1d 1h");
        assert_eq!(offset_label(1_445), "+1d 5m");
        assert_eq!(offset_label(0), "+0m");
    }

    #[test]
    fn the_countdown_says_how_long_is_left() {
        assert_eq!(countdown_label(45), "Goes off in 45m");
        assert_eq!(countdown_label(135), "Goes off in 2h 15m");
        assert_eq!(countdown_label(0), "Going off now");
    }

    #[test]
    fn minutes_are_counted_between_clock_faces() {
        // 44 minutes and 20 seconds later, the clock still says 45.
        assert_eq!(minutes_between(at(10, 0, 40), at(10, 45, 0)), 45);
        assert_eq!(minutes_between(at(10, 0, 0), at(9, 30, 0)), -30);
    }

    #[test]
    fn the_past_and_the_current_minute_cannot_be_set() {
        let now = at(10, 0, 40);

        assert!(!can_set(now, at(9, 59, 0)));
        assert!(!can_set(now, at(10, 0, 0)), "this minute is already gone");
        assert!(can_set(now, at(10, 1, 0)));
        assert!(can_set(now, at(10, 0, 0) + Span::days(1)));
    }

    #[test]
    fn times_are_read_as_twenty_four_hour_clock() {
        assert_eq!(parse_time("07:30"), NaiveTime::from_hms_opt(7, 30, 0));
        assert_eq!(parse_time(" 7:05 "), NaiveTime::from_hms_opt(7, 5, 0));
        assert_eq!(parse_time("25:00"), None);
        assert_eq!(parse_time("soon"), None);
    }

    #[test]
    fn alarms_come_back_in_time_order_with_fresh_ids() {
        let mut alarms = Alarms::default();

        let late = alarms.add(at(12, 0, 0), AlarmProfile::default());
        let early = alarms.add(at(8, 0, 0), AlarmProfile::default());

        assert_ne!(late, early);
        assert_eq!(alarms.items()[0].id, early);
    }

    #[test]
    fn only_alarms_that_are_due_are_taken() {
        let mut alarms = Alarms::default();
        alarms.add(at(8, 0, 0), AlarmProfile::default());
        let later = alarms.add(at(12, 0, 0), AlarmProfile::default());

        let due = alarms.take_due(at(8, 0, 0));

        assert_eq!(due.len(), 1);
        assert_eq!(alarms.items().len(), 1);
        assert_eq!(alarms.items()[0].id, later);
        assert!(alarms.take_due(at(8, 0, 1)).is_empty());
    }

    #[test]
    fn a_label_becomes_the_notification_headline() {
        let mut alarms = Alarms::default();
        let id = alarms.add(at(8, 5, 0), AlarmProfile::default());
        alarms.get_mut(id).unwrap().label = "  Stand up ".to_owned();

        let profile = alarms.get(id).unwrap().firing_profile();

        assert_eq!(profile.notification.summary, "Stand up");
        assert_eq!(profile.notification.body, "Alarm for 08:05");
    }

    #[test]
    fn alarms_survive_the_file() {
        let path = std::env::temp_dir().join(format!(
            "timetracker-alarms-{}-{}.json",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_file(&path);

        let mut alarms = Alarms::default();
        let soon = Local::now() + Span::hours(1);
        let id = alarms.add(soon, AlarmProfile::default());
        alarms.get_mut(id).unwrap().label = "Tea".to_owned();

        write(&path, alarms.items().to_vec()).unwrap();

        let mut loaded = Alarms::default();
        loaded.keep_in(Some(path.clone()));
        let _ = fs::remove_file(&path);

        assert_eq!(loaded.items(), alarms.items());
    }

    #[test]
    fn an_alarm_long_overdue_at_startup_is_dropped() {
        let path = std::env::temp_dir().join(format!(
            "timetracker-alarms-{}-{}.json",
            std::process::id(),
            line!()
        ));
        let _ = fs::remove_file(&path);

        let mut alarms = Alarms::default();
        alarms.add(Local::now() - Span::hours(3), AlarmProfile::default());
        write(&path, alarms.items().to_vec()).unwrap();

        let mut loaded = Alarms::default();
        loaded.keep_in(Some(path.clone()));
        let _ = fs::remove_file(&path);

        assert!(loaded.is_empty());
    }
}
