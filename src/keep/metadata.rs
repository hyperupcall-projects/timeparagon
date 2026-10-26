//! Turn browser metadata (including older cached snapshots) into card fields.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct Metadata {
    pub created: Option<String>,
    pub modified: Option<String>,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub reminders: Vec<String>,
    pub collaborators: Vec<String>,
    pub sources: Vec<String>,
    pub other: Vec<String>,
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty() && !values.iter().any(|v| v.eq_ignore_ascii_case(value)) {
        values.push(value.to_owned());
    }
}

fn after_prefix<'a>(value: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|prefix| {
        let start = value.get(..prefix.len())?;
        if !start.eq_ignore_ascii_case(prefix) {
            return None;
        }
        let rest = value.get(prefix.len()..)?;
        if !rest.starts_with([':', ' ', '\t']) {
            return None;
        }
        let rest = rest.trim_start_matches([':', ' ', '\t']);
        (!rest.is_empty()).then_some(rest)
    })
}

impl Metadata {
    pub(super) fn parse(&mut self, values: &[String], labels: &[String]) {
        for value in values.iter().flat_map(|value| value.lines()) {
            let value = value.trim();
            if value.is_empty()
                || labels.iter().any(|label| label.eq_ignore_ascii_case(value))
                || [
                    "Remove label",
                    "Delete task",
                    "Remove source",
                    "Remove reminder",
                    "Edit labels",
                    "Pin",
                    "Unpin",
                ]
                .contains(&value)
            {
                continue;
            }
            if let Some(date) = after_prefix(
                value,
                &["Created on", "Created at", "Created", "Creation date"],
            ) {
                self.created.get_or_insert_with(|| date.to_owned());
            } else if let Some(date) = after_prefix(
                value,
                &[
                    "Last modified",
                    "Last edited",
                    "Modified",
                    "Edited",
                    "Updated",
                ],
            ) {
                // The tooltip often supplies a fuller timestamp than the footer.
                if self
                    .modified
                    .as_ref()
                    .is_none_or(|old| date.len() > old.len())
                {
                    self.modified = Some(date.to_owned());
                }
            } else if let Some(category) = after_prefix(value, &["Category", "Categories"]) {
                for part in category.split(',') {
                    push_unique(&mut self.categories, part);
                }
            } else if let Some(tags) = after_prefix(value, &["Tags", "Tag", "Labels", "Label"]) {
                for part in tags.split(',') {
                    push_unique(&mut self.tags, part.trim().trim_start_matches('#'));
                }
            } else if let Some(reminder) =
                after_prefix(value, &["Reminder", "Remind me", "Remind", "Due"])
            {
                push_unique(&mut self.reminders, reminder);
            } else if let Some(person) = after_prefix(
                value,
                &["Collaborators", "Collaborator", "Shared with", "Owner"],
            ) {
                push_unique(&mut self.collaborators, person);
            } else if let Some(source) = after_prefix(value, &["Source", "Sources"]) {
                push_unique(&mut self.sources, source);
            } else if value.starts_with('#') && !value.contains(' ') {
                push_unique(&mut self.tags, value.trim_start_matches('#'));
            } else if value.contains('@') && !value.contains(' ') {
                push_unique(&mut self.collaborators, value);
            } else if value.contains(":")
                && [
                    "Today",
                    "Tomorrow",
                    "Yesterday",
                    "Mon",
                    "Tue",
                    "Wed",
                    "Thu",
                    "Fri",
                    "Sat",
                    "Sun",
                ]
                .iter()
                .any(|prefix| value.starts_with(prefix))
            {
                push_unique(&mut self.reminders, value);
            } else {
                push_unique(&mut self.other, value);
            }
        }
        self.tags
            .retain(|tag| !labels.iter().any(|label| label.eq_ignore_ascii_case(tag)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_and_deduplicates_old_cache_metadata_without_leaking_controls() {
        let mut details = Metadata::default();
        let raw = [
            "Created Sep 17, 2026, 8:30 AM\nLast edited: Oct 24, 2026, 9:45 AM",
            "Edited Oct 24",
            "Category: Personal",
            "Tags: #routine, morning",
            "Reminder: Today, 8:00 AM",
            "Reminder: Today, 8:00 AM",
            "Shared with: friend@example.com",
            "Source: Schedule",
            "Habits & Rituals",
            "Remove label",
            "Something else",
        ];
        details.parse(&raw.map(str::to_owned), &["Habits & Rituals".into()]);
        assert_eq!(details.created.as_deref(), Some("Sep 17, 2026, 8:30 AM"));
        assert_eq!(details.modified.as_deref(), Some("Oct 24, 2026, 9:45 AM"));
        assert_eq!(details.categories, ["Personal"]);
        assert_eq!(details.tags, ["routine", "morning"]);
        assert_eq!(details.reminders, ["Today, 8:00 AM"]);
        assert_eq!(details.collaborators, ["friend@example.com"]);
        assert_eq!(details.sources, ["Schedule"]);
        assert_eq!(details.other, ["Something else"]);
    }
    #[test]
    fn preserves_structured_fields_and_does_not_invent_dates() {
        let mut details = Metadata {
            created: Some("2026-09-17T08:30:00Z".into()),
            ..Default::default()
        };
        details.parse(&["Created Sep 17".into(), "Createdyesterday".into()], &[]);
        assert_eq!(details.created.as_deref(), Some("2026-09-17T08:30:00Z"));
        assert!(details.modified.is_none());
        assert_eq!(details.other, ["Createdyesterday"]);
    }
}
