//! The command line of the one `timeslay` binary.
//!
//! The side panel, dock and shortcut daemon share this binary.
//! With no flags it opens the side panel.

use clap::error::ErrorKind;
use clap::{ArgGroup, CommandFactory, Parser};

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

/// The command line, as declared to clap.
#[derive(Debug, Parser)]
#[command(
    name = "timeslay",
    version,
    about = "TimeSlay: slay the day with TimeSlay.",
    after_help = "With no options, TimeSlay opens its side panel.",
    group(ArgGroup::new("mode").args(["new_task", "sidebar", "dock", "daemon", "webextension"]))
)]
struct Args {
    /// Open the full task creation editor
    #[arg(long, overrides_with = "new_task")]
    new_task: bool,

    /// Open the side panel: the full app in a wide strip down the edge of the
    /// screen (the width and edge are set in the [panel] table of config.toml)
    #[arg(long, overrides_with = "sidebar")]
    sidebar: bool,

    /// Open the dock: a very thin strip of today along the edge of the screen,
    /// a few pixels wide (the width is "Dock width" in Settings)
    #[arg(long, overrides_with = "dock")]
    dock: bool,

    /// Run the background process that owns the global shortcuts (Toggle
    /// Visibility and Toggle Shape) and shows, hides and switches the side
    /// panel and dock
    #[arg(long, overrides_with = "daemon")]
    daemon: bool,

    /// Run the browser native-messaging host: read one message from the
    /// extension on stdin, add its link to the inbox through the running app,
    /// and answer on stdout
    #[arg(long, overrides_with = "webextension")]
    webextension: bool,

    // Browsers launch a native-messaging host with the caller's origin as an
    // extra argument, so `--webextension` tolerates positional arguments
    // instead of failing the way every other mode does.
    #[arg(hide = true)]
    origin: Vec<String>,
}

/// Parses the arguments, which include the program name first.
///
/// `--help` and `--version` come back as errors that print and exit
/// successfully through [`clap::Error::exit`].
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Mode, clap::Error> {
    let args = Args::try_parse_from(args)?;
    let mode = if args.new_task {
        Mode::NewTask
    } else if args.sidebar {
        Mode::Sidebar
    } else if args.dock {
        Mode::Dock
    } else if args.daemon {
        Mode::Daemon
    } else if args.webextension {
        Mode::WebExtension
    } else {
        Mode::default()
    };

    if mode != Mode::WebExtension && !args.origin.is_empty() {
        return Err(Args::command().error(
            ErrorKind::UnknownArgument,
            format!("unexpected argument '{}'", args.origin[0]),
        ));
    }
    Ok(mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(list: &[&str]) -> Result<Mode, clap::Error> {
        parse(
            std::iter::once("timeslay")
                .chain(list.iter().copied())
                .map(str::to_owned),
        )
    }

    #[test]
    fn no_arguments_open_the_panel() {
        assert_eq!(parse_all(&[]).unwrap(), Mode::Sidebar);
    }

    #[test]
    fn each_mode_has_its_own_flag() {
        assert_eq!(parse_all(&["--new-task"]).unwrap(), Mode::NewTask);
        assert_eq!(parse_all(&["--sidebar"]).unwrap(), Mode::Sidebar);
        assert_eq!(parse_all(&["--dock"]).unwrap(), Mode::Dock);
        assert_eq!(parse_all(&["--daemon"]).unwrap(), Mode::Daemon);
        assert_eq!(parse_all(&["--webextension"]).unwrap(), Mode::WebExtension);
    }

    #[test]
    fn help_and_version_win_over_everything() {
        let kind = |args: &[&str]| parse_all(args).unwrap_err().kind();
        assert_eq!(kind(&["--dock", "--help"]), ErrorKind::DisplayHelp);
        assert_eq!(kind(&["-h"]), ErrorKind::DisplayHelp);
        assert_eq!(kind(&["-V"]), ErrorKind::DisplayVersion);
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
        assert_eq!(parse_all(&["--dock", "--dock"]).unwrap(), Mode::Dock);
    }

    #[test]
    fn the_webextension_host_tolerates_the_browsers_origin_argument() {
        // Chrome passes "chrome-extension://<id>/"; Firefox passes the add-on
        // id. Neither may be treated as a mistake.
        assert_eq!(
            parse_all(&["--webextension", "chrome-extension://abc/"]).unwrap(),
            Mode::WebExtension
        );
        assert_eq!(
            parse_all(&["--webextension", "timeslay@example.org"]).unwrap(),
            Mode::WebExtension
        );
    }
}
