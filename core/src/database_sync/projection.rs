//! A UI can keep a document open while remote edits arrive. Its next save
//! applies only the changes from what it last read, preserving unseen fields.
use super::{model, other, shared};
use rusqlite::{Connection, OptionalExtension};
use std::{
	collections::BTreeMap,
	io,
	path::{Path, PathBuf},
	sync::{Mutex, OnceLock},
};
type Seen = BTreeMap<(PathBuf, String), Option<String>>;
static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
pub(crate) fn remember(path: &Path, key: &str, value: Option<String>) {
	if shared(key)
		&& let Ok(mut seen) = SEEN.get_or_init(Default::default).lock()
	{
		seen.insert((path.to_owned(), key.into()), value);
	}
}
pub(crate) fn prepare(
	path: &Path,
	db: &Connection,
	key: &str,
	desired: &Option<String>,
) -> io::Result<Option<String>> {
	if !shared(key) {
		return Ok(desired.clone());
	}
	let baseline = SEEN
		.get_or_init(Default::default)
		.lock()
		.map_err(other)?
		.get(&(path.to_owned(), key.into()))
		.cloned();
	let Some(baseline) = baseline else {
		return Ok(desired.clone());
	};
	let current: Option<String> = db
		.query_row("SELECT value FROM documents WHERE key=?1", [key], |r| {
			r.get(0)
		})
		.optional()
		.map_err(other)?;
	if current == baseline {
		return Ok(desired.clone());
	}
	model::rebase(current, baseline, desired.clone()).map_err(other)
}
#[cfg(test)]
mod tests {
	use serde_json::json;
	#[test]
	fn saving_a_stale_view_preserves_remote_additions_and_local_deletions() {
		let d = tempfile::tempdir().unwrap();
		let p = d.path().join("test.db");
		let mut db = crate::task_db::Database::open(&p).unwrap();
		let initial = json!({"habits":[{"id":"a","name":"A"}]});
		crate::task_db::write_document(&p, "shared.projects", &initial).unwrap();
		let _: serde_json::Value = crate::task_db::read_document(&p, "shared.projects")
			.unwrap()
			.unwrap();
		let remote = json!({"habits":[{"id":"a","name":"A"},{"id":"b","name":"B"}]});
		db.connection()
			.execute(
				"UPDATE documents SET value=?1 WHERE key='shared.projects'",
				[remote.to_string()],
			)
			.unwrap();
		crate::task_db::write_document(&p, "shared.projects", &json!({"habits":[]})).unwrap();
		let merged: serde_json::Value = crate::task_db::read_document(&p, "shared.projects")
			.unwrap()
			.unwrap();
		assert_eq!(merged, json!({"habits":[{"id":"b","name":"B"}]}));
	}
}
