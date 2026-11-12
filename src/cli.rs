//! The command line of the one `timeparagon` binary.
//!
//! The side panel, dock and shortcut daemon share this binary.
//! With no command it prints the help.

use clap::{ArgGroup, CommandFactory, Parser, Subcommand};

/// The selected command (or flag) to run on the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
	/// Print the help and exit.
	Help,
	/// The wide side panel.
	Sidebar,
	/// The thin dock.
	Dock,
	/// The background process that requests to listen to the global keyboard shortcuts.
	Daemon,
	/// The browser extensions' native messaging host.
	WebExtension,
	/// Opens the menu to add a new task.
	Add,
	/// Print what keeping time blocks in step would change, and exit.
	CheckTimeBlocks,
}

/// The commands that pick a running mode.
#[derive(Debug, Subcommand)]
enum ModeCommand {
	/// Run the browser extension's native messaging host
	Webextension {
		// Browsers launch a native-messaging host with the caller's origin as
		// an extra argument. The host does not need it, but taking it here
		// keeps a wrapper that forwards its arguments working.
		#[arg(hide = true)]
		origin: Vec<String>,
	},

	/// Run the background process that requests to listens to global keyboard shortcuts.
	Daemon,

	/// Launch the thin dock.
	Dock,

	/// Launch the wide sidebar.
	Sidebar,

	/// Launch the menu to add a new task.
	Add,
}

/// The command line, as declared to clap.
#[derive(Debug, Parser)]
#[command(
    name = "timeparagon",
    version,
    about = "TimeParagon: Master the day with TimeParagon.",
    args_conflicts_with_subcommands = true,
    group(ArgGroup::new("mode").args(["check_time_blocks"]))
)]
struct Args {
	#[command(subcommand)]
	command: Option<ModeCommand>,

	/// Print what keeping time blocks in step with Google would change over
	/// the next eight weeks, and any task that repeats on its own inside a
	/// block, without changing anything
	#[arg(long, overrides_with = "check_time_blocks")]
	check_time_blocks: bool,
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
		Some(ModeCommand::Add) => Mode::Add,
		None if args.check_time_blocks => Mode::CheckTimeBlocks,
		None => Mode::Help,
	})
}

/// Prints the command line's help to stdout.
pub fn print_help() -> std::io::Result<()> {
	Args::command().print_help()
}

#[cfg(test)]
mod tests {
	use clap::error::ErrorKind;

	use super::*;

	fn parse_all(list: &[&str]) -> Result<Mode, clap::Error> {
		parse(
			std::iter::once("timeparagon")
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
	fn no_arguments_show_the_help() {
		assert_eq!(parse_all(&[]).unwrap(), Mode::Help);
	}

	#[test]
	fn each_mode_has_its_own_command() {
		assert_eq!(parse_all(&["sidebar"]).unwrap(), Mode::Sidebar);
		assert_eq!(parse_all(&["dock"]).unwrap(), Mode::Dock);
		assert_eq!(parse_all(&["daemon"]).unwrap(), Mode::Daemon);
		assert_eq!(parse_all(&["webextension"]).unwrap(), Mode::WebExtension);
		assert_eq!(parse_all(&["add"]).unwrap(), Mode::Add);
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
		assert!(parse_all(&["--check-time-blocks", "dock"]).is_err());
	}

	#[test]
	fn repeating_a_flag_is_harmless() {
		assert_eq!(
			parse_all(&["--check-time-blocks", "--check-time-blocks"]).unwrap(),
			Mode::CheckTimeBlocks
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
			parse_all(&["webextension", "timeparagon@example.org"]).unwrap(),
			Mode::WebExtension
		);
	}
}
