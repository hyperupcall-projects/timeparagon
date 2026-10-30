//! The command line of the one `timeslay` binary.
//!
//! The side panel, dock and shortcut daemon share this binary.
//! With no flags it opens the side panel.

/// What the binary was asked to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    NewTask,
    /// The wide side panel (the default launch mode).
    #[default]
    Sidebar,
    /// The thin dock strip.
    Dock,
    /// The background process that owns the global shortcuts.
    Daemon,
    /// The native-messaging host the browser extension talks to.
    WebExtension,
}

/// What to do with the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Run { mode: Mode },
    Help,
    Version,
}

pub const HELP: &str = "\
TimeSlay: slay the day with TimeSlay.

Usage: timeslay [OPTIONS]

With no options, TimeSlay opens its side panel.

Options:
      --new-task       Open the full task creation editor\n      --sidebar        Open the side panel: the full app in a wide strip
                       down the edge of the screen (the width and edge are
                       set in the [panel] table of config.toml)
      --dock           Open the dock: a very thin strip of today along the
                       edge of the screen, a few pixels wide (the width
                       is \"Dock width\" in Settings)
      --daemon         Run the background process that owns the global
                       shortcuts (Toggle Visibility and Toggle Shape) and
                       shows, hides and switches the side panel and dock
      --webextension   Run the browser native-messaging host: read one
                       message from the extension on stdin, add its link to
                       the inbox through the running app, and answer on stdout
  -h, --help           Print this help
  -V, --version        Print the version

Only one of --new-task, --sidebar, --dock, --daemon and --webextension can be given.
";

/// Parses the arguments that follow the program name.
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Action, String> {
    let args = args.into_iter();
    let mut mode: Option<Mode> = None;
    // Browsers launch a native-messaging host with the caller's origin as an
    // extra argument, so `--webextension` tolerates arguments it does not
    // recognize instead of failing the way every other mode does.
    let mut unexpected: Vec<String> = Vec::new();

    for arg in args {
        let requested = match arg.as_str() {
            "-h" | "--help" => return Ok(Action::Help),
            "-V" | "--version" => return Ok(Action::Version),
            "--new-task" => Some(Mode::NewTask),
            "--sidebar" => Some(Mode::Sidebar),
            "--dock" => Some(Mode::Dock),
            "--daemon" => Some(Mode::Daemon),
            "--webextension" => Some(Mode::WebExtension),
            _ => None,
        };

        if let Some(requested) = requested {
            match mode {
                Some(existing) if existing != requested => {
                    return Err(
                        "only one of --new-task, --sidebar, --dock, --daemon and --webextension can be given"
                            .into(),
                    );
                }
                _ => mode = Some(requested),
            }
            continue;
        }

        unexpected.push(arg);
    }

    let mode = mode.unwrap_or_default();
    if mode != Mode::WebExtension {
        if let Some(arg) = unexpected.first() {
            return Err(format!("unknown option '{arg}'"));
        }
    }

    Ok(Action::Run { mode })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(list: &[&str]) -> Result<Action, String> {
        parse(list.iter().map(|arg| (*arg).to_owned()))
    }

    fn run(mode: Mode) -> Action {
        Action::Run { mode }
    }

    #[test]
    fn no_arguments_open_the_panel() {
        assert_eq!(parse_all(&[]), Ok(run(Mode::Sidebar)));
    }

    #[test]
    fn each_mode_has_its_own_flag() {
        assert_eq!(parse_all(&["--new-task"]), Ok(run(Mode::NewTask)));
        assert_eq!(parse_all(&["--sidebar"]), Ok(run(Mode::Sidebar)));
        assert_eq!(parse_all(&["--dock"]), Ok(run(Mode::Dock)));
        assert_eq!(parse_all(&["--daemon"]), Ok(run(Mode::Daemon)));
        assert_eq!(parse_all(&["--webextension"]), Ok(run(Mode::WebExtension)));
    }

    #[test]
    fn help_and_version_win_over_everything() {
        assert_eq!(parse_all(&["--dock", "--help"]), Ok(Action::Help));
        assert_eq!(parse_all(&["-h"]), Ok(Action::Help));
        assert_eq!(parse_all(&["-V"]), Ok(Action::Version));
    }

    #[test]
    fn removed_window_origins_are_rejected() {
        for origin in ["window", "sidebar"] {
            assert!(parse_all(&["--dock", "--from", origin]).is_err());
            assert!(parse_all(&["--dock", &format!("--from={origin}")]).is_err());
        }
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

    #[test]
    fn the_webextension_host_tolerates_the_browsers_origin_argument() {
        // Chrome passes "chrome-extension://<id>/"; Firefox passes the add-on
        // id. Neither may be treated as a mistake.
        assert_eq!(
            parse_all(&["--webextension", "chrome-extension://abc/"]),
            Ok(run(Mode::WebExtension)),
        );
        assert_eq!(
            parse_all(&["--webextension", "timeslay@example.org"]),
            Ok(run(Mode::WebExtension)),
        );
    }
}
