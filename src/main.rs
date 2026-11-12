//! TimeParagon: master the day with TimeParagon.

use timeparagon::cli::{self, Mode};
use timeparagon::{dock, panel, webextension};

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
				env_logger::Env::default().default_filter_or("timeparagon=info"),
			)
			.init();
			let result = panel::layer::run();
			timeparagon::window_effect::shutdown();
			result.map_err(anyhow::Error::from)
		}
		Mode::Add => {
			env_logger::Builder::from_env(
				env_logger::Env::default().default_filter_or("timeparagon=info"),
			)
			.init();
			let result = panel::layer::run_task_popover();
			timeparagon::window_effect::shutdown();
			result.map_err(anyhow::Error::from)
		}
		Mode::CheckTimeBlocks => {
			let settings = timeparagon_core::config::Config::load().calendar;
			timeparagon_core::calendar::api::block_sync::report(&settings)
				.map(|report| print!("{report}"))
				.map_err(anyhow::Error::msg)
		}
	};
	if let Err(error) = result {
		eprintln!("timeparagon: {error:#}");
		std::process::exit(1);
	}
	Ok(())
}
