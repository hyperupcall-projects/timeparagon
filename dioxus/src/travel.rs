//! The Travel sheet, opened from More: the saved places, whose titles can be
//! changed, and the trips between them, first those of the day in view and then
//! every trip of the days loaded so far. It offers what the desktop's Travel
//! tab does, and trips are worked out with the same Mapbox requests.

use std::collections::HashMap;

use chrono::NaiveDate;
use dioxus::prelude::*;
use timeparagon_core::calendar::{self, Snapshot};
use timeparagon_core::config::{self as saved_config, Config};
use timeparagon_core::places::{self, Leg, Place, PlaceSettings, Route, SavedRoute, TravelMode};

use crate::day;
use crate::editor::worker;

/// What is shown when a trip needs Mapbox and there is no token to ask with.
const NO_KEY: &str = "Add a Mapbox access token in the desktop app's Settings first.";

/// The trips the sheet lists.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Connections {
	/// The different trips of the day in view, in the order it makes them.
	pub day: Vec<Leg>,
	/// The different trips of every loaded day, earliest day first.
	pub all: Vec<Leg>,
}

/// Trips in the currently loaded day snapshot.
pub fn connections(view: NaiveDate, snapshot: Option<&Snapshot>, config: &Config) -> Connections {
	let hops = snapshot
		.filter(|snapshot| snapshot.day == view)
		.map(|snapshot| {
			day::day_hops(
				&day::build_model(snapshot, &config.calendar),
				&config.travel,
			)
		})
		.unwrap_or_default();
	let day = places::unique_legs(hops);
	Connections {
		all: day.clone(),
		day,
	}
}

/// A trip from one place to another by one method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct RouteKey {
	from: u64,
	to: u64,
	mode: TravelMode,
}

impl RouteKey {
	fn of(travel: &PlaceSettings, leg: Leg) -> Self {
		Self {
			from: leg.from,
			to: leg.to,
			mode: travel.mode_for(leg.from, leg.to),
		}
	}
}

/// A question put to Mapbox that has no saved answer to show.
#[derive(Debug, Clone, PartialEq)]
enum Asked {
	Loading,
	Failed(String),
}

/// What is known about a trip.
#[derive(Debug, Clone, PartialEq)]
enum Summary {
	/// There is no token to ask Mapbox with.
	NoKey,
	/// Nobody has asked yet.
	Unknown,
	Loading,
	Failed(String),
	Ready(Route),
}

/// A question in progress or refused speaks over a saved answer, so pressing
/// Recalculate shows what became of it.
fn summary(travel: &PlaceSettings, asked: &HashMap<RouteKey, Asked>, leg: Leg) -> Summary {
	match asked.get(&RouteKey::of(travel, leg)) {
		Some(Asked::Loading) => Summary::Loading,
		Some(Asked::Failed(error)) => Summary::Failed(error.clone()),
		None => match travel.route(leg) {
			Some(route) => Summary::Ready(route),
			None if travel.key().is_none() => Summary::NoKey,
			None => Summary::Unknown,
		},
	}
}

/// The map page for a trip, while both of its places are saved.
fn trip_url(travel: &PlaceSettings, leg: Leg) -> Option<String> {
	Some(places::maps_url(
		travel.map_app,
		travel.mode_for(leg.from, leg.to),
		travel.place(leg.from)?,
		travel.place(leg.to)?,
	))
}

/// The sheet's state. Handlers using it are written in [`TravelSheet`]'s own
/// scope, so a request survives the trip card that started it being closed.
#[derive(Clone, Copy, PartialEq)]
struct Sheet {
	config: Signal<Config>,
	asked: Signal<HashMap<RouteKey, Asked>>,
	/// The trip whose card is open.
	open: Signal<Option<Leg>>,
	/// The place being renamed and the title typed so far.
	rename: Signal<Option<(u64, String)>>,
	error: Signal<Option<String>>,
}

impl Sheet {
	/// Asks Mapbox for a trip it has not been asked about, or, with `again`,
	/// for a new answer. The answer is saved on the worker, so it is kept even
	/// if the sheet closes while it is on its way.
	fn ask(self, leg: Leg, again: bool) {
		let Self {
			mut config,
			mut asked,
			..
		} = self;
		let travel = config.peek().travel.clone();
		let Some(token) = travel.key() else {
			return;
		};
		let key = RouteKey::of(&travel, leg);
		let known = asked.peek().get(&key).cloned();
		if known == Some(Asked::Loading)
			|| (!again && (known.is_some() || travel.route(leg).is_some()))
		{
			return;
		}
		let (Some(from), Some(to)) = (
			travel.place(leg.from).cloned(),
			travel.place(leg.to).cloned(),
		) else {
			return;
		};

		asked.write().insert(key, Asked::Loading);
		spawn(async move {
			let result = worker(move || {
				let route = places::compute_route(&token, &from, &to, key.mode)?;
				let saved = SavedRoute {
					from: key.from,
					to: key.to,
					mode: key.mode,
					seconds: route.seconds,
					meters: route.meters,
				};
				saved_config::save_travel_route(saved)
					.map_err(|error| format!("Could not save the trip: {error}"))?;
				Ok(saved)
			})
			.await;
			match result {
				Ok(saved) => {
					config.write().travel.set_route(saved);
					asked.write().remove(&key);
				}
				Err(message) => {
					asked.write().insert(key, Asked::Failed(message));
				}
			}
		});
	}

	fn set_mode(self, leg: Leg, mode: TravelMode) {
		let Self {
			mut config,
			mut error,
			..
		} = self;
		match saved_config::save_travel_mode(leg, mode) {
			Ok(()) => {
				config.write().travel.set_mode(leg.from, leg.to, mode);
				error.set(None);
				self.ask(leg, false);
			}
			Err(message) => error.set(Some(format!("Could not save the travel method: {message}"))),
		}
	}

	/// Saves the title being typed. A place without a title would be a blank
	/// row, so the edit stays open instead.
	fn save_rename(self) {
		let Self {
			mut config,
			mut rename,
			mut error,
			..
		} = self;
		let Some((id, name)) = rename.peek().clone() else {
			return;
		};
		let name = name.trim().to_owned();
		if name.is_empty() {
			return;
		}
		match saved_config::save_place_name(id, &name) {
			Ok(()) => {
				if let Some(place) = config
					.write()
					.travel
					.places
					.iter_mut()
					.find(|place| place.id == id)
				{
					place.name = name;
				}
				rename.set(None);
				error.set(None);
			}
			Err(message) => error.set(Some(format!("Could not save the title: {message}"))),
		}
	}

	fn open_in_maps(self, leg: Leg) {
		let mut error = self.error;
		let Some(url) = trip_url(&self.config.peek().travel, leg) else {
			return;
		};
		spawn(async move {
			if let Err(message) = worker(move || calendar::oauth::open_in_browser(&url)).await {
				error.set(Some(message));
			}
		});
	}
}

#[component]
pub fn TravelSheet(
	config: Signal<Config>,
	day: NaiveDate,
	connections: Connections,
	on_close: EventHandler<()>,
) -> Element {
	// The complete trips list reads persisted events, rather than depending on
	// which days the user happened to leave in a UI cache.
	let saved_trips = use_resource(move || {
		let config = config();
		async move {
			worker(move || {
				let path = timeparagon_core::task_db::path().ok_or("Cannot locate database")?;
				let events = timeparagon_core::calendar::repository::all_events(&path)?;
				let days: std::collections::BTreeSet<_> = events
					.iter()
					.filter(|event| !event.all_day)
					.map(|event| {
						timeparagon_core::calendar::day_at(
							event.start,
							config.calendar.day_start_hour(),
						)
					})
					.collect();
				let mut snapshot = Snapshot {
					revision: 0,
					calendars: Vec::new(),
					events,
					day,
					fetched_at: timeparagon_core::clock::now(),
					notice: None,
				};
				let mut legs = Vec::new();
				for day in days {
					snapshot.day = day;
					legs.extend(day::day_hops(
						&day::build_model(&snapshot, &config.calendar),
						&config.travel,
					));
				}
				Ok(places::unique_legs(legs))
			})
			.await
		}
	});
	let sheet = Sheet {
		config,
		asked: use_signal(HashMap::new),
		open: use_signal(|| None),
		rename: use_signal(|| None),
		error: use_signal(|| None),
	};

	// As on the desktop, the day's trips are worked out while travel is shown,
	// and any other trip once its card is opened.
	use_effect(use_reactive((&connections.day,), move |(day_legs,)| {
		let shown = config.read().travel.show;
		let open = (sheet.open)();
		let legs = day_legs.into_iter().filter(|_| shown).chain(open);
		for leg in legs {
			sheet.ask(leg, false);
		}
	}));

	let travel = config.read().travel.clone();
	let asked = (sheet.asked)();
	let rename = (sheet.rename)();
	let error = (sheet.error)();
	let unit = config.read().appearance.distance_unit;
	let trip_rows = |legs: &[Leg]| -> Vec<Element> {
		legs.iter()
			.filter_map(|leg| trip_row(sheet, &travel, &asked, *leg, unit))
			.collect()
	};
	let day_rows = trip_rows(&connections.day);
	let saved_trips = saved_trips.read();
	let all_legs = saved_trips
		.as_ref()
		.and_then(|result| result.as_ref().ok())
		.map(|legs| {
			let mut all = connections.all.clone();
			for leg in legs {
				if !all.contains(leg) {
					all.push(*leg);
				}
			}
			all
		})
		.unwrap_or_else(|| connections.all.clone());
	let all_rows = trip_rows(&all_legs);

	rsx! {
		div { class: "modal-backdrop", onclick: move |_| on_close.call(()),
			div {
				class: "modal",
				role: "dialog",
				aria_label: "Travel",
				onclick: move |event| event.stop_propagation(),
				div { class: "modal-head",
					div { class: "modal-titles",
						h2 { class: "modal-title", "Travel" }
						p { class: "modal-sub", "Places and routes between your events" }
					}
					button {
						class: "modal-close",
						title: "Close",
						aria_label: "Close",
						onclick: move |_| on_close.call(()),
						"×"
					}
				}
				div { class: "modal-body travel-body",
					if let Some(message) = error.clone() {
						p { class: "error-text", role: "alert", "{message}" }
					}
					section { class: "travel-section",
						h3 { class: "travel-heading",
							"Locations"
							span { class: "travel-count", "{travel.places.len()}" }
						}
						for place in travel.sorted() {
							{place_row(sheet, place, travel.home == Some(place.id), rename.as_ref())}
						}
						if travel.places.is_empty() {
							p { class: "empty-note",
								"No locations yet. Add one to a task, or choose Home in the desktop app's Settings."
							}
						}
					}
					section { class: "travel-section",
						h3 { class: "travel-heading", "Day connections" }
						p { class: "hint travel-day", "{day.format(\"%A, %B %-d\")}" }
						{day_rows.into_iter()}
						if connections.day.is_empty() {
							p { class: "empty-note",
								"No trips this day. Back-to-back events with locations show up here."
							}
						}
					}
					section { class: "travel-section",
						h3 { class: "travel-heading", "All connections" }
						{all_rows.into_iter()}
						if all_legs.is_empty() {
							p { class: "empty-note", "No trips yet." }
						}
					}
				}
			}
		}
		// Beside the sheet rather than in it, so a tap on this backdrop closes
		// only the card.
		if let Some(leg) = (sheet.open)() {
			{trip_card(sheet, &travel, &asked, leg, unit, error.as_deref())}
		}
	}
}

/// A saved place, or the field that changes its title.
fn place_row(sheet: Sheet, place: &Place, home: bool, rename: Option<&(u64, String)>) -> Element {
	let id = place.id;
	let mut renaming = sheet.rename;
	if let Some((_, name)) = rename.filter(|(renamed, _)| *renamed == id) {
		let blank = name.trim().is_empty();
		return rsx! {
			form {
				key: "{id}",
				class: "travel-card travel-rename",
				onsubmit: move |event| {
					event.prevent_default();
					sheet.save_rename();
				},
				label { class: "hint", r#for: "travel-rename", "Title" }
				input {
					id: "travel-rename",
					class: "client-input",
					r#type: "text",
					value: "{name}",
					oninput: move |event| renaming.set(Some((id, event.value()))),
				}
				div { class: "travel-actions",
					button { class: "btn primary", r#type: "submit", disabled: blank, "Save" }
					button {
						class: "btn",
						r#type: "button",
						onclick: move |_| renaming.set(None),
						"Cancel"
					}
				}
			}
		};
	}

	let title = place.name.clone();
	rsx! {
		div { key: "{id}", class: "travel-card travel-place",
			div { class: "travel-text",
				span { class: "travel-name", "{place.name}" }
				if !place.address.is_empty() && place.address != place.name {
					span { class: "hint", "{place.address}" }
				}
				if home {
					span { class: "hint", "Home" }
				}
			}
			button {
				class: "btn small",
				aria_label: "Rename {place.name}",
				onclick: move |_| renaming.set(Some((id, title.clone()))),
				"Rename"
			}
		}
	}
}

/// One trip: where it starts and ends, with the way it is made at the right.
/// `None` once one of its places has been removed.
fn trip_row(
	sheet: Sheet,
	travel: &PlaceSettings,
	asked: &HashMap<RouteKey, Asked>,
	leg: Leg,
	unit: places::DistanceUnit,
) -> Option<Element> {
	let from = travel.place(leg.from)?;
	let to = travel.place(leg.to)?;
	let current = travel.mode_for(leg.from, leg.to);
	let detail = match summary(travel, asked, leg) {
		Summary::Ready(route) => Some(places::route_label(route, unit)),
		Summary::Loading => Some("Calculating…".to_owned()),
		Summary::Failed(_) => Some("No route".to_owned()),
		Summary::Unknown | Summary::NoKey => None,
	};
	let mut open = sheet.open;
	let mut error = sheet.error;

	Some(rsx! {
		div { key: "{leg.from}-{leg.to}", class: "travel-card travel-trip",
			button {
				class: "travel-open",
				onclick: move |_| {
					error.set(None);
					open.set(Some(leg));
				},
				span { class: "travel-name", "{from.name}" }
				span { class: "travel-name", "→ {to.name}" }
				if let Some(detail) = detail {
					span { class: "hint", "{detail}" }
				}
			}
			select {
				class: "client-input travel-mode",
				aria_label: "Travel method from {from.name} to {to.name}",
				onchange: move |event: FormEvent| {
					if let Some(mode) = TravelMode::ALL
						.into_iter()
						.find(|mode| mode.label() == event.value())
					{
						sheet.set_mode(leg, mode);
					}
				},
				for mode in TravelMode::ALL {
					option {
						key: "{mode}",
						value: "{mode}",
						selected: mode == current,
						"{mode}"
					}
				}
			}
		}
	})
}

/// The card for one trip: where it goes, how long it takes and how it is made.
fn trip_card(
	sheet: Sheet,
	travel: &PlaceSettings,
	asked: &HashMap<RouteKey, Asked>,
	leg: Leg,
	unit: places::DistanceUnit,
	error: Option<&str>,
) -> Element {
	let mut open = sheet.open;
	let places = travel.place(leg.from).zip(travel.place(leg.to));
	let current = travel.mode_for(leg.from, leg.to);
	let summary = summary(travel, asked, leg);
	let can_recalculate = travel.key().is_some() && summary != Summary::Loading;

	let place = |place: &Place| {
		rsx! {
			div { class: "travel-text",
				span { class: "travel-name", "{place.name}" }
				if !place.address.is_empty() && place.address != place.name {
					span { class: "hint", "{place.address}" }
				}
			}
		}
	};

	rsx! {
		div { class: "modal-backdrop", onclick: move |_| open.set(None),
			div {
				class: "modal",
				role: "dialog",
				aria_label: "Trip",
				onclick: move |event| event.stop_propagation(),
				div { class: "modal-head",
					h2 { class: "modal-title", "Travel" }
					button {
						class: "modal-close",
						title: "Close",
						aria_label: "Close",
						onclick: move |_| open.set(None),
						"×"
					}
				}
				div { class: "modal-body travel-body",
					if let Some(message) = error {
						p { class: "error-text", role: "alert", "{message}" }
					}
					if let Some((from, to)) = places {
						div { class: "travel-field",
							span { class: "hint", "From" }
							{place(from)}
						}
						div { class: "travel-field",
							span { class: "hint", "To" }
							{place(to)}
						}
						div { class: "travel-field",
							span { class: "hint", "Travel method" }
							div { class: "travel-actions",
								for mode in TravelMode::ALL {
									button {
										key: "{mode}",
										class: if mode == current { "btn primary" } else { "btn" },
										aria_pressed: (mode == current).to_string(),
										onclick: move |_| sheet.set_mode(leg, mode),
										"{mode}"
									}
								}
							}
						}
						div { class: "travel-field",
							span { class: "hint", "Travel time" }
							match summary {
								Summary::Ready(route) => rsx! {
									span { class: "travel-time", "{places::route_label(route, unit)}" }
								},
								Summary::Loading => rsx! { span { "Calculating…" } },
								Summary::Unknown => rsx! { span { "Not calculated yet" } },
								Summary::NoKey => rsx! { p { class: "error-text", "{NO_KEY}" } },
								Summary::Failed(message) => rsx! { p { class: "error-text", "{message}" } },
							}
						}
						div { class: "travel-actions",
							button {
								class: "btn",
								disabled: !can_recalculate,
								onclick: move |_| sheet.ask(leg, true),
								"Recalculate"
							}
							button {
								class: "btn",
								onclick: move |_| sheet.open_in_maps(leg),
								"Open in Maps"
							}
						}
					} else {
						p { class: "empty-note", "This trip is no longer available" }
					}
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use chrono::{Local, TimeZone};
	use timeparagon_core::calendar::{Details, Event};

	const HOME: u64 = 1;
	const OFFICE: u64 = 2;
	const CAFE: u64 = 3;

	fn place(id: u64, name: &str, address: &str) -> Place {
		Place {
			id,
			name: name.to_owned(),
			address: address.to_owned(),
			place_id: format!("mapbox-{id}"),
			lat: id as f64,
			lng: id as f64 + 0.5,
			favorite: false,
		}
	}

	fn config() -> Config {
		let mut config = Config::default();
		config.travel.api_key = "pk.test".into();
		config.travel.home = Some(HOME);
		config.travel.places = vec![
			place(HOME, "Home", "12 Elm St"),
			place(OFFICE, "Office", "500 Market St"),
			place(CAFE, "Cafe", "Cafe"),
		];
		config
	}

	fn view() -> NaiveDate {
		NaiveDate::from_ymd_opt(2026, 11, 17).unwrap()
	}

	fn event(id: &str, day: NaiveDate, hour: u32, location: &str) -> Event {
		let at = |hour| {
			Local
				.from_local_datetime(&day.and_hms_opt(hour, 0, 0).unwrap())
				.unwrap()
		};
		Event {
			id: id.to_owned(),
			calendar_id: "work".to_owned(),
			title: id.to_owned(),
			start: at(hour),
			end: at(hour + 1),
			all_day: false,
			color: calendar::colors::DEFAULT_EVENT,
			completed: false,
			details: Details {
				location: location.to_owned(),
				..Details::default()
			},
		}
	}

	fn snapshot(day: NaiveDate, events: Vec<Event>) -> Snapshot {
		Snapshot {
			revision: 0,
			day,
			calendars: calendar::sample_snapshot().calendars,
			events,
			fetched_at: Local::now(),
			notice: None,
		}
	}

	#[test]
	fn the_day_in_view_is_read_from_its_snapshot_and_repeated_trips_list_once() {
		let config = config();
		let events = vec![
			event("a", view(), 9, "Office"),
			event("b", view(), 11, "Cafe"),
			event("c", view(), 13, "Office"),
			event("d", view(), 15, "Cafe"),
		];
		let found = connections(view(), Some(&snapshot(view(), events)), &config);

		let trip = |from, to| Leg { from, to };
		let expected = [trip(HOME, OFFICE), trip(OFFICE, CAFE), trip(CAFE, OFFICE)];
		assert_eq!(found.day, expected);
		assert_eq!(found.all, expected);
	}

	#[test]
	fn a_snapshot_of_another_day_says_nothing_about_the_day_in_view() {
		let config = config();
		let other = view().succ_opt().unwrap();
		let found = connections(
			view(),
			Some(&snapshot(other, vec![event("a", other, 9, "Office")])),
			&config,
		);
		assert_eq!(found, Connections::default());
	}

	#[test]
	fn a_question_in_progress_or_refused_speaks_over_the_saved_answer() {
		let mut travel = config().travel;
		let leg = Leg {
			from: HOME,
			to: OFFICE,
		};
		let mut asked = HashMap::new();
		travel.api_key.clear();
		// A token in the environment counts as one too.
		if travel.key().is_none() {
			assert_eq!(summary(&travel, &asked, leg), Summary::NoKey);
		}
		travel.api_key = "pk.test".into();
		assert_eq!(summary(&travel, &asked, leg), Summary::Unknown);

		let route = Route {
			seconds: 600,
			meters: 2000,
		};
		travel.set_route(SavedRoute {
			from: HOME,
			to: OFFICE,
			mode: TravelMode::Walk,
			seconds: route.seconds,
			meters: route.meters,
		});
		assert_eq!(summary(&travel, &asked, leg), Summary::Ready(route));

		let key = RouteKey::of(&travel, leg);
		asked.insert(key, Asked::Loading);
		assert_eq!(summary(&travel, &asked, leg), Summary::Loading);
		asked.insert(key, Asked::Failed("Quota".into()));
		assert_eq!(
			summary(&travel, &asked, leg),
			Summary::Failed("Quota".into())
		);

		// Another method is another trip, with no answer yet.
		travel.set_mode(HOME, OFFICE, TravelMode::Car);
		assert_eq!(summary(&travel, &asked, leg), Summary::Unknown);
	}

	#[test]
	fn open_in_maps_follows_the_map_chosen_on_the_desktop() {
		let mut travel = config().travel;
		let leg = Leg {
			from: HOME,
			to: OFFICE,
		};
		travel.set_mode(HOME, OFFICE, TravelMode::Bike);
		assert!(
			trip_url(&travel, leg)
				.unwrap()
				.starts_with("https://www.openstreetmap.org/directions?engine=fossgis_osrm_bike")
		);

		travel.map_app = places::MapApp::GoogleMaps;
		assert!(
			trip_url(&travel, leg)
				.unwrap()
				.ends_with("travelmode=bicycling")
		);
		assert!(trip_url(&travel, Leg { from: HOME, to: 99 }).is_none());
	}

	#[derive(Clone, PartialEq, Props)]
	struct PreviewProps {
		config: Config,
		connections: Connections,
		open: Option<Leg>,
		rename: Option<(u64, String)>,
	}

	#[allow(non_snake_case)]
	fn Preview(props: PreviewProps) -> Element {
		let config = use_signal(|| props.config.clone());
		let sheet = Sheet {
			config,
			asked: use_signal(HashMap::new),
			open: use_signal(|| props.open),
			rename: use_signal(|| props.rename.clone()),
			error: use_signal(|| None),
		};
		let travel = config.read().travel.clone();
		let unit = config.read().appearance.distance_unit;
		let asked = HashMap::new();
		rsx! {
			TravelSheet {
				config,
				day: view(),
				connections: props.connections.clone(),
				on_close: |_| {},
			}
			for place in travel.sorted() {
				{place_row(sheet, place, travel.home == Some(place.id), (sheet.rename)().as_ref())}
			}
			if let Some(leg) = props.open {
				{trip_card(sheet, &travel, &asked, leg, unit, None)}
			}
		}
	}

	fn render(props: PreviewProps) -> String {
		let mut dom = VirtualDom::new_with_props(Preview, props);
		dom.rebuild_in_place();
		dioxus_ssr::render(&dom)
	}

	fn trips() -> Connections {
		let trip = |from, to| Leg { from, to };
		Connections {
			day: vec![trip(HOME, OFFICE)],
			all: vec![trip(HOME, OFFICE), trip(OFFICE, CAFE)],
		}
	}

	#[test]
	fn the_sheet_lists_places_and_both_kinds_of_connection() {
		let mut config = config();
		config.travel.set_route(SavedRoute {
			from: HOME,
			to: OFFICE,
			mode: TravelMode::Walk,
			seconds: 600,
			meters: 2000,
		});
		let html = render(PreviewProps {
			config,
			connections: trips(),
			open: None,
			rename: None,
		});

		for text in [
			"Places and routes between your events",
			"Locations",
			"500 Market St",
			"Rename",
			"Day connections",
			"Tuesday, November 17",
			"All connections",
			"→ Office",
			"→ Cafe",
			"10 min · 2.0 km",
			"Walk",
			"Bike",
			"Car",
		] {
			assert!(html.contains(text), "{text} is missing from {html}");
		}
	}

	#[test]
	fn an_empty_sheet_says_how_trips_come_about() {
		let mut config = config();
		config.travel.places.clear();
		let html = render(PreviewProps {
			config,
			connections: Connections::default(),
			open: None,
			rename: None,
		});

		for text in ["No locations yet.", "No trips this day.", "No trips yet."] {
			assert!(html.contains(text), "{text} is missing from {html}");
		}
	}

	#[test]
	fn a_blank_title_cannot_be_saved() {
		let html = render(PreviewProps {
			config: config(),
			connections: Connections::default(),
			open: None,
			rename: Some((OFFICE, "   ".into())),
		});
		assert!(html.contains("travel-rename"));
		assert!(html.contains("disabled=true") || html.contains("disabled=\"true\""));
	}

	#[test]
	fn the_trip_card_offers_every_method_and_both_actions() {
		let html = render(PreviewProps {
			config: config(),
			connections: trips(),
			open: Some(Leg {
				from: HOME,
				to: OFFICE,
			}),
			rename: None,
		});
		for text in [
			"From",
			"To",
			"Travel method",
			"Travel time",
			"Not calculated yet",
			"Recalculate",
			"Open in Maps",
		] {
			assert!(html.contains(text), "{text} is missing from {html}");
		}
	}

	#[test]
	fn a_trip_whose_place_was_removed_is_no_longer_offered() {
		let html = render(PreviewProps {
			config: config(),
			connections: Connections {
				day: vec![Leg { from: HOME, to: 99 }],
				all: Vec::new(),
			},
			open: Some(Leg { from: HOME, to: 99 }),
			rename: None,
		});
		assert!(html.contains("This trip is no longer available"));
		assert!(!html.contains("→ "));
	}
}
