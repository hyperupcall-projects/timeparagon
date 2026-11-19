//! Habit persistence shared by both clients, independent of their save scheduling.

use std::path::{Path, PathBuf};

use super::Library;
use crate::task_db::{self, keys};

/// A writable habit document. Only a successful read enables writes, so an
/// unreadable document cannot be replaced with an empty library.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Storage {
	path: Option<PathBuf>,
}

impl Storage {
	pub fn load(path: Option<PathBuf>) -> Result<(Self, Library), String> {
		let path = path.ok_or("There is nowhere to keep habits on this device.")?;
		let storage = Self { path: Some(path) };
		let library = storage.read()?.unwrap_or_default();
		Ok((storage, library))
	}

	/// Reads the current document without treating a removed document as an
	/// empty replacement for the client's in-memory library.
	pub fn read(&self) -> Result<Option<Library>, String> {
		let Some(path) = self.path() else {
			return Ok(None);
		};
		task_db::read_document(path, keys::HABITS).map_err(|error| {
			log::warn!("the saved habits could not be read: {error}");
			"The saved habits could not be read, so changes to habits will not be saved.".to_owned()
		})
	}

	pub fn path(&self) -> Option<&Path> {
		self.path.as_deref()
	}

	pub fn save(&self, library: &Library) -> Result<(), String> {
		let Some(path) = self.path() else {
			return Ok(());
		};
		task_db::write_document(path, keys::HABITS, library)
			.map_err(|error| format!("Could not save habits: {error}"))
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::habits::Habit;

	#[test]
	fn a_missing_document_loads_empty_and_round_trips_the_library() {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("nested/habits.sqlite3");
		let (storage, mut library) = Storage::load(Some(path.clone())).unwrap();
		assert!(library.habits.is_empty());
		assert_eq!(storage.path(), Some(path.as_path()));
		library.habits.push(Habit::new("run", "Run"));
		storage.save(&library).unwrap();
		assert_eq!(Storage::load(Some(path)).unwrap().1, library);
	}

	#[test]
	fn unreadable_documents_never_enable_writes() {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("habits.sqlite3");
		task_db::write_document_json(&path, keys::HABITS, "not json").unwrap();
		assert!(Storage::load(Some(path.clone())).is_err());
		let raw: String = rusqlite::Connection::open(&path)
			.unwrap()
			.query_row(
				"SELECT value FROM documents WHERE key=?1",
				[keys::HABITS],
				|row| row.get(0),
			)
			.unwrap();
		assert_eq!(raw, "not json");
		assert!(Storage::load(None).is_err());
		assert!(Storage::default().path().is_none());
		Storage::default().save(&Library::default()).unwrap();
	}

	#[test]
	fn failed_writes_report_an_error_and_can_be_retried() {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("habits.sqlite3");
		let (storage, library) = Storage::load(Some(path.clone())).unwrap();
		std::fs::remove_file(&path).unwrap();
		std::fs::create_dir(&path).unwrap();
		assert!(
			storage
				.save(&library)
				.unwrap_err()
				.starts_with("Could not save habits:")
		);
		std::fs::remove_dir(&path).unwrap();
		storage.save(&library).unwrap();
		assert_eq!(Storage::load(Some(path)).unwrap().1, library);
	}
}
