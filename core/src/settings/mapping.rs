//! Settings as one JSON tree, shaped like the apps' `Config`, and as rows of
//! the settings tables.
//!
//! Both directions work on the tree rather than on a Rust type, so the
//! desktop's config and the phone's smaller one share them: each app only
//! sees, and only ever changes, the parts of the tree its type has.
//!
//! A field of the tree is kept in one of three places:
//!
//! * a table of its own, for collections whose items have an identity
//!   (places, reminders, the sleep schedule, …), so that devices merge them
//!   item by item;
//! * a [`SETTINGS`] or [`DEVICE_SETTINGS`] row named `section.field`;
//! * a shared document ([`DOCUMENT_SECTIONS`], and the notepad's text), as
//!   before this module existed. See [`crate::shared_data`].

use std::collections::BTreeMap;

use rusqlite::Connection;
use serde_json::{Map, Value, json};

use super::schema::{
	self, CALENDAR_SOURCES, DEVICE_SETTINGS, EVENT_PLACES, PLACES, REMINDERS, SETTINGS,
	SLEEP_SCHEDULE, SLEEP_SCHEDULE_CHANGES, TRAVEL_MODES, TRAVEL_ROUTES, Table, WEEKLY_TASK_ORDER,
};

/// Rows of the settings tables, by table and primary key (as JSON text).
pub(crate) type Rows = BTreeMap<(&'static str, String), Map<String, Value>>;

/// Whole sections kept as shared documents rather than in these tables.
pub(crate) const DOCUMENT_SECTIONS: [&str; 3] = ["brain_dump", "projects", "task_work"];

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

fn insert(rows: &mut Rows, table: &'static Table, row: Map<String, Value>) {
	let key = Value::Array(table.key_of(&row)).to_string();
	rows.insert((table.name, key), row);
}

fn row(fields: Value) -> Map<String, Value> {
	match fields {
		Value::Object(fields) => fields,
		_ => Map::new(),
	}
}

/// A field that is present and not null.
fn field<'a>(value: &'a Value, name: &str) -> Option<&'a Value> {
	value.get(name).filter(|value| !value.is_null())
}

/// The rows that hold `tree`. Fields of the tree in no table and in no
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
			if !table_field(&mut rows, section, name, value) {
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
	insert(rows, table, row(json!({"key": key, "value": value})));
}

/// Adds the rows of a field that has a table of its own, and says whether it
/// had one.
fn table_field(rows: &mut Rows, section: &str, name: &str, value: &Value) -> bool {
	let empty = Vec::new();
	let items = value.as_array().unwrap_or(&empty);
	match (section, name) {
		// A shared document; see `shared_data`.
		("notepad", "text") => {}
		// See `calendar_sources`.
		("calendar", "disabled" | "source_order" | "task_colors" | "calendar_colors") => {}
		("calendar", "sleep_schedule") => {
			for (weekday, times) in items.iter().enumerate() {
				if let (Some(bedtime), Some(wakeup)) =
					(field(times, "bedtime"), field(times, "wakeup"))
				{
					let fields = json!({"weekday": weekday, "bedtime": bedtime, "wakeup": wakeup});
					insert(rows, &SLEEP_SCHEDULE, row(fields));
				}
			}
		}
		("calendar", "sleep_schedule_changes") => {
			for change in items {
				let times = |part: &str, which: &str| {
					change
						.get(part)
						.and_then(|times| field(times, which))
						.cloned()
				};
				if let (Some(from), Some(pb), Some(pw), Some(b), Some(w)) = (
					field(change, "from"),
					times("previous", "bedtime"),
					times("previous", "wakeup"),
					times("times", "bedtime"),
					times("times", "wakeup"),
				) {
					let fields = json!({"from_date": from, "previous_bedtime": pb,
                        "previous_wakeup": pw, "bedtime": b, "wakeup": w});
					insert(rows, &SLEEP_SCHEDULE_CHANGES, row(fields));
				}
			}
		}
		("calendar", "weekly_task_order") => {
			for (day, cards) in value.as_object().into_iter().flatten() {
				insert(
					rows,
					&WEEKLY_TASK_ORDER,
					row(json!({"day": day, "cards": cards})),
				);
			}
		}
		("travel", "places") => {
			for place in items {
				if let (Some(id), Some(name), Some(lat), Some(lng)) = (
					field(place, "id"),
					field(place, "name"),
					field(place, "lat"),
					field(place, "lng"),
				) {
					let text =
						|name: &str| field(place, name).cloned().unwrap_or_else(|| "".into());
					let fields = json!({"id": id, "name": name, "address": text("address"),
                        "provider_id": text("place_id"), "lat": lat, "lng": lng,
                        "favorite": field(place, "favorite").and_then(Value::as_bool).unwrap_or(false)});
					insert(rows, &PLACES, row(fields));
				}
			}
		}
		("travel", "event_places") => {
			for (event, place) in value.as_object().into_iter().flatten() {
				let fields = json!({"event_id": event, "place_id": place});
				insert(rows, &EVENT_PLACES, row(fields));
			}
		}
		("travel", "modes") => {
			for trip in items {
				if let (Some(from), Some(to), Some(mode)) =
					(field(trip, "from"), field(trip, "to"), field(trip, "mode"))
				{
					let fields = json!({"from_place": from, "to_place": to, "mode": mode});
					insert(rows, &TRAVEL_MODES, row(fields));
				}
			}
		}
		("travel", "routes") => {
			for trip in items {
				if let (Some(from), Some(to), Some(mode), Some(seconds), Some(meters)) = (
					field(trip, "from"),
					field(trip, "to"),
					field(trip, "mode"),
					field(trip, "seconds"),
					field(trip, "meters"),
				) {
					let fields = json!({"from_place": from, "to_place": to, "mode": mode,
                        "seconds": seconds, "meters": meters});
					insert(rows, &TRAVEL_ROUTES, row(fields));
				}
			}
		}
		("reminders", category) if crate::reminders::Category::from_id(category).is_some() => {
			let default = crate::reminders::CategorySettings::default();
			let fields = json!({
				"category": category,
				"enabled": field(value, "enabled").cloned().unwrap_or(default.enabled.into()),
				"minutes_before": field(value, "minutes_before").cloned().unwrap_or(default.minutes_before.into()),
				"sound": field(value, "sound").cloned().unwrap_or_else(|| json!(default.sound)),
			});
			insert(rows, &REMINDERS, row(fields));
		}
		_ => return false,
	}
	true
}

/// One row per calendar, task list or TimeParagon source that the user hid,
/// ordered or colored.
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
		let fields = json!({"id": id, "hidden": source.hidden, "position": source.position,
            "color": source.color});
		insert(rows, &CALENDAR_SOURCES, row(fields));
	}
}

/// `rows` without those holding the same as `defaults`: a value left at its
/// default is not stored, so it cannot override another device's choice.
pub(crate) fn without_defaults(mut rows: Rows, defaults: &Rows) -> Rows {
	rows.retain(|key, row| defaults.get(key) != Some(row));
	rows
}

/// Every row of every settings table.
pub(crate) fn read(db: &Connection) -> rusqlite::Result<Rows> {
	let mut rows = Rows::new();
	for table in schema::TABLES {
		for row in table.select_all(db)? {
			insert(&mut rows, table, row);
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

fn number(row: &Map<String, Value>, name: &str) -> i64 {
	row.get(name).and_then(Value::as_i64).unwrap_or_default()
}

/// The tree `rows` hold: the reverse of [`rows`], apart from the order of
/// lists that storage keeps as sets.
pub(crate) fn tree(rows: &Rows) -> Value {
	let mut tree = Map::new();
	let of = |table: &'static Table| {
		rows.iter()
			.filter(move |((name, _), _)| *name == table.name)
			.map(|(_, row)| row)
	};

	// Shared settings first, so that a device setting of the same name wins.
	for table in [&SETTINGS, &DEVICE_SETTINGS] {
		for row in of(table) {
			let (Some(Value::String(key)), Some(value)) = (row.get("key"), row.get("value")) else {
				continue;
			};
			if value.is_null() {
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

	let sources: Vec<_> = of(&CALENDAR_SOURCES).collect();
	if !sources.is_empty() {
		let mut disabled = Vec::new();
		let mut order = Vec::new();
		let mut task_colors = Map::new();
		let mut calendar_colors = Map::new();
		for source in sources {
			let Some(id) = source.get("id").and_then(Value::as_str) else {
				continue;
			};
			if source.get("hidden").and_then(Value::as_bool) == Some(true) {
				disabled.push(Value::from(id));
			}
			if let Some(position) = source.get("position").and_then(Value::as_i64) {
				order.push((position, id));
			}
			if let Some(color) = source.get("color").filter(|color| !color.is_null()) {
				// Task lists are named `tasks:<list>`; everything else that
				// can be colored is a Google calendar.
				let colors = if id.starts_with(crate::config::CalendarSettings::GOOGLE_TASKS_ID) {
					&mut task_colors
				} else {
					&mut calendar_colors
				};
				colors.insert(id.to_owned(), color.clone());
			}
		}
		order.sort();
		let calendar = section(&mut tree, "calendar");
		calendar.insert("disabled".into(), disabled.into());
		let order: Vec<Value> = order.into_iter().map(|(_, id)| id.into()).collect();
		calendar.insert("source_order".into(), order.into());
		calendar.insert("task_colors".into(), task_colors.into());
		calendar.insert("calendar_colors".into(), calendar_colors.into());
	}

	let schedule: Vec<_> = of(&SLEEP_SCHEDULE).collect();
	if !schedule.is_empty() {
		let default = json!(crate::calendar::sleep::Times::default());
		let days: Vec<Value> = (0..7)
			.map(|weekday| {
				schedule
					.iter()
					.find(|row| number(row, "weekday") == weekday)
					.map_or_else(
						|| default.clone(),
						|row| json!({"bedtime": row["bedtime"], "wakeup": row["wakeup"]}),
					)
			})
			.collect();
		section(&mut tree, "calendar").insert("sleep_schedule".into(), days.into());
	}

	let changes: Vec<Value> = of(&SLEEP_SCHEDULE_CHANGES)
		.map(|row| {
			json!({
				"from": row["from_date"],
				"previous": {"bedtime": row["previous_bedtime"], "wakeup": row["previous_wakeup"]},
				"times": {"bedtime": row["bedtime"], "wakeup": row["wakeup"]},
			})
		})
		.collect();
	if !changes.is_empty() {
		section(&mut tree, "calendar").insert("sleep_schedule_changes".into(), changes.into());
	}

	let order: Map<String, Value> = of(&WEEKLY_TASK_ORDER)
		.filter_map(|row| {
			Some((
				row.get("day")?.as_str()?.to_owned(),
				row.get("cards")?.clone(),
			))
		})
		.collect();
	if !order.is_empty() {
		section(&mut tree, "calendar").insert("weekly_task_order".into(), order.into());
	}

	let mut places: Vec<_> = of(&PLACES).collect();
	places.sort_by_key(|row| number(row, "id"));
	let places: Vec<Value> = places
		.into_iter()
		.map(|row| {
			let mut place = json!({"id": row["id"], "name": row["name"], "address": row["address"],
                "place_id": row["provider_id"], "lat": row["lat"], "lng": row["lng"]});
			if row.get("favorite").and_then(Value::as_bool) == Some(true) {
				place["favorite"] = true.into();
			}
			place
		})
		.collect();
	if !places.is_empty() {
		section(&mut tree, "travel").insert("places".into(), places.into());
	}

	let event_places: Map<String, Value> = of(&EVENT_PLACES)
		.filter_map(|row| {
			Some((
				row.get("event_id")?.as_str()?.to_owned(),
				row.get("place_id")?.clone(),
			))
		})
		.collect();
	if !event_places.is_empty() {
		section(&mut tree, "travel").insert("event_places".into(), event_places.into());
	}

	let trip = |row: &Map<String, Value>| (number(row, "from_place"), number(row, "to_place"));
	let mut modes: Vec<_> = of(&TRAVEL_MODES).collect();
	modes.sort_by_key(|row| trip(row));
	let modes: Vec<Value> = modes
		.into_iter()
		.map(|row| json!({"from": row["from_place"], "to": row["to_place"], "mode": row["mode"]}))
		.collect();
	if !modes.is_empty() {
		section(&mut tree, "travel").insert("modes".into(), modes.into());
	}

	let mut routes: Vec<_> = of(&TRAVEL_ROUTES).collect();
	routes.sort_by(|a, b| {
		(trip(a), a.get("mode").and_then(Value::as_str))
			.cmp(&(trip(b), b.get("mode").and_then(Value::as_str)))
	});
	let routes: Vec<Value> = routes
		.into_iter()
		.map(|row| {
			json!({"from": row["from_place"], "to": row["to_place"], "mode": row["mode"],
                "seconds": row["seconds"], "meters": row["meters"]})
		})
		.collect();
	if !routes.is_empty() {
		section(&mut tree, "travel").insert("routes".into(), routes.into());
	}

	for row in of(&REMINDERS) {
		let Some(category) = row.get("category").and_then(Value::as_str) else {
			continue;
		};
		let settings = json!({"enabled": row["enabled"], "minutes_before": row["minutes_before"],
            "sound": row["sound"]});
		section(&mut tree, "reminders").insert(category.to_owned(), settings);
	}

	Value::Object(tree)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn sample() -> Value {
		json!({
			"appearance": {"theme": "Ocean", "time_format": "compact"},
			"alarm": {"sound": {"volume": 25, "choice": {"file": "/tmp/a.ogg"}}},
			"window": {"width": 320.0},
			"notepad": {"text": "kept elsewhere", "font_size": 120},
			"projects": {"items": []},
			"calendar": {
				"sync_minutes": 30,
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

	#[test]
	fn every_field_lands_in_the_table_that_fits_it() {
		let rows = rows(&sample());
		let tables = |name: &str| rows.keys().filter(|(table, _)| *table == name).count();
		assert_eq!(tables("places"), 1);
		assert_eq!(tables("event_places"), 1);
		assert_eq!(tables("travel_modes"), 1);
		assert_eq!(tables("travel_routes"), 1);
		assert_eq!(tables("reminders"), 1);
		assert_eq!(tables("sleep_schedule"), 1);
		assert_eq!(tables("sleep_schedule_changes"), 1);
		assert_eq!(tables("weekly_task_order"), 1);
		// work, tasks:abc, b and the colored calendar.
		assert_eq!(tables("calendar_sources"), 4);
		let key = |table: &'static str, key: &str| (table, Value::from(vec![key]).to_string());
		assert_eq!(rows[&key("settings", "calendar.sync_minutes")]["value"], 30);
		assert_eq!(rows[&key("settings", "appearance.theme")]["value"], "Ocean");
		assert_eq!(rows[&key("settings", "reminders.enabled")]["value"], true);
		assert_eq!(rows[&key("settings", "alarm.sound")]["value"]["volume"], 25);
		assert_eq!(
			rows[&key("device_settings", "window.width")]["value"],
			320.0
		);
		assert_eq!(
			rows[&key("device_settings", "notepad.font_size")]["value"],
			120
		);
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
			rows[&key("calendar_sources", "b")]["position"],
			0,
			"the first place counts"
		);
		assert_eq!(rows[&key("calendar_sources", "work")]["hidden"], true);
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
		assert_eq!(calendar["sync_minutes"], 30);
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
	fn values_at_their_default_are_not_stored() {
		let defaults = rows(&json!({"calendar": {"sync_minutes": 60, "show_tasks": true}}));
		let stored = without_defaults(
			rows(&json!({"calendar": {"sync_minutes": 60, "show_tasks": false}})),
			&defaults,
		);
		let keys: Vec<&String> = stored.keys().map(|(_, key)| key).collect();
		assert_eq!(keys, [&json!(["calendar.show_tasks"]).to_string()]);
	}
}
