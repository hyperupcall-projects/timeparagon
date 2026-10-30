//! Running blocking work without stalling the interface.
//!
//! Decoding a sound file, posting a D-Bus notification and writing the config
//! are all blocking, and all of them would be visible as a dropped frame if
//! they happened in `update`. They go on a thread instead, with the result
//! delivered back as a message.

use iced::Task;
use iced::futures::channel::oneshot;

/// Runs `work` on a scratch thread and resolves to its result.
pub fn run<T, F>(work: F) -> Task<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (sender, receiver) = oneshot::channel();

    // A thread per call rather than a pool: these happen at human frequency
    // (a timer firing, a button press), so the spawn cost is irrelevant and
    // there is no pool to keep alive while the app sits idle.
    let spawned = std::thread::Builder::new()
        .name("timeslay-blocking".to_owned())
        .spawn(move || {
            let _ = sender.send(work());
        });

    if let Err(error) = spawned {
        log::error!("could not spawn a worker thread: {error}");
        return Task::none();
    }

    // `ok()` drops the cancellation case, which only happens if the worker
    // panics; there is nothing useful to report to the update loop then.
    Task::future(async move { receiver.await.ok() }).and_then(Task::done)
}
