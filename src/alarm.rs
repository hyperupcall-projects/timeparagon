//! What happens when an alarm goes off.
//!
//! This module decides; the app layer carries it out. Keeping the decision
//! separate from the iced plumbing is what makes the behavior testable
//! without a window on screen.

use std::time::Duration;

use crate::config::{self, AlarmProfile};

/// Everything that should happen now, in one description.
#[derive(Debug, Clone, PartialEq)]
pub struct Actions {
    /// The sound to render and play, if any.
    pub sound: Option<Sound>,
    /// The notification to post, if any.
    pub notification: Option<config::Notification>,
    /// The screen flash to run, if any.
    pub flash: Option<config::Flash>,
    /// Whether to bring the window forward.
    pub raise: bool,
}

/// A sound to play, resolved from the profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    pub choice: config::SoundChoice,
    pub gain: f32,
    pub repeats: u8,
    pub fade_in: Duration,
}

impl Actions {
    /// Nothing to do, for a profile with everything switched off.
    pub fn none() -> Self {
        Self {
            sound: None,
            notification: None,
            flash: None,
            raise: false,
        }
    }

    /// Whether the alarm leaves anything on screen or playing that the user
    /// has to dismiss.
    pub fn is_dismissable(&self) -> bool {
        self.sound.is_some() || self.flash.is_some()
    }
}

/// Builds the action list for an alarm that just went off.
pub fn on_finished(profile: &AlarmProfile) -> Actions {
    Actions {
        sound: profile.sound.enabled.then(|| Sound {
            choice: profile.sound.choice.clone(),
            gain: profile.sound.gain(),
            repeats: profile.sound.repeats,
            fade_in: profile.sound.fade_in(),
        }),
        notification: profile
            .notification
            .enabled
            .then(|| profile.notification.clone()),
        flash: profile.flash.enabled.then(|| profile.flash.clone()),
        raise: profile.raise_window,
    }
}

/// Builds the action list for the Preview button in the settings pane, which
/// plays the sound and nothing else.
pub fn preview(sound: &config::Sound) -> Actions {
    Actions {
        sound: Some(Sound {
            choice: sound.choice.clone(),
            gain: sound.gain(),
            // A preview plays once however the alarm is configured; nobody
            // wants to audition a sound set to loop forever.
            repeats: 1,
            fade_in: sound.fade_in(),
        }),
        ..Actions::none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::Tone;

    #[test]
    fn a_default_profile_does_all_three_things() {
        let actions = on_finished(&AlarmProfile::default());

        assert!(actions.sound.is_some());
        assert!(actions.notification.is_some());
        assert!(actions.flash.is_some());
    }

    #[test]
    fn switching_everything_off_leaves_nothing_to_do() {
        let mut profile = AlarmProfile::default();
        profile.sound.enabled = false;
        profile.notification.enabled = false;
        profile.flash.enabled = false;

        let actions = on_finished(&profile);

        assert_eq!(actions, Actions::none());
        assert!(!actions.is_dismissable());
    }

    #[test]
    fn a_notification_only_alarm_needs_no_dismissing() {
        let mut profile = AlarmProfile::default();
        profile.sound.enabled = false;
        profile.flash.enabled = false;

        assert!(!on_finished(&profile).is_dismissable());
    }

    #[test]
    fn the_sound_carries_the_configured_volume_as_gain() {
        let mut profile = AlarmProfile::default();
        profile.sound.volume = 50;

        let sound = on_finished(&profile).sound.unwrap();

        assert_eq!(sound.gain, 0.25);
    }

    #[test]
    fn a_preview_plays_once_even_when_the_alarm_loops() {
        let sound = config::Sound {
            repeats: 0,
            choice: config::SoundChoice::Builtin(Tone::Bell),
            ..config::Sound::default()
        };

        let actions = preview(&sound);

        assert_eq!(actions.sound.as_ref().unwrap().repeats, 1);
        assert!(actions.notification.is_none());
        assert!(actions.flash.is_none());
    }
}
