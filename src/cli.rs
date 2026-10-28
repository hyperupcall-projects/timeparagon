//! The command line of the one `time-slay` binary.
//!
//! The window, the side panel, the dock and the shortcut daemon are all this
//! binary; the flag picks which one runs. With no flags it is the window.

use crate::dock::launch::Origin;

/// What the binary was asked to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// The regular window.
    #[default]
    Window,
    /// The wide side panel.
    Sidebar,
    /// The thin dock strip.
    Dock,
    /// The background process that owns the global shortcuts.
    Daemon,
}

/// What to do with the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Run { mode: Mode, origin: Origin },
    Help,
    Version,
}

pub const HELP: &str = "\
TimeSlay: slay the day with TimeSlay.

Usage: time-slay [OPTIONS]

With no options, TimeSlay opens its regular window.

Options:
      --sidebar        Open the side panel: the full app in a wide strip
                       down the edge of the screen (the width and edge are
                       set in the [panel] table of config.toml)
      --dock           Open the dock: a very thin strip of today along the
                       edge of the screen, a few pixels wide (the width
                       is \"Dock width\" in Settings)
      --daemon         Run the background process that owns the global
                       shortcuts (Toggle Visibility and Toggle Shape) and
                       shows, hides and switches the side panel and dock
      --from <ORIGIN>  With --dock, what the dock's back button returns to:
                       \"window\" (default) or \"sidebar\"
  -h, --help           Print this help
  -V, --version        Print the version

Only one of --sidebar, --dock and --daemon can be given.
";

/// Parses the arguments that follow the program name.
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Action, String> {
    let mut args = args.into_iter();
    let mut mode: Option<Mode> = None;
    let mut origin: Option<Origin> = None;

    while let Some(arg) = args.next() {
        let requested = match arg.as_str() {
            "-h" | "--help" => return Ok(Action::Help),
            "-V" | "--version" => return Ok(Action::Version),
            "--sidebar" => Some(Mode::Sidebar),
            "--dock" => Some(Mode::Dock),
            "--daemon" => Some(Mode::Daemon),
            _ => None,
        };

        if let Some(requested) = requested {
            match mode {
                Some(existing) if existing != requested => {
                    return Err("only one of --sidebar, --dock and --daemon can be given".into());
                }
                _ => mode = Some(requested),
            }
            continue;
        }

        let value = if arg == "--from" {
            Some(
                args.next()
                    .ok_or_else(|| "--from needs a value: window or sidebar".to_owned())?,
            )
        } else {
            arg.strip_prefix("--from=").map(str::to_owned)
        };
        match value {
            Some(value) => {
                origin = Some(Origin::parse(&value).ok_or_else(|| {
                    format!("unknown --from value '{value}': use window or sidebar")
                })?);
            }
            None => return Err(format!("unknown option '{arg}'")),
        }
    }

    let mode = mode.unwrap_or_default();
    if origin.is_some() && mode != Mode::Dock {
        return Err("--from only applies with --dock".into());
    }

    Ok(Action::Run {
        mode,
        origin: origin.unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(list: &[&str]) -> Result<Action, String> {
        parse(list.iter().map(|arg| (*arg).to_owned()))
    }

    fn run(mode: Mode) -> Action {
        Action::Run {
            mode,
            origin: Origin::Window,
        }
    }

    #[test]
    fn no_arguments_open_the_window() {
        assert_eq!(parse_all(&[]), Ok(run(Mode::Window)));
    }

    #[test]
    fn each_mode_has_its_own_flag() {
        assert_eq!(parse_all(&["--sidebar"]), Ok(run(Mode::Sidebar)));
        assert_eq!(parse_all(&["--dock"]), Ok(run(Mode::Dock)));
        assert_eq!(parse_all(&["--daemon"]), Ok(run(Mode::Daemon)));
    }

    #[test]
    fn help_and_version_win_over_everything() {
        assert_eq!(parse_all(&["--dock", "--help"]), Ok(Action::Help));
        assert_eq!(parse_all(&["-h"]), Ok(Action::Help));
        assert_eq!(parse_all(&["-V"]), Ok(Action::Version));
    }

    #[test]
    fn the_dock_origin_is_read_in_both_spellings() {
        let sidebar = Action::Run {
            mode: Mode::Dock,
            origin: Origin::Sidebar,
        };
        assert_eq!(parse_all(&["--dock", "--from", "sidebar"]), Ok(sidebar));
        assert_eq!(parse_all(&["--dock", "--from=sidebar"]), Ok(sidebar));
    }

    #[test]
    fn mistakes_are_reported() {
        assert!(parse_all(&["--sidebar", "--dock"]).is_err());
        assert!(parse_all(&["--nope"]).is_err());
        assert!(parse_all(&["--dock", "--from"]).is_err());
        assert!(parse_all(&["--dock", "--from=moon"]).is_err());
        assert!(parse_all(&["--sidebar", "--from=window"]).is_err());
    }

    #[test]
    fn repeating_a_flag_is_harmless() {
        assert_eq!(parse_all(&["--dock", "--dock"]), Ok(run(Mode::Dock)));
    }
}
