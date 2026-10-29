//! Project assignments live on each event's private Google Calendar copy.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Selection {
    pub ids: Vec<u64>,
    pub query: String,
    pub open: bool,
}

#[derive(Debug, Clone)]
pub enum Action {
    Add(u64),
    Remove(u64),
    Query(String),
    Toggle,
}

impl Selection {
    pub fn update(&mut self, action: Action) {
        match action {
            Action::Add(id) => {
                self.ids.push(id);
                self.ids.sort_unstable();
                self.ids.dedup();
                self.query.clear();
            }
            Action::Remove(id) => self.ids.retain(|value| *value != id),
            Action::Query(query) => self.query = query,
            Action::Toggle => {
                self.open = !self.open;
                self.query.clear();
            }
        }
    }
}

pub(super) fn read(properties: &serde_json::Value) -> Vec<u64> {
    let mut ids: Vec<u64> = properties["private"]["ttProjects"]
        .as_str()
        .and_then(|value| serde_json::from_str(value).ok())
        .unwrap_or_default();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// PATCH merges this key with other private properties. Sending `[]` also
/// clears the last assignment. Reject oversize values rather than letting
/// Google silently truncate them (the limit is 1024 characters per value).
pub(super) fn write(body: &mut serde_json::Value, ids: &[u64]) -> Result<(), String> {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let value = serde_json::to_string(&ids).map_err(|error| error.to_string())?;
    if value.len() > 1024 {
        return Err("Too many projects for one calendar event".into());
    }
    body["extendedProperties"]["private"]["ttProjects"] = value.into();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_round_trip_without_changing_other_metadata() {
        let mut body = serde_json::json!({"extendedProperties": {"private": {"ttKind": "habit"}}});
        write(&mut body, &[7, 3, 7]).unwrap();
        assert_eq!(read(&body["extendedProperties"]), vec![3, 7]);
        assert_eq!(body["extendedProperties"]["private"]["ttKind"], "habit");
        write(&mut body, &[]).unwrap();
        assert!(read(&body["extendedProperties"]).is_empty());
        assert_eq!(body["extendedProperties"]["private"]["ttProjects"], "[]");
    }

    #[test]
    fn old_or_malformed_properties_are_unassigned() {
        for value in [
            serde_json::json!({}),
            serde_json::json!({"private": {"ttProjects": "oops"}}),
        ] {
            assert!(read(&value).is_empty());
        }
    }

    #[test]
    fn oversized_assignments_are_rejected() {
        assert!(write(&mut serde_json::json!({}), &(1..1000).collect::<Vec<_>>()).is_err());
    }
}
