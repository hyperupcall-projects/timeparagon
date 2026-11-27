//! Normalized storage for habits, work logs and brain dumps. JSON is only an
//! interchange format at the UI boundary; no collection is persisted as JSON.
//! These tables bypass database_sync::model and replicate as native Turso rows.
mod codec;
pub(crate) mod sync;

use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde_json::Value;
use std::{
	collections::BTreeMap,
	io,
	path::{Path, PathBuf},
	sync::{Mutex, OnceLock},
};

pub(crate) type Rows = BTreeMap<(usize, String), Vec<Value>>;
pub(crate) fn error(e: impl std::fmt::Display) -> io::Error {
	io::Error::other(e.to_string())
}
pub(crate) fn invalid(e: impl std::fmt::Display) -> io::Error {
	io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

pub(crate) struct Table {
	pub name: &'static str,
	pub columns: &'static [(&'static str, &'static str)],
	pub keys: usize,
	constraints: &'static str,
	pub document: &'static str,
}
macro_rules! table {
    ($name:literal, $doc:literal, $keys:literal, $constraints:literal, $($col:literal => $ty:literal),+ $(,)?) => {
        Table { name: $name, document: $doc, keys: $keys, constraints: $constraints, columns: &[$(($col,$ty)),+] }
    };
}
// Parents precede children. Deletions run in reverse order.
pub(crate) const TABLES: &[Table] = &[
	table!("habit_sections", "habits", 1, "", "id"=>"TEXT", "name"=>"TEXT NOT NULL", "collapsed"=>"BOOLEAN NOT NULL", "position"=>"INTEGER NOT NULL"),
	// Each frequency kind has exactly its own numbers, and a habit has all of a
	// measure's fields or none.
	table!("habits", "habits", 1, ", FOREIGN KEY(section_id) REFERENCES habit_sections(id) ON DELETE SET NULL DEFERRABLE INITIALLY DEFERRED,
        CHECK(CASE frequency_kind WHEN 'interval' THEN frequency_times IS NOT NULL AND frequency_days IS NOT NULL
            WHEN 'times_per_day' THEN frequency_times IS NOT NULL AND frequency_days IS NULL
            ELSE frequency_times IS NULL AND frequency_days IS NULL END),
        CHECK(CASE WHEN measure_present THEN measure_unit IS NOT NULL AND measure_days IS NOT NULL
            ELSE measure_unit IS NULL AND measure_target IS NULL AND measure_days IS NULL END)",
        "id"=>"TEXT", "name"=>"TEXT NOT NULL", "kind"=>"TEXT NOT NULL CHECK(kind IN ('yes','no'))", "created"=>"TEXT", "question"=>"TEXT NOT NULL", "notes"=>"TEXT NOT NULL", "color"=>"INTEGER NOT NULL",
        "frequency_kind"=>"TEXT NOT NULL CHECK(frequency_kind IN ('interval','times_per_day','weekdays'))", "frequency_times"=>"INTEGER", "frequency_days"=>"INTEGER",
        "archived"=>"BOOLEAN NOT NULL", "skip_days"=>"BOOLEAN NOT NULL", "question_marks"=>"BOOLEAN NOT NULL", "icons"=>"TEXT NOT NULL", "section_id"=>"TEXT",
        "measure_present"=>"BOOLEAN NOT NULL", "measure_unit"=>"TEXT", "measure_target"=>"REAL", "measure_days"=>"INTEGER", "area_id"=>"TEXT", "distraction"=>"BOOLEAN NOT NULL", "position"=>"INTEGER NOT NULL"),
	table!("habit_weekdays", "habits", 2, ", FOREIGN KEY(habit_id) REFERENCES habits(id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED", "habit_id"=>"TEXT", "weekday"=>"INTEGER CHECK(weekday BETWEEN 0 AND 6)"),
	// A count of repetitions belongs to a `reps` mark, and only to one.
	table!("habit_days", "habits", 2, ", FOREIGN KEY(habit_id) REFERENCES habits(id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED,
        CHECK((mark IS 'reps') = (repetitions IS NOT NULL))",
        "habit_id"=>"TEXT", "date"=>"TEXT", "mark"=>"TEXT CHECK(mark IN ('yes','no','skip','reps'))", "repetitions"=>"INTEGER", "resisted"=>"INTEGER", "value"=>"INTEGER", "has_log"=>"BOOLEAN NOT NULL"),
	// Keyed by a content-derived ID (see `codec::log_entry_id`), not by position,
	// so entries logged on two devices at once are both kept.
	table!("habit_entries", "habits", 3, ", FOREIGN KEY(habit_id,date) REFERENCES habit_days(habit_id,date) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED",
        "habit_id"=>"TEXT", "date"=>"TEXT", "id"=>"TEXT", "kind"=>"TEXT NOT NULL CHECK(kind IN ('happened','resisted'))", "position"=>"INTEGER NOT NULL", "at"=>"TEXT", "amount"=>"INTEGER", "note"=>"TEXT NOT NULL"),
	table!("habit_preferences", "habits", 1, "", "id"=>"INTEGER CHECK(id=1)", "sort"=>"TEXT NOT NULL", "reversed"=>"BOOLEAN NOT NULL", "hide_archived"=>"BOOLEAN NOT NULL", "hide_completed"=>"BOOLEAN NOT NULL", "score_bucket"=>"TEXT NOT NULL", "history_bucket"=>"TEXT NOT NULL"),
	table!("brain_dump_entries", "shared.brain_dump", 1, "", "date"=>"TEXT", "text"=>"TEXT NOT NULL", "position"=>"INTEGER NOT NULL"),
	table!("task_work_sessions", "shared.task_work", 1, "", "id"=>"TEXT", "task_key"=>"TEXT NOT NULL", "task_title"=>"TEXT NOT NULL", "block_key"=>"TEXT", "started_at"=>"TEXT NOT NULL", "stopped_at"=>"TEXT", "position"=>"INTEGER NOT NULL"),
	table!("task_work_distractions", "shared.task_work", 1, "", "id"=>"TEXT", "block_key"=>"TEXT NOT NULL", "minutes"=>"INTEGER NOT NULL CHECK(minutes>=0)", "recorded_at"=>"TEXT NOT NULL", "position"=>"INTEGER NOT NULL"),
];

impl Table {
	pub fn names(&self) -> String {
		self.columns
			.iter()
			.map(|c| c.0)
			.collect::<Vec<_>>()
			.join(",")
	}
	pub fn key_sql(&self, prefix: &str) -> String {
		format!(
			"json_array({})",
			self.columns[..self.keys]
				.iter()
				.map(|c| format!("{prefix}{}", c.0))
				.collect::<Vec<_>>()
				.join(",")
		)
	}
	pub fn create(&self, remote: bool) -> String {
		self.create_as(self.name, remote)
	}
	/// The local table is `STRICT`, so a value of the wrong type is refused
	/// rather than stored. `STRICT` has no `BOOLEAN`: such a column is an
	/// `INTEGER` holding 0 or 1, and decodes as a boolean by its listed type.
	/// The server's copy keeps only each column's type, since its tombstones
	/// leave the other columns empty.
	fn create_as(&self, name: &str, remote: bool) -> String {
		let columns = self
			.columns
			.iter()
			.enumerate()
			.map(|(i, (column, ty))| {
				let key = if i < self.keys { " NOT NULL" } else { "" };
				if remote {
					format!("{column} {}{key}", ty.split_whitespace().next().unwrap())
				} else if let Some(rest) = ty.strip_prefix("BOOLEAN") {
					format!("{column} INTEGER{rest}{key} CHECK({column} IN (0,1))")
				} else {
					format!("{column} {ty}{key}")
				}
			})
			.collect::<Vec<_>>()
			.join(",");
		let keys = self.columns[..self.keys]
			.iter()
			.map(|c| c.0)
			.collect::<Vec<_>>()
			.join(",");
		format!(
			"CREATE TABLE IF NOT EXISTS {name}({columns}{},PRIMARY KEY({keys}){}){}",
			if remote {
				",sync_revision TEXT NOT NULL,sync_deleted INTEGER NOT NULL DEFAULT 0"
			} else {
				""
			},
			if remote { "" } else { self.constraints },
			if remote { "" } else { " STRICT" }
		)
	}
	pub fn upsert_sql(&self) -> String {
		let keys = self.columns[..self.keys]
			.iter()
			.map(|c| c.0)
			.collect::<Vec<_>>()
			.join(",");
		let updates = self.columns[self.keys..]
			.iter()
			.map(|c| format!("{}=excluded.{}", c.0, c.0))
			.collect::<Vec<_>>()
			.join(",");
		format!(
			"INSERT INTO {}({}) VALUES({}) ON CONFLICT({keys}) DO {}",
			self.name,
			self.names(),
			vec!["?"; self.columns.len()].join(","),
			if updates.is_empty() {
				"NOTHING".to_owned()
			} else {
				format!("UPDATE SET {updates}")
			}
		)
	}
	pub fn delete(&self, db: &Connection, key: &str) -> io::Result<()> {
		db.execute(
			&format!("DELETE FROM {} WHERE {}", self.name, self.where_key()),
			[key],
		)
		.map_err(error)?;
		Ok(())
	}
	pub fn where_key(&self) -> String {
		self.columns[..self.keys]
			.iter()
			.enumerate()
			.map(|(i, c)| format!("{}=json_extract(?1,'$[{i}]')", c.0))
			.collect::<Vec<_>>()
			.join(" AND ")
	}
	pub fn read_one(&self, db: &Connection, key: &str) -> io::Result<Option<Vec<Value>>> {
		let mut statement = db
			.prepare(&format!(
				"SELECT {} FROM {} WHERE {}",
				self.names(),
				self.name,
				self.where_key()
			))
			.map_err(error)?;
		let mut rows = statement.query([key]).map_err(error)?;
		rows.next()
			.map_err(error)?
			.map(|row| self.read_row(row))
			.transpose()
	}
	fn read_row(&self, row: &rusqlite::Row<'_>) -> io::Result<Vec<Value>> {
		self.columns
			.iter()
			.enumerate()
			.map(|(j, (_, ty))| {
				use rusqlite::types::ValueRef as S;
				Ok(match row.get_ref(j).map_err(error)? {
					S::Null => Value::Null,
					S::Integer(n) if ty.starts_with("BOOLEAN") => Value::Bool(n != 0),
					S::Integer(n) => n.into(),
					S::Real(n) => serde_json::json!(n),
					S::Text(s) => std::str::from_utf8(s).map_err(invalid)?.into(),
					S::Blob(_) => return Err(invalid("unexpected blob in relational data")),
				})
			})
			.collect()
	}
	pub fn upsert(&self, db: &Connection, row: &[Value]) -> io::Result<()> {
		let values = row.iter().map(sql_value).collect::<io::Result<Vec<_>>>()?;
		db.execute(&self.upsert_sql(), params_from_iter(values))
			.map_err(error)?;
		Ok(())
	}
}
pub(crate) fn sql_value(value: &Value) -> io::Result<rusqlite::types::Value> {
	use rusqlite::types::Value as S;
	Ok(match value {
		Value::Null => S::Null,
		Value::String(v) => S::Text(v.clone()),
		Value::Bool(v) => S::Integer(i64::from(*v)),
		Value::Number(v) => {
			if let Some(n) = v.as_i64() {
				S::Integer(n)
			} else {
				S::Real(v.as_f64().ok_or_else(|| invalid("number out of range"))?)
			}
		}
		_ => return Err(invalid("relational columns must be scalar")),
	})
}
pub(crate) fn insert(rows: &mut Rows, table: usize, values: Vec<Value>) -> io::Result<()> {
	let key = key_for(table, &values);
	if rows.insert((table, key.clone()), values).is_some() {
		return Err(invalid(format!(
			"duplicate {} key {key}",
			TABLES[table].name
		)));
	}
	Ok(())
}
pub(crate) fn key_for(table: usize, values: &[Value]) -> String {
	Value::Array(values[..TABLES[table].keys].to_vec()).to_string()
}

pub(crate) fn install(db: &Connection) -> io::Result<()> {
	db.execute_batch("CREATE TABLE IF NOT EXISTS timeparagon_relational_control(id INTEGER PRIMARY KEY CHECK(id=1),projecting INTEGER NOT NULL DEFAULT 0); INSERT OR IGNORE INTO timeparagon_relational_control VALUES(1,0);
        CREATE TABLE IF NOT EXISTS timeparagon_relational_state(table_name TEXT NOT NULL,row_key TEXT NOT NULL,revision TEXT NOT NULL,deleted INTEGER NOT NULL,dirty INTEGER NOT NULL,PRIMARY KEY(table_name,row_key));
        CREATE INDEX IF NOT EXISTS timeparagon_relational_dirty ON timeparagon_relational_state(dirty);
        CREATE TABLE IF NOT EXISTS timeparagon_relational_legacy(key TEXT PRIMARY KEY,value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS timeparagon_relational_initialized(key TEXT PRIMARY KEY);
		CREATE TABLE IF NOT EXISTS timeparagon_relational_ignored(id TEXT PRIMARY KEY);
        CREATE TABLE IF NOT EXISTS timeparagon_relational_servers(url TEXT PRIMARY KEY);").map_err(error)?;
	for table in TABLES {
		db.execute_batch(&table.create(false)).map_err(error)?;
		crate::local_changes::watch(
			db,
			table.name,
			if table.document == "habits" {
				"doc:habits"
			} else {
				"settings"
			},
		)
		.map_err(error)?;
		for (op, prefix, deleted) in [
			("INSERT", "NEW.", 0),
			("UPDATE", "NEW.", 0),
			("DELETE", "OLD.", 1),
		] {
			let changed = if op == "UPDATE" {
				let object = |prefix: &str| {
					format!(
						"json_array({})",
						table
							.columns
							.iter()
							.map(|c| format!("{prefix}{}", c.0))
							.collect::<Vec<_>>()
							.join(",")
					)
				};
				format!(" AND {} IS NOT {}", object("OLD."), object("NEW."))
			} else {
				String::new()
			};
			// A coalescing outbox: one token per row, no before/after copies.
			db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS timeparagon_relational_{}_{op} AFTER {op} ON {} WHEN (SELECT projecting FROM timeparagon_relational_control WHERE id=1)=0{changed} BEGIN
                INSERT INTO timeparagon_relational_state(table_name,row_key,revision,deleted,dirty) VALUES('{}',{},lower(hex(randomblob(16))),{deleted},1)
                ON CONFLICT(table_name,row_key) DO UPDATE SET revision=excluded.revision,deleted=excluded.deleted,dirty=1; END;",table.name,table.name,table.name,table.key_sql(prefix))).map_err(error)?;
		}
		db.execute_batch(&format!("CREATE TRIGGER IF NOT EXISTS timeparagon_relational_{}_rekey AFTER UPDATE ON {} WHEN (SELECT projecting FROM timeparagon_relational_control WHERE id=1)=0 AND {} IS NOT {} BEGIN
            INSERT INTO timeparagon_relational_state VALUES('{}',{},lower(hex(randomblob(16))),1,1) ON CONFLICT(table_name,row_key) DO UPDATE SET revision=excluded.revision,deleted=1,dirty=1; END;", table.name, table.name, table.key_sql("OLD."), table.key_sql("NEW."), table.name, table.key_sql("OLD."))).map_err(error)?;
	}
	db.execute_batch("CREATE INDEX IF NOT EXISTS habit_days_date ON habit_days(date,habit_id);
        CREATE INDEX IF NOT EXISTS habits_section ON habits(section_id);
        CREATE INDEX IF NOT EXISTS task_work_sessions_task ON task_work_sessions(task_key,started_at);
        CREATE INDEX IF NOT EXISTS task_work_sessions_block ON task_work_sessions(block_key,started_at);
        CREATE INDEX IF NOT EXISTS task_work_distractions_block ON task_work_distractions(block_key,recorded_at);").map_err(error)?;
	migrate_log_entries(db)?;
	for key in ["habits", "shared.brain_dump", "shared.task_work"] {
		let legacy: Option<String> = db
			.query_row("SELECT value FROM documents WHERE key=?1", [key], |r| {
				r.get(0)
			})
			.optional()
			.map_err(error)?;
		if let Some(text) = legacy {
			let value: Value = serde_json::from_str(&text).map_err(invalid)?;
			let rows = codec::encode(key, &value)?;
			if !initialized(db, key)? {
				for ((i, _), row) in rows {
					TABLES[i].upsert(db, &row)?;
				}
			}
			db.execute(
				"INSERT OR IGNORE INTO timeparagon_relational_legacy VALUES(?1,?2)",
				params![key, text],
			)
			.map_err(error)?;
			mark_initialized(db, key)?;
			// Old schema-10 triggers still watch these keys during the upgrade.
			// Removing the source is migration, not a user deletion to replay
			// when importing legacy cloud history at the first connection.
			let has_control: bool = db
				.query_row(
					"SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='timeparagon_sync_control')",
					[],
					|r| r.get(0),
				)
				.map_err(error)?;
			let projecting: Option<i64> = if has_control {
				db.query_row(
					"SELECT projecting FROM timeparagon_sync_control WHERE singleton=1",
					[],
					|r| r.get(0),
				)
				.optional()
				.map_err(error)?
			} else {
				None
			};
			if projecting.is_some() {
				db.execute(
					"UPDATE timeparagon_sync_control SET projecting=1 WHERE singleton=1",
					[],
				)
				.map_err(error)?;
			}
			db.execute("DELETE FROM documents WHERE key=?1", [key])
				.map_err(error)?;
			if let Some(projecting) = projecting {
				db.execute(
					"UPDATE timeparagon_sync_control SET projecting=?1 WHERE singleton=1",
					[projecting],
				)
				.map_err(error)?;
			}
		}
	}
	Ok(())
}
/// Schema 12: `habit_log_entries`, keyed by `(habit_id, date, kind, position)`,
/// becomes `habit_entries`, keyed by `(habit_id, date, id)`. Each entry keeps
/// its sync token and whether it still has to be sent, under its new key, so
/// devices that migrate the same synced rows agree on them. An unsent deletion
/// cannot be carried over (the deleted entry's content, which its new ID comes
/// from, is gone) and is dropped. The server's old table is left for builds
/// that still use it. Idempotent: does nothing once the old table is gone.
fn migrate_log_entries(db: &Connection) -> io::Result<()> {
	const OLD: &str = "habit_log_entries";
	let exists: bool = db
		.query_row(
			"SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
			[OLD],
			|r| r.get(0),
		)
		.map_err(error)?;
	if !exists {
		return Ok(());
	}
	let table = TABLES
		.iter()
		.find(|t| t.name == "habit_entries")
		.expect("habit_entries is a relational table");
	let mut renamed = BTreeMap::<String, String>::new();
	let mut rows = Vec::new();
	{
		let mut query = db
			.prepare(&format!(
				"SELECT habit_id,date,kind,position,at,amount,note FROM {OLD} ORDER BY habit_id,date,kind,position"
			))
			.map_err(error)?;
		let mut result = query.query([]).map_err(error)?;
		let mut counts = BTreeMap::<(String, String), BTreeMap<String, usize>>::new();
		while let Some(row) = result.next().map_err(error)? {
			let habit: String = row.get(0).map_err(error)?;
			let date: String = row.get(1).map_err(error)?;
			let kind: String = row.get(2).map_err(error)?;
			let position: i64 = row.get(3).map_err(error)?;
			let at: Value = row.get::<_, Option<String>>(4).map_err(error)?.into();
			let amount: Value = row.get::<_, Option<i64>>(5).map_err(error)?.into();
			let note: String = row.get(6).map_err(error)?;
			let id = codec::log_entry_id(
				&kind,
				&at,
				&amount,
				&note,
				counts.entry((habit.clone(), date.clone())).or_default(),
			);
			let old_key = serde_json::json!([habit, date, kind, position]).to_string();
			renamed.insert(old_key, serde_json::json!([habit, date, id]).to_string());
			rows.push(vec![
				habit.into(),
				date.into(),
				id.into(),
				kind.into(),
				position.into(),
				at,
				amount,
				note.into(),
			]);
		}
	}
	// The copies are not new edits: their sync state moves with them below.
	let projecting: i64 = db
		.query_row(
			"SELECT projecting FROM timeparagon_relational_control WHERE id=1",
			[],
			|r| r.get(0),
		)
		.map_err(error)?;
	db.execute(
		"UPDATE timeparagon_relational_control SET projecting=1 WHERE id=1",
		[],
	)
	.map_err(error)?;
	for row in &rows {
		table.upsert(db, row)?;
	}
	let states = db
		.prepare(
			"SELECT row_key,revision,deleted,dirty FROM timeparagon_relational_state WHERE table_name=?1",
		)
		.and_then(|mut q| {
			q.query_map([OLD], |r| {
				Ok((
					r.get::<_, String>(0)?,
					r.get::<_, String>(1)?,
					r.get::<_, i64>(2)?,
					r.get::<_, i64>(3)?,
				))
			})?
			.collect::<rusqlite::Result<Vec<_>>>()
		})
		.map_err(error)?;
	for (key, revision, deleted, dirty) in states {
		if let (0, Some(key)) = (deleted, renamed.get(&key)) {
			db.execute(
				"INSERT INTO timeparagon_relational_state VALUES(?1,?2,?3,0,?4) ON CONFLICT(table_name,row_key) DO UPDATE SET revision=excluded.revision,deleted=0,dirty=excluded.dirty",
				params![table.name, key, revision, dirty],
			)
			.map_err(error)?;
		}
	}
	db.execute(
		"DELETE FROM timeparagon_relational_state WHERE table_name=?1",
		[OLD],
	)
	.map_err(error)?;
	// Also drops its triggers.
	db.execute_batch(&format!("DROP TABLE {OLD};"))
		.map_err(error)?;
	db.execute(
		"UPDATE timeparagon_relational_control SET projecting=?1 WHERE id=1",
		[projecting],
	)
	.map_err(error)?;
	Ok(())
}

/// Schema 18: work started on a Google Task before the task had its
/// TimeParagon ID was keyed by the task's Google identity,
/// `<calendar_id>/<event_id>` (see `timeblocking::task_key`), which changes
/// when the task moves. Each such session whose identity is an alias of one
/// task moves to that task's ID, where the task's later work is. A session's
/// row ID comes from its key (see [`codec::session_id`]), so the move is a new
/// row: the triggers record the old row's deletion and the new row, sent
/// together, and the new row keeps the old one's sync token, so devices that
/// move the same synced session agree on it. A session another device already
/// moved, and sent here, is kept, and the copy under the Google identity goes.
/// A session whose identity is no task's alias keeps it. Idempotent.
pub(crate) fn rekey_work_sessions(db: &Connection) -> io::Result<()> {
	const TABLE: &str = "task_work_sessions";
	let sessions = db
		.prepare(
			"SELECT s.id,min(a.task_id),s.started_at FROM task_work_sessions s
             JOIN task_aliases a ON s.task_key=a.calendar_id||'/'||a.event_id
             GROUP BY s.id HAVING count(DISTINCT a.task_id)=1",
		)
		.and_then(|mut q| {
			q.query_map([], |r| {
				Ok((
					r.get::<_, String>(0)?,
					r.get::<_, i64>(1)?,
					r.get::<_, String>(2)?,
				))
			})?
			.collect::<rusqlite::Result<Vec<_>>>()
		})
		.map_err(error)?;
	for (old, task, started_at) in sessions {
		let key = task.to_string();
		let new = codec::session_id(&key, &Value::String(started_at), 0);
		let revision: Option<String> = db
			.query_row(
				"SELECT revision FROM timeparagon_relational_state WHERE table_name=?1 AND row_key=?2 AND deleted=0",
				params![TABLE, serde_json::json!([old]).to_string()],
				|r| r.get(0),
			)
			.optional()
			.map_err(error)?;
		let moved: bool = db
			.query_row(
				"SELECT EXISTS(SELECT 1 FROM task_work_sessions WHERE id=?1)",
				[&new],
				|r| r.get(0),
			)
			.map_err(error)?;
		if moved {
			db.execute("DELETE FROM task_work_sessions WHERE id=?1", [&old])
				.map_err(error)?;
			continue;
		}
		db.execute(
			"UPDATE task_work_sessions SET id=?2,task_key=?3 WHERE id=?1",
			params![old, new, key],
		)
		.map_err(error)?;
		if let Some(revision) = revision {
			db.execute(
				"UPDATE timeparagon_relational_state SET revision=?3 WHERE table_name=?1 AND row_key=?2",
				params![TABLE, serde_json::json!([new]).to_string(), revision],
			)
			.map_err(error)?;
		}
	}
	Ok(())
}

/// Schema 15: rebuilds each table of [`TABLES`] made before it was `STRICT` and
/// had its `CHECK`s, the way SQLite documents changing a table's definition:
/// with foreign keys off, so rebuilding a parent leaves its children alone, and
/// checked before committing. Triggers and indexes go with the old tables;
/// [`install`] puts them back in the same transaction, so no write goes
/// unrecorded in between. Must run outside a transaction, since the foreign key
/// pragma cannot change inside one. A row that does not fit fails the whole
/// rebuild, leaving every table as it was, and the error is returned.
pub(crate) fn make_strict(db: &Connection) -> io::Result<()> {
	let loose = |db: &Connection| -> io::Result<Vec<&'static Table>> {
		let mut loose = Vec::new();
		for table in TABLES {
			let strict: Option<bool> = db
				.query_row(
					"SELECT strict FROM pragma_table_list WHERE schema='main' AND name=?1",
					[table.name],
					|r| r.get(0),
				)
				.optional()
				.map_err(error)?;
			if strict == Some(false) {
				loose.push(table);
			}
		}
		Ok(loose)
	};
	if loose(db)?.is_empty() {
		return Ok(());
	}
	db.execute_batch("PRAGMA foreign_keys=OFF").map_err(error)?;
	let result = (|| {
		let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
			.map_err(error)?;
		// Another process may have done it while this one waited.
		for table in loose(&tx)? {
			let names = table.names();
			tx.execute_batch(&format!(
				"{}; INSERT INTO timeparagon_strict_new({names}) SELECT {names} FROM {table};
                 DROP TABLE {table}; ALTER TABLE timeparagon_strict_new RENAME TO {table};",
				table.create_as("timeparagon_strict_new", false),
				table = table.name,
			))
			.map_err(error)?;
		}
		let broken = tx
			.query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
			.optional()
			.map_err(error)?
			.is_some();
		if broken {
			return Err(invalid("a relational row refers to a missing parent"));
		}
		install(&tx)?;
		tx.commit().map_err(error)
	})();
	db.execute_batch("PRAGMA foreign_keys=ON").map_err(error)?;
	result
}

pub(crate) fn initialized(db: &Connection, key: &str) -> io::Result<bool> {
	db.query_row(
		"SELECT EXISTS(SELECT 1 FROM timeparagon_relational_initialized WHERE key=?1)",
		[key],
		|r| r.get(0),
	)
	.map_err(error)
}
fn mark_initialized(db: &Connection, key: &str) -> io::Result<()> {
	db.execute(
		"INSERT OR IGNORE INTO timeparagon_relational_initialized VALUES(?1)",
		[key],
	)
	.map_err(error)?;
	Ok(())
}
pub(crate) fn all(db: &Connection, document: Option<&str>) -> io::Result<Rows> {
	let mut result = Rows::new();
	for (i, table) in TABLES
		.iter()
		.enumerate()
		.filter(|(_, t)| document.is_none_or(|d| t.document == d))
	{
		let mut query = db
			.prepare(&format!("SELECT {} FROM {}", table.names(), table.name))
			.map_err(error)?;
		let mut rows = query.query([]).map_err(error)?;
		while let Some(row) = rows.next().map_err(error)? {
			insert(&mut result, i, table.read_row(row)?)?;
		}
	}
	Ok(result)
}

// Config's existing API saves snapshots. Compare rows against the last read
// view so an unseen row from sync is never mistaken for a local deletion.
type Seen = BTreeMap<(PathBuf, String), Rows>;
static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
pub(crate) fn remember(path: &Path, key: &str, rows: Rows) -> io::Result<()> {
	SEEN.get_or_init(Default::default)
		.lock()
		.map_err(error)?
		.insert((path.to_owned(), key.to_owned()), rows);
	Ok(())
}
pub(crate) fn read(db: &Connection, path: &Path, key: &str) -> io::Result<Option<Value>> {
	let rows = all(db, Some(key))?;
	if rows.is_empty() && !initialized(db, key)? {
		remember(path, key, rows)?;
		return Ok(None);
	}
	let value = codec::decode(key, &rows)?;
	remember(path, key, rows)?;
	Ok(Some(value))
}
pub(crate) fn save(db: &Connection, path: &Path, key: &str, value: &Value) -> io::Result<Rows> {
	let desired = codec::encode(key, value)?;
	let baseline = SEEN
		.get_or_init(Default::default)
		.lock()
		.map_err(error)?
		.get(&(path.to_owned(), key.to_owned()))
		.cloned();
	let baseline = match baseline {
		Some(rows) => rows,
		None => all(db, Some(key))?,
	};
	write_diff(db, &baseline, &desired)?;
	mark_initialized(db, key)?;
	Ok(desired)
}
pub(crate) fn save_from(
	db: &Connection,
	key: &str,
	before: &Value,
	after: &Value,
) -> io::Result<()> {
	write_diff(
		db,
		&codec::encode(key, before)?,
		&codec::encode(key, after)?,
	)?;
	mark_initialized(db, key)
}
pub(crate) fn write_diff(db: &Connection, baseline: &Rows, desired: &Rows) -> io::Result<()> {
	if baseline == desired {
		return Ok(());
	}
	let document = desired
		.keys()
		.chain(baseline.keys())
		.next()
		.map(|(i, _)| TABLES[*i].document);
	let current = all(db, document)?;
	for ((i, key), row) in desired {
		if baseline.get(&(*i, key.clone())) == Some(row)
			|| current.get(&(*i, key.clone())) == Some(row)
		{
			continue;
		}
		if *i <= 1
			&& baseline.contains_key(&(*i, key.clone()))
			&& !current.contains_key(&(*i, key.clone()))
		{
			continue;
		}
		// A stale day/log save cannot resurrect a habit deleted elsewhere.
		if (2..=4).contains(i) {
			let parent = serde_json::json!([row[0]]).to_string();
			if !desired.contains_key(&(1, parent.clone()))
				|| (baseline.contains_key(&(1, parent.clone()))
					&& !current.contains_key(&(1, parent)))
			{
				continue;
			}
		}
		let mut row = row.clone();
		if *i == 1 && !row[14].is_null() {
			let exists: bool = db
				.query_row(
					"SELECT EXISTS(SELECT 1 FROM habit_sections WHERE id=?1)",
					[row[14]
						.as_str()
						.ok_or_else(|| invalid("invalid section id"))?],
					|r| r.get(0),
				)
				.map_err(error)?;
			if !exists {
				row[14] = Value::Null;
			}
		}
		TABLES[*i].upsert(db, &row)?;
	}
	for ((i, key), _) in baseline.iter().rev() {
		if !desired.contains_key(&(*i, key.clone())) {
			TABLES[*i].delete(db, key)?;
		}
	}
	Ok(())
}
pub(crate) fn handles(key: &str) -> bool {
	matches!(key, "habits" | "shared.brain_dump" | "shared.task_work")
}
pub(crate) fn remember_value(path: &Path, key: &str, value: &Value) -> io::Result<()> {
	remember(path, key, codec::encode(key, value)?)
}

#[cfg(test)]
mod tests;
