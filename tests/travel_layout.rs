//! The Travel tab, the calendar menu's Travel toggle, the place search and the
//! trip card, rendered headlessly at the widths the layout is designed around.

use iced::{Element, Size};
use iced_test::core::Settings;
use iced_test::{Error, Simulator as Sim};

use timeslay::app::NavigationMessage;
use timeslay::app::{self, Message, Screen, State};
use timeslay::calendar::{Store, sample_snapshot};
use timeslay::config::TravelMode;
use timeslay::travel::{Action, Found, Leg, PickTarget, Route, RouteKey};

const WIDTHS: [f32; 4] = [180.0, 200.0, 260.0, 420.0];

type Simulator<'a> = Sim<'a, Message>;

fn render(state: &State, size: Size) -> Simulator<'_> {
    at(size, app::view(state, iced::window::Id::unique()))
}

fn at<'a>(size: Size, element: Element<'a, Message>) -> Simulator<'a> {
    Sim::with_size(Settings::default(), size, element)
}

/// A day with a start and end at home and meetings at two other places.
/// Returns the state with the ids of home, the office and the cafe.
fn state() -> (State, u64, u64, u64) {
    let mut state = State::blank();
    state.calendar = Store::with_snapshot(sample_snapshot());

    let travel = &mut state.config.travel;
    let home = travel.add_place("Home", "12 Elm St", "m-home", 1.0, 1.0);
    let office = travel.add_place("Office", "500 Market St", "m-office", 1.0, 2.0);
    let cafe = travel.add_place("Blue Bottle", "66 Mint St", "m-cafe", 1.0, 3.0);
    travel.home = Some(home);
    travel.api_key = "key".into();
    travel.set_mode(office, cafe, TravelMode::Bike);

    for event in &mut state.calendar.events {
        event.details.location = match event.id.as_str() {
            "standup" | "review" => "Office".into(),
            "lunch" => "Blue Bottle".into(),
            _ => String::new(),
        };
    }
    (state, home, office, cafe)
}

fn messages(ui: Simulator<'_>) -> Vec<Message> {
    ui.into_messages().collect()
}

/// Travel is toggled from the calendar menu beside the TimeSlay sources, and
/// pressing its switch asks for travel to be shown. The header no longer
/// carries a Toggle Travel button, whatever the width.
#[test]
fn travel_is_toggled_from_the_calendar_menu() {
    for width in WIDTHS.into_iter().chain([340.0, 460.0, 640.0]) {
        let (mut state, ..) = state();
        state.main_size = Size::new(width, 4000.0);

        // The day view's header keeps View and Actions and drops Toggle
        // Travel.
        let mut header = Sim::with_size(
            Settings::default(),
            Size::new(width, 500.0),
            app::view(&state, iced::window::Id::unique()),
        );
        assert!(
            header.find("Actions").is_ok(),
            "no Actions at {width}px"
        );
        assert!(header.find("View").is_ok(), "no View at {width}px");
        assert!(
            header.find("Toggle Travel").is_err(),
            "Toggle Travel is still in the header at {width}px"
        );
        drop(header);

        // Travel sits in the Calendars tab under the TimeSlay heading, after Sleep.
        state.calendar.menu_open = true;
        let mut ui = Sim::with_size(
            Settings::default(),
            state.main_size,
            timeslay::ui::calendar::view(
                &state,
                &timeslay::ui::layout::Metrics::of(state.main_size),
            ),
        );
        let timeslay_y = ui.find("TimeSlay").unwrap().bounds().y;
        let sleep_y = ui.find("Sleep").unwrap().bounds().y;
        let travel = ui.find("Travel").expect("Travel row").bounds();
        assert!(travel.y > timeslay_y, "Travel above TimeSlay at {width}px");
        assert!(travel.y > sleep_y, "Travel above Sleep at {width}px");
        assert!(travel.x >= 0.0 && travel.x + travel.width <= width);

        // Click the switch just right of the label, as a person would.
        ui.point_at(iced::Point::new(
            travel.x + travel.width + 20.0,
            travel.center_y(),
        ));
        ui.simulate(iced_test::simulator::click());
        assert!(
            messages(ui)
                .iter()
                .any(|m| matches!(m, Message::Travel(Action::Toggle))),
            "pressing Travel did nothing at {width}px"
        );
    }
}

#[test]
fn toggling_travel_turns_the_day_view_trips_on_and_off() {
    let (mut state, ..) = state();
    assert!(!state.travel.show);

    let _ = app::update(&mut state, Message::Travel(Action::Toggle));
    assert!(state.travel.show);
    // Every trip of the day is being asked for.
    let asked = state.travel.routes.len();
    // Events without a place break the chain, so only the trips between
    // neighbours that both have one are asked for: home to the standup,
    // lunch to the review, and the review back home.
    assert_eq!(asked, 3, "{:?}", state.travel.routes);

    let _ = app::update(&mut state, Message::Travel(Action::Toggle));
    assert!(!state.travel.show);
}

/// Travel and Habits live together in the More menu, with Travel first, and
/// clicking Travel opens the Travel screen.
#[test]
fn the_travel_item_is_in_the_more_menu() -> Result<(), Error> {
    for width in WIDTHS {
        let (mut state, ..) = state();
        state.more_menu_open = true;
        let mut ui = render(&state, Size::new(width, 400.0));

        let travel = ui.find("Travel").expect("Travel item").bounds();
        let habits = ui.find("Habits").expect("Habits item").bounds();
        assert!(travel.y + travel.height <= habits.y, "order at {width}px");

        let _ = ui.click("Travel")?;
        assert!(
            messages(ui).iter().any(|m| matches!(
                m,
                Message::Navigation(NavigationMessage::ScreenSelected(Screen::Travel))
            )),
            "at {width}px"
        );
    }
    Ok(())
}

#[test]
fn the_travel_tab_lists_places_then_day_connections_then_all_connections() {
    for width in WIDTHS {
        let (mut state, ..) = state();
        state.screen = Screen::Travel;
        let mut ui = render(&state, Size::new(width, 2000.0));

        let top = |ui: &mut Simulator<'_>, label: &str| {
            ui.find(label)
                .unwrap_or_else(|_| panic!("{label:?} is missing at {width}px"))
                .bounds()
                .y
        };

        let locations = top(&mut ui, "Locations");
        let home = top(&mut ui, "Home");
        let day = top(&mut ui, "Day connections");
        let all = top(&mut ui, "All connections");
        assert!(
            locations < home && home < day && day < all,
            "sections are out of order at {width}px"
        );

        // Each place can be renamed.
        assert!(ui.find("Rename").is_ok());
    }
}

/// A connection names where it starts and ends, and its method sits to the
/// right of the names.
#[test]
fn a_connection_shows_its_places_and_its_method_beside_them() -> Result<(), Error> {
    let (mut state, home, office, _) = state();
    state.screen = Screen::Travel;
    let _ = app::update(
        &mut state,
        Message::Travel(Action::Routed(
            RouteKey {
                from: home,
                to: office,
                mode: TravelMode::Walk,
            },
            Ok(Route {
                seconds: 1500,
                meters: 2100,
            }),
        )),
    );

    // Wide enough for the method to be a dropdown, which draws its own text,
    // so what is checked is the names and the trip's time beside them.
    let mut ui = render(&state, Size::new(520.0, 2000.0));
    assert!(ui.find("→ Office").is_ok(), "the end of the trip");
    assert!(ui.find("25 min · 2.1 km").is_ok(), "how long it takes");
    drop(ui);

    // A narrow window gets a button that moves on to the next method.
    let mut ui = render(&state, Size::new(200.0, 2000.0));
    let _ = ui.click("Walk")?;
    assert!(messages(ui).iter().any(|m| matches!(
        m,
        Message::Travel(Action::SetMode(leg, TravelMode::Bike))
            if *leg == (Leg { from: home, to: office })
    )));
    Ok(())
}

#[test]
fn clicking_a_connection_opens_its_details() -> Result<(), Error> {
    let (mut state, home, office, _) = state();
    state.screen = Screen::Travel;

    let mut ui = render(&state, Size::new(300.0, 2000.0));
    let _ = ui.click("→ Office")?;
    assert!(messages(ui).iter().any(|m| matches!(
        m,
        Message::Travel(Action::OpenLeg(leg, None))
            if *leg == (Leg { from: home, to: office })
    )));
    Ok(())
}

/// The card for a trip says where it goes, offers all four methods, and
/// choosing one changes how the trip is made.
#[test]
fn the_trip_card_offers_every_method() -> Result<(), Error> {
    for width in WIDTHS {
        let (mut state, home, office, _) = state();
        let leg = Leg {
            from: home,
            to: office,
        };
        let _ = app::update(
            &mut state,
            Message::Travel(Action::OpenLeg(
                leg,
                Some(("Start of day".into(), "Standup".into())),
            )),
        );

        let mut ui = render(&state, Size::new(width, 900.0));
        for label in ["Travel", "Home", "Office", "Walk", "Bike", "Car", "Close"] {
            assert!(ui.find(label).is_ok(), "{label:?} is missing at {width}px");
        }

        let _ = ui.click("Car")?;
        assert!(messages(ui).iter().any(|m| matches!(
            m,
            Message::Travel(Action::SetMode(chosen, TravelMode::Car)) if *chosen == leg
        )));
    }
    Ok(())
}

#[test]
fn choosing_a_method_changes_it_for_that_trip_only() {
    let (mut state, home, office, cafe) = state();
    let leg = Leg {
        from: home,
        to: office,
    };

    let _ = app::update(&mut state, Message::Travel(Action::OpenLeg(leg, None)));
    let _ = app::update(
        &mut state,
        Message::Travel(Action::SetMode(leg, TravelMode::Car)),
    );

    assert_eq!(state.config.travel.mode_for(home, office), TravelMode::Car);
    assert_eq!(state.config.travel.mode_for(office, home), TravelMode::Walk);
    assert_eq!(state.config.travel.mode_for(office, cafe), TravelMode::Bike);

    // Closing the card leaves the choice.
    let _ = app::update(&mut state, Message::Travel(Action::CloseLeg));
    assert!(state.travel.open.is_none());
    assert_eq!(state.config.travel.mode_for(home, office), TravelMode::Car);
}

/// Renaming a place changes the title and nothing Mapbox needs.
#[test]
fn a_place_can_be_renamed_from_the_travel_tab() -> Result<(), Error> {
    let (mut state, _, office, _) = state();
    state.screen = Screen::Travel;

    let _ = app::update(&mut state, Message::Travel(Action::Rename(office)));
    let mut ui = render(&state, Size::new(260.0, 2000.0));
    assert!(ui.find("Save").is_ok() && ui.find("Cancel").is_ok());
    drop(ui);

    let _ = app::update(
        &mut state,
        Message::Travel(Action::RenameText("The office".into())),
    );
    let _ = app::update(&mut state, Message::Travel(Action::SaveRename));

    let place = state.config.travel.place(office).unwrap();
    assert_eq!(place.name, "The office");
    assert_eq!(place.address, "500 Market St");
    assert_eq!(place.place_id, "m-office");

    let mut ui = render(&state, Size::new(260.0, 2000.0));
    assert!(ui.find("The office").is_ok());
    Ok(())
}

/// The settings page can search for Home.
#[test]
fn settings_choose_home() -> Result<(), Error> {
    for width in WIDTHS {
        let (mut state, ..) = state();
        state.screen = Screen::Settings;

        let mut ui = render(&state, Size::new(width, 5200.0));
        for label in ["Travel", "Mapbox access token", "Home"] {
            assert!(ui.find(label).is_ok(), "{label:?} is missing at {width}px");
        }
        let _ = ui.click("Change")?;
        assert!(
            messages(ui)
                .iter()
                .any(|m| matches!(m, Message::Travel(Action::OpenPicker(PickTarget::Home))))
        );
    }

    // Searching shows what Mapbox found, and choosing one sets the place.
    let (mut state, ..) = state();
    state.screen = Screen::Settings;
    state.config.travel.home = None;
    let _ = app::update(
        &mut state,
        Message::Travel(Action::OpenPicker(PickTarget::Home)),
    );
    let picker = state.travel.picker.as_mut().unwrap();
    picker.query = "coffee".into();
    picker.searched = true;
    picker.results = vec![Found {
        place_id: "m-philz".into(),
        name: "Philz Coffee".into(),
        address: "1 Main St".into(),
        lat: 5.0,
        lng: 6.0,
    }];

    let mut ui = render(&state, Size::new(300.0, 5000.0));
    assert!(ui.find("Search").is_ok());
    let _ = ui.click("Philz Coffee")?;
    assert!(
        messages(ui)
            .iter()
            .any(|m| matches!(m, Message::Travel(Action::Choose(0))))
    );

    let _ = app::update(&mut state, Message::Travel(Action::Choose(0)));
    let home = state.config.travel.home.expect("Home is set");
    let place = state.config.travel.place(home).unwrap();
    assert_eq!(
        (place.name.as_str(), place.place_id.as_str()),
        ("Philz Coffee", "m-philz")
    );
    assert_eq!((place.lat, place.lng), (5.0, 6.0));
    Ok(())
}

/// Clicking a Google Calendar event offers to add a location, the same as a
/// task does, and shows the place once it has one.
#[test]
fn a_calendar_event_can_be_given_a_location() -> Result<(), Error> {
    for width in [260.0, 360.0, 620.0] {
        let (mut state, _, office, _) = state();
        state.config.travel.event_places.clear();
        for event in &mut state.calendar.events {
            event.details.location = String::new();
        }
        state.calendar.open_event = Some("standup".into());

        let mut ui = at(Size::new(width, 900.0), timeslay::ui::editor_window(&state));
        let _ = ui
            .click("Add location")
            .unwrap_or_else(|_| panic!("no button at {width}px"));
        assert!(
            messages(ui).iter().any(|m| matches!(
                m,
                Message::Travel(Action::OpenPicker(PickTarget::Event(id))) if id == "standup"
            )),
            "at {width}px"
        );

        // With a place set it is named, with a way to change or clear it.
        state
            .config
            .travel
            .event_places
            .insert("standup".into(), office);
        let mut ui = at(Size::new(width, 900.0), timeslay::ui::editor_window(&state));
        assert!(ui.find("Office").is_ok() && ui.find("Change").is_ok() && ui.find("Clear").is_ok());
        assert!(ui.find("Add location").is_err());
    }
    Ok(())
}

#[test]
fn map_picker_opens_from_search_and_requires_a_selection() -> Result<(), Error> {
    for width in WIDTHS {
        let (mut state, ..) = state();
        state.screen = Screen::Settings;
        let _ = app::update(
            &mut state,
            Message::Travel(Action::OpenPicker(PickTarget::Home)),
        );
        let mut ui = render(&state, Size::new(width, 5200.0));
        let _ = ui.click("Choose on map")?;
        assert!(messages(ui).iter().any(|m| matches!(
            m,
            Message::Travel(Action::Map(timeslay::travel::map::Action::Open))
        )));

        // Render native controls without initializing CEF in headless tests.
        state.travel.map.active = true;
        let mut ui = render(&state, Size::new(width, 900.0));
        assert!(ui.find("Confirm").is_ok());
        let _ = ui.click("Confirm")?;
        assert!(!messages(ui).iter().any(|m| matches!(
            m,
            Message::Travel(Action::Map(timeslay::travel::map::Action::Confirm))
        )));
        state.travel.map.selected = Some(Found {
            place_id: "cafe".into(),
            name: "Cafe".into(),
            address: "1 Main St".into(),
            lat: 37.8,
            lng: -122.4,
        });
        let mut ui = render(&state, Size::new(width, 900.0));
        let _ = ui.click("Confirm")?;
        assert!(messages(ui).iter().any(|m| matches!(
            m,
            Message::Travel(Action::Map(timeslay::travel::map::Action::Confirm))
        )));
    }
    Ok(())
}

/// With a panel open the map belongs to the floating window at its right:
/// the panel keeps what it was showing.
#[test]
fn map_picker_is_shown_only_in_the_floating_window() {
    let (mut state, ..) = state();
    state.screen = Screen::Settings;
    state.main_window = Some(iced::window::Id::unique());
    let _ = app::update(
        &mut state,
        Message::Travel(Action::OpenPicker(PickTarget::Home)),
    );
    state.travel.map.active = true;
    assert!(state.map_in_editor_window());

    let mut panel = render(&state, Size::new(420.0, 900.0));
    assert!(panel.find("Confirm").is_err(), "not in the panel");
    let mut editor = at(Size::new(420.0, 900.0), timeslay::ui::editor_window(&state));
    assert!(editor.find("Confirm").is_ok(), "in the floating window");
}

/// Search recents lists the saved places without Mapbox, filters them as
/// you type, and each can be chosen or starred.
#[test]
fn search_recents_lists_saved_places_with_stars() -> Result<(), Error> {
    for width in WIDTHS {
        let (mut state, _, office, _) = state();
        state.screen = Screen::Settings;
        let _ = app::update(
            &mut state,
            Message::Travel(Action::OpenPicker(PickTarget::Home)),
        );
        let mut ui = render(&state, Size::new(width, 5200.0));
        let _ = ui.click("Search recents")?;
        assert!(
            messages(ui)
                .iter()
                .any(|m| matches!(m, Message::Travel(Action::Recents))),
            "at {width}px"
        );

        let _ = app::update(&mut state, Message::Travel(Action::Recents));
        let _ = app::update(&mut state, Message::Travel(Action::Query("market".into())));
        let mut ui = render(&state, Size::new(width, 5200.0));
        assert!(ui.find("500 Market St").is_ok(), "at {width}px");
        assert!(ui.find("66 Mint St").is_err(), "filtered at {width}px");
        let _ = ui.click("☆")?;
        assert!(
            messages(ui)
                .iter()
                .any(|m| matches!(m, Message::Travel(Action::Favorite(id)) if *id == office))
        );

        let mut ui = render(&state, Size::new(width, 5200.0));
        let _ = ui.click("500 Market St")?;
        assert!(
            messages(ui)
                .iter()
                .any(|m| matches!(m, Message::Travel(Action::ChooseSaved(id)) if *id == office))
        );
    }
    Ok(())
}
