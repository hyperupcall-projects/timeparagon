//! TimeSlay: slay the day with TimeSlay.

use timeslay::cli::{self, Mode};
use timeslay::{app, dock, panel, webextension};

fn main() -> iced::Result {
    if iced_webview::cef_subprocess_check() {
        return Ok(());
    }
    let (mode, origin) = match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Action::Run { mode, origin }) => (mode, origin),
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

    // The dock and the shortcut daemon are not iced programs, and set up
    // their own logging.
    let result = match mode {
        Mode::Dock => Some(dock::run(origin)),
        Mode::Daemon => Some(panel::run()),
        Mode::WebExtension => Some(webextension::run()),
        Mode::Window | Mode::Sidebar | Mode::NewTask => None,
    };
    if let Some(result) = result {
        if let Err(error) = result {
            eprintln!("timeslay: {error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("timeslay=info"))
        .init();

    if matches!(mode, Mode::Sidebar | Mode::NewTask) {
        let result = if mode == Mode::NewTask {
            panel::layer::run_task_popover()
        } else {
            panel::layer::run()
        };
        timeslay::window_effect::shutdown();
        if let Err(error) = result {
            eprintln!("timeslay: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }

    // A daemon rather than an application: it opens no window on its own, so
    // the clock and the flash overlay are peers, and it does not quit the
    // moment the overlay closes.
    let result = iced::daemon(app::boot, app::update, app::view)
        .antialiasing(true)
        .title(app::title)
        .theme(app::active_theme)
        .style(|_state, theme| {
            let mut style = iced::theme::default(theme);
            style.background_color = iced::Color::TRANSPARENT;
            style
        })
        .subscription(app::subscription)
        .run();

    timeslay::window_effect::shutdown();
    result
}
