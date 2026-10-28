//! The Travel tab, the place search, and the card for one trip.
//!
//! The tab lists the saved places (whose titles can be changed) and then the
//! trips between them: first those the day in view makes, then every trip
//! there is. The search and the trip card are shared with the task editor,
//! the settings and the day view.

use iced::widget::{
    Space, button, column, container, mouse_area, pick_list, row, scrollable, stack, text,
    text_input,
};
use iced::{Alignment, Color, Element, Fill, Theme};

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
        text("Locations").size(metrics.label() + 3.0),
        places(state, metrics),
        text("Day connections").size(metrics.label() + 3.0),
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
    body = body.push(text("All connections").size(metrics.label() + 3.0));
    body = body.push(connections(
        state,
        travel::all_connections(state),
        "No trips yet.",
        metrics,
    ));

    container(
        scrollable(container(body).padding(iced::Padding {
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
    let palette = theme.extended_palette();
    container::Style {
        background: Some(palette.background.base.color.into()),
        border: iced::Border {
            color: palette.background.strong.color,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..container::Style::default()
    }
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
            text("No locations yet. Add one to a task, or choose where the day starts and ends in Settings.")
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
            .style(button::text),
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

    let mut uses = Vec::new();
    if state.config.travel.start == Some(place.id) {
        uses.push("Start of day");
    }
    if state.config.travel.end == Some(place.id) {
        uses.push("End of day");
    }
    if !uses.is_empty() {
        content = content.push(
            text(uses.join(" · "))
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
        text_input("Title", name)
            .id(iced::widget::Id::new("travel-rename"))
            .on_input(|name| msg(Action::RenameText(name)))
            .on_submit(msg(Action::SaveRename))
            .size(metrics.label())
            .width(Fill),
        row![
            button(text("Save").size(metrics.label()).center())
                .on_press(msg(Action::SaveRename))
                .style(button::primary),
            button(text("Cancel").size(metrics.label()).center())
                .on_press(msg(Action::CancelRename))
                .style(button::secondary),
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
        Summary::Ready(route) => Some(travel::route_label(route)),
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
        .style(button::text)
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
            .style(button::secondary)
            .into()
    } else {
        pick_list(TravelMode::ALL, Some(mode), move |mode| {
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
                            .style(button::secondary),
                        button(text("Clear").size(metrics.label()).center())
                            .on_press(msg(Action::Clear(target)))
                            .style(button::text),
                    ]
                    .spacing(6),
                )
                .into()
        }
        None => button(text(add_label).size(metrics.label()).center().width(Fill))
            .width(Fill)
            .on_press(msg(Action::OpenPicker(target)))
            .style(button::secondary)
            .into(),
    }
}

fn picker_view<'a>(state: &State, picker: &'a Picker, metrics: &Metrics) -> Element<'a, Message> {
    let search = button(
        text(if picker.searching {
            "Searching…"
        } else {
            "Search"
        })
        .size(metrics.label())
        .center(),
    )
    .style(button::primary);
    let search = if picker.searching || picker.query.trim().is_empty() {
        search
    } else {
        search.on_press(msg(Action::Search))
    };

    let mut input = text_input("Search for a place", &picker.query)
        .id(iced::widget::Id::new(PICKER_INPUT))
        .on_input(|query| msg(Action::Query(query)))
        .size(metrics.label())
        .width(Fill);
    if !picker.searching && !picker.query.trim().is_empty() {
        input = input.on_submit(msg(Action::Search));
    }

    let mut content = column![row![input, search].spacing(6).align_y(Alignment::Center),]
        .spacing(6)
        .width(Fill);

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
                .style(button::secondary),
        );
    }

    content
        .push(
            button(text("Cancel").size(metrics.label()))
                .on_press(msg(Action::ClosePicker))
                .style(button::text),
        )
        .into()
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/// The travel part of the settings page: the key, the usual method, and where
/// the day starts and ends.
pub(super) fn settings_section<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let settings = &state.config.travel;
    let label = |label: &'static str| text(label).size(metrics.label()).style(text::secondary);

    column![
        super::settings::field(
            "Mapbox access token",
            text_input("Access token", &settings.api_key)
                .on_input(|key| msg(Action::ApiKey(key)))
                .secure(true)
                .size(metrics.label())
                .into(),
            metrics,
        ),
        text("Create a public access token (pk.) in your Mapbox account. It is used to search for places and to work out how long a trip takes.")
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
        label("Start location"),
        location_control(
            state,
            PickTarget::Start,
            settings.start.and_then(|id| settings.place(id)),
            "Search for a start location",
            metrics,
        ),
        label("End location"),
        location_control(
            state,
            PickTarget::End,
            settings.end.and_then(|id| settings.place(id)),
            "Search for an end location",
            metrics,
        ),
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
            .style(button::secondary),
    );

    let card = container(scrollable(body))
        .padding(16)
        .width(Fill)
        .max_width(420)
        .max_height((metrics.size.height - 32.0).max(80.0))
        .style(|theme: &Theme| {
            let palette = theme.extended_palette();
            container::Style {
                background: Some(palette.background.base.color.into()),
                text_color: Some(palette.background.base.text),
                border: iced::Border {
                    radius: 10.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });

    let backdrop =
        mouse_area(
            container(Space::new())
                .width(Fill)
                .height(Fill)
                .style(|_| container::Style {
                    background: Some(Color::BLACK.scale_alpha(0.58).into()),
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
            button::primary
        } else {
            button::secondary
        })
    };
    content = content
        .push(row![choice(TravelMode::Walk), choice(TravelMode::Bike)].spacing(6))
        .push(row![choice(TravelMode::Car)].spacing(6));

    content = content.push(caption("Travel time"));
    content = content.push(match travel::summary(state, leg) {
        Summary::Ready(route) => text(travel::route_label(route)).size(metrics.label() + 3.0),
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
    .style(button::secondary);
    let maps = button(
        text("Open in Maps")
            .size(metrics.label())
            .center()
            .width(Fill),
    )
    .width(Fill)
    .on_press(msg(Action::OpenMaps(leg)))
    .style(button::secondary);

    // Two buttons side by side need more than a 200px window has to give.
    if metrics.breakpoint.is_compact() {
        content.push(column![recalculate, maps].spacing(6)).into()
    } else {
        content.push(row![recalculate, maps].spacing(6)).into()
    }
}
