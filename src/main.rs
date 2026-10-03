//! TimeTracker: a count-up timer that stays out of the way.

use timetracker::app;

fn main() -> iced::Result {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("timetracker=info"),
    )
    .init();

    // A daemon rather than an application: it opens no window on its own, so
    // the clock and the flash overlay are peers, and it does not quit the
    // moment the overlay closes.
    iced::daemon(app::boot, app::update, app::view)
        .title(app::title)
        .theme(app::active_theme)
        .subscription(app::subscription)
        .run()
}
