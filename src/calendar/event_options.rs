//! Google Calendar's writable notification, availability and privacy fields.
use serde::{Deserialize, Serialize};

macro_rules! choice {
    ($name:ident { $first:ident => ($value:literal, $label:literal), $( $variant:ident => ($v:literal, $l:literal) ),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
        pub enum $name {
            #[default]
            #[serde(rename = $value)]
            $first,
            $( #[serde(rename = $v)] $variant, )*
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(match self { Self::$first => $label, $( Self::$variant => $l, )* })
            }
        }
    };
}
choice!(Availability { Busy => ("opaque", "Busy"), Free => ("transparent", "Free") });
choice!(Visibility { Default => ("default", "Default visibility"), Public => ("public", "Public"), Private => ("private", "Private"), Confidential => ("confidential", "Confidential") });
choice!(Method { Notification => ("popup", "Notification"), Email => ("email", "Email") });

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Unit {
    #[default]
    Minutes,
    Hours,
    Days,
    Weeks,
}
impl Unit {
    pub fn factor(self) -> u32 {
        match self {
            Self::Minutes => 1,
            Self::Hours => 60,
            Self::Days => 1440,
            Self::Weeks => 10080,
        }
    }
}
impl std::fmt::Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Minutes => "minutes",
            Self::Hours => "hours",
            Self::Days => "days",
            Self::Weeks => "weeks",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reminder {
    pub method: Method,
    pub minutes: u32,
    #[serde(skip)]
    pub unit: Unit,
    #[serde(skip)]
    pub input: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reminders {
    #[serde(rename = "useDefault")]
    pub use_default: bool,
    #[serde(default)]
    pub overrides: Vec<Reminder>,
}
impl Default for Reminders {
    fn default() -> Self {
        Self {
            use_default: true,
            overrides: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EventOptions {
    /// Calendar defaults returned by events.list, retained for display and caching.
    pub default_reminders: Vec<Reminder>,
    pub reminders: Reminders,
    pub transparency: Availability,
    pub visibility: Visibility,
}

#[derive(Debug, Clone)]
pub enum Action {
    Defaults(bool),
    Add,
    Remove(usize),
    Method(usize, Method),
    Minutes(usize, String),
    Unit(usize, Unit),
    Availability(Availability),
    Visibility(Visibility),
}
impl EventOptions {
    pub fn update(&mut self, action: Action) {
        match action {
            Action::Defaults(value) => {
                self.reminders.use_default = value;
                self.reminders.overrides = if value {
                    Vec::new()
                } else {
                    self.default_reminders.clone()
                };
            }
            Action::Add if self.reminders.overrides.len() < 5 => {
                if self.reminders.use_default {
                    self.reminders.overrides = self.default_reminders.clone();
                }
                self.reminders.use_default = false;
                if self.reminders.overrides.len() >= 5 {
                    return;
                }
                self.reminders.overrides.push(Reminder {
                    method: Method::Notification,
                    minutes: 10,
                    unit: Unit::Minutes,
                    input: None,
                });
            }
            Action::Remove(i) if i < self.reminders.overrides.len() => {
                self.reminders.overrides.remove(i);
            }
            Action::Method(i, method) => {
                if let Some(r) = self.reminders.overrides.get_mut(i) {
                    r.method = method;
                }
            }
            Action::Minutes(i, text) => {
                if let Some(r) = self.reminders.overrides.get_mut(i) {
                    if let Ok(n) = text.parse::<u32>() {
                        if let Some(minutes) = n.checked_mul(r.unit.factor()) {
                            r.minutes = minutes;
                        }
                    }
                    r.input = Some(text);
                }
            }
            Action::Unit(i, unit) => {
                if let Some(r) = self.reminders.overrides.get_mut(i) {
                    let count = r
                        .input
                        .as_deref()
                        .and_then(|v| v.parse::<u32>().ok())
                        .unwrap_or(r.minutes / r.unit.factor());
                    r.unit = unit;
                    r.minutes = count.saturating_mul(unit.factor());
                    r.input = Some(count.to_string());
                }
            }
            Action::Availability(value) => self.transparency = value,
            Action::Visibility(value) => self.visibility = value,
            _ => {}
        }
    }
    pub fn write(&self, body: &mut serde_json::Value) -> Result<(), String> {
        if !self.reminders.use_default
            && (self.reminders.overrides.len() > 5
                || self.reminders.overrides.iter().any(|r| {
                    r.minutes > 40320
                        || r.input.as_deref().is_some_and(|v| {
                            v.parse::<u32>()
                                .ok()
                                .and_then(|n| n.checked_mul(r.unit.factor()))
                                != Some(r.minutes)
                        })
                }))
        {
            return Err(
                "Use up to 5 notifications, each between 0 and 40320 minutes before the event"
                    .into(),
            );
        }
        body["reminders"] = if self.reminders.use_default {
            serde_json::json!({"useDefault": true})
        } else {
            serde_json::to_value(&self.reminders).map_err(|e| e.to_string())?
        };
        body["transparency"] =
            serde_json::to_value(self.transparency).map_err(|e| e.to_string())?;
        body["visibility"] = serde_json::to_value(self.visibility).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notification_limits_and_units_are_validated_before_writing() {
        let mut options = EventOptions::default();
        for _ in 0..6 {
            options.update(Action::Add);
        }
        assert_eq!(options.reminders.overrides.len(), 5);
        options.update(Action::Minutes(0, "2".into()));
        options.update(Action::Unit(0, Unit::Days));
        let mut body = serde_json::json!({});
        options.write(&mut body).unwrap();
        assert_eq!(body["reminders"]["overrides"][0]["minutes"], 2880);
        options.update(Action::Minutes(0, "29".into()));
        assert!(options.write(&mut body).is_err());
        options.update(Action::Minutes(0, "".into()));
        assert!(options.write(&mut body).is_err());
        options.update(Action::Minutes(0, "0".into()));
        options.write(&mut body).unwrap();
    }
}
