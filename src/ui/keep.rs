//! Google Keep's read-only browser snapshots with local search and labels.
use super::layout::Metrics;
use crate::app::{Message, State};
use crate::keep::{Note, Scope};
use iced::widget::{
    button, column, container, pick_list, responsive, rich_text, row, scrollable, span, text,
    text_input,
};
use iced::{Element, Fill};

pub fn view<'a>(state: &'a State, metrics: &Metrics) -> Element<'a, Message> {
    let keep = &state.keep;
    let mut refresh = button(text("Refresh").size(metrics.label()));
    let mut connect = button(text("Reconnect").size(metrics.label()));
    if !keep.loading {
        refresh = refresh.on_press(Message::KeepRefresh);
        connect = connect.on_press(Message::KeepSignin);
    }
    let mut body = column![
        text("Keep Notes").size(metrics.label() + 3.0),
        row![refresh, connect].spacing(metrics.spacing),
        text_input("Search title or description…", &keep.query)
            .on_input(Message::KeepQueryChanged)
            .size(metrics.label()),
        row![
            scope_button("Active", Scope::Active, keep.scope, metrics),
            scope_button("All", Scope::All, keep.scope, metrics),
            scope_button("Archived", Scope::Archived, keep.scope, metrics)
        ]
        .spacing(metrics.spacing),
    ]
    .spacing(metrics.spacing);
    let mut labels = vec![LabelOption(None)];
    labels.extend(keep.labels().into_iter().map(|l| LabelOption(Some(l))));
    body = body.push(text("Labels").size(metrics.label())).push(
        pick_list(labels, Some(LabelOption(keep.label.clone())), |label| {
            Message::KeepLabelChanged(label.0)
        })
        .width(Fill),
    );
    if keep.loading {
        body = body.push(text("Loading Keep notes…").size(metrics.label()));
    }
    if let Some(error) = &keep.error {
        body = body.push(text(error).size(metrics.label()));
    }
    for warning in &keep.snapshot.warnings {
        body = body.push(text(warning).size(metrics.label()));
    }
    let notes = keep.visible();
    let metrics = *metrics;
    body.push(
        responsive(move |size| {
            if notes.is_empty() && !keep.loading {
                return scrollable(
                    text(if keep.snapshot.notes.is_empty() {
                        "No Keep notes loaded."
                    } else {
                        "No matching notes."
                    })
                    .size(metrics.label()),
                )
                .height(Fill)
                .into();
            }
            // Each column stacks independently so short notes do not leave the
            // empty row-sized gaps of a conventional grid. Reserve scrollbar room.
            let count = column_count(size.width, metrics.spacing);
            let mut columns: Vec<_> = (0..count)
                .map(|_| column![].spacing(metrics.spacing).width(Fill))
                .collect();
            for (index, note) in notes.iter().enumerate() {
                let slot = index % count;
                columns[slot] =
                    std::mem::replace(&mut columns[slot], column![]).push(card(note, &metrics));
            }
            let cards = row(columns.into_iter().map(Element::from))
                .spacing(metrics.spacing)
                .padding(iced::Padding::default().right(12))
                .width(Fill);
            scrollable(cards).height(Fill).into()
        })
        .height(Fill),
    )
    .padding(metrics.padding)
    .height(Fill)
    .width(Fill)
    .into()
}

fn column_count(width: f32, spacing: f32) -> usize {
    const MIN_CARD_WIDTH: f32 = 280.0;
    (((width - 12.0 + spacing) / (MIN_CARD_WIDTH + spacing)) as usize).clamp(1, 3)
}

fn card<'a>(note: &'a Note, metrics: &Metrics) -> Element<'a, Message> {
    let color = card_color(&note.color);
    let dark = color.is_some_and(|color| color.r * 0.299 + color.g * 0.587 + color.b * 0.114 < 0.5);
    let ink = if dark {
        iced::Color::WHITE
    } else {
        iced::Color::from_rgb8(32, 33, 36)
    };
    let muted = iced::Color { a: 0.65, ..ink };
    let details = note.parsed_metadata();
    let mut heading = row![text(&note.title).size(metrics.label() + 5.0).width(Fill)].spacing(6);
    if note.pinned {
        heading = heading.push(chip("Pinned".into(), ink, metrics));
    }
    let mut content = column![heading].spacing(metrics.spacing).width(Fill);
    if note.checklist.is_empty() {
        if note.body.is_empty() {
            if !note.description.is_empty() {
                content = content.push(text(&note.description).size(metrics.label() + 2.0));
            }
        } else {
            let runs: Vec<iced::widget::text::Span<'_, (), iced::Font>> = note
                .body
                .iter()
                .map(|run| {
                    span(run.text.as_str())
                        .font(iced::Font {
                            weight: if run.bold {
                                iced::font::Weight::Bold
                            } else {
                                iced::font::Weight::Normal
                            },
                            style: if run.italic {
                                iced::font::Style::Italic
                            } else {
                                iced::font::Style::Normal
                            },
                            ..iced::Font::DEFAULT
                        })
                        .underline(run.underline)
                        .strikethrough(run.strikethrough)
                })
                .collect();
            content = content.push(rich_text(runs).size(metrics.label() + 2.0));
        }
    }
    for item in &note.checklist {
        content = content.push(
            row![
                text(if item.checked { "☑" } else { "☐" }).color(if item.checked {
                    muted
                } else {
                    ink
                }),
                text(&item.text)
                    .size(metrics.label() + 2.0)
                    .color(if item.checked { muted } else { ink })
            ]
            .spacing(6)
            .padding(iced::Padding::default().left(item.level.min(8) as f32 * 12.0)),
        );
    }

    let mut chips = Vec::new();
    for reminder in &details.reminders {
        chips.push(chip(format!("Reminder · {reminder}"), ink, metrics));
    }
    for source in &details.sources {
        chips.push(chip(format!("Source · {source}"), ink, metrics));
    }
    for label in &note.labels {
        chips.push(chip(label.clone(), ink, metrics));
    }
    for category in &details.categories {
        chips.push(chip(format!("Category · {category}"), ink, metrics));
    }
    for tag in &details.tags {
        chips.push(chip(format!("#{tag}"), ink, metrics));
    }
    for person in &details.collaborators {
        chips.push(chip(format!("Shared · {person}"), ink, metrics));
    }
    for value in &details.other {
        chips.push(chip(value.clone(), ink, metrics));
    }
    if note.archived {
        chips.push(chip("Archived".into(), ink, metrics));
    }
    if !chips.is_empty() {
        content = content.push(row(chips).spacing(6).wrap());
    }
    let mut timestamps = column![].spacing(3).width(Fill);
    if let Some(created) = details.created {
        timestamps = timestamps.push(
            text(format!("Created {created}"))
                .size(metrics.label() - 2.0)
                .color(muted),
        );
    }
    if let Some(modified) = details.modified {
        timestamps = timestamps.push(
            text(format!("Edited {modified}"))
                .size(metrics.label() - 2.0)
                .color(muted),
        );
    }
    let footer = row![
        timestamps,
        button(text("Edit").size(metrics.label()))
            .on_press(Message::KeepEdit(note.url.clone()))
            .style(move |_, status| {
                let background = match status {
                    button::Status::Hovered | button::Status::Pressed => {
                        Some(iced::Color { a: 0.08, ..ink }.into())
                    }
                    _ => None,
                };
                button::Style {
                    text_color: ink,
                    background,
                    ..Default::default()
                }
            }),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);
    content = content.push(container(footer).padding(iced::Padding::default().top(6)));
    container(content)
        .padding(metrics.padding)
        .width(Fill)
        .style(move |theme| {
            let mut style = container::bordered_box(theme);
            style.border.radius = 10.0.into();
            style.border.color = iced::Color { a: 0.18, ..ink };
            style.text_color = Some(ink);
            style.background = Some(color.unwrap_or(iced::Color::WHITE).into());
            style
        })
        .into()
}

fn chip<'a>(label: String, ink: iced::Color, metrics: &Metrics) -> Element<'a, Message> {
    container(text(label).size(metrics.label() - 1.0).color(ink))
        .padding([4, 9])
        .style(move |_| container::Style {
            background: Some(iced::Color { a: 0.08, ..ink }.into()),
            border: iced::Border {
                radius: 14.0.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

fn card_color(value: &str) -> Option<iced::Color> {
    let value = value.strip_prefix("rgb(")?.strip_suffix(')')?;
    let parts: Vec<f32> = value
        .split(',')
        .filter_map(|p| p.trim().parse().ok())
        .collect();
    (parts.len() == 3)
        .then(|| iced::Color::from_rgb(parts[0] / 255.0, parts[1] / 255.0, parts[2] / 255.0))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LabelOption(Option<String>);
impl std::fmt::Display for LabelOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_deref().unwrap_or("All labels"))
    }
}

fn scope_button<'a>(
    label: &'a str,
    scope: Scope,
    selected: Scope,
    metrics: &Metrics,
) -> Element<'a, Message> {
    button(text(label).size(metrics.label()))
        .padding(4)
        .on_press(Message::KeepScopeChanged(scope))
        .style(if scope == selected {
            button::primary
        } else {
            button::secondary
        })
        .into()
}
