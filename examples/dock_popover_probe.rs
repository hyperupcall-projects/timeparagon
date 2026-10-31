//! Exercise the dock editor against a real Wayland compositor.
fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("timeslay=debug"))
        .init();
    let (created, _rx) = smithay_client_toolkit::reexports::calloop::channel::channel();
    let editor = timeslay::panel::layer::spawn_task_popover(created).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(2));
    for _ in 0..2 {
        editor.toggle();
        std::thread::sleep(std::time::Duration::from_secs(2));
        editor.toggle();
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    drop(editor);
    std::thread::sleep(std::time::Duration::from_millis(500));
}
