//! Reminders on the desktop.
//!
//! What is due comes from `timeslay_core::reminders`, planned from the same
//! day cache the phone uses. Delivery is the freedesktop way: a notification
//! through the session's notification daemon, with the sound either left to
//! the daemon's sound theme or played here through PipeWire.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Local};
use iced::Task;

use crate::app::{self, Message, State, SystemMessage};
use crate::{alarm, blocking, config};

pub use timeslay_core::reminders::{Category, Due, ReminderSettings, Sound};

/// How often the calendar is checked while reminders are on. A minute's
/// resolution is what a reminder needs; every 20 seconds keeps a reminder
/// within a third of that, and planning is a cheap read of the day cache.
pub const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(20);

/// The sound theme name a notification daemon plays for an ordinary
/// notification. See the freedesktop sound naming specification.
const DEFAULT_SOUND_NAME: &str = "message-new-instant";

/// Fallback names tried in the sound theme, in order, when looking for the
/// default sound to play ourselves.
const DEFAULT_SOUND_NAMES: [&str; 4] =
    [DEFAULT_SOUND_NAME, "message", "dialog-information", "bell"];

/// The reminder state the app keeps between polls.
#[derive(Debug, Default)]
pub struct Reminders {
    /// Keys of the reminders already delivered, so overlapping polls never
    /// deliver one twice.
    delivered: HashSet<String>,
    /// Where the last poll's window ended.
    checked_until: Option<DateTime<Local>>,
    /// Whether a poll is running on a worker thread.
    polling: bool,
    /// The sounds the desktop's sound themes provide, once looked up.
    pub device_sounds: Option<Vec<Sound>>,
    /// Set in the one long-running instance, the side panel. The task
    /// popover is its own short-lived process and must not remind too.
    pub active: bool,
}

#[derive(Debug, Clone)]
pub enum Action {
    Poll,
    Planned {
        until: DateTime<Local>,
        due: Vec<Due>,
    },
    Toggled(bool),
    CalendarRemindersToggled(bool),
    CategoryToggled(Category, bool),
    LeadChanged(Category, u32),
    SoundChanged(Category, Sound),
    /// Plays the category's sound and posts a sample notification.
    Test(Category),
    DeviceSoundsLoaded(Vec<Sound>),
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
    match action {
        Action::Poll => poll(state),
        Action::Planned { until, due } => {
            state.reminders.polling = false;
            state.reminders.checked_until = Some(until);
            let fresh: Vec<Due> = due
                .into_iter()
                .filter(|due| state.reminders.delivered.insert(due.key.clone()))
                .collect();
            if state.reminders.delivered.len() > 512 {
                // Polls cover back-to-back windows, so a key only needs to
                // outlive the poll after the one that delivered it.
                state.reminders.delivered = fresh.iter().map(|due| due.key.clone()).collect();
            }
            Task::batch(fresh.into_iter().map(|due| deliver(state, due)))
        }
        Action::Toggled(enabled) => {
            state.config.reminders.enabled = enabled;
            // Start from now: turning reminders on must not replay the past.
            state.reminders.checked_until = None;
            Task::batch([
                app::queue_save(state),
                if enabled {
                    load_device_sounds(state)
                } else {
                    Task::none()
                },
            ])
        }
        Action::CalendarRemindersToggled(enabled) => {
            state.config.reminders.use_calendar_reminders = enabled;
            app::queue_save(state)
        }
        Action::CategoryToggled(category, enabled) => {
            state.config.reminders.get_mut(category).enabled = enabled;
            app::queue_save(state)
        }
        Action::LeadChanged(category, minutes) => {
            state.config.reminders.get_mut(category).minutes_before = minutes;
            app::queue_save(state)
        }
        Action::SoundChanged(category, sound) => {
            state.config.reminders.get_mut(category).sound = sound;
            Task::batch([app::queue_save(state), preview(state, category)])
        }
        Action::Test(category) => {
            let label = category.label();
            let sound = state.config.reminders.get(category).sound.clone();
            deliver_with(
                state,
                format!("{label} reminder"),
                "This is how TimeSlay reminds you.".to_owned(),
                &sound,
            )
        }
        Action::DeviceSoundsLoaded(sounds) => {
            state.reminders.device_sounds = Some(sounds);
            Task::none()
        }
    }
}

/// Whether the poll subscription should run.
pub fn needs_poll(state: &State) -> bool {
    state.reminders.active && state.config.reminders.any_enabled()
}

/// Looks up the desktop's sounds once, when the settings first need them.
pub fn load_device_sounds(state: &State) -> Task<Message> {
    if state.reminders.device_sounds.is_some() {
        return Task::none();
    }
    blocking::run(|| Some(device_sounds()))
        .map(|sounds| Message::Reminders(Action::DeviceSoundsLoaded(sounds.unwrap_or_default())))
}

fn poll(state: &mut State) -> Task<Message> {
    if state.reminders.polling || !needs_poll(state) {
        return Task::none();
    }
    let Some(database) = timeslay_core::task_db::path() else {
        return Task::none();
    };

    let now = crate::clock::now();
    let earliest = now - Duration::minutes(timeslay_core::reminders::LATE_GRACE_MINUTES);
    // The first poll starts at now, so a restart does not replay reminders
    // that went off before it.
    let from = state
        .reminders
        .checked_until
        .map_or(now, |checked| checked.max(earliest))
        .min(now);

    state.reminders.polling = true;
    let settings = state.config.reminders.clone();
    let calendar = state.config.calendar.clone();
    let travel = &state.config.travel;
    let places = timeslay_core::places::PlaceSettings {
        api_key: String::new(),
        home: travel.home,
        places: travel.places.clone(),
        event_places: travel.event_places.clone(),
        default_mode: travel.default_mode,
        modes: travel.modes.clone(),
        show: travel.show,
        routes: travel.routes.clone(),
    };
    let time_format = state.config.appearance.time_format;

    blocking::run(move || {
        Some(timeslay_core::reminders::plan_from_cache(
            &database,
            &settings,
            &calendar,
            &places,
            time_format,
            from,
            now,
        ))
    })
    .map(move |due| {
        Message::Reminders(Action::Planned {
            until: now,
            due: due.unwrap_or_default(),
        })
    })
}

fn deliver(state: &mut State, due: Due) -> Task<Message> {
    let sound = state.config.reminders.get(due.category).sound.clone();
    deliver_with(state, due.title, due.body, &sound)
}

fn deliver_with(state: &mut State, title: String, body: String, sound: &Sound) -> Task<Message> {
    let (played, hint) = resolve(sound);
    let notification = crate::notify::Reminder {
        summary: title,
        body,
        sound_name: hint,
        silent: played.is_some() || matches!(sound, Sound::Silent),
    };

    let mut tasks = vec![
        blocking::run(move || crate::notify::show_reminder(&notification))
            .map(|result| Message::System(SystemMessage::NotificationPosted(result))),
    ];
    if let Some(choice) = played {
        tasks.push(play(state, choice));
    }
    Task::batch(tasks)
}

/// Plays a category's sound on its own, as the settings change it.
fn preview(state: &mut State, category: Category) -> Task<Message> {
    match resolve(&state.config.reminders.get(category).sound).0 {
        Some(choice) => play(state, choice),
        None => Task::none(),
    }
}

fn play(state: &mut State, choice: config::SoundChoice) -> Task<Message> {
    // The alarm's volume is the app's one volume setting.
    let gain = state.config.alarm.sound.gain();
    let actions = alarm::Actions {
        sound: Some(alarm::Sound {
            choice,
            gain,
            repeats: 1,
            fade_in: std::time::Duration::ZERO,
        }),
        ..alarm::Actions::none()
    };
    app::perform(state, actions)
}

/// What to play here, if anything, and otherwise which sound to ask the
/// notification daemon for.
fn resolve(sound: &Sound) -> (Option<config::SoundChoice>, Option<&'static str>) {
    match sound {
        Sound::Silent => (None, None),
        Sound::Tone { tone } => (Some(config::SoundChoice::Builtin(*tone)), None),
        Sound::Device { uri, .. } if Path::new(uri).is_file() => (
            Some(config::SoundChoice::Custom {
                file: PathBuf::from(uri),
            }),
            None,
        ),
        // A sound that went away with its theme plays the default instead.
        Sound::Device { .. } | Sound::Default => match default_sound_file() {
            Some(file) => (Some(config::SoundChoice::Custom { file }), None),
            // No theme installed: leave it to the daemon, which may have one.
            None => (None, Some(DEFAULT_SOUND_NAME)),
        },
    }
}

/// Where sound themes are installed, most specific first.
fn sound_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
    {
        dirs.push(home.join("sounds"));
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    dirs.extend(
        data_dirs
            .split(':')
            .filter(|dir| !dir.is_empty())
            .map(|dir| Path::new(dir).join("sounds")),
    );
    dirs
}

/// The sound theme the desktop is set to, as far as can be told without a
/// settings daemon: KDE writes it to `kdeglobals`.
fn current_theme() -> Option<String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let text = std::fs::read_to_string(config.join("kdeglobals")).ok()?;
    theme_from_kdeglobals(&text)
}

fn theme_from_kdeglobals(text: &str) -> Option<String> {
    let mut in_sounds = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_sounds = line == "[Sounds]";
        } else if in_sounds && let Some(value) = line.strip_prefix("Theme=") {
            return Some(value.trim().to_owned()).filter(|theme| !theme.is_empty());
        }
    }
    None
}

/// Theme names to search, in order of preference.
fn theme_order() -> Vec<String> {
    let mut themes: Vec<String> = current_theme().into_iter().collect();
    let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.contains("KDE"));
    if kde {
        themes.push("ocean".to_owned());
    }
    themes.push("freedesktop".to_owned());
    themes.dedup();
    themes
}

const EXTENSIONS: [&str; 4] = ["oga", "ogg", "wav", "flac"];

/// The file the desktop would play for an ordinary notification.
fn default_sound_file() -> Option<PathBuf> {
    let dirs = sound_dirs();
    theme_order().iter().find_map(|theme| {
        DEFAULT_SOUND_NAMES.iter().find_map(|name| {
            dirs.iter().find_map(|dir| {
                EXTENSIONS.iter().find_map(|extension| {
                    let file = dir
                        .join(theme)
                        .join("stereo")
                        .join(format!("{name}.{extension}"));
                    file.is_file().then_some(file)
                })
            })
        })
    })
}

/// Every sound the installed sound themes offer, the current theme first.
fn device_sounds() -> Vec<Sound> {
    let order = theme_order();
    let mut themes: Vec<(String, PathBuf)> = Vec::new();
    for dir in sound_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let stereo = entry.path().join("stereo");
            if stereo.is_dir() && !themes.iter().any(|(seen, _)| *seen == name) {
                themes.push((name, stereo));
            }
        }
    }
    themes.sort_by_key(|(name, _)| {
        (
            order
                .iter()
                .position(|preferred| preferred == name)
                .unwrap_or(usize::MAX),
            name.clone(),
        )
    });

    let mut sounds = Vec::new();
    for (theme, dir) in themes {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| EXTENSIONS.contains(&extension))
            })
            .collect();
        files.sort();
        sounds.extend(files.into_iter().filter_map(|file| {
            let stem = file.file_stem()?.to_string_lossy().into_owned();
            Some(Sound::Device {
                name: format!("{} · {}", title_case(&theme), title_case(&stem)),
                uri: file.to_string_lossy().into_owned(),
            })
        }));
    }
    sounds
}

/// "message-new-instant" as "Message New Instant".
fn title_case(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kde_sound_theme_is_read_from_its_section() {
        let text = "[General]\nTheme=breeze\n\n[Sounds]\nEnable=true\nTheme=ocean\n";
        assert_eq!(theme_from_kdeglobals(text).as_deref(), Some("ocean"));
        assert_eq!(theme_from_kdeglobals("[General]\nTheme=x\n"), None);
    }

    #[test]
    fn sound_names_read_as_titles() {
        assert_eq!(title_case("message-new-instant"), "Message New Instant");
        assert_eq!(title_case("freedesktop"), "Freedesktop");
    }

    #[test]
    fn silent_and_tones_never_ask_the_daemon_for_a_sound() {
        assert_eq!(resolve(&Sound::Silent), (None, None));
        let (played, hint) = resolve(&Sound::Tone {
            tone: crate::audio::Tone::Ding,
        });
        assert!(played.is_some());
        assert!(hint.is_none());
    }

    #[test]
    fn the_default_sound_is_played_or_left_to_the_daemon() {
        let (played, hint) = resolve(&Sound::Default);
        assert!(played.is_some() != hint.is_some());
    }
}
