//! TimeSlay: slay the day with TimeSlay.

use timeslay::cli::{self, Mode};
use timeslay::{dock, panel, webextension};

fn main() -> iced::Result {
	if iced_webview::cef_subprocess_check() {
		return Ok(());
	}
	let mode = match cli::parse(std::env::args()) {
		Ok(mode) => mode,
		Err(error) => error.exit(),
	};

	let result = match mode {
		Mode::Help => cli::print_help().map_err(anyhow::Error::from),
		Mode::Dock => dock::run(),
		Mode::Daemon => panel::run(),
		Mode::WebExtension => webextension::run(),
		Mode::Sidebar => {
			env_logger::Builder::from_env(
				env_logger::Env::default().default_filter_or("timeslay=info"),
			)
			.init();
			let result = panel::layer::run();
			timeslay::window_effect::shutdown();
			result.map_err(anyhow::Error::from)
		}
		Mode::Add => {
			env_logger::Builder::from_env(
				env_logger::Env::default().default_filter_or("timeslay=info"),
			)
			.init();
			let result = panel::layer::run_task_popover();
			timeslay::window_effect::shutdown();
			result.map_err(anyhow::Error::from)
		}
		Mode::CheckTimeBlocks => {
			let settings = timeslay_core::config::Config::load().calendar;
			timeslay_core::calendar::api::block_sync::report(&settings)
				.map(|report| print!("{report}"))
				.map_err(anyhow::Error::msg)
		}
	};
	if let Err(error) = result {
		eprintln!("timeslay: {error:#}");
		std::process::exit(1);
	}
	Ok(())
}
