//! Database metadata and the one-time import of legacy Markdown properties.
use std::{collections::HashSet, io, path::Path};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use super::{Library, Note, Saved};

fn metadata(db: &Connection, note: &mut Note) -> io::Result<()> {
	let stored: Option<(String, String)> = db
		.query_row(
			"SELECT color,links FROM notes WHERE id=?1",
			[&note.id],
			|row| Ok((row.get(0)?, row.get(1)?)),
		)
		.optional()
		.map_err(io::Error::other)?;
	if let Some((color, links)) = stored {
		note.color = serde_json::from_value(color.into()).map_err(io::Error::other)?;
		note.links = serde_json::from_str(&links).map_err(io::Error::other)?;
	}
	Ok(())
}

fn insert(db: &Connection, note: &Note, update: bool) -> io::Result<()> {
	let color = note.color.name().to_lowercase();
	let links = serde_json::to_value(&note.links)
		.map_err(io::Error::other)?
		.to_string();
	let conflict = if update {
		"DO UPDATE SET color=excluded.color,links=excluded.links
         WHERE notes.color IS NOT excluded.color OR notes.links IS NOT excluded.links"
	} else {
		"DO NOTHING"
	};
	db.execute(
		&format!(
			"INSERT INTO notes(id,color,links) VALUES(?1,?2,?3)
        ON CONFLICT(id) {conflict}"
		),
		params![note.id, color, links],
	)
	.map_err(io::Error::other)?;
	Ok(())
}

pub(super) fn read(directory: &Path, db: &mut Connection) -> io::Result<Library> {
	let mut library = Library::read_files(directory)?;
	let tx = db
		.transaction_with_behavior(TransactionBehavior::Immediate)
		.map_err(io::Error::other)?;
	let mut seen = HashSet::new();
	let mut migrated = Library::default();
	for note in &mut library.notes {
		let missing_id = note.id.trim().is_empty();
		if missing_id {
			note.id = super::local_id();
		}
		metadata(&tx, note)?;
		// A copied file becomes a separate note, starting with the original's metadata.
		if !seen.insert(note.id.clone()) {
			note.id = super::local_id();
			note.migrate = true;
		}
		// An ID-only file can arrive before its database row during sync.
		// Do not journal default metadata that could overwrite that row.
		if missing_id || note.migrate {
			insert(&tx, note, false)?;
		}
		if missing_id || note.migrate || note.cleanup {
			note.cleanup = false;
			note.migrate = false;
			migrated.notes.push(note.clone());
		}
	}
	// Commit the metadata before removing its old copy from the files. A failed
	// file write can be retried; a failed database commit leaves the files intact.
	tx.commit().map_err(io::Error::other)?;
	if !migrated.notes.is_empty() {
		migrated.write_files(directory)?;
	}
	Ok(library)
}

pub(super) fn save(library: &Library, directory: &Path, db: &mut Connection) -> io::Result<Saved> {
	let tx = db
		.transaction_with_behavior(TransactionBehavior::Immediate)
		.map_err(io::Error::other)?;
	for note in &library.notes {
		insert(&tx, note, true)?;
	}
	tx.commit().map_err(io::Error::other)?;
	let saved = library.write_files(directory)?;
	// Only explicit deletions remove metadata. A file temporarily absent during
	// folder sync must not erase another device's connections.
	let tx = db
		.transaction_with_behavior(TransactionBehavior::Immediate)
		.map_err(io::Error::other)?;
	for id in &library.removed_ids {
		tx.execute("DELETE FROM notes WHERE id=?1", [id])
			.map_err(io::Error::other)?;
	}
	tx.commit().map_err(io::Error::other)?;
	Ok(saved)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::notes::{Link, LinkKind, Pastel};

	fn database() -> Connection {
		let db = Connection::open_in_memory().unwrap();
		db.execute_batch(crate::settings::schema::CREATE_DATA)
			.unwrap();
		db
	}

	#[test]
	fn migration_survives_restart_and_external_rename() {
		let directory = tempfile::tempdir().unwrap();
		let old = directory.path().join("Old.md");
		std::fs::write(&old, "---\ntags: [keep]\ncolor: rose\nlinks:\n  - kind: habit\n    id: h1\n    title: Read\n---\n# Body\n").unwrap();
		let path = directory.path().join("metadata.sqlite3");
		let mut db = Connection::open(&path).unwrap();
		db.execute_batch(crate::settings::schema::CREATE_DATA)
			.unwrap();
		let library = read(directory.path(), &mut db).unwrap();
		let id = library.notes[0].id.clone();
		assert_eq!(library.notes[0].color, Pastel::Rose);
		assert_eq!(library.notes[0].links[0].id, "h1");
		let text = std::fs::read_to_string(&old).unwrap();
		assert!(text.contains(&id));
		assert!(!text.contains("color:"));
		assert!(!text.contains("links:"));
		assert!(!text.contains("tags:"));
		assert_eq!(text, format!("---\nid: \"{id}\"\n---\n# Body\n"));
		drop(db);
		std::fs::rename(old, directory.path().join("New.md")).unwrap();
		let mut db = Connection::open(path).unwrap();
		let mut again = read(directory.path(), &mut db).unwrap();
		again.adopt_identities(&library);
		assert_eq!(again.notes[0].id, id);
		assert_eq!(again.notes[0].title, "New");
		assert_eq!(again.notes[0].color, Pastel::Rose);
		assert_eq!(again.notes[0].links, library.notes[0].links);
		let stored: i64 = db
			.query_row("SELECT count(*) FROM notes", [], |r| r.get(0))
			.unwrap();
		assert_eq!(stored, 1);
	}

	#[test]
	fn metadata_edits_and_explicit_deletions_persist() {
		let directory = tempfile::tempdir().unwrap();
		let mut db = database();
		let mut library = Library::default();
		let id = library.create(
			"Plan",
			vec![Link {
				kind: LinkKind::Task,
				id: "task".into(),
				title: "Task".into(),
				day: None,
			}],
		);
		let saved = save(&library, directory.path(), &mut db).unwrap();
		library.saved(&saved);
		let changes = db.total_changes();
		save(&library, directory.path(), &mut db).unwrap();
		assert_eq!(
			db.total_changes(),
			changes,
			"unchanged saves must not journal metadata"
		);
		library.edit(&id, |note| {
			note.color = Pastel::Mint;
			note.links.clear();
		});
		save(&library, directory.path(), &mut db).unwrap();
		let again = read(directory.path(), &mut db).unwrap();
		assert_eq!(again.note(&id).unwrap().color, Pastel::Mint);
		assert!(again.note(&id).unwrap().links.is_empty());
		// An absent file is not a metadata deletion.
		std::fs::remove_file(directory.path().join("Plan.md")).unwrap();
		assert!(read(directory.path(), &mut db).unwrap().notes.is_empty());
		assert!(
			db.query_row("SELECT 1 FROM notes WHERE id=?1", [&id], |_| Ok(()))
				.is_ok()
		);
		library.delete(&id);
		save(&library, directory.path(), &mut db).unwrap();
		assert!(
			db.query_row("SELECT 1 FROM notes WHERE id=?1", [&id], |_| Ok(()))
				.optional()
				.unwrap()
				.is_none()
		);
	}

	#[test]
	fn file_arriving_before_metadata_does_not_journal_defaults() {
		let directory = tempfile::tempdir().unwrap();
		std::fs::write(
			directory.path().join("Synced.md"),
			"---\nid: synced-note\ntags: [old]\nunused:\n  nested: true\n---\nBody",
		)
		.unwrap();
		let mut db = database();
		let library = read(directory.path(), &mut db).unwrap();
		assert_eq!(library.notes[0].id, "synced-note");
		assert_eq!(
			std::fs::read_to_string(directory.path().join("Synced.md")).unwrap(),
			"---\nid: \"synced-note\"\n---\nBody"
		);
		assert_eq!(
			db.query_row("SELECT count(*) FROM notes", [], |r| r.get::<_, i64>(0))
				.unwrap(),
			0
		);
		db.execute_batch("INSERT INTO notes VALUES('synced-note','rose','[]');")
			.unwrap();
		assert_eq!(
			read(directory.path(), &mut db).unwrap().notes[0].color,
			Pastel::Rose
		);
	}

	#[test]
	fn failed_database_write_preserves_legacy_file() {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("Legacy.md");
		let original = "---\ncolor: mint\nlinks: []\n---\nBody";
		std::fs::write(&path, original).unwrap();
		let mut db = database();
		db.execute_batch("CREATE TRIGGER refuse_note BEFORE INSERT ON notes BEGIN SELECT RAISE(ABORT,'unavailable'); END;").unwrap();
		assert!(read(directory.path(), &mut db).is_err());
		assert_eq!(std::fs::read_to_string(path).unwrap(), original);
	}

	#[test]
	fn copied_ids_are_separated_without_losing_metadata() {
		let directory = tempfile::tempdir().unwrap();
		let mut db = database();
		let mut library = Library::default();
		let id = library.create("Original", Vec::new());
		library.edit(&id, |note| note.color = Pastel::Sky);
		save(&library, directory.path(), &mut db).unwrap();
		std::fs::copy(
			directory.path().join("Original.md"),
			directory.path().join("Copy.md"),
		)
		.unwrap();
		let again = read(directory.path(), &mut db).unwrap();
		assert_ne!(again.notes[0].id, again.notes[1].id);
		assert!(again.notes.iter().all(|note| note.color == Pastel::Sky));
		let mut reread = read(directory.path(), &mut db).unwrap();
		reread.adopt_identities(&again);
		assert_eq!(reread, again);
	}
}
