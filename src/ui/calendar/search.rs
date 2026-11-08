//! The Filter tab of the View sheet, which narrows every calendar view down
//! to what matches.

use super::*;
use crate::calendar::search::{Action, AvailabilityFilter};
use iced::widget::column;

/// The search bar, focused as the Filter tab opens.
pub(crate) const SEARCH_INPUT: &str = "calendar-search";

fn message(action: Action) -> Message {
    Message::Calendar(CalendarMessage::CalendarSearch(action))
}

/// An entry in the location list, where nothing chosen means every location.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LocationChoice(Option<String>);

impl std::fmt::Display for LocationChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_deref().unwrap_or("Any location"))
    }
}

fn section_title<'a>(title: &'a str, metrics: &Metrics) -> Element<'a, Message> {
    text(title)
        .size(metrics.label() + 1.0)
        .font(Font {
            weight: Weight::Bold,
            ..Font::DEFAULT
        })
        .into()
}

fn field_title<'a>(title: &'a str, metrics: &Metrics) -> Element<'a, Message> {
    text(title)
        .size(metrics.label())
        .style(text::secondary)
        .into()
}

fn toggle_row<'a>(
    label: &'a str,
    value: bool,
    change: fn(bool) -> Action,
    metrics: &Metrics,
) -> Element<'a, Message> {
    row![
        text(label).size(metrics.label()).width(Fill),
        crate::ui::controls::toggler(value).on_toggle(move |value| message(change(value))),
    ]
    .spacing(8.0)
    .align_y(Alignment::Center)
    .into()
}

fn time_field<'a>(state: &State, metrics: &Metrics) -> Element<'a, Message> {
    let time = state.calendar.search.time;
    let value = time.map(|time| time.format("%H:%M").to_string());
    let mut field = column![
        field_title("Time of day", metrics),
        crate::ui::controls::time(
            state.config.appearance.time_format,
            value.as_deref().unwrap_or_default(),
            true,
            metrics.label(),
            |value| message(Action::Time(value)),
        ),
    ]
    .spacing(6.0)
    .width(Fill);
    if time.is_some() {
        field = field.push(
            button(text("Any time").size(metrics.label()).center().width(Fill))
                .width(Fill)
                .on_press(message(Action::ClearTime))
                .style(crate::ui::controls::secondary),
        );
    }
    field.into()
}

fn labels_field<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut field = column![field_title("Labels", metrics)]
        .spacing(6.0)
        .width(Fill);
    let labels = state.calendar.search_labels();
    if labels.is_empty() {
        field = field.push(
            text("No labels on the events loaded so far")
                .size(metrics.label())
                .style(text::secondary),
        );
    }
    for label in labels {
        let checked = state.calendar.search.labels.contains(&label);
        let name = label.clone();
        field = field.push(
            crate::ui::controls::checkbox(checked)
                .label(label)
                .text_size(metrics.label())
                .on_toggle(move |value| message(Action::Label(name.clone(), value))),
        );
    }
    field.into()
}

fn location_field<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let mut choices = vec![LocationChoice(None)];
    choices.extend(
        state
            .calendar
            .search_locations()
            .into_iter()
            .map(|location| LocationChoice(Some(location))),
    );
    column![
        field_title("Location", metrics),
        crate::ui::controls::pick_list(
            choices,
            Some(LocationChoice(state.calendar.search.location.clone())),
            |choice| message(Action::Location(choice.0)),
        )
        .text_size(metrics.label())
        .width(Fill),
    ]
    .spacing(6.0)
    .width(Fill)
    .into()
}

pub(super) fn search_overlay<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let search = &state.calendar.search;

    let panel = column![
        crate::ui::controls::text_input("Search", &search.query)
            .id(iced::widget::Id::new(SEARCH_INPUT))
            .on_input(|query| message(Action::Query(query)))
            .size(metrics.label())
            .width(Fill),
        toggle_row(
            "Search description",
            search.in_description,
            Action::InDescription,
            metrics,
        ),
        section_title("Search filters", metrics),
        time_field(state, metrics),
        super::event_projects(state, &search.projects, metrics, |action| {
            message(Action::Projects(action))
        }),
        labels_field(state, metrics),
        section_title("Event filters", metrics),
        column![
            field_title("Show events that are", metrics),
            crate::ui::controls::pick_list(
                AvailabilityFilter::ALL,
                Some(search.availability),
                |value| message(Action::Availability(value)),
            )
            .text_size(metrics.label())
            .width(Fill),
        ]
        .spacing(6.0)
        .width(Fill),
        toggle_row(
            "Only show events with public visibility",
            search.public_only,
            Action::PublicOnly,
            metrics,
        ),
        location_field(state, metrics),
        section_title("Detail filter", metrics),
        toggle_row(
            "Show details by visibility",
            search.details_by_visibility,
            Action::DetailsByVisibility,
            metrics,
        ),
        button(
            text("Clear search")
                .size(metrics.label())
                .center()
                .width(Fill)
        )
        .width(Fill)
        .on_press_maybe(search.is_modified().then(|| message(Action::Clear)))
        .style(crate::ui::controls::secondary),
    ]
    .spacing(metrics.spacing)
    .width(Fill);

    super::view_sheet(
        state,
        metrics,
        super::ViewTab::Filter,
        panel,
        message(Action::Toggle),
    )
}
