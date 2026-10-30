//! TimeSlay: slay the day with TimeSlay.

use timeslay::cli::{self, Mode};
use timeslay::{dock, panel, webextension};

fn main() -> iced::Result {
    if iced_webview::cef_subprocess_check() {
        return Ok(());
    }
    let mode = match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Action::Run { mode }) => mode,
        Ok(cli::Action::Help) => {
            print!("{}", cli::HELP);
            return Ok(());
        }
        Ok(cli::Action::Version) => {
            println!("timeslay {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Err(error) => {
            eprintln!("timeslay: {error}\nTry 'timeslay --help'.");
            std::process::exit(2);
        }
    };

    let result = match mode {
        Mode::Dock => dock::run(),
        Mode::Daemon => panel::run(),
        Mode::WebExtension => webextension::run(),
        Mode::Sidebar | Mode::NewTask => {
            env_logger::Builder::from_env(
                env_logger::Env::default().default_filter_or("timeslay=info"),
            )
            .init();
            let result = if mode == Mode::NewTask {
                panel::layer::run_task_popover()
            } else {
                panel::layer::run()
            };
            timeslay::window_effect::shutdown();
            result.map_err(anyhow::Error::from)
        }
    };
    if let Err(error) = result {
        eprintln!("timeslay: {error:#}");
        std::process::exit(1);
    }
    Ok(())
}
