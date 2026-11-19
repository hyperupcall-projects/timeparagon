//! Habit persistence shared by both clients, independent of their save scheduling.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::Library;
use crate::task_db::{self, keys};

/// A writable relational habit library with a baseline per editor. Only a
/// successful read enables writes, protecting unreadable data from replacement.
#[derive(Clone, Debug, Default)]
pub struct Storage {
	path: Option<PathBuf>,
	baseline: Arc<Mutex<Library>>,
}
impl PartialEq for Storage {
	fn eq(&self, other: &Self) -> bool {
		self.path == other.path
	}
}

impl Storage {
	pub fn load(path: Option<PathBuf>) -> Result<(Self, Library), String> {
		let path = path.ok_or("There is nowhere to keep habits on this device.")?;
		let storage = Self {
			path: Some(path),
			..Self::default()
		};
		let library = storage.read()?.unwrap_or_default();
		Ok((storage, library))
	}

	/// Reads the current library. A missing path leaves the editor untouched;
	/// an initialized database with no habits returns an empty library.
	pub fn read(&self) -> Result<Option<Library>, String> {
		let Some(path) = self.path() else {
			return Ok(None);
		};
		let result: Option<Library> =
			task_db::read_document(path, keys::HABITS).map_err(|error| {
				log::warn!("the saved habits could not be read: {error}");
				"The saved habits could not be read, so changes to habits will not be saved."
					.to_owned()
			})?;
		*self.baseline.lock().map_err(|e| e.to_string())? = result.clone().unwrap_or_default();
		Ok(result)
	}

	pub fn path(&self) -> Option<&Path> {
		self.path.as_deref()
	}

	pub fn save(&self, library: &Library) -> Result<(), String> {
		let Some(path) = self.path() else {
			return Ok(());
		};
		let save = || -> Result<(), String> {
			let mut baseline = self.baseline.lock().map_err(|e| e.to_string())?;
			let mut database = task_db::Database::open(path)?;
			let tx = database
				.connection()
				.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
				.map_err(|e| e.to_string())?;
			crate::relational::save_from(
				&tx,
				keys::HABITS,
				&serde_json::to_value(&*baseline).map_err(|e| e.to_string())?,
				&serde_json::to_value(library).map_err(|e| e.to_string())?,
			)
			.map_err(|e| e.to_string())?;
			tx.commit().map_err(|e| e.to_string())?;
			*baseline = library.clone();
			Ok(())
		};
		save().map_err(|error| format!("Could not save habits: {error}"))
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
		let db = rusqlite::Connection::open(&path).unwrap();
		db.execute_batch("CREATE TABLE documents(key TEXT PRIMARY KEY,value TEXT NOT NULL); INSERT INTO documents VALUES('habits','not json'); PRAGMA user_version=10;").unwrap();
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
