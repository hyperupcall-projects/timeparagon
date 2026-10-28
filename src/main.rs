//! TimeTracker: a calendar with alarms that stays out of the way.

use timetracker::cli::{self, Mode};
use timetracker::{app, dock, panel};

/// The boot function the daemon starts from.
type Boot = fn() -> (app::State, iced::Task<app::Message>);

fn main() -> iced::Result {
    let (mode, origin) = match cli::parse(std::env::args().skip(1)) {
        Ok(cli::Action::Run { mode, origin }) => (mode, origin),
        Ok(cli::Action::Help) => {
            print!("{}", cli::HELP);
            return Ok(());
        }
        Ok(cli::Action::Version) => {
            println!("timetracker {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Err(error) => {
            eprintln!("timetracker: {error}\nTry 'timetracker --help'.");
            std::process::exit(2);
        }
    };

    // The dock and the shortcut daemon are not iced programs, and set up
    // their own logging.
    let result = match mode {
        Mode::Dock => Some(dock::run(origin)),
        Mode::Daemon => Some(panel::run()),
        Mode::Window | Mode::Sidebar => None,
    };
    if let Some(result) = result {
        if let Err(error) = result {
            eprintln!("timetracker: {error:#}");
            std::process::exit(1);
        }
        return Ok(());
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("timetracker=info"))
        .init();

    let boot: Boot = if mode == Mode::Sidebar {
        app::boot_sidebar
    } else {
        app::boot
    };

    // A daemon rather than an application: it opens no window on its own, so
    // the clock and the flash overlay are peers, and it does not quit the
    // moment the overlay closes.
    iced::daemon(boot, app::update, app::view)
        .title(app::title)
        .theme(app::active_theme)
        .style(|_state, theme| {
            let mut style = iced::theme::default(theme);
            style.background_color = iced::Color::TRANSPARENT;
            style
        })
        .subscription(app::subscription)
        .run()
}
