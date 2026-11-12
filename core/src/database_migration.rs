//! Verified, non-destructive database snapshots used before sync enrollment.
//! Opens the source read-only: calling the app's normal database opener here
//! would initialize or change storage before we have a backup.
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
	fs, io,
	path::{Path, PathBuf},
	time::Duration,
};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct Inventory {
	pub tables: Vec<(String, i64)>,
	pub documents: Vec<(String, i64)>,
}

fn error(e: impl std::fmt::Display) -> io::Error {
	io::Error::other(e.to_string())
}

pub fn inspect(path: &Path) -> io::Result<Inventory> {
	let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(error)?;
	inspect_connection(&db)
}

fn inspect_connection(db: &Connection) -> io::Result<Inventory> {
	let mut integrity = db.prepare("PRAGMA integrity_check").map_err(error)?;
	let checks = integrity
		.query_map([], |row| row.get::<_, String>(0))
		.map_err(error)?;
	for check in checks {
		let check = check.map_err(error)?;
		if check != "ok" {
			return Err(error(format!("database integrity failure: {check}")));
		}
	}
	let mut statement = db
		.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
		.map_err(error)?;
	let names = statement
		.query_map([], |r| r.get::<_, String>(0))
		.map_err(error)?
		.collect::<Result<Vec<_>, _>>()
		.map_err(error)?;
	let mut tables = Vec::new();
	for name in &names {
		let sql = format!("SELECT count(*) FROM \"{}\"", name.replace('"', "\"\""));
		let count: i64 = db.query_row(&sql, [], |row| row.get(0)).map_err(error)?;
		tables.push((name.clone(), count));
	}
	let documents = if names.iter().any(|name| name == "documents") {
		db.prepare("SELECT key,length(value) FROM documents ORDER BY key")
			.map_err(error)?
			.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
			.map_err(error)?
			.collect::<Result<_, _>>()
			.map_err(error)?
	} else {
		Vec::new()
	};
	Ok(Inventory { tables, documents })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
	pub source: PathBuf,
	pub sha256: String,
	pub inventory: Inventory,
}

/// Publishes a complete snapshot directory by rename only after checking it.
/// A failed attempt leaves the original untouched. Existing backups are never
/// overwritten. SQLite's backup API includes committed WAL data.
pub fn backup(source: &Path, destination: &Path) -> io::Result<Snapshot> {
	if destination.exists() {
		return Err(io::Error::new(
			io::ErrorKind::AlreadyExists,
			"backup already exists",
		));
	}
	let parent = destination
		.parent()
		.filter(|p| !p.as_os_str().is_empty())
		.unwrap_or(Path::new("."));
	fs::create_dir_all(parent)?;
	let temporary = tempfile::Builder::new()
		.prefix(".timeslay-backup-")
		.tempdir_in(parent)?;
	crate::fsutil::make_private(temporary.path())?;
	let backup_path = temporary.path().join("timeslay.sqlite3");
	let db =
		Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(error)?;
	db.busy_timeout(Duration::from_secs(10)).map_err(error)?;
	let mut target = Connection::open(&backup_path).map_err(error)?;
	{
		let copy = rusqlite::backup::Backup::new(&db, &mut target).map_err(error)?;
		copy.run_to_completion(128, Duration::from_millis(10), None)
			.map_err(error)?;
	}
	let inventory = inspect_connection(&target)?;
	target.close().map_err(|(_, e)| error(e))?;
	crate::fsutil::make_private(&backup_path)?;
	fs::File::open(&backup_path)?.sync_all()?;
	let sha256 = format!("{:x}", Sha256::digest(fs::read(&backup_path)?));
	let snapshot = Snapshot {
		source: source.to_owned(),
		sha256,
		inventory,
	};
	crate::fsutil::write_atomic(
		&temporary.path().join("manifest.json"),
		&serde_json::to_vec_pretty(&snapshot).map_err(error)?,
		crate::fsutil::Kind::Private,
	)?;
	fs::File::open(temporary.path())?.sync_all()?;
	// Reserve the name so simultaneous enrollment cannot replace another backup.
	fs::create_dir(destination)?;
	if let Err(e) = fs::rename(temporary.path(), destination) {
		let _ = fs::remove_dir(destination);
		return Err(e);
	}
	fs::File::open(parent)?.sync_all()?;
	Ok(snapshot)
}

/// Checks both the bytes and the SQLite inventory without opening a live app.
pub fn verify(directory: &Path) -> io::Result<Snapshot> {
	let snapshot: Snapshot =
		serde_json::from_slice(&fs::read(directory.join("manifest.json"))?).map_err(error)?;
	let path = directory.join("timeslay.sqlite3");
	if format!("{:x}", Sha256::digest(fs::read(&path)?)) != snapshot.sha256 {
		return Err(error("backup checksum mismatch"));
	}
	if inspect(&path)? != snapshot.inventory {
		return Err(error("backup inventory mismatch"));
	}
	Ok(snapshot)
}

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn backup_includes_wal_and_preserves_unknown_and_invalid_data() {
		let directory = tempfile::tempdir().unwrap();
		let source = directory.path().join("live.sqlite3");
		let db = Connection::open(&source).unwrap();
		db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE documents(key TEXT PRIMARY KEY,value TEXT NOT NULL); INSERT INTO documents VALUES('habits','not json'); CREATE TABLE future_data(value BLOB); INSERT INTO future_data VALUES(x'0001ff');").unwrap();
		let destination = directory.path().join("backup");
		backup(&source, &destination).unwrap();
		verify(&destination).unwrap();
		let restored = Connection::open(destination.join("timeslay.sqlite3")).unwrap();
		assert_eq!(
			restored
				.query_row("SELECT value FROM future_data", [], |r| r
					.get::<_, Vec<u8>>(0))
				.unwrap(),
			vec![0, 1, 255]
		);
		assert_eq!(
			restored
				.query_row("SELECT value FROM documents", [], |r| r.get::<_, String>(0))
				.unwrap(),
			"not json"
		);
		assert_eq!(
			backup(&source, &destination).unwrap_err().kind(),
			io::ErrorKind::AlreadyExists
		);
		fs::write(destination.join("timeslay.sqlite3"), b"damaged").unwrap();
		assert!(verify(&destination).is_err());
	}
	#[test]
	fn missing_database_is_never_created() {
		let directory = tempfile::tempdir().unwrap();
		let source = directory.path().join("absent");
		assert!(backup(&source, &directory.path().join("backup")).is_err());
		assert!(!source.exists());
		assert!(!directory.path().join("backup").exists());
	}
}
