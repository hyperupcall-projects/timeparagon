//! Settings for the side-panel companion: the `[panel]` table of the main
//! `config.toml`.
//!
//! Every field is optional; a missing table or field falls back to the
//! defaults below, so the panel runs out of the box.

use serde::{Deserialize, Serialize};

/// Which screen edge the strip is anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    #[default]
    Left,
    Right,
}

/// What a global shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Hides TimeSlay in whatever shape it is in, or brings it back in the
    /// same shape.
    ToggleVisibility,
    /// Switches between the side panel and the dock.
    ToggleShape,
    ToggleReservation,
}

impl Action {
    pub const ALL: [Self; 3] = [
        Self::ToggleVisibility,
        Self::ToggleShape,
        Self::ToggleReservation,
    ];

    /// The id KGlobalAccel knows the action by.
    pub fn id(self) -> &'static str {
        match self {
            Self::ToggleVisibility => "toggle-visibility",
            Self::ToggleShape => "toggle-shape",
            Self::ToggleReservation => "toggle-reservation",
        }
    }

    /// The name shown to people, including in KDE's System Settings.
    pub fn label(self) -> &'static str {
        match self {
            Self::ToggleVisibility => "Toggle Visibility",
            Self::ToggleShape => "Toggle Shape",
            Self::ToggleReservation => "Toggle Space Reservation",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.id() == id)
    }

    pub fn index(self) -> usize {
        match self {
            Self::ToggleVisibility => 0,
            Self::ToggleShape => 1,
            Self::ToggleReservation => 2,
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::ToggleVisibility => {
                "Hides TimeSlay, as the side panel or the dock, or shows it again exactly as it was."
            }
            Self::ToggleShape => "Switches between the side panel and the dock.",
            Self::ToggleReservation => {
                "Toggles reserved space for the visible panel or dock only. Does nothing when hidden."
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelConfig {
    /// Screen edge to anchor the strip to. The README ships `left`.
    pub side: Side,
    /// Strip width in logical pixels. Height always fills the screen.
    pub width: u32,
    /// Global shortcuts for [`Action::ToggleVisibility`], each in
    /// `Modifier+Key` form, e.g. `"Meta+U"`. Any of them triggers the action.
    /// The old single `shortcut = "Meta+U"` key is still read.
    #[serde(alias = "shortcut", deserialize_with = "string_or_list")]
    pub toggle_visibility: Vec<String>,
    /// Global shortcuts for [`Action::ToggleShape`].
    #[serde(deserialize_with = "string_or_list")]
    pub toggle_shape: Vec<String>,
    #[serde(deserialize_with = "string_or_list")]
    pub toggle_reservation: Vec<String>,
}

/// Accepts `"Meta+U"` as well as `["Meta+U", "Meta+J"]`.
fn string_or_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }

    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(one) => vec![one],
        OneOrMany::Many(many) => many,
    })
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            side: Side::Left,
            width: 200,
            toggle_visibility: vec!["Meta+U".to_owned()],
            toggle_shape: vec!["Meta+Shift+U".to_owned()],
            toggle_reservation: Vec::new(),
        }
    }
}

impl PanelConfig {
    /// The shortcuts that trigger `action`.
    pub fn shortcuts(&self, action: Action) -> &[String] {
        match action {
            Action::ToggleVisibility => &self.toggle_visibility,
            Action::ToggleShape => &self.toggle_shape,
            Action::ToggleReservation => &self.toggle_reservation,
        }
    }

    pub fn set_shortcuts(&mut self, action: Action, shortcuts: Vec<String>) {
        match action {
            Action::ToggleVisibility => self.toggle_visibility = shortcuts,
            Action::ToggleShape => self.toggle_shape = shortcuts,
            Action::ToggleReservation => self.toggle_reservation = shortcuts,
        }
    }

    /// The `[panel]` table of the main config, falling back to defaults.
    pub fn load() -> Self {
        crate::config::Config::load().panel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_defaults_to_the_left_edge_at_two_hundred() {
        let config = PanelConfig::default();

        assert_eq!(config.side, Side::Left);
        assert_eq!(config.width, 200);
        assert_eq!(config.toggle_visibility, ["Meta+U"]);
        assert_eq!(config.toggle_shape, ["Meta+Shift+U"]);
    }

    #[test]
    fn a_partial_panel_config_keeps_the_other_defaults() {
        let config: PanelConfig = toml::from_str("width = 240").unwrap();

        assert_eq!(config.width, 240);
        assert_eq!(config.side, Side::Left);
        assert_eq!(config.toggle_visibility, ["Meta+U"]);
    }

    #[test]
    fn an_action_can_have_several_shortcuts() {
        let config: PanelConfig =
            toml::from_str("toggle_visibility = [\"Meta+J\", \"Meta+F\"]\ntoggle_shape = []")
                .unwrap();

        assert_eq!(config.toggle_visibility, ["Meta+J", "Meta+F"]);
        assert!(config.toggle_shape.is_empty());

        let restored: PanelConfig = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(restored, config);
    }

    #[test]
    fn the_old_single_shortcut_key_still_loads() {
        let config: PanelConfig = toml::from_str("shortcut = \"Meta+K\"").unwrap();

        assert_eq!(config.toggle_visibility, ["Meta+K"]);
        assert_eq!(config.toggle_shape, ["Meta+Shift+U"]);
    }

    #[test]
    fn actions_are_found_by_their_ids() {
        for action in Action::ALL {
            assert_eq!(Action::from_id(action.id()), Some(action));
        }
        assert_eq!(Action::from_id("panel"), None);
    }

    #[test]
    fn the_side_parses_lowercase() {
        assert_eq!(
            toml::from_str::<PanelConfig>("side = \"right\"")
                .unwrap()
                .side,
            Side::Right
        );
        assert_eq!(
            toml::from_str::<PanelConfig>("side = \"left\"")
                .unwrap()
                .side,
            Side::Left
        );
    }
}
