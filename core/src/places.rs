//! Saved places: where tasks and calendar events happen, and the Mapbox search
//! that finds new ones.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::calendar::Event;

const SEARCH_URL: &str = "https://api.mapbox.com/search/searchbox/v1/forward";

/// The most places a search offers.
const MAX_RESULTS: u32 = 8;

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
}

impl PlaceSettings {
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
    });
    id
}

/// How a place reads in an event's location: its title and then its address.
pub fn location_text(place: &Place) -> String {
    if place.address.is_empty() || place.address == place.name {
        place.name.clone()
    } else {
        format!("{}, {}", place.name, place.address)
    }
}

/// Which saved place an event or task happens at.
///
/// A task records it. An event may have been given one here; otherwise the
/// location text Google keeps is matched against the saved titles and addresses.
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
        .find(|place| normalize(&place.name) == text || normalize(&place.address) == text)
        .or_else(|| {
            // Mapbox's addresses run on into the country, which is rarely typed.
            places
                .iter()
                .find(|place| text.len() >= 6 && normalize(&place.address).starts_with(&text))
        })
        .map(|place| place.id)
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
