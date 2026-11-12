//! Running blocking work without stalling the interface.
//!
//! Decoding a sound file, posting a D-Bus notification and writing the config
//! are all blocking, and all of them would be visible as a dropped frame if
//! they happened in `update`. They go on a thread instead, with the result
//! delivered back as a message.

use std::panic::{AssertUnwindSafe, catch_unwind};

use iced::Task;
use iced::futures::channel::oneshot;

/// A worker died rather than finishing. The panic itself is already logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerFailed;

impl std::fmt::Display for WorkerFailed {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("Something went wrong in the background. Please try again")
	}
}

impl std::error::Error for WorkerFailed {}

impl From<WorkerFailed> for String {
	fn from(failed: WorkerFailed) -> Self {
		failed.to_string()
	}
}

impl From<WorkerFailed> for std::io::Error {
	fn from(failed: WorkerFailed) -> Self {
		std::io::Error::other(failed)
	}
}

/// What a worker's caller receives if the work never finished.
///
/// Callers mark something as busy when they start work and clear it when the
/// result arrives, so a result has to arrive even from a worker that panicked.
pub trait Outcome {
	fn failed() -> Self;
}

impl<T, E: From<WorkerFailed>> Outcome for Result<T, E> {
	fn failed() -> Self {
		Err(WorkerFailed.into())
	}
}

impl<T> Outcome for Option<T> {
	fn failed() -> Self {
		None
	}
}

impl Outcome for () {
	fn failed() -> Self {}
}

impl<A: Outcome, B: Outcome> Outcome for (A, B) {
	fn failed() -> Self {
		(A::failed(), B::failed())
	}
}

/// Runs `work` on a scratch thread and resolves to its result.
pub fn run<T, F>(work: F) -> Task<T>
where
	F: FnOnce() -> T + Send + 'static,
	T: Outcome + Send + 'static,
{
	let (sender, receiver) = oneshot::channel();

	// A thread per call rather than a pool: these happen at human frequency
	// (a timer firing, a button press), so the spawn cost is irrelevant and
	// there is no pool to keep alive while the app sits idle.
	let spawned = std::thread::Builder::new()
		.name("timeparagon-blocking".to_owned())
		.spawn(move || {
			// Under `panic = "abort"` a panic ends the process here instead.
			let result = catch_unwind(AssertUnwindSafe(work)).unwrap_or_else(|_| {
				log::error!("a background worker panicked");
				T::failed()
			});
			let _ = sender.send(result);
		});

	if let Err(error) = spawned {
		log::error!("could not spawn a worker thread: {error}");
		return Task::done(T::failed());
	}

	// `ok()` only drops the result if the thread is gone without sending,
	// which `catch_unwind` above leaves no way to do.
	Task::future(async move { receiver.await.ok() }).and_then(Task::done)
}

#[cfg(test)]
mod tests {
	use super::*;
	use iced::futures::{StreamExt, executor::block_on};
	use iced_runtime::{Action, task};

	fn outcome<T: Send + 'static>(task: Task<T>) -> Option<T> {
		let mut stream = task::into_stream(task)?;
		block_on(async {
			while let Some(action) = stream.next().await {
				if let Action::Output(value) = action {
					return Some(value);
				}
			}
			None
		})
	}

	#[test]
	fn a_finished_worker_delivers_its_result() {
		let result: Result<u8, String> = outcome(run(|| Ok(7))).unwrap();
		assert_eq!(result, Ok(7));
	}

	#[test]
	fn a_panicking_worker_still_delivers_a_failure() {
		let result: Result<u8, String> = outcome(run(|| panic!("boom"))).unwrap();
		assert_eq!(result, Err(String::from(WorkerFailed)));
	}

	#[test]
	fn workers_without_a_result_type_report_their_own_empty_outcome() {
		let result: Option<u8> = outcome(run(|| panic!("boom"))).unwrap();
		assert_eq!(result, None);
		let pair: (Result<(), String>, Option<u8>) = outcome(run(|| panic!("boom"))).unwrap();
		assert!(pair.0.is_err());
		assert_eq!(pair.1, None);
	}
}
