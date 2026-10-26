//! TimeTracker: a count-up timer that stays out of the way.

use timetracker::app;

/// The boot function the daemon starts from.
type Boot = fn() -> (app::State, iced::Task<app::Message>);

fn main() -> iced::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("timetracker=info"))
        .init();

    // The same binary is both the main window and the side panel; the switch
    // is how `timetracker-panel` opens a panel instance of the real app.
    let sidebar = std::env::args().skip(1).any(|arg| arg == "--sidebar");
    let boot: Boot = if sidebar {
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
        .subscription(app::subscription)
        .run()
}
