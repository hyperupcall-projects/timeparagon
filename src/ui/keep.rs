//! Google Keep's read-only browser snapshots with local search and labels.
use super::layout::Metrics;
use crate::app::{Message, State};
use crate::keep::{Note, Scope};
use iced::widget::{
    button, column, container, image, pick_list, row, scrollable, text, text_input,
};
use iced::{Element, Fill, Length};

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
    let mut cards = column![].spacing(metrics.spacing).width(Fill);
    if notes.is_empty() && !keep.loading {
        cards = cards.push(
            text(if keep.snapshot.notes.is_empty() {
                "No Keep notes loaded."
            } else {
                "No matching notes."
            })
            .size(metrics.label()),
        );
    }
    for note in notes {
        cards = cards.push(card(note, keep.text_view.contains(&note.id), metrics));
    }
    body.push(scrollable(cards).height(Fill))
        .padding(metrics.padding)
        .height(Fill)
        .width(Fill)
        .into()
}
fn card<'a>(note: &'a Note, text_view: bool, metrics: &Metrics) -> Element<'a, Message> {
    let mut content = column![].spacing(metrics.spacing).width(Fill);
    if let Some(handle) = note.preview_handle.as_ref().filter(|_| !text_view) {
        content = content.push(image(handle.clone()).width(Fill));
    } else {
        content = content.push(text(&note.title).size(metrics.label() + 2.0));
        if note.checklist.is_empty() {
            content = content.push(text(&note.description).size(metrics.label()));
        }
        for item in &note.checklist {
            content = content.push(
                text(format!(
                    "{}{} {}",
                    "  ".repeat(item.level.min(8)),
                    if item.checked { "☑" } else { "☐" },
                    item.text
                ))
                .size(metrics.label()),
            );
        }
        for label in &note.labels {
            content = content.push(text(label).size(metrics.label()));
        }
        for meta in &note.metadata {
            content = content.push(text(meta).size(metrics.label()));
        }
    }
    if note.preview_handle.is_some() {
        content = content.push(
            button(
                text(if text_view {
                    "Original appearance"
                } else {
                    "Readable text"
                })
                .size(metrics.label()),
            )
            .on_press(Message::KeepTextToggled(note.id.clone()))
            .width(Fill),
        );
    }
    if note.pinned {
        content = content.push(text("Pinned").size(metrics.label()));
    }
    if note.archived {
        content = content.push(text("Archived").size(metrics.label()));
    }
    content = content.push(
        button(text("Edit").size(metrics.label()))
            .on_press(Message::KeepEdit(note.url.clone()))
            .width(Length::Fill),
    );
    let color = note
        .color
        .strip_prefix("rgb(")
        .and_then(|s| s.strip_suffix(')'))
        .and_then(|s| {
            let parts: Vec<f32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
            (parts.len() == 3).then(|| {
                iced::Color::from_rgb(parts[0] / 255.0, parts[1] / 255.0, parts[2] / 255.0)
            })
        });
    container(content)
        .padding(metrics.spacing)
        .width(Fill)
        .style(move |theme| {
            let mut style = container::bordered_box(theme);
            if let Some(color) = color {
                style.background = Some(color.into());
                style.text_color = Some(
                    if color.r * 0.299 + color.g * 0.587 + color.b * 0.114 > 0.5 {
                        iced::Color::BLACK
                    } else {
                        iced::Color::WHITE
                    },
                );
            }
            style
        })
        .into()
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
