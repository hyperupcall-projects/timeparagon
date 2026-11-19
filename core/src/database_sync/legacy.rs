//! Schema 10: the settings tables that held one collection each are retired,
//! and nights of sleep are no longer replicated.
//!
//! Each row of the retired tables becomes a `settings` row, named the way
//! `settings::mapping` names that item (`places:[17]` becomes
//! `travel.places/17`). Their sync history moves with them: every edit keeps
//! its ID and clock and is renamed and reshaped in place, here and when it
//! arrives from the server or from a device still on an older build. So every
//! device holds the same history under the same names, and merging carries on
//! as before.

use std::io;

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use super::{delete_row, model::Edit, other};
use crate::settings::mapping::item_name;

/// A retired table: its primary key, and what its rows are named now.
struct Retired {
	table: &'static str,
	key: &'static [&'static str],
	/// The name of a row is this followed by its key, as `item_name` joins it.
	prefix: &'static str,
}

const RETIRED: [Retired; 9] = [
	Retired {
		table: "calendar_sources",
		key: &["id"],
		prefix: "calendar.sources/",
	},
	Retired {
		table: "sleep_schedule",
		key: &["weekday"],
		prefix: "calendar.sleep_schedule/",
	},
	Retired {
		table: "sleep_schedule_changes",
		key: &["from_date"],
		prefix: "calendar.sleep_schedule_changes/",
	},
	Retired {
		table: "weekly_task_order",
		key: &["day"],
		prefix: "calendar.weekly_task_order/",
	},
	Retired {
		table: "places",
		key: &["id"],
		prefix: "travel.places/",
	},
	Retired {
		table: "event_places",
		key: &["event_id"],
		prefix: "travel.event_places/",
	},
	Retired {
		table: "travel_modes",
		key: &["from_place", "to_place"],
		prefix: "travel.modes/",
	},
	Retired {
		table: "travel_routes",
		key: &["from_place", "to_place", "mode"],
		prefix: "travel.routes/",
	},
	// Each kind of reminder is an ordinary setting.
	Retired {
		table: "reminders",
		key: &["category"],
		prefix: "reminders.",
	},
];

/// Columns of the retired tables that held JSON.
const JSON_COLUMNS: [&str; 2] = ["cards", "sound"];

/// The journal's documents for nights of sleep.
const SLEEP: &str = "sleep_nights:";

/// Whether `document` is a night of sleep, which is no longer replicated.
pub(super) fn is_sleep(document: &str) -> bool {
	document.starts_with(SLEEP)
}

/// A retired row, as the journal holds it, in the shape `settings::mapping`
/// gives the same item. Integers stood for flags.
fn value(table: &str, row: &Map<String, Value>) -> Value {
	let get = |name: &str| row.get(name).cloned().unwrap_or(Value::Null);
	let flag = |name: &str| {
		row.get(name).is_some_and(|value| {
			value
				.as_i64()
				.map_or(value == &Value::Bool(true), |n| n != 0)
		})
	};
	let renamed = |fields: &[(&str, &str)]| -> Value {
		let fields = fields
			.iter()
			.filter_map(|(name, column)| Some(((*name).to_owned(), row.get(*column)?.clone())));
		Value::Object(fields.collect())
	};
	match table {
		"calendar_sources" => {
			json!({"hidden": flag("hidden"), "position": get("position"), "color": get("color")})
		}
		"sleep_schedule" => renamed(&[("bedtime", "bedtime"), ("wakeup", "wakeup")]),
		"sleep_schedule_changes" => json!({
			"from": get("from_date"),
			"previous": {"bedtime": get("previous_bedtime"), "wakeup": get("previous_wakeup")},
			"times": {"bedtime": get("bedtime"), "wakeup": get("wakeup")},
		}),
		"weekly_task_order" => get("cards"),
		"places" => {
			let mut place = renamed(&[
				("id", "id"),
				("name", "name"),
				("address", "address"),
				("place_id", "provider_id"),
				("lat", "lat"),
				("lng", "lng"),
			]);
			if flag("favorite") {
				place["favorite"] = true.into();
			}
			place
		}
		"event_places" => get("place_id"),
		"travel_modes" => renamed(&[("from", "from_place"), ("to", "to_place"), ("mode", "mode")]),
		"travel_routes" => renamed(&[
			("from", "from_place"),
			("to", "to_place"),
			("mode", "mode"),
			("seconds", "seconds"),
			("meters", "meters"),
		]),
		"reminders" => json!({"enabled": flag("enabled"),
            "minutes_before": get("minutes_before"), "sound": get("sound")}),
		_ => unreachable!("only retired tables are translated"),
	}
}

impl Retired {
	fn name(&self, key: &[Value]) -> String {
		format!("{}{}", self.prefix, item_name(key))
	}

	/// A journaled row of this table as the `settings` row `name`. Text that
	/// is not a row is kept as it is, for sync to report as it always has.
	fn row(&self, name: &str, text: &str) -> String {
		match serde_json::from_str::<Map<String, Value>>(text) {
			Ok(row) => json!({"key": name, "value": value(self.table, &row)}).to_string(),
			Err(_) => text.to_owned(),
		}
	}
}

/// `edit` renamed and reshaped for `settings`, when it is an edit to a row
/// of a retired table.
pub(super) fn translate(edit: &Edit) -> Option<Edit> {
	let (table, key) = edit.document.split_once(':')?;
	let retired = RETIRED.iter().find(|retired| retired.table == table)?;
	let key: Vec<Value> = serde_json::from_str(key).ok()?;
	if key.len() != retired.key.len() {
		return None;
	}
	let name = retired.name(&key);
	let row = |text: &Option<String>| text.as_deref().map(|text| retired.row(&name, text));
	Some(Edit {
		document: format!("settings:{}", json!([name])),
		before: row(&edit.before),
		after: row(&edit.after),
		..edit.clone()
	})
}

fn exists(tx: &Connection, table: &str) -> rusqlite::Result<bool> {
	Ok(tx
		.query_row(
			"SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
			[table],
			|_| Ok(()),
		)
		.optional()?
		.is_some())
}

/// Stops journaling nights of sleep, and deletes their history here and,
/// through tombstones, on the server.
fn retire_sleep(tx: &Connection, at: i64) -> io::Result<()> {
	for kind in ["insert", "update", "rekey", "delete"] {
		tx.execute_batch(&format!(
			"DROP TRIGGER IF EXISTS timeparagon_sync_sleep_nights_{kind};"
		))
		.map_err(other)?;
	}
	let ids: Vec<String> = tx
		.prepare(
			"SELECT id FROM timeparagon_sync_edits WHERE document GLOB 'sleep_nights:*'
			 UNION ALL SELECT id FROM timeparagon_sync_snapshots WHERE document GLOB 'sleep_nights:*'",
		)
		.and_then(|mut query| query.query_map([], |row| row.get(0))?.collect())
		.map_err(other)?;
	for id in ids {
		delete_row(tx, &id, false, at)?;
	}
	Ok(())
}

/// Renames and reshapes the journal's edits to rows of retired tables.
fn translate_journal(tx: &Connection) -> io::Result<()> {
	let filter = RETIRED
		.iter()
		.map(|retired| format!("document GLOB '{}:*'", retired.table))
		.collect::<Vec<_>>()
		.join(" OR ");
	let edits: Vec<Edit> = tx
		.prepare(&format!(
			"SELECT id,document,before_json,after_json FROM timeparagon_sync_edits WHERE {filter}"
		))
		.and_then(|mut query| {
			query
				.query_map([], |row| {
					Ok(Edit {
						id: row.get(0)?,
						document: row.get(1)?,
						before: row.get(2)?,
						after: row.get(3)?,
						..Default::default()
					})
				})?
				.collect()
		})
		.map_err(other)?;
	for edit in edits.iter().filter_map(translate) {
		tx.execute(
			"UPDATE timeparagon_sync_edits SET document=?2,before_json=?3,after_json=?4 WHERE id=?1",
			params![edit.id, edit.document, edit.before, edit.after],
		)
		.map_err(other)?;
	}
	let snapshots: Vec<Edit> = tx
		.prepare(&format!(
			"SELECT id,document,after_json FROM timeparagon_sync_snapshots WHERE {filter}"
		))
		.and_then(|mut query| {
			query
				.query_map([], |row| {
					Ok(Edit {
						id: row.get(0)?,
						document: row.get(1)?,
						after: row.get(2)?,
						..Default::default()
					})
				})?
				.collect()
		})
		.map_err(other)?;
	for snapshot in snapshots.iter().filter_map(translate) {
		tx.execute(
			"UPDATE timeparagon_sync_snapshots SET document=?2,after_json=?3 WHERE id=?1",
			params![snapshot.id, snapshot.document, snapshot.after],
		)
		.map_err(other)?;
	}
	Ok(())
}

/// Moves the rows of `retired` into `settings` and drops the table, with its
/// triggers. The rows are already journaled (their history was renamed), so
/// they are written the way sync writes, without journaling them again.
fn move_rows(tx: &Connection, retired: &Retired) -> io::Result<()> {
	let columns: Vec<String> = tx
		.prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
		.and_then(|mut query| {
			query
				.query_map([retired.table], |row| row.get(0))?
				.collect()
		})
		.map_err(other)?;
	let object = columns
		.iter()
		.map(|column| {
			if JSON_COLUMNS.contains(&column.as_str()) {
				// Refuse a damaged cell rather than dropping its only original
				// when the retired table is removed. The transaction can retry
				// after the data has been repaired.
				format!("'{column}',json({column})")
			} else {
				format!("'{column}',{column}")
			}
		})
		.collect::<Vec<_>>()
		.join(",");
	let rows: Vec<(String, String)> = tx
		.prepare(&format!(
			"SELECT json_array({}), json_object({object}) FROM {}",
			retired.key.join(","),
			retired.table
		))
		.and_then(|mut query| {
			query
				.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
				.collect()
		})
		.map_err(other)?;
	for (key, row) in rows {
		let key: Vec<Value> = serde_json::from_str(&key).map_err(other)?;
		let row: Map<String, Value> = serde_json::from_str(&row).map_err(other)?;
		tx.execute(
			"INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
			params![retired.name(&key), value(retired.table, &row).to_string()],
		)
		.map_err(other)?;
	}
	tx.execute_batch(&format!("DROP TABLE {};", retired.table))
		.map_err(other)
}

/// Brings a file's settings and sync journal to schema 10, inside the
/// caller's transaction, which must have created the journal's tables.
/// Idempotent, and cheap once done.
pub(super) fn migrate(tx: &Connection, at: i64) -> io::Result<()> {
	retire_sleep(tx, at)?;
	translate_journal(tx)?;
	let mut present = Vec::new();
	for retired in &RETIRED {
		if exists(tx, retired.table).map_err(other)? {
			present.push(retired);
		}
	}
	if present.is_empty() {
		return Ok(());
	}
	tx.execute_batch(crate::settings::schema::CREATE)
		.map_err(other)?;
	tx.execute(
		"UPDATE timeparagon_sync_control SET projecting=1 WHERE singleton=1",
		[],
	)
	.map_err(other)?;
	for retired in present {
		move_rows(tx, retired)?;
	}
	tx.execute(
		"UPDATE timeparagon_sync_control SET projecting=0 WHERE singleton=1",
		[],
	)
	.map_err(other)?;
	log::info!("moved the retired settings tables into settings");
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::database_sync::{apply, edits, enroll, history, now, open, tests::send};
	use std::path::Path;

	/// The rows of the retired tables, as their journals held them, that hold
	/// the settings of `settings::mapping`'s sample.
	fn old_rows() -> Vec<(&'static str, Value, Value)> {
		vec![
			(
				"calendar_sources",
				json!(["work"]),
				json!({"id": "work", "hidden": 1, "position": 1, "color": null}),
			),
			(
				"calendar_sources",
				json!(["tasks:abc"]),
				json!({"id": "tasks:abc", "hidden": 1, "position": null, "color": "#112233"}),
			),
			(
				"calendar_sources",
				json!(["b"]),
				json!({"id": "b", "hidden": 0, "position": 0, "color": null}),
			),
			(
				"calendar_sources",
				json!(["cal@group.calendar.google.com"]),
				json!({"id": "cal@group.calendar.google.com", "hidden": 0, "position": null,
					"color": "#445566"}),
			),
			(
				"sleep_schedule",
				json!([0]),
				json!({"weekday": 0, "bedtime": "23:00:00", "wakeup": "07:00:00"}),
			),
			(
				"sleep_schedule_changes",
				json!(["2026-11-10"]),
				json!({"from_date": "2026-11-10", "previous_bedtime": "00:00:00",
					"previous_wakeup": "08:00:00", "bedtime": "23:00:00", "wakeup": "07:00:00"}),
			),
			(
				"weekly_task_order",
				json!(["2026-11-10"]),
				json!({"day": "2026-11-10", "cards": ["x", "y"]}),
			),
			(
				"places",
				json!([1730000000000_u64]),
				json!({"id": 1730000000000_u64, "name": "Home", "address": "1 Main St",
					"provider_id": "mapbox-1", "lat": 1.5, "lng": 2.5, "favorite": 1}),
			),
			(
				"event_places",
				json!(["standup_20261110T090000Z"]),
				json!({"event_id": "standup_20261110T090000Z", "place_id": 1730000000000_u64}),
			),
			(
				"travel_modes",
				json!([1, 2]),
				json!({"from_place": 1, "to_place": 2, "mode": "car"}),
			),
			(
				"travel_routes",
				json!([1, 2, "car"]),
				json!({"from_place": 1, "to_place": 2, "mode": "car", "seconds": 600,
					"meters": 5000}),
			),
			(
				"reminders",
				json!(["events"]),
				json!({"category": "events", "enabled": 0, "minutes_before": 15,
					"sound": {"kind": "tone", "tone": "bell"}}),
			),
		]
	}

	fn old_edit(id: &str, table: &str, key: &Value, row: &Value, clock: i64) -> Edit {
		Edit {
			id: id.into(),
			document: format!("{table}:{key}"),
			after: Some(row.to_string()),
			clock,
			..Default::default()
		}
	}

	#[test]
	fn translated_rows_are_what_saving_the_same_settings_stores() {
		let translated: std::collections::BTreeMap<String, Value> = old_rows()
			.iter()
			.map(|(table, key, row)| {
				let edit = translate(&old_edit("e", table, key, row, 1)).unwrap();
				let row: Value = serde_json::from_str(edit.after.as_ref().unwrap()).unwrap();
				let name = row["key"].as_str().unwrap().to_owned();
				assert_eq!(edit.document, format!("settings:{}", json!([name])));
				(name, row["value"].clone())
			})
			.collect();
		let saved: std::collections::BTreeMap<String, Value> =
			crate::settings::mapping::rows(&crate::settings::mapping::tests::sample())
				.into_iter()
				.filter(|((_, key), _)| key.contains('/') || key == "reminders.events")
				.map(|((_, key), value)| (key, value))
				.collect();
		assert_eq!(translated, saved);
	}

	#[test]
	fn a_translated_name_is_the_one_the_journal_triggers_give() {
		let (_d, path) = db();
		let db = open(&path).unwrap();
		for (table, key, row) in old_rows() {
			let edit = translate(&old_edit("e", table, &key, &row, 1)).unwrap();
			let name: Value = serde_json::from_str(edit.after.as_ref().unwrap()).unwrap();
			db.execute(
				"INSERT INTO settings VALUES(?1,'1')",
				[name["key"].as_str().unwrap()],
			)
			.unwrap();
			let journaled: String = db
				.query_row(
					"SELECT document FROM timeparagon_sync_edits ORDER BY rowid DESC LIMIT 1",
					[],
					|row| row.get(0),
				)
				.unwrap();
			assert_eq!(journaled, edit.document);
		}
	}

	fn db() -> (tempfile::TempDir, std::path::PathBuf) {
		let directory = tempfile::tempdir().unwrap();
		let path = directory.path().join("test.sqlite3");
		crate::task_db::Database::open(&path).unwrap();
		(directory, path)
	}

	const OLD_TABLES: &str = "
		CREATE TABLE calendar_sources (id TEXT PRIMARY KEY, hidden INTEGER NOT NULL DEFAULT 0, position INTEGER, color TEXT);
		CREATE TABLE sleep_schedule (weekday INTEGER PRIMARY KEY CHECK (weekday BETWEEN 0 AND 6), bedtime TEXT NOT NULL, wakeup TEXT NOT NULL);
		CREATE TABLE sleep_schedule_changes (from_date TEXT PRIMARY KEY, previous_bedtime TEXT NOT NULL, previous_wakeup TEXT NOT NULL, bedtime TEXT NOT NULL, wakeup TEXT NOT NULL);
		CREATE TABLE weekly_task_order (day TEXT PRIMARY KEY, cards TEXT NOT NULL);
		CREATE TABLE places (id INTEGER PRIMARY KEY, name TEXT NOT NULL, address TEXT NOT NULL DEFAULT '', provider_id TEXT NOT NULL DEFAULT '', lat REAL NOT NULL, lng REAL NOT NULL, favorite INTEGER NOT NULL DEFAULT 0);
		CREATE TABLE event_places (event_id TEXT PRIMARY KEY, place_id INTEGER NOT NULL);
		CREATE TABLE travel_modes (from_place INTEGER NOT NULL, to_place INTEGER NOT NULL, mode TEXT NOT NULL, PRIMARY KEY (from_place, to_place));
		CREATE TABLE travel_routes (from_place INTEGER NOT NULL, to_place INTEGER NOT NULL, mode TEXT NOT NULL, seconds INTEGER NOT NULL, meters INTEGER NOT NULL, PRIMARY KEY (from_place, to_place, mode));
		CREATE TABLE reminders (category TEXT PRIMARY KEY, enabled INTEGER NOT NULL, minutes_before INTEGER NOT NULL, sound TEXT NOT NULL);";

	/// A schema 9 file: the retired tables with their rows, every row
	/// journaled and on the server already.
	fn old_file(path: &Path) {
		let db = open(path).unwrap();
		db.execute_batch(OLD_TABLES).unwrap();
		for (index, (table, key, row)) in old_rows().into_iter().enumerate() {
			let row = row.as_object().unwrap();
			let names: Vec<&str> = row.keys().map(String::as_str).collect();
			let values: Vec<rusqlite::types::Value> = row
				.values()
				.map(|value| match value {
					Value::Null => rusqlite::types::Value::Null,
					Value::String(text) => rusqlite::types::Value::Text(text.clone()),
					Value::Number(number) if number.is_f64() => {
						rusqlite::types::Value::Real(number.as_f64().unwrap())
					}
					Value::Number(number) => {
						rusqlite::types::Value::Integer(number.as_i64().unwrap())
					}
					other => rusqlite::types::Value::Text(other.to_string()),
				})
				.collect();
			let placeholders: Vec<String> = (1..=names.len()).map(|n| format!("?{n}")).collect();
			db.execute(
				&format!(
					"INSERT INTO {table}({}) VALUES({})",
					names.join(","),
					placeholders.join(",")
				),
				rusqlite::params_from_iter(values),
			)
			.unwrap();
			let edit = old_edit(
				&format!("old-{index}"),
				table,
				&key,
				&Value::Object(row.clone()),
				2,
			);
			db.execute(
				"INSERT INTO timeparagon_sync_edits VALUES(?1,?2,NULL,?3,?4,1)",
				params![edit.id, edit.document, edit.after, edit.clock],
			)
			.unwrap();
		}
		db.execute_batch("PRAGMA user_version=9;").unwrap();
	}

	#[test]
	fn upgrades_preserve_snapshots_and_pending_offline_edits_on_both_devices() {
		use crate::{config::Config, task_db::Database};
		let (_a, desktop) = db();
		let (_b, phone) = db();
		let (_, key, original) = &old_rows()[7];
		let snapshot = Edit {
			snapshot: true,
			folds: vec!["old-7".into()],
			..old_edit("snapshot-place", "places", key, original, 2)
		};
		let mut server: std::collections::HashMap<String, Edit> = old_rows()
			.iter()
			.enumerate()
			.filter(|(i, _)| *i != 7)
			.map(|(i, (table, key, row))| {
				let edit = old_edit(&format!("old-{i}"), table, key, row, 2);
				(edit.id.clone(), edit)
			})
			.collect();
		server.insert(snapshot.id.clone(), snapshot.clone());
		for (path, id, field, changed) in [
			(&desktop, "offline-desktop", "name", json!("Campus")),
			(&phone, "offline-phone", "favorite", json!(0)),
		] {
			old_file(path);
			let connection = open(path).unwrap();
			connection
				.execute("DELETE FROM timeparagon_sync_edits WHERE id='old-7'", [])
				.unwrap();
			connection
				.execute(
					"INSERT INTO timeparagon_sync_snapshots VALUES(?1,?2,?3,?4,?5,1,?6)",
					params![
						snapshot.id,
						snapshot.document,
						snapshot.after,
						snapshot.clock,
						serde_json::to_string(&snapshot.folds).unwrap(),
						now()
					],
				)
				.unwrap();
			let mut desired = original.clone();
			desired[field] = changed;
			let pending = Edit {
				before: Some(original.to_string()),
				..old_edit(id, "places", key, &desired, 3)
			};
			connection
				.execute(
					"INSERT INTO timeparagon_sync_edits VALUES(?1,?2,?3,?4,?5,0)",
					params![
						pending.id,
						pending.document,
						pending.before,
						pending.after,
						pending.clock
					],
				)
				.unwrap();
			connection
				.execute(
					"UPDATE places SET name=?1,favorite=?2",
					params![
						desired["name"].as_str().unwrap(),
						desired["favorite"].as_i64().unwrap()
					],
				)
				.unwrap();
			drop(connection);

			// Exercise the versioned open, not just direct enrollment.
			drop(Database::open(path).unwrap());
			let pending_after = edits(&open(path).unwrap(), true).unwrap();
			assert_eq!(pending_after, vec![translate(&pending).unwrap()]);
			assert!(
				history(path)
					.unwrap()
					.contains(&translate(&snapshot).unwrap())
			);
			send(path, &mut server);
		}
		let incoming: Vec<Edit> = server.into_values().collect();
		for path in [&desktop, &phone] {
			apply(path, &incoming, None, now()).unwrap();
			let config: Config = crate::settings::load(path, |_| {}).unwrap().value;
			assert_eq!(config.travel.places[0].name, "Campus");
			assert!(!config.travel.places[0].favorite);
			assert!(edits(&open(path).unwrap(), true).unwrap().is_empty());
			assert!(!apply(path, &incoming, None, now()).unwrap().changed);
		}
	}

	#[test]
	fn a_damaged_legacy_cell_rolls_back_the_migration_and_can_be_repaired() {
		use crate::task_db::{Database, SCHEMA_VERSION};
		let (_directory, path) = db();
		old_file(&path);
		let connection = open(&path).unwrap();
		connection
			.execute("UPDATE reminders SET sound='damaged JSON'", [])
			.unwrap();
		assert!(Database::open(&path).is_err());
		for retired in &RETIRED {
			assert!(
				exists(&connection, retired.table).unwrap(),
				"{}",
				retired.table
			);
		}
		let raw: String = connection
			.query_row("SELECT sound FROM reminders", [], |r| r.get(0))
			.unwrap();
		assert_eq!(raw, "damaged JSON");
		let document: String = connection
			.query_row(
				"SELECT document FROM timeparagon_sync_edits WHERE id='old-7'",
				[],
				|r| r.get(0),
			)
			.unwrap();
		assert!(document.starts_with("places:"));
		let version: i64 = connection
			.query_row("PRAGMA user_version", [], |r| r.get(0))
			.unwrap();
		assert_eq!(version, 9);
		let copied: i64 = connection
			.query_row("SELECT count(*) FROM settings", [], |r| r.get(0))
			.unwrap();
		assert_eq!(copied, 0);
		connection
			.execute(
				"UPDATE reminders SET sound=?1",
				[json!({"kind":"tone","tone":"bell"}).to_string()],
			)
			.unwrap();
		drop(Database::open(&path).unwrap());
		let version: i64 = connection
			.query_row("PRAGMA user_version", [], |r| r.get(0))
			.unwrap();
		assert_eq!(version, SCHEMA_VERSION);
		assert!(!exists(&connection, "reminders").unwrap());
	}

	#[test]
	fn the_retired_tables_move_into_settings_with_their_history() {
		use crate::config::Config;
		let (_d, path) = db();
		old_file(&path);
		enroll(&path).unwrap();

		let db = open(&path).unwrap();
		for retired in &RETIRED {
			assert!(!exists(&db, retired.table).unwrap(), "{}", retired.table);
		}
		let config: Config = crate::settings::load(&path, |_| {}).unwrap().value;
		let place = &config.travel.places[0];
		assert_eq!(
			(place.name.as_str(), place.place_id.as_str()),
			("Home", "mapbox-1")
		);
		assert!(place.favorite);
		assert_eq!(
			config.travel.event_places["standup_20261110T090000Z"],
			place.id
		);
		assert_eq!(config.travel.modes.len(), 1);
		assert_eq!(config.travel.routes[0].meters, 5000);
		assert_eq!(config.calendar.disabled, ["tasks:abc", "work"]);
		assert_eq!(config.calendar.source_order, ["b", "work"]);
		assert_eq!(config.calendar.task_colors.len(), 1);
		assert_eq!(config.calendar.calendar_colors.len(), 1);
		assert_eq!(
			config.calendar.sleep_schedule[0].bedtime.to_string(),
			"23:00:00"
		);
		assert_eq!(config.calendar.sleep_schedule_changes.len(), 1);
		assert_eq!(config.calendar.weekly_task_order["2026-11-10"], ["x", "y"]);
		assert_eq!(config.reminders.events.minutes_before, 15);
		assert!(!config.reminders.events.enabled);

		// The history kept its IDs, clocks and upload state under new names,
		// and the rows already are what it projects to.
		let journal = history(&path).unwrap();
		assert_eq!(journal.len(), old_rows().len());
		assert!(
			journal
				.iter()
				.all(|edit| edit.document.starts_with("settings:"))
		);
		assert!(journal.iter().all(|edit| edit.clock == 2));
		assert!(edits(&db, true).unwrap().is_empty());
		assert!(!apply(&path, &[], None, now()).unwrap().changed);

		// A device still on the old build keeps merging.
		let (_, key, row) = &old_rows()[7];
		let mut renamed = row.clone();
		renamed["name"] = "Campus".into();
		let from_old_build = Edit {
			before: Some(row.to_string()),
			..old_edit("from-old-build", "places", key, &renamed, 3)
		};
		assert!(
			apply(&path, &[from_old_build], None, now())
				.unwrap()
				.changed
		);
		let config: Config = crate::settings::load(&path, |_| {}).unwrap().value;
		assert_eq!(config.travel.places[0].name, "Campus");
		assert!(config.travel.places[0].favorite);
		let received = history(&path).unwrap();
		let edit = received.iter().find(|e| e.id == "from-old-build").unwrap();
		assert_eq!(edit.document, r#"settings:["travel.places/1730000000000"]"#);

		// Enrolling again changes nothing.
		enroll(&path).unwrap();
		assert_eq!(history(&path).unwrap(), received);
	}

	#[test]
	fn nights_of_sleep_leave_the_journal_here_and_on_the_server() {
		let (_d, path) = db();
		let db = open(&path).unwrap();
		// A schema 9 file journaled nights, and its trigger still does.
		db.execute_batch(
			"CREATE TRIGGER timeparagon_sync_sleep_nights_insert AFTER INSERT ON sleep_nights
			 BEGIN INSERT INTO timeparagon_sync_edits VALUES('night-trigger','sleep_nights:[\"app\",\"n2\"]',NULL,'{}',9,0); END;
			 INSERT INTO timeparagon_sync_edits VALUES('old-night','sleep_nights:[\"app\",\"n1\"]',NULL,'{}',1,1);",
		)
		.unwrap();
		enroll(&path).unwrap();
		db.execute(
			"INSERT INTO sleep_nights VALUES('app','n2','2030-11-04','a','b','{}')",
			[],
		)
		.unwrap();
		assert!(history(&path).unwrap().is_empty());

		// One still arriving from an older device is dropped, not refused.
		let incoming = Edit {
			id: "remote-night".into(),
			document: r#"sleep_nights:["app","n3"]"#.into(),
			after: Some("{}".into()),
			clock: 4,
			..Default::default()
		};
		let applied = apply(&path, &[incoming], None, now()).unwrap();
		assert!(!applied.changed);
		let mut deletions = applied.remote_deletions;
		deletions.sort();
		assert_eq!(deletions, ["old-night", "remote-night"]);
		assert!(history(&path).unwrap().is_empty());
		let nights: i64 = db
			.query_row("SELECT count(*) FROM sleep_nights", [], |row| row.get(0))
			.unwrap();
		assert_eq!(nights, 1);
	}

	#[test]
	fn retiring_an_incoming_night_cannot_delete_another_documents_history() {
		let (_directory, path) = db();
		crate::task_db::write_document_json(&path, "shared.projects", "{}").unwrap();
		let before = history(&path).unwrap();
		let collision = Edit {
			document: r#"sleep_nights:["app","n1"]"#.into(),
			..before[0].clone()
		};
		assert!(apply(&path, &[collision], None, now()).is_err());
		assert_eq!(history(&path).unwrap(), before);
	}
}
