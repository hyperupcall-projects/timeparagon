//! The tables that hold the settings and the other synced rows, described once
//! for the code that reads and writes them and for database sync, which
//! replicates their rows.

use rusqlite::types::{Value as Sql, ValueRef};
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde_json::{Map, Value};

/// How a column is kept, and how it appears in a row as JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
	Text,
	/// Text holding JSON. Sync merges inside it field by field, as it does
	/// for documents.
	Json,
}

#[derive(Debug)]
pub(crate) struct Column {
	pub name: &'static str,
	pub kind: Kind,
}

#[derive(Debug)]
pub(crate) struct Table {
	pub name: &'static str,
	/// The primary key's columns, in order.
	pub key: &'static [&'static str],
	/// Every column, the key's first.
	pub columns: &'static [Column],
	/// Rows are shared between devices by database sync.
	pub synced: bool,
	/// What one row is called in the sync log, as in "3 setting edits".
	pub noun: &'static str,
}

const fn column(name: &'static str, kind: Kind) -> Column {
	Column { name, kind }
}

use Kind::{Json, Text};

/// Preferences, named `section.field`, and the items of collections, named
/// `section.field/item` (see `settings::mapping`). Only values that differ
/// from the built-in default are stored.
pub(crate) const SETTINGS: Table = Table {
	name: "settings",
	key: &["key"],
	columns: &[column("key", Text), column("value", Json)],
	synced: true,
	noun: "setting",
};

/// The same, for preferences that describe this device and stay on it.
pub(crate) const DEVICE_SETTINGS: Table = Table {
	name: "device_settings",
	key: &["key"],
	columns: &[column("key", Text), column("value", Json)],
	synced: false,
	noun: "device setting",
};

/// Every settings table.
pub(crate) const TABLES: [&Table; 2] = [&SETTINGS, &DEVICE_SETTINGS];

/// Creates the tables. Idempotent.
pub(crate) const CREATE: &str = "
    CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS device_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);";

/// Metadata for Markdown notes, keyed by the ID in their front matter.
pub(crate) const NOTES: Table = Table {
	name: "notes",
	key: &["id"],
	columns: &[
		column("id", Text),
		column("color", Text),
		column("links", Json),
	],
	synced: true,
	noun: "note",
};

/// Synced tables of app data rather than preferences: database sync
/// replicates their rows like the settings', but they are not part of
/// `Config`, so nothing that reads or saves settings looks at them.
pub(crate) const DATA_TABLES: [&Table; 1] = [&NOTES];

/// Creates [`DATA_TABLES`], and `sleep_nights`, the nights of sleep as Google
/// Calendar last had them (see `calendar::sleep::store`). Those stay on the
/// device: each one gets them from Google. Installed during versioned database
/// setup and sync enrollment. Idempotent to preserve rows from earlier lazy
/// setup.
pub(crate) const CREATE_DATA: &str = "
    CREATE TABLE IF NOT EXISTS notes (id TEXT PRIMARY KEY, color TEXT NOT NULL DEFAULT 'butter', links TEXT NOT NULL DEFAULT '[]');
    CREATE TABLE IF NOT EXISTS sleep_nights (calendar_id TEXT NOT NULL, id TEXT NOT NULL, wake_date TEXT NOT NULL, start_at TEXT NOT NULL, end_at TEXT NOT NULL, event TEXT NOT NULL, PRIMARY KEY (calendar_id, id));";

/// Creates [`CREATE_DATA`]'s tables and the change counters' triggers on
/// them. Idempotent and cheap once done.
pub(crate) fn create_data(db: &Connection) -> rusqlite::Result<()> {
	db.execute_batch(CREATE_DATA)?;
	crate::local_changes::install_data(db)
}

pub(crate) fn table(name: &str) -> Option<&'static Table> {
	TABLES.into_iter().find(|table| table.name == name)
}

/// The tables whose rows database sync replicates.
pub(crate) fn synced() -> impl Iterator<Item = &'static Table> {
	TABLES
		.into_iter()
		.chain(DATA_TABLES)
		.filter(|table| table.synced)
}

fn invalid(error: impl std::fmt::Display) -> rusqlite::Error {
	rusqlite::Error::ToSqlConversionFailure(error.to_string().into())
}

/// A JSON value as the column stores it.
fn to_sql(kind: Kind, value: &Value) -> rusqlite::Result<Sql> {
	Ok(match (kind, value) {
		(_, Value::Null) => Sql::Null,
		(Json, value) => Sql::Text(value.to_string()),
		(Text, Value::String(text)) => Sql::Text(text.clone()),
		(Text, value) => return Err(invalid(format!("{value} cannot be stored as text"))),
	})
}

/// A column's value as JSON. JSON that does not parse reads as null, which
/// every reader takes as "use the default", rather than hiding every other
/// setting behind one damaged cell. Nothing writes the null back: saves only
/// touch what they changed.
fn from_sql(kind: Kind, value: ValueRef<'_>) -> rusqlite::Result<Value> {
	Ok(match (kind, value) {
		(_, ValueRef::Null) => Value::Null,
		(Json, ValueRef::Text(text)) => serde_json::from_slice(text).unwrap_or_else(|error| {
			log::warn!("ignoring a setting that is not JSON: {error}");
			Value::Null
		}),
		(_, ValueRef::Integer(number)) => number.into(),
		(_, ValueRef::Real(number)) => serde_json::Number::from_f64(number)
			.map(Value::Number)
			.unwrap_or(Value::Null),
		(_, ValueRef::Text(text)) => Value::String(String::from_utf8_lossy(text).into_owned()),
		(_, ValueRef::Blob(_)) => Value::Null,
	})
}

impl Table {
	fn kind(&self, name: &str) -> Kind {
		self.columns
			.iter()
			.find(|column| column.name == name)
			.map_or(Text, |column| column.kind)
	}

	/// The row as JSON, the way the sync journal holds it: `json_object(…)`
	/// over the columns of `prefix` (`NEW.`, `OLD.` or nothing).
	pub(crate) fn object_sql(&self, prefix: &str) -> String {
		let fields: Vec<String> = self
			.columns
			.iter()
			.map(|column| match column.kind {
				Json => format!("'{0}',json({prefix}{0})", column.name),
				Text => format!("'{0}',{prefix}{0}", column.name),
			})
			.collect();
		format!("json_object({})", fields.join(","))
	}

	/// The primary key as a JSON array, which names the row in the journal.
	pub(crate) fn key_sql(&self, prefix: &str) -> String {
		let columns: Vec<String> = self
			.key
			.iter()
			.map(|column| format!("{prefix}{column}"))
			.collect();
		format!("json_array({})", columns.join(","))
	}

	fn where_key(&self) -> String {
		self.key
			.iter()
			.enumerate()
			.map(|(index, column)| format!("{column}=?{}", index + 1))
			.collect::<Vec<_>>()
			.join(" AND ")
	}

	fn key_params(&self, key: &[Value]) -> rusqlite::Result<Vec<Sql>> {
		if key.len() != self.key.len() {
			return Err(invalid(format!(
				"{} needs a key of {}",
				self.name,
				self.key.len()
			)));
		}
		self.key
			.iter()
			.zip(key)
			.map(|(column, value)| to_sql(self.kind(column), value))
			.collect()
	}

	/// Stores a row given as JSON, replacing the columns it has. Columns it
	/// lacks keep the table's defaults (or their values, for a row already
	/// there); fields that are not columns are ignored.
	pub(crate) fn upsert(&self, db: &Connection, row: &Map<String, Value>) -> rusqlite::Result<()> {
		let present: Vec<&Column> = self
			.columns
			.iter()
			.filter(|column| row.contains_key(column.name))
			.collect();
		let values = present
			.iter()
			.map(|column| to_sql(column.kind, &row[column.name]))
			.collect::<rusqlite::Result<Vec<_>>>()?;
		let names: Vec<&str> = present.iter().map(|column| column.name).collect();
		let placeholders: Vec<String> = (1..=names.len()).map(|n| format!("?{n}")).collect();
		let updates: Vec<String> = names
			.iter()
			.filter(|name| !self.key.contains(name))
			.map(|name| format!("{name}=excluded.{name}"))
			.collect();
		let conflict = if updates.is_empty() {
			"DO NOTHING".to_owned()
		} else {
			format!("DO UPDATE SET {}", updates.join(","))
		};
		db.execute(
			&format!(
				"INSERT INTO {} ({}) VALUES ({}) ON CONFLICT({}) {conflict}",
				self.name,
				names.join(","),
				placeholders.join(","),
				self.key.join(",")
			),
			params_from_iter(values),
		)?;
		Ok(())
	}

	pub(crate) fn delete(&self, db: &Connection, key: &[Value]) -> rusqlite::Result<()> {
		db.execute(
			&format!("DELETE FROM {} WHERE {}", self.name, self.where_key()),
			params_from_iter(self.key_params(key)?),
		)?;
		Ok(())
	}

	/// Every row, each column as JSON of its kind.
	pub(crate) fn select_all(&self, db: &Connection) -> rusqlite::Result<Vec<Map<String, Value>>> {
		let names: Vec<&str> = self.columns.iter().map(|column| column.name).collect();
		let mut query = db.prepare(&format!(
			"SELECT {} FROM {} ORDER BY {}",
			names.join(","),
			self.name,
			self.key.join(",")
		))?;
		query
			.query_map([], |row| {
				let mut object = Map::new();
				for (index, column) in self.columns.iter().enumerate() {
					object.insert(
						column.name.to_owned(),
						from_sql(column.kind, row.get_ref(index)?)?,
					);
				}
				Ok(object)
			})?
			.collect()
	}

	/// One row as the sync journal would hold it, or `None` when it is absent.
	pub(crate) fn journal_row(
		&self,
		db: &Connection,
		key: &[Value],
	) -> rusqlite::Result<Option<String>> {
		db.query_row(
			&format!(
				"SELECT {} FROM {} WHERE {}",
				self.object_sql(""),
				self.name,
				self.where_key()
			),
			params_from_iter(self.key_params(key)?),
			|row| row.get(0),
		)
		.optional()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_descriptions_match_the_tables_created() {
		let db = Connection::open_in_memory().unwrap();
		db.execute_batch(CREATE).unwrap();
		db.execute_batch(CREATE_DATA).unwrap();
		for table in TABLES.into_iter().chain(DATA_TABLES) {
			let columns: Vec<(String, i64)> = db
				.prepare(&format!(
					"SELECT name, pk FROM pragma_table_info('{}') ORDER BY cid",
					table.name
				))
				.unwrap()
				.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
				.unwrap()
				.collect::<Result<_, _>>()
				.unwrap();
			let names: Vec<&str> = columns.iter().map(|(name, _)| name.as_str()).collect();
			let described: Vec<&str> = table.columns.iter().map(|column| column.name).collect();
			assert_eq!(names, described, "{}", table.name);
			let mut key: Vec<(i64, &str)> = columns
				.iter()
				.filter(|(_, pk)| *pk > 0)
				.map(|(name, pk)| (*pk, name.as_str()))
				.collect();
			key.sort();
			let key: Vec<&str> = key.into_iter().map(|(_, name)| name).collect();
			assert_eq!(key, table.key, "{}", table.name);
			assert_eq!(
				&described[..table.key.len()],
				table.key,
				"{} lists its key first",
				table.name
			);
		}
	}

	#[test]
	fn rows_round_trip_and_upserts_keep_the_columns_they_lack() {
		let db = Connection::open_in_memory().unwrap();
		db.execute_batch(CREATE_DATA).unwrap();
		let note = serde_json::json!({"id": "n1", "color": "rose", "links": [{"id": "h1"}]});
		let note = note.as_object().unwrap();
		NOTES.upsert(&db, note).unwrap();
		assert_eq!(NOTES.select_all(&db).unwrap(), vec![note.clone()]);

		let recolored = serde_json::json!({"id": "n1", "color": "mint"});
		NOTES.upsert(&db, recolored.as_object().unwrap()).unwrap();
		let stored = &NOTES.select_all(&db).unwrap()[0];
		assert_eq!(stored["color"], "mint");
		assert_eq!(
			stored["links"], note["links"],
			"only the given columns change"
		);

		let journal: Value =
			serde_json::from_str(&NOTES.journal_row(&db, &["n1".into()]).unwrap().unwrap())
				.unwrap();
		assert_eq!(journal["links"][0]["id"], "h1");
		NOTES.delete(&db, &["n1".into()]).unwrap();
		assert!(NOTES.journal_row(&db, &["n1".into()]).unwrap().is_none());

		db.execute_batch(CREATE).unwrap();
		let setting = serde_json::json!({"key": "alarm.sound", "value": {"volume": 25}});
		SETTINGS.upsert(&db, setting.as_object().unwrap()).unwrap();
		assert_eq!(SETTINGS.select_all(&db).unwrap()[0]["value"]["volume"], 25);
	}
}
