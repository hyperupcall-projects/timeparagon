//! The command line of the one `timeslay` binary.
//!
//! The side panel, dock and shortcut daemon share this binary.
//! With no command it opens the side panel.

use clap::{ArgGroup, Parser, Subcommand};

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
    /// Print what keeping time blocks in step would change, and exit.
    CheckTimeBlocks,
}

/// The commands that pick a running mode.
#[derive(Debug, Subcommand)]
enum ModeCommand {
    /// Run the browser native-messaging host: read one message from the
    /// extension on stdin, add its link to the inbox through the running app,
    /// and answer on stdout
    Webextension {
        // Browsers launch a native-messaging host with the caller's origin as
        // an extra argument. The host does not need it, but taking it here
        // keeps a wrapper that forwards its arguments working.
        #[arg(hide = true)]
        origin: Vec<String>,
    },

    /// Run the background process that listens to global keyboard shortcuts and launches/hides the dock and sidebar.
    Daemon,

    /// Launch the thin dock.
    Dock,

    /// Launch the wide sidebar.
    Sidebar,
}

/// The command line, as declared to clap.
#[derive(Debug, Parser)]
#[command(
    name = "timeslay",
    version,
    about = "TimeSlay: slay the day with TimeSlay.",
    after_help = "With no options, TimeSlay opens its side panel.",
    args_conflicts_with_subcommands = true,
    group(ArgGroup::new("mode").args(["new_task", "check_time_blocks"]))
)]
struct Args {
    /// Open the full task creation editor
    #[arg(long, overrides_with = "new_task")]
    new_task: bool,

    /// Print what keeping time blocks in step with Google would change over
    /// the next eight weeks, and any task that repeats on its own inside a
    /// block, without changing anything
    #[arg(long, overrides_with = "check_time_blocks")]
    check_time_blocks: bool,

    #[command(subcommand)]
    command: Option<ModeCommand>,
}

/// Parses the arguments, which include the program name first.
///
/// `--help` and `--version` come back as errors that print and exit
/// successfully through [`clap::Error::exit`].
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Mode, clap::Error> {
    let args = Args::try_parse_from(args)?;
    Ok(match args.command {
        Some(ModeCommand::Webextension { origin: _origin }) => Mode::WebExtension,
        Some(ModeCommand::Daemon) => Mode::Daemon,
        Some(ModeCommand::Dock) => Mode::Dock,
        Some(ModeCommand::Sidebar) => Mode::Sidebar,
        None if args.new_task => Mode::NewTask,
        None if args.check_time_blocks => Mode::CheckTimeBlocks,
        None => Mode::default(),
    })
}

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;

    use super::*;

    fn parse_all(list: &[&str]) -> Result<Mode, clap::Error> {
        parse(
            std::iter::once("timeslay")
                .chain(list.iter().copied())
                .map(str::to_owned),
        )
    }

    #[test]
    fn checking_time_blocks_is_its_own_mode() {
        assert_eq!(
            parse_all(&["--check-time-blocks"]).unwrap(),
            Mode::CheckTimeBlocks
        );
        assert!(parse_all(&["--check-time-blocks", "dock"]).is_err());
    }

    #[test]
    fn no_arguments_open_the_panel() {
        assert_eq!(parse_all(&[]).unwrap(), Mode::Sidebar);
    }

    #[test]
    fn each_mode_has_its_own_command() {
        assert_eq!(parse_all(&["--new-task"]).unwrap(), Mode::NewTask);
        assert_eq!(parse_all(&["sidebar"]).unwrap(), Mode::Sidebar);
        assert_eq!(parse_all(&["dock"]).unwrap(), Mode::Dock);
        assert_eq!(parse_all(&["daemon"]).unwrap(), Mode::Daemon);
        assert_eq!(parse_all(&["webextension"]).unwrap(), Mode::WebExtension);
    }

    #[test]
    fn help_and_version_win_over_everything() {
        let kind = |args: &[&str]| parse_all(args).unwrap_err().kind();
        assert_eq!(kind(&["dock", "--help"]), ErrorKind::DisplayHelp);
        assert_eq!(kind(&["-h"]), ErrorKind::DisplayHelp);
        assert_eq!(kind(&["-V"]), ErrorKind::DisplayVersion);
    }

    #[test]
    fn removed_window_origins_are_rejected() {
        for origin in ["window", "sidebar"] {
            assert!(parse_all(&["dock", "--from", origin]).is_err());
            assert!(parse_all(&["dock", &format!("--from={origin}")]).is_err());
        }
    }

    #[test]
    fn mistakes_are_reported() {
        assert!(parse_all(&["sidebar", "dock"]).is_err());
        assert!(parse_all(&["--nope"]).is_err());
        assert!(parse_all(&["dock", "--from"]).is_err());
        assert!(parse_all(&["dock", "--from=moon"]).is_err());
        assert!(parse_all(&["--new-task", "dock"]).is_err());
    }

    #[test]
    fn repeating_a_flag_is_harmless() {
        assert_eq!(
            parse_all(&["--new-task", "--new-task"]).unwrap(),
            Mode::NewTask
        );
    }

    #[test]
    fn only_the_webextension_host_takes_positionals() {
        assert!(parse_all(&["dock", "chrome-extension://abc/"]).is_err());
        assert!(parse_all(&["sidebar", "extra"]).is_err());
    }

    #[test]
    fn the_webextension_host_tolerates_the_browsers_origin_argument() {
        // Chrome passes "chrome-extension://<id>/"; Firefox passes the add-on
        // id. Neither may be treated as a mistake.
        assert_eq!(
            parse_all(&["webextension", "chrome-extension://abc/"]).unwrap(),
            Mode::WebExtension
        );
        assert_eq!(
            parse_all(&["webextension", "timeslay@example.org"]).unwrap(),
            Mode::WebExtension
        );
    }
}
