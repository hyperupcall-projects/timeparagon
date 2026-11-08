//! The Travel tab, the place search, and the card for one trip.
//!
//! The tab lists the saved places (whose titles can be changed) and then the
//! trips between them: first those the day in view makes, then every trip
//! there is. The search and the trip card are shared with the task editor,
//! the settings and the day view.

use iced::widget::{Space, button, column, container, mouse_area, row, stack, text};
use iced::{Alignment, Element, Fill, Theme};

use crate::app::{Message, State};
use crate::config::{MapApp, Place, TravelMode};
use crate::travel::{self, Action, Leg, PICKER_INPUT, PickTarget, Picker, Summary};

use super::layout::Metrics;

/// Room left for the scrollbar, which is drawn over the content.
const SCROLLBAR_GUTTER: f32 = 14.0;

fn msg(action: Action) -> Message {
    Message::Travel(action)
}

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut body = column![
        super::controls::page_heading(
            "Travel",
            "Places and routes between your events",
            None,
            metrics
        ),
        super::controls::section_heading(
            "Locations",
            Some(state.config.travel.places.len()),
            metrics.label() + 3.0
        ),
        places(state, metrics),
        super::controls::section_heading("Day connections", None, metrics.label() + 3.0),
        text(state.calendar.day.format("%A, %B %-d").to_string())
            .size(metrics.label() - 1.0)
            .style(text::secondary),
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    body = body.push(connections(
        state,
        travel::day_connections(state),
        "No trips this day. Back-to-back events with locations show up here.",
        metrics,
    ));
    body = body.push(super::controls::section_heading(
        "All connections",
        None,
        metrics.label() + 3.0,
    ));
    body = body.push(connections(
        state,
        travel::all_connections(state),
        "No trips yet.",
        metrics,
    ));

    container(
        crate::ui::controls::scrollable(container(body).padding(iced::Padding {
            right: SCROLLBAR_GUTTER,
            ..iced::Padding::ZERO
        }))
        .width(Fill)
        .height(Fill),
    )
    .padding(metrics.padding)
    .width(Fill)
    .height(Fill)
    .into()
}

fn card_style(theme: &Theme) -> container::Style {
    crate::ui::controls::row_surface(theme)
}

// ---------------------------------------------------------------------------
// Locations
// ---------------------------------------------------------------------------

fn places<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut list = column![].spacing(metrics.spacing).width(Fill);

    for place in state.config.travel.sorted() {
        list = list.push(
            container(place_row(state, place, metrics))
                .padding([6, 8])
                .width(Fill)
                .style(card_style),
        );
    }

    if state.config.travel.places.is_empty() {
        list = list.push(
            text("No locations yet. Add one to a task, or choose Home in Settings.")
                .size(metrics.label())
                .style(text::secondary),
        );
    }

    list.into()
}

fn place_row<'a>(state: &'a State, place: &'a Place, metrics: &Metrics) -> Element<'a, Message> {
    if let Some((id, name)) = &state.travel.rename
        && *id == place.id
    {
        return rename_row(name, metrics);
    }

    let header = row![
        text(&place.name).size(metrics.label() + 1.0).width(Fill),
        button(text("Rename").size(metrics.label()))
            .on_press(msg(Action::Rename(place.id)))
            .padding([2, 6])
            .style(crate::ui::controls::ghost),
    ]
    .spacing(6)
    .align_y(Alignment::Center);

    let mut content = column![header].spacing(2).width(Fill);

    if !place.address.is_empty() && place.address != place.name {
        content = content.push(
            text(&place.address)
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }

    if state.config.travel.home == Some(place.id) {
        content = content.push(
            text("Home")
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }

    content.into()
}

/// A place's title being changed. Only the title is edited: what Mapbox needs
/// to find the place stays as it was.
fn rename_row<'a>(name: &'a str, metrics: &Metrics) -> Element<'a, Message> {
    column![
        text("Title")
            .size(metrics.label() - 1.0)
            .style(text::secondary),
        crate::ui::controls::text_input("Title", name)
            .id(iced::widget::Id::new("travel-rename"))
            .on_input(|name| msg(Action::RenameText(name)))
            .on_submit(msg(Action::SaveRename))
            .size(metrics.label())
            .width(Fill),
        row![
            button(text("Save").size(metrics.label()).center())
                .on_press(msg(Action::SaveRename))
                .style(crate::ui::controls::primary),
            button(text("Cancel").size(metrics.label()).center())
                .on_press(msg(Action::CancelRename))
                .style(crate::ui::controls::secondary),
        ]
        .spacing(6),
    ]
    .spacing(4)
    .width(Fill)
    .into()
}

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

fn connections<'a>(
    state: &'a State,
    legs: Vec<Leg>,
    empty: &'a str,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let mut list = column![].spacing(metrics.spacing).width(Fill);

    for leg in &legs {
        if let Some(entry) = connection_row(state, *leg, metrics) {
            list = list.push(entry);
        }
    }

    if legs.is_empty() {
        list = list.push(text(empty).size(metrics.label()).style(text::secondary));
    }

    list.into()
}

/// One trip: where it starts and ends, with the way it is made at the right.
fn connection_row<'a>(
    state: &'a State,
    leg: Leg,
    metrics: &Metrics,
) -> Option<Element<'a, Message>> {
    let from = state.config.travel.place(leg.from)?;
    let to = state.config.travel.place(leg.to)?;

    let mut names = column![
        text(&from.name).size(metrics.label()),
        text(format!("→ {}", to.name)).size(metrics.label()),
    ]
    .spacing(1)
    .width(Fill);

    let detail = match travel::summary(state, leg) {
        Summary::Ready(route) => Some(travel::route_label(
            route,
            state.config.appearance.distance_unit,
        )),
        Summary::Loading => Some("Calculating…".to_owned()),
        Summary::Failed(_) => Some("No route".to_owned()),
        Summary::Unknown | Summary::NoKey => None,
    };
    if let Some(detail) = detail {
        names = names.push(
            text(detail)
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    }

    let open = button(names)
        .width(Fill)
        .padding([4, 6])
        .style(crate::ui::controls::ghost)
        .on_press(msg(Action::OpenLeg(leg, None)));

    Some(
        container(
            row![open, mode_control(state, leg, metrics)]
                .spacing(6)
                .align_y(Alignment::Center),
        )
        .padding([2, 4])
        .width(Fill)
        .style(card_style)
        .into(),
    )
}

/// How the trip is made. A dropdown needs more room than a 200px window has,
/// so there the control is a button that moves on to the next method.
fn mode_control<'a>(state: &State, leg: Leg, metrics: &Metrics) -> Element<'a, Message> {
    let mode = state.config.travel.mode_for(leg.from, leg.to);

    if metrics.breakpoint.is_compact() {
        button(text(mode.label()).size(metrics.label()).center())
            .on_press(msg(Action::SetMode(leg, mode.next())))
            .style(crate::ui::controls::secondary)
            .into()
    } else {
        crate::ui::controls::pick_list(TravelMode::ALL, Some(mode), move |mode| {
            msg(Action::SetMode(leg, mode))
        })
        .text_size(metrics.label())
        .into()
    }
}

// ---------------------------------------------------------------------------
// Choosing a place
// ---------------------------------------------------------------------------

/// A place field: the place that is set with the means to change or clear it,
/// or a button that opens the search while there is none. While the search is
/// open for this target it takes the field's place.
pub(super) fn location_control<'a>(
    state: &'a State,
    target: PickTarget,
    current: Option<&'a Place>,
    add_label: &'a str,
    metrics: &Metrics,
) -> Element<'a, Message> {
    if let Some(picker) = state
        .travel
        .picker
        .as_ref()
        .filter(|picker| picker.target == target)
    {
        return picker_view(state, picker, metrics);
    }

    match current {
        Some(place) => {
            let mut content = column![text(&place.name).size(metrics.label())]
                .spacing(2)
                .width(Fill);
            if !place.address.is_empty() && place.address != place.name {
                content = content.push(
                    text(&place.address)
                        .size(metrics.label() - 1.0)
                        .style(text::secondary),
                );
            }
            content
                .push(
                    row![
                        button(text("Change").size(metrics.label()).center())
                            .on_press(msg(Action::OpenPicker(target.clone())))
                            .style(crate::ui::controls::secondary),
                        button(text("Clear").size(metrics.label()).center())
                            .on_press(msg(Action::Clear(target)))
                            .style(crate::ui::controls::ghost),
                    ]
                    .spacing(6),
                )
                .into()
        }
        None => button(text(add_label).size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(msg(Action::OpenPicker(target)))
            .style(crate::ui::controls::secondary)
            .into(),
    }
}

fn picker_view<'a>(
    state: &'a State,
    picker: &'a Picker,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let mut input = crate::ui::controls::text_input(
        if picker.recents {
            "Search saved places"
        } else {
            "Search for a place"
        },
        &picker.query,
    )
    .id(iced::widget::Id::new(PICKER_INPUT))
    .on_input(|query| msg(Action::Query(query)))
    .size(metrics.label())
    .width(Fill);

    let search_row: Element<'a, Message> = if picker.recents {
        // The saved places are filtered as you type; Enter takes the first.
        if let Some(first) =
            travel::saved_matches(&state.config.travel.places, &picker.query).first()
        {
            input = input.on_submit(msg(Action::ChooseSaved(first.id)));
        }
        input.into()
    } else {
        let search = button(
            text(if picker.searching {
                "Searching…"
            } else {
                "Search"
            })
            .size(metrics.label())
            .center(),
        )
        .style(crate::ui::controls::primary);
        let search = if picker.searching || picker.query.trim().is_empty() {
            search
        } else {
            search.on_press(msg(Action::Search))
        };
        if !picker.searching && !picker.query.trim().is_empty() {
            input = input.on_submit(msg(Action::Search));
        }
        row![input, search]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
    };

    let map_button = button(text("Choose on map").size(metrics.label()))
        .on_press_maybe(
            state
                .config
                .travel
                .key()
                .map(|_| msg(Action::Map(travel::map::Action::Open))),
        )
        .style(crate::ui::controls::secondary);
    let recents_button = button(
        text(if picker.recents {
            "Search Mapbox"
        } else {
            "Search recents"
        })
        .size(metrics.label()),
    )
    .on_press(msg(Action::Recents))
    .style(if picker.recents {
        crate::ui::controls::primary
    } else {
        crate::ui::controls::secondary
    });
    // Two buttons side by side need more than a 200px window has to give.
    let modes: Element<'a, Message> = if metrics.breakpoint.is_compact() {
        column![map_button, recents_button].spacing(6).into()
    } else {
        row![map_button, recents_button].spacing(6).into()
    };

    let mut content = column![search_row, modes].spacing(6).width(Fill);

    if picker.recents {
        content = content.push(saved_places(state, picker, metrics));
    } else {
        content = mapbox_results(state, picker, content, metrics);
    }

    content
        .push(
            button(text("Cancel").size(metrics.label()))
                .on_press(msg(Action::ClosePicker))
                .style(crate::ui::controls::ghost),
        )
        .into()
}

fn mapbox_results<'a>(
    state: &State,
    picker: &'a Picker,
    mut content: iced::widget::Column<'a, Message>,
    metrics: &Metrics,
) -> iced::widget::Column<'a, Message> {
    if let Some(error) = &picker.error {
        content = content.push(text(error).size(metrics.label()).style(text::danger));
    } else if state.config.travel.key().is_none() {
        content = content.push(
            text(travel::NO_KEY)
                .size(metrics.label() - 1.0)
                .style(text::secondary),
        );
    } else if picker.searched && picker.results.is_empty() {
        content = content.push(
            text("No places found")
                .size(metrics.label())
                .style(text::secondary),
        );
    }

    for (index, found) in picker.results.iter().enumerate() {
        let mut label = column![text(&found.name).size(metrics.label())]
            .spacing(1)
            .width(Fill);
        if !found.address.is_empty() && found.address != found.name {
            // The button's own text color is used: the secondary one is
            // close to the button's background.
            label = label.push(text(&found.address).size(metrics.label() - 1.0));
        }
        content = content.push(
            button(label)
                .width(Fill)
                .padding([4, 6])
                .on_press(msg(Action::Choose(index)))
                .style(crate::ui::controls::secondary),
        );
    }
    content
}

/// The places already saved, matched against the query here rather than by
/// Mapbox: favorites first, then the most recently saved. Each has a star
/// that marks it a favorite or takes that away.
fn saved_places<'a>(state: &'a State, picker: &Picker, metrics: &Metrics) -> Element<'a, Message> {
    let matches = travel::saved_matches(&state.config.travel.places, &picker.query);
    let mut list = column![].spacing(6).width(Fill);

    if matches.is_empty() {
        list = list.push(
            text(if state.config.travel.places.is_empty() {
                "No saved places yet. Places you choose are saved here."
            } else {
                "No saved places match"
            })
            .size(metrics.label())
            .style(text::secondary),
        );
    }

    for place in matches {
        let mut label = column![text(&place.name).size(metrics.label())]
            .spacing(1)
            .width(Fill);
        if !place.address.is_empty() && place.address != place.name {
            label = label.push(text(&place.address).size(metrics.label() - 1.0));
        }
        let choose = button(label)
            .width(Fill)
            .padding([4, 6])
            .on_press(msg(Action::ChooseSaved(place.id)))
            .style(crate::ui::controls::secondary);
        let star = iced::widget::tooltip(
            button(
                text(if place.favorite { "★" } else { "☆" })
                    .size(metrics.label() + 3.0)
                    .center(),
            )
            .padding([2, 6])
            .on_press(msg(Action::Favorite(place.id)))
            .style(crate::ui::controls::ghost),
            text(if place.favorite {
                "Unfavorite"
            } else {
                "Favorite"
            })
            .size(12),
            iced::widget::tooltip::Position::Left,
        )
        .style(crate::ui::controls::surface);
        list = list.push(row![choose, star].spacing(4).align_y(Alignment::Center));
    }

    list.into()
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// The travel part of the settings page: the key, the usual method, and where
/// Home is.
pub(super) fn settings_section<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = &state.config.travel;
    let label = |label: &'static str| text(label).size(metrics.label()).style(text::secondary);

    column![
        super::settings::field(
            "Mapbox access token",
            crate::ui::controls::text_input("Access token", &settings.api_key)
                .on_input(|key| msg(Action::ApiKey(key)))
                .secure(true)
                .size(metrics.label())
                .into(),
            metrics,
        ),
        text("Create a public access token from your Mapbox account. It is used to search for places and to calculate trip durations.")
            .size((metrics.label() - 1.0).max(10.0))
            .style(text::secondary),
        super::settings::choice(
            "Usual travel method",
            TravelMode::ALL.to_vec(),
            Some(settings.default_mode),
            |mode| msg(Action::DefaultMode(mode)),
            metrics,
        ),
        super::settings::choice(
            "Open in Maps uses",
            MapApp::ALL.to_vec(),
            Some(settings.map_app),
            |app| msg(Action::SetMapApp(app)),
            metrics,
        ),
        label("Home"),
        location_control(
            state,
            PickTarget::Home,
            settings.home.and_then(|id| settings.place(id)),
            "Search for your home address",
            metrics,
        ),
        text("Your day’s travel starts and ends at Home. Place searches favor results near this address.")
            .size((metrics.label() - 1.0).max(10.0))
            .style(text::secondary),
    ]
    .spacing(metrics.spacing)
    .into()
}

// ---------------------------------------------------------------------------
// One trip
// ---------------------------------------------------------------------------

/// The card for one trip: where it goes, how long it takes, and how it is
/// made. It covers the window the way the estimate dialog does, and a click
/// outside it closes it.
pub fn leg_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut body = column![text("Travel").size(18)].spacing(10);

    let open = state.travel.open.as_ref();
    let places = open.and_then(|open| {
        Some((
            open,
            state.config.travel.place(open.leg.from)?,
            state.config.travel.place(open.leg.to)?,
        ))
    });

    match places {
        Some((open, from, to)) => body = body.push(leg_details(state, open, from, to, metrics)),
        None => {
            body = body.push(text("This trip is no longer available"));
        }
    }

    body = body.push(
        button(text("Close").size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(msg(Action::CloseLeg))
            .style(crate::ui::controls::secondary),
    );

    let card = container(crate::ui::controls::scrollable(body))
        .padding(16)
        .width(Fill)
        .max_width(420)
        .max_height((metrics.size.height - 32.0).max(80.0))
        .style(crate::ui::controls::surface);

    let backdrop =
        mouse_area(
            container(Space::new())
                .width(Fill)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(crate::ui::controls::SCRIM.into()),
                    ..Default::default()
                }),
        )
        .on_press(msg(Action::CloseLeg))
        .on_scroll(|_| Message::Ignored);

    let foreground = container(
        mouse_area(card)
            .on_press(Message::Ignored)
            .on_scroll(|_| Message::Ignored),
    )
    .padding(16)
    .center_x(Fill)
    .center_y(Fill);

    stack![backdrop, foreground].into()
}

fn leg_details<'a>(
    state: &'a State,
    open: &'a travel::Open,
    from: &'a Place,
    to: &'a Place,
    metrics: &Metrics,
) -> Element<'a, Message> {
    let leg = open.leg;
    let current = state.config.travel.mode_for(leg.from, leg.to);
    let caption = |label: &'static str| {
        text(label)
            .size(metrics.label() - 1.0)
            .style(text::secondary)
    };
    let place = |place: &'a Place| {
        let mut content = column![text(&place.name).size(metrics.label() + 1.0)].spacing(1);
        if !place.address.is_empty() && place.address != place.name {
            content = content.push(
                text(&place.address)
                    .size(metrics.label() - 1.0)
                    .style(text::secondary),
            );
        }
        content
    };

    let mut content = column![].spacing(8).width(Fill);

    if let Some((before, after)) = &open.between {
        content = content.push(
            text(format!("{before} → {after}"))
                .size(metrics.label())
                .style(text::secondary),
        );
    }

    content = content
        .push(caption("From"))
        .push(place(from))
        .push(caption("To"))
        .push(place(to))
        .push(caption("Travel method"));

    let choice = |mode: TravelMode| {
        button(
            text(mode.label())
                .size(metrics.label())
                .center()
                .width(Fill),
        )
        .width(Fill)
        .on_press(msg(Action::SetMode(leg, mode)))
        .style(if mode == current {
            crate::ui::controls::primary
        } else {
            crate::ui::controls::secondary
        })
    };
    content = content
        .push(row![choice(TravelMode::Walk), choice(TravelMode::Bike)].spacing(6))
        .push(row![choice(TravelMode::Car)].spacing(6));

    content = content.push(caption("Travel time"));
    content = content.push(match travel::summary(state, leg) {
        Summary::Ready(route) => text(travel::route_label(
            route,
            state.config.appearance.distance_unit,
        ))
        .size(metrics.label() + 3.0),
        Summary::Loading => text("Calculating…").size(metrics.label()),
        Summary::Unknown => text("Not calculated yet").size(metrics.label()),
        Summary::NoKey => text(travel::NO_KEY)
            .size(metrics.label())
            .style(text::danger),
        Summary::Failed(error) => text(error).size(metrics.label()).style(text::danger),
    });

    let recalculate = button(
        text("Recalculate")
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill)
    .on_press(msg(Action::Recalculate(leg)))
    .style(crate::ui::controls::secondary);
    let maps = button(
        text("Open in Maps")
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill)
    .on_press(msg(Action::OpenMaps(leg)))
    .style(crate::ui::controls::secondary);

    // Two buttons side by side need more than a 200px window has to give.
    if metrics.breakpoint.is_compact() {
        content.push(column![recalculate, maps].spacing(6)).into()
    } else {
        content.push(row![recalculate, maps].spacing(6)).into()
    }
}
