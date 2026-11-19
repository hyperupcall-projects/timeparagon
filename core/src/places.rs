//! Saved places: where tasks and calendar events happen, and the Mapbox search
//! that finds new ones.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::calendar::Event;

const SEARCH_URL: &str = "https://api.mapbox.com/search/searchbox/v1/forward";
const DIRECTIONS_URL: &str = "https://api.mapbox.com/directions/v5/mapbox";

/// The most places a search offers.
const MAX_RESULTS: u32 = 8;

/// What a trip's start is called when it is Home.
pub const START_OF_DAY: &str = "Start of day";

/// Display units only; route requests and cached distances always use meters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DistanceUnit {
	#[default]
	Kilometers,
	Miles,
}

impl DistanceUnit {
	pub const ALL: [Self; 2] = [Self::Kilometers, Self::Miles];
}

impl std::fmt::Display for DistanceUnit {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(match self {
			Self::Kilometers => "Kilometers",
			Self::Miles => "Miles",
		})
	}
}

/// How a trip between two places is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TravelMode {
	#[default]
	Walk,
	Bike,
	Car,
}

impl TravelMode {
	pub const ALL: [TravelMode; 3] = [Self::Walk, Self::Bike, Self::Car];

	pub fn label(self) -> &'static str {
		match self {
			Self::Walk => "Walk",
			Self::Bike => "Bike",
			Self::Car => "Car",
		}
	}

	/// The profile the Mapbox Directions API expects.
	pub fn profile(self) -> &'static str {
		match self {
			Self::Walk => "walking",
			Self::Bike => "cycling",
			Self::Car => "driving",
		}
	}

	/// The `travelmode` of a Google Maps directions link.
	pub fn google_travelmode(self) -> &'static str {
		match self {
			Self::Walk => "walking",
			Self::Bike => "bicycling",
			Self::Car => "driving",
		}
	}

	/// The routing engine of an OpenStreetMap directions link.
	pub fn osm_engine(self) -> &'static str {
		match self {
			Self::Walk => "fossgis_osrm_foot",
			Self::Bike => "fossgis_osrm_bike",
			Self::Car => "fossgis_osrm_car",
		}
	}

	/// The mode after this one, wrapping around.
	pub fn next(self) -> Self {
		let index = Self::ALL.iter().position(|mode| *mode == self).unwrap_or(0);
		Self::ALL[(index + 1) % Self::ALL.len()]
	}
}

impl std::fmt::Display for TravelMode {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.label())
	}
}

/// The map that "Open in Maps" shows a trip on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MapApp {
	#[default]
	OpenStreetMap,
	GoogleMaps,
}

impl MapApp {
	pub const ALL: [MapApp; 2] = [Self::OpenStreetMap, Self::GoogleMaps];

	pub fn label(self) -> &'static str {
		match self {
			Self::OpenStreetMap => "OpenStreetMap",
			Self::GoogleMaps => "Google Maps",
		}
	}
}

impl std::fmt::Display for MapApp {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(self.label())
	}
}

/// How the trip from one place to another is made, when that is not the default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteMode {
	pub from: u64,
	pub to: u64,
	pub mode: TravelMode,
}

/// A trip already worked out, as the synced config holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRoute {
	pub from: u64,
	pub to: u64,
	pub mode: TravelMode,
	pub seconds: u32,
	pub meters: u32,
}

/// How far, and for how long, a trip goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
	pub seconds: u32,
	pub meters: u32,
}

/// A trip between two places, whichever events it falls between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Leg {
	pub from: u64,
	pub to: u64,
}

/// A trip a day needs, and where it sits among the day's events.
#[derive(Debug, Clone, PartialEq)]
pub struct Hop {
	pub leg: Leg,
	/// The event the trip leads to, which is where its box is drawn.
	pub before: String,
	pub from_title: String,
	pub to_title: String,
}

/// An event, as far as travel is concerned: when it is, and where.
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
	pub id: String,
	pub title: String,
	pub place: Option<u64>,
}

/// The trips a day needs, in order, given its events in the order they happen.
///
/// Every event with a place is a destination. Its trip starts at the place of
/// the nearest earlier event that has one, or at Home when there is none. No
/// trip is needed when the two places are the same.
pub fn hops(places: &[Place], home: Option<u64>, stops: &[Stop]) -> Vec<Hop> {
	let home = home.filter(|id| places.iter().any(|place| place.id == *id));
	let mut found = Vec::new();
	let mut previous: Option<(u64, &str)> = None;

	for stop in stops {
		let Some(place) = stop.place else { continue };
		let source = previous.or(home.map(|id| (id, START_OF_DAY)));
		if let Some((from, from_title)) = source
			&& from != place
		{
			found.push(Hop {
				leg: Leg { from, to: place },
				before: stop.id.clone(),
				from_title: from_title.to_owned(),
				to_title: stop.title.clone(),
			});
		}
		previous = Some((place, &stop.title));
	}

	found
}

/// The different trips among `hops`, in the order they are first made.
pub fn unique_legs(hops: impl IntoIterator<Item = Hop>) -> Vec<Leg> {
	let mut legs: Vec<Leg> = Vec::new();
	for hop in hops {
		if !legs.contains(&hop.leg) {
			legs.push(hop.leg);
		}
	}
	legs
}

/// "12 min", or "1 h 05 min".
pub fn duration_label(seconds: u32) -> String {
	let minutes = seconds.div_ceil(60).max(1);
	if minutes < 60 {
		format!("{minutes} min")
	} else {
		format!("{} h {:02} min", minutes / 60, minutes % 60)
	}
}

/// Convert a meter distance for display without changing the stored route.
pub fn distance_label(meters: u32, unit: DistanceUnit) -> String {
	if unit == DistanceUnit::Miles {
		format!("{:.1} mi", f64::from(meters) / 1609.344)
	} else if meters < 1000 {
		format!("{meters} m")
	} else {
		format!("{:.1} km", f64::from(meters) / 1000.0)
	}
}

pub fn route_label(route: Route, unit: DistanceUnit) -> String {
	format!(
		"{} · {}",
		duration_label(route.seconds),
		distance_label(route.meters, unit)
	)
}

/// A place that tasks and events can happen at, found with Mapbox search.
///
/// Everything needed to find the place again is kept apart from the title,
/// so the title can be renamed to whatever reads best ("Home", "Campus")
/// without the place losing its address or its Mapbox ID.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
	/// What tasks refer to. Unlike the title, it never changes.
	pub id: u64,
	/// The title shown in the app. Free to edit.
	pub name: String,
	/// The address as Mapbox spelled it when the place was chosen.
	#[serde(default)]
	pub address: String,
	/// Mapbox's ID for the place, which keeps one place from being saved
	/// twice. (Places saved before Mapbox hold a Google ID here; routes use
	/// the coordinates, so they keep working.)
	#[serde(default)]
	pub place_id: String,
	pub lat: f64,
	pub lng: f64,
	/// Starred, so it comes first when choosing among saved places.
	#[serde(default, skip_serializing_if = "std::ops::Not::not")]
	pub favorite: bool,
}

/// A place Mapbox found, before it is saved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Found {
	pub place_id: String,
	pub name: String,
	pub address: String,
	pub lat: f64,
	pub lng: f64,
}

/// The part of the `[travel]` settings that editing a location needs.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaceSettings {
	pub api_key: String,
	pub home: Option<u64>,
	pub places: Vec<Place>,
	pub event_places: BTreeMap<String, u64>,
	/// How a trip is made until it is set otherwise.
	pub default_mode: TravelMode,
	/// The map "Open in Maps" opens a trip on.
	pub map_app: MapApp,
	/// Trips whose method was chosen by hand.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub modes: Vec<RouteMode>,
	/// Whether travel is shown on the calendar.
	pub show: bool,
	/// The trips worked out so far, by any device.
	#[serde(skip_serializing_if = "Vec::is_empty")]
	pub routes: Vec<SavedRoute>,
}

impl PlaceSettings {
	/// How the trip from `from` to `to` is made.
	pub fn mode_for(&self, from: u64, to: u64) -> TravelMode {
		self.modes
			.iter()
			.find(|route| route.from == from && route.to == to)
			.map_or(self.default_mode, |route| route.mode)
	}

	/// Chooses how the trip from `from` to `to` is made.
	pub fn set_mode(&mut self, from: u64, to: u64, mode: TravelMode) {
		match self
			.modes
			.iter_mut()
			.find(|route| route.from == from && route.to == to)
		{
			Some(route) => route.mode = mode,
			None => self.modes.push(RouteMode { from, to, mode }),
		}
	}

	/// Keeps a trip that was worked out, replacing any older answer for it.
	pub fn set_route(&mut self, route: SavedRoute) {
		match self
			.routes
			.iter_mut()
			.find(|old| old.from == route.from && old.to == route.to && old.mode == route.mode)
		{
			Some(old) => *old = route,
			None => self.routes.push(route),
		}
	}

	/// The trips a day needs, given its timed events in the order they happen.
	pub fn day_hops<'a>(&self, events: impl IntoIterator<Item = &'a Event>) -> Vec<Hop> {
		let stops: Vec<Stop> = events
			.into_iter()
			.map(|event| Stop {
				id: event.id.clone(),
				title: event.title.clone(),
				place: place_of(&self.places, &self.event_places, event),
			})
			.collect();
		hops(&self.places, self.home, &stops)
	}

	/// How long this trip takes by its method, if that has been worked out.
	pub fn route(&self, leg: Leg) -> Option<Route> {
		let mode = self.mode_for(leg.from, leg.to);
		self.routes
			.iter()
			.find(|saved| saved.from == leg.from && saved.to == leg.to && saved.mode == mode)
			.map(|saved| Route {
				seconds: saved.seconds,
				meters: saved.meters,
			})
	}

	pub fn place(&self, id: u64) -> Option<&Place> {
		self.places.iter().find(|place| place.id == id)
	}

	/// The places in alphabetical order, ignoring case.
	pub fn sorted(&self) -> Vec<&Place> {
		let mut places: Vec<&Place> = self.places.iter().collect();
		places.sort_by_cached_key(|place| (place.name.to_lowercase(), place.id));
		places
	}

	/// The token to send to Mapbox: the one in settings, or else the
	/// `MAPBOX_ACCESS_TOKEN` environment variable.
	pub fn key(&self) -> Option<String> {
		let key = self.api_key.trim();
		if !key.is_empty() {
			return Some(key.to_owned());
		}
		std::env::var("MAPBOX_ACCESS_TOKEN")
			.ok()
			.map(|key| key.trim().to_owned())
			.filter(|key| !key.is_empty())
	}

	/// Home as Mapbox's proximity bias: longitude, then latitude.
	pub fn proximity(&self) -> Option<(f64, f64)> {
		self.home
			.and_then(|id| self.place(id))
			.map(|home| (home.lng, home.lat))
	}
}

/// Saves a place Mapbox found, unless it is saved already, and returns its id.
/// Choosing the same place twice gives the same place back, with the title it
/// has been renamed to.
pub fn add_place(
	places: &mut Vec<Place>,
	name: &str,
	address: &str,
	place_id: &str,
	lat: f64,
	lng: f64,
) -> u64 {
	if !place_id.is_empty()
		&& let Some(existing) = places.iter().find(|place| place.place_id == place_id)
	{
		return existing.id;
	}

	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |elapsed| elapsed.as_millis() as u64);
	let highest = places.iter().map(|place| place.id).max().unwrap_or(0);
	let id = now.max(highest + 1);

	places.push(Place {
		id,
		name: name.trim().to_owned(),
		address: address.trim().to_owned(),
		place_id: place_id.to_owned(),
		lat,
		lng,
		favorite: false,
	});
	id
}

/// The saved places that match `query`, without asking Mapbox: favorites
/// first, then the most recently saved (ids are the time a place was saved).
/// Every word of the query must appear in the title or the address, in any
/// case; an empty query matches every place.
pub fn saved_matches<'a>(places: &'a [Place], query: &str) -> Vec<&'a Place> {
	let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
	let mut found: Vec<&Place> = places
		.iter()
		.filter(|place| {
			let haystack = format!("{} {}", place.name, place.address).to_lowercase();
			words.iter().all(|word| haystack.contains(word.as_str()))
		})
		.collect();
	found.sort_by_key(|place| (!place.favorite, std::cmp::Reverse(place.id)));
	found
}

/// How a place reads in an event's location: its title and then its address.
pub fn location_text(place: &Place) -> String {
	if place.address.is_empty() || place.address == place.name {
		place.name.clone()
	} else {
		format!("{}, {}", place.name, place.address)
	}
}

/// How much of an address has to be there for it to say which place is meant.
const MIN_ADDRESS_MATCH: usize = 6;

/// Which saved place an event or task happens at.
///
/// A task records it. An event may have been given one here; otherwise the
/// location text Google keeps is matched against the saved places, including
/// the text [`location_text`] writes. That has to keep reading back as the
/// place: it is all that is left when an event has no row in `event_places`,
/// such as another occurrence of a series or a row a config reload replaced.
pub fn place_of(
	places: &[Place],
	event_places: &BTreeMap<String, u64>,
	event: &Event,
) -> Option<u64> {
	let saved = |id: &u64| places.iter().any(|place| place.id == *id);
	if event.is_task() {
		return crate::timeblocking::metadata(event)
			.location
			.filter(|id| saved(id));
	}

	if let Some(place) = event_places.get(&event.id).filter(|id| saved(id)) {
		return Some(*place);
	}

	let text = normalize(&event.details.location);
	if text.is_empty() {
		return None;
	}

	places
		.iter()
		.find(|place| {
			normalize(&place.name) == text
				|| normalize(&place.address) == text
				|| normalize(&location_text(place)) == text
		})
		.or_else(|| places.iter().find(|place| address_matches(place, &text)))
		.map(|place| place.id)
}

/// Whether a location is this place's address, give or take. Only tried once no
/// place matches outright, as places in one building share an address.
fn address_matches(place: &Place, text: &str) -> bool {
	let address = normalize(&place.address);
	if address.len() < MIN_ADDRESS_MATCH {
		return false;
	}
	// Mapbox's addresses run on into the country, which is rarely typed.
	(text.len() >= MIN_ADDRESS_MATCH && address.starts_with(text))
        // What `location_text` wrote before the place was renamed.
        || text.ends_with(&format!(", {address}"))
}

fn normalize(text: &str) -> String {
	text.split_whitespace()
		.collect::<Vec<_>>()
		.join(" ")
		.to_lowercase()
}

/// Searches Mapbox for `query`. Blocking: run it on a worker thread.
pub fn search_places(
	key: &str,
	query: &str,
	proximity: Option<(f64, f64)>,
) -> Result<Vec<Found>, String> {
	Ok(parse_places(&get(&search_url(key, query, proximity))?))
}

/// Proximity is longitude, latitude; it biases ranking without limiting the area.
pub fn search_url(key: &str, query: &str, proximity: Option<(f64, f64)>) -> String {
	let mut url = format!(
		"{SEARCH_URL}?q={}&limit={MAX_RESULTS}&access_token={}",
		urlencoding::encode(query),
		urlencoding::encode(key),
	);

	if let Some((lng, lat)) = proximity
		&& lng.is_finite()
		&& lat.is_finite()
		&& (-180.0..=180.0).contains(&lng)
		&& (-90.0..=90.0).contains(&lat)
	{
		url.push_str(&format!("&proximity={lng},{lat}"));
	}
	url
}

pub fn parse_places(response: &Value) -> Vec<Found> {
	response["features"]
		.as_array()
		.into_iter()
		.flatten()
		.filter_map(|feature| {
			let properties = &feature["properties"];
			let place_id = properties["mapbox_id"].as_str()?.to_owned();
			let address = properties["full_address"]
				.as_str()
				.or_else(|| properties["place_formatted"].as_str())
				.unwrap_or("")
				.to_owned();
			let name = properties["name"]
				.as_str()
				.filter(|name| !name.trim().is_empty())
				.map(str::to_owned)
				.or_else(|| address.split(',').next().map(|part| part.trim().to_owned()))
				.filter(|name| !name.is_empty())?;

			// `[longitude, latitude]`, as GeoJSON has it.
			let (lng, lat) = match (
				feature["geometry"]["coordinates"][0].as_f64(),
				feature["geometry"]["coordinates"][1].as_f64(),
			) {
				(Some(lng), Some(lat)) => (lng, lat),
				_ => (
					properties["coordinates"]["longitude"].as_f64()?,
					properties["coordinates"]["latitude"].as_f64()?,
				),
			};

			Some(Found {
				place_id,
				name,
				address,
				lat,
				lng,
			})
		})
		.collect()
}

/// How long and how far a trip is, from Mapbox Directions. Blocking: run it on
/// a worker thread.
pub fn compute_route(
	key: &str,
	from: &Place,
	to: &Place,
	mode: TravelMode,
) -> Result<Route, String> {
	parse_route(&get(&route_url(key, from, to, mode))?)
}

/// The Directions request for a trip. Mapbox takes coordinates only, as
/// `longitude,latitude` pairs. Its route distance is always returned in meters;
/// display units must not affect this request or the cache key.
pub fn route_url(key: &str, from: &Place, to: &Place, mode: TravelMode) -> String {
	format!(
		"{DIRECTIONS_URL}/{}/{},{};{},{}?overview=false&access_token={}",
		mode.profile(),
		from.lng,
		from.lat,
		to.lng,
		to.lat,
		urlencoding::encode(key),
	)
}

pub fn parse_route(response: &Value) -> Result<Route, String> {
	let route = response["routes"]
		.as_array()
		.and_then(|routes| routes.first())
		.ok_or("Mapbox found no route between these places")?;

	let seconds = route["duration"]
		.as_f64()
		.ok_or("Mapbox did not say how long the trip takes")?;

	Ok(Route {
		seconds: seconds.round().max(0.0) as u32,
		meters: route["distance"].as_f64().unwrap_or(0.0).round().max(0.0) as u32,
	})
}

/// A page of the chosen map showing the trip. Mapbox has no page of its own to
/// send someone to. Both pages are given coordinates, as that is what a place
/// is known by here.
pub fn maps_url(app: MapApp, mode: TravelMode, from: &Place, to: &Place) -> String {
	match app {
		MapApp::OpenStreetMap => format!(
			"https://www.openstreetmap.org/directions?engine={}&route={}%2C{}%3B{}%2C{}",
			mode.osm_engine(),
			from.lat,
			from.lng,
			to.lat,
			to.lng,
		),
		MapApp::GoogleMaps => format!(
			"https://www.google.com/maps/dir/?api=1&origin={}%2C{}&destination={}%2C{}&travelmode={}",
			from.lat,
			from.lng,
			to.lat,
			to.lng,
			mode.google_travelmode(),
		),
	}
}

/// A GET against Mapbox. Blocking.
pub fn get(url: &str) -> Result<Value, String> {
	match crate::calendar::api::agent().get(url).call() {
		Ok(response) => response
			.into_json::<Value>()
			.map_err(|error| format!("Mapbox: {error}")),
		Err(ureq::Error::Status(code, response)) => {
			let detail = response
				.into_json::<Value>()
				.ok()
				.and_then(|body| {
					body["message"]
						.as_str()
						.or_else(|| body["error"].as_str())
						.map(str::to_owned)
				})
				.filter(|message| !message.is_empty())
				.unwrap_or_else(|| format!("HTTP {code}"));
			Err(format!("Mapbox: {detail}"))
		}
		// The URL carries the token, which must not end up in the settings page.
		Err(ureq::Error::Transport(error)) => Err(format!("Mapbox: {}", error.kind())),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn distance_units_convert_only_the_display() {
		let route = Route {
			seconds: 600,
			meters: 1609,
		};
		assert_eq!(
			route_label(route, DistanceUnit::Kilometers),
			"10 min · 1.6 km"
		);
		assert_eq!(route_label(route, DistanceUnit::Miles), "10 min · 1.0 mi");
		assert_eq!(distance_label(0, DistanceUnit::Miles), "0.0 mi");
		assert_eq!(distance_label(850, DistanceUnit::Kilometers), "850 m");
		assert_eq!(distance_label(8047, DistanceUnit::Miles), "5.0 mi");
	}

	#[test]
	fn a_trip_worked_out_again_replaces_only_its_own_answer() {
		let route = |mode, seconds| SavedRoute {
			from: 1,
			to: 2,
			mode,
			seconds,
			meters: 100,
		};
		let mut travel = PlaceSettings::default();
		travel.set_route(route(TravelMode::Walk, 600));
		travel.set_route(route(TravelMode::Car, 120));
		travel.set_route(route(TravelMode::Walk, 660));

		assert_eq!(
			travel.routes,
			[route(TravelMode::Walk, 660), route(TravelMode::Car, 120)]
		);
	}

	#[test]
	fn a_days_repeated_trip_is_listed_once_in_the_order_it_is_first_made() {
		let hop = |from, to| Hop {
			leg: Leg { from, to },
			before: String::new(),
			from_title: String::new(),
			to_title: String::new(),
		};
		assert_eq!(
			unique_legs([hop(1, 2), hop(2, 1), hop(1, 2)]),
			[Leg { from: 1, to: 2 }, Leg { from: 2, to: 1 }]
		);
	}

	#[test]
	fn saved_matches_puts_favorites_then_recent_first() {
		let mut places = vec![
			place(1, "Cafe", "1 Main St"),
			place(2, "Library", "5 Oak Ave"),
			place(3, "Main Office", "9 Pine Rd"),
		];
		let ids = |found: Vec<&Place>| found.iter().map(|p| p.id).collect::<Vec<_>>();
		assert_eq!(ids(saved_matches(&places, "")), vec![3, 2, 1]);
		assert_eq!(ids(saved_matches(&places, "main")), vec![3, 1]);
		assert_eq!(ids(saved_matches(&places, "MAIN st")), vec![1]);
		assert!(saved_matches(&places, "zoo").is_empty());
		places[0].favorite = true;
		assert_eq!(ids(saved_matches(&places, "")), vec![1, 3, 2]);
	}

	fn place(id: u64, name: &str, address: &str) -> Place {
		Place {
			id,
			name: name.to_owned(),
			address: address.to_owned(),
			place_id: format!("mapbox-{id}"),
			lat: 1.0,
			lng: 2.0,
			favorite: false,
		}
	}

	fn at(places: &[Place], location: &str) -> Option<u64> {
		let mut event = crate::calendar::sample_snapshot().events.remove(0);
		event.id = "event".into();
		event.details.location = location.to_owned();
		place_of(places, &BTreeMap::new(), &event)
	}

	#[test]
	fn an_occurrences_saved_place_survives_retiming_and_splitting() {
		let places = [place(1, "Office", ""), place(2, "Home", "")];
		let mut event = crate::calendar::sample_snapshot().events.remove(0);
		event.calendar_id = "app".into();
		event.all_day = false;
		event.details = Default::default();
		event.details.location = location_text(&places[0]);
		event.details.recurring_event_id = Some("series".into());
		let day = event.start.date_naive();
		event.start = crate::calendar::at_clock(day, 9, 0);
		let old_id = format!("series_{}", event.start.to_utc().format("%Y%m%dT%H%M%SZ"));
		let mut assigned = BTreeMap::from([(old_id, 1)]);
		event.start += chrono::Duration::hours(1);
		event.details.original_start = Some(event.start);
		event.id = format!("series_{}", event.start.to_utc().format("%Y%m%dT%H%M%SZ"));
		assert_eq!(place_of(&places, &assigned, &event), Some(1));
		event.id = format!("split_{}", event.start.to_utc().format("%Y%m%dT%H%M%SZ"));
		event.details.recurring_event_id = Some("split".into());
		event.details.split_from = vec!["app/series".into()];
		assert_eq!(place_of(&places, &assigned, &event), Some(1));
		assigned.insert(event.id.clone(), 2);
		assert_eq!(
			place_of(&places, &assigned, &event),
			Some(2),
			"an explicit newer choice wins"
		);
		assigned.remove(&event.id);
		event.details.location.clear();
		assert_eq!(place_of(&places, &assigned, &event), None);
	}

	/// What is written to an event's location has to read back as that place,
	/// or an event whose `event_places` row is missing silently has no place
	/// and the trips around it never change.
	#[test]
	fn the_location_written_for_a_place_reads_back_as_that_place() {
		let places = [
			place(1, "Campus", "500 Market St, San Francisco, California"),
			place(2, "Same Street Cafe", "66 Mint St, San Francisco"),
			place(3, "No address", ""),
			place(4, "Same", "Same"),
		];

		for place in &places {
			let written = location_text(place);
			assert_eq!(at(&places, &written), Some(place.id), "{written:?}");
			assert_eq!(
				at(&places, &format!("  {}  ", written.to_uppercase())),
				Some(place.id),
				"{written:?}, as another client might respell it"
			);
		}
	}

	#[test]
	fn a_renamed_place_still_owns_what_was_written_under_its_old_title() {
		let mut places = [place(1, "Starbucks", "1 Main St, Springfield")];
		let written = location_text(&places[0]);
		places[0].name = "Study spot".into();

		assert_eq!(at(&places, &written), Some(1));
	}

	#[test]
	fn places_sharing_an_address_are_told_apart_by_their_titles() {
		let places = [
			place(1, "Office", "1 Main St, Springfield"),
			place(2, "Cafe", "1 Main St, Springfield"),
		];

		assert_eq!(at(&places, &location_text(&places[1])), Some(2));
		assert_eq!(at(&places, &location_text(&places[0])), Some(1));
	}

	#[test]
	fn other_locations_are_still_no_place() {
		let places = [
			place(1, "Campus", "500 Market St"),
			place(2, "Short", "1 A"),
		];

		for text in [
			"",
			"Room 4",
			"Not 500 Market St",
			"Campus, elsewhere",
			"Foo, 1 A",
		] {
			assert_eq!(at(&places, text), None, "{text:?}");
		}
	}
}
