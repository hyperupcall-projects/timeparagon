//! The tables that hold the settings, described once for the code that reads
//! and writes them and for database sync, which replicates their rows.
//!
//! References between these tables (a route's places, an event's place, the
//! `travel.home` setting) are not enforced by SQLite. Two devices can delete a
//! place and refer to it at the same moment, and the merged result must still
//! be storable, so readers skip a reference to a row that is gone instead.

use rusqlite::types::{Value as Sql, ValueRef};
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde_json::{Map, Value};

/// How a column is kept, and how it appears in a row as JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
	Text,
	Integer,
	Real,
	/// An INTEGER that is 0 or 1; `true` or `false` in a row.
	Bool,
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
	/// What one row is called in the sync log, as in "3 place edits".
	pub noun: &'static str,
}

const fn column(name: &'static str, kind: Kind) -> Column {
	Column { name, kind }
}

use Kind::{Bool, Integer, Json, Real, Text};

/// Preferences that are one value each, named `section.field`. Only values
/// that differ from the built-in default are stored.
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

/// What the user chose for each calendar, task list or TimeParagon source.
pub(crate) const CALENDAR_SOURCES: Table = Table {
	name: "calendar_sources",
	key: &["id"],
	columns: &[
		column("id", Text),
		column("hidden", Bool),
		column("position", Integer),
		column("color", Text),
	],
	synced: true,
	noun: "calendar",
};

pub(crate) const SLEEP_SCHEDULE: Table = Table {
	name: "sleep_schedule",
	key: &["weekday"],
	columns: &[
		column("weekday", Integer),
		column("bedtime", Text),
		column("wakeup", Text),
	],
	synced: true,
	noun: "sleep schedule",
};

pub(crate) const SLEEP_SCHEDULE_CHANGES: Table = Table {
	name: "sleep_schedule_changes",
	key: &["from_date"],
	columns: &[
		column("from_date", Text),
		column("previous_bedtime", Text),
		column("previous_wakeup", Text),
		column("bedtime", Text),
		column("wakeup", Text),
	],
	synced: true,
	noun: "sleep schedule",
};

pub(crate) const WEEKLY_TASK_ORDER: Table = Table {
	name: "weekly_task_order",
	key: &["day"],
	columns: &[column("day", Text), column("cards", Json)],
	synced: true,
	noun: "week board",
};

pub(crate) const PLACES: Table = Table {
	name: "places",
	key: &["id"],
	columns: &[
		column("id", Integer),
		column("name", Text),
		column("address", Text),
		column("provider_id", Text),
		column("lat", Real),
		column("lng", Real),
		column("favorite", Bool),
	],
	synced: true,
	noun: "place",
};

pub(crate) const EVENT_PLACES: Table = Table {
	name: "event_places",
	key: &["event_id"],
	columns: &[column("event_id", Text), column("place_id", Integer)],
	synced: true,
	noun: "event place",
};

pub(crate) const TRAVEL_MODES: Table = Table {
	name: "travel_modes",
	key: &["from_place", "to_place"],
	columns: &[
		column("from_place", Integer),
		column("to_place", Integer),
		column("mode", Text),
	],
	synced: true,
	noun: "travel",
};

pub(crate) const TRAVEL_ROUTES: Table = Table {
	name: "travel_routes",
	key: &["from_place", "to_place", "mode"],
	columns: &[
		column("from_place", Integer),
		column("to_place", Integer),
		column("mode", Text),
		column("seconds", Integer),
		column("meters", Integer),
	],
	synced: true,
	noun: "travel",
};

pub(crate) const REMINDERS: Table = Table {
	name: "reminders",
	key: &["category"],
	columns: &[
		column("category", Text),
		column("enabled", Bool),
		column("minutes_before", Integer),
		column("sound", Json),
	],
	synced: true,
	noun: "reminder",
};

/// Every settings table.
pub(crate) const TABLES: [&Table; 11] = [
	&SETTINGS,
	&DEVICE_SETTINGS,
	&CALENDAR_SOURCES,
	&SLEEP_SCHEDULE,
	&SLEEP_SCHEDULE_CHANGES,
	&WEEKLY_TASK_ORDER,
	&PLACES,
	&EVENT_PLACES,
	&TRAVEL_MODES,
	&TRAVEL_ROUTES,
	&REMINDERS,
];

/// Creates the tables. Idempotent. A column added later needs a `DEFAULT`,
/// because rows synced from an older build do not have it.
pub(crate) const CREATE: &str = "
    CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS device_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS calendar_sources (id TEXT PRIMARY KEY, hidden INTEGER NOT NULL DEFAULT 0, position INTEGER, color TEXT);
    CREATE TABLE IF NOT EXISTS sleep_schedule (weekday INTEGER PRIMARY KEY CHECK (weekday BETWEEN 0 AND 6), bedtime TEXT NOT NULL, wakeup TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS sleep_schedule_changes (from_date TEXT PRIMARY KEY, previous_bedtime TEXT NOT NULL, previous_wakeup TEXT NOT NULL, bedtime TEXT NOT NULL, wakeup TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS weekly_task_order (day TEXT PRIMARY KEY, cards TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS places (id INTEGER PRIMARY KEY, name TEXT NOT NULL, address TEXT NOT NULL DEFAULT '', provider_id TEXT NOT NULL DEFAULT '', lat REAL NOT NULL, lng REAL NOT NULL, favorite INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE IF NOT EXISTS event_places (event_id TEXT PRIMARY KEY, place_id INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS travel_modes (from_place INTEGER NOT NULL, to_place INTEGER NOT NULL, mode TEXT NOT NULL, PRIMARY KEY (from_place, to_place));
    CREATE TABLE IF NOT EXISTS travel_routes (from_place INTEGER NOT NULL, to_place INTEGER NOT NULL, mode TEXT NOT NULL, seconds INTEGER NOT NULL, meters INTEGER NOT NULL, PRIMARY KEY (from_place, to_place, mode));
    CREATE TABLE IF NOT EXISTS reminders (category TEXT PRIMARY KEY, enabled INTEGER NOT NULL, minutes_before INTEGER NOT NULL, sound TEXT NOT NULL);";

/// Nights of sleep as Google Calendar last had them (see
/// `calendar::sleep::store`). Replicated so another device shows them before
/// its own Google sync. `event` is plain text, not merged field by field:
/// each copy is a whole Google event, and the newest one wins.
pub(crate) const SLEEP_NIGHTS: Table = Table {
	name: "sleep_nights",
	key: &["calendar_id", "id"],
	columns: &[
		column("calendar_id", Text),
		column("id", Text),
		column("wake_date", Text),
		column("start_at", Text),
		column("end_at", Text),
		column("event", Text),
	],
	synced: true,
	noun: "sleep",
};

/// Synced tables of app data rather than preferences: database sync
/// replicates their rows like the settings tables', but they are not part of
/// `Config`, so nothing that reads or saves settings looks at them.
pub(crate) const DATA_TABLES: [&Table; 1] = [&SLEEP_NIGHTS];

/// Creates [`DATA_TABLES`]. Idempotent, and run on first use and whenever
/// database sync starts, so no schema version change was needed to add them.
pub(crate) const CREATE_DATA: &str = "
    CREATE TABLE IF NOT EXISTS sleep_nights (calendar_id TEXT NOT NULL, id TEXT NOT NULL, wake_date TEXT NOT NULL, start_at TEXT NOT NULL, end_at TEXT NOT NULL, event TEXT NOT NULL, PRIMARY KEY (calendar_id, id));";

/// Creates [`DATA_TABLES`] and the change counters' triggers on them.
/// Idempotent and cheap once done.
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
		(Integer | Bool, Value::Bool(flag)) => Sql::Integer(i64::from(*flag)),
		(Bool, Value::Number(number)) => Sql::Integer(i64::from(number.as_f64() != Some(0.0))),
		(Integer, Value::Number(number)) => Sql::Integer(
			number
				.as_i64()
				.ok_or_else(|| invalid(format!("{number} is not a whole number")))?,
		),
		(Real, Value::Number(number)) => Sql::Real(
			number
				.as_f64()
				.ok_or_else(|| invalid(format!("{number} is not a number")))?,
		),
		(kind, value) => return Err(invalid(format!("{value} cannot be stored as {kind:?}"))),
	})
}

/// A column's value as JSON. JSON that does not parse reads as null, which
/// every reader takes as "use the default", rather than hiding every other
/// setting behind one damaged cell. Nothing writes the null back: saves only
/// touch what they changed.
fn from_sql(kind: Kind, value: ValueRef<'_>) -> rusqlite::Result<Value> {
	Ok(match (kind, value) {
		(_, ValueRef::Null) => Value::Null,
		(Bool, ValueRef::Integer(number)) => Value::Bool(number != 0),
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
	pub(crate) fn column(&self, name: &str) -> Option<&Column> {
		self.columns.iter().find(|column| column.name == name)
	}

	fn kind(&self, name: &str) -> Kind {
		self.column(name).map_or(Text, |column| column.kind)
	}

	/// The row as JSON, the way the sync journal holds it: `json_object(…)`
	/// over the columns of `prefix` (`NEW.`, `OLD.` or nothing).
	pub(crate) fn object_sql(&self, prefix: &str) -> String {
		let fields: Vec<String> = self
			.columns
			.iter()
			.map(|column| match column.kind {
				Json => format!("'{0}',json({prefix}{0})", column.name),
				_ => format!("'{0}',{prefix}{0}", column.name),
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

	/// The primary key of a row given as JSON.
	pub(crate) fn key_of(&self, row: &Map<String, Value>) -> Vec<Value> {
		self.key
			.iter()
			.map(|column| row.get(*column).cloned().unwrap_or(Value::Null))
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

	/// Sets `columns` of an existing row from `row`, leaving its other
	/// columns as they are. Returns whether the row was there.
	pub(crate) fn update(
		&self,
		db: &Connection,
		row: &Map<String, Value>,
		columns: &[&str],
	) -> rusqlite::Result<bool> {
		let key = self.key_params(&self.key_of(row))?;
		let columns: Vec<&Column> = columns
			.iter()
			.filter_map(|name| self.column(name))
			.collect();
		if columns.is_empty() {
			return Ok(db
				.query_row(
					&format!("SELECT 1 FROM {} WHERE {}", self.name, self.where_key()),
					params_from_iter(key),
					|_| Ok(()),
				)
				.optional()?
				.is_some());
		}
		let offset = key.len();
		let assignments: Vec<String> = columns
			.iter()
			.enumerate()
			.map(|(index, column)| format!("{}=?{}", column.name, offset + index + 1))
			.collect();
		let mut values = key;
		for column in &columns {
			values.push(to_sql(
				column.kind,
				row.get(column.name).unwrap_or(&Value::Null),
			)?);
		}
		let changed = db.execute(
			&format!(
				"UPDATE {} SET {} WHERE {}",
				self.name,
				assignments.join(","),
				self.where_key()
			),
			params_from_iter(values),
		)?;
		Ok(changed > 0)
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
			assert!(table.key.iter().all(|name| table.column(name).is_some()));
			assert_eq!(
				&described[..table.key.len()],
				table.key,
				"{} lists its key first",
				table.name
			);
		}
	}

	#[test]
	fn rows_round_trip_and_partial_updates_keep_other_columns() {
		let db = Connection::open_in_memory().unwrap();
		db.execute_batch(CREATE).unwrap();
		let place = serde_json::json!({"id": 7, "name": "Library", "address": "1 Main St",
            "provider_id": "mapbox-1", "lat": 1.5, "lng": 2.5, "favorite": true});
		let place = place.as_object().unwrap();
		PLACES.upsert(&db, place).unwrap();
		assert_eq!(PLACES.select_all(&db).unwrap(), vec![place.clone()]);

		let mut renamed = place.clone();
		renamed.insert("name".into(), "Campus".into());
		renamed.insert("favorite".into(), false.into());
		assert!(PLACES.update(&db, &renamed, &["name"]).unwrap());
		let stored = &PLACES.select_all(&db).unwrap()[0];
		assert_eq!(stored["name"], "Campus");
		assert_eq!(stored["favorite"], true, "only the named columns change");

		let journal: Value =
			serde_json::from_str(&PLACES.journal_row(&db, &[7.into()]).unwrap().unwrap()).unwrap();
		assert_eq!(journal["favorite"], 1);
		PLACES.delete(&db, &[7.into()]).unwrap();
		assert!(!PLACES.update(&db, &renamed, &["name"]).unwrap());
		assert!(PLACES.journal_row(&db, &[7.into()]).unwrap().is_none());

		let setting = serde_json::json!({"key": "alarm.sound", "value": {"volume": 25}});
		SETTINGS.upsert(&db, setting.as_object().unwrap()).unwrap();
		assert_eq!(SETTINGS.select_all(&db).unwrap()[0]["value"]["volume"], 25);
	}
}
