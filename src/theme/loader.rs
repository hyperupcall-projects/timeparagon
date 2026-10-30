//! Reading themes out of `~/.config/timeslay/themes/*.toml`.
//!
//! Adding a theme should not require rebuilding the app, so the picker is
//! populated from disk at startup. A broken file is skipped with a warning
//! rather than taken as a reason to refuse to start.

use std::fs;
use std::path::Path;

use iced::theme::Palette;
use serde::{Deserialize, Serialize};

use crate::hex::HexColor;

use super::{Accents, Entry};

/// The on-disk shape of a theme file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeFile {
    pub name: String,

    /// Accepted for documentation value; the tone is derived from the
    /// background, so this does not need to be trusted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,

    pub background: HexColor,
    pub text: HexColor,
    pub primary: HexColor,
    pub success: HexColor,
    pub warning: HexColor,
    pub danger: HexColor,

    /// Optional overrides for the parts a six-color palette cannot express.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digits: Option<HexColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ring: Option<HexColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flash: Option<HexColor>,
}

impl ThemeFile {
    pub fn palette(&self) -> Palette {
        Palette {
            background: self.background.0,
            text: self.text.0,
            primary: self.primary.0,
            success: self.success.0,
            warning: self.warning.0,
            danger: self.danger.0,
        }
    }

    pub fn entry(&self) -> Entry {
        let theme = iced::Theme::custom(self.name.clone(), self.palette());
        let derived = Accents::derive(&theme);

        Entry {
            accents: Accents {
                digits: self.digits.map_or(derived.digits, |color| color.0),
                ring: self.ring.map_or(derived.ring, |color| color.0),
                flash: self.flash.map_or(derived.flash, |color| color.0),
            },
            name: self.name.clone(),
            theme,
        }
    }
}

/// Loads every theme in `directory`, sorted by name for a stable picker.
///
/// A missing directory is normal and not worth a warning.
pub fn load(directory: &Path) -> Vec<Entry> {
    let listing = match fs::read_dir(directory) {
        Ok(listing) => listing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            log::warn!("{} could not be read: {error}", directory.display());
            return Vec::new();
        }
    };

    let mut entries: Vec<Entry> = listing
        .filter_map(|entry| match entry {
            Ok(entry) => Some(entry.path()),
            Err(error) => {
                log::warn!("skipping unreadable theme file: {error}");
                None
            }
        })
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .filter_map(|path| match parse_file(&path) {
            Ok(entry) => Some(entry),
            Err(error) => {
                log::warn!("skipping theme {}: {error}", path.display());
                None
            }
        })
        .collect();

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

fn parse_file(path: &Path) -> Result<Entry, Box<dyn std::error::Error>> {
    let contents = fs::read_to_string(path)?;
    let file: ThemeFile = toml::from_str(&contents)?;

    if file.name.trim().is_empty() {
        return Err("`name` must not be empty".into());
    }

    Ok(file.entry())
}

/// A commented example, written out the first time the app runs so there is
/// something to copy rather than a bare empty directory.
pub const EXAMPLE: &str = "\
# Drop a file like this one in this directory and it shows up in the theme
# picker the next time TimeSlay starts. `name` is what the picker displays.
#
# `digits`, `ring` and `flash` are optional; leave them out and they are
# derived from `primary` and `danger`.

name = \"Sakura (copy)\"
mode = \"light\"

background = \"#FFF3F7\"
text       = \"#4A2C39\"
primary    = \"#D6558A\"
success    = \"#6FB07A\"
warning    = \"#E0A54B\"
danger     = \"#D2504F\"

# digits = \"#C13F77\"
# ring   = \"#D6558A\"
# flash  = \"#FF2020\"
";

/// Writes the example theme if the directory has no themes in it yet.
pub fn ensure_example(directory: &Path) {
    let has_themes = fs::read_dir(directory).is_ok_and(|mut listing| {
        listing.any(|entry| {
            entry.is_ok_and(|entry| entry.path().extension().is_some_and(|ext| ext == "toml"))
        })
    });

    if has_themes {
        return;
    }

    let write = fs::create_dir_all(directory)
        .and_then(|()| fs::write(directory.join("example.toml.sample"), EXAMPLE));

    if let Err(error) = write {
        log::debug!("could not write the example theme: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_example_parses() {
        // The sample we write out must actually be valid, minus the fact that
        // it is saved with a `.sample` suffix so it is not loaded by default.
        let file: ThemeFile = toml::from_str(EXAMPLE).unwrap();
        assert_eq!(file.name, "Sakura (copy)");
        assert_eq!(file.primary.to_string(), "#D6558A");
    }

    #[test]
    fn accents_fall_back_to_derived_values() {
        let file: ThemeFile = toml::from_str(EXAMPLE).unwrap();
        let entry = file.entry();
        let derived = Accents::derive(&entry.theme);

        assert_eq!(entry.accents.digits, derived.digits);
    }

    #[test]
    fn explicit_accents_win() {
        let mut file: ThemeFile = toml::from_str(EXAMPLE).unwrap();
        file.digits = Some(HexColor(iced::Color::BLACK));

        assert_eq!(file.entry().accents.digits, iced::Color::BLACK);
    }

    #[test]
    fn unknown_fields_are_rejected_so_typos_are_visible() {
        let result: Result<ThemeFile, _> = toml::from_str(
            r##"
            name = "Typo"
            background = "#FFFFFF"
            text = "#000000"
            primary = "#FF0000"
            success = "#00FF00"
            warning = "#FFFF00"
            danger = "#FF0000"
            primry = "#FF0000"
            "##,
        );

        assert!(result.is_err());
    }

    #[test]
    fn a_missing_directory_yields_no_themes() {
        assert!(load(Path::new("/nonexistent/timeslay/themes")).is_empty());
    }
}
