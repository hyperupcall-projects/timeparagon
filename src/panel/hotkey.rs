//! Global shortcut registration via KDE's KGlobalAccel D-Bus service.
//!
//! On Wayland a client cannot grab global keys itself — the compositor owns the
//! keyboard. KGlobalAccel is KDE's broker: we register a *component* and an
//! *action*, KWin performs the actual grab, and the daemon emits
//! `globalShortcutPressed` when the user presses the key. This is the same
//! mechanism the side panel uses, and it works on Wayland.
//!
//! Each action ([`Action`]) can have several shortcuts. They also show up in
//! System Settings → Shortcuts, so the user can rebind them without touching
//! our config file.

use anyhow::{Context, Result, anyhow};
use std::sync::mpsc::Sender;

use super::config::{Action, PanelConfig};

/// Component (application) id used with KGlobalAccel.
pub const COMPONENT: &str = "timetracker";
const COMPONENT_FRIENDLY: &str = "TimeTracker";

/// The one action older versions registered, "Toggle Side Panel". Its binding
/// carries over to [`Action::ToggleVisibility`] and the action is removed.
const LEGACY_ACTION: &str = "panel";
const LEGACY_ACTION_FRIENDLY: &str = "Toggle Side Panel";

/// `KGlobalAccelD::SetShortcutFlag` values (see `kglobalacceld.h`).
///
/// `SetPresent` marks the shortcut as active. `IsDefault` records it as the
/// action's default binding (what "Reset" in System Settings restores).
const SET_PRESENT: u32 = 0x2;
const IS_DEFAULT: u32 = 0x8;
/// Apply an explicit user edit instead of reloading the saved binding.
const NO_AUTOLOADING: u32 = 0x4;

/// The four-string id KGlobalAccel identifies an action by.
fn action_id(action: Action) -> [&'static str; 4] {
    [COMPONENT, action.id(), COMPONENT_FRIENDLY, action.label()]
}

#[zbus::proxy(
    interface = "org.kde.KGlobalAccel",
    default_service = "org.kde.kglobalaccel",
    default_path = "/kglobalaccel"
)]
trait KGlobalAccel {
    // KGlobalAccel uses camelCase D-Bus names; zbus would otherwise PascalCase
    // the Rust method names, so spell them out explicitly.
    #[zbus(name = "doRegister")]
    fn do_register(&self, action_id: &[&str]) -> zbus::Result<()>;

    // The D-Bus signature is `a(ai)` — an array of structs each holding one
    // `QKeySequence`. A 1-tuple gives us that struct wrapper; `Vec<Vec<i32>>`
    // would serialise as `aai` and be rejected. Each inner array must hold
    // exactly four ints; see `encode_sequence`.
    #[zbus(name = "setShortcutKeys")]
    fn set_shortcut_keys(
        &self,
        action_id: &[&str],
        keys: &[(Vec<i32>,)],
        flags: u32,
    ) -> zbus::Result<Vec<(Vec<i32>,)>>;

    #[zbus(name = "shortcutKeys")]
    fn shortcut_keys(&self, action_id: &[&str]) -> zbus::Result<Vec<(Vec<i32>,)>>;

    #[zbus(name = "unregister")]
    fn unregister(&self, component_unique: &str, action_unique: &str) -> zbus::Result<bool>;

    #[zbus(name = "getComponent")]
    fn get_component(
        &self,
        component_unique: &str,
    ) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

#[zbus::proxy(
    interface = "org.kde.kglobalaccel.Component",
    default_service = "org.kde.kglobalaccel"
)]
trait Component {
    #[zbus(name = "globalShortcutPressed", signal)]
    fn global_shortcut_pressed(
        &self,
        component_unique: &str,
        action_unique: &str,
        timestamp: i64,
    ) -> zbus::Result<()>;
}

/// Replaces every shortcut of `action` with `specs`, which may be empty.
/// KGlobalAccel persists them across daemon restarts; startup deliberately
/// respects those saved bindings.
pub fn rebind(action: Action, specs: &[String]) -> Result<()> {
    let keys = encode_all(specs)?;
    let conn = zbus::blocking::Connection::session()
        .context("Could not connect to the desktop session")?;
    let kga = KGlobalAccelProxyBlocking::new(&conn)?;
    // Do not create an action owned by this temporary connection: the daemon
    // must already own it and be listening for presses.
    kga.get_component(COMPONENT)
        .context("Shortcut daemon unavailable. Start timetracker-shortcuts.service and try again")?;
    let id = action_id(action);
    let previous = kga
        .shortcut_keys(&id)
        .context("Could not read the current shortcuts")?;
    let applied = kga
        .set_shortcut_keys(&id, &keys, SET_PRESENT | NO_AUTOLOADING)
        .context("Could not change the shortcuts")?;
    if !same_keys(&applied, &keys) {
        kga.set_shortcut_keys(&id, &previous, SET_PRESENT | NO_AUTOLOADING)
            .context("Shortcut unavailable, and restoring the previous shortcuts failed")?;
        anyhow::bail!(
            "Shortcut unavailable. It may already be used by another application; try another combination"
        );
    }
    Ok(())
}

/// Whether `applied` holds exactly the `wanted` sequences, in any order.
fn same_keys(applied: &[(Vec<i32>,)], wanted: &[(Vec<i32>,)]) -> bool {
    applied.len() == wanted.len() && wanted.iter().all(|key| applied.contains(key))
}

/// Checks that `spec` is a shortcut that can be bound.
pub fn validate(spec: &str) -> Result<()> {
    parse_shortcut(spec).map(drop)
}

/// Whether two spellings are the same key combination, e.g. `Meta+U` and
/// `super + u`.
pub fn same_shortcut(a: &str, b: &str) -> bool {
    matches!((parse_shortcut(a), parse_shortcut(b)), (Ok(a), Ok(b)) if a == b)
}

/// Encodes every shortcut for the wire, dropping repeats.
fn encode_all(specs: &[String]) -> Result<Vec<(Vec<i32>,)>> {
    let mut keys: Vec<(Vec<i32>,)> = Vec::new();
    for spec in specs {
        let key = (encode_sequence(parse_shortcut(spec)?),);
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    Ok(keys)
}

/// Spawns the background thread that registers the shortcuts and forwards
/// presses to `sender`.
pub fn spawn(sender: Sender<Action>, config: PanelConfig) -> Result<()> {
    std::thread::Builder::new()
        .name("kglobalaccel".into())
        .spawn(move || {
            if let Err(err) = run(sender, &config) {
                log::error!("global-shortcut listener stopped: {err:#}");
            }
        })
        .context("failed to spawn the global-shortcut thread")?;
    Ok(())
}

fn run(sender: Sender<Action>, config: &PanelConfig) -> Result<()> {
    let conn =
        zbus::blocking::Connection::session().context("failed to connect to the session bus")?;
    let kga = KGlobalAccelProxyBlocking::new(&conn)
        .context("KGlobalAccel is unavailable (is kglobalacceld running?)")?;

    // What the old single action was bound to, so a binding the user chose in
    // System Settings is not lost to the new defaults.
    let legacy_id = [
        COMPONENT,
        LEGACY_ACTION,
        COMPONENT_FRIENDLY,
        LEGACY_ACTION_FRIENDLY,
    ];
    let legacy_keys = kga.shortcut_keys(&legacy_id).unwrap_or_default();
    // Removed before the new actions are bound, because it would otherwise
    // still claim its key, and linger in System Settings.
    let _ = kga.unregister(COMPONENT, LEGACY_ACTION);

    for action in Action::ALL {
        let id = action_id(action);
        kga.do_register(&id)
            .context("failed to register the component with KGlobalAccel")?;

        let wanted = encode_all(config.shortcuts(action))?;

        // A binding that is already saved (maybe edited in System Settings)
        // wins over the config, which only fills in an action with none.
        let saved = kga.shortcut_keys(&id).unwrap_or_default();
        let keys = if !saved.is_empty() {
            saved
        } else if action == Action::ToggleVisibility && !legacy_keys.is_empty() {
            legacy_keys.clone()
        } else {
            wanted.clone()
        };

        let applied = kga
            .set_shortcut_keys(&id, &keys, SET_PRESENT)
            .context("failed to set the global shortcut")?;

        // Also record our binding as the action's default, so it shows up in
        // System Settings → Shortcuts and can be restored there.
        kga.set_shortcut_keys(&id, &wanted, IS_DEFAULT)
            .context("failed to set the default shortcut")?;

        log::info!(
            "global shortcuts for '{}' registered (active bindings: {})",
            action.label(),
            describe(&applied)
        );
    }

    let path = kga
        .get_component(COMPONENT)
        .context("failed to resolve the component object")?;
    let component = ComponentProxyBlocking::builder(&conn)
        .path(path.as_str())
        .context("invalid component object path")?
        .build()
        .context("failed to build the component proxy")?;

    let signals = component
        .receive_global_shortcut_pressed()
        .context("failed to subscribe to globalShortcutPressed")?;
    for signal in signals {
        let args = signal.args().context("failed to decode the signal")?;
        if let Some(action) = Action::from_id(args.action_unique())
            && sender.send(action).is_err()
        {
            break;
        }
    }
    Ok(())
}

/// Parses a `Modifier+Key` shortcut into a Qt "combined" key code.
///
/// KGlobalAccel speaks Qt's `QKeySequence` encoding: the modifier flags live in
/// the high bits and the key code in the low bits, OR-ed together. See
/// `sequencehelpers_p.cpp` in the kglobalaccel sources.
fn parse_shortcut(spec: &str) -> Result<i32> {
    const SHIFT: i32 = 0x0200_0000;
    const CTRL: i32 = 0x0400_0000;
    const ALT: i32 = 0x0800_0000;
    const META: i32 = 0x1000_0000;

    let mut modifiers = 0i32;
    let mut key: Option<i32> = None;

    for raw in spec.split('+') {
        let part = raw.trim();
        anyhow::ensure!(!part.is_empty(), "Shortcut contains an empty key");
        match part.to_ascii_lowercase().as_str() {
            "meta" | "super" | "logo" | "win" => modifiers |= META,
            "ctrl" | "control" => modifiers |= CTRL,
            "alt" => modifiers |= ALT,
            "shift" => modifiers |= SHIFT,
            other => {
                anyhow::ensure!(key.is_none(), "Use exactly one key with optional modifiers");
                key =
                    Some(key_code(other).ok_or_else(|| {
                        anyhow!("unsupported key '{other}' in shortcut '{spec}'")
                    })?);
            }
        }
    }

    let key = key.ok_or_else(|| anyhow!("shortcut '{spec}' does not contain a key"))?;
    anyhow::ensure!(
        modifiers != 0 || (0x0100_0030..=0x0100_003b).contains(&key),
        "Use Ctrl, Alt, Shift or Meta with the key, or use F1–F12"
    );
    Ok(modifiers | key)
}

/// Maps a key name to its Qt `Qt::Key` code.
fn key_code(name: &str) -> Option<i32> {
    // Single ASCII letter or digit.
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_alphabetic() {
            return Some(c.to_ascii_uppercase() as i32);
        }
        if c.is_ascii_digit() {
            return Some(c as i32);
        }
    }

    Some(match name {
        "space" => 0x20,
        "escape" | "esc" => 0x0100_0000,
        "tab" => 0x0100_0001,
        "backspace" => 0x0100_0003,
        "return" | "enter" => 0x0100_0004,
        "delete" | "del" => 0x0100_0007,
        "home" => 0x0100_0010,
        "end" => 0x0100_0011,
        "left" => 0x0100_0012,
        "up" => 0x0100_0013,
        "right" => 0x0100_0014,
        "down" => 0x0100_0015,
        "pageup" => 0x0100_0016,
        "pagedown" => 0x0100_0017,
        "f1" => 0x0100_0030,
        "f2" => 0x0100_0031,
        "f3" => 0x0100_0032,
        "f4" => 0x0100_0033,
        "f5" => 0x0100_0034,
        "f6" => 0x0100_0035,
        "f7" => 0x0100_0036,
        "f8" => 0x0100_0037,
        "f9" => 0x0100_0038,
        "f10" => 0x0100_0039,
        "f11" => 0x0100_003a,
        "f12" => 0x0100_003b,
        _ => return None,
    })
}

/// Encodes a Qt "combined" key code as a `QKeySequence` for the D-Bus wire.
///
/// KGlobalAccel marshals a `QKeySequence` as a struct wrapping an array of
/// **exactly four** int32s — a chord of up to four keys, zero-padded. Qt's
/// demarshaller reads all four unconditionally (it never checks for the end of
/// the array), so a shorter array makes the receiver read past the end and abort
/// the whole process on a libdbus assertion. Always pad to four.
fn encode_sequence(combined: i32) -> Vec<i32> {
    vec![combined, 0, 0, 0]
}

/// Renders the keys returned by `setShortcutKeys` for logging.
fn describe(applied: &[(Vec<i32>,)]) -> String {
    if applied.is_empty() {
        return "none".to_string();
    }
    applied
        .iter()
        .map(|(seq,)| match seq.first() {
            Some(code) => format!("0x{code:08x}"),
            None => "none".to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::zvariant::{Endian, Type, serialized::Context, to_bytes};

    #[test]
    fn meta_u_parses_to_the_expected_combined_code() {
        assert_eq!(parse_shortcut("Meta+U").unwrap(), 0x1000_0055);
    }

    #[test]
    fn the_defaults_are_valid_bindings() {
        let default = PanelConfig::default();

        assert_eq!(
            parse_shortcut(&default.toggle_visibility[0]).unwrap(),
            0x1000_0055
        );
        assert_eq!(
            parse_shortcut(&default.toggle_shape[0]).unwrap(),
            0x1200_0055
        );
    }

    #[test]
    fn several_shortcuts_are_encoded_without_repeats() {
        let specs = ["Meta+J", "Meta+F", "super + j"].map(String::from);
        let keys = encode_all(&specs).unwrap();

        assert_eq!(
            keys,
            vec![
                (encode_sequence(0x1000_004a),),
                (encode_sequence(0x1000_0046),)
            ]
        );
        assert!(encode_all(&["Nope".to_owned()]).is_err());
    }

    #[test]
    fn applied_keys_are_compared_without_regard_to_order() {
        let j = (encode_sequence(0x1000_004a),);
        let f = (encode_sequence(0x1000_0046),);

        assert!(same_keys(&[f.clone(), j.clone()], &[j.clone(), f.clone()]));
        assert!(!same_keys(&[j.clone()], &[j, f]));
        assert!(same_keys(&[], &[]));
    }

    #[test]
    fn spellings_of_one_combination_are_the_same_shortcut() {
        assert!(same_shortcut("Meta+U", " super + u "));
        assert!(!same_shortcut("Meta+U", "Meta+J"));
        assert!(!same_shortcut("Meta+U", "nonsense"));
    }

    #[test]
    fn custom_combinations_and_function_keys_parse() {
        assert_eq!(parse_shortcut("Ctrl+Alt+T").unwrap(), 0x0c00_0054);
        assert_eq!(parse_shortcut(" super + shift + u ").unwrap(), 0x1200_0055);
        assert_eq!(parse_shortcut("F2").unwrap(), 0x0100_0031);
        assert_eq!(parse_shortcut("Meta+Space").unwrap(), 0x1000_0020);
    }

    #[test]
    fn malformed_or_unmodified_typing_keys_are_rejected() {
        for spec in [
            "",
            "Meta",
            "Meta+",
            "Ctrl++T",
            "Meta+U+T",
            "U",
            "Space",
            "Meta+Unknown",
        ] {
            assert!(parse_shortcut(spec).is_err(), "{spec:?} should be rejected");
        }
    }

    #[test]
    fn sequence_is_padded_to_four_ints() {
        assert_eq!(encode_sequence(0x1000_0055), vec![0x1000_0055, 0, 0, 0]);
    }

    #[test]
    fn keys_argument_has_the_a_ai_signature() {
        assert_eq!(<(Vec<i32>,) as Type>::SIGNATURE, "(ai)");
        assert_eq!(<Vec<(Vec<i32>,)> as Type>::SIGNATURE, "a(ai)");
    }

    /// Regression test for the KWin crash: Qt's `QKeySequence` demarshaller reads
    /// exactly four ints, so the wire form must always carry four.
    #[test]
    fn keys_round_trip_with_four_ints() {
        let keys: Vec<(Vec<i32>,)> = vec![(encode_sequence(0x1000_0055),)];
        let ctxt = Context::new_dbus(Endian::Little, 0);
        let data = to_bytes(ctxt, &keys).unwrap();
        let (decoded, _): (Vec<(Vec<i32>,)>, usize) = data.deserialize().unwrap();
        assert_eq!(decoded, keys);
        assert_eq!(decoded[0].0.len(), 4);
    }
}
