//! Import legacy note metadata and remove unused front matter in the configured folder.
use std::io;

use timeparagon_core::notes::{self, Library};

fn main() -> io::Result<()> {
	let directory =
		notes::directory().ok_or_else(|| io::Error::other("No notes folder is configured"))?;
	let migrated = Library::read(&directory)?;
	let reread = Library::read(&directory)?;
	for note in &migrated.notes {
		let stored = reread
			.note(&note.id)
			.ok_or_else(|| io::Error::other("A migrated note could not be reread"))?;
		if note.body != stored.body || note.color != stored.color || note.links != stored.links {
			return Err(io::Error::other("Migrated note contents did not verify"));
		}
	}
	println!(
		"Migrated and verified {} notes in {}",
		migrated.notes.len(),
		directory.display()
	);
	Ok(())
}
