//! Travel between the places that events and tasks happen at.
//!
//! Places are found with the Mapbox Search Box API and saved in the config
//! (`config::TravelSettings`), where they keep the Mapbox ID and the
//! address whatever they are renamed to. A task points at a place by id; a
//! calendar event is matched to a place by its location text.
//!
//! Travel is only worked out between events that follow one another: with
//! events A, B and C in a day, that is A to B and B to C, never A to C. The
//! day's first event is reached from Home, and the day's
//! last event is left for Home. How long a trip takes comes
//! from the Mapbox Directions API, and is kept on disk so that it is only asked
//! for once.

pub mod map;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use iced::Task;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::{Message, State, queue_save};
use crate::blocking;
use crate::calendar::Event;
use crate::config::{MapApp, Place, TravelMode, TravelSettings};
use crate::timeblocking as tb;

const SEARCH_URL: &str = "https://api.mapbox.com/search/searchbox/v1/forward";
const DIRECTIONS_URL: &str = "https://api.mapbox.com/directions/v5/mapbox";

/// The most places a search offers.
const MAX_RESULTS: u32 = 8;

/// What the Travel tab calls the start and end of the day.
pub const START_OF_DAY: &str = "Start of day";
pub const END_OF_DAY: &str = "End of day";

/// A place Mapbox found, before it is saved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Found {
    pub place_id: String,
    pub name: String,
    pub address: String,
    pub lat: f64,
    pub lng: f64,
}

/// What a chosen place is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickTarget {
    /// Where this task, by id, happens.
    Task(String),
    /// Where this calendar event, by id, happens.
    Event(String),
    /// Home, where the day's travel starts and ends.
    Home,
}

/// A place search that is open: what was typed and what came back.
#[derive(Debug, Clone, PartialEq)]
pub struct Picker {
    pub target: PickTarget,
    pub query: String,
    pub results: Vec<Found>,
    pub searching: bool,
    /// Whether a search has finished, so "no results" is not shown before one.
    pub searched: bool,
    pub error: Option<String>,
}

/// How far, and for how long, a trip goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    pub seconds: u32,
    pub meters: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RouteState {
    Loading,
    Ready(Route),
    Failed(String),
}

/// A trip from one place to another by one method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RouteKey {
    pub from: u64,
    pub to: u64,
    pub mode: TravelMode,
}

/// A trip between two places, whichever events it falls between.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Leg {
    pub from: u64,
    pub to: u64,
}

/// The details card that is open, for the trip it is about.
#[derive(Debug, Clone, PartialEq)]
pub struct Open {
    pub leg: Leg,
    /// What the trip leaves and what it arrives at, when it is one of a day's.
    pub between: Option<(String, String)>,
}

/// A trip a day needs, and where it sits among the day's events.
#[derive(Debug, Clone, PartialEq)]
pub struct Hop {
    pub leg: Leg,
    /// The event the trip comes before, which is where its box is drawn.
    /// `None` for the trip home, which follows the last event.
    pub before: Option<String>,
    /// The event the trip follows, for the trip home.
    pub after: Option<String>,
    pub from_title: String,
    pub to_title: String,
}

/// What the Travel tab is in the middle of.
#[derive(Debug, Default)]
pub struct Workspace {
    pub map: map::Workspace,
    /// Whether travel is shown on the day view.
    pub show: bool,
    pub picker: Option<Picker>,
    pub routes: HashMap<RouteKey, RouteState>,
    pub open: Option<Open>,
    /// The place being renamed and the title typed so far.
    pub rename: Option<(u64, String)>,
    /// Counts searches, so a slow answer to an old one is thrown away.
    serial: u64,
    cache: Option<PathBuf>,
}

impl Workspace {
    /// Keeps worked-out trips in `path`, and reads those already there. Only a
    /// real boot calls this: a state built by hand must not touch the user's
    /// cache.
    pub fn cache_in(&mut self, path: Option<PathBuf>) {
        if let Some(path) = &path {
            for (key, route) in read_cache(path) {
                self.routes.insert(key, RouteState::Ready(route));
            }
        }
        self.cache = path;
    }

    pub fn state(&self, key: RouteKey) -> Option<&RouteState> {
        self.routes.get(&key)
    }
}

#[derive(Debug, Clone)]
pub enum Action {
    Map(map::Action),
    /// Show or hide travel on the day view.
    Toggle,
    ApiKey(String),
    DefaultMode(TravelMode),
    SetMapApp(MapApp),

    OpenPicker(PickTarget),
    ClosePicker,
    Query(String),
    Search,
    Searched(u64, Result<Vec<Found>, String>),
    /// Use the search result at this position.
    Choose(usize),
    /// Forget where the target is.
    Clear(PickTarget),

    /// Start renaming this place.
    Rename(u64),
    RenameText(String),
    SaveRename,
    CancelRename,

    OpenLeg(Leg, Option<(String, String)>),
    CloseLeg,
    SetMode(Leg, TravelMode),
    Recalculate(Leg),
    OpenMaps(Leg),
    Routed(RouteKey, Result<Route, String>),
    /// Google answered a change to an event's location.
    EventLocationSaved(Result<(), String>),
}

pub fn update(state: &mut State, action: Action) -> Task<Message> {
    match action {
        Action::Map(action) => map::update(state, action),
        Action::Toggle => {
            state.travel.show = !state.travel.show;
            if state.travel.show {
                // A trip that failed is tried again when travel is turned back on.
                state
                    .travel
                    .routes
                    .retain(|_, route| !matches!(route, RouteState::Failed(_)));
            }
            Task::none()
        }

        Action::ApiKey(key) => {
            state.config.travel.api_key = key;
            // A key that was wrong may be right now.
            state
                .travel
                .routes
                .retain(|_, route| !matches!(route, RouteState::Failed(_)));
            queue_save(state)
        }

        Action::DefaultMode(mode) => {
            state.config.travel.default_mode = mode;
            queue_save(state)
        }

        Action::SetMapApp(app) => {
            state.config.travel.map_app = app;
            queue_save(state)
        }

        Action::OpenPicker(target) => {
            state.travel.picker = Some(Picker {
                target,
                query: String::new(),
                results: Vec::new(),
                searching: false,
                searched: false,
                error: None,
            });
            iced::widget::operation::focus(iced::widget::Id::new(PICKER_INPUT))
        }

        Action::ClosePicker => {
            state.travel.picker = None;
            Task::none()
        }

        Action::Query(query) => {
            if let Some(picker) = &mut state.travel.picker {
                picker.query = query;
                picker.error = None;
            }
            Task::none()
        }

        Action::Search => search(state),

        Action::Searched(serial, result) => {
            if serial != state.travel.serial {
                return Task::none();
            }
            if let Some(picker) = &mut state.travel.picker {
                picker.searching = false;
                picker.searched = true;
                match result {
                    Ok(results) => {
                        picker.results = results;
                        picker.error = None;
                    }
                    Err(error) => {
                        picker.results.clear();
                        picker.error = Some(error);
                    }
                }
            }
            Task::none()
        }

        Action::Choose(index) => choose(state, index),

        Action::Clear(target) => assign(state, &target, None),

        Action::Rename(id) => {
            if let Some(place) = state.config.travel.place(id) {
                state.travel.rename = Some((id, place.name.clone()));
            }
            Task::none()
        }

        Action::RenameText(text) => {
            if let Some((_, name)) = &mut state.travel.rename {
                *name = text;
            }
            Task::none()
        }

        Action::SaveRename => {
            let Some((id, name)) = state.travel.rename.clone() else {
                return Task::none();
            };
            let name = name.trim();
            // A place without a title would be a blank row, so it keeps the
            // edit open instead.
            if name.is_empty() {
                return Task::none();
            }
            if let Some(place) = state
                .config
                .travel
                .places
                .iter_mut()
                .find(|place| place.id == id)
            {
                place.name = name.to_owned();
            }
            state.travel.rename = None;
            queue_save(state)
        }

        Action::CancelRename => {
            state.travel.rename = None;
            Task::none()
        }

        Action::OpenLeg(leg, between) => {
            state.travel.open = Some(Open { leg, between });
            Task::none()
        }

        Action::CloseLeg => {
            state.travel.open = None;
            Task::none()
        }

        Action::SetMode(leg, mode) => {
            state.config.travel.set_mode(leg.from, leg.to, mode);
            let fetch = fetch_missing(state, [leg]);
            Task::batch([queue_save(state), fetch])
        }

        Action::Recalculate(leg) => {
            let mode = state.config.travel.mode_for(leg.from, leg.to);
            state.travel.routes.remove(&RouteKey {
                from: leg.from,
                to: leg.to,
                mode,
            });
            fetch_missing(state, [leg])
        }

        Action::OpenMaps(leg) => {
            if let Some(url) = maps_url(&state.config.travel, leg) {
                if let Err(error) = crate::calendar::oauth::open_in_browser(&url) {
                    log::warn!("could not open the map: {error}");
                }
            }
            Task::none()
        }

        Action::EventLocationSaved(Ok(())) => Task::none(),

        Action::EventLocationSaved(Err(error)) => {
            // The place is kept here, so trips still work; only Google's copy
            // of the location is behind.
            log::warn!("could not set the event's location on Google: {error}");
            state.calendar.error = Some(format!(
                "The place was saved here but not on Google: {error}"
            ));
            Task::none()
        }

        Action::Routed(key, result) => match result {
            Ok(route) => {
                state.travel.routes.insert(key, RouteState::Ready(route));
                save_cache(state)
            }
            Err(error) => {
                state.travel.routes.insert(key, RouteState::Failed(error));
                Task::none()
            }
        },
    }
}

/// The id of the search box, so opening a search can focus it.
pub const PICKER_INPUT: &str = "travel-search";

fn search(state: &mut State) -> Task<Message> {
    let Some(picker) = &mut state.travel.picker else {
        return Task::none();
    };
    let query = picker.query.trim().to_owned();
    if query.is_empty() || picker.searching {
        return Task::none();
    }
    let Some(key) = state.config.travel.key() else {
        picker.error = Some(NO_KEY.to_owned());
        return Task::none();
    };

    picker.searching = true;
    picker.error = None;
    state.travel.serial += 1;
    let serial = state.travel.serial;

    let proximity = state
        .config
        .travel
        .home
        .and_then(|id| state.config.travel.place(id))
        .map(|home| (home.lng, home.lat));
    blocking::run(move || search_places(&key, &query, proximity))
        .map(move |result| Message::Travel(Action::Searched(serial, result)))
}

/// What is shown when a key is needed and there is none.
pub const NO_KEY: &str = "Add a Mapbox access token in Settings first.";

fn choose(state: &mut State, index: usize) -> Task<Message> {
    let Some(picker) = state.travel.picker.take() else {
        return Task::none();
    };
    let Some(found) = picker.results.get(index) else {
        state.travel.picker = Some(picker);
        return Task::none();
    };

    let id = state.config.travel.add_place(
        &found.name,
        &found.address,
        &found.place_id,
        found.lat,
        found.lng,
    );
    let save = queue_save(state);
    let assigned = assign(state, &picker.target, Some(id));

    Task::batch([save, assigned])
}

/// Points the target at a place, or at none.
fn assign(state: &mut State, target: &PickTarget, place: Option<u64>) -> Task<Message> {
    match target {
        PickTarget::Home => {
            state.config.travel.home = place;
            queue_save(state)
        }
        PickTarget::Task(id) => {
            crate::timeblocking::update(state, tb::Action::Location(id.clone(), place))
        }
        PickTarget::Event(id) => set_event_place(state, id, place),
    }
}

/// Gives a calendar event a place, or takes it away. The choice is kept here,
/// so trips are known at once, and the place's name and address are written
/// to the event's location on Google so it reads the same there.
fn set_event_place(state: &mut State, id: &str, place: Option<u64>) -> Task<Message> {
    let Some(calendar_id) = state
        .calendar
        .event(id)
        .map(|event| event.calendar_id.clone())
    else {
        return Task::none();
    };

    let text = place
        .and_then(|id| state.config.travel.place(id))
        .map(location_text)
        .unwrap_or_default();

    match place {
        Some(place) => {
            state
                .config
                .travel
                .event_places
                .insert(id.to_owned(), place);
        }
        None => {
            state.config.travel.event_places.remove(id);
        }
    }
    state.calendar.set_location(id, &text);
    let save = queue_save(state);

    let client_id = state.config.calendar.client_id.clone();
    let client_secret = state.config.calendar.client_secret.clone();
    let id = id.to_owned();
    let write = blocking::run(move || {
        crate::calendar::set_event_location(&client_id, &client_secret, &calendar_id, &id, &text)
    })
    .map(|result| Message::Travel(Action::EventLocationSaved(result)));

    Task::batch([save, write])
}

/// How a place reads in an event's location: its title and then its address.
pub fn location_text(place: &Place) -> String {
    if place.address.is_empty() || place.address == place.name {
        place.name.clone()
    } else {
        format!("{}, {}", place.name, place.address)
    }
}

/// Works out whichever trips the day view or the open card needs and does
/// not have yet. Runs after every message, so a synced event, a day change
/// or a new location all find their trips without each having to ask.
pub fn reconcile(state: &mut State) -> Task<Message> {
    let mut legs: Vec<Leg> = Vec::new();

    if state.travel.show {
        if state.range_view_active() {
            for day in state.calendar_dates() {
                legs.extend(day_hops(state, day).into_iter().map(|hop| hop.leg));
            }
        } else {
            legs.extend(
                day_hops(state, state.calendar.day)
                    .into_iter()
                    .map(|hop| hop.leg),
            );
        }
    }
    if let Some(open) = &state.travel.open {
        legs.push(open.leg);
    }

    fetch_missing(state, legs)
}

/// Asks Mapbox for each trip that has not been asked for.
fn fetch_missing(state: &mut State, legs: impl IntoIterator<Item = Leg>) -> Task<Message> {
    let Some(key) = state.config.travel.key() else {
        return Task::none();
    };

    let mut tasks = Vec::new();
    for leg in legs {
        let mode = state.config.travel.mode_for(leg.from, leg.to);
        let route = RouteKey {
            from: leg.from,
            to: leg.to,
            mode,
        };
        if state.travel.routes.contains_key(&route) {
            continue;
        }
        let (Some(from), Some(to)) = (
            state.config.travel.place(leg.from).cloned(),
            state.config.travel.place(leg.to).cloned(),
        ) else {
            continue;
        };

        state.travel.routes.insert(route, RouteState::Loading);
        let key = key.clone();
        tasks.push(
            blocking::run(move || compute_route(&key, &from, &to, mode))
                .map(move |result| Message::Travel(Action::Routed(route, result))),
        );
    }

    Task::batch(tasks)
}

fn save_cache(state: &State) -> Task<Message> {
    let Some(path) = state.travel.cache.clone() else {
        return Task::none();
    };
    let ready: Vec<(RouteKey, Route)> = state
        .travel
        .routes
        .iter()
        .filter_map(|(key, route)| match route {
            RouteState::Ready(route) => Some((*key, *route)),
            _ => None,
        })
        .collect();

    blocking::run(move || {
        if let Err(error) = write_cache(&path, &ready) {
            log::warn!("could not save travel times: {error}");
        }
    })
    .map(|()| Message::Ignored)
}

// ---------------------------------------------------------------------------
// Which trips a day needs
// ---------------------------------------------------------------------------

/// An event, as far as travel is concerned: when it is, and where.
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    pub id: String,
    pub title: String,
    pub place: Option<u64>,
}

/// The place an event happens at, if it has one that is saved.
///
/// A task has the place it was given. A calendar event has only the text
/// Google keeps, so it is matched against the saved places' titles and
/// addresses.
pub fn place_of(travel: &TravelSettings, event: &Event) -> Option<u64> {
    if event.is_task() {
        return tb::metadata(event)
            .location
            .filter(|id| travel.place(*id).is_some());
    }

    if let Some(place) = travel
        .event_places
        .get(&event.id)
        .filter(|id| travel.place(**id).is_some())
    {
        return Some(*place);
    }

    let text = normalize(&event.details.location);
    if text.is_empty() {
        return None;
    }

    travel
        .places
        .iter()
        .find(|place| normalize(&place.name) == text || normalize(&place.address) == text)
        .or_else(|| {
            // Mapbox's addresses run on into the country, which is rarely typed.
            travel
                .places
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

/// The trips a day needs, in order, given its events in the order they happen.
///
/// Only events that follow one another are joined, and only when both have a
/// place and the places differ. The first event is reached from Home
/// and the last one is left for Home.
pub fn hops(travel: &TravelSettings, stops: &[Stop]) -> Vec<Hop> {
    let mut found = Vec::new();
    let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
        return found;
    };

    if let (Some(start), Some(place)) = (
        travel.home.filter(|id| travel.place(*id).is_some()),
        first.place,
    ) && start != place
    {
        found.push(Hop {
            leg: Leg {
                from: start,
                to: place,
            },
            before: Some(first.id.clone()),
            after: None,
            from_title: START_OF_DAY.to_owned(),
            to_title: first.title.clone(),
        });
    }

    for pair in stops.windows(2) {
        let [from, to] = pair else { continue };
        if let (Some(a), Some(b)) = (from.place, to.place)
            && a != b
        {
            found.push(Hop {
                leg: Leg { from: a, to: b },
                before: Some(to.id.clone()),
                after: None,
                from_title: from.title.clone(),
                to_title: to.title.clone(),
            });
        }
    }

    if let (Some(place), Some(end)) = (
        last.place,
        travel.home.filter(|id| travel.place(*id).is_some()),
    ) && place != end
    {
        found.push(Hop {
            leg: Leg {
                from: place,
                to: end,
            },
            before: None,
            after: Some(last.id.clone()),
            from_title: last.title.clone(),
            to_title: END_OF_DAY.to_owned(),
        });
    }

    found
}

/// The trips of one day, from the events the grid draws for it.
pub fn day_hops(state: &State, day: NaiveDate) -> Vec<Hop> {
    let stops: Vec<Stop> = state
        .calendar
        .timed_events_on(day, &state.config.calendar)
        .into_iter()
        .map(|event| Stop {
            id: event.id.clone(),
            title: event.title.clone(),
            place: place_of(&state.config.travel, event),
        })
        .collect();

    hops(&state.config.travel, &stops)
}

/// The different trips the day in view makes, in the order it makes them.
pub fn day_connections(state: &State) -> Vec<Leg> {
    unique(day_hops(state, state.calendar.day))
}

/// The different trips every loaded day makes, earliest day first.
pub fn all_connections(state: &State) -> Vec<Leg> {
    unique(
        state
            .calendar
            .loaded_days()
            .into_iter()
            .flat_map(|day| day_hops(state, day)),
    )
}

fn unique(hops: impl IntoIterator<Item = Hop>) -> Vec<Leg> {
    let mut legs: Vec<Leg> = Vec::new();
    for hop in hops {
        if !legs.contains(&hop.leg) {
            legs.push(hop.leg);
        }
    }
    legs
}

// ---------------------------------------------------------------------------
// How a trip reads
// ---------------------------------------------------------------------------

/// What is known about a trip.
#[derive(Debug, Clone, PartialEq)]
pub enum Summary {
    /// There is no token to ask Mapbox with.
    NoKey,
    /// Nobody has asked yet.
    Unknown,
    Loading,
    Failed(String),
    Ready(Route),
}

pub fn summary(state: &State, leg: Leg) -> Summary {
    let mode = state.config.travel.mode_for(leg.from, leg.to);
    match state.travel.state(RouteKey {
        from: leg.from,
        to: leg.to,
        mode,
    }) {
        Some(RouteState::Ready(route)) => Summary::Ready(*route),
        Some(RouteState::Loading) => Summary::Loading,
        Some(RouteState::Failed(error)) => Summary::Failed(error.clone()),
        None if state.config.travel.key().is_none() => Summary::NoKey,
        None => Summary::Unknown,
    }
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

/// "850 m", or "1.2 km".
pub fn distance_label(meters: u32) -> String {
    if meters < 1000 {
        format!("{meters} m")
    } else {
        format!("{:.1} km", f64::from(meters) / 1000.0)
    }
}

pub fn route_label(route: Route) -> String {
    format!(
        "{} · {}",
        duration_label(route.seconds),
        distance_label(route.meters)
    )
}

/// The one line a trip is given on the day view and in lists.
pub fn short_label(state: &State, leg: Leg) -> String {
    let mode = state.config.travel.mode_for(leg.from, leg.to);
    match summary(state, leg) {
        Summary::Ready(route) => format!("{mode} · {}", route_label(route)),
        Summary::Loading | Summary::Unknown => format!("{mode} · calculating…"),
        Summary::Failed(_) => format!("{mode} · no route"),
        Summary::NoKey => "Add a Mapbox token in Settings".to_owned(),
    }
}

/// A page of the chosen map showing the trip. Mapbox has no page of its own
/// to send someone to. Both pages are given coordinates, as that is what a
/// place is known by here.
fn maps_url(travel: &TravelSettings, leg: Leg) -> Option<String> {
    let from = travel.place(leg.from)?;
    let to = travel.place(leg.to)?;
    let mode = travel.mode_for(leg.from, leg.to);

    Some(match travel.map_app {
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
    })
}

// ---------------------------------------------------------------------------
// Talking to Mapbox
// ---------------------------------------------------------------------------

/// Searches Mapbox for `query`. Blocking: call it from
/// [`crate::blocking::run`].
pub fn search_places(
    key: &str,
    query: &str,
    proximity: Option<(f64, f64)>,
) -> Result<Vec<Found>, String> {
    Ok(parse_places(&get(&search_url(key, query, proximity))?))
}

/// Proximity is longitude, latitude; it biases ranking without limiting the area.
fn search_url(key: &str, query: &str, proximity: Option<(f64, f64)>) -> String {
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

/// How long and how far a trip is, from Mapbox Directions. Blocking: call it
/// from [`crate::blocking::run`].
pub fn compute_route(
    key: &str,
    from: &Place,
    to: &Place,
    mode: TravelMode,
) -> Result<Route, String> {
    parse_route(&get(&route_url(key, from, to, mode))?)
}

/// The Directions request for a trip. Mapbox takes coordinates only, as
/// `longitude,latitude` pairs.
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

fn get(url: &str) -> Result<Value, String> {
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

// ---------------------------------------------------------------------------
// The cache of worked-out trips
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Stored {
    from: u64,
    to: u64,
    mode: TravelMode,
    seconds: u32,
    meters: u32,
}

fn read_cache(path: &Path) -> Vec<(RouteKey, Route)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let stored: Vec<Stored> = serde_json::from_str(&text).unwrap_or_default();

    stored
        .into_iter()
        .map(|trip| {
            (
                RouteKey {
                    from: trip.from,
                    to: trip.to,
                    mode: trip.mode,
                },
                Route {
                    seconds: trip.seconds,
                    meters: trip.meters,
                },
            )
        })
        .collect()
}

fn write_cache(path: &Path, routes: &[(RouteKey, Route)]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let stored: Vec<Stored> = routes
        .iter()
        .map(|(key, route)| Stored {
            from: key.from,
            to: key.to,
            mode: key.mode,
            seconds: route.seconds,
            meters: route.meters,
        })
        .collect();

    // Written beside the file and renamed over it, so a crash leaves the old
    // cache rather than half of the new one.
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, serde_json::to_string(&stored)?)?;
    std::fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::theme::Catalog;
    use serde_json::json;

    fn state() -> State {
        State::new(Config::default(), Catalog::built_in())
    }

    fn place(travel: &mut TravelSettings, name: &str, mapbox: &str) -> u64 {
        travel.add_place(name, &format!("{name}, 1 Main St"), mapbox, 1.0, 2.0)
    }

    fn stop(id: &str, place: Option<u64>) -> Stop {
        Stop {
            id: id.to_owned(),
            title: id.to_owned(),
            place,
        }
    }

    fn found(name: &str) -> Found {
        Found {
            place_id: format!("mapbox-{name}"),
            name: name.to_owned(),
            address: format!("{name}, 1 Main St"),
            lat: 10.0,
            lng: 20.0,
        }
    }

    #[test]
    fn only_events_that_follow_each_other_are_joined() {
        let mut travel = TravelSettings::default();
        let a = place(&mut travel, "A", "g-a");
        let b = place(&mut travel, "B", "g-b");
        let c = place(&mut travel, "C", "g-c");

        let found = hops(
            &travel,
            &[stop("a", Some(a)), stop("b", Some(b)), stop("c", Some(c))],
        );
        let legs: Vec<_> = found.iter().map(|hop| hop.leg).collect();

        assert_eq!(
            legs,
            [Leg { from: a, to: b }, Leg { from: b, to: c }],
            "A to B and B to C, never A to C"
        );
        assert_eq!(found[0].before.as_deref(), Some("b"));
        assert_eq!(found[1].before.as_deref(), Some("c"));
    }

    #[test]
    fn staying_put_and_not_knowing_where_make_no_trip() {
        let mut travel = TravelSettings::default();
        let a = place(&mut travel, "A", "g-a");
        let b = place(&mut travel, "B", "g-b");

        let same = hops(&travel, &[stop("1", Some(a)), stop("2", Some(a))]);
        assert!(same.is_empty());

        // The event in the middle has no place, so neither side of it is a trip,
        // and the two either side are not joined across it.
        let gap = hops(
            &travel,
            &[stop("1", Some(a)), stop("2", None), stop("3", Some(b))],
        );
        assert!(gap.is_empty());
    }

    #[test]
    fn the_day_starts_and_ends_at_the_default_places() {
        let mut travel = TravelSettings::default();
        let home = place(&mut travel, "Home", "g-home");
        let work = place(&mut travel, "Work", "g-work");
        let gym = place(&mut travel, "Gym", "g-gym");
        travel.home = Some(home);

        let found = hops(&travel, &[stop("w", Some(work)), stop("g", Some(gym))]);

        assert_eq!(found.len(), 3);
        assert_eq!(
            found[0].leg,
            Leg {
                from: home,
                to: work
            }
        );
        assert_eq!(found[0].before.as_deref(), Some("w"));
        assert_eq!(found[0].from_title, START_OF_DAY);
        assert_eq!(
            found[1].leg,
            Leg {
                from: work,
                to: gym
            }
        );
        assert_eq!(
            found[2].leg,
            Leg {
                from: gym,
                to: home
            }
        );
        assert_eq!(found[2].before, None);
        assert_eq!(found[2].after.as_deref(), Some("g"));
        assert_eq!(found[2].to_title, END_OF_DAY);

        // An event at home needs no trip to or from it.
        assert!(hops(&travel, &[stop("h", Some(home))]).is_empty());
        assert!(hops(&travel, &[]).is_empty());
    }

    #[test]
    fn a_default_place_that_was_removed_makes_no_trip() {
        let mut travel = TravelSettings::default();
        let work = place(&mut travel, "Work", "g-work");
        travel.home = Some(12345);

        assert!(hops(&travel, &[stop("w", Some(work))]).is_empty());
    }

    #[test]
    fn a_calendar_event_is_matched_to_a_place_by_its_location_text() {
        let mut travel = TravelSettings::default();
        let work = place(&mut travel, "Work", "g-work");

        let mut event = crate::calendar::sample_snapshot().events.remove(0);
        event.id = "event-1".into();
        event.calendar_id = "primary".into();

        for text in ["work", "  Work  ", "Work, 1 Main St", "WORK,   1 main st"] {
            event.details.location = text.into();
            assert_eq!(place_of(&travel, &event), Some(work), "{text:?}");
        }

        event.details.location = "Elsewhere".into();
        assert_eq!(place_of(&travel, &event), None);
        event.details.location = String::new();
        assert_eq!(place_of(&travel, &event), None);
    }

    #[test]
    fn a_calendar_event_given_a_place_keeps_it_whatever_google_says() {
        let mut state = State::new(
            crate::config::Config::default(),
            crate::theme::Catalog::built_in(),
        );
        state.calendar = crate::calendar::Store::with_snapshot(crate::calendar::sample_snapshot());
        let work = place(&mut state.config.travel, "Work", "g-work");
        state.config.travel.places[0].address = "1 Main St".into();

        let _ = update(
            &mut state,
            Action::OpenPicker(PickTarget::Event("standup".into())),
        );
        state.travel.picker.as_mut().unwrap().results = vec![Found {
            place_id: "g-work".into(),
            name: "Work".into(),
            address: "1 Main St".into(),
            lat: 0.0,
            lng: 0.0,
        }];
        let _ = update(&mut state, Action::Choose(0));

        // The place is remembered and the event's own location reads like it.
        assert_eq!(state.config.travel.event_places.get("standup"), Some(&work));
        let event = state.calendar.event("standup").unwrap();
        assert_eq!(event.details.location, "Work, 1 Main St");
        assert_eq!(place_of(&state.config.travel, event), Some(work));

        // A location typed in Google later does not undo the choice.
        state.calendar.set_location("standup", "Room 4");
        let event = state.calendar.event("standup").unwrap();
        assert_eq!(place_of(&state.config.travel, event), Some(work));

        // Clearing removes both.
        let _ = update(
            &mut state,
            Action::Clear(PickTarget::Event("standup".into())),
        );
        assert!(state.config.travel.event_places.is_empty());
        let event = state.calendar.event("standup").unwrap();
        assert_eq!(event.details.location, "");
        assert_eq!(place_of(&state.config.travel, event), None);

        // Only a choice made here is written down: an untouched file stays as it was.
        assert!(
            !toml::to_string(&state.config.travel)
                .unwrap()
                .contains("event_places")
        );
    }

    #[test]
    fn a_task_has_the_place_it_was_given() {
        let mut travel = TravelSettings::default();
        let gym = place(&mut travel, "Gym", "g-gym");

        let mut task = crate::calendar::sample_snapshot().events.remove(0);
        task.id = "task:1".into();
        task.calendar_id = "tasks:list".into();
        task.details.task_data = Some(crate::task_db::TaskData {
            id: 100,
            metadata: tb::Metadata {
                location: Some(gym),
                ..Default::default()
            },
            legacy_key: None,
        });
        assert_eq!(place_of(&travel, &task), Some(gym));

        // A place that has gone reads as no place.
        task.details.task_data.as_mut().unwrap().metadata.location = Some(7);
        assert_eq!(place_of(&travel, &task), None);
    }

    #[test]
    fn choosing_the_same_mapbox_place_twice_keeps_one_place_and_its_title() {
        let mut travel = TravelSettings::default();
        let first = travel.add_place("Cafe", "1 Main St", "g-cafe", 1.0, 2.0);
        travel.places[0].name = "The usual".into();
        let second = travel.add_place("Cafe", "1 Main St", "g-cafe", 1.0, 2.0);

        assert_eq!(first, second);
        assert_eq!(travel.places.len(), 1);
        assert_eq!(travel.places[0].name, "The usual");
        assert_eq!(travel.places[0].place_id, "g-cafe");
    }

    #[test]
    fn the_method_is_the_default_until_a_trip_is_set() {
        let mut travel = TravelSettings::default();
        assert_eq!(travel.mode_for(1, 2), TravelMode::Walk);

        travel.default_mode = TravelMode::Car;
        assert_eq!(travel.mode_for(1, 2), TravelMode::Car);

        travel.set_mode(1, 2, TravelMode::Bike);
        travel.set_mode(1, 2, TravelMode::Walk);
        assert_eq!(travel.mode_for(1, 2), TravelMode::Walk);
        assert_eq!(
            travel.mode_for(2, 1),
            TravelMode::Car,
            "the way back is its own trip"
        );
        assert_eq!(travel.modes.len(), 1);
    }

    #[test]
    fn travel_settings_survive_the_config_file_and_stay_out_of_an_untouched_one() {
        let untouched = toml::to_string_pretty(&Config::default()).unwrap();
        assert!(!untouched.contains("travel"), "{untouched}");

        let mut config = Config::default();
        let home = place(&mut config.travel, "Home", "g-home");
        let work = place(&mut config.travel, "Work", "g-work");
        config.travel.api_key = "key".into();
        config.travel.home = Some(home);
        config.travel.default_mode = TravelMode::Car;
        config.travel.set_mode(home, work, TravelMode::Bike);

        let text = toml::to_string_pretty(&config).unwrap();
        let read: Config = toml::from_str(&text).unwrap();
        assert_eq!(read.travel, config.travel);
    }

    #[test]
    fn places_are_read_out_of_a_search_response() {
        let response = json!({
            "type": "FeatureCollection",
            "features": [
                {
                    "type": "Feature",
                    "geometry": { "type": "Point", "coordinates": [-122.40, 37.78] },
                    "properties": {
                        "mapbox_id": "dXJu1",
                        "name": "Blue Bottle",
                        "full_address": "66 Mint St, San Francisco, California 94103, United States",
                        "place_formatted": "San Francisco, California 94103, United States"
                    }
                },
                {
                    "type": "Feature",
                    "geometry": { "type": "Point", "coordinates": [-122.27, 37.80] },
                    "properties": {
                        "mapbox_id": "dXJu2",
                        "name": "",
                        "full_address": "12 Pine Rd, Oakland, California, United States"
                    }
                },
                {
                    "type": "Feature",
                    "geometry": { "type": "Point", "coordinates": [-122.0, 37.0] },
                    "properties": { "name": "No id" }
                },
                {
                    "type": "Feature",
                    "properties": {
                        "mapbox_id": "no-location",
                        "name": "Nowhere"
                    }
                }
            ]
        });

        let places = parse_places(&response);
        assert_eq!(
            places.len(),
            2,
            "a place without an id or location is skipped"
        );
        assert_eq!(places[0].name, "Blue Bottle");
        assert_eq!(places[0].place_id, "dXJu1");
        assert_eq!(
            places[0].address,
            "66 Mint St, San Francisco, California 94103, United States"
        );
        assert_eq!(
            (places[0].lat, places[0].lng),
            (37.78, -122.40),
            "GeoJSON lists longitude first"
        );
        assert_eq!(
            places[1].name, "12 Pine Rd",
            "no title falls back to the street"
        );

        assert!(
            parse_places(&json!({})).is_empty(),
            "no match is an empty body"
        );
    }

    #[test]
    fn a_route_response_gives_seconds_and_meters() {
        let route = parse_route(&json!({
            "code": "Ok",
            "routes": [{ "duration": 753.6, "distance": 1233.8 }]
        }))
        .unwrap();
        assert_eq!(
            route,
            Route {
                seconds: 754,
                meters: 1234
            }
        );

        let none = parse_route(&json!({ "routes": [{ "duration": 0.0 }] })).unwrap();
        assert_eq!(
            none,
            Route {
                seconds: 0,
                meters: 0
            }
        );

        assert!(parse_route(&json!({})).is_err());
        assert!(parse_route(&json!({ "routes": [] })).is_err());
        assert!(parse_route(&json!({ "routes": [{ "distance": 5.0 }] })).is_err());
    }

    #[test]
    fn a_route_request_names_the_places_and_the_method() {
        let mut travel = TravelSettings::default();
        let a = travel.add_place("A", "", "m-a", 1.5, 2.5);
        let b = travel.add_place("B", "", "m-b", 3.5, 4.5);

        let url = route_url(
            "pk.key",
            travel.place(a).unwrap(),
            travel.place(b).unwrap(),
            TravelMode::Car,
        );
        assert_eq!(
            url,
            "https://api.mapbox.com/directions/v5/mapbox/driving/2.5,1.5;4.5,3.5\
             ?overview=false&access_token=pk.key",
            "coordinates are longitude then latitude"
        );

        for (mode, profile) in [
            (TravelMode::Walk, "walking"),
            (TravelMode::Bike, "cycling"),
            (TravelMode::Car, "driving"),
        ] {
            let url = route_url(
                "k",
                travel.place(a).unwrap(),
                travel.place(b).unwrap(),
                mode,
            );
            assert!(url.contains(&format!("/mapbox/{profile}/")), "{url}");
        }
    }

    #[test]
    fn the_map_choice_is_kept_in_the_config_only_when_changed() {
        let untouched = toml::to_string_pretty(&TravelSettings::default()).unwrap();
        assert!(untouched.contains("map_app = \"OpenStreetMap\""));

        let mut config = Config::default();
        config.travel.map_app = MapApp::GoogleMaps;
        let text = toml::to_string_pretty(&config).unwrap();
        let read: Config = toml::from_str(&text).unwrap();
        assert_eq!(read.travel.map_app, MapApp::GoogleMaps);

        let old: TravelSettings = toml::from_str("api_key = \"k\"").unwrap();
        assert_eq!(old.map_app, MapApp::OpenStreetMap);
    }

    #[test]
    fn home_migrates_legacy_endpoints_and_saves_only_home() {
        for (input, expected) in [
            ("start = 1\nend = 2", Some(1)),
            ("end = 2", Some(2)),
            ("home = 3\nstart = 1\nend = 2", Some(3)),
            ("", None),
        ] {
            let read: TravelSettings = toml::from_str(input).unwrap();
            assert_eq!(read.home, expected);
            let saved = toml::to_string(&read).unwrap();
            assert!(!saved.contains("start ="));
            assert!(!saved.contains("end ="));
            let again: TravelSettings = toml::from_str(&saved).unwrap();
            assert_eq!(again, read);
        }
    }

    #[test]
    fn search_biases_near_home_with_longitude_first_and_escapes_input() {
        let url = search_url("key&value", "coffee & tea", Some((-122.4, 37.8)));
        let parsed = url::Url::parse(&url).unwrap();
        let params: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(params["q"], "coffee & tea");
        assert_eq!(params["access_token"], "key&value");
        assert_eq!(params["proximity"], "-122.4,37.8");
        assert!(
            !params.contains_key("bbox"),
            "distant places remain searchable"
        );
        for proximity in [
            None,
            Some((f64::NAN, 0.0)),
            Some((0.0, 91.0)),
            Some((181.0, 0.0)),
        ] {
            assert!(!search_url("key", "coffee", proximity).contains("proximity="));
        }
    }

    #[test]
    fn a_config_from_before_mapbox_still_reads() {
        // Public transport used to be a method; it is a drive now.
        let read: TravelSettings = toml::from_str("default_mode = \"multimodal\"").unwrap();
        assert_eq!(read.default_mode, TravelMode::Car);
    }

    #[test]
    fn trips_read_in_minutes_and_kilometers() {
        assert_eq!(duration_label(0), "1 min");
        assert_eq!(duration_label(61), "2 min");
        assert_eq!(duration_label(754), "13 min");
        assert_eq!(duration_label(3900), "1 h 05 min");
        assert_eq!(distance_label(850), "850 m");
        assert_eq!(distance_label(1234), "1.2 km");
        assert_eq!(
            route_label(Route {
                seconds: 600,
                meters: 2000
            }),
            "10 min · 2.0 km"
        );
    }

    #[test]
    fn a_search_needs_a_key_and_something_to_look_for() {
        let mut state = state();
        let _ = update(&mut state, Action::OpenPicker(PickTarget::Home));

        let _ = update(&mut state, Action::Search);
        let picker = state.travel.picker.as_ref().unwrap();
        assert!(!picker.searching && picker.error.is_none(), "nothing typed");

        let _ = update(&mut state, Action::Query("coffee".into()));
        // Keep the environment's own key out of the test.
        if state.config.travel.key().is_none() {
            let _ = update(&mut state, Action::Search);
            let picker = state.travel.picker.as_ref().unwrap();
            assert_eq!(picker.error.as_deref(), Some(NO_KEY));
            assert!(!picker.searching);
        }
    }

    #[test]
    fn a_late_answer_to_an_old_search_is_ignored() {
        let mut state = state();
        let _ = update(&mut state, Action::OpenPicker(PickTarget::Home));
        state.travel.serial = 5;

        let _ = update(&mut state, Action::Searched(4, Ok(vec![found("Old")])));
        assert!(state.travel.picker.as_ref().unwrap().results.is_empty());

        let _ = update(&mut state, Action::Searched(5, Ok(vec![found("New")])));
        let picker = state.travel.picker.as_ref().unwrap();
        assert_eq!(picker.results.len(), 1);
        assert!(picker.searched);

        let _ = update(&mut state, Action::Searched(5, Err("Quota".into())));
        let picker = state.travel.picker.as_ref().unwrap();
        assert!(picker.results.is_empty());
        assert_eq!(picker.error.as_deref(), Some("Quota"));
    }

    #[test]
    fn choosing_a_result_sets_and_clears_home() {
        let mut state = state();

        for _ in 0..2 {
            let _ = update(&mut state, Action::OpenPicker(PickTarget::Home));
            state.travel.picker.as_mut().unwrap().results = vec![found("Home")];
            let _ = update(&mut state, Action::Choose(0));

            assert!(state.travel.picker.is_none(), "choosing closes the search");
            let id = state.config.travel.home.expect("Home is set");
            assert_eq!(state.config.travel.place(id).unwrap().name, "Home");

            let _ = update(&mut state, Action::Clear(PickTarget::Home));
            assert_eq!(state.config.travel.home, None);
        }
        assert_eq!(
            state.config.travel.places.len(),
            1,
            "one place, chosen twice"
        );
    }

    #[test]
    fn renaming_a_place_keeps_what_mapbox_needs() {
        let mut state = state();
        let id = state
            .config
            .travel
            .add_place("Starbucks", "1 Main St", "g-sbux", 1.0, 2.0);

        let _ = update(&mut state, Action::Rename(id));
        let _ = update(&mut state, Action::RenameText("   ".into()));
        let _ = update(&mut state, Action::SaveRename);
        assert!(state.travel.rename.is_some(), "a blank title is refused");
        assert_eq!(state.config.travel.place(id).unwrap().name, "Starbucks");

        let _ = update(&mut state, Action::RenameText(" Study spot ".into()));
        let _ = update(&mut state, Action::SaveRename);

        assert!(state.travel.rename.is_none());
        let place = state.config.travel.place(id).unwrap();
        assert_eq!(place.name, "Study spot");
        assert_eq!(place.address, "1 Main St");
        assert_eq!(place.place_id, "g-sbux");
        assert_eq!((place.lat, place.lng), (1.0, 2.0));
    }

    #[test]
    fn a_finished_trip_is_kept_and_a_failed_one_is_tried_again_later() {
        let mut state = state();
        let key = RouteKey {
            from: 1,
            to: 2,
            mode: TravelMode::Walk,
        };
        let leg = Leg { from: 1, to: 2 };

        let _ = update(&mut state, Action::Routed(key, Err("Quota".into())));
        assert_eq!(summary(&state, leg), Summary::Failed("Quota".into()));

        let _ = update(&mut state, Action::Toggle);
        assert!(state.travel.show);
        assert_ne!(summary(&state, leg), Summary::Failed("Quota".into()));

        let route = Route {
            seconds: 300,
            meters: 400,
        };
        let _ = update(&mut state, Action::Routed(key, Ok(route)));
        assert_eq!(summary(&state, leg), Summary::Ready(route));
        assert_eq!(short_label(&state, leg), "Walk · 5 min · 400 m");

        // Another method is another trip.
        state.config.travel.set_mode(1, 2, TravelMode::Car);
        assert_ne!(summary(&state, leg), Summary::Ready(route));
    }

    #[test]
    fn worked_out_trips_come_back_after_a_restart() {
        let dir = std::env::temp_dir().join(format!("tt-travel-{}", std::process::id()));
        let path = dir.join("routes.json");
        let key = RouteKey {
            from: 1,
            to: 2,
            mode: TravelMode::Bike,
        };
        let route = Route {
            seconds: 90,
            meters: 300,
        };

        write_cache(&path, &[(key, route)]).unwrap();

        let mut travel = Workspace::default();
        travel.cache_in(Some(path.clone()));
        assert_eq!(travel.state(key), Some(&RouteState::Ready(route)));

        std::fs::write(&path, "not json").unwrap();
        let mut travel = Workspace::default();
        travel.cache_in(Some(path));
        assert!(travel.routes.is_empty(), "a damaged cache is just empty");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_maps_link_names_both_places_and_the_method() {
        let mut travel = TravelSettings::default();
        let a = travel.add_place("A", "", "m-a", 1.5, 2.5);
        let b = travel.add_place("B", "", "m-b", 3.5, 4.5);
        travel.set_mode(a, b, TravelMode::Car);

        let url = maps_url(&travel, Leg { from: a, to: b }).unwrap();
        assert_eq!(
            url,
            "https://www.openstreetmap.org/directions?engine=fossgis_osrm_car&route=1.5%2C2.5%3B3.5%2C4.5"
        );
        assert!(maps_url(&travel, Leg { from: a, to: 99 }).is_none());

        travel.map_app = MapApp::GoogleMaps;
        assert_eq!(
            maps_url(&travel, Leg { from: a, to: b }).unwrap(),
            "https://www.google.com/maps/dir/?api=1&origin=1.5%2C2.5&destination=3.5%2C4.5&travelmode=driving"
        );
        travel.set_mode(a, b, TravelMode::Bike);
        assert!(
            maps_url(&travel, Leg { from: a, to: b })
                .unwrap()
                .ends_with("travelmode=bicycling")
        );
    }
}
