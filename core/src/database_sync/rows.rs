//! Rows of the synced settings tables, replicated the way documents are.
//!
//! Every row is a document of its own in the journal, named after its table
//! and primary key: `places:[17]`, `settings:["calendar.sync_minutes"]`,
//! `travel_modes:[1,2]`. Its value is the row as a JSON object, with JSON
//! columns embedded as JSON. So the same journal, merge, snapshots and
//! compaction serve rows too: two devices editing different columns of one
//! row both keep their edit, and deleting a row removes it whatever was
//! changed in it meanwhile.
//!
//! Triggers journal every change an app makes to these tables. The sync
//! worker writes merged rows back with `projecting` set, so those writes are
//! not journaled again.

use rusqlite::{Connection, OptionalExtension};
use serde_json::{Map, Value};
use std::io;

use super::other;
use crate::settings::schema::{self, Table};

/// The table and primary key a journal document names, when it names a row.
pub(super) fn parse(document: &str) -> Option<(&'static Table, Vec<Value>)> {
	let (name, key) = document.split_once(':')?;
	let table = schema::synced().find(|table| table.name == name)?;
	let key: Vec<Value> = serde_json::from_str(key).ok()?;
	(key.len() == table.key.len()).then_some((table, key))
}

/// An SQL condition on `document` that holds for journal rows of synced rows.
pub(super) fn predicate() -> String {
	schema::synced()
		.map(|table| format!("document GLOB '{}:*'", table.name))
		.collect::<Vec<_>>()
		.join(" OR ")
}

const ACTIVE: &str = "(SELECT projecting FROM timeparagon_sync_control WHERE singleton=1)=0";

fn journal(document: &str, before: &str, after: &str) -> String {
	format!(
		"INSERT INTO timeparagon_sync_edits VALUES(lower(hex(randomblob(16))),{document},{before},{after},(SELECT coalesce(max(clock),0)+1 FROM timeparagon_sync_edits),0);"
	)
}

/// The triggers that journal `table`, by name.
fn triggers(table: &Table) -> Vec<(String, String)> {
	let name = table.name;
	let document = |prefix: &str| format!("'{name}:'||{}", table.key_sql(prefix));
	let (old, new) = (table.object_sql("OLD."), table.object_sql("NEW."));
	let (old_key, new_key) = (table.key_sql("OLD."), table.key_sql("NEW."));
	let trigger = |kind: &str, event: &str, condition: String, body: String| {
		let trigger = format!("timeparagon_sync_{name}_{kind}");
		let sql = format!(
			"CREATE TRIGGER {trigger} AFTER {event} ON {name} WHEN {condition} BEGIN {body} END"
		);
		(trigger, sql)
	};
	vec![
		trigger(
			"insert",
			"INSERT",
			ACTIVE.to_owned(),
			journal(&document("NEW."), "NULL", &new),
		),
		trigger(
			"update",
			"UPDATE",
			format!("{ACTIVE} AND {old_key} IS {new_key} AND {old} IS NOT {new}"),
			journal(&document("NEW."), &old, &new),
		),
		// A changed primary key is a different row: the old one is deleted.
		trigger(
			"rekey",
			"UPDATE",
			format!("{ACTIVE} AND {old_key} IS NOT {new_key}"),
			format!(
				"{} {}",
				journal(&document("OLD."), &old, "NULL"),
				journal(&document("NEW."), "NULL", &new)
			),
		),
		trigger(
			"delete",
			"DELETE",
			ACTIVE.to_owned(),
			journal(&document("OLD."), &old, "NULL"),
		),
	]
}

fn normalized(sql: &str) -> String {
	sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Creates the synced tables' triggers, replacing any that differ from what
/// this build would create (a table that gained a column, say).
pub(super) fn install(tx: &Connection) -> rusqlite::Result<()> {
	tx.execute_batch(schema::CREATE)?;
	schema::create_data(tx)?;
	for table in schema::synced() {
		for (name, sql) in triggers(table) {
			let existing: Option<String> = tx
				.query_row(
					"SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
					[&name],
					|row| row.get(0),
				)
				.optional()?;
			if existing.as_deref().map(normalized) == Some(normalized(&sql)) {
				continue;
			}
			if existing.is_some() {
				tx.execute_batch(&format!("DROP TRIGGER {name};"))?;
			}
			tx.execute_batch(&sql)?;
		}
	}
	Ok(())
}

/// Journals every row already there, as enrollment does for documents.
pub(super) fn journal_existing(tx: &Connection) -> rusqlite::Result<()> {
	for table in schema::synced() {
		tx.execute_batch(&format!(
            "INSERT INTO timeparagon_sync_edits SELECT lower(hex(randomblob(16))),'{}:'||{},NULL,{},1,0 FROM {};",
            table.name,
            table.key_sql(""),
            table.object_sql(""),
            table.name
        ))?;
	}
	Ok(())
}

/// Whether the stored row already holds every column `wanted` has. Columns
/// `wanted` lacks (it came from an older build) are not compared, or a row
/// would be rewritten on every cycle.
fn holds(table: &Table, current: &Map<String, Value>, wanted: &Map<String, Value>) -> bool {
	table
		.columns
		.iter()
		.filter(|column| wanted.contains_key(column.name))
		.all(|column| current.get(column.name) == wanted.get(column.name))
}

/// Makes the row named `key` of `table` what the journal says, `None` being
/// deleted. Returns whether that changed anything.
pub(super) fn project(
	tx: &Connection,
	table: &Table,
	key: &[Value],
	value: Option<&str>,
) -> io::Result<bool> {
	let current: Option<Value> = table
		.journal_row(tx, key)
		.map_err(other)?
		.map(|text| serde_json::from_str(&text))
		.transpose()
		.map_err(other)?;
	let wanted: Option<Value> = value.map(serde_json::from_str).transpose().map_err(other)?;
	match (current, wanted) {
		(None, None) => Ok(false),
		(Some(_), None) => {
			table.delete(tx, key).map_err(other)?;
			Ok(true)
		}
		(current, Some(Value::Object(mut wanted))) => {
			if let Some(Value::Object(current)) = &current
				&& holds(table, current, &wanted)
			{
				return Ok(false);
			}
			// The key comes from the document's name, which is what
			// identifies the row.
			for (column, value) in table.key.iter().zip(key) {
				wanted.insert((*column).to_owned(), value.clone());
			}
			table.upsert(tx, &wanted).map_err(other)?;
			Ok(true)
		}
		(_, Some(_)) => Err(other(format!(
			"a synced {} row is not an object",
			table.name
		))),
	}
}
