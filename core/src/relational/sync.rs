//! Native row replication. The durable outbox coalesces repeated edits to one
//! row. Tokens make acknowledgements safe against a save during network I/O.
//! Deletions are retained as remote tombstones so a stale device cannot revive
//! them simply by reconnecting. No edit replay, JSON merge or compaction.
use super::*;
use crate::database_sync::model::Edit;

#[derive(Clone, Debug)]
pub(crate) struct Change {
	pub table: usize,
	pub key: String,
	pub revision: String,
	pub deleted: bool,
	pub row: Option<Vec<Value>>,
}
fn open(path: &Path) -> io::Result<Connection> {
	let db = Connection::open(path).map_err(error)?;
	db.busy_timeout(std::time::Duration::from_secs(10))
		.map_err(error)?;
	db.execute_batch("PRAGMA foreign_keys=ON").map_err(error)?;
	Ok(db)
}
pub(crate) fn pending(db: &Connection) -> io::Result<Vec<Change>> {
	let mut q=db.prepare("SELECT table_name,row_key,revision,deleted FROM timeparagon_relational_state WHERE dirty=1 ORDER BY table_name,row_key").map_err(error)?;
	q.query_map([], |r| {
		Ok((
			r.get::<_, String>(0)?,
			r.get::<_, String>(1)?,
			r.get::<_, String>(2)?,
			r.get::<_, bool>(3)?,
		))
	})
	.map_err(error)?
	.map(|r| {
		let (name, key, revision, deleted) = r.map_err(error)?;
		let table = TABLES
			.iter()
			.position(|t| t.name == name)
			.ok_or_else(|| invalid("unknown relational outbox table"))?;
		let row = if deleted {
			None
		} else {
			TABLES[table].read_one(db, &key)?
		};
		if row.is_none() && !deleted {
			return Err(invalid("missing row in relational outbox"));
		}
		Ok(Change {
			table,
			key,
			revision,
			deleted,
			row,
		})
	})
	.collect()
}
pub(crate) fn acknowledge(db: &Connection, changes: &[Change]) -> io::Result<()> {
	if changes.is_empty() {
		return Ok(());
	}
	let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
		.map_err(error)?;
	for c in changes {
		tx.execute("UPDATE timeparagon_relational_state SET dirty=0 WHERE table_name=?1 AND row_key=?2 AND revision=?3",params![TABLES[c.table].name,c.key,c.revision]).map_err(error)?;
	}
	tx.commit().map_err(error)
}
pub(crate) fn apply(db: &Connection, changes: &[Change]) -> io::Result<bool> {
	if changes.is_empty() {
		return Ok(false);
	}
	let tx = rusqlite::Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Immediate)
		.map_err(error)?;
	tx.execute(
		"UPDATE timeparagon_relational_control SET projecting=1 WHERE id=1",
		[],
	)
	.map_err(error)?;
	let mut accepted = Vec::new();
	for c in changes {
		let dirty: bool=tx.query_row("SELECT coalesce((SELECT dirty FROM timeparagon_relational_state WHERE table_name=?1 AND row_key=?2),0)",params![TABLES[c.table].name,c.key],|r|r.get(0)).map_err(error)?;
		if !dirty {
			accepted.push(c);
		}
	}
	accepted.sort_by_key(|c| c.table);
	let before = all(&tx, None)?;
	for c in accepted.iter().rev().filter(|c| c.deleted) {
		TABLES[c.table].delete(&tx, &c.key)?;
	}
	for c in accepted.iter().filter(|c| !c.deleted) {
		let mut row = c
			.row
			.clone()
			.ok_or_else(|| invalid("replicated row has no columns"))?;
		let exists = |table: usize, key: String| -> io::Result<bool> {
			tx.query_row(
				&format!(
					"SELECT EXISTS(SELECT 1 FROM {} WHERE {})",
					TABLES[table].name,
					TABLES[table].where_key()
				),
				[key],
				|r| r.get(0),
			)
			.map_err(error)
		};
		if c.table == 1
			&& !row[14].is_null()
			&& !exists(0, serde_json::json!([row[14]]).to_string())?
		{
			row[14] = Value::Null;
		}
		if (2..=4).contains(&c.table) && !exists(1, serde_json::json!([row[0]]).to_string())? {
			continue;
		}
		if c.table == 4 && !exists(3, serde_json::json!([row[0], row[1]]).to_string())? {
			continue;
		}
		if before.get(&(c.table, c.key.clone())) != Some(&row) {
			TABLES[c.table].upsert(&tx, &row)?;
		}
	}
	let after = all(&tx, None)?;
	// Validate the complete typed view before committing anything received.
	for key in ["habits", "shared.brain_dump", "shared.task_work"] {
		codec::decode(key, &after)?;
	}
	for c in &accepted {
		if !c.deleted && !after.contains_key(&(c.table, c.key.clone())) {
			continue;
		}
		tx.execute("INSERT INTO timeparagon_relational_state VALUES(?1,?2,?3,?4,0) ON CONFLICT(table_name,row_key) DO UPDATE SET revision=excluded.revision,deleted=excluded.deleted,dirty=0",params![TABLES[c.table].name,c.key,c.revision,c.deleted]).map_err(error)?;
		mark_initialized(&tx, TABLES[c.table].document)?;
	}
	// A received parent deletion also retires pending edits to its children.
	for ((table, key), _) in before.iter().filter(|(key, _)| !after.contains_key(*key)) {
		tx.execute(
			"UPDATE timeparagon_relational_state SET dirty=0,deleted=1 WHERE table_name=?1 AND row_key=?2",
			params![TABLES[*table].name, key],
		)
		.map_err(error)?;
	}
	tx.execute(
		"UPDATE timeparagon_relational_control SET projecting=0 WHERE id=1",
		[],
	)
	.map_err(error)?;
	let changed = before != after;
	if changed {
		tx.execute(
			"UPDATE timeparagon_sync_control SET generation=generation+1 WHERE singleton=1",
			[],
		)
		.map_err(error)?;
	}
	tx.commit().map_err(error)?;
	Ok(changed)
}

pub(crate) async fn install(conn: &turso::Connection) -> io::Result<()> {
	for table in TABLES {
		conn.execute(&table.create(true), ()).await.map_err(error)?;
	}
	conn.execute(
		"CREATE TABLE IF NOT EXISTS timeparagon_relational_cutover(id INTEGER PRIMARY KEY)",
		(),
	)
	.await
	.map_err(error)?;
	Ok(())
}
fn native(value: &Value) -> io::Result<turso::Value> {
	use rusqlite::types::Value as S;
	Ok(match sql_value(value)? {
		S::Null => turso::Value::Null,
		S::Integer(n) => turso::Value::Integer(n),
		S::Real(n) => turso::Value::Real(n),
		S::Text(s) => turso::Value::Text(s),
		S::Blob(b) => turso::Value::Blob(b),
	})
}
pub(crate) async fn send(conn: &turso::Connection, changes: &[Change]) -> io::Result<()> {
	// One replica transaction prevents partial batches after an interrupted write.
	conn.execute("BEGIN IMMEDIATE", ()).await.map_err(error)?;
	let result = async {
		for c in changes {
			let table = &TABLES[c.table];
			let row: Vec<Value> = if c.deleted {
				serde_json::from_str(&c.key).map_err(invalid)?
			} else {
				c.row
					.clone()
					.ok_or_else(|| invalid("missing outbound row"))?
			};
			let columns = if c.deleted {
				&table.columns[..table.keys]
			} else {
				table.columns
			};
			let names = columns
				.iter()
				.map(|c| c.0)
				.chain(["sync_revision", "sync_deleted"])
				.collect::<Vec<_>>();
			let update = names[table.keys..]
				.iter()
				.map(|n| format!("{n}=excluded.{n}"))
				.collect::<Vec<_>>()
				.join(",");
			let keys = names[..table.keys].join(",");
			let mut values = row.iter().map(native).collect::<io::Result<Vec<_>>>()?;
			values.push(turso::Value::Text(c.revision.clone()));
			values.push(turso::Value::Integer(i64::from(c.deleted)));
			conn.execute(
				&format!(
					"INSERT INTO {}({}) VALUES({}) ON CONFLICT({keys}) DO UPDATE SET {update}",
					table.name,
					names.join(","),
					vec!["?"; names.len()].join(",")
				),
				values,
			)
			.await
			.map_err(error)?;
		}
		// The cutover marker is committed with the seed rows. An interrupted
		// push cannot publish the marker ahead of its migrated data.
		conn.execute(
			"INSERT OR IGNORE INTO timeparagon_relational_cutover VALUES(1)",
			(),
		)
		.await
		.map_err(error)?;
		Ok::<_, io::Error>(())
	}
	.await;
	match result {
		Ok(()) => {
			conn.execute("COMMIT", ()).await.map_err(error)?;
			Ok(())
		}
		Err(e) => {
			let _ = conn.execute("ROLLBACK", ()).await;
			Err(e)
		}
	}
}
pub(crate) async fn receive(conn: &turso::Connection, db: &Connection) -> io::Result<Vec<Change>> {
	receive_inner(conn, db, false).await
}
async fn receive_inner(
	conn: &turso::Connection,
	db: &Connection,
	force: bool,
) -> io::Result<Vec<Change>> {
	let mut result = Vec::new();
	for (i, table) in TABLES.iter().enumerate() {
		let mut rows = conn
			.query(
				&format!(
					"SELECT {},sync_revision,sync_deleted FROM {}",
					table.columns[..table.keys]
						.iter()
						.map(|c| c.0)
						.collect::<Vec<_>>()
						.join(","),
					table.name
				),
				(),
			)
			.await
			.map_err(error)?;
		let mut wanted = Vec::new();
		while let Some(row) = rows.next().await.map_err(error)? {
			let key_values = (0..table.keys)
				.map(|j| from_native(row.get_value(j).map_err(error)?, table.columns[j].1))
				.collect::<io::Result<Vec<_>>>()?;
			let key = Value::Array(key_values.clone()).to_string();
			let revision: String = row.get(table.keys).map_err(error)?;
			let deleted = row.get::<i64>(table.keys + 1).map_err(error)? != 0;
			let same: bool=db.query_row("SELECT EXISTS(SELECT 1 FROM timeparagon_relational_state WHERE table_name=?1 AND row_key=?2 AND revision=?3 AND dirty=0)",params![table.name,key,revision],|r|r.get(0)).map_err(error)?;
			if force || !same {
				wanted.push((key, key_values, revision, deleted));
			}
		}
		drop(rows);
		for (key, key_values, revision, deleted) in wanted {
			let row = if deleted {
				None
			} else {
				let condition = table.columns[..table.keys]
					.iter()
					.map(|c| format!("{}=?", c.0))
					.collect::<Vec<_>>()
					.join(" AND ");
				let mut rows = conn
					.query(
						&format!(
							"SELECT {} FROM {} WHERE {condition}",
							table.names(),
							table.name
						),
						key_values
							.iter()
							.map(native)
							.collect::<io::Result<Vec<_>>>()?,
					)
					.await
					.map_err(error)?;
				let row = rows
					.next()
					.await
					.map_err(error)?
					.ok_or_else(|| invalid("replica row disappeared"))?;
				Some(
					table
						.columns
						.iter()
						.enumerate()
						.map(|(j, (_, ty))| from_native(row.get_value(j).map_err(error)?, ty))
						.collect::<io::Result<Vec<_>>>()?,
				)
			};
			result.push(Change {
				table: i,
				key,
				revision,
				deleted,
				row,
			});
		}
	}
	Ok(result)
}
fn from_native(v: turso::Value, ty: &str) -> io::Result<Value> {
	Ok(match v {
		turso::Value::Null => Value::Null,
		turso::Value::Integer(n) if ty.starts_with("BOOLEAN") => Value::Bool(n != 0),
		turso::Value::Integer(n) => n.into(),
		turso::Value::Real(n) => serde_json::json!(n),
		turso::Value::Text(s) => s.into(),
		turso::Value::Blob(_) => return Err(invalid("unexpected blob in native row")),
	})
}

/// One-time import of the old cloud documents. This is the only use of the
/// retired merge engine for these features, strictly at the migration boundary.
/// Each device records the server cutover before normal row sync starts.
pub(crate) async fn bootstrap(
	conn: &turso::Connection,
	path: &Path,
	url: &str,
) -> io::Result<bool> {
	let db = open(path)?;
	let done: bool = db
		.query_row(
			"SELECT EXISTS(SELECT 1 FROM timeparagon_relational_servers WHERE url=?1)",
			[url],
			|r| r.get(0),
		)
		.map_err(error)?;
	let mut marker = conn
		.query(
			"SELECT id FROM timeparagon_relational_cutover WHERE id=1",
			(),
		)
		.await
		.map_err(error)?;
	let cutover = marker.next().await.map_err(error)?.is_some();
	drop(marker);
	if done && cutover {
		return Ok(false);
	}
	let mut history = BTreeMap::<String, Edit>::new();
	if !cutover {
		let mut rows = conn
			.query("SELECT payload FROM timeparagon_revisions_v1", ())
			.await
			.map_err(error)?;
		while let Some(row) = rows.next().await.map_err(error)? {
			let edit: Edit =
				serde_json::from_str(&row.get::<String>(0).map_err(error)?).map_err(invalid)?;
			if handles(&edit.document) {
				history.insert(edit.id.clone(), edit);
			}
		}
		let mut q=db.prepare("SELECT id,document,before_json,after_json,clock FROM timeparagon_sync_edits WHERE document IN ('habits','shared.brain_dump','shared.task_work')").map_err(error)?;
		let local = q
			.query_map([], |r| {
				Ok(Edit {
					id: r.get(0)?,
					document: r.get(1)?,
					before: r.get(2)?,
					after: r.get(3)?,
					clock: r.get(4)?,
					..Default::default()
				})
			})
			.map_err(error)?;
		for edit in local {
			let edit = edit.map_err(error)?;
			history.entry(edit.id.clone()).or_insert(edit);
		}
		let mut q=db.prepare("SELECT id,document,after_json,clock,folds FROM timeparagon_sync_snapshots WHERE document IN ('habits','shared.brain_dump','shared.task_work')").map_err(error)?;
		let local = q
			.query_map([], |r| {
				Ok((
					r.get::<_, String>(0)?,
					r.get::<_, String>(1)?,
					r.get::<_, Option<String>>(2)?,
					r.get::<_, i64>(3)?,
					r.get::<_, String>(4)?,
				))
			})
			.map_err(error)?;
		for e in local {
			let (id, document, after, clock, folds) = e.map_err(error)?;
			history.entry(id.clone()).or_insert(Edit {
				id,
				document,
				after,
				clock,
				folds: serde_json::from_str(&folds).map_err(invalid)?,
				snapshot: true,
				..Default::default()
			});
		}
	}
	let canonical = if cutover {
		receive_inner(conn, &db, true).await?
	} else {
		Vec::new()
	};
	let merged = crate::database_sync::model::project(&history.into_values().collect::<Vec<_>>())
		.map_err(error)?;
	let tx = rusqlite::Transaction::new_unchecked(&db, rusqlite::TransactionBehavior::Immediate)
		.map_err(error)?;
	for key in ["habits", "shared.brain_dump", "shared.task_work"] {
		let archive: Option<String> = tx
			.query_row(
				"SELECT value FROM timeparagon_relational_legacy WHERE key=?1",
				[key],
				|r| r.get(0),
			)
			.optional()
			.map_err(error)?;
		let baseline = archive
			.as_deref()
			.map(serde_json::from_str)
			.transpose()
			.map_err(invalid)?
			.map(|v| codec::encode(key, &v))
			.transpose()?
			.unwrap_or_default();
		let current = all(&tx, Some(key))?;
		if cutover {
			// Start with the canonical rows, then overlay only edits made on
			// this device since its legacy snapshot was imported.
			let mut wanted = Rows::new();
			for c in canonical
				.iter()
				.filter(|c| TABLES[c.table].document == key && !c.deleted)
			{
				wanted.insert(
					(c.table, c.key.clone()),
					c.row
						.clone()
						.ok_or_else(|| invalid("missing canonical row"))?,
				);
			}
			for (k, row) in &current {
				if baseline.get(k) != Some(row) {
					wanted.insert(k.clone(), row.clone());
				}
			}
			for k in baseline.keys().filter(|k| !current.contains_key(*k)) {
				wanted.remove(k);
			}
			normalize_parents(&mut wanted);
			// Clear outbox entries created solely by importing old JSON. New
			// edits retain their tokens and are sent after the cutover.
			for ((i, k), row) in &baseline {
				if current.get(&(*i, k.clone())) == Some(row)
					|| !wanted.contains_key(&(*i, k.clone()))
				{
					tx.execute(
						"DELETE FROM timeparagon_relational_state WHERE table_name=?1 AND row_key=?2",
						params![TABLES[*i].name, k],
					)
					.map_err(error)?;
				}
			}
			tx.execute(
				"UPDATE timeparagon_relational_control SET projecting=1 WHERE id=1",
				[],
			)
			.map_err(error)?;
			replace(&tx, &current, &wanted)?;
			tx.execute(
				"UPDATE timeparagon_relational_control SET projecting=0 WHERE id=1",
				[],
			)
			.map_err(error)?;
			for c in canonical.iter().filter(|c| TABLES[c.table].document == key) {
				let dirty: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM timeparagon_relational_state WHERE table_name=?1 AND row_key=?2 AND dirty=1)",params![TABLES[c.table].name,c.key],|r|r.get(0)).map_err(error)?;
				if !dirty {
					tx.execute("INSERT INTO timeparagon_relational_state VALUES(?1,?2,?3,?4,0) ON CONFLICT(table_name,row_key) DO UPDATE SET revision=excluded.revision,deleted=excluded.deleted,dirty=0",params![TABLES[c.table].name,c.key,c.revision,c.deleted]).map_err(error)?;
				}
			}
			mark_initialized(&tx, key)?;
		} else if let Some(merged) = merged.get(key) {
			let v = merged
				.as_deref()
				.map(serde_json::from_str)
				.transpose()
				.map_err(invalid)?
				.unwrap_or_else(|| serde_json::json!({}));
			let mut wanted = codec::encode(key, &v)?;
			// Preserve edits made locally after the versioned migration.
			for (k, row) in &current {
				if baseline.get(k) != Some(row) {
					wanted.insert(k.clone(), row.clone());
				}
			}
			for k in baseline.keys().filter(|k| !current.contains_key(*k)) {
				wanted.remove(k);
			}
			normalize_parents(&mut wanted);
			replace(&tx, &current, &wanted)?;
		}
	}
	if !cutover {
		// A new server also receives rows already acknowledged by a previous
		// server. State tokens remain stable across a retry.
		tx.execute("UPDATE timeparagon_relational_state SET dirty=1", [])
			.map_err(error)?;
	}
	tx.execute(
		"INSERT OR IGNORE INTO timeparagon_relational_servers VALUES(?1)",
		[url],
	)
	.map_err(error)?;
	tx.commit().map_err(error)?;
	Ok(!cutover)
}

pub(crate) async fn cycle(
	conn: &turso::Connection,
	replica: &turso::sync::Database,
	path: &Path,
) -> io::Result<()> {
	let pending = pending(&open(path)?)?;
	if !pending.is_empty() {
		send(conn, &pending).await?;
		replica.push().await.map_err(error)?;
		acknowledge(&open(path)?, &pending)?;
	}
	replica.pull().await.map_err(error)?;
	let db = open(path)?;
	let changes = receive(conn, &db).await?;
	apply(&db, &changes)?;
	Ok(())
}

// A deleted parent wins over a concurrent child edit. The remote tombstones
// retain the identity, while the local projection obeys its foreign keys.
fn normalize_parents(rows: &mut Rows) {
	let snapshot = rows.clone();
	rows.retain(|(i, _), r| {
		if (2..=4).contains(i)
			&& !snapshot.contains_key(&(1, serde_json::json!([r[0]]).to_string()))
		{
			return false;
		}
		if *i == 4 && !snapshot.contains_key(&(3, serde_json::json!([r[0], r[1]]).to_string())) {
			return false;
		}
		true
	});
	for ((i, _), r) in rows.iter_mut() {
		if *i == 1
			&& !r[14].is_null()
			&& !snapshot.contains_key(&(0, serde_json::json!([r[14]]).to_string()))
		{
			r[14] = Value::Null;
		}
	}
}
fn replace(db: &Connection, current: &Rows, wanted: &Rows) -> io::Result<()> {
	for ((i, k), row) in wanted {
		if current.get(&(*i, k.clone())) != Some(row) {
			TABLES[*i].upsert(db, row)?;
		}
	}
	for ((i, k), _) in current.iter().rev() {
		if !wanted.contains_key(&(*i, k.clone())) {
			TABLES[*i].delete(db, k)?;
		}
	}
	Ok(())
}
