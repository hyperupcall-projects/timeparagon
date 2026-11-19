//! Settings as one JSON tree, shaped like the apps' `Config`, and as rows of
//! the settings tables.
//!
//! Both directions work on the tree rather than on a Rust type, so the
//! desktop's config and the phone's smaller one share them: each app only
//! sees, and only ever changes, the parts of the tree its type has.
//!
//! A field of the tree is kept in one of three places:
//!
//! * a [`SETTINGS`] or [`DEVICE_SETTINGS`] row named `section.field`;
//! * for a collection whose items have an identity (places, the sleep
//!   schedule, …), one [`SETTINGS`] row per item, named `section.field/item`
//!   (see [`COLLECTIONS`]), so that devices merge them item by item;
//! * a shared document ([`DOCUMENT_SECTIONS`], and the notepad's text), as
//!   before this module existed. See [`crate::shared_data`].

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde_json::{Map, Value, json};

use super::schema::{DEVICE_SETTINGS, SETTINGS, TABLES, Table};

/// Rows of the settings tables: their values, by table and key.
pub(crate) type Rows = BTreeMap<(&'static str, String), Value>;

/// Whole sections kept as shared documents rather than in these tables.
pub(crate) const DOCUMENT_SECTIONS: [&str; 4] = ["brain_dump", "projects", "task_work", "routines"];

/// Sections that describe this device rather than the person using it: its
/// files and folders, the sync server it talks to, its screen and windows,
/// its speech-to-text setup and the timer running on it. Another device
/// would be broken by them, so they stay in [`DEVICE_SETTINGS`].
pub(crate) const DEVICE_SECTIONS: [&str; 7] = [
	"database_sync",
	"notes",
	"panel",
	"pomodoro",
	"views",
	"voice",
	"window",
];

/// Whether `section.field` stays on this device. The notepad's font size is
/// sized for this screen, while its text is shared.
pub(crate) fn device_local(section: &str, field: &str) -> bool {
	DEVICE_SECTIONS.contains(&section) || (section, field) == ("notepad", "font_size")
}

/// How the items of a collection are named.
enum Items {
	/// A list whose items are named by these fields.
	Keyed(&'static [&'static str]),
	/// An object, whose entries are named by their keys.
	Entries,
	/// The sleep schedule: seven items named by position, Monday first. A
	/// day without a row has the default times.
	Weekdays,
}

struct Collection {
	path: &'static str,
	items: Items,
}

/// Fields stored one row per item.
const COLLECTIONS: [Collection; 7] = [
	Collection {
		path: "calendar.sleep_schedule",
		items: Items::Weekdays,
	},
	Collection {
		path: "calendar.sleep_schedule_changes",
		items: Items::Keyed(&["from"]),
	},
	Collection {
		path: "calendar.weekly_task_order",
		items: Items::Entries,
	},
	Collection {
		path: "travel.places",
		items: Items::Keyed(&["id"]),
	},
	Collection {
		path: "travel.event_places",
		items: Items::Entries,
	},
	Collection {
		path: "travel.modes",
		items: Items::Keyed(&["from", "to"]),
	},
	Collection {
		path: "travel.routes",
		items: Items::Keyed(&["from", "to", "mode"]),
	},
];

/// One row per calendar, task list or TimeParagon source that the user hid,
/// ordered or colored, named `calendar.sources/<id>`: see
/// [`calendar_sources`].
pub(crate) const SOURCES: &str = "calendar.sources";

/// The fields of the tree that [`SOURCES`] holds.
const SOURCE_FIELDS: [&str; 4] = [
	"calendar.disabled",
	"calendar.source_order",
	"calendar.task_colors",
	"calendar.calendar_colors",
];

/// An item's name from the values that identify it, joined by `/`.
pub(crate) fn item_name(key: &[Value]) -> String {
	key.iter()
		.map(|part| {
			part.as_str()
				.map_or_else(|| part.to_string(), str::to_owned)
		})
		.collect::<Vec<_>>()
		.join("/")
}

/// A field that is present and not null.
fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
	value.get(name).filter(|value| !value.is_null())
}

impl Collection {
	fn rows(&self, value: &Value, rows: &mut Rows) {
		let mut add = |name: String, item: &Value| {
			if !item.is_null() {
				rows.insert(
					(SETTINGS.name, format!("{}/{name}", self.path)),
					item.clone(),
				);
			}
		};
		match self.items {
			Items::Keyed(fields) => {
				for item in value.as_array().into_iter().flatten() {
					let key: Option<Vec<Value>> = fields
						.iter()
						.map(|name| field(item, name).cloned())
						.collect();
					if let Some(key) = key {
						add(item_name(&key), item);
					}
				}
			}
			Items::Entries => {
				for (name, item) in value.as_object().into_iter().flatten() {
					add(name.clone(), item);
				}
			}
			Items::Weekdays => {
				for (day, item) in value.as_array().into_iter().flatten().enumerate().take(7) {
					add(day.to_string(), item);
				}
			}
		}
	}

	/// The field, from its items by name.
	fn tree(&self, items: &[(&str, &Value)]) -> Value {
		match self.items {
			Items::Keyed(fields) => {
				// Numbers in numeric order, and text in text order.
				let order = |item: &Value| -> Vec<(Option<u64>, Option<String>)> {
					fields
						.iter()
						.map(|name| {
							let part = item.get(*name);
							(
								part.and_then(Value::as_u64),
								part.and_then(Value::as_str).map(str::to_owned),
							)
						})
						.collect()
				};
				let mut list: Vec<Value> = items.iter().map(|(_, item)| (*item).clone()).collect();
				list.sort_by_cached_key(order);
				Value::Array(list)
			}
			Items::Entries => Value::Object(
				items
					.iter()
					.map(|(name, item)| ((*name).to_owned(), (*item).clone()))
					.collect(),
			),
			Items::Weekdays => {
				let default = json!(crate::calendar::sleep::Times::default());
				let days = (0..7).map(|day| {
					items
						.iter()
						.find(|(name, _)| name.parse() == Ok(day))
						.map_or_else(|| default.clone(), |(_, item)| (*item).clone())
				});
				Value::Array(days.collect())
			}
		}
	}
}

/// The rows that hold `tree`. Fields of the tree in no collection and in no
/// document become settings, so nothing in it is lost.
pub(crate) fn rows(tree: &Value) -> Rows {
	let mut rows = Rows::new();
	let Some(sections) = tree.as_object() else {
		return rows;
	};
	for (section, value) in sections {
		if DOCUMENT_SECTIONS.contains(&section.as_str()) {
			continue;
		}
		let Some(fields) = value.as_object() else {
			setting(&mut rows, section, None, value);
			continue;
		};
		for (name, value) in fields {
			let path = format!("{section}.{name}");
			if let Some(collection) = COLLECTIONS.iter().find(|c| c.path == path) {
				collection.rows(value, &mut rows);
			// The notepad's text is a shared document; see `shared_data`.
			} else if path != "notepad.text" && !SOURCE_FIELDS.contains(&path.as_str()) {
				setting(&mut rows, section, Some(name), value);
			}
		}
		if section == "calendar" {
			calendar_sources(&mut rows, fields);
		}
	}
	rows
}

fn setting(rows: &mut Rows, section: &str, name: Option<&str>, value: &Value) {
	// Neither TOML nor an unset optional field has a value here, and serde
	// writes a NaN as null: in every case the default is what is meant.
	if value.is_null() {
		return;
	}
	let key = match name {
		Some(name) => format!("{section}.{name}"),
		None => section.to_owned(),
	};
	let table = if device_local(section, name.unwrap_or_default()) {
		&DEVICE_SETTINGS
	} else {
		&SETTINGS
	};
	rows.insert((table.name, key), value.clone());
}

/// One row per source that the user hid, ordered or colored, holding all
/// three, so a source's choices merge as one item.
fn calendar_sources(rows: &mut Rows, calendar: &Map<String, Value>) {
	#[derive(Default)]
	struct Source {
		hidden: bool,
		position: Option<usize>,
		color: Option<Value>,
	}
	let mut sources = BTreeMap::<String, Source>::new();
	let strings = |name: &str| -> Vec<String> {
		calendar
			.get(name)
			.and_then(Value::as_array)
			.into_iter()
			.flatten()
			.filter_map(|id| id.as_str().map(str::to_owned))
			.collect()
	};
	for id in strings("disabled") {
		sources.entry(id).or_default().hidden = true;
	}
	for (position, id) in strings("source_order").into_iter().enumerate() {
		sources
			.entry(id)
			.or_default()
			.position
			.get_or_insert(position);
	}
	for name in ["calendar_colors", "task_colors"] {
		for (id, color) in calendar
			.get(name)
			.and_then(Value::as_object)
			.into_iter()
			.flatten()
		{
			if !color.is_null() {
				sources.entry(id.clone()).or_default().color = Some(color.clone());
			}
		}
	}
	for (id, source) in sources {
		let value = json!({"hidden": source.hidden, "position": source.position,
            "color": source.color});
		rows.insert((SETTINGS.name, format!("{SOURCES}/{id}")), value);
	}
}

/// The fields [`calendar_sources`] made the rows from.
fn calendar_source_fields(calendar: &mut Map<String, Value>, sources: &[(&str, &Value)]) {
	let mut disabled = Vec::new();
	let mut order = Vec::new();
	let mut task_colors = Map::new();
	let mut calendar_colors = Map::new();
	for (id, source) in sources {
		if source.get("hidden").and_then(Value::as_bool) == Some(true) {
			disabled.push(Value::from(*id));
		}
		if let Some(position) = source.get("position").and_then(Value::as_i64) {
			order.push((position, *id));
		}
		if let Some(color) = field(source, "color") {
			// Task lists are named `tasks:<list>`; everything else that can
			// be colored is a Google calendar.
			let colors = if id.starts_with(crate::config::CalendarSettings::GOOGLE_TASKS_ID) {
				&mut task_colors
			} else {
				&mut calendar_colors
			};
			colors.insert((*id).to_owned(), color.clone());
		}
	}
	order.sort();
	calendar.insert("disabled".into(), disabled.into());
	let order: Vec<Value> = order.into_iter().map(|(_, id)| id.into()).collect();
	calendar.insert("source_order".into(), order.into());
	calendar.insert("task_colors".into(), task_colors.into());
	calendar.insert("calendar_colors".into(), calendar_colors.into());
}

/// `rows` without those holding the same as `defaults`: a value left at its
/// default is not stored, so it cannot override another device's choice.
pub(crate) fn without_defaults(mut rows: Rows, defaults: &Rows) -> Rows {
	rows.retain(|key, value| defaults.get(key) != Some(value));
	rows
}

/// Every row of every settings table.
pub(crate) fn read(db: &Connection) -> rusqlite::Result<Rows> {
	let mut rows = Rows::new();
	for table in TABLES {
		for mut row in table.select_all(db)? {
			if let Some(Value::String(key)) = row.remove("key") {
				let value = row.remove("value").unwrap_or_default();
				rows.insert((table.name, key), value);
			}
		}
	}
	Ok(rows)
}

fn section<'a>(tree: &'a mut Map<String, Value>, name: &str) -> &'a mut Map<String, Value> {
	let entry = tree
		.entry(name)
		.or_insert_with(|| Value::Object(Map::new()));
	if !entry.is_object() {
		*entry = Value::Object(Map::new());
	}
	entry.as_object_mut().expect("just made an object")
}

/// The tree `rows` hold: the reverse of [`rows`], apart from the order of
/// lists that storage keeps as sets.
pub(crate) fn tree(rows: &Rows) -> Value {
	let mut tree = Map::new();
	let mut items = BTreeMap::<&str, Vec<(&str, &Value)>>::new();
	// Shared settings first, so that a device setting of the same name wins.
	for table in [&SETTINGS, &DEVICE_SETTINGS] {
		let of_table = rows.iter().filter(|((name, _), _)| *name == table.name);
		for ((_, key), value) in of_table {
			if value.is_null() {
				continue;
			}
			if let Some((path, item)) = key.split_once('/')
				&& (path == SOURCES || COLLECTIONS.iter().any(|c| c.path == path))
			{
				items.entry(path).or_default().push((item, value));
				continue;
			}
			match key.split_once('.') {
				Some((name, field)) => {
					section(&mut tree, name).insert(field.to_owned(), value.clone());
				}
				None => {
					tree.insert(key.clone(), value.clone());
				}
			}
		}
	}
	for collection in &COLLECTIONS {
		if let Some(items) = items.get(collection.path) {
			let (name, field) = collection.path.split_once('.').expect("section.field");
			section(&mut tree, name).insert(field.to_owned(), collection.tree(items));
		}
	}
	if let Some(sources) = items.get(SOURCES) {
		calendar_source_fields(section(&mut tree, "calendar"), sources);
	}
	Value::Object(tree)
}

/// The table a row of [`Rows`] belongs to.
pub(crate) fn table(name: &str) -> &'static Table {
	super::schema::table(name).expect("rows are only made for settings tables")
}

#[cfg(test)]
pub(crate) mod tests {
	use super::*;

	pub(crate) fn sample() -> Value {
		json!({
			"appearance": {"theme": "Ocean", "time_format": "compact"},
			"alarm": {"sound": {"volume": 25, "choice": {"file": "/tmp/a.ogg"}}},
			"window": {"width": 320.0},
			"notepad": {"text": "kept elsewhere", "font_size": 120},
			"projects": {"items": []},
			"calendar": {
				"weekly_day_width": 200,
				"app_calendar": null,
				"disabled": ["work", "tasks:abc"],
				"source_order": ["b", "work", "b"],
				"task_colors": {"tasks:abc": "#112233"},
				"calendar_colors": {"cal@group.calendar.google.com": "#445566"},
				"sleep_schedule": [{"bedtime": "23:00:00", "wakeup": "07:00:00"}],
				"sleep_schedule_changes": [{"from": "2026-11-10",
					"previous": {"bedtime": "00:00:00", "wakeup": "08:00:00"},
					"times": {"bedtime": "23:00:00", "wakeup": "07:00:00"}}],
				"weekly_task_order": {"2026-11-10": ["x", "y"]},
			},
			"travel": {
				"api_key": "pk",
				"places": [{"id": 1730000000000_u64, "name": "Home", "address": "1 Main St",
					"place_id": "mapbox-1", "lat": 1.5, "lng": 2.5, "favorite": true}],
				"event_places": {"standup_20261110T090000Z": 1730000000000_u64},
				"modes": [{"from": 1, "to": 2, "mode": "car"}],
				"routes": [{"from": 1, "to": 2, "mode": "car", "seconds": 600, "meters": 5000}],
			},
			"reminders": {"enabled": true, "events": {"enabled": false, "minutes_before": 15,
				"sound": {"kind": "tone", "tone": "bell"}}},
		})
	}

	fn key(table: &'static str, key: &str) -> (&'static str, String) {
		(table, key.to_owned())
	}

	#[test]
	fn every_field_lands_in_the_row_that_fits_it() {
		let rows = rows(&sample());
		let items = |path: &str| {
			rows.keys()
				.filter(|(_, key)| key.starts_with(&format!("{path}/")))
				.count()
		};
		assert_eq!(items("travel.places"), 1);
		assert_eq!(items("travel.event_places"), 1);
		assert_eq!(items("travel.modes"), 1);
		assert_eq!(items("travel.routes"), 1);
		assert_eq!(items("calendar.sleep_schedule"), 1);
		assert_eq!(items("calendar.sleep_schedule_changes"), 1);
		assert_eq!(items("calendar.weekly_task_order"), 1);
		// work, tasks:abc, b and the colored calendar.
		assert_eq!(items("calendar.sources"), 4);
		assert_eq!(rows[&key("settings", "calendar.weekly_day_width")], 200);
		assert_eq!(rows[&key("settings", "appearance.theme")], "Ocean");
		assert_eq!(rows[&key("settings", "reminders.enabled")], true);
		assert_eq!(
			rows[&key("settings", "reminders.events")]["minutes_before"],
			15
		);
		assert_eq!(rows[&key("settings", "alarm.sound")]["volume"], 25);
		assert_eq!(rows[&key("settings", "travel.modes/1/2")]["mode"], "car");
		assert_eq!(
			rows[&key("settings", "travel.routes/1/2/car")]["meters"],
			5000
		);
		assert_eq!(
			rows[&key("settings", "travel.places/1730000000000")]["name"],
			"Home"
		);
		assert_eq!(
			rows[&key("settings", "calendar.sleep_schedule_changes/2026-11-10")]["times"]["bedtime"],
			"23:00:00"
		);
		assert_eq!(rows[&key("device_settings", "window.width")], 320.0);
		assert_eq!(rows[&key("device_settings", "notepad.font_size")], 120);
		assert!(
			!rows.contains_key(&key("settings", "calendar.app_calendar")),
			"null is the default"
		);
		assert!(
			!rows
				.keys()
				.any(|(_, key)| key.contains("notepad.text") || key.contains("projects"))
		);
		assert_eq!(
			rows[&key("settings", "calendar.sources/b")]["position"],
			0,
			"the first place counts"
		);
		assert_eq!(
			rows[&key("settings", "calendar.sources/work")]["hidden"],
			true
		);
	}

	#[test]
	fn the_tree_comes_back_from_its_rows() {
		let original = sample();
		let back = tree(&rows(&original));
		assert_eq!(back["appearance"], original["appearance"]);
		assert_eq!(back["alarm"], original["alarm"]);
		assert_eq!(back["window"], original["window"]);
		assert_eq!(back["notepad"], json!({"font_size": 120}));
		assert_eq!(back["travel"], original["travel"]);
		assert_eq!(back["reminders"], original["reminders"]);
		let calendar = &back["calendar"];
		assert_eq!(calendar["weekly_day_width"], 200);
		assert_eq!(calendar["disabled"], json!(["tasks:abc", "work"]));
		assert_eq!(calendar["source_order"], json!(["b", "work"]));
		assert_eq!(calendar["task_colors"], original["calendar"]["task_colors"]);
		assert_eq!(
			calendar["calendar_colors"],
			original["calendar"]["calendar_colors"]
		);
		assert_eq!(calendar["sleep_schedule"].as_array().unwrap().len(), 7);
		assert_eq!(
			calendar["sleep_schedule"][0],
			original["calendar"]["sleep_schedule"][0]
		);
		assert_eq!(
			calendar["sleep_schedule"][1],
			json!({"bedtime": "00:00:00", "wakeup": "08:00:00"})
		);
		assert_eq!(
			calendar["sleep_schedule_changes"],
			original["calendar"]["sleep_schedule_changes"]
		);
		assert_eq!(
			calendar["weekly_task_order"],
			original["calendar"]["weekly_task_order"]
		);
		// And again, unchanged.
		assert_eq!(rows(&back), rows(&tree(&rows(&back))));
	}

	#[test]
	fn items_come_back_in_the_order_of_what_names_them() {
		let places = json!({"travel": {"places": [
			{"id": 100, "name": "C", "lat": 0, "lng": 0},
			{"id": 9, "name": "A", "lat": 0, "lng": 0},
			{"id": 20, "name": "B", "lat": 0, "lng": 0},
			{"name": "No ID", "lat": 0, "lng": 0},
		]}});
		let back = tree(&rows(&places));
		let names: Vec<&str> = back["travel"]["places"]
			.as_array()
			.unwrap()
			.iter()
			.map(|place| place["name"].as_str().unwrap())
			.collect();
		assert_eq!(
			names,
			["A", "B", "C"],
			"and an item without its ID is not kept"
		);
	}

	#[test]
	fn values_at_their_default_are_not_stored() {
		let defaults = rows(&json!({"calendar": {"weekly_day_width": 150, "show_tasks": true}}));
		let stored = without_defaults(
			rows(&json!({"calendar": {"weekly_day_width": 150, "show_tasks": false}})),
			&defaults,
		);
		let keys: Vec<&String> = stored.keys().map(|(_, key)| key).collect();
		assert_eq!(keys, ["calendar.show_tasks"]);
	}
}
