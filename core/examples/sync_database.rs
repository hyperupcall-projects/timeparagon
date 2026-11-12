//! Enroll and sync the local app database without starting a graphical window.
use std::time::{Duration, Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
	let watch = std::env::args().skip(1).any(|arg| arg == "--watch");
	timeslay_core::database_sync::start();
	let deadline = Instant::now() + Duration::from_secs(65);
	let mut previous = String::new();
	loop {
		let status = timeslay_core::database_sync::status();
		if status != previous {
			println!("{status}");
			previous = status.clone();
		}
		if !watch && status == "Database synced" {
			return Ok(());
		}
		if !watch && Instant::now() > deadline {
			return Err("sync did not finish; local data and journal retained".into());
		}
		std::thread::sleep(Duration::from_millis(250));
	}
}
